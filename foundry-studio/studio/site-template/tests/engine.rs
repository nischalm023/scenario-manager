//! Spec 05 section 9's engine statements, held on a small probe template built here: a floor, a piece placed on a
//! grid, a sector naming the placement, a stitch with explicit pairs into the products, a cohort in the fleet
//! scope pinning a catalog class.

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use rustc_hash::FxHashMap;
use serde_json::{Value, json};
use site_template::{
    Count, Declaration, DocId, Evaluation, IdSource, KnobValue, KnobValues, Mint, Prior, ProblemCode, Record,
    RefAnnotation, RegisteredComponent, ScopeSnapshot, SourceKey, TemplateRead, evaluate,
};
use studio_protocol::{DeclaredId, Seq};
use site_model::document::Document;
use site_model::pipeline::derive;
use site_model::pipeline::layers::LayerSet;

/// The probe's ids, each minted once per test.
struct Ids(FxHashMap<&'static str, DeclaredId>);

impl Ids {
    fn new() -> Self {
        Self(FxHashMap::default())
    }

    fn get(&mut self, name: &'static str) -> String {
        self.0.entry(name).or_insert_with(DeclaredId::mint).to_string()
    }
}

fn knob(key: &str, unit: &str, min: i64, max: i64, step: i64, default: i64) -> Value {
    json!({ "template.knob": { "key": key, "label": key, "unit": unit, "min": min, "max": max, "step": step, "default": default } })
}

/// The probe template's documents, root first, then the catalog's; `edit` changes any of them before they are read.
fn probe(ids: &mut Ids, unit_mm: f64, edit: impl FnOnce(&mut Ids, &mut BTreeMap<&'static str, Value>)) -> TemplateRead {
    let (site, floor, piece, grid) = (ids.get("site"), ids.get("floor"), ids.get("piece"), ids.get("grid"));
    let (post, class) = (ids.get("post"), ids.get("class"));
    let mut documents: BTreeMap<&'static str, Value> = BTreeMap::new();
    documents.insert("template/site/probe", json!({ "doc": { "id": ids.get("doc:root") }, "primary_frame": site, "entities": [
        { "id": ids.get("rows"), "components": knob("rows", "count", 0, 10, 1, 3) },
        { "id": ids.get("cols"), "components": knob("cols", "count", 0, 5, 1, 2) },
        { "id": ids.get("len"), "components": knob("len", "mm", 850, 85000, 850, 8500) },
        { "id": ids.get("off"), "components": knob("off", "count", 0, 1, 1, 0) },
        { "id": ids.get("card"), "components": { "template.description": { "title": "Probe", "text": "A probe of the engine." } } },
    ]}));
    documents.insert("template/site/probe/map", json!({ "doc": { "id": ids.get("doc:map") }, "entities": [
        { "id": site, "name": "site", "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": unit_mm } } },
        { "id": floor, "name": "floor", "components": {
            "region": { "frame": site, "origin": [0, 0], "size": [8500, 1700] },
            "tiling": { "pitch": [850, 850], "derives": { "cell": {} } },
            "template.knob_binding": { "/region/size/0": "len" } } },
        { "id": piece, "name": "cell piece", "components": { "template": { "params": {}, "entities": [
            { "local": "floor", "components": { "region": { "origin": [0, 0], "size": [1700, 850] }, "tiling": { "pitch": [850, 850] } } },
            { "local": "post", "components": { "pose": { "position": [0, 900] }, "stamp": { "template": format!("{post}@1") } } },
            { "local": "tag", "components": { "sector": { "members": [{ "local_ref": "floor" }] } } } ] } } },
        { "id": grid, "name": "grid", "components": {
            "pose": { "frame": site, "position": [0, 5000], "heading_deg": 0 },
            "template.placement": { "piece": piece, "repeat": { "count": [3, 2], "pitch": [2000, 2000] }, "stands_for": ["floor"] },
            "template.knob_binding": { "/template.placement/repeat/count/0": "rows", "/template.placement/repeat/count/1": "cols" } } },
        { "id": ids.get("floors"), "name": "floors", "components": { "sector": { "members": [grid] } } },
        { "id": ids.get("pairs"), "name": "pairs", "components": { "stitch": {
            "a": floor, "b": format!("{grid}/instance:0,0/local:floor"), "rule": "explicit",
            "pairs": [[{ "generator": floor, "key": "cell:0,0" }, { "generator": grid, "key": "instance:0,0/local:floor/cell:0,0" }]] } } },
        { "id": ids.get("lane"), "name": "lane", "components": { "queue": { "path": [format!("{grid}/instance:1,0/local:floor/cell:1,0")] } } },
        { "id": ids.get("ghost"), "name": "ghost", "components": {
            "region": { "frame": site, "origin": [0, 3000], "size": [850, 850] },
            "template.presence": { "when": "off" },
            "template.knob_binding": { "/region/size/0": "850 / off" } } },
    ]}));
    documents.insert("template/site/probe/fleet", json!({ "doc": { "id": ids.get("doc:fleet") }, "entities": [
        { "id": ids.get("crew"), "name": "crew", "components": { "cohort": {
            "robot_class": format!("{class}@1"), "count": 2, "assignment": { "kind": "hard", "to": [ids.get("floors")] } } } },
    ]}));
    documents.insert("catalog/agent/probe", json!({ "doc": { "id": ids.get("doc:class") }, "entities": [
        { "id": class, "components": { "robot_class": { "role": "vtm", "model": "probe", "configuration": "standard", "carries": ["tote"] } } } ]}));
    documents.insert(
        "catalog/artifact/probe-post",
        json!({ "doc": { "id": ids.get("doc:post") }, "entities": [
        { "id": post, "components": { "template": { "params": {}, "entities": [
            { "local": "spot", "components": { "pose": { "position": [0, 0] }, "cell": {} } } ] } } } ]}),
    );
    edit(ids, &mut documents);
    let read = |path: &str| {
        let mut document = documents[path].clone();
        document["proto_version"] = json!("0.7.0");
        ScopeSnapshot::from_document(path.parse().unwrap(), Seq::new(1), &document).unwrap()
    };
    TemplateRead {
        template: "template/site/probe".parse().unwrap(),
        tag: None,
        scopes: ["template/site/probe", "template/site/probe/map", "template/site/probe/fleet"].map(read).into(),
        catalog: ["catalog/agent/probe", "catalog/artifact/probe-post"].map(read).into(),
        registry: support::registry(),
    }
}

