//! The development seed, `data/seed/studio/` (spec 04 section 3.13's manifest; spec 05 section 7, its entries): every
//! occurrence of one of Agent Studio's eleven components validates against its definition in design/vocabulary
//! (copied into the fixtures), every top-level component is one world-proto-server's registry defines, every id
//! is a random v4 minted once, and every document depends only on documents seeded before it. The customer seed,
//! `data/seed/customer/`, is held to the first and the last, its `operator`, `fence` and `light_curtain` against
//! their definitions in `data/seed/application/components`, which the fixtures do not copy: its map's older ids are
//! v5, and the fixtures' registry holds no schema for those three.

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::{Resource, Validator};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value, json};
use studio_protocol::DeclaredId;

/// The components world-proto's demo seed defines at master 60aa424 (`server/scripts/seed-demo.sh`, `vocabulary`).
const CORE: [&str; 35] = [
    "frame",
    "pose",
    "region",
    "cell",
    "tiling",
    "flow",
    "stitch",
    "highway",
    "pick",
    "induct",
    "external_ids",
    "rack",
    "relay",
    "access",
    "access_cell",
    "slot",
    "storage",
    "station",
    "queue",
    "port",
    "template",
    "stamp",
    "robot_class",
    "cohort",
    "robot",
    "carrier",
    "carrier_type",
    "carrier_pool",
    "charging",
    "lineage",
    "waiting",
    "presentation_point",
    "sector",
    "demand",
    "easyloop",
];

/// Each of the eleven definitions' schemas, compiled with the server's definitions document as the server does, and
/// each one in `data/seed/application/components` the fixtures do not copy; where both hold one, the fixtures' copy is
/// collected last and kept.
fn vocabulary() -> FxHashMap<String, Validator> {
    let defs = support::json(&support::fixtures().join("registry/server-defs.json"));
    let resource = Resource::from_contents(defs).unwrap();
    support::application_definitions()
        .into_iter()
        .chain(support::vocabulary())
        .map(|definition| {
            let mut options = jsonschema::options();
            options.with_resource("urn:world-graph-server:defs", resource.clone());
            (definition["component"].as_str().unwrap().to_owned(), options.build(&definition["schema"]).unwrap())
        })
        .collect()
}

/// Both seeds: the development seed, then the customer seed.
fn seeds() -> [PathBuf; 2] {
    [support::seed_dir(), support::customer_seed_dir()]
}

fn documents(seed: &Path) -> Vec<(String, PathBuf, Value)> {
    support::manifest_in(seed)
        .into_iter()
        .map(|(scope, path)| {
            let document = support::json(&path);
            (scope.to_string(), path, document)
        })
        .collect()
}

/// Every component map of an entity: its own, then each internal's inside a `template` it carries.
fn component_maps(entity: &Value) -> Vec<&Map<String, Value>> {
    let Some(own) = entity.get("components").and_then(Value::as_object) else { return Vec::new() };
    let mut out = vec![own];
    let internals = own.get("template").and_then(|template| template.get("entities")).and_then(Value::as_array);
    for internal in internals.into_iter().flatten() {
        out.extend(internal.get("components").and_then(Value::as_object));
    }
    out
}

