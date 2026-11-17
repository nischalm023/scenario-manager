//! Order parents select machine work and retain the order HTM through every
//! pick. Their children own retrieval, navigation, custody transfer and storage.
//!
//! Each active order owns a waiting berth across repeated presentations.
//! Inventory collection runs independently so retained HTMs can advance while
//! their totes are presented and subsequent picks are prepared.

use crate::pathfinding::{PathCacheKey, PathfinderError};
use desgraph::{CellId, StationId, StationType};
use desmap::{ConveyorMachine, MachinePresentation, OrderHtmArea};
use rustc_hash::FxHashMap;
use serde_json::Value;
use std::mem;
use tokio::select;

use super::fetch_bound::FetchBounds;
use super::induct_container::InductContainer;
use super::outbound::OutboundStations;
use super::pending_orders::{ClaimNextOrder, PendingOrders};
use super::pending_returns::{PendingReturns, ReturnWork};
use super::preparation::PairPreparation;
use super::present_tote::PrepareOrderPresentation;
use super::progress::{BoundaryCrossing, OrderProgress};
use super::retrieve_tote::{RetrieveAndDeliverTote, RetrieveTote};
use super::store_tote::StoreTote;
use super::supply_conveyor::{CollectingSupply, InventoryPipeline, SupplyAndCollect, SupplyConveyor};
use super::tote_handoff::{ExtractOrder, ExtractedOrder};
use super::transfer_items::TransferItems;
use crate::composition::{policy, resources::Orders};
use crate::events::{
    EventJournal, InventoryVisitPhase, InventoryVisitPhaseEvent, OrderStationPhase, PresentationStream, ToteMovedEvent,
    TotePresentation, ToteSource, ToteTransitEvent, TripEndpoint, TripPurpose,
};
use crate::flows::transport::{AdmitArea, HtmPosition, Navigate, PathCommitObserver};
use crate::flows::ttp::demand::order_completion::OrderCompletionTracker;
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::flows::ttp::task::clearance::ClearPresentation;
use crate::model::ParkingRegistry;
use crate::model::ToteType;
use crate::model::area_admission::AreaAdmission;
use crate::model::conveyance::{Conveyance, Conveyances};
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::navigation::Navigation;
use crate::model::order::{ExitPath, OrderContainer};
use crate::model::relay::ToteSearchCriteria;
use crate::observer::time_slice::ToteVisualState;
use crate::pathfinding::Orientation;
use crate::prelude::*;
use crate::scenario_gen::RackUse;
use crate::task;
use crate::task::TaskContext;

#[task]
#[derive(Display)]
#[display("FulfillOrders")]
pub struct FulfillOrders {
    orders: Vec<OrderSpec>,
    max_active_orders_per_machine: usize,
    inventory_caps: Vec<u32>,
    inventory_fetch_leads: Vec<SimDuration>,
    machines: Vec<(ConveyorMachine, Conveyance)>,
    inventory_pipeline: Arc<InventoryPipeline>,
    admission: Arc<AreaAdmission<StationId, OrderId>>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    completion: OrderCompletionTracker,
    extraction: OutboundStations,
    induction: Vec<(StationId, CellId)>,

    pick_time: SimDuration,
    extraction_time: SimDuration,
    buffer_dwell: SimDuration,
    release_all_at_start: bool,
    direct_extraction: bool,
    bypass_extraction: bool,
    bypass_box_induction: bool,
    embargo: SimTime,
}