fn run(read: &TemplateRead, knobs: KnobValues, prior: &Prior) -> Evaluation {
    evaluate(read, &knobs, prior, &mut Mint)
}

fn codes(evaluation: &Evaluation) -> Vec<ProblemCode> {
    evaluation.problems.iter().map(|problem| problem.code).collect()
}

/// Every made entity, keyed by source key, with every made id in its components replaced by that id's key: two
/// evaluations compared this way agree under canonical keys whatever ids they drew.
fn canonical(evaluation: &Evaluation) -> BTreeMap<String, String> {
    let ids = evaluation.ids();
    let mut out = BTreeMap::new();
    for plan in evaluation.plan.values() {
        for declaration in &plan.declarations {
            let mut text = Value::Object(declaration.components.clone()).to_string();
            for (key, id) in &ids {
                text = text.replace(&id.to_string(), &format!("key:{key}"));
            }
            out.insert(declaration.key.to_string(), text);
        }
    }
    out
}

fn made<'e>(evaluation: &'e Evaluation, key: &str) -> &'e Declaration {
    evaluation
        .plan
        .values()
        .flat_map(|plan| plan.declarations.iter())
        .find(|declaration| declaration.key.as_str() == key)
        .unwrap_or_else(|| panic!("no made entity keyed {key}"))
}

fn id_of(evaluation: &Evaluation, key: &str) -> String {
    made(evaluation, key).id.to_string()
}

#[test]
fn the_probe_evaluates_without_problems() {
    let read = probe(&mut Ids::new(), 1.0, |_, _| {});
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
}

/// Spec 05 section 9: a placement with either count 0 makes nothing; `count` `[3, 2]` makes six instances at the
/// pitch, keyed `instance:{i},{j}`; growing either count keeps earlier keys.
#[test]
fn a_placements_counts_make_its_instances_and_keep_their_keys() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |_, _| {});
    let grid = ids.get("grid");
    let prior = Prior::new(DocId::mint());
    let six = run(&read, KnobValues::new(), &prior);
    for i in 0..3 {
        for j in 0..2 {
            let floor = made(&six, &format!("{grid}/instance:{i},{j}/local:floor"));
            let origin = &floor.components["region"]["origin"];
            assert_eq!(origin, &json!([i * 2000, 5000 + j * 2000]), "instance {i},{j}");
        }
    }
    assert_eq!(six.census.by_knob.values().copied().collect::<Vec<_>>(), [Count::new(6), Count::new(6)]);
    let none = run(&read, KnobValues::new().with("rows", 0), &prior);
    assert!(none.ids().iter().all(|(key, _)| !key.as_str().starts_with(&format!("{grid}/"))));
    // The probe's stitch names instance (0, 0)'s floor in its single-valued `b` and in a pair.
    assert_eq!(codes(&none), [ProblemCode::ReferenceToAbsentEntity, ProblemCode::ReferenceToAbsentEntity]);

    let grown = run(&read, KnobValues::new().with("rows", 5).with("cols", 3), &prior.clone().after(&six));
    for (key, id) in six.ids() {
        assert_eq!(id_of(&grown, key.as_str()), id.to_string(), "{key}");
    }
}

/// Spec 05 section 9: references rewrite at every protocol reference position: a stitch pair, a product address
/// under a kept tiling, a placement in a list by `stands_for`; catalog versions are untouched.
#[test]
fn references_rewrite_at_every_position() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |_, _| {});
    let (grid, class) = (ids.get("grid"), ids.get("class"));
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    let made_floor = id_of(&evaluation, &ids.get("floor"));
    let product_floor = |i: u64, j: u64| id_of(&evaluation, &format!("{grid}/instance:{i},{j}/local:floor"));

    let stitch = &made(&evaluation, &ids.get("pairs")).components["stitch"];
    assert_eq!(stitch["a"], json!(made_floor));
    assert_eq!(stitch["b"], json!(product_floor(0, 0)));
    assert_eq!(stitch["pairs"][0][0], json!({ "generator": made_floor, "key": "cell:0,0" }));
    assert_eq!(stitch["pairs"][0][1], json!({ "generator": product_floor(0, 0), "key": "cell:0,0" }));

    let lane = &made(&evaluation, &ids.get("lane")).components["queue"]["path"];
    assert_eq!(lane, &json!([format!("{}/cell:1,0", product_floor(1, 0))]));

    let members = &made(&evaluation, &ids.get("floors")).components["sector"]["members"];
    let in_key_order: Vec<String> =
        [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)].iter().map(|&(i, j)| product_floor(i, j)).collect();
    assert_eq!(members, &json!(in_key_order));

    let cohort = &made(&evaluation, &ids.get("crew")).components["cohort"];
    assert_eq!(cohort["robot_class"], json!(format!("{class}@1")));
    let floors = id_of(&evaluation, &ids.get("floors"));
    assert_eq!(cohort["assignment"]["to"], json!([format!("{floors}@head")]), "a kin reference across made scopes");
}

