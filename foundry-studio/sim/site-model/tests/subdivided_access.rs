//! A serving region may own several floor sections while remaining one access
//! and one fleet assignment. Validation must inspect the cells it contributes.

use serde_json::json;
use std::path::Path;
use site_model::pipeline::layers::LayerSet;
use site_model::pipeline::{World, derive};
use site_model::{Document, Warning};

const FLOOR: &str = "f0000000-0000-4000-8000-000000000001";
const AISLE: &str = "a0000000-0000-4000-8000-000000000001";
const RACK: &str = "a0000000-0000-4000-8000-000000000002";
const ACCESS: &str = "a0000000-0000-4000-8000-000000000003";
const CLASS: &str = "a0000000-0000-4000-8000-000000000004";
const TEMPLATE: &str = "a0000000-0000-4000-8000-000000000005";

fn load(subdivided: bool, remove_floor: bool, locomotion: &str) -> World {
    let mut tiling = json!({"pitch": [800, 800], "derives": {"cell": {"traversable_by": ["vtm"]}}});
    if remove_floor {
        tiling["exclusions"] = json!([{"origin": [0, 0], "size": [800, 3200]}]);
    }
    let mut aisle = json!({"region": {"frame": FLOOR, "origin": [0, 0], "size": [800, 3200]}});
    let mut sections = Vec::new();
    if subdivided {
        aisle["pose"] = json!({"frame": FLOOR, "position": [0, 0]});
        aisle["stamp"] = json!({"template": TEMPLATE});
        for (name, y) in [("south", 0), ("north", 1600)] {
            sections.push(json!({"local": name, "components": {
                "region": {"origin": [0, y], "size": [800, 1600]},
                "tiling": tiling.clone()
            }}));
        }
        if !remove_floor {
            sections.push(json!({"local": "join", "components": {"stitch": {
                "a": {"local_ref": "south"}, "b": {"local_ref": "north"},
                "rule": "explicit", "bidirectional": false,
                "pairs": [[
                    {"local_ref": "south", "key": "cell:0,1"},
                    {"local_ref": "north", "key": "cell:0,0"}
                ]]
            }}}));
        }
    } else {
        aisle["tiling"] = tiling;
    }
    let source = json!({
        "proto_version": "0.7.0",
        "doc": {"id": "d0000000-0000-4000-8000-000000000001"},
        "primary_frame": FLOOR,
        "entities": [
            {"id": FLOOR, "components": {"frame": {"dims": 2, "units": "mm", "unit_mm": 1}}},
            {"id": AISLE, "components": aisle},
            {"id": TEMPLATE, "components": {"template": {"entities": sections}}},
            {"id": CLASS, "components": {"robot_class": {"role": "vtm", "locomotion": locomotion}}},
            {"id": RACK, "components": {
                "pose": {"frame": FLOOR, "position": [1000, 0]},
                "rack": {"slots_wide": 1, "slots_deep": 1, "slot_pitch": [800, 800],
                    "footprint": [800, 800], "levels": [{"height": 800}]}
            }},
            {"id": ACCESS, "components": {"access": {"rack": RACK, "along": AISLE,
                "face": "-x", "serves": [CLASS]}}}
        ]
    });
    let scenario = Document::from_value(Path::new("subdivided-access"), &source).unwrap();
    derive(&LayerSet { scenario, layers: vec![] }).unwrap()
}

fn missing_floor_warnings(world: &World) -> usize {
    world.warnings.iter().filter(|warning| matches!(warning, Warning::FloorServedAlongIsNoCellGrid { .. })).count()
}

#[test]
fn tiled_and_subdivided_accesses_both_provide_floor() {
    for subdivided in [false, true] {
        let world = load(subdivided, false, "floor");
        assert_eq!(world.cells.len(), 4);
        assert_eq!(missing_floor_warnings(&world), 0);
    }
}

#[test]
fn an_access_whose_floor_was_removed_still_warns_for_floor_robots() {
    for subdivided in [false, true] {
        let world = load(subdivided, true, "floor");
        assert!(world.cells.is_empty());
        assert_eq!(missing_floor_warnings(&world), 1);
    }
}

#[test]
fn ladder_access_does_not_require_floor() {
    let world = load(true, true, "ladder");
    assert!(world.cells.is_empty());
    assert_eq!(missing_floor_warnings(&world), 0);
}

#[test]
fn a_directed_stitch_resolves_cells_of_template_local_floor_sections() {
    let world = load(true, false, "floor");
    let south = world.cells.values().find(|cell| cell.centre.map(|v| v.0) == [400, 1200]).unwrap();
    let north = world.cells.values().find(|cell| cell.centre.map(|v| v.0) == [400, 2000]).unwrap();
    let join = world.edges.iter().find(|edge| edge.from == south.id && edge.to == north.id).unwrap();
    assert!(!join.bidirectional, "the bay seam preserves its declared travel direction");
    assert!(!world.edges.iter().any(|edge| edge.from == north.id && edge.to == south.id));
}