#[async_trait(?Send)]
impl Task for FulfillOrders {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let orders = ctx.state::<Orders>()?.service.snapshot();
        let bypass_extraction = ctx.parameter("bypass_extraction", false)?;
        let bypass_box_induction = ctx.parameter("bypass_box_induction", false)?;
        if bypass_extraction || bypass_box_induction {
            eprintln!(
                "[flow isolation] bypass_extraction={bypass_extraction} bypass_box_induction={bypass_box_induction}"
            );
        }
        let world = world_inputs(ctx.states())?;
        let machines = ctx.state::<Conveyances>()?.machines().to_vec();
        if machines.is_empty() && !orders.is_empty() {
            return Err(super::invalid("FulfillOrders needs a declared conveyor machine"));
        }
        for (machine, _) in &machines {
            if machine.dropoff_cell.is_none()
                || machine.collection_cell.is_none()
                || machine.waiting_cells.is_empty()
                || machine.presentations.len() != 2
                || machine
                    .presentations
                    .iter()
                    .any(|presentation| presentation.waiting_cells.is_empty() || presentation.order_area.is_none())
                || machine.waiting_cells.iter().any(|waiting| {
                    machine.presentations.iter().filter(|p| p.waiting_cells.contains(waiting)).count() != 1
                })
                || machine.inventory_holding_capacity() == 0
            {
                return Err(super::invalid(format!(
                    "machine {} is missing required work locations",
                    machine.conveyor_group
                )));
            }
        }
        let mut induction: Vec<_> = world
            .data
            .map
            .stations
            .iter()
            .filter(|station| station.station_type == StationType::BoxInduction)
            .map(|station| (station.id, station.delivery_cell()))
            .collect();
        induction.sort_unstable();
        if !bypass_box_induction
            && induction.is_empty()
            && orders.iter().any(|order| order.container == OrderContainer::Carton)
        {
            return Err(super::invalid("carton orders require a declared box-induction presentation"));
        }
        let extraction = OutboundStations::new(&world.data.map.stations);
        if !bypass_extraction {
            for order in &orders {
                extraction.for_order(order)?;
            }
        }
        let admission = ctx.state::<AreaAdmission<StationId, OrderId>>()?;
        for (machine, _) in &machines {
            for presentation in &machine.presentations {
                admission
                    .declare_area(presentation.station_id, presentation.waiting_cells.len())
                    .map_err(|error| super::invalid(format!("order area declaration: {error:?}")))?;
            }
        }
        let parking = ctx.state::<ParkingRegistry>()?;
        let navigation = ctx.state::<Navigation>()?;
        let mut bounds = FetchBounds::new(&world.data, &navigation, &world.config)?;
        bounds.admission.multiplier = ctx.parameter("queue_factor", world.config.queue_factor)?;
        bounds.admission.period = policy::duration(
            "FulfillOrders.pick_secs",
            ctx.parameter("pick_secs", world.config.operator.time_per_pick_secs)?,
        )?;
        let inventory_pipeline = ctx.state::<InventoryPipeline>()?;
        let mut inventory_caps = Vec::new();
        let mut inventory_fetch_leads = Vec::new();
        for (machine, conveyor) in &machines {
            let estimate = bounds.for_machine(&navigation, machine)?;
            let cap = bounds.admission.cap(estimate.total());
            inventory_pipeline
                .declare_consumer(conveyor.id(), cap as usize)
                .map_err(|error| super::invalid(format!("inventory pipeline declaration: {error:?}")))?;
            eprintln!(
                "[inventory admission] machine={} qf={:.2} Fmax={:.1}s period={:.1}s cap={} (retrieval through pick); VTM={:.1}s empty approach={:.1}s loaded={:.1}s inbound={:.1}s",
                machine.conveyor_group,
                bounds.admission.multiplier,
                estimate.total().as_ms() as f64 / 1000.0,
                bounds.admission.period.as_ms() as f64 / 1000.0,
                cap,
                estimate.vtm_retrieval.as_ms() as f64 / 1000.0,
                estimate.empty_htm_approach.as_ms() as f64 / 1000.0,
                estimate.loaded_travel.as_ms() as f64 / 1000.0,
                estimate.inbound_conveyor.as_ms() as f64 / 1000.0
            );
            inventory_caps.push(cap);
            inventory_fetch_leads.push(estimate.total());
        }
        if orders.iter().any(|order| order.container == OrderContainer::Carton) {
            if parking.candidate_count() == 0 {
                return Err(super::invalid("carton extraction requires relay egress for its released HTMs"));
            }
            let origins: Vec<_> = if bypass_extraction {
                machines
                    .iter()
                    .flat_map(|(machine, _)| {
                        machine
                            .presentations
                            .iter()
                            .map(|presentation| presentation.order_area.as_ref().unwrap().egress)
                    })
                    .collect()
            } else {
                extraction.all().map(|(_, cell)| cell).collect()
            };
            let models = navigation.models();
            if models.is_empty() {
                return Err(super::invalid("carton extraction requires an eligible HTM model"));
            }
            for from in origins {
                for model in models {
                    let has_egress =
                        navigation.map.graph.cells().filter(|cell| parking.is_candidate(cell.id)).any(|cell| {
                            navigation
                                .pathfinder
                                .find_path(from, Orientation::East, cell.id, model.estimate[0].as_ref(), SimTime::ZERO)
                                .is_ok()
                        });
                    if !has_egress {
                        return Err(super::invalid(format!(
                            "carton extraction cell {from:?} has no relay egress for model {:?}",
                            model.id
                        )));
                    }
                }
            }
        }

