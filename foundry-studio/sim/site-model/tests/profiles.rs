//! What the schema layer's two profiles admit, read by a JSON Schema validator
//! rather than by this crate.
//!
//! `proto/schema/` states the shape of a document for anyone with a validator
//! and no Rust toolchain. Two rulings of 2026-10-18 changed that shape: the
//! authoring profile closes the fields of the core's components and no longer
//! the names of the components beyond them, and the `doc` block may say which
//! scopes and changesets a server assembled the document from. A widening of
//! 2026-10-19 changed it again: inside a template, a parameter may stand for a
//! reference, a cell address or a derivation key, and a bare local handle for a
//! cell address, as SPEC §6 and every reader already allowed. These tests hold
//! the files to all three.

use jsonschema::{Resource, Validator};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const WIRE: &str = "urn:worldproto:0.7:wire";
const AUTHORING: &str = "urn:worldproto:0.7:authoring";
const DOC: &str = "da000000-0000-4000-8000-000000000001";
const ENTITY: &str = "a0000000-0000-4000-8000-000000000001";

fn schema_files(directory: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("the schema directory is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            schema_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "json") {
            out.push(path);
        }
    }
}

/// One profile, compiled with every other schema file registered under its own
/// `$id`: the files name each other by URN, so nothing is fetched.
fn profile(id: &str) -> Validator {
    let mut files = Vec::new();
    schema_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application/schema"), &mut files);
    let mut options = jsonschema::options();
    let mut root = None;
    for path in files {
        let schema: Value = serde_json::from_str(&fs::read_to_string(&path).expect("readable")).expect("JSON");
        let Some(file_id) = schema.get("$id").and_then(Value::as_str).map(str::to_string) else { continue };
        if file_id == id {
            root = Some(schema);
        } else {
            options.with_resource(file_id, Resource::from_contents(schema).expect("a draft the validator knows"));
        }
    }
    options.build(&root.expect("a schema file registers the profile")).expect("the profile compiles")
}

/// Where a profile refuses a document, by the instance path of each refusal.
fn refusals(profile: &Validator, document: &Value) -> Vec<String> {
    profile.iter_errors(document).map(|error| error.instance_path.to_string()).collect()
}

/// A world document holding one entity.
fn with_entity(required: Value, components: Value) -> Value {
    json!({
        "proto_version": "0.7.0",
        "doc": { "id": DOC, "kind": "world" },
        "required_components": required,
        "entities": [{ "id": ENTITY, "components": components }],
    })
}

fn with_doc(doc: Value) -> Value {
    json!({ "proto_version": "0.7.0", "doc": doc, "entities": [] })
}

/// A component the core does not define passes the authoring profile whatever
/// its name, plain or dotted, its fields unread: whether it names anything is a
/// registry's question. The grammar is the registry's own, so a segment may
/// begin with a digit.
#[test]
fn a_component_the_core_does_not_define_passes_whatever_its_name() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    for name in ["radio_shadow", "easyloop", "mapcreator.station", "go.marker", "3d_scan"] {
        let document = with_entity(json!([]), json!({ name: { "anything": [1, 2], "at": "all" } }));
        assert_eq!(refusals(&wire, &document), Vec::<String>::new(), "{name}, wire");
        assert_eq!(refusals(&authoring, &document), Vec::<String>::new(), "{name}, authoring");
    }
}

/// The closure stays on the core's components: beside a plain component the
/// core does not define, a field `charging` does not define is still a typo
/// while authoring, and still carried on the wire.
#[test]
fn a_field_the_core_does_not_define_on_its_own_component_is_still_refused() {
    let document = with_entity(json!([]), json!({ "radio_shadow": {}, "charging": { "voltage": 48 } }));
    assert_eq!(refusals(&profile(WIRE), &document), Vec::<String>::new());
    assert_eq!(refusals(&profile(AUTHORING), &document), ["/entities/0/components/charging"]);
}

