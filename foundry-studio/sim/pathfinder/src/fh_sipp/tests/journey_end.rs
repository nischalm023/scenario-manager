//! Which cells `reserve_path` accepts as a journey's target, read off the
//! target's kind and nothing else. One test per `CellKind`: each reserves from
//! open floor to one neighbour of that kind over an ordinary link, so a
//! refusal can only be the kind's.

use super::prelude::*;

/// Open floor at `c(0)` beside one cell of `kind` at `c(1)`.
fn beside(kind: CellKind) -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, kind, 1, 0));
    b.add_edge_bidi(c(0), c(1), Edge::uniform(1000));
    b.build()
}

fn config() -> CollisionConfig {
    CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: None,
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 1_000,
        motion_model: MotionModel::ConstantSpeed,
    }
}

fn assert_end(kind: CellKind) {
    let oracle = TableOracle::new(ReservationTable::default());
    let cfg = config();
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved = pathfinder(&beside(kind))
        .reserve_path_without_waiting(c(0), Orientation::East, c(1), drive, &oracle, &cfg)
        .unwrap_or_else(|e| panic!("a journey may end on {kind:?}: {e}"));
    assert_eq!(reserved.path.waypoints.last().unwrap().cell, c(1));
    assert!(reserved.committed_reservations.last().is_some_and(|hold| hold.cell() == c(1)));
}

fn assert_refused(kind: CellKind) {
    let oracle = TableOracle::new(ReservationTable::default());
    let cfg = config();
    let drive = &uniform_drive(RobotType::Htm, false);
    let refused =
        pathfinder(&beside(kind)).reserve_path_without_waiting(c(0), Orientation::East, c(1), drive, &oracle, &cfg);
    assert!(
        matches!(refused, Err(PathfinderError::NotAJourneyEnd { cell, kind: named }) if cell == c(1) && named == kind),
        "{kind:?}: {refused:?}"
    );
    assert!(oracle.reservations_for_cell(c(0)).is_empty() && oracle.reservations_for_cell(c(1)).is_empty());
}

#[test]
fn presentation_is_an_end() {
    assert_end(CellKind::Presentation);
}

#[test]
fn exchange_is_an_end() {
    assert_end(CellKind::Exchange);
}

#[test]
fn relay_is_an_end() {
    assert_end(CellKind::Relay);
}

#[test]
fn charging_is_an_end() {
    assert_end(CellKind::Charging);
}

#[test]
fn open_is_an_end() {
    assert_end(CellKind::Open);
}

#[test]
fn queue_cell_is_refused() {
    assert_refused(CellKind::QueueCell);
}

#[test]
fn highway_is_refused() {
    assert_refused(CellKind::Highway);
}

#[test]
fn aisle_is_refused() {
    assert_refused(CellKind::Aisle);
}

#[test]
fn storage_is_refused() {
    assert_refused(CellKind::Storage);
}

#[test]
fn station_area_is_refused() {
    assert_refused(CellKind::StationArea);
}

#[test]
fn junction_is_refused() {
    assert_refused(CellKind::Junction);
}

#[test]
fn blank_is_refused() {
    assert_refused(CellKind::Blank);
}
