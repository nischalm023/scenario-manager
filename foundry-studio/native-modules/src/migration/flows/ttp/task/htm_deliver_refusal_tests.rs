//! A direct supply's delivery, whose path search may not await (scheduler
//! time) other robots' commits before its first window: refused when the
//! station's cell is held open-ended and no queue cell feeds it, it assigns
//! nothing; departing, its HTM's assignment is announced once, ahead of its
//! arrival in the event stream, whether its drive reaches the station's cell
//! in one window or waits in the station's queue first.

use std::sync::Arc;

use super::queue_tests::{RELEASE, STATION, STATION_CELL, delivery_to, station_queue};
use super::*;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{EmittedFacts, TestModel, TestModelConfig, block, bounded_scheduler, build_model};
use crate::model::{StationArrivalPlan, StationArrivalSource};
use crate::scheduler::Scheduler;
use crate::simtime::TimeRange;

const ROBOT: RobotId = RobotId(60);

/// Station 1 on the test line's cell 10, no queue cell feeding it, and a
/// journal that keeps every fact.
fn station_alone() -> (Arc<TestModel>, Arc<TableOracle>, Scheduler, Arc<EmittedFacts>) {
    let (mut model, oracle) =
        build_model(TestModelConfig { stations: vec![(STATION, STATION_CELL)], ..Default::default() });
    let scheduler = bounded_scheduler();
    let emitted = Arc::new(EmittedFacts::new(scheduler.barrier().clock()));
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(emitted.clone()));
    (model, oracle, scheduler, emitted)
}

/// Robot 60, loaded on cell 5, delivering to station 1 as a direct supply
/// does: refusable, with its assignment held until the journey commits.
fn direct_supply(model: &TestModel, oracle: &TableOracle, scheduler: &Scheduler) -> HtmDeliver {
    let mut delivery = delivery_to(model, oracle, scheduler, ROBOT, CellId(5), STATION, SimTime::ZERO);
    let quoted = SimTime::seconds(5);
    let source = StationArrivalSource::PutawayDirect;
    delivery.handoff.as_mut().unwrap().station_arrival_plan =
        Some(StationArrivalPlan::new(STATION, source, SimTime::ZERO, quoted));
    delivery.superseding_clearance(HtmAssignmentEvent {
        robot_id: ROBOT,
        station_id: STATION,
        tote_id: Some(ToteId(ROBOT.0)),
        source,
        relay_cell: None,
        dispatch_time: SimTime::ZERO,
        quoted_arrival: quoted,
    })
}

#[test]
fn a_direct_supply_that_could_only_wait_on_its_origin_is_refused_and_assigns_nothing() {
    let (model, oracle, mut scheduler, emitted) = station_alone();
    oracle.table().reserve_cell(RobotId(61), STATION_CELL, TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let delivered = scheduler.add_task(direct_supply(&model, &oracle, &scheduler));
    scheduler.run().unwrap();

    assert!(block(delivered).is_err(), "the delivery is refused");
    assert!(emitted.windows_of(ROBOT).is_empty(), "nothing of its journey is committed");
    assert!(emitted.assignments_and_arrivals_of(ROBOT).is_empty(), "a refused delivery assigns nothing");
}

#[test]
fn a_direct_supply_whose_drive_is_one_window_is_assigned_once_before_it_arrives() {
    let (model, oracle, mut scheduler, emitted) = station_alone();
    let delivered = scheduler.add_task(direct_supply(&model, &oracle, &scheduler));
    scheduler.run().unwrap();

    assert!(block(delivered).is_ok());
    assert_eq!(emitted.windows_of(ROBOT), [(STATION_CELL, SimTime::ZERO)], "one window, at its departure");
    assert_eq!(emitted.assignments_and_arrivals_of(ROBOT), ["assignment", "arrival"]);
    assert!(emitted.behind_the_barrier().is_empty(), "{:?}", emitted.behind_the_barrier());
}

/// Robot 61 holds station 1's cell open-ended until its departure is
/// scheduled at 30 s: the direct supply's first window ends on the queue cell
/// next to the station, and its HTM waits there, which is not a wait on its
/// origin.
#[test]
fn a_direct_supply_that_drives_into_the_queue_waits_there_and_is_not_refused() {
    let (model, oracle, mut scheduler, emitted) = station_queue();
    let delivered = scheduler.add_task(direct_supply(&model, &oracle, &scheduler));
    scheduler.run().unwrap();

    assert!(block(delivered).is_ok(), "the delivery is not refused");
    let windows = emitted.windows_of(ROBOT);
    assert_eq!(windows.len(), 2, "one window into the queue, one onto the station's cell: {windows:?}");
    assert_eq!(windows[0], (CellId(9), SimTime::ZERO));
    assert_eq!(windows[1].0, STATION_CELL);
    assert!(windows[1].1 >= RELEASE, "it moves up once the holder's departure is scheduled: {windows:?}");
    assert_eq!(emitted.assignments_and_arrivals_of(ROBOT), ["assignment", "arrival"]);
    assert!(emitted.behind_the_barrier().is_empty(), "{:?}", emitted.behind_the_barrier());
}
