//! Scenario task lists govern real work: root selection, task parameters, and
//! deployment boundaries are checked against runnable fixtures.

mod common;

use std::path::Path;
use std::sync::Arc;

use desgraph::StationType;
use dessim::SimTime;
use dessim::composition::{self, Composition, Definition, PickTaskConfig};
use dessim::config::PickSequencing;
use dessim::events::SimEvent;
use dessim::model::ids::ToteId;
use dessim::model::{BotKind, BotService, InventoryService, RelayService};
use dessim::robots::RobotCatalog;
use dessim::runner::resolve_scenario_with_racking;
use dessim::scenario::{InlineOrder, InlineOrderLine, InlineScenario, InlineSku};
use dessim::simtime::SimOffset;
use dessim::{FidelityMode, ScenarioConfig, SimConfig, StopReason, apply_cli_settings};
use native_modules as _;

#[test]
fn unused_putaway_topology_does_not_compute_supply_policy() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
    }
    config.composition = Composition::Inline(Definition { tasks: vec![], task_defaults: Default::default() });
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    data.station_data[0].station_type = StationType::Putaway;
    config.putaway.direct_bias_secs = Some(f64::NAN);
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (_, reports, bias) = composition::build(&mut data, &config, pathfinder, None).unwrap();
    assert_eq!(bias, SimOffset::ZERO);
    assert!(reports.existing::<InventoryService>().is_none());
    assert!(reports.existing::<RelayService>().is_none());
}

#[test]
fn task_selection_and_root_order_govern_a_real_pick_run() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
    }
    config.fidelity = FidelityMode::Ghost;
    config.num_htms = 1;
    let (events, result, _) = common::run_world_capture_all(&world, &scenario, config.clone());
    assert_eq!(result.total_picks, 20);
    assert_eq!(result.total_orders_completed, 20);
    let cycles = |events: Vec<SimEvent>| {
        events
            .into_iter()
            .filter(|event| matches!(event, SimEvent::CycleComplete(_)))
            .map(|event| format!("{event:?}"))
            .collect::<Vec<_>>()
    };
    let expected_cycles = cycles(events);

    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.reverse();
    // An inline list uses the same factories, without a configuration task
    // whose scheduling order could become a hidden dependency.
    config.composition = serde_json::from_value(serde_json::json!({ "tasks": definition.tasks })).unwrap();
    let (events, result, _) = common::run_world_capture_all(&world, &scenario, config.clone());
    assert_eq!(result.total_picks, 20);
    assert_eq!(cycles(events), expected_cycles, "root ordering changed physical pick timing");

    config.composition = Composition::Inline(Definition { tasks: vec![], task_defaults: Default::default() });
    let (events, result, _) = common::run_world_capture_all(&world, &scenario, config);
    assert_eq!(result.total_picks, 0);
    assert!(!events.iter().any(|e| matches!(e, SimEvent::CycleComplete(_))));
}

#[test]
fn walmart_is_explicit_and_cannot_fall_back_to_cencora() {
    let (world, scenario, mut config) = common::load_world_spec("minimal_easyloop_spec.json");
    assert_eq!(config.composition, Composition::Named("walmart".into()));
    config.composition = Composition::default();
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let error = composition::build(&mut data, &config, pathfinder, None).err().expect("wrong composition refused");
    assert!(error.to_string().contains("select composition \"walmart\""));
}

