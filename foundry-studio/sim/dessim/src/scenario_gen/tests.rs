//! Unit tests for rack layout, SKU/order generation, VTM zone assignment,
//! and ID sequencing — the `scenario_gen` steps that can be checked against a
//! hand-built map without running the full pipeline.

use desgraph::{CellId, GridPos, ZoneId};
use desmap::{WarehouseDesign, WarehouseMap, Zone};

use crate::scenario::GeneratedScenario;

use super::orders::{generate_orders, generate_skus};
use super::topology::{build_rack_layout, collect_storage_cells};
use super::vtm_placement::assign_vtm_zones;

/// Compile a minimal single-zone v3 design with extra raw top-level JSON
/// (e.g. a `"racks"` table) and extra raw zone JSON.
/// Dual zone at [1,1], length 3, on an 8×6 grid with open floor around it.
fn compile_one_zone(top_extras: &str, zone_extras: &str) -> WarehouseMap {
    let json = format!(
        r#"{{
            "version":3,
            "grid":{{"width":8,"depth":6,"cell_size_mm":1000.0}}{top_extras},
            "zones":[{{"type":"relay","id":1,"origin":[1,1],"length":3,"sides":"Dual",
                       "vertical_tote_capacity":4,"zone_height_mm":9000,"real_shelf_count":10{zone_extras}}}],
            "stations":[]
        }}"#
    );
    let design: WarehouseDesign = serde_json::from_str(&json).expect("parse design");
    design.compile().expect("compile design")
}

#[test]
fn rack_layout_default_is_one_column_per_cell() {
    let map = compile_one_zone("", "");
    let storage_cells = collect_storage_cells(&map, true);
    let layout = build_rack_layout(&map, &storage_cells).unwrap().columns;
    // One column per storage cell, in the same order, no geometry override.
    assert_eq!(layout.len(), storage_cells.len());
    assert!(layout.iter().all(|c| c.world_pos.is_none() && c.floor_m.is_none()));
    assert_eq!(layout.iter().map(|c| c.cell).collect::<Vec<_>>(), storage_cells);
}

#[test]
fn rack_layout_emits_each_explicit_slot() {
    // Two prototypes stamped onto the zone: the Left side (absolute col 1)
    // gets 3 storage slots on two levels plus a 2-board diving stack; the
    // Right side (absolute col 3) gets 1 storage slot. Diving boards are
    // exchange positions, not racks. Anchor cells are materialized from
    // each slot's x on the zone-wide frame (span 0..2000 → rows 0..2).
    let map = compile_one_zone(
        r#","racks":[
            {"name":"left","length_mm":2200.0,"slots":[
                {"x":400.0,"y":394.0,"diving_board":true,"diving_board_slot":0},
                {"x":400.0,"y":952.0,"diving_board":true,"diving_board_slot":1},
                {"x":0.0,"y":1580.0},
                {"x":2000.0,"y":1580.0},
                {"x":1000.0,"y":2138.0}]},
            {"name":"right","length_mm":2200.0,"slots":[{"x":1000.0,"y":1580.0}]}]"#,
        r#","rack_placements":[{"rack":"left","side":"Left"},{"rack":"right","side":"Right"}]"#,
    );
    let storage_cells = collect_storage_cells(&map, true);
    assert_eq!(storage_cells.len(), 6);

    let rack_layout = build_rack_layout(&map, &storage_cells).unwrap();
    let layout = &rack_layout.columns;
    assert_eq!(layout.len(), 4, "4 storage slots, boards excluded");
    assert!(layout.iter().all(|s| s.world_pos.is_some() && s.floor_m.is_some()));

    // Left column emitted first (placement order), then right column.
    let floors: Vec<f64> = layout.iter().map(|s| s.floor_m.unwrap()).collect();
    assert_eq!(floors, vec![1.580, 1.580, 2.138, 1.580]);

    // Anchor cells materialize from x: 0 → row 0, 2000 → row 2, 1000 →
    // row 1 (absolute grid rows 1/3/2 from zone origin [1,1]).
    let anchor_rows: Vec<u32> = layout.iter().map(|s| map.graph.cell(s.cell).unwrap().grid_pos.unwrap().y).collect();
    assert_eq!(anchor_rows, vec![1, 3, 2, 2]);

    // World positions follow the zone-wide x frame (0..2000 span → rows 1..3).
    let ys: Vec<f64> = layout.iter().map(|s| s.world_pos.unwrap().y_mm).collect();
    assert!(ys[0] < ys[2] && ys[2] < ys[1], "x ordering not reflected: {ys:?}");
    assert!((((ys[2] - ys[0]) / (ys[1] - ys[0])) - 0.5).abs() < 1e-9, "mid slot not centered: {ys:?}");

    // Board 0 at relative cell [0,0] (absolute [1,1]) yields that relay cell's
    // exchange position; the higher board (952) does not add a second entry.
    let relay_cell = map.graph.cells().find(|c| c.grid_pos == Some(GridPos { x: 1, y: 1 })).unwrap().id;
    assert!(rack_layout.relay_exchange_pos.contains_key(&relay_cell), "board-0 exchange pos missing");
}