/// Spec 05 section 9: every made entity carries exactly one `external_ids` entry from the engine; no made entity
/// carries a `template.` component; nothing in a made scope references a template scope; source keys are unique.
#[test]
fn made_entities_carry_their_key_and_nothing_of_the_template() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |_, _| {});
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    let template_ids: Vec<String> = ids.0.values().map(DeclaredId::to_string).collect();
    let mut keys = Vec::new();
    for plan in evaluation.plan.values() {
        for declaration in &plan.declarations {
            let external = declaration.components["external_ids"].as_object().unwrap();
            assert_eq!(external.len(), 1);
            assert_eq!(external["template/site/probe"], json!(declaration.key.to_string()));
            assert!(declaration.components.keys().all(|name| !name.starts_with("template.")));
            let mut text = Value::Object(declaration.components.clone()).to_string();
            text = text.replace(&Value::Object(external.clone()).to_string(), "");
            for id in &template_ids {
                assert!(
                    !text.contains(id.as_str())
                        || ids.0.get("class").is_some_and(|c| c.to_string() == *id)
                        || ids.0.get("post").is_some_and(|p| p.to_string() == *id),
                    "{} names template entity {id}",
                    declaration.key
                );
            }
            keys.push(declaration.key.clone());
        }
    }
    let count = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), count);
}

/// Spec 05 section 9: two evaluations at the same knobs with no prior ids are equal under canonical keys; with prior
/// ids they are equal outright. A key removed by one knob change and restored by a later one gets its first id back;
/// in update mode a key whose entity the site deleted earlier gets a fresh one.
#[test]
fn ids_follow_source_keys_for_the_drafts_life() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |_, _| {});
    let grid = ids.get("grid");
    let draft = Prior::new(DocId::mint());
    let first = run(&read, KnobValues::new(), &draft);
    let second = run(&read, KnobValues::new(), &draft);
    assert_eq!(canonical(&first), canonical(&second));
    assert_ne!(first.ids(), second.ids());
    let draft = draft.after(&first);
    assert_eq!(run(&read, KnobValues::new(), &draft).ids(), first.ids());

    let last = format!("{grid}/instance:2,1/local:floor");
    let shrunk = run(&read, KnobValues::new().with("rows", 2), &draft);
    assert!(shrunk.ids().iter().all(|(key, _)| key.as_str() != last));
    let draft = draft.after(&shrunk);
    let restored = run(&read, KnobValues::new(), &draft);
    assert_eq!(id_of(&restored, &last), id_of(&first, &last));

    let mut site_live = draft.clone();
    site_live.ids.remove(&last.parse::<SourceKey>().unwrap());
    let updated = run(&read, KnobValues::new(), &site_live);
    assert_ne!(id_of(&updated, &last), id_of(&first, &last));
}

/// Spec 05 section 9: the evaluation's document has a `doc` block, the made id of the template's primary frame as
/// `primary_frame` and no `depends_on`, and foundry's reader loads it; a piece's internal real stamp stays a stamp.
#[test]
fn the_document_names_the_made_primary_frame_and_loads() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |_, _| {});
    let (site, grid) = (ids.get("site"), ids.get("grid"));
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    let document = evaluation.document(&read);
    let value = document.as_value();
    assert_eq!(value["primary_frame"], json!(id_of(&evaluation, &site)));
    assert_eq!(value["doc"]["id"], json!(evaluation.prologue.doc.to_string()));
    assert!(value["doc"].get("depends_on").is_none());
    let post = made(&evaluation, &format!("{grid}/instance:0,0/local:post"));
    assert!(post.components.contains_key("stamp"), "placements expand one level: the internal stamp stays a stamp");

    let scenario = Document::from_value(Path::new("<probe>"), value).unwrap();
    let world = derive(&LayerSet { scenario, layers: Vec::new() }).unwrap_or_else(|fault| panic!("{fault}"));
    let spot = format!("{}/instance:0/local:spot", post.id);
    assert!(world.entities.keys().any(|id| id.to_string() == spot), "the reader expanded the kept stamp");
}

/// Spec 05 section 9: an absent entity's bindings are never evaluated (one that would divide by zero stays silent),
/// and present, the same binding is the named problem.
#[test]
fn absent_entities_bind_nothing() {
    let read = probe(&mut Ids::new(), 1.0, |_, _| {});
    let absent = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(absent.problems.is_empty(), "{:?}", absent.problems);
    let read = probe(&mut Ids::new(), 1.0, |ids, documents| {
        let ghost = ids.get("ghost");
        let map = documents.get_mut("template/site/probe/map").unwrap();
        for entity in map["entities"].as_array_mut().unwrap() {
            if entity["id"] == json!(ghost) {
                entity["components"]["template.presence"]["when"] = json!("1 - off");
            }
        }
    });
    let present = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&present), [ProblemCode::DivisionByZero]);
}

