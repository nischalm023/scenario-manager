//! Within `observer`, `output` is WHAT THE RUN PRODUCED: the typed fact
//! records (pick/putaway/leg/cycle/operation/aisle journals) and the final
//! `MetricsOutput` bundle handed to summaries, profiles, and reports.
//! Records are facts — they carry no metric semantics; builders in
//! `metrics::reporting` interpret them. Named log residence is temporary
//! pending issue #300's tenant migration into container stream frames.
//!
//! [`MetricsOutput`] is the final result returned when the metrics thread joins.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use desgraph::{RelayId, RobotType, StationId, StationType};

use crate::events::{TotePayloadState, ToteSource, VtmCycleDirection, VtmOperationKind};
use serde::{Deserialize, Serialize};

use crate::events::{StripePlanEvent, VtmIdleReasonKind};
use crate::model::ids::{RackId, SkuId};
use crate::model::relay::{RelayUtilizationSummary, RelayUtilizationTimeline};
use crate::model::{OrderId, RobotId, SimDuration, SimTime};
use crate::observer::duty::{BusyInterval, DutyBreakdown, StationBusyInterval};
use crate::scenario_gen::demand_pool::DistributionCenterId;

use super::digest::EventDigestSummary;
use super::time_slice::{GridCounters, ScalarCounters, TimeSliceData};

// ---------------------------------------------------------------------------
// Final output (returned when metrics thread joins)
// ---------------------------------------------------------------------------

