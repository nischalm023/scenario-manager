//! Within `demand`, `close_out` is what happens to an order AFTER its last
//! physical pick: the order stops being an abstraction at a station and becomes
//! a tote that has to be fetched, loaded, and carried out of the building.
//!
//! It owns the whole tail of that journey and the three legs it can be made of:
//!
//! - **load** — fetch an empty ORDER tote from inventory racking and load the
//!   closed order into it at the station that picked it;
//! - **extract** — carry the loaded tote to an extraction station;
//! - **store / release** — for an order whose [`ExitPath`] is `Buffer`, file
//!   the loaded tote into the order buffer instead, and fetch it back out
//!   again once it is due, at which point it takes the extract leg after all.
//!
//! The buffer is not a new kind of place: it is racking whose zones state
//! `holds: order` (world protocol's `storage` component), served by its own
//! gripper VTMs and its own relays. So a buffered tote rests where a tote is
//! allowed to rest — `ToteLocation::InRack` — and the legs that put it there
//! and take it back are the same `HtmStore`/`VtmReturn` and
//! `VtmForward`/`HtmPickup` chains everything else uses. What the buffer needs
//! that inventory racking does not is EXCLUSIVITY, and that is enforced one
//! level down, in `RelayService`, by zone racking.

use crate::composition::resources::{RunInputs, inputs};
use crate::events::EventJournal;
use crate::flows::ttp::task::clearance::ClearPresentation;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::relay::RelayService;
use crate::model::station_service::StationService;
use crate::state::{StateError, StateManager, States};
use crate::task;
use crate::task::TaskContext;
use desmap::WarehouseMap;
use serde_json::json;

use crate::composition::CloseOutTaskConfig;
use crate::composition::policy;
use crate::task::parse_config;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use desgraph::{CellId, StationId, ZoneId};
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::select;

use crate::{
    error::ReservationError,
    events::ToteSource,
    flows::ttp::{handoff::HtmCarryingTote, inflight::InflightSpan, task::*},
    model::order::ExitPath,
    model::relay::{ProposedRetrieve, ToteSearchCriteria},
    model::station::Station,
    prelude::*,
    utils::temporal_counter::TemporalCounter,
};

use super::order_completion::{ClosedOrder, OrderCompletionTracker};

/// Order totes loaded at a pick station and still owed a trip to extraction.
#[derive(Clone, Default)]
pub struct AwaitingExtraction {
    // ordered: none needed; membership only.
    inner: Arc<Mutex<FxHashMap<ToteId, ClosedOrder>>>,
}

impl AwaitingExtraction {
    pub fn new() -> Self {
        Self::default()
    }

    fn mark(&self, tote_id: ToteId, order: ClosedOrder) {
        self.inner.lock().unwrap().insert(tote_id, order);
    }

    fn take(&self, tote_id: ToteId) -> Option<ClosedOrder> {
        self.inner.lock().unwrap().remove(&tote_id)
    }

    /// The order riding this tote, without claiming it. `ClosedOrder` is
    /// `Copy`, so a look is a look.
    fn peek(&self, tote_id: ToteId) -> Option<ClosedOrder> {
        self.inner.lock().unwrap().get(&tote_id).copied()
    }

    /// Totes mid-trip. A run is not done while this is non-zero.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl StateManager for AwaitingExtraction {
    fn create(states: &States) -> Result<Self, StateError> {
        inputs(states)?;
        Ok(Self::new())
    }
}

/// Order totes lying in the order buffer, each with the time it may leave.
///
/// A buffered tote is `InRack` in buffer racking, so nothing is holding it and
/// nothing is holding a robot for it; this map is only the memory of WHICH
/// order is on it and WHEN it is due, which the rack cannot carry.
///
/// The tote is unreachable until the store chain's `VtmReturn` actually files
/// it, which is fine and needs no extra state: the release search asks for that
/// exact tote in the buffer, and `source_retrieve` suspends until it arrives.
#[derive(Clone, Default)]
struct BufferedOrders {
    // ordered: released in due order, so `first_key_value` is the next
    // candidate and a not-yet-due head means nothing behind it is due either.
    inner: Arc<Mutex<BTreeMap<(SimTime, ToteId), ClosedOrder>>>,
}

