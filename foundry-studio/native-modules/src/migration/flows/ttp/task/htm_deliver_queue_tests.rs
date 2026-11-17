//! A delivery foreshortened into its station's queue: on the test line, cells
//! 7, 8 and 9 are the queue feeding station 1's cell 10, and an idle HTM holds
//! that cell open-ended until its departure is scheduled at 30 s.

use std::sync::{Arc, Mutex};

use desgraph::ZoneId;

use super::*;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{
    EmittedFacts, TestModel, TestModelConfig, bounded_scheduler, build_model, register_idle_htm, register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::Orientation;
use crate::scheduler::Scheduler;
use crate::utils::temporal_counter::TemporalCounter;
use pathfinder::ReservationOracle as _;

pub(super) const STATION: StationId = StationId(1);
pub(super) const STATION_CELL: CellId = CellId(10);
pub(super) const RELEASE: SimTime = SimTime::seconds(30);

pub(super) fn station_queue() -> (Arc<TestModel>, Arc<TableOracle>, Scheduler, Arc<EmittedFacts>) {
    let (mut model, oracle) = build_model(TestModelConfig {
        stations: vec![(STATION, STATION_CELL)],
        queue_cells: vec![CellId(7), CellId(8), CellId(9)],
        ..Default::default()
    });
    let mut scheduler = bounded_scheduler();
    let emitted = Arc::new(EmittedFacts::new(scheduler.barrier().clock()));
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(emitted.clone()));
    register_idle_htm(&model, &oracle, RobotId(61), STATION_CELL, SimTime::ZERO);
    let holder = model.available_htm.take(RobotId(61)).unwrap();
    scheduler.add_task(Leave {
        journey: Some(navigate(&model, &scheduler, holder.cell_reservation, RELEASE, CellId(59), None)),
        available: model.available_htm.clone(),
    });
    (model, oracle, scheduler, emitted)
}

fn navigate(
    model: &TestModel,
    scheduler: &Scheduler,
    from: CellReservation,
    at: SimTime,
    to: CellId,
    carrying: Option<ToteId>,
) -> Navigate {
    Navigate::new(
        HtmPosition { cell_reservation: from, action_ready_at: at, orientation: Orientation::East },
        to,
        carrying,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    )
}

/// A loaded HTM at `from` delivering tote `tote` to the station.
fn delivery(
    model: &TestModel,
    oracle: &TableOracle,
    scheduler: &Scheduler,
    robot: RobotId,
    from: CellId,
) -> HtmDeliver {
    delivery_to(model, oracle, scheduler, robot, from, STATION, SimTime::ZERO)
}

/// A loaded HTM at `from` delivering its tote to `station`, ready to depart
/// at `ready`.
pub(super) fn delivery_to(
    model: &TestModel,
    oracle: &TableOracle,
    scheduler: &Scheduler,
    robot: RobotId,
    from: CellId,
    station: StationId,
    ready: SimTime,
) -> HtmDeliver {
    let tote = ToteId(robot.0);
    model
        .inventory
        .register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, ToteLocation::WithRobot { robot_id: robot })
        .unwrap();
    register_loaded_htm(model, oracle, robot, from, tote);
    let source = model.available_htm.take(robot).unwrap();
    HtmDeliver::new(
        station,
        InflightSpan::open(Arc::new(Mutex::new(TemporalCounter::new())), SimTime::ZERO, Version::new()),
        HtmCarryingTote {
            tote_id: tote,
            zone_id: ZoneId(1),
            cell_reservation: source.cell_reservation,
            action_ready_at: ready,
            orientation: Orientation::East,
            station_arrival_plan: None,
            presentation_id: PresentationId(u64::from(robot.0)),
            source_relay_id: None,
            deliver_from: None,
        },
        scheduler.barrier().clock(),
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        model.station_service.clone(),
        model.station_arrivals.clone(),
        None,
        false,
        None,
    )
}

#[test]
fn a_delivery_to_a_held_station_cell_stands_on_its_deepest_free_queue_cell_until_the_holders_departure_is_scheduled() {
    let (model, oracle, mut scheduler, emitted) = station_queue();
    let robot = RobotId(60);
    scheduler.add_task(delivery(&model, &oracle, &scheduler, robot, CellId(5)));
    scheduler.run().unwrap();

    let windows = emitted.windows_of(robot);
    assert_eq!(windows.len(), 2, "one window into the queue, one onto the station's cell: {windows:?}");
    assert_eq!(windows[0], (CellId(9), SimTime::ZERO), "it drives at once to the queue cell next to the station");
    assert_eq!(windows[1].0, STATION_CELL);
    assert!(windows[1].1 >= RELEASE, "it continues once the holder's departure is scheduled: {windows:?}");
    assert!(emitted.behind_the_barrier().is_empty(), "{:?}", emitted.behind_the_barrier());
    assert!(oracle.find_overlaps().is_empty());
}

