use crate::PresentationPoint;
use crate::design::*;
use crate::generator::{self, GeneratorConfig, StationLayout};
use crate::types::*;
use std::collections::BTreeSet;

/// Generate a map, extract v3 design, compile back, verify identical.
fn round_trip(config: &GeneratorConfig) {
    let original_map = generator::generate(config).unwrap();
    let ast = extract(&original_map).unwrap();
    assert_eq!(ast.version, 3);
    let compiled_map = ast.compile().unwrap();

    assert_eq!(original_map.graph.cell_count(), compiled_map.graph.cell_count(), "cell count");
    assert_eq!(original_map.graph.edge_count(), compiled_map.graph.edge_count(), "edge count");

    for cell in original_map.graph.cells() {
        let cc = compiled_map.graph.cell(cell.id).unwrap_or_else(|| panic!("cell {:?} missing", cell.id));
        assert_eq!(cell.kind, cc.kind, "kind mismatch {:?}", cell.id);
        assert_eq!(cell.zone, cc.zone, "zone mismatch {:?}", cell.id);
        assert_eq!(cell.grid_pos, cc.grid_pos, "grid_pos mismatch {:?}", cell.id);
        if let (Some(wp1), Some(wp2)) = (cell.world_pos, cc.world_pos) {
            assert!((wp1.x_mm - wp2.x_mm).abs() < 0.001);
            assert!((wp1.y_mm - wp2.y_mm).abs() < 0.001);
        }
    }

    for cell in original_map.graph.cells() {
        let orig: BTreeSet<_> = original_map
            .graph
            .neighbors(cell.id)
            .iter()
            .map(|(id, e)| {
                (*id, e.traversability.vtm_traversal, e.traversability.htm_traversal, e.traversability.rtp_traversal)
            })
            .collect();
        let comp: BTreeSet<_> = compiled_map
            .graph
            .neighbors(cell.id)
            .iter()
            .map(|(id, e)| {
                (*id, e.traversability.vtm_traversal, e.traversability.htm_traversal, e.traversability.rtp_traversal)
            })
            .collect();
        assert_eq!(orig, comp, "neighbors mismatch {:?}", cell.id);
        for (nid, edge) in original_map.graph.neighbors(cell.id) {
            let ce = compiled_map.graph.edge_between(cell.id, nid).unwrap();
            assert_eq!(edge.distance_mm, ce.distance_mm);
        }
    }

    assert_eq!(original_map.stations.len(), compiled_map.stations.len());
    for (o, c) in original_map.stations.iter().zip(&compiled_map.stations) {
        assert_eq!(o.id, c.id);
        assert_eq!(o.position, c.position);
        assert_eq!(o.presentation_points, c.presentation_points);
        assert_eq!(o.edge, c.edge);
    }

    assert_eq!(original_map.relays.len(), compiled_map.relays.len());
    for (o, c) in original_map.relays.iter().zip(&compiled_map.relays) {
        assert_eq!(o.id, c.id);
        assert_eq!(o.position, c.position);
        assert_eq!(o.htm_access, c.htm_access);
        assert_eq!(o.vtm_access, c.vtm_access);
        assert_eq!(o.block, c.block);
        assert_eq!(o.side, c.side);
    }

    assert_eq!(original_map.blocks.len(), compiled_map.blocks.len());
    for (o, c) in original_map.blocks.iter().zip(&compiled_map.blocks) {
        assert_eq!(o.id, c.id);
        assert_eq!(o.top_left, c.top_left);
        assert_eq!(o.bottom_right, c.bottom_right);
        let or: BTreeSet<_> = o.rack_cells.iter().copied().collect();
        let cr: BTreeSet<_> = c.rack_cells.iter().copied().collect();
        assert_eq!(or, cr, "rack_cells block {:?}", o.id);
    }

    assert_eq!(original_map.zones.len(), compiled_map.zones.len());
    for oz in &original_map.zones {
        let cz =
            compiled_map.zones.iter().find(|z| z.id == oz.id).unwrap_or_else(|| panic!("zone {:?} missing", oz.id));
        let oc: BTreeSet<_> = oz.cells.iter().copied().collect();
        let cc: BTreeSet<_> = cz.cells.iter().copied().collect();
        assert_eq!(oc, cc, "zone {:?} cells", oz.id);
        let orr: BTreeSet<_> = oz.relays.iter().copied().collect();
        let crr: BTreeSet<_> = cz.relays.iter().copied().collect();
        assert_eq!(orr, crr, "zone {:?} relays", oz.id);
    }
}

#[test]
fn default_rtp() {
    round_trip(&GeneratorConfig::default());
}

#[test]
fn relay_20x20() {
    round_trip(&GeneratorConfig { system_type: SystemType::Relay, ..Default::default() });
}

#[test]
fn java_smoke_rtp() {
    round_trip(&GeneratorConfig {
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
    });
}

