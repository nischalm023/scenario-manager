use super::*;

const D30: RobotId = RobotId(7);
const DA50: RobotId = RobotId(9);

fn at(seconds: u64) -> SimTime {
    SimTime::ms(seconds * 1_000)
}

fn secs(seconds: u64) -> SimDuration {
    SimDuration::ms(seconds * 1_000)
}

fn waiting(cause: WaitCause, cell: CellRole) -> RobotState {
    RobotState::Waiting { cause, cell: Some(cell) }
}

fn no_task(cell: CellRole) -> RobotState {
    RobotState::NoTask { cell: Some(cell) }
}

fn fleet() -> HashMap<RobotId, RobotType> {
    HashMap::from([(D30, RobotType::Htm), (DA50, RobotType::Vtm)])
}

/// The spans `robot` is named with over `[from, to)`, or `None` when the slice does not name it.
fn slice(
    log: &mut RobotStateLog,
    robot: RobotId,
    from: u64,
    to: u64,
) -> Option<(bool, Vec<(SimDuration, RobotState)>)> {
    let named = log.state_slice(at(from), at(to), &fleet());
    named.into_iter().find(|spans| spans.robot == robot).map(|spans| (spans.continues, spans.spans))
}

/// The explainer's drawing: laden travel entered once at 14 s is cut at 20 s and 30 s, each slice holding its own
/// part of it, the slice in between not naming the robot, and the last carrying it on into the drop.
#[test]
fn a_span_cut_at_the_slice_edges_is_one_entry_the_slices_after_carry_on() {
    let mut log = RobotStateLog::default();
    log.stand(D30, at(36), Some(CellRole::Route));
    log.note(D30, (at(5), at(14)), RobotState::EmptyTravel, Claim::Committed);
    log.note(D30, (at(14), at(33)), RobotState::LadenTravel, Claim::Committed);
    log.note(D30, (at(33), at(36)), RobotState::Drop, Claim::Committed);
    slice(&mut log, D30, 0, 10);

    assert_eq!(
        slice(&mut log, D30, 10, 20),
        Some((true, vec![(secs(4), RobotState::EmptyTravel), (secs(6), RobotState::LadenTravel)]))
    );
    assert_eq!(slice(&mut log, D30, 20, 30), None, "laden all slice, in the span it was in as the slice began");
    let after = no_task(CellRole::Route);
    assert_eq!(
        slice(&mut log, D30, 30, 40),
        Some((true, vec![(secs(3), RobotState::LadenTravel), (secs(3), RobotState::Drop), (secs(4), after)]))
    );
}

/// A committed span claims its time before the operator's work, which claims it before the visit: awaiting
/// service before the work; after it, no task until the nav that takes the robot away is read, then awaiting that
/// departure; and no task once the visit is over.
#[test]
fn a_visit_awaits_service_before_the_operators_work_and_its_departure_once_that_is_committed() {
    let mut log = RobotStateLog::default();
    log.stand(D30, SimTime::ZERO, Some(CellRole::Relay));
    log.note(D30, (at(0), at(2)), RobotState::LadenTravel, Claim::Committed);
    log.open_visit(D30, at(2), Some(CellRole::Presentation));
    // A putaway's work can begin while the bot still travels to it, and the travel keeps its time.
    log.note(D30, (at(1), at(6)), RobotState::StationProcessing, Claim::OperatorWork);
    log.serve_visit(D30, at(2), at(6));
    log.close_visit(D30, at(2), at(9));
    // The nav that takes the robot away departs at 9 s and is read with B at 7 s.
    log.await_departure(D30, at(7), at(9));
    log.note(D30, (at(9), at(10)), RobotState::LadenTravel, Claim::Committed);

    let (continues, spans) = slice(&mut log, D30, 0, 10).expect("named in its first slice");
    assert!(!continues, "nothing comes before the run's first slice");
    assert_eq!(
        spans,
        [
            (secs(2), RobotState::LadenTravel),
            (secs(4), RobotState::StationProcessing),
            (secs(1), no_task(CellRole::Presentation)),
            (secs(2), waiting(WaitCause::AwaitingDeparture, CellRole::Presentation)),
            (secs(1), RobotState::LadenTravel),
        ]
    );
}

