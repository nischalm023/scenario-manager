//! Within `observer`, `time_slice` is the LEGACY per-slice surface: the
//! fixed-width slice records, scalar counters, and grid layers desvis's
//! time-series views read today.
//!
//! TODO(legacy): the string-keyed `SliceRecord.metrics` map is dynamic but
//! not self-describing; it migrates to descriptor-carrying metric streams
//! under issues #281/#300.
//!
//! Each time slice covers a configurable window (default 5 minutes of sim time).
//! [`GridCounters`] tracks per-cell 2D heatmap data, [`ScalarCounters`] tracks
//! aggregate scalar metrics, and [`TimeSliceData`] bundles both for one slice.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use desgraph::{CellId, CellKind, CircularArc, RelayId, RobotType, StationId, StationType, WorldPos, ZoneId};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::events::VtmIdleReasonKind;
use crate::model::{RobotId, SimDuration, SimTime, ToteDesignation, ToteType};
use crate::observer::activity::ActivityKind;
use crate::observer::duty::DutyBreakdown;
use crate::observer::station_diag::StationDiagRecord;
use crate::observer::traffic::MotionSpan;
use crate::robots::runtime::FloorRobotModel;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the metrics time-slice bucketing.
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    /// Width of each time slice in milliseconds.
    pub slice_width_ms: u64,
    /// The simtime the run is limited to: it stops once its planned frontier reaches it, and a run stopped so ends
    /// there. A slice ending past it is made final only as the stream closes, since only then is it known whether the
    /// run reaches that end. `None` for a run with no limit.
    pub sim_time_limit: Option<SimTime>,
    /// Robot travel speed in mm/s (for nav invariant checks). 0.0 disables checks.
    pub speed_mm_per_sec: f64,
    /// Nominal cell spacing in mm (for adjacency + diagonal invariant checks). 0.0 disables checks.
    pub cell_spacing_mm: f64,
    /// Nav congestion factor (>= 1.0). Waypoint time_delta_sec values are already scaled by
    /// this factor, so the effective speed limit is speed_mm_per_sec / nav_factor.
    pub nav_factor: f64,
    /// Station type map for station-type-aware throughput bucketing.
    pub station_types: FxHashMap<StationId, StationType>,
    /// Each station's presentation cell, where a station visit holds the bot.
    pub station_cells: FxHashMap<StationId, CellId>,
    /// Zone id per relay. Lets the observer stamp each journey-leg journal row
    /// with the zone of its relay endpoint(s) (the `From/To Relay Zone` columns),
    /// so relay ids in the HTM/VTM Journey Legs sheets carry their zone the same
    /// way `VtmCycleRecord.zone_id` already does. Relay→zone is static topology,
    /// so a lookup table built once at observer construction suffices. Empty in
    /// tests that don't exercise relays (zone columns come out blank).
    pub relay_zones: FxHashMap<RelayId, u32>,
    /// Physical diving-board cell (`htm_access`) → aisle (`ZoneId.0`). Used
    /// for tote-at-relay occupancy, which is distinct from reservation state,
    /// and for each zone's share of the relays' busy time.
    pub relay_cell_zone: Arc<FxHashMap<u32, u32>>,
    /// Physical diving-board capacity per aisle.
    pub relay_zone_capacity: Arc<BTreeMap<u32, u32>>,
    /// Pick embargo lead-in, in milliseconds (`PickConfig::embargo_secs`).
    /// Picking is suspended before this instant, so any Pick cycle completing
    /// earlier is an embargo violation — counted into
    /// `ScalarCounters::pre_embargo_picks` and excluded from throughput. 0
    /// disables the check (no embargo configured).
    pub pick_embargo_ms: u64,
    /// World position per cell, indexed by `CellId.0`. Populated from the map at
    /// observer construction. Used by `handle_nav` to resolve the world position
    /// of each pathfinder `Waypoint`. Empty means tests that don't care about
    /// positions (will get `WorldPos::default()` for every cell).
    pub cell_positions: Arc<Vec<WorldPos>>,
    /// Directed curved links, mirrored from immutable map geometry for playback.
    pub link_curves: Arc<FxHashMap<(CellId, CellId), CircularArc>>,
    /// Directed HTM links traversed with the chassis facing opposite travel.
    pub htm_reverse_links: Arc<FxHashSet<(CellId, CellId)>>,
    /// Immutable models keyed by the same physical RobotId as navigation
    /// events. A missing binding is an invariant violation, never a selector
    /// or kinematics fallback to run settings.
    pub htm_models: Arc<FxHashMap<RobotId, Arc<FloorRobotModel>>>,
    /// Sample period for trapezoidal rasterization, in milliseconds. Smaller =
    /// smoother visualization, more memory. Default 100ms. Phase 4 reads this
    /// at the rasterization site; harmless under `ConstantSpeed` because
    /// rasterization is gated off entirely.
    pub motion_step_ms: u32,
    /// When `true`, append every recorded `BusyInterval` to
    /// `MetricsOutput::robot_timelines` (the per-bot raw timeline). When
    /// `false` (the default), only aggregate `DutyBreakdown` totals are
    /// retained. Off by default because timelines grow with VTM count ×
    /// cycles and bloat the serialized profile; enable in desvis sessions
    /// where you want the per-bot Gantt strip.
    pub record_robot_timelines: bool,
    /// Total tote storage capacity across all rack zones. Computed at
    /// solver build time as `sum over relay zones of (storage cell
    /// count in zone × zone.vertical_tote_capacity)`. Zero when the
    /// scenario has no rack (degenerate / test maps); the rack fill %
    /// chart renders 0% in that case. Emitted per slice as the
    /// `rack_capacity` scalar (the fill-% denominator), so live and
    /// playback read the same value.
    pub total_rack_slots: u32,
    /// Rack cell (`CellId.0`) → aisle (`ZoneId.0`). Inverted from the
    /// scenario's `zone_storage`. Attributes `ToteMoved` rack in/out events
    /// to an aisle for per-aisle occupancy.
    pub rack_cell_zone: Arc<FxHashMap<u32, u32>>,
    /// Rack (`RackId.0`) → aisle (`ZoneId.0`). Composed from `rack_cells`
    /// (rack → cell) and `zone_storage` (cell → zone). Attributes
    /// inventory-change events (which carry a `RackId`) to an aisle.
    pub rack_id_zone: Arc<FxHashMap<u32, u32>>,
    /// Per-aisle rack tote-slot capacity: `ZoneId.0` → slots. Surfaced on
    /// `MetricsOutput` for per-aisle fill in the xlsx export.
    pub zone_capacity: Arc<BTreeMap<u32, u32>>,
    /// Total number of relays in the scenario (one HTM-access cell each).
    /// The denominator for the per-slice relay-occupancy % chart: a relay
    /// counts as occupied while a deposited tote sits in it awaiting HTM
    /// pickup (`reserved_relay_cells`). Constant scenario fact, stamped on
    /// every slice like `total_rack_slots`. Zero on relay-less maps → the
    /// occupancy chart renders 0%.
    pub total_relays: u32,
    /// RelayId.0 → HTM-access cell id. Lets the resource-diagnostics handler
    /// key `VtmIdleReason` / `HtmRelayPickup` records by the relay's clicked
    /// cell (matching `RelayStateChangeEvent`'s cell keying) without adding a
    /// cell field to those events. Empty in tests that don't inspect relays.
    pub relay_htm_access_cells: FxHashMap<u32, u32>,
    /// Map cell kind per cell, indexed by `CellId.0` — the graph's own
    /// `CellKind`, mirrored once at observer construction exactly as
    /// `cell_positions` mirrors world positions. This is the observer's only
    /// source of cell MEMBERSHIP, and #272's waiting attribution reads it to
    /// say which kind of cell a hold left the D30 standing on (parking,
    /// presentation, station queue, relay, other). `None` entries are cell-id
    /// slots the graph has no cell for; an empty table (tests, mapless runs)
    /// makes every hold position `Unknown` rather than a guess.
    pub cell_kinds: Arc<Vec<Option<CellKind>>>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            slice_width_ms: 10_000, // 10 seconds
            sim_time_limit: None,
            speed_mm_per_sec: 0.0,
            cell_spacing_mm: 0.0,
            nav_factor: 1.0,
            station_types: FxHashMap::default(),
            station_cells: FxHashMap::default(),
            relay_zones: FxHashMap::default(),
            relay_cell_zone: Arc::new(FxHashMap::default()),
            relay_zone_capacity: Arc::new(BTreeMap::new()),
            pick_embargo_ms: 0,
            cell_positions: Arc::new(Vec::new()),
            link_curves: Arc::new(FxHashMap::default()),
            htm_reverse_links: Arc::new(FxHashSet::default()),
            htm_models: Arc::new(FxHashMap::default()),
            motion_step_ms: 100,
            record_robot_timelines: false,
            total_rack_slots: 0,
            rack_cell_zone: Arc::new(FxHashMap::default()),
            rack_id_zone: Arc::new(FxHashMap::default()),
            zone_capacity: Arc::new(BTreeMap::new()),
            total_relays: 0,
            relay_htm_access_cells: FxHashMap::default(),
            cell_kinds: Arc::new(Vec::new()),
        }
    }
}

