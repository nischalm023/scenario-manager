//! Shared utilities for fh_sipp single-shot Phase 1 tests.
//!
//! Tiny constructors keep test arithmetic readable: `t(1000)` for `SimTime`,
//! `ms(1000)` for `SimDuration`, `c(7)` for `CellId`, `r(2)` for `RobotId`.
//! Graph builders cover the common topologies (`corridor(n)`, `grid(w, h)`);
//! ad-hoc graphs build inline with `cell()` + `WarehouseGraphBuilder`.
//! Pathfinder fixtures (`pathfinder`, `pathfinder_speed`,
//! `pathfinder_holding`, `pathfinder_traced`, `pathfinder_traced_holding`)
//! wrap `FHSippPathfinder::new`; the `*drive*` helpers build the drive a call
//! passes, with the same shared speed and turn as the fixture they go with.
//!
//! `TraceCollector` + `assert_trace!` collect `TraceEvent`s and assert
//! exact equality. **Never relax exact-match trace assertions** — update
//! the expected event list under the new contract instead.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use desgraph::WarehouseGraph;
use desgraph::graph::{Cell, Edge, WarehouseGraphBuilder};
use desgraph::types::{CellId, CellKind, GridPos, RobotType, ZoneId};
use desmap::{GridBounds, SystemType, WarehouseMap, WarehouseMapBuilder};

use super::super::FHSippPathfinder;
use crate::Orientation;
use crate::path::{Waypoint, WaypointAction};
use crate::pathfinder_trace::TraceEvent;
use crate::reservation::oracle::ReservationOracle;
use crate::{Constant, HtmKinematics, KinematicParams, RobotId, SimDuration, SimTime, Velocity};

// ----------------------------------------------------------------------------
// Tiny constructors
// ----------------------------------------------------------------------------

pub(super) const fn t(value: u64) -> SimTime {
    SimTime::ms(value)
}

pub(super) const fn ms(value: u64) -> SimDuration {
    SimDuration::ms(value)
}

pub(super) const fn c(id: u32) -> CellId {
    CellId(id)
}

pub(super) const fn r(id: u32) -> RobotId {
    RobotId(id)
}

// ----------------------------------------------------------------------------
// Cell + graph builders
// ----------------------------------------------------------------------------

/// Build a graph cell at grid position `(x, y)`. Zone fixed to `ZoneId(0)`;
/// sector / world position / barcode / rack orientation / station all absent.
pub(super) fn cell(id: u32, kind: CellKind, x: u32, y: u32) -> Cell {
    Cell {
        id: c(id),
        kind,
        zone: ZoneId(0),
        sector: None,
        grid_pos: Some(GridPos { x, y }),
        world_pos: None,
        barcode: None,
        htm_rotation_forbidden: false,
        rack_orientation: None,
        station_id: None,
    }
}

/// N-cell linear corridor: all `Open`, 1000 mm bidirectional edges.
pub(super) fn corridor(n: u32) -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    for i in 0..n {
        b.add_cell(cell(i, CellKind::Open, i, 0));
    }
    for i in 0..n.saturating_sub(1) {
        b.add_edge_bidi(c(i), c(i + 1), Edge::uniform(1000));
    }
    b.build()
}

/// W×H 4-connected grid: all `Open`, 1000 mm bidirectional edges.
pub(super) fn grid(w: u32, h: u32) -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    for y in 0..h {
        for x in 0..w {
            b.add_cell(cell(y * w + x, CellKind::Open, x, y));
        }
    }
    for y in 0..h {
        for x in 0..w {
            let id = y * w + x;
            if x + 1 < w {
                b.add_edge_bidi(c(id), c(y * w + x + 1), Edge::uniform(1000));
            }
            if y + 1 < h {
                b.add_edge_bidi(c(id), c((y + 1) * w + x), Edge::uniform(1000));
            }
        }
    }
    b.build()
}

// ----------------------------------------------------------------------------
// Pathfinder fixtures
// ----------------------------------------------------------------------------

/// Wrap a synthetic graph in a minimal WarehouseMap with 1000 mm cell spacing
/// so the pathfinder constructors (which now take `&WarehouseMap`) work.
pub(super) fn map_from(graph: &WarehouseGraph) -> WarehouseMap {
    WarehouseMapBuilder::new(SystemType::Relay)
        .grid_bounds(GridBounds { width: 0, depth: 0, cell_size_mm: 1000.0 })
        .graph(graph.clone())
        .build()
}

