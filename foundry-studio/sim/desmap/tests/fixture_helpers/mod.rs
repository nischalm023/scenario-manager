//! Shared helpers for fixture-based integration tests.
#![allow(dead_code, unused_macros, unused_imports)]

use desmap::design::WarehouseDesign;
use desmap::generator::GeneratorConfig;
use desmap::map::WarehouseMap;
use desmap::types::*;
use std::collections::{HashSet, VecDeque};
use std::path::Path;

/// Load a fixture design JSON file, compile it to a WarehouseMap.
pub fn load_fixture(name: &str) -> WarehouseMap {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{}.json", name));
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read fixture {:?}: {}", path, e));
    let design: WarehouseDesign =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("cannot deserialize fixture {:?}: {}", path, e));
    design.compile().unwrap_or_else(|e| panic!("cannot compile fixture {:?}: {}", path, e))
}

/// Generate a map from config.
pub fn generate_map(config: &GeneratorConfig) -> WarehouseMap {
    desmap::generator::generate(config).expect("generation failed")
}

/// Deep-compare two maps on all structural properties.
pub fn assert_maps_equal(name: &str, expected: &WarehouseMap, actual: &WarehouseMap) {
    assert_eq!(expected.version, actual.version, "{name}: version");
    assert_eq!(expected.system_type, actual.system_type, "{name}: system_type");
    assert_eq!(expected.grid_bounds, actual.grid_bounds, "{name}: grid_bounds");
    assert_eq!(expected.graph.cell_count(), actual.graph.cell_count(), "{name}: cell_count");
    assert_eq!(expected.graph.edge_count(), actual.graph.edge_count(), "{name}: edge_count");
    assert_eq!(expected.stations.len(), actual.stations.len(), "{name}: station count");
    assert_eq!(expected.relays.len(), actual.relays.len(), "{name}: relay count");
    assert_eq!(expected.zones.len(), actual.zones.len(), "{name}: zone count");
    assert_eq!(expected.blocks.len(), actual.blocks.len(), "{name}: block count");

    for (i, (es, as_)) in expected.stations.iter().zip(actual.stations.iter()).enumerate() {
        assert_eq!(es.id, as_.id, "{name}: station[{i}] id");
        assert_eq!(es.position, as_.position, "{name}: station[{i}] position");
        assert_eq!(es.presentation_points, as_.presentation_points, "{name}: station[{i}] presentation points");
        assert_eq!(es.edge, as_.edge, "{name}: station[{i}] edge");
    }

    for (i, (er, ar)) in expected.relays.iter().zip(actual.relays.iter()).enumerate() {
        assert_eq!(er.id, ar.id, "{name}: relay[{i}] id");
        assert_eq!(er.position, ar.position, "{name}: relay[{i}] position");
        assert_eq!(er.htm_access, ar.htm_access, "{name}: relay[{i}] htm_access");
        assert_eq!(er.vtm_access, ar.vtm_access, "{name}: relay[{i}] vtm_access");
        assert_eq!(er.block, ar.block, "{name}: relay[{i}] block");
        assert_eq!(er.side, ar.side, "{name}: relay[{i}] side");
    }

    for (i, (eb, ab)) in expected.blocks.iter().zip(actual.blocks.iter()).enumerate() {
        assert_eq!(eb.id, ab.id, "{name}: block[{i}] id");
        assert_eq!(eb.top_left, ab.top_left, "{name}: block[{i}] top_left");
        assert_eq!(eb.bottom_right, ab.bottom_right, "{name}: block[{i}] bottom_right");
        assert_eq!(eb.rack_cells.len(), ab.rack_cells.len(), "{name}: block[{i}] rack_cells count");
    }

    for cell in expected.graph.cells().take(50) {
        let ac = actual.graph.cell(cell.id);
        assert!(ac.is_some(), "{name}: cell {:?} missing", cell.id);
        let ac = ac.unwrap();
        assert_eq!(cell.kind, ac.kind, "{name}: cell {:?} kind", cell.id);
        assert_eq!(cell.grid_pos, ac.grid_pos, "{name}: cell {:?} grid_pos", cell.id);
    }
}

/// Validate structural invariants on a map, independent of fixture comparison.
pub fn assert_invariants(name: &str, map: &WarehouseMap, config: &GeneratorConfig) {
    let bounds = map.grid_bounds.unwrap();
    let expected_cells = (bounds.width * bounds.depth) as usize;

    assert_eq!(map.graph.cell_count(), expected_cells, "{name}: cell count vs grid");
    assert_eq!(map.stations.len(), config.station_count as usize, "{name}: station count vs config");
    assert_eq!(map.system_type, config.system_type, "{name}: system_type vs config");

    if config.system_type == SystemType::Rtp {
        assert!(map.relays.is_empty(), "{name}: RTP should have no relays");
    } else if !map.blocks.is_empty() {
        assert!(!map.relays.is_empty(), "{name}: RELAY with blocks should have relays");
    }

    for relay in &map.relays {
        assert_eq!(relay.htm_access, relay.position, "{name}: relay {:?} htm_access != position", relay.id);
        assert_ne!(relay.vtm_access.anchor(), relay.position, "{name}: relay {:?} vtm_access == position", relay.id);
    }

    for station in &map.stations {
        assert!(map.graph.cell(station.position).is_some(), "{name}: station {:?} position invalid", station.id);
        let [point] = station.presentation_points.as_slice() else {
            panic!("{name}: station {:?} presents at one cell", station.id)
        };
        assert!(
            map.graph.cell(point.presentation_cell).is_some(),
            "{name}: station {:?} presentation_cell invalid",
            station.id
        );
        let cell = map.graph.cell(station.position).unwrap();
        assert_eq!(cell.kind, CellKind::StationArea, "{name}: station {:?} cell kind != StationArea", station.id);
    }

    for zone in &map.zones {
        assert!(!zone.cells.is_empty(), "{name}: zone {:?} has no cells", zone.id);
    }

    if config.system_type == SystemType::Rtp {
        for block in &map.blocks {
            assert!(!block.rack_cells.is_empty(), "{name}: block {:?} has no rack cells (RTP)", block.id);
        }
    }

    // Graph connectivity
    let total = map.graph.cell_count();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(CellId(0));
    queue.push_back(CellId(0));
    while let Some(current) = queue.pop_front() {
        for (neighbor, _) in map.graph.neighbors(current) {
            if visited.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
    assert_eq!(visited.len(), total, "{name}: not fully connected ({} of {})", visited.len(), total);

    // Bidirectionality
    for cell in map.graph.cells() {
        for (nid, _) in map.graph.neighbors(cell.id) {
            assert!(
                map.graph.edge_between(nid, cell.id).is_some(),
                "{name}: edge {:?} -> {:?} has no reverse",
                cell.id,
                nid
            );
        }
    }
}

/// Define a fixture test: generates from config, loads fixture, compares.
macro_rules! fixture_test {
    ($test_name:ident, $fixture_name:expr_2021, $config:expr_2021) => {
        #[test]
        fn $test_name() {
            let config = $config;
            let expected = $crate::fixture_helpers::generate_map(&config);
            let actual = $crate::fixture_helpers::load_fixture($fixture_name);
            $crate::fixture_helpers::assert_maps_equal($fixture_name, &expected, &actual);
            $crate::fixture_helpers::assert_invariants($fixture_name, &actual, &config);
        }
    };
}

pub(crate) use fixture_test;