impl MetricsConfig {
    pub fn htm_model(&self, robot: RobotId) -> Arc<FloorRobotModel> {
        self.htm_models.get(&robot).cloned().unwrap_or_else(|| panic!("observer has no motion model for HTM {robot:?}"))
    }

    /// Resolve a cell's world position via the configured table. Returns
    /// `WorldPos::default()` if the table is empty (tests) or the cell is
    /// out of bounds.
    pub fn cell_pos(&self, cell_id: u32) -> WorldPos {
        self.cell_positions.get(cell_id as usize).copied().unwrap_or_default()
    }

    /// How much simtime each slice spans.
    pub fn slice_width(&self) -> SimDuration {
        SimDuration::ms(self.slice_width_ms)
    }
}

/// Whether B, standing at `barrier`, has passed the start of slice `index`,
/// slices being `width` wide. Strictly past: a fact stamped at B is still
/// allowed, and it falls in the slice that starts at its stamp or, for work
/// that ends then, in the slice that ends there, so B standing on a boundary
/// has passed neither slice.
pub fn passed_slice_start(barrier: SimTime, index: usize, width: SimDuration) -> bool {
    barrier > SimTime::ZERO + width * index as u64
}

/// Whether B, standing at `barrier`, has passed the end of slice `index`,
/// which is the start of the slice after it.
pub fn passed_slice_end(barrier: SimTime, index: usize, width: SimDuration) -> bool {
    passed_slice_start(barrier, index + 1, width)
}

