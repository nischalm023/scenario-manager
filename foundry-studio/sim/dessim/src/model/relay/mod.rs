//! Within `model`, `relay` is the tote-transfer shelf family: the fixed
//! shelf positions where VTMs and HTMs exchange totes, with one service
//! owning their slot state, reservations, and retrieve candidacies.
//!
//! This file is the retrieve family's candidacy service: it mints and
//! invalidates [`ProposedRetrieve`] candidacies, spends an accepted one
//! into a real relay reservation plus the tote claim (zone inbound
//! headroom is its admissible surface), suspends `source_retrieve`
//! callers until a tentative can be minted, and floors every selection
//! at the admission barrier.
//!
//! TODO(overstuffed): tote/slot selection, zone headroom accounting, the
//! seeded random-deposit probe, the leak ledger, and the acceptance and
//! deposit-quote instrumentation all live here beside the candidacy
//! service. Split the selectors and the bookkeeping out so this file is
//! the candidacy service alone.

mod acceptance_stats;
pub mod config;
mod deposit_quote_stats;
mod headroom;
pub mod htm_index;
pub mod inventory_index;
mod pickup_units;
mod reservation;
mod timeline;
pub mod vtm;

#[cfg(any(test, feature = "test-support"))]
pub mod tests;

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::future::{Future, pending};
use std::ops::Deref;
use std::pin::Pin;

use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use desgraph::{CellId, RelayId, WorldPos, ZoneId};
use desmap::{WarehouseMap, ZoneHolds};

