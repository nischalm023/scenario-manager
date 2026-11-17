//! The export's check on files written by hand: what `close` refuses that WPS's own server never writes, an identity
//! two scopes hold, a file that is not the cut's and one that does not state all three flags; and the bytes it writes,
//! every object's keys sorted.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use studio_protocol::{BranchName, DeclaredId, Pin, ScopePath, Seq};

use super::{close, component_names, written};
use crate::error::{ProblemKind, WpsError};
use crate::value::{Exactness, UnitCut};

fn path(text: &str) -> ScopePath {
    text.parse().expect("a scope path")
}

/// The cut a file is held to: `customer/c/site/s` alone, at changeset 1.
fn one_scope_cut() -> UnitCut {
    let unit = path("customer/c/site/s");
    UnitCut {
        unit: unit.clone(),
        requested: Pin::Branch(BranchName::main()),
        branch: BranchName::main(),
        seqs: BTreeMap::from([(unit, Seq::new(1))]),
        exact: Exactness::Exact,
    }
}

/// A file for `one_scope_cut` holding `entities`, all three flags set.
fn one_scope_file(entities: Vec<Value>) -> Value {
    json!({
        "derived": true,
        "entities": entities,
        "nested": true,
        "referenced": true,
        "scope": "customer/c/site/s",
        "scopes": [{ "changeset": 1, "scope": "customer/c/site/s", "whole": true }],
    })
}

/// `file` closed against `one_scope_cut`.
fn closed(file: Value) -> Result<(), WpsError> {
    let names = component_names(&file);
    close(&one_scope_cut(), file, &names, &Default::default(), &Default::default()).map(|_| ())
}

/// Statement 17: an identity two entries list is `HeldTwice`, which WPS's owner index makes unreachable on one
/// server, so the check is held to it with a file written by hand.
#[test]
fn an_identity_listed_twice_is_refused() {
    let id = DeclaredId::mint();
    let entry = |scope: &str| json!({ "components": {}, "id": id.to_string(), "scope": scope });
    let mut file = one_scope_file(vec![entry("customer/c/site/s"), entry("catalog/agent/a")]);
    file["scopes"] = json!([
        { "changeset": 1, "scope": "catalog/agent/a", "whole": false },
        { "changeset": 1, "scope": "customer/c/site/s", "whole": true },
    ]);
    let Err(WpsError::ExportNotClosed { problems }) = closed(file) else {
        panic!("an identity listed twice is refused");
    };
    assert!(
        problems.iter().any(|problem| problem.kind == ProblemKind::HeldTwice && problem.entity == Some(id.into())),
        "{problems:#?}"
    );
}

/// A file that exports another scope, holds the cut at another changeset or only in part, holds whole a scope the cut
/// does not name, or lists an entity in a scope its `scopes` does not name, is not the export asked for: a pin to that
/// entity would have no changeset to be held to.
#[test]
fn a_file_that_does_not_hold_the_cut_is_refused() {
    let mut other = one_scope_file(Vec::new());
    other["scope"] = json!("customer/c/site/t");
    let mut moved = one_scope_file(Vec::new());
    moved["scopes"][0]["changeset"] = json!(2);
    let mut in_part = one_scope_file(Vec::new());
    in_part["scopes"][0]["whole"] = json!(false);
    let mut stray = one_scope_file(Vec::new());
    stray["scopes"] = json!([
        { "changeset": 1, "scope": "catalog/agent/a", "whole": true },
        { "changeset": 1, "scope": "customer/c/site/s", "whole": true },
    ]);
    let lost = json!({ "components": {}, "id": DeclaredId::mint().to_string(), "scope": "catalog/agent/a" });
    let unnamed = one_scope_file(vec![lost]);
    for file in [other, moved, in_part, stray, unnamed] {
        let refused = closed(file.clone());
        assert!(matches!(refused, Err(WpsError::Invalid { .. })), "{file}: {refused:?}");
    }
    closed(one_scope_file(Vec::new())).expect("the file asked for closes");
}

/// Ruling D185: a run reads a site whole as WPS generated it with everything it references, so a file that does not
/// state each of `nested`, `derived` and `referenced` as true is refused, naming each flag it leaves unset and the
/// request that sets them; one that states a flag as neither `true` nor `false` is refused naming it.
#[test]
fn a_file_without_all_three_flags_is_refused_naming_each_flag_unset() {
    let mut unset = one_scope_file(Vec::new());
    unset["derived"] = json!(false);
    unset.as_object_mut().expect("an object").remove("referenced");
    let Err(WpsError::Invalid { text }) = closed(unset) else { panic!("a file with flags unset is refused") };
    for named in ["`derived` and `referenced`", "?nested=1&derived=1&referenced=1"] {
        assert!(text.contains(named), "{named} in {text}");
    }
    assert!(!text.contains("`nested`"), "{text}");

    let mut spelled = one_scope_file(Vec::new());
    spelled["nested"] = json!(1);
    let Err(WpsError::Invalid { text }) = closed(spelled) else { panic!("a flag stated as 1 is refused") };
    assert!(text.contains("`nested`"), "{text}");
}

/// Ruling D77: the API and a runner hash one file alike, though serde_json keeps a map in the order it was built in
/// where `preserve_order` is on, as rig-core turns it on in the API and this crate's tests turn it on here. A file whose
/// objects were built out of order, nested ones included, is written compact with every object's keys sorted.
#[test]
fn the_export_is_written_with_every_objects_keys_sorted() {
    let built = r#"{"z":1,"entities":[{"scope":"s","id":"b","components":{"zeta":{"b":2,"a":1},"alpha":{}}}],"a":[{"y":true,"x":null}]}"#;
    let file: Value = serde_json::from_str(built).expect("JSON");
    assert_eq!(file.to_string(), built, "the map keeps the order the file was built in");
    let bytes = written(file).expect("the file writes");
    let sorted = r#"{"a":[{"x":null,"y":true}],"entities":[{"components":{"alpha":{},"zeta":{"a":1,"b":2}},"id":"b","scope":"s"}],"z":1}"#;
    assert_eq!(String::from_utf8(bytes.to_vec()).expect("UTF-8"), sorted);
}
