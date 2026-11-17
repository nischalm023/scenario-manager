//! Within `metrics`, `profile` is THE RUN MODEL: one finished run held in
//! memory — the documents it was built from, its per-slice time series, its
//! cumulative counters and its summary — with everything downstream (desvis
//! playback, the charts, the workbook, `compare`, `repeat`) written against
//! it and nothing else.
//!
//! It is not a file format. A run is recorded as a `.worldview`, and
//! `metrics::worldview` is the one place that turns a [`PerfProfile`] into
//! one and back. [`convert`] holds what the observer's typed counters have to
//! become before they can be a time series: the string-keyed scalars, each
//! written with its class, and the per-cell grid layers.
//!
//! TODO(#300): the enumerated named-log fields are scheduled to dissolve into
//! per-table rows; new fact families must not add named fields here.

mod convert;

pub use convert::{
    ClassedScalar, ScalarClass, classed_scalars, grid_to_dynamic, grid_visit_layers, relay_zone_busy_lanes,
    scalars_to_map,
};

use serde_json::Value;
use std::collections::HashMap;

use desgraph::types::{CellId, RobotType};
use desgraph::{StationId, StationType};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::config::SimConfig;
use crate::metrics::streams::LogStream;
use crate::model::order::OrderSpec;
use crate::model::relay::{RelayUtilizationSummary, RelayUtilizationTimeline};
use crate::model::{RobotId, SimOffset};
use crate::scenario::ScenarioConfig;
use crate::scenario_gen::SkuInfo;
use crate::sim_types::SimResult;

use crate::events::{StripePlanEvent, VtmIdleReasonKind};
use crate::observer::duty::{BusyInterval, DutyBreakdown, StationBusyInterval};
use crate::observer::output::{
    AisleFluxRecord, AisleStateRecord, CompletedOrderRecord, HtmLegRecord, PickRecord, PutawayRecord, VtmCycleRecord,
    VtmLegRecord, VtmOperationRecord,
};
use crate::observer::station_diag::StationDiagRecord;
use crate::observer::time_slice::{BotLocation, RelayStateRecord, ToteStateRecord};

// ---------------------------------------------------------------------------
// PerfProfile
// ---------------------------------------------------------------------------

/// A fully self-contained performance profile for a simulation run.
///
/// Contains everything needed for playback in desvis without external files:
/// the warehouse map, scenario data, simulation config, time-series metrics,
/// and summary statistics.
#[derive(Serialize, Deserialize)]
pub struct PerfProfile {
    /// Human-readable run name (e.g. "cencora_phase0 + scenario_a @ main/abc1234").
    pub name: String,
    /// Build/environment metadata.
    pub metadata: ProfileMetadata,
    /// The world-protocol scenario document, embedded verbatim
    /// (self-contained map data), parsed.
    pub world: Value,
    /// The same document and everything beside it, as the bytes they were
    /// read from disk as: a recording resolves every reference against
    /// itself and the filesystem never.
    #[serde(default)]
    pub documents: RunDocuments,
    /// What the run's own cell, station and robot ids are called in the
    /// world document.
    #[serde(default)]
    pub ids: IdTables,
    /// Simulation configuration.
    pub sim_config: SimConfig,
    /// Scenario source data (parametric config or loaded SKUs+orders).
    pub scenario: ScenarioSource,
    /// The workload the scenario resolved to.
    #[serde(default)]
    pub workload: ResolvedWorkload,
    /// The DA50 fleet the world gave this run. No configuration states it, so
    /// a consumer that has the run and not the map — the workbook's cover
    /// sheet, the DA50 state report's fleet-time denominator — reads it here.
    #[serde(default)]
    pub vtm_fleet: VtmFleet,
    /// Derived top-level summary statistics.
    pub summary: SummaryStats,
    /// Raw simulation result (durations, picks, throughput, detailed metrics).
    pub sim_result: SimResult,
    /// Per-time-slice scalar time series (dynamic metric keys).
    pub time_slices: Vec<SliceRecord>,
    /// Cumulative grid layers (layer_name -> cell_id -> value).
    pub cumulative_grid: DynamicGrid,
    /// Cumulative scalar metrics (metric_name -> value).
    pub cumulative_scalars: HashMap<String, f64>,
}