/// A visit no departure has closed runs past the slice: the next slice carries on its wait, and its work, read
/// later but before B passes it, ends that wait; once served, the robot has no task until a nav is read, which
/// commits its departure.
#[test]
fn an_open_visit_is_stated_from_what_is_known_as_each_slice_closes() {
    let mut log = RobotStateLog::default();
    log.open_visit(D30, at(5), Some(CellRole::Presentation));
    let awaiting = waiting(WaitCause::AwaitingStationService, CellRole::Presentation);
    let (_, spans) = slice(&mut log, D30, 0, 10).expect("named");
    assert_eq!(spans[1], (secs(5), awaiting), "no work read yet: the bot awaits station service");

    log.note(D30, (at(12), at(15)), RobotState::StationProcessing, Claim::OperatorWork);
    log.serve_visit(D30, at(5), at(15));
    let idle = no_task(CellRole::Presentation);
    assert_eq!(
        slice(&mut log, D30, 10, 20),
        Some((true, vec![(secs(2), awaiting), (secs(3), RobotState::StationProcessing), (secs(5), idle)]))
    );
    log.close_visit(D30, at(5), at(26));
    log.await_departure(D30, at(23), at(26));
    log.note(D30, (at(26), at(30)), RobotState::LadenTravel, Claim::Committed);
    let departing = waiting(WaitCause::AwaitingDeparture, CellRole::Presentation);
    assert_eq!(
        slice(&mut log, D30, 20, 30),
        Some((true, vec![(secs(3), idle), (secs(3), departing), (secs(4), RobotState::LadenTravel)]))
    );
}

/// A nav read before it departs commits the robot's next move from then, wherever the robot stands: it awaits the
/// departure on that cell. Before the nav is read, no fact names a task, and the robot has none.
#[test]
fn a_robot_awaits_a_departure_from_the_instant_its_nav_is_read() {
    let mut log = RobotStateLog::default();
    log.stand(D30, SimTime::ZERO, Some(CellRole::Route));
    log.await_departure(D30, at(3), at(8));
    log.note(D30, (at(8), at(10)), RobotState::EmptyTravel, Claim::Committed);
    log.await_departure(D30, at(8), at(8));
    let (_, spans) = slice(&mut log, D30, 0, 10).expect("named");
    let departing = waiting(WaitCause::AwaitingDeparture, CellRole::Route);
    assert_eq!(
        spans,
        [(secs(3), no_task(CellRole::Route)), (secs(5), departing), (secs(2), RobotState::EmptyTravel)],
        "a nav read as it departs commits no stand"
    );
}

/// The operator's work outside any visit, as the fulfillment tasks report it: the robot awaits it where its nav
/// left it, from the instant the work was read, and before that stands in no task.
#[test]
fn the_stand_before_work_outside_a_visit_awaits_it_once_the_work_is_read() {
    let mut log = RobotStateLog::default();
    log.note(D30, (at(0), at(2)), RobotState::LadenTravel, Claim::Committed);
    log.stand(D30, at(2), Some(CellRole::Presentation));
    log.await_work(D30, at(6), at(4));
    log.note(D30, (at(6), at(9)), RobotState::StationProcessing, Claim::OperatorWork);
    let (_, spans) = slice(&mut log, D30, 0, 10).expect("named");
    assert_eq!(
        spans,
        [
            (secs(2), RobotState::LadenTravel),
            (secs(2), no_task(CellRole::Presentation)),
            (secs(2), waiting(WaitCause::AwaitingStationService, CellRole::Presentation)),
            (secs(3), RobotState::StationProcessing),
            (secs(1), no_task(CellRole::Presentation)),
        ]
    );
}

/// One span is one entry: two pickups side by side stay two spans, and a pickup that begins with the slice is new
/// though the robot ended the slice before in a pickup; a gap across a slice edge is one gap carried on.
#[test]
fn spans_side_by_side_in_one_state_stay_apart_and_a_gap_carries_on() {
    let mut log = RobotStateLog::default();
    log.note(DA50, (at(2), at(4)), RobotState::Pickup, Claim::Committed);
    log.note(DA50, (at(4), at(6)), RobotState::Pickup, Claim::Committed);
    log.note(DA50, (at(8), at(10)), RobotState::Pickup, Claim::Committed);
    log.note(DA50, (at(10), at(12)), RobotState::Pickup, Claim::Committed);
    let on_rail = RobotState::NoTask { cell: None };
    let (_, spans) = slice(&mut log, DA50, 0, 10).expect("named");
    let pickup = (secs(2), RobotState::Pickup);
    assert_eq!(spans, [(secs(2), on_rail), pickup, pickup, (secs(2), on_rail), pickup], "a DA50 stands on no cell");
    let (continues, spans) = slice(&mut log, DA50, 10, 20).expect("named");
    assert!(!continues, "a pickup beginning at the slice's start is a new entry, after a pickup or not");
    assert_eq!(spans, [pickup, (secs(8), on_rail)]);
    assert_eq!(slice(&mut log, DA50, 20, 30), None, "the gap from 12 s carries on, unnamed");
}

/// A fact read once the slices it reaches back into were stated changes none of them: the slice it is read
/// before states it as a new entry from its own start.
#[test]
fn a_fact_read_after_its_first_slice_was_stated_reaches_only_the_slices_after() {
    let mut log = RobotStateLog::default();
    log.stand(D30, SimTime::ZERO, Some(CellRole::Charging));
    let idle = no_task(CellRole::Charging);
    assert_eq!(slice(&mut log, D30, 0, 10), Some((false, vec![(secs(10), idle)])));
    log.note(D30, (at(6), at(14)), RobotState::EmptyTravel, Claim::Committed);
    assert_eq!(slice(&mut log, D30, 10, 20), Some((false, vec![(secs(4), RobotState::EmptyTravel), (secs(6), idle)])));
}

