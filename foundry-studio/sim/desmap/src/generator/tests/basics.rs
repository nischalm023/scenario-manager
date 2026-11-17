//! Basic generator tests: default config, validation, cell kinds, coordinates, serde.

use std::collections::{HashSet, VecDeque};

use super::super::*;

#[test]
fn default_config_produces_valid_map() {
    let config = GeneratorConfig::default();
    let map = generate(&config).unwrap();

    assert_eq!(map.version, 1);
    assert_eq!(map.system_type, SystemType::Rtp);
    assert_eq!(map.graph.cell_count(), 20 * 20);

    let bounds = map.grid_bounds.unwrap();
    assert_eq!(bounds.width, 20);
    assert_eq!(bounds.depth, 20);
    assert_eq!(bounds.cell_size_mm, 1000.0);

    assert!(!map.stations.is_empty(), "should have stations");
    assert!(!map.blocks.is_empty(), "should have rack blocks");
    assert!(map.relays.is_empty(), "RTP should have no relays");
}

#[test]
fn invalid_config_returns_error() {
    let config = GeneratorConfig { grid_width: 3, ..Default::default() };
    assert!(generate(&config).is_err());

    let config = GeneratorConfig { cell_size_mm: 100.0, ..Default::default() };
    assert!(generate(&config).is_err());
}

#[test]
fn all_cells_have_grid_positions() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    for cell in map.graph.cells() {
        assert!(cell.grid_pos.is_some(), "parametric cells must have grid positions");
        assert!(cell.world_pos.is_some(), "parametric cells must have world positions");
    }
}

#[test]
fn graph_is_fully_connected() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    let total = map.graph.cell_count();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    let start = CellId(0);
    visited.insert(start);
    queue.push_back(start);

    while let Some(current) = queue.pop_front() {
        for (neighbor, _edge) in map.graph.neighbors(current) {
            if visited.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }

    assert_eq!(
        visited.len(),
        total,
        "all {} cells should be reachable from cell 0, but only reached {}",
        total,
        visited.len()
    );
}

#[test]
fn edges_are_bidirectional() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    for cell in map.graph.cells() {
        for (neighbor_id, _edge) in map.graph.neighbors(cell.id) {
            assert!(
                map.graph.edge_between(neighbor_id, cell.id).is_some(),
                "edge {:?} -> {:?} exists but reverse does not",
                cell.id,
                neighbor_id
            );
        }
    }
}

#[test]
fn cell_kinds_are_correct() {
    let config = GeneratorConfig::default();
    let map = generate(&config).unwrap();

    let mut storage_count = 0usize;
    let mut highway_count = 0usize;
    let mut station_count = 0usize;

    for cell in map.graph.cells() {
        match cell.kind {
            CellKind::Storage => storage_count += 1,
            CellKind::Highway => highway_count += 1,
            CellKind::StationArea => station_count += 1,
            _ => {}
        }
    }

    assert!(storage_count > 0, "should have storage cells");
    assert!(highway_count > 0, "should have highway cells");
    assert_eq!(station_count, config.station_count as usize, "station area count should match config");
}

#[test]
fn world_coordinates_are_correct() {
    let config = GeneratorConfig {
        grid_width: 5,
        grid_depth: 5,
        cell_size_mm: 1000.0,
        station_count: 1,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 1,
        rows_per_block: 3,
        rack_columns: 1,
        cross_aisle_width: 1,
        system_type: SystemType::Rtp,
    };
    let map = generate(&config).unwrap();

    let cell_id = CellId(3 * 5 + 2);
    let cell = map.graph.cell(cell_id).unwrap();
    let wp = cell.world_pos.unwrap();
    assert!((wp.x_mm - 2500.0).abs() < 0.001);
    assert!((wp.y_mm - 3500.0).abs() < 0.001);
}

#[test]
fn edge_distances_match_cell_size() {
    let config = GeneratorConfig { cell_size_mm: 1380.0, ..Default::default() };
    let map = generate(&config).unwrap();

    let neighbors = map.graph.neighbors(CellId(0));
    assert!(!neighbors.is_empty());
    for (_id, edge) in &neighbors {
        assert_eq!(edge.distance_mm, 1380);
    }
}

#[test]
fn serde_round_trip() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    let json = serde_json::to_string(&map).unwrap();
    let map2: WarehouseMap = serde_json::from_str(&json).unwrap();

    assert_eq!(map.graph.cell_count(), map2.graph.cell_count());
    assert_eq!(map.graph.edge_count(), map2.graph.edge_count());
    assert_eq!(map.stations.len(), map2.stations.len());
    assert_eq!(map.blocks.len(), map2.blocks.len());
    assert_eq!(map.relays.len(), map2.relays.len());
    assert_eq!(map.zones.len(), map2.zones.len());
}

#[test]
fn no_duplicate_cells() {
    let map = generate(&GeneratorConfig::default()).unwrap();
    let mut ids: HashSet<CellId> = HashSet::new();
    for cell in map.graph.cells() {
        assert!(ids.insert(cell.id), "duplicate CellId: {:?}", cell.id);
    }
}
