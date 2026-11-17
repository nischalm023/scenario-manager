//! A clearance's store leaves the station's cell at the release: its path
//! search may not await (scheduler time) commits by robots that hold its way
//! before its first window, so a store whose way is held stops the run with
//! nothing committed; only its departure from the cell is bounded, not when a
//! later window of its journey starts; a squatter on its relay's access cell
//! that no bay takes stops the run as the barrier passes the release, however
//! many such stores await (scheduler time) their squatters' parkings; and one
//! that a bay takes lets the store leave at the release.

use std::sync::mpsc;

use desgraph::{CellId, RelayId};

use super::*;
use crate::events::SimEvent;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::ids::RackId;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{
    Takes, TestModel, TestModelConfig, bay_held_by_a_busy_htm, block, bounded_scheduler, build_model, one_bay,
    register_idle_htm, register_loaded_htm, squatter_ready_at,
};
use crate::model::{ToteDesignation, ToteType};

/// The operator's release of robot 60's tote on the station's cell 15.
const RELEASE: SimTime = SimTime::seconds(10);

/// A model built from `config`, whose facts stream to the returned receiver.
fn watched_model(config: TestModelConfig) -> (Arc<TestModel>, Arc<TableOracle>, mpsc::Receiver<SimEvent>) {
    let (mut model, oracle) = build_model(config);
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    (model, oracle, rx)
}

/// A model whose relay 2, at cell 20, is fed by a station's queue of cells 18
/// and 19, whose facts stream to the returned receiver.
fn queued_relay_model() -> (Arc<TestModel>, Arc<TableOracle>, mpsc::Receiver<SimEvent>) {
    watched_model(TestModelConfig { queue_cells: vec![CellId(18), CellId(19)], ..Default::default() })
}

/// Robot 61 standing on `cell` since simtime zero, busy, with its stay.
fn busy_htm_on(model: &TestModel, oracle: &TableOracle, cell: CellId) -> CellReservation {
    register_idle_htm(model, oracle, RobotId(61), cell, SimTime::ZERO);
    model.available_htm.take(RobotId(61)).unwrap().cell_reservation
}

/// Robot 60's clearance store: off the station's cell 15 at `RELEASE` with
/// the tote it took out of rack 2, to relay 2 at cell 20.
fn clearance_store(model: &TestModel, oracle: &TableOracle, scheduler: &Scheduler) -> HtmStore {
    clearance_store_of(model, oracle, scheduler, RobotId(60), CellId(15), RelayId(2))
}

/// `robot`'s clearance store: off the station's `cell` at `RELEASE` with a
/// tote it took out of the rack beside `relay`, to `relay`, whose access cell
/// is 10 times its number.
fn clearance_store_of(
    model: &TestModel,
    oracle: &TableOracle,
    scheduler: &Scheduler,
    robot: RobotId,
    cell: CellId,
    relay: RelayId,
) -> HtmStore {
    let tote = ToteId(robot.0);
    let racked = ToteLocation::InRack { rack_id: RackId(relay.0), z_index: 0 };
    model.inventory.register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, racked).unwrap();
    model.inventory.tote(tote).unwrap().move_to(ToteLocation::WithRobot { robot_id: robot }, SimTime::ZERO).unwrap();
    register_loaded_htm(model, oracle, robot, cell, tote);
    let stay = model.available_htm.take(robot).unwrap();
    let beside_the_relay = CellId(relay.0 * 10 - 1);
    let deposit =
        block(model.relay_service.reserve_relay_for_deposit(beside_the_relay, Orientation::East, RELEASE, robot, None))
            .unwrap();
    assert_eq!(deposit.relay_id, relay, "the relay beside cell {beside_the_relay:?}");
    HtmStore::new(
        StationId(1),
        robot,
        tote,
        stay.cell_reservation,
        RELEASE,
        deposit,
        None,
        ToteSource::Putaway,
        scheduler.barrier().clock(),
        model.navigation.clone(),
        model.relay_service.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        None,
        model.events.clone(),
    )
    .departing_by(Some(RELEASE))
}

