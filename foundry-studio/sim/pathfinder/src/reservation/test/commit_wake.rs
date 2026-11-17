//! The table's wake for `reserve_path`'s await on a held cell: a commit
//! resolves the awaits on the cells it changes, and no others, and calls
//! their wakers, whether it is made through the oracle, on the table itself
//! or through a clone of it, and whether it adds a hold, closes one or moves
//! one's start; an await takes its registrations back from every cell once
//! it resolves or is dropped.

use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Wake, Waker};

use desgraph::CellId;

use crate::reservation::oracle::{ReservationOracle, TableOracle};
use crate::{CommitPlan, MoveOp, ReservationRequest, RobotId, SimTime, TimeRange};

fn commit_on(oracle: &TableOracle, robot: u32, cell: CellId) {
    let plan = CommitPlan::single(ReservationRequest::spanning(cell, SimTime::ms(0), Some(SimTime::ms(1_000))));
    oracle.commit_path(RobotId(robot), &plan, None).expect("a commit on a free cell");
}

fn is_ready<F: Future>(future: Pin<&mut F>) -> bool {
    future.poll(&mut Context::from_waker(Waker::noop())).is_ready()
}

#[test]
fn an_await_on_a_cell_resolves_on_a_commit_touching_it_and_not_on_a_commit_elsewhere() {
    let oracle = TableOracle::default();
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    let mut held = pin!(oracle.commit_touching(&[CellId(1)], seen));
    assert!(!is_ready(held.as_mut()), "no commit yet");

    commit_on(&oracle, 7, CellId(2));
    assert!(!is_ready(held.as_mut()), "a commit on another cell leaves the await pending");

    commit_on(&oracle, 8, CellId(1));
    assert!(is_ready(held.as_mut()), "a commit touching the cell resolves it");
}

#[test]
fn an_await_counted_before_a_commit_on_its_cell_resolves_at_once() {
    let oracle = TableOracle::default();
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    commit_on(&oracle, 7, CellId(1));
    assert!(is_ready(pin!(oracle.commit_touching(&[CellId(1)], seen))), "the commit came after the count was read");
}

#[test]
fn a_commit_made_on_the_table_itself_wakes_an_await_on_the_oracle() {
    let oracle = TableOracle::default();
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    let mut held = pin!(oracle.commit_touching(&[CellId(1)], seen));
    assert!(!is_ready(held.as_mut()));
    oracle.table().reserve_cell(RobotId(8), CellId(1), TimeRange::open_ended(SimTime::ms(0))).unwrap();
    assert!(is_ready(held.as_mut()), "the table stamps its own commits");
}

/// A waker that counts its wakes; each registration holds a clone of it.
#[derive(Default)]
struct CountingWaker(AtomicUsize);

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn poll_with<F: Future>(future: Pin<&mut F>, waker: &Arc<CountingWaker>) -> bool {
    future.poll(&mut Context::from_waker(&Waker::from(waker.clone()))).is_ready()
}

#[test]
fn a_commit_touching_an_awaited_cell_calls_the_await_s_waker() {
    let oracle = TableOracle::default();
    let waker = Arc::new(CountingWaker::default());
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    let mut held = pin!(oracle.commit_touching(&[CellId(1)], seen));
    assert!(!poll_with(held.as_mut(), &waker));
    commit_on(&oracle, 7, CellId(2));
    assert_eq!(waker.0.load(Ordering::Relaxed), 0, "a commit elsewhere calls no waker");
    commit_on(&oracle, 8, CellId(1));
    assert_eq!(waker.0.load(Ordering::Relaxed), 1, "the commit on the awaited cell calls it");
}

#[test]
fn an_await_resolved_through_one_cell_withdraws_its_registration_from_the_others() {
    let oracle = TableOracle::default();
    let waker = Arc::new(CountingWaker::default());
    for round in 0..3 {
        let touched = CellId(10 + round);
        let cells = [CellId(1), touched];
        let seen = oracle.commit_count().expect("the table oracle counts commits");
        let mut held = pin!(oracle.commit_touching(&cells, seen));
        assert!(!poll_with(held.as_mut(), &waker));
        assert_eq!(Arc::strong_count(&waker), 3, "one registration on each awaited cell");
        commit_on(&oracle, 10 + round, touched);
        assert!(poll_with(held.as_mut(), &waker), "the commit on the other cell resolves it");
        assert_eq!(Arc::strong_count(&waker), 1, "cell 1, never touched, keeps no registration of a resolved await");
    }
}

#[test]
fn a_dropped_await_withdraws_its_registrations() {
    let oracle = TableOracle::default();
    let waker = Arc::new(CountingWaker::default());
    let cells = [CellId(1), CellId(2)];
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    {
        let mut held = pin!(oracle.commit_touching(&cells, seen));
        assert!(!poll_with(held.as_mut(), &waker));
        assert!(!poll_with(held.as_mut(), &waker), "a second poll replaces its registrations, adding none");
        assert_eq!(Arc::strong_count(&waker), 3);
    }
    assert_eq!(Arc::strong_count(&waker), 1, "dropping a pending await leaves no registration behind");
}

#[test]
fn a_departure_committed_on_the_table_itself_wakes_an_await_on_the_cell_it_leaves() {
    let oracle = TableOracle::default();
    let hold = oracle.table().reserve_cell(RobotId(8), CellId(1), TimeRange::open_ended(SimTime::ms(0))).unwrap();
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    let mut held = pin!(oracle.commit_touching(&[CellId(1)], seen));
    assert!(!is_ready(held.as_mut()));
    let (leave, arrive) = (SimTime::ms(1_000), SimTime::ms(2_000));
    let departure = CommitPlan {
        requests: vec![ReservationRequest::spanning(CellId(2), leave, None)],
        move_ops: vec![Some(MoveOp {
            source_cell: CellId(1),
            dest_cell: CellId(2),
            window: TimeRange::new(leave, arrive),
        })],
        close_predecessor_at: Some(arrive),
    };
    oracle.table().reserve_path(RobotId(8), &departure, Some(&hold)).unwrap();
    assert!(is_ready(held.as_mut()), "closing the hold on cell 1 stamps cell 1");
}

#[test]
fn a_commit_through_a_clone_of_the_table_wakes_an_await_on_the_original() {
    let oracle = TableOracle::default();
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    let mut held = pin!(oracle.commit_touching(&[CellId(1)], seen));
    assert!(!is_ready(held.as_mut()));
    let clone = oracle.table().clone();
    clone.reserve_cell(RobotId(8), CellId(1), TimeRange::open_ended(SimTime::ms(0))).unwrap();
    assert!(is_ready(held.as_mut()), "clones share one wake");
}

#[test]
fn a_hold_whose_start_is_subsumed_right_wakes_an_await_on_its_cell() {
    let oracle = TableOracle::default();
    let row = oracle.table().reserve_cell(RobotId(8), CellId(1), TimeRange::open_ended(SimTime::ms(1_000))).unwrap();
    let seen = oracle.commit_count().expect("the table oracle counts commits");
    let mut held = pin!(oracle.commit_touching(&[CellId(1)], seen));
    assert!(!is_ready(held.as_mut()));
    oracle.table().subsume_start_right(&row, SimTime::ms(1_500));
    assert!(is_ready(held.as_mut()), "moving a hold's start changes its cell");
}
