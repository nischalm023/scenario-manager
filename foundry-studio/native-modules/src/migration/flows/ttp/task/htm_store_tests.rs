//! Storage retries must observe a relay access holder's navigation and plan
//! the loaded departure from the current simulation time after that wait.

use crate::pathfinding::FHSippPathfinder;
use crate::robots::runtime::RobotModelId;
use desgraph::BlockId;
use desmap::{BlockSide, RelayPosition, VtmPosition};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use tokio::runtime::Builder;

use super::*;
use crate::events::SimEvent;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::flows::ttp::task::VtmReturn;
use crate::model::reservation::oracle::{ReservationOracle, TableOracle};
use crate::model::test_model::block;
use crate::model::test_model::{
    TestModel, TestModelConfig, bounded_scheduler, build_model, register_idle_htm, register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteType};
use crate::pathfinding::PathCacheKey;
use pathfinder::ReservationOracle as _;

#[derive(Display)]
#[display("StoreAndReturn")]
struct StoreAndReturn {
    store: Option<HtmStore>,
    model: Arc<TestModel>,
}

#[async_trait(?Send)]
impl Task for StoreAndReturn {
    type Proposal = EmptyProposal;
    type Schedule = SimTime;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<SimTime, PlanError> {
        let deposited = ctx.add_task(self.store.take().unwrap()).await;
        Ok(ctx
            .add_task(VtmReturn::new(
                deposited,
                self.model.relay_service.vtm_service.clone(),
                self.model.inventory.clone(),
                self.model.events.clone(),
            ))
            .await)
    }
}

#[test]
fn storage_wakes_on_oracle_clearance_and_cannot_backdate_the_retry() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let storing = RobotId(60);
    let blocker = RobotId(61);
    let tote = ToteId(1);
    model
        .inventory
        .register_tote(
            tote,
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
        )
        .unwrap();
    model.inventory.tote(tote).unwrap().move_to(ToteLocation::WithRobot { robot_id: storing }, SimTime::ZERO).unwrap();
    register_loaded_htm(&model, &oracle, storing, CellId(5), tote);
    let source = model.available_htm.take(storing).unwrap();
    let deposit = block(model.relay_service.reserve_relay_for_deposit(
        CellId(5),
        Default::default(),
        SimTime::ZERO,
        storing,
        None,
    ))
    .unwrap();
    let access = model.relay_service.htm_access_cell(deposit.relay_id).unwrap();
    assert_eq!(access, CellId(10));
    register_idle_htm(&model, &oracle, blocker, access, SimTime::ZERO);
    // The blocker is already assigned: its departure changes neither the
    // available fleet nor inventory, only the navigation oracle and bot plan.
    let blocking = model.available_htm.take(blocker).unwrap();
    assert_eq!(model.relay_service.htm_access_holder(deposit.relay_id), Some(blocker));
    let mut scheduler = bounded_scheduler();
    scheduler.add_task(StoreAndReturn {
        store: Some(HtmStore::new(
            StationId(0),
            storing,
            tote,
            source.cell_reservation,
            SimTime::ZERO,
            deposit,
            Some(PresentationId(1)),
            ToteSource::Pick,
            scheduler.barrier().clock(),
            model.navigation.clone(),
            model.relay_service.clone(),
            model.bot_service.clone(),
            model.inventory.clone(),
            model.available_htm.clone(),
            None,
            model.events.clone(),
        )),
        model: model.clone(),
    });
    let release_at = SimTime::seconds(30);
    scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: blocking.cell_reservation,
            action_ready_at: release_at,
            orientation: Orientation::East,
        },
        CellId(15),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    ));

    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "storage missed the oracle-only change and reached the test deadline");
    let events: Vec<_> = rx.try_iter().collect();
    let nav = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == storing => Some(nav),
            _ => None,
        })
        .unwrap();
    assert!(nav.time >= release_at, "storage backdated its departure after waiting: {:?}", nav.time);
    assert!(nav.waypoints.iter().all(|waypoint| waypoint.action.start_time() >= nav.time));
    let deposited_at = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteMoved(moved)
                if moved.tote_id == tote && matches!(moved.new_state, ToteVisualState::AtRelay { .. }) =>
            {
                Some(moved.time)
            }
            _ => None,
        })
        .unwrap();
    let stored_tote = model.inventory.tote(tote).unwrap();
    assert_eq!(
        stored_tote.location_at(deposited_at - SimDuration::ms(1)),
        ToteLocation::WithRobot { robot_id: storing }
    );
    assert_eq!(stored_tote.location_at(deposited_at), ToteLocation::AtRelay { relay_id: RelayId(1) });
    assert!(matches!(stored_tote.location(), ToteLocation::InRack { .. }));
    assert_eq!(model.relay_service.unfinalized_count(), 0);
    assert!(oracle.find_overlaps().is_empty());
}