/// Spec 05 section 7 and the brief's acceptance: every component of a seed document outside the core validates
/// against its definition, and one with no definition fails.
#[test]
fn every_seed_component_of_ours_validates_against_its_definition() {
    let schemas = vocabulary();
    let mut checked = 0;
    for (scope, path, document) in seeds().iter().flat_map(|seed| documents(seed)) {
        for entity in document["entities"].as_array().unwrap() {
            for components in component_maps(entity) {
                for (name, value) in components.iter().filter(|(name, _)| !name.starts_with('$')) {
                    let Some(validator) = schemas.get(name) else {
                        assert!(
                            CORE.contains(&name.as_str()),
                            "{scope} ({}): `{name}` has no definition",
                            path.display()
                        );
                        continue;
                    };
                    let faults: Vec<String> = validator.iter_errors(value).map(|fault| fault.to_string()).collect();
                    assert!(faults.is_empty(), "{scope} ({}): `{name}`: {faults:?}", path.display());
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 0);
}

/// What world-proto-server holds each written component to (`server/server/src/storage/apply.rs`,
/// `validate_payloads`, at 60aa424): every top-level component of every seed document validates against the schema
/// the registry holds for it, the protocol's as derived at that master and Agent Studio's eleven. A piece's
/// internals are held to it inside its `template`, `$param` placeholders and all, which is where the refusal of
/// 2026-10-19 stood (`{"key": {"$param": "queue"}, "local_ref": "patch"}` in a queue's path).
#[test]
fn every_seed_component_validates_against_the_registered_schema() {
    let registry = support::registry();
    for (scope, path, document) in documents(&support::seed_dir()) {
        for entity in document["entities"].as_array().unwrap() {
            for (name, value) in entity["components"].as_object().unwrap() {
                let held = registry.component(name).and_then(|component| component.schema.as_ref());
                assert!(held.is_some(), "{scope}: the fixtures hold no registered schema for `{name}`");
                if let Err(why) = registry.validate(name, value) {
                    panic!("{scope} ({}), entity {}: {why}", path.display(), entity["id"]);
                }
            }
        }
    }
}

/// World-proto-server refuses a write that carries a component its registry does not define.
#[test]
fn every_top_level_component_is_registered() {
    let ours: BTreeSet<String> =
        support::vocabulary().iter().map(|d| d["component"].as_str().unwrap().to_owned()).collect();
    for (scope, _, document) in documents(&support::seed_dir()) {
        for entity in document["entities"].as_array().unwrap() {
            for name in entity["components"].as_object().unwrap().keys().filter(|name| !name.starts_with('$')) {
                assert!(
                    CORE.contains(&name.as_str()) || ours.contains(name),
                    "{scope}: `{name}` is no registered component"
                );
            }
        }
    }
}

/// Every id is a random v4 minted once when its file was written: v4-shaped, and never two entities or documents
/// sharing one.
#[test]
fn every_id_is_a_v4_minted_once() {
    let mut seen: FxHashSet<String> = FxHashSet::default();
    for (scope, _, document) in documents(&support::seed_dir()) {
        let ids = document["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entity| entity.get("id"))
            .chain([&document["doc"]["id"]]);
        for id in ids {
            let id = id.as_str().unwrap();
            let parsed = id.parse::<DeclaredId>().unwrap_or_else(|fault| panic!("{scope}: {fault}"));
            assert_eq!(parsed.as_uuid().get_version_num(), 4, "{scope}: {id}");
            assert!(seen.insert(id.to_owned()), "{scope}: {id} is minted twice");
        }
    }
}

/// Each seed's manifest lists every document under its folder once, each scope once, and in dependency order: a
/// `depends_on` names only documents seeded before it.
#[test]
fn each_manifest_is_whole_and_in_dependency_order() {
    for seed in seeds() {
        let entries = documents(&seed);
        let mut seeded: FxHashMap<String, String> = FxHashMap::default();
        let (mut scopes, mut listed) = (BTreeSet::new(), BTreeSet::new());
        for (scope, path, document) in &entries {
            assert!(scopes.insert(scope.clone()), "{scope} is seeded twice");
            assert!(listed.insert(path.canonicalize().unwrap()), "{} is listed twice", path.display());
            for dependency in document["doc"]["depends_on"].as_array().unwrap() {
                assert!(
                    seeded.contains_key(dependency.as_str().unwrap()),
                    "{scope} depends on {dependency}, seeded later or never"
                );
            }
            seeded.insert(document["doc"]["id"].as_str().unwrap().to_owned(), scope.clone());
        }
        let mut on_disk = BTreeSet::new();
        walk(&seed, &mut on_disk);
        assert_eq!(listed, on_disk, "{}", seed.display());
    }
}

fn walk(dir: &Path, out: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.to_string_lossy().ends_with(".world.json") {
            out.insert(path.canonicalize().unwrap());
        }
    }
}

/// Storage accepts a historical model without a kind, while stated models and
/// figures must be meaningful. Studio readiness separately requires the model.
#[test]
fn catalog_registry_accepts_unstated_kind_but_refuses_unknown_kind_and_nonpositive_speed() {
    let registry = support::registry();
    let document = support::json(&support::seed_dir().join("catalog/agent-d30.world.json"));
    let drive = &document["entities"][0]["components"]["floor_drive"];
    let mut missing = drive.clone();
    missing.as_object_mut().unwrap().remove("kind");
    assert!(
        registry.validate("floor_drive", &missing).is_ok(),
        "storage may hold an incomplete historical model; Studio readiness is stricter"
    );
    let mut unknown = drive.clone();
    unknown["kind"] = Value::String("teleport".into());
    assert!(registry.validate("floor_drive", &unknown).is_err());
    let mut zero = drive.clone();
    zero["loaded"]["max_speed_mm_s"] = Value::from(0);
    assert!(registry.validate("floor_drive", &zero).is_err());
    for load in ["loaded", "empty"] {
        for invalid in [json!(0), json!(-1), Value::Null] {
            let mut cruise = drive.clone();
            cruise[load]["constant_speed_mm_s"] = invalid;
            assert!(registry.validate("floor_drive", &cruise).is_err());
        }
    }
}
