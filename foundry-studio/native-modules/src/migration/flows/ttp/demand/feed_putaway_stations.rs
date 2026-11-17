//! Keeping the putaway stations stocked with staged empty totes, whatever
//! they are inducting: `PutawaySupplyDemand` decides which stations are
//! short, one `StageEmpty` per opening (`stage_empty`) decides where that
//! station's next empty comes from, and `PutawaySupply` is the engine both
//! read. Once the last lot's fill is scheduled, a `StageEmpty` hands the
//! empty it staged, which nothing will fill, to a clearance once a deposit
//! relay is reserved for its tote: the drain.

use crate::composition::resources::inputs;
use crate::events::EventJournal;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::station_service::StationService;
use crate::task;
use crate::task::TaskContext;

use crate::composition::SupplyTaskConfig;
use crate::composition::policy;
use crate::task::parse_config;
use serde_json::Value;
use std::any::type_name;
use std::cmp::Reverse;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use desgraph::{CellId, StationId};
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::select;

use crate::error::ReservationError;
use crate::pathfinding::{Orientation, PathCacheKey, PathfinderError};
use crate::scenario_gen::{DemandPool, SkuInfo};
use crate::state::{StateError, StateManager, States};
use crate::utils::temporal_counter::TemporalCounter;
use crate::{flows::ttp::inflight::*, model::htm::registry::AvailableHtm, model::relay::*, prelude::*};

mod stage_empty;

use stage_empty::StageEmpty;

type ArrivalItem = (SkuId, u32, SimTime);

#[derive(Clone, Copy)]
struct ArrivalWork {
    order_id: OrderId,
    sequence: usize,
    sku: SkuId,
    qty: u32,
}

pub type ArrivalQueue = Mut<VecDeque<ArrivalItem>>;

pub(crate) struct Arrivals(pub ArrivalQueue);

impl StateManager for Arrivals {
    fn create(states: &States) -> Result<Self, StateError> {
        inputs(states)?;
        Ok(Self(ArrivalQueue::new(Default::default())))
    }
}

pub fn build_arrival_queues(
    skus: &[SkuInfo],
    orders: &OrderService,
    pool: Option<&DemandPool>,
    units_per_tote: u32,
) -> ArrivalQueue {
    assert!(units_per_tote > 0);
    let mut queue: VecDeque<ArrivalItem> = VecDeque::new();
    let mut append = |sku, mut quantity: u32, net_time| {
        while quantity > 0 {
            let batch = quantity.min(units_per_tote);
            queue.push_back((sku, batch, net_time));
            quantity -= batch;
        }
    };
    match pool {
        Some(pool) => {
            for line in pool.lines() {
                append(line.sku, line.qty, line.putaway_net_time);
            }
        }
        None => {
            let mut remaining: BTreeMap<SkuId, u32> = skus.iter().map(|sku| (sku.id, sku.inventory_units)).collect();
            for work in global_arrival_work(orders) {
                let Some(cap) = remaining.get_mut(&work.sku) else { continue };
                let quantity = work.qty.max(units_per_tote).min(*cap);
                append(work.sku, quantity, SimTime::ZERO);
                *cap -= quantity;
            }
            for sku in skus {
                append(sku.id, remaining.get(&sku.id).copied().unwrap_or(0), SimTime::ZERO);
            }
        }
    }
    Mut::new(queue)
}

fn global_arrival_work(orders: &OrderService) -> Vec<ArrivalWork> {
    let raw: Vec<(OrderId, SkuId, u32)> = orders.with(|orders| {
        orders
            .values()
            .flat_map(|order| order.lines.iter().filter(|l| l.quantity > 0).map(|l| (order.id, l.sku, l.quantity)))
            .collect()
    });
    let mut sku_demand: FxHashMap<SkuId, u32> = FxHashMap::default();
    for &(_, sku, qty) in &raw {
        *sku_demand.entry(sku).or_insert(0) += qty;
    }
    let mut work: Vec<ArrivalWork> = raw
        .into_iter()
        .enumerate()
        .map(|(sequence, (order_id, sku, qty))| ArrivalWork { order_id, sequence, sku, qty })
        .collect();
    work.sort_by_key(|w| (Reverse(sku_demand.get(&w.sku).copied().unwrap_or(0)), w.order_id, w.sequence));
    work
}