impl BufferedOrders {
    fn park(&self, tote_id: ToteId, release_at: SimTime, order: ClosedOrder) {
        self.inner.lock().unwrap().insert((release_at, tote_id), order);
    }

    /// When the order due soonest comes due, if anything is resting at all.
    fn next_due(&self) -> Option<SimTime> {
        self.inner.lock().unwrap().first_key_value().map(|(&(release_at, _), _)| release_at)
    }

    /// Take the order due soonest, if it is due by `now`.
    fn take_due(&self, now: SimTime) -> Option<(ToteId, ClosedOrder)> {
        let mut inner = self.inner.lock().unwrap();
        let (&(release_at, tote_id), _) = inner.first_key_value()?;
        if release_at > now {
            return None;
        }
        let order = inner.remove(&(release_at, tote_id))?;
        Some((tote_id, order))
    }
}

/// Whether close-out can run at all on this map: at least one extraction
/// station reachable from every close-out origin. `close_out_enabled` gates both
/// the actor and picking's per-order close-out intent on this, so a map that
/// cannot drain closed orders never queues them.
pub fn close_out_can_run(map: &WarehouseMap, stations: &StationSet, relay: &RelayService) -> bool {
    let reachable = reachable_from_close_out_origins(map, stations, relay);
    stations.extraction_stations().iter().any(|s| reachable.contains(&s.presentation_cell))
}

pub(crate) fn close_out_enabled(input: &RunInputs, states: &States) -> Result<bool, StateError> {
    if !input.definition.tasks.iter().any(|task| task.name() == "OrderCloseOut") || !input.config.order.close_out {
        return Ok(false);
    }
    let navigation = states.get::<Navigation>()?;
    let stations = states.get::<StationSet>()?;
    let relay = states.get::<RelayService>()?;
    Ok(close_out_can_run(&navigation.map, &stations, &relay))
}

/// Cells reachable from every cell a close-out leg can start at: the pick
/// stations that load order totes, and the buffer relays that release them.
///
/// The INTERSECTION, not the union. One extraction-station list serves both
/// legs, so a station only the pick side can reach would strand every release
/// mid-run — which is the failure this filter exists to name up front rather
/// than discover as a `no path exists` hours in.
fn reachable_from_close_out_origins(
    map: &WarehouseMap,
    stations: &StationSet,
    relay: &RelayService,
) -> FxHashSet<CellId> {
    let mut reachable = reachable_from(map, stations.pick_stations().iter().map(|s| s.presentation_cell));
    let buffer_origins: Vec<CellId> = relay.buffer_relay_htm_cells().collect();
    // A map with no order buffer states no second origin, so the pick side's
    // reach stands unrestricted.
    if !buffer_origins.is_empty() {
        let from_buffer = reachable_from(map, buffer_origins.into_iter());
        reachable.retain(|cell| from_buffer.contains(cell));
    }
    reachable
}