/// Complete metrics output produced when the simulation completes.
///
/// Returned by `SimObserver::run()` after the event channel closes.
/// Contains all time slices, cumulative counters, and the full time-series.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsOutput {
    /// Maximum simulation timestamp observed while processing events.
    pub sim_elapsed_ms: u64,
    /// Per-time-slice data (grid + scalar counters).
    pub time_slices: Vec<TimeSliceData>,
    /// Cumulative grid counters (sum across all slices).
    pub cumulative_grid: GridCounters,
    /// The run's counters as its final slices state them (`finality::run_totals`).
    pub cumulative_scalars: ScalarCounters,
    /// Total number of events processed by the metrics service.
    pub total_events: u64,
    /// Event-stream digest (counts always; FNV hash under `DES_DIGEST=1`).
    /// The A/B identity line for transformation-PR gates — see
    /// [`crate::observer::digest`].
    pub event_digest: EventDigestSummary,
    /// Total number of time slices produced.
    pub total_slices: usize,
    /// Per-station inter-presentation gap samples in milliseconds.
    ///
    /// Computed at finalization from physical dwell intervals sorted by
    /// simulated time. Each sample spans the previous busy end to the next
    /// non-overlapping start; back-to-back work contributes zero. Overlapping
    /// work extends the busy interval and contributes no sample. The initial
    /// idle period is excluded. Used for station flow smoothness.
    pub station_gaps_ms: HashMap<StationId, Vec<u64>>,

    /// Cumulative operator-busy time per station in milliseconds.
    ///
    /// Sum of every `CycleCompleteEvent.dwell_ms` accumulated per `StationId`.
    /// Station utilization = `station_busy_ms / sim_elapsed_ms`. Both pick
    /// and putaway stations are counted; consumers can resolve type via
    /// `MetricsConfig::station_types`.
    pub station_busy_ms: HashMap<StationId, u64>,

    /// Cumulative duty-cycle breakdown per robot.
    ///
    /// Sum of every `BusyInterval` recorded for the robot, bucketed by
    /// kind. A gap is implicit (`sim_elapsed − busy`).
    pub robot_duty_totals: HashMap<RobotId, DutyBreakdown>,

    /// The simtime each robot's first interval of work starts at.
    /// Used as the lower bound when computing "active-window" duty.
    pub robot_first_seen: HashMap<RobotId, SimTime>,

    /// The simtime each robot's latest interval of work ends at.
    /// Used as the upper bound when computing "active-window" duty.
    pub robot_last_seen: HashMap<RobotId, SimTime>,

    /// Raw per-bot timeline of `BusyInterval`s. Populated only when
    /// `MetricsConfig::record_robot_timelines` is true; empty otherwise.
    /// Drives per-bot Gantt rendering in desvis.
    pub robot_timelines: HashMap<RobotId, Vec<BusyInterval>>,

    /// Cumulative `VtmIdleReasonEvent` count by classification across the
    /// whole run. One increment per VTM cycle commit.
    pub vtm_idle_reason_counts: HashMap<VtmIdleReasonKind, u64>,

    /// Robot type per robot, seeded from `InitialPositions`. Lets
    /// consumers filter `robot_duty_totals` / `robot_timelines` by VTM/HTM
    /// without keeping a parallel map.
    pub robot_types: HashMap<RobotId, RobotType>,

    /// Per-station operator-busy intervals. One entry per
    /// `CycleCompleteEvent`: `[sim_time_ms, sim_time_ms + dwell_ms)`.
    /// Idle is implicit — gaps between intervals are operator-cell idle.
    /// Always recorded (storage is small even at long horizons; no
    /// `record_robot_timelines`-style gate).
    pub station_timelines: HashMap<StationId, Vec<StationBusyInterval>>,

    /// Station type per station. Populated lazily on the first
    /// `CycleCompleteEvent` for each station from `MetricsConfig::station_types`.
    /// Lets the Gantt renderer color lanes by pick vs putaway.
    pub station_types: HashMap<StationId, StationType>,

    /// Earliest simulated dwell start recorded for each station, independent
    /// of event emission order.
    /// Used as the lower bound when computing "active-window" utilization
    /// so a station that bootstraps late doesn't get penalized for the
    /// warmup period it had no work to do.
    pub station_first_seen: HashMap<StationId, SimTime>,

    /// The destination stripe-plan snapshot log, ordered by emission time:
    /// one entry per stripe (re)computation — exactly one (t=0) under the
    /// static v1 map, empty when the scenario has no bulk demand pool. The
    /// stripe state at time T is the last snapshot with `sim_time_ms <= T`.
    pub stripe_plan_log: Vec<StripePlanEvent>,

    /// One row per pick presentation, in event order. Drives the "Picks"
    /// xlsx sheet — one HTM presentation at a pick station = one entry.
    pub pick_log: Vec<PickRecord>,

    /// One row per distinct completed order, attributed to its pick station
    /// and DC when those facts are available.
    pub completed_order_log: Vec<CompletedOrderRecord>,

    /// One row per putaway presentation, in event order.
    pub putaway_log: Vec<PutawayRecord>,
    /// Completed order-container extraction work, before stream encoding.
    pub extraction_log: Vec<ExtractionRecord>,

    /// One journey-leg journal row per HTM nav event under a known trip. Source
    /// for the "HTM Journey Legs" sheet. Stored at maximum granularity (one row
    /// per nav event) so per-leg aggregates are derived downstream without loss.
    /// Phase 1: Travel rows only. HTM-only.
    pub htm_leg_log: Vec<HtmLegRecord>,

    /// One row per committed VTM cycle (forward or return). Source for the
    /// "VTM Cycles" xlsx sheet.
    pub vtm_cycle_log: Vec<VtmCycleRecord>,

    /// One immutable fact per DA50 pickup/drop boundary.
    pub vtm_operation_log: Vec<VtmOperationRecord>,

    /// Per-aisle storage and physical diving-board state changes. A row is a
    /// point-in-time snapshot that remains effective until the next row for
    /// the same aisle. This is independent of relay reservation state.
    pub aisle_state_log: Vec<AisleStateRecord>,

    /// One journey-leg journal row per VTM micro-phase: a Travel row per
    /// `RailMove`, a Mechanical row per `Fork`, and a Wait row per non-
    /// `Continuous` pre-cycle idle gap. Source for the "VTM Journey Legs"
    /// sheet; the VTM analog of `htm_leg_log`. Wait rows are pushed when the
    /// idle event arrives (at cycle start), so the xlsx sorts by `start_ms`.
    pub vtm_leg_log: Vec<VtmLegRecord>,

    /// One row per aisle crossing: a tote carried `delta` units of a SKU into
    /// (+) or out of (−) an aisle (relay zone), in event order. Source for the
    /// "Inventory Delta" xlsx sheet. A tote moving between two racks in the
    /// same aisle, or an empty tote, produces no row.
    pub aisle_flux_log: Vec<AisleFluxRecord>,

    /// Per-aisle rack tote-slot capacity: `ZoneId.0` → slots. Mirrored from
    /// `MetricsConfig::zone_capacity` at observer finish; the denominator for
    /// per-aisle fill % in the "Rack" xlsx sheet. Empty on rackless scenarios.
    pub rack_zone_capacity: BTreeMap<u32, u32>,
    /// Relay-access utilization (overall + per-zone), injected by the solver
    /// after the run from the oracle — the same numbers as the headless
    /// `relay_utilization_pct` line. `None` on profiles predating this field
    /// or when no relay data exists. Feeds the desvis relay-utilization
    /// charts; the per-slice occupancy gantt divides slice reserved-cell
    /// counts by `total_relays`.
    #[serde(default)]
    pub relay_utilization: Option<RelayUtilizationSummary>,
    /// Per-zone × per-slice diving-board **slot** occupancy — the fraction of
    /// each slice a relay's buffer slot was held (reserved), an upper bound on
    /// physical tote dwell and a resource distinct from the access cell, whose
    /// busy time every slice states. Injected by the solver; drives the desvis
    /// diving-board occupancy heat lanes. `None` on a rackless scenario.
    #[serde(default)]
    pub relay_slot_occupancy_timeline: Option<RelayUtilizationTimeline>,
}