// ---------------------------------------------------------------------------
// 2D Grid Counters (per time slice, per cell)
// ---------------------------------------------------------------------------

/// Per-cell counters for a single time slice.
///
/// Uses `HashMap<u32, u32>` (CellId.0 → count) for sparse storage.
/// Only cells that were actually visited have entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GridCounters {
    /// VTM cell traversal counts.
    pub nav_visits_vtm: FxHashMap<u32, u32>,
    /// HTM cell traversal counts.
    pub nav_visits_htm: FxHashMap<u32, u32>,
    /// VTM robot-seconds of presence at each cell (stored as milliseconds for integer math).
    pub robot_presence_vtm_ms: FxHashMap<u32, u64>,
    /// HTM robot-seconds of presence at each cell (stored as milliseconds for integer math).
    pub robot_presence_htm_ms: FxHashMap<u32, u64>,
}

// ---------------------------------------------------------------------------
// RelayStateRecord (per-slice relay reservation state)
// ---------------------------------------------------------------------------

/// Per-slice relay reservation state, following the same pattern as [`BotLocation`].
///
/// Stores the set of reserved relay cell IDs at slice start, plus intra-slice
/// transitions (reserve/release events with timing). During playback, the UI
/// reconstructs the reserved set at any point by starting from `start_reserved`
/// and applying transitions up to the desired offset.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelayStateRecord {
    /// Cell IDs (htm_access.0) of relays that are reserved/occupied at slice start.
    pub start_reserved: Vec<u32>,
    /// Intra-slice transitions: (cell_id, offset_ms from slice start, reserved).
    /// `reserved=true` means the cell became reserved; `false` means freed.
    pub transitions: Vec<(u32, u64, bool)>,
}

impl RelayStateRecord {
    pub fn is_empty(&self) -> bool {
        self.start_reserved.is_empty() && self.transitions.is_empty()
    }
}

// ---------------------------------------------------------------------------
// ToteVisualState + ToteStateRecord (per-slice tote positions)
// ---------------------------------------------------------------------------

/// Visual state of a single tote for rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToteVisualState {
    /// Container at an induction or presentation cell.
    AtCell { cell_id: u32 },
    /// Container has left the simulated system and is no longer drawn.
    Extracted,
    /// Tote sitting in a rack at a specific cell and shelf level.
    InRack { cell_id: u32, level: u16 },
    /// Tote sitting at a relay location (on the floor).
    AtRelay { cell_id: u32 },
    /// Tote being carried by a bot (position inferred from BotLocation).
    OnBot { bot_id: u32 },
    /// Tote on no robot, at its last conveyor landmark. Transit records supply
    /// timed movement between landmarks; without a transit it stays here.
    OnConveyor { cell_id: u32 },
}

/// A container's birth and immutable metadata, stated only in its birth slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToteCreationRecord {
    pub offset_ms: u64,
    pub tote_id: u32,
    #[serde(default)]
    pub tote_type: ToteType,
    pub designation: ToteDesignation,
}

