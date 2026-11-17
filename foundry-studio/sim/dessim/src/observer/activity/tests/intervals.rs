//! The interval each milestone pattern makes: a leg is bounded by the
//! milestone it exists to reach, an operation by its own extent, a station
//! visit by its arrival and departure, and the stream's order is not its
//! chronology.

use desgraph::RobotType;

use crate::events::{SimEvent, TripPurpose};
use crate::model::{RobotId, SimTime};
use crate::observer::activity::ActivityKind;
use crate::observer::output::HtmWaitCause;
use crate::observer::time_slice::MetricsConfig;

use super::{
    ROBOT, arrive, depart, duty, dwell, fork_get, fork_get_resident, fork_put, gaps, holds, nav_to, observe,
    order_complete, run, seed, seed_typed,
};

/// THE EMISSION-ORDER TRAP. `HtmDeliver` reports its station arrival BEFORE
/// the departure that precedes it, because a task commits its whole chain in
/// one step and emits the facts in code order. Pairing milestones in the order
/// the facts are read closes the delivery leg with an instant earlier than
/// its own start, which books no travel and folds the station visit into it.
/// The leg must be bounded by chronology, not by the order the facts are read.
#[test]
fn a_delivery_chain_reported_arrival_first_still_pairs_by_instant() {
    let output = run(vec![
        seed(&[ROBOT]),
        // As HtmDeliver emits it: arrival first, then the departure before it.
        arrive(5_000),
        depart(TripPurpose::ToStation, 1_000),
        dwell(6_000, 2_000),
        // The return leg's departure closes the station visit.
        depart(TripPurpose::ToReturnRelay, 9_000),
        fork_put(11_000, 13_000),
    ]);

    let (traveling, loading, processing) = duty(&output, ROBOT);
    assert_eq!(traveling, 4_000 + 2_000, "[1s,5s) to the station and [9s,11s) to the relay");
    assert_eq!(processing, 4_000, "the station visit is [5s, 9s), not folded into travel");
    assert_eq!(loading, 2_000, "the fork-Put is its own interval");
}

/// A station visit the run stopped inside before its departure was scheduled
/// runs to the current world time, the earliest a departure scheduled later
/// could start: not to its dwell's end, nor to the run's last fact. Its stand
/// after the dwell is the bot loitering on the presentation cell until it could
/// leave, not a gap.
#[test]
fn a_station_visit_the_run_stopped_inside_runs_to_the_current_world_time() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 1_000),
        arrive(2_000),
        dwell(2_000, 3_000),
        SimEvent::BarrierAdvanced(SimTime::ms(8_000)),
        order_complete(10_000),
    ]);

    let (_, _, processing) = duty(&output, ROBOT);
    assert_eq!(processing, 6_000, "the visit runs [2s, 8s), past its dwell [2s, 5s) and short of the last fact");
    assert_eq!(holds(&output, HtmWaitCause::AwaitingDeparture), [(5_000, 3_000, false)], "the stand after the dwell");
    assert_eq!(gaps(&output), [(0, 1_000, false), (8_000, 2_000, false)], "and no gap before the current world time");
}

/// The other side of that rule: a departure scheduled later could not start
/// before the recorded dwell ends either, so a visit whose dwell ends after the
/// current world time runs to the dwell's end. No stand follows the dwell, and
/// from the dwell's end to the run's last fact the bot is in a gap.
#[test]
fn a_station_visit_the_run_stopped_inside_runs_to_a_dwell_that_ends_later() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 1_000),
        arrive(2_000),
        dwell(3_000, 6_000),
        SimEvent::BarrierAdvanced(SimTime::ms(6_000)),
        order_complete(10_000),
    ]);

    let (_, _, processing) = duty(&output, ROBOT);
    assert_eq!(processing, 7_000, "the visit runs [2s, 9s), to the end of its dwell [3s, 9s), past the world time");
    assert_eq!(holds(&output, HtmWaitCause::AwaitingStationService), [(2_000, 1_000, false)], "the hold before it");
    assert_eq!(holds(&output, HtmWaitCause::AwaitingDeparture), [], "and no stand after it");
    assert_eq!(gaps(&output), [(0, 1_000, false), (9_000, 1_000, false)], "the gaps before the visit and after it");
}