/// One aisle crossing: tote `tote_id` carried `delta` units of `sku` into (+)
/// or out of (−) aisle (relay zone) `zone_id` at `sim_time_ms`. A tote leaving
/// one aisle and entering another produces two records (a `−` for the aisle it
/// left, a `+` for the aisle it entered). Renders as one row of the "Inventory
/// Delta" xlsx sheet.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AisleFluxRecord {
    pub sim_time_ms: u64,
    /// Aisle (relay zone) the tote crossed into (+) or out of (−).
    pub zone_id: u32,
    /// The tote that carried the units across the aisle boundary.
    pub tote_id: u32,
    pub sku: SkuId,
    /// Signed unit change for the aisle. Positive = entered, negative = left.
    pub delta: i32,
}

/// Authoritative per-aisle buffer/storage snapshot after a physical state
/// transition. Counts are slot-based, so transient duplicate tote records do
/// not push occupancy above capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AisleStateRecord {
    pub sim_time_ms: u64,
    pub zone_id: u32,
    pub storage_capacity: u32,
    pub storage_occupied: u32,
    pub storage_empty: u32,
    pub storage_non_empty: u32,
    /// Reserved for imported/future facts with unknown tote contents. Live
    /// simulations emit zero and preserve occupied = empty + non-empty.
    pub storage_unknown: u32,
    pub diving_board_capacity: u32,
    /// Physical totes at diving boards, not reservations.
    pub diving_board_occupied: u32,
}

/// Row kind in the journey-leg journal. HTM producers emit `Travel` and
/// `Wait`; `Mechanical` and `Alignment` remain available for richer endpoint
/// facts. `turn_count` is a column on a Travel row, never its own `Activity` —
/// travel already includes time spent turning, so a Turns row would double-
/// count (spec DR-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activity {
    Travel,
    Wait,
    Mechanical,
    /// Fine-grained stationary time spent aligning a fork with a rack or relay.
    /// Appended to preserve the serialized discriminants of existing variants.
    Alignment,
}

/// Pick vs putaway nature of a presentation cycle, resolved at the observer
/// from the leg's station endpoint via `config.station_types`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TripType {
    Pick,
    Putaway,
    Extraction,
}

/// Semantic endpoint of a journal leg, with station kind already resolved so
/// the xlsx export needs no `config` lookup. The `From`/`To` label and the
/// `*_relay_id` / `*_station_id` id columns all derive from this. The
/// `Trip Type` column is derived at export from a leg's station endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmJournalEndpoint {
    Parking,
    Relay(RelayId),
    PickStation(StationId),
    PutawayStation(StationId),
    ExtractionStation(StationId),
}

