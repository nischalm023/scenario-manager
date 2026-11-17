//! Within `scenario_gen`, `pipeline` is the ORDERING of the generation
//! steps: it is the only module that knows topology extraction must precede
//! inventory placement, which must precede VTM assignment and the
//! completability check, and it assembles their outputs into the one
//! `ScenarioData` the run starts from. Any step's refusal is returned to the
//! launcher, so an unbuildable scenario never reaches the clock. The first
//! step checks the run's configuration, because every run's scenario is built
//! here once its settings are final.

use std::collections::BTreeMap;

use desgraph::{CellId, WorldPos};
use desmap::{BlockSide, WarehouseMap};

use crate::composition::InventoryTaskConfig;
use crate::config::SimConfig;
use crate::model::OrderSpec;
use crate::model::RackId;
use crate::model::StorageSpaceId;
use crate::model::relay::config::RelayZones;
use crate::scenario::{GeneratedScenario, InventoryDistribution};

use super::demand_pool::DemandPool;
use super::inventory::{generate_relay_inventory, generate_rtp_inventory};
use super::orders::{generate_orders, generate_skus};
use super::topology::*;
use super::types::*;
use super::validation::validate_inventory_completability;
use super::vtm_placement::{assign_vtm_zones, compute_vtm_positions};

/// Generate a complete scenario from an already-compiled `WarehouseMap`.
pub fn generate_scenario_from_map(
    map: WarehouseMap,
    scenario: &GeneratedScenario,
    config: &SimConfig,
    vertical_tote_capacity: u32,
) -> Result<ScenarioData, String> {
    let racking = racking_from_map(&map, vertical_tote_capacity)?;
    generate_scenario_from_map_with_racking(map, racking, scenario, config)
}

/// The same, for a map that states its own storage layout instead of leaving
/// it to be read back out of grid geometry.
pub fn generate_scenario_from_map_with_racking(
    map: WarehouseMap,
    racking: Racking,
    scenario: &GeneratedScenario,
    config: &SimConfig,
) -> Result<ScenarioData, String> {
    let skus = generate_skus(scenario);

    let order_result = generate_orders(scenario, &skus);
    if order_result.picks_dropped > 0 {
        eprintln!(
            "scenario_gen: {} picks dropped due to inventory exhaustion (requested {}, generated {})",
            order_result.picks_dropped,
            scenario.total_order_picks,
            scenario.total_order_picks - order_result.picks_dropped,
        );
    }
    if order_result.popular_exhausted_early {
        eprintln!("scenario_gen: popular SKU inventory was exhausted before all popular picks were assigned");
    }

    let units_per_tote = scenario.units_per_tote.max(1);
    build_scenario_data(
        map,
        racking,
        skus,
        order_result.orders,
        None,
        config,
        units_per_tote,
        InventoryDistribution::SingleSku,
        scenario.order_totes,
    )
}

/// Generate a scenario using pre-loaded SKUs and orders (from Java CSV files).
///
/// Steps 3/4 (SKU/order generation) are replaced by the loaded data.
pub fn generate_scenario_with_loaded_data(
    map: WarehouseMap,
    skus: Vec<SkuInfo>,
    orders: Vec<OrderSpec>,
    config: &SimConfig,
    vertical_tote_capacity: u32,
    units_per_tote: u32,
) -> Result<ScenarioData, String> {
    let racking = racking_from_map(&map, vertical_tote_capacity)?;
    build_scenario_data(
        map,
        racking,
        skus,
        orders,
        None,
        config,
        units_per_tote.max(1),
        InventoryDistribution::SingleSku,
        0,
    )
}

/// Generate a bulk-source scenario from an already-compiled map. Orders
/// are not provided; the order builder emits them during simulation
/// drawing from `pool`. Pallet bounds travel inside the pool itself.
pub fn generate_scenario_with_bulk(
    map: WarehouseMap,
    skus: Vec<SkuInfo>,
    pool: DemandPool,
    config: &SimConfig,
    vertical_tote_capacity: u32,
    units_per_tote: u32,
) -> Result<ScenarioData, String> {
    let racking = racking_from_map(&map, vertical_tote_capacity)?;
    build_scenario_data(
        map,
        racking,
        skus,
        Vec::new(),
        Some(pool),
        config,
        units_per_tote.max(1),
        InventoryDistribution::SingleSku,
        0,
    )
}

