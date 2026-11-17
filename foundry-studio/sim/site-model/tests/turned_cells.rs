//! A cell stamped at a quarter turn meets its neighbours with the footprint the
//! stamp turned.
//!
//! A template states a cell's `size` along the template's own axes, as it states
//! a tiling's pitch, and a stamp carries both into the axes of the frame it is
//! posed in. Contact matching is where a footprint is read, so that is where
//! this is held: a 3 × 1 m cell stamped at 90° stands 1 m wide and 3 m deep, and
//! it touches the lane beside it only if its footprint was turned.

use serde_json::json;
use site_model::pipeline::layers::LayerSet;
use site_model::pipeline::{derive, World};
use site_model::{Document, EntityId, Steps};
use std::path::Path;

const FLOOR: &str = "f0000000-0000-4000-8000-000000000002";
const LANE: &str = "a0000000-0000-4000-8000-000000000001";
const TEMPLATE: &str = "c0000000-0000-4000-8000-000000000001";
const STAMP: &str = "b0000000-0000-4000-8000-000000000001";
const SEAM: &str = "d0000000-0000-4000-8000-000000000001";
/// A cell written out by hand with the components the stamp writes onto its
/// own, standing well clear of everything else.
const BY_HAND: &str = "e0000000-0000-4000-8000-000000000001";

fn id(text: &str) -> EntityId {
    text.parse().expect("a literal identity in this test")
}

/// A lane of three 1 m tiles along y = 0..1000, and above it a 3 × 1 m cell a
/// template states lying along its own x, stamped at 90°: the stamp puts its
/// centre at [4500, 2500], and turned it stands on x = 4000..5000, y =
/// 1000..4000, its short edge on the lane's long one. Unturned it would lie on
/// y = 2000..3000, a metre clear of the lane, and the seam — tolerance zero —
/// would link nothing. One step is one millimetre.
fn world() -> World {
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-000000000001" },
        "primary_frame": FLOOR,
        "entities": [
            { "id": FLOOR, "components": {
                "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } },
            { "id": LANE, "name": "lane", "components": {
                "region": { "frame": FLOOR, "origin": [3000, 0], "size": [3000, 1000] },
                "tiling": { "pitch": [1000, 1000],
                            "derives": { "cell": { "traversable_by": ["htm"] } } }
            }},
            { "id": TEMPLATE, "name": "a stand three metres long", "components": {
                "template": { "entities": [
                    { "local": "stand", "components": {
                        "pose": { "position": [1500, 500] },
                        "cell": { "size": [3000, 1000], "traversable_by": ["htm"] } } }
                ] }
            }},
            { "id": STAMP, "name": "the stand, turned", "components": {
                "pose": { "frame": FLOOR, "position": [5000, 1000], "heading_deg": 90 },
                "stamp": { "template": TEMPLATE }
            }},
            { "id": SEAM, "components": {
                "stitch": { "a": STAMP, "b": LANE, "rule": "contact", "tolerance": 0 }
            }},
            { "id": BY_HAND, "components": {
                "pose": { "frame": FLOOR, "position": [20000, 20000], "heading_deg": 90 },
                "cell": { "size": [1000, 3000], "traversable_by": ["htm"] }
            }}
        ],
    });
    let scenario = Document::from_value(Path::new("turned"), &value).expect("the document parses");
    derive(&LayerSet { scenario, layers: Vec::new() }).expect("a turned cell touches the lane, so the seam links")
}

/// The seam links the turned cell to the lane tile its short edge stands on,
/// and to no other: the two tiles beside that one are a full tile off its
/// centre line, beyond half the wider of the two footprints.
#[test]
#[ignore = "Known Foundry rotated-cell gap; fwd bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg (ownership preserves current geometry)"]
fn a_cell_stamped_at_a_quarter_turn_meets_the_lane_with_its_turned_footprint() {
    let world = world();
    let stand = id(&format!("{STAMP}/instance:0/local:stand"));
    let geometry = &world.cells[&stand];
    assert_eq!(geometry.centre, [Steps(4500), Steps(2500)]);
    assert_eq!(geometry.footprint, [Steps(1000), Steps(3000)]);

    let links: Vec<(&EntityId, &EntityId)> =
        world.edges.iter().filter(|edge| edge.owner == id(SEAM)).map(|edge| (&edge.from, &edge.to)).collect();
    assert_eq!(links, [(&stand, &id(&format!("{LANE}/cell:1,0")))]);
}

/// The stamp turned the size it wrote, and the heading it composed onto the same
/// cell turns nothing more: a cell written by hand with those very components —
/// heading 90°, size 1 × 3 m — stands on the same footprint. A cell's extent is
/// read along its frame's axes whatever its heading says.
#[test]
#[ignore = "Known Foundry rotated-cell gap; fwd bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg (ownership preserves current geometry)"]
fn a_hand_written_cell_with_the_stamped_components_has_the_same_footprint() {
    let world = world();
    let stand = id(&format!("{STAMP}/instance:0/local:stand"));
    let stamped = &world.entities[&stand].components;
    assert_eq!(stamped["pose"]["heading_deg"], json!(90.0));
    assert_eq!(stamped["cell"]["size"], json!([1000, 3000]));
    assert_eq!(world.cells[&id(BY_HAND)].footprint, world.cells[&stand].footprint);
}
