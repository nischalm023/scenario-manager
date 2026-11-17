//! The protocol's payloads read the specs' own examples and write them back unchanged: spec 03 section 3.3's
//! `panel.command` entry and tool input, spec 03 section 3.4's page context, spec 01 section 3.5's envelope, and spec
//! 01 section 3.14's problem body.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use studio_protocol::{
    ContextPanel, DeclaredId, JournalEntry, JournalKind, Key, PageContext, Panel, PanelClearInput, PanelCommandEntry,
    PanelHighlightInput, PanelSelectInput, PanelShowInput, PanelTarget, Problem, ProblemType, Ref,
};

/// Read `example` as `T` and write it back: the same JSON must come out.
fn round_trip<T: DeserializeOwned + Serialize>(example: Value) -> T {
    let value: T = serde_json::from_value(example.clone()).expect("the spec's example reads");
    assert_eq!(serde_json::to_value(&value).expect("it writes"), example);
    value
}

/// Spec 03 section 3.3: the `panel.command` entry's example reads and writes back unchanged, its unused options null.
#[test]
fn the_panel_command_entry_of_spec_03_reads_and_writes_back() {
    let entry: PanelCommandEntry = round_trip(json!({
        "action": "show", "panel": "map",
        "location": "customer/demo/site/lshape-6/map@main",
        "resolved": [ { "scope": "customer/demo/site/lshape-6/map", "seq": 1 },
                      { "scope": "customer/demo/site/lshape-6/flow", "seq": 1 } ],
        "select": null, "layers": null, "draft": null, "sim_ms": null, "compare": null, "metrics": null,
        "highlight": [ { "refs": ["b6a9a763-687c-4067-909b-460e986e2229"], "note": "where stock totes arrive for picking",
                         "marks": 4, "on_floor": 4 } ],
        "route": "/w/customer/demo/site/lshape-6/map@main?highlights=412",
        "as_it_was": "/w/customer/demo/site/lshape-6/map@1?highlights=412",
        "announce": "The agent opened the Map panel: Small L-shape, 6 aisles, map at changeset 1. Highlighted 4 entities: where stock totes arrive for picking."
    }));
    assert!(matches!(entry.location, Some(PanelTarget::Location(_))));
}

/// Spec 03 section 3.4: a page context reads and writes back unchanged, its selection and highlights null when the
/// panel shows none.
#[test]
fn the_page_context_of_spec_03_reads_and_writes_back() {
    let context: PageContext = round_trip(json!({
        "route": "/w/customer/demo/site/lshape-6/map@1?select=b6a9a763-687c-4067-909b-460e986e2229",
        "panel": "map",
        "title": "Map: Small L-shape, 6 aisles, map at changeset 1",
        "seen": [ { "scope": "customer/demo/site/lshape-6/map", "seq": 1 } ],
        "selection": { "key": "b6a9a763-687c-4067-909b-460e986e2229", "label": "Pick station 3" },
        "highlights": 412
    }));
    assert_eq!(context.panel, ContextPanel::Map);
    let home: PageContext = round_trip(json!({
        "route": "/", "panel": "home", "title": "Home", "seen": [], "selection": null, "highlights": null
    }));
    assert!(home.selection.is_none() && home.highlights.is_none());
}

/// Spec 03 section 3.3's tool schemas: an input takes the model's strings as written, and `additionalProperties`
/// false refuses a field no schema names.
#[test]
fn a_panel_tool_input_takes_the_models_strings_and_refuses_unknown_fields() {
    let input: PanelShowInput = serde_json::from_value(json!({
        "panel": "runs", "location": "run:01KN0TJZANP5N0PGJD8F7A3FDB", "sim_ms": 90000
    }))
    .expect("an input with the schema's fields reads");
    assert_eq!(input.panel, Panel::Runs);
    let target: PanelTarget = input.location.expect("the location was given").parse().expect("it is an object key");
    assert!(matches!(target, PanelTarget::Object(_)));

    let unknown = serde_json::from_value::<PanelShowInput>(json!({ "panel": "map", "zoom": 3 }));
    assert!(unknown.is_err(), "the schema's additionalProperties is false");
}