/// One journey-leg journal row — the "HTM Journey Legs" sheet. A nav event
/// becomes alternating Travel rows (contiguous move/rotation runs) and Wait
/// rows (holds between those runs); endpoint dwells have their own producers.
///
/// Times and ids carry their domain types all the way onto the wire: a row
/// read back from a recording is the same typed fact the observer wrote,
/// so nothing downstream re-derives a unit or an id space from a field name.
///
/// `turn_count` is geometric: actual curve sweep or orientation change, summed
/// over one Nav event and rounded to whole 90° equivalents (nearest, ties-even).
/// Prefix differences allocate that total across Travel rows, so Wait splits
/// do not change it. Separate Nav events are quantized independently; this
/// integer field cannot retain arbitrary fractional turns. Rotation *time*
/// remains the explicit Rotate windows, excluding steering along Move curves.
///
/// No back-compat: this schema renamed `sim_time_ms`/`travel_ms` to typed
/// `start`/`duration`. Nothing reads a row written before that rename — the
/// container that carried them is gone, and the recordings that hold them are
/// read by the binaries that wrote them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmLegRecord {
    /// Row start time: for Travel rows, the FIRST `Move` action's start
    /// (falling back to the nav-event time when the chain has no `Move`),
    /// so a rotate-first journey does not book travel into the pre-rotation
    /// span; Wait rows start at their true interval start. The `Hour`
    /// column is derived at export (`start / 3_600_000 ms`, 0-based
    /// elapsed-hour index).
    pub start: SimTime,
    #[serde(default = "default_activity")]
    pub activity: Activity,
    #[serde(default = "default_htm_endpoint")]
    pub from: HtmJournalEndpoint,
    #[serde(default = "default_htm_endpoint")]
    pub to: HtmJournalEndpoint,
    /// Zone of the `from`/`to` relay endpoint (`Some` only when that endpoint is
    /// a `Relay`), so the sheet's relay id columns each carry their zone.
    #[serde(default)]
    pub from_zone_id: Option<u32>,
    #[serde(default)]
    pub to_zone_id: Option<u32>,
    /// Straight-line travel distance accrued during this nav event. 0 for
    /// Wait/Mechanical rows (Phase 2/3).
    pub distance_mm: u64,
    /// Row duration. Travel includes the real clock window of its contiguous
    /// move/rotation run; Wait covers its stationary hold.
    pub duration: SimDuration,
    /// Trip ID grouping all legs of one presentation cycle. `None` for
    /// parking-relocation legs (`park_idle_htms` emits their `TripStart`
    /// with no presentation), which journal like any other leg.
    #[serde(default)]
    pub presentation_id: Option<u64>,
    pub robot_id: RobotId,
    /// Tote physically on board for this leg (spec DR-4).
    #[serde(default)]
    pub carrying_cargo: bool,
    /// 90°-equivalent turns over this nav event's waypoint chain (geometric).
    pub turn_count: u32,
    /// Wait classification — Phase 2 only; always `None` in Phase 1.
    ///
    /// The pre-#272 vocabulary, kept populated for the two reasons it can
    /// name so the xlsx "Wait reason" column keeps rendering. [`Self::hold`]
    /// is the fact the #272 classifier reads; this field is `None` on every
    /// hold the old two-name vocabulary cannot express.
    #[serde(default)]
    pub wait_reason: Option<HtmWaitReason>,
    /// The #272 facts of a `Wait` row: what the schedule was holding for and
    /// which kind of cell it left the D30 standing on. `Some` on every hold
    /// the live observer journals; `None` on `Travel` rows and on any row a
    /// foreign producer builds without the facts.
    #[serde(default)]
    pub hold: Option<HtmHold>,
    /// Free-text remarks; empty in Phase 1.
    #[serde(default)]
    pub notes: String,
}

