//! Scale, parity, and property-based generator tests.

use super::super::*;

#[test]
fn smoke_test_rtp_config() {
    let config = GeneratorConfig {
        grid_width: 20,
        grid_depth: 20,
        cell_size_mm: 1000.0,
        station_count: 4,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 2,
        rows_per_block: 4,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Rtp,
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.graph.cell_count(), 400);
    assert_eq!(map.stations.len(), 4);
    assert!(map.relays.is_empty());
    assert!(!map.blocks.is_empty());
}

#[test]
fn smoke_test_relay_config() {
    let config = GeneratorConfig {
        grid_width: 20,
        grid_depth: 20,
        cell_size_mm: 1000.0,
        station_count: 1,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 2,
        rows_per_block: 4,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.graph.cell_count(), 400);
    assert_eq!(map.stations.len(), 1);
    assert!(!map.relays.is_empty());
    assert!(!map.blocks.is_empty());

    for relay in &map.relays {
        assert_eq!(relay.htm_access, relay.position);
    }
}

#[test]
fn larger_grid_works() {
    let config = GeneratorConfig {
        grid_width: 50,
        grid_depth: 30,
        station_count: 2,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 3,
        rows_per_block: 10,
        rack_columns: 3,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
        ..Default::default()
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.graph.cell_count(), 50 * 30);
    assert_eq!(map.stations.len(), 2);
    assert!(!map.relays.is_empty());
}

#[test]
fn minimum_grid_produces_valid_map() {
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

    assert_eq!(map.graph.cell_count(), 25);
    assert_eq!(map.stations.len(), 1);

    for cell in map.graph.cells() {
        assert!(map.zone(cell.zone).is_some(), "cell {:?} zone {:?} not found in minimum grid", cell.id, cell.zone);
    }
}

#[test]
fn maximum_scale_200x200_works() {
    let config = GeneratorConfig {
        grid_width: 200,
        grid_depth: 200,
        cell_size_mm: 1000.0,
        station_count: 10,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 3,
        rows_per_block: 10,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.graph.cell_count(), 40_000);
    assert!(!map.blocks.is_empty());
    assert!(!map.relays.is_empty());

    for cell in map.graph.cells() {
        assert!(map.zone(cell.zone).is_some(), "cell {:?} zone {:?} not found in 200x200 grid", cell.id, cell.zone);
    }
}

#[test]
fn parity_smoke_rtp_counts() {
    let config = GeneratorConfig {
        grid_width: 20,
        grid_depth: 20,
        cell_size_mm: 1000.0,
        station_count: 4,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 2,
        rows_per_block: 4,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Rtp,
    };
    let map = generate(&config).unwrap();

    let mut storage = 0usize;
    let mut highway_like = 0usize;
    let mut station_area = 0usize;
    for cell in map.graph.cells() {
        match cell.kind {
            CellKind::Storage => storage += 1,
            CellKind::Highway | CellKind::Presentation | CellKind::QueueCell => highway_like += 1,
            CellKind::StationArea => station_area += 1,
            _ => {}
        }
    }

    assert_eq!(map.graph.cell_count(), 400, "total cells");
    assert_eq!(map.stations.len(), 4, "stations");
    assert_eq!(station_area, 4, "station area cells");
    assert!(highway_like >= 40, "highways: 2 rows * 20 cols = 40 minimum");
    assert!(storage > 0, "must have storage cells");
    assert!(map.blocks.len() >= 2, "should have at least 2 blocks");
    assert_eq!(map.graph.edge_count(), 1520, "edge count for 20x20 grid");
}

#[test]
fn parity_smoke_relay_counts() {
    let config = GeneratorConfig {
        grid_width: 20,
        grid_depth: 20,
        cell_size_mm: 1000.0,
        station_count: 1,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 2,
        rows_per_block: 4,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.graph.cell_count(), 400, "total cells");
    assert_eq!(map.stations.len(), 1, "stations");
    assert!(!map.relays.is_empty(), "must have relays");
    assert!(!map.blocks.is_empty(), "must have blocks");

    let total_block_rows: usize = map.blocks.iter().map(|b| (b.bottom_right.y - b.top_left.y + 1) as usize).sum();
    assert!(
        map.relays.len() <= total_block_rows * 2,
        "relay count {} should be <= 2 * block rows {}",
        map.relays.len(),
        total_block_rows
    );
}

#[test]
fn cell_count_equals_grid_area() {
    let configs = vec![
        GeneratorConfig::default(),
        GeneratorConfig { grid_width: 10, grid_depth: 15, ..Default::default() },
        GeneratorConfig {
            grid_width: 50,
            grid_depth: 30,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            ..Default::default()
        },
    ];

    for config in &configs {
        let map = generate(config).unwrap();
        assert_eq!(
            map.graph.cell_count(),
            (config.grid_width * config.grid_depth) as usize,
            "cell_count mismatch for {}x{}",
            config.grid_width,
            config.grid_depth
        );
    }
}

#[test]
fn all_station_relay_block_ids_exist_in_graph() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let map = generate(&config).unwrap();

    for station in &map.stations {
        assert!(map.graph.cell(station.position).is_some());
        let [point] = station.presentation_points.as_slice() else {
            panic!("station {:?} presents at one cell", station.id)
        };
        assert!(map.graph.cell(point.presentation_cell).is_some());
    }
    for relay in &map.relays {
        assert!(map.graph.cell(relay.position).is_some());
        assert!(map.graph.cell(relay.htm_access).is_some());
        assert!(map.graph.cell(relay.vtm_access.anchor()).is_some());
    }
    for block in &map.blocks {
        for &cid in &block.rack_cells {
            assert!(map.graph.cell(cid).is_some());
        }
        for &cid in &block.aisle_cells {
            assert!(map.graph.cell(cid).is_some());
        }
    }
    for zone in &map.zones {
        for &cid in &zone.cells {
            assert!(map.graph.cell(cid).is_some());
        }
    }
}