// ---------------------------------------------------------------------------
// RunDocuments and IdTables
// ---------------------------------------------------------------------------

/// The files a run was built from, byte for byte.
///
/// A recording carries them so that replaying, re-running or inspecting a run
/// needs nothing but the recording — the spec that named the world, the world
/// document itself, and every workload file the scenario points at. Nothing
/// parses them here; they are handed back exactly as they were read.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RunDocuments {
    /// The world spec (`--map`) as read from disk. Empty for a run started
    /// from a recording, or from a world document with no spec beside it.
    #[serde(default, with = "serde_bytes")]
    pub spec: Vec<u8>,
    /// The world document the spec named, as read from disk.
    #[serde(default, with = "serde_bytes")]
    pub world: Vec<u8>,
    /// Each file the scenario references, keyed by the reference exactly as
    /// the scenario states it, so a consumer resolves the reference against
    /// this list.
    #[serde(default)]
    pub data: Vec<(String, Vec<u8>)>,
}

/// One robot the run holds: what the run calls it, which class of machine it
/// is, and the world document's entity for that class when the loader knows
/// one.
#[derive(Clone, Serialize, Deserialize)]
pub struct RobotIdentity {
    pub robot: RobotId,
    pub name: String,
    pub class: RobotType,
    /// Nil for a generated fleet, which is every one of master's fixtures:
    /// the document declares how many machines, not which entity each is.
    #[serde(default)]
    pub class_entity: Option<String>,
}

/// The observer's roster as the run's robots, by id. The fleet is generated,
/// so a robot's class names no entity: the document says how many machines,
/// not which entity each one is.
pub fn robot_identities(roster: &HashMap<RobotId, RobotType>) -> Vec<RobotIdentity> {
    let mut robots: Vec<RobotIdentity> = roster
        .iter()
        .map(|(&robot, &class)| RobotIdentity { robot, name: robot.to_string(), class, class_entity: None })
        .collect();
    robots.sort_by_key(|robot| robot.robot.0);
    robots
}

/// What the run's dense ids are called in the world document.
///
/// The simulator mints `CellId`, `StationId` and `RobotId` for its own speed;
/// a reader asking "which aisle is cell 4102?" needs the document's own
/// identity for it, and this is where the two meet.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct IdTables {
    pub cells: Vec<(CellId, String)>,
    pub stations: Vec<(StationId, String)>,
    pub robots: Vec<RobotIdentity>,
    /// The document's root 3-D frame, by entity id — the frame every position
    /// the run reports is measured in. Empty where the document declares no
    /// primary frame.
    #[serde(default)]
    pub frame: String,
}

// ---------------------------------------------------------------------------
// ProfileMetadata
// ---------------------------------------------------------------------------

/// Build and environment metadata for a profile.
#[derive(Clone, Serialize, Deserialize)]
pub struct ProfileMetadata {
    /// Git branch name.
    pub branch: String,
    /// Full git commit hash.
    pub commit: String,
    /// Short git commit hash (7 chars).
    pub commit_short: String,
    /// Tracked working-tree changes vs `HEAD` at run time — the recorded
    /// commit does not fully describe the run when set. Defaulted for profiles
    /// recorded before dirty-tracking existed.
    #[serde(default)]
    pub dirty: bool,
    /// ISO 8601 timestamp when the profile was created.
    pub timestamp: String,
    /// Hostname of the machine that ran the benchmark.
    pub hostname: String,
    /// Grid width (columns).
    pub grid_width: u32,
    /// Grid depth (rows).
    pub grid_depth: u32,
    /// Time slice width in milliseconds.
    pub slice_width_ms: u64,
}

