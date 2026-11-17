//! A clearance's return that carries the deposit relay its putaway fill
//! reserved before it was committed deposits there without searching, gives
//! the relay back when it is dropped unused, takes another relay when, at
//! its decision, another robot holds that relay's access cell with no
//! departure scheduled and one can be reserved, and otherwise stops the run
//! rather than leave the station's cell late when the barrier passes the
//! departure with the access cell held.

use desgraph::{CellId, RelayId};

use super::tests::{DEPART, TOTE, cleared_at, departure, deposit, leaving, ret, watched_model, watched_scheduler};
use super::*;
use crate::events::SimEvent;
use crate::model::ToteLocation;
use crate::model::bot::BotKind;
use crate::model::test_model::{TestModel, block, claim_the_zones_deposits, empty_rack_slots, register_idle_htm};
use crate::pathfinding::Orientation;

/// The relay a deposit off cell 15 at the departure reserves for `robot`.
fn reserve_off_cell_15(model: &TestModel, robot: RobotId) -> Option<RelayReservation<RackSlot>> {
    block(model.relay_service.reserve_relay_for_deposit(CellId(15), Orientation::East, DEPART, robot, None)).ok()
}

/// The fill reserved relay 4, at cell 40, for robot 60's return; from the
/// station's cell 15 a search at the departure would choose a nearer relay.
/// The return deposits at relay 4 without searching and drives home from its
/// departure.
#[test]
fn a_return_holding_its_relay_deposits_there_without_searching() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    empty_rack_slots(&model, 2..=4);
    let mut scheduler = watched_scheduler(&model);
    let held =
        block(model.relay_service.reserve_relay_for_deposit(CellId(40), Orientation::East, DEPART, RobotId(60), None))
            .expect("a free relay for the fill's return");
    assert_eq!(held.relay_id, RelayId(4), "the relay at cell 40");
    scheduler.add_task(ret(&model, entry, None, DEPART, scheduler.barrier().clock()).with_return_relay(Some(held)));
    scheduler.run().unwrap();
    let facts: Vec<_> = events.try_iter().collect();
    assert_eq!(deposit(&facts).0, RelayId(4), "the relay the fill reserved");
    assert_eq!(departure(&facts, RobotId(60)), DEPART);
    assert!(matches!(model.inventory.tote(TOTE).unwrap().location(), ToteLocation::InRack { .. }));
}

/// The return holds a relay and other deposits claimed whatever was left,
/// but its HTM was handed over without a tote, so the return fails before it
/// drives. Dropped unused, it gives the relay back: a deposit for another
/// robot reserves it.
#[test]
fn a_return_dropped_unused_gives_its_relay_back() {
    let (model, oracle, _events) = watched_model();
    register_idle_htm(&model, &oracle, RobotId(60), CellId(15), SimTime::ZERO);
    let without_tote = model.available_htm.take(RobotId(60)).unwrap();
    empty_rack_slots(&model, 1..=4);
    let mut scheduler = watched_scheduler(&model);
    let held = reserve_off_cell_15(&model, RobotId(60)).expect("a free relay for the fill's return");
    let held_relay = held.relay_id;
    let others = claim_the_zones_deposits(&model, CellId(15), DEPART);
    scheduler
        .add_task(ret(&model, without_tote, None, DEPART, scheduler.barrier().clock()).with_return_relay(Some(held)));
    let error = scheduler.run().unwrap_err();
    assert!(matches!(error, PlanError::TaskGraphInvariant { context: "ReturnClearedTote", .. }), "{error}");
    model.bot_service.register(RobotId(91), BotKind::Htm, CellId(1)).unwrap();
    let after = reserve_off_cell_15(&model, RobotId(91)).expect("the dropped return gave its relay back");
    assert_eq!(after.relay_id, held_relay);
    after.cancel();
    others.into_iter().for_each(RelayReservation::cancel);
}

/// The fill reserved a relay for robot 60's return, the only rack slot free;
/// then robot 61 came to stand on that relay's access cell, busy, its
/// departure scheduled only once the barrier reaches 50 s. No other relay
/// can be reserved, and the return's drive cannot leave the station's cell
/// at its departure, so as the barrier passes the departure the run stops,
/// naming the access cell's hold, rather than leave at 50 s.
#[test]
fn a_return_whose_relay_access_cell_is_held_past_its_departure_stops_the_run() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    let mut scheduler = watched_scheduler(&model);
    let held = reserve_off_cell_15(&model, RobotId(60)).expect("a free relay for the fill's return");
    let access = model.relay_service.htm_access_cell(held.relay_id).unwrap();
    register_idle_htm(&model, &oracle, RobotId(61), access, SimTime::ZERO);
    let holder = model.available_htm.take(RobotId(61)).unwrap();
    scheduler.add_task(ret(&model, entry, None, DEPART, scheduler.barrier().clock()).with_return_relay(Some(held)));
    scheduler.add_task(leaving(&model, &holder, CellId(25), SimTime::seconds(50), scheduler.barrier().clock()));
    let error = scheduler.run().unwrap_err();
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                station: StationId(1),
                robot: RobotId(60),
                departure: DEPART,
                cause: ClearanceBlock::DepositAccessHeld,
            }
        ),
        "{error}"
    );
    assert_eq!(scheduler.barrier().floor(), DEPART + SimDuration::ms(1), "the run stops as the barrier passes it");
    let facts: Vec<_> = events.try_iter().collect();
    assert!(!facts.iter().any(|fact| matches!(fact, SimEvent::Nav(nav) if nav.robot_id == RobotId(60))), "no drive");
}

/// As above, but the fill's reservation took relay 1, at cell 10, and the
/// racks have room for another deposit: when the return is decided, robot 61
/// holds relay 1's access cell with no departure scheduled, so the return
/// reserves another relay, gives relay 1 back (no reservation of it is
/// recorded as leaked), and drives home from its departure.
#[test]
fn a_return_whose_relay_access_cell_is_held_at_its_decision_deposits_at_another_relay() {
    let (model, oracle, events) = watched_model();
    let entry = cleared_at(&model, &oracle, CellId(15));
    empty_rack_slots(&model, 2..=4);
    let mut scheduler = watched_scheduler(&model);
    let held =
        block(model.relay_service.reserve_relay_for_deposit(CellId(5), Orientation::East, DEPART, RobotId(60), None))
            .expect("a free relay for the fill's return");
    let held_relay = held.relay_id;
    assert_eq!(held_relay, RelayId(1), "the relay at cell 10, which the drive to the others does not cross");
    let access = model.relay_service.htm_access_cell(held_relay).unwrap();
    register_idle_htm(&model, &oracle, RobotId(61), access, SimTime::ZERO);
    let holder = model.available_htm.take(RobotId(61)).unwrap();
    scheduler.add_task(ret(&model, entry, None, DEPART, scheduler.barrier().clock()).with_return_relay(Some(held)));
    scheduler.add_task(leaving(&model, &holder, CellId(5), SimTime::seconds(50), scheduler.barrier().clock()));
    scheduler.run().unwrap();
    let facts: Vec<_> = events.try_iter().collect();
    assert_ne!(deposit(&facts).0, held_relay, "a relay whose access cell is free");
    assert_eq!(departure(&facts, RobotId(60)), DEPART);
    assert_eq!(model.relay_service.drain_leaked(), [], "relay 1 was given back, not dropped");
}