use crate::config::VtmConfig;
use crate::error::{InvalidScenario, ReservationErrContext, ReservationError};
use crate::model::RackId;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::ids::{RackSlot, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::inventory::error::InventoryError;
use crate::model::inventory::events::InventoryObserver;
use crate::model::location::ToteDesignation;
use crate::model::location::ToteLocation;
use crate::model::navigation::Navigation;
use crate::model::navigation::heading::unassigned_htm_orientation;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::reservation::oracle::{CellStatus, ReservationOracle};
use crate::model::{RobotId, SimDuration, SimTime, SkuId};
use crate::pathfinding::{CellCandidate, NearestCellSelector, Orientation};
use crate::pathfinding::{PathCacheKey, PathSelectionError};
use crate::perf_span;
use crate::robots::runtime::RobotModelId;
use crate::robots::vtm::motion::CycleBreakdown;
use crate::scenario_gen::RackUse;
use crate::scenario_gen::ScenarioData;
use crate::scheduler::task::proposal::{AcceptError, ProposalId, ProposedReservation};
use crate::signals::{Mut, Signal, Version, notified};
use crate::utils::temporal_counter::TemporalCounter;
use tokio::select;

use crate::utils::rng::Rng;

use crate::model::location::ToteType;
use acceptance_stats::AcceptanceStats;
use config::Relay;
use deposit_quote_stats::DepositQuoteStats;
use inventory_index::RelayInventoryIndex;

pub use config::RelayZones;
pub use headroom::RetrieveAdmission;
pub use pickup_units::PickupUnits;
pub use reservation::{RelayReservation, RelayStartCause};
pub use timeline::ReservationDirection;

pub use htm_index::{PICK_HTM_TO_RELAY_DEFAULT, RelayHtmIndex};
pub use inventory_index::{IndexCandidate, SlotCandidate, ToteCandidate};
pub use vtm::{VtmAssignment, VtmFrontier, VtmService};

pub const DEPOSIT_ACCESS_MAX_WAIT: SimDuration = SimDuration::seconds(300);

pub const RETRIEVE_ACCESS_MAX_WAIT: SimDuration = SimDuration::seconds(300);

#[derive(Clone, Copy)]
pub enum ToteSearchCriteria {
    ContainsSku {
        sku: SkuId,
    }, // TODO: qty optimization later when we have >1 unit per tote
    /// An empty tote in rack, from one pool. The two pools never substitute
    /// for each other: putaway asks for `Inventory`, close-out for `Order`,
    /// and neither can drain the other.
    Empty {
        designation: ToteDesignation,
    },
    /// An empty container of a particular physical type, from one pool.
    EmptyOfType {
        designation: ToteDesignation,
        tote_type: ToteType,
    },
    /// One NAMED order tote, wherever it is racked. The only search that says
    /// WHICH tote it wants rather than what kind, because releasing an order
    /// from the buffer fetches that order's tote, not any spare.
    ///
    /// It is also the only search that may reach into buffer racking; see
    /// [`ToteSearchCriteria::racking`].
    BufferedOrder {
        tote_id: ToteId,
    },
}

impl ToteSearchCriteria {
    /// Which ZONES answer this search.
    ///
    /// The only zone-wide distinction is the order buffer, which is whole
    /// aisles of it: a pick must not reach in there, and the empty order tote
    /// close-out wants must come from the row of empty order totes rather than
    /// from under a closed order already resting in the buffer. The row itself
    /// is bays inside the ordinary storage aisles, so it needs no zone of its
    /// own -- the tote's own designation is what separates it from stock.
    fn racking(&self) -> ZoneHolds {
        match self {
            Self::ContainsSku { .. } | Self::Empty { .. } | Self::EmptyOfType { .. } => ZoneHolds::Inventory,
            Self::BufferedOrder { .. } => ZoneHolds::Order,
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RelayUtilizationSummary {
    pub overall_pct: f64,
    pub total_relays: usize,
    pub report_horizon_ms: u64,

    pub per_zone: Vec<(u32, f64)>,
}

impl RelayUtilizationSummary {
    pub fn zone_min_avg_max(&self) -> (f64, f64, f64) {
        if self.per_zone.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sum = 0.0;
        for &(_, p) in &self.per_zone {
            min = min.min(p);
            max = max.max(p);
            sum += p;
        }
        (min, sum / self.per_zone.len() as f64, max)
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RelayUtilizationTimeline {
    pub slice_width_ms: u64,

    pub zones: Vec<RelayZoneUtilizationLane>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RelayZoneUtilizationLane {
    pub zone_id: u32,

    pub relays: u32,

    pub busy_ms: Vec<u64>,
}

impl RelayUtilizationTimeline {
    pub fn per_zone_pct(&self) -> Vec<(u32, f64)> {
        self.zones.iter().map(|zone| (zone.zone_id, self.zone_pct(zone))).collect()
    }

    pub fn overall_pct(&self) -> f64 {
        let busy: u64 = self.zones.iter().flat_map(|z| &z.busy_ms).sum();
        let relays: usize = self.zones.iter().map(|z| z.relays as usize).sum();
        let slices = self.zones.first().map_or(0, |z| z.busy_ms.len()) as u64;
        duty_pct(busy, relays, slices * self.slice_width_ms)
    }

    fn zone_pct(&self, zone: &RelayZoneUtilizationLane) -> f64 {
        let busy: u64 = zone.busy_ms.iter().sum();
        duty_pct(busy, zone.relays as usize, zone.busy_ms.len() as u64 * self.slice_width_ms)
    }
}

fn bucket_busy_interval(busy: &mut [u64], start: u64, end: u64, slice_width_ms: u64) {
    if end <= start {
        return;
    }
    let mut cursor = start;
    while cursor < end {
        let index = (cursor / slice_width_ms) as usize;
        if index >= busy.len() {
            break;
        }
        let slice_end = (index as u64 + 1) * slice_width_ms;
        let segment_end = end.min(slice_end);
        busy[index] += segment_end - cursor;
        cursor = segment_end;
    }
}

fn duty_pct(busy_ms: u64, relays: usize, horizon_ms: u64) -> f64 {
    let denom = relays as u64 * horizon_ms;
    if denom == 0 { 0.0 } else { 100.0 * busy_ms as f64 / denom as f64 }
}

fn bucket_merged_intervals(busy: &mut [u64], mut intervals: Vec<(u64, u64)>, slice_width_ms: u64) {
    intervals.sort_unstable();
    let mut current: Option<(u64, u64)> = None;
    for (start, end) in intervals {
        match current {
            None => current = Some((start, end)),
            Some((cur_start, cur_end)) if start > cur_end => {
                bucket_busy_interval(busy, cur_start, cur_end, slice_width_ms);
                current = Some((start, end));
            }
            Some((cur_start, cur_end)) => current = Some((cur_start, cur_end.max(end))),
        }
    }
    if let Some((cur_start, cur_end)) = current {
        bucket_busy_interval(busy, cur_start, cur_end, slice_width_ms);
    }
}

use timeline::RelayTimeline;

#[derive(Clone, Debug)]
struct ScoredCandidate<R> {
    relay: Relay,
    item: R,
    start: SimTime,
    path_start_at: SimTime,
    estimated_htm_to_relay: SimDuration,

    slot_wait: SimDuration,

    start_cause: Option<RelayStartCause>,
}

#[derive(Clone, Debug)]
pub struct RelayRetrieveSelection {
    pub recommendation: ToteId,
    pub relay_id: RelayId,
    pub zone_id: ZoneId,
    pub start: SimTime,

    pub path_start_at: SimTime,

    pub estimated_htm_to_relay: SimDuration,
    pub completion_time: SimTime,

    pub slot_wait: SimDuration,

    pub start_cause: RelayStartCause,
}

// Dropping one deregisters the tentative and releases an accepted slot
// reservation; a claimed tote stays claimed, exactly as an aborted
// chain leaves it.
pub struct ProposedRetrieve {
    service: RelayService,
    owner: ProposalId,

    pub selection: RelayRetrieveSelection,
    poisoned: Arc<AtomicBool>,
    registered: bool,
    accepted: Option<(RelayReservation<ToteId>, RackSlot)>,
    admission: Option<RetrieveAdmission>,
}

impl fmt::Display for ProposedRetrieve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "retrieve{{{} relay={:?} tote={:?} net={}}}",
            self.owner, self.selection.relay_id, self.selection.recommendation, self.selection.start
        )
    }
}

impl Drop for ProposedRetrieve {
    fn drop(&mut self) {
        if self.registered {
            self.service.deregister_retrieve_tentative(self.owner);
        }
        if let Some((reservation, _)) = self.accepted.take() {
            reservation.cancel();
        }
    }
}

impl ProposedRetrieve {
    fn claim_tote(&self, tote_id: ToteId, start: SimTime) -> Result<RackSlot, AcceptError> {
        let tote = self.service.inventory.tote(tote_id).map_err(|e| AcceptError::TornState {
            detail: format!("accepted retrieve tote {tote_id:?} lookup failed: {e}"),
        })?;
        match tote.location_at(start) {
            ToteLocation::InRack { rack_id, z_index } => {
                tote.move_to(ToteLocation::Reserved { rack_id, z_index }, start).map_err(|e| {
                    AcceptError::TornState { detail: format!("accepted retrieve tote {tote_id:?} claim failed: {e}") }
                })?;
                Ok(RackSlot { rack_id, level: z_index })
            }
            ToteLocation::Reserved { rack_id, z_index } => Ok(RackSlot { rack_id, level: z_index }),
            other => Err(AcceptError::TornState {
                detail: format!("accepted retrieve tote {tote_id:?} not in rack at {start}: {other:?}"),
            }),
        }
    }

    pub fn into_accepted(mut self) -> Result<(RelayReservation<ToteId>, RackSlot), ReservationError> {
        self.accepted.take().ok_or(ReservationError::NotAccepted)
    }

    fn protected_slots(&self) -> u32 {
        self.admission.as_ref().map_or(0, RetrieveAdmission::protected_slots)
    }

    fn is_valid(&self) -> bool {
        !ProposedReservation::poisoned(self)
    }
}

impl ProposedReservation for ProposedRetrieve {
    fn start_time(&self) -> Option<SimTime> {
        Some(self.selection.start)
    }

    fn poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Relaxed)
            || !self.service.can_admit_retrieve_with_reserve(self.selection.zone_id, self.protected_slots())
    }

    // The retrieve claim beyond the relay itself: acceptance spends one
    // unit of the zone's inbound headroom, so the scheduler asks this
    // before accepting.
    fn admissible(&self) -> bool {
        self.service.can_admit_retrieve_with_reserve(self.selection.zone_id, self.protected_slots())
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        self.registered = false;
        let reservation =
            self.service.accept_retrieve(self.owner, &self.poisoned, &self.selection, self.protected_slots())?;

        let rack_slot = match self.claim_tote(reservation.recommendation, reservation.start) {
            Ok(rack_slot) => rack_slot,
            Err(e) => {
                reservation.cancel();
                return Err(e);
            }
        };
        self.accepted = Some((reservation, rack_slot));
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct RetrieveCandidate {
    resident: Option<(RobotId, Orientation)>,
    inner: IndexCandidate<ToteId>,

    relay_ready_at: SimTime,

    path_start_at: SimTime,

    estimated_htm_to_relay: SimDuration,

    slot_wait: SimDuration,

    start_cause: RelayStartCause,
}

impl CellCandidate for RetrieveCandidate {
    fn cell(&self) -> CellId {
        self.inner.relay.cell_id
    }
}

#[derive(Clone, Debug)]
struct DepositCandidate {
    inner: IndexCandidate<RackSlot>,

    relay_ready_at: SimTime,

    slot_wait: SimDuration,
}

impl CellCandidate for DepositCandidate {
    fn cell(&self) -> CellId {
        self.inner.relay.cell_id
    }
}

#[derive(Clone)]
pub struct RelayService(Arc<Inner>);

impl Deref for RelayService {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Inner {
    // ordered: selection candidate walks and status reports iterate relays deterministically
    relays: BTreeMap<RelayId, Relay>,
    zone_capacity: FxHashMap<ZoneId, u32>,
    /// Zones whose racking is the order buffer, and its complement. Every
    /// reservation names one of the two and unions the other into its skip
    /// set, so inventory work can never land in the buffer and buffer work
    /// can never land outside it.
    ///
    /// `holds` is a per-zone property, so a racking restriction IS a zone
    /// restriction; nothing finer is needed. Precomputed because a
    /// reservation pass unions one of these on every call.
    buffer_zones: FxHashSet<ZoneId>,
    inventory_zones: FxHashSet<ZoneId>,
    inventory: InventoryService,
    inv_index: RelayInventoryIndex,

    _inv_index_observer: Arc<dyn InventoryObserver>,
    navigation: Arc<Navigation>,
    map: Arc<WarehouseMap>,
    state: Mut<State>,

    pub vtm_service: VtmService,

    oracle: Arc<dyn ReservationOracle>,

    htm_index: RelayHtmIndex,
    relay_headings: FxHashMap<(RobotModelId, RelayId), Result<Orientation, InvalidScenario>>,

    // Reservations dropped unfinalized land here instead of panicking (Drop
    // never panics); the planner audits it at end of run — leaks after a
    // NATURAL completion are a bug, leaks in a stopped run's teardown are
    // the dropped chains' expected remains. std Mutex, poison-tolerated:
    // recording during an unwinding teardown must never re-panic.
    leaked: StdMutex<Vec<(RelayId, ZoneId, SimTime)>>,
    accept_stats: StdMutex<AcceptanceStats>,
    deposit_quotes: StdMutex<DepositQuoteStats>,
    /// `Some` when `putaway.random_slots` is on: the seeded stream the
    /// deposit selector draws its ranking keys from. `None` is the ranked
    /// default, which draws nothing at all.
    random_deposit: StdMutex<Option<Rng>>,
    admission_floor: OnceLock<Mut<SimTime>>,
}

#[derive(Clone, Debug)]
pub struct ParkingReliefTarget {
    /// The relay whose access the squatter blocks; `None` when the caller is
    /// clearing a non-relay cell (a belt dropoff, housekeeping) — log-only.
    pub relay_id: Option<RelayId>,
    pub zone_id: Option<ZoneId>,
    pub cell: CellId,
    pub robot: RobotId,
    pub not_before: SimTime,
}

/// A relief call's answer, whether the squatter was moved, once the
/// pathfinder it drives has answered.
pub type ReliefFuture<'a> = Pin<Box<dyn Future<Output = Result<bool, ReservationError>> + 'a>>;

pub struct ParkingRelief<'a> {
    park: Box<dyn Fn(ParkingReliefTarget) -> ReliefFuture<'a> + 'a>,
}

impl<'a> ParkingRelief<'a> {
    pub fn new<F>(park: F) -> Self
    where
        F: Fn(ParkingReliefTarget) -> ReliefFuture<'a> + 'a,
    {
        Self { park: Box::new(park) }
    }

    async fn park(&self, target: ParkingReliefTarget) -> Result<bool, ReservationError> {
        (self.park)(target).await
    }
}

#[derive(Clone, Debug)]
struct BlockedDepositCandidate {
    relay: Relay,
    item: RackSlot,

    start: SimTime,
    blocker: RobotId,

    slot_wait: SimDuration,
}

#[derive(Default)]
struct RelayZoneDutyRow {
    relays: usize,
    cell_reservations: usize,
    open_cell_holds: usize,
    unfinalized_slots: usize,
    unfinalized_inbound: usize,
    unfinalized_outbound: usize,
    max_cell_frontier_ms: u64,
    max_slot_frontier_ms: u64,

    busy_ms: u64,
    gap_count: usize,
    gap_ms: u64,
    max_gap_ms: u64,
    long_holds: usize,
}

pub(crate) struct RelayStatusReport<'a> {
    service: &'a RelayService,
    oracle: &'a dyn ReservationOracle,
    report_horizon_ms: u64,
}

impl fmt::Display for RelayStatusReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.service.fmt_status_report(f, self.oracle, self.report_horizon_ms)
    }
}

struct State {
    // ordered: the forcing sweep (retrieve_fillable_relays) walks relays deterministically
    timelines: BTreeMap<RelayId, RelayTimeline>,
    zone_pressure: FxHashMap<ZoneId, TemporalCounter>,

    direction_counts: FxHashMap<(ZoneId, ReservationDirection), u32>,

    retrieve_tentatives: Vec<RetrieveTentative>,
}

struct RetrieveTentative {
    owner: ProposalId,
    relay_id: RelayId,
    tote: ToteId,
    net_start: SimTime,
    poisoned: Arc<AtomicBool>,
}

impl State {
    fn relay_available(&self, relay_id: RelayId) -> Option<SimTime> {
        self.timelines.get(&relay_id).filter(|tl| !tl.is_unfinalized()).map(|tl| tl.available_at())
    }
}

impl RelayService {
    pub fn new(
        data: &ScenarioData,
        inventory: InventoryService,
        navigation: Arc<Navigation>,
        htm_registry: &AvailableHtmRegistry,
        vtm_cfg: &VtmConfig,
    ) -> Result<Self, InventoryError> {
        let zones = &data.relay_zones;
        let relays = zones.relay_index();

        let zone_capacity: FxHashMap<ZoneId, u32> = zones.zone_capacities().into_iter().collect();
        for (&zid, &cap) in &zone_capacity {
            if cap > 0 && cap <= 2 {
                eprintln!(
                    "WARNING: zone {:?} has {} relays — deadlock headroom requires ≥3. \
                     Zone will reject all reservations.",
                    zid, cap,
                );
            }
        }
        let zone_pressure: FxHashMap<ZoneId, TemporalCounter> =
            zone_capacity.keys().map(|&z| (z, TemporalCounter::new())).collect();
        // What each zone's steel is FOR, read off the map. A map that states
        // no `holds` is all inventory, which is what every map said before
        // order buffers existed.
        let buffer_zones: FxHashSet<ZoneId> =
            data.map.zones.iter().filter(|z| z.holds == ZoneHolds::Order).map(|z| z.id).collect();
        let inventory_zones: FxHashSet<ZoneId> =
            zone_capacity.keys().copied().filter(|z| !buffer_zones.contains(z)).collect();
        let rack_use: FxHashMap<RackId, RackUse> = data.rack_use.iter().map(|(&rack, &uses)| (rack, uses)).collect();
        let inv_index = RelayInventoryIndex::new(zones, inventory.clone(), &rack_use);
        let inv_index_observer = inventory
            .add_observer_with_snapshot(Arc::new(inv_index.clone()), |snapshot| inv_index.initialize(snapshot))?;
        let map = Arc::new(data.map.clone());
        let relay_headings = navigation
            .models()
            .into_iter()
            .flat_map(|model| {
                relays
                    .iter()
                    .map(|(&id, relay)| {
                        ((model.id, id), unassigned_htm_orientation(&map, relay.cell_id, model.estimate[0].as_ref()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let oracle = navigation.oracle.clone();
        let vtm_service = VtmService::new(data, vtm_cfg);
        let htm_index = RelayHtmIndex::new(relays.values().map(|r| (r.id, r.cell_id)));
        htm_registry.add_observer(Arc::new(htm_index.clone()));
        Ok(Self(Arc::new(Inner {
            zone_capacity,
            buffer_zones,
            inventory_zones,
            inventory,
            inv_index,
            _inv_index_observer: inv_index_observer,
            state: Mut::new(State {
                timelines: relays.keys().map(|&id| (id, RelayTimeline::new())).collect(),
                zone_pressure,
                direction_counts: FxHashMap::default(),
                retrieve_tentatives: Vec::new(),
            }),
            relays,
            navigation,
            map,
            vtm_service,
            oracle,
            htm_index,
            relay_headings,
            leaked: StdMutex::new(Vec::new()),
            accept_stats: StdMutex::new(AcceptanceStats::default()),
            deposit_quotes: StdMutex::new(DepositQuoteStats::default()),
            random_deposit: StdMutex::new(None),
            admission_floor: OnceLock::new(),
        })))
    }

    pub(crate) fn htm_access_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.0.relays.values().map(|relay| relay.cell_id)
    }

    pub fn htm_access_cell(&self, relay: RelayId) -> Option<CellId> {
        self.0.relays.get(&relay).map(|r| r.cell_id)
    }

    pub fn htm_access_holder(&self, relay: RelayId) -> Option<RobotId> {
        let cell = self.htm_access_cell(relay)?;
        match self.0.oracle.cell_status(cell) {
            CellStatus::PartiallyPlanned { robot } => Some(robot),
            CellStatus::Schedulable => None,
        }
    }

    /// The HTM-side access cells of the order buffer's relays: where a robot
    /// stands to hand a closed order's tote to a buffer VTM, and to take it
    /// back. Empty on a map that states no buffer racking.
    pub fn buffer_relay_htm_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.0.relays.values().filter(|relay| self.0.buffer_zones.contains(&relay.zone_id)).map(|relay| relay.cell_id)
    }

    /// Whether this map states any order-buffer racking at all. A map without
    /// it routes every closed order straight to extraction.
    pub fn has_order_buffer(&self) -> bool {
        !self.0.buffer_zones.is_empty()
    }

    /// Read the HTM-to-relay travel-time estimate from the per-relay index.
    /// 0 if a resident no-tote HTM holds the relay's access cell;
    /// `PICK_HTM_TO_RELAY_DEFAULT` otherwise. Trace-only accessor; scoring
    /// reads `htm_index` directly to avoid a duplicate lookup.
    pub fn htm_to_relay_estimate(&self, relay_id: RelayId) -> SimDuration {
        self.0.htm_index.approach(relay_id).htm_to_relay
    }

    pub fn zone_ids(&self) -> Vec<ZoneId> {
        let mut zones = self.0.relays.values().map(|relay| relay.zone_id).collect::<Vec<_>>();
        zones.sort_unstable();
        zones.dedup();
        zones
    }

    pub fn utilization_summary(
        &self,
        oracle: &dyn ReservationOracle,
        report_horizon_ms: u64,
    ) -> RelayUtilizationSummary {
        let mut busy_by_zone: BTreeMap<u32, (u64, usize)> = BTreeMap::new();
        for relay in self.0.relays.values() {
            let stats = oracle.cell_duty_stats(relay.cell_id, report_horizon_ms);
            let e = busy_by_zone.entry(relay.zone_id.0).or_insert((0, 0));
            e.0 += stats.busy_ms;
            e.1 += 1;
        }
        let per_zone: Vec<(u32, f64)> =
            busy_by_zone.iter().map(|(&z, &(busy, relays))| (z, duty_pct(busy, relays, report_horizon_ms))).collect();
        let total_relays: usize = busy_by_zone.values().map(|(_, r)| *r).sum();
        let total_busy: u64 = busy_by_zone.values().map(|(b, _)| *b).sum();
        RelayUtilizationSummary {
            overall_pct: duty_pct(total_busy, total_relays, report_horizon_ms),
            total_relays,
            report_horizon_ms,
            per_zone,
        }
    }

    fn build_zone_timeline(
        &self,
        report_horizon_ms: u64,
        slice_width_ms: u64,
        mut intervals_for: impl FnMut(&Relay) -> Vec<(u64, u64)>,
    ) -> RelayUtilizationTimeline {
        let num_slices = (report_horizon_ms / slice_width_ms + 1) as usize;

        let mut zones: BTreeMap<u32, (u32, Vec<u64>)> = BTreeMap::new();
        for relay in self.0.relays.values() {
            let entry = zones.entry(relay.zone_id.0).or_insert_with(|| (0, vec![0u64; num_slices]));
            entry.0 += 1;
            bucket_merged_intervals(&mut entry.1, intervals_for(relay), slice_width_ms);
        }
        RelayUtilizationTimeline {
            slice_width_ms,
            zones: zones
                .into_iter()
                .map(|(zone_id, (relays, busy_ms))| RelayZoneUtilizationLane { zone_id, relays, busy_ms })
                .collect(),
        }
    }

    pub fn slot_occupancy_timeline(&self, report_horizon_ms: u64, slice_width_ms: u64) -> RelayUtilizationTimeline {
        self.0.state.with(|state| {
            self.build_zone_timeline(report_horizon_ms, slice_width_ms, |relay| {
                let Some(timeline) = state.timelines.get(&relay.id) else {
                    return Vec::new();
                };
                let mut intervals = timeline.occupancy_intervals().to_vec();
                if let Some(start) = timeline.pending_start() {
                    intervals.push((start.as_ms(), report_horizon_ms));
                }
                intervals
            })
        })
    }

    pub(crate) fn status_report<'a>(
        &'a self,
        oracle: &'a dyn ReservationOracle,
        report_horizon_ms: u64,
    ) -> RelayStatusReport<'a> {
        RelayStatusReport { service: self, oracle, report_horizon_ms }
    }

    fn fmt_status_report(
        &self,
        f: &mut fmt::Formatter<'_>,
        oracle: &dyn ReservationOracle,
        report_horizon_ms: u64,
    ) -> fmt::Result {
        fn seconds(ms: u64) -> f64 {
            ms as f64 / 1000.0
        }

        fn open_or_seconds(has_open: bool, ms: u64) -> String {
            if has_open { "open".to_string() } else { format!("{:.1}", seconds(ms)) }
        }

        fn direction_label(direction: Option<ReservationDirection>) -> &'static str {
            match direction {
                Some(ReservationDirection::Inbound) => "in",
                Some(ReservationDirection::Outbound) => "out",
                None => "-",
            }
        }

        let slot_rows: FxHashMap<RelayId, (bool, u64, Option<ReservationDirection>)> = self.0.state.with(|state| {
            self.0
                .relays
                .keys()
                .map(|&rid| {
                    let tl = state.timelines.get(&rid);
                    (
                        rid,
                        (
                            tl.map(|t| t.is_unfinalized()).unwrap_or(false),
                            tl.map(|t| t.available_at().as_ms()).unwrap_or(0),
                            tl.and_then(|t| t.direction()),
                        ),
                    )
                })
                .collect()
        });
        let mut zone_rows: BTreeMap<u32, RelayZoneDutyRow> = BTreeMap::new();
        for relay in self.0.relays.values() {
            let stats = oracle.cell_duty_stats(relay.cell_id, report_horizon_ms);
            let row = zone_rows.entry(relay.zone_id.0).or_default();
            row.relays += 1;
            row.cell_reservations += stats.reservations;
            row.busy_ms += stats.busy_ms;
            row.open_cell_holds += stats.open_ended;
            row.max_cell_frontier_ms = row.max_cell_frontier_ms.max(stats.frontier_ms);
            row.max_gap_ms = row.max_gap_ms.max(stats.longest_gap_ms);
            row.gap_count += stats.gap_count;
            row.gap_ms += stats.gap_ms;
            row.long_holds += usize::from(stats.longest_hold_ms >= 300_000);
            if let Some(&(unfinalized, frontier_ms, direction)) = slot_rows.get(&relay.id) {
                row.max_slot_frontier_ms = row.max_slot_frontier_ms.max(frontier_ms);
                if unfinalized {
                    row.unfinalized_slots += 1;
                    match direction {
                        Some(ReservationDirection::Inbound) => row.unfinalized_inbound += 1,
                        Some(ReservationDirection::Outbound) => row.unfinalized_outbound += 1,
                        None => {}
                    }
                }
            }
        }

        let util = self.utilization_summary(oracle, report_horizon_ms);
        let (z_min, z_avg, z_max) = util.zone_min_avg_max();
        writeln!(
            f,
            "relay_utilization_pct={:.1} ({} relays, {} zones, horizon {:.0}s) | per-zone min={:.1} avg={:.1} max={:.1}",
            util.overall_pct,
            util.total_relays,
            util.per_zone.len(),
            seconds(report_horizon_ms),
            z_min,
            z_avg,
            z_max,
        )?;

        writeln!(f, "relay_zones:")?;
        writeln!(
            f,
            "  {:>4} {:>6} {:>6} {:>8} {:>9} {:>9} {:>3} {:>3} {:>11} {:>11} {:>5} {:>10} {:>10} {:>8}",
            "zone",
            "relays",
            "util%",
            "cell_res",
            "open_cell",
            "slot_open",
            "in",
            "out",
            "cell_fr_s",
            "slot_fr_s",
            "gaps",
            "gap_s",
            "max_gap_s",
            "holds5m",
        )?;
        for (zone, row) in &zone_rows {
            writeln!(
                f,
                "  {:>4} {:>6} {:>6.1} {:>8} {:>9} {:>9} {:>3} {:>3} {:>11} {:>11} {:>5} {:>10.1} {:>10.1} {:>8}",
                zone,
                row.relays,
                duty_pct(row.busy_ms, row.relays, report_horizon_ms),
                row.cell_reservations,
                row.open_cell_holds,
                row.unfinalized_slots,
                row.unfinalized_inbound,
                row.unfinalized_outbound,
                open_or_seconds(row.open_cell_holds > 0, row.max_cell_frontier_ms),
                open_or_seconds(row.unfinalized_slots > 0, row.max_slot_frontier_ms),
                row.gap_count,
                seconds(row.gap_ms),
                seconds(row.max_gap_ms),
                row.long_holds,
            )?;
        }

        let mut relay_rows: Vec<_> = self
            .0
            .relays
            .values()
            .map(|relay| {
                let stats = oracle.cell_duty_stats(relay.cell_id, report_horizon_ms);
                let (slot_unfinalized, slot_frontier_ms, direction) =
                    slot_rows.get(&relay.id).copied().unwrap_or((false, 0, None));
                (*relay, stats, slot_unfinalized, slot_frontier_ms, direction)
            })
            .collect();
        relay_rows.sort_by_key(|(relay, stats, slot_unfinalized, slot_frontier_ms, _)| {
            (
                Reverse(stats.open_ended),
                Reverse(stats.longest_hold_ms),
                Reverse(*slot_unfinalized as u8),
                Reverse(*slot_frontier_ms),
                relay.zone_id.0,
                relay.id.0,
            )
        });

        writeln!(f, "top_relay_cells:")?;
        writeln!(
            f,
            "  {:>4} {:>5} {:>6} {:>4} {:>4} {:>10} {:>10} {:>5} {:>3} {:>6} {:>5} {:>9} {:>9} {:>10}",
            "zone",
            "relay",
            "cell",
            "res",
            "open",
            "cell_fr_s",
            "slot_fr_s",
            "slot?",
            "dir",
            "duty%",
            "gaps",
            "gap_s",
            "max_gap_s",
            "max_hold_s",
        )?;
        for (relay, stats, slot_unfinalized, slot_frontier_ms, direction) in relay_rows.into_iter().take(30) {
            writeln!(
                f,
                "  {:>4} {:>5} {:>6} {:>4} {:>4} {:>10} {:>10} {:>5} {:>3} {:>6.1} {:>5} {:>9.1} {:>9.1} {:>10}",
                relay.zone_id.0,
                relay.id.0,
                relay.cell_id.0,
                stats.reservations,
                stats.open_ended,
                open_or_seconds(stats.open_ended > 0, stats.frontier_ms),
                open_or_seconds(slot_unfinalized, slot_frontier_ms),
                if slot_unfinalized { "open" } else { "no" },
                direction_label(direction),
                stats.duty_pct(),
                stats.gap_count,
                seconds(stats.gap_ms),
                seconds(stats.longest_gap_ms),
                open_or_seconds(stats.open_ended > 0, stats.longest_hold_ms),
            )?;
        }
        Ok(())
    }

    pub fn vtm_forward_cycle(&self, relay_id: RelayId, slot: RackSlot, from_pos: WorldPos) -> Option<CycleBreakdown> {
        self.0.inv_index.forward_cycle(relay_id, slot, from_pos)
    }

    pub fn vtm_return_cycle(&self, relay_id: RelayId, slot: RackSlot, from_pos: WorldPos) -> Option<CycleBreakdown> {
        self.0.inv_index.return_cycle(relay_id, slot, from_pos)
    }

    fn deadlock_headroom_limit(&self, zone_id: ZoneId) -> u32 {
        self.0.zone_capacity.get(&zone_id).copied().unwrap_or(0).saturating_sub(2)
    }

    fn earliest_vtm_available(&self, zone_id: ZoneId) -> Option<SimTime> {
        self.0.vtm_service.frontier_in_zone(zone_id).into_iter().map(|f| f.available_at).min()
    }

    fn open_ended_count(&self, zone_id: ZoneId, direction: ReservationDirection, state: &State) -> u32 {
        let slots = state.direction_counts.get(&(zone_id, direction)).copied().unwrap_or(0);
        if direction != ReservationDirection::Inbound {
            return slots;
        }
        slots + self.0.navigation.pickup_units.count(zone_id, |hold| self.pickup_hold_is_open(hold))
    }

    fn pickup_hold_is_open(&self, hold: &CellReservation) -> bool {
        self.0
            .oracle
            .open_ended_reservation(hold.cell())
            .is_some_and(|live| live.id() == hold.id() && live.robot() == hold.robot())
    }

    /// Pickup units are checked against the oracle by exact identity, so a
    /// closed or replaced hold cannot keep an old credit alive.
    fn prune_pickup_units(&self, zone_id: ZoneId) {
        self.0.navigation.pickup_units.prune(zone_id, |hold| self.pickup_hold_is_open(hold));
    }

    fn can_admit_retrieve_with_reserve(&self, zone_id: ZoneId, protected: u32) -> bool {
        let limit = self.deadlock_headroom_limit(zone_id).saturating_sub(protected);
        self.0.state.with(|state| self.open_ended_count(zone_id, ReservationDirection::Inbound, state) < limit)
    }

    pub fn zone_can_admit_reservation(&self, zone_id: ZoneId, direction: ReservationDirection) -> bool {
        let limit = self.deadlock_headroom_limit(zone_id);
        self.0.state.with(|state| self.open_ended_count(zone_id, direction, state) < limit)
    }

    fn check_deadlock_headroom(
        &self,
        zone_id: ZoneId,
        direction: ReservationDirection,
        state: &State,
    ) -> Result<(), ReservationError> {
        let limit = self.deadlock_headroom_limit(zone_id);
        let count = self.open_ended_count(zone_id, direction, state);
        if count >= limit {
            Err(ReservationError::HeadroomExhausted {
                zone_id: zone_id.0,
                direction: match direction {
                    ReservationDirection::Inbound => "inbound",
                    ReservationDirection::Outbound => "outbound",
                },
                count,
                limit,
            })
        } else {
            Ok(())
        }
    }

    pub fn reserve_relay_for_retrieve(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
    ) -> Result<RelayReservation<ToteId>, ReservationError> {
        perf_span!("relay_retrieve");
        let direction = ReservationDirection::Inbound;
        self.0.state.update(|state| {
            let (sc, _) = self.select_relay_for_retrieve_locked(state, criteria, to, after)?;
            self.commit_reservation(state, sc, direction)
        })
    }

    pub fn select_relay_for_retrieve(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
    ) -> Result<RelayRetrieveSelection, ReservationError> {
        self.select_retrieve_with_reserve(criteria, to, after, None, 0)
    }

    pub fn select_relay_for_retrieve_near(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
        target: SimTime,
    ) -> Result<RelayRetrieveSelection, ReservationError> {
        self.select_retrieve_with_reserve(criteria, to, after, Some(target), 0)
    }

    fn select_retrieve_with_reserve(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
        target: Option<SimTime>,
        protected: u32,
    ) -> Result<RelayRetrieveSelection, ReservationError> {
        perf_span!("relay_retrieve_select");
        let (sc, completion_time) = self.0.state.with(|state| {
            self.select_relay_for_retrieve_locked_with_score(state, criteria, to, after, protected, |completion| {
                Some(target.map_or(completion, |target| SimTime::ms(completion.abs_diff(target).as_ms())))
            })
        })?;
        Ok(RelayRetrieveSelection {
            recommendation: sc.item,
            relay_id: sc.relay.id,
            zone_id: sc.relay.zone_id,
            start: sc.start,
            path_start_at: sc.path_start_at,
            estimated_htm_to_relay: sc.estimated_htm_to_relay,
            completion_time,
            slot_wait: sc.slot_wait,
            start_cause: sc.start_cause.expect("retrieve scored candidate carries a start cause"),
        })
    }

    pub fn reserve_selected_relay_for_retrieve(
        &self,
        selection: &RelayRetrieveSelection,
    ) -> Result<RelayReservation<ToteId>, ReservationError> {
        self.0.state.update(|state| self.reserve_selected_locked(state, selection))
    }

    fn reserve_selected_locked(
        &self,
        state: &mut State,
        selection: &RelayRetrieveSelection,
    ) -> Result<RelayReservation<ToteId>, ReservationError> {
        perf_span!("relay_retrieve_commit_selected");
        let direction = ReservationDirection::Inbound;
        let relay = *self
            .0
            .relays
            .get(&selection.relay_id)
            .ok_or(ReservationError::InvalidRelay { relay_id: selection.relay_id.0 })?;
        self.check_deadlock_headroom(relay.zone_id, direction, state)?;
        let Some(relay_ready) = state.relay_available(selection.relay_id) else {
            return Err(ReservationError::NoCandidates {
                operation: "retrieve",
                reason: "selected relay is no longer available",
            });
        };
        if relay_ready > selection.start {
            return Err(ReservationError::NoCandidates {
                operation: "retrieve",
                reason: "selected relay availability moved later than quoted start",
            });
        }
        self.commit_reservation(
            state,
            ScoredCandidate {
                relay,
                item: selection.recommendation,
                start: selection.start,
                path_start_at: selection.path_start_at,
                estimated_htm_to_relay: selection.estimated_htm_to_relay,
                slot_wait: selection.slot_wait,
                start_cause: Some(selection.start_cause),
            },
            direction,
        )
    }

    // Suspends until the family can mint a tentative starting at or
    // after `after` and landing at or after `arrival_floor`; `aim()` is
    // re-read per attempt and clamped up to the floor. "Not yet" is
    // awaited inside; only hard crafting errors return. The probe is
    // read-only — only a selection the caller will actually offer gets
    // registered, so a decline never writes the signal this wait watches.
    pub async fn source_retrieve(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
        arrival_floor: SimTime,
        aim: impl Fn() -> Option<SimTime>,
        aim_version: &Version,
        owner: ProposalId,
    ) -> Result<ProposedRetrieve, ReservationError> {
        self.source_retrieve_with_admission(criteria, to, after, arrival_floor, aim, aim_version, owner, None).await
    }

    /// Opt-in preparation admission. The predicate is sampled for each source
    /// attempt and again at acceptance; consumer-right changes and physical
    /// relay departures wake a suspended attempt.
    pub async fn source_retrieve_with_admission(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
        arrival_floor: SimTime,
        aim: impl Fn() -> Option<SimTime>,
        aim_version: &Version,
        owner: ProposalId,
        admission: Option<&RetrieveAdmission>,
    ) -> Result<ProposedRetrieve, ReservationError> {
        loop {
            let relay_changed = notified(&self.0.state);
            let oracle_seen = self.0.oracle.version().map(Version::seen);
            let admission_seen = admission.map(|policy| policy.version().seen());
            let inventory_seen = self.0.inventory.version().seen();
            let aim_seen = aim_version.seen();
            // Re-read per attempt: B may have advanced while this wait parked.
            let after = after.max(self.admission_floor());
            let target = match aim() {
                Some(aim) => Some(aim.max(arrival_floor)),
                None if arrival_floor > after => Some(arrival_floor),
                None => None,
            };
            let protected = admission.map_or(0, RetrieveAdmission::protected_slots);
            let selection = self.select_retrieve_with_reserve(criteria, to, after, target, protected);
            match selection {
                Ok(selection) if selection.completion_time >= arrival_floor => {
                    let mut proposed = self.register_retrieve(selection, owner);
                    proposed.admission = admission.cloned();
                    return Ok(proposed);
                }
                Ok(_) | Err(ReservationError::NoCandidates { .. } | ReservationError::HeadroomExhausted { .. }) => {
                    select! {
                        biased;
                        _ = relay_changed => {}
                        _ = self.0.inventory.version().past(inventory_seen) => {}
                        _ = aim_version.past(aim_seen) => {}
                        _ = async {
                            if let (Some(version), Some(seen)) = (self.0.oracle.version(), oracle_seen) {
                                version.past(seen).await;
                            } else {
                                pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let (Some(policy), Some(seen)) = (admission, admission_seen) {
                                policy.version().past(seen).await;
                            } else {
                                pending::<()>().await;
                            }
                        } => {}
                    }
                }
                Err(hard) => return Err(hard),
            }
        }
    }

    pub fn propose_open_retrieve(
        &self,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
        target: Option<SimTime>,
        owner: ProposalId,
    ) -> Result<ProposedRetrieve, ReservationError> {
        perf_span!("relay_retrieve_propose");
        let after = after.max(self.admission_floor());
        let selection = match target {
            Some(target) => self.select_relay_for_retrieve_near(criteria, to, after, target)?,
            None => self.select_relay_for_retrieve(criteria, to, after)?,
        };
        Ok(self.register_retrieve(selection, owner))
    }

    fn register_retrieve(&self, selection: RelayRetrieveSelection, owner: ProposalId) -> ProposedRetrieve {
        let poisoned = Arc::new(AtomicBool::new(false));
        self.0.state.update(|state| {
            self.prune_pickup_units(selection.zone_id);
            state.retrieve_tentatives.push(RetrieveTentative {
                owner,
                relay_id: selection.relay_id,
                tote: selection.recommendation,
                net_start: selection.start,
                poisoned: poisoned.clone(),
            });
        });
        ProposedRetrieve {
            service: self.clone(),
            owner,
            selection,
            poisoned,
            registered: true,
            accepted: None,
            admission: None,
        }
    }

    pub fn retrieve_fillers(&self, relay: RelayId) -> Vec<(ProposalId, SimTime)> {
        let mut fillers: Vec<(ProposalId, SimTime)> = self.0.state.with(|state| {
            state
                .retrieve_tentatives
                .iter()
                .filter(|t| t.relay_id == relay && !t.poisoned.load(Ordering::Relaxed))
                .map(|t| (t.owner, t.net_start))
                .collect()
        });
        fillers.sort_by_key(|&(owner, net_start)| (net_start, owner));
        fillers
    }

    pub fn retrieve_fillable_relays(&self) -> Vec<(RelayId, ZoneId, SimTime)> {
        self.0.state.with(|state| {
            state
                .timelines
                .iter()
                .filter_map(|(&relay_id, tl)| {
                    if tl.is_unfinalized() {
                        return None;
                    }
                    let has_filler = state
                        .retrieve_tentatives
                        .iter()
                        .any(|t| t.relay_id == relay_id && !t.poisoned.load(Ordering::Relaxed));
                    let zone = self.0.relays.get(&relay_id)?.zone_id;
                    has_filler.then_some((relay_id, zone, tl.available_at()))
                })
                .collect()
        })
    }

    // The retrieve claim beyond the relay itself: acceptance spends one unit
    // of the zone's inbound headroom, so forcing asks this before accepting.
    pub fn can_admit_retrieve(&self, zone_id: ZoneId) -> bool {
        self.zone_can_admit_reservation(zone_id, ReservationDirection::Inbound)
    }

    pub fn retrieve_tentative_count(&self) -> usize {
        self.0.state.with(|state| state.retrieve_tentatives.len())
    }

    pub fn relay_zone(&self, relay_id: RelayId) -> Option<ZoneId> {
        self.0.relays.get(&relay_id).map(|relay| relay.zone_id)
    }

    pub(crate) fn record_leak(&self, relay_id: RelayId, zone_id: ZoneId, start: SimTime) {
        if let Ok(mut leaked) = self.0.leaked.lock() {
            leaked.push((relay_id, zone_id, start));
        }
    }

    pub fn drain_leaked(&self) -> Vec<(RelayId, ZoneId, SimTime)> {
        self.0.leaked.lock().map(|mut leaked| std::mem::take(&mut *leaked)).unwrap_or_default()
    }

    // The sweeps set only the tentatives' poison flags; the scheduler
    // reads them generically through `Proposal::poisoned` and evicts at
    // its next crossing round.
    fn sweep_retrieve_tentatives_on_relay(state: &mut State, relay_id: RelayId) {
        for t in state.retrieve_tentatives.iter().filter(|t| t.relay_id == relay_id) {
            t.poisoned.store(true, Ordering::Relaxed);
        }
    }

    fn sweep_retrieve_tentatives_on_tote(state: &mut State, tote: ToteId) {
        for t in state.retrieve_tentatives.iter().filter(|t| t.tote == tote) {
            t.poisoned.store(true, Ordering::Relaxed);
        }
    }

    fn deregister_retrieve_tentative(&self, owner: ProposalId) {
        self.0.state.update(|state| state.retrieve_tentatives.retain(|t| t.owner != owner));
    }

    fn accept_retrieve(
        &self,
        owner: ProposalId,
        poisoned: &AtomicBool,
        selection: &RelayRetrieveSelection,
        protected: u32,
    ) -> Result<RelayReservation<ToteId>, AcceptError> {
        // Choice depth is read before acceptance mutates the pool: the
        // winner plus its live rivals on this relay, and the family-wide
        // tentative count at the same instant.
        let (depth, pool) = self.0.state.with(|state| {
            let depth = state
                .retrieve_tentatives
                .iter()
                .filter(|t| t.relay_id == selection.relay_id && !t.poisoned.load(Ordering::Relaxed))
                .count();
            (depth, state.retrieve_tentatives.len())
        });
        let reservation =
            self.0.state.update(|state| self.accept_retrieve_locked(state, owner, poisoned, selection, protected))?;
        if let Ok(mut stats) = self.0.accept_stats.lock() {
            stats.record_accept(depth, pool, selection.start);
        }
        Ok(reservation)
    }

    /// Turn the load-spreading probe on with its seed (see
    /// `PutawayConfig::random_slots`). Left off, the deposit selector ranks
    /// as it always has and no draw ever happens.
    pub fn set_random_deposit_slots(&self, seed: u64) {
        if let Ok(mut guard) = self.0.random_deposit.lock() {
            *guard = Some(Rng::new(seed));
        }
    }

    /// One ranking key per candidate relay, or `None` when the probe is off.
    ///
    /// Keys are drawn in relay-id order because the candidate list arrives
    /// in hash order: drawing straight down it would tie each key to an
    /// iteration order that shifts with the map's capacity and history, and
    /// the same seed would stop replaying the same run.
    fn random_deposit_keys(&self, candidates: &[DepositCandidate]) -> Option<FxHashMap<RelayId, u64>> {
        let mut guard = self.0.random_deposit.lock().ok()?;
        let rng = guard.as_mut()?;
        let mut ids: Vec<RelayId> = candidates.iter().map(|c| c.inner.relay.id).collect();
        ids.sort_unstable();
        Some(ids.into_iter().map(|id| (id, rng.next_u64())).collect())
    }

    /// Wire the scheduler's barrier clock: candidacy selections floor
    /// their `after` at the settled frontier. B is the ADMISSION
    /// frontier — accepted chains' tails may plan behind it, but no new
    /// candidacy may start there.
    pub fn set_admission_floor(&self, clock: Mut<SimTime>) {
        let _ = self.0.admission_floor.set(clock);
    }

    fn admission_floor(&self) -> SimTime {
        self.0.admission_floor.get().map_or(SimTime::ZERO, |clock| clock.value())
    }

    /// The retrieve family's end-of-run laziness audit; `None` when no
    /// acceptance happened.
    pub fn acceptance_report(&self) -> Option<String> {
        self.0.accept_stats.lock().ok().and_then(|stats| stats.report())
    }

    /// Record how much later than the relay slot's start a committed
    /// forward cycle lands its tote. Called once per forward commit.
    pub fn record_deposit_quote(&self, slip: SimDuration) {
        if let Ok(mut stats) = self.0.deposit_quotes.lock() {
            stats.record(slip);
        }
    }

    /// The forward family's end-of-run deposit-quote audit; `None` when no
    /// forward cycle committed.
    pub fn deposit_quote_report(&self) -> Option<String> {
        self.0.deposit_quotes.lock().ok().and_then(|stats| stats.report())
    }

    fn accept_retrieve_locked(
        &self,
        state: &mut State,
        owner: ProposalId,
        poisoned: &AtomicBool,
        selection: &RelayRetrieveSelection,
        protected: u32,
    ) -> Result<RelayReservation<ToteId>, AcceptError> {
        state.retrieve_tentatives.retain(|t| t.owner != owner);
        if poisoned.load(Ordering::Relaxed) {
            return Err(AcceptError::Poisoned {
                detail: format!("retrieve tentative {owner} poisoned before acceptance"),
            });
        }
        let relay = *self.0.relays.get(&selection.relay_id).ok_or_else(|| AcceptError::ActualConflict {
            detail: format!("retrieve at acceptance names unknown relay {:?}", selection.relay_id),
        })?;
        let limit = self.deadlock_headroom_limit(relay.zone_id).saturating_sub(protected);
        if self.open_ended_count(relay.zone_id, ReservationDirection::Inbound, state) >= limit {
            return Err(AcceptError::Poisoned {
                detail: "retrieve admission headroom changed before acceptance".into(),
            });
        }
        self.check_deadlock_headroom(relay.zone_id, ReservationDirection::Inbound, state).map_err(|e| {
            AcceptError::ActualConflict { detail: format!("headroom regressed under a valid tentative: {e}") }
        })?;
        let relay_ready = state.relay_available(selection.relay_id).ok_or_else(|| AcceptError::ActualConflict {
            detail: format!("relay {:?} under an open actual at acceptance with a valid tentative", selection.relay_id),
        })?;
        if relay_ready > selection.start {
            return Err(AcceptError::ActualConflict {
                detail: format!(
                    "relay {:?} frontier {} moved past the tentative start {} without a sweep",
                    selection.relay_id, relay_ready, selection.start
                ),
            });
        }

        let tote = self.0.inventory.tote(selection.recommendation).map_err(|e| AcceptError::ActualConflict {
            detail: format!("retrieve at acceptance tote lookup failed: {e}"),
        })?;
        match tote.location_at(selection.start) {
            ToteLocation::InRack { .. } | ToteLocation::Reserved { .. } => {}
            other => {
                return Err(AcceptError::ActualConflict {
                    detail: format!(
                        "retrieve at acceptance tote {:?} not in rack at {}: {:?}",
                        selection.recommendation, selection.start, other
                    ),
                });
            }
        }
        let reservation = self
            .commit_reservation(
                state,
                ScoredCandidate {
                    relay,
                    item: selection.recommendation,
                    start: selection.start,
                    path_start_at: selection.path_start_at,
                    estimated_htm_to_relay: selection.estimated_htm_to_relay,
                    slot_wait: selection.slot_wait,
                    start_cause: Some(selection.start_cause),
                },
                ReservationDirection::Inbound,
            )
            .map_err(|e| AcceptError::ActualConflict { detail: format!("slot write refused at acceptance: {e}") })?;
        Self::sweep_retrieve_tentatives_on_tote(state, selection.recommendation);
        Ok(reservation)
    }

    fn select_relay_for_retrieve_locked(
        &self,
        state: &State,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
    ) -> Result<(ScoredCandidate<ToteId>, SimTime), ReservationError> {
        self.select_relay_for_retrieve_locked_with_score(state, criteria, to, after, 0, Some)
    }

    fn select_relay_for_retrieve_locked_with_score<Score>(
        &self,
        state: &State,
        criteria: ToteSearchCriteria,
        to: CellId,
        after: SimTime,
        protected: u32,
        score_completion: Score,
    ) -> Result<(ScoredCandidate<ToteId>, SimTime), ReservationError>
    where
        Score: Fn(SimTime) -> Option<SimTime> + Sync,
    {
        let direction = ReservationDirection::Inbound;
        // The criteria names the racking: only a buffer release may read the
        // buffer, and nothing else may.
        let mut skip_zones = self.skip_zones(criteria.racking(), direction, state);
        if protected > 0 {
            skip_zones.extend(self.0.zone_capacity.keys().copied().filter(|&zone| {
                self.open_ended_count(zone, direction, state)
                    >= self.deadlock_headroom_limit(zone).saturating_sub(protected)
            }));
        }
        let candidates = self.0.inv_index.best_tote_per_relay(criteria, &skip_zones)?;
        for candidate in &candidates {
            for model in self.0.navigation.models() {
                self.0.relay_headings[&(model.id, candidate.relay.id)].as_ref().map_err(|error| error.clone())?;
            }
        }

        perf_span!("relay_retrieve_prescreen_refine");
        let oracle = self.0.oracle.as_ref();
        let htm_index = &self.0.htm_index;

        let vtm_floor_by_zone: HashMap<ZoneId, SimTime> = {
            let mut by_zone: HashMap<ZoneId, Option<SimTime>> = HashMap::new();
            for c in &candidates {
                by_zone.entry(c.relay.zone_id).or_insert_with(|| self.earliest_vtm_available(c.relay.zone_id));
            }

            by_zone.into_iter().filter_map(|(z, t)| t.map(|t| (z, t))).collect()
        };
        let approach_by_relay: HashMap<RelayId, _> = {
            let mut by_relay = HashMap::new();
            for c in &candidates {
                by_relay.entry(c.relay.id).or_insert_with(|| htm_index.approach(c.relay.id));
            }
            by_relay
        };

        let relay_available_by_id: HashMap<RelayId, SimTime> = {
            let mut by_relay: HashMap<RelayId, Option<SimTime>> = HashMap::new();
            for c in &candidates {
                by_relay.entry(c.relay.id).or_insert_with(|| state.relay_available(c.relay.id));
            }
            by_relay.into_iter().filter_map(|(id, t)| t.map(|t| (id, t))).collect()
        };

        let candidates = candidates.into_iter().filter_map(|c| {
            let relay_slot_avail = relay_available_by_id.get(&c.relay.id).copied()?;

            let non_slot_floor = after.max(c.available_at);
            let relay_ready_at = relay_slot_avail.max(non_slot_floor);

            let slot_wait = relay_slot_avail - non_slot_floor;

            let start_cause = if relay_slot_avail > non_slot_floor {
                RelayStartCause::Slot
            } else if after >= c.available_at {
                RelayStartCause::Demand
            } else {
                RelayStartCause::Tote
            };

            let vtm_floor = *vtm_floor_by_zone.get(&c.relay.zone_id)?;
            let vtm_deposit_est = (vtm_floor + c.vtm_cost).max(relay_ready_at);

            let approach =
                approach_by_relay.get(&c.relay.id).copied().expect("approach precomputed for every candidate relay");
            let estimated_htm_to_relay = approach.htm_to_relay;
            let path_start_at = if let Some(resident_ready_at) = approach.resident_ready_at {
                vtm_deposit_est.max(resident_ready_at)
            } else {
                let access_horizon = vtm_deposit_est + RETRIEVE_ACCESS_MAX_WAIT;
                oracle.entry_times(c.relay.cell_id, vtm_deposit_est, access_horizon, None).into_iter().next()?
            };
            Some(RetrieveCandidate {
                resident: approach.resident,
                relay_ready_at,
                path_start_at,
                estimated_htm_to_relay,
                slot_wait,
                start_cause,
                inner: c,
            })
        });

        let score =
            |c: &RetrieveCandidate, travel| score_completion(c.path_start_at + travel + c.estimated_htm_to_relay);
        let navigation = &self.0.navigation;
        let models = navigation.models();
        let candidates: Vec<_> = candidates.collect();
        for candidate in &candidates {
            if let Some((robot, _)) = candidate.resident {
                navigation.cache(robot).map_err(|e| InvalidScenario::RobotModels(e.to_string()))?;
            }
        }
        let selected = if models.len() == 1 {
            let model = &models[0];
            let cache =
                navigation.cache_for_model(model.id).map_err(|e| InvalidScenario::RobotModels(e.to_string()))?;
            NearestCellSelector::new(cache, &self.0.map).select_nearest_to(
                candidates,
                to,
                |candidate| {
                    candidate.resident.map(|(_, orientation)| orientation).unwrap_or_else(|| {
                        *self.0.relay_headings[&(model.id, candidate.inner.relay.id)]
                            .as_ref()
                            .expect("candidate heading validated")
                    })
                },
                true,
                |_| true,
                score,
            )
        } else {
            NearestCellSelector::select_all(candidates, to, score, |candidate| {
                let quote = |model_id, orientation| {
                    let cache = navigation
                        .cache_for_model(model_id)
                        .map_err(|e| PathSelectionError::ModelBinding(e.to_string()))?;
                    if candidate.cell() == to {
                        return Ok(SimDuration::ZERO);
                    }
                    Ok::<_, PathSelectionError>(cache.travel_time(PathCacheKey {
                        from: candidate.cell(),
                        from_orientation: orientation,
                        to,
                        carrying_cargo: true,
                    })?)
                };
                if let Some((robot, orientation)) = candidate.resident {
                    let model = navigation.model(robot).map_err(|e| PathSelectionError::ModelBinding(e.to_string()))?;
                    return quote(model.id, orientation);
                }
                let mut maximum = None;
                for model in models {
                    let orientation = *self.0.relay_headings[&(model.id, candidate.inner.relay.id)]
                        .as_ref()
                        .expect("candidate heading validated");
                    maximum = Some(maximum.unwrap_or(SimDuration::ZERO).max(quote(model.id, orientation)?));
                }
                maximum.ok_or_else(|| PathSelectionError::ModelBinding("no eligible floor robot models".into()))
            })
        }
        .err_context("retrieve", "all relays unfinalized or unreachable")?;

        let candidate = selected.candidate;

        let completion_time = candidate.path_start_at + selected.travel_time;
        let sc = ScoredCandidate {
            relay: candidate.inner.relay,
            item: candidate.inner.item,
            start: candidate.relay_ready_at,
            path_start_at: candidate.path_start_at,
            estimated_htm_to_relay: candidate.estimated_htm_to_relay,
            slot_wait: candidate.slot_wait,
            start_cause: Some(candidate.start_cause),
        };
        Ok((sc, completion_time))
    }

    // Selection probes read-only and relief runs outside the lock: a
    // declined reserve must not write the family signal (the caller's wait
    // would wake on its own probe), and relief re-enters foreign services.
    pub async fn reserve_relay_for_deposit(
        &self,
        from: CellId,
        from_orientation: Orientation,
        after: SimTime,
        self_robot: RobotId,
        parking_relief: Option<&ParkingRelief<'_>>,
    ) -> Result<RelayReservation<RackSlot>, ReservationError> {
        self.reserve_racking_for_deposit(RackUse::Inventory, from, from_orientation, after, self_robot, parking_relief)
            .await
    }

    /// Reserve a relay for a deposit into the ORDER BUFFER.
    ///
    /// Same scoring as [`Self::reserve_relay_for_deposit`], restricted to
    /// buffer racking. Separate rather than a parameter because the buffer has
    /// exactly one depositor -- a closed order on its way to dwell -- and every
    /// other caller means inventory racking and should keep saying so by not
    /// having to choose.
    pub async fn reserve_buffer_relay_for_deposit(
        &self,
        from: CellId,
        from_orientation: Orientation,
        after: SimTime,
        self_robot: RobotId,
        parking_relief: Option<&ParkingRelief<'_>>,
    ) -> Result<RelayReservation<RackSlot>, ReservationError> {
        self.reserve_racking_for_deposit(
            RackUse::OrderBuffer,
            from,
            from_orientation,
            after,
            self_robot,
            parking_relief,
        )
        .await
    }

    /// Reserve a relay for a deposit into the ROW OF EMPTY ORDER TOTES.
    pub async fn reserve_depot_relay_for_deposit(
        &self,
        from: CellId,
        from_orientation: Orientation,
        after: SimTime,
        self_robot: RobotId,
        parking_relief: Option<&ParkingRelief<'_>>,
    ) -> Result<RelayReservation<RackSlot>, ReservationError> {
        self.reserve_racking_for_deposit(RackUse::OrderDepot, from, from_orientation, after, self_robot, parking_relief)
            .await
    }

    /// One deposit path for racking of ONE KIND: the tmv2 locking and
    /// blocked-candidate relief, filtered to the steel the tote's pool owns.
    async fn reserve_racking_for_deposit(
        &self,
        uses: RackUse,
        from: CellId,
        from_orientation: Orientation,
        after: SimTime,
        self_robot: RobotId,
        parking_relief: Option<&ParkingRelief<'_>>,
    ) -> Result<RelayReservation<RackSlot>, ReservationError> {
        let (selected, blocked) = self
            .0
            .state
            .with(|state| self.select_deposit_locked(state, uses, from, from_orientation, after, self_robot));
        let sc = match selected {
            Ok(sc) => sc,
            Err(declined @ (ReservationError::NoCandidates { .. } | ReservationError::BeyondAccessHorizon { .. })) => {
                self.relieve_blocked_deposit(blocked, parking_relief, declined).await?
            }
            Err(err) => return Err(err),
        };
        let rack_slot_reservation = self.0.inv_index.reserve_slot(sc.item, sc.start)?;
        let mut reservation =
            self.0.state.update(|state| self.commit_reservation(state, sc, ReservationDirection::Outbound))?;
        reservation.rack_slot_reservation = Some(rack_slot_reservation);
        Ok(reservation)
    }

    /// Moves a holder of a `blocked` relay's access cell, or says why the
    /// deposit is declined: `declined`, the selection's own rejection, unless
    /// a caller with no relief is to move a holder itself.
    async fn relieve_blocked_deposit(
        &self,
        blocked: Vec<BlockedDepositCandidate>,
        parking_relief: Option<&ParkingRelief<'_>>,
        declined: ReservationError,
    ) -> Result<ScoredCandidate<RackSlot>, ReservationError> {
        // A caller with no relief moves a holder itself (a `Parking` task):
        // the decline names whom to move.
        let Some(relief) = parking_relief else {
            if blocked.is_empty() {
                return Err(declined);
            }
            let held = blocked.iter().map(|candidate| (candidate.relay.cell_id, candidate.blocker)).collect();
            return Err(ReservationError::AccessHeld {
                operation: "deposit",
                held,
                earliest_admitted_departure: declined.earliest_admitted_departure(),
            });
        };
        for b in blocked {
            let target = ParkingReliefTarget {
                relay_id: Some(b.relay.id),
                zone_id: Some(b.relay.zone_id),
                cell: b.relay.cell_id,
                robot: b.blocker,
                not_before: b.start,
            };
            if relief.park(target).await? {
                return Ok(ScoredCandidate {
                    relay: b.relay,
                    item: b.item,
                    start: b.start,
                    path_start_at: b.start,
                    estimated_htm_to_relay: SimDuration::ZERO,
                    slot_wait: b.slot_wait,
                    start_cause: None,
                });
            }
        }
        Err(declined)
    }

    fn select_deposit_locked(
        &self,
        state: &State,
        uses: RackUse,
        from: CellId,
        from_orientation: Orientation,
        after: SimTime,
        self_robot: RobotId,
    ) -> (Result<ScoredCandidate<RackSlot>, ReservationError>, Vec<BlockedDepositCandidate>) {
        perf_span!("relay_deposit");
        let direction = ReservationDirection::Outbound;
        let skip_zones = self.skip_zones(uses.racking(), direction, state);

        let candidates = match self.0.inv_index.best_empty_slot_per_relay(uses, &skip_zones) {
            Ok(candidates) => candidates,
            Err(e) => return (Err(e), Vec::new()),
        };

        let oracle = self.0.oracle.as_ref();
        let access_horizon = after + DEPOSIT_ACCESS_MAX_WAIT;
        let mut blocked: Vec<BlockedDepositCandidate> = Vec::new();

        perf_span!("relay_deposit_prescreen_refine");

        let relay_available_by_id: HashMap<RelayId, SimTime> = {
            let mut by_relay: HashMap<RelayId, Option<SimTime>> = HashMap::new();
            for c in &candidates {
                by_relay.entry(c.relay.id).or_insert_with(|| state.relay_available(c.relay.id));
            }
            by_relay.into_iter().filter_map(|(id, t)| t.map(|t| (id, t))).collect()
        };
        let mut candidates: Vec<DepositCandidate> = candidates
            .into_iter()
            .filter_map(|c| {
                let relay_ready = relay_available_by_id.get(&c.relay.id).copied()?;

                let slot_wait = relay_ready - after;
                Some(DepositCandidate {
                    relay_ready_at: relay_ready.max(after).max(c.available_at),
                    slot_wait,
                    inner: c,
                })
            })
            .collect();
        let opens_beyond_horizon =
            candidates.iter().map(|c| c.relay_ready_at).filter(|&opens| opens > access_horizon).min();
        // An open access hold has no entry time, so the selector's time
        // prescreen would discard it before its eligibility callback ran.
        // Retain those blockers separately so a declined deposit can move them.
        candidates.retain(|c| {
            if let CellStatus::PartiallyPlanned { robot } = oracle.cell_status(c.inner.relay.cell_id)
                && robot != self_robot
            {
                if c.relay_ready_at <= access_horizon {
                    blocked.push(BlockedDepositCandidate {
                        relay: c.inner.relay,
                        item: c.inner.item,
                        start: c.relay_ready_at,
                        blocker: robot,
                        slot_wait: c.slot_wait,
                    });
                }
                return false;
            }
            true
        });
        blocked.sort_by_key(|candidate| (candidate.start, candidate.relay.id));
        let random_keys = self.random_deposit_keys(&candidates);

        let cache = match self.0.navigation.cache(self_robot) {
            Ok(cache) => cache,
            Err(error) => return (Err(InvalidScenario::RobotModels(error.to_string()).into()), blocked),
        };
        let selected = NearestCellSelector::new(cache, &self.0.map)
            .select_nearest_from(
                candidates,
                from,
                from_orientation,
                true,
                |_| true,
                |c, htm_travel_time| {
                    let htm_arrival = after + htm_travel_time;
                    let earliest_access = htm_arrival.max(c.relay_ready_at);

                    let vtm_floor = self.earliest_vtm_available(c.inner.relay.zone_id)?;

                    let ranked = oracle
                        .entry_times(c.inner.relay.cell_id, earliest_access, access_horizon, None)
                        .into_iter()
                        .next()
                        .map(|access_at| access_at.max(vtm_floor) + c.inner.vtm_cost)?;
                    // Feasibility above decides WHETHER a relay can serve and
                    // is never skipped; the probe replaces only the key that
                    // decides WHICH of the feasible ones wins. Safe because
                    // the winner's score is discarded — the reservation below
                    // is built from the candidate's own times, not from this.
                    Some(match &random_keys {
                        Some(keys) => SimTime::ms(keys.get(&c.inner.relay.id).copied().unwrap_or(u64::MAX) >> 16),
                        None => ranked,
                    })
                },
            )
            .err_context("deposit", "all relays unfinalized, unreachable, or access cells held by other robots")
            .map_err(|error| match (error, opens_beyond_horizon) {
                (ReservationError::NoCandidates { .. }, Some(opens_at)) => ReservationError::BeyondAccessHorizon {
                    operation: "deposit",
                    depart: after,
                    horizon: DEPOSIT_ACCESS_MAX_WAIT,
                    opens_at,
                },
                (error, _) => error,
            });

        let selected = selected.map(|selected| {
            let candidate = selected.candidate;
            ScoredCandidate {
                relay: candidate.inner.relay,
                item: candidate.inner.item,
                start: candidate.relay_ready_at,
                path_start_at: candidate.relay_ready_at,
                estimated_htm_to_relay: SimDuration::ZERO,
                slot_wait: candidate.slot_wait,
                start_cause: None,
            }
        });
        (selected, blocked)
    }

    fn exhausted_zones(&self, direction: ReservationDirection, state: &State) -> FxHashSet<ZoneId> {
        self.0
            .zone_capacity
            .keys()
            .filter(|&&zid| self.check_deadlock_headroom(zid, direction, state).is_err())
            .copied()
            .collect()
    }

    /// Zones this reservation may not use: the ones with no headroom left,
    /// plus every zone whose racking is not the kind asked for.
    fn skip_zones(&self, racking: ZoneHolds, direction: ReservationDirection, state: &State) -> FxHashSet<ZoneId> {
        let mut skip = self.exhausted_zones(direction, state);
        let wrong_racking = match racking {
            ZoneHolds::Inventory => &self.0.buffer_zones,
            ZoneHolds::Order => &self.0.inventory_zones,
        };
        skip.extend(wrong_racking.iter().copied());
        skip
    }

    fn commit_reservation<R: Send + Sync + 'static>(
        &self,
        state: &mut State,
        sc: ScoredCandidate<R>,
        direction: ReservationDirection,
    ) -> Result<RelayReservation<R>, ReservationError> {
        let relay_id = sc.relay.id;
        let zone_id = sc.relay.zone_id;
        self.prune_pickup_units(zone_id);
        state
            .timelines
            .get_mut(&relay_id)
            .ok_or(ReservationError::InvalidRelay { relay_id: relay_id.0 })?
            .reserve(direction, sc.start);

        Self::sweep_retrieve_tentatives_on_relay(state, relay_id);
        state
            .zone_pressure
            .get_mut(&zone_id)
            .ok_or(ReservationError::InvalidZone { zone_id: zone_id.0 })?
            .add_open(sc.start);
        *state.direction_counts.entry((zone_id, direction)).or_insert(0) += 1;
        if direction == ReservationDirection::Inbound
            && self.open_ended_count(zone_id, direction, state) >= self.deadlock_headroom_limit(zone_id)
        {
            // A quote on another relay also depended on this headroom. Leaving
            // it live would pin admission before the accepted chains can drain.
            for tentative in &state.retrieve_tentatives {
                if self.0.relays[&tentative.relay_id].zone_id == zone_id {
                    tentative.poisoned.store(true, Ordering::Relaxed);
                }
            }
        }
        Ok(RelayReservation {
            recommendation: sc.item,
            relay_id,
            zone_id,
            start: sc.start,
            slot_wait: sc.slot_wait,
            start_cause: sc.start_cause,
            service: self.clone(),
            finalized: false,
            rack_slot_reservation: None,
        })
    }

    fn finalize_inner(
        &self,
        relay_id: RelayId,
        zone_id: ZoneId,
        end: SimTime,
        pickup_hold: Option<&CellReservation>,
    ) -> Result<(), ReservationError> {
        crate::perf_span!("relay_finalize");
        self.0.state.update(|state| {
            self.prune_pickup_units(zone_id);
            let tl =
                state.timelines.get_mut(&relay_id).ok_or(ReservationError::InvalidRelay { relay_id: relay_id.0 })?;
            let direction = tl.direction();
            if direction.is_none() {
                return Ok(());
            }
            if let Some(hold) = pickup_hold {
                // Finalization of tote custody and transfer of inbound admission
                // happen under the same service lock: no phantom capacity gap.
                assert_eq!(direction, Some(ReservationDirection::Inbound), "only retrievals transfer pickup headroom");
                assert_eq!(
                    self.htm_access_cell(relay_id),
                    Some(hold.cell()),
                    "pickup hold must belong to the relay access"
                );
                if self.pickup_hold_is_open(hold) {
                    self.0.navigation.pickup_units.record(zone_id, hold.clone());
                }
            }
            tl.finalize(end);
            let dir = direction.unwrap();
            let count = state.direction_counts.entry((zone_id, dir)).or_insert(0);
            *count = count.saturating_sub(1);
            state
                .zone_pressure
                .get_mut(&zone_id)
                .ok_or(ReservationError::InvalidZone { zone_id: zone_id.0 })?
                .close_open(end);
            Ok(())
        })
    }

    pub(super) fn cancel_inner(&self, relay_id: RelayId, zone_id: ZoneId, start: SimTime) {
        self.0.state.update(|state| {
            let tl = state
                .timelines
                .get_mut(&relay_id)
                .unwrap_or_else(|| panic!("cancel_inner: no timeline for {:?}", relay_id));
            let direction = tl.direction().unwrap_or_else(|| panic!("cancel_inner: {:?} not reserved", relay_id));
            tl.cancel(start);
            let count = state.direction_counts.entry((zone_id, direction)).or_insert(0);
            *count = count.saturating_sub(1);
            if let Some(zp) = state.zone_pressure.get_mut(&zone_id) {
                zp.cancel_open(start);
            }
        });
    }

    pub fn signal(&self) -> &dyn Signal {
        &self.0.state
    }

    pub fn unfinalized_count(&self) -> usize {
        self.0.state.with(|state| state.timelines.values().filter(|tl| tl.is_unfinalized()).count())
    }

    pub fn debug_relay_states(
        &self,
    ) -> (
        Vec<(RelayId, ZoneId, bool, SimTime, Option<ReservationDirection>)>,
        FxHashMap<(ZoneId, ReservationDirection), u32>,
    ) {
        self.0.state.with(|state| {
            let per_relay: Vec<_> = self
                .0
                .relays
                .iter()
                .map(|(&rid, relay)| {
                    let tl = state.timelines.get(&rid);
                    let unfinalized = tl.map(|t| t.is_unfinalized()).unwrap_or(false);
                    let avail = tl.map(|t| t.available_at()).unwrap_or(SimTime::ZERO);
                    let dir = tl.and_then(|t| t.direction());
                    (rid, relay.zone_id, unfinalized, avail, dir)
                })
                .collect();
            (per_relay, state.direction_counts.clone())
        })
    }
}
