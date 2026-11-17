use crate::model::BotService;
use crate::model::relay::RelayZones;
use crate::robots::runtime::RuntimeRobots;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use super::*;
use crate::model::reservation::ReservationTable;
use crate::model::reservation::oracle::TableOracle;
use crate::pathfinding::{FHSippPathfinder, holding_cells};
use crate::runner::resolve_scenario_with_racking;
use crate::world_spec;

fn fixture(edit: impl FnOnce(&mut ScenarioData, &mut SimConfig)) -> (ScenarioData, SimConfig, Navigation) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop_flow_spec.json");
    let loaded = world_spec::load(&path).unwrap();
    let mut config = loaded.settings;
    let mut data =
        resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &config).unwrap();
    edit(&mut data, &mut config);
    // This synthetic fixture explicitly rebuilds physical bindings after its settings edits.
    data.robot_models =
        RuntimeRobots::legacy(&config, data.htm_initial_positions.len(), data.vtm_initial_positions.len());
    data.relay_zones = RelayZones::from_scenario(&data).unwrap();
    let map = Arc::new(data.map.clone());
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding_cells(&map),
    ));
    let navigation = Navigation::new(
        map,
        pathfinder,
        BotService::from_runtime(&data.robot_models, data.vtm_zone_assignments.len()),
        Arc::new(TableOracle::new(ReservationTable::new(SimDuration::ZERO))),
    );
    (data, config, navigation)
}

fn estimate(edit: impl FnOnce(&mut ScenarioData, &mut SimConfig)) -> FetchBound {
    let (data, config, navigation) = fixture(edit);
    FetchBounds::new(&data, &navigation, &config)
        .unwrap()
        .for_machine(&navigation, &data.map.conveyor_machines[0])
        .unwrap()
}

#[test]
fn retrieval_and_empty_approach_overlap_before_the_loaded_supply_stages() {
    let value = estimate(|_, _| {});
    assert!(value.vtm_retrieval > SimDuration::ZERO);
    assert!(value.empty_htm_approach > SimDuration::ZERO);
    let serial = value.vtm_retrieval
        + value.empty_htm_approach
        + value.relay_pickup
        + value.loaded_travel
        + value.deposit
        + value.inbound_conveyor;
    assert!(value.total() < serial);
    assert_eq!(serial - value.total(), value.vtm_retrieval.min(value.empty_htm_approach));
}

#[test]
fn vtm_handling_and_htm_travel_latency_never_shrink_the_design_lead() {
    let baseline = estimate(|_, _| {});
    let slower_vtm = estimate(|_, config| config.vtm.pick_secs += 1000.0);
    assert_eq!(slower_vtm.vtm_retrieval, baseline.vtm_retrieval + SimDuration::seconds(1000));
    assert!(slower_vtm.total() > baseline.total());
    let slower_htm = estimate(|_, config| {
        config.htm.empty_speed *= 0.5;
        config.htm.loaded_speed *= 0.5;
        config.htm.empty_constant_speed *= 0.5;
        config.htm.loaded_constant_speed *= 0.5;
    });
    assert!(slower_htm.empty_htm_approach >= baseline.empty_htm_approach);
    assert!(slower_htm.loaded_travel > baseline.loaded_travel);
    assert!(slower_htm.total() >= baseline.total());
}

#[test]
fn longer_conveyor_and_fork_dwells_add_their_actual_latency() {
    let baseline = estimate(|_, _| {});
    let longer = estimate(|data, config| {
        config.htm.pickup_secs += 3.0;
        config.htm.deposit_secs += 5.0;
        for station in &mut data.map.stations {
            if let Some(circuit) = &mut station.conveyor_transit_ms {
                *circuit += 2000;
            }
        }
    });
    assert_eq!(longer.relay_pickup, baseline.relay_pickup + SimDuration::seconds(3));
    assert_eq!(longer.deposit, baseline.deposit + SimDuration::seconds(5));
    assert_eq!(longer.inbound_conveyor, baseline.inbound_conveyor + SimDuration::seconds(1));
    assert_eq!(longer.total(), baseline.total() + SimDuration::seconds(9));
}