/// Robot 60's windows: when each departs and the cell it ends on.
fn windows_of_robot_60(facts: &[SimEvent]) -> Vec<(SimTime, CellId)> {
    facts
        .iter()
        .filter_map(|fact| match fact {
            SimEvent::Nav(nav) if nav.robot_id == RobotId(60) => Some((nav.time, nav.waypoints.last()?.cell)),
            _ => None,
        })
        .collect()
}

/// Relay 2 is fed by a queue of cells 18 and 19, a fixture no shipped map has
/// (a queue there feeds only a station). Robot 61 stands on queue cell 18 with
/// no departure scheduled, so every way to relay 2 crosses its cell. The
/// store's path search may not await robot 61's next commit (scheduler time):
/// the run stops at the release, naming the held way, and robot 60 has driven
/// nowhere.
#[test]
fn a_clearance_store_whose_way_is_held_stops_the_run_with_nothing_committed() {
    let (model, oracle, events) = queued_relay_model();
    busy_htm_on(&model, &oracle, CellId(18));
    let mut scheduler = bounded_scheduler();
    scheduler.add_task(clearance_store(&model, &oracle, &scheduler));
    let error = scheduler.run().expect_err("the run stops typed at the store's decision");
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                station: StationId(1),
                robot: RobotId(60),
                departure: RELEASE,
                cause: ClearanceBlock::WayHeld,
            }
        ),
        "{error}"
    );
    assert!(windows_of_robot_60(&events.try_iter().collect::<Vec<_>>()).is_empty(), "nothing committed");
}

/// Relay 2 is fed by a queue of cells 18 and 19, a fixture no shipped map has
/// (a queue there feeds only a station). Robot 61 stands on queue cell 19, its
/// departure scheduled once the barrier reaches 50 s. Robot 60's journey
/// departs at the release into queue cell 18 and goes on to the relay in a
/// later window that starts after 50 s: the departure is at the release, so
/// the run is not stopped.
#[test]
fn a_clearance_store_departing_at_the_release_is_not_stopped_by_a_later_window() {
    let (model, oracle, events) = queued_relay_model();
    let held = busy_htm_on(&model, &oracle, CellId(19));
    let mut scheduler = bounded_scheduler();
    let stored = scheduler.add_task(clearance_store(&model, &oracle, &scheduler));
    scheduler.add_task(Navigate::new(
        HtmPosition { cell_reservation: held, action_ready_at: SimTime::seconds(50), orientation: Orientation::East },
        CellId(25),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    ));
    scheduler.run().unwrap();
    block(stored).reservation.cancel();
    let windows = windows_of_robot_60(&events.try_iter().collect::<Vec<_>>());
    assert_eq!(windows.len(), 2, "{windows:?}");
    assert_eq!(windows[0], (RELEASE, CellId(18)), "departs at the release into the queue");
    assert!(windows[1].0 >= SimTime::seconds(50), "goes on once robot 61 departs: {windows:?}");
}

/// Robot 62, empty, stands on relay 2's access cell 20 from simtime zero,
/// idle from 50 s, and a busy HTM holds the one parking bay. The squatter's
/// parking is decided before the barrier passes the release and finds no
/// bay: its stay is taken and given back, and the run stops as the barrier
/// passes the release, naming the held access cell, not once the barrier
/// reaches 50 s.
#[test]
fn a_clearance_store_whose_squatter_no_bay_takes_stops_the_run_as_the_barrier_passes_the_release() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let mut scheduler = bounded_scheduler();
    let mut store = clearance_store(&model, &oracle, &scheduler);
    squatter_ready_at(&model, &oracle, RobotId(62), CellId(20), SimTime::seconds(50));
    let (navigation, bay) = one_bay(&model);
    store.navigation = navigation;
    store.parking = Some(bay);
    bay_held_by_a_busy_htm(&model, &oracle);
    let takes = Arc::new(Takes::default());
    model.available_htm.add_observer(takes.clone());
    scheduler.add_task(store);
    let error = scheduler
        .run()
        .expect_err("the run stops typed as the barrier passes the release, not at the test's wall deadline");
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                station: StationId(1),
                robot: RobotId(60),
                departure: RELEASE,
                cause: ClearanceBlock::DepositAccessHeld,
            }
        ),
        "{error}"
    );
    assert_eq!(scheduler.barrier().floor(), RELEASE + SimDuration::ms(1), "the run stops as the barrier passes it");
    assert_eq!(*takes.0.lock().unwrap(), [RobotId(62)], "the squatter's parking took its stay");
    assert!(model.available_htm.contains(RobotId(62)), "and gave it back");
}