        Ok(Self {
            orders,
            max_active_orders_per_machine: ctx.parameter("max_active_orders_per_machine", usize::MAX)?,
            machines,
            inventory_caps,
            inventory_fetch_leads,
            inventory_pipeline,
            admission,
            navigation,
            relay: ctx.state::<RelayService>()?.as_ref().clone(),
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            completion: ctx.state::<OrderCompletionTracker>()?.as_ref().clone(),
            extraction,
            induction,

            pick_time: policy::duration(
                "FulfillOrders.pick_secs",
                ctx.parameter("pick_secs", world.config.operator.time_per_pick_secs)?,
            )?,
            extraction_time: policy::duration(
                "FulfillOrders.extraction_secs",
                ctx.parameter("extraction_secs", world.config.order.extraction_secs)?,
            )?,
            buffer_dwell: policy::duration(
                "FulfillOrders.buffer_dwell_secs",
                ctx.parameter("buffer_dwell_secs", world.config.order.buffer_dwell_secs)?,
            )?,
            release_all_at_start: world.config.order.release_all_at_start,
            direct_extraction: world.config.order.direct_extraction,
            bypass_extraction,
            bypass_box_induction,
            embargo: SimTime::ZERO + policy::duration("pick.embargo_secs", world.config.pick.embargo_secs)?,
        })
    }

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        let fields =
            config.as_object().ok_or_else(|| super::invalid("FulfillOrders configuration must be an object"))?;
        for (key, value) in fields {
            if matches!(key.as_str(), "bypass_extraction" | "bypass_box_induction") {
                if !value.is_boolean() {
                    return Err(super::invalid(format!("{key} must be a boolean")));
                }
                continue;
            }
            if key == "max_active_orders_per_machine" {
                if value.as_u64().and_then(|n| usize::try_from(n).ok()).is_none_or(|n| n == 0) {
                    return Err(super::invalid("max_active_orders_per_machine must be a positive integer"));
                }
                continue;
            }
            if !matches!(key.as_str(), "pick_secs" | "extraction_secs" | "buffer_dwell_secs" | "queue_factor") {
                return Err(super::invalid(format!("unknown FulfillOrders parameter {key}")));
            }
            policy::duration(key, value.as_f64().ok_or_else(|| super::invalid(format!("{key} must be numeric")))?)?;
        }
        Ok(())
    }
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        if self.orders.is_empty() {
            return Ok(());
        }
        if self.bots.count_kind(BotKind::Htm) < 2 {
            return Err(super::invalid(
                "FulfillOrders needs at least two HTMs: one for the order tote and one for inventory transport",
            ));
        }
        // Inventory demand and upstream order preparation are sized from the
        // consumer's fetch pipeline. Physical resident/return rights remain
        // independently limited by each presentation's waiting cells.
        let mut streams = Vec::new();
        let desired: Vec<_> =
            self.inventory_caps.iter().map(|&cap| (cap as usize).min(self.max_active_orders_per_machine)).collect();
        for row in 0..desired.iter().copied().max().unwrap_or(0) {
            for (machine_index, (machine, _)) in self.machines.iter().enumerate() {
                if row < desired[machine_index] {
                    let presentation = machine.presentations[row % machine.presentations.len()].clone();
                    streams.push((machine_index, presentation));
                }
            }
        }
        streams.truncate(self.orders.len());
        for (index, (machine, _)) in self.machines.iter().enumerate() {
            eprintln!(
                "[order preparation] machine={} streams={} resident capacity={} (physical resources admitted separately)",
                machine.conveyor_group,
                streams.iter().filter(|(i, _)| *i == index).count(),
                machine.waiting_cells.len()
            );
        }
        let pending =
            PendingOrders::new(std::mem::take(&mut self.orders))?.with_release_all_at_start(self.release_all_at_start);
        let returns: Vec<_> = self.machines.iter().map(|_| PendingReturns::new()).collect();
        let mut workers = Vec::new();
        for (machine_index, presentation) in streams {
            let (machine, conveyor) = &self.machines[machine_index];
            workers.push(ctx.add_task(FulfillOrderStream {
                pending: pending.clone(),
                returns: returns[machine_index].clone(),
                presentation,
                machine: machine.clone(),
                conveyor: conveyor.clone(),
                inventory_pipeline: self.inventory_pipeline.clone(),
                inventory_fetch_lead: self.inventory_fetch_leads[machine_index],
                admission: self.admission.clone(),
                navigation: self.navigation.clone(),
                relay: self.relay.clone(),
                bots: self.bots.clone(),
                inventory: self.inventory.clone(),
                available: self.available.clone(),
                parking: self.parking.clone(),
                events: self.events.clone(),
                completion: self.completion.clone(),
                extraction: self.extraction.clone(),
                induction: self.induction.clone(),

                pick_time: self.pick_time.clone(),
                extraction_time: self.extraction_time.clone(),
                buffer_dwell: self.buffer_dwell.clone(),
                direct_extraction: self.direct_extraction,
                bypass_extraction: self.bypass_extraction,
                bypass_box_induction: self.bypass_box_induction,
                embargo: self.embargo.clone(),
            }));
        }
        let mut outbound = Vec::new();
        for worker in workers {
            outbound.extend(worker.await);
        }
        for completion in outbound {
            completion.await;
        }
        for (_, conveyor) in &self.machines {
            if conveyor.unfinished() != 0 || !conveyor.abandoned_custody().is_empty() {
                return Err(super::invalid(format!("machine {:?} has unfinished or abandoned custody", conveyor.id())));
            }
        }
        Ok(())
    }
}

#[derive(Display)]
#[display("FulfillOrderStream({:?})", presentation.station_id)]
struct FulfillOrderStream {
    pending: PendingOrders,
    returns: PendingReturns,
    presentation: MachinePresentation,
    machine: ConveyorMachine,
    conveyor: Conveyance,
    inventory_pipeline: Arc<InventoryPipeline>,
    inventory_fetch_lead: SimDuration,
    admission: Arc<AreaAdmission<StationId, OrderId>>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    completion: OrderCompletionTracker,
    extraction: OutboundStations,
    induction: Vec<(StationId, CellId)>,

    pick_time: SimDuration,
    extraction_time: SimDuration,
    buffer_dwell: SimDuration,
    direct_extraction: bool,
    bypass_extraction: bool,
    bypass_box_induction: bool,
    embargo: SimTime,
}