/// Spec 05 section 9: `mm` knobs become steps at `unit_mm` 1 and 0.1, and one not whole at a coarser step is
/// refused.
#[test]
fn millimetres_become_steps_of_the_primary_frame() {
    let mut ids = Ids::new();
    let at_one = probe(&mut ids, 1.0, |_, _| {});
    let floor = ids.get("floor");
    let size = |evaluation: &Evaluation| made(evaluation, &floor).components["region"]["size"][0].clone();
    assert_eq!(size(&run(&at_one, KnobValues::new(), &Prior::new(DocId::mint()))), json!(8500));
    let mut ids = Ids::new();
    let at_tenth = probe(&mut ids, 0.1, |_, _| {});
    let floor = ids.get("floor");
    let tenths = run(&at_tenth, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(made(&tenths, &floor).components["region"]["size"][0], json!(85000));
    let coarse = probe(&mut Ids::new(), 1000.0, |_, _| {});
    let refused = run(&coarse, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&refused), [ProblemCode::KnobNotWholeSteps]);
    assert!(refused.plan.is_empty());
}

/// Spec 05 section 9: a binding into a frame scaled by 2 is `binding-in-scaled-frame`.
#[test]
fn a_length_bound_into_a_scaled_frame_is_refused() {
    let read = probe(&mut Ids::new(), 1.0, |ids, documents| {
        let (site, scaled, floor) = (ids.get("site"), ids.get("scaled"), ids.get("floor"));
        let map = documents.get_mut("template/site/probe/map").unwrap();
        let entities = map["entities"].as_array_mut().unwrap();
        entities.push(json!({ "id": scaled, "components": { "frame": { "parent": site, "dims": 2, "units": "mm",
            "transform_to_parent": { "translation": [0, 0], "scale": [2, 1] } } } }));
        for entity in entities.iter_mut() {
            if entity["id"] == json!(floor) {
                entity["components"]["region"]["frame"] = json!(scaled);
            }
        }
    });
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&evaluation), [ProblemCode::BindingInScaledFrame]);
}

/// Spec 05 section 9: length times length is `dimension-mismatch`, and the grammar's refusals and an unknown knob
/// make the template invalid.
#[test]
fn a_bad_expression_refuses_the_template() {
    let with_binding = |expression: &'static str| {
        probe(&mut Ids::new(), 1.0, move |ids, documents| {
            let floor = ids.get("floor");
            let map = documents.get_mut("template/site/probe/map").unwrap();
            for entity in map["entities"].as_array_mut().unwrap() {
                if entity["id"] == json!(floor) {
                    entity["components"]["template.knob_binding"]["/region/size/0"] = json!(expression);
                }
            }
        })
    };
    let evaluation = run(&with_binding("len * len"), KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&evaluation), [ProblemCode::DimensionMismatch]);
    for expression in ["-850", "length", "min(len)"] {
        let evaluation = run(&with_binding(expression), KnobValues::new(), &Prior::new(DocId::mint()));
        assert_eq!(codes(&evaluation), [ProblemCode::TemplateInvalid], "{expression}");
        assert!(evaluation.plan.is_empty());
    }
}

/// Spec 05 section 9: a knob declaration with both a range and options, a default off its step, or a duplicate key
/// is `template-invalid`.
#[test]
fn a_bad_knob_refuses_the_template() {
    let with_knob = |component: Value| {
        probe(&mut Ids::new(), 1.0, move |ids, documents| {
            let extra = ids.get("extra");
            let root = documents.get_mut("template/site/probe").unwrap();
            root["entities"].as_array_mut().unwrap().push(json!({ "id": extra, "components": component }));
        })
    };
    let both = json!({ "template.knob": { "key": "both", "label": "Both", "unit": "count", "min": 0, "max": 2, "step": 1,
        "options": [{ "value": 0, "label": "a" }, { "value": 1, "label": "b" }], "default": 0 } });
    let off_step = knob("off_step", "mm", 850, 8500, 850, 900);
    let duplicate = knob("rows", "count", 0, 3, 1, 1);
    for component in [both, off_step, duplicate] {
        let evaluation = run(&with_knob(component.clone()), KnobValues::new(), &Prior::new(DocId::mint()));
        assert!(codes(&evaluation).contains(&ProblemCode::TemplateInvalid), "{component}");
    }
}

/// Spec 05 section 3.2: a count is zero or more, so a placement whose count a binding takes below zero is
/// `template-invalid` at those knobs, never a placement quietly making nothing; at zero it makes nothing.
#[test]
fn a_count_bound_below_zero_refuses_the_template() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |ids, documents| {
        let grid = ids.get("grid");
        let map = documents.get_mut("template/site/probe/map").unwrap();
        for entity in map["entities"].as_array_mut().unwrap() {
            if entity["id"] == json!(grid) {
                entity["components"]["template.knob_binding"]["/template.placement/repeat/count/0"] = json!("rows - 4");
            }
        }
    });
    let below = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&below), [ProblemCode::TemplateInvalid]);
    assert_eq!(below.problems[0].reference, Some(ids.get("grid")));
    assert!(below.plan.is_empty());
    // At zero the stitch's two references to instance (0, 0) name an absent entity, and nothing else is wrong.
    let zero = run(&read, KnobValues::new().with("rows", 4), &Prior::new(DocId::mint()));
    assert_eq!(codes(&zero), [ProblemCode::ReferenceToAbsentEntity, ProblemCode::ReferenceToAbsentEntity]);
}

