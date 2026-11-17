use worldview::Error as WorldviewError;
use worldview::frames::robot_state::{CellRole, StateSpan};

use super::*;

const SLICE: SimDuration = SimDuration::seconds(10);
const D30: RobotId = RobotId(7);
const DA50: RobotId = RobotId(9);
const NO_TASK: RobotState = RobotState::NoTask { cell: Some(CellRole::Route) };

fn at(seconds: u64) -> SimTime {
    SimTime::seconds(seconds)
}

fn named(robot: RobotId, continues: bool, spans: &[(u64, RobotState)]) -> RobotSpans {
    let spans = spans.iter().map(|(dt, state)| StateSpan { dt: SimDuration::seconds(*dt), state: *state }).collect();
    RobotSpans { robot, continues, spans }
}

fn delta(entries: Vec<RobotSpans>) -> RobotStateDelta {
    RobotStateDelta(entries)
}

/// A reader seeded at `seconds` with each robot's class and the state the keyframe states for it.
fn seeded(seconds: u64, robots: &[(RobotId, RobotClass, RobotState)]) -> SpanReader {
    let keyframe = RobotStateKeyframe(robots.iter().map(|(robot, _, state)| (*robot, *state)).collect());
    let roster = robots.iter().map(|(robot, class, _)| (*robot, *class));
    SpanReader::seeded(roster, SLICE, at(seconds), &keyframe).expect("a keyframe stating every robot of the roster")
}

fn spent(class: RobotClass, state: RobotState, from: u64, to: u64) -> SpanEvent {
    SpanEvent::Spent { class, state, from: at(from), to: at(to) }
}

use RobotClass::{Htm, Vtm};
use RobotState::{EmptyTravel, LadenTravel};

const DROP: RobotState = RobotState::Drop;

/// Robot 7 travels empty, travels laden from 4 s to 23 s across two slice edges, drops its tote until 26 s, then
/// stands with no task: each slice's inside is told once the reader has taken the next slice, a slice that names no
/// robot told as the state it holds all slice, and the last slice's inside up to the run's stop, where it ends.
#[test]
fn each_slices_inside_is_told_once_the_reader_has_taken_the_next_and_the_last_up_to_the_runs_stop() {
    let mut reader = seeded(0, &[(D30, Htm, EmptyTravel)]);
    let first = reader.slice(at(0), delta(vec![named(D30, false, &[(4, EmptyTravel), (6, LadenTravel)])]));
    assert_eq!(first.expect("taken"), [SpanEvent::Through(at(0))], "nothing inside the slice is told yet");

    let second = reader.slice(at(10), delta(Vec::new())).expect("taken");
    let first_inside = [spent(Htm, EmptyTravel, 0, 4), spent(Htm, LadenTravel, 4, 10), SpanEvent::Through(at(10))];
    assert_eq!(second, first_inside);

    let carried_on = [(3, LadenTravel), (3, DROP), (4, NO_TASK)];
    let third = reader.slice(at(20), delta(vec![named(D30, true, &carried_on)])).expect("taken");
    assert_eq!(third, [spent(Htm, LadenTravel, 10, 20), SpanEvent::Through(at(20))], "a robot no slice names");

    let fourth = reader.slice(at(30), delta(Vec::new())).expect("taken");
    let third_inside = [
        spent(Htm, LadenTravel, 20, 23),
        spent(Htm, DROP, 23, 26),
        spent(Htm, NO_TASK, 26, 30),
        SpanEvent::Through(at(30)),
    ];
    assert_eq!(fourth, third_inside);

    let end = reader.end(at(35)).expect("the run ends inside the last slice");
    assert_eq!(end, [spent(Htm, NO_TASK, 30, 35), SpanEvent::RunEnd(at(35))], "told up to the run's end");
}

/// A reader seeded at 20 s tells each robot's time from the keyframe's instant on: the D30's laden travel in force
/// there, then its drop, and the DA50's wait.
#[test]
fn a_reader_seeded_mid_run_tells_each_robots_time_from_its_keyframe() {
    let mut reader = seeded(20, &[(D30, Htm, LadenTravel), (DA50, Vtm, NO_TASK)]);
    let first = vec![named(D30, true, &[(3, LadenTravel), (7, DROP)]), named(DA50, false, &[(10, NO_TASK)])];
    assert_eq!(reader.slice(at(20), delta(first)).expect("taken"), [SpanEvent::Through(at(20))]);
    let end = reader.end(at(30)).expect("ends");
    let told = [
        spent(Htm, LadenTravel, 20, 23),
        spent(Htm, DROP, 23, 30),
        spent(Vtm, NO_TASK, 20, 30),
        SpanEvent::RunEnd(at(30)),
    ];
    assert_eq!(end, told);
}