/// Timed visual travel between conveyance cells, using absolute simulation
/// timestamps so every overlapping slice can independently locate the tote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToteTransitRecord {
    pub tote_id: u32,
    pub from_cell: u32,
    pub to_cell: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Per-slice tote state, following the same pattern as [`BotLocation`] and
/// [`RelayStateRecord`].
///
/// New recordings store the initial positions once and carry them forward
/// through timed transitions. Older profiles contain a full checkpoint per
/// slice; `carries_previous` distinguishes the two without losing playback.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToteStateRecord {
    #[serde(default)]
    pub creations: Vec<ToteCreationRecord>,
    /// Physical contents changes: (offset_ms, tote_id, absolute units).
    /// Absent from older profiles, whose unknown contents remain undrawn.
    #[serde(default)]
    pub contents: Vec<(u64, u32, u64)>,
    #[serde(default)]
    pub transits: Vec<ToteTransitRecord>,
    /// (tote_id, visual_state) for every tote at slice start.
    pub start_states: Vec<(u32, ToteVisualState)>,
    /// Intra-slice transitions: (offset_ms from slice start, tote_id, new_state).
    pub transitions: Vec<(u64, u32, ToteVisualState)>,
    /// Ids of the totes designated ORDER, on slice 0 only.
    ///
    /// The designation is permanent per instance, so it is stated once and
    /// read for the whole run rather than repeated per slice. Present so the
    /// renderer can tell the two flows apart on sight (Vinay, 2026-08-31:
    /// "should we use a different color for order totes? Will be useful to
    /// observe traffic movement between the 2 flows").
    #[serde(default)]
    pub order_totes: Vec<u32>,
    /// Continue the preceding slice instead of replacing it with a checkpoint.
    /// Missing in legacy profiles, including intentionally empty checkpoints.
    #[serde(default)]
    pub carries_previous: bool,
}

impl ToteStateRecord {
    /// Reconstruct where each tote stands at `cursor_ms` from chronological
    /// slices: the last checkpoint at or before the cursor, then each later
    /// slice's transitions. Restarting from the checkpoint also makes reverse
    /// scrubbing independent of the previous cursor.
    pub fn positions_at<'a>(
        slices: impl IntoIterator<Item = (u64, &'a Self)>,
        cursor_ms: u64,
    ) -> Vec<(u32, ToteVisualState)> {
        let slices: Vec<_> = slices.into_iter().take_while(|(start, _)| *start <= cursor_ms).collect();
        let checkpoint = slices.iter().rposition(|(_, record)| !record.carries_previous).unwrap_or(0);
        let mut states = FxHashMap::default();
        for &(start, record) in &slices[checkpoint..] {
            if !record.carries_previous {
                states.clear();
            }
            states.extend(record.start_states.iter().cloned());
            for &(offset, id, ref state) in &record.transitions {
                if offset <= cursor_ms - start {
                    states.insert(id, state.clone());
                }
            }
        }
        let mut states: Vec<_> = states.into_iter().collect();
        states.sort_unstable_by_key(|&(id, _)| id);
        states
    }

    pub fn is_empty(&self) -> bool {
        !self.carries_previous
            && self.order_totes.is_empty()
            && self.start_states.is_empty()
            && self.transitions.is_empty()
            && self.creations.is_empty()
            && self.contents.is_empty()
            && self.transits.is_empty()
    }
}

// ---------------------------------------------------------------------------
// BotLocation (per-bot location record within a time slice)
// ---------------------------------------------------------------------------

/// What kind of action a `BotStep` represents. The renderer uses this to
/// decide whether to interpolate position over `delta_ms` or hold in place.
///
/// `Move` and `Reverse` lerp from the prior step's position to this step's position.
/// `Wait` and `Rotate` keep the bot at this step's position for the full
/// `delta_ms`. The observer also synthesizes `Wait` steps to represent
/// dwell gaps between Nav events (e.g., a bot sitting still after an
/// `HtmStore` finishes and before `ParkIdleHtms` dispatches it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BotStepKind {
    Move,
    Reverse,
    Wait,
    Rotate,
}

/// A single movement step within a bot's path in one time slice.
///
/// Nothing deserializes one from an older shape any more — the container that
/// carried them is gone and a `.worldview` states every field — so the fields
/// have no serde defaults. What remains is the shape the observer builds and
/// the recording carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotStep {
    /// Destination cell ID.
    pub cell_id: u32,
    /// Where the bot really ends up, in world millimetres on the floor plane:
    /// the destination cell's centre for a straight leg, and a point on the
    /// arc for a leg the planner curved or the kinematics rasterised.
    pub pos: WorldPos,
    /// How high the bot is above the world's root frame at the end of this
    /// step, in millimetres. See [`standing_height_mm`] for where it comes
    /// from and for what the simulator does not model.
    pub z_mm: f64,
    /// Duration of this step in milliseconds.
    pub delta_ms: u64,
    /// Action kind. `Move`/`Reverse` lerp `prev_pos -> pos`; `Wait`/`Rotate` hold at `pos`.
    pub kind: BotStepKind,
}