#[test]
fn cencora_like() {
    round_trip(&GeneratorConfig {
        grid_width: 56,
        grid_depth: 34,
        cell_size_mm: 1380.0,
        station_count: 10,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 3,
        rows_per_block: 10,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    });
}

#[test]
fn relay_50x30() {
    round_trip(&GeneratorConfig {
        grid_width: 50,
        grid_depth: 30,
        cell_size_mm: 1000.0,
        station_count: 2,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 3,
        rows_per_block: 10,
        rack_columns: 3,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    });
}

#[test]
fn minimum_5x5() {
    round_trip(&GeneratorConfig {
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
}

#[test]
fn four_edge_layout() {
    round_trip(&GeneratorConfig {
        grid_width: 30,
        grid_depth: 30,
        station_count: 8,
        station_layout: StationLayout::FourEdge,
        ..Default::default()
    });
}

#[test]
fn opposite_horizontal() {
    round_trip(&GeneratorConfig {
        grid_width: 20,
        grid_depth: 20,
        station_count: 4,
        station_layout: StationLayout::OppositeHorizontal,
        ..Default::default()
    });
}

#[test]
fn cross_aisle_2() {
    round_trip(&GeneratorConfig {
        grid_width: 20,
        grid_depth: 30,
        cell_size_mm: 1000.0,
        station_count: 2,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 3,
        rows_per_block: 5,
        rack_columns: 2,
        cross_aisle_width: 2,
        system_type: SystemType::Rtp,
    });
}

#[test]
fn relay_100x100() {
    round_trip(&GeneratorConfig {
        grid_width: 100,
        grid_depth: 100,
        cell_size_mm: 1000.0,
        station_count: 20,
        station_layout: StationLayout::OppositeVertical,
        highways_per_edge: 3,
        rows_per_block: 10,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    });
}

#[test]
fn json_size_is_compact() {
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
    let map = generator::generate(&config).unwrap();
    let ast = extract(&map).unwrap();
    assert!(ast.edge_exceptions.is_empty());
    let json = serde_json::to_string(&ast).unwrap();
    let size_kb = json.len() / 1024;
    // Threshold raised from 100KB after the Zone struct gained
    // zone_height_mm / zone_width_mm / real_shelf_count. Current size
    // is ~140KB; ceiling guards against further unintended growth.
    assert!(size_kb < 150, "v3 design should be under 150KB, got {} KB", size_kb);
}

#[test]
fn serde_round_trip() {
    let map = generator::generate(&GeneratorConfig::default()).unwrap();
    let ast = extract(&map).unwrap();
    let json = serde_json::to_string_pretty(&ast).unwrap();
    let ast2: WarehouseDesign = serde_json::from_str(&json).unwrap();
    let m1 = ast.compile().unwrap();
    let m2 = ast2.compile().unwrap();
    assert_eq!(m1.graph.cell_count(), m2.graph.cell_count());
    assert_eq!(m1.graph.edge_count(), m2.graph.edge_count());
}

/// A v3 design states one presentation cell per station, so extraction
/// refuses a station that presents at two rather than dropping one.
#[test]
fn a_station_presenting_at_two_cells_is_not_extracted() {
    let mut map = generator::generate(&GeneratorConfig::default()).unwrap();
    let station = &mut map.stations[0];
    station.presentation_points.push(PresentationPoint { presentation_cell: station.position, queue_path: vec![] });
    let refusal = extract(&map).expect_err("a station presenting at two cells has no v3 design");
    assert!(refusal.contains("presents at 2 cells"), "{refusal}");
}

#[test]
fn double_round_trip() {
    let config = GeneratorConfig { system_type: SystemType::Relay, ..Default::default() };
    let m1 = generator::generate(&config).unwrap();
    let a1 = extract(&m1).unwrap();
    let m2 = a1.compile().unwrap();
    let a2 = extract(&m2).unwrap();
    let m3 = a2.compile().unwrap();
    assert_eq!(m2.graph.cell_count(), m3.graph.cell_count());
    assert_eq!(m2.graph.edge_count(), m3.graph.edge_count());
    assert_eq!(serde_json::to_string(&a1).unwrap(), serde_json::to_string(&a2).unwrap());
}

#[test]
fn directional_round_trip() {
    let ast = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(10, 8, 1000.0),
        highways: vec![
            HighwayDefinition { origin: [0, 1], width: 10, height: 1, direction: LaneDirection::Eastbound },
            HighwayDefinition { origin: [0, 2], width: 10, height: 1, direction: LaneDirection::Westbound },
            HighwayDefinition { origin: [0, 3], width: 10, height: 1, direction: LaneDirection::Bidirectional },
        ],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [0, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    };
    let extracted = extract(&ast.compile().unwrap()).unwrap();
    assert_eq!(extracted.highways.len(), 3);
    assert_eq!(extracted.highways[0].direction, LaneDirection::Eastbound);
    assert_eq!(extracted.highways[1].direction, LaneDirection::Westbound);
    assert_eq!(extracted.highways[2].direction, LaneDirection::Bidirectional);
}
