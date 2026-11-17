//! A real putaway run must finish after its finite arrivals drain, with full
//! induction dwells in robot plans. Task parameters must preserve that timing.

mod common;
use dessim::composition::Composition;
use dessim::events::SimEvent;
use dessim::{FidelityMode, ScenarioConfig, StopReason};
use native_modules as _;

#[test]
fn induction_parameters_keep_supply_and_fill_timing_in_agreement() {
    let (world, mut scenario, mut config) = common::load_world_spec("minimal_putaway1_spec.json");
    if let ScenarioConfig::Generated(generated) = &mut scenario {
        generated.num_skus = 1;
        generated.total_inventory_units = 12;
    }
    config.fidelity = FidelityMode::Ghost;
    config.putaway.max_presentations_per_hour = 120.0;
    config.putaway.direct_bias_secs = Some(0.0);
    let definition: Composition = serde_json::from_str(
        r#"{"tasks":[
        {"type":"InitializeFleet"},
        {"type":"FillInventory","after":["InitializeFleet"]},
        {"type":"LoadOrders","after":["FillInventory"]},
        {"type":"PutawaySupplyDemand","after":["LoadOrders"]},
        {"type":"PutawayInduction","after":["LoadOrders"]}
    ]}"#,
    )
    .unwrap();
    config.composition = definition;
    let (expected, result, _) = common::run_world_capture_all(&world, &scenario, config.clone());
    assert!(matches!(&result.stop_reason, StopReason::Complete), "{}", result.stop_reason);
    let cycles = |events: Vec<SimEvent>| {
        events
            .into_iter()
            .filter_map(|event| match event {
                SimEvent::CycleComplete(cycle) => Some((cycle.dwell_ms, format!("{cycle:?}"))),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let expected = cycles(expected);
    assert_eq!(expected.len(), 12);
    assert!(expected.iter().all(|(dwell, _)| *dwell == 30_000));

    config.composition = serde_json::from_str(
        r#"{"tasks":[
        {"type":"InitializeFleet"},
        {"type":"FillInventory","after":["InitializeFleet"]},
        {"type":"LoadOrders","after":["FillInventory"]},
        {"type":"PutawaySupplyDemand","after":["LoadOrders"],"config":{"direct_bias_secs":0}},
        {"type":"PutawayInduction","after":["LoadOrders"],"config":{"max_presentations_per_hour":120}}
    ]}"#,
    )
    .unwrap();
    config.putaway.max_presentations_per_hour = 200.0;
    config.putaway.direct_bias_secs = Some(-10.0);
    let (events, result, _) = common::run_world_capture_all(&world, &scenario, config);
    assert!(matches!(&result.stop_reason, StopReason::Complete), "{}", result.stop_reason);
    assert_eq!(
        cycles(events),
        expected,
        "task parameters changed supply/fill timing relative to identical legacy settings"
    );
}