/// Default pathfinder: 1000 mm/s, 1 s turn, 1000 mm cell spacing, no holding destinations.
pub(super) fn pathfinder(graph: &WarehouseGraph) -> FHSippPathfinder {
    FHSippPathfinder::new(&map_from(graph), 1000, SimDuration::seconds(1), HashSet::new())
}

/// The constant drive that goes with `pathfinder`, `pathfinder_holding` and
/// the traced fixtures: a robot of `robot_type`, laden when `carrying_cargo`,
/// at the shared 1000 mm/s with 1 s turns. An HTM drives the same uniform
/// round-number profile (no loaded/empty or constant/v_max split), so
/// geometry/logic tests stay decoupled from the M5F production defaults.
pub(super) fn uniform_drive(robot_type: RobotType, carrying_cargo: bool) -> Constant {
    drive_speed_uniform(1000, robot_type, carrying_cargo)
}

/// Pathfinder with a custom shared robot speed (mm/s).
pub(super) fn pathfinder_speed(graph: &WarehouseGraph, speed_mm_per_sec: u64) -> FHSippPathfinder {
    FHSippPathfinder::new(&map_from(graph), speed_mm_per_sec, SimDuration::seconds(1), HashSet::new())
}

/// The constant drive that goes with `pathfinder_speed`: a VTM or RTP at the
/// shared `speed_mm_per_sec` with 1 s turns, and an HTM at the M5F's figures
/// on purpose — `constant_speed_htm.rs`/`kinematic*.rs` rely on this to prove
/// an HTM drives at its own M5F speed regardless of the shared (VTM) speed.
/// Geometry tests that want the HTM itself to run at a chosen round-number
/// speed use `drive_speed_uniform` instead.
pub(super) fn drive_speed(speed_mm_per_sec: u64, robot_type: RobotType, carrying_cargo: bool) -> Constant {
    match robot_type {
        RobotType::Htm => Constant::htm(&reference_htm(), carrying_cargo),
        RobotType::Vtm | RobotType::Rtp => drive_speed_uniform(speed_mm_per_sec, robot_type, carrying_cargo),
    }
}

/// A constant drive where any robot, the HTM included, runs the uniform
/// round-number profile at `speed_mm_per_sec` (1 s turns, no loaded/empty or
/// constant/v_max split), to go with `pathfinder_speed`. For geometry/logic
/// tests whose blocker windows are tuned to a non-default per-cell cadence and
/// need the HTM — not just a VTM — to honor it, decoupled from the M5F
/// production defaults.
pub(super) fn drive_speed_uniform(speed_mm_per_sec: u64, robot_type: RobotType, carrying_cargo: bool) -> Constant {
    Constant {
        speed: Velocity(speed_mm_per_sec as f64),
        turn_duration: SimDuration::seconds(1),
        reversal_duration: None,
        robot_type,
        carrying_cargo,
    }
}

/// Pathfinder with explicit holding cells (required for `dest_dwell = MAX`);
/// otherwise as `pathfinder`.
pub(super) fn pathfinder_holding(graph: &WarehouseGraph, holding: HashSet<CellId>) -> FHSippPathfinder {
    FHSippPathfinder::new(&map_from(graph), 1000, SimDuration::seconds(1), holding)
}

/// The pathfinder for tests that build an HTM's drives from custom kinematics
/// (`Constant::htm`, `Trapezoidal::htm`): shared VTM speed 6000 mm/s, 1 s VTM
/// turn, 1000 mm cells, no holding destinations. Exercises per-run kinematics
/// overrides — the HTM drives at its own configured speed/turn regardless of
/// the shared VTM value.
pub(super) fn pathfinder_kinematics(graph: &WarehouseGraph) -> FHSippPathfinder {
    FHSippPathfinder::new(&map_from(graph), 6000, SimDuration::seconds(1), HashSet::new())
}