/// The two facts a `Wait` row carries for waiting attribution: the task
/// context the position cannot supply, and the position the task context
/// cannot supply. Both are captured facts about the SCHEDULE — this is an
/// offline-planned simulator, so a hold is an interval the solved schedule
/// leaves between two committed actions, never an observation of a robot
/// deciding to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmHold {
    pub cause: HtmWaitCause,
    pub position: HtmWaitPosition,
}

/// What the schedule was holding the D30 for across a `Wait` row — the
/// task-context half of [`HtmHold`]. Each variant names the committed
/// scheduling fact that bounds the interval, so attribution never has to
/// infer intent from geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmWaitCause {
    /// No task was assigned over the interval: a gap between the robot's
    /// scheduled tasks, which is what idle IS.
    NoTask,
    /// The tote was not yet on the diving board and the D30 never drove
    /// here: the chain chose a D30 already standing on this relay's access
    /// cell, so the interval is the VTM cycle's lead time elapsing under a
    /// bot that was going to be here anyway (`HtmRelayPickupEvent` with
    /// `resident`).
    AwaitingDepositResident,
    /// The tote was not yet on the diving board and the D30 drove here to
    /// meet a quoted deposit instant, arriving before the tote: the
    /// interval runs from its arrival at the relay to the fork-Get. Unlike
    /// the resident case this is travel that could have started later.
    AwaitingDepositDrove,
    /// The return diving-board slot reservation was not yet valid: the
    /// interval runs from the D30's arrival at the return relay to the
    /// deposit (`HtmRelayDepositEvent`). The schedule does NOT hold the D30
    /// at the station for this — it departs on time and the wait lands at
    /// the relay access cell (issue #272, resolved 2026-07-30). That is the
    /// model's current placement; physically gating access-cell entry awaits
    /// an arrive-after path constraint (#153).
    AwaitingSlot,
    /// The operator dwell had not started yet: the interval runs from the
    /// bot's arrival at the presentation cell, which opens its station visit,
    /// to the recorded dwell's start or, when the run stopped inside the visit
    /// before any dwell was recorded, to the current world time.
    AwaitingStationService,
    /// The dwell was over and the D30 stood on the presentation cell until
    /// its next leg's admitted departure or, when the run stopped before one
    /// was scheduled, until the current world time. Bounded by the recorded
    /// dwell's end and by that departure or the current world time, so it is
    /// real occupancy, unlike `PostStationService`.
    ///
    /// Its producer is `activity::visits`, which journals it for a visit a
    /// departure closed: the D30 leaving on a committed leg (the store leg's,
    /// or a direct-putaway delivery re-claiming it), whose departure instant
    /// bounds the stand. A visit the run stopped inside before any departure
    /// was scheduled stands after its dwell until the current world time,
    /// when that is later, since a departure scheduled later could not start
    /// before it, and that stand is journaled here too.
    ///
    /// A positive interval here became reachable when the store leg's
    /// departure started flooring at the scheduler's settled frontier: the
    /// return can now be admitted later than the presentation finished, and
    /// the D30 genuinely waits out the difference at the station.
    AwaitingDeparture,
    /// Integrity sentinel: station time past both the current world time and
    /// the recorded dwell's end with no departure to bound it, the factless
    /// tail ruling #49 says cannot happen; a positive row makes the D30 report
    /// fail hard. Nothing journals it: a visit the run stopped inside ends at
    /// the current world time, or at its recorded dwell's end if that is later.
    PostStationService,
    /// An explicit `Wait` hold the pathfinder baked into the selected route,
    /// or a stretch of a nav chain no committed action covers.
    RouteHold,
}

/// Which kind of cell the schedule left the D30 standing on across a `Wait`
/// row — the positional half of [`HtmHold`], read off the map's own
/// [`desgraph::CellKind`] for the cell the observer last tracked the robot
/// at. Positional only: it says where, never why, and never that the robot
/// "chose" to be there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmWaitPosition {
    /// An explicitly authored charger, occupied initially until assigned work.
    Charging,
    /// A station's presentation cell, where the operator dwell happens.
    Presentation,
    /// A station-owned queue cell on the approach chain.
    StationQueue,
    /// A relay cell (the diving board's HTM-access side for a D30).
    Relay,
    /// A mapped cell that is none of the above: open floor, highway,
    /// junction, aisle, station area, storage.
    Route,
    /// The captured facts did not place the robot — no tracked cell, or a
    /// cell the map table does not cover. Never a guess.
    Unknown,
}

