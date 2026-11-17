use super::*;
use crate::events::SimEvent;
use crate::model::htm::registry::ProposedLastResort;
use crate::model::ids::TaskId;
use crate::model::reservation::ReservationTable;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{TestModel, TestModelConfig, build_model, register_idle_htm};
use crate::pathfinding::ReservationOracle;
use crate::simtime::TimeRange;
use desgraph::StationId;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::mpsc;
use std::task::{Context, Waker};
use tokio::runtime::Builder;

fn observer_navigation(model: &TestModel, scheduler: &Scheduler, target: CellId) -> Navigate {
    let source = model.available_htm.take(RobotId(1)).unwrap();
    Navigate::new(
        HtmPosition {
            cell_reservation: source.cell_reservation,
            action_ready_at: source.action_ready_at,
            orientation: source.orientation,
        },
        target,
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
fn commit_observer_sees_the_exact_reserved_route_once_without_changing_it() {
    let mut control = None;
    for observe in [false, true] {
        let (mut model, oracle) = build_model(TestModelConfig::default());
        let (tx, rx) = mpsc::channel();
        Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
        register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
        let mut scheduler = Scheduler::new();
        let mut task = observer_navigation(&model, &scheduler, CellId(9))
            .with_destinations(vec![(CellId(5), TripEndpoint::Station(StationId(1)))]);
        let observed = Rc::new(RefCell::new(Vec::new()));
        if observe {
            let observed = observed.clone();
            let oracle = oracle.clone();
            task = task.with_commit_observer(Box::new(move |path, _| {
                assert!(oracle.reservations_for_cell(CellId(0)).iter().all(|hold| !hold.is_open_ended()));
                assert!(
                    oracle.reservations_for_cell(CellId(5)).iter().any(|hold| hold.is_open_ended()),
                    "the callback must run after the actual route commit"
                );
                observed.borrow_mut().push(path.clone());
                Ok(())
            }));
        }
        let arrived = scheduler.add_task(task);
        scheduler.run().unwrap();
        let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
        let events: Vec<_> = rx.try_iter().collect();
        let routes: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::Nav(nav) => Some(nav),
                _ => None,
            })
            .collect();
        assert_eq!(routes.len(), 1);
        assert_eq!(arrived.cell_reservation.cell(), CellId(5));
        if observe {
            let observed = observed.borrow();
            assert_eq!(observed.len(), 1);
            assert_eq!(observed[0].waypoints, routes[0].waypoints);
            assert_eq!(observed[0].waypoints.last().unwrap().action.start_time(), arrived.action_ready_at);
            assert_eq!(control.as_ref().unwrap(), &(routes[0].waypoints.clone(), arrived.action_ready_at));
        } else {
            control = Some((routes[0].waypoints.clone(), arrived.action_ready_at));
        }
        assert!(oracle.find_overlaps().is_empty());
    }
}

struct ObserverDrop(Rc<Cell<bool>>);

