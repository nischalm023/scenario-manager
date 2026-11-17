//! Multi-bot dwell-stacking: when several robots share a destination
//! with finite dwell, single-shot SIPP serializes them — each robot's
//! goal-cell hold defers the next robot's arrival by at least the dwell
//! duration. The corridor case is small enough to trace-pin; the
//! 10×10 grid and cencora cases assert the behavioral invariants
//! (success, ordering, no overlaps) without locking the search shape.
//!
//! Note: under single-shot, two robots cannot both start at the same
//! cell at the same `sim_start` — the start-cell epoch in
//! `compose_commit_plan` would directly conflict. The pre-pivot
//! windowed allocator's looser semantics tolerated this; the modern
//! tests stagger the second robot's `sim_start` past the first's
//! start-cell hold.

use std::collections::HashSet;

use desmap::WarehouseDesign;
use serde_json::Value;

use super::prelude::*;

use crate::FHSippPathfinder;

/// Two robots head from c(0) to c(9) with 9 s dwell each. Robot A
/// starts at t=0; Robot B starts at t=2_000 (after A clears its
/// start-cell epoch). B's path follows A through the corridor, hitting
/// the goal-cell wait at c(8) where it holds for 8 s before stepping
/// into c(9) at t=18_000. Goal hold is open between the two robots —
/// no overlaps, exactly the dwell-stacking invariant.
#[test]
fn dwell_stacking_two_robots_in_corridor() {
    let graph = corridor(10);
    let pf = pathfinder(&graph);
    let oracle = TableOracle::new(ReservationTable::default());

    let config = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(9_000)),
        collision_depth: 8,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved_a =
        pf.reserve_path_without_waiting(c(0), Orientation::East, c(9), drive, &oracle, &config).expect("A path");
    assert_eq!(reserved_a.path.travel_time, ms(9_000));

    // Fresh traced pathfinder for B so the trace captures only B's
    // search (A is already committed to the shared oracle).
    let trace = TraceCollector::new();
    let pf_b = pathfinder(&graph).with_trace_hook(trace.hook());
    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(2_000),
        dest_dwell: Some(ms(9_000)),
        collision_depth: 8,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved_b =
        pf_b.reserve_path_without_waiting(c(0), Orientation::East, c(9), drive, &oracle, &config).expect("B path");

    use Orientation::{East, North, South, West};
    // B's path: 8 forward hops c(0)..c(8), then 8 s wait at c(8) until
    // A's c(9) hold (`[8_000, 17_000)`) clears at t=17_000 and the half-
    // open boundary admits B's leading-edge entry at t=18_000. PathEnd
    // at t=19_000 (action-ready at goal); travel_time excludes dwell.
    assert_waypoints!(
        reserved_b.path,
        [
            path_start(c(0), East, t(2_000)),
            mv(c(1), East, t(2_000), t(3_000)),
            mv(c(2), East, t(3_000), t(4_000)),
            mv(c(3), East, t(4_000), t(5_000)),
            mv(c(4), East, t(5_000), t(6_000)),
            mv(c(5), East, t(6_000), t(7_000)),
            mv(c(6), East, t(7_000), t(8_000)),
            mv(c(7), East, t(8_000), t(9_000)),
            mv(c(8), East, t(9_000), t(10_000)),
            wait(c(8), East, t(10_000), t(18_000)),
            mv(c(9), East, t(18_000), t(19_000)),
            path_end(c(9), East, t(19_000)),
        ]
    );
    // The forward chain expands c(0)..c(8) cleanly (each cell admits at
    // its own A-shifted free interval starting at t=k+2_000); at c(8)
    // the goal-cell EntryIntervals returns (18_000, ∞) — A's
    // [8_000, 17_000) hold pushes the next free start past t=17_000 and
    // the canonical interval starts at 18_000 (one-traverse leading-edge
    // entry). Push Move c(8)→c(9) at g=17_000 with wait_at_source=8_000.
    // After that, the perpendicular states from earlier in the corridor
    // become non-dominated and pop in cell-order; their successors
    // backtrack toward c(0) without making goalward progress, then a
    // late `Expand c(8) East g=16_000` (a stale entry from the dedup
    // race against the Move-into-c(9) push) and a re-query of c(9)'s
    // intervals confirm no improvement before the goal pop at g=17_000.
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(2_000), None)].into() },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            EntryIntervals { cell: c(2), intervals: [(t(3_000), None)].into() },
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
            Rotate(c(2), East, North, ms(3_000), ms(0)),
            Rotate(c(2), East, South, ms(3_000), ms(0)),
            EntryIntervals { cell: c(3), intervals: [(t(4_000), None)].into() },
            Move(c(2), c(3), ms(3_000), ms(0)),
            Expand { cell: c(3), orientation: East, g: ms(3_000), step: 0 },
            Rotate(c(3), East, North, ms(4_000), ms(0)),
            Rotate(c(3), East, South, ms(4_000), ms(0)),
            EntryIntervals { cell: c(4), intervals: [(t(5_000), None)].into() },
            Move(c(3), c(4), ms(4_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(4), East, North, ms(5_000), ms(0)),
            Rotate(c(4), East, South, ms(5_000), ms(0)),
            EntryIntervals { cell: c(5), intervals: [(t(6_000), None)].into() },
            Move(c(4), c(5), ms(5_000), ms(0)),
            Expand { cell: c(5), orientation: East, g: ms(5_000), step: 0 },
            Rotate(c(5), East, North, ms(6_000), ms(0)),
            Rotate(c(5), East, South, ms(6_000), ms(0)),
            EntryIntervals { cell: c(6), intervals: [(t(7_000), None)].into() },
            Move(c(5), c(6), ms(6_000), ms(0)),
            Expand { cell: c(6), orientation: East, g: ms(6_000), step: 0 },
            Rotate(c(6), East, North, ms(7_000), ms(0)),
            Rotate(c(6), East, South, ms(7_000), ms(0)),
            EntryIntervals { cell: c(7), intervals: [(t(8_000), None)].into() },
            Move(c(6), c(7), ms(7_000), ms(0)),
            Expand { cell: c(7), orientation: East, g: ms(7_000), step: 0 },
            Rotate(c(7), East, North, ms(8_000), ms(0)),
            Rotate(c(7), East, South, ms(8_000), ms(0)),
            EntryIntervals { cell: c(8), intervals: [(t(9_000), None)].into() },
            Move(c(7), c(8), ms(8_000), ms(0)),
            Expand { cell: c(8), orientation: East, g: ms(8_000), step: 0 },
            Rotate(c(8), East, North, ms(9_000), ms(0)),
            Rotate(c(8), East, South, ms(9_000), ms(0)),
            EntryIntervals { cell: c(9), intervals: [(t(18_000), None)].into() },
            Move(c(8), c(9), ms(17_000), ms(8_000)),
            Expand { cell: c(1), orientation: North, g: ms(2_000), step: 0 },
            Rotate(c(1), North, West, ms(3_000), ms(0)),
            Expand { cell: c(1), orientation: South, g: ms(2_000), step: 0 },
            Expand { cell: c(2), orientation: North, g: ms(3_000), step: 0 },
            Rotate(c(2), North, West, ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: South, g: ms(3_000), step: 0 },
            Expand { cell: c(3), orientation: North, g: ms(4_000), step: 0 },
            Rotate(c(3), North, West, ms(5_000), ms(0)),
            Expand { cell: c(3), orientation: South, g: ms(4_000), step: 0 },
            Expand { cell: c(4), orientation: North, g: ms(5_000), step: 0 },
            Rotate(c(4), North, West, ms(6_000), ms(0)),
            Expand { cell: c(4), orientation: South, g: ms(5_000), step: 0 },
            Expand { cell: c(5), orientation: North, g: ms(6_000), step: 0 },
            Rotate(c(5), North, West, ms(7_000), ms(0)),
            Expand { cell: c(5), orientation: South, g: ms(6_000), step: 0 },
            Expand { cell: c(6), orientation: North, g: ms(7_000), step: 0 },
            Rotate(c(6), North, West, ms(8_000), ms(0)),
            Expand { cell: c(6), orientation: South, g: ms(7_000), step: 0 },
            Expand { cell: c(7), orientation: North, g: ms(8_000), step: 0 },
            Rotate(c(7), North, West, ms(9_000), ms(0)),
            Expand { cell: c(7), orientation: South, g: ms(8_000), step: 0 },
            Expand { cell: c(8), orientation: North, g: ms(9_000), step: 0 },
            Rotate(c(8), North, West, ms(10_000), ms(0)),
            Expand { cell: c(8), orientation: South, g: ms(9_000), step: 0 },
            Expand { cell: c(1), orientation: West, g: ms(3_000), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(1_000), None)].into() },
            Move(c(1), c(0), ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: West, g: ms(4_000), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(2_000), None)].into() },
            Expand { cell: c(3), orientation: West, g: ms(5_000), step: 0 },
            EntryIntervals { cell: c(2), intervals: [(t(3_000), None)].into() },
            Expand { cell: c(4), orientation: West, g: ms(6_000), step: 0 },
            EntryIntervals { cell: c(3), intervals: [(t(4_000), None)].into() },
            Expand { cell: c(5), orientation: West, g: ms(7_000), step: 0 },
            EntryIntervals { cell: c(4), intervals: [(t(5_000), None)].into() },
            Expand { cell: c(6), orientation: West, g: ms(8_000), step: 0 },
            EntryIntervals { cell: c(5), intervals: [(t(6_000), None)].into() },
            Expand { cell: c(7), orientation: West, g: ms(9_000), step: 0 },
            EntryIntervals { cell: c(6), intervals: [(t(7_000), None)].into() },
            Expand { cell: c(8), orientation: West, g: ms(10_000), step: 0 },
            EntryIntervals { cell: c(7), intervals: [(t(8_000), None)].into() },
            Expand { cell: c(0), orientation: West, g: ms(4_000), step: 0 },
            Rotate(c(0), West, North, ms(5_000), ms(0)),
            Rotate(c(0), West, South, ms(5_000), ms(0)),
            Expand { cell: c(0), orientation: North, g: ms(5_000), step: 0 },
            Expand { cell: c(0), orientation: South, g: ms(5_000), step: 0 },
            Expand { cell: c(8), orientation: East, g: ms(16_000), step: 0 },
            EntryIntervals { cell: c(9), intervals: [(t(18_000), None)].into() },
            Expand { cell: c(8), orientation: North, g: ms(16_000), step: 0 },
            Expand { cell: c(8), orientation: South, g: ms(16_000), step: 0 },
            Expand { cell: c(8), orientation: West, g: ms(16_000), step: 0 },
            EntryIntervals { cell: c(7), intervals: [(t(8_000), None)].into() },
            Expand { cell: c(9), orientation: East, g: ms(17_000), step: 0 },
        ]
    );
    assert_eq!(reserved_b.path.travel_time, ms(17_000));

    // Goal cell c(9) holds A first ([8_000, 17_000) = arrival + dwell),
    // then B ([18_000, 27_000)). No same-cell overlap.
    assert!(oracle.table().find_overlaps().is_empty(), "no overlaps");
}

