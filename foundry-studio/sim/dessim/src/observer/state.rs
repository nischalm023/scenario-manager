//! Within `observer`, `state` is the ACCUMULATOR: the mutable working set
//! (`SimObserverState`) that `handlers` mutates per event — position maps,
//! per-slice scratch, pending attributions — plus the growing output. Holds
//! no capture logic itself; handlers.rs owns all mutation semantics.
//!
//! Separated from `SimObserver` to avoid borrow issues — the observer owns
//! the `mpsc::Receiver` while state is mutated by event handlers.

use rustc_hash::{FxHashMap, FxHashSet};

use desgraph::{CellId, RobotType, StationId, WorldPos};

use super::finality::Finality;
use super::finality::physical_inventory::{OccupancySlot, PhysicalReplay};
use crate::SimTime;
use crate::events::{TripEndpoint, TripPurpose};
use crate::model::ids::PresentationId;
use crate::model::{OrderId, RobotId, ToteLocation};
use crate::observer::activity::ActivityLog;
use crate::observer::duty::BusyInterval;
use crate::observer::output::{AisleStateRecord, MetricsOutput};
use crate::observer::robot_state::RobotStateLog;

use crate::observer::time_slice::{
    GridCounters, MetricsConfig, RelayStateRecord, TimeSliceData, ToteStateRecord, ToteVisualState,
};

/// Per-robot trip context captured at `TripStart` and read by `handle_nav` for
/// both distance-by-purpose attribution and the per-leg journal row.
#[derive(Clone, Copy)]
pub(super) struct TripContext {
    pub(super) purpose: TripPurpose,
    pub(super) presentation_id: Option<PresentationId>,
    pub(super) from: TripEndpoint,
    pub(super) to: TripEndpoint,
}

