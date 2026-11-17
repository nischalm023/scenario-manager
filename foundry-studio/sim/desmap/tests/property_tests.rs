//! Property-based and edge-case validation tests for compiled fixtures.

mod fixture_helpers;

use desmap::generator::{GeneratorConfig, StationLayout};
use desmap::map::WarehouseMap;
use desmap::types::*;
use std::collections::{HashSet, VecDeque};

use fixture_helpers::{generate_map, load_fixture};

#[test]
fn minimum_grid_has_no_or_few_blocks() {
    let map = load_fixture("edge_minimum_grid");
    let generated = generate_map(&GeneratorConfig {
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
    });
    assert_eq!(map.blocks.len(), generated.blocks.len());
}

#[test]
fn cencora_like_has_significant_storage() {
    let map = load_fixture("cencora_like");
    let storage_cells: usize =
        map.graph.cells().filter(|c| c.kind == CellKind::Storage || c.kind == CellKind::Relay).count();
    let total = map.graph.cell_count();
    let density = storage_cells as f64 / total as f64;
    assert!(density > 0.2, "cencora_like should have >20% storage density, got {:.1}%", density * 100.0);
}

#[test]
fn cencora_like_l_shaped_compiles_and_has_l_shape() {
    let map = load_fixture("cencora_like_l_shaped");

    assert_eq!(map.graph.cell_count(), 56 * 42, "grid should be 56x42");
    assert_eq!(map.stations.len(), 10);
    assert!(!map.relays.is_empty());
    assert_eq!(map.blocks.len(), 48, "should have 48 blocks (19+19+10)");

    // L-shape: right side extends further down than left side
    let left_max_y = map.blocks.iter().filter(|b| b.top_left.x < 26).map(|b| b.bottom_right.y).max().unwrap_or(0);
    let right_max_y = map.blocks.iter().filter(|b| b.top_left.x >= 29).map(|b| b.bottom_right.y).max().unwrap_or(0);
    assert_eq!(left_max_y, 24);
    assert_eq!(right_max_y, 35);
    assert!(right_max_y > left_max_y, "right side should extend further (L-shape)");

    let storage_cells: usize =
        map.graph.cells().filter(|c| c.kind == CellKind::Storage || c.kind == CellKind::Relay).count();
    let density = storage_cells as f64 / map.graph.cell_count() as f64;
    assert!(density > 0.15, "should have >15% density, got {:.1}%", density * 100.0);

    assert_connected(&map, "cencora_like_l_shaped");
}

#[test]
fn relay_positions_cover_block_edges() {
    let map = load_fixture("java_relay_30x30");
    for block in &map.blocks {
        let has_relays = map.relays.iter().any(|r| r.block == block.id);
        assert!(has_relays, "block {:?} should have relays on at least one side", block.id);
    }
}

#[test]
fn four_edge_distribution() {
    let map = load_fixture("layout_four_edge");
    let edges_used: HashSet<_> = map.stations.iter().filter_map(|s| s.edge).collect();
    assert_eq!(edges_used.len(), 4, "FourEdge should use all 4 edges, got {:?}", edges_used);
}

#[test]
fn opposite_vertical_edge_split() {
    let map = load_fixture("layout_opposite_vertical");
    let bottom = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Bottom)).count();
    let top = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Top)).count();
    assert_eq!(bottom, 3);
    assert_eq!(top, 3);
}

#[test]
fn opposite_horizontal_edge_split() {
    let map = load_fixture("layout_opposite_horizontal");
    let left = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Left)).count();
    let right = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Right)).count();
    assert_eq!(left, 2);
    assert_eq!(right, 2);
}

#[test]
fn custom_layout_correct_edges() {
    let map = load_fixture("layout_custom");
    let bottom = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Bottom)).count();
    let right = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Right)).count();
    assert_eq!(bottom, 3);
    assert_eq!(right, 2);
    assert_eq!(map.stations.len(), 5);
}