/// How high above the world's root frame a robot of this class stands, in
/// millimetres.
///
/// It exists so that every recorded height comes from one place. The simulator
/// models no vertical MOTION for any robot: a VTM's lift is a duration inside
/// its cycle time (`robots::vtm::motion`) and the rack level it reaches is a
/// slot index on its operation records, never a position — every nav event a
/// recorded step is built from carries a floor-plane `WorldPos` and nothing
/// else. So a robot's height is the height of the surface it stands on, and
/// the loader reads only the planar part of a document pose, so the run model
/// holds no elevation for a cell to take it from. Zero, therefore, until the
/// VTM model computes a height — and then here, not in the writer.
pub fn standing_height_mm(_robot_type: RobotType) -> f64 {
    0.0
}

impl BotStep {
    /// A step at the height its robot stands at, which is every step the
    /// simulator produces.
    pub fn whole(cell_id: u32, pos: WorldPos, delta_ms: u64, kind: BotStepKind) -> Self {
        Self { cell_id, pos, z_mm: 0.0, delta_ms, kind }
    }
}

/// Per-bot location record within a single time slice.
///
/// Stores the bot's position at slice start and all movements within the slice.
/// Nav events that span slice boundaries are split: waypoints before the
/// boundary go in the current slice, the remainder starts the next slice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotLocation {
    /// Canonical motion for traffic measurement. None denotes an older recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Vec<MotionSpan>>,
    /// Cell the bot occupied at the start of this slice.
    pub start_cell: u32,
    /// World position of start_cell. Zero if unknown.
    pub start_pos: WorldPos,
    /// How high the bot is above the world's root frame as the slice opens, in
    /// millimetres. See [`standing_height_mm`].
    #[serde(default)]
    pub start_z_mm: f64,
    /// The cell the bot is travelling to as the slice opens, when the slice
    /// before it cut the leg part way. `None` says the bot is standing in
    /// `start_cell` rather than between two cells — which is what every bot
    /// that was action-ready at the boundary is doing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_toward: Option<u32>,
    /// Chassis heading at `start_pos`, when explicitly known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_heading_rad: Option<f32>,
    /// Robot type (Vtm, Htm, Rtp).
    pub robot_type: RobotType,
    /// Waypoint steps within this slice.
    /// Empty means the bot didn't move during this slice.
    pub steps: SmallVec<[BotStep; 4]>,
}

// ---------------------------------------------------------------------------
// Scalar Counters (per time slice)
// ---------------------------------------------------------------------------

