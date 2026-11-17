//! The automatic preload uses the whole on-hand snapshot even for a small
//! order sample. This remains separate from a future tote-addressed mock-put file.

mod common;

use dessim::model::{SkuId, ToteDesignation};
use dessim::runner::resolve_scenario_with_racking;
use dessim::scenario::{InventoryDistribution, ScenarioConfig, StockPolicy};
use dessim::scenario_gen::RackUse;
use native_modules as _;
use rustc_hash::FxHashMap;

#[test]
fn full_onhand_is_conserved_and_evenly_distributed_independently_of_order_count() {
    let (world, mut scenario, mut config) = common::load_world_spec("walmart_dfw6_mixed_spec.json");
    let ScenarioConfig::SiteExtract(site) = &mut scenario else { panic!("expected site extracts") };
    site.stock_top_up_csv = None; // Preserve the original extract regression separately from demo additions.
    assert_eq!(site.stock, StockPolicy::AsExtracted);
    assert_eq!(site.inventory_distribution, InventoryDistribution::Even);
    let data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config).unwrap();
    assert_eq!(data.orders.len(), 500);
    assert_eq!(data.skus.len(), 174_997);
    let mut per_sku: FxHashMap<SkuId, u64> = FxHashMap::default();
    let mut per_tote = FxHashMap::default();
    let mut skus_per_tote = FxHashMap::default();
    for (&(sku, space), &quantity) in &data.inventory {
        *per_sku.entry(sku).or_default() += u64::from(quantity);
        *per_tote.entry(space.0).or_insert(0_u32) += quantity;
        *skus_per_tote.entry(space.0).or_insert(0_u32) += 1;
    }
    for sku in &data.skus {
        assert_eq!(per_sku[&sku.id], u64::from(sku.inventory_units), "exact source quantity for {:?}", sku.id);
    }
    assert_eq!(per_sku.values().sum::<u64>(), 1_397_589);
    assert_eq!(per_tote.len(), 162_060);
    assert_eq!(per_tote.values().filter(|&&qty| qty == 8).count(), 60_951);
    assert_eq!(per_tote.values().filter(|&&qty| qty == 9).count(), 101_109);
    assert!(skus_per_tote.values().any(|count| *count > 1));
    assert!(data.inventory.values().any(|qty| *qty > 1));
    for placement in &data.tote_placements {
        assert_eq!(per_tote.contains_key(&placement.tote_id.0), placement.designation == ToteDesignation::Inventory);
    }
    assert_eq!(data.rack_use.values().filter(|&&uses| uses == RackUse::Inventory).count(), 162_060);
    config.max_orders = Some(0);
    let no_orders = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    assert!(no_orders.orders.is_empty());
    assert_eq!(no_orders.inventory, data.inventory, "limiting demand never changes the warehouse preload");
}

#[test]
fn main_spec_top_up_is_exactly_the_source_deficit_and_full_preload_passes_validation() {
    let (world, scenario, config) = common::load_world_spec("walmart_dfw6_spec.json");
    assert!(config.max_orders.is_none());
    assert!(!config.waive_completability_check);
    let ScenarioConfig::SiteExtract(site) = &scenario else { panic!("expected site extracts") };
    assert!(site.stock_top_up_csv.is_some());
    let mut original = site.clone();
    original.stock_top_up_csv = None;
    let mut source_config = config.clone();
    source_config.max_orders = Some(0);
    let source_data = resolve_scenario_with_racking(
        world.map.clone(),
        world.racking.clone(),
        &ScenarioConfig::SiteExtract(original),
        &source_config,
    )
    .unwrap();
    let data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    assert_eq!(data.orders.len(), 22_160);
    let mut demand: FxHashMap<SkuId, u32> = FxHashMap::default();
    for order in &data.orders {
        for line in &order.lines {
            *demand.entry(line.sku).or_default() += line.quantity;
        }
    }
    let mut added_units = 0;
    let mut added_skus = 0;
    for (old, new) in source_data.skus.iter().zip(&data.skus) {
        let deficit = demand.get(&old.id).copied().unwrap_or(0).saturating_sub(old.inventory_units);
        assert_eq!(new.inventory_units, old.inventory_units + deficit);
        added_units += deficit;
        added_skus += usize::from(deficit > 0);
    }
    assert_eq!((added_units, added_skus), (43, 40));
    assert_eq!(data.inventory.values().map(|&n| u64::from(n)).sum::<u64>(), 1_397_632);
}
