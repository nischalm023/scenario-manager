//! The relay flow of spec 06 section 6.1 on the L-Shape made at 6 aisles: what it compiles to (statement 2) and the
//! all-in companion of its station nodes (statement 10).

use std::collections::BTreeSet;

use dessim::composition::flowgraph::{Executor, PlanExecutor, Request, all_in};
use dessim::composition::{Composition, RootTask};
use serde_json::{Value, json};
use site_model::ids::EntityId;

use crate::support::{self, L_SHAPE};

/// Spec 06 section 9, statement 2: the demo document compiles with no problem to the seven roots of
/// `compositions/cencora.json`, in its order, with its `after` lists.
#[test]
fn the_l_shape_compiles_to_the_roots_of_the_cencora_composition() {
    let compiled = support::compile_l_shape(&Request::validation());
    assert!(compiled.problems.is_empty(), "{:#?}", compiled.problems);
    let plan = compiled.plan.expect("a plan");
    let cencora = Composition::Named("cencora".into()).resolve().unwrap();
    let shape = |tasks: &[RootTask]| -> Vec<(String, Vec<String>)> {
        tasks.iter().map(|task| (task.task_type.clone(), task.after.clone())).collect()
    };
    assert_eq!(shape(&plan.composition.tasks), shape(&cencora.tasks));
    plan.composition.validate().expect("every root's configuration is one its registered task accepts");
    let config = |name: &str| plan.composition.tasks.iter().find(|task| task.task_type == name).unwrap().config.clone();
    assert_eq!(config("FillInventory"), json!({}), "validation mode carries no setup");
    assert_eq!(
        config("PickDemand"),
        json!({ "embargo_secs": 0.0, "pace_to_station": false, "queue_factor": 4.0, "sequencing": "all_totes" })
    );
    assert_eq!(
        config("PutawayInduction"),
        json!({ "max_presentations_per_hour": 240.0, "sequence": "sku_demand", "units_per_tote": 1 })
    );
    let settings: Vec<(&str, &Value)> =
        plan.settings.iter().map(|entry| (entry.setting.as_str(), &entry.value)).collect();
    assert_eq!(
        settings,
        [
            ("operator.pallet_swap_secs", &json!(0.0)),
            ("operator.time_per_pick_secs", &json!(7.0)),
            ("putaway.enabled", &json!(true)),
            ("putaway.forbid_rack_after_secs", &Value::Null),
            ("putaway.htm_wip_factor", &json!(1.5)),
            ("putaway.ignore_hour_gate", &json!(false)),
            ("putaway.random_slots", &json!(false)),
        ]
    );
}

/// Statement 2's second half (ruling D105): the plan resolves each VTM segment rack row by rack row to the six aisles'
/// cohorts, while no flow entity names an aisle, its sector or its cohort.
#[test]
fn the_plan_names_each_aisles_cohort_and_the_flow_names_none() {
    let compiled = support::compile_l_shape(&Request::validation());
    let plan = compiled.plan.expect("a plan");
    let world = &L_SHAPE.world;
    let with = |component: &str| -> BTreeSet<EntityId> {
        world.entities.values().filter(|e| e.has(component)).map(|e| e.id.clone()).collect()
    };
    let vtm_cohorts: BTreeSet<EntityId> = with("cohort")
        .into_iter()
        .filter(|id| world.entities[id].components["cohort"]["assignment"]["kind"] == "hard")
        .collect();
    assert_eq!(vtm_cohorts.len(), 6);
    let mut reached = BTreeSet::new();
    let assigned =
        plan.flows.edges.iter().flat_map(|edge| &edge.segments).filter_map(|segment| match &segment.executor {
            PlanExecutor::Assigned { by_member, .. } => Some(by_member),
            _ => None,
        });
    for by_member in assigned {
        assert_eq!(by_member.len(), 12, "two rack rows in each of six aisles");
        for row in by_member {
            assert_eq!(row.cohorts.len(), 1, "one cohort works each rack row");
            let cohort = &world.entities[&row.cohorts[0]];
            let sector =
                cohort.components["cohort"]["assignment"]["to"][0].as_str().unwrap().parse::<EntityId>().unwrap();
            let rows = world.entities[&sector].components["sector"]["members"].as_array().unwrap();
            assert!(
                rows.iter().any(|member| member == &json!(row.member.to_string())),
                "the cohort is the row's aisle's"
            );
            reached.insert(row.cohorts[0].clone());
        }
    }
    assert_eq!(reached, vtm_cohorts, "the VTM segments reach all six aisles' cohorts");
    let named: BTreeSet<EntityId> = (compiled.graph.segments.iter())
        .filter_map(|segment| match &segment.spec.executor {
            Executor::Cohort(cohort) => Some(cohort.id.clone()),
            _ => None,
        })
        .collect();
    assert!(named.is_disjoint(&vtm_cohorts), "segments name the HTM fleet only");
    let flow_text =
        serde_json::to_string(&support::flows_of(&L_SHAPE).entities.iter().map(|e| &e.components).collect::<Vec<_>>())
            .unwrap();
    for id in vtm_cohorts.iter().chain(
        &with("sector")
            .into_iter()
            .filter(|id| world.entities[id].name.as_deref() == Some("aisle"))
            .collect::<Vec<_>>(),
    ) {
        assert!(!flow_text.contains(&id.to_string()), "no flow entity names {id}");
    }
}

/// Spec 06 section 9, statement 10: the all-in companion of the demo site's pick and put nodes, and nothing for a
/// node that states no tote swap.
#[test]
fn the_all_in_companion_reads_as_section_3_4_words_it() {
    let compiled = support::compile_l_shape(&Request::validation());
    let node = |name: &str| compiled.graph.nodes.iter().find(|node| node.name.as_deref() == Some(name)).unwrap();
    let pick = all_in(&node("Pick stations").spec).expect("the pick node states a swap");
    assert_eq!(pick.text, "7.0 s operator time + 2.0 s tote swap = 9.0 s, 400 an hour per station, all-in");
    assert_eq!((pick.field.as_str(), pick.value.get(), pick.unit), ("/flow.node/work/pick_sim_ms", 400.0, "per hour"));
    let put = all_in(&node("Put stations").spec).expect("the put node states a swap");
    assert_eq!(put.text, "240 an hour fill pace = 211.8 an hour all-in");
    let mut unswapped = node("Pick stations").spec.clone();
    unswapped.work.tote_swap_sim_ms = None;
    assert_eq!(all_in(&unswapped), None);
    assert_eq!(all_in(&node("Inventory storage").spec), None);
}
