//! Foreshortening decided by search: which cell a window ends on, and which
//! cell the call awaits, follow from what the robot can actually reach, not
//! from a queue's depths alone. Fixtures beside `super::foreshortening`'s
//! station queue `A Q1 Q2 Q3 P`.

use std::pin::pin;
use std::task::Poll;

use super::super::FHSippPathfinder;
use super::foreshortening::{A, BEYOND, P, Q1, Q2, Q3, X, config, ends_on, poll, station_queue, x_departs};
use super::prelude::*;
use super::windows::RecordedWindows;
use crate::reservation::CellReservation;

#[test]
fn a_held_cell_on_another_queue_branch_into_the_target_leaves_a_free_route_one_window() {
    // A -> Qb -> P is free; W holds Qa2 on the other branch Qa2 -> Qa1 -> P.
    let (qb, qa1, qa2, target) = (c(1), c(2), c(3), c(4));
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::QueueCell, 1, 0));
    b.add_cell(cell(2, CellKind::QueueCell, 2, 1));
    b.add_cell(cell(3, CellKind::QueueCell, 2, 2));
    b.add_cell(cell(4, CellKind::Presentation, 2, 0));
    let gated = Edge { dest_only: Some(DestGate::Cell(target)), ..Edge::uniform(1000) };
    b.add_edge(A, qb, gated);
    b.add_edge(qb, target, gated);
    b.add_edge(qa2, qa1, gated);
    b.add_edge(qa1, target, gated);
    let pf = pathfinder(&b.build());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(r(7), qa2, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let reserved =
        pf.reserve_path_without_waiting(A, Orientation::East, target, &drive, &oracle, &config(r(1), hold, t(0)));
    assert_eq!(ends_on(&reserved.unwrap()), (target, true), "the held branch stops nothing on the free one");
}

/// An open cell before the queue, with its own way out to `E`.
const N: CellId = c(6);
const E: CellId = c(7);

/// The station queue entered through `N`: `A -> N -> Q1 -> Q2 -> Q3 -> P`.
fn queue_behind_an_approach() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    for queue in 1..=3 {
        b.add_cell(cell(queue, CellKind::QueueCell, queue + 1, 0));
    }
    b.add_cell(cell(4, CellKind::Presentation, 5, 0));
    b.add_cell(cell(5, CellKind::Open, 6, 0));
    b.add_cell(cell(6, CellKind::Open, 1, 0));
    b.add_cell(cell(7, CellKind::Open, 1, 1));
    let open = Edge::uniform(1000);
    let gated = Edge { dest_only: Some(DestGate::Cell(P)), ..open };
    b.add_edge(A, N, open);
    b.add_edge(N, E, open);
    b.add_edge(N, Q1, gated);
    b.add_edge(Q1, Q2, gated);
    b.add_edge(Q2, Q3, gated);
    b.add_edge(Q3, P, gated);
    b.add_edge(P, BEYOND, open);
    b.build()
}

#[test]
fn a_journey_whose_approach_is_held_keeps_its_origin_and_awaits_the_approach_cell() {
    // Z stands on N; X holds P.
    let pf = pathfinder(&queue_behind_an_approach());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let z_hold = oracle.table().reserve_cell(r(2), N, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config_r1 = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config_r1, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "no queue cell is in reach while Z stands on N");
    assert!(windows.windows.borrow().is_empty(), "the robot keeps its origin");
    pf.reserve_window(N, Orientation::North, E, &drive, &oracle, &config(r(2), z_hold, t(1_000))).unwrap();
    assert!(poll(journey.as_mut()).is_pending(), "X still holds P");
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q3, true), "Z's departure lets the robot into the queue");
}

#[test]
fn a_finite_stay_on_a_held_target_is_foreshortened_like_an_open_ended_one() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = CollisionConfig { dest_dwell: Some(SimDuration::seconds(5)), ..config(r(1), hold, t(0)) };
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "X's hold admits no five-second stay on P");
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q3, true));
}