/// Spec 05 section 9: a piece with an exclusion tiling at heading 90 is refused (section 3.4, rule 4).
#[test]
fn a_piece_with_exclusions_stands_at_heading_zero_only() {
    let turned = |heading: i64| {
        probe(&mut Ids::new(), 1.0, move |ids, documents| {
            let (piece, grid) = (ids.get("piece"), ids.get("grid"));
            let map = documents.get_mut("template/site/probe/map").unwrap();
            for entity in map["entities"].as_array_mut().unwrap() {
                if entity["id"] == json!(piece) {
                    entity["components"]["template"]["entities"][0]["components"]["tiling"]["exclusions"] =
                        json!([{ "origin": [0, 0], "size": [850, 850] }]);
                }
                if entity["id"] == json!(grid) {
                    entity["components"]["pose"]["heading_deg"] = json!(heading);
                }
            }
        })
    };
    let straight = run(&turned(0), KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(straight.problems.is_empty(), "{:?}", straight.problems);
    let quarter = run(&turned(90), KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&quarter), [ProblemCode::TemplateInvalid]);
}

/// Spec 05 section 3.4, rule 1: a piece is valid as world-proto-server stores it, its `template` held to the
/// registered schema. At 60aa424 that schema refuses a `$param` at a reference, here a stitch's `b`, though the
/// protocol's readers bind one there.
#[test]
fn a_piece_the_registry_refuses_refuses_the_template() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |ids, documents| {
        let (piece, grid, floor) = (ids.get("piece"), ids.get("grid"), ids.get("floor"));
        let map = documents.get_mut("template/site/probe/map").unwrap();
        for entity in map["entities"].as_array_mut().unwrap() {
            if entity["id"] == json!(piece) {
                let template = &mut entity["components"]["template"];
                template["params"] = json!({ "floor": {} });
                template["entities"].as_array_mut().unwrap().push(json!({ "local": "join", "components": { "stitch": {
                    "a": { "local_ref": "floor" }, "b": { "$param": "floor" }, "rule": "contact", "tolerance": 100 } } }));
            }
            if entity["id"] == json!(grid) {
                entity["components"]["template.placement"]["params"] = json!({ "floor": floor });
            }
        }
    });
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&evaluation), [ProblemCode::TemplateInvalid]);
    assert_eq!(evaluation.problems[0].reference, Some(ids.get("piece")));
    assert!(evaluation.plan.is_empty());
}

/// Spec 05 section 3.4, rule 6, and section 4.3, step 6.12: a reference a placement's parameter brings into a
/// product keeps to the pins one the template writes keeps to: the catalog at a changeset, a template entity bare.
#[test]
fn a_reference_through_a_parameter_keeps_to_the_pin_rule() {
    let succeeding = |pinned: &'static str| {
        probe(&mut Ids::new(), 1.0, move |ids, documents| {
            let (piece, grid, class, floor) = (ids.get("piece"), ids.get("grid"), ids.get("class"), ids.get("floor"));
            let of = if pinned == "floor@head" { format!("{floor}@head") } else { format!("{class}{pinned}") };
            let map = documents.get_mut("template/site/probe/map").unwrap();
            for entity in map["entities"].as_array_mut().unwrap() {
                if entity["id"] == json!(piece) {
                    let template = &mut entity["components"]["template"];
                    template["params"] = json!({ "class": {} });
                    template["entities"].as_array_mut().unwrap().push(json!({ "local": "override", "components": {
                        "successor": { "of": { "$param": "class" } }, "template.target_scope": { "scope": "fleet" } } }));
                }
                if entity["id"] == json!(grid) {
                    entity["components"]["template.placement"]["params"] = json!({ "class": of });
                }
            }
        })
    };
    let pinned = run(&succeeding("@1"), KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(pinned.problems.is_empty(), "{:?}", pinned.problems);
    assert_eq!(pinned.census.declared["successor"], Count::new(6));
    for unpinned in ["", "@head", "floor@head"] {
        let evaluation = run(&succeeding(unpinned), KnobValues::new(), &Prior::new(DocId::mint()));
        assert_eq!(codes(&evaluation), [ProblemCode::TemplateInvalid; 6], "`{unpinned}`");
        assert!(evaluation.plan.is_empty(), "`{unpinned}`");
    }
}

/// Spec 05 section 4.3, step 6.10: a single-valued reference to a placement of several products, to an absent
/// entity, or outside the template and its catalog is a named problem; an absent entity drops from a list.
#[test]
fn references_the_made_site_cannot_hold_are_problems() {
    let refer = |field: &'static str, to: &'static str| {
        probe(&mut Ids::new(), 1.0, move |ids, documents| {
            let target = if to == "elsewhere" { DeclaredId::mint().to_string() } else { ids.get(to) };
            let (pairs, floors) = (ids.get("pairs"), ids.get("floors"));
            let map = documents.get_mut("template/site/probe/map").unwrap();
            for entity in map["entities"].as_array_mut().unwrap() {
                if field == "stitch" && entity["id"] == json!(pairs) {
                    entity["components"]["stitch"]["b"] = json!(target.clone());
                }
                if field == "sector" && entity["id"] == json!(floors) {
                    entity["components"]["sector"]["members"] = json!([target.clone(), ids.get("floor")]);
                }
            }
        })
    };
    let fresh = || Prior::new(DocId::mint());
    assert_eq!(
        codes(&run(&refer("stitch", "grid"), KnobValues::new(), &fresh())),
        [ProblemCode::PlacementInSingleReference]
    );
    assert_eq!(
        codes(&run(&refer("stitch", "ghost"), KnobValues::new(), &fresh())),
        [ProblemCode::ReferenceToAbsentEntity]
    );
    assert_eq!(
        codes(&run(&refer("stitch", "elsewhere"), KnobValues::new(), &fresh())),
        [ProblemCode::ReferenceLeavesTemplate]
    );
    let mut ids = Ids::new();
    let dropped = probe(&mut ids, 1.0, |ids, documents| {
        let (floors, ghost, floor) = (ids.get("floors"), ids.get("ghost"), ids.get("floor"));
        let map = documents.get_mut("template/site/probe/map").unwrap();
        for entity in map["entities"].as_array_mut().unwrap() {
            if entity["id"] == json!(floors) {
                entity["components"]["sector"]["members"] = json!([ghost, floor]);
            }
        }
    });
    let evaluation = run(&dropped, KnobValues::new(), &fresh());
    assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
    let members = &made(&evaluation, &ids.get("floors")).components["sector"]["members"];
    assert_eq!(members, &json!([id_of(&evaluation, &ids.get("floor"))]));
}

