//! Single-bot corridor pathfinding under full-horizon SIPP. Pins the
//! search expansion shape (heuristic + tie-break + perpendicular
//! intermediates from the U-turn fix) and the committed reservation
//! count against unintended pivots in successor emission, dedup, or
//! commit composition. The pre-pivot windowed-era stats
//! (`cells_beyond_depth`, `optimistic_tail_committed`) are absent —
//! every cell is validated, every commit is all-or-nothing.

use super::prelude::*;

#[test]
fn no_conflicts_finds_spatial_path() {
    let graph = corridor(10);
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
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let (path, _stats) = pf.find_path_reserved(c(0), Orientation::East, c(9), drive, &oracle, &config).expect("path");

    use Orientation::{East, North, South, West};
    // 9-cell forward chain with 2-rotate fanout per intermediate
    // (perpendicular intermediates from the U-turn fix). The forward path
    // dominates by f-score throughout, so the perpendicular states are
    // pushed but never popped.
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            EntryIntervals { cell: c(2), intervals: [(t(0), None)].into() },
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
            Rotate(c(2), East, North, ms(3_000), ms(0)),
            Rotate(c(2), East, South, ms(3_000), ms(0)),
            EntryIntervals { cell: c(3), intervals: [(t(0), None)].into() },
            Move(c(2), c(3), ms(3_000), ms(0)),
            Expand { cell: c(3), orientation: East, g: ms(3_000), step: 0 },
            Rotate(c(3), East, North, ms(4_000), ms(0)),
            Rotate(c(3), East, South, ms(4_000), ms(0)),
            EntryIntervals { cell: c(4), intervals: [(t(0), None)].into() },
            Move(c(3), c(4), ms(4_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(4), East, North, ms(5_000), ms(0)),
            Rotate(c(4), East, South, ms(5_000), ms(0)),
            EntryIntervals { cell: c(5), intervals: [(t(0), None)].into() },
            Move(c(4), c(5), ms(5_000), ms(0)),
            Expand { cell: c(5), orientation: East, g: ms(5_000), step: 0 },
            Rotate(c(5), East, North, ms(6_000), ms(0)),
            Rotate(c(5), East, South, ms(6_000), ms(0)),
            EntryIntervals { cell: c(6), intervals: [(t(0), None)].into() },
            Move(c(5), c(6), ms(6_000), ms(0)),
            Expand { cell: c(6), orientation: East, g: ms(6_000), step: 0 },
            Rotate(c(6), East, North, ms(7_000), ms(0)),
            Rotate(c(6), East, South, ms(7_000), ms(0)),
            EntryIntervals { cell: c(7), intervals: [(t(0), None)].into() },
            Move(c(6), c(7), ms(7_000), ms(0)),
            Expand { cell: c(7), orientation: East, g: ms(7_000), step: 0 },
            Rotate(c(7), East, North, ms(8_000), ms(0)),
            Rotate(c(7), East, South, ms(8_000), ms(0)),
            EntryIntervals { cell: c(8), intervals: [(t(0), None)].into() },
            Move(c(7), c(8), ms(8_000), ms(0)),
            Expand { cell: c(8), orientation: East, g: ms(8_000), step: 0 },
            Rotate(c(8), East, North, ms(9_000), ms(0)),
            Rotate(c(8), East, South, ms(9_000), ms(0)),
            EntryIntervals { cell: c(9), intervals: [(t(0), None)].into() },
            Move(c(8), c(9), ms(9_000), ms(0)),
            Expand { cell: c(9), orientation: East, g: ms(9_000), step: 0 },
        ]
    );
    assert_waypoints!(
        path,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            mv(c(3), East, t(2_000), t(3_000)),
            mv(c(4), East, t(3_000), t(4_000)),
            mv(c(5), East, t(4_000), t(5_000)),
            mv(c(6), East, t(5_000), t(6_000)),
            mv(c(7), East, t(6_000), t(7_000)),
            mv(c(8), East, t(7_000), t(8_000)),
            mv(c(9), East, t(8_000), t(9_000)),
            path_end(c(9), East, t(9_000)),
        ]
    );
    assert_eq!(path.travel_time, ms(9_000));
}

