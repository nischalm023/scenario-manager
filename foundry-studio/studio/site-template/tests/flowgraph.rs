//! The engine and the registry held to spec 06's flow compiler and its fixtures, both dessim's, from this side since
//! dessim depends on no Studio crate: the L-Shape fixture is the engine's own output (statement 2's document); each
//! seeded shell at its defaults compiles to the relay flow's roots (section 8's request to spec 05); and each flow
//! component's registered schema accepts the examples of sections 3.1 and 6 and statement 4's fixture and refuses
//! each missing required field, while its reference annotations extract every reference those graphs hold, no more,
//! no fewer (statement 1).

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use dessim::composition::flowgraph::{Request, compile};
use dessim::composition::{Composition, RootTask};
use dessim::worldmap::flows::read as read_flows;
use dessim::worldmap::{load, load_from_bytes};
use native_modules as _;
use rustc_hash::FxHashSet;
use serde_json::{Value, json};
use site_template::{DocId, KnobValues, Mint, Prior, SourceKey, evaluate};

const L_SHAPE: &str = "l_shape_6_aisles.json";
const LITMUS: &str = "store_replenishment.json";
const SYSTEM: &str = "template/site/l-shape";

/// The authored litmus fixture; the engine-generated L-Shape comes from the WPS snapshot.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/flowgraph")
}

fn l_shape_snapshot() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json")
}

/// The L-Shape at the demo story's knobs (spec 05 section 5.5; lead decision 40 in LEAD-DECISIONS-IN-ABSENCE.md:
/// the seeded default stays 4 aisles, the story turns it to 6 and widens the shell to fit).
fn knobs() -> KnobValues {
    KnobValues::new().with("aisles", 6).with("width", 54400)
}

/// Evaluate the L-Shape with the prior identities, in the identity order used by WPS export.
fn made(prior: &Prior) -> Value {
    let read = support::read_template("l-shape");
    let evaluation = evaluate(&read, &knobs(), prior, &mut Mint);
    assert!(evaluation.problems.is_empty(), "the L-Shape at 6 aisles evaluates cleanly: {:?}", evaluation.problems);
    let mut document = evaluation.document(&read).as_value().clone();
    document["entities"].as_array_mut().unwrap().sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    document
}

/// The prior a written fixture implies: its document id and every made entity's id by its source key.
fn prior_of(document: &Value) -> Prior {
    let mut prior = Prior::new(document["doc"]["id"].as_str().unwrap().parse::<DocId>().unwrap());
    for entity in document["entities"].as_array().unwrap() {
        let Some(key) = entity.pointer(&format!("/components/external_ids/{}", SYSTEM.replace('/', "~1"))) else {
            continue;
        };
        let key: SourceKey = key.as_str().unwrap().parse().unwrap();
        prior.ids.insert(key, entity["id"].as_str().unwrap().parse().unwrap());
    }
    prior
}

/// Spec 06 section 9, statement 2's document: current engine content survives WPS unchanged. Only the
/// server/client's explicit assembly provenance is removed; catalog values and all authored comments stay exact.
#[test]
fn the_l_shape_snapshot_is_the_engines_output() {
    let mut written = support::json(&l_shape_snapshot());
    let provenance = written["doc"].as_object_mut().unwrap().remove("assembled_from").expect("WPS provenance");
    assert!(!provenance.as_array().unwrap().is_empty(), "the snapshot records its WPS scope and changeset");
    assert_eq!(
        made(&prior_of(&written)),
        written,
        "{L_SHAPE} differs from the current engine; regenerate its authored seed and export it through WPS"
    );
}

/// Refresh the authored input while keeping the snapshot's identities. This never writes the snapshot: import
/// the seed into WPS and capture a strictly compared export before replacing it (see data/seed/worlds/flowgraph).
#[test]
#[ignore = "writes the authored seed for a subsequent WPS import/export"]
fn write_l_shape_seed() {
    let prior = prior_of(&support::json(&l_shape_snapshot()));
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/worlds/flowgraph/l_shape_6_aisles.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string_pretty(&made(&prior)).unwrap() + "\n").unwrap();
}

/// Every shell template's default flow is a putaway-and-pick relay flow, so each of the five seeded shells,
/// evaluated at its defaults, compiles in validation mode with no problem to the seven roots of the relay flow.
#[test]
fn every_shell_at_its_defaults_compiles_to_the_relay_roots() {
    let roots = |tasks: &[RootTask]| tasks.iter().map(|task| task.task_type.clone()).collect::<Vec<_>>();
    let cencora = roots(&Composition::Named("cencora".into()).resolve().unwrap().tasks);
    for shell in ["rectangle", "square", "l-shape", "u-shape", "i-shape"] {
        let read = support::read_template(shell);
        let evaluation = evaluate(&read, &KnobValues::new(), &Prior::new(DocId::mint()), &mut Mint);
        let world = load_from_bytes(&evaluation.document(&read).to_bytes()).expect("the shell loads");
        let compiled = compile(&world, &read_flows(&world.world), &Request::validation());
        assert!(compiled.problems.is_empty(), "{shell}: {:#?}", compiled.problems);
        assert_eq!(roots(&compiled.plan.expect("a plan").composition.tasks), cencora, "{shell}");
    }
}

