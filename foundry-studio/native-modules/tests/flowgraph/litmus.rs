//! Spec 06 section 9, statement 4, the litmus test (ruling D102): the store-replenishment flow on the map of the
//! study of record compiles, in run mode with the study's setup, to the study's roots and settings; its closed-order
//! guards are exclusive and complete, and dropping one is `guard-gap` naming the node; the VTM segments resolve per
//! rack row to each aisle's cohort, standard or gripper, while the fixture names none.

use std::path::Path;

use dessim::composition::flowgraph::{Code, PlanExecutor, Served, Severity, compile};
use dessim::composition::{InductionTaskConfig, RootTask};
use serde_json::{Value, json};
use site_model::ids::EntityId;

use crate::support::{self, LITMUS};

fn study() -> Vec<RootTask> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6_throughput_spec.json");
    let spec: Value = support::json(&path);
    serde_json::from_value(spec["composition"]["tasks"].clone()).expect("the study's composition parses")
}

#[test]
fn the_litmus_flow_compiles_to_the_studys_roots_and_settings() {
    let flows = support::flows_of(&LITMUS);
    let compiled = compile(&LITMUS, &flows, &support::study_run());
    assert!(compiled.problems.is_empty(), "{:#?}", compiled.problems);
    for edge in ["Put stations to Inventory storage: empty tote", "Put stations to Inventory storage: stock tote"] {
        let id = support::id_of(&compiled, &flows, edge);
        assert_eq!(
            compiled.served_by(compiled.graph.edge(&id).unwrap()),
            Served::Clearance,
            "{edge}: a return is each finished presentation's clearance, which no root runs"
        );
    }
    let plan = compiled.plan.expect("a plan");
    let shape = |tasks: &[RootTask]| -> Vec<(String, Vec<String>)> {
        tasks.iter().map(|task| (task.task_type.clone(), task.after.clone())).collect()
    };
    assert_eq!(
        shape(&plan.composition.tasks),
        shape(&study()),
        "the study's roots, in its order, with its after lists"
    );
    plan.composition.validate().expect("every root's configuration is one its registered task accepts");
    let config = |name: &str| plan.composition.tasks.iter().find(|task| task.task_type == name).unwrap().config.clone();
    assert_eq!(config("FillInventory"), json!({ "order_trays": 128, "preload_fraction": 0.5 }));
    assert_eq!(
        config("FulfillOrders"),
        json!({ "bypass_box_induction": false, "bypass_extraction": false, "buffer_dwell_secs": 3600.0,
                "extraction_secs": 9.0, "queue_factor": 3.0 }),
        "extraction from the extract nodes, the dwell from the buffer, the queue factor from the lane's intake, isolation off"
    );
    assert_eq!(config("PutawaySupplyDemand"), json!({}), "no direct-empty alternative, so unconfigured");
    let induction: InductionTaskConfig = serde_json::from_value(config("PutawayInduction")).unwrap();
    let studied: InductionTaskConfig =
        serde_json::from_value(study().into_iter().find(|task| task.task_type == "PutawayInduction").unwrap().config)
            .unwrap();
    assert_eq!(induction, studied, "60 presentations an hour, 6 units a tote, the order_release sequence");
    let settings: Vec<(&str, &Value)> =
        plan.settings.iter().map(|entry| (entry.setting.as_str(), &entry.value)).collect();
    assert_eq!(
        settings,
        [
            ("operator.time_per_pick_secs", &json!(10.0)),
            ("order.direct_extraction", &json!(false)),
            ("pick.embargo_secs", &json!(0.0)),
            ("putaway.enabled", &json!(true)),
            ("putaway.forbid_rack_after_secs", &Value::Null),
            ("putaway.htm_wip_factor", &json!(1.0)),
            ("putaway.ignore_hour_gate", &json!(false)),
            ("putaway.random_slots", &json!(false)),
        ],
        "the lane's 10 s (the map's 9 s display only), the buffer edge, and the putaway figures at dessim's defaults"
    );
}

/// Each VTM segment resolves per rack row to its aisle's cohort: the standard VTMs for inventory and the empty order
/// tote row, the grippers for the order buffer; the fixture names no cohort but the free HTM fleet.
#[test]
fn the_litmus_vtm_segments_resolve_per_rack_row_to_each_aisles_cohort() {
    let compiled = compile(&LITMUS, &support::flows_of(&LITMUS), &support::study_run());
    let plan = compiled.plan.expect("a plan");
    let world = &LITMUS.world;
    let configuration = |cohort: &EntityId| {
        let class =
            world.entities[cohort].components["cohort"]["robot_class"].as_str().unwrap().parse::<EntityId>().unwrap();
        world.entities[&class].components["robot_class"]["configuration"].as_str().unwrap().to_owned()
    };
    let mut seen = Vec::new();
    for edge in &plan.flows.edges {
        for segment in &edge.segments {
            let PlanExecutor::Assigned { by_member, .. } = &segment.executor else { continue };
            let node = &segment.from.node.id;
            let holds = compiled.graph.node(node).unwrap().spec.holds.unwrap();
            for row in by_member {
                assert_eq!(row.cohorts.len(), 1, "one cohort works each rack row");
                seen.push((format!("{holds:?}"), configuration(&row.cohorts[0])));
            }
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen,
        [
            ("ClosedOrder".to_owned(), "gripper".to_owned()),
            ("InventoryTote".to_owned(), "standard".to_owned()),
            ("OrderContainer".to_owned(), "standard".to_owned()),
        ]
    );
    let flow_text =
        serde_json::to_string(&support::flows_of(&LITMUS).entities.iter().map(|e| &e.components).collect::<Vec<_>>())
            .unwrap();
    let cohorts = world.entities.values().filter(|e| e.has("cohort"));
    let named: Vec<String> =
        cohorts.filter(|e| flow_text.contains(&e.id.to_string())).map(|e| e.name.clone().unwrap()).collect();
    assert_eq!(named, ["HTM fleet"]);
}

/// Dropping one guarded edge leaves its order kinds with nowhere to go: `guard-gap` naming the lane, an error with
/// the study's demand.
#[test]
fn dropping_one_guard_is_a_guard_gap_naming_the_node() {
    let mut flows = support::flows_of(&LITMUS);
    let clean = compile(&LITMUS, &flows, &support::study_run());
    let bagging = support::id_of(&clean, &flows, "EasyLoop machines to Bagging: closed order");
    let carrying: Vec<EntityId> = clean.graph.chain(&bagging).iter().map(|segment| segment.id.clone()).collect();
    support::remove(&mut flows, &bagging);
    for segment in &carrying {
        support::remove(&mut flows, segment);
    }
    let lane = support::id_of(&clean, &flows, "EasyLoop machines");
    let compiled = compile(&LITMUS, &flows, &support::study_run());
    assert_eq!(support::named_by(&compiled, Code::GuardGap), [lane]);
    let gap = compiled.problems.iter().find(|p| p.code == Code::GuardGap).unwrap();
    assert_eq!(gap.severity, Severity::Error);
    assert!(gap.text.contains("tote orders leaving by sorter, tote orders leaving by dock"), "{}", gap.text);
    assert!(compiled.plan.is_none());
}
