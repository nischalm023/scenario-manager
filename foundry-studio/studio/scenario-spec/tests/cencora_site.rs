//! The seeded Cencora site (`data/seed/customer/`) needs no launch line for a Studio run: joined as an export joins
//! a site's scopes, it states its HTM drive model, and its flow compiles in run mode to the relay roots with the
//! quantities the flow states.

use std::fs;
use std::path::{Path, PathBuf};

use dessim::composition::Composition;
use dessim::world_spec::WorldSpec;
use dessim::worldmap::load_from_bytes;
use desunits::SimDuration;
use native_modules as _;
use scenario_spec::{HOUR_MS, RUN_FILES, compile, generate, require_catalog_motion, site_facts, stated_defaults};
use serde_json::{Value, json};
use studio_protocol::{FillPercent, LinesPerOrder, OrdersPerHour, RunOverrides, ScenarioContent, SkuCount};

const SITE: &str = "customer/cencora/site/ndc2";

fn customer_seed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/customer")
}

fn read(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()))
}

/// The site's documents in manifest order, the first (its map) keeping its prologue and every later one adding its
/// entities, as a site's export holds the entities of all its scopes.
fn site() -> Value {
    let manifest = read(&customer_seed_dir().join("manifest.json"));
    let mut documents = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["scope"].as_str().unwrap().starts_with(SITE))
        .map(|entry| read(&customer_seed_dir().join(entry["document"].as_str().unwrap())));
    let mut site = documents.next().expect("the manifest seeds the site");
    for document in documents {
        site["entities"].as_array_mut().unwrap().extend(document["entities"].as_array().unwrap().iter().cloned());
    }
    site
}

/// A generated workload sized like Dematic's Scenario 13: 8 h of simtime, 1,179 SKUs, pallets of 44 lines on
/// average, about 1,504 orders generated in preparation and released at simtime 0, every unit arriving by putaway.
fn sized_like_scenario_13() -> ScenarioContent {
    let mut content = stated_defaults();
    let generator = &mut content.workload.generator;
    generator.orders_per_hour = OrdersPerHour(188.0);
    generator.lines_per_order = LinesPerOrder(44.0);
    generator.sku_count = SkuCount(1179);
    generator.inventory_fill_pct = FillPercent(0.0);
    content.run_controls.duration_sim_ms = SimDuration::ms(8 * HOUR_MS);
    content
}

fn root_config<'a>(composition: &'a Value, name: &str) -> &'a Value {
    let tasks = composition["tasks"].as_array().unwrap();
    &tasks.iter().find(|task| task["type"] == name).unwrap_or_else(|| panic!("no {name} root"))["config"]
}

#[test]
fn the_seeded_cencora_site_compiles_to_the_relay_roots_with_the_quantities_its_flow_states() {
    let world = load_from_bytes(&serde_json::to_vec(&site()).unwrap()).expect("the seeded site loads");
    require_catalog_motion(&world).expect("every deployed robot class states its drive model");
    let content = sized_like_scenario_13();
    let workload = generate(&content.workload.generator, content.run_controls.duration_sim_ms, site_facts(&world));
    assert_eq!(workload.header.preload_fraction.0, 0.0, "every unit arrives by putaway");
    let outcome = compile(&world, &content, &workload);
    assert!(outcome.problems.is_empty(), "no error and no warning: {:?}", outcome.problems);
    let compiled = outcome.compiled.expect("the flow compiles to a plan");

    let named = serde_json::to_value(Composition::Named("cencora".into()).resolve().unwrap()).unwrap();
    let roots = |composition: &Value| -> Vec<(Value, Value)> {
        let tasks = composition["tasks"].as_array().unwrap();
        tasks.iter().map(|task| (task["type"].clone(), task["after"].clone())).collect()
    };
    assert_eq!(roots(&compiled.composition), roots(&named), "the relay roots, in their order and after lists");
    let plan = &compiled.composition;
    assert_eq!(root_config(plan, "FillInventory"), &json!({ "order_trays": 0, "preload_fraction": 0.0 }));
    assert_eq!(
        root_config(plan, "PutawayInduction"),
        &json!({ "max_presentations_per_hour": 261.4, "sequence": "sku_demand", "units_per_tote": 1 })
    );
    assert_eq!(
        root_config(plan, "PickDemand"),
        &json!({ "embargo_secs": 7200.0, "sequencing": "sequenced", "pace_to_station": false, "queue_factor": 6.0 })
    );
    assert_eq!(
        root_config(plan, "PutawaySupplyDemand"),
        &Value::Null,
        "a fleet-pressure bias states no direct_bias_secs"
    );
    let settings = &compiled.settings;
    assert_eq!(settings["operator"]["time_per_pick_secs"], 6.77);
    assert_eq!(settings["operator"]["pallet_swap_secs"], 0.0);
    assert_eq!(settings["putaway"]["enabled"], true);
    assert_eq!(settings["putaway"]["forbid_rack_after_secs"], 7150.0);
    assert_eq!(settings["putaway"]["htm_wip_factor"], 2.5);
    assert_eq!(settings["putaway"]["ignore_hour_gate"], true);
    assert_eq!(settings["putaway"]["random_slots"], true);

    let spec = compiled.spec(&content, &RunOverrides::default(), RUN_FILES);
    let read: WorldSpec = serde_json::from_value(spec).expect("dessim reads the run's spec");
    assert_eq!(read.settings.operator.time_per_pick_secs, 6.77);
    assert!(read.settings.putaway.enabled);
}