/// 10×10 grid: A from (0,0) to (9,9) — 18 hops + 1 turn = 19 s travel
/// + 9 s dwell. B from (1,0) to (9,9) — 17 hops + 1 turn = 18 s travel
/// without conflicts, but A reserves the row-then-column path so B's
/// arrival is shifted past A's dwell. Behavioral assertions only
/// (Manhattan-18 trace would be enormous).
#[test]
fn dwell_stacking_two_robots_on_grid() {
    let graph = grid(10, 10);
    let pf = pathfinder(&graph);
    let oracle = TableOracle::new(ReservationTable::default());

    let config = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(9_000)),
        collision_depth: 8,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 500_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved_a =
        pf.reserve_path_without_waiting(c(0), Orientation::East, c(99), drive, &oracle, &config).expect("A path");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(2_000),
        dest_dwell: Some(ms(9_000)),
        collision_depth: 8,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 500_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved_b =
        pf.reserve_path_without_waiting(c(1), Orientation::East, c(99), drive, &oracle, &config).expect("B path");

    let a_arrival = reserved_a.path.travel_time.as_ms();
    let b_arrival_absolute = reserved_b.path.travel_time.as_ms() + 2_000;
    assert!(
        b_arrival_absolute >= a_arrival + 9_000,
        "B must arrive after A's 9 s dwell: A={a_arrival}ms + 9_000 = {}ms, B={b_arrival_absolute}ms",
        a_arrival + 9_000,
    );
    assert!(oracle.table().find_overlaps().is_empty(), "no overlaps after grid stacking");
}

