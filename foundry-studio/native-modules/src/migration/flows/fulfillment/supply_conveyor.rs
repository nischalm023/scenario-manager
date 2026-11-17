//! Supply a conveyor independently of its consumer: retrieve and deliver one
//! tote, unload within capacity, and hand back its retained HTM on the dropoff cell.

use std::future::Future;
use std::pin::Pin;

use desgraph::{CellId, RelayId, ZoneId};

use super::collect_tote::CollectAndStoreTote;
use super::pending_returns::PendingReturns;
use super::retrieve_tote::RetrieveAndDeliverTote;
use super::tote_handoff::UnloadTote;
use crate::events::{InventoryVisitPhase, InventoryVisitPhaseEvent, TripEndpoint};
use crate::flows::transport::{AcquirePipeline, HtmPosition, Navigate};
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::model::conveyance::{Conveyance, ConveyedTote, RetainedHtm};
use crate::model::conveyor::ConveyorId;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::pipeline_admission::{PipelineAdmission, PipelineVisit};
use crate::prelude::*;

pub(super) type InventoryPipeline = PipelineAdmission<ConveyorId, (OrderId, u32)>;

pub(super) struct ConveyorSupply {
    pub visit: PipelineVisit<ConveyorId, (OrderId, u32)>,
    pub conveyed: ConveyedTote,
    pub retained: RetainedHtm,
    pub boarded_at: SimTime,
    /// The retained HTM standing on the dropoff cell after unloading.
    pub position: HtmPosition,
    pub zone_id: ZoneId,
    pub source_relay_id: Option<RelayId>,
    pub presentation_id: PresentationId,
    pub deliver_from: Option<TripEndpoint>,
}

/// Inventory is ready independently of the collector's full return to rack.
/// The order parent retains the completion only for end-of-workload cleanup.
pub(super) struct CollectingSupply {
    pub visit: PipelineVisit<ConveyorId, (OrderId, u32)>,
    pub conveyed: ConveyedTote,
    pub boarded_at: SimTime,
    pub collector: Completion<()>,
    pub zone_id: ZoneId,
    pub source_relay_id: Option<RelayId>,
    pub presentation_id: PresentationId,
    pub deliver_from: Option<TripEndpoint>,
}

/// Once unloaded, the retained HTM must be able to collect any ready return,
/// even while the order needing its original inventory tote has no container.
#[derive(Display)]
#[display("SupplyAndCollect")]
pub(super) struct SupplyAndCollect {
    supply: Option<SupplyConveyor>,
    returns: PendingReturns,
    collection: CellId,

    relay: RelayService,
    available: AvailableHtmRegistry,
}

impl SupplyAndCollect {
    pub fn new(
        supply: SupplyConveyor,
        returns: PendingReturns,
        collection: CellId,

        relay: RelayService,
        available: AvailableHtmRegistry,
    ) -> Self {
        Self { supply: Some(supply), returns, collection, relay, available }
    }
}

#[async_trait(?Send)]
impl Task for SupplyAndCollect {
    type Proposal = EmptyProposal;
    type Schedule = CollectingSupply;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let supply = self.supply.take().ok_or_else(|| super::invalid("supply and collection already started"))?;
        let delivery = supply.delivery.as_ref().ok_or_else(|| super::invalid("supply has no delivery"))?;
        let navigation = delivery.navigation.clone();
        let bots = delivery.bots.clone();
        let inventory = delivery.inventory.clone();
        let parking = delivery.parking.clone();
        let events = delivery.events.clone();
        let conveyance = supply.conveyor.clone();
        let ConveyorSupply {
            visit,
            conveyed,
            retained,
            boarded_at,
            position,
            zone_id,
            source_relay_id,
            presentation_id,
            deliver_from,
        } = ctx.add_task(supply).await;
        let collector = ctx.add_task(CollectAndStoreTote {
            position: Some(position),
            returns: self.returns.clone(),
            retained,
            conveyance,
            collection: self.collection,

            navigation,
            relay: self.relay.clone(),
            bots,
            inventory,
            available: self.available.clone(),
            parking,
            events,
        });
        Ok(CollectingSupply {
            visit,
            conveyed,
            boarded_at,
            collector,
            zone_id,
            source_relay_id,
            presentation_id,
            deliver_from,
        })
    }
}

#[derive(Display)]
#[display("SupplyConveyor")]
pub(super) struct SupplyConveyor {
    pipeline: InventoryPipeline,
    visit_id: (OrderId, u32),
    retrieval_release: Option<Pin<Box<dyn Future<Output = SimTime>>>>,
    loaded_observer: Option<Box<dyn FnOnce()>>,
    delivery: Option<RetrieveAndDeliverTote>,
    conveyor: Conveyance,
}

impl SupplyConveyor {
    pub fn new(
        delivery: RetrieveAndDeliverTote,
        pipeline: InventoryPipeline,
        visit_id: (OrderId, u32),
        conveyor: Conveyance,
    ) -> Self {
        Self { delivery: Some(delivery), pipeline, visit_id, retrieval_release: None, loaded_observer: None, conveyor }
    }