/// Aggregate scalar counters for a single time slice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScalarCounters {
    /// Total picks completed in this slice.
    pub picks_completed: u64,
    /// Picks completed before the pick embargo lifts (`sim_time_ms <
    /// MetricsConfig::pick_embargo_ms`). Should always be 0 — the dispatch path
    /// floors every admission at the embargo — so a nonzero value flags a real
    /// embargo-enforcement bug. Subtracted from the throughput numerator so a
    /// violation is visible here instead of silently inflating picks/hr
    /// (greymatter-foundry#208).
    pub pre_embargo_picks: u64,
    /// Total putaway items deposited in this slice (qty-weighted, mirror
    /// of `picks_completed` for the putaway side).
    pub putaways_completed: u64,
    /// Inventory totes returned from decant whose VTM rack deposit has ended.
    pub putaway_totes_stored: u64,
    /// Per-station pick counts. Populated only for Pick cycles — Putaway
    /// station ids never appear as keys.
    pub picks_per_station: FxHashMap<StationId, u64>,
    /// All cycle presentations. One per CycleComplete.
    pub presentations: u64,
    /// Pick-kind presentations only.
    pub pick_presentations: u64,
    /// Putaway-kind presentations only.
    pub putaway_presentations: u64,
    /// Order-tote loads at a pick station.
    pub order_load_presentations: u64,
    /// Order-tote emptyings at an extraction station.
    pub order_extract_presentations: u64,
    /// Units removed from order containers at completed extraction.
    pub order_extracted_units: u64,
    /// Orders fully completed in this slice.
    pub orders_completed: u64,
    /// How many VTMs were inside an interval of each kind, or in a gap, at
    /// the slice's start.
    pub vtm_activity_counts: FxHashMap<ActivityKind, u32>,
    /// How many HTMs were inside an interval of each kind, or in a gap, at
    /// the slice's start.
    pub htm_activity_counts: FxHashMap<ActivityKind, u32>,
    /// Backpressure events.
    pub backpressure_events: u64,
    /// Tote claim successes.
    pub tote_claim_successes: u64,
    /// Tote claim failures.
    pub tote_claim_failures: u64,
    /// Relay reservation successes.
    pub relay_reservation_successes: u64,
    /// Relay reservation failures.
    pub relay_reservation_failures: u64,
    /// Pathfinder queries.
    pub pathfinder_queries: u64,
    /// Pathfinder failures.
    pub pathfinder_failures: u64,
    /// VTM forward matches (rack-to-relay assignments).
    pub vtm_forward_matches: u64,
    /// VTM return matches (relay-to-rack assignments).
    pub vtm_return_matches: u64,
    /// VTM cross-zone assignments.
    pub vtm_cross_zone_assignments: u64,
    /// Cumulative VTM scoring duration in nanoseconds.
    pub vtm_scoring_duration_ns: u64,
    /// Anti-starvation age bonus applications.
    pub starvation_age_bonus_applied: u64,
    /// Deadlock headroom override triggers.
    pub deadlock_headroom_triggered: u64,
    /// Wall-clock duration of this slice in milliseconds.
    ///
    /// Measured by the SimObserver as the elapsed wall time between sealing
    /// this slice and the previous one. Useful for identifying sim phases where
    /// the engine is slower than real-time.
    pub wall_clock_duration_ms: f64,

    // -- Travel distance --
    /// Cumulative VTM travel distance in millimeters.
    pub vtm_travel_distance_mm: u64,
    /// Cumulative HTM travel distance in millimeters.
    pub htm_travel_distance_mm: u64,

    // -- Trip completion counts --
    /// VTM forward cycles completed (rack→relay).
    pub vtm_forward_trips: u64,
    /// VTM return cycles completed (relay→rack).
    pub vtm_return_trips: u64,
    /// HTM pickup cycles completed (relay→station→picks).
    pub htm_pickup_trips: u64,
    /// HTM return cycles completed (→return relay).
    pub htm_return_trips: u64,

    // -- Cycle durations (cumulative; average = total / trip_count) --
    /// Cumulative VTM forward cycle time in ms.
    pub vtm_forward_cycle_ms: u64,
    /// Cumulative VTM return cycle time in ms.
    pub vtm_return_cycle_ms: u64,
    /// Cumulative HTM full cycle time in ms.
    pub htm_cycle_ms: u64,
    /// Cumulative HTM operator wait time in ms.
    pub htm_operator_wait_ms: u64,
    /// Number of completed operator waits (denominator for htm_operator_wait_ms average).
    pub htm_operator_wait_count: u64,

    // -- Round trip distance --
    /// Cumulative HTM round-trip distance in millimeters (dispatch→relay→station→return relay).
    pub htm_round_trip_distance_mm: f64,
    /// Cumulative HTM full round-trip time in milliseconds (dispatch→relay→station→picks→return relay).
    pub htm_round_trip_ms: u64,

    // -- Congestion (promoted from atomics to time-series) --
    /// HTM return relay retries.
    pub htm_return_relay_retries: u64,
    /// Operator queue waits.
    pub operator_queue_waits: u64,

    // -- Metrics V2: trip distance by purpose --
    #[serde(default)]
    pub trip_distance: TripDistanceCounters,

    // -- Metrics V2: rack occupancy --
    #[serde(default)]
    pub rack: RackCounters,

    /// Total inventory units across all totes at slice end. Snapshot,
    /// not a sum — populated from the observer's running ledger
    /// (`InventoryDelta` events).
    #[serde(default)]
    pub inventory_units: u64,

    /// Relays holding a deposited tote awaiting HTM pickup at slice start
    /// (`reserved_relay_cells.len()`). Instantaneous snapshot, not a sum.
    /// Numerator for the relay-occupancy % chart; the denominator is
    /// `relay_cells_total`.
    #[serde(default)]
    pub relay_cells_reserved: u32,
    /// Total relay count (`MetricsConfig::total_relays`). Constant scenario
    /// fact stamped on every slice so the chart reads numerator and
    /// denominator from the same per-slice map (mirrors `rack.capacity_total`).
    #[serde(default)]
    pub relay_cells_total: u32,
    /// Relay-access *reservation* busy-ms attributed to this slice, summed
    /// over all relay HTM-access cells (navigation's holds, not the physical
    /// tote-dwell of `relay_cells_reserved`), read from the holds navigation
    /// emits as the slice is made final (`finality::relay_busy`). Divide by
    /// `relay_cells_total * slice_width_ms` for the per-slice reservation
    /// utilization %, which integrates to the `relay_utilization_pct` summary.
    #[serde(default)]
    pub relay_reserved_busy_ms: u64,
    /// `relay_reserved_busy_ms` per relay zone: every relay zone, an idle one
    /// as zero. Each zone's relay-access lane is read back from these
    /// (`metrics::profile::relay_utilization_timeline`).
    #[serde(default)]
    pub relay_reserved_busy_per_zone: FxHashMap<ZoneId, SimDuration>,

    // -- Metrics V2: station-type throughput --
    #[serde(default)]
    pub throughput: ThroughputCounters,

    // -- Metrics V2: pipeline snapshot --
    #[serde(default)]
    pub pipeline: PipelineCounters,

    /// Sum of HTM dwell-at-relay durations on inbound pickup, in ms — time
    /// the HTM spent at the relay waiting for the tote to be deposited.
    /// Driven by `HtmRelayPickup` events emitted from HtmPickup.
    /// Average = `htm_wait_at_relay_ms / htm_relay_pickup_count`.
    #[serde(default)]
    pub htm_wait_at_relay_ms: u64,
    /// Count of HtmPickup commits (denominator for average wait).
    #[serde(default)]
    pub htm_relay_pickup_count: u64,

    /// Sum of pure diving-board slot-wait durations, in ms — the portion of
    /// each committed relay reservation's start that was delayed specifically
    /// by the slot still being occupied by the previous tote (KPI row 24).
    /// Distinct from `htm_wait_at_relay_ms` (HTM dwell at the relay). Driven
    /// by `DivingBoardSlotWait` events. Average =
    /// `diving_board_slot_wait_ms / diving_board_presentation_count`.
    #[serde(default)]
    pub diving_board_slot_wait_ms: u64,
    /// Count of committed relay reservations (denominator for average slot
    /// wait).
    #[serde(default)]
    pub diving_board_presentation_count: u64,

    /// Per-robot duty-cycle totals attributed to this slice.
    ///
    /// Keyed by `RobotId.0`. Each `DutyBreakdown` sums the durations of
    /// `BusyInterval`s overlapping this slice, clipped to the slice
    /// boundary. Idle time is implicit (`slice_width − busy`). Empty for
    /// slices in which no robot was active.
    #[serde(default)]
    pub duty_per_robot: FxHashMap<u32, DutyBreakdown>,

    /// Count of `VtmIdleReasonEvent`s by classification, within this slice.
    /// One event is emitted per VTM cycle commit; the kind tells you
    /// whether the VTM rolled straight in (`Continuous`) or which floor
    /// pinned the next cycle later than it was free (`AwaitingDemand` /
    /// `AwaitingTote` / `AwaitingSlot` / `AwaitingHtmDeposit`).
    #[serde(default)]
    pub vtm_idle_reason_counts: FxHashMap<VtmIdleReasonKind, u64>,

    // -- Fleet duty aggregates (sum across all robots of this type, in ms).
    // Dividing by `slice_width_ms` yields "average number of robots in this
    // kind of interval at any moment within the slice" — the time-weighted
    // analog of the instantaneous `*_activity_counts`. Drives the line graphs.
    /// Total VTM-milliseconds spent in travel within this slice.
    pub vtm_duty_traveling_ms: u64,
    /// Total VTM-milliseconds spent in fork operations within this slice.
    pub vtm_duty_loading_unloading_ms: u64,
    /// Total HTM-milliseconds spent in travel within this slice.
    pub htm_duty_traveling_ms: u64,
    /// HTM-milliseconds spent rotating in place within this slice.
    ///
    /// Invariant: this is a *carve-out subset* of `htm_duty_traveling_ms`,
    /// not a mutually-exclusive duty state. Rotation time is sourced from
    /// the authoritative `WaypointAction::Rotate` windows and is always
    /// also counted in `htm_duty_traveling_ms` — so `htm_duty_rotating_ms
    /// <= htm_duty_traveling_ms` holds per slice. There is no VTM analog:
    /// VTMs model 2-axis linear motion only and never rotate.
    pub htm_duty_rotating_ms: u64,
    /// Total HTM-milliseconds spent in fork operations within this slice.
    pub htm_duty_loading_unloading_ms: u64,
    /// Total HTM-milliseconds spent in station visits within this slice.
    pub htm_duty_processing_ms: u64,

    // -- Loaded-time KPI overlays (D30). Each is an additive *sub-slice* of a
    // coarse bucket above, NOT a repartition: they overlap the travel and
    // station-visit buckets and must never be folded into
    // `DutyBreakdown::busy()` or the HtmIdle complement. They are independent
    // "% of sim time" series and do not sum to total loaded time (loaded
    // productive travel is the unsurfaced residual in `htm_duty_traveling_ms`).
    /// KPI #4 Wait Loaded — HTM has a tote but isn't presenting yet (inbound):
    /// loaded `Wait` waypoints on the to-station leg (corridor blocking,
    /// ⊂ travel). The loaded stand at the pickup relay after the fork-Get
    /// is a gap between tasks, not part of it.
    pub htm_duty_wait_loaded_ms: u64,
    /// KPI #5 Dwell Loaded — HTM holding a tote, blocked on the return
    /// (outbound): loaded `Wait` waypoints on the to-return-relay leg,
    /// including the opening one that holds the HTM on the presentation cell
    /// until its corridor clears. The station visit itself never reaches
    /// here: a bot standing on the presentation cell between its dwell and
    /// its departure is still in its visit.
    pub htm_duty_dwell_loaded_ms: u64,
    /// KPI #6 Pick-at-station — operator dwell at **pick** stations
    /// (Σ `CycleComplete.dwell_ms`, ⊂ station visits). Putaway dwell is
    /// excluded.
    pub htm_duty_pick_at_station_ms: u64,
}

