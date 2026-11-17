//! A configured flow is executable input: run setup, task configuration and settings survive the shared seam,
//! their digest changes with executable values, and an unsupported or absent flow cannot select a default.

use dessim::world_spec::WorldSpec;
use dessim::worldmap::{WorldMap, load_from_bytes};
use native_modules as _;
use scenario_spec::{COMPILER, Compiled, RUN_FILES, compile, generate, site_facts, stated_defaults};
use serde_json::{Value, json};
use studio_protocol::{OrderTrays, RunOverrides, ScenarioContent, Severity};

fn document() -> Value {
    serde_json::from_str(include_str!("../../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json")).unwrap()
}

fn load(document: &Value) -> WorldMap {
    load_from_bytes(&serde_json::to_vec(document).unwrap()).expect("the fixture loads")
}

fn compiled(world: &WorldMap, content: &ScenarioContent) -> Compiled {
    let workload = generate(&content.workload.generator, content.run_controls.duration_sim_ms, site_facts(world));
    let outcome = compile(world, content, &workload);
    assert!(!outcome.problems.iter().any(|problem| problem.severity == Severity::Error), "{:?}", outcome.problems);
    outcome.compiled.expect("the configured flow compiles")
}

fn pick(document: &mut Value) -> &mut Value {
    document["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["name"] == "Pick stations" && entry["components"]["flow.node"].is_object())
        .unwrap()
}

fn task<'a>(spec: &'a Value, name: &str) -> &'a Value {
    spec["composition"]["tasks"].as_array().unwrap().iter().find(|task| task["type"] == name).unwrap()
}

#[test]
fn run_setup_composition_and_settings_reach_the_same_spec_without_replacing_run_controls() {
    let world = load(&document());
    let mut content = stated_defaults();
    content.workload.order_trays = OrderTrays(123);
    let workload = generate(&content.workload.generator, content.run_controls.duration_sim_ms, site_facts(&world));
    let flow = compiled(&world, &content);
    let spec = flow.spec(&content, &RunOverrides::default(), RUN_FILES);
    assert_eq!(flow.compiler, COMPILER);
    assert!(flow.digest.starts_with("sha256:"));
    assert_eq!(flow.digest.len(), 71);
    assert_eq!(task(&spec, "FillInventory")["config"]["preload_fraction"], json!(workload.header.preload_fraction));
    assert_eq!(task(&spec, "FillInventory")["config"]["order_trays"], 123);
    assert_eq!(task(&spec, "PickDemand")["config"]["sequencing"], "all_totes");
    assert_eq!(spec["settings"]["operator"]["time_per_pick_secs"], 7.0);
    assert_eq!(spec["settings"]["putaway"]["enabled"], true);
    assert_eq!(spec["settings"]["order"]["release_all_at_start"], false);
    assert_eq!(spec["settings"]["rng_seed"], json!(content.run_controls.seed));
    let read: WorldSpec = serde_json::from_value(spec).expect("dessim reads the configured spec");
    assert!(read.composition.is_some());
    assert_eq!(read.settings.operator.time_per_pick_secs, 7.0);
    assert!(read.settings.putaway.enabled);
    assert!(!read.settings.order.release_all_at_start);
}

#[test]
fn executable_values_change_the_digest_but_labels_do_not() {
    let content = stated_defaults();
    let mut doc = document();
    let baseline = compiled(&load(&doc), &content);
    pick(&mut doc)["components"]["flow.node"]["work"]["pick_sim_ms"] = json!(9000);
    let slower = compiled(&load(&doc), &content);
    assert_eq!(slower.composition, baseline.composition, "pick service time is read as a setting");
    assert_eq!(slower.settings["operator"]["time_per_pick_secs"], 9.0);
    assert_ne!(slower.digest, baseline.digest);
    pick(&mut doc)["components"]["flow.node"]["intake"][0]["admission_factor"] = json!(5.0);
    let admission = compiled(&load(&doc), &content);
    assert_ne!(admission.composition, slower.composition);
    assert_ne!(admission.digest, slower.digest);
    pick(&mut doc)["name"] = json!("Renamed pick stations");
    assert_eq!(compiled(&load(&doc), &content).digest, admission.digest);
    let mut more_trays = content.clone();
    more_trays.workload.order_trays = OrderTrays(content.workload.order_trays.0 + 1);
    assert_ne!(compiled(&load(&doc), &more_trays).digest, admission.digest);
}

#[test]
fn missing_and_unsupported_flows_refuse_an_executable_plan() {
    let content = stated_defaults();
    let mut empty = document();
    empty["entities"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| !entry["components"].as_object().unwrap().keys().any(|key| key.starts_with("flow.")));
    let mut unsupported = document();
    pick(&mut unsupported)["components"]["flow.node"]["work"]["sequencing"] = json!("strict");
    for (doc, code) in [(empty, "flow-empty"), (unsupported, "unsupported-value")] {
        let world = load(&doc);
        let workload = generate(&content.workload.generator, content.run_controls.duration_sim_ms, site_facts(&world));
        let outcome = compile(&world, &content, &workload);
        assert!(outcome.compiled.is_none());
        assert!(
            outcome.problems.iter().any(|problem| problem.code == code && problem.severity == Severity::Error),
            "{:?}",
            outcome.problems
        );
    }
}