/// A run that stops inside its last slice ends that slice there: spans laid end to end up to the stop are told, and
/// spans stated past it are refused, before anything is told. No slice begins at the stop: one there, naming a robot
/// with spans of any length, lies across no time.
#[test]
fn the_runs_stop_ends_its_last_slice_and_spans_past_it_are_refused() {
    let mut reader = seeded(0, &[(D30, Htm, NO_TASK)]);
    reader.slice(at(0), delta(vec![named(D30, false, &[(2, NO_TASK), (3, EmptyTravel)])])).expect("taken");
    let told = [spent(Htm, NO_TASK, 0, 2), spent(Htm, EmptyTravel, 2, 5), SpanEvent::RunEnd(at(5))];
    assert_eq!(reader.end(at(5)).expect("ends at the stop"), told);

    let mut reader = seeded(0, &[(D30, Htm, NO_TASK)]);
    reader.slice(at(0), delta(vec![named(D30, false, &[(6, NO_TASK), (4, EmptyTravel)])])).expect("taken");
    let past = reader.end(at(5)).expect_err("the travel is stated past the stop");
    assert!(matches!(past, SpanError::Channel(WorldviewError::RobotSpansOffTheSlice { robot: 7, .. })), "{past}");

    let mut reader = seeded(0, &[(D30, Htm, NO_TASK)]);
    reader.slice(at(0), delta(vec![named(D30, false, &[(10, NO_TASK)])])).expect("taken");
    reader.slice(at(10), delta(vec![named(D30, false, &[(10, EmptyTravel)])])).expect("taken");
    let at_the_stop = reader.end(at(10)).expect_err("a slice beginning at the stop");
    assert!(matches!(at_the_stop, SpanError::Channel(WorldviewError::RobotSpansOffTheSlice { .. })), "{at_the_stop}");
}

/// What a reader cannot take it refuses before telling anything, and the reader reads on from where it stood.
#[test]
fn a_slice_out_of_turn_or_one_the_channels_rules_refuse_is_refused_before_anything_is_told() {
    let mut short = seeded(0, &[(D30, Htm, LadenTravel)]);
    short.slice(at(0), delta(vec![named(D30, false, &[(4, LadenTravel)])])).expect("its end is not known yet");
    let unstated = short.slice(at(10), delta(Vec::new())).expect_err("4 s stated of a slice the next ends at 10 s");
    assert!(matches!(unstated, SpanError::Channel(WorldviewError::RobotSpansOffTheSlice { .. })), "{unstated}");

    let mut reader = seeded(0, &[(D30, Htm, LadenTravel)]);
    reader.slice(at(0), delta(vec![named(D30, false, &[(10, LadenTravel)])])).expect("taken");

    let early = reader.slice(at(20), delta(Vec::new())).expect_err("the slice at 10 s comes first");
    assert!(matches!(early, SpanError::NotNextSlice { begins, next } if (begins, next) == (at(20), at(10))), "{early}");
    let another = delta(vec![named(D30, true, &[(10, DROP)])]);
    let refused = reader.slice(at(10), another).expect_err("a span carrying on a state the robot was not in");
    assert!(matches!(refused, SpanError::Channel(WorldviewError::SpanDoesNotCarryOn { .. })), "{refused}");

    let carried = reader.slice(at(10), delta(vec![named(D30, true, &[(10, LadenTravel)])])).expect("taken");
    assert_eq!(carried, [spent(Htm, LadenTravel, 0, 10), SpanEvent::Through(at(10))]);
    let outside = reader.end(at(25)).expect_err("the last slice ends at 20 s");
    assert!(matches!(outside, SpanError::RunEndOutsideLastSlice { run_end, .. } if run_end == at(25)), "{outside}");
    reader.end(at(20)).expect("ends");
    let after = reader.slice(at(20), delta(Vec::new())).expect_err("the run has ended");
    assert!(matches!(after, SpanError::AfterRunEnd { run_end } if run_end == at(20)), "{after}");
}

/// A reader knows every robot's state from its first instant, so a keyframe must state each robot of the roster, and
/// only those, whose class says which definitions count it.
#[test]
fn a_keyframe_stating_other_robots_than_the_roster_names_seeds_no_reader() {
    let keyframe = RobotStateKeyframe(vec![(D30, NO_TASK)]);
    let both = [(D30, Htm), (DA50, Vtm)];
    let unstated = SpanReader::seeded(both, SLICE, at(0), &keyframe).expect_err("robot 9 has no state");
    assert!(matches!(unstated, SpanError::Unstated { robot: DA50, .. }), "{unstated}");
    let both_stated = RobotStateKeyframe(vec![(D30, NO_TASK), (DA50, NO_TASK)]);
    let off = SpanReader::seeded([(DA50, Vtm)], SLICE, at(0), &both_stated).expect_err("robot 7 has no class");
    assert!(matches!(off, SpanError::OffRoster { robot: D30, .. }), "{off}");
}