pub use super::counters_v2::{PipelineCounters, RackCounters, ThroughputCounters, TripDistanceCounters};

#[cfg(test)]
mod bot_step_tests {
    use super::{BotLocation, BotStep, BotStepKind};
    use desgraph::WorldPos;

    /// A step states every one of its fields.
    ///
    /// It used to default its kind and its fractions so a recording written
    /// before they existed would still load. Nothing reads such a recording
    /// now — the container that carried them is gone and a `.worldview`
    /// states them all — so a step missing one is a shape nobody wrote, and
    /// taking it as a whole-leg Move would draw a robot along a leg it never
    /// travelled.
    #[test]
    fn a_step_missing_a_field_is_refused_rather_than_defaulted() {
        let short = r#"{
            "start_cell": 1,
            "start_pos": {"x_mm": 0.0, "y_mm": 0.0},
            "robot_type": "Htm",
            "steps": [{"cell_id": 2, "pos": {"x_mm": 1000.0, "y_mm": 0.0}, "delta_ms": 1000}]
        }"#;
        assert!(serde_json::from_str::<BotLocation>(short).is_err(), "a step with no kind names no motion");

        let whole = BotStep::whole(2, WorldPos { x_mm: 1_000.0, y_mm: 0.0 }, 1_000, BotStepKind::Move);
        let back: BotStep = serde_json::from_str(&serde_json::to_string(&whole).unwrap()).unwrap();
        assert_eq!(back.kind, BotStepKind::Move);
        assert_eq!(back.z_mm, 0.0, "a step states the height its robot stands at");
    }
}

