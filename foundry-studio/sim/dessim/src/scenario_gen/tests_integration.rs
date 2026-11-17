//! Integration tests: end-to-end scenario generation for relay and RTP systems.

use std::collections::{BTreeMap, BTreeSet};

use desgraph::{CellId, CellKind};
use desmap::{GeneratorConfig, StationLayout, SystemType, WarehouseDesign};

use desmap::design::ZoneDef;

use super::types::ScenarioData;

use crate::config::SimConfig;
use crate::model::{OrderId, OrderLine, OrderSpec, SkuId, StorageSpaceId};
use crate::model::{RackId, ToteId};
use crate::scenario::GeneratedScenario;

use super::validation::validate_inventory_completability;

/// What `pipeline::generate_scenario` used to do, kept HERE as a test helper.
///
/// The production entry point took a v3 `WarehouseDesign` and retired with the
/// v3 runtime; these tests are about scenario GENERATION (SKU catalogue, order
/// distribution, inventory placement, VTM zone assignment, id sequencing) which
/// is live code reached from `resolve_scenario_with_racking` on every world
/// map. A v3 design is just the cheapest way to hand them a map, so the
/// three-line wrapper lives in the tests instead of resurrecting API nothing
/// ships.
fn generate_scenario(
    design: &WarehouseDesign,
    scenario: &GeneratedScenario,
    config: &SimConfig,
) -> Result<ScenarioData, String> {
    let vmax = design
        .zones
        .iter()
        .find_map(|z| match z {
            ZoneDef::Relay(relay) => Some(relay.effective_vertical_tote_capacity()),
            _ => None,
        })
        .unwrap_or(1);
    let map = design.compile()?;
    super::generate_scenario_from_map(map, scenario, config, vmax)
}

fn default_relay_design() -> WarehouseDesign {
    let config = GeneratorConfig {
        // Terminal relay blocks need an outer HTM approach lane. Twenty
        // columns provide only 768 VTC8 slots; 25 is the minimum width that
        // preserves this fixture's 1,000-unit inventory assertion on
        // physically serviceable racks.
        grid_width: 25,
        grid_depth: 20,
        cell_size_mm: 1000.0,
        station_count: 2,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 2,
        rows_per_block: 4,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Relay,
    };
    let map = desmap::generator::generate(&config).unwrap();
    desmap::design::extract(&map).unwrap()
}

fn default_rtp_design() -> WarehouseDesign {
    let config = GeneratorConfig {
        grid_width: 20,
        grid_depth: 20,
        cell_size_mm: 1000.0,
        station_count: 2,
        station_layout: StationLayout::SingleBottom,
        highways_per_edge: 2,
        rows_per_block: 4,
        rack_columns: 2,
        cross_aisle_width: 1,
        system_type: SystemType::Rtp,
    };
    let map = desmap::generator::generate(&config).unwrap();
    desmap::design::extract(&map).unwrap()
}

fn small_scenario() -> GeneratedScenario {
    GeneratedScenario {
        num_skus: 20,
        total_inventory_units: 200,
        num_orders: 50,
        total_order_picks: 150,
        popular_skus_pct: 0.2,
        inventory_popular_share: 0.8,
        order_popular_share: 0.7,
        ..Default::default()
    }
}

fn small_config(num_htms: usize, num_rtps: usize) -> SimConfig {
    SimConfig { num_htms, num_rtps, ..SimConfig::default() }
}

#[test]
fn generate_relay_scenario_produces_valid_data() {
    let data = generate_scenario(&default_relay_design(), &small_scenario(), &small_config(2, 0)).unwrap();

    assert_eq!(data.skus.len(), 20);
    assert_eq!(data.skus.iter().filter(|s| s.is_popular).count(), 4);
    assert_eq!(data.skus.iter().map(|s| s.inventory_units).sum::<u32>(), 200);

    assert!(data.orders.len() <= 50);
    let total_picks: u32 = data.orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity).sum();
    assert!(total_picks <= 150 && total_picks > 0);

    assert!(!data.inventory.is_empty());
    assert!(!data.tote_placements.is_empty());
    assert!(data.rack_placements.is_empty());
    assert!(!data.zone_data.is_empty());
    assert!(!data.relay_data.is_empty());
    assert_eq!(data.station_data.len(), 2);

    assert!(data.vtm_zone_assignments.len() >= 4);
    for a in &data.vtm_zone_assignments {
        assert_eq!(a.len(), 1);
    }
    assert_eq!(data.vtm_initial_positions.len(), data.vtm_zone_assignments.len());
    for &pos in &data.vtm_initial_positions {
        assert!(data.map.graph.cell(pos.anchor()).is_some());
    }
}