/// Forward closure of the travel graph from `origins`.
fn reachable_from(map: &WarehouseMap, origins: impl Iterator<Item = CellId>) -> FxHashSet<CellId> {
    let mut seen = FxHashSet::default();
    let mut queue: VecDeque<CellId> = origins
        .inspect(|c| {
            seen.insert(*c);
        })
        .collect();
    while let Some(cell) = queue.pop_front() {
        for (next, _) in map.graph.neighbors(cell) {
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    seen
}

#[derive(Display)]
#[display("OrderCloseOut")]
#[task]
pub struct OrderCloseOut {
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    relay: RelayService,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    order_presentation: FxHashMap<StationId, CellId>,

    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
    completion: OrderCompletionTracker,
    awaiting: AwaitingExtraction,
    buffered: BufferedOrders,
    /// Extraction stations, with a per-station inflight counter each. Not
    /// `pick_stations()`, so none of the pick side's counters exist for them.
    extraction: Vec<(Station, Arc<Mutex<TemporalCounter>>)>,
    dwell: SimDuration,
    /// Orders the source says leave via the order buffer. Empty when the
    /// scenario states no exit path (every generated scenario but Walmart's)
    /// or when the map has no buffer racking, and then close-out behaves
    /// exactly as it did before buffers existed.
    // ordered: none needed; membership only.
    buffer_bound: FxHashSet<OrderId>,
    /// When each order is due out, from the source's own cutoff. Empty for a
    /// scenario whose orders state no deadline, which is every generated one.
    due_at: FxHashMap<OrderId, SimTime>,
    /// FALLBACK for an order with no stated deadline: how long it rests in the
    /// buffer before it is due out.
    ///
    /// A global constant cannot express a wave — the real extract has 39
    /// distinct cutoffs across 24,789 lines, so orders arrive continuously and
    /// leave in batches, which is the occupancy behaviour a buffer exists for.
    /// Where the source states a cutoff, `due_at` carries it and this is
    /// unused. Zero means "released as soon as a chain can be built", which
    /// models the buffer's TRAFFIC honestly while claiming nothing about its
    /// OCCUPANCY.
    buffer_dwell: SimDuration,
}

impl OrderCloseOut {
    /// Whether close-out can run at all on this map.
    pub fn has_extraction(&self) -> bool {
        !self.extraction.is_empty()
    }

    fn station_cell(&self, station_id: StationId) -> Result<CellId, PlanError> {
        self.station_service.presentation_cell(station_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "OrderCloseOut",
            detail: format!("station {station_id:?} has no presentation cell"),
        })
    }

    /// The extraction station with the least work in flight at `now`.
    fn least_loaded_extraction(&self, now: SimTime) -> Option<&(Station, Arc<Mutex<TemporalCounter>>)> {
        self.extraction
            .iter()
            .min_by_key(|(station, counter)| (counter.lock().unwrap().max_count_from(now), station.id.0))
    }

    /// Whether this closed order's tote goes to the buffer rather than
    /// straight to extraction.
    fn is_buffer_bound(&self, order: &ClosedOrder) -> bool {
        self.buffer_bound.contains(&order.order_id)
    }

    /// First leg: every freshly closed order gets a `CloseOutLoad` chain head,
    /// which suspends in `source_retrieve` until an empty ORDER tote can be
    /// fetched to the station that picked it.
    fn dispatch_loads(&self, ctx: &SchedCtx) -> Result<bool, PlanError> {
        let mut dispatched = false;
        while let Some(order) = self.completion.take_closed() {
            let station_cell = self.station_cell(order.station_id)?;
            ctx.add_task(CloseOutLoad {
                order,
                station_cell,
                awaiting: self.awaiting.clone(),
                dwell: self.dwell,
                aim_version: Version::new(),
                navigation: self.navigation.clone(),
                bots: self.bots.clone(),
                inventory: self.inventory.clone(),
                available: self.available.clone(),
                parking: self.parking.clone(),
                events: self.events.clone(),
                relay: self.relay.clone(),
                station_service: self.station_service.clone(),
                station_arrivals: self.station_arrivals.clone(),
                order_presentation: self.order_presentation.clone(),

                tripwires: self.tripwires,
                delivery_max_wait_override: self.delivery_max_wait_override,
            });
            dispatched = true;
        }
        Ok(dispatched)
    }

    /// Second leg: an idle HTM holding a loaded order tote carries it to an
    /// extraction station and it is emptied.
    fn extract_one(&self, ctx: &SchedCtx) -> Result<bool, PlanError> {
        let candidates = self.available.peek_matching(|htm| {
            htm.tote.as_ref().is_some_and(|t| {
                // A buffer-bound tote is the store step's to take. Peeking it
                // here and putting it back would race the two for the same HTM.
                self.awaiting.peek(t.tote_id).is_some_and(|order| !self.is_buffer_bound(&order))
            })
        });
        let Some(htm) = candidates.into_iter().min_by_key(|h| (h.available_at(), h.robot())) else {
            return Ok(false);
        };
        let tote_id = htm.tote.as_ref().expect("peeked with a tote").tote_id;
        let Some((station, counter)) = self.least_loaded_extraction(htm.available_at()) else {
            return Ok(false);
        };
        let station_id = station.id;
        let counter = counter.clone();
        let Some(order) = self.awaiting.take(tote_id) else { return Ok(false) };
        let Some(entry) = self.available.take(htm.robot()) else {
            self.awaiting.mark(tote_id, order);
            return Ok(false);
        };

        let inflight = InflightSpan::open(counter, entry.action_ready_at, Version::new());
        let handoff = HtmCarryingTote {
            tote_id,
            zone_id: ZoneId(0),
            cell_reservation: entry.cell_reservation,
            action_ready_at: entry.action_ready_at,
            orientation: entry.orientation,
            station_arrival_plan: None,
            presentation_id: self.events.next_presentation_id(),
            source_relay_id: None,
            deliver_from: None,
        };
        ctx.add_task(CloseOutExtract {
            order,
            station_id,
            handoff: Some(handoff),
            inflight: Some(inflight),
            dwell: self.dwell,
            navigation: self.navigation.clone(),
            bots: self.bots.clone(),
            inventory: self.inventory.clone(),
            available: self.available.clone(),
            parking: self.parking.clone(),
            events: self.events.clone(),
            relay: self.relay.clone(),
            station_service: self.station_service.clone(),
            station_arrivals: self.station_arrivals.clone(),
            order_presentation: self.order_presentation.clone(),

            tripwires: self.tripwires,
            delivery_max_wait_override: self.delivery_max_wait_override,
        });
        Ok(true)
    }

    /// Buffer-bound second leg: an idle HTM holding a loaded order tote carries
    /// it to a BUFFER relay instead of an extraction station, and a gripper VTM
    /// files it into buffer racking — the same `HtmStore → VtmReturn` chain
    /// a presentation's clearance builds, aimed at the buffer by asking the
    /// relay service for buffer racking.
    async fn store_one(&self, ctx: &SchedCtx) -> Result<bool, PlanError> {
        let candidates = self.available.peek_matching(|htm| {
            htm.tote.as_ref().is_some_and(|t| self.awaiting.peek(t.tote_id).is_some_and(|o| self.is_buffer_bound(&o)))
        });
        let Some(htm) = candidates.into_iter().min_by_key(|h| (h.available_at(), h.robot())) else {
            return Ok(false);
        };
        let robot_id = htm.robot();
        let from_cell = htm.cell();
        let action_ready = htm.action_ready_at;

        let deposit = match self
            .relay
            .reserve_buffer_relay_for_deposit(
                from_cell,
                htm.orientation,
                action_ready,
                robot_id,
                // No parking relief: a blocked buffer relay is a wait, not a
                // deadlock — every other buffer relay is still scored.
                None,
            )
            .await
        {
            Ok(d) => d,
            // A full buffer, no headroom in any buffer zone, relays opening
            // beyond the access horizon, or access cells held: all are
            // legitimate backpressure. The tote waits on the HTM (guarded
            // by `AwaitingExtraction`) and the step is retried on the next
            // relay or fleet signal.
            Err(
                ReservationError::NoCandidates { .. }
                | ReservationError::BeyondAccessHorizon { .. }
                | ReservationError::AccessHeld { .. }
                | ReservationError::HeadroomExhausted { .. },
            ) => {
                return Ok(false);
            }
            Err(e) => {
                return Err(PlanError::TaskGraphInvariant {
                    context: "OrderCloseOut",
                    detail: format!("buffer deposit failed: {e}"),
                });
            }
        };
        let relay_start = deposit.start;

        let tote_id = htm.tote.as_ref().expect("peeked with a tote").tote_id;
        let Some(order) = self.awaiting.take(tote_id) else {
            deposit.cancel();
            return Ok(false);
        };
        let Some(entry) = self.available.take(robot_id) else {
            // Put the claim back before leaving, or the order is stranded on a
            // tote nothing is looking for.
            self.awaiting.mark(tote_id, order);
            deposit.cancel();
            return Ok(false);
        };
        let presentation_id = entry.tote.as_ref().and_then(|t| t.presentation_id);

        ctx.add_task(ReturnTote {
            station_id: order.station_id,
            robot_id,
            tote_id,
            predecessor: entry.cell_reservation,
            action_ready: entry.action_ready_at,
            departs_by: None,
            orientation: entry.orientation,
            deposit: Some(deposit),
            presentation_id,
            // ATTRIBUTION CAVEAT: the buffer leg is neither a pick nor a
            // putaway return; the idle-gap split has no third bucket for it.
            tote_source: ToteSource::Pick,
            navigation: self.navigation.clone(),
            relay: self.relay.clone(),
            bots: self.bots.clone(),
            inventory: self.inventory.clone(),
            available: self.available.clone(),
            parking: self.parking.clone(),
            events: self.events.clone(),
        });

        // Due at the ORDER'S OWN cutoff when the source states one, and only
        // otherwise at a global dwell from now. A cutoff is a property of the
        // order; the dwell is a property of nothing, and could not express a
        // wave.
        //
        // The fallback clock starts at the relay reservation, the earliest sim
        // time the tote is committed to the buffer.
        let due = self.due_at.get(&order.order_id).copied().unwrap_or(relay_start + self.buffer_dwell);
        self.buffered.park(tote_id, due, order);
        Ok(true)
    }

    /// Buffer release: each due order gets a `CloseOutRelease` chain head,
    /// which suspends in `source_retrieve` until its own tote can be fetched
    /// out of the buffer — including while it is still riding the store
    /// chain's robot, when the search simply finds nothing yet.
    fn dispatch_due_releases(&self, ctx: &SchedCtx) -> Result<bool, PlanError> {
        let now = ctx.barrier().clock().value();
        let mut dispatched = false;
        while let Some((tote_id, order)) = self.buffered.take_due(now) {
            let Some((station, counter)) = self.least_loaded_extraction(now) else {
                return Err(PlanError::TaskGraphInvariant {
                    context: "OrderCloseOut",
                    detail: "buffered order due but no extraction station stands".into(),
                });
            };
            ctx.add_task(CloseOutRelease {
                tote_id,
                order,
                station_id: station.id,
                station_cell: self.station_cell(station.id)?,
                counter: counter.clone(),
                dwell: self.dwell,
                aim_version: Version::new(),
                navigation: self.navigation.clone(),
                bots: self.bots.clone(),
                inventory: self.inventory.clone(),
                available: self.available.clone(),
                parking: self.parking.clone(),
                events: self.events.clone(),
                relay: self.relay.clone(),
                station_service: self.station_service.clone(),
                station_arrivals: self.station_arrivals.clone(),
                order_presentation: self.order_presentation.clone(),

                tripwires: self.tripwires,
                delivery_max_wait_override: self.delivery_max_wait_override,
            });
            dispatched = true;
        }
        Ok(dispatched)
    }
}

#[async_trait(?Send)]
impl Task for OrderCloseOut {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let params: CloseOutTaskConfig = ctx.config()?;
        let input = inputs(ctx.states())?;
        let world = world_inputs(ctx.states())?;
        let stations = ctx.state::<StationSet>()?;
        let relay = ctx.state::<RelayService>()?;
        let parking = ctx.state::<ParkingRegistry>()?;
        let orders = &input.orders;
        // Only stations an HTM can actually reach from every close-out origin.
        // A map may declare one that no path serves (DFW6 ships three and one
        // of them is stranded); routing a tote there is a hard
        // TaskGraphInvariant inside HtmDeliver, so it is filtered here and
        // named, not discovered mid-run.
        let reachable = reachable_from_close_out_origins(&world.data.map, &stations, &relay);
        let mut extraction = Vec::new();
        for station in stations.extraction_stations() {
            if reachable.contains(&station.presentation_cell) {
                extraction.push((station.clone(), Arc::new(Mutex::new(TemporalCounter::new()))));
            } else {
                eprintln!(
                    "[close-out] extraction station {:?} is unreachable from some close-out origin \
                     (cell {:?}); close-out will not route to it",
                    station.id, station.presentation_cell
                );
            }
        }
        // A buffer-bound order needs a buffer to go to. Without one the exit
        // path is unroutable, so it is dropped here rather than waiting
        // forever in the store step.
        let buffer_bound: FxHashSet<OrderId> = if relay.has_order_buffer() {
            orders.iter().filter(|o| o.exit_path == ExitPath::Buffer).map(|o| o.id).collect()
        } else {
            FxHashSet::default()
        };
        let due_at: FxHashMap<OrderId, SimTime> = orders.iter().filter_map(|o| Some((o.id, o.deadline?))).collect();
        if !close_out_enabled(&input, ctx.states())? {
            extraction.clear();
        }
        Ok(Self {
            navigation: ctx.state::<Navigation>()?,
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            relay: relay.as_ref().clone(),
            station_service: ctx.state::<StationService>()?,
            station_arrivals: ctx.state::<StationArrivalDiagnostics>()?.as_ref().clone(),
            order_presentation: world
                .data
                .station_data
                .iter()
                .filter_map(|station| Some((station.station_id, station.order_presentation_cell?)))
                .collect(),

            tripwires: world.config.tripwires,
            delivery_max_wait_override: world
                .config
                .pick
                .delivery_max_wait_secs
                .map(|secs| SimDuration::ms((secs * 1000.0) as u64)),
            completion: ctx.state::<OrderCompletionTracker>()?.as_ref().clone(),
            awaiting: ctx.state::<AwaitingExtraction>()?.as_ref().clone(),
            buffered: BufferedOrders::default(),
            extraction,
            dwell: policy::duration("operator.time_per_pick_secs", world.config.operator.time_per_pick_secs)?,
            buffer_bound,
            due_at,
            buffer_dwell: policy::duration(
                "OrderCloseOut.buffer_dwell_secs",
                params.buffer_dwell_secs.unwrap_or(input.config.order.buffer_dwell_secs),
            )?,
        })
    }

    fn holds_completion() -> bool {
        false
    }

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        parse_config::<CloseOutTaskConfig>(config).map(|_| ())
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        // A map with no reachable extraction station runs no close-out. The
        // composition also disables `retain_closed` in that case, so nothing
        // ever queues; exiting is correct, not a stall.
        if self.extraction.is_empty() {
            return Ok(());
        }
        loop {
            let completion_seen = self.completion.version().seen();
            let fleet_seen = self.available.version().seen();
            let relay_changed = notified(self.relay.signal());

            // Oldest work first: a due buffered order predates everything, a
            // loaded tote holds an HTM idle at its station, a fresh close only
            // holds a queue slot.
            let mut progress = self.dispatch_due_releases(&ctx)?;
            progress |= self.store_one(&ctx).await?;
            progress |= self.extract_one(&ctx)?;
            progress |= self.dispatch_loads(&ctx)?;
            if progress {
                continue;
            }

            let next_due = self.buffered.next_due();
            select! {
                biased;
                _ = self.completion.version().past(completion_seen) => {}
                _ = self.available.version().past(fleet_seen) => {}
                _ = relay_changed => {}
                _ = wait_due(&ctx, next_due) => {}
            }
        }
    }
}

