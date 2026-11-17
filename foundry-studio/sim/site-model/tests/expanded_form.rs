//! What the expanded form may not contain, checked over every fixture this
//! crate loads.
//!
//! The other expanded-form tests live beside the fixture they belong to and ask
//! whether the artifact says what its source said. This file asks the different
//! question a *second* implementation makes urgent: whether the artifact
//! contains anything only this implementation would have written. SPEC §9 holds
//! two implementations to the canonical projection of the expanded form, so a
//! byte one of them mints out of an internal representation — rather than out of
//! the document — is a disagreement waiting to happen, and none of the
//! per-example tests would see it.

use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use site_model::components::fleet::{Assignment, Placement};
use site_model::document::DocMeta;
use site_model::DocId;

/// Every fixture, by the scenario a reader is handed.
fn fixtures() -> [(&'static str, PathBuf); 3] {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application-tests/native/fixtures");
    [
        ("walmart-dallas", root.join("walmart-dallas/scenario.json")),
        (
            "minimal-putaway2",
            root.join("minimal-putaway2/minimal_putaway2.json"),
        ),
        ("cencora-v3", root.join("cencora-v3/cencora_v3.json")),
    ]
}

/// An absent optional is written as nothing, never as an explicit `null`.
///
/// A null can reach this artifact by exactly two routes: a document authored one
/// and the reader carried it through untouched, or expansion re-serialized a
/// typed view whose optional field was empty. No example does the first. The
/// second is the hazard — the field is absent in the source, present as `null`
/// in the artifact, and a second implementation reading the same source has no
/// reason to write the key at all.
///
/// Killing it moved exactly one artifact. Dallas's 60 derived carriers each lost
/// a `"sector": null` from the `placement` their pool re-serialized onto them,
/// and nothing else in the file changed — while minimal-putaway2's and
/// cencora-v3's expanded forms came out byte for byte as they were, neither one
/// declaring a carrier pool that leaves `sector` unstated.
#[test]
fn no_expanded_form_writes_an_explicit_null() {
    for (name, scenario) in fixtures() {
        let (set, world) = site_model::load(&scenario).expect("the fixture loads");
        let document = site_model::expanded::emit(&set, &world);
        let mut nulls = Vec::new();
        find_nulls(&document, String::new(), &mut nulls);
        assert!(
            nulls.is_empty(),
            "{name}'s expanded form writes {} explicit null(s), first at {}",
            nulls.len(),
            nulls[0]
        );
    }
}

/// Nothing expansion writes back out spells an absence.
///
/// Three typed views are re-serialized into the expanded form, and they are the
/// only three: a cohort's `assignment` onto every robot it mints, a pool's
/// `placement` onto every carrier, and the scenario's own metadata block. They
/// are therefore the only places a key can enter the artifact that no document
/// wrote. A field left unstated has to come out unstated, whichever way the
/// absence could have been spelled — `null` and `[]` both read back as exactly
/// what was already there, so writing either puts a byte in the artifact that a
/// second implementation reading the same source has no reason to reproduce.
///
/// This is said about the shapes rather than about the fixtures, which is the
/// half the artifact test above cannot reach: whether a null or an empty list
/// appears in a fixture is an accident of what those documents happen to state,
/// and none of the three states a scenario with no name.
#[test]
fn nothing_re_serialized_into_the_expanded_form_spells_an_absence() {
    let assignment = Assignment {
        kind: "free".to_string(),
        to: Vec::new(),
    };
    assert_eq!(keys(&assignment), ["kind"], "a cohort confining nobody");

    let placement = Placement {
        kind: "distributed_storage".to_string(),
        sector: None,
        rest: Map::new(),
    };
    assert_eq!(keys(&placement), ["kind"], "a placement naming no sector");

    let meta = DocMeta {
        id: DocId(
            "da000000-0000-4000-8000-000000000001"
                .parse()
                .expect("a literal UUID in this test"),
        ),
        name: None,
        kind: None,
        depends_on: Vec::new(),
        assembled_from: Vec::new(),
    };
    assert_eq!(keys(&meta), ["id"], "a document stating only its identity");
}

/// The keys one value serializes to.
fn keys<T: Serialize>(value: &T) -> Vec<String> {
    serde_json::to_value(value)
        .expect("these shapes serialize")
        .as_object()
        .expect("each of them is a JSON object")
        .keys()
        .cloned()
        .collect()
}

/// Every place a null sits, by the path a reader would follow to reach it.
fn find_nulls(value: &Value, path: String, out: &mut Vec<String>) {
    match value {
        Value::Null => out.push(if path.is_empty() {
            "the document itself".to_string()
        } else {
            path
        }),
        Value::Object(map) => {
            for (key, child) in map {
                find_nulls(child, format!("{path}.{key}"), out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                find_nulls(child, format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}