#[test]
fn a_second_delivery_ends_one_queue_cell_back_and_moves_up_after_the_first() {
    let (model, oracle, mut scheduler, emitted) = station_queue();
    let (first, second) = (RobotId(60), RobotId(62));
    scheduler.add_task(DeliverThenLeave {
        deliver: Some(delivery(&model, &oracle, &scheduler, first, CellId(5))),
        model: model.clone(),
        leave_at: SimTime::seconds(60),
    });
    scheduler.add_task(delivery(&model, &oracle, &scheduler, second, CellId(3)));
    scheduler.run().unwrap();

    let leads = emitted.windows_of(first);
    let follows = emitted.windows_of(second);
    assert_eq!(follows.len(), 3, "behind the first, then up one, then the station: {follows:?}");
    assert_eq!(follows[0].0, CellId(8), "the second stands one cell back: {follows:?}");
    assert_eq!(follows[1].0, CellId(9));
    assert!(follows[1].1 >= leads[1].1, "it moves up only once the first's onward window is scheduled");
    assert_eq!(follows[2].0, STATION_CELL);
    assert!(
        follows[2].1 >= SimTime::seconds(60),
        "and onto the station once the first's departure from it is scheduled"
    );
    assert!(emitted.behind_the_barrier().is_empty(), "{:?}", emitted.behind_the_barrier());
    assert!(oracle.find_overlaps().is_empty());
}

/// A direct supply's shape: robot 61, loaded, stands on station 1's cell and
/// delivers to station 2 once its operator releases it at `RELEASE`. Robot
/// 60's delivery to station 1 stands in the queue meanwhile; it moves up only
/// once robot 61's departure is scheduled, and that departure is not moved.
#[test]
fn a_delivery_leaving_the_station_cell_departs_at_its_release_and_the_one_in_the_queue_moves_up_after_it() {
    let (mut model, oracle) = build_model(TestModelConfig {
        stations: vec![(STATION, STATION_CELL), (StationId(2), CellId(15))],
        queue_cells: vec![CellId(7), CellId(8), CellId(9)],
        ..Default::default()
    });
    let mut scheduler = bounded_scheduler();
    let emitted = Arc::new(EmittedFacts::new(scheduler.barrier().clock()));
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(emitted.clone()));
    let (leaving, arriving) = (RobotId(61), RobotId(60));
    scheduler.add_task(delivery_to(&model, &oracle, &scheduler, leaving, STATION_CELL, StationId(2), RELEASE));
    scheduler.add_task(delivery(&model, &oracle, &scheduler, arriving, CellId(5)));
    scheduler.run().unwrap();

    let leaves = emitted.windows_of(leaving);
    assert_eq!(leaves, [(CellId(15), RELEASE)], "the leaving delivery departs at its release, not earlier or later");
    let arrives = emitted.windows_of(arriving);
    assert_eq!(arrives.len(), 2, "one window into the queue, one onto the station's cell: {arrives:?}");
    assert_eq!(arrives[0], (CellId(9), SimTime::ZERO));
    assert_eq!(arrives[1].0, STATION_CELL);
    assert!(arrives[1].1 >= RELEASE, "it moves up once the leaving delivery's departure is scheduled: {arrives:?}");
    assert!(emitted.behind_the_barrier().is_empty(), "{:?}", emitted.behind_the_barrier());
    assert!(oracle.find_overlaps().is_empty());
}

/// Move a robot on at a known instant and give it back to the fleet.
#[derive(Display)]
#[display("Leave")]
struct Leave {
    journey: Option<Navigate>,
    available: AvailableHtmRegistry,
}

#[async_trait(?Send)]
impl Task for Leave {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let position = ctx.add_task(self.journey.take().unwrap()).await;
        self.available
            .register(position.cell_reservation, position.action_ready_at, position.orientation, None)
            .map_err(|error| PlanError::TaskGraphInvariant { context: "Leave", detail: error.to_string() })?;
        Ok(())
    }
}

/// A delivery whose HTM leaves the station at `leave_at`, as its operator
/// releases it.
#[derive(Display)]
#[display("DeliverThenLeave")]
struct DeliverThenLeave {
    deliver: Option<HtmDeliver>,
    model: Arc<TestModel>,
    leave_at: SimTime,
}

#[async_trait(?Send)]
impl Task for DeliverThenLeave {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let presented = ctx.add_task(self.deliver.take().unwrap()).await?;
        let mut journey = Navigate::new(
            HtmPosition {
                cell_reservation: presented.cell_reservation,
                action_ready_at: presented.action_ready_at.max(self.leave_at),
                orientation: presented.orientation,
            },
            CellId(40),
            Some(presented.tote_id),
            self.model.navigation.clone(),
            self.model.bot_service.clone(),
            self.model.inventory.clone(),
            None,
            self.model.events.clone(),
            ctx.barrier().clock(),
        );
        journey = journey.not_before(self.leave_at);
        ctx.add_task(journey).await;
        Ok(())
    }
}