/// Spec 05 section 4.3, step 6.11: a record follows its generator to the made id, and is left out with a note when
/// its generator is absent.
#[test]
fn records_follow_their_generators() {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |ids, documents| {
        let (floor, ghost) = (ids.get("floor"), ids.get("ghost"));
        let map = documents.get_mut("template/site/probe/map").unwrap();
        let entities = map["entities"].as_array_mut().unwrap();
        entities.push(json!({ "derivation": { "generator": floor, "key": "cell:1,1" }, "components": { "cell": { "blocked": true } } }));
        entities.push(json!({ "derivation": { "generator": ghost, "key": "cell:0,0" }, "suppress": true }));
    });
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
    assert_eq!(evaluation.notes.len(), 1);
    let floor = id_of(&evaluation, &ids.get("floor"));
    let records: Vec<String> = evaluation
        .plan
        .values()
        .flat_map(|plan| plan.records.iter())
        .map(|record| match record {
            Record::Override { target, .. } | Record::Suppress { target } => target.to_string(),
        })
        .collect();
    assert_eq!(records, [format!("{floor}/cell:1,1")]);
}

/// The probe with one more component on its floor (`top`) or on its piece's floor internal.
fn with_component(top: bool, name: &'static str) -> TemplateRead {
    probe(&mut Ids::new(), 1.0, move |ids, documents| {
        let (floor, piece) = (ids.get("floor"), ids.get("piece"));
        let map = documents.get_mut("template/site/probe/map").unwrap();
        for entity in map["entities"].as_array_mut().unwrap() {
            if top && entity["id"] == json!(floor) {
                entity["components"][name] = json!({});
            }
            if !top && entity["id"] == json!(piece) {
                entity["components"]["template"]["entities"][0]["components"][name] = json!({});
            }
        }
    })
}

/// World-proto-server refuses a component its registry does not define (design/vocabulary/README.md), so a
/// misspelt one is `template-invalid` at the top of an entity and inside a piece alike; one registered without a
/// schema has nothing to fail, and `$comment` is the protocol's comment, which every consumer ignores.
#[test]
fn a_component_the_registry_does_not_define_refuses_the_template() {
    for top in [true, false] {
        for name in ["tilling", "$param"] {
            let evaluation = run(&with_component(top, name), KnobValues::new(), &Prior::new(DocId::mint()));
            assert_eq!(codes(&evaluation), [ProblemCode::TemplateInvalid], "{name}, at the top: {top}");
            assert!(evaluation.plan.is_empty());
        }
        for name in ["cell", "$comment"] {
            let evaluation = run(&with_component(top, name), KnobValues::new(), &Prior::new(DocId::mint()));
            assert!(evaluation.problems.is_empty(), "{name}, at the top: {top}: {:?}", evaluation.problems);
        }
    }
}

/// A derivation record aimed at `key` beneath `generator`, holding `components`.
fn record(generator: String, key: &str, components: Value) -> Value {
    json!({ "derivation": { "generator": generator, "key": key }, "components": components })
}

/// What writes one record with the probe's ids.
type MakeRecord = fn(&mut Ids) -> Value;

/// The probe with `make`'s record added to the document of `scope`.
fn with_record(scope: &'static str, make: impl FnOnce(&mut Ids) -> Value) -> (Ids, TemplateRead) {
    let mut ids = Ids::new();
    let read = probe(&mut ids, 1.0, |ids, documents| {
        let made = make(ids);
        documents.get_mut(scope).unwrap()["entities"].as_array_mut().unwrap().push(made);
    });
    (ids, read)
}

/// Spec 05 section 4.3, steps 6.2 and 6.11: a record's target and its presence are held to the template's rules
/// before any knob is read; a presence with no value at the knobs is its named problem, the record left out.
#[test]
fn records_are_held_to_the_template_rules() {
    const MAP: &str = "template/site/probe/map";
    let (_, read) = with_record(MAP, |ids| {
        record(ids.get("floor"), "cell:1,1", json!({ "cell": {}, "template.presence": { "when": "1 / 0" } }))
    });
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&evaluation), [ProblemCode::DivisionByZero]);
    assert!(evaluation.plan.values().all(|plan| plan.records.is_empty()));

    let refused: [(&str, &'static str, MakeRecord); 7] = [
        ("an unknown knob", MAP, |ids| {
            record(ids.get("floor"), "cell:1,1", json!({ "cell": {}, "template.presence": { "when": "nope" } }))
        }),
        ("a sign", MAP, |ids| {
            record(ids.get("floor"), "cell:1,1", json!({ "cell": {}, "template.presence": { "when": "-1" } }))
        }),
        ("a binding", MAP, |ids| {
            record(
                ids.get("floor"),
                "cell:1,1",
                json!({ "cell": { "x": 1 }, "template.knob_binding": { "/cell/x": "rows" } }),
            )
        }),
        ("a schema fault", MAP, |ids| record(ids.get("floor"), "cell:1,1", json!({ "pose": { "position": "here" } }))),
        ("a placement's product", MAP, |ids| {
            record(ids.get("grid"), "instance:0,0/local:floor/cell:0,0", json!({ "cell": {} }))
        }),
        ("a knob", MAP, |ids| record(ids.get("rows"), "cell:0,0", json!({ "cell": {} }))),
        ("another scope's generator", "template/site/probe/fleet", |ids| {
            record(ids.get("floor"), "cell:1,1", json!({ "cell": {} }))
        }),
    ];
    for (what, scope, make) in refused {
        let (_, read) = with_record(scope, make);
        let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
        assert!(!evaluation.problems.is_empty(), "{what}");
        assert!(
            evaluation.problems.iter().all(|p| p.code == ProblemCode::TemplateInvalid),
            "{what}: {:?}",
            evaluation.problems
        );
        assert!(evaluation.plan.is_empty(), "{what}");
    }
}