#[test]
fn conflict_in_corridor_inserts_wait() {
    let graph = corridor(10);
    let (pf, trace) = pathfinder_traced(&graph);
    // c(3) blocked [2_000, 5_000). The bot waits at c(2) for the cell to
    // clear, then steps in at t=5_000.
    let oracle = ScriptedConflicts::new(vec![(c(3), t(2_000), t(5_000))]);

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
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let (path, _stats) =
        pf.find_path_reserved(c(0), Orientation::East, c(9), drive, &oracle, &config).expect("path with wait");

    use Orientation::{East, North, South, West};
    // c(3)'s EntryIntervals returns two intervals: free (0, 2_000) and
    // free (5_000, ∞). The first interval doesn't admit (m_in_start would
    // land at the boundary of a half-open window); the second pushes a
    // delayed Move at g=6_000 with wait_at_source=3_000. After the wait
    // pushes goalward, the perpendicular states (c(1)/c(2) North/South,
    // c(1) West) become non-dominated and pop briefly; their successors
    // either backtrack to c(0)/c(1) or fail traversability checks. The
    // forward chain resumes from c(3) E onwards.
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            EntryIntervals { cell: c(2), intervals: [(t(0), None)].into() },
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
            Rotate(c(2), East, North, ms(3_000), ms(0)),
            Rotate(c(2), East, South, ms(3_000), ms(0)),
            EntryIntervals { cell: c(3), intervals: [(t(0), Some(t(2_000))), (t(5_000), None)].into() },
            Move(c(2), c(3), ms(6_000), ms(3_000)),
            Expand { cell: c(1), orientation: North, g: ms(2_000), step: 0 },
            Rotate(c(1), North, West, ms(3_000), ms(0)),
            Expand { cell: c(1), orientation: South, g: ms(2_000), step: 0 },
            Expand { cell: c(2), orientation: North, g: ms(3_000), step: 0 },
            Rotate(c(2), North, West, ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: South, g: ms(3_000), step: 0 },
            Expand { cell: c(1), orientation: West, g: ms(3_000), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(0), None)].into() },
            Move(c(1), c(0), ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: West, g: ms(4_000), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Expand { cell: c(3), orientation: East, g: ms(6_000), step: 0 },
            Rotate(c(3), East, North, ms(7_000), ms(0)),
            Rotate(c(3), East, South, ms(7_000), ms(0)),
            EntryIntervals { cell: c(4), intervals: [(t(0), None)].into() },
            Move(c(3), c(4), ms(7_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(7_000), step: 0 },
            Rotate(c(4), East, North, ms(8_000), ms(0)),
            Rotate(c(4), East, South, ms(8_000), ms(0)),
            EntryIntervals { cell: c(5), intervals: [(t(0), None)].into() },
            Move(c(4), c(5), ms(8_000), ms(0)),
            Expand { cell: c(5), orientation: East, g: ms(8_000), step: 0 },
            Rotate(c(5), East, North, ms(9_000), ms(0)),
            Rotate(c(5), East, South, ms(9_000), ms(0)),
            EntryIntervals { cell: c(6), intervals: [(t(0), None)].into() },
            Move(c(5), c(6), ms(9_000), ms(0)),
            Expand { cell: c(6), orientation: East, g: ms(9_000), step: 0 },
            Rotate(c(6), East, North, ms(10_000), ms(0)),
            Rotate(c(6), East, South, ms(10_000), ms(0)),
            EntryIntervals { cell: c(7), intervals: [(t(0), None)].into() },
            Move(c(6), c(7), ms(10_000), ms(0)),
            Expand { cell: c(7), orientation: East, g: ms(10_000), step: 0 },
            Rotate(c(7), East, North, ms(11_000), ms(0)),
            Rotate(c(7), East, South, ms(11_000), ms(0)),
            EntryIntervals { cell: c(8), intervals: [(t(0), None)].into() },
            Move(c(7), c(8), ms(11_000), ms(0)),
            Expand { cell: c(8), orientation: East, g: ms(11_000), step: 0 },
            Rotate(c(8), East, North, ms(12_000), ms(0)),
            Rotate(c(8), East, South, ms(12_000), ms(0)),
            EntryIntervals { cell: c(9), intervals: [(t(0), None)].into() },
            Move(c(8), c(9), ms(12_000), ms(0)),
            Expand { cell: c(9), orientation: East, g: ms(12_000), step: 0 },
        ]
    );
    // Bot waits at c(2) (the predecessor of the blocked c(3)).
    assert_waypoints!(
        path,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            wait(c(2), East, t(2_000), t(5_000)),
            mv(c(3), East, t(5_000), t(6_000)),
            mv(c(4), East, t(6_000), t(7_000)),
            mv(c(5), East, t(7_000), t(8_000)),
            mv(c(6), East, t(8_000), t(9_000)),
            mv(c(7), East, t(9_000), t(10_000)),
            mv(c(8), East, t(10_000), t(11_000)),
            mv(c(9), East, t(11_000), t(12_000)),
            path_end(c(9), East, t(12_000)),
        ]
    );
    assert_eq!(path.travel_time, ms(12_000));
}