fn seeded_return(
    model: &TestModel,
    oracle: &TableOracle,
    scheduler: &Scheduler,
    ready: SimTime,
) -> (HtmStore, CellReservation) {
    let robot = RobotId(60);
    let tote = ToteId(1);
    model
        .inventory
        .register_tote(
            tote,
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
        )
        .unwrap();
    model.inventory.tote(tote).unwrap().move_to(ToteLocation::WithRobot { robot_id: robot }, SimTime::ZERO).unwrap();
    register_loaded_htm(model, oracle, robot, CellId(5), tote);
    let source = model.available_htm.take(robot).unwrap();
    let deposit =
        block(model.relay_service.reserve_relay_for_deposit(CellId(5), Default::default(), ready, robot, None))
            .unwrap();
    let store = HtmStore::new(
        StationId(0),
        robot,
        tote,
        source.cell_reservation.clone(),
        ready,
        deposit,
        Some(PresentationId(1)),
        ToteSource::Pick,
        scheduler.barrier().clock(),
        model.navigation.clone(),
        model.relay_service.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        None,
        model.events.clone(),
    );
    (store, source.cell_reservation)
}

#[test]
fn storage_commit_observer_reports_the_exact_departure_before_deposit() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let mut scheduler = bounded_scheduler();
    let (store, source) = seeded_return(&model, &oracle, &scheduler, SimTime::seconds(10));
    let observed = Rc::new(RefCell::new(Vec::new()));
    let store = store.with_commit_observer(Box::new({
        let observed = observed.clone();
        let oracle = oracle.clone();
        let inventory = model.inventory.clone();
        move |path, _| {
            let holds = oracle.reservations_for_cell(source.cell());
            assert!(!holds.iter().find(|hold| hold.id() == source.id()).unwrap().is_open_ended());
            assert_eq!(inventory.tote(ToteId(1))?.location(), ToteLocation::WithRobot { robot_id: RobotId(60) });
            observed.borrow_mut().push(path.clone());
            Ok(())
        }
    }));
    scheduler.add_task(StoreAndReturn { store: Some(store), model });
    scheduler.run().unwrap();
    assert!(!scheduler.stopped());
    let events: Vec<_> = rx.try_iter().collect();
    let routes: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == RobotId(60) => Some(nav),
            _ => None,
        })
        .collect();
    let observed = observed.borrow();
    assert_eq!(observed.len(), 1);
    assert_eq!(routes.len(), 1);
    assert_eq!(observed[0].waypoints, routes[0].waypoints);
    assert!(oracle.find_overlaps().is_empty());
}

fn incoming_to_source(model: &TestModel, oracle: &TableOracle, scheduler: &Scheduler) -> Navigate {
    register_idle_htm(model, oracle, RobotId(61), CellId(0), SimTime::ZERO);
    let incoming = model.available_htm.take(RobotId(61)).unwrap();
    Navigate::new(
        HtmPosition {
            cell_reservation: incoming.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
        },
        CellId(5),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    )
}