/// Suspend until the barrier reaches the next buffered due time, or forever
/// when nothing is resting in the buffer.
async fn wait_due(ctx: &SchedCtx, due: Option<SimTime>) {
    match due {
        Some(t) => {
            let _ = ctx.barrier().wait_until(t).await;
        }
        None => std::future::pending().await,
    }
}

/// Chain head for the load leg: fetch an empty ORDER tote from rack and load a
/// closed order into it at the station that picked it. The retrieve proposal
/// suspends until such a tote is actually reachable.
#[derive(Display)]
#[display("CloseOutLoad(order={:?} stn={:?})", order.order_id, order.station_id)]
struct CloseOutLoad {
    order: ClosedOrder,
    station_cell: CellId,
    awaiting: AwaitingExtraction,
    dwell: SimDuration,
    /// `source_retrieve` wants an aim signal; close-out paces nothing, so
    /// this version never bumps.
    aim_version: Version,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    relay: RelayService,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    order_presentation: FxHashMap<StationId, CellId>,

    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
}

#[async_trait(?Send)]
impl Task for CloseOutLoad {
    type Proposal = ProposedRetrieve;
    type Schedule = ();

    async fn propose(&self, owner: ProposalId) -> Result<ProposedRetrieve, PlanError> {
        Ok(self
            .relay
            .source_retrieve(
                // An order rides an ORDER tote. Taking a stock tote here is
                // the starvation the permanent designation exists to prevent.
                ToteSearchCriteria::Empty { designation: ToteDesignation::Order },
                self.station_cell,
                self.order.completed_at,
                SimTime::ZERO,
                || None,
                &self.aim_version,
                owner,
            )
            .await?)
    }