/// Regression for the single-shot pivot: the pre-pivot windowed search
/// truncated reservation checks past `collision_depth`, so a conflict
/// 7 cells in was "ignored" and the bot would phantom-pass through it.
/// Under full-horizon SIPP every cell is validated, so a conflict at
/// c(7) produces a wait of equal magnitude to a conflict at c(3) — the
/// only difference is which cell the bot waits at.
#[test]
fn conflict_far_in_corridor_still_validated() {
    let graph = corridor(10);
    let (pf, trace) = pathfinder_traced(&graph);
    // c(7) blocked [6_000, 9_000). 7 hops to reach c(7), so the bot
    // would arrive at t=7_000 — colliding. It must wait at c(6) until
    // c(7) clears at t=9_000.
    let oracle = ScriptedConflicts::new(vec![(c(7), t(6_000), t(9_000))]);

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
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let (path, _stats) =
        pf.find_path_reserved(c(0), Orientation::East, c(9), drive, &oracle, &config).expect("path with wait");

    use Orientation::{East, North, South, West};
    // The forward chain expands c(0)..c(6) before encountering the
    // conflict — c(7)'s EntryIntervals returns (0, 6_000) finite and
    // (9_000, ∞) open. The first interval doesn't admit (boundary).
    // The second pushes Move-into-c(7) at g=10_000 with wait_at_source=
    // 3_000. After that, every perpendicular state pushed during the
    // forward expansion (c(1)..c(6) North/South/West) becomes
    // non-dominated and pops briefly — none reach the goal because
    // they either backtrack or hit traversability dead-ends. Search
    // resumes forward from c(7) E and reaches the goal at g=12_000.
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            EntryIntervals { cell: c(2), intervals: [(t(0), None)].into() },
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
            Rotate(c(2), East, North, ms(3_000), ms(0)),
            Rotate(c(2), East, South, ms(3_000), ms(0)),
            EntryIntervals { cell: c(3), intervals: [(t(0), None)].into() },
            Move(c(2), c(3), ms(3_000), ms(0)),
            Expand { cell: c(3), orientation: East, g: ms(3_000), step: 0 },
            Rotate(c(3), East, North, ms(4_000), ms(0)),
            Rotate(c(3), East, South, ms(4_000), ms(0)),
            EntryIntervals { cell: c(4), intervals: [(t(0), None)].into() },
            Move(c(3), c(4), ms(4_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(4), East, North, ms(5_000), ms(0)),
            Rotate(c(4), East, South, ms(5_000), ms(0)),
            EntryIntervals { cell: c(5), intervals: [(t(0), None)].into() },
            Move(c(4), c(5), ms(5_000), ms(0)),
            Expand { cell: c(5), orientation: East, g: ms(5_000), step: 0 },
            Rotate(c(5), East, North, ms(6_000), ms(0)),
            Rotate(c(5), East, South, ms(6_000), ms(0)),
            EntryIntervals { cell: c(6), intervals: [(t(0), None)].into() },
            Move(c(5), c(6), ms(6_000), ms(0)),
            Expand { cell: c(6), orientation: East, g: ms(6_000), step: 0 },
            Rotate(c(6), East, North, ms(7_000), ms(0)),
            Rotate(c(6), East, South, ms(7_000), ms(0)),
            EntryIntervals { cell: c(7), intervals: [(t(0), Some(t(6_000))), (t(9_000), None)].into() },
            Move(c(6), c(7), ms(10_000), ms(3_000)),
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
            Expand { cell: c(1), orientation: West, g: ms(3_000), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(0), None)].into() },
            Move(c(1), c(0), ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: West, g: ms(4_000), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Expand { cell: c(3), orientation: West, g: ms(5_000), step: 0 },
            EntryIntervals { cell: c(2), intervals: [(t(0), None)].into() },
            Expand { cell: c(4), orientation: West, g: ms(6_000), step: 0 },
            EntryIntervals { cell: c(3), intervals: [(t(0), None)].into() },
            Expand { cell: c(5), orientation: West, g: ms(7_000), step: 0 },
            EntryIntervals { cell: c(4), intervals: [(t(0), None)].into() },
            Expand { cell: c(6), orientation: West, g: ms(8_000), step: 0 },
            EntryIntervals { cell: c(5), intervals: [(t(0), None)].into() },
            Expand { cell: c(7), orientation: East, g: ms(10_000), step: 0 },
            Rotate(c(7), East, North, ms(11_000), ms(0)),
            Rotate(c(7), East, South, ms(11_000), ms(0)),
            EntryIntervals { cell: c(8), intervals: [(t(0), None)].into() },
            Move(c(7), c(8), ms(11_000), ms(0)),
            Expand { cell: c(8), orientation: East, g: ms(11_000), step: 0 },
            Rotate(c(8), East, North, ms(12_000), ms(0)),
            Rotate(c(8), East, South, ms(12_000), ms(0)),
            EntryIntervals { cell: c(9), intervals: [(t(0), None)].into() },
            Move(c(8), c(9), ms(12_000), ms(0)),
            Expand { cell: c(9), orientation: East, g: ms(12_000), step: 0 },
        ]
    );
    // Bot waits at c(6) — same magnitude wait as the c(3)-conflict case,
    // just at a later cell. The wait location is the regression payload.
    assert_waypoints!(
        path,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            mv(c(3), East, t(2_000), t(3_000)),
            mv(c(4), East, t(3_000), t(4_000)),
            mv(c(5), East, t(4_000), t(5_000)),
            mv(c(6), East, t(5_000), t(6_000)),
            wait(c(6), East, t(6_000), t(9_000)),
            mv(c(7), East, t(9_000), t(10_000)),
            mv(c(8), East, t(10_000), t(11_000)),
            mv(c(9), East, t(11_000), t(12_000)),
            path_end(c(9), East, t(12_000)),
        ]
    );
    assert_eq!(path.travel_time, ms(12_000));
}