/// Spec 03 section 3.3's tool schemas: an option the model leaves out is written back absent, never as `null`, which
/// no option's type admits.
#[test]
fn a_panel_tool_input_writes_back_only_the_options_given() {
    let (map, run) = ("customer/demo/site/lshape-6/map@main", "01KN0TJZANP5N0PGJD8F7A3FDB");
    let group = json!({ "refs": ["b6a9a763-687c-4067-909b-460e986e2229"], "note": "where totes arrive" });
    round_trip::<PanelShowInput>(json!({ "panel": "runs" }));
    round_trip::<PanelShowInput>(json!({
        "panel": "flow", "location": map, "select": "b6a9a763-687c-4067-909b-460e986e2229", "highlight": [group],
        "layers": ["material"], "draft": run, "sim_ms": 90000, "compare": [run, run], "metrics": ["picks"]
    }));
    round_trip::<PanelHighlightInput>(json!({ "highlight": [group] }));
    round_trip::<PanelSelectInput>(json!({ "panel": "runs", "select": format!("run:{run}") }));
    round_trip::<PanelClearInput>(json!({}));
    round_trip::<PanelClearInput>(json!({ "panel": "map", "location": map }));
}

/// Spec 01 section 3.5: the envelope's example reads and writes back unchanged, and its `v` is always 1.
#[test]
fn the_envelope_reads_and_writes_back_and_refuses_another_version() {
    let envelope = json!({
        "v": 1, "seq": 412, "at": "2026-10-18T19:23:24.512Z", "conversation": "01K3RF9EXP94DCPTSWSSVB8SW8",
        "turn": "01KTJ153M7D109TG4G7BBVCZYW", "kind": "tool.finished", "caused_by": "toolu_01A",
        "body": { "call": "toolu_01A", "outcome": "ok", "summary": "Read the flow at changeset 3", "refs": [] }
    });
    let entry: JournalEntry = round_trip(envelope.clone());
    assert_eq!(entry.kind, JournalKind::ToolFinished);
    assert_eq!(entry.seq.get(), 412);

    let mut v2 = envelope;
    v2["v"] = json!(2);
    assert!(serde_json::from_value::<JournalEntry>(v2).is_err());
}

/// Spec 01 section 3.5: every journaled kind travels under its name in the table of kinds.
#[test]
fn every_journal_kind_is_written_as_its_wire_name() {
    for kind in JournalKind::ALL {
        assert_eq!(serde_json::to_value(kind).expect("a kind writes"), json!(kind.as_str()));
    }
}

/// Spec 01 section 3.14: a problem carries its type's title and status, and leaves out an `instance` it lacks.
#[test]
fn a_problem_carries_its_types_title_and_status() {
    let problem = Problem::new(ProblemType::IdempotencyKeyReused, "Key k1 was first sent with another body.").at("/x");
    assert_eq!(
        serde_json::to_value(&problem).expect("a problem writes"),
        json!({ "type": "idempotency-key-reused", "title": "That Idempotency-Key was used for another request",
                "status": 422, "detail": "Key k1 was first sent with another body.", "instance": "/x" })
    );
    let without_instance = serde_json::to_value(Problem::new(ProblemType::NotFound, "No route.")).expect("it writes");
    assert!(without_instance.get("instance").is_none(), "an absent instance is left out, never null");
}

/// Spec 03 section 3.1, rule 6: matching is by key, never by pin, so a reference's key drops its pin.
#[test]
fn a_references_key_drops_its_pin() {
    let reference: Ref = "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10@tag:v2".parse().expect("a lateral reference");
    assert_eq!(reference.key().to_string(), "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10");
    let scope: Ref = "scope:template/site/l-shape@tag:v3".parse().expect("a scope reference");
    assert_eq!(scope.key(), "scope:template/site/l-shape".parse::<Key>().expect("a scope key"));
}

/// Spec 03 section 6: `DeclaredId::mint` is the one place a new entity id is made, and it is a random v4.
#[test]
fn a_minted_id_is_a_random_v4_that_reads_back() {
    let (first, second) = (DeclaredId::mint(), DeclaredId::mint());
    assert_ne!(first, second);
    assert_eq!(first.as_uuid().get_version_num(), 4);
    assert_eq!(first.to_string().parse::<DeclaredId>().expect("its text reads back"), first);
}