    async fn schedule(&mut self, proposal: ProposedRetrieve, ctx: SchedCtx) -> Result<(), PlanError> {
        let quoted_arrival = proposal.selection.completion_time;
        let (reservation, rack_slot) = proposal.into_accepted()?;
        // ATTRIBUTION CAVEAT: `VtmForward` reads only the plan's source, and a
        // close-out fetch has no source of its own, so it counts in the pick
        // idle-gap split.
        let plan = StationArrivalPlan::new(
            self.order.station_id,
            StationArrivalSource::PickFromRack,
            quoted_arrival,
            quoted_arrival,
        );
        // The load happens at a pick station, whose inflight counter belongs
        // to PickDemand. A local counter keeps this chain's span honest
        // without borrowing that one.
        let inflight =
            InflightSpan::open(Arc::new(Mutex::new(TemporalCounter::new())), reservation.start, Version::new());

        let handoff = ctx
            .add_task(VtmForward::new(
                rack_slot,
                reservation,
                Some(plan),
                self.relay.clone(),
                self.inventory.clone(),
                self.events.clone(),
            ))
            .await;
        let context = ctx.task_context::<HtmPickup>(json!({}))?;
        let pickup = HtmPickup::new(&context, handoff)?;
        context.finish()?;
        let carrying = ctx.add_task(pickup).await;
        let carrying = ctx
            .add_task(
                HtmDeliver::new(
                    self.order.station_id,
                    inflight,
                    carrying,
                    ctx.barrier().clock(),
                    self.navigation.clone(),
                    self.bots.clone(),
                    self.inventory.clone(),
                    self.parking.clone(),
                    self.events.clone(),
                    self.station_service.clone(),
                    self.station_arrivals.clone(),
                    self.order_presentation.get(&self.order.station_id).copied(),
                    self.tripwires,
                    self.delivery_max_wait_override,
                )
                .presenting_order(),
            )
            .await?;

        // Marked BEFORE the presentation registers the HTM available, so the
        // close-out step that wakes on that registration finds its order.
        self.awaiting.mark(carrying.tote_id, self.order);
        ctx.add_task(Presentation {
            station_id: self.order.station_id,
            op: PresentationOp::OrderLoad { order_id: self.order.order_id, qty: self.order.units },
            dwell: self.dwell,
            handoff: Some(carrying),
            order_completion: None,
            bin_hold: None,
            inventory: self.inventory.clone(),
            bots: self.bots.clone(),
            available: self.available.clone(),
            events: self.events.clone(),
        })
        .await;
        Ok(())
    }
}