/// Default pathfinder with a `TraceCollector` wired in.
pub(super) fn pathfinder_traced(graph: &WarehouseGraph) -> (FHSippPathfinder, TraceCollector) {
    let trace = TraceCollector::new();
    let pf = pathfinder(graph).with_trace_hook(trace.hook());
    (pf, trace)
}

/// `pathfinder_holding` with a `TraceCollector`.
pub(super) fn pathfinder_traced_holding(
    graph: &WarehouseGraph,
    holding: HashSet<CellId>,
) -> (FHSippPathfinder, TraceCollector) {
    let trace = TraceCollector::new();
    let pf = pathfinder_holding(graph, holding).with_trace_hook(trace.hook());
    (pf, trace)
}

// ----------------------------------------------------------------------------
// Waypoint constructors + assert_waypoints!
// ----------------------------------------------------------------------------

pub(super) fn path_start(cell: CellId, orientation: Orientation, time: SimTime) -> Waypoint {
    Waypoint { cell, orientation, action: WaypointAction::PathStart { time } }
}

pub(super) fn path_end(cell: CellId, orientation: Orientation, time: SimTime) -> Waypoint {
    Waypoint { cell, orientation, action: WaypointAction::PathEnd { time } }
}

/// Default-speed (1000 mm/s) Move waypoint used by tests that build their
/// pathfinder via `pathfinder()`. Tests that override speed via
/// `pathfinder_speed(_, v)` should use `mv_v(cell, orient, start, end, v)` so
/// the expected velocities match the reconstructed waypoint.
pub(super) fn mv(cell: CellId, orientation: Orientation, start: SimTime, end: SimTime) -> Waypoint {
    mv_v(cell, orientation, start, end, 1000.0)
}

/// Move waypoint with explicit constant velocity. Use when the test's
/// pathfinder runs at a non-default speed. `entry_velocity_mm_s == exit_velocity_mm_s == v`
/// (constant-speed mode). Kinematic-mode tests construct the action directly
/// via `WaypointAction::Move { ... }`.
pub(super) fn mv_v(cell: CellId, orientation: Orientation, start: SimTime, end: SimTime, v: f64) -> Waypoint {
    Waypoint {
        cell,
        orientation,
        action: WaypointAction::Move { start, end, entry_velocity_mm_s: v, exit_velocity_mm_s: v },
    }
}

/// Constant-speed Move waypoint for an UNLADEN HTM. In `ConstantSpeed` mode
/// an unloaded HTM cruises at the M5F empty chassis max (4500 mm/s) — the
/// reconstructed waypoint carries that velocity regardless of the pathfinder's
/// shared constructor speed. Cell cost is `1000_000 / 4500 = 222` ms per 1000mm.
/// The custom `Waypoint` Debug prints these as bare `mv(...)`; paste-back from a
/// failing HTM assertion must restore the `_htm` suffix by hand.
pub(super) fn mv_htm(cell: CellId, orientation: Orientation, start: SimTime, end: SimTime) -> Waypoint {
    mv_v(cell, orientation, start, end, reference_htm().unloaded.max_velocity_mm_s)
}

/// Constant-speed Move waypoint for a LADEN HTM (carrying a tote). M5F loaded
/// chassis max = 4500 mm/s, field-calibrated to the empty; cell cost
/// `1000_000 / 4500 = 222` ms per 1000mm.
pub(super) fn mv_htm_laden(cell: CellId, orientation: Orientation, start: SimTime, end: SimTime) -> Waypoint {
    mv_v(cell, orientation, start, end, reference_htm().loaded.max_velocity_mm_s)
}

pub(super) fn rotate(cell: CellId, orientation: Orientation, start: SimTime, end: SimTime) -> Waypoint {
    Waypoint { cell, orientation, action: WaypointAction::Rotate { start, end } }
}

pub(super) fn wait(cell: CellId, orientation: Orientation, start: SimTime, end: SimTime) -> Waypoint {
    Waypoint { cell, orientation, action: WaypointAction::Wait { start, end } }
}

