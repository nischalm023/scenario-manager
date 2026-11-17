//! Foreshortening: where `reserve_path` ends a window when the way ahead is
//! held. On a synthetic station queue, the approach `A`, the queue cells
//! `Q1 Q2 Q3` and the presentation cell `P`, every arc into and along the
//! queue is gated to `P` as a station's queue is, so a window that ends on a
//! queue cell still drives arcs only a robot bound for `P` may take. Robot X
//! holds `P` open-ended until its departure is committed.

use std::error::Error;
use std::future::Future;
use std::pin::{Pin, pin};
use std::task::{Context, Poll, Waker};

use super::super::FHSippPathfinder;
use super::prelude::*;
use super::windows::RecordedWindows;
use crate::reservation::CellReservation;
use crate::{ReservedPath, WindowSink};

pub(super) const A: CellId = c(0);
pub(super) const Q1: CellId = c(1);
pub(super) const Q2: CellId = c(2);
pub(super) const Q3: CellId = c(3);
pub(super) const P: CellId = c(4);
pub(super) const BEYOND: CellId = c(5);
pub(super) const X: RobotId = r(9);

pub(super) fn station_queue() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    for queue in 1..=3 {
        b.add_cell(cell(queue, CellKind::QueueCell, queue, 0));
    }
    b.add_cell(cell(4, CellKind::Presentation, 4, 0));
    b.add_cell(cell(5, CellKind::Open, 5, 0));
    let open = Edge::uniform(1000);
    let gated = Edge { dest_only: Some(DestGate::Cell(P)), ..open };
    b.add_edge(A, Q1, gated);
    b.add_edge(Q1, Q2, gated);
    b.add_edge(Q2, Q3, gated);
    b.add_edge(Q3, P, gated);
    b.add_edge(P, BEYOND, open);
    b.build()
}

pub(super) fn config(robot: RobotId, predecessor: CellReservation, sim_start: SimTime) -> CollisionConfig {
    CollisionConfig {
        self_robot: robot,
        predecessor: Some(predecessor),
        destination: None,
        sim_start,
        dest_dwell: None,
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(60),
        max_iterations: 10_000,
        motion_model: MotionModel::ConstantSpeed,
    }
}

/// Commit X's departure from `P` at `at`, through the oracle, as a task would.
pub(super) fn x_departs(pf: &FHSippPathfinder, oracle: &TableOracle, at: SimTime) {
    let hold = oracle.open_ended_reservation(P).filter(|hold| hold.robot() == X).expect("X holds P");
    let drive = uniform_drive(RobotType::Htm, false);
    pf.reserve_window(P, Orientation::East, BEYOND, &drive, oracle, &config(X, hold, at)).unwrap();
}

pub(super) fn ends_on(reserved: &ReservedPath) -> (CellId, bool) {
    let hold = reserved.committed_reservations.last().expect("a committed hold at the path's end");
    assert_eq!(reserved.path.waypoints.last().unwrap().cell, hold.cell());
    (hold.cell(), hold.is_open_ended())
}

pub(super) fn arrival(reserved: &ReservedPath) -> SimTime {
    reserved.path.waypoints.last().unwrap().end_time()
}

pub(super) fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn a_journey_to_a_held_target_stands_on_the_deepest_queue_cell_and_continues_once_the_holders_departure_is_committed() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));

    assert!(poll(journey.as_mut()).is_pending(), "the journey waits while X holds P");
    let on_q3 = {
        let committed = windows.windows.borrow();
        assert_eq!(committed.len(), 1, "one window, committed at once");
        assert_eq!(ends_on(&committed[0]), (Q3, true));
        assert_eq!(committed[0].path.waypoints[0].start_time(), t(0), "the window leaves A at the call's start");
        arrival(&committed[0])
    };
    assert!(poll(journey.as_mut()).is_pending(), "nothing changed on P");

    let departure = t(10_000);
    windows.barrier.set(t(7_000));
    x_departs(&pf, &oracle, departure);
    let Poll::Ready(last) = poll(journey.as_mut()) else {
        panic!("the journey resumes once X's departure is committed")
    };
    let last = last.unwrap();
    assert_eq!(ends_on(&last), (P, true));
    assert_eq!(last.path.waypoints[0].cell, Q3);
    assert_eq!(last.path.waypoints[0].start_time(), on_q3.max(t(7_000)), "it starts at the barrier, from Q3");
    let entry = last.path.waypoints.iter().find(|waypoint| waypoint.cell == P).unwrap();
    assert!(entry.start_time() >= departure, "it enters P at {:?}, before X departs", entry.start_time());
    assert_eq!(windows.windows.borrow().len(), 1, "the last window is returned, not handed over");
    assert!(oracle.find_overlaps().is_empty());
}