/// A station visit the run stopped inside with no `CycleComplete` recorded:
/// the presentation was never committed. It is a visit in progress all the
/// same, so it runs to the current world time like any other visit the run
/// stopped inside, rather than vanishing into idle or running on to the
/// run's last fact.
#[test]
fn a_visit_the_run_stopped_inside_with_no_dwell_runs_to_the_current_world_time() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 1_000),
        arrive(2_000),
        SimEvent::BarrierAdvanced(SimTime::ms(6_000)),
        order_complete(10_000),
    ]);

    let (traveling, _, processing) = duty(&output, ROBOT);
    assert_eq!(traveling, 1_000, "the inbound leg [1s, 2s)");
    assert_eq!(processing, 4_000, "the visit runs [2s, 6s), to the current world time");
    assert_eq!(gaps(&output), [(0, 1_000, false), (6_000, 4_000, false)], "the gaps before it and after it");
}

/// A run that stops inside a station visit with no fact stamped after the
/// dwell still lasts until the current world time: B had reached it, and the
/// visit stands until it. Ended at its last fact, the run would be shorter
/// than the visit it contains.
#[test]
fn a_run_stopped_inside_a_visit_lasts_until_the_current_world_time() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 1_000),
        arrive(2_000),
        dwell(3_000, 5_000),
        SimEvent::BarrierAdvanced(SimTime::ms(12_000)),
    ]);

    assert_eq!(output.sim_elapsed_ms, 12_000, "the current world time, past the last fact, the dwell ending at 8s");
    let (_, _, processing) = duty(&output, ROBOT);
    assert_eq!(processing, 10_000, "the visit [2s, 12s)");
    assert_eq!(holds(&output, HtmWaitCause::AwaitingDeparture), [(8_000, 4_000, false)], "the stand after the dwell");
    assert_eq!(gaps(&output), [(0, 1_000, false)], "and no gap after the visit, which lasts as long as the run");
}

/// A robot whose last reported fact is a fork operation: the run stopped with
/// the tote mid-handover. The relay fact carries the operation's own window,
/// so the whole of it books even though nothing closed it, and the run's last
/// instant is its end — leaving no trailing gap to journal.
#[test]
fn a_fork_operation_the_run_ended_on_books_whole() {
    let output = run(vec![seed(&[ROBOT]), depart(TripPurpose::ToRelay, 1_000), fork_get(4_000, 9_000)]);

    assert_eq!(duty(&output, ROBOT), (3_000, 5_000, 0), "the leg to the relay, then the whole fork");
    assert_eq!(gaps(&output), [(0, 1_000, false)], "only the gap before the bot set off");
}

/// A run that completes while a bot is still driving to park: nothing after
/// the drive's own nav says where it ends, so the nav's end is the run's
/// horizon, and the drive books whole. Ended at the departure instead, the
/// last leg of every completed run that parks its bots books no travel.
#[test]
fn a_run_that_ends_on_a_parking_drive_books_the_whole_drive() {
    let output = run(vec![seed(&[ROBOT]), depart(TripPurpose::ToParking, 10_000), nav_to(50, 10_000, 20_000)]);

    assert_eq!(duty(&output, ROBOT), (10_000, 0, 0), "the drive [10s, 20s)");
    assert_eq!(output.sim_elapsed_ms, 20_000, "the run's horizon is where the drive ends");
    assert_eq!(gaps(&output), [(0, 10_000, false)], "the gap before it set off, and none after it");
}

/// Two milestones at one instant bound an interval that occupies no time.
/// That is not a stretch of work, and every reader below assumes an interval
/// has extent: duty guards against it, but the inspector would take a record
/// it draws as a zero-width band and buckets into whichever slice the instant
/// lands in, and the journal would split one gap into two rows around it.
#[test]
fn an_interval_with_no_extent_reaches_nobody() {
    let output = run(vec![
        seed(&[ROBOT]),
        // A task commits its chain in one step: the leg to the relay and the
        // fork operation it exists to reach are both reported at 5s, so the
        // leg spans nothing.
        depart(TripPurpose::ToRelay, 5_000),
        fork_get(5_000, 6_000),
    ]);

    let records: Vec<_> = output
        .time_slices
        .iter()
        .flat_map(|slice| slice.station_diag.activity.iter())
        .map(|r| (r.kind, r.start.as_ms(), r.end.as_ms()))
        .collect();
    assert_eq!(records, [(ActivityKind::ForkOp, 5_000, 6_000)], "the fork alone; the empty leg is not an interval");
    assert_eq!(duty(&output, ROBOT), (0, 1_000, 0), "and only the fork is booked");
    assert_eq!(gaps(&output), [(0, 5_000, false)], "one gap up to the fork, not two around the empty leg");
}