// ---------------------------------------------------------------------------
// ScenarioSource
// ---------------------------------------------------------------------------

/// How the scenario data was sourced.
#[derive(Clone, Serialize, Deserialize)]
pub enum ScenarioSource {
    /// From a ScenarioConfig (Generated, Inline, or Csv).
    Parametric(ScenarioConfig),
    /// Loaded from external CSV files (data embedded in profile).
    Loaded { dir_name: String, skus: Vec<SkuInfo>, orders: Vec<OrderSpec> },
}

impl ScenarioSource {
    /// Wrap a ScenarioConfig into a ScenarioSource.
    pub fn from_config(config: ScenarioConfig) -> Self {
        ScenarioSource::Parametric(config)
    }
}

/// How many DA50s a world gave a run: the sum of its zones' VTM counts.
///
/// A count of its own type and a fixed width, because a recording persists it:
/// `usize` is 64 bits on the machine that wrote a file and could be 32 on the
/// machine that reads it, and a fleet size that changes meaning between the
/// two would move every DA50 share in the workbook.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VtmFleet(pub u32);

impl TryFrom<usize> for VtmFleet {
    type Error = String;

    /// The fleet a map counted, refusing a count no fixed width can state
    /// rather than rounding one — the workbook refuses it too, and a recording
    /// that quietly shrank a fleet would misreport every share computed from it.
    fn try_from(machines: usize) -> Result<Self, Self::Error> {
        u32::try_from(machines)
            .map(VtmFleet)
            .map_err(|_| format!("the map states {machines} DA50s, more than a fleet count can hold"))
    }
}

impl From<VtmFleet> for usize {
    fn from(fleet: VtmFleet) -> Self {
        fleet.0 as usize
    }
}

/// The catalogue the run stocked and the orders it was given, as the scenario
/// generator settled them against the map.
///
/// A scenario states how to build a workload — a seed and counts, or the files
/// to read — and the workload itself exists only once the generator has run.
/// The workbook lists both SKUs and orders row by row, so a finished run holds
/// them and no consumer re-runs the generator, or reaches for the files the
/// scenario named, to get them back.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ResolvedWorkload {
    /// Every SKU the run stocked, in the order the generator produced them.
    pub skus: Vec<SkuInfo>,
    /// Every order the run was given, in the order the generator produced them.
    pub orders: Vec<OrderSpec>,
}

// ---------------------------------------------------------------------------
// SummaryStats
// ---------------------------------------------------------------------------

