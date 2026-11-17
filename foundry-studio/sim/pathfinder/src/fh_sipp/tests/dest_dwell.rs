//! Terminal-dwell semantics. Under the single-shot contract every path
//! ends at `path_end(...)` regardless of `dest_dwell`; the dwell flows
//! through `compose_commit_plan` to set the goal cell's
//! `CellReservation.window().end`. Finite dwell extends the goal hold past
//! arrival; `MAX` leaves it open-ended (eligible holding destinations only). The `Unreachable`
//! case is the trapped-by-reservations sibling — search exhausts cleanly,
//! no commit attempted.

use super::prelude::*;

#[test]
fn explicit_indefinite_hold_requires_an_eligible_destination() {
    let graph = corridor(2);
    let oracle = TableOracle::new(ReservationTable::default());
    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(MAX),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let denied = pathfinder(&graph)
        .reserve_path_without_waiting(c(1), Orientation::West, c(0), drive, &oracle, &config)
        .unwrap_err();
    assert!(matches!(denied, PathfinderError::MaxDwellNonHolding { cell } if cell == c(0)));
    assert!(oracle.reservations_for_cell(c(0)).is_empty());

    let drive = &uniform_drive(RobotType::Htm, false);
    let allowed = pathfinder_holding(&graph, [c(0)].into_iter().collect())
        .reserve_path_without_waiting(c(1), Orientation::West, c(0), drive, &oracle, &config)
        .expect("declared holding destination accepts the same route");
    assert!(allowed.committed_reservations.last().unwrap().is_open_ended());
}

/// Bot waits at the source cell while the goal is busy, then steps in;
/// finite 9 s dwell extends the goal hold past arrival. The path
/// terminates at `path_end(...)` at arrival; the dwell shows up only on
/// the goal cell's committed window.
#[test]
fn dest_dwell_inflates_check_dur() {
    let graph = corridor(2);
    let (pf, trace) = pathfinder_traced(&graph);
    let oracle = TableOracle::new(ReservationTable::default());

    // r1 holds the goal cell c(0) for [0, 10_000). r2 plans c(1) → c(0)
    // with a 9 s terminal dwell; the wait-and-step path arrives at
    // t=11_000 and the goal hold extends to t=20_000.
    oracle
        .commit_path(r(1), &CommitPlan::single(ReservationRequest::spanning(c(0), t(0), Some(t(10_000)))), None)
        .expect("r1 c0 hold");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(9_000)),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved = pf
        .reserve_path_without_waiting(c(1), Orientation::West, c(0), drive, &oracle, &config)
        .expect("wait-and-step path");

    use Orientation::West;
    // Step 0: pop start. Step 1-2: query c(0) intervals (open-ended at
    // t=10_000), push the delayed Move-into-c(0) at g=11_000 with
    // wait_at_source=10s. Step 3: an event-driven Wait state at
    // wait_label=t(10_000) (goal-event source admits the 9 s dwell). Step 4:
    // re-query c(0) intervals from the wait state — same canonical interval,
    // same g=11_000 successor — dedup'd, no Move push. Step 5: pop the goal.
    assert_trace!(
        trace,
        [
            Expand { cell: c(1), orientation: West, g: ms(0), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(10_000), None)].into() },
            Move(c(1), c(0), ms(11_000), ms(10_000)),
            Expand { cell: c(1), orientation: West, g: ms(10_000), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(10_000), None)].into() },
            Expand { cell: c(0), orientation: West, g: ms(11_000), step: 0 },
        ]
    );
    // Path is motion-only: PathStart → gap-fill Wait at source → Move into
    // goal → PathEnd at arrival. The 9 s dwell never appears as a Wait
    // waypoint — it lives on the goal cell's reservation window only.
    assert_waypoints!(
        reserved.path,
        [
            path_start(c(1), West, t(0)),
            wait(c(1), West, t(0), t(10_000)),
            mv(c(0), West, t(10_000), t(11_000)),
            path_end(c(0), West, t(11_000)),
        ]
    );
    assert_eq!(reserved.path.travel_time, ms(11_000));
    let goal = reserved.committed_reservations.iter().find(|res| res.cell() == c(0)).expect("goal cell reservation");
    assert_eq!(goal.window().start(), t(10_000));
    assert_eq!(goal.window().end(), Some(t(20_000)));
}

