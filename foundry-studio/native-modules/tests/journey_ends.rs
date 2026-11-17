//! Every place a task delivers an HTM to is a journey end by its kind alone,
//! and every queue cell it passes through on the way is not. `reserve_path`
//! reads nothing but the kind of the cell it is asked to reach, so a fixture
//! that labelled one of these places otherwise would abort the first journey
//! to it, or let a task stage its way through a queue.

use std::path::PathBuf;

use desgraph::CellId;
use dessim::pathfinding::is_journey_end;
use dessim::worldmap::{self, WorldMap};

fn load(name: &str) -> WorldMap {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    worldmap::load(&path).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The cells each role names, by role, so a failure says which place is
/// mislabelled and how.
fn ends_and_queues(world: &WorldMap) -> (Vec<(&'static str, CellId)>, Vec<(&'static str, CellId)>) {
    let map = &world.map;
    let mut ends = Vec::new();
    let mut queues = Vec::new();
    for station in &map.stations {
        ends.extend(station.presentation_cells().map(|cell| ("station presentation", cell)));
        ends.extend(station.order_presentation_cell.map(|cell| ("order presentation", cell)));
        ends.extend(station.dropoff_cell.map(|cell| ("dropoff port", cell)));
        ends.extend(station.collection_cell.map(|cell| ("collection port", cell)));
        queues.extend(
            station.presentation_points.iter().flat_map(|point| &point.queue_path).map(|&cell| ("station queue", cell)),
        );
        for lane in &station.handoff_queues {
            queues.extend(lane.path.iter().map(|&cell| ("handoff lane", cell)));
        }
    }
    for machine in &map.conveyor_machines {
        ends.extend(machine.presentations.iter().map(|presentation| ("machine presentation", presentation.cell)));
        if let Some(lane) = &machine.inventory_queue {
            queues.extend(lane.path.iter().map(|&cell| ("machine inventory lane", cell)));
        }
    }
    ends.extend(map.relays.iter().map(|relay| ("relay access", relay.htm_access)));
    (ends, queues)
}

fn assert_ends_by_kind(name: &str) {
    let world = load(name);
    let kind = |cell: CellId| world.map.graph.cell(cell).unwrap_or_else(|| panic!("{name}: {cell:?} is no cell")).kind;
    let (ends, queues) = ends_and_queues(&world);
    assert!(ends.iter().any(|(role, _)| *role == "relay access"), "{name} has relays");
    assert!(!queues.is_empty(), "{name} has station queues");
    let refused: Vec<_> =
        ends.iter().map(|&(role, cell)| (role, cell, kind(cell))).filter(|&(_, _, k)| !is_journey_end(k)).collect();
    assert!(refused.is_empty(), "{name}: places a task delivers to that reserve_path would refuse: {refused:?}");
    let accepted: Vec<_> =
        queues.iter().map(|&(role, cell)| (role, cell, kind(cell))).filter(|&(_, _, k)| is_journey_end(k)).collect();
    assert!(accepted.is_empty(), "{name}: queue cells reserve_path would accept as a target: {accepted:?}");
}

#[test]
fn cencora_v3_presentations_and_relay_access_cells_are_ends_and_its_queues_are_not() {
    assert_ends_by_kind("cencora_v3.json");
}

#[test]
fn dfw6_presentations_ports_and_relay_access_cells_are_ends_and_its_queues_are_not() {
    assert_ends_by_kind("walmart_dfw6.json");
}