#[test]
fn generate_rtp_scenario_produces_valid_data() {
    let data = generate_scenario(&default_rtp_design(), &small_scenario(), &small_config(0, 3)).unwrap();

    assert!(data.tote_placements.is_empty());
    assert!(!data.rack_placements.is_empty());
    for rack in &data.rack_placements {
        assert_eq!(data.map.graph.cell(rack.pos).unwrap().kind, CellKind::Storage);
    }
    assert_eq!(data.vtm_zone_assignments.len(), 0);
}

#[test]
fn tote_ids_are_sequential() {
    let data = generate_scenario(&default_relay_design(), &small_scenario(), &small_config(1, 0)).unwrap();
    for (i, tp) in data.tote_placements.iter().enumerate() {
        assert_eq!(tp.tote_id, ToteId(i as u32));
    }
}

#[test]
fn rack_ids_are_sequential() {
    let data = generate_scenario(&default_rtp_design(), &small_scenario(), &small_config(0, 2)).unwrap();
    for (i, rp) in data.rack_placements.iter().enumerate() {
        assert_eq!(rp.rack_id, RackId(i as u32));
    }
}

#[test]
fn all_inventory_units_are_placed() {
    let scenario = GeneratedScenario {
        num_skus: 10,
        total_inventory_units: 100,
        num_orders: 10,
        total_order_picks: 30,
        ..GeneratedScenario::default()
    };
    let data = generate_scenario(&default_relay_design(), &scenario, &small_config(1, 0)).unwrap();
    assert_eq!(data.inventory.values().sum::<u32>(), 100);
}

#[test]
fn htm_initial_positions_are_unique() {
    let config = small_config(5, 0);
    let data = generate_scenario(&default_relay_design(), &small_scenario(), &config).unwrap();
    assert_eq!(data.htm_initial_positions.len(), 5);
    let unique: BTreeSet<_> = data.htm_initial_positions.iter().collect();
    assert_eq!(unique.len(), 5, "HTM initial positions must be unique");
}

#[test]
fn completability_check_catches_insufficient_inventory() {
    let orders = vec![OrderSpec::unstated(OrderId(0), vec![OrderLine { sku: SkuId(0), quantity: 10 }])];
    let mut inventory = BTreeMap::new();
    inventory.insert((SkuId(0), StorageSpaceId(0)), 5);

    let result = validate_inventory_completability(&orders, &inventory);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("Inventory cannot fulfill all orders"));
    assert!(msg.contains("waive_completability_check"));
}

#[test]
fn completability_check_passes_with_sufficient_inventory() {
    let scenario = GeneratedScenario {
        num_skus: 10,
        total_inventory_units: 1000,
        num_orders: 10,
        total_order_picks: 30,
        popular_skus_pct: 0.2,
        inventory_popular_share: 0.8,
        order_popular_share: 0.7,
        ..Default::default()
    };
    let config = SimConfig { num_htms: 1, waive_completability_check: false, ..SimConfig::default() };
    let result = generate_scenario(&default_relay_design(), &scenario, &config);
    assert!(result.is_ok(), "should pass completability check: {:?}", result.err());
}

#[test]
fn inventory_aware_orders_pass_completability() {
    let config = SimConfig { num_htms: 1, waive_completability_check: false, ..SimConfig::default() };
    let result = generate_scenario(&default_relay_design(), &small_scenario(), &config);
    assert!(result.is_ok(), "inventory-aware orders should pass: {:?}", result.err());
}