    /// Keep logical pipeline credit while the consumer's plan is pending.
    /// The returned simulation time bounds physical retrieval, including
    /// relay/tote selection and HTM acquisition, rather than just delivery.
    pub fn with_retrieval_release(mut self, release: impl Future<Output = SimTime> + 'static) -> Self {
        self.retrieval_release = Some(Box::pin(release));
        self
    }

    pub fn on_loaded(mut self, observer: impl FnOnce() + 'static) -> Self {
        self.loaded_observer = Some(Box::new(observer));
        self
    }
}

#[async_trait(?Send)]
impl Task for SupplyConveyor {
    type Proposal = EmptyProposal;
    type Schedule = ConveyorSupply;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<ConveyorSupply, PlanError> {
        let mut delivery = self.delivery.take().ok_or_else(|| super::invalid("conveyor supply already handed off"))?;
        let mut retrieval = delivery.retrieval.take().ok_or_else(|| super::invalid("supply has no retrieval"))?;
        let permit = ctx
            .add_task(AcquirePipeline::new(
                self.pipeline.clone(),
                self.conveyor.id(),
                self.visit_id,
                retrieval.after,
                ctx.barrier().clock(),
            ))
            .await;
        let start = permit.quoted_start();
        let visit =
            permit.start(start).map_err(|error| super::invalid(format!("inventory pipeline start: {error:?}")))?;
        retrieval.events.emit(InventoryVisitPhaseEvent {
            conveyor_id: self.conveyor.id(),
            order_id: self.visit_id.0,
            pick_index: self.visit_id.1,
            phase: InventoryVisitPhase::Released,
            at: start,
        });
        retrieval.after = retrieval.after.max(start);
        if let Some(release) = self.retrieval_release.take() {
            retrieval.after = retrieval.after.max(release.await).max(ctx.barrier().floor());
        }
        let destination = retrieval.target;
        let navigation = delivery.navigation.clone();
        let bots = delivery.bots.clone();
        let inventory = delivery.inventory.clone();
        let parking = delivery.parking.clone();
        let events = delivery.events.clone();
        let mut handoff = ctx.add_task(retrieval).await;
        if let Some(observer) = self.loaded_observer.take() {
            observer();
        }
        events.emit(InventoryVisitPhaseEvent {
            conveyor_id: self.conveyor.id(),
            order_id: self.visit_id.0,
            pick_index: self.visit_id.1,
            phase: InventoryVisitPhase::Loaded,
            at: handoff.action_ready_at,
        });
        let position = ctx
            .add_task(Navigate::new(
                HtmPosition {
                    cell_reservation: handoff.cell_reservation.clone(),
                    action_ready_at: handoff.action_ready_at,
                    orientation: handoff.orientation,
                },
                destination,
                Some(handoff.tote_id),
                navigation.clone(),
                bots.clone(),
                inventory.clone(),
                parking.clone(),
                events.clone(),
                ctx.barrier().clock(),
            ))
            .await;
        handoff.cell_reservation = position.cell_reservation;
        handoff.action_ready_at = position.action_ready_at;
        handoff.orientation = position.orientation;
        let HtmCarryingTote {
            tote_id,
            zone_id,
            cell_reservation,
            action_ready_at,
            orientation,
            presentation_id,
            source_relay_id,
            deliver_from,
            ..
        } = handoff;
        let conveyor_deposit_time = bots.model(cell_reservation.robot())?.figures.conveyor_deposit_time();
        let admission = loop {
            let seen = self.conveyor.version().seen();
            if let Some(admission) = self
                .conveyor
                .try_admit(tote_id, cell_reservation.robot(), action_ready_at)
                .map_err(|error| super::invalid(error.to_string()))?
            {
                break admission;
            }
            self.conveyor.version().past(seen).await;
        };
        // On the real EasyLoop the HTM raises its lift on the approach, so the tote is on the belt as the
        // HTM reaches the dropoff cell, slowing but not necessarily stopping. The simulation models that
        // lift as the standing `conveyor_deposit_time` dwell on the cell until the fwd task "Model the EasyLoop
        // lift: raise the tote on the approach to the dropoff port instead of a standing dwell" makes it a motion.
        let (position, conveyed, retained, boarded_at) = ctx
            .add_task(UnloadTote {
                position: Some(HtmPosition { cell_reservation, action_ready_at, orientation }),
                tote: tote_id,
                conveyor: self.conveyor.id(),
                admission: Some(admission),
                dwell: conveyor_deposit_time,
                inventory: inventory.clone(),
                bots: bots.clone(),
                events: events.clone(),
            })
            .await;
        events.emit(InventoryVisitPhaseEvent {
            conveyor_id: self.conveyor.id(),
            order_id: self.visit_id.0,
            pick_index: self.visit_id.1,
            phase: InventoryVisitPhase::Boarded,
            at: boarded_at,
        });
        Ok(ConveyorSupply {
            visit,
            conveyed,
            retained,
            boarded_at,
            position,
            zone_id,
            source_relay_id,
            presentation_id,
            deliver_from,
        })
    }
}