/// Chain head for the extract leg: the HTM is already holding the loaded tote,
/// so the chain is just the drive to the extraction station and the emptying.
#[derive(Display)]
#[display("CloseOutExtract(order={:?} stn={:?})", order.order_id, station_id)]
struct CloseOutExtract {
    order: ClosedOrder,
    station_id: StationId,
    handoff: Option<HtmCarryingTote>,
    inflight: Option<InflightSpan>,
    dwell: SimDuration,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    relay: RelayService,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    order_presentation: FxHashMap<StationId, CellId>,

    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
}

#[async_trait(?Send)]
impl Task for CloseOutExtract {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let missing = |what: &str| PlanError::TaskGraphInvariant {
            context: "CloseOutExtract",
            detail: format!("scheduled without its {what}"),
        };
        let handoff = self.handoff.take().ok_or_else(|| missing("handoff"))?;
        let inflight = self.inflight.take().ok_or_else(|| missing("inflight span"))?;
        let carrying = ctx
            .add_task(HtmDeliver::new(
                self.station_id,
                inflight,
                handoff,
                ctx.barrier().clock(),
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                self.parking.clone(),
                self.events.clone(),
                self.station_service.clone(),
                self.station_arrivals.clone(),
                self.order_presentation.get(&self.station_id).copied(),
                self.tripwires,
                self.delivery_max_wait_override,
            ))
            .await?;
        let available = ctx
            .add_task(Presentation {
                station_id: self.station_id,
                op: PresentationOp::OrderExtract { order_id: self.order.order_id, qty: self.order.units },
                dwell: self.dwell,
                handoff: Some(carrying),
                order_completion: None,
                bin_hold: None,
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                available: self.available.clone(),
                events: self.events.clone(),
            })
            .await;
        ctx.add_task(ClearPresentation::new(
            available,
            self.station_id,
            ToteSource::Pick,
            self.navigation.clone(),
            self.available.clone(),
            ctx.barrier().clock(),
        ));
        Ok(())
    }
}