#[async_trait(?Send)]
impl Task for FulfillOrderStream {
    type Proposal = EmptyProposal;
    type Schedule = Vec<Completion<()>>;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let mut outbound = Vec::new();
        let mut available_at = self.embargo;
        while let Some((order, assigned_at)) =
            ctx.add_task(ClaimNextOrder::new(self.pending.clone(), available_at, ctx.barrier().clock())).await
        {
            let progress = OrderProgress::new(
                order.id,
                self.conveyor.id(),
                self.presentation.station_id,
                None,
                self.events.clone(),
            );
            progress.phase(OrderStationPhase::Assigned, assigned_at);
            let extraction = if self.bypass_extraction { Vec::new() } else { self.extraction.for_order(&order)? };
            let (next_available, dispatch) = ctx
                .add_task(FulfillOrder {
                    order,
                    progress,
                    preparation: Arc::new(PairPreparation::default()),
                    assigned_at,
                    returns: self.returns.clone(),
                    presentation: self.presentation.clone(),
                    machine: self.machine.clone(),
                    conveyor: self.conveyor.clone(),
                    inventory_pipeline: self.inventory_pipeline.clone(),
                    inventory_fetch_lead: self.inventory_fetch_lead,
                    admission: self.admission.clone(),
                    navigation: self.navigation.clone(),
                    relay: self.relay.clone(),
                    bots: self.bots.clone(),
                    inventory: self.inventory.clone(),
                    available: self.available.clone(),
                    parking: self.parking.clone(),
                    events: self.events.clone(),
                    completion: self.completion.clone(),
                    extraction,
                    induction: self.induction.clone(),

                    pick_time: self.pick_time.clone(),
                    extraction_time: self.extraction_time.clone(),
                    buffer_dwell: self.buffer_dwell.clone(),
                    release_all_at_start: self.pending.release_all_at_start(),
                    direct_extraction: self.direct_extraction,
                    bypass_extraction: self.bypass_extraction,
                    bypass_box_induction: self.bypass_box_induction,
                    embargo: self.embargo.clone(),
                })
                .await;
            available_at = next_available;
            outbound.extend(dispatch);
        }
        Ok(outbound)
    }
}

#[derive(Display)]
#[display("FulfillOrder({:?})", order.id)]
struct FulfillOrder {
    order: OrderSpec,
    progress: OrderProgress,
    preparation: Arc<PairPreparation>,
    assigned_at: SimTime,
    returns: PendingReturns,
    presentation: MachinePresentation,
    machine: ConveyorMachine,
    conveyor: Conveyance,
    inventory_pipeline: Arc<InventoryPipeline>,
    inventory_fetch_lead: SimDuration,
    admission: Arc<AreaAdmission<StationId, OrderId>>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    completion: OrderCompletionTracker,
    extraction: Vec<(StationId, CellId)>,
    induction: Vec<(StationId, CellId)>,

    pick_time: SimDuration,
    extraction_time: SimDuration,
    buffer_dwell: SimDuration,
    release_all_at_start: bool,
    direct_extraction: bool,
    bypass_extraction: bool,
    bypass_box_induction: bool,
    embargo: SimTime,
}

impl FulfillOrder {
    fn retrieve(&self, criteria: ToteSearchCriteria, target: CellId, after: SimTime) -> RetrieveTote {
        RetrieveTote {
            criteria,
            target,
            after,
            relay: self.relay.clone(),
            inventory: self.inventory.clone(),
            events: self.events.clone(),
            aim_version: Version::new(),
            admission: None,
            pickup_headroom: None,
        }
    }

    fn supply_inventory(&self, sku: SkuId, after: SimTime, pick_index: u32) -> SupplyAndCollect {
        let retrieval =
            self.retrieve(ToteSearchCriteria::ContainsSku { sku }, self.machine.dropoff_cell.unwrap(), after);
        let supply = SupplyConveyor::new(
            RetrieveAndDeliverTote {
                retrieval: Some(retrieval),
                navigation: self.navigation.clone(),
                bots: self.bots.clone(),
                inventory: self.inventory.clone(),
                parking: self.parking.clone(),
                events: self.events.clone(),
            },
            self.inventory_pipeline.as_ref().clone(),
            (self.order.id, pick_index),
            self.conveyor.clone(),
        )
        .on_loaded({
            let preparation = self.preparation.clone();
            move || preparation.own_inventory()
        })
        .with_retrieval_release({
            let preparation = self.preparation.clone();
            let fetch_lead = self.inventory_fetch_lead;
            async move { preparation.inventory_release(fetch_lead).await }
        });
        SupplyAndCollect::new(
            supply,
            self.returns.clone(),
            self.machine.collection_cell.unwrap(),
            self.relay.clone(),
            self.available.clone(),
        )
    }

    fn navigate(&self, position: HtmPosition, to: CellId, tote: Option<ToteId>, ctx: &SchedCtx) -> Navigate {
        Navigate::new(
            position,
            to,
            tote,
            self.navigation.clone(),
            self.bots.clone(),
            self.inventory.clone(),
            self.parking.clone(),
            self.events.clone(),
            ctx.barrier().clock(),
        )
    }

    async fn move_tote(&self, mut handoff: HtmCarryingTote, to: CellId, ctx: &SchedCtx) -> HtmCarryingTote {
        let position = ctx.add_task(self.navigate(position_of(&handoff), to, Some(handoff.tote_id), ctx)).await;
        set_position(&mut handoff, position);
        handoff
    }