/// A key no registry could hold as a component's name is refused while
/// authoring, and carried on the wire, which reads no names at all.
#[test]
fn a_key_that_could_never_name_a_component_is_refused_while_authoring() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    for key in ["Radio_shadow", "radio-shadow", "radio..shadow", ".radio", "radio shadow"] {
        let document = with_entity(json!([]), json!({ key: {} }));
        assert_eq!(refusals(&wire, &document), Vec::<String>::new(), "{key}, wire");
        assert_eq!(refusals(&authoring, &document), ["/entities/0/components"], "{key}, authoring");
    }
}

/// A document may require any legal component name, plain ones included; never
/// an annotation key or a name outside the grammar.
#[test]
fn required_components_may_name_any_legal_component() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    let document = with_entity(json!(["radio_shadow", "mapcreator.export", "charging"]), json!({ "radio_shadow": {} }));
    assert_eq!(refusals(&wire, &document), Vec::<String>::new());
    assert_eq!(refusals(&authoring, &document), Vec::<String>::new());
    for name in ["$comment", "Radio_shadow"] {
        let document = with_entity(json!([name]), json!({}));
        assert_eq!(refusals(&wire, &document), ["/required_components/0"], "{name}, wire");
        assert_eq!(refusals(&authoring, &document), ["/required_components/0"], "{name}, authoring");
    }
}

/// An assembled document lists the scopes it was read from, each at its
/// changeset, the tag only where the read named one; an empty list says
/// nothing, as an empty `depends_on` does.
#[test]
fn the_doc_block_may_say_what_a_document_was_assembled_from() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    for sources in [
        json!([
            { "scope": "customer/acme/site/dfw6/map", "changeset": 41, "tag": "run-41" },
            { "scope": "module/go-catalog", "changeset": 0, "$comment": "never written" }
        ]),
        json!([]),
    ] {
        let document = with_doc(json!({ "id": DOC, "kind": "world", "assembled_from": sources }));
        assert_eq!(refusals(&wire, &document), Vec::<String>::new(), "{sources}, wire");
        assert_eq!(refusals(&authoring, &document), Vec::<String>::new(), "{sources}, authoring");
    }
}

/// A source names its scope and a count of changesets, and nothing else: the
/// `doc` block stays closed, one key the larger.
#[test]
fn a_malformed_source_or_a_stray_doc_key_is_refused() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    let doc = |source: Value| json!({ "id": DOC, "assembled_from": [source] });
    for document in [
        with_doc(doc(json!({ "scope": "module/go-catalog", "changeset": -1 }))),
        with_doc(doc(json!({ "scope": "module/go-catalog", "changeset": "41" }))),
        with_doc(doc(json!({ "scope": "", "changeset": 41 }))),
        with_doc(doc(json!({ "changeset": 41 }))),
        with_doc(doc(json!({ "scope": "module/go-catalog", "changeset": 41, "tag": "" }))),
        with_doc(doc(json!({ "scope": "module/go-catalog", "changeset": 41, "branch": "main" }))),
        with_doc(json!({ "id": DOC, "provenance": [] })),
    ] {
        assert!(!refusals(&wire, &document).is_empty(), "{document}, wire");
        assert!(!refusals(&authoring, &document).is_empty(), "{document}, authoring");
    }
}

/// A template that wires itself through every form a stamp resolves: a seam
/// from its own patch to whichever floor the stamp binds, and a queue over a
/// tile of that patch the stamp picks, then a cell the template declares, then
/// a cell the stamp names outright.
fn placeholder_template() -> Value {
    with_entity(
        json!([]),
        json!({ "template": {
            "params": { "floor": {}, "queue": {}, "exit": {} },
            "entities": [
                { "local": "patch", "components": {
                    "region": { "origin": [0, 0], "size": [2000, 1000] },
                    "tiling": { "pitch": [1000, 1000] } } },
                { "local": "stand", "components": { "pose": { "position": [2500, 500] }, "cell": {} } },
                { "local": "seam", "components": {
                    "stitch": { "a": { "local_ref": "patch" }, "b": { "$param": "floor" }, "rule": "contact" } } },
                { "local": "lane", "components": { "queue": { "path": [
                    { "local_ref": "patch", "key": { "$param": "queue" } },
                    { "local_ref": "stand", "$comment": "the template's own cell" },
                    { "$param": "exit" }
                ] } } }
            ]
        } }),
    )
}

