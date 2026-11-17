//! Application behaviour retained from the former WPS samples, run against Foundry-owned modules.
use foundry_generators_parity::{anchor_object, cohort_module, expanded_form};
use serde_json::{json, Value};
use std::path::PathBuf;
use worldgen_host::{Budget, Engine, HostError, ImportPolicy, Module, Session};
const COHORT: &str = "c0000000-0000-4000-8000-0000000000ff";
const CLASS: &str = "b1000000-0000-4000-8000-0000000000a1";
const SECTOR: &str = "b1000000-0000-4000-8000-0000000000e1";

#[test]
fn one_cohort_shows_the_keys_and_the_bytes_of_its_robots() {
    let module = loaded();
    let manifest = module.describe(Budget::default()).expect("a manifest");
    assert_eq!((manifest.name.as_str(), manifest.version.as_str()), ("cohort", "0.1.0"));
    assert_eq!(manifest.writes, ["robot"]);

    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let cohort = json!({ "robot_class": CLASS, "count": 2, "assignment": { "kind": "hard", "to": [SECTOR] } });
    let made = session.generate("cohort", &anchor_object(COHORT, json!({ "cohort": cohort }))).expect("two robots");
    let robot = |index: usize| {
        format!(
            r#"{{"robot":{{"assignment":{{"kind":"hard","to":["{SECTOR}"]}},"cohort":"{COHORT}","index":{index},"robot_class":"{CLASS}"}}}}"#
        )
    };
    assert_eq!(
        made.iter().map(|one| (one.key.clone(), one.components.clone())).collect::<Vec<_>>(),
        [("robot:0".to_string(), robot(0)), ("robot:1".to_string(), robot(1))],
    );
    assert!(made.iter().all(|one| one.name.is_none()), "a robot has nothing to call itself");
}

#[test]
fn a_cohort_the_reader_refuses_and_native_reader_both_refuse() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let cases = [
        (
            json!({ "robot_class": "a class", "count": 1 }),
            "`a class` is not an entity identity: a declared identity is a UUID",
        ),
        (
            json!({ "robot_class": format!("{CLASS}/Opaque/"), "count": 1 }),
            "derivation key `Opaque/` is malformed at segment ``: a key segment must not be empty",
        ),
        (
            json!({ "robot_class": format!("{CLASS}/robot@1"), "count": 1 }),
            "derivation key `robot@1` is malformed at segment `robot@1`: a key segment must contain neither `/` nor `@`",
        ),
        (
            json!({ "robot_class": format!("{CLASS}/"), "count": 1 }),
            "derivation key `` is malformed at segment ``: a derivation key must name at least one segment",
        ),
        (json!({ "robot_class": 5, "count": 1 }), "invalid type: integer `5`, expected a string"),
        (json!({ "robot_class": CLASS }), "missing field `count`"),
        (json!({ "robot_class": CLASS, "count": 1, "assignment": { "to": [] } }), "missing field `kind`"),
        (
            json!({ "robot_class": CLASS, "count": 1, "assignment": { "kind": "hard", "to": ["a sector"] } }),
            "`a sector` is not an entity identity: a declared identity is a UUID",
        ),
    ];
    for (index, (cohort, wrong)) in cases.into_iter().enumerate() {
        let said = refusal(&mut session, &cohort);
        assert_eq!(said, format!("entity {COHORT}: component `cohort` does not parse: {wrong}"), "case {index}");
        let reader = reader_refusal(&format!("cohort-refused-{index}"), &cohort);
        assert!(reader.contains(COHORT) && reader.contains("component `cohort`"),
            "case {index}: native must refuse this component; reader said {reader:?}, generator {said:?}");
    }
}

#[test]
fn a_count_that_is_no_count_is_refused_by_both_naming_each_one_s_type() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let cohort = json!({ "robot_class": CLASS, "count": -1 });
    let said = refusal(&mut session, &cohort);
    let common = format!("entity {COHORT}: component `cohort` does not parse: invalid value: integer `-1`, expected ");
    assert_eq!(said, format!("{common}u64"));
    let reader = reader_refusal("cohort-negative-count", &cohort);
    assert!(reader.ends_with(&format!("{common}usize")), "{reader}");
}

#[test]
fn a_count_past_thirty_two_bits_is_read_as_the_reader_reads_it() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = anchor_object(COHORT, json!({ "cohort": { "robot_class": CLASS, "count": 4_294_967_296u64 } }));
    match session.generate("cohort", &object).expect_err("more robots than one call may make") {
        HostError::OutOfFuel { .. } | HostError::OutOfMemory { .. } => {}
        other => panic!("expected the budget to stop the loop, got {other}"),
    }
}

#[test]
fn opaque_derived_catalog_references_are_preserved_verbatim() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).unwrap();
    let target = format!("{CLASS}/DeckA/42/north-port/Δ");
    let made = session.generate("cohort", &anchor_object(COHORT, json!({"cohort":{
        "robot_class":target, "count":1
    }}))).unwrap();
    let value: Value = serde_json::from_str(&made[0].components).unwrap();
    assert_eq!(value["robot"]["robot_class"], target);
}

fn loaded() -> Module {
    let engine = Engine::new().expect("an engine");
    Module::load(&engine, cohort_module(), ImportPolicy::TrapOnUse).expect("the cohort component loads")
}

fn refusal(session: &mut Session, cohort: &Value) -> String {
    match session.generate("cohort", &anchor_object(COHORT, json!({ "cohort": cohort }))).expect_err("a refusal") {
        HostError::Refused { message, .. } => message,
        other => panic!("expected a refusal, got {other}"),
    }
}

fn reader_refusal(name: &str, cohort: &Value) -> String {
    let frame = "6a2f8c41-0d93-4e57-b1c8-2f7e9a3d5c06";
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "4c7e1a90-3b25-4d68-9f0e-8a21c6d5b3f8", "name": name, "kind": "world" },
        "primary_frame": frame,
        "entities": [
            { "id": frame, "components": { "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } } },
            { "id": CLASS, "components": { "robot_class": { "role": "htm" } } },
            { "id": SECTOR, "components": { "sector": { "members": [] } } },
            { "id": COHORT, "components": { "cohort": cohort } },
        ],
    });
    let path = written(name, &document);
    expanded_form(&path).expect_err("the reader refuses this cohort").to_string()
}

fn written(name: &str, document: &Value) -> PathBuf {
    let directory = std::env::temp_dir().join("seed-generators-fleet");
    std::fs::create_dir_all(&directory).expect("somewhere to write a document");
    let path = directory.join(format!("{name}.world.json"));
    std::fs::write(&path, document.to_string()).expect("a document on disk");
    path
}