/// Derived top-level statistics for quick comparison.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SummaryStats {
    /// Self-described journals travel in container stream frames, not the
    /// legacy profile document. Empty means no producer declared a stream.
    #[serde(skip)]
    pub log_streams: Vec<LogStream>,
    /// Wall-clock duration in seconds.
    pub wall_clock_secs: f64,
    /// Simulated time in seconds.
    pub sim_time_secs: f64,
    /// Total picks completed.
    pub total_picks: u64,
    /// Total orders fully completed.
    pub total_orders_completed: u64,
    /// Throughput in picks per hour, over the post-embargo window only.
    pub throughput_picks_per_hour: f64,
    /// Total putaway items completed (qty-weighted) — the analog of
    /// `total_picks`. Defaulted so pre-existing profiles still load.
    #[serde(default)]
    pub total_putaways: u64,
    /// Putaway items per hour — qty-weighted analog of
    /// `throughput_picks_per_hour`, over the full sim window.
    #[serde(default)]
    pub throughput_putaway_items_per_hour: f64,
    /// Effective HTM cruise speed (m/s) while carrying a tote — the
    /// distance-weighted mean translation speed over every `Travel` leg with
    /// `carrying_cargo`, i.e. `Σ distance_mm / Σ duration` (mm/ms = m/s).
    /// A Travel row's duration is its whole move/rotation run (#272 stopped
    /// summing move time alone), so this is the flat per-cell speed that
    /// reproduces the run's kinematic travel time — plug it into
    /// `htm.loaded_constant_speed`. Under
    /// `ConstantSpeed` mode it is a self-check: it equals the configured cruise.
    /// 0.0 when no loaded travel occurred. Defaulted so older profiles load.
    #[serde(default)]
    pub htm_effective_speed_loaded_mps: f64,
    /// Effective HTM cruise speed (m/s) while empty (fork down) — the empty
    /// analog of `htm_effective_speed_loaded_mps`; plug into
    /// `htm.empty_constant_speed`. 0.0 when no empty travel occurred.
    #[serde(default)]
    pub htm_effective_speed_empty_mps: f64,
    /// Effective HTM cruise speed (m/s) over all `Travel` legs regardless of
    /// load state — the combined distance-weighted mean. 0.0 when no HTM
    /// travel occurred.
    #[serde(default)]
    pub htm_effective_speed_combined_mps: f64,
    /// VTM idle percentage — fraction of configured-fleet time (fleet size ×
    /// sim elapsed) spent in no interval of work (travel or a fork
    /// operation). VTMs that never received work count as fully idle,
    /// matching the DA50 state family's fleet-share denominator
    /// (greymatter-foundry#254's eligible count × included duration rule).
    /// Time-weighted across the run; not the instantaneous count of Idle
    /// VTMs at any point.
    pub vtm_idle_pct: f64,
    /// HTM idle percentage — analogous to `vtm_idle_pct` over the configured
    /// HTM fleet: time-weighted fraction of fleet time spent in no interval of
    /// work (travel, a fork operation or a station visit).
    pub htm_idle_pct: f64,
    /// Configured VTM fleet size — the denominator population behind
    /// `vtm_idle_pct`, from the InitialPositions roster. `0` in profiles
    /// captured before this field existed; those idle values were computed
    /// over dispatched robots only, and consumers should treat their idle
    /// numbers as a different statistic.
    #[serde(default)]
    pub vtm_fleet_size: usize,
    /// Configured HTM fleet size backing `htm_idle_pct` — see
    /// `vtm_fleet_size` for the legacy-`0` meaning.
    #[serde(default)]
    pub htm_fleet_size: usize,
    /// Per-station presentations per hour.
    pub presentations_per_station_per_hour: Vec<(StationId, f64)>,
    /// Average presentations per station per hour.
    pub avg_presentations_per_station_per_hour: f64,
    /// Per-station operator utilization as a fraction in [0, 1]:
    /// `station_busy_ms / active_window`, where the active window runs from
    /// the station's first observed activity to sim end (floored at 1 ms)
    /// so a late-bootstrapping station is not penalized for warmup it had
    /// no work for; clamped to 1.0. Both pick and putaway stations are
    /// reported; consumers can filter by `station_types` if needed.
    /// Empty when no `CycleCompleteEvent`s landed.
    #[serde(default)]
    pub station_utilization_pct: Vec<(StationId, f64)>,
    /// Mean of `station_utilization_pct` across observed stations.
    /// `0.0` when no station has been observed busy.
    #[serde(default)]
    pub avg_station_utilization_pct: f64,
    /// Total simulation events processed.
    pub event_count: u64,
    /// Pathfinder failure rate (failures / queries).
    pub pathfinder_failure_rate: f64,
    /// Per-station inter-presentation gap statistics.
    #[serde(default)]
    pub station_gap_stats: Vec<StationGapStats>,
    /// Fleet-wide flow score: fraction of inter-presentation gaps <= pick_time_ms.
    /// 1.0 = perfectly smooth (every gap is zero or within one pick period),
    /// 0.0 = fully wave-driven (all gaps are large). Values near 1.0 indicate
    /// the two-phase dispatch is keeping stations continuously fed.
    #[serde(default)]
    pub flow_score: f64,
    /// Per-VTM duty-cycle breakdown over the full configured roster. Sum of
    /// every `BusyInterval` recorded for the robot, bucketed by state; a VTM
    /// with no activity has a default, zero-busy row. Idle is implicit
    /// (`sim_elapsed − busy`). Legacy profiles may omit zero-duty VTMs.
    #[serde(default)]
    pub vtm_duty_totals: HashMap<RobotId, DutyBreakdown>,
    /// Per-HTM duty-cycle breakdown. Same full-roster shape and legacy caveat
    /// as `vtm_duty_totals`.
    #[serde(default)]
    pub htm_duty_totals: HashMap<RobotId, DutyBreakdown>,
    /// Cumulative count of `VtmIdleReasonEvent`s by classification across
    /// the whole run. One increment per VTM cycle commit. Lets the
    /// consumer answer "what fraction of VTM cycles started back-to-back
    /// vs. with a relay wait?".
    #[serde(default)]
    pub vtm_idle_reason_counts: HashMap<VtmIdleReasonKind, u64>,
    /// Relay-access utilization (overall + per-zone %), for the desvis
    /// relay-utilization charts. Mirrors `MetricsOutput::relay_utilization`
    /// (oracle-based, same numbers as the headless `relay_utilization_pct`
    /// line). `None` on older profiles / rackless scenarios.
    #[serde(default)]
    pub relay_utilization: Option<RelayUtilizationSummary>,
    /// Per-zone × per-slice diving-board slot occupancy, for the desvis
    /// diving-board occupancy heat lanes. Mirrors
    /// `MetricsOutput::relay_slot_occupancy_timeline`. `None` on older
    /// profiles.
    #[serde(default)]
    pub relay_slot_occupancy_timeline: Option<RelayUtilizationTimeline>,
    /// Per-VTM `BusyInterval` timelines. Empty unless
    /// `SimConfig::record_robot_timelines` was set. Drives the per-VTM
    /// Gantt panel in desvis.
    #[serde(default)]
    pub vtm_robot_timelines: HashMap<RobotId, Vec<BusyInterval>>,
    /// Per-HTM `BusyInterval` timelines. Same gating and shape as
    /// `vtm_robot_timelines`.
    #[serde(default)]
    pub htm_robot_timelines: HashMap<RobotId, Vec<BusyInterval>>,
    /// Per-station operator-busy timelines for the Gantt view. One
    /// interval per cycle. Idle is implicit (gaps).
    #[serde(default)]
    pub station_timelines: HashMap<StationId, Vec<StationBusyInterval>>,
    /// Station type per station — lets the renderer color lanes by pick
    /// vs putaway.
    #[serde(default)]
    pub station_types: HashMap<StationId, StationType>,

    /// The destination stripe-plan snapshot log (phase 1 of the
    /// putaway-locality work), ordered by emission time — a series so
    /// dynamic restriping changes the data, not the interface. Exactly one
    /// t=0 entry under the static v1 map; empty for pre-formed scenarios
    /// and profiles predating stripe capture.
    #[serde(default)]
    pub stripe_plan_log: Vec<StripePlanEvent>,

    /// One row per pick presentation, in event order. Source for the
    /// "Picks" xlsx sheet.
    #[serde(default)]
    pub pick_log: Vec<PickRecord>,

    /// Distinct completed orders attributed to DC and pick station; empty when
    /// the loaded profile predates order-completion capture.
    /// Named-field residence is temporary pending the #300 tenant migration
    /// into container stream frames.
    #[serde(default)]
    pub completed_order_log: Vec<CompletedOrderRecord>,

    /// One row per putaway presentation, in event order.
    #[serde(default)]
    pub putaway_log: Vec<PutawayRecord>,

    /// Journey-leg journal rows: a nav chain becomes alternating Travel and
    /// Wait rows, and endpoint dwells add their own. Source for the "HTM
    /// Journey Legs" xlsx sheet; per-leg aggregates are derived downstream
    /// from this maximum-granularity log.
    ///
    /// The pre-rename `htm_trip_log` key alias is gone with the element
    /// tolerance it fed: [`HtmLegRecord`]'s fields are typed and renamed, so
    /// matching the old key would only get an old profile as far as a serde
    /// error on the first row. Container revision 2 refuses those files whole.
    #[serde(default)]
    pub htm_leg_log: Vec<HtmLegRecord>,

    /// One row per committed VTM cycle. Source for the "VTM Cycles" xlsx sheet.
    #[serde(default)]
    pub vtm_cycle_log: Vec<VtmCycleRecord>,

    /// One row per physical DA50 pickup/drop. Payload state is captured at the
    /// operation boundary; empty when the loaded profile predates operation capture.
    #[serde(default)]
    pub vtm_operation_log: Vec<VtmOperationRecord>,

    /// Per-aisle storage composition and physical diving-board occupancy
    /// snapshots; empty when the loaded profile predates aisle-state capture.
    /// Named-field residence is temporary pending the #300 tenant migration
    /// into container stream frames.
    #[serde(default)]
    pub aisle_state_log: Vec<AisleStateRecord>,

    /// One journey-leg journal row per VTM micro-phase (Travel / Alignment / Mechanical /
    /// Wait). Source for the "VTM Journey Legs" xlsx sheet; the VTM analog of
    /// `htm_leg_log`.
    #[serde(default)]
    pub vtm_leg_log: Vec<VtmLegRecord>,

    /// One row per aisle crossing (SKU units a tote carried in/out of an
    /// aisle), in event order. Source for the "Inventory Delta" xlsx sheet.
    #[serde(default)]
    pub aisle_flux_log: Vec<AisleFluxRecord>,

    /// Per-aisle rack tote-slot capacity: `(ZoneId.0, slots)`, ascending by
    /// aisle. Denominator for per-aisle fill % in the "Rack" xlsx sheet.
    #[serde(default)]
    pub rack_zone_capacity: Vec<(u32, u32)>,

    /// Resolved putaway direct-chain bias — the value the run actually used
    /// (mirrors `SimResult::putaway_direct_bias`). Auto (`None` config)
    /// resolves to the map's Fleet Pressure Index; a fixed config keeps that
    /// value; zero with no putaway stations. Lets the xlsx cover report the
    /// effective bias, including the auto-derived FPI.
    #[serde(default, rename = "putaway_direct_bias_ms")]
    pub putaway_direct_bias: SimOffset,
}

