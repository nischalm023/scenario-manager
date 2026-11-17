//! Spec 06 section 3.4's parameter list: every scalar field the four components state, labelled from one static
//! field catalogue, with what a sweep may vary, a closed field's options in the customer's words, and the all-in
//! companion beside a station figure.

use dessim::composition::flowgraph::{Accepts, Request, compile, lookup, parameters};
use serde_json::{Value, json};

use crate::support::{self, L_SHAPE, LITMUS};

#[test]
fn the_catalogue_names_every_field_both_fixtures_state() {
    for world in [&*L_SHAPE, &*LITMUS] {
        for entity in support::flows_of(world).entities {
            for (component, value) in entity.flow_components() {
                let mut pending = vec![(format!("/{component}"), value.clone())];
                while let Some((at, value)) = pending.pop() {
                    match value {
                        Value::Object(fields) => {
                            pending.extend(fields.into_iter().map(|(k, v)| (format!("{at}/{k}"), v)))
                        }
                        Value::Array(items) => {
                            pending.extend(items.into_iter().enumerate().map(|(i, v)| (format!("{at}/{i}"), v)))
                        }
                        _ => assert!(lookup(&at).is_some(), "the catalogue names {at}"),
                    }
                }
            }
        }
    }
}

#[test]
fn the_l_shapes_parameters_read_as_section_3_4_words_them() {
    let flows = support::flows_of(&L_SHAPE);
    let compiled = compile(&L_SHAPE, &flows, &Request::validation());
    let list = parameters(&flows, &compiled, &Accepts::this_build());
    let find = |label: &str| list.iter().find(|p| p.label == label).unwrap_or_else(|| panic!("{label} is listed"));
    let pace = find("Put stations, fill pace per station");
    assert_eq!(
        (pace.short_label, pace.unit, pace.sweepable, &pace.value),
        ("pace", Some("per hour"), true, &json!(240))
    );
    assert_eq!(pace.all_in.as_ref().map(|a| a.text.as_str()), Some("240 an hour fill pace = 211.8 an hour all-in"));
    let pick = find("Pick stations, time per pick");
    assert_eq!((pick.unit, pick.sweepable), (Some("ms"), true));
    assert!(pick.all_in.as_ref().is_some_and(|a| a.text.starts_with("7.0 s operator time + 2.0 s tote swap")));
    let sequencing = find("Pick stations, sequencing");
    let words: Vec<&str> = sequencing.options.iter().map(|choice| choice.label).collect();
    assert_eq!(words, ["Any case order (all totes)", "Line order (sequenced)"]);
    let only_all_totes = parameters(&flows, &compiled, &Accepts::image(vec!["all_totes".into()]));
    let narrowed = only_all_totes.iter().find(|p| p.label == "Pick stations, sequencing").unwrap();
    assert_eq!(narrowed.options.len(), 1, "only values both the image and this build accept");
    for pointer in
        ["/flow.material_edge/moves", "/flow.material_edge/from", "/flow.node/kind", "/flow.node/work/tote_swap_sim_ms"]
    {
        assert!(list.iter().filter(|p| p.pointer == pointer).all(|p| !p.sweepable), "{pointer} is not sweepable");
    }
    assert!(list.iter().any(|p| p.pointer == "/flow.material_edge/moves"), "every scalar field is listed");
    let backlog = list.iter().find(|p| p.pointer == "/flow.return_rule/min_backlog").expect("the backlog is listed");
    assert!(!backlog.sweepable, "no run reads the return rule's backlog, so no sweep varies it");
}