/// Why an HTM was waiting during a `Wait` journal row — the two-name
/// vocabulary that predates #272, kept alive because the xlsx "Wait reason"
/// column and the metrics panel still render it.
///
/// SUPERSEDED for attribution by [`HtmHold`], which every live `Wait` row now
/// carries: the claim this enum was built on — that route holds and station
/// holds have no per-robot signal — was repealed on #272 (2026-07-30), and
/// the observer journals a row for each of them. This field stays `Some` only
/// for the two holds these names describe correctly (`NoTask` = a gap between
/// tasks, `AwaitingDeposit` = the relay dwell awaiting the DA50 drop) and
/// is `None` on every other hold rather than being stretched to cover it.
/// New variants here would break the `match`es in the xlsx export, so the
/// #272 vocabulary lives on [`HtmWaitCause`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmWaitReason {
    /// Idle with no task assigned: a gap between the robot's tasks.
    NoTask,
    /// Dwelling at the relay awaiting the VTM to deposit the tote into the
    /// buffer before the HTM can fork-pick it up (`HtmRelayPickupEvent`).
    AwaitingDeposit,
}

/// One committed VTM cycle (forward or return), decomposed into empty and
/// loaded travel plus alignment and transfer at the rack and relay. Includes
/// relay/rack/tote/chain context and the pre-cycle wait. The leg split is read
/// straight off the scheduler's `CycleBreakdown`.
///
/// Serialized ids and times are still raw `u32`/`u64`. That used to cite
/// `HtmLegRecord` as precedent; the leg row now serializes domain types, so
/// this row is a holdout awaiting the same treatment, not a followed
/// convention. The typed `direction` / `wait_reason` enums derive `Serialize`
/// and render via the xlsx label helpers. The `Cycle Type` column
/// (PickSupport/PutawaySupport) is derived at export from `direction`.
///
/// Every field is populated on a live row and stated in a recording, so the
/// `serde` defaults left below carry nothing: they exist for the shapes the
/// retired container held, and the one alias that rescued its `mechanical_ms`
/// key is gone with it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VtmCycleRecord {
    /// Cycle start (`vtm_start`), schedule clock; the leg fields are
    /// geometric (see `VtmCycleEvent`) — overlapping rows union by cycle id.
    pub sim_time_ms: u64,
    pub vtm_id: u32,
    /// Cycle ID (`vcyc{N}`); joins cycle ↔ leg rows.
    #[serde(default)]
    pub vtm_cycle_id: Option<u64>,
    pub zone_id: u32,
    pub direction: VtmCycleDirection,
    pub empty_leg_ms: u64,
    pub loaded_leg_ms: u64,
    /// Rack-side fork time (`CycleBreakdown.rack_transfer_secs`).
    pub fork_ms: u64,
    /// Legacy relay-service time: relay alignment + relay transfer.
    #[serde(default)]
    pub buffer_ms: u64,
    /// Rack-side alignment before fork insertion.
    #[serde(default)]
    pub rack_alignment_ms: u64,
    /// Relay-side alignment before fork insertion.
    #[serde(default)]
    pub relay_alignment_ms: u64,
    /// Relay pickup/drop transfer excluding alignment.
    #[serde(default)]
    pub relay_transfer_ms: u64,
    pub total_cycle_ms: u64,
    #[serde(default)]
    pub relay_id: u32,
    #[serde(default)]
    pub rack_id: u32,
    #[serde(default)]
    pub tote_id: u32,
    /// Trip ID (header `Trip ID`); groups this cycle with its leg-journal rows.
    /// `None` only if an id never threaded (not expected on live cycles).
    #[serde(default)]
    pub presentation_id: Option<u64>,
    /// Pre-cycle idle gap (`vtm_start − available_at`); 0 for `Continuous`.
    #[serde(default)]
    pub pre_cycle_wait_ms: u64,
    #[serde(default = "default_vtm_idle_reason")]
    pub wait_reason: VtmIdleReasonKind,
    /// Which flow the moved tote belongs to (pick vs. putaway).
    #[serde(default = "default_tote_source")]
    pub tote_source: ToteSource,
}