/// Gap statistics for a single station.
#[derive(Clone, Serialize, Deserialize)]
pub struct StationGapStats {
    pub station_id: StationId,
    /// Number of inter-presentation gaps recorded.
    pub count: usize,
    /// Minimum gap in milliseconds.
    pub min_ms: u64,
    /// Mean gap in milliseconds.
    pub mean_ms: f64,
    /// 95th-percentile gap in milliseconds.
    pub p95_ms: u64,
    /// Maximum gap in milliseconds.
    pub max_ms: u64,
}

/// Compute gap statistics from a slice of gap samples.
/// Returns (min, mean, p95, max) in milliseconds.
pub fn compute_gap_stats(gaps: &mut Vec<u64>) -> (u64, f64, u64, u64) {
    if gaps.is_empty() {
        return (0, 0.0, 0, 0);
    }
    gaps.sort_unstable();
    let min = gaps[0];
    let max = *gaps.last().unwrap();
    let mean = gaps.iter().map(|&g| g as f64).sum::<f64>() / gaps.len() as f64;
    let p95_idx = ((gaps.len() as f64 * 0.95) as usize).min(gaps.len() - 1);
    let p95 = gaps[p95_idx];
    (min, mean, p95, max)
}

// ---------------------------------------------------------------------------
// Dynamic grid and slice types
// ---------------------------------------------------------------------------

