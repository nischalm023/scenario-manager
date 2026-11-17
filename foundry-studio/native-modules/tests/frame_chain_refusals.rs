//! What the loader refuses when a document's frames do not all measure from
//! the one frame the document calls primary.
//!
//! Every position a run records is stated in the primary frame, and the
//! protocol library reaches that frame by climbing a frame's parents until one
//! has none — then calls whatever it arrived at the primary frame. A document
//! with a second root therefore converts a position measured from that root
//! and hands back a claim about the primary one, and a primary frame posed
//! inside something else is transformed above itself. Neither is refused by
//! `world-protocol`; both are refused here, before a single cell is placed.
//!
//! Every document is hand-built and tiny. The first test is the POSITIVE
//! control — the sound shape gets past this check — so a refusal below is the
//! defect and not the scaffolding.

use std::path::PathBuf;

use dessim::worldmap::{self, WorldMap, WorldMapError};
use serde_json::{Value, json};

const SITE: &str = "30000000-0000-4000-8000-000000000001";
const AISLE: &str = "30000000-0000-4000-8000-000000000002";
const ELSEWHERE: &str = "30000000-0000-4000-8000-000000000003";

fn load(name: &str, entities: Vec<Value>) -> Result<WorldMap, WorldMapError> {
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "30000000-0000-4000-8000-0000000000ff", "kind": "world", "name": name },
        "primary_frame": SITE,
        "required_components": ["frame"],
        "entities": entities,
    });
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&document).expect("the document serializes"))
        .expect("the scratch document is writable");
    worldmap::load(&path)
}

/// A frame, optionally posed inside another one.
fn frame(id: &str, parent: Option<&str>) -> Value {
    let mut components = json!({ "frame": { "dims": 2, "units": "mm" } });
    if let Some(parent) = parent {
        components["frame"]["parent"] = json!(parent);
        components["frame"]["transform_to_parent"] = json!({ "translation": [100, 0] });
    }
    json!({ "id": id, "components": components })
}

/// The site frame, which is this document's primary one and states what a
/// step is worth.
fn primary(parent: Option<&str>) -> Value {
    let mut site = frame(SITE, parent);
    site["components"]["frame"]["unit_mm"] = json!(1);
    site
}

/// The refusal a load came back with, or a failure naming what should have
/// been refused. `WorldMap` carries the loaded world and has no `Debug`.
fn refused(loaded: Result<WorldMap, WorldMapError>, what: &str) -> WorldMapError {
    match loaded {
        Ok(_) => panic!("{what} loaded without a word"),
        Err(refusal) => refusal,
    }
}

/// The positive control: one root, everything under it. This document has no
/// cells, so the load still fails — but on the cells, which is proof that the
/// frame check let it through.
#[test]
fn every_frame_under_the_primary_one_gets_past_the_frame_check() {
    let refusal = refused(load("frames_one_root", vec![primary(None), frame(AISLE, Some(SITE))]), "a cell-less world");
    assert!(matches!(refusal, WorldMapError::NoCells), "got {refusal}");
}

/// A frame whose parent chain ends at a root of its own. Every cell posed in
/// it would be pushed to that root's origin and then labelled as the primary
/// frame's, so a rack a hundred millimetres away in the document lands a
/// hundred millimetres away from where it is.
#[test]
fn a_frame_that_climbs_to_another_root_is_refused_by_name() {
    let document = vec![primary(None), frame(ELSEWHERE, None), frame(AISLE, Some(ELSEWHERE))];
    let refusal = refused(load("frames_two_roots", document), "a document with two roots");
    let WorldMapError::FrameOutsideThePrimaryFrame { frame, root, primary } = &refusal else { panic!("got {refusal}") };
    assert_eq!((frame.to_string().as_str(), root.to_string().as_str()), (AISLE, ELSEWHERE));
    assert_eq!(primary.to_string(), SITE, "and the frame it should have reached");
}

/// The primary frame is the one the document measures from, so it has no
/// parent to be measured from itself.
#[test]
fn a_primary_frame_posed_inside_something_else_is_refused() {
    let document = vec![primary(Some(ELSEWHERE)), frame(ELSEWHERE, None)];
    let refusal = refused(load("frames_nested_primary", document), "a primary frame with a parent");
    let WorldMapError::PrimaryFrameHasAParent { frame, parent } = &refusal else { panic!("got {refusal}") };
    assert_eq!((frame.to_string().as_str(), parent.to_string().as_str()), (SITE, ELSEWHERE));
}
