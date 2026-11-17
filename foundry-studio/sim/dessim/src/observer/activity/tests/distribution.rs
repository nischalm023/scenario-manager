//! The per-slice distribution: what each fleet's robots were doing at a
//! slice's start, derived once when B passes that instant, and never changed
//! afterwards.

use rustc_hash::FxHashMap;

use desgraph::RobotType;

use crate::events::{SimEvent, TripPurpose};
use crate::model::SimTime;
use crate::observer::activity::ActivityKind;
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::MetricsConfig;

use super::{
    ROBOT, arrive_of, depart, dwell_of, fork_get, fork_get_resident, nav_to, observe, order_complete, read, seed,
};

/// The bot whose run stops while it stands at its station after its dwell.
const STOPPED: u32 = 8;

/// The distribution of a slice whose start B has passed counts a bot inside a
/// fork operation as doing it, though the operation is the last mark behind B
/// and nothing after it has been reported.
#[test]
fn a_slice_sampled_inside_the_last_fork_operation_counts_it() {
    let width = MetricsConfig::default().slice_width_ms;
    let state = observe(&[
        seed(&[ROBOT]),
        depart(TripPurpose::ToRelay, 0),
        fork_get(width - 1_000, width + 1_000),
        // Some fact of slice 1 has been read, so the slice exists.
        order_complete(width + 500),
        SimEvent::BarrierAdvanced(SimTime::ms(width + 1)),
    ]);
    assert_eq!(state.slices()[0].scalars.htm_activity_counts.get(&ActivityKind::Travel), Some(&1));
    assert_eq!(state.slices()[1].scalars.htm_activity_counts.get(&ActivityKind::ForkOp), Some(&1));
}

/// While the run is going, a slice sampled after the open leg's nav has ended,
/// with no operation reported after it, counts the bot as standing in a gap.
#[test]
fn a_slice_sampled_after_an_open_legs_nav_counts_a_gap() {
    let width = MetricsConfig::default().slice_width_ms;
    let state = observe(&[
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 0),
        nav_to(50, 0, 2_000),
        order_complete(width + 500),
        SimEvent::BarrierAdvanced(SimTime::ms(width + 1)),
    ]);
    assert_eq!(state.slices()[0].scalars.htm_activity_counts.get(&ActivityKind::Travel), Some(&1));
    assert_eq!(state.slices()[1].scalars.htm_activity_counts.get(&ActivityKind::Gap), Some(&1));
}

/// D186: a slice goes out once, when B passes it, so the distribution it has
/// then is final, and the finished run's intervals have to agree with it. Two
/// bots stand still while B passes slice starts. One is seeded on a relay's
/// access cell, and only after B has passed two slice starts is the pickup
/// that finds it there reported: the stand was a gap, and does not turn into
/// work behind B. The other reaches its station, its operator finishes with
/// it, and the run stops before its return is scheduled: the slices B passed
/// counted it as presenting, so its visit runs on to the current world time
/// rather than ending at its dwell.
#[test]
fn a_slice_keeps_the_distribution_it_had_when_b_passed_it() {
    let width = MetricsConfig::default().slice_width_ms;
    let pickup = 5 * width / 2;
    let advance = |at| SimEvent::BarrierAdvanced(SimTime::ms(at));
    let live = [
        seed(&[ROBOT, STOPPED]),
        // A fact of a later slice, read early, so slices 0 to 3 exist.
        order_complete(3 * width + 500),
        // At its station from w/2, with its dwell [6w/10, 7w/10) and no
        // departure ever scheduled.
        arrive_of(STOPPED, width / 2),
        dwell_of(STOPPED, 6 * width / 10, width / 10),
        advance(width + 1),
        advance(2 * width + 1),
        // The pickup task is accepted when B reaches the fork-Get's start.
        advance(pickup),
        fork_get_resident(pickup, pickup + 1_000, pickup),
        advance(3 * width + 1),
    ];

    let mut state = SimObserverState::new(MetricsConfig::default());
    let mut when_passed: Vec<FxHashMap<ActivityKind, u32>> = Vec::new();
    for event in &live {
        read(&mut state, event);
        while when_passed.len() < state.activity.distribution_upto {
            when_passed.push(state.slices()[when_passed.len()].scalars.htm_activity_counts.clone());
        }
    }
    state.build_activity_intervals().unwrap();
    let at_the_end: Vec<_> = state.slices().iter().map(|slice| slice.scalars.htm_activity_counts.clone()).collect();
    let from_the_intervals: Vec<_> =
        (0..state.slices().len()).map(|index| recorded_kinds_at(&state, SimTime::ms(index as u64 * width))).collect();

    let standing_and_presenting = FxHashMap::from_iter([(ActivityKind::Gap, 1), (ActivityKind::Presentation, 1)]);
    assert_eq!(when_passed.len(), 4, "B passed the starts of slices 0 to 3 while the run was going");
    assert_eq!(
        when_passed[1..],
        [standing_and_presenting.clone(), standing_and_presenting.clone(), standing_and_presenting],
        "the stand is a gap, and the stopped bot is presenting, at slices 1 to 3"
    );
    assert_eq!(when_passed[..], at_the_end[..4], "no slice's distribution changed after B passed it");
    assert_eq!(at_the_end, from_the_intervals, "and each is what the finished run's intervals give it");
}

/// What the finished run's intervals, as the bot inspector records them, say
/// each D30 was doing at `sample`.
fn recorded_kinds_at(state: &SimObserverState, sample: SimTime) -> FxHashMap<ActivityKind, u32> {
    let records: Vec<_> = state.slices().iter().flat_map(|slice| slice.station_diag.activity.iter()).collect();
    let mut kinds = FxHashMap::default();
    for (robot, _) in state.output.robot_types.iter().filter(|(_, fleet)| **fleet == RobotType::Htm) {
        let covering = records.iter().find(|r| r.robot_id == robot.0 && r.start <= sample && sample < r.end);
        *kinds.entry(covering.map_or(ActivityKind::Gap, |r| r.kind)).or_insert(0) += 1;
    }
    kinds
}