// Splitting on the sign keeps the handicap exact over the whole u64 miss
// range; `as i64` wraps a saturated miss negative, scoring it best.
fn apply_direct_bias(arrival_miss: SimDuration, direct_bias: SimOffset) -> u64 {
    let handicap = direct_bias.magnitude().as_ms();
    if direct_bias.is_negative() {
        arrival_miss.as_ms().saturating_add(handicap)
    } else {
        arrival_miss.as_ms().saturating_sub(handicap)
    }
}

fn direct_wins(direct: SimTime, rack: SimTime, target: SimTime, bias: SimOffset) -> bool {
    let key = |arrival: SimTime, miss: u64| (miss, arrival < target, arrival);
    key(direct, apply_direct_bias(direct.abs_diff(target), bias)) <= key(rack, rack.abs_diff(target).as_ms())
}

struct SupplyStation {
    id: StationId,
    cell: CellId,
    f_max: SimDuration,
    inflight: Arc<Mutex<TemporalCounter>>,
    pending: Arc<AtomicU32>,
}

#[derive(Clone, Copy)]
pub(crate) struct StagingOpening {
    station_id: StationId,
    station_cell: CellId,
    after: SimTime,
    search_after: SimTime,
}

pub(crate) struct StagingTicket {
    pending: Arc<AtomicU32>,
    inflight: Arc<Mutex<TemporalCounter>>,
    version: Version,
    released: bool,
}

impl StagingTicket {
    pub(crate) fn open_span(mut self, start: SimTime) -> InflightSpan {
        let span = InflightSpan::open(self.inflight.clone(), start, self.version.clone());
        self.release();
        span
    }

    fn release(&mut self) {
        if !std::mem::replace(&mut self.released, true) {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            self.version.bump();
        }
    }
}

impl Drop for StagingTicket {
    fn drop(&mut self) {
        self.release();
    }
}

pub struct PutawaySupply {
    // ordered: the station sweep must be deterministic
    stations: BTreeMap<StationId, SupplyStation>,
    /// Every putaway station's presentation cell.
    putaway_presentation_cells: FxHashSet<CellId>,
    /// Every station's presentation cell.
    presentation_cells: FxHashSet<CellId>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    rack_forbidden_from: Option<SimTime>,
    version: Version,
}

impl PutawaySupply {
    /// Supply for the set's putaway stations, which learns every station's
    /// presentation cell from it.
    pub fn new(
        stations: &StationSet,
        bounds: &StationFetchBounds,
        navigation: Arc<Navigation>,
        relay: RelayService,
        inventory: InventoryService,
        available: AvailableHtmRegistry,
        rack_forbidden_from: Option<SimTime>,
    ) -> Self {
        let putaway = stations.putaway_stations();
        Self {
            stations: putaway
                .iter()
                .map(|station| {
                    let f_max = bounds.f_max(station.id);
                    (
                        station.id,
                        SupplyStation {
                            id: station.id,
                            cell: station.presentation_cell,
                            f_max,
                            inflight: Arc::new(Mutex::new(TemporalCounter::new())),
                            pending: Arc::new(AtomicU32::new(0)),
                        },
                    )
                })
                .collect(),
            putaway_presentation_cells: putaway.iter().map(|station| station.presentation_cell).collect(),
            presentation_cells: stations.presentation_cells().collect(),
            navigation,
            relay,
            inventory,
            available,
            rack_forbidden_from,
            version: Version::new(),
        }
    }

    /// Whether a staging decided at `now` may still source from the rack.
    pub(crate) fn rack_allowed_at(&self, now: SimTime) -> bool {
        self.rack_forbidden_from.is_none_or(|from| now < from)
    }

