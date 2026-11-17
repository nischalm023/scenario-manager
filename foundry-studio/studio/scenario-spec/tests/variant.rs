//! Spec 07 section 8, statement 7's world half: the world a run loads differs from the export it fetched only at the
//! variant's pointers, their JSON difference equal to the patch list, and each patch's `was` equals the export's
//! value; a spec patch changes the version's content at its pointer alone; a target the world lacks is refused.

use std::path::Path;

use scenario_spec::{VariantError, apply_variant, stated_defaults};
use serde_json::{Value, json};
use studio_protocol::{AppliedTo, OrdersPerHour, Patch, Variant, VariantNumber};

const VTM: &str = "d257c36b-53ea-5ba2-a0f3-6fd0346f00ac";

fn export() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_putaway.json");
    serde_json::from_slice(&std::fs::read(path).expect("the fixture reads")).expect("JSON")
}

fn patch(target: &str, pointer: &str, value: Value, applied_to: AppliedTo) -> Patch {
    Patch { target: target.to_owned(), pointer: pointer.to_owned(), value, label: "a field".to_owned(), applied_to }
}

/// Every leaf where two documents differ, as an entity id and a pointer into its components.
fn differences(before: &Value, after: &Value) -> Vec<(String, String)> {
    fn walk(at: String, a: &Value, b: &Value, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
                keys.sort();
                keys.dedup();
                for key in keys {
                    walk(
                        format!("{at}/{key}"),
                        a.get(key).unwrap_or(&Value::Null),
                        b.get(key).unwrap_or(&Value::Null),
                        out,
                    );
                }
            }
            _ if a != b => out.push(at),
            _ => {}
        }
    }
    let entities = |doc: &Value| doc["entities"].as_array().expect("entities").clone();
    let mut found = Vec::new();
    for (a, b) in entities(before).iter().zip(entities(after).iter()) {
        let mut leaves = Vec::new();
        walk(String::new(), &a["components"], &b["components"], &mut leaves);
        found.extend(leaves.into_iter().map(|leaf| (a["id"].as_str().expect("an id").to_owned(), leaf)));
    }
    assert_eq!(before["entities"].as_array().map(Vec::len), after["entities"].as_array().map(Vec::len));
    found
}

#[test]
fn the_world_differs_from_the_export_only_at_the_variants_pointers_and_was_is_the_exports_value() {
    let fetched = export();
    let mut world = fetched.clone();
    let mut content = stated_defaults();
    let variant = Variant {
        n: VariantNumber(1),
        label: "drop 5.0 s".to_owned(),
        patches: vec![
            patch(VTM, "/end_effector/drop_secs", json!(5.0), AppliedTo::World),
            patch(VTM, "/end_effector/reach_slots_deep", json!(3), AppliedTo::World),
        ],
    };
    let applied = apply_variant(&variant, &mut world, &mut content).expect("the variant applies");
    let expected: Vec<(String, String)> =
        variant.patches.iter().map(|patch| (patch.target.clone(), patch.pointer.clone())).collect();
    let mut found = differences(&fetched, &world);
    found.sort();
    let mut expected_sorted = expected.clone();
    expected_sorted.sort();
    assert_eq!(found, expected_sorted, "the JSON difference is the patch list");
    assert_eq!(applied[0].was, json!(2.0), "the export's drop time");
    assert_eq!(applied[1].was, Value::Null, "a field the export did not state was nothing");
    assert_eq!(content, stated_defaults(), "a world patch leaves the content alone");
}

#[test]
fn a_spec_patch_changes_the_content_at_its_pointer_and_says_what_it_held() {
    let mut world = export();
    let mut content = stated_defaults();
    let variant = Variant {
        n: VariantNumber(2),
        label: "rate 350".to_owned(),
        patches: vec![patch(
            "scenario:01KK8AJ7RYA6GNNQ3QTRQCW7Z8",
            "/workload/generator/orders_per_hour",
            json!(350),
            AppliedTo::Spec,
        )],
    };
    let applied = apply_variant(&variant, &mut world, &mut content).expect("the variant applies");
    assert_eq!(applied[0].was, json!(300.0));
    let mut expected = stated_defaults();
    expected.workload.generator.orders_per_hour = OrdersPerHour(350.0);
    assert_eq!(content, expected);
    assert_eq!(world, export(), "a spec patch leaves the world alone");
}

#[test]
fn a_target_the_world_lacks_and_a_value_the_content_refuses_are_refused() {
    let (mut world, mut content) = (export(), stated_defaults());
    let lacking = Variant {
        n: VariantNumber(1),
        label: "x".to_owned(),
        patches: vec![patch(
            "00000000-0000-4000-8000-000000000000",
            "/flow.node/work/pick_sim_ms",
            json!(1),
            AppliedTo::World,
        )],
    };
    let refused = apply_variant(&lacking, &mut world, &mut content);
    assert!(matches!(refused, Err(VariantError::TargetMissing { .. })), "{refused:?}");
    let bad = Variant {
        n: VariantNumber(1),
        label: "x".to_owned(),
        patches: vec![patch(
            "scenario:01KK8AJ7RYA6GNNQ3QTRQCW7Z8",
            "/workload/generator/release",
            json!("hourly"),
            AppliedTo::Spec,
        )],
    };
    assert!(matches!(apply_variant(&bad, &mut world, &mut content), Err(VariantError::Refused { .. })));
    assert_eq!(content, stated_defaults(), "a refused patch changes nothing");
}