impl Drop for ObserverDrop {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn no_committed_route_drops_the_observer_without_invoking_it() {
    for (target, cancelled) in [(CellId(0), false), (CellId(70), false), (CellId(5), true)] {
        let (model, oracle) = build_model(TestModelConfig { island_cells: vec![CellId(70)], ..Default::default() });
        register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
        let mut scheduler = Scheduler::new();
        let called = Rc::new(Cell::new(false));
        let dropped = Rc::new(Cell::new(false));
        let guard = ObserverDrop(dropped.clone());
        let task = observer_navigation(&model, &scheduler, target).with_commit_observer(Box::new({
            let called = called.clone();
            move |_, _| {
                let _ = &guard;
                called.set(true);
                Ok(())
            }
        }));
        if cancelled {
            drop(task);
        } else {
            scheduler.add_task(task);
            let result = scheduler.run();
            if target == CellId(70) {
                assert!(result.is_err());
            } else {
                result.unwrap();
            }
        }
        assert!(!called.get());
        assert!(dropped.get(), "a provisional claim must not survive a journey with no route commit");
        assert!(oracle.reservations_for_cell(CellId(0)).iter().any(|hold| hold.is_open_ended()));
    }
}

#[test]
fn observer_error_propagates_after_commit_without_claiming_a_rollback() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let mut scheduler = Scheduler::new();
    let called = Rc::new(Cell::new(false));
    let task = observer_navigation(&model, &scheduler, CellId(5)).with_commit_observer(Box::new({
        let called = called.clone();
        move |_, _| {
            called.set(true);
            Err(PlanError::TaskGraphInvariant { context: "route observer test", detail: "invalid boundary".into() })
        }
    }));
    scheduler.add_task(task);
    assert!(matches!(scheduler.run(), Err(PlanError::TaskGraphInvariant { context: "route observer test", .. })));
    assert!(called.get());
    assert!(oracle.reservations_for_cell(CellId(0)).iter().all(|hold| !hold.is_open_ended()));
    assert!(oracle.reservations_for_cell(CellId(5)).iter().any(|hold| hold.is_open_ended()));
}

#[test]
fn an_incoming_journey_can_plan_against_a_future_owned_departure() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let holder_ready = SimTime::seconds(100);
    register_idle_htm(&model, &oracle, RobotId(2), CellId(5), holder_ready);
    let holder = model.available_htm.take(RobotId(2)).unwrap();
    let source = holder.cell_reservation.clone();
    let mut scheduler = Scheduler::new();
    let committed_at = Rc::new(RefCell::new(Vec::new()));
    let departure = Navigate::new(
        HtmPosition {
            cell_reservation: holder.cell_reservation,
            action_ready_at: holder.action_ready_at,
            orientation: holder.orientation,
        },
        CellId(9),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    );
    let incoming = observer_navigation(&model, &scheduler, CellId(5));
    let mut completions = Vec::new();
    for task in [departure, incoming] {
        let clock = scheduler.barrier().clock();
        let committed_at = committed_at.clone();
        completions.push(scheduler.add_task(task.with_commit_observer(Box::new(move |_, _| {
            committed_at.borrow_mut().push(clock.value());
            Ok(())
        }))));
    }
    scheduler.run().unwrap();
    let mut completions = completions.into_iter();
    let runtime = Builder::new_current_thread().build().unwrap();
    let departed = runtime.block_on(completions.next().unwrap());
    let arrived = runtime.block_on(completions.next().unwrap());
    assert_eq!(committed_at.borrow().len(), 2);
    assert!(
        committed_at.borrow().iter().all(|&at| at < holder_ready),
        "both owned journeys must be planned before the holder finishes its dwell"
    );
    let released = oracle
        .reservations_for_cell(CellId(5))
        .into_iter()
        .find(|hold| hold.id() == source.id())
        .unwrap()
        .window()
        .end()
        .unwrap();
    assert!(released >= holder_ready, "earlier planning must not shorten the holder's dwell");
    assert!(arrived.action_ready_at >= released);
    assert!(departed.action_ready_at > holder_ready);
    assert_eq!(arrived.cell_reservation.cell(), CellId(5));
    assert_eq!(model.navigation.cells.vacate_count(), 0, "a Vacate outlived its proposal");
    assert!(oracle.find_overlaps().is_empty());
}