#[test]
fn a_journey_whose_way_is_free_drives_through_the_queue_without_a_stop() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let reserved =
        pf.reserve_path_without_waiting(A, Orientation::East, P, &drive, &oracle, &config(r(1), hold, t(0))).unwrap();
    assert_eq!(ends_on(&reserved), (P, true));
    assert!(
        reserved.path.waypoints.iter().all(|waypoint| !matches!(waypoint.action, WaypointAction::Wait { .. })),
        "no stop in the queue"
    );
}

#[test]
fn a_journey_behind_a_waiting_robot_ends_its_window_one_cell_back_and_moves_up_after_it() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let y_hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let y_config = config(r(1), y_hold, t(0));
    let mut y_sink = RecordedWindows::default();
    let mut y = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &y_config, &mut y_sink));
    assert!(poll(y.as_mut()).is_pending());

    let z_hold = oracle.table().reserve_cell(r(2), A, TimeRange::open_ended(t(5_000))).unwrap();
    let z_config = config(r(2), z_hold.clone(), t(5_000));
    let z_windows = RecordedWindows::default();
    let mut z_sink = z_windows.clone();
    let mut z = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &z_config, &mut z_sink));
    assert!(poll(z.as_mut()).is_pending());
    assert_eq!(ends_on(&z_windows.windows.borrow()[0]), (Q2, true), "Z waits behind Y, on Q2");

    // X's departure gives P an end while Y still holds Q3 open-ended: Z,
    // polled first, stays on Q2 rather than searching through Y's cell.
    x_departs(&pf, &oracle, t(10_000));
    assert!(poll(z.as_mut()).is_pending());
    assert_eq!(z_windows.windows.borrow().len(), 1);
    assert!(matches!(poll(y.as_mut()), Poll::Ready(Ok(_))), "Y completes onto P");
    assert!(poll(z.as_mut()).is_pending(), "Y's window onto P is committed, open-ended");
    assert_eq!(z_windows.windows.borrow().len(), 2);
    assert_eq!(ends_on(&z_windows.windows.borrow()[1]), (Q3, true), "Z moves up behind Y");
    let on_q2 = z_windows.windows.borrow()[0].committed_reservations.last().map(CellReservation::id);
    let closed: Vec<_> = z_windows.closed.borrow().iter().map(|hold| hold.as_ref().map(CellReservation::id)).collect();
    assert_eq!(closed, vec![Some(z_hold.id()), on_q2], "each window closes the hold Z stood on before it");
    assert!(oracle.find_overlaps().is_empty());
}

#[test]
fn a_journey_to_a_held_target_no_queue_cell_feeds_keeps_its_origin_until_the_holders_departure_is_committed() {
    // A -> P directly: no queue cell to wait on.
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(4, CellKind::Presentation, 1, 0));
    b.add_cell(cell(5, CellKind::Open, 2, 0));
    b.add_edge(A, P, Edge::uniform(1000));
    b.add_edge(P, BEYOND, Edge::uniform(1000));
    let pf = pathfinder(&b.build());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let windows = RecordedWindows::default();
    let mut sink = windows.clone();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));
    assert!(poll(journey.as_mut()).is_pending());
    assert!(windows.windows.borrow().is_empty(), "no window: the robot stays on A");

    windows.barrier.set(t(4_000));
    x_departs(&pf, &oracle, t(6_000));
    let Poll::Ready(Ok(last)) = poll(journey.as_mut()) else { panic!("the journey completes once X departs") };
    assert_eq!(last.path.waypoints[0].start_time(), t(4_000), "it departs A at the barrier, not the call's start");
    assert_eq!(ends_on(&last), (P, true));
}

/// A sink that refuses every window, as a caller that cannot record one.
struct Refusing;

impl WindowSink for Refusing {
    fn barrier(&self) -> SimTime {
        t(0)
    }

