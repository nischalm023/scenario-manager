//! Spatial-only `find_path`, basic `find_path_reserved`, error paths,
//! `direction_between`, and the VTM-ghost-mode regression.

use super::prelude::*;

/// Pinned traces in `tests/{basic, dest_only, directional_lane,
/// unladen_only_edges}.rs` encode `BucketQueue`'s tie-break. If
/// `find_path`'s open set changes, paste-back per
/// `sim/dessim/docs/test-conventions.md`.
#[test]
fn find_path_uses_bucket_queue() {
    let src = include_str!("../interface.rs");
    assert!(
        src.contains("let mut open = BucketQueue::with_capacity("),
        "find_path open set is no longer BucketQueue. Paste-back the new traces \
         in tests/{{basic, dest_only, directional_lane, unladen_only_edges}}.rs \
         per sim/dessim/docs/test-conventions.md.",
    );
}

#[test]
fn straight_line_path() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO).unwrap();

    // c(1) is a bidi corridor cell (neighbors c(0) west, c(2) east), so
    // useful_orients includes the perpendicular intermediates needed for
    // a possible U-turn even though the bot ends up moving forward.
    use Orientation::{East, North, South};
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            path_end(c(2), East, t(2_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(2_000));
}

#[test]
fn straight_line_with_initial_turn() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced(&graph);

    // West → East at c(0) requires two 90° rotations (search emits only
    // 1-turn rotations). 2 turns × 1 s + 2 moves × 1 s = 4 s.
    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.find_path(c(0), Orientation::West, c(2), drive, SimTime::ZERO).unwrap();

    use Orientation::{East, North, South, West};
    // Tie-break: BucketQueue is LIFO within equal-f, so successors pushed
    // last (South) pop first. W→East via the S intermediate wins.
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: West, g: ms(0), step: 0 },
            Rotate(c(0), West, North, ms(1_000), ms(0)),
            Rotate(c(0), West, South, ms(1_000), ms(0)),
            Expand { cell: c(0), orientation: South, g: ms(1_000), step: 0 },
            Rotate(c(0), South, East, ms(2_000), ms(0)),
            Expand { cell: c(0), orientation: North, g: ms(1_000), step: 0 },
            Expand { cell: c(0), orientation: East, g: ms(2_000), step: 0 },
            Move(c(0), c(1), ms(3_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(3_000), step: 0 },
            Rotate(c(1), East, North, ms(4_000), ms(0)),
            Rotate(c(1), East, South, ms(4_000), ms(0)),
            Move(c(1), c(2), ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(4_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(0), West, t(0)),
            rotate(c(0), South, t(0), t(1_000)),
            rotate(c(0), East, t(1_000), t(2_000)),
            mv(c(1), East, t(2_000), t(3_000)),
            mv(c(2), East, t(3_000), t(4_000)),
            path_end(c(2), East, t(4_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(4_000));
}

#[test]
fn grid_path_with_turn() {
    let graph = grid(3, 3);
    let (pf, trace) = pathfinder_traced(&graph);

    // (0,0) East → (2,2). Optimal: 4 moves + 1 turn = 5 s. The L1 heuristic
    // gives row-first and column-first L-paths identical h-values, so A*
    // can't prune either branch and the trace fans out across the 3×3
    // grid before the goal is accepted. The verbosity is intentional —
    // any heuristic upgrade (cached reverse Dijkstra, orientation-aware,
    // etc.) will collapse this trace and the diff is the receipt.
    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.find_path(c(0), Orientation::East, c(8), drive, SimTime::ZERO).unwrap();

    // Both row-first and column-first L-paths are equal-cost; BucketQueue's
    // LIFO tie-break picks the row-first chain (single rotate at c(2)).
    use Orientation::{East, North, South, West};
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            Rotate(c(0), East, South, ms(1_000), ms(0)),
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
            Rotate(c(2), East, North, ms(3_000), ms(0)),
            Rotate(c(2), East, South, ms(3_000), ms(0)),
            Expand { cell: c(2), orientation: South, g: ms(3_000), step: 0 },
            Rotate(c(2), South, West, ms(4_000), ms(0)),
            Move(c(2), c(5), ms(4_000), ms(0)),
            Expand { cell: c(5), orientation: South, g: ms(4_000), step: 0 },
            Rotate(c(5), South, East, ms(5_000), ms(0)),
            Rotate(c(5), South, West, ms(5_000), ms(0)),
            Move(c(5), c(8), ms(5_000), ms(0)),
            Expand { cell: c(8), orientation: South, g: ms(5_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            rotate(c(2), South, t(2_000), t(3_000)),
            mv(c(5), South, t(3_000), t(4_000)),
            mv(c(8), South, t(4_000), t(5_000)),
            path_end(c(8), South, t(5_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(5_000));
}

/// Regression for the U-turn correctness gap in `useful_orients` (fix
/// in 85040f8 added the `opposite_reachable` clause). A bot in a bidi
/// corridor facing away from its goal must be able to synthesize a
/// 180-degree rotation through a perpendicular intermediate. Without
/// the fix, every cell along the corridor prunes its perpendicular
/// intermediates (current_orient is in reachable, so old formula
/// dropped the U-turn intermediates), the bot can only move forward,
/// and `find_path` returns `Unreachable`.
///
/// This test only asserts the path exists and has the right shape.
/// The trace is intentionally not pinned — the regression class is
/// "path becomes unreachable", not a specific exploration order.
#[test]
fn u_turn_in_bidi_corridor_succeeds() {
    let graph = corridor(3);
    let pf = pathfinder(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(2), Orientation::East, c(0), drive, SimTime::ZERO)
        .expect("U-turn through perpendicular intermediate must remain feasible");

    use Orientation::{East, South, West};
    assert_waypoints!(
        result,
        [
            path_start(c(2), East, t(0)),
            rotate(c(2), South, t(0), t(1_000)),
            rotate(c(2), West, t(1_000), t(2_000)),
            mv(c(1), West, t(2_000), t(3_000)),
            mv(c(0), West, t(3_000), t(4_000)),
            path_end(c(0), West, t(4_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(4_000));
}

#[test]
fn reservation_aware_path_emits_explicit_rotate_before_move() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced(&graph);
    let oracle = TableOracle::new(ReservationTable::default());
    let config = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(1_000)),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 5_000_000,
        motion_model: MotionModel::ConstantSpeed,
    };

    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved = pf
        .reserve_path_without_waiting(c(0), Orientation::North, c(2), drive, &oracle, &config)
        .expect("reservation-aware path should commit");

    use Orientation::{East, North, South};
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: North, g: ms(0), step: 0 },
            Rotate(c(0), North, East, ms(1_000), ms(0)),
            Expand { cell: c(0), orientation: East, g: ms(1_000), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Move(c(0), c(1), ms(2_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(2_000), step: 0 },
            Rotate(c(1), East, North, ms(3_000), ms(0)),
            Rotate(c(1), East, South, ms(3_000), ms(0)),
            EntryIntervals { cell: c(2), intervals: [(t(0), None)].into() },
            Move(c(1), c(2), ms(3_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(3_000), step: 0 },
        ]
    );
    // Path describes motion only — terminates at PathEnd at the arrival
    // instant. The dest_dwell is a reservation property and shows up in
    // the goal cell's committed window below.
    assert_waypoints!(
        reserved.path,
        [
            path_start(c(0), North, t(0)),
            rotate(c(0), East, t(0), t(1_000)),
            mv(c(1), East, t(1_000), t(2_000)),
            mv(c(2), East, t(2_000), t(3_000)),
            path_end(c(2), East, t(3_000)),
        ]
    );
    // Goal cell hold extends from move-in (t=2_000) to arrival + dwell
    // (t=3_000 + 1_000 = t=4_000).
    let goal_res = reserved.committed_reservations.iter().find(|r| r.cell() == c(2)).expect("goal cell reservation");
    assert_eq!(goal_res.window().start(), t(2_000));
    assert_eq!(goal_res.window().end(), Some(t(4_000)));
}

/// `dest_dwell = Some(SimDuration::ZERO)` is a degenerate-but-finite
/// terminal hold — search admits it (`expand.rs::try_accept_goal` checks
/// `[m_in_start, m_in_end + 0)` against the goal interval), and the commit
/// plan must agree by emitting `Some(PathEnd.time + 0)`. The pre-Codex-
/// finding-1 filter `> ZERO && != MAX` would have left the goal cell hold
/// open-ended here, drifting from search.
#[test]
fn dest_dwell_zero_holds_until_arrival() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced(&graph);
    let oracle = TableOracle::new(ReservationTable::default());
    let config = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(SimDuration::ZERO),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 5_000_000,
        motion_model: MotionModel::ConstantSpeed,
    };

    // Bound the goal interval to prove the search did NOT need an
    // open-ended free interval to commit zero dwell.
    oracle
        .commit_path(r(99), &CommitPlan::single(ReservationRequest::spanning(c(2), t(10_000), None)), None)
        .expect("blocker hold should commit");

    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved = pf
        .reserve_path_without_waiting(c(0), Orientation::East, c(2), drive, &oracle, &config)
        .expect("zero dwell should commit a finite goal hold ending at arrival");

    assert_waypoints!(
        reserved.path,
        [
            path_start(c(0), Orientation::East, t(0)),
            mv(c(1), Orientation::East, t(0), t(1_000)),
            mv(c(2), Orientation::East, t(1_000), t(2_000)),
            path_end(c(2), Orientation::East, t(2_000)),
        ]
    );
    let _ = trace; // search trace not the focus here; correctness is the
    // committed goal-hold window.
    let goal_res = reserved.committed_reservations.iter().find(|r| r.cell() == c(2)).expect("goal cell reservation");
    assert_eq!(goal_res.window().start(), t(1_000));
    assert_eq!(goal_res.window().end(), Some(t(2_000)));
}

/// Pre-pivot bug c5f960f: VTM `travel()` passed a reservation table to
/// `compute_travel`, blocking VTMs on HTM reservations. Fix moved VTM
/// pathing to spatial-only `find_path`, which never consults the oracle.
#[test]
fn vtm_ghost_mode_ignores_htm_reservations() {
    let graph = corridor(3);
    let trace = TraceCollector::new();
    let pf = pathfinder_speed(&graph, 1500).with_trace_hook(trace.hook());

    let table = ReservationTable::default();
    table.reserve_cell(r(99), c(1), TimeRange::open_ended(t(0))).expect("HTM open-ended hold");
    let oracle = TableOracle::new(table);
    assert!(oracle.reservations_for_cell(c(1)).iter().any(|res| res.is_open_ended()));

    let drive = &drive_speed(1500, RobotType::Vtm, false);
    let path = pf
        .find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO)
        .expect("VTM spatial pathfind must succeed (no reservations checked)");

    // 1500 mm/s on 1000 mm cells → traverse = 666 ms.
    use Orientation::{East, North, South};
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            Move(c(0), c(1), ms(666), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(666), step: 0 },
            Rotate(c(1), East, North, ms(1_666), ms(0)),
            Rotate(c(1), East, South, ms(1_666), ms(0)),
            Move(c(1), c(2), ms(1_332), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(1_332), step: 0 },
        ]
    );
    assert_waypoints!(
        path,
        [
            path_start(c(0), East, t(0)),
            mv_v(c(1), East, t(0), t(666), 1500.0),
            mv_v(c(2), East, t(666), t(1_332), 1500.0),
            path_end(c(2), East, t(1_332)),
        ]
    );
    assert_eq!(path.travel_time, ms(1_332));
}

#[test]
fn same_cell_returns_zero_length_error() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.find_path(c(1), Orientation::North, c(1), drive, SimTime::ZERO);
    assert_trace!(trace, []);
    assert!(matches!(result, Err(PathfinderError::ZeroLength { cell }) if cell == c(1)));
}

#[test]
fn unreachable_destination_returns_error() {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    // No edge between cells — unreachable.
    let graph = b.build();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.find_path(c(0), Orientation::East, c(1), drive, SimTime::ZERO);
    use Orientation::East;
    // No edge out of c(0): reachable_orients is empty, so useful_orients
    // is also empty — no rotates or moves are emitted. The search exhausts
    // after one Expand.
    assert_trace!(trace, [Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },]);
    assert!(matches!(result, Err(PathfinderError::Unreachable { .. })));
}

#[test]
fn direction_between_grid_cells() {
    let graph = grid(3, 3);

    assert_eq!(direction_between(&graph, c(0), c(1)), Orientation::East);
    assert_eq!(direction_between(&graph, c(1), c(0)), Orientation::West);
    assert_eq!(direction_between(&graph, c(0), c(3)), Orientation::South);
    assert_eq!(direction_between(&graph, c(3), c(0)), Orientation::North);
}
