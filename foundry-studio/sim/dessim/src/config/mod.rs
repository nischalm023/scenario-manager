//! Simulation configuration types.
//!
//! [`SimConfig`] contains all tunable simulation parameters grouped into nested
//! config structs. Every field has a sensible default so that callers only need
//! to specify the values they want to override. The `#[serde(default)]`
//! annotations ensure partial JSON payloads deserialize correctly.

mod robots;
mod subsystems;

use crate::composition::Composition;

pub use robots::{HtmConfig, IdleRelaySelection, InvalidFigure, VtmConfig, VtmIsolationMode};
pub use subsystems::{
    BarrierConfig, OperatorConfig, OrderConfig, PickConfig, PickSequencing, PutawayConfig, RelayConfig,
    VtmScoringConfig,
};

use crate::pathfinding::MotionModel;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fidelity mode
// ---------------------------------------------------------------------------

/// Movement fidelity level for the simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FidelityMode {
    /// Ghost mode: hold(total_time * navFactor). Java parity baseline.
    #[default]
    Ghost,
    /// Cell-time reservation table (future).
    CollisionDes,
    /// Per-tick advancement (future).
    TickStepper,
}

impl FidelityMode {
    /// All variants in display order.
    pub const ALL: &[FidelityMode] = &[FidelityMode::Ghost, FidelityMode::CollisionDes, FidelityMode::TickStepper];

    pub fn label(self) -> &'static str {
        match self {
            FidelityMode::Ghost => "Ghost",
            FidelityMode::CollisionDes => "Collision DES",
            FidelityMode::TickStepper => "Tick Stepper",
        }
    }
}

impl FromStr for FidelityMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ghost" => Ok(FidelityMode::Ghost),
            "collision-des" | "collision_des" | "collisiondes" => Ok(FidelityMode::CollisionDes),
            "tick-stepper" | "tick_stepper" | "tickstepper" => Ok(FidelityMode::TickStepper),
            _ => Err(format!("unknown fidelity mode: '{}'. Expected: ghost, collision-des, tick-stepper", s)),
        }
    }
}

// ---------------------------------------------------------------------------
// Order strategy
// ---------------------------------------------------------------------------

/// Order processing strategy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStrategy {
    /// Picks-per-face directly.
    #[default]
    PpfDirectly,
    /// Picks-per-face with tiering.
    PpfWithTiering,
    /// Totes-per-queue with VUT.
    TpqWithVut,
}

// ---------------------------------------------------------------------------
// Trace output
// ---------------------------------------------------------------------------

/// Trace record output format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceFormat {
    #[default]
    Text,
    Jsonl,
}

impl std::fmt::Display for TraceFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => f.write_str("text"),
            Self::Jsonl => f.write_str("jsonl"),
        }
    }
}

impl FromStr for TraceFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "jsonl" | "json" => Ok(Self::Jsonl),
            other => Err(format!("unknown trace format '{other}'. Expected: text, jsonl")),
        }
    }
}

/// Inclusive lower / exclusive upper SimTime filter for trace records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceWindow {
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

impl TraceWindow {
    pub fn parse(input: &str) -> Result<Self, String> {
        let Some((start, end)) = input.split_once("..") else {
            return Err(format!("invalid trace window '{input}'. Expected e.g. 60s..180s"));
        };
        Ok(Self { start_ms: parse_trace_time_bound(start.trim())?, end_ms: parse_trace_time_bound(end.trim())? })
    }

    pub fn contains(&self, ms: u64) -> bool {
        self.start_ms.map(|start| ms >= start).unwrap_or(true) && self.end_ms.map(|end| ms < end).unwrap_or(true)
    }
}

fn parse_trace_time_bound(input: &str) -> Result<Option<u64>, String> {
    if input.is_empty() {
        return Ok(None);
    }
    if let Some(seconds) = input.strip_suffix('s') {
        let seconds = seconds.parse::<f64>().map_err(|_| format!("invalid trace window bound '{input}'"))?;
        if seconds < 0.0 {
            return Err(format!("invalid negative trace window bound '{input}'"));
        }
        return Ok(Some((seconds * 1000.0).round() as u64));
    }
    let ms = input
        .parse::<u64>()
        .map_err(|_| format!("invalid trace window bound '{input}'. Use milliseconds or seconds like 60s"))?;
    Ok(Some(ms))
}

