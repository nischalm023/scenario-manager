//! A clearance's return reserves a deposit relay for the tote it carries, for
//! a drive departing at the clearance's departure. When robots hold every
//! candidate relay's access cell, the return parks a holder that is an idle
//! empty HTM, decided before the barrier passes the departure, and otherwise
//! waits for a holder's departure to be scheduled. A holder no bay takes is
//! tried again only once a path commits or the barrier advances. If the
//! barrier passes the departure with no relay reserved, the run stops.

use std::sync::mpsc;

use desgraph::{CellId, RelayId};

use super::*;
use crate::error::ReservationError;
use crate::events::{EventJournal, SimEvent};
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::ParkingRegistry;
use crate::model::ids::RackId;
use crate::model::relay::DEPOSIT_ACCESS_MAX_WAIT;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{
    TestModel, TestModelConfig, bay_held_by_a_busy_htm, bounded_scheduler, build_model, empty_rack_slots, one_bay,
    register_idle_htm, register_loaded_htm, relays_busy_until,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::Orientation;

pub(super) const TOTE: ToteId = ToteId(700);
/// The clearance's accepted departure, for the tests that schedule the return
/// before it.
pub(super) const DEPART: SimTime = SimTime::seconds(10);

/// Robot 60 at `cell` carrying an inventory tote it took out of rack 1, its
/// stay taken by the clearance that sends it home.
pub(super) fn cleared_at(model: &TestModel, oracle: &TableOracle, cell: CellId) -> AvailableHtm {
    cleared_carrying(model, oracle, RobotId(60), TOTE, RackId(1), cell)
}

/// `robot` at `cell` carrying inventory tote `tote` it took out of `rack`, its
/// stay taken by the clearance that sends it home.
fn cleared_carrying(
    model: &TestModel,
    oracle: &TableOracle,
    robot: RobotId,
    tote: ToteId,
    rack: RackId,
    cell: CellId,
) -> AvailableHtm {
    let racked = ToteLocation::InRack { rack_id: rack, z_index: 0 };
    model.inventory.register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, racked).expect("racked tote");
    let carried = ToteLocation::WithRobot { robot_id: robot };
    model.inventory.tote(tote).unwrap().move_to(carried, SimTime::ZERO).expect("carried tote");
    register_loaded_htm(model, oracle, robot, cell, tote);
    model.available_htm.take(robot).expect("the cleared stay")
}

/// Robots 61 to 64 standing open-ended on the four relays' access cells
/// (10, 20, 30, 40) since simtime zero, each idle in the registry.
fn holders_on_every_relay(model: &TestModel, oracle: &TableOracle) -> Vec<AvailableHtm> {
    (1..=4)
        .map(|index| {
            let robot = RobotId(60 + index);
            register_idle_htm(model, oracle, robot, CellId(index * 10), SimTime::ZERO);
            model.available_htm.get(robot).unwrap()
        })
        .collect()
}

/// Robots 61 to 64 holding every relay's access cell, none of them idle in
/// the registry: there is no one for a return to park.
fn busy_holders_on_every_relay(model: &TestModel, oracle: &TableOracle) -> Vec<AvailableHtm> {
    holders_on_every_relay(model, oracle).iter().map(|held| model.available_htm.take(held.robot()).unwrap()).collect()
}

/// A holder's next task: a journey off the cell it holds to `to`, departing
/// at `at`, which the scheduler accepts once the barrier reaches it.
pub(super) fn leaving(
    model: &TestModel,
    holder: &AvailableHtm,
    to: CellId,
    at: SimTime,
    clock: Mut<SimTime>,
) -> Navigate {
    Navigate::new(
        HtmPosition {
            cell_reservation: holder.cell_reservation.clone(),
            action_ready_at: at,
            orientation: Orientation::East,
        },
        to,
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        clock,
    )
}

pub(super) fn ret(
    model: &TestModel,
    entry: AvailableHtm,
    parking: Option<ParkingRegistry>,
    depart: SimTime,
    clock: Mut<SimTime>,
) -> ReturnClearedTote {
    ReturnClearedTote {
        entry,
        station: StationId(1),
        source: ToteSource::Pick,
        depart,
        return_relay: None,
        relays: ReturnRelays {
            navigation: model.navigation.clone(),
            relay: model.relay_service.clone(),
            bots: model.bot_service.clone(),
            inventory: model.inventory.clone(),
            available: model.available_htm.clone(),
            parking,
            events: model.events.clone(),
        },
        clock,
    }
}

