//! Delayed-move commit semantics: when an intermediate cell is reserved
//! against the bot, single-shot SIPP synthesizes a wait at the
//! predecessor cell and the allocator commits the full chain atomically.
//! The pre-pivot windowed allocator handled this via an extension loop;
//! these tests assert the modern shape — one full `CommitPlan`, no
//! optimistic tail, source cell hold extended to cover the wait.
//!
//! `corridor.rs` already pins the search-decision trace for these
//! scenarios via `find_path_reserved`; these tests focus on the commit
//! side via `reserve_path` and the resulting `CellReservation` windows.

use super::prelude::*;

/// Conflict at c(4) [3_000, 6_000). Bot waits at c(3) for 3 s, then
/// steps in at t=6_000. Commit succeeds atomically with the c(3) hold
/// extended across the wait period.
#[test]
fn delayed_move_at_cell_4_commits_atomically() {
    let graph = corridor(10);
    let pf = pathfinder(&graph);
    let table = ReservationTable::default();
    table.reserve_cell(r(99), c(4), TimeRange::new(t(3_000), t(6_000))).expect("blocker setup");
    let oracle = TableOracle::new(table);

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
            wait(c(3), East, t(3_000), t(6_000)),
            mv(c(4), East, t(6_000), t(7_000)),
            mv(c(5), East, t(7_000), t(8_000)),
            mv(c(6), East, t(8_000), t(9_000)),
            mv(c(7), East, t(9_000), t(10_000)),
            mv(c(8), East, t(10_000), t(11_000)),
            mv(c(9), East, t(11_000), t(12_000)),
            path_end(c(9), East, t(12_000)),
        ]
    );
    assert_eq!(reserved.committed_reservations.len(), 10);
    assert_eq!(reserved.path.travel_time, ms(12_000));

    // c(3) hold runs from leading-edge entry (t=2_000) through the wait
    // to leading-edge exit (Move-out.end = t=7_000). The wait is folded
    // into the cell hold, not a separate reservation.
    let c3_hold = reserved.committed_reservations.iter().find(|res| res.cell() == c(3)).expect("c(3) hold");
    assert_eq!(c3_hold.window().start(), t(2_000));
    assert_eq!(c3_hold.window().end(), Some(t(7_000)));

    // Goal cell hold: arrival at t=12_000, dest_dwell=1_000 → end at t=13_000.
    let goal = reserved.committed_reservations.iter().find(|res| res.cell() == c(9)).expect("goal hold");
    assert_eq!(goal.window().start(), t(11_000));
    assert_eq!(goal.window().end(), Some(t(13_000)));
}

/// Conflict at c(7) [6_000, 9_000). Bot waits at c(6) for 3 s. Same
/// commit semantics as the cell-4 case but with the wait deeper in the
/// corridor — proves single-shot validates and commits the full chain
/// regardless of where in the path the wait sits.
#[test]
fn delayed_move_at_cell_7_commits_atomically() {
    let graph = corridor(10);
    let pf = pathfinder(&graph);
    let table = ReservationTable::default();
    table.reserve_cell(r(99), c(7), TimeRange::new(t(6_000), t(9_000))).expect("blocker setup");
    let oracle = TableOracle::new(table);

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
            wait(c(6), East, t(6_000), t(9_000)),
            mv(c(7), East, t(9_000), t(10_000)),
            mv(c(8), East, t(10_000), t(11_000)),
            mv(c(9), East, t(11_000), t(12_000)),
            path_end(c(9), East, t(12_000)),
        ]
    );
    assert_eq!(reserved.committed_reservations.len(), 10);
    assert_eq!(reserved.path.travel_time, ms(12_000));

    let c6_hold = reserved.committed_reservations.iter().find(|res| res.cell() == c(6)).expect("c(6) hold");
    assert_eq!(c6_hold.window().start(), t(5_000));
    assert_eq!(c6_hold.window().end(), Some(t(10_000)));

    let goal = reserved.committed_reservations.iter().find(|res| res.cell() == c(9)).expect("goal hold");
    assert_eq!(goal.window().start(), t(11_000));
    assert_eq!(goal.window().end(), Some(t(13_000)));
}

/// Non-uniform speed: 1500 mm/s gives 666 ms / cell. c(5) blocked
/// [3_000, 4_000) — direct arrival lands inside the blocker (5 × 666 =
/// 3_330), so the search must delay the move into c(5) to leading-edge
/// entry t=4_000 (= reservation end). Asserts the move-into-c(5) start
/// and that c(5) ends up with exactly two reservations on the table:
/// the blocker plus the bot's transit window. Adapted from the
/// pre-pivot regression of the same name.
#[test]
fn delayed_move_into_busy_cell_records_correct_move_window() {
    let graph = corridor(10);
    let pf = pathfinder_speed(&graph, 1_500);
    let table = ReservationTable::default();
    table.reserve_cell(r(99), c(5), TimeRange::new(t(3_000), t(4_000))).expect("blocker setup");
    let oracle = TableOracle::new(table);

    let config = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(666)),
        collision_depth: 10,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 5_000_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &drive_speed_uniform(1_500, RobotType::Htm, false);
    let reserved = pf
        .reserve_path_without_waiting(c(0), Orientation::East, c(9), drive, &oracle, &config)
        .expect("path should commit atomically");

    use Orientation::East;
    assert_waypoints!(
        reserved.path,
        [
            path_start(c(0), East, t(0)),
            mv_v(c(1), East, t(0), t(666), 1500.0),
            mv_v(c(2), East, t(666), t(1_332), 1500.0),
            mv_v(c(3), East, t(1_332), t(1_998), 1500.0),
            mv_v(c(4), East, t(1_998), t(2_664), 1500.0),
            wait(c(4), East, t(2_664), t(4_000)),
            mv_v(c(5), East, t(4_000), t(4_666), 1500.0),
            mv_v(c(6), East, t(4_666), t(5_332), 1500.0),
            mv_v(c(7), East, t(5_332), t(5_998), 1500.0),
            mv_v(c(8), East, t(5_998), t(6_664), 1500.0),
            mv_v(c(9), East, t(6_664), t(7_330), 1500.0),
            path_end(c(9), East, t(7_330)),
        ]
    );
    assert_eq!(reserved.path.travel_time, ms(7_330));

    // Bot's transit through c(5): leading-edge entry at t=4_000 (= end
    // of blocker), action-ready at c(5) one traverse later (t=4_666),
    // leading-edge exit (start of Move c(5)→c(6)) also at t=4_666, full
    // arrival at c(6) at t=5_332.
    let c5_transit = reserved.committed_reservations.iter().find(|res| res.cell() == c(5)).expect("c(5) transit");
    assert_eq!(c5_transit.window().start(), t(4_000));
    assert_eq!(c5_transit.window().end(), Some(t(5_332)));

    // Table holds two non-overlapping reservations on c(5): the blocker
    // and the bot's transit. Same-cell cross-robot overlap would be a
    // bug; cross-cell same-robot overlap during straddle is by design.
    let c5_all = oracle.reservations_for_cell(c(5));
    assert_eq!(c5_all.len(), 2, "blocker + transit");
    assert!(oracle.find_overlaps().is_empty(), "no cross-robot overlaps after commit");
}