/// Each fleet is read by the path that fleet belongs to, and by no other.
/// An Rtp is neither a D30 nor a DA50: booking its work as D30 duty would
/// inflate the D30 totals and put its holds in the D30 journal, and counting
/// it in the DA50 distribution would inflate a fleet it is not part of.
#[test]
fn a_third_fleet_is_counted_as_neither_d30_nor_da50() {
    let output =
        run(vec![seed_typed(&[(ROBOT, RobotType::Rtp)]), depart(TripPurpose::ToRelay, 1_000), fork_get(4_000, 9_000)]);

    assert_eq!(duty(&output, ROBOT), (0, 0, 0), "an Rtp books no D30 duty");
    assert_eq!(gaps(&output), [], "and no rows in the D30 journal");
    let closing = &output.cumulative_scalars;
    assert_eq!(closing.htm_activity_counts.values().sum::<u32>(), 0, "counted in neither distribution");
    assert_eq!(closing.vtm_activity_counts.values().sum::<u32>(), 0, "counted in neither distribution");
}

/// A bot that reached its presentation cell and left again in the same
/// instant made a station visit with no extent. Which of the two milestones
/// sorts first decides whether the run reads that as a zero-length visit
/// followed by the outbound leg, or as an outbound leg that started before
/// the bot arrived and swallowed the visit. The order the facts are read
/// cannot decide it: here the departure is read FIRST, the way a task that
/// commits its whole chain in one step reports it.
#[test]
fn an_arrival_and_a_departure_at_one_instant_make_a_zero_length_visit() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 1_000),
        // Read before the arrival it follows, and stamped at the same
        // instant: the visit has no extent.
        depart(TripPurpose::ToReturnRelay, 5_000),
        arrive(5_000),
        fork_put(9_000, 10_000),
    ]);

    let (traveling, fork, processing) = duty(&output, ROBOT);
    assert_eq!(processing, 0, "a visit with no extent is not a station visit");
    assert_eq!(traveling, 8_000, "the inbound leg [1s, 5s) and the outbound leg [5s, 9s)");
    assert_eq!(fork, 1_000, "the deposit that ends the outbound leg");
    assert_eq!(gaps(&output), [(0, 1_000, false)], "only the gap before the bot set off");
}

/// The slice convention, from both sides. A slice covers `[n * width,
/// (n + 1) * width)`, so an interval ending exactly on a boundary stops
/// there and reaches no further, while an instant stamped exactly on that
/// boundary opens the next slice and mints it: the event happened, and the
/// slice it happened in reports it. A run that stops exactly there has no
/// slice beginning at its stop, so no slice counts the instant, which the
/// run's journal still states.
#[test]
fn a_boundary_instant_opens_its_slice_and_a_boundary_interval_stops_short_of_it() {
    let width = MetricsConfig::default().slice_width_ms;
    let events = |stop_ms: u64| {
        vec![
            seed(&[ROBOT]),
            // A fork filling slice 0 exactly, ending on the boundary.
            fork_get(0, width),
            // The run's last fact, stamped exactly on that same boundary.
            order_complete(width),
            // B reaches the run's stop.
            SimEvent::BarrierAdvanced(SimTime::ms(stop_ms)),
        ]
    };
    let output = run(events(width + 1));

    assert_eq!(output.time_slices[0].scalars.htm_duty_loading_unloading_ms, width, "the fork fills slice 0");
    assert_eq!(output.total_slices, 2, "and the boundary instant opens slice 1");
    assert_eq!(
        output.time_slices[1].scalars.htm_duty_loading_unloading_ms, 0,
        "the fork ended on the boundary, so it reaches no part of slice 1"
    );
    assert_eq!(output.time_slices[1].scalars.orders_completed, 1, "slice 1 is where the closing order landed");

    let stopped_there = run(events(width));
    assert_eq!(stopped_there.total_slices, 1, "no slice begins at the run's stop");
    assert_eq!(stopped_there.completed_order_log.len(), 1, "the order at the stop is the run's");
}

/// The run's only interval, a leg ending exactly on a slice boundary with the
/// run, belongs to the slice it ran in and mints no slice after it. The
/// clipping convention is shared with the duty path, so the two can never
/// disagree about which slice a boundary-aligned interval lands in.
#[test]
fn an_interval_ending_on_a_slice_boundary_mints_no_empty_slice() {
    let width = MetricsConfig::default().slice_width_ms;
    let output = run(vec![seed(&[ROBOT]), depart(TripPurpose::ToParking, 0), nav_to(50, 0, width)]);

    assert_eq!(output.total_slices, 1, "slice 0 only: nothing happened in slice 1");
    assert_eq!(output.time_slices[0].scalars.htm_duty_traveling_ms, width, "the leg fills slice 0");
}