    fn extraction(&self, handoff: &HtmCarryingTote) -> Result<(StationId, CellId), PlanError> {
        if self.bypass_extraction {
            return Ok((self.presentation.station_id, self.presentation.order_area.as_ref().unwrap().egress));
        }
        let cache = self.navigation.cache(handoff.cell_reservation.robot())?;
        let mut candidates = Vec::new();
        for &(station, cell) in &self.extraction {
            let key = PathCacheKey {
                from: handoff.cell_reservation.cell(),
                from_orientation: handoff.orientation,
                to: cell,
                carrying_cargo: true,
            };
            let quote = if self.navigation.models().len() == 1 {
                cache.find_path(key).map(|path| path.travel_time)
            } else {
                cache.travel_time(key)
            };
            match quote {
                Ok(time) => candidates.push((time, station, cell)),
                Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        candidates
            .into_iter()
            .min()
            .map(|(_, station, cell)| (station, cell))
            .ok_or_else(|| super::invalid("no reachable extraction station"))
    }
}

fn position_of(handoff: &HtmCarryingTote) -> HtmPosition {
    HtmPosition {
        cell_reservation: handoff.cell_reservation.clone(),
        action_ready_at: handoff.action_ready_at,
        orientation: handoff.orientation,
    }
}
fn set_position(handoff: &mut HtmCarryingTote, position: HtmPosition) {
    handoff.cell_reservation = position.cell_reservation;
    handoff.action_ready_at = position.action_ready_at;
    handoff.orientation = position.orientation;
}

#[async_trait(?Send)]
impl Task for FulfillOrder {
    type Proposal = EmptyProposal;
    type Schedule = (SimTime, Vec<Completion<()>>);
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let release =
            if self.release_all_at_start { SimTime::ZERO } else { self.order.arrival.unwrap_or(SimTime::ZERO) };
        let earliest = release.max(self.embargo).max(self.assigned_at);
        let mut demand = FxHashMap::<SkuId, u32>::default();
        for line in &self.order.lines {
            *demand.entry(line.sku).or_default() += line.quantity;
        }
        let mut lines: Vec<_> = demand.into_iter().collect();
        lines.sort_unstable_by_key(|&(sku, _)| sku);
        let barrier = ctx.barrier();
        let after = loop {
            let seen = self.inventory.version().seen();
            let at = earliest.max(barrier.floor());
            if self.inventory.reserve_for_order(&lines, at) {
                break at;
            }
            // Recorded future stock does not produce another notification
            // when the barrier reaches its simtime. Give the barrier the
            // simtime from which every line stays available, then recheck
            // every line together before reserving.
            let next = lines.iter().try_fold(at, |ready, &(sku, qty)| {
                self.inventory.earliest_sku_qty_lasting(sku, qty, at).map(|time| ready.max(time))
            });
            if let Some(next) = next.filter(|&next| next > at) {
                select! {
                    biased;
                    _ = self.inventory.version().past(seen) => {}
                    result = barrier.wait_until(next) => result?,
                }
            } else {
                self.inventory.version().past(seen).await;
            }
        };
        self.progress.phase(OrderStationPhase::PreparationStarted, after);
        let station = self.presentation.station_id;
        self.completion
            .register(self.order.id, station, self.order.unit_count())
            .map_err(|_| super::invalid("order was already admitted"))?;
        let area = self
            .presentation
            .order_area
            .clone()
            .ok_or_else(|| super::invalid("order presentation has no circulation area"))?;
        let entry = area.entry;
        // Accept a return right independently of the two retrieval streams.
        // Only the later committed entry route establishes physical residence.
        let admission = ctx.add_task(AdmitArea::new(
            self.admission.as_ref().clone(),
            station,
            self.order.id,
            after,
            ctx.barrier().clock(),
        ));
        let dropoff = self.machine.dropoff_cell.unwrap();
        // Prepare only the next required inventory visit: first alongside
        // container acquisition, then alongside each return to the waiting berth.
        let mut prepared_supply = self
            .order
            .lines
            .iter()
            .find(|line| line.quantity > 0)
            .map(|line| ctx.add_task(self.supply_inventory(line.sku, after, 0)));
        // A right may start at a committed future exit time. Obtain it before
        // acquiring a container so speculative orders cannot pin the only box
        // induction port or consume the fleet needed by resident orders.
        let claim = admission.await;
        let mut order = match self.order.container {
            OrderContainer::Carton => {
                let (source_station, source_cell) = if self.bypass_box_induction {
                    (None, entry)
                } else {
                    let mut candidates = Vec::new();
                    for &(station, cell) in &self.induction {
                        match self.navigation.unassigned_selection_time(PathCacheKey {
                            from: cell,
                            from_orientation: Orientation::East,
                            to: entry,
                            carrying_cargo: true,
                        }) {
                            Ok(time) => candidates.push((time, station, cell)),
                            Err(PlanError::Pathfinder(
                                PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. },
                            )) => {}
                            Err(error) => return Err(error),
                        }
                    }
                    candidates.into_iter().min().map(|(_, station, cell)| (Some(station), cell)).ok_or_else(|| {
                        super::invalid("no box-induction presentation can reach the order waiting cell")
                    })?
                };
                let mut supply = InductContainer::new(
                    source_cell,
                    ToteType::Carton,
                    ToteDesignation::Order,
                    after,
                    self.navigation.clone(),
                    self.bots.clone(),
                    self.inventory.clone(),
                    self.available.clone(),
                    self.parking.clone(),
                    self.events.clone(),
                    ctx.barrier().clock(),
                )
                .with_headroom(self.preparation.order_headroom());
                if let Some(station) = source_station {
                    supply = supply.with_station(station);
                }
                if self.bypass_box_induction {
                    supply = supply.with_instant_supply();
                }
                ctx.add_task(supply).await
            }
            OrderContainer::Tote | OrderContainer::Tray => {
                let tote_type = self.order.container;
                let mut retrieval = self.retrieve(
                    ToteSearchCriteria::EmptyOfType { designation: ToteDesignation::Order, tote_type },
                    entry,
                    after,
                );
                retrieval.pickup_headroom = Some(self.preparation.order_headroom());
                ctx.add_task(retrieval).await
            }
        };
        self.preparation.own_order(order.action_ready_at.max(claim.entry()))?;
        // A newcomer without a return right must not occupy the single
        // doorway ahead of an order that already owns one.
        let progress = self.progress.clone();
        let position = ctx
            .add_task(self.navigate(position_of(&order), entry, Some(order.tote_id), &ctx).with_commit_observer(
                Box::new({
                    let mut first = true;
                    move |window, _| {
                        if mem::take(&mut first) {
                            progress.dispatched(window);
                        }
                        Ok(())
                    }
                }),
            ))
            .await;
        set_position(&mut order, position);
        let waiting = self.presentation.waiting_cells[claim.slot()];
        self.progress.set_waiting_cell(waiting);
        let progress = self.progress.clone();
        let not_before = claim.entry();
        let position = ctx
            .add_task(
                self.navigate(position_of(&order), waiting, Some(order.tote_id), &ctx)
                    .not_before(not_before)
                    .with_commit_observer(Box::new({
                        let mut entry = BoundaryCrossing::entry(area);
                        let mut claim = Some(claim);
                        move |window, which| {
                            let Some(entered_at) = entry.in_window(window, which)? else { return Ok(()) };
                            claim
                                .take()
                                .ok_or_else(|| super::invalid("order area entered twice"))?
                                .finalize(entered_at)
                                .map_err(|error| super::invalid(format!("order admission finalize: {error:?}")))?;
                            progress.phase(OrderStationPhase::Admitted, entered_at);
                            Ok(())
                        }
                    })),
            )
            .await;
        set_position(&mut order, position);
        self.progress.phase(OrderStationPhase::Waiting, order.action_ready_at);
        let collection = self.machine.collection_cell.unwrap();
        let circuit = self
            .navigation
            .map
            .stations
            .iter()
            .find(|s| s.id == station)
            .and_then(|s| s.conveyor_transit_ms)
            .map(SimDuration::ms)
            .unwrap_or(SimDuration::ZERO);
        // Preserve the existing full-circuit, equal-leg approximation pending
        // physical calibration; queueing, picking and fork dwell are additional.
        let inbound = SimDuration::ms(circuit.as_ms() / 2);
        let outbound = circuit - inbound;
        let mut pending = Vec::new();
        let mut total_remaining = self.order.unit_count();
        let mut pick_index = 0;
        let mut order_lines = self.order.lines.iter().filter(|line| line.quantity > 0).peekable();
        while let Some(line) = order_lines.next() {
            let mut remaining = line.quantity;
            while remaining > 0 {
                let supply =
                    prepared_supply.take().ok_or_else(|| super::invalid("inventory visit has no prepared supply"))?;
                let CollectingSupply {
                    visit,
                    conveyed,
                    boarded_at,
                    collector,
                    zone_id,
                    source_relay_id,
                    presentation_id,
                    deliver_from,
                } = supply.await;
                let qty = remaining.min(self.inventory.tote(conveyed.tote())?.sku_qty_at(line.sku, boarded_at));
                if qty == 0 {
                    return Err(super::invalid("retrieved tote has none of its requested SKU"));
                }
                pending.push(collector);
                let prepared = ctx
                    .add_task(PrepareOrderPresentation::new(
                        conveyed.clone(),
                        self.conveyor.clone(),
                        boarded_at + inbound,
                        position_of(&order),
                        self.navigation.clone(),
                        vec![(self.presentation.station_id, self.presentation.cell)],
                    ))
                    .await;
                let presented = prepared.inventory;
                let presentation = self
                    .machine
                    .presentations
                    .iter()
                    .find(|presentation| presentation.station_id == presented.station())
                    .ok_or_else(|| super::invalid("conveyor selected an undeclared presentation"))?;
                self.events.emit(ToteTransitEvent {
                    tote_id: conveyed.tote(),
                    from_cell: dropoff,
                    to_cell: presentation.inventory_cell,
                    start: boarded_at,
                    end: presented.ready_at(),
                });
                self.events.emit(ToteMovedEvent {
                    tote_id: conveyed.tote(),
                    new_state: ToteVisualState::OnConveyor { cell_id: presentation.inventory_cell.0 },
                    time: presented.ready_at(),
                });
                let position = ctx
                    .add_task(
                        self.navigate(position_of(&order), presentation.cell, Some(order.tote_id), &ctx)
                            .not_before(prepared.depart_at)
                            .with_trip(
                                TripPurpose::ToStation,
                                Some(order.presentation_id),
                                TripEndpoint::Station(presentation.station_id),
                            ),
                    )
                    .await;
                set_position(&mut order, position);
                self.progress.phase(OrderStationPhase::Presenting, order.action_ready_at);
                self.progress.readiness(pick_index, PresentationStream::Order, order.action_ready_at);
                self.progress.readiness(pick_index, PresentationStream::Inventory, presented.ready_at());
                let source = TotePresentation {
                    order_id: self.order.id,
                    tote_id: conveyed.tote(),
                    source_zone_id: zone_id,
                    source_relay_id: source_relay_id
                        .ok_or_else(|| super::invalid("inventory tote has no source relay"))?,
                    presentation_id,
                };
                let (updated_order, pair) = ctx
                    .add_task(TransferItems::new(
                        order,
                        presented.clone(),
                        self.conveyor.clone(),
                        presentation.cell,
                        source,
                        line.sku,
                        qty,
                        self.pick_time,
                        self.inventory.clone(),
                        self.bots.clone(),
                        self.events.clone(),
                        self.completion.clone(),
                        None,
                    )?)
                    .await;
                order = updated_order;
                self.events.emit(InventoryVisitPhaseEvent {
                    conveyor_id: self.conveyor.id(),
                    order_id: self.order.id,
                    pick_index,
                    phase: InventoryVisitPhase::PickCompleted,
                    at: pair.end,
                });
                visit
                    .finish(pair.end)
                    .map_err(|error| super::invalid(format!("inventory pipeline completion: {error:?}")))?;
                presented.clear(pair.end, pair.end + outbound).map_err(|e| super::invalid(e.to_string()))?;
                self.events.emit(ToteTransitEvent {
                    tote_id: conveyed.tote(),
                    from_cell: presentation.inventory_cell,
                    to_cell: collection,
                    start: pair.end,
                    end: pair.end + outbound,
                });
                self.events.emit(ToteMovedEvent {
                    tote_id: conveyed.tote(),
                    new_state: ToteVisualState::OnConveyor { cell_id: collection.0 },
                    time: pair.end + outbound,
                });
                self.returns.publish(ReturnWork {
                    conveyed,
                    ready_at: pair.end + outbound,
                    station: presentation.station_id,
                    zone: zone_id,
                    source_relay: source_relay_id,
                    presentation_id,
                    deliver_from,
                })?;
                total_remaining -= qty;
                remaining -= qty;
                pick_index += 1;
                if total_remaining > 0 {
                    let next_sku = if remaining > 0 {
                        line.sku
                    } else {
                        order_lines.peek().ok_or_else(|| super::invalid("remaining order has no next line"))?.sku
                    };
                    prepared_supply = Some(ctx.add_task(self.supply_inventory(next_sku, pair.end, pick_index)));
                    order = self.move_tote(order, waiting, &ctx).await;
                    self.progress.phase(OrderStationPhase::Waiting, order.action_ready_at);
                }
            }
        }
        let (extraction_station, extraction_cell) = self.extraction(&order)?;
        let final_pick_end = order.action_ready_at;
        self.progress.phase(OrderStationPhase::FinalPickCompleted, final_pick_end);
        pending.push(ctx.add_task(DispatchOrder {
            handoff: Some(order),
            order_id: self.order.id,
            progress: self.progress.clone(),
            area: self.presentation.order_area.clone().unwrap(),
            exit_path: self.order.exit_path,
            origin: station,
            extraction_hint: (extraction_station, extraction_cell),
            extraction: self.extraction.clone(),
            admission: self.admission.clone(),
            navigation: self.navigation.clone(),
            relay: self.relay.clone(),
            bots: self.bots.clone(),
            inventory: self.inventory.clone(),
            available: self.available.clone(),
            parking: self.parking.clone(),
            events: self.events.clone(),

            extraction_time: self.extraction_time,
            buffer_dwell: self.buffer_dwell,
            direct_extraction: self.direct_extraction,
            bypass_extraction: self.bypass_extraction,
        }));
        Ok((final_pick_end, pending))
    }
}