/// Runtime trace-output configuration.
///
/// `filters` is a list of independent filter clauses. Each clause is a
/// comma-separated string parsed as AND-of-tokens. An event passes the
/// overall filter when **any** clause matches (OR across clauses, AND
/// within). Empty `filters` means "accept all events."
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<TraceWindow>,
    pub format: TraceFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<PathBuf>,
}

impl TraceConfig {
    pub fn from_cli(
        enabled: bool,
        filters: Vec<String>,
        window: Option<&str>,
        format: &str,
        out: Option<PathBuf>,
    ) -> Result<Self, String> {
        let window = window.map(TraceWindow::parse).transpose()?;
        let format = format.parse::<TraceFormat>()?;
        let enabled = enabled || !filters.is_empty() || window.is_some() || out.is_some();
        Ok(Self { enabled, filters, window, format, out })
    }
}

// ---------------------------------------------------------------------------
// Top-level SimConfig
// ---------------------------------------------------------------------------

/// Top-level simulation configuration.
///
/// All fields have sensible defaults. A minimal [`super::SimRequest`] only
/// needs the warehouse design and scenario config; `SimConfig::default()` is
/// a valid starting point.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SimConfig {
    /// Top-level task set; also selectable at the root of a scenario spec.
    pub composition: Composition,
    // -- Global ----------------------------------------------------------
    /// Movement fidelity level.
    pub fidelity: FidelityMode,
    /// Total simulated duration.
    pub sim_duration: Duration,
    /// Per-type stagger interval between robot activations.
    pub stagger_interval: Duration,
    /// Seed for the deterministic PRNG.
    pub rng_seed: u64,
    /// Executor threads for the run. Nothing reads it today: the scheduler is
    /// single-threaded, and the figure only reaches the cover sheet.
    pub num_threads: usize,

    // -- Fleet sizing ----------------------------------------------------
    // VTM fleet size is intentionally absent: it is fully determined by
    // the map (each relay zone deploys `Zone::vtm_count` VTMs — see
    // `scenario_gen::assign_vtm_zones` and `WarehouseMap::vtm_count`).
    /// Number of HTMs to deploy.
    pub num_htms: usize,
    /// Number of RTPs to deploy.
    pub num_rtps: usize,

    // -- Per-robot-type timing -------------------------------------------
    /// VTM timing parameters.
    pub vtm: VtmConfig,
    /// HTM timing parameters.
    pub htm: HtmConfig,

    // -- Subsystems ------------------------------------------------------
    /// Relay system parameters.
    pub relay: RelayConfig,
    /// Order processing parameters.
    pub order: OrderConfig,
    /// Pick subsystem parameters.
    pub pick: PickConfig,
    /// Station operator parameters.
    pub operator: OperatorConfig,
    /// VTM scoring parameters for unified matching.
    pub vtm_scoring: VtmScoringConfig,
    /// Putaway subsystem parameters.
    pub putaway: PutawayConfig,
    /// Trailing-global-barrier parameters.
    #[serde(default)]
    pub barrier: BarrierConfig,

    // -- Navigation ------------------------------------------------------
    /// Multiplier applied to A* path distance in ghost mode. Default 1.8 = raw
    /// A* path distance with congestion inflation.
    pub nav_factor: f64,

    // -- Validation ------------------------------------------------------
    /// If true, skip the pre-simulation inventory completability check.
    /// When false (default), scenario generation will fail if any SKU's
    /// total demand across all orders exceeds its total supply in inventory.
    pub waive_completability_check: bool,

    /// HTM-only mode: HTMs pick totes directly from InRack positions,
    /// bypassing VTM fetch entirely. VTM dispatch is suppressed.
    pub htm_only: bool,

    // -- Choreography --------------------------------------------------------
    /// Station pipeline queue factor. Controls the arrival timing of HTMs
    /// at a station relative to pick completion.
    /// 1.0 = next HTM arrives exactly as current pick departs (zero gap).
    /// >1.0 = arrives early (buffer absorbs variance).
    /// <1.0 = arrives late (deliberate gap).
    #[serde(alias = "htm_queue_factor")]
    pub queue_factor: f64,
    /// VTM pipeline depth per station (unused in v1; reserved for future VTM
    /// coordination). Kept for config-file compatibility.
    pub vtm_queue_factor: f64,

    // -- Metrics ------------------------------------------------------------
    /// Metrics time-slice width in milliseconds. Default 10_000 (10s).
    pub slice_width_ms: u64,
    /// Whether to record raw per-bot `BusyInterval` timelines in the
    /// observer's output. Off by default (saves memory on bench runs); set
    /// true by desvis so the Gantt UI has timeline data to render.
    #[serde(default)]
    pub record_robot_timelines: bool,

    // -- Diagnostics ---------------------------------------------------------
    /// Disable the task completion watchdog. When true, no watchdog timeouts
    /// are scheduled. Useful for profiling or debugging stuck simulations.
    pub no_watchdog: bool,
    /// Enable strict throughput diagnostic tripwires. When false (default),
    /// the station-presentation gap-abort and tight pathfinder horizons are
    /// relaxed so cencora-scale runs complete even if throughput is poor.
    /// When true, the planner errors on these conditions — useful while
    /// tuning chain timing or debugging timing-drift regressions. Set via
    /// `--tripwires` CLI arg.
    #[serde(default)]
    pub tripwires: bool,
    /// Wall-clock timeout in seconds. When set, the simulation stops
    /// gracefully after this many seconds of real time have elapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_timeout_secs: Option<f64>,

    /// Print all simulation events to stderr. Enabled by --events CLI flag.
    #[serde(default)]
    pub event_log: bool,

    /// Structured internal trace output for simulator diagnostics/profiling.
    #[serde(default)]
    pub trace: TraceConfig,

    /// Maximum number of orders to include. When set, orders are truncated
    /// after scenario resolution. Works for all scenario types (Generated,
    /// Inline, Csv).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_orders: Option<usize>,

    // -- Pathfinding --------------------------------------------------------
    /// Runtime model for HTM pathfinding, derived from `floor_drive.kind`
    /// when the world states it. The settings selector remains only for older
    /// worlds; it cannot override a catalog model. VTMs use their cycle model.
    #[serde(default)]
    pub motion_model: MotionModel,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            composition: Default::default(),
            fidelity: FidelityMode::default(),
            sim_duration: Duration::from_secs(4 * 3600),
            stagger_interval: Duration::from_millis(500),
            rng_seed: 0,
            num_threads: 1,
            num_htms: 0,
            num_rtps: 0,
            vtm: VtmConfig::default(),
            htm: HtmConfig::default(),
            relay: RelayConfig::default(),
            order: OrderConfig::default(),
            pick: PickConfig::default(),
            operator: OperatorConfig::default(),
            vtm_scoring: VtmScoringConfig::default(),
            putaway: PutawayConfig::default(),
            barrier: BarrierConfig::default(),
            nav_factor: 1.0,
            waive_completability_check: false,
            htm_only: false,
            queue_factor: 1.0,
            vtm_queue_factor: 0.0,
            slice_width_ms: 10_000,
            record_robot_timelines: false,
            no_watchdog: false,
            tripwires: false,
            wall_timeout_secs: None,
            event_log: false,
            trace: TraceConfig::default(),
            max_orders: None,
            motion_model: MotionModel::default(),
        }
    }
}

impl SimConfig {
    /// Refuse a stated figure the simulation cannot use, such as a negative
    /// time or a speed of zero, naming its settings key. Every run calls this
    /// once, where its scenario is built
    /// (`scenario_gen::build_scenario_data`), after the spec's settings and any
    /// `--settings` overlay have landed, so both sources meet the same rule.
    pub fn validate(&self) -> Result<(), InvalidFigure> {
        self.vtm.validate()?;
        self.htm.validate()?;
        self.putaway.validate()
    }
}