/// A named collection of per-cell grid layers.
///
/// Each layer is a string-keyed sparse map of cell_id -> value.
/// Example layers: "nav_visits_vtm", "nav_visits_htm", "robot_presence_vtm_ms".
pub type DynamicGrid = HashMap<String, HashMap<u32, u64>>;

/// Scalar counters for a single time slice, stored as dynamic string-keyed map.
///
/// Keys match the field names in [`ScalarCounters`] plus synthesized keys for
/// state distributions (e.g. "vtm_state_Gap", "htm_state_Travel").
#[derive(Serialize, Deserialize, Clone)]
pub struct SliceRecord {
    /// Start time of this slice in simulation milliseconds.
    pub start_ms: u64,
    /// Dynamic scalar metrics for this slice.
    pub metrics: HashMap<String, f64>,
    /// What this slice alone added to each cell's counters. Every visit is
    /// booked to the slice it starts in, so the slices sum to
    /// [`PerfProfile::cumulative_grid`] exactly.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub grid: DynamicGrid,
    /// Per-bot location data (optional, backward compatible). Key = RobotId.0.
    /// Mirrors `TimeSliceData::bot_locations` (also `FxHashMap`) — both are
    /// `u32`-keyed and rebuilt every slice flush; FxHash on small integer keys
    /// is meaningfully faster than std's default AHash. The serde
    /// `skip_serializing_if = "HashMap::is_empty"` attribute still resolves
    /// because `FxHashMap<K, V>` is a type alias for `HashMap<K, V,
    /// FxBuildHasher>` — same struct, just a different hasher type parameter.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub bot_locations: FxHashMap<u32, BotLocation>,
    /// Relay reservation state (start set + intra-slice transitions).
    #[serde(default, skip_serializing_if = "RelayStateRecord::is_empty")]
    pub relay_state: RelayStateRecord,
    /// Tote positions (all totes at start + intra-slice moves).
    #[serde(default, skip_serializing_if = "ToteStateRecord::is_empty")]
    pub tote_state: ToteStateRecord,
    /// Station dispatch diagnostics (HTM assignments/arrivals, dispatch
    /// rejections, pipeline depths). Defaulted so old profiles still load.
    #[serde(default, skip_serializing_if = "StationDiagRecord::is_empty")]
    pub station_diag: StationDiagRecord,
}