#[test]
fn a_finite_stay_that_fits_before_a_later_open_ended_hold_goes_to_the_target_at_once() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(30_000))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = CollisionConfig { dest_dwell: Some(SimDuration::seconds(5)), ..config(r(1), hold, t(0)) };
    let reserved = pf.reserve_path_without_waiting(A, Orientation::East, P, &drive, &oracle, &config).unwrap();
    assert_eq!(ends_on(&reserved), (P, false), "a five-second stay ends long before X arrives at 30 s");
}

#[test]
fn the_first_window_starts_no_earlier_than_the_barrier() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    windows.barrier.set(t(7_000));
    let mut sink = windows.clone();
    let Poll::Ready(Ok(reserved)) =
        poll(pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink)))
    else {
        panic!("a free way is one window")
    };
    assert_eq!(reserved.path.waypoints[0].start_time(), t(7_000), "nothing is planned behind the barrier");
}

#[test]
fn a_kinematic_drive_is_foreshortened_by_routes_its_own_drive_prices() {
    let pf = pathfinder_kinematics(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = Trapezoidal::htm(&reference_htm(), false);
    let config = CollisionConfig { motion_model: MotionModel::HtmKinematic, ..config(r(1), hold, t(0)) };
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "the robot awaits X on the queue cell next to P");
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q3, true));
}

#[test]
fn an_await_is_for_every_held_cell_on_the_robots_route_not_only_the_first() {
    let pf = pathfinder(&queue_behind_an_approach());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(r(2), N, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let held = pf.held_on_route(A, Orientation::East, P, &drive, &oracle, &config(r(1), hold, t(0)));
    assert_eq!(held.as_slice(), [N, P], "X leaving P may matter as much as Z leaving N");
}

#[test]
fn a_journey_whose_target_is_held_open_ended_runs_no_search_to_the_target_while_it_is() {
    // X holds P open-ended and the stay asked for is open-ended: no path to P
    // can fit, so the pass searches only for the queue cell it ends on.
    let trace = TraceCollector::new();
    let pf = pathfinder(&station_queue()).with_trace_hook(trace.hook());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "the journey awaits on Q3 while X holds P");
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q3, true));
    let failed_to_p =
        trace.events().iter().filter(|event| matches!(event, TraceEvent::Exhausted(e) if e.to == P)).count();
    assert_eq!(failed_to_p, 0, "no search to P ran while X held it open-ended");
}

#[test]
fn a_robot_in_the_queue_never_ends_a_window_back_in_it_through_a_shortcut_or_a_reverse_edge() {
    // Queue Q0 -> Q1 -> Q2 -> P, with a shortcut Q0 -> P and a reverse edge
    // Q1 -> Q0. R stands on Q1; W holds Q2 and X holds P, open-ended. Q0 is
    // one move from P, fewer than Q1, but it is behind R in the queue.
    let (q0, q1, q2, target) = (c(1), c(2), c(3), c(4));
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(1, CellKind::QueueCell, 0, 0));
    b.add_cell(cell(2, CellKind::QueueCell, 1, 0));
    b.add_cell(cell(3, CellKind::QueueCell, 2, 0));
    b.add_cell(cell(4, CellKind::Presentation, 3, 0));
    let gated = Edge { dest_only: Some(DestGate::Cell(target)), ..Edge::uniform(1000) };
    b.add_edge(q0, q1, gated);
    b.add_edge(q1, q2, gated);
    b.add_edge(q2, target, gated);
    b.add_edge(q0, target, gated);
    b.add_edge(q1, q0, gated);
    let pf = pathfinder(&b.build());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(r(7), q2, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(X, target, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), q1, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(q1, Orientation::East, target, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "R awaits on Q1");
    assert!(windows.windows.borrow().is_empty(), "no window back to Q0");
}

#[test]
fn a_journey_whose_deepest_queue_cell_frees_beyond_the_horizon_ends_its_window_one_cell_back() {
    // X holds P open-ended; W stands on Q3 until 70 s, past the 60 s horizon,
    // so no open-ended stay on Q3 can start in reach and the window ends on Q2.
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(r(7), Q3, TimeRange::new(t(0), t(70_000))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "the journey awaits P");
    let committed = windows.windows.borrow();
    assert_eq!(committed.len(), 1);
    assert_eq!(ends_on(&committed[0]), (Q2, true), "Q3 opens too late; Q2 is the deepest cell in reach");
}