pub(super) struct SimObserverState {
    pub(super) config: MetricsConfig,
    pub(super) relay_access_cells: FxHashSet<CellId>,
    /// Every robot's reported milestones and the intervals built from them.
    pub(super) activity: ActivityLog,
    pub(super) output: MetricsOutput,
    pub(super) latest_sim_time: SimTime,
    pub(super) total_events: u64,
    /// The run's slices, which of them are final and sent, and what making
    /// the next ones final reads.
    pub(super) finality: Finality,
    /// Last known position per bot (cell_id), carried across slices.
    pub(super) bot_positions: FxHashMap<u32, u32>,
    /// Last known world position per bot (mm), updated as nav events arrive.
    pub(super) bot_world_pos: FxHashMap<u32, WorldPos>,
    /// End of each robot's latest committed navigation, independent of display backfill.
    pub(super) navigation_end_ms: FxHashMap<u32, u64>,
    /// Robot type per bot (seeded from InitialPositions).
    pub(super) bot_types: FxHashMap<u32, RobotType>,
    /// Running set of reserved/occupied relay cell IDs (carried across slices).
    pub(super) reserved_relay_cells: FxHashSet<u32>,
    /// What each slice has added to the cell counters, keyed by slice index.
    ///
    /// It is held HERE rather than on the slices themselves because booking a
    /// visit must never bring a slice into existence: a waypoint of no
    /// duration sitting exactly on a boundary would otherwise create the
    /// slice after the run's last, and that slice's scalars are snapshots
    /// taken when it is created — a rack fill, a relay count — so the run
    /// would report numbers master never took. A slice's share folds into it
    /// when the slice is made final (`finality`); a share booked past the
    /// run's last slice belongs to no slice.
    pub(super) slice_grids: FxHashMap<usize, GridCounters>,
    /// Each tote's drawing position as the physical replay has reached it,
    /// in simulation-time order. The replay is its only writer.
    pub(super) tote_states: FxHashMap<u32, ToteVisualState>,
    /// Authoritative inventory custody, reconstructed in simulation-time order.
    /// Presence overrides drawing positions for rack occupancy.
    pub(super) tote_custody: FxHashMap<u32, ToteLocation>,
    /// Snapshot of initial bot positions from `InitialPositions`, from which
    /// `drawable` places a robot in a slice before it has first moved.
    /// Each entry: (robot_id, cell_id, world_pos, robot_type, heading_rad).
    pub(super) initial_bot_state: Vec<(u32, u32, WorldPos, RobotType, Option<f32>)>,
    /// Current trip context per robot: purpose (for distance-by-purpose
    /// attribution) plus the journal fields (Trip ID + semantic endpoints)
    /// stamped on each per-nav-event journal row in `handle_nav`.
    pub(super) current_trip: FxHashMap<RobotId, TripContext>,
    /// Each robot's previous leg destination. A leg's semantic origin is the
    /// prior leg's destination — the bot doesn't teleport between legs — so
    /// this supplies the journal `from` (e.g. a re-pickup reads as Relay→Relay,
    /// matching the workflow, instead of a hardcoded Parking sentinel). The
    /// event's declared origin seeds only a robot's first leg.
    pub(super) last_trip_to: FxHashMap<RobotId, TripEndpoint>,
    /// Physical payload ledger reconstructed in simulation-time order, including
    /// picked units in order containers. Legacy InventoryDelta-only producers
    /// retain their per-tote delta fallback.
    pub(super) tote_units: FxHashMap<u32, i64>,
    pub(super) physical_replay: PhysicalReplay,
    /// Laden-tote count per physical rack slot. Normally 0 or 1;
    /// briefly 2 if a new tote is recorded in a slot before the previous
    /// occupant's departure event lands. A slot counts as filled while this
    /// is ≥ 1, so that transient double-book can never push fill above
    /// capacity. Entries at 0 are removed.
    pub(super) slot_laden: FxHashMap<OccupancySlot, u32>,
    /// Running count of rack slots holding a laden tote (= `occupancy_total`).
    pub(super) rack_laden_slots: u32,
    /// Per-aisle counterpart of `rack_laden_slots`: `ZoneId.0` → filled slots.
    pub(super) laden_slots_by_zone: FxHashMap<u32, u32>,
    /// Last aisle (relay zone) each tote was *settled* in — its most recent
    /// location that maps to an aisle. Transit hops (`WithRobot`) map to no
    /// aisle and are skipped, so a tote that leaves aisle 3, rides a robot,
    /// and returns to aisle 3 registers no crossing. Aisle flux is logged only
    /// when this changes. See `handle_inventory_move`.
    pub(super) last_settled_aisle: FxHashMap<u32, u32>,
    /// Physical tote count per rack slot. Refcounted so a transient duplicate
    /// record still counts one occupied slot.
    pub(super) slot_occupied: FxHashMap<OccupancySlot, u32>,
    pub(super) occupied_slots_by_zone: FxHashMap<u32, u32>,
    /// Physical tote count per diving-board cell. Reservation state is tracked
    /// separately in `reserved_relay_cells` and is not used by this metric.
    pub(super) relay_totes: FxHashMap<u32, u32>,
    pub(super) occupied_relays_by_zone: FxHashMap<u32, u32>,
    /// Last emitted per-aisle snapshot. Keeps change detection O(1) instead
    /// of rescanning the growing output log on every tote transition.
    pub(super) last_aisle_state: FxHashMap<u32, AisleStateRecord>,
    /// Running warehouse inventory unit count, accumulated from every
    /// `SimEvent::InventoryDelta` (bridged from `InventoryEvent::ContentsChanged`).
    /// Initial seeding at T=0 fires deltas through the same channel, so this
    /// arrives at the true initial total by the time slice 0 closes.
    pub(super) inventory_units: u64,
    /// Order → station mapping (from OrderDispatch events, for pallet counting).
    pub(super) order_station: FxHashMap<OrderId, StationId>,
    /// Order → DC mapping learned from its pick completion facts. Presence
    /// with `None` means a known pre-formed order, distinct from missing data.
    pub(super) order_dc: FxHashMap<OrderId, Option<String>>,
    /// Geometric busy windows from diverged VtmNav legs (Teleport only; see
    /// `record_vtm_leg`). Consumed by `finalize_vtm_union_duty`; empty in
    /// every other mode.
    pub(super) vtm_geo_intervals: FxHashMap<RobotId, Vec<BusyInterval>>,
    /// What the facts read so far commit each robot to, until the slices they
    /// reach are stated in the robot state channel.
    pub(super) robot_state_log: RobotStateLog,
}

impl SimObserverState {
    pub(super) fn new(config: MetricsConfig) -> Self {
        Self {
            relay_access_cells: config.relay_htm_access_cells.values().copied().map(CellId).collect(),
            finality: Finality::new(&config),
            config,
            activity: ActivityLog::default(),
            output: MetricsOutput::default(),
            latest_sim_time: SimTime::ZERO,
            total_events: 0,
            bot_positions: FxHashMap::default(),
            bot_world_pos: FxHashMap::default(),
            navigation_end_ms: FxHashMap::default(),
            bot_types: FxHashMap::default(),
            reserved_relay_cells: FxHashSet::default(),
            slice_grids: FxHashMap::default(),
            tote_states: FxHashMap::default(),
            tote_custody: FxHashMap::default(),
            initial_bot_state: Vec::new(),
            current_trip: FxHashMap::default(),
            last_trip_to: FxHashMap::default(),
            tote_units: FxHashMap::default(),
            physical_replay: PhysicalReplay::default(),
            slot_laden: FxHashMap::default(),
            rack_laden_slots: 0,
            laden_slots_by_zone: FxHashMap::default(),
            last_settled_aisle: FxHashMap::default(),
            slot_occupied: FxHashMap::default(),
            occupied_slots_by_zone: FxHashMap::default(),
            relay_totes: FxHashMap::default(),
            occupied_relays_by_zone: FxHashMap::default(),
            last_aisle_state: FxHashMap::default(),
            inventory_units: 0,
            order_station: FxHashMap::default(),
            order_dc: FxHashMap::default(),
            vtm_geo_intervals: FxHashMap::default(),
            robot_state_log: RobotStateLog::default(),
        }
    }