/// The run's last slice ends at its stop: a robot's spans are stated up to there, and a span committed past the
/// stop is not.
#[test]
fn a_slice_the_runs_stop_cuts_states_the_robots_up_to_the_stop() {
    let mut log = RobotStateLog::default();
    log.stand(D30, SimTime::ZERO, Some(CellRole::Relay));
    log.note(D30, (at(3), at(14)), RobotState::EmptyTravel, Claim::Committed);
    assert_eq!(
        slice(&mut log, D30, 0, 6),
        Some((false, vec![(secs(3), no_task(CellRole::Relay)), (secs(3), RobotState::EmptyTravel)])),
    );
}

/// Every robot of the fleet is named in its first slice, a robot no fact names included, and a slice forgets the
/// facts it used up, so what the log holds stays bounded by what is still ahead of the stated slices.
#[test]
fn every_robot_is_named_once_and_what_a_slice_used_up_is_forgotten() {
    let mut log = RobotStateLog::default();
    log.stand(D30, SimTime::ZERO, Some(CellRole::Route));
    log.stand(D30, at(4), Some(CellRole::Relay));
    log.note(D30, (at(2), at(4)), RobotState::EmptyTravel, Claim::Committed);
    log.note(D30, (at(15), at(25)), RobotState::EmptyTravel, Claim::Committed);
    log.open_visit(D30, at(30), Some(CellRole::Presentation));
    log.await_departure(D30, at(1), at(2));
    log.await_departure(D30, at(5), at(15));
    let named: Vec<RobotId> = log.state_slice(at(0), at(10), &fleet()).iter().map(|spans| spans.robot).collect();
    assert_eq!(named, [D30, DA50], "in ascending id, the DA50 with no fact included");

    let facts = &log.robots[&D30];
    assert_eq!(facts.noted.len(), 1, "the travel over by 10 s is forgotten, the one from 15 s kept");
    assert_eq!(facts.stands, [(at(4), Some(CellRole::Relay))], "only the stand in force at 10 s is kept");
    assert_eq!(facts.visits.len(), 1, "the visit not yet closed is kept");
    assert_eq!(facts.departures.len(), 1, "the departure at 2 s is forgotten, the one at 15 s kept");
    assert!(log.state_slice(at(10), at(10), &fleet()).is_empty(), "a slice with no extent names no robot");
}

/// A visit forgotten before it closes holds the robot for no time: the robot stands with no task where it stood.
/// A closed visit with the same arrival is kept.
#[test]
fn a_visit_discarded_before_it_closes_holds_the_robot_for_no_time() {
    let mut log = RobotStateLog::default();
    log.stand(D30, SimTime::ZERO, Some(CellRole::Route));
    log.open_visit(D30, at(4), Some(CellRole::Presentation));
    log.discard_visit(D30, at(4));
    assert_eq!(slice(&mut log, D30, 0, 10), Some((false, vec![(secs(10), no_task(CellRole::Route))])));

    log.open_visit(D30, at(12), Some(CellRole::Presentation));
    log.close_visit(D30, at(12), at(15));
    log.discard_visit(D30, at(12));
    let awaiting = waiting(WaitCause::AwaitingStationService, CellRole::Presentation);
    let (_, spans) = slice(&mut log, D30, 10, 20).expect("named");
    assert_eq!(spans[1], (secs(3), awaiting), "a closed visit is not discarded");
}

/// Two committed spans over one instant of one robot's time contradict each other: the first such pair is kept for
/// the observer, which stops the run on it. Spans that only meet, the operator's work over a committed span, and two
/// robots' spans do not.
#[test]
fn two_committed_spans_over_one_instant_are_an_overlap_the_observer_reads() {
    let mut log = RobotStateLog::default();
    log.note(D30, (at(0), at(4)), RobotState::EmptyTravel, Claim::Committed);
    log.note(D30, (at(4), at(6)), RobotState::Pickup, Claim::Committed);
    log.note(D30, (at(3), at(8)), RobotState::StationProcessing, Claim::OperatorWork);
    log.note(DA50, (at(3), at(5)), RobotState::Drop, Claim::Committed);
    assert_eq!(log.take_overlap(), None, "spans that meet, work over travel, and another robot's span");

    log.note(D30, (at(5), at(7)), RobotState::LadenTravel, Claim::Committed);
    let overlap = log.take_overlap().expect("the travel from 5 s overlaps the pickup to 6 s");
    assert_eq!(
        overlap,
        CommittedSpansOverlap {
            robot: D30,
            first: (at(4), at(6), RobotState::Pickup),
            second: (at(5), at(7), RobotState::LadenTravel),
        }
    );
    assert_eq!(log.take_overlap(), None, "an overlap is read once");
}
