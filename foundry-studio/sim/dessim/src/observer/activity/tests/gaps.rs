//! The journal's gaps: idle is the complement of the intervals, journaled as
//! no-task rows before the first, between any two and after the last, holding
//! what the bot carried, and leaving a stand a fact journals itself to that
//! fact.

use crate::events::{SimEvent, TripPurpose};
use crate::model::RobotId;
use crate::observer::output::HtmWaitCause;

use super::{
    ROBOT, arrive, depart, duty, dwell, fork_get, fork_get_resident, fork_put, gaps, holds, nav_to, order_complete,
    run, seed,
};

/// Idle is the complement of the intervals: before the first one, between any
/// two, and after the last.
#[test]
fn every_gap_between_intervals_is_journaled_as_idle() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToRelay, 2_000),
        fork_get(4_000, 6_000),
        // A gap here: the bot holds the tote but has no task until 10s.
        depart(TripPurpose::ToStation, 10_000),
        arrive(12_000),
        dwell(12_000, 1_000),
        depart(TripPurpose::ToReturnRelay, 14_000),
        fork_put(16_000, 18_000),
        // Nothing after 18s; the run's last instant is 20s.
        order_complete(20_000),
    ]);

    assert_eq!(
        gaps(&output),
        [(0, 2_000, false), (6_000, 4_000, true), (18_000, 2_000, false)],
        "leading gap, the gap between the fork-Get and the next departure, and the trailing gap"
    );
}

/// A gap after a fork-Get carries the tote; a gap after a fork-Put does not.
/// The flag comes from the last operation that changed what the bot holds,
/// which is the only fact that says so.
#[test]
fn a_gap_carries_the_tote_when_the_last_fork_was_a_get() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToRelay, 1_000),
        fork_get(2_000, 3_000),
        depart(TripPurpose::ToStation, 8_000),
        arrive(9_000),
        dwell(9_000, 500),
        depart(TripPurpose::ToReturnRelay, 10_000),
        fork_put(11_000, 12_000),
        order_complete(15_000),
    ]);

    let rows = gaps(&output);
    assert!(rows.contains(&(3_000, 5_000, true)), "the gap after the fork-Get holds the tote: {rows:?}");
    assert!(rows.contains(&(12_000, 3_000, false)), "the gap after the fork-Put does not: {rows:?}");
}

/// A gap holds what the leg before it carried, as its nav says: the
/// fulfillment tasks hand totes on with no fork operation to report it.
#[test]
fn a_gap_carries_what_the_leg_before_it_carried() {
    let SimEvent::Nav(mut laden) = nav_to(50, 1_000, 4_000) else { unreachable!() };
    laden.carrying_cargo = true;
    let output =
        run(vec![seed(&[ROBOT]), SimEvent::Nav(laden), depart(TripPurpose::ToRelay, 10_000), fork_get(12_000, 13_000)]);
    assert_eq!(gaps(&output), [(0, 1_000, false), (4_000, 6_000, true)]);
}

/// A robot that reported no work at all is idle for the whole run. Its absence
/// from the milestone map is a fact about it, not a reason to say nothing.
#[test]
fn a_robot_that_never_moved_is_idle_for_the_whole_run() {
    let idle_bot = 9;
    let output = run(vec![
        seed(&[ROBOT, idle_bot]),
        depart(TripPurpose::ToRelay, 1_000),
        fork_get(2_000, 3_000),
        order_complete(10_000),
    ]);

    let rows: Vec<_> = output
        .htm_leg_log
        .iter()
        .filter(|r| r.robot_id == RobotId(idle_bot) && r.hold.map(|h| h.cause) == Some(HtmWaitCause::NoTask))
        .map(|r| (r.start.as_ms(), r.duration.as_ms()))
        .collect();
    assert_eq!(rows, [(0, 10_000)], "one row covering the run");
    assert_eq!(duty(&output, idle_bot), (0, 0, 0), "and no duty at all");
}

/// The same when the parking leg is the robot's last milestone: the stand runs
/// to the end of the run as a gap, rather than the leg being extended to the
/// run's end as travel.
#[test]
fn a_parking_leg_at_the_end_of_a_run_leaves_the_stand_a_gap() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToParking, 1_000),
        nav_to(50, 1_000, 3_000),
        order_complete(10_000),
    ]);

    assert_eq!(duty(&output, ROBOT), (2_000, 0, 0), "the drive [1s, 3s) only");
    assert_eq!(
        gaps(&output),
        [(0, 1_000, false), (3_000, 7_000, false)],
        "the gap before it set off, and the stand to the end of the run"
    );
}

/// A resident pickup: the task chose a bot already standing on the relay's
/// access cell, and the task is accepted at the fork-Get's own start, so the
/// bot's stand there came before any task held it. That stand is a gap, not
/// travel, and it is the awaited-deposit row's to journal: a no-task row over
/// the same stretch would disagree with it, and the D30 report files two rows
/// disagreeing about one stretch under conflicting facts.
#[test]
fn a_resident_bots_stand_before_its_pickup_is_a_gap_its_own_row_journals() {
    let output = run(vec![seed(&[ROBOT]), fork_get_resident(5_000, 6_000, 2_000)]);

    assert_eq!(duty(&output, ROBOT), (0, 1_000, 0), "the fork-Get alone; the stand [3s, 5s) is not work");
    assert_eq!(holds(&output, HtmWaitCause::AwaitingDepositResident), [(3_000, 2_000, false)], "the stand's row");
    assert_eq!(gaps(&output), [(0, 3_000, false)], "and the no-task rows stop where the stand began");
}