/// Robot 60's store to relay 2 and robot 66's to relay 3 must both leave by
/// the release; robot 62, empty, stands on relay 2's access cell 20 and robot
/// 63 on relay 3's access cell 30, each from simtime zero and idle from 50 s,
/// and a busy HTM holds the one parking bay. Each squatter's parking is tried
/// a single time and finds no bay, and the other store's take and give-back
/// of its own squatter does not try it again: the run stops as the barrier
/// passes the release, naming a held access cell.
#[test]
fn two_clearance_stores_whose_squatters_no_bay_takes_stop_the_run_as_the_barrier_passes_the_release() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let mut scheduler = bounded_scheduler();
    let mut stores = [
        clearance_store(&model, &oracle, &scheduler),
        clearance_store_of(&model, &oracle, &scheduler, RobotId(66), CellId(35), RelayId(3)),
    ];
    squatter_ready_at(&model, &oracle, RobotId(62), CellId(20), SimTime::seconds(50));
    squatter_ready_at(&model, &oracle, RobotId(63), CellId(30), SimTime::seconds(50));
    let (navigation, bay) = one_bay(&model);
    for store in &mut stores {
        store.navigation = navigation.clone();
        store.parking = Some(bay.clone());
    }
    bay_held_by_a_busy_htm(&model, &oracle);
    let takes = Arc::new(Takes::default());
    model.available_htm.add_observer(takes.clone());
    for store in stores {
        scheduler.add_task(store);
    }
    let error = scheduler
        .run()
        .expect_err("the run stops typed as the barrier passes the release, not at the test's wall deadline");
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                robot: RobotId(60) | RobotId(66),
                departure: RELEASE,
                cause: ClearanceBlock::DepositAccessHeld,
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(scheduler.barrier().floor(), RELEASE + SimDuration::ms(1), "the run stops as the barrier passes it");
    let mut taken = takes.0.lock().unwrap().clone();
    taken.sort();
    assert_eq!(taken, [RobotId(62), RobotId(63)], "each squatter's parking tried once");
}

/// Robot 62, empty, stands on relay 1's access cell 10 from simtime zero,
/// idle from 50 s, and the one parking bay, at cell 3, is free. The
/// squatter's parking is committed before the barrier passes the release,
/// departing at 50 s: robot 60's path off the station's cell 15 starts at the
/// release, with its wait (simtime) for robot 62 to leave cell 10 planned,
/// and deposits at relay 1, and robot 62 stands in the bay.
#[test]
fn a_clearance_store_whose_squatter_parks_starts_its_path_at_the_release() {
    let (model, oracle, events) = watched_model(TestModelConfig::default());
    let mut scheduler = bounded_scheduler();
    let mut store = clearance_store_of(&model, &oracle, &scheduler, RobotId(60), CellId(15), RelayId(1));
    squatter_ready_at(&model, &oracle, RobotId(62), CellId(10), SimTime::seconds(50));
    let (navigation, bay) = one_bay(&model);
    store.navigation = navigation;
    store.parking = Some(bay);
    let stored = scheduler.add_task(store);
    scheduler.run().unwrap();
    let at_relay = block(stored);
    assert_eq!(at_relay.relay_id, RelayId(1));
    at_relay.reservation.cancel();
    assert_eq!(model.available_htm.get(RobotId(62)).expect("the squatter is idle again").cell(), CellId(3));
    let windows = windows_of_robot_60(&events.try_iter().collect::<Vec<_>>());
    assert_eq!(windows.first(), Some(&(RELEASE, CellId(10))), "its path starts at the release: {windows:?}");
}