/// A parking leg is the one leg with nothing waiting at the end of it, so its
/// own navigation is what closes it. Standing at the parking cell afterwards
/// is a gap: the bot is inside no scheduled task, it is waiting for one. Left
/// open, the leg would run all the way to the next task's departure and book
/// the whole stand as travel duty.
#[test]
fn a_parking_leg_ends_where_its_navigation_ends() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToParking, 1_000),
        nav_to(50, 1_000, 3_000),
        // The next task picks the bot up again much later.
        depart(TripPurpose::ToRelay, 20_000),
        fork_get(22_000, 23_000),
    ]);

    let (traveling, fork, _) = duty(&output, ROBOT);
    assert_eq!(traveling, 4_000, "the parking drive [1s, 3s) and the pickup leg [20s, 22s), not the stand between");
    assert_eq!(fork, 1_000);
    assert_eq!(
        gaps(&output),
        [(0, 1_000, false), (3_000, 17_000, false)],
        "the gap before it set off, and the stand at the parking cell"
    );
}

/// A leg with no operation of its own task after it ends with its nav, even
/// when the next task opens with an operation where the leg left the bot: no
/// leg was sent to a fork-Get its task found the bot already standing at, so
/// the stand between them is a gap, not the tail of the earlier leg.
#[test]
fn a_leg_does_not_run_on_into_the_next_tasks_resident_operation() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToParking, 1_000),
        nav_to(50, 1_000, 3_000),
        // A pickup task later finds the bot standing there.
        fork_get_resident(10_000, 11_000, 7_000),
    ]);

    assert_eq!(duty(&output, ROBOT), (2_000, 1_000, 0), "the drive [1s, 3s) and the fork-Get, not the stand");
}

/// A dwell no station visit contains, as the fulfillment tasks report their
/// station work, is the robot's work at the station: the leg that brought it
/// there runs to the dwell, and the stand after the dwell is a gap. No visit
/// will claim it, so it waits in no queue.
#[test]
fn a_dwell_no_visit_contains_is_the_robots_station_work() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 0),
        nav_to(50, 0, 1_000),
        dwell(2_000, 500),
        depart(TripPurpose::ToParking, 5_000),
    ]);
    let (traveling, _, processing) = duty(&output, ROBOT);
    assert_eq!((traveling, processing), (2_000, 500), "the leg to the dwell [0s, 2s), and the dwell itself");
    assert_eq!(gaps(&output), [(2_500, 2_500, false)], "the stand after the work, until the next leg");

    let state = observe(&[seed(&[ROBOT]), depart(TripPurpose::ToStation, 0), dwell(2_000, 500)]);
    assert!(!state.activity.visits.pending_dwell.contains_key(&RobotId(ROBOT)));
}

/// A nav no `TripStart` announced, as the fulfillment tasks move totes between
/// cells, is a leg all the same: it opens at the nav's start and, with no
/// operation after it, ends with the nav.
#[test]
fn a_nav_no_trip_announced_is_a_leg_of_its_own() {
    let output = run(vec![
        seed(&[ROBOT]),
        nav_to(50, 1_000, 4_000),
        depart(TripPurpose::ToRelay, 10_000),
        fork_get(12_000, 13_000),
    ]);
    let (traveling, fork, _) = duty(&output, ROBOT);
    assert_eq!((traveling, fork), (3_000 + 2_000, 1_000), "the unannounced leg [1s, 4s) and the pickup leg");
    assert_eq!(gaps(&output), [(0, 1_000, false), (4_000, 6_000, false)]);
}

/// A leg with no operation after it ends where its nav does, whatever its
/// trip's purpose: the bot standing at its destination with nothing to do
/// there is in a gap until its next leg.
#[test]
fn a_leg_nothing_waits_at_ends_with_its_nav() {
    let output = run(vec![
        seed(&[ROBOT]),
        depart(TripPurpose::ToStation, 1_000),
        nav_to(50, 1_000, 3_000),
        depart(TripPurpose::ToRelay, 10_000),
        fork_get(12_000, 13_000),
    ]);
    assert_eq!(duty(&output, ROBOT), (2_000 + 2_000, 1_000, 0));
    assert_eq!(gaps(&output), [(0, 1_000, false), (3_000, 7_000, false)]);
}