#[test]
fn sku_catalog_respects_popularity_split() {
    let scenario = GeneratedScenario {
        num_skus: 100,
        total_inventory_units: 10_000,
        popular_skus_pct: 0.2,
        inventory_popular_share: 0.8,
        ..GeneratedScenario::default()
    };
    let skus = generate_skus(&scenario);

    assert_eq!(skus.len(), 100);
    let popular: Vec<_> = skus.iter().filter(|s| s.is_popular).collect();
    let unpopular: Vec<_> = skus.iter().filter(|s| !s.is_popular).collect();
    assert_eq!(popular.len(), 20);
    assert_eq!(unpopular.len(), 80);

    let popular_inv: u32 = popular.iter().map(|s| s.inventory_units).sum();
    let unpopular_inv: u32 = unpopular.iter().map(|s| s.inventory_units).sum();
    assert_eq!(popular_inv + unpopular_inv, 10_000);
    assert_eq!(popular_inv, 8_000);
}

#[test]
fn order_generation_distributes_picks_correctly() {
    let scenario = GeneratedScenario {
        num_skus: 10,
        total_inventory_units: 100,
        num_orders: 5,
        total_order_picks: 25,
        popular_skus_pct: 0.2,
        inventory_popular_share: 0.8,
        order_popular_share: 0.7,
        ..Default::default()
    };
    let skus = generate_skus(&scenario);
    let result = generate_orders(&scenario, &skus);

    assert_eq!(result.orders.len(), 5);
    let total: u32 = result.orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity).sum();
    assert_eq!(total, 25);
    assert_eq!(result.picks_dropped, 0);
}

#[test]
fn empty_scenario_produces_no_orders() {
    let scenario = GeneratedScenario {
        num_skus: 10,
        total_inventory_units: 100,
        num_orders: 0,
        total_order_picks: 0,
        ..GeneratedScenario::default()
    };
    let skus = generate_skus(&scenario);
    let result = generate_orders(&scenario, &skus);
    assert!(result.orders.is_empty());
    assert_eq!(result.picks_dropped, 0);
}

#[test]
fn inventory_aware_orders_drop_picks_when_inventory_low() {
    let scenario = GeneratedScenario {
        num_skus: 2,
        total_inventory_units: 10,
        num_orders: 10,
        total_order_picks: 100,
        popular_skus_pct: 0.5,
        inventory_popular_share: 0.5,
        order_popular_share: 0.5,
        ..Default::default()
    };
    let skus = generate_skus(&scenario);
    let result = generate_orders(&scenario, &skus);

    let total_picks: u32 = result.orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity).sum();
    assert!(total_picks <= 10);
    assert!(result.picks_dropped > 0);
    assert_eq!(result.picks_dropped + total_picks as usize, 100);
}

#[test]
fn vtm_zone_assignment_uses_vtm_count() {
    use desgraph::RelayId;
    // Each relay zone carries its own vtm_count from the map.
    let zones = vec![
        Zone {
            id: ZoneId(1),
            name: None,
            cells: vec![CellId(10)],
            relays: vec![RelayId(0)],
            vtm_count: 2,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 9000,
            zone_width_mm: None,
            real_shelf_count: 10,
            slots: None,
        },
        Zone {
            id: ZoneId(2),
            name: None,
            cells: vec![CellId(20)],
            relays: vec![RelayId(1)],
            vtm_count: 3,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 9000,
            zone_width_mm: None,
            real_shelf_count: 10,
            slots: None,
        },
    ];
    let assignments = assign_vtm_zones(&zones);
    // Zone 1 gets 2 VTMs, zone 2 gets 3 VTMs = 5 total.
    assert_eq!(assignments.len(), 5);
    assert_eq!(assignments.iter().filter(|a| a.contains(&ZoneId(1))).count(), 2);
    assert_eq!(assignments.iter().filter(|a| a.contains(&ZoneId(2))).count(), 3);
    for a in &assignments {
        assert_eq!(a.len(), 1);
    }
}

#[test]
fn vtm_zone_assignment_skips_zones_with_zero_vtm_count() {
    use desgraph::RelayId;
    let zones = vec![
        Zone {
            id: ZoneId(1),
            name: None,
            cells: vec![CellId(10)],
            relays: vec![RelayId(0)],
            vtm_count: 2,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 9000,
            zone_width_mm: None,
            real_shelf_count: 10,
            slots: None,
        },
        Zone {
            id: ZoneId(2),
            name: None,
            cells: vec![CellId(20)],
            relays: vec![RelayId(1)],
            vtm_count: 0,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 9000,
            zone_width_mm: None,
            real_shelf_count: 10,
            slots: None,
        },
        Zone {
            id: ZoneId(3),
            name: None,
            cells: vec![CellId(30)],
            relays: vec![],
            vtm_count: 2,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 9000,
            zone_width_mm: None,
            real_shelf_count: 10,
            slots: None,
        },
    ];
    let assignments = assign_vtm_zones(&zones);
    // Zone 2 has vtm_count=0, zone 3 has no relays — only zone 1's 2 VTMs count.
    assert_eq!(assignments.len(), 2);
    assert!(assignments.iter().all(|a| a.contains(&ZoneId(1))));
}

#[test]
fn zero_vtms_produces_empty_assignments() {
    use desgraph::RelayId;
    let zones = vec![Zone {
        id: ZoneId(1),
        name: None,
        cells: vec![],
        relays: vec![RelayId(0)],
        vtm_count: 0,
        holds: Default::default(),
        vtm_configuration: None,
        zone_height_mm: 9000,
        zone_width_mm: None,
        real_shelf_count: 10,
        slots: None,
    }];
    let assignments = assign_vtm_zones(&zones);
    assert!(assignments.is_empty());
}