    pub fn station_cells(&self) -> impl Iterator<Item = (StationId, CellId)> + '_ {
        self.stations.values().map(|station| (station.id, station.cell))
    }

    pub(crate) fn version(&self) -> &Version {
        &self.version
    }

    // Putaway supply is an inventory tote. An order tote holds no modelled
    // inventory, so it reads as empty even while it carries a closed order
    // to extraction.
    fn live_empty(&self, htm: &AvailableHtm) -> bool {
        htm.tote.as_ref().is_some_and(|carried| {
            self.inventory.tote(carried.tote_id).is_ok_and(|tote| {
                tote.designation() == ToteDesignation::Inventory && tote.is_empty_at(htm.action_ready_at)
            })
        })
    }

    // Counting all three keeps one gap from being fed twice: empties presenting
    // at the cell, chains in flight, and tickets StageEmpty tasks still hold.
    fn staged_depth(&self, station: &SupplyStation) -> isize {
        let presenting = self.available.peek_matching(|htm| htm.cell() == station.cell && self.live_empty(htm)).len();
        presenting as isize
            + station.inflight.lock().unwrap().frontier().1
            + station.pending.load(Ordering::Relaxed) as isize
    }

    pub(crate) fn openings(&self, admission: AdmissionLaw) -> Vec<StagingOpening> {
        self.stations
            .values()
            .filter(|station| self.staged_depth(station) < admission.cap(station.f_max) as isize)
            .filter_map(|station| {
                let after = station
                    .inflight
                    .lock()
                    .unwrap()
                    .earliest_new_inflight_start(admission.cap(station.f_max) as isize, SimTime::ZERO)?;
                Some(StagingOpening {
                    station_id: station.id,
                    station_cell: station.cell,
                    after,
                    search_after: after.saturating_sub(station.f_max),
                })
            })
            .collect()
    }

    pub(crate) fn begin_staging(&self, station_id: StationId) -> Result<StagingTicket, PlanError> {
        let station = self.stations.get(&station_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "PutawaySupply",
            detail: format!("staging ticket asked for unknown putaway station {station_id:?}"),
        })?;
        station.pending.fetch_add(1, Ordering::Relaxed);
        self.version.bump();
        Ok(StagingTicket {
            pending: station.pending.clone(),
            inflight: station.inflight.clone(),
            version: self.version.clone(),
            released: false,
        })
    }

    fn travel_to(
        &self,
        robot: RobotId,
        from: CellId,
        orientation: Orientation,
        to: CellId,
    ) -> Result<Option<SimDuration>, PlanError> {
        let cache = self.navigation.cache(robot)?;
        if from == to {
            return Ok(Some(SimDuration::ZERO));
        }
        let key = PathCacheKey { from, from_orientation: orientation, to, carrying_cargo: false };
        match cache.travel_time(key) {
            Ok(travel) => Ok(Some(travel)),
            Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The live empty that would reach the station soonest. An empty standing
    /// idle since before the opening departs at the opening (simtime), not
    /// at its own availability; but one released on a station's presentation
    /// cell before the opening is passed over, since it may leave that cell
    /// only at its release.
    pub(crate) fn best_direct(
        &self,
        station_cell: CellId,
        after: SimTime,
    ) -> Result<Option<DirectCandidate>, PlanError> {
        let mut best: Option<DirectCandidate> = None;
        for entry in self.available.peek_matching(|htm| self.live_empty(htm)) {
            // An empty presenting at ANY putaway station is staged supply awaiting
            // its fill; re-sourcing it bounces empties between stations forever.
            if self.putaway_presentation_cells.contains(&entry.cell()) {
                continue;
            }
            if self.presentation_cells.contains(&entry.cell()) && entry.available_at() < after {
                continue;
            }
            let Some(travel) = self.travel_to(entry.robot(), entry.cell(), entry.orientation, station_cell)? else {
                continue;
            };
            let depart = entry.available_at().max(after);
            let candidate = DirectCandidate { entry, depart, travel };
            if best.as_ref().is_none_or(|best| candidate.rank() < best.rank()) {
                best = Some(candidate);
            }
        }
        Ok(best)
    }

    pub(crate) fn quote_from_rack(
        &self,
        station_cell: CellId,
        after: SimTime,
        target: SimTime,
    ) -> Result<Option<RelayRetrieveSelection>, PlanError> {
        match self.relay.select_relay_for_retrieve_near(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            station_cell,
            after,
            target,
        ) {
            Ok(selection) => Ok(Some(selection)),
            Err(e @ (ReservationError::NoCandidates { .. } | ReservationError::HeadroomExhausted { .. })) => {
                crate::debug_log!(
                    "[PutawaySupply.quote_from_rack] declined station_cell={station_cell:?} after={}ms: {e}",
                    after.as_ms()
                );
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    // The HTM leaves the relay only once the tote is deposited AND the HTM has
    // reached it; the legs overlap, so only its slack past the deposit adds.
    pub(crate) fn rack_arrival(&self, selection: &RelayRetrieveSelection) -> SimTime {
        let free_htm =
            self.available.peek_matching(|htm| htm.tote.is_none()).iter().map(|htm| htm.available_at()).min();
        let dispatch = free_htm.unwrap_or(selection.start);
        selection.completion_time + ((dispatch + selection.estimated_htm_to_relay) - selection.path_start_at)
    }
}

impl StateManager for PutawaySupply {
    fn create(states: &States) -> Result<Self, StateError> {
        let input = inputs(states)?;
        let rack_forbidden_from = input
            .config
            .putaway
            .forbid_rack_after_secs
            .map(|secs| policy::duration("putaway.forbid_rack_after_secs", secs).map(|after| SimTime::ZERO + after))
            .transpose()
            .map_err(|error| StateError::Construction { kind: type_name::<Self>(), detail: error.to_string() })?;
        let stations = states.get::<StationSet>()?;
        let bounds = states.get::<StationFetchBounds>()?;
        Ok(Self::new(
            &stations,
            &bounds,
            states.get::<Navigation>()?,
            states.get::<RelayService>()?.as_ref().clone(),
            states.get::<InventoryService>()?.as_ref().clone(),
            states.get::<AvailableHtmRegistry>()?.as_ref().clone(),
            rack_forbidden_from,
        ))
    }
}

#[derive(Display)]
#[display("PutawaySupplyDemand")]
#[task]
pub struct PutawaySupplyDemand {
    arrivals: ArrivalQueue,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
    relay: RelayService,
    pub supply: Arc<PutawaySupply>,
    pub admission: AdmissionLaw,
    pub direct_bias: SimOffset,
}

#[async_trait(?Send)]
impl Task for PutawaySupplyDemand {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let params: SupplyTaskConfig = ctx.config()?;
        let input = inputs(ctx.states())?;
        let world = world_inputs(ctx.states())?;
        let parking = ctx.state::<ParkingRegistry>()?;
        let stations = ctx.state::<StationSet>()?;
        let bounds = ctx.state::<StationFetchBounds>()?;
        let admission = AdmissionLaw {
            multiplier: input.config.putaway.htm_wip_multiplier(),
            period: input.definition.induction_period(&input.config)?,
            override_cap: None,
        };
        policy::echo_admission_caps(&stations, &bounds, None, Some(&admission), input.config.putaway.queue_depth);
        Ok(Self {
            arrivals: ctx.state::<Arrivals>()?.0.clone(),
            supply: ctx.state()?,
            navigation: ctx.state::<Navigation>()?,
            relay: ctx.state::<RelayService>()?.as_ref().clone(),
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            station_service: ctx.state::<StationService>()?,
            station_arrivals: ctx.state::<StationArrivalDiagnostics>()?.as_ref().clone(),
            tripwires: world.config.tripwires,
            delivery_max_wait_override: world
                .config
                .pick
                .delivery_max_wait_secs
                .map(|secs| SimDuration::ms((secs * 1000.0) as u64)),
            admission,
            direct_bias: policy::direct_bias(
                !stations.putaway_stations().is_empty(),
                &world.data.map,
                params.direct_bias_secs.or(input.config.putaway.direct_bias_secs),
            )?,
        })
    }

    fn holds_completion() -> bool {
        false
    }

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        parse_config::<SupplyTaskConfig>(config).map(|_| ())
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        loop {
            if self.arrivals.with(|queue| queue.is_empty()) {
                return Ok(());
            }
            let fleet_seen = self.available.version().seen();
            let inventory_seen = self.inventory.version().seen();
            let relay_changed = notified(self.relay.signal());
            let supply_seen = self.supply.version().seen();
            let openings = self.supply.openings(self.admission);
            for opening in &openings {
                let ticket = self.supply.begin_staging(opening.station_id)?;
                ctx.add_task(StageEmpty::new(self, *opening, ticket, ctx.barrier().clock()));
            }
            if !openings.is_empty() {
                continue;
            }
            select! {
                biased;
                _ = self.available.version().past(fleet_seen) => {}
                _ = self.inventory.version().past(inventory_seen) => {}
                _ = relay_changed => {}
                _ = self.supply.version().past(supply_seen) => {}
            }
        }
    }
}

/// A live empty that could feed an opening directly: its stay, when it
/// would leave for the station (simtime), and the drive's length.
pub(crate) struct DirectCandidate {
    pub(crate) entry: AvailableHtm,
    pub(crate) depart: SimTime,
    pub(crate) travel: SimDuration,
}

impl DirectCandidate {
    pub(crate) fn arrival(&self) -> SimTime {
        self.depart + self.travel
    }

    fn rank(&self) -> (SimTime, SimTime, RobotId) {
        (self.arrival(), self.depart, self.entry.robot())
    }
}

#[cfg(test)]
#[path = "feed_putaway_stations_tests.rs"]
mod tests;