/// Trapped-by-reservations: c(0) has a finite gap between r1 and r3 too
/// short for `MAX` dwell, the next open interval starts at t=16_000, but
/// the bot cannot hold its source cell c(1) until then because c(1)'s
/// safe interval ends at t=14_000. Search exhausts after one expand with
/// `src_hold_blocked` recorded.
#[test]
fn wait_cell_not_available_long_enough_to_wait() {
    let graph = corridor(2);
    let (pf, trace) = pathfinder_traced_holding(&graph, [c(0)].into_iter().collect());
    let oracle = TableOracle::new(ReservationTable::default());

    oracle
        .commit_path(
            r(1),
            &CommitPlan::direct(vec![
                ReservationRequest::spanning(c(1), t(0), Some(t(1_000))),
                ReservationRequest::spanning(c(0), t(1), Some(t(10_000))),
            ]),
            None,
        )
        .expect("r1 setup");
    oracle
        .commit_path(
            r(3),
            &CommitPlan::direct(vec![
                ReservationRequest::spanning(c(1), t(14_000), Some(t(15_000))),
                ReservationRequest::spanning(c(0), t(14_001), Some(t(16_000))),
            ]),
            None,
        )
        .expect("r3 setup");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(2_000),
        dest_dwell: Some(MAX),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.reserve_path_without_waiting(c(1), Orientation::West, c(0), drive, &oracle, &config);

    use Orientation::West;
    // The three events are the failed reserved search: query c(0)
    // intervals — finite [10_000, 14_001) can't admit MAX dwell, open
    // [16_000, ∞) requires holding c(1) past its safe-interval end at
    // t=14_000 — both rejected, no Move pushed; the search exhausts after
    // one expand with `src_hold_blocked = 1`. The static-route walk that
    // classifies the failure emits no events; it finds the route, so the
    // failure is `UnreachableWithinHorizon`, not `NoStaticRoute`.
    assert_trace!(
        trace,
        [
            Expand { cell: c(1), orientation: West, g: ms(0), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(10_000), Some(t(14_001))), (t(16_000), None)].into() },
            Exhausted {
                from: c(1),
                to: c(0),
                iterations: 1,
                expanded: 1,
                turns_skipped: 0,
                src_hold_blocked: 1,
                delayed_count: 0,
                permanent_blocked: 0,
            },
        ]
    );
    assert!(matches!(result, Err(PathfinderError::UnreachableWithinHorizon { .. })));
}

/// Bug: source-cell hold must cover the entire move-out, not just the
/// delayed move start.
///
/// r1 holds the goal until 10s. r3 starts using the source cell at 10.5s.
/// The tempting path waits on c(1), starts moving into c(0) at 10s, and
/// arrives at 11s. Search must reject it because the bot's trailing edge
/// still occupies c(1) until 11s, conflicting with r3's [10.5s, 12s)
/// source-cell reservation.
#[test]
fn wait_source_must_be_available_until_move_completion() {
    let graph = corridor(2);
    let pf = pathfinder(&graph);
    let oracle = TableOracle::new(ReservationTable::default());

    oracle
        .commit_path(r(1), &CommitPlan::single(ReservationRequest::spanning(c(0), t(0), Some(t(10_000)))), None)
        .expect("r1 goal hold");
    oracle
        .commit_path(r(3), &CommitPlan::single(ReservationRequest::spanning(c(1), t(10_500), Some(t(12_000)))), None)
        .expect("r3 source hold");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(1_000)),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf.reserve_path_without_waiting(c(1), Orientation::West, c(0), drive, &oracle, &config);

    assert!(
        matches!(result, Err(PathfinderError::UnreachableWithinHorizon { .. })),
        "search should reject the source-hold violation before commit; got {result:?}"
    );
}