#[test]
fn need_bys_change_only_the_decision_deadline_and_drop_with_the_attempt() {
    for (target, alternatives, leaves_source) in [(5, false, true), (0, false, false), (5, true, false)] {
        let (model, oracle) = build_model(TestModelConfig::default());
        let ready = SimTime::seconds(100);
        register_idle_htm(&model, &oracle, RobotId(1), CellId(0), ready);
        let scheduler = Scheduler::new();
        let mut task = observer_navigation(&model, &scheduler, CellId(target));
        if alternatives {
            task = task.with_destinations(vec![
                (CellId(0), TripEndpoint::Station(StationId(1))),
                (CellId(5), TripEndpoint::Station(StationId(2))),
            ]);
        }
        let runtime = Builder::new_current_thread().build().unwrap();
        let proposal = runtime.block_on(Task::propose(&task, ProposalId { task: TaskId(1), attempt: 0 })).unwrap();
        let cells = &model.navigation.cells;
        let source = &task.position().unwrap().cell_reservation;
        assert_eq!(ProposedReservation::resolve_by(&proposal), Some(ready));
        assert_eq!(cells.vacating(source).is_some(), leaves_source);
        if let Some(vacating) = cells.vacating(source) {
            let need_by = cells.need_by(vacating, SimTime::seconds(5)).unwrap();
            assert_eq!(ProposedReservation::resolve_by(&proposal), Some(SimTime::seconds(5)));
            assert_eq!(ProposedReservation::start_time(&proposal), Some(ready));
            drop(need_by);
            assert_eq!(ProposedReservation::resolve_by(&proposal), Some(ready));
        }
        let seen = cells.changed().seen();
        drop(proposal);
        assert!(cells.vacating(source).is_none());
        assert_eq!(cells.changed().seen() > seen, leaves_source);
        assert!(source.is_open_ended(), "dropping a Vacate must not release the physical cell");
    }
}

#[test]
fn mixed_departures_choose_one_useful_holder_and_leave_free_alternatives_alone() {
    for (parking_ready, return_ready, free_target, chosen) in
        [(50, 100, false, "parking"), (150, 100, false, "return"), (50, 100, true, "none")]
    {
        let (model, oracle) = build_model(TestModelConfig::default());
        register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
        register_idle_htm(&model, &oracle, RobotId(2), CellId(10), SimTime::seconds(parking_ready));
        let incoming = model.available_htm.take(RobotId(1)).unwrap();
        let parked = model.available_htm.peek_matching(|entry| entry.robot() == RobotId(2)).pop().unwrap();
        let return_source =
            oracle.table().reserve_cell(RobotId(3), CellId(30), TimeRange::open_ended(SimTime::ZERO)).unwrap();
        let mut scheduler = Scheduler::new();
        let parking = ProposedLastResort::parking(
            &model.available_htm,
            &model.navigation,
            &parked,
            ProposalId { task: TaskId(10), attempt: 0 },
            scheduler.barrier().clock(),
        );
        let returning = model.navigation.cells.vacate(
            &return_source,
            ProposalId { task: TaskId(11), attempt: 0 },
            SimTime::seconds(return_ready),
        );
        let mut targets =
            vec![(CellId(10), TripEndpoint::Station(StationId(1))), (CellId(30), TripEndpoint::Station(StationId(2)))];
        if free_target {
            targets.push((CellId(5), TripEndpoint::Station(StationId(3))));
        }
        let navigation = Navigate::new(
            HtmPosition {
                cell_reservation: incoming.cell_reservation,
                action_ready_at: SimTime::ZERO,
                orientation: Orientation::East,
            },
            CellId(10),
            None,
            model.navigation.clone(),
            model.bot_service.clone(),
            model.inventory.clone(),
            None,
            model.events.clone(),
            scheduler.barrier().clock(),
        )
        .with_destinations(targets.clone());
        let need_by = if free_target {
            let arrived = scheduler.add_task(navigation);
            scheduler.run().unwrap();
            let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
            assert_eq!(arrived.cell_reservation.cell(), CellId(5));
            None
        } else {
            assert!(
                navigation
                    .available_destination(navigation.position().unwrap(), SimTime::ZERO, &targets)
                    .unwrap()
                    .is_none()
            );
            Some(
                navigation
                    .need_destination_vacated(
                        navigation.position().unwrap(),
                        SimTime::ZERO,
                        targets.iter().map(|(cell, _)| *cell),
                    )
                    .unwrap()
                    .unwrap(),
            )
        };
        assert_eq!(
            ProposedReservation::resolve_by(&parking),
            Some(if chosen == "parking" { SimTime::ZERO } else { SimTime::seconds(parking_ready) }),
        );
        assert_eq!(
            returning.resolve_by(SimTime::seconds(return_ready), SimTime::ZERO),
            if chosen == "return" { SimTime::ZERO } else { SimTime::seconds(return_ready) },
        );
        drop(need_by);
        assert_eq!(ProposedReservation::resolve_by(&parking), Some(SimTime::seconds(parking_ready)));
        assert_eq!(returning.resolve_by(SimTime::seconds(return_ready), SimTime::ZERO), SimTime::seconds(return_ready));
    }
}

