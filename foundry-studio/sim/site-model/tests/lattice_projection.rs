//! Where two cells land in one square of a declared integer frame.
//!
//! The lattice frame is nominal and lossy by declaration (SPEC §4): real sites'
//! lattices are not affine images of the measured world, and the format keeps
//! one only so legacy consumers can project into it. This file pins the two ends
//! of what that projection may do — a fold the author asked for is silent, a
//! fold nobody asked for is a warning — using fixtures built here rather than
//! any example, so the rule is tested against geometry chosen to test it.

use serde_json::{json, Value};
use std::path::Path;
use site_model::pipeline::derive;
use site_model::pipeline::layers::LayerSet;
use site_model::{Document, Warning};

const SITE: &str = "44d924cd-e36e-4da9-b7ac-401b932bf445";
const FLOOR: &str = "edd0316d-a9bc-48f4-bb37-a1594ced8cd1";
const LATTICE: &str = "af1f165f-c886-42df-9766-bafb062d5e56";

/// Site, floor, and a 750-step integer lattice over the floor — the shape every
/// converted fixture carries. One step is one millimetre here.
///
/// The lattice translation is half its pitch because a lattice index names a
/// square and a square's place is its centre: index `[c,r]` is the millimetre
/// point `[750c+375, 750r+375]`, which is what every converted document
/// declares. The offset is not decoration here — it moves which square a cell
/// centre folds into, and folding is the thing this file measures.
fn frames() -> Vec<Value> {
    vec![
        json!({ "id": SITE, "components": {
            "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } } }),
        json!({ "id": FLOOR, "components": { "frame": {
            "parent": SITE, "dims": 2, "units": "mm",
            "transform_to_parent": { "translation": [0, 0, 0], "scale": [1, 1] } } } }),
        json!({ "id": LATTICE, "components": { "frame": {
            "parent": FLOOR, "dims": 2, "units": "index", "integer_only": true,
            "transform_to_parent": { "translation": [375, 375], "scale": [750, 750] } } } }),
    ]
}

fn world(required: &[&str], mut entities: Vec<Value>) -> Vec<Warning> {
    let mut all = frames();
    all.append(&mut entities);
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "3cbd200e-580a-40d8-b5ee-edf406aa1c88" },
        "primary_frame": SITE,
        "required_components": required,
        "entities": all,
    });
    let scenario = Document::from_value(Path::new("<test>"), &document).expect("it parses");
    derive(&LayerSet {
        scenario,
        layers: Vec::new(),
    })
    .expect("it loads")
    .warnings
}

/// Two cell_grids a third of a lattice pitch apart. Nothing joins them, and the
/// lattice cannot tell their cells apart: a legacy consumer reading that square
/// gets one of the two and no way to know which.
#[test]
fn two_unlinked_cell_grids_inside_one_lattice_square_warn() {
    // Cell centres at [450, 450] and [650, 450]. Projected — subtract the
    // half-pitch translation, divide by 750, round half-up — that is
    // (450−375)/750 = 0.1 and (650−375)/750 = 0.3667, both rounding to lattice
    // column 0; 0.1 rounds to row 0 for both.
    let warnings = world(
        &["frame", "region", "tiling"],
        vec![
            json!({ "id": "272542c8-2ea2-441f-8807-8a37aafc8f10", "components": {
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 900] },
                "tiling": { "pitch": [900, 900],
                            "derives": { "cell": { "traversable_by": ["htm"] } } } } }),
            json!({ "id": "cdda8cce-008d-4c20-b5b7-4d5a816e05c8", "components": {
                "region": { "frame": FLOOR, "origin": [200, 0], "size": [900, 900] },
                "tiling": { "pitch": [900, 900],
                            "derives": { "cell": { "traversable_by": ["htm"] } } } } }),
        ],
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    match &warnings[0] {
        Warning::GridProjectionCollisions {
            frame,
            squares,
            cells,
            samples,
        } => {
            assert_eq!(frame.to_string(), LATTICE);
            assert_eq!(*squares, 1);
            assert_eq!(*cells, 2);
            assert_eq!(samples[0].coordinate, [0, 0]);
            assert!(
                samples[0]
                    .cells
                    .iter()
                    .any(|c| c.ends_with("272542c8-2ea2-441f-8807-8a37aafc8f10/cell:0,0")),
                "{:?}",
                samples[0].cells
            );
            assert!(
                samples[0]
                    .cells
                    .iter()
                    .any(|c| c.ends_with("cdda8cce-008d-4c20-b5b7-4d5a816e05c8/cell:0,0")),
                "{:?}",
                samples[0].cells
            );
        }
        other => panic!("expected the lattice summary, got {other}"),
    }
}

/// The same fold, asked for: a cell standing exactly on the floor cell it is
/// stitched to. One seam, one square, nothing to say — this is the shape two
/// deliberately overlapping cell_grids have, and the whole reason the rule needs
/// an exemption rather than a threshold.
#[test]
fn a_stitch_linked_pair_sharing_a_lattice_square_is_silent() {
    let floor = "272542c8-2ea2-441f-8807-8a37aafc8f10";
    let perched = "fed8af20-b454-4764-bea8-123b0a780724";
    let warnings = world(
        &["frame", "region", "tiling", "stitch"],
        vec![
            json!({ "id": floor, "components": {
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [3000, 3000] },
                "tiling": { "pitch": [1000, 1000],
                            "derives": { "cell": { "traversable_by": ["htm"] } } } } }),
            // Dead centre of floor cell:1,1, which sits at [1500, 1500].
            json!({ "id": perched, "components": {
                "pose": { "frame": FLOOR, "position": [1500, 1500] },
                "cell": { "traversable_by": ["htm"] } } }),
            json!({ "id": "b71bb1ff-6a5b-4e12-babc-c39bae3de3ae", "components": {
                "stitch": { "a": perched, "b": floor, "rule": "contact",
                            "tolerance": 100 } } }),
        ],
    );
    assert_eq!(warnings.len(), 0, "{warnings:?}");
}

/// And the exemption is not a blanket one: take the stitch away and the same
/// geometry warns. The rule keys on what the author declared, not on how close
/// the two happen to be.
#[test]
fn the_same_pair_without_a_stitch_warns() {
    let floor = "272542c8-2ea2-441f-8807-8a37aafc8f10";
    let perched = "fed8af20-b454-4764-bea8-123b0a780724";
    let warnings = world(
        &["frame", "region", "tiling"],
        vec![
            json!({ "id": floor, "components": {
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [3000, 3000] },
                "tiling": { "pitch": [1000, 1000],
                            "derives": { "cell": { "traversable_by": ["htm"] } } } } }),
            json!({ "id": perched, "components": {
                "pose": { "frame": FLOOR, "position": [1500, 1500] },
                "cell": { "traversable_by": ["htm"] } } }),
        ],
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    // Both sit at [1500, 1500], which is (1500−375)/750 = 1.5 on each axis and
    // rounds half-up into square [2, 2]. The floor's other centres, 500 and
    // 2500, land on 0 and 3, so this pair is the only fold.
    assert!(matches!(
        &warnings[0],
        Warning::GridProjectionCollisions { squares: 1, cells: 2, samples, .. }
            if samples[0].coordinate == [2, 2]
    ));
}