// ---------------------------------------------------------------------------
// TimeSliceData (grid + scalar for one slice)
// ---------------------------------------------------------------------------

/// Complete data for a single time slice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeSliceData {
    /// Start time of this slice in simulation milliseconds.
    pub start_ms: u64,
    /// Scalar counters for this slice.
    pub scalars: ScalarCounters,
    /// What this slice alone added to each cell's counters. Every visit is
    /// booked to the slice it starts in, so the slices sum to
    /// `MetricsOutput::cumulative_grid` exactly.
    #[serde(default)]
    pub grid: GridCounters,
    /// Per-bot location data for this slice. Key = RobotId.0.
    /// Only bots that moved have entries (lazy insertion).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub bot_locations: FxHashMap<u32, BotLocation>,
    /// Relay reservation state for this slice (start set + intra-slice transitions).
    #[serde(default, skip_serializing_if = "RelayStateRecord::is_empty")]
    pub relay_state: RelayStateRecord,
    /// Tote positions for this slice (all totes at start + intra-slice moves).
    #[serde(default, skip_serializing_if = "ToteStateRecord::is_empty")]
    pub tote_state: ToteStateRecord,
    /// Station dispatch diagnostics for this slice (HTM assignments/arrivals,
    /// dispatch rejections, pipeline depths). Drives the desvis station
    /// inspector. Defaulted so pre-existing profiles still load.
    #[serde(default, skip_serializing_if = "StationDiagRecord::is_empty")]
    pub station_diag: StationDiagRecord,
}