/// `reserve_path` commits the full chain in a single atomic operation.
/// The commit plan opens an epoch per cell advance — 10 epochs for a
/// 10-cell corridor with a fresh start (no predecessor), each cell's
/// reservation overlapping the next during the straddle phase. The goal
/// cell hold extends `dest_dwell` past arrival.
#[test]
fn reserve_path_commits_full_chain() {
    let graph = corridor(10);
    let pf = pathfinder(&graph);
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
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved =
        pf.reserve_path_without_waiting(c(0), Orientation::East, c(9), drive, &oracle, &config).expect("reserved");

    use Orientation::East;
    assert_waypoints!(
        reserved.path,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            mv(c(3), East, t(2_000), t(3_000)),
            mv(c(4), East, t(3_000), t(4_000)),
            mv(c(5), East, t(4_000), t(5_000)),
            mv(c(6), East, t(5_000), t(6_000)),
            mv(c(7), East, t(6_000), t(7_000)),
            mv(c(8), East, t(7_000), t(8_000)),
            mv(c(9), East, t(8_000), t(9_000)),
            path_end(c(9), East, t(9_000)),
        ]
    );
    assert_eq!(reserved.committed_reservations.len(), 10);
    assert_eq!(reserved.path.travel_time, ms(9_000));
    let goal = reserved.committed_reservations.iter().find(|res| res.cell() == c(9)).expect("goal cell reservation");
    assert_eq!(goal.window().start(), t(8_000));
    assert_eq!(goal.window().end(), Some(t(10_000)));
}

/// TOCTOU regression guard: the oracle reports cells free during search
/// but blocks at commit time. Search succeeds (admits the path); commit
/// hard-fails with `CommitConflict`. There is no soft-tail fallback —
/// the single-shot allocator surfaces commit drift as a hard error.
#[test]
fn commit_conflict_when_oracle_disagrees() {
    let graph = corridor(10);
    let pf = pathfinder(&graph);
    let oracle = CommitBlockingOracle::new(vec![c(2)]);

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
        max_iterations: 100_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.reserve_path_without_waiting(c(0), Orientation::East, c(9), drive, &oracle, &config);

    assert!(matches!(result, Err(PathfinderError::CommitConflict { .. })));
}
