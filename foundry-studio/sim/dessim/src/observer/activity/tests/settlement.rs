//! What B's advances settle while the run is still going: every interval once
//! B has passed what closes it, a span on its own end, each exactly once, and
//! no work reported from before B stood when it was reported.

use crate::events::{SimEvent, TripPurpose};
use crate::model::{RobotId, SimTime};
use crate::observer::StreamViolation;
use crate::observer::activity::{ActivityKind, ActivityViolation};
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::MetricsConfig;
use crate::sim_types::StopReason;

use super::{ROBOT, depart, fork_get, fork_put, observe, putaway_filled, run_stopping, seed};

/// The fork operations booked so far for `ROBOT`, as the inspector has them.
fn fork_records(state: &SimObserverState) -> Vec<(u64, u64)> {
    let records = state.slices().iter().flat_map(|slice| slice.station_diag.activity.iter());
    records.filter(|r| r.kind == ActivityKind::ForkOp).map(|r| (r.start.as_ms(), r.end.as_ms())).collect()
}

/// A fork operation states its own end, so it settles once B passes that end
/// rather than waiting for the robot's next milestone, which may come long
/// after or never: its duty, its inspector record and the slice it fills are
/// all booked while the run is still going, and booked once.
#[test]
fn a_fork_operation_settles_once_b_passes_its_end() {
    let width = MetricsConfig::default().slice_width_ms;
    let mut state = observe(&[
        seed(&[ROBOT]),
        depart(TripPurpose::ToRelay, 0),
        fork_get(width / 2, width),
        // B passes the end of slice 0. Nothing has closed the stream, and the
        // robot has reported nothing after the fork.
        SimEvent::BarrierAdvanced(SimTime::ms(width + 1)),
    ]);

    let slice_0 = &state.slices()[0].scalars;
    assert_eq!(slice_0.htm_duty_traveling_ms, width / 2, "the leg [0, w/2) is booked while the run is going");
    assert_eq!(slice_0.htm_duty_loading_unloading_ms, width / 2, "and so is the fork [w/2, w)");
    assert_eq!(
        slice_0.htm_activity_counts.get(&ActivityKind::Travel),
        Some(&1),
        "and the distribution says what the bot was doing at the slice's start"
    );
    assert_eq!(fork_records(&state), [(width / 2, width)], "the inspector has the fork");

    state.settle_activity(SimTime::ms(width + 2)).unwrap();
    let duty = state.output.robot_duty_totals[&RobotId(ROBOT)];
    assert_eq!(duty.loading_unloading.as_ms(), width / 2, "another advance books the fork no second time");
    assert_eq!(fork_records(&state), [(width / 2, width)], "nor records it twice");
}

/// Work reported from before B stood when it was reported falls in slices
/// already read without it. A fact is never stamped behind B, but a putaway
/// dwell with no station visit open is work from the dwell's start, which can
/// lie behind the fill its fact is stamped at; the observer reads nothing
/// after it and stops the run there and then, while the scheduler is still
/// going.
#[test]
fn work_reported_from_behind_b_stops_the_run() {
    let (observed, stopped) = run_stopping(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToRelay, 0),
        depart(TripPurpose::ToParking, 5_000),
        SimEvent::BarrierAdvanced(SimTime::ms(6_000)),
        putaway_filled(7_000, 6_000),
        SimEvent::BarrierAdvanced(SimTime::ms(8_000)),
    ]);

    assert_eq!(
        observed.unwrap_err(),
        StreamViolation::Activity(ActivityViolation::WorkBehindBarrier {
            robot: RobotId(ROBOT),
            at: SimTime::ms(1_000),
            barrier: SimTime::ms(6_000)
        })
    );
    assert!(matches!(stopped, Some(StopReason::Error(_))), "the run is told to stop: {stopped:?}");
}

/// Two fork operations whose windows overlap say the robot was inside two
/// scheduled tasks at once, which cannot happen. The two readers below
/// disagree about it — duty counts the shared time twice, the gap sweep
/// unions it and counts it once — so the disagreement stops the run rather
/// than being resolved silently in favour of either.
#[test]
fn overlapping_intervals_for_one_robot_stop_the_run() {
    let (observed, stopped) = run_stopping(vec![
        seed(&[ROBOT]),
        fork_get(1_000, 5_000),
        fork_put(3_000, 7_000),
        SimEvent::BarrierAdvanced(SimTime::ms(8_000)),
    ]);

    assert!(
        matches!(
            observed,
            Err(StreamViolation::Activity(ActivityViolation::InsideTwoTasks { robot: RobotId(ROBOT), .. }))
        ),
        "{observed:?}"
    );
    assert!(matches!(stopped, Some(StopReason::Error(_))), "the run is told to stop: {stopped:?}");
}