#[test]
fn unknown_tasks_and_incomplete_wiring_are_refused() {
    assert!(serde_json::from_str::<Composition>(r#"{"tasks":[{"type":"PickDemnad"}]}"#).is_err());
    assert!(serde_json::from_str::<Composition>(r#"{"tasks":[{"type":"PickDemand","typo":1}]}"#).is_err());
    assert!(serde_json::from_str::<Composition>(r#"{"tasks":[{"type":"PickDemand","config":{"typo":1}}]}"#).is_err());
    assert!(serde_json::from_str::<Composition>(r#"{"tasks":[{"type":"ConfigureTtp","close_out":true}]}"#).is_err());
    for json in [
        r#"{"tasks":[{"type":"LoopDeposit"}]}"#,
        r#"{"tasks":[{"type":"FleetConservation"},{"type":"FleetConservation"}]}"#,
    ] {
        let composition: Composition = serde_json::from_str(json).unwrap();
        assert!(composition.resolve().is_err(), "accepted {json}");
    }
    assert!(Composition::Named("unknown".into()).resolve().is_err());
}

#[test]
fn saved_config_and_cli_preserve_the_selected_composition() {
    let mut config = SimConfig::default();
    assert_eq!(config.composition, Composition::Named("cencora".into()));
    let (no_figures, world) = (RobotCatalog::default(), Path::new("a world"));
    apply_cli_settings(&mut config, &["composition=walmart".into()], &no_figures, world).unwrap();
    let restored: SimConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(restored.composition, config.composition);
    assert!(apply_cli_settings(&mut config, &["composition=walmat".into()], &no_figures, world).is_err());
}

#[test]
fn task_parameters_override_legacy_settings_and_reach_physical_picks() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
    }
    config.fidelity = FidelityMode::Ghost;
    config.num_htms = 1;
    config.pick.embargo_secs = 60.0;
    config.pick.sequencing = PickSequencing::Sequenced;
    config.queue_factor = 3.0;
    config.order.max_inflight_picks_override = Some(2);
    let (expected, expected_result, _) = common::run_world_capture_all(&world, &scenario, config.clone());
    let cycles = |events: Vec<SimEvent>| {
        events
            .into_iter()
            .filter_map(|event| match event {
                SimEvent::CycleComplete(cycle) => Some((cycle.sim_time_ms, format!("{cycle:?}"))),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let expected = cycles(expected);
    assert_eq!(expected_result.total_picks, 20);
    assert!(expected.iter().all(|(at, _)| *at >= 60_000));

    let mut definition = config.composition.resolve().unwrap();
    for task in &mut definition.tasks {
        if task.name() == "PickDemand" {
            task.config = serde_json::to_value(PickTaskConfig {
                embargo_secs: Some(60.0),
                sequencing: Some(PickSequencing::Sequenced),
                queue_factor: Some(3.0),
                max_inflight_picks: Some(2),
                pace_to_station: Some(false),
            })
            .unwrap();
        }
    }
    config.composition = Composition::Inline(definition);
    config.pick.embargo_secs = 0.0;
    config.pick.sequencing = PickSequencing::AllTotes;
    config.pick.pace_to_station = true;
    config.queue_factor = 1.0;
    config.order.max_inflight_picks_override = None;
    let restored: SimConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(restored.composition, config.composition);
    let (events, result, _) = common::run_world_capture_all(&world, &scenario, restored);
    assert_eq!(result.total_picks, 20);
    assert_eq!(cycles(events), expected, "task parameters did not reproduce the same physical work");

    let mut definition = config.composition.resolve().unwrap();
    definition.task_defaults = serde_json::from_value(serde_json::json!({
        "PickDemand": { "embargo_secs": 60, "sequencing": "sequenced", "queue_factor": 99,
                        "max_inflight_picks": 2, "pace_to_station": false }
    }))
    .unwrap();
    for task in &mut definition.tasks {
        if task.name() == "PickDemand" {
            task.config = serde_json::json!({"queue_factor": 3});
        }
    }
    config.composition = Composition::Inline(definition);
    let (events, result, _) = common::run_world_capture_all(&world, &scenario, config);
    assert_eq!(result.total_picks, 20);
    assert_eq!(cycles(events), expected, "defaults and instance overrides changed physical pick timing");
}

/// A relay pickup takes its HTM model's catalog pickup figure,
/// late pickup children included, and a composition cannot state one of its
/// own: a pickup default fails the run rather than moving it.
#[test]
fn ordinary_pickup_children_take_the_catalog_pickup_figure() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
    }
    config.fidelity = FidelityMode::Ghost;
    config.num_htms = 1;
    let cycles = |events: Vec<SimEvent>| {
        events
            .into_iter()
            .filter_map(|event| match event {
                SimEvent::CycleComplete(cycle) => Some(format!("{cycle:?}")),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let with_pickup = |seconds| {
        let (mut world, _, _) = common::load_world_spec("minimal_pick_only_spec.json");
        let mut catalog = (*world.robot_catalog).clone();
        assert!(!catalog.htm.is_empty());
        for model in catalog.htm.values_mut() {
            model.pickup_secs = seconds;
        }
        world.robot_catalog = Arc::new(catalog);
        world.racking.robot_deployment.catalog = world.robot_catalog.clone();
        world
    };
    config.htm.pickup_secs = 99.0;
    let (events, result, _) = common::run_world_capture_all(&with_pickup(7.0), &scenario, config.clone());
    assert_eq!(result.total_picks, 20);
    let slow = cycles(events);
    let (events, result, _) = common::run_world_capture_all(&with_pickup(2.0), &scenario, config.clone());
    assert_eq!(result.total_picks, 20);
    assert_ne!(cycles(events), slow, "late pickup children ignored their catalog pickup figure");

    let mut definition = config.composition.resolve().unwrap();
    definition.task_defaults = serde_json::from_value(serde_json::json!({
        "HtmPickup": { "pickup_secs": 7 }
    }))
    .unwrap();
    config.composition = Composition::Inline(definition);
    let refused = common::try_run_world(&world, &scenario, config).err().expect("a composition stated a pickup time");
    assert!(refused.contains("HtmPickup accepts no configuration"), "{refused}");
}

#[test]
fn invalid_task_parameters_fail_before_startup() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
    }
    let data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    for json in [
        r#"{"tasks":[{"type":"PickDemand","config":{"embargo_secs":-1}}]}"#,
        r#"{"tasks":[{"type":"PickDemand","config":{"max_inflight_picks":0}}]}"#,
        r#"{"tasks":[{"type":"PutawayInduction","config":{"max_presentations_per_hour":0}}]}"#,
        r#"{"tasks":[{"type":"OrderCloseOut","config":{"buffer_dwell_secs":-1}}]}"#,
    ] {
        config.composition = serde_json::from_str(json).unwrap();
        let error = composition::build(&mut data.clone(), &config, pathfinder.clone(), None)
            .err()
            .expect("invalid parameter refused");
        assert!(error.to_string().contains("composition"), "{error}");
    }
    config.barrier.enabled = false;
    config.composition =
        serde_json::from_str(r#"{"tasks":[{"type":"PickDemand","config":{"embargo_secs":1}}]}"#).unwrap();
    let error =
        composition::build(&mut data.clone(), &config, pathfinder, None).err().expect("embargo without clock refused");
    assert!(error.to_string().contains("barrier.enabled"));
}

#[test]
fn completion_dependencies_refuse_missing_cycles_and_perpetual_roots() {
    for json in [
        r#"{"tasks":[{"type":"PickDemand","after":["LoadOrders"]}]}"#,
        r#"{"tasks":[{"type":"LoadOrders","after":["LoadOrders"]}]}"#,
        r#"{"tasks":[{"type":"LoadOrders","after":["FillInventory"]},{"type":"FillInventory","after":["LoadOrders"]}]}"#,
        r#"{"tasks":[{"type":"LoadOrders","after":["FleetConservation"]},{"type":"FleetConservation"}]}"#,
    ] {
        let composition: Composition = serde_json::from_str(json).unwrap();
        assert!(composition.resolve().is_err(), "accepted {json}");
    }
}

#[test]
fn inventory_population_is_work_owned_by_the_setup_root() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
    }
    config.fidelity = FidelityMode::Ghost;
    config.num_htms = 1;
    config.composition = serde_json::from_value(serde_json::json!({
        "tasks": [{"type": "FillInventory"}, {"type": "LoadOrders", "after": ["FillInventory"]}]
    }))
    .unwrap();
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, None).unwrap();
    let inventory = reports.existing::<InventoryService>().expect("FillInventory resolved its service");
    assert_eq!(inventory.tote_count(), 0, "construction populated inventory");
    assert_eq!(reports.existing::<BotService>().unwrap().count_kind(BotKind::Htm), 0, "construction placed robots");
    assert!(reports.existing::<RelayService>().is_none(), "assembly constructed relays");
    scheduler.run().unwrap();
    assert_eq!(inventory.tote_count(), data.tote_placements.len());
    for (&(sku, space), &quantity) in &data.inventory {
        let tote = inventory.tote(ToteId(space.0)).unwrap();
        assert_eq!(tote.sku_qty_at(sku, SimTime::ZERO), quantity);
    }
}

/// An order with nothing to pick is not work. It must not wait in the ready
/// queue forever and hold the run open after the real work completes.
#[test]
fn an_order_with_nothing_to_pick_does_not_hold_the_run_open() {
    let (world, _, mut config) = common::load_world_spec("minimal_pick_only_spec.json");
    let line = |quantity| InlineOrderLine { sku: "A".into(), quantity };
    let scenario = ScenarioConfig::Inline(InlineScenario {
        skus: vec![InlineSku { sku: "A".into(), weight_kg: 1.0, inventory_units: 5 }],
        orders: vec![InlineOrder { lines: vec![line(1)] }, InlineOrder { lines: vec![line(0)] }],
        units_per_tote: 1,
    });
    config.fidelity = FidelityMode::Ghost;
    config.num_htms = 1;
    // Without the fix the run never finishes; the timeout turns that into a failure.
    config.wall_timeout_secs = Some(30.0);
    let (_, result, _) = common::run_world_capture_all(&world, &scenario, config);
    assert!(matches!(result.stop_reason, StopReason::Complete), "stopped by {:?}", result.stop_reason);
    assert_eq!(result.total_picks, 1);
}