    /// Make slices exist up to and including `idx`, writing into none that
    /// already does. A fact writes through `slice_to_write` instead.
    pub(super) fn extend_slices_through(&mut self, idx: usize) {
        while self.slices().len() <= idx {
            let start_ms = self.slices().len() as u64 * self.config.slice_width_ms;
            let relay_start: Vec<u32> = self.reserved_relay_cells.iter().copied().collect();
            let mut slice = TimeSliceData {
                start_ms,
                relay_state: RelayStateRecord { start_reserved: relay_start, transitions: Vec::new() },
                // Stated on slice 0 only; later slices carry the ids forward by
                // not restating them (see `ToteStateRecord::order_totes`).
                tote_state: ToteStateRecord {
                    creations: Vec::new(),
                    contents: Vec::new(),
                    transits: Vec::new(),
                    start_states: Vec::new(),
                    carries_previous: !self.slices().is_empty(),
                    transitions: Vec::new(),
                    order_totes: Vec::new(),
                },
                ..Default::default()
            };
            // A new slice's distribution waits for B to pass its start, the
            // instant it samples (`activity`).
            // Seed rack fill + inventory units from the running snapshot so
            // slices with no events still report the current totals.
            slice.scalars.rack.occupancy_total = self.rack_laden_slots;
            slice.scalars.rack.occupancy_peak = self.rack_laden_slots;
            // Capacity is a constant scenario fact (the fill-% denominator);
            // stamp it on every slice so the chart reads occupancy and
            // capacity from the same per-slice map.
            slice.scalars.rack.capacity_total = self.config.total_rack_slots;
            slice.scalars.rack.per_zone_occupancy = self.laden_slots_by_zone.iter().map(|(&z, &n)| (z, n)).collect();
            slice.scalars.inventory_units = self.inventory_units;
            // Relay occupancy: relays currently holding a deposited tote
            // (reserved htm-access cells) at slice start, over the constant
            // total relay count. Same snapshot the `relay_state` overlay reads.
            slice.scalars.relay_cells_reserved = self.reserved_relay_cells.len() as u32;
            slice.scalars.relay_cells_total = self.config.total_relays;
            self.finality.push_slice(slice);
        }
    }
}

#[cfg(test)]
mod sparse_tote_tests {
    use super::*;
    use crate::events::{SimEvent, ToteInitialStatesEvent, ToteMovedEvent};
    use crate::model::ToteId;

    #[test]
    fn six_hour_idle_history_stores_initial_inventory_once() {
        let mut state = SimObserverState::new(MetricsConfig { slice_width_ms: 10_000, ..Default::default() });
        let initial: Vec<_> = (0..162_060).map(|id| (id, ToteVisualState::InRack { cell_id: id, level: 0 })).collect();
        state.dispatch_event(&SimEvent::ToteInitialStates(ToteInitialStatesEvent {
            totes: initial.clone(),
            order_totes: Vec::new(),
        }));
        state.extend_slices_through(2160);
        for index in 0..state.slices().len() {
            state.make_drawable(index);
        }
        assert_eq!(state.slices().len(), 2161);
        let entries: usize = state.slices().iter().map(|s| s.tote_state.start_states.len()).sum();
        assert_eq!(entries, initial.len(), "unchanged warehouse state must not multiply by elapsed slices");
        assert_eq!(
            ToteStateRecord::positions_at(state.slices().iter().map(|s| (s.start_ms, &s.tote_state)), 21_600_000),
            initial
        );
        eprintln!(
            "[memory] tote history entries={entries}, bytes={}, old repeated-snapshot bytes={}",
            entries * size_of::<(u32, ToteVisualState)>(),
            entries * state.slices().len() * size_of::<(u32, ToteVisualState)>()
        );
    }

    #[test]
    fn late_location_revision_changes_only_its_slice_and_replays_through_idle_gaps() {
        let mut state = SimObserverState::new(MetricsConfig { slice_width_ms: 100, ..Default::default() });
        let rack = ToteVisualState::InRack { cell_id: 10, level: 0 };
        let bot = ToteVisualState::OnBot { bot_id: 7 };
        state.dispatch_event(&SimEvent::ToteInitialStates(ToteInitialStatesEvent {
            totes: vec![(1, rack.clone())],
            order_totes: Vec::new(),
        }));
        for (at, position) in [(500, rack.clone()), (100, bot.clone())] {
            state.dispatch_event(&SimEvent::ToteMoved(ToteMovedEvent {
                tote_id: ToteId(1),
                new_state: position,
                time: SimTime::ms(at),
            }));
        }
        for index in 0..state.slices().len() {
            state.make_drawable(index);
        }
        for (cursor, expected) in [(99, rack.clone()), (100, bot.clone()), (450, bot), (500, rack)] {
            assert_eq!(
                ToteStateRecord::positions_at(state.slices().iter().map(|s| (s.start_ms, &s.tote_state)), cursor),
                [(1, expected)]
            );
        }
        assert!(state.slices()[1..].iter().all(|s| s.tote_state.start_states.is_empty()));
    }
}