/// A test model whose facts stream to the returned receiver.
pub(super) fn watched_model() -> (Arc<TestModel>, Arc<TableOracle>, mpsc::Receiver<SimEvent>) {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    (model, oracle, rx)
}

/// A scheduler wired to the model as a run wires it: the journal hears each
/// advance of the barrier, so a fact stamped behind it fails the test, and
/// the relay and VTM selections floor at the barrier.
pub(super) fn watched_scheduler(model: &TestModel) -> Scheduler {
    let journal = model.events.clone();
    let scheduler = bounded_scheduler().with_advance_listener(Box::new(move |at| journal.barrier_advanced(at)));
    model.relay_service.set_admission_floor(scheduler.barrier().clock());
    model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
    scheduler
}

/// The deposit the return reserved: its relay and its start.
pub(super) fn deposit(facts: &[SimEvent]) -> (RelayId, SimTime) {
    facts
        .iter()
        .find_map(|fact| match fact {
            SimEvent::DivingBoardSlotWait(wait) => Some((wait.relay_id, SimTime::ms(wait.sim_time_ms))),
            _ => None,
        })
        .expect("the return reserved a deposit")
}

/// When `robot`'s first drive departs: the simtime its path was planned from.
pub(super) fn departure(facts: &[SimEvent], robot: RobotId) -> SimTime {
    facts
        .iter()
        .find_map(|fact| match fact {
            SimEvent::Nav(nav) if nav.robot_id == robot => Some(nav.time),
            _ => None,
        })
        .expect("the robot drove")
}

/// Robots hold every relay's access cell, and none of them is idle in the
/// registry, so the return has no one to park and its deposit waits. Relay
/// 2's holder's departure is scheduled once the barrier reaches 5 s, before
/// it passes the return's departure: the oracle's change wakes the return,
/// which reserves relay 2 and drives home from its departure.
#[test]
fn a_return_whose_relay_frees_before_the_barrier_passes_its_departure_departs_then() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    let holders = busy_holders_on_every_relay(&model, &oracle);
    let mut scheduler = watched_scheduler(&model);
    scheduler.add_task(ret(&model, entry, None, DEPART, scheduler.barrier().clock()));
    scheduler.add_task(leaving(&model, &holders[1], CellId(25), SimTime::seconds(5), scheduler.barrier().clock()));
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "the return missed the holder's departure and reached the test deadline");
    let facts: Vec<_> = events.try_iter().collect();
    assert_eq!(deposit(&facts).0, RelayId(2), "the relay whose holder left");
    assert_eq!(departure(&facts, RobotId(60)), DEPART);
    assert!(matches!(model.inventory.tote(TOTE).unwrap().location(), ToteLocation::InRack { .. }));
}

/// As above, but relay 2's holder's departure is scheduled only once the
/// barrier reaches 50 s. Once the barrier passes the return's departure with
/// no relay reserved, any relay reserved would have the HTM leave the
/// station's cell later, so the run stops, naming the station, the HTM and
/// the departure.
#[test]
fn a_return_still_blocked_when_the_barrier_passes_its_departure_stops_the_run() {
    let (model, oracle, _events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    let holders = busy_holders_on_every_relay(&model, &oracle);
    let mut scheduler = watched_scheduler(&model);
    scheduler.add_task(ret(&model, entry, None, DEPART, scheduler.barrier().clock()));
    scheduler.add_task(leaving(&model, &holders[1], CellId(25), SimTime::seconds(50), scheduler.barrier().clock()));
    let error = scheduler.run().unwrap_err();
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                station: StationId(1),
                robot: RobotId(60),
                departure: DEPART,
                cause: ClearanceBlock::NoDepositRelay(ReservationError::AccessHeld { .. }),
            }
        ),
        "{error}"
    );
    assert_eq!(scheduler.barrier().floor(), DEPART + SimDuration::ms(1), "the run stops as the barrier passes it");
    assert!(matches!(model.inventory.tote(TOTE).unwrap().location(), ToteLocation::WithRobot { .. }));
}