#[cfg(test)]
mod summary_wire_tests {
    use Value;

    use super::*;

    /// `SummaryStats` is the struct a recording's end block and the xlsx cover
    /// read, and it is packed with `rmp_serde::to_vec_named`, so its field
    /// names ARE the format. `putaway_direct_bias` is `#[serde(default)]`, so
    /// losing the `rename` would not fail a read — every archived run would
    /// quietly report a zero effective bias instead.
    ///
    /// Deliberately separate from `SimResult`'s equivalent test: the attribute
    /// can be dropped from either struct independently, so one test cannot
    /// guard both.
    #[test]
    fn summary_stats_putaway_bias_wire_key_is_stable() {
        let s = SummaryStats { putaway_direct_bias: SimOffset::ms(-3_000), ..Default::default() };
        let json: Value = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json.get("putaway_direct_bias_ms"),
            Some(&serde_json::json!(-3_000)),
            "wire key must stay `putaway_direct_bias_ms` and stay a bare integer"
        );
        assert!(json.get("putaway_direct_bias").is_none(), "the Rust field name must not reach the wire");

        // A profile written before the retype still reads back correctly. Built
        // as a complete object rather than a one-key stub: 11 of these fields
        // have no serde default, so a stub fails for reasons unrelated to the
        // key under test and would make this assertion untrustworthy.
        let mut legacy = serde_json::to_value(SummaryStats::default()).unwrap();
        legacy
            .as_object_mut()
            .expect("SummaryStats serializes as a map")
            .insert("putaway_direct_bias_ms".to_string(), serde_json::json!(-3_000));
        let back: SummaryStats = serde_json::from_value(legacy).unwrap();
        assert_eq!(back.putaway_direct_bias, SimOffset::ms(-3_000));
    }
}