/// Chain head for the buffer release: fetch the due order's own tote back out
/// of the buffer and take it to extraction — the same shape as the load leg,
/// with the extraction station as the destination and the buffer as the
/// source. The one search in the model that names its tote: this order's, not
/// any spare.
#[derive(Display)]
#[display("CloseOutRelease(order={:?} tote={:?})", order.order_id, tote_id)]
struct CloseOutRelease {
    tote_id: ToteId,
    order: ClosedOrder,
    station_id: StationId,
    station_cell: CellId,
    counter: Arc<Mutex<TemporalCounter>>,
    dwell: SimDuration,
    /// Never bumps; see `CloseOutLoad::aim_version`.
    aim_version: Version,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    relay: RelayService,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    order_presentation: FxHashMap<StationId, CellId>,

    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
}

#[async_trait(?Send)]
impl Task for CloseOutRelease {
    type Proposal = ProposedRetrieve;
    type Schedule = ();

    async fn propose(&self, owner: ProposalId) -> Result<ProposedRetrieve, PlanError> {
        Ok(self
            .relay
            .source_retrieve(
                ToteSearchCriteria::BufferedOrder { tote_id: self.tote_id },
                self.station_cell,
                self.order.completed_at,
                SimTime::ZERO,
                || None,
                &self.aim_version,
                owner,
            )
            .await?)
    }