/// The design lead has to use the handoff times the flow uses. `FulfillOrder`
/// sets the source tote down on the machine's belt, so a stated belt figure
/// governs this estimate's destination handling, while the relay pick that
/// starts the fetch keeps the diving-board figure. Sizing the pipeline on 2 s
/// when the belt takes 7.28 s would under-supply every machine.
#[test]
fn the_machine_dropoff_takes_the_belt_figure_and_the_relay_pick_does_not() {
    let baseline = estimate(|_, _| {});
    assert_eq!(baseline.deposit, baseline.relay_pickup, "unstated, both are the relay figure");

    let belt = estimate(|_, config| config.htm.conveyor_deposit_secs = Some(7.28));
    assert_eq!(belt.deposit, SimDuration::ms(7_280));
    assert_eq!(belt.relay_pickup, baseline.relay_pickup);
    assert_eq!(belt.total(), baseline.total() + SimDuration::ms(7_280) - baseline.deposit);

    // Once the belt states its own figure, the relay drop does not reach it:
    // a longer relay figure neither replaces the stated 7.28 s nor adds to it.
    let both = estimate(|_, config| {
        config.htm.deposit_secs = 3.0;
        config.htm.conveyor_deposit_secs = Some(7.28);
    });
    assert_eq!(both.deposit, SimDuration::ms(7_280));
}

#[test]
fn inventory_limit_uses_geometry_and_queue_factor_independent_of_order_waiting_places() {
    let (data, config, navigation) = fixture(|_, config| config.order.max_inflight_picks_override = None);
    let bounds = FetchBounds::new(&data, &navigation, &config).unwrap();
    let mut machine = data.map.conveyor_machines[0].clone();
    let before = bounds.for_machine(&navigation, &machine).unwrap();
    machine.waiting_cells.clear();
    for presentation in &mut machine.presentations {
        presentation.waiting_cells.clear();
        presentation.order_area = None;
    }
    assert_eq!(before, bounds.for_machine(&navigation, &machine).unwrap());
    let expected = (config.queue_factor * before.total().as_ms() as f64 / bounds.admission.period.as_ms() as f64)
        .ceil()
        .max(1.0) as u32
        + 2;
    assert_eq!(bounds.admission.cap(before.total()), expected);
    assert!(expected > 3);
    let pinned = AdmissionLaw { override_cap: Some(3), ..bounds.admission };
    assert_eq!(pinned.cap(before.total()), 3);
    let delivery = bounds.for_destination(&navigation, RackUse::Inventory, machine.dropoff_cell.unwrap()).unwrap();
    assert_eq!(delivery.total() + before.deposit + before.inbound_conveyor, before.total());
}

#[test]
fn walmart_specs_derive_inventory_limit_and_keep_only_the_two_control_differences() {
    let main: Value = serde_json::from_str(include_str!("../../../../tests/fixtures/walmart_dfw6_spec.json")).unwrap();
    let mut control: Value =
        serde_json::from_str(include_str!("../../../../tests/fixtures/walmart_dfw6_throughput_control_spec.json"))
            .unwrap();
    for spec in [&main, &control] {
        assert!(spec["settings"]["order"].get("max_inflight_picks_override").is_none());
        assert_eq!(spec["settings"]["queue_factor"], 3.0);
        assert_eq!(spec["settings"]["fidelity"], "CollisionDes");
    }
    let order = control["settings"]["order"].as_object_mut().unwrap();
    assert_eq!(order.remove("release_all_at_start"), Some(Value::Bool(true)));
    assert_eq!(order.remove("direct_extraction"), Some(Value::Bool(true)));
    assert_eq!(main, control);
}