    fn committed(
        &mut self,
        _window: &ReservedPath,
        _closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("no plan to extend".into())
    }
}

#[test]
fn a_window_the_caller_refuses_stops_the_journey_with_window_refused() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let mut refusing = Refusing;
    let journey = pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut refusing);
    let Poll::Ready(result) = poll(pin!(journey)) else { panic!("a refused window ends the call at once") };
    assert!(matches!(result, Err(PathfinderError::WindowRefused(_))), "{:?}", result.map(|_| ()));
}

#[test]
fn a_journey_whose_sink_refuses_a_wait_at_its_origin_is_refused_with_nothing_committed_when_the_queue_is_full() {
    // X stands on P and three robots fill the queue, each holding its cell
    // open-ended (no departure is committed, scheduler time), so no window can
    // be committed at the call's start.
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    for (robot, queue) in [(r(6), Q1), (r(7), Q2), (r(8), Q3)] {
        oracle.table().reserve_cell(robot, queue, TimeRange::open_ended(t(0))).unwrap();
    }
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let windows = RecordedWindows { refuses_origin_wait: true, ..RecordedWindows::default() };
    let mut sink = windows.clone();
    let config = config(r(1), hold, t(0));
    let journey = pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink);
    let Poll::Ready(refused) = poll(pin!(journey)) else {
        panic!("the refusal is returned at once, without an await (scheduler time)")
    };
    let Err(PathfinderError::OriginWaitRefused { origin, held }) = refused else {
        panic!("refused at its origin, got {:?}", refused.map(|_| ()))
    };
    assert_eq!(origin, A);
    assert!(held.contains(&Q1), "the queue's tail is among the held cells named: {held:?}");
    assert!(windows.windows.borrow().is_empty(), "no window was committed");
    let on_a = oracle.open_ended_reservation(A).expect("the robot still holds A open-ended");
    assert_eq!(on_a.robot(), r(1));
}

#[test]
fn a_journey_whose_sink_refuses_a_wait_at_its_origin_departs_at_the_calls_start_when_a_queue_cell_is_free() {
    // Q1 is free: the first window drives onto it, departing A (simtime) at the
    // call's start, and the refusal is never asked for.
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    for (robot, queue) in [(r(7), Q2), (r(8), Q3)] {
        oracle.table().reserve_cell(robot, queue, TimeRange::open_ended(t(0))).unwrap();
    }
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(2_000))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let windows = RecordedWindows { refuses_origin_wait: true, ..RecordedWindows::default() };
    let mut sink = windows.clone();
    let config = config(r(1), hold, t(2_000));
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));
    assert!(poll(journey.as_mut()).is_pending(), "the journey awaits on Q1 (scheduler time)");
    let committed = windows.windows.borrow();
    assert_eq!(committed.len(), 1);
    assert_eq!(ends_on(&committed[0]), (Q1, true));
    assert_eq!(committed[0].path.waypoints[0].start_time(), t(2_000), "it departs A at the call's start");
}

#[test]
fn a_holders_departure_scheduled_beyond_the_horizon_starts_the_next_window_toward_it() {
    let pf = pathfinder(&station_queue());
    let oracle = TableOracle::new(ReservationTable::default());
    oracle.table().reserve_cell(X, P, TimeRange::open_ended(t(0))).unwrap();
    let hold = oracle.table().reserve_cell(r(1), A, TimeRange::open_ended(t(0))).unwrap();
    let drive = uniform_drive(RobotType::Htm, false);
    let config = config(r(1), hold, t(0));
    let mut sink = RecordedWindows::default();
    let mut journey = pin!(pf.reserve_path(A, Orientation::East, P, &drive, &oracle, &config, &mut sink));
    assert!(poll(journey.as_mut()).is_pending());

    // X departs at 100 s, past the 60 s horizon of a window starting on Q3.
    x_departs(&pf, &oracle, t(100_000));
    let Poll::Ready(last) = poll(journey.as_mut()) else { panic!("X's departure is scheduled") };
    let last = last.expect("the wait on Q3 does not spend the window's horizon");
    assert_eq!(ends_on(&last), (P, true));
    assert!(last.path.waypoints[0].start_time() >= t(40_000), "the window starts toward the clearance");
}