#[test]
fn a_queue_cell_that_frees_inside_the_horizon_but_too_late_to_reach_in_it_ends_the_window_one_cell_back() {
    // W stands on Q3 until 59.5 s, inside the 60 s horizon, but the move onto
    // Q3 once it frees ends past 60 s (freed at 57 s, Q3 is reached), so the
    // window ends on Q2.
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(r(7), Q3, TimeRange::new(t(0), t(59_500))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "the journey awaits P");
    let committed = windows.windows.borrow();
    assert_eq!(committed.len(), 1);
    assert_eq!(ends_on(&committed[0]), (Q2, true), "Q3 frees in the horizon, too late to reach in it");
}

#[test]
fn after_an_await_the_target_is_searched_from_the_barrier_before_any_clearance_moves_the_start() {
    // A -> N -> P is the shortest route and A -> M -> P a bypass. Z stands on
    // N and X on P, open-ended, so R (five seconds on P) awaits both. Then Z's
    // departure is committed at 1000 s, beyond the horizon, X's at 20 s, and
    // Y's arrival on P from 100 s: P is free from 20 s to 100 s, through M.
    let (n, m, beyond_n) = (c(1), c(2), c(5));
    let target = c(3);
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 0, 1));
    b.add_cell(cell(3, CellKind::Presentation, 1, 1));
    b.add_cell(cell(4, CellKind::Open, 2, 1));
    b.add_cell(cell(5, CellKind::Open, 2, 0));
    let open = Edge::uniform(1000);
    b.add_edge(A, n, open);
    b.add_edge(n, target, open);
    b.add_edge(n, beyond_n, open);
    b.add_edge(A, m, Edge::uniform(2000));
    b.add_edge(m, target, Edge::uniform(2000));
    b.add_edge(target, c(4), open);
    let pf = pathfinder(&b.build());
    let oracle = TableOracle::new(ReservationTable::default());
    let z = oracle.table().reserve_cell(r(2), n, TimeRange::open_ended(t(0))).unwrap();
    let x = oracle.table().reserve_cell(X, target, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let asked = CollisionConfig { dest_dwell: Some(SimDuration::seconds(5)), ..config(r(1), hold, t(0)) };
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, target, &drive, &oracle, &asked, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "R awaits N and P");

    pf.reserve_window(n, Orientation::East, beyond_n, &drive, &oracle, &config(r(2), z, t(1_000_000))).unwrap();
    pf.reserve_window(target, Orientation::East, c(4), &drive, &oracle, &config(X, x, t(20_000))).unwrap();
    oracle.table().reserve_cell(r(3), target, TimeRange::open_ended(t(100_000))).unwrap();
    windows.barrier.set(t(10_000));
    let Poll::Ready(reserved) = poll(journey.as_mut()) else { panic!("the bypass reaches P between 20 s and 100 s") };
    let reserved = reserved.unwrap();
    let entry = reserved.path.waypoints.iter().find(|waypoint| waypoint.cell == target).unwrap().start_time();
    assert!(entry >= t(20_000) && entry < t(95_000), "R enters P at {entry:?}, in the free interval");
    assert!(reserved.path.waypoints.iter().any(|waypoint| waypoint.cell == m), "through the bypass");
}

#[test]
fn a_journey_whose_way_into_the_target_crosses_a_held_queue_cell_runs_no_search_to_the_target() {
    // X stands on P until its departure at 100 s, beyond R's arrival; Y holds
    // Q3 open-ended. Every way into P crosses Q3, so the pass searches only
    // for Q2, where it ends.
    let trace = TraceCollector::new();
    let pf = pathfinder(&station_queue()).with_trace_hook(trace.hook());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::new(t(0), t(100_000))).unwrap();
    oracle.table().reserve_cell(r(8), Q3, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "the journey awaits on Q2 while Y holds Q3");
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q2, true));
    let failed_to_p =
        trace.events().iter().filter(|event| matches!(event, TraceEvent::Exhausted(e) if e.to == P)).count();
    assert_eq!(failed_to_p, 0, "no search to P ran while Y held Q3 open-ended");
}