#[test]
fn a_finite_destination_gap_beyond_the_horizon_defers_only_the_owned_approach() {
    let (model, _) = build_model(TestModelConfig::default());
    let oracle = Arc::new(TableOracle::new(ReservationTable::new(SimDuration::ms(100))));
    let holder_end = SimTime::ms(899_950);
    oracle.table().reserve_cell(RobotId(2), CellId(5), TimeRange::new(SimTime::ZERO, holder_end)).unwrap();
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let source = model.available_htm.take(RobotId(1)).unwrap();
    let navigation = Arc::new(Navigation::new(
        model.navigation.map.clone(),
        model.navigation.pathfinder.clone(),
        model.bot_service.clone(),
        oracle.clone(),
    ));
    let mut scheduler = Scheduler::new();
    let arrived = scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: source.cell_reservation.clone(),
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
        },
        CellId(5),
        None,
        navigation,
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    ));
    scheduler.run().unwrap();
    let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
    assert!(
        arrived.action_ready_at >= holder_end + SimDuration::ms(100),
        "arrival must respect the real clearance gap"
    );
    let source_end = oracle
        .reservations_for_cell(CellId(0))
        .iter()
        .find(|hold| hold.id() == source.cell_reservation.id())
        .unwrap()
        .window()
        .end()
        .unwrap();
    assert!(source_end < holder_end, "the incoming HTM physically approaches before destination clearance");
    assert!(source_end > SimTime::seconds(890), "the source retains the HTM during the distant known wait");
    assert!(oracle.find_overlaps().is_empty());
    assert_eq!(
        scheduler.barrier().floor(),
        SimTime::ZERO,
        "planning a known future journey does not advance global time"
    );
}

#[test]
fn a_vacate_the_target_s_holder_registers_after_the_journey_began_is_needed_while_the_call_is_pending() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let holder_ready = SimTime::seconds(100);
    register_idle_htm(&model, &oracle, RobotId(2), CellId(5), holder_ready);
    let holder = model.available_htm.take(RobotId(2)).unwrap();
    let scheduler = Scheduler::new();
    let task = observer_navigation(&model, &scheduler, CellId(5));
    let position = task.position().unwrap();
    let cells = &model.navigation.cells;
    let mut cx = Context::from_waker(Waker::noop());
    let vacate;
    {
        let mut keeper = pin!(task.keep_target_need_by(position, CellId(5), SimTime::ZERO));
        assert!(keeper.as_mut().poll(&mut cx).is_pending(), "nothing to need yet: R2 registered no Vacate");

        // R2's next task is proposed while R1's call is pending (scheduler time).
        vacate = cells.vacate(&holder.cell_reservation, ProposalId { task: TaskId(2), attempt: 0 }, holder_ready);
        assert_eq!(vacate.resolve_by(holder_ready, SimTime::ZERO), holder_ready);
        assert!(keeper.as_mut().poll(&mut cx).is_pending(), "the keeper never completes");
        assert_eq!(
            vacate.resolve_by(holder_ready, SimTime::ZERO),
            SimTime::ZERO,
            "R2's next task is needed decided at the barrier"
        );
    }
    assert_eq!(vacate.resolve_by(holder_ready, SimTime::ZERO), holder_ready, "the need-by ends with the call");
}