/// One DA50 pickup/drop fact with the tote payload captured at the physical
/// transfer boundary. Stored independently of cycle aggregates so throughput
/// can be grouped by operation, aisle, and payload without later inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VtmOperationRecord {
    pub sim_time_ms: u64,
    pub vtm_id: u32,
    pub vtm_cycle_id: u64,
    pub zone_id: u32,
    pub kind: VtmOperationKind,
    pub rack_id: u32,
    /// Together with rack_id, identifies physical capacity. Older recordings
    /// omitted the level; their visual arrival can supply it during an audit.
    #[serde(default)]
    pub rack_level: Option<u16>,
    pub relay_id: u32,
    pub tote_id: u32,
    pub payload: TotePayloadState,
    pub presentation_id: Option<u64>,
}

/// Semantic endpoint of a VTM journey leg. VTMs are rail-bound, so the only
/// endpoints are a rack slot, a relay buffer, or an `Aisle` rail rest position
/// (the honest label for a between-cycles idle — VTMs idle on the rail, not a
/// park cell; spec DR-2). Resolved from `VtmEndpoint` at the observer so the
/// xlsx export needs no extra lookup; the `From`/`To` label and `*_rack_id` /
/// `*_relay_id` columns all derive from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VtmJournalEndpoint {
    Aisle,
    Relay(RelayId),
    Rack(RackId),
}

/// One VTM journey-leg journal row — the "VTM Journey Legs" sheet. The VTM
/// analog of [`HtmLegRecord`]: a `Travel` row per `RailMove`, an `Alignment`
/// row per endpoint approach, a `Mechanical` row per `Fork`, and a `Wait` row
/// per non-`Continuous` pre-cycle gap.
///
/// Serialized ids and times are still raw `u32`/`u64`, as on
/// `VtmCycleRecord` — a holdout the [`HtmLegRecord`] retyping has not reached
/// yet, no longer a precedent it sets. The typed enums derive
/// `Serialize` and render via xlsx label helpers. Per-cycle the legs are
/// strictly sequential — a pre-cycle idle Wait followed by the six phase legs
/// in increasing time — so within one cycle the journal partitions wall-clock
/// without double-counting (spec §6). Under
/// Teleport rows are geometric-clock: cycles can overlap (key by
/// `vtm_cycle_id`) and Wait rows are suppressed. The
/// `Cycle Type` column (PickSupport/PutawaySupport) is derived at export from
/// `direction`.
///
/// Every field is populated on a live row and stated in a recording; the
/// `serde` defaults left below carry nothing, and the alias that rescued the
/// pre-rename `chain_id` key went with the container that held it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtmLegRecord {
    /// Absolute leg start. The `Hour` column is derived at export
    /// (`start_ms / 3_600_000`, 0-based elapsed-hour index).
    pub start_ms: u64,
    pub duration_ms: u64,
    pub activity: Activity,
    pub from: VtmJournalEndpoint,
    pub to: VtmJournalEndpoint,
    /// Zone of the `from`/`to` relay endpoint (`Some` only when that endpoint is
    /// a `Relay`), so the sheet's relay id columns each carry their zone.
    #[serde(default)]
    pub from_zone_id: Option<u32>,
    #[serde(default)]
    pub to_zone_id: Option<u32>,
    /// Straight-line rail distance for `Travel` rows; 0 for Mechanical/Wait.
    pub distance_mm: u64,
    pub vtm_id: u32,
    /// Cycle ID (`vcyc{N}`); joins cycle ↔ leg rows.
    #[serde(default)]
    pub vtm_cycle_id: Option<u64>,
    /// Tote on board this leg; `None` for an empty-approach Travel or a Wait.
    #[serde(default)]
    pub tote_id: Option<u32>,
    /// Trip ID (header `Trip ID`) grouping this leg with the cycle's others
    /// and the correlated HTM legs.
    #[serde(default)]
    pub presentation_id: Option<u64>,
    pub direction: VtmCycleDirection,
    /// Whether a tote is physically on board (`RailMove.tote_id.is_some()`;
    /// always true for a `Fork`).
    pub carrying_tote: bool,
    /// Set on `Wait` rows only — the pre-cycle gap's classification.
    #[serde(default)]
    pub wait_reason: Option<VtmIdleReasonKind>,
    /// Free-text remarks: the mechanical sub-kind (fork-at-rack vs
    /// buffer-at-relay) for Mechanical rows; empty otherwise.
    #[serde(default)]
    pub notes: String,
}