    async fn schedule(&mut self, proposal: ProposedRetrieve, ctx: SchedCtx) -> Result<(), PlanError> {
        let quoted_arrival = proposal.selection.completion_time;
        let (reservation, rack_slot) = proposal.into_accepted()?;
        // Same attribution caveat as the load leg.
        let plan = StationArrivalPlan::new(
            self.station_id,
            StationArrivalSource::PickFromRack,
            quoted_arrival,
            quoted_arrival,
        );
        let inflight = InflightSpan::open(self.counter.clone(), reservation.start, Version::new());

        let handoff = ctx
            .add_task(VtmForward::new(
                rack_slot,
                reservation,
                Some(plan),
                self.relay.clone(),
                self.inventory.clone(),
                self.events.clone(),
            ))
            .await;
        let context = ctx.task_context::<HtmPickup>(json!({}))?;
        let pickup = HtmPickup::new(&context, handoff)?;
        context.finish()?;
        let carrying = ctx.add_task(pickup).await;
        let carrying = ctx
            .add_task(HtmDeliver::new(
                self.station_id,
                inflight,
                carrying,
                ctx.barrier().clock(),
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                self.parking.clone(),
                self.events.clone(),
                self.station_service.clone(),
                self.station_arrivals.clone(),
                self.order_presentation.get(&self.station_id).copied(),
                self.tripwires,
                self.delivery_max_wait_override,
            ))
            .await?;
        let available = ctx
            .add_task(Presentation {
                station_id: self.station_id,
                op: PresentationOp::OrderExtract { order_id: self.order.order_id, qty: self.order.units },
                dwell: self.dwell,
                handoff: Some(carrying),
                order_completion: None,
                bin_hold: None,
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                available: self.available.clone(),
                events: self.events.clone(),
            })
            .await;
        ctx.add_task(ClearPresentation::new(
            available,
            self.station_id,
            ToteSource::Pick,
            self.navigation.clone(),
            self.available.clone(),
            ctx.barrier().clock(),
        ));
        Ok(())
    }
}
