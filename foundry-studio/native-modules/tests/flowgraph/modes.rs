//! Spec 06 section 9, statement 7: the two modes (section 3.2), and section 3.4's `flow-validate` patches, of which a
//! structural field is refused as `not-sweepable`.

use std::slice;

use dessim::composition::flowgraph::{Code, Patch, Request, Severity, compile};
use serde_json::{Value, json};

use crate::support::{self, LITMUS};

/// Validation mode emits `setup: null` and keeps an order kind no guard covers a warning; run mode, with the study's
/// demand, makes it an error.
#[test]
fn an_uncovered_order_kind_is_a_warning_without_a_scenario_and_an_error_with_one() {
    let mut flows = support::flows_of(&LITMUS);
    let clean = compile(&LITMUS, &flows, &Request::validation());
    let validated = clean.plan.as_ref().expect("the litmus flow validates");
    assert_eq!(serde_json::to_value(validated).unwrap()["setup"], Value::Null);
    let flex = support::id_of(&clean, &flows, "EasyLoop machines to Flex: closed order");
    let lane = support::id_of(&clean, &flows, "EasyLoop machines");
    let carrying: Vec<_> = clean.graph.chain(&flex).iter().map(|segment| segment.id.clone()).collect();
    support::remove(&mut flows, &flex);
    carrying.iter().for_each(|segment| support::remove(&mut flows, segment));
    let validation = compile(&LITMUS, &flows, &Request::validation());
    let gaps: Vec<_> = validation.problems.iter().filter(|p| p.code == Code::GuardGap).collect();
    assert_eq!(
        gaps.iter().map(|p| (p.severity, p.reference.clone())).collect::<Vec<_>>(),
        [(Severity::Warning, Some(lane.clone()))]
    );
    assert!(gaps[0].text.starts_with("Warning: "));
    let run = compile(&LITMUS, &flows, &support::study_run());
    let gaps: Vec<_> = run.problems.iter().filter(|p| p.code == Code::GuardGap).collect();
    assert_eq!(
        gaps.iter().map(|p| (p.severity, p.reference.clone())).collect::<Vec<_>>(),
        [(Severity::Error, Some(lane))]
    );
    assert!(run.plan.is_none(), "an error refuses the plan");
}

/// A patch at a structural pointer is refused as `not-sweepable`; a sweepable one compiles with the variant applied
/// and recorded.
#[test]
fn flow_validate_refuses_a_patch_at_what_an_edge_moves() {
    let flows = support::flows_of(&support::L_SHAPE);
    let clean = compile(&support::L_SHAPE, &flows, &Request::validation());
    let supply = support::id_of(&clean, &flows, "Inventory storage to Put stations: empty tote");
    let structural =
        Patch { target: supply.clone(), pointer: "/flow.material_edge/moves".into(), value: json!("units") };
    let refused = compile(&support::L_SHAPE, &flows, &Request { variant: vec![structural], ..Request::validation() });
    assert_eq!(support::named_by(&refused, Code::NotSweepable), slice::from_ref(&supply));
    assert!(refused.plan.is_none());
    let sweep = Patch { target: supply, pointer: "/flow.material_edge/until_sim_ms".into(), value: json!(7_150_000) };
    let swept = compile(&support::L_SHAPE, &flows, &Request { variant: vec![sweep.clone()], ..Request::validation() });
    let plan = swept.plan.expect("a sweepable patch compiles");
    let cutoff = plan.settings.iter().find(|entry| entry.setting == "putaway.forbid_rack_after_secs").unwrap();
    assert_eq!(cutoff.value, json!(7150.0));
    assert_eq!(plan.variant, [sweep]);
}

/// A sweep's variant is held to section 3.1's lower bounds as the site is: an admission factor of zero is refused as
/// `schema-unknown` on the node it patches, and nothing compiles (Astra's task-11 read: the variant patches too).
#[test]
fn a_variant_below_a_lower_bound_compiles_to_nothing() {
    let flows = support::flows_of(&support::L_SHAPE);
    let clean = compile(&support::L_SHAPE, &flows, &Request::validation());
    let pick = support::id_of(&clean, &flows, "Pick stations");
    for value in [json!(0.0), json!(-4.0)] {
        let zero = Patch { target: pick.clone(), pointer: "/flow.node/intake/0/admission_factor".into(), value };
        let refused = compile(&support::L_SHAPE, &flows, &Request { variant: vec![zero], ..Request::validation() });
        assert_eq!(support::named_by(&refused, Code::SchemaUnknown), slice::from_ref(&pick));
        assert!(refused.plan.is_none());
    }
}