/// One completed extraction, whether the container leaves or is emptied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionRecord {
    pub sim_time_ms: u64,
    pub station_id: StationId,
    pub robot_id: RobotId,
    pub quantity: u32,
    pub presentation_id: Option<u64>,
}

/// A single recorded pick presentation. `dc_id` is the forward distribution
/// center (`None` for pre-formed-order picks); renders as the xlsx DC column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickRecord {
    pub sim_time_ms: u64,
    pub station_id: StationId,
    pub sku: SkuId,
    pub quantity: u32,
    pub dc_id: Option<DistributionCenterId>,
    #[serde(default)]
    pub order_id: Option<OrderId>,
    #[serde(default)]
    pub tote_id: Option<u32>,
    #[serde(default)]
    pub source_zone_id: Option<u32>,
    #[serde(default)]
    pub source_relay_id: Option<RelayId>,
    /// Trip ID joining this pick to its journey legs.
    #[serde(default)]
    pub presentation_id: Option<u64>,
}

/// A completed order (Dematic "pallet") with its final attribution facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedOrderRecord {
    pub sim_time_ms: u64,
    pub order_id: OrderId,
    pub station_id: Option<StationId>,
    pub dc_id: Option<DistributionCenterId>,
}

/// A single recorded putaway presentation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PutawayRecord {
    pub sim_time_ms: u64,
    pub station_id: StationId,
    pub sku: SkuId,
    pub quantity: u32,
    /// Trip ID joining this putaway to its journey legs; `None` when the tote
    /// carried no chain.
    #[serde(default)]
    pub presentation_id: Option<u64>,
}

// ---------------------------------------------------------------------------
// Defaults for the journal records' enum fields
// ---------------------------------------------------------------------------
//
// Named defaults rather than `Default` on the shared domain enums, which have
// no sensible one: an `Activity` or an `HtmJournalEndpoint` means something
// specific everywhere else it is used. A live row fills every one of these.

fn default_activity() -> Activity {
    Activity::Travel
}

fn default_htm_endpoint() -> HtmJournalEndpoint {
    HtmJournalEndpoint::Parking
}

fn default_vtm_idle_reason() -> VtmIdleReasonKind {
    VtmIdleReasonKind::Continuous
}

fn default_tote_source() -> ToteSource {
    ToteSource::Pick
}

impl MetricsOutput {
    /// Cumulative VTM visit counts as a dense CELL-ID-indexed vector
    /// (index == CellId.0). The counters are id-keyed at the source; this is
    /// just densification for the renderer's id-keyed instance buffer.
    pub fn vtm_counts_dense(&self, len: usize) -> Vec<u32> {
        sparse_to_dense(&self.cumulative_grid.nav_visits_vtm, len)
    }

    /// Cumulative HTM visit counts, dense by cell id.
    pub fn htm_counts_dense(&self, len: usize) -> Vec<u32> {
        sparse_to_dense(&self.cumulative_grid.nav_visits_htm, len)
    }
}

/// Densify CellId-keyed counters: index == CellId.0.
fn sparse_to_dense<S: BuildHasher>(sparse: &HashMap<u32, u32, S>, len: usize) -> Vec<u32> {
    let mut dense = vec![0u32; len];
    for (&cell_id, &count) in sparse {
        if let Some(slot) = dense.get_mut(cell_id as usize) {
            *slot = count;
        }
    }
    dense
}
