//! A relay exchange happens under its shelf: the HTM drives in forward from
//! its one approach cell, exchanges the tote, and backs out the way it came
//! without turning. Pin that link shape on every compiled map, including
//! both rack faces and the legacy Cencora loader. The back-out itself is the
//! first move of the HTM's next journey, never a journey of its own, so the
//! search for it is tested where journeys are searched: the pathfinder's
//! reverse_drive tests.

use std::path::Path;

use desgraph::{DestGate, HtmDriveDirection, HtmTraversal};
use desmap::{WarehouseDesign, WarehouseMap};
use dessim::worldmap;
use rustc_hash::FxHashSet;

fn check(map: &WarehouseMap) {
    assert!(!map.relays.is_empty());
    // Starting from one relay, ordinary loaded travel must reach every
    // relay approach without passing through another terminal destination.
    let origin = map.relays[0].htm_access;
    let mut reachable = FxHashSet::default();
    reachable.insert(origin);
    let mut pending = vec![origin];
    while let Some(cell) = pending.pop() {
        for (next, edge) in map.graph.neighbors(cell) {
            if edge.traversability.htm_traversal == HtmTraversal::Both
                && edge.dest_only.is_none()
                && reachable.insert(next)
            {
                pending.push(next);
            }
        }
    }
    for relay in &map.relays {
        let stand = relay.htm_access;
        assert!(map.graph.cell(stand).unwrap().htm_rotation_forbidden);
        let exits: Vec<_> = map
            .graph
            .neighbors(stand)
            .into_iter()
            .filter(|(_, edge)| edge.traversability.htm_traversal != HtmTraversal::None)
            .collect();
        assert_eq!(exits.len(), 1, "relay {:?} must only back out to its approach", relay.id);
        let (approach, exit) = exits[0];
        assert!(reachable.contains(&approach), "relay {:?} is disconnected from the shared travel floor", relay.id);
        assert_eq!(exit.htm_drive, HtmDriveDirection::Reverse);
        let entrances: Vec<_> = map
            .graph
            .edges()
            .filter(|(_, to, edge)| *to == stand && edge.traversability.htm_traversal != HtmTraversal::None)
            .collect();
        assert_eq!(entrances.len(), 1, "relay {:?} must not admit along-rack traffic", relay.id);
        assert_eq!(entrances[0].0, approach);
        assert_eq!(entrances[0].2.htm_drive, HtmDriveDirection::Forward);
        assert_eq!(entrances[0].2.dest_only, Some(DestGate::Cell(stand)));

        let at = map.cell_world_pos(stand).unwrap();
        let from = map.cell_world_pos(approach).unwrap();
        let dx = at.x_mm - from.x_mm;
        let dy = at.y_mm - from.y_mm;
        assert!(dx.abs() < 0.11 || dy.abs() < 0.11, "relay entry is perpendicular, not diagonal");
    }
}

#[test]
fn world_maps_give_every_relay_one_gated_forward_entry_and_one_reverse_exit() {
    for fixture in ["minimal_easyloop.json", "walmart_dfw6.json", "cencora_v3.json"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let world = worldmap::load(&path).unwrap_or_else(|e| panic!("{fixture}: {e}"));
        check(&world.map);
    }
}

#[test]
fn legacy_cencora_compiles_the_same_relay_links() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sim/desmap/tests/fixtures/cencora_v3.json");
    let design: WarehouseDesign = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    check(&design.compile().unwrap());
}