#[test]
fn a_free_queue_cell_beyond_a_held_one_is_not_searched_for() {
    // X stands on P until 100 s, beyond R's arrival; Y holds Q2 open-ended, so
    // Q3 is free but every way into it crosses Q2: the window ends on Q1 with
    // no search for Q3.
    let trace = TraceCollector::new();
    let pf = pathfinder(&station_queue()).with_trace_hook(trace.hook());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::new(t(0), t(100_000))).unwrap();
    oracle.table().reserve_cell(r(8), Q2, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "the journey awaits on Q1 while Y holds Q2");
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q1, true));
    let failed = trace
        .events()
        .iter()
        .filter(|event| matches!(event, TraceEvent::Exhausted(e) if e.to == Q3 || e.to == P))
        .count();
    assert_eq!(failed, 0, "no search to Q3 or P ran while Y held Q2 open-ended");
}

/// A second queue cell into `P`, off R's way in the tests below.
const W: CellId = c(8);

/// The table of the tests below. `A -> N -> Q1 -> Q2 -> P` takes 10, 20, 10
/// and 10 s, and `loop_back` adds a way from BEYOND back to N (7 s). Z stands
/// on N, X on P and V on W, open-ended; U holds Q2 until 1010 s; R stands on
/// A. Z on N seals Q1 and Q2, and with V on W seals P, so R commits nothing
/// before its first await; V on W keeps R's journey foreshortened from A after X's
/// departure is committed; U on Q2 makes the search that starts toward P's
/// clearance fail from A and reach P from Q1. Returns Z's hold and R's.
fn queue_beside_a_second_lane(loop_back: bool) -> (FHSippPathfinder, TableOracle, CellReservation, CellReservation) {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::QueueCell, 2, 0));
    b.add_cell(cell(2, CellKind::QueueCell, 3, 0));
    b.add_cell(cell(4, CellKind::Presentation, 4, 0));
    b.add_cell(cell(5, CellKind::Open, 5, 0));
    b.add_cell(cell(6, CellKind::Open, 1, 0));
    b.add_cell(cell(7, CellKind::Open, 1, 1));
    b.add_cell(cell(8, CellKind::QueueCell, 4, 1));
    let gated = |length| Edge { dest_only: Some(DestGate::Cell(P)), ..Edge::uniform(length) };
    b.add_edge(A, N, Edge::uniform(10_000));
    b.add_edge(N, E, Edge::uniform(1000));
    b.add_edge(N, Q1, gated(20_000));
    b.add_edge(Q1, Q2, gated(10_000));
    b.add_edge(Q2, P, gated(10_000));
    b.add_edge(W, P, gated(1000));
    b.add_edge(P, BEYOND, Edge::uniform(1000));
    if loop_back {
        b.add_edge(BEYOND, N, Edge::uniform(7000));
    }
    let pf = pathfinder(&b.build());
    let oracle = TableOracle::new(ReservationTable::default());
    let z = oracle.table().reserve_cell(r(2), N, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(r(3), W, TimeRange::open_ended(t(0))).unwrap();
    oracle.table().reserve_cell(r(5), Q2, TimeRange::new(t(0), t(1_010_000))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    (pf, oracle, z, hold)
}

/// Commit Z's departure from N at 10 s, as a task would.
fn z_departs(pf: &FHSippPathfinder, oracle: &TableOracle, z: CellReservation) {
    let drive = uniform_drive(RobotType::Htm, false);
    pf.reserve_window(N, Orientation::East, E, &drive, oracle, &config(r(2), z, t(10_000))).unwrap();
}

#[test]
fn after_a_window_onto_a_queue_cell_the_target_is_searched_from_the_clearance_of_a_cell_awaited_before_it() {
    // R awaits N and P. Then Z's departure at 10 s is committed, and X's at
    // 1000 s, beyond the horizon, before R is polled. R's window ends on Q1
    // (arriving at about 40 s); from there P is reached only by the search
    // that starts at P's clearance less the travel (about 980 s), entering Q2
    // at 1010 s.
    let (pf, oracle, z, hold) = queue_beside_a_second_lane(false);
    let drive = uniform_drive(RobotType::Htm, false);
    let asked = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &asked, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "R awaits N and P");
    assert!(windows.windows.borrow().is_empty(), "R awaits on A");

    z_departs(&pf, &oracle, z);
    x_departs(&pf, &oracle, t(1_000_000));
    let Poll::Ready(reserved) = poll(journey.as_mut()) else { panic!("nothing is left to await") };
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q1, true), "R's first window ends on Q1");
    let reserved = reserved.expect("from Q1, the search that starts toward P's clearance reaches P");
    let entry = reserved.path.waypoints.iter().find(|waypoint| waypoint.cell == P).unwrap().start_time();
    assert!(entry >= t(1_000_000), "R enters P at {entry:?}, after X's departure at 1000 s");
}

