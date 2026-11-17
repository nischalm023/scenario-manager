//! Spec 06 section 9, statement 5: one input compiles to one plan, byte for byte, and what the run executes (the
//! composition and the settings, all that a digest of the plan covers) moves with the flow's values and the setup
//! and with nothing else.

use std::collections::BTreeMap;

use dessim::composition::flowgraph::{Changeset, Plan, Request, RunSetup, compile, settings_object};
use dessim::worldmap::flows::FlowScope;
use dessim::worldmap::load_from_bytes;
use serde_json::{Value, json};

use crate::support::{self, L_SHAPE};

fn plan(request: &Request, edit: impl FnOnce(&mut FlowScope)) -> Plan {
    let mut flows = support::flows_of(&L_SHAPE);
    edit(&mut flows);
    let compiled = compile(&L_SHAPE, &flows, request);
    compiled.plan.unwrap_or_else(|| panic!("a plan: {:#?}", compiled.problems))
}

/// What a run executes: the composition and the settings block as the runner writes them into its `spec.json`.
fn executed(plan: &Plan) -> Value {
    json!({ "composition": plan.composition, "settings": settings_object(&plan.settings) })
}

fn node_named<'a>(flows: &'a mut FlowScope, name: &str) -> &'a mut Value {
    let entity = flows.entities.iter_mut().find(|e| e.name.as_deref() == Some(name)).unwrap();
    entity.components.get_mut("flow.node").unwrap()
}

#[test]
fn one_document_compiles_to_one_plan_byte_for_byte() {
    let request = Request::validation();
    let first = serde_json::to_vec(&plan(&request, |_| {})).unwrap();
    assert_eq!(first, serde_json::to_vec(&plan(&request, |_| {})).unwrap());
    let mut document = support::json(&support::l_shape_snapshot());
    document["entities"].as_array_mut().unwrap().reverse();
    let reordered = load_from_bytes(&serde_json::to_vec(&document).unwrap()).unwrap();
    let again = compile(&reordered, &support::flows_of(&reordered), &request).plan.unwrap();
    assert_eq!(first, serde_json::to_vec(&again).unwrap(), "reordering the entities changes no byte of the plan");
}

#[test]
fn what_a_run_executes_moves_with_the_flows_values_and_the_setup_only() {
    let base = plan(&Request::validation(), |_| {});
    let changed = plan(&Request::validation(), |flows| {
        node_named(flows, "Pick stations")["intake"][0]["admission_factor"] = json!(6.0);
    });
    assert_ne!(executed(&changed), executed(&base), "one value changed (section 4.3's draft)");
    let run = |stock| Request::run(RunSetup { stock_at_start: stock, order_trays: 0 }, support::study_order_kinds());
    assert_ne!(executed(&plan(&run(0.4), |_| {})), executed(&plan(&run(0.5), |_| {})), "the setup changed");
    let renamed = plan(&Request::validation(), |flows| {
        flows.entities.iter_mut().find(|e| e.name.as_deref() == Some("Pick stations")).unwrap().name =
            Some("Pickers".into());
    });
    assert_eq!(executed(&renamed), executed(&base), "a label");
    let swapped = plan(&Request::validation(), |flows| {
        node_named(flows, "Pick stations")["work"]["tote_swap_sim_ms"] = json!(2230);
    });
    assert_eq!(executed(&swapped), executed(&base), "a display-only figure");
    assert_eq!(serde_json::to_vec(&swapped).unwrap(), serde_json::to_vec(&base).unwrap(), "nor does it reach the plan");
    let seqs: BTreeMap<String, Changeset> = [
        ("customer/demo/site/lshape-6", 1),
        ("customer/demo/site/lshape-6/flow", 2),
        ("customer/demo/site/lshape-6/map", 1),
        ("customer/demo/site/lshape-6/fleet", 1),
        ("catalog/agent/d30", 4),
        ("catalog/agent/da50", 7),
    ]
    .into_iter()
    .map(|(scope, seq)| (scope.to_owned(), Changeset(seq)))
    .collect();
    let provenance = plan(&Request { seqs: seqs.clone(), exact: false, ..Request::validation() }, |_| {});
    assert_eq!(executed(&provenance), executed(&base), "the read's provenance");
    assert_eq!(provenance.seqs, seqs, "seqs names every scope the caller read");
    assert!(!provenance.exact);
    let bytes: Value = serde_json::to_value(&provenance).unwrap();
    assert_eq!(bytes["compiler"], json!("0.2.0"));
    assert_eq!(bytes["v"], json!(1));
}