#[test]
fn the_keeper_needs_only_the_chosen_target_s_holder_so_a_holder_the_target_gains_later_is_needed() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let ready = SimTime::seconds(100);
    register_idle_htm(&model, &oracle, RobotId(2), CellId(10), ready);
    let other = model.available_htm.take(RobotId(2)).unwrap();
    let scheduler = Scheduler::new();
    // Cell 5 is the alternative chosen; cell 10's holder has registered its Vacate.
    let task = observer_navigation(&model, &scheduler, CellId(5)).with_destinations(vec![
        (CellId(5), TripEndpoint::Station(StationId(1))),
        (CellId(10), TripEndpoint::Station(StationId(2))),
    ]);
    let position = task.position().unwrap();
    let cells = &model.navigation.cells;
    let not_chosen = cells.vacate(&other.cell_reservation, ProposalId { task: TaskId(2), attempt: 0 }, ready);
    let mut cx = Context::from_waker(Waker::noop());
    let mut keeper = pin!(task.keep_target_need_by(position, CellId(5), SimTime::ZERO));
    assert!(keeper.as_mut().poll(&mut cx).is_pending());
    assert_eq!(not_chosen.resolve_by(ready, SimTime::ZERO), ready, "the alternative not chosen is not needed");

    // R3 takes cell 5 open-ended and registers its Vacate while the call is pending (scheduler time).
    let taken = oracle.table().reserve_cell(RobotId(3), CellId(5), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let chosen = cells.vacate(&taken, ProposalId { task: TaskId(3), attempt: 0 }, ready);
    assert!(keeper.as_mut().poll(&mut cx).is_pending());
    assert_eq!(chosen.resolve_by(ready, SimTime::ZERO), SimTime::ZERO, "the target's new holder is needed");
}

#[test]
fn a_departure_moved_toward_a_far_clearance_chooses_among_the_alternatives_again() {
    // R2 stands on cell 5 until 899.95 s and R3 on cell 3 until 900 s; the
    // table keeps 100 ms between holds. For a departure at 0 s, cell 5's hold
    // ends first, so it is chosen; its clearance (900.05 s) is beyond the
    // horizon, so the departure moves toward it and the alternatives are
    // ranked again there. There cell 5 ranks at its clearance and cell 3 at
    // its hold's end, 900 s (the rank leaves out the gap), so cell 3 is
    // chosen and entered from 900.1 s; without the second ranking the robot
    // ends on cell 5. On a table with no gap, as production's, the second
    // ranking never changes the choice: this pins only that it runs.
    let (model, _) = build_model(TestModelConfig::default());
    let oracle = Arc::new(TableOracle::new(ReservationTable::new(SimDuration::ms(100))));
    oracle.table().reserve_cell(RobotId(2), CellId(5), TimeRange::new(SimTime::ZERO, SimTime::ms(899_950))).unwrap();
    oracle.table().reserve_cell(RobotId(3), CellId(3), TimeRange::new(SimTime::ZERO, SimTime::ms(900_000))).unwrap();
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let source = model.available_htm.take(RobotId(1)).unwrap();
    let navigation = Arc::new(Navigation::new(
        model.navigation.map.clone(),
        model.navigation.pathfinder.clone(),
        model.bot_service.clone(),
        oracle.clone(),
    ));
    let mut scheduler = Scheduler::new();
    let task = Navigate::new(
        HtmPosition {
            cell_reservation: source.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
        },
        CellId(5),
        None,
        navigation,
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    )
    .with_destinations(vec![
        (CellId(5), TripEndpoint::Station(StationId(1))),
        (CellId(3), TripEndpoint::Station(StationId(2))),
    ]);
    let arrived = scheduler.add_task(task);
    scheduler.run().unwrap();
    let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
    assert_eq!(arrived.cell_reservation.cell(), CellId(3), "cell 3 is chosen at the later departure");
    let entered = arrived.cell_reservation.window().start();
    assert!(entered >= SimTime::ms(900_100), "R1 enters cell 3 at {entered:?}, after cell 5's clearance");
    assert!(oracle.find_overlaps().is_empty());
}