#[test]
fn scale_200x200_properties() {
    let map = load_fixture("scale_200x200");
    assert_eq!(map.graph.cell_count(), 40000);
    assert_eq!(map.stations.len(), 40);
    assert!(!map.relays.is_empty());
    assert!(!map.blocks.is_empty());

    let utilized = map.graph.cells().filter(|c| c.kind == CellKind::Storage || c.kind == CellKind::Relay).count();
    assert!(utilized > 10000, "200x200 should have >10K storage+relay cells, got {}", utilized);
}

#[test]
fn large_cell_size_world_coordinates() {
    let map = load_fixture("edge_large_cell_size");
    assert_eq!(map.grid_bounds.unwrap().cell_size_mm, 2000.0);

    let wp0 = map.graph.cell(CellId(0)).unwrap().world_pos.unwrap();
    assert!((wp0.x_mm - 1000.0).abs() < 0.001);
    assert!((wp0.y_mm - 1000.0).abs() < 0.001);

    let wp1 = map.graph.cell(CellId(1)).unwrap().world_pos.unwrap();
    assert!((wp1.x_mm - 3000.0).abs() < 0.001);

    let edge = map.graph.edge_between(CellId(0), CellId(1)).unwrap();
    assert_eq!(edge.distance_mm, 2000);
}

#[test]
fn narrow_grids_are_connected() {
    for fixture in ["edge_narrow_5x50", "edge_narrow_50x5"] {
        let map = load_fixture(fixture);
        assert_connected(&map, fixture);
    }
}

#[test]
fn cross_aisle_width_2_produces_wider_gaps() {
    let map = load_fixture("edge_cross_aisle_2");
    if map.blocks.len() >= 2 {
        let mut blocks_sorted: Vec<_> = map.blocks.iter().collect();
        blocks_sorted.sort_by_key(|b| (b.top_left.x, b.top_left.y));
        for pair in blocks_sorted.windows(2) {
            if pair[0].top_left.x == pair[1].top_left.x {
                let gap = pair[1].top_left.y as i32 - pair[0].bottom_right.y as i32 - 1;
                if gap > 0 {
                    assert!(gap >= 2, "cross-aisle gap should be >= 2, got {}", gap);
                }
            }
        }
    }
}

#[test]
fn serde_round_trip_stability() {
    let map = load_fixture("java_smoke_rtp");
    let json1 = serde_json::to_string_pretty(&map).unwrap();
    let map2: WarehouseMap = serde_json::from_str(&json1).unwrap();
    let json2 = serde_json::to_string_pretty(&map2).unwrap();
    assert_eq!(json1, json2, "re-serialization should produce identical JSON");
}

#[test]
fn station_ids_unique() {
    for fixture in ["default_rtp", "layout_four_edge", "layout_custom", "cencora_like"] {
        let map = load_fixture(fixture);
        let ids: HashSet<_> = map.stations.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), map.stations.len(), "{fixture}: station IDs should be unique");
    }
}

#[test]
fn relay_ids_unique() {
    for fixture in ["java_smoke_relay", "java_relay_30x30", "cencora_like", "edge_relay_small"] {
        let map = load_fixture(fixture);
        let ids: HashSet<_> = map.relays.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), map.relays.len(), "{fixture}: relay IDs should be unique");
    }
}

#[test]
fn block_ids_unique() {
    for fixture in ["default_rtp", "java_integration_rtp", "cencora_like", "scale_100x100"] {
        let map = load_fixture(fixture);
        let ids: HashSet<_> = map.blocks.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), map.blocks.len(), "{fixture}: block IDs should be unique");
    }
}

#[test]
fn no_station_overlap() {
    for fixture in ["layout_four_edge", "layout_custom", "cencora_like"] {
        let map = load_fixture(fixture);
        let positions: HashSet<_> = map.stations.iter().map(|s| s.position).collect();
        assert_eq!(positions.len(), map.stations.len(), "{fixture}: no station overlap");
    }
}

fn assert_connected(map: &WarehouseMap, name: &str) {
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
}