/// Outbound work continues after the machine can admit its next order. The
/// root retains this child's completion, including its configured buffer dwell.
#[derive(Display)]
#[display("DispatchOrder({order_id:?})")]
struct DispatchOrder {
    handoff: Option<HtmCarryingTote>,
    order_id: OrderId,
    progress: OrderProgress,
    area: OrderHtmArea,
    exit_path: ExitPath,
    origin: StationId,
    extraction_hint: (StationId, CellId),
    extraction: Vec<(StationId, CellId)>,
    admission: Arc<AreaAdmission<StationId, OrderId>>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,

    extraction_time: SimDuration,
    buffer_dwell: SimDuration,
    direct_extraction: bool,
    bypass_extraction: bool,
}

fn routes_through_buffer(source: ExitPath, direct_extraction: bool) -> bool {
    source == ExitPath::Buffer && !direct_extraction
}

impl DispatchOrder {
    fn exit_observer(&self) -> PathCommitObserver {
        let admission = self.admission.clone();
        let progress = self.progress.clone();
        let area = self.area.clone();
        let order = self.order_id;
        let presentation = self.origin;
        let mut exit = BoundaryCrossing::exit(area);
        Box::new(move |window, which| {
            let Some(clear_at) = exit.in_window(window, which)? else { return Ok(()) };
            admission
                .commit_exit(&presentation, &order, clear_at)
                .map_err(|error| super::invalid(format!("order area exit: {error:?}")))?;
            progress.phase(OrderStationPhase::AreaExited, clear_at);
            Ok(())
        })
    }

