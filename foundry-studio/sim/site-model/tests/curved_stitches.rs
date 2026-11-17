//! Curvature belongs to explicit links; it neither generates cells nor changes
//! the default meaning of existing straight stitches.
use serde_json::{json, Value};
use std::path::Path;
use site_model::pipeline::layers::LayerSet;
use site_model::pipeline::{derive, World};
use site_model::{Document, LoadError};

const FLOOR: &str = "f0000000-0000-4000-8000-000000000001";
const FRAME: &str = "f0000000-0000-4000-8000-000000000002";
const A: &str = "a0000000-0000-4000-8000-000000000001";
const B: &str = "a0000000-0000-4000-8000-000000000002";
const LINK: &str = "e0000000-0000-4000-8000-000000000001";

fn load(scale: [f64; 2], endpoint: [i64; 2], extra: Value) -> Result<World, LoadError> {
    let mut stitch = json!({"a": A, "b": B, "rule": "explicit", "pairs": [[A, B]]});
    stitch
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let source = json!({
        "proto_version": "0.7.0", "doc": {"id": "d0000000-0000-4000-8000-000000000001"},
        "primary_frame": FLOOR,
        "entities": [
            {"id": FLOOR, "components": {"frame": {"dims": 2,"units": "mm","unit_mm": 0.1}}},
            {"id": FRAME, "components": {"frame": {"dims": 2,"units": "mm","parent": FLOOR,
                "transform_to_parent": {"scale": scale,"translation": [20000,30000]}}}},
            {"id": A, "components": {"pose": {"frame": FRAME,"position": [-10000,0]},"cell": {}}},
            {"id": B, "components": {"pose": {"frame": FRAME,"position": endpoint},"cell": {}}},
            {"id": LINK, "components": {"stitch": stitch}}
        ]
    });
    let scenario = Document::from_value(Path::new("curved-link"), &source)?;
    derive(&LayerSet {
        scenario,
        layers: vec![],
    })
}

fn curve() -> Value {
    json!({"bidirectional": false,"curve": {"frame": FRAME,"center": [0,0],"clockwise": false}})
}

#[test]
fn reflection_transforms_the_curve_without_minting_any_cells() {
    for (scale, clockwise) in [([1.0, 1.0], false), ([-1.0, 1.0], true)] {
        let world = load(scale, [0, -10000], curve()).unwrap();
        assert_eq!(world.cells.len(), 2);
        assert_eq!(world.edges.len(), 1);
        let link = &world.edges[0];
        assert!(!link.bidirectional);
        let arc = link.curve.as_ref().unwrap();
        assert_eq!(arc.center.map(|v| v.0), [20000, 30000]);
        assert_eq!(arc.clockwise, clockwise);
    }
}

#[test]
fn old_stitches_remain_straight_and_bidirectional() {
    let world = load([1.0, 1.0], [0, -10000], json!({})).unwrap();
    assert!(world.edges[0].bidirectional);
    assert!(world.edges[0].curve.is_none());
}

#[test]
fn an_ellipse_or_a_point_off_the_circle_is_refused() {
    for (scale, end) in [
        ([2.0, 1.0], [0, -10000]),
        ([1.0, 1.0], [0, -12000]),
        ([0.0, 0.0], [0, -10000]),
    ] {
        assert!(load(scale, end, curve()).is_err());
    }
}

#[test]
fn contact_direction_cannot_be_silently_ignored() {
    let mut extra = curve();
    extra["rule"] = json!("contact");
    assert!(load([1.0, 1.0], [0, -10000], extra).is_err());
}

#[test]
fn a_tangent_lead_transforms_with_its_circle_and_is_not_a_cell() {
    for scale in [[1.0, 1.0], [-1.0, 1.0]] {
        let mut extra = curve();
        extra["curve"]["end"] = json!([10000, 0]);
        let world = load(scale, [10000, 7000], extra).unwrap();
        assert_eq!(world.cells.len(), 2);
        let arc = world.edges[0].curve.as_ref().unwrap();
        assert_eq!(
            arc.end.unwrap().map(|v| v.0),
            [(20000.0 + scale[0] * 10000.0) as i64, 30000]
        );
        assert_eq!(arc.start, None);
    }
}

#[test]
fn a_lead_cannot_cut_a_corner_or_reverse_at_the_join() {
    for endpoint in [[11000, 7000], [10000, -7000]] {
        let mut extra = curve();
        extra["curve"]["end"] = json!([10000, 0]);
        assert!(load([1.0, 1.0], endpoint, extra).is_err());
    }
}