#[test]
fn a_cell_awaited_before_the_last_await_still_moves_the_target_s_search() {
    // R awaits N and P. X's departure at 1000 s, beyond the horizon, is
    // committed first; R is polled and awaits N alone, Z's departure not yet
    // committed. Then Z's departure at 10 s is committed: R's window ends on
    // Q1, and from there P is reached by the search that starts toward the
    // clearance of P, which only the first await watched.
    let (pf, oracle, z, hold) = queue_beside_a_second_lane(false);
    let drive = uniform_drive(RobotType::Htm, false);
    let asked = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &asked, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "R awaits N and P");

    x_departs(&pf, &oracle, t(1_000_000));
    assert!(poll(journey.as_mut()).is_pending(), "R awaits N, Z's departure not yet committed");
    assert!(windows.windows.borrow().is_empty(), "R awaits on A");
    z_departs(&pf, &oracle, z);
    let Poll::Ready(reserved) = poll(journey.as_mut()) else { panic!("nothing is left to await") };
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q1, true), "R's first window ends on Q1");
    let reserved = reserved.expect("from Q1, the search that starts toward P's clearance reaches P");
    let entry = reserved.path.waypoints.iter().find(|waypoint| waypoint.cell == P).unwrap().start_time();
    assert!(entry < t(1_041_000), "R enters P at {entry:?}, within the horizon of the search from 981 s");
}

#[test]
fn an_awaited_cell_off_the_route_from_the_robot_s_cell_does_not_move_the_target_s_search() {
    // With a way from BEYOND back to N, N can be reached from Q1 through P
    // and BEYOND. R awaits N and P; Z's departure at 10 s and X's at 1000 s are committed,
    // and so are another robot's holds on N from 900 s to 2000 s and on Q2
    // from 1900 s to 2040 s. From A the search starts toward N's clearance
    // (at about 1990 s) and cannot reach P within its 60 s horizon, so R's
    // window ends on Q1.
    // From Q1, N is off R's route: the search starts toward P's clearance
    // alone (about 980 s) and enters P at about 1020 s.
    let (pf, oracle, z, hold) = queue_beside_a_second_lane(true);
    let drive = uniform_drive(RobotType::Htm, false);
    let asked = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &asked, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "R awaits N and P");

    z_departs(&pf, &oracle, z);
    x_departs(&pf, &oracle, t(1_000_000));
    oracle.table().reserve_cell(r(6), N, TimeRange::new(t(900_000), t(2_000_000))).unwrap();
    oracle.table().reserve_cell(r(6), Q2, TimeRange::new(t(1_900_000), t(2_040_000))).unwrap();
    let Poll::Ready(reserved) = poll(journey.as_mut()) else { panic!("nothing is left to await") };
    assert_eq!(ends_on(&windows.windows.borrow()[0]), (Q1, true), "R's first window ends on Q1");
    let reserved = reserved.expect("from Q1, the search that starts toward P's clearance reaches P");
    let entry = reserved.path.waypoints.iter().find(|waypoint| waypoint.cell == P).unwrap().start_time();
    assert!(entry < t(1_041_000), "R enters P at {entry:?}, within the horizon of the search from 981 s");
}