/// Every relay's slot is busy until 400 s, beyond the access horizon of the
/// return's drive departing at 10 s, and no robot holds an access cell. Once
/// the barrier passes the departure, the run stops, and its cause names the
/// horizon and the relays' first opening.
#[test]
fn a_return_whose_relays_open_beyond_its_access_horizon_stops_naming_the_horizon() {
    let (model, oracle, _events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    empty_rack_slots(&model, 2..=4);
    relays_busy_until(&model, CellId(15), SimTime::seconds(400), None);
    let mut scheduler = watched_scheduler(&model);
    scheduler.add_task(ret(&model, entry, None, DEPART, scheduler.barrier().clock()));
    let error = scheduler.run().unwrap_err();
    let PlanError::ClearanceCannotDepart {
        cause: ClearanceBlock::NoDepositRelay(ReservationError::BeyondAccessHorizon { opens_at, .. }),
        ..
    } = &error
    else {
        panic!("{error}");
    };
    assert_eq!(*opens_at, SimTime::seconds(400), "the relays' first opening");
    assert!(error.to_string().contains(&format!("{DEPOSIT_ACCESS_MAX_WAIT} access horizon")), "{error}");
    assert_eq!(scheduler.barrier().floor(), DEPART + SimDuration::ms(1), "the run stops as the barrier passes it");
}

/// Idle empty HTMs squat on every relay's access cell. The declined deposit
/// names them, earliest relay first; the return parks relay 1's squatter in
/// the one free bay, reserves relay 1, and takes the tote home.
#[test]
fn a_return_parks_a_squatter_off_a_relay_and_deposits_there() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    let squatters = holders_on_every_relay(&model, &oracle);
    let (navigation, bay) = one_bay(&model);
    let mut scheduler = watched_scheduler(&model);
    let mut ret = ret(&model, entry, Some(bay), SimTime::ZERO, scheduler.barrier().clock());
    ret.relays.navigation = navigation;
    scheduler.add_task(ret);
    scheduler.run().unwrap();
    assert!(!scheduler.stopped());
    let parked = model.available_htm.get(squatters[0].robot()).expect("the squatter is idle again");
    assert_eq!(parked.cell(), CellId(3), "relay 1's squatter parked in the bay");
    assert!(squatters[1..].iter().all(|squatter| model.available_htm.is_available(squatter.availability_id())));
    let facts: Vec<_> = events.try_iter().collect();
    assert_eq!(deposit(&facts).0, RelayId(1), "the relay the squatter left");
    assert!(matches!(model.inventory.tote(TOTE).unwrap().location(), ToteLocation::InRack { .. }));
}

/// Idle empty HTMs squat on every relay's access cell, each available only
/// from 40 s, after the return's departure. The return needs relay 1's
/// squatter's parking decided before the barrier passes its departure, not
/// at 40 s: the squatter parks in the one free bay from 40 s, and the return
/// reserves relay 1 and drives home from its departure.
#[test]
fn a_squatter_available_after_the_departure_is_parked_before_the_barrier_passes_it() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    for squatter in holders_on_every_relay(&model, &oracle) {
        let stay = model.available_htm.take(squatter.robot()).unwrap();
        model.available_htm.register(stay.cell_reservation, SimTime::seconds(40), stay.orientation, None).unwrap();
    }
    let (navigation, bay) = one_bay(&model);
    let mut scheduler = watched_scheduler(&model);
    let mut ret = ret(&model, entry, Some(bay), DEPART, scheduler.barrier().clock());
    ret.relays.navigation = navigation;
    scheduler.add_task(ret);
    scheduler.run().unwrap();
    assert!(!scheduler.stopped());
    assert_eq!(model.available_htm.get(RobotId(61)).expect("the squatter is idle again").cell(), CellId(3));
    let facts: Vec<_> = events.try_iter().collect();
    assert_eq!(deposit(&facts).0, RelayId(1), "the relay the squatter left");
    assert_eq!(departure(&facts, RobotId(60)), DEPART);
}