/// Spec 05 section 4.3, step 6.12: what a record carries over into the made site validates against the registry as
/// a made entity does. The probe's mark is valid naming the template's floor and invalid naming any other entity,
/// so only the rewritten record fails.
#[test]
fn a_record_carried_over_validates() {
    let (mut ids, read) = with_record("template/site/probe/map", |ids| {
        record(ids.get("floor"), "cell:1,1", json!({ "probe.mark": { "names": ids.get("floor") } }))
    });
    let mark = RegisteredComponent {
        name: "probe.mark".into(),
        version: "0.1.0".into(),
        schema: Some(json!({ "type": "object", "properties": { "names": { "const": ids.get("floor") } } })),
        refs: vec![RefAnnotation { path: "/names".into(), expects: None }],
    };
    let read = TemplateRead { registry: support::registry_with(vec![mark]), ..read };
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&evaluation), [ProblemCode::TemplateInvalid], "{:?}", evaluation.problems);
    assert_eq!(evaluation.problems[0].reference, Some(format!("{}/cell:1,1", ids.get("floor"))));
}

/// A deterministic id source would let two evaluations agree by construction; the engine takes any source.
#[test]
fn every_new_id_comes_from_the_callers_source() {
    struct Counted(usize);
    impl IdSource for Counted {
        fn fresh(&mut self) -> DeclaredId {
            self.0 += 1;
            DeclaredId::mint()
        }
    }
    let read = probe(&mut Ids::new(), 1.0, |_, _| {});
    let mut source = Counted(0);
    let evaluation = evaluate(&read, &KnobValues::new(), &Prior::new(DocId::mint()), &mut source);
    assert_eq!(source.0, evaluation.ids().len());
}

/// The probe with `len` declared anew, its default `default` and its checks `checks`, and `extra` declared after it.
fn derived_len(unit_mm: f64, default: &'static str, checks: Value, extra: Option<Value>) -> (Ids, TemplateRead) {
    let mut ids = Ids::new();
    let read = probe(&mut ids, unit_mm, move |ids, documents| {
        let (len, more) = (ids.get("len"), ids.get("extra"));
        let entities = documents.get_mut("template/site/probe").unwrap()["entities"].as_array_mut().unwrap();
        for entity in entities.iter_mut().filter(|entity| entity["id"] == json!(len)) {
            entity["components"]["template.knob"]["default"] = json!(default);
            entity["components"]["template.knob"]["checks"] = checks.clone();
        }
        entities.extend(extra.map(|components| json!({ "id": more, "components": components })));
    });
    (ids, read)
}

fn derived(evaluation: &Evaluation) -> Vec<String> {
    let mut keys: Vec<String> = evaluation.derived.iter().map(ToString::to_string).collect();
    keys.sort();
    keys
}

fn texts(evaluation: &Evaluation) -> Vec<&str> {
    evaluation.problems.iter().map(|problem| problem.text.as_str()).collect()
}

/// D120, spec 05 section 3.2: a knob nobody set stands at its default expression's value, worked out at each
/// evaluation from the knobs it names; a value someone set holds. The evaluation answers every knob's value and
/// which of them a default expression gave.
#[test]
fn an_unset_knob_follows_its_default_expression_and_a_set_one_holds() {
    let (mut ids, read) = derived_len(1.0, "rows * 2550 + 1700", json!([]), None);
    let floor = ids.get("floor");
    let at_defaults = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(at_defaults.problems.is_empty(), "{:?}", at_defaults.problems);
    let every: Vec<(String, i64)> = at_defaults.knobs.iter().map(|(key, value)| (key.to_string(), value.0)).collect();
    let wanted = [("cols", 2), ("len", 9350), ("off", 0), ("rows", 3)].map(|(key, value)| (key.to_owned(), value));
    assert_eq!(every, wanted);
    assert_eq!(made(&at_defaults, &floor).components["region"]["size"][0], json!(9350));
    assert_eq!(derived(&at_defaults), ["len"]);
    let more = run(&read, KnobValues::new().with("rows", 5), &Prior::new(DocId::mint()));
    assert_eq!(more.knobs.get("len"), Some(KnobValue(14450)));
    let held = run(&read, KnobValues::new().with("rows", 5).with("len", 8500), &Prior::new(DocId::mint()));
    assert_eq!((held.knobs.get("len"), derived(&held)), (Some(KnobValue(8500)), Vec::new()));
}