#[test]
fn an_incoming_journeys_need_by_lets_it_approach_before_the_return_departs() {
    for ready_seconds in [100, 899, 900, 1000] {
        let (mut model, oracle) = build_model(TestModelConfig::default());
        let (tx, rx) = mpsc::channel();
        Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
        let mut scheduler = bounded_scheduler();
        let ready = if ready_seconds == 899 {
            // The predecessor ends on completion of the first physical move.
            let first_step = model
                .navigation
                .cache_for_model(RobotModelId(0))
                .unwrap()
                .travel_time(PathCacheKey {
                    from: CellId(5),
                    from_orientation: Orientation::East,
                    to: CellId(6),
                    carrying_cargo: true,
                })
                .unwrap();
            SimTime::seconds(900) - first_step
        } else {
            SimTime::seconds(ready_seconds)
        };
        let (store, source) = seeded_return(&model, &oracle, &scheduler, ready);
        scheduler.add_task(StoreAndReturn { store: Some(store), model: model.clone() });
        let arrived = scheduler.add_task(incoming_to_source(&model, &oracle, &scheduler));
        scheduler.run().unwrap();
        assert!(!scheduler.stopped());
        let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
        let events: Vec<_> = rx.try_iter().collect();
        let journey = |robot| {
            events
                .iter()
                .find_map(|event| match event {
                    SimEvent::Nav(nav) if nav.robot_id == robot => Some(nav),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(journey(RobotId(60)).time, ready, "requesting a decision cannot move physical departure earlier");
        let incoming = journey(RobotId(61));
        assert!(
            arrived.action_ready_at - incoming.time <= SimDuration::seconds(900),
            "known distant clearance must not inflate the pathfinding horizon"
        );
        let first_move = incoming
            .waypoints
            .iter()
            .find_map(|waypoint| match waypoint.action {
                WaypointAction::Move { start, .. } => Some(start),
                _ => None,
            })
            .unwrap();
        let source_end = oracle
            .reservations_for_cell(source.cell())
            .iter()
            .find(|hold| hold.id() == source.id())
            .unwrap()
            .window()
            .end()
            .unwrap();
        assert!(arrived.action_ready_at >= source_end);
        if ready_seconds == 899 {
            assert_eq!(source_end, SimTime::seconds(900), "exercise clearance exactly at the planning horizon");
        }
        assert!(source_end > ready, "the old hold closes on physical move completion, after dwell");
        assert!(first_move < ready, "incoming actual movement still waits until {first_move}");
        let incoming_source_end = oracle
            .reservations_for_cell(CellId(0))
            .iter()
            .find(|hold| hold.robot() == RobotId(61))
            .unwrap()
            .window()
            .end()
            .unwrap();
        assert!(incoming_source_end < ready, "the incoming journey's need-by let it leave before the return departs");
        assert!(oracle.find_overlaps().is_empty());
        assert_eq!(model.navigation.cells.vacate_count(), 0, "a Vacate outlived its proposal");
        assert_eq!(model.relay_service.unfinalized_count(), 0);
    }
}

#[derive(Display)]
#[display("ObserveBlockedReturn")]
struct ObserveBlockedReturn {
    navigation: Arc<Navigation>,
    source: CellReservation,
    observed: Rc<Cell<bool>>,
}

#[async_trait(?Send)]
impl Task for ObserveBlockedReturn {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(SimTime::seconds(50)).await?;
        assert!(
            self.navigation
                .oracle
                .reservations_for_cell(self.source.cell())
                .iter()
                .any(|hold| { hold.id() == self.source.id() && hold.is_open_ended() }),
            "accepting the known return must not pretend the blocked source is free"
        );
        assert!(
            self.navigation.cells.vacating(&self.source).is_none(),
            "accepted return is waiting locally, not holding a due proposal"
        );
        self.observed.set(true);
        Ok(())
    }
}

#[test]
fn an_early_return_with_blocked_relay_retains_its_hold_without_pinning_the_clock() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let mut scheduler = bounded_scheduler();
    let (store, source) = seeded_return(&model, &oracle, &scheduler, SimTime::seconds(100));
    let access = model.relay_service.htm_access_cell(store.deposit_relay_id().unwrap()).unwrap();
    register_idle_htm(&model, &oracle, RobotId(62), access, SimTime::ZERO);
    let blocker = model.available_htm.take(RobotId(62)).unwrap();
    scheduler.add_task(StoreAndReturn { store: Some(store), model: model.clone() });
    scheduler.add_task(incoming_to_source(&model, &oracle, &scheduler));
    let observed = Rc::new(Cell::new(false));
    scheduler.add_task(ObserveBlockedReturn {
        navigation: model.navigation.clone(),
        source: source.clone(),
        observed: observed.clone(),
    });
    scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: blocker.cell_reservation,
            action_ready_at: SimTime::seconds(150),
            orientation: Orientation::East,
        },
        CellId(15),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    ));
    scheduler.run().unwrap();
    assert!(!scheduler.stopped());
    assert!(observed.get(), "the clock must advance while the accepted return waits on its relay");
    let events: Vec<_> = rx.try_iter().collect();
    let return_nav = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == RobotId(60) => Some(nav),
            _ => None,
        })
        .unwrap();
    assert!(return_nav.time >= SimTime::seconds(150));
    assert!(return_nav.waypoints.iter().all(|waypoint| waypoint.action.start_time() >= return_nav.time));
    assert!(oracle.find_overlaps().is_empty());
    assert_eq!(model.relay_service.unfinalized_count(), 0);
}

#[test]
fn storage_relocates_an_idle_relay_resident_and_finishes_without_overlap() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let mut map = model.navigation.map.as_ref().clone();
    map.relays = [CellId(10), CellId(20)]
        .into_iter()
        .map(|cell| RelayPosition {
            id: RelayId(cell.0 / 10),
            position: cell,
            htm_access: cell,
            vtm_access: VtmPosition::Cell(cell),
            block: BlockId(0),
            side: BlockSide::Left,
        })
        .collect();
    let parking = ParkingRegistry::from_map(&map);
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &map,
        1000,
        SimDuration::seconds(1),
        [CellId(10), CellId(20)].into_iter().collect(),
    ));
    let navigation = Arc::new(Navigation::new(Arc::new(map), pathfinder, model.bot_service.clone(), oracle.clone()));
    Arc::get_mut(&mut model).unwrap().navigation = navigation;
    let mut scheduler = bounded_scheduler();
    let (mut store, source) = seeded_return(&model, &oracle, &scheduler, SimTime::ZERO);
    store.parking = Some(parking.clone());
    let blocker = RobotId(61);
    register_idle_htm(&model, &oracle, blocker, CellId(10), SimTime::ZERO);
    parking.reserve(CellId(10), blocker, SimTime::ZERO);
    scheduler.add_task(StoreAndReturn { store: Some(store), model: model.clone() });
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "storage cannot wait forever for an unassigned relay resident");
    assert!(matches!(model.inventory.tote(ToteId(1)).unwrap().location(), ToteLocation::InRack { .. }));
    let source_holds = oracle.table().reservations_for_cell(source.cell());
    let closed_source = source_holds.iter().find(|hold| hold.id() == source.id()).unwrap();
    assert!(!closed_source.is_open_ended(), "the loaded HTM's source hold closes when it actually leaves");
    assert_eq!(model.available_htm.peek_matching(|entry| entry.robot() == blocker)[0].cell(), CellId(20));
    assert!(oracle.find_overlaps().is_empty());
}