    fn store(&self, handoff: HtmCarryingTote, pool: RackUse, station: StationId) -> StoreTote {
        StoreTote {
            handoff: Some(handoff),
            commit_observer: (pool == RackUse::OrderBuffer).then(|| self.exit_observer()),
            pool,
            station,
            navigation: self.navigation.clone(),
            relay: self.relay.clone(),
            bots: self.bots.clone(),
            inventory: self.inventory.clone(),
            available: self.available.clone(),
            parking: self.parking.clone(),
            events: self.events.clone(),
        }
    }
}

#[async_trait(?Send)]
impl Task for DispatchOrder {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let mut order = self.handoff.take().ok_or_else(|| super::invalid("DispatchOrder scheduled twice"))?;
        let buffered = !self.bypass_extraction && routes_through_buffer(self.exit_path, self.direct_extraction);
        if buffered {
            if !self.relay.has_order_buffer() {
                return Err(super::invalid("buffer-bound order has no order buffer"));
            }
            let tote_id = order.tote_id;
            let stored_at = ctx.add_task(self.store(order, RackUse::OrderBuffer, self.origin)).await;
            let due = stored_at + self.buffer_dwell;
            order = ctx
                .add_task(RetrieveTote {
                    criteria: ToteSearchCriteria::BufferedOrder { tote_id },
                    target: self.extraction_hint.1,
                    after: due,
                    relay: self.relay.clone(),
                    inventory: self.inventory.clone(),
                    events: self.events.clone(),
                    aim_version: Version::new(),
                    admission: None,
                    pickup_headroom: None,
                })
                .await;
        }
        let mut journey = Navigate::new(
            position_of(&order),
            self.extraction_hint.1,
            Some(order.tote_id),
            self.navigation.clone(),
            self.bots.clone(),
            self.inventory.clone(),
            self.parking.clone(),
            self.events.clone(),
            ctx.barrier().clock(),
        );
        if !self.bypass_extraction {
            journey = journey
                .with_destinations(
                    self.extraction.iter().map(|&(station, cell)| (cell, TripEndpoint::Station(station))).collect(),
                )
                .with_trip(
                    TripPurpose::ToStation,
                    Some(order.presentation_id),
                    TripEndpoint::Station(self.extraction_hint.0),
                );
        }
        if !buffered {
            journey = journey.with_commit_observer(self.exit_observer());
        }
        let position = ctx.add_task(journey).await;
        let extraction_station = if self.bypass_extraction {
            None
        } else {
            Some(
                self.extraction
                    .iter()
                    .find(|(_, cell)| *cell == position.cell_reservation.cell())
                    .map(|&(station, _)| station)
                    .ok_or_else(|| super::invalid("outbound HTM arrived outside the declared extraction ports"))?,
            )
        };
        set_position(&mut order, position);
        let extracted = ctx
            .add_task(ExtractOrder {
                order: Some(order),
                order_id: self.order_id,
                station: extraction_station,
                dwell: if self.bypass_extraction { SimDuration::ZERO } else { self.extraction_time },
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                events: self.events.clone(),
            })
            .await;
        match extracted {
            ExtractedOrder::Reusable(order) => {
                ctx.add_task(self.store(order, RackUse::OrderDepot, extraction_station.unwrap_or(self.origin))).await;
            }
            ExtractedOrder::Removed(position) => {
                let entry = self.available.register(
                    position.cell_reservation,
                    position.action_ready_at,
                    position.orientation,
                    None,
                )?;
                // The extraction cell's clearance: a cheap last resort with a
                // Vacate on the HTM's hold. A productive claim that departs at
                // the extraction's end takes the HTM; otherwise the clearance
                // parks it, or stops the run when no bay can take it by the
                // time the barrier passes the extraction's end.
                ctx.add_task(ClearPresentation::new(
                    entry,
                    extraction_station.unwrap_or(self.origin),
                    ToteSource::Pick,
                    self.navigation.clone(),
                    self.available.clone(),
                    ctx.barrier().clock(),
                ))
                .await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod config_tests {
    use serde_json::json;

    use super::FulfillOrders;
    use crate::prelude::Task;

    #[test]
    fn workflow_bypasses_are_explicit_booleans() {
        for key in ["bypass_extraction", "bypass_box_induction"] {
            for value in [json!(true), json!(false)] {
                assert!(FulfillOrders::validate_config(&json!({key: value})).is_ok());
            }
            for value in [json!(1), json!("true"), json!(null)] {
                assert!(FulfillOrders::validate_config(&json!({key: value})).is_err());
            }
        }
    }

    #[test]
    fn admission_limit_requires_a_positive_integer_and_remains_optional() {
        for config in [json!({}), json!({"max_active_orders_per_machine": 4})] {
            assert!(FulfillOrders::validate_config(&config).is_ok());
        }
        for value in [json!(0), json!(-1), json!(2.5), json!("4"), json!(null)] {
            assert!(FulfillOrders::validate_config(&json!({"max_active_orders_per_machine": value})).is_err());
        }
    }
}

#[cfg(test)]
#[path = "order_berth_tests.rs"]
mod berth_tests;
