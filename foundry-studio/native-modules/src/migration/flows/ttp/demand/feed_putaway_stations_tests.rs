//! Direct putaway supply's search: the live empty that would reach a putaway
//! station soonest, departing no earlier than the station's opening
//! (simtime), never an empty released on a station's cell before the
//! opening, and never an order tote.

use super::*;
use crate::model::reservation::oracle::TableOracle;
use crate::model::station::{Station, StationSet};
use crate::model::test_model::{TestModel, TestModelConfig, build_model, register_loaded_htm};
use crate::model::{ToteLocation, ToteType};
use desgraph::{CardinalDirection, StationType};

pub(super) const STATION: StationId = StationId(1);
pub(super) const STATION_CELL: CellId = CellId(30);
pub(super) const OPENING: SimTime = SimTime::seconds(100);
/// A pick station's presentation cell.
pub(super) const PICK_CELL: CellId = CellId(8);

/// Robot `robot` idle at `cell` since simtime zero, carrying an empty tote.
pub(super) fn idle_empty(model: &TestModel, oracle: &TableOracle, robot: RobotId, tote: ToteId, cell: CellId) {
    model
        .inventory
        .register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, ToteLocation::WithRobot { robot_id: robot })
        .expect("carried tote");
    register_loaded_htm(model, oracle, robot, cell, tote);
}

/// Putaway supply for one station, `STATION` at `STATION_CELL`, beside a
/// pick station at `PICK_CELL`.
pub(super) fn supply(model: &TestModel) -> PutawaySupply {
    let station = |id, station_type, cell| Station::new(id, station_type, cell, vec![], CardinalDirection::East, 1);
    let stations = StationSet::new(vec![
        station(STATION, StationType::Putaway, STATION_CELL),
        station(StationId(2), StationType::Pick, PICK_CELL),
    ]);
    PutawaySupply::new(
        &stations,
        &model.station_fetch_bounds,
        model.navigation.clone(),
        model.relay_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        None,
    )
}

#[test]
fn idle_empty_departs_at_the_opening() {
    let (model, oracle) = build_model(TestModelConfig::default());
    idle_empty(&model, &oracle, RobotId(60), ToteId(700), CellId(5));
    let candidate = supply(&model).best_direct(STATION_CELL, OPENING).expect("reachable").expect("the standing empty");
    assert_eq!((candidate.entry.robot(), candidate.depart), (RobotId(60), OPENING));
    assert!(candidate.arrival() > OPENING);
}

/// The pick station's operator releases its emptied tote's HTM on the
/// station's cell, which the HTM may leave only at that release. Released
/// before the opening, it is passed over; released after it, it departs at
/// its release.
#[test]
fn an_empty_released_on_a_station_cell_departs_at_its_release_or_is_passed_over() {
    let (model, oracle) = build_model(TestModelConfig::default());
    idle_empty(&model, &oracle, RobotId(60), ToteId(700), PICK_CELL);
    let supply = supply(&model);
    assert!(supply.best_direct(STATION_CELL, OPENING).expect("reachable").is_none(), "released before the opening");

    let stay = model.available_htm.take(RobotId(60)).unwrap();
    let release = OPENING + SimDuration::seconds(10);
    model.available_htm.register(stay.cell_reservation, release, stay.orientation, None).unwrap();
    let candidate = supply.best_direct(STATION_CELL, OPENING).expect("reachable").expect("released after the opening");
    assert_eq!((candidate.entry.robot(), candidate.depart), (RobotId(60), release));
}

#[test]
fn an_order_tote_is_never_direct_putaway_supply() {
    let (model, oracle) = build_model(TestModelConfig::default());
    model
        .inventory
        .register_tote(
            ToteId(700),
            ToteType::Tote,
            ToteDesignation::Order,
            ToteLocation::WithRobot { robot_id: RobotId(60) },
        )
        .expect("carried order tote");
    register_loaded_htm(&model, &oracle, RobotId(60), CellId(5), ToteId(700));
    assert!(supply(&model).best_direct(STATION_CELL, OPENING).expect("reachable").is_none());
}