/// Shared pipeline: topology extraction, inventory placement, VTM assignment,
/// spatial positions, completability check, and ScenarioData assembly.
///
/// `demand_pool` is `Some` only for `ScenarioConfig::BulkSource`; in that
/// case `orders` is expected to be empty and the [`OrderBuilder`] will
/// emit orders during simulation from the pool.
pub fn build_scenario_data(
    map: WarehouseMap,
    racking: Racking,
    skus: Vec<SkuInfo>,
    mut orders: Vec<OrderSpec>,
    demand_pool: Option<DemandPool>,
    config: &SimConfig,
    units_per_tote: u32,
    distribution: InventoryDistribution,
    order_totes: u32,
) -> Result<ScenarioData, String> {
    // Spec runs, replayed recordings, the desvis GUI and the tests all build
    // their scenario here after the spec's settings and any `--settings`
    // overlay have landed, so a figure the simulation cannot use is refused
    // here whichever source stated it, before the pathfinder divides by it.
    config.validate().map_err(|error| error.to_string())?;
    let htm_initial_positions = super::htm_placement::initial_positions(&map, config.num_htms, config.rng_seed)?;
    let vertical_tote_capacity = racking.vertical_tote_capacity;
    if let Some(max) = config.max_orders {
        if max < orders.len() {
            eprintln!("max_orders={}: truncating {} → {} orders", max, orders.len(), max);
            orders.truncate(max);
        }
    }
    let zone_data = extract_zone_data(&map);
    let relay_data = extract_relay_data(&map);
    let station_data = extract_station_data(&map);
    let zone_storage = extract_zone_storage(&map);

    let is_relay = !map.relays.is_empty();
    let storage_cells = collect_storage_cells(&map, is_relay);
    if racking.columns.is_empty() {
        return Err("warehouse has no usable storage cells".into());
    }

    let definition = config.composition.resolve().map_err(|error| error.to_string())?;
    let initial: InventoryTaskConfig = definition.parameters("FillInventory").map_err(|error| error.to_string())?;
    let preload_fraction = initial.preload_fraction.unwrap_or(if config.putaway.enabled { 0.0 } else { 1.0 });
    let incoming_stock = ["LoadOrders", "PutawaySupplyDemand", "PutawayInduction"]
        .iter()
        .all(|name| definition.tasks.iter().any(|task| task.name() == *name));
    let rack_columns = &racking.columns;
    let relay_exchange_pos = racking.relay_exchange_pos.clone();
    let rack_world_pos: BTreeMap<RackId, WorldPos> =
        rack_columns.iter().enumerate().filter_map(|(i, rack)| Some((RackId(i as u32), rack.world_pos?))).collect();
    let rack_floor_m: BTreeMap<RackId, f64> =
        rack_columns.iter().enumerate().filter_map(|(i, rack)| Some((RackId(i as u32), rack.floor_m?))).collect();
    let rack_side: BTreeMap<RackId, BlockSide> =
        rack_columns.iter().enumerate().filter_map(|(i, rack)| Some((RackId(i as u32), rack.side?))).collect();
    let rack_use: BTreeMap<RackId, RackUse> =
        rack_columns.iter().enumerate().map(|(i, rack)| (RackId(i as u32), rack.uses)).collect();
    let (inventory, tote_placements, rack_cells, rack_placements) = if is_relay {
        let mut inv_skus = skus.clone();
        for sku in &mut inv_skus {
            sku.inventory_units = (f64::from(sku.inventory_units) * preload_fraction).ceil() as u32;
        }
        let (inv, totes, rc) = generate_relay_inventory(
            &inv_skus,
            rack_columns,
            vertical_tote_capacity,
            units_per_tote,
            distribution,
            order_totes,
            preload_fraction,
        )?;
        (inv, totes, rc, Vec::new())
    } else {
        if preload_fraction != 1.0 {
            return Err("partial/empty preload requires tote storage".into());
        }
        if distribution == InventoryDistribution::Even {
            return Err("even inventory preload requires tote storage".into());
        }
        let (inv, racks) = generate_rtp_inventory(&skus, &storage_cells, &map.stations, units_per_tote.max(1));
        (inv, Vec::new(), BTreeMap::<RackId, CellId>::new(), racks)
    };

    let vtm_zone_assignments = assign_vtm_zones(&map.zones);
    let vtm_initial_positions = compute_vtm_positions(&vtm_zone_assignments, &map, &racking.vtm_start_positions);
    let robot_models = racking.robot_deployment.resolve(config, htm_initial_positions.len(), &vtm_zone_assignments)?;
    let rack_to_vtm_access = build_rack_to_vtm_access(&map)?;

    // Skip pre-order completability check for bulk-source scenarios. In
    // bulk mode there are no pre-formed orders to validate; the order
    // builder only emits orders whose SKUs already exist in rack, so the
    // failure mode the check guards against ("order needs SKU X but
    // inventory will never reach qty Y") can't apply at order admission.
    let in_bulk_mode = demand_pool.is_some();
    if !config.waive_completability_check && !in_bulk_mode {
        if incoming_stock {
            // The inbound queue contains exactly what was not preloaded. Check
            // final supply without claiming those units are already available.
            let total_supply = skus.iter().map(|sku| ((sku.id, StorageSpaceId(0)), sku.inventory_units)).collect();
            validate_inventory_completability(&orders, &total_supply)?;
        } else {
            validate_inventory_completability(&orders, &inventory)?;
        }
    }

    let mut data = ScenarioData {
        robot_models,
        map,
        skus,
        orders,
        inventory,
        zone_data,
        relay_data,
        station_data,
        zone_storage,
        tote_placements,
        rack_cells,
        rack_world_pos,
        rack_floor_m,
        relay_exchange_pos,
        rack_side,
        rack_use,
        board_floors_m: racking.board_floors_m,
        rack_placements,
        vtm_zone_assignments,
        vtm_initial_positions,
        htm_initial_positions,
        rack_to_vtm_access,
        vertical_tote_capacity: vertical_tote_capacity.min(u16::MAX as u32) as u16,
        relay_zones: Default::default(),
        demand_pool,
    };
    data.relay_zones = RelayZones::from_scenario(&data).map_err(|e| e.to_string())?;
    Ok(data)
}