/// Cencora `phase1` map, 10 robots dispatched serially to the same
/// presentation cell, each with a 9 s dwell. Verifies the single-shot
/// allocator scales to a real warehouse: every robot succeeds, the
/// dwell-stacking gap stays in [9_000, 10_000) ms (one traverse of
/// approach-cell straddle = 333 ms), no cross-robot overlaps. No
/// trace assertions — a full search trace at this scale is brittle.
#[test]
fn cencora_10_robots_same_station() {
    // The v3 file keeps its design flattened at the root beside `scenario`
    // and `settings`; `SimSpec` (which used to strip those) retired with the
    // v3 runtime, so drop them here instead. The map is only scenery for this
    // test -- what it exercises is stacking in the pathfinder.
    let json = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../desmap/tests/fixtures/cencora_phase1.json"));
    let mut value: Value = serde_json::from_str(json).expect("parse cencora_phase1.json");
    if let Some(obj) = value.as_object_mut() {
        obj.remove("scenario");
        obj.remove("settings");
    }
    let design: WarehouseDesign = serde_json::from_value(value).expect("the fixture's design block");
    let map = design.compile().expect("compile cencora map");

    let pf = FHSippPathfinder::new(&map, 1_500, SimDuration::seconds(1), HashSet::new());

    let station = &map.stations[0];
    let dest = station.delivery_cell();
    let start_cells: Vec<CellId> = map.relays.iter().take(10).map(|r| r.htm_access).collect();
    assert!(start_cells.len() >= 10, "need at least 10 relays");

    let oracle = TableOracle::new(ReservationTable::default());
    let mut arrivals: Vec<u64> = Vec::new();

    for (i, &start) in start_cells.iter().enumerate() {
        let robot = r(i as u32 + 1);
        let config = CollisionConfig {
            self_robot: robot,
            predecessor: None,
            destination: None,
            sim_start: t(0),
            dest_dwell: Some(ms(9_000)),
            collision_depth: 8,
            window_size: 0,
            replan_interval: 0,
            max_wait: SimDuration::seconds(120),
            max_iterations: 5_000_000,
            motion_model: MotionModel::ConstantSpeed,
        };
        let drive = &drive_speed_uniform(1_500, RobotType::Htm, false);
        let reserved = pf
            .reserve_path_without_waiting(start, Orientation::East, dest, drive, &oracle, &config)
            .unwrap_or_else(|err| panic!("robot {robot:?} from start {start:?} → dest {dest:?} failed: {err:?}"));
        arrivals.push(reserved.path.travel_time.as_ms());
    }

    assert_eq!(arrivals.len(), 10, "every robot must commit a plan under single-shot");
    for w in arrivals.windows(2) {
        let gap = w[1] - w[0];
        assert!(
            (9_000..=10_000).contains(&gap),
            "consecutive dwell gap out of range: {}ms (expected 9_000..=10_000)",
            gap,
        );
    }
    assert!(oracle.table().find_overlaps().is_empty(), "no overlaps under stacking");
}