/// Assert exact waypoint chain equality. Reads as a chronological narration
/// of the bot's path. Mismatches print a side-by-side diff and a paste-ready
/// `[ ... ]` block of the actual chain (one waypoint per line, formatted via
/// the custom `Debug` on `Waypoint` which emits the test-helper constructor
/// form). Drop the paste-ready block between the `[`...`]` of the failing
/// macro call to relock the assertion.
macro_rules! assert_waypoints {
    ($path:expr, [$($wp:expr),* $(,)?]) => {{
        let expected: Vec<crate::path::Waypoint> = vec![$($wp),*];
        let actual: Vec<crate::path::Waypoint> = $path.waypoints.clone();
        if actual != expected {
            eprintln!("=== WAYPOINTS MISMATCH ===");
            let max = actual.len().max(expected.len());
            for i in 0..max {
                let a = actual.get(i);
                let e = expected.get(i);
                let marker = if a == e { " " } else { "!" };
                eprintln!("{marker} [{i:>2}] actual:   {a:?}");
                eprintln!("{marker}      expected: {e:?}");
            }
            eprintln!();
            eprintln!("=== PASTE-READY EXPECTED (actual chain) ===");
            eprintln!("[");
            for wp in &actual {
                eprintln!("    {wp:?},");
            }
            eprintln!("]");
            panic!("waypoint mismatch: {} actual vs {} expected", actual.len(), expected.len());
        }
    }};
}

pub(super) use assert_waypoints;

// ----------------------------------------------------------------------------
// TraceCollector + assert_trace!
// ----------------------------------------------------------------------------

/// Collects `TraceEvent`s emitted during A\* search for test assertions.
pub(super) struct TraceCollector {
    events: Arc<Mutex<Vec<TraceEvent>>>,
}

impl TraceCollector {
    pub(super) fn new() -> Self {
        Self { events: Arc::new(Mutex::new(Vec::new())) }
    }

    pub(super) fn hook(&self) -> impl Fn(&TraceEvent, Option<&dyn ReservationOracle>) + Send + Sync + 'static {
        let events = self.events.clone();
        move |e, _| events.lock().unwrap().push(e.clone())
    }

    pub(super) fn events(&self) -> Vec<TraceEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// Assert exact `TraceEvent` equality. Each element is `.into()`'d to
/// `TraceEvent`, so any payload struct from
/// `crate::pathfinder_trace::prelude::*` (Expand, Rotate, Move, …)
/// can appear as-is. Prints a side-by-side diff on mismatch and a
/// paste-ready `[ ... ]` block of the actual events (custom `Debug` on
/// `TraceEvent` drops the variant wrapper; field types print their
/// constructor form). Drop the paste-ready block between the `[`...`]`
/// of the failing macro call to relock the assertion. **Never** relax
/// to looser presence checks — update the expected list under the new
/// contract.
macro_rules! assert_trace {
    ($trace:expr, [$($event:expr),* $(,)?]) => {{
        let expected: Vec<crate::pathfinder_trace::TraceEvent> = vec![$($event.into()),*];
        let actual = $trace.events();
        if actual != expected {
            eprintln!("=== TRACE MISMATCH ===");
            let max = actual.len().max(expected.len());
            for i in 0..max {
                let a = actual.get(i);
                let e = expected.get(i);
                let marker = if a == e { " " } else { "!" };
                eprintln!("{marker} [{i:>2}] actual:   {a:?}");
                eprintln!("{marker}      expected: {e:?}");
            }
            eprintln!();
            eprintln!("=== PASTE-READY EXPECTED (actual events) ===");
            eprintln!("[");
            for ev in &actual {
                eprintln!("    {ev:?},");
            }
            eprintln!("]");
            panic!("trace mismatch: {} actual vs {} expected events", actual.len(), expected.len());
        }
    }};
}

pub(super) use assert_trace;

/// The Quicktron M5F's chassis kinematics, as dessim's `robots::reference_htm`
/// reads them from its model file: the fixed robot these tests measure with.
pub fn reference_htm() -> HtmKinematics {
    let params = KinematicParams { max_velocity_mm_s: 4500.0, acceleration_mm_s2: 2000.0, deceleration_mm_s2: 2000.0 };
    HtmKinematics {
        loaded: params,
        unloaded: params,
        loaded_constant_speed_mm_s: 4500.0,
        empty_constant_speed_mm_s: 4500.0,
        loaded_turn: SimDuration::ms(2_100),
        empty_turn: SimDuration::ms(2_100),
        loaded_reversal: None,
        empty_reversal: None,
    }
}