/// Spec 05 section 3.2 (D120): default expressions are worked out in dependency order, whatever the order the knobs
/// are declared in, each reading the value the knobs it names came to or were set to.
#[test]
fn default_expressions_follow_one_another_in_dependency_order() {
    let wide = json!({ "template.knob": { "key": "wide", "label": "wide", "unit": "mm", "min": 850, "max": 85000,
        "step": 850, "default": "rows * 1700" } });
    let (_, read) = derived_len(1.0, "wide + 850", json!([]), Some(wide));
    let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
    assert_eq!(
        (evaluation.knobs.get("wide"), evaluation.knobs.get("len")),
        (Some(KnobValue(5100)), Some(KnobValue(5950)))
    );
    assert_eq!(derived(&evaluation), ["len", "wide"]);
    let set = run(&read, KnobValues::new().with("wide", 8500), &Prior::new(DocId::mint()));
    assert_eq!((set.knobs.get("len"), derived(&set)), (Some(KnobValue(9350)), vec!["len".to_owned()]));
}

/// Spec 05 section 3.2 (D120): a default expression naming no declared knob, or reaching its own knob through the
/// others, makes the template invalid; one of another dimension than its knob is `dimension-mismatch`.
#[test]
fn a_default_expression_naming_no_knob_or_itself_refuses_the_template() {
    let wide = || {
        json!({ "template.knob": { "key": "wide", "label": "wide", "unit": "mm", "min": 850, "max": 85000,
        "step": 850, "default": "len" } })
    };
    let faults = [
        ("len + 850", None, "the default of `len` names itself"),
        ("wide + 850", Some(wide()), "the defaults of `len`, `wide` name one another in a cycle"),
        ("depth * 2", None, "the default of `len` names `depth`, which no knob declares"),
    ];
    for (default, extra, fault) in faults {
        let (_, read) = derived_len(1.0, default, json!([]), extra);
        let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
        assert_eq!(texts(&evaluation)[0], format!("Error: This template cannot be evaluated: {fault}"));
        assert!(evaluation.problems.iter().all(|problem| problem.code == ProblemCode::TemplateInvalid), "{default}");
        assert!(evaluation.plan.is_empty(), "{default}");
    }
    let (_, read) = derived_len(1.0, "rows", json!([]), None);
    let mismatched = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(codes(&mismatched)[0], ProblemCode::DimensionMismatch);
    assert_eq!(texts(&mismatched)[0], "Error: the default of `len`: `rows` is a count, where a length is needed");
}

/// Spec 05 section 3.2 (D120): a default expression's value is snapped up to the knob's next valid value, its
/// lengths read in steps of the primary frame at `unit_mm` 1 and 0.1; outside the knob's range it is held at the
/// nearer bound with an error under that knob, which stands in for that knob's own checks.
#[test]
fn a_derived_value_snaps_up_and_outside_its_range_is_held_with_a_problem() {
    for (unit_mm, default) in [(1.0, "rows * 1000"), (0.1, "rows * 10000")] {
        let (_, read) = derived_len(unit_mm, default, json!([]), None);
        let evaluation = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
        assert_eq!(evaluation.knobs.get("len"), Some(KnobValue(3400)), "3,000 mm at unit_mm {unit_mm}");
        assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
    }
    let fits = json!([{ "severity": "error", "value_unit": "mm", "fails_when": "rows * 10000 - len",
        "problem": "The rows need {value} mm more length." }]);
    let (_, read) = derived_len(1.0, "rows * 10000", fits, None);
    let above = run(&read, KnobValues::new().with("rows", 10), &Prior::new(DocId::mint()));
    assert_eq!(above.knobs.get("len"), Some(KnobValue(85000)));
    let text =
        "Error: len's default comes to 100,000 mm, more than its maximum of 85,000 mm, so it stays at 85,000 mm.";
    assert_eq!(texts(&above), [text]);
    let problem = &above.problems[0];
    assert_eq!(
        (problem.code, problem.knob.as_ref().map(ToString::to_string)),
        (ProblemCode::DerivedOutOfRange, Some("len".into()))
    );
    let (_, read) = derived_len(1.0, "cols * 1000 - 2000", json!([]), None);
    let below = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(below.knobs.get("len"), Some(KnobValue(850)));
    assert_eq!(
        texts(&below),
        ["Error: len's default comes to 0 mm, less than its minimum of 850 mm, so it stays at 850 mm."]
    );
    assert!(!below.plan.is_empty(), "a value held at a bound still lays the template out");
}

/// D120: a derived knob's check reports once someone sets that knob, its default being written to pass the check;
/// a default that falls short of its own check is reported all the same, never hidden.
#[test]
fn a_derived_knobs_check_reports_once_the_knob_is_set() {
    let fits = |fails_when: &str| json!([{ "severity": "error", "value_unit": "mm", "fails_when": fails_when, "problem": "The rows need {value} mm more length." }]);
    let (_, read) = derived_len(1.0, "rows * 2550 + 1700", fits("rows * 2550 + 1700 - len"), None);
    let follows = run(&read, KnobValues::new().with("rows", 5), &Prior::new(DocId::mint()));
    assert!(follows.problems.is_empty(), "{:?}", follows.problems);
    let set = run(&read, KnobValues::new().with("rows", 5).with("len", 8500), &Prior::new(DocId::mint()));
    assert_eq!(texts(&set), ["Error: The rows need 5,950 mm more length."]);
    assert_eq!(set.problems[0].knob.as_ref().map(ToString::to_string), Some("len".to_owned()));
    let (_, read) = derived_len(1.0, "rows * 850", fits("rows * 2550 - len"), None);
    let short = run(&read, KnobValues::new(), &Prior::new(DocId::mint()));
    assert_eq!(texts(&short), ["Error: The rows need 5,100 mm more length."]);
}
