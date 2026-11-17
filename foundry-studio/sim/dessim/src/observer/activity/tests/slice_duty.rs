//! A D30's duty in a slice is derived once B has passed the slice's end, from
//! its work as B stands then, and nothing books into the slice afterwards; the
//! run's totals and the inspector's records read the same work.

use crate::events::{SimEvent, TripPurpose};
use crate::model::SimTime;
use crate::observer::activity::ActivityKind;
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::MetricsConfig;

use super::{ROBOT, depart, duty, fork_get, gaps, nav_to, observe, read, seed};

/// The observer's slice width.
fn width() -> u64 {
    MetricsConfig::default().slice_width_ms
}

/// `(travel, fork operations)` booked into `slice`, in ms.
fn slice_duty(state: &SimObserverState, slice: usize) -> (u64, u64) {
    let scalars = &state.slices()[slice].scalars;
    (scalars.htm_duty_traveling_ms, scalars.htm_duty_loading_unloading_ms)
}

/// A leg from 0.2 w arriving at 1.5 w, sent for a fork operation at 1.8 w that
/// is reported with it: the stand before the fork is the leg's.
fn leg_to_a_fork() -> Vec<SimEvent> {
    let w = width();
    vec![seed(&[ROBOT]), depart(TripPurpose::ToRelay, w / 5), nav_to(3, w / 5, 3 * w / 2), fork_get(9 * w / 5, 2 * w)]
}

/// B passes slice 0's end while the leg is still open: the slice reads the
/// leg as B stands, and the leg built later books nothing into it.
#[test]
fn a_slice_reads_the_open_leg_once_b_passes_its_end() {
    let w = width();
    let mut events = leg_to_a_fork();
    events.push(SimEvent::BarrierAdvanced(SimTime::ms(w + 1)));
    let mut state = observe(&events);
    assert_eq!(slice_duty(&state, 0), (w - w / 5, 0), "the open leg is read as B passes the slice's end");

    read(&mut state, &SimEvent::BarrierAdvanced(SimTime::ms(5 * w / 2)));
    assert_eq!(slice_duty(&state, 0), (w - w / 5, 0), "the leg built later books no second share");
    assert_eq!(slice_duty(&state, 1), (4 * w / 5, w / 5), "the next slice reads the leg and the fork");
    assert_eq!(duty(&state.output, ROBOT), (8 * w / 5, w / 5, 0), "the run's totals are the slices' sum");
}

/// B passes slice 0's end with the bot standing after its leg and nothing
/// reported for it to do: the stand is a gap there. The operation it then
/// turns out to have waited for is reported with B at 1.2 w, so the stand is
/// the leg's only from then, in the next slice and in the run's totals.
#[test]
fn a_stand_b_passed_before_its_operation_was_reported_stays_a_gap() {
    let w = width();
    let mut state = observe(&[
        seed(&[ROBOT]),
        depart(TripPurpose::ToRelay, w / 10),
        nav_to(3, w / 10, 2 * w / 5),
        SimEvent::BarrierAdvanced(SimTime::ms(6 * w / 5)),
    ]);
    assert_eq!(slice_duty(&state, 0), (3 * w / 10, 0), "the leg ends with its nav");

    read(&mut state, &fork_get(3 * w / 2, 8 * w / 5));
    read(&mut state, &SimEvent::BarrierAdvanced(SimTime::ms(3 * w)));
    assert_eq!(slice_duty(&state, 1), (3 * w / 10, w / 10), "the stand is the leg's from 1.2 w");
    assert_eq!(duty(&state.output, ROBOT), (6 * w / 10, w / 10, 0), "the run's totals read the stand the same way");
    assert_eq!(
        gaps(&state.output),
        [(0, w / 10, false), (2 * w / 5, 4 * w / 5, false)],
        "the journal has the stand B read as a gap until 1.2 w"
    );
}

/// The leg and the fork are built once B passes 2.5 w, and their inspector
/// records go out in the slice B stands in, not in the slices they started in,
/// which went out without them.
#[test]
fn an_inspector_record_goes_out_where_b_stands_when_its_interval_is_built() {
    let w = width();
    let mut events = leg_to_a_fork();
    events.push(SimEvent::BarrierAdvanced(SimTime::ms(5 * w / 2)));
    let state = observe(&events);
    let filed: Vec<Vec<(ActivityKind, u64)>> = state
        .slices()
        .iter()
        .map(|slice| slice.station_diag.activity.iter().map(|r| (r.kind, r.start.as_ms())).collect())
        .collect();
    assert_eq!(
        filed,
        [vec![], vec![], vec![(ActivityKind::Travel, w / 5), (ActivityKind::ForkOp, 9 * w / 5)]],
        "both records travel in slice 2, where B stands"
    );
}