/// Inside a template a parameter stands for a reference, a cell address or a
/// derivation key, and a bare handle for a cell address, as SPEC §6 and every
/// reader allow: Foundry Agent Studio's site templates stitch a piece to the
/// floor their stamp names and queue over a tile their stamp picks.
#[test]
fn inside_a_template_a_parameter_or_a_bare_handle_stands_where_a_reference_or_a_cell_goes() {
    let document = placeholder_template();
    assert_eq!(refusals(&profile(WIRE), &document), Vec::<String>::new(), "wire");
    assert_eq!(refusals(&profile(AUTHORING), &document), Vec::<String>::new(), "authoring");
}

/// Seven shapes near a placeholder, each refused where it stands in the
/// template above. In the queue path: a local address whose key is a string no
/// key reads, one whose key is a parameter with no name, and one with a stray
/// field beside its key; a bare handle with a stray field; a parameter with a
/// stray field. Where an entity reference goes: a local address, which names a
/// cell an internal derives rather than an entity, and a handle with a stray
/// field. The parameter is refused by its own closure; every other case passes
/// unless the bare handle is closed to its one key, since an open one admits
/// any object holding a `local_ref`, and the local address with a stray field
/// passes unless that address is closed to its two.
#[test]
fn a_placeholder_in_any_other_shape_is_still_refused() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    let internal =
        |index: usize, rest: &str| format!("/entities/0/components/template/entities/{index}/components/{rest}");
    for (at, value) in [
        (internal(3, "queue/path/0"), json!({ "local_ref": "patch", "key": "Bad//Key" })),
        (internal(3, "queue/path/0"), json!({ "local_ref": "patch", "key": { "$param": "" } })),
        (internal(3, "queue/path/0"), json!({ "local_ref": "patch", "key": { "$param": "queue" }, "depth": 2 })),
        (internal(3, "queue/path/1"), json!({ "local_ref": "stand", "depth": 2 })),
        (internal(3, "queue/path/2"), json!({ "$param": "exit", "default": "cell:0,0" })),
        (internal(2, "stitch/b"), json!({ "local_ref": "patch", "key": "cell:0,0" })),
        (internal(2, "stitch/b"), json!({ "local_ref": "patch", "of": "floor" })),
    ] {
        let mut document = placeholder_template();
        *document.pointer_mut(&at).unwrap_or_else(|| panic!("the template holds {at}")) = value.clone();
        assert!(!refusals(&wire, &document).is_empty(), "{at} = {value}, wire");
        assert!(!refusals(&authoring, &document).is_empty(), "{at} = {value}, authoring");
    }
}

#[test]
fn application_derived_ids_and_local_keys_pass_both_profiles() {
    let (wire, authoring) = (profile(WIRE), profile(AUTHORING));
    let derived = format!("{ENTITY}/DeckA/North-Port");
    let document = with_entity(json!([]), json!({"pose":{"frame":derived,"position":[0,0]}}));
    assert!(refusals(&wire, &document).is_empty());
    assert!(refusals(&authoring, &document).is_empty());
    let mut template = placeholder_template();
    template["entities"][0]["components"]["template"]["entities"][3]["components"]["queue"]["path"][0] =
        json!({"local_ref":"patch","key":"42"});
    assert!(refusals(&wire, &template).is_empty());
    assert!(refusals(&authoring, &template).is_empty());
}