/// As above, but a busy HTM holds the one parking bay. Each squatter's
/// parking is decided before the barrier passes the return's departure and
/// finds no bay; it comes back at that decision rather than once the barrier
/// reaches 40 s, so the return tries every squatter, and the run stops as
/// the barrier passes the departure.
#[test]
fn squatters_available_after_the_departure_that_no_bay_takes_stop_the_run_as_the_barrier_passes_it() {
    let (model, oracle, _events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    for squatter in holders_on_every_relay(&model, &oracle) {
        let stay = model.available_htm.take(squatter.robot()).unwrap();
        model.available_htm.register(stay.cell_reservation, SimTime::seconds(40), stay.orientation, None).unwrap();
    }
    let (navigation, bay) = one_bay(&model);
    bay_held_by_a_busy_htm(&model, &oracle);
    let mut scheduler = watched_scheduler(&model);
    let mut ret = ret(&model, entry, Some(bay), DEPART, scheduler.barrier().clock());
    ret.relays.navigation = navigation;
    scheduler.add_task(ret);
    let error = scheduler.run().unwrap_err();
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                departure: DEPART,
                cause: ClearanceBlock::NoDepositRelay(ReservationError::AccessHeld { .. }),
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(scheduler.barrier().floor(), DEPART + SimDuration::ms(1), "the run stops as the barrier passes it");
    assert!((61..=64).all(|robot| model.available_htm.contains(RobotId(robot))), "every squatter stays idle");
}

/// Idle empty HTMs squat on every relay's access cell, and a busy HTM holds
/// the one parking bay, so no squatter can be parked. Two stations'
/// clearances are blocked at once, and each fails to park every squatter. A
/// failed parking takes and restores the squatter's stay, which changes the
/// fleet: if that change woke the other clearance, each would park again and
/// wake the other in turn, and the barrier would never pass the departure.
/// No relay frees, so the run stops as the barrier passes the departure.
#[test]
fn two_clearances_whose_squatters_no_bay_takes_stop_the_run_as_the_barrier_passes_their_departure() {
    let (model, oracle, _events) = watched_model();
    let first = cleared_at(&model, &oracle, CellId(15));
    let second = cleared_carrying(&model, &oracle, RobotId(70), ToteId(701), RackId(2), CellId(25));
    holders_on_every_relay(&model, &oracle);
    let (navigation, bay) = one_bay(&model);
    bay_held_by_a_busy_htm(&model, &oracle);
    let mut scheduler = watched_scheduler(&model);
    for (entry, station) in [(first, StationId(1)), (second, StationId(2))] {
        let mut task = ret(&model, entry, Some(bay.clone()), DEPART, scheduler.barrier().clock());
        task.relays.navigation = navigation.clone();
        task.station = station;
        scheduler.add_task(task);
    }
    let outcome = scheduler.run();
    assert!(!scheduler.stopped(), "the two clearances woke each other until the test's deadline");
    let error = outcome.unwrap_err();
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                departure: DEPART,
                cause: ClearanceBlock::NoDepositRelay(ReservationError::AccessHeld { .. }),
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(scheduler.barrier().floor(), DEPART + SimDuration::ms(1), "the run stops as the barrier passes it");
}

/// Idle empty HTMs squat on every relay's access cell, and a busy HTM holds
/// the one parking bay until its departure to cell 2, which the scheduler
/// accepts once the barrier reaches 5 s. The return's first parkings find no
/// bay. Once that departure's path commits, the return parks relay 1's
/// squatter in the bay, reserves relay 1, and drives home from its departure.
#[test]
fn a_squatter_no_bay_took_is_parked_once_a_path_commit_frees_the_bay() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    holders_on_every_relay(&model, &oracle);
    let (navigation, bay) = one_bay(&model);
    let bay_holder = bay_held_by_a_busy_htm(&model, &oracle);
    let mut scheduler = watched_scheduler(&model);
    let mut task = ret(&model, entry, Some(bay), DEPART, scheduler.barrier().clock());
    task.relays.navigation = navigation;
    scheduler.add_task(task);
    scheduler.add_task(leaving(&model, &bay_holder, CellId(2), SimTime::seconds(5), scheduler.barrier().clock()));
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "the return missed the bay's freeing and reached the test deadline");
    assert_eq!(model.available_htm.get(RobotId(61)).expect("the squatter is idle again").cell(), CellId(3));
    let facts: Vec<_> = events.try_iter().collect();
    assert_eq!(deposit(&facts).0, RelayId(1), "the relay the squatter left");
    assert_eq!(departure(&facts, RobotId(60)), DEPART);
}
