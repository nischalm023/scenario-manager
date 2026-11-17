//! Inbound stock must complement the preload and still finish its physical
//! rack return while EasyLoop fulfillment competes for the same robot fleet.

mod common;

use dessim::composition::Composition;
use dessim::events::{CycleKind, SimEvent, ToteSource, VtmCycleDirection};
use dessim::runner::resolve_scenario_with_racking;
use dessim::{FidelityMode, ScenarioConfig, StopReason};
use native_modules as _;
use serde_json::json;

#[test]
fn partial_and_empty_starts_induct_only_missing_stock_and_complete_shared_flows() {
    for (fraction, sequence) in
        [(0.5, "sku_demand"), (0.0, "sku_demand"), (0.5, "order_release"), (0.0, "order_release")]
    {
        let (world, mut scenario, mut config) = common::load_world_spec("minimal_easyloop_flow_spec.json");
        let ScenarioConfig::Generated(generated) = &mut scenario else { panic!("generated scenario") };
        generated.total_inventory_units = 72;
        generated.units_per_tote = 6;
        config.num_htms = 10;
        config.fidelity = FidelityMode::CollisionDes;
        config.wall_timeout_secs = Some(60.0);
        config.composition = serde_json::from_value::<Composition>(json!({"tasks": [
            {"type":"InitializeFleet"},
            {"type":"FillInventory", "after":["InitializeFleet"], "config":{"preload_fraction":fraction}},
            {"type":"LoadOrders", "after":["FillInventory"]},
            {"type":"FulfillOrders", "after":["LoadOrders"]},
            {"type":"PutawaySupplyDemand", "after":["LoadOrders"]},
            {"type":"PutawayInduction", "after":["LoadOrders"], "config":{"max_presentations_per_hour":60, "units_per_tote":6,"sequence":sequence}}
        ]})).unwrap();
        let data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config).unwrap();
        let initial: u64 = data.inventory.values().map(|&qty| u64::from(qty)).sum();
        let total: u64 = data.skus.iter().map(|sku| u64::from(sku.inventory_units)).sum();
        assert!(initial < total);
        let (events, result, metrics) = common::run_world_capture_all(&world, &scenario, config);
        assert!(matches!(result.stop_reason, StopReason::Complete), "fraction={fraction}: {}", result.stop_reason);
        assert_eq!(result.total_picks, 24);
        let fills: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::Putaway { .. }) => Some(cycle),
                _ => None,
            })
            .collect();
        let inducted: u64 = fills
            .iter()
            .map(|cycle| match cycle.kind {
                CycleKind::Putaway { qty, .. } => u64::from(qty),
                _ => unreachable!(),
            })
            .sum();
        assert_eq!(initial + inducted, total, "no duplicated or lost stock");
        assert!(fills.iter().any(|cycle| matches!(cycle.kind, CycleKind::Putaway { qty: 6, .. })));
        assert_eq!(metrics.putaway_log.iter().map(|row| u64::from(row.quantity)).sum::<u64>(), inducted);
        assert_eq!(metrics.cumulative_scalars.putaway_totes_stored, fills.len() as u64);
        for fill in &fills {
            assert_eq!(fill.dwell_ms, 60_000);
            let stores: Vec<_> = metrics
                .vtm_cycle_log
                .iter()
                .filter(|cycle| {
                    cycle.direction == VtmCycleDirection::Return
                        && cycle.tote_source == ToteSource::Putaway
                        && cycle.presentation_id == fill.presentation_id.map(|id| id.0)
                })
                .collect();
            assert_eq!(stores.len(), 1, "every inducted tote completes exactly one store");
            assert!(stores[0].sim_time_ms >= fill.sim_time_ms);
        }
    }
}