/// Same r1/r3 reservations as the trapped case, but the bot starts one
/// cell further out on `corridor(3)`. The extra room lets it hold c(2)
/// until r3 clears c(1) at t=15_000, hop through c(1), and hop into c(0)
/// (holding) at t=17_000. The path is 5 waypoints; goal hold open-ended
/// because `dest_dwell = MAX`.
#[test]
fn holding_with_wait_hop_wait_hop() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced_holding(&graph, [c(0)].into_iter().collect());
    let oracle = TableOracle::new(ReservationTable::default());

    oracle
        .commit_path(
            r(1),
            &CommitPlan::direct(vec![
                ReservationRequest::spanning(c(1), t(0), Some(t(1_000))),
                ReservationRequest::spanning(c(0), t(1), Some(t(10_000))),
            ]),
            None,
        )
        .expect("r1 setup");
    oracle
        .commit_path(
            r(3),
            &CommitPlan::direct(vec![
                ReservationRequest::spanning(c(1), t(14_000), Some(t(15_000))),
                ReservationRequest::spanning(c(0), t(14_001), Some(t(16_000))),
            ]),
            None,
        )
        .expect("r3 setup");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(2_000),
        dest_dwell: Some(MAX),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 500,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved = pf
        .reserve_path_without_waiting(c(2), Orientation::West, c(0), drive, &oracle, &config)
        .expect("wait-hop-hop path");

    use Orientation::{East, North, South, West};
    // Two Move successors push from the start: the early one at g=1_000
    // (no wait, c(1)'s first interval) and the late one at g=14_000 (wait
    // 13 s for r3 to clear c(1), enters c(1)'s second open-ended interval
    // at t=15_000). The early state expands first: every c(0) interval
    // fails (finite can't admit MAX, open-ended requires holding c(1) past
    // its interval end at t=14_000). The search detours back through c(2)
    // (steps 12-18) without finding a feasible second approach. Eventually
    // the late c(1) state (step 19) at g=14_000 — interval (15_000, None) —
    // emits Move-into-c(0) at g=15_000 with the open-ended c(0) interval
    // admitting MAX dwell. Goal pops at step 24.
    assert_trace!(
        trace,
        [
            Expand { cell: c(2), orientation: West, g: ms(0), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(1_000), Some(t(14_000))), (t(15_000), None)].into() },
            Move(c(2), c(1), ms(1_000), ms(0)),
            Move(c(2), c(1), ms(14_000), ms(13_000)),
            Expand { cell: c(1), orientation: West, g: ms(1_000), step: 0 },
            Rotate(c(1), West, North, ms(2_000), ms(0)),
            Rotate(c(1), West, South, ms(2_000), ms(0)),
            EntryIntervals { cell: c(0), intervals: [(t(10_000), Some(t(14_001))), (t(16_000), None)].into() },
            Expand { cell: c(1), orientation: North, g: ms(2_000), step: 0 },
            Rotate(c(1), North, East, ms(3_000), ms(0)),
            Expand { cell: c(1), orientation: South, g: ms(2_000), step: 0 },
            Expand { cell: c(1), orientation: East, g: ms(3_000), step: 0 },
            EntryIntervals { cell: c(2), intervals: [(t(0), None)].into() },
            Move(c(1), c(2), ms(4_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(2), East, North, ms(5_000), ms(0)),
            Rotate(c(2), East, South, ms(5_000), ms(0)),
            Expand { cell: c(2), orientation: North, g: ms(5_000), step: 0 },
            Expand { cell: c(2), orientation: South, g: ms(5_000), step: 0 },
            Expand { cell: c(1), orientation: West, g: ms(14_000), step: 0 },
            Rotate(c(1), West, North, ms(15_000), ms(0)),
            Rotate(c(1), West, South, ms(15_000), ms(0)),
            EntryIntervals { cell: c(0), intervals: [(t(16_000), None)].into() },
            Move(c(1), c(0), ms(15_000), ms(0)),
            Expand { cell: c(0), orientation: West, g: ms(15_000), step: 0 },
        ]
    );
    // Wait at c(2) until r3 clears c(1) at t=15_000, then walk straight
    // through. Path terminates at PathEnd t=17_000; goal hold open-ended
    // because dest_dwell=MAX.
    assert_waypoints!(
        reserved.path,
        [
            path_start(c(2), West, t(2_000)),
            wait(c(2), West, t(2_000), t(15_000)),
            mv(c(1), West, t(15_000), t(16_000)),
            mv(c(0), West, t(16_000), t(17_000)),
            path_end(c(0), West, t(17_000)),
        ]
    );
    assert_eq!(reserved.path.travel_time, ms(15_000));
    let goal = reserved.committed_reservations.iter().find(|res| res.cell() == c(0)).expect("goal cell reservation");
    assert_eq!(goal.window().start(), t(16_000));
    assert_eq!(goal.window().end(), None);
}