/// Section 3.1's example: the demo site's stock-tote delivery.
fn section_3_1() -> Vec<(&'static str, Value)> {
    vec![
        (
            "flow.node",
            json!({ "kind": "stations", "sectors": ["0f2c7e0d-20ac-472f-bce2-2847d9a4f264@head"],
            "process": { "operation": "pick", "gives_by": ["3f21fb54-2c7a-40ee-951e-9cfa8fb7b674"], "receives_in": "station_bins" },
            "work": { "pick_sim_ms": 7000, "sequencing": "all_totes" },
            "intake": [{ "moves": "inventory_tote:stock", "admission_factor": 4.0 }] }),
        ),
        (
            "flow.material_edge",
            json!({ "from": "321d8797-7ac8-4e48-902b-e9c62308d2d7",
            "to": "b6a9a763-687c-4067-909b-460e986e2229", "moves": "inventory_tote:stock" }),
        ),
        (
            "flow.agent_segment",
            json!({ "edge": "3f21fb54-2c7a-40ee-951e-9cfa8fb7b674", "step": 2,
            "from": { "node": "321d8797-7ac8-4e48-902b-e9c62308d2d7", "port": "relay" },
            "to": { "node": "b6a9a763-687c-4067-909b-460e986e2229", "port": "presentation" },
            "executor": { "cohort": "e0e2d658-ffd5-47eb-9676-2191a13ac962@head" }, "at_end": "keeps_through_work",
            "retained_for": ["34ce3690-f0d2-4181-a9b5-038a57792eee", "edd6aebc-afde-48e1-860c-2dde095efaee"] }),
        ),
    ]
}

/// Every flow component of the examples and of both fixtures, each fixture read through dessim's reader.
fn examples() -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> =
        section_3_1().into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
    for path in [l_shape_snapshot(), fixtures().join(LITMUS)] {
        let world = load(&path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
        for entity in read_flows(&world.world).entities {
            out.extend(entity.flow_components().map(|(name, value)| (name.to_owned(), value.clone())));
        }
    }
    out
}

/// The fields whose absence the schema refuses for this value: the required ones, and those its kind requires.
fn required(component: &str, value: &Value) -> Vec<&'static str> {
    match component {
        "flow.node" => {
            let mut fields = vec!["kind"];
            match value["kind"].as_str() {
                Some("stations" | "conveyance") => fields.extend(["sectors", "process"]),
                Some("storage") => fields.extend(["sectors", "holds"]),
                Some("source") => fields.push("demand"),
                _ => {}
            }
            let operation = value["process"]["operation"].as_str();
            if matches!(operation, Some("pick" | "put" | "extract")) || value["holds"] == "closed_order" {
                fields.push("work");
            }
            fields
        }
        "flow.material_edge" => vec!["from", "to", "moves"],
        "flow.agent_segment" if value["carries"] == "nothing" => vec!["from", "to", "executor", "at_end"],
        "flow.agent_segment" => vec!["from", "to", "executor", "at_end", "edge", "step"],
        "flow.return_rule" => vec!["min_backlog"],
        other => panic!("{other} is not a flow component"),
    }
}

#[test]
fn each_schema_accepts_the_examples_and_refuses_each_missing_required_field() {
    let registry = support::registry();
    let examples = examples();
    assert_eq!(examples.len(), 3 + 27 + 63, "section 3.1's three, the relay flow's 27, the litmus flow's 63");
    for (component, value) in &examples {
        registry.validate(component, value).unwrap_or_else(|fault| panic!("{fault}: {value}"));
        for field in required(component, value) {
            let mut without = value.clone();
            without.as_object_mut().unwrap().remove(field);
            assert!(registry.validate(component, &without).is_err(), "{component} without {field} is refused: {value}");
        }
    }
}

/// The values at an annotation's path: a JSON pointer whose trailing `/*` stands for each element of a list.
fn at<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    match path.strip_suffix("/*") {
        Some(list) => {
            value.pointer(list).and_then(Value::as_array).map(|items| items.iter().collect()).unwrap_or_default()
        }
        None => value.pointer(path).into_iter().collect(),
    }
}

/// Every string in a component that names an entity: a UUID, bare or pinned.
fn identities(value: &Value, out: &mut FxHashSet<String>) {
    match value {
        Value::String(text) => {
            let id = text.split('@').next().unwrap_or_default();
            if id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4 {
                out.insert(text.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|item| identities(item, out)),
        Value::Object(fields) => fields.values().for_each(|field| identities(field, out)),
        _ => {}
    }
}

#[test]
fn the_annotations_extract_every_reference_and_nothing_else() {
    let registry = support::registry();
    for (component, value) in examples() {
        let mut annotated = FxHashSet::default();
        for annotation in registry.refs(&component) {
            for reference in at(&value, &annotation.path) {
                annotated.insert(reference.as_str().expect("a reference is a string").to_owned());
            }
        }
        let mut held = FxHashSet::default();
        identities(&value, &mut held);
        assert_eq!(annotated, held, "{component}: {value}");
    }
}
