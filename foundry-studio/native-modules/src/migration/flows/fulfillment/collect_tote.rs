//! Take an emptied HTM from the dropoff cell to the collection cell, load the
//! next returning inventory tote there and restore that tote to stock.

use desgraph::{CellId, StationId};

use super::pending_returns::{ClaimReturnedTote, PendingReturns};
use super::store_tote::StoreTote;
use super::tote_handoff::LoadTote;
use crate::events::EventJournal;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::model::ParkingRegistry;
use crate::model::conveyance::{Conveyance, RetainedHtm};
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::navigation::Navigation;
use crate::prelude::*;
use crate::scenario_gen::RackUse;

#[derive(Display)]
#[display("CollectAndStoreTote({:?})", retained.robot())]
pub(super) struct CollectAndStoreTote {
    pub position: Option<HtmPosition>,
    pub returns: PendingReturns,
    pub retained: RetainedHtm,
    pub conveyance: Conveyance,
    pub collection: CellId,
    /// Lifting the tote off the belt at the collection cell.

    /// Setting it down at the relay on the way back to stock.
    pub navigation: Arc<Navigation>,
    pub relay: RelayService,
    pub bots: BotService,
    pub inventory: InventoryService,
    pub available: AvailableHtmRegistry,
    pub parking: Option<ParkingRegistry>,
    pub events: EventJournal,
}

#[async_trait(?Send)]
impl Task for CollectAndStoreTote {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let position = self.position.take().ok_or_else(|| super::invalid("collection has no unloaded HTM"))?;
        let (trip, station) = ctx
            .add_task(CollectTote {
                position: Some(position),
                returns: self.returns.clone(),
                retained: self.retained.clone(),
                conveyance: self.conveyance.clone(),
                collection: self.collection,

                navigation: self.navigation.clone(),
                bots: self.bots.clone(),
                inventory: self.inventory.clone(),
                parking: self.parking.clone(),
                events: self.events.clone(),
            })
            .await;
        ctx.add_task(StoreTote {
            commit_observer: None,
            handoff: Some(trip),
            pool: RackUse::Inventory,
            station,
            navigation: self.navigation.clone(),
            relay: self.relay.clone(),
            bots: self.bots.clone(),
            inventory: self.inventory.clone(),
            available: self.available.clone(),
            parking: self.parking.clone(),
            events: self.events.clone(),
        })
        .await;
        Ok(())
    }
}

#[derive(Display)]
#[display("CollectTote({:?})", retained.robot())]
struct CollectTote {
    position: Option<HtmPosition>,
    returns: PendingReturns,
    retained: RetainedHtm,
    conveyance: Conveyance,
    collection: CellId,

    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
}

#[async_trait(?Send)]
impl Task for CollectTote {
    type Proposal = EmptyProposal;
    type Schedule = (HtmCarryingTote, StationId);

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let position = self.position.take().ok_or_else(|| super::invalid("collection HTM already handed off"))?;
        if position.cell_reservation.robot() != self.retained.robot() {
            return Err(super::invalid("unloaded HTM differs from the retained collection robot"));
        }
        // The next delivery needs the dropoff cell, so the emptied HTM waits for
        // its return on the collection cell.
        let position = ctx
            .add_task(Navigate::new(
                position,
                self.collection,
                None,
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                self.parking.clone(),
                self.events.clone(),
                ctx.barrier().clock(),
            ))
            .await;
        let (work, claim) = ctx
            .add_task(ClaimReturnedTote::new(
                self.returns.clone(),
                self.conveyance.clone(),
                self.retained.clone(),
                position.action_ready_at,
                ctx.barrier().clock(),
            ))
            .await;
        let dwell = self.bots.model(position.cell_reservation.robot())?.figures.conveyor_pickup_time();
        let position = ctx
            .add_task(LoadTote {
                position: Some(position),
                tote: work.conveyed.tote(),
                claim: Some(claim),
                dwell,
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                events: self.events.clone(),
            })
            .await;
        Ok((
            HtmCarryingTote {
                tote_id: work.conveyed.tote(),
                zone_id: work.zone,
                cell_reservation: position.cell_reservation,
                action_ready_at: position.action_ready_at,
                orientation: position.orientation,
                station_arrival_plan: None,
                presentation_id: work.presentation_id,
                source_relay_id: work.source_relay,
                deliver_from: work.deliver_from,
            },
            work.station,
        ))
    }
}

#[cfg(test)]
#[path = "collect_tote_tests.rs"]
mod tests;
