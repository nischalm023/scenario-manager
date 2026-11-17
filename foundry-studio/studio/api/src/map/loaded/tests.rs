//! The frame a loaded site states its positions in (spec 03 section 3.6), over the made L-Shape the viewer's tests draw
//! (`studio/viewer/tests/fixtures/l_shape/world.json`) with its primary frame's `unit_mm` restated.

use std::fs::read;
use std::path::Path;

use serde_json::json;

use super::*;

/// The made L-Shape, its primary frame's `unit_mm` set to `stated` or left out, and that frame's id; or why foundry's
/// reader refused it.
fn made_l_shape(stated: Option<Value>) -> Result<(WorldMap, EntityId), String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/tests/fixtures/l_shape/world.json");
    let mut document: Value = serde_json::from_slice(&read(path).expect("the viewer's fixture")).expect("JSON");
    let primary = document["primary_frame"].as_str().expect("a primary frame").to_owned();
    let entities = document["entities"].as_array_mut().expect("entities");
    let site = entities.iter_mut().find(|entity| entity["id"] == primary.as_str()).expect("the primary frame's entity");
    let frame = site["components"]["frame"].as_object_mut().expect("a frame component");
    match stated {
        Some(value) => frame.insert("unit_mm".into(), value),
        None => frame.remove("unit_mm"),
    };
    let world = floors::load(&serde_json::to_vec(&document).expect("bytes")).map_err(|why| why.to_string())?;
    Ok((world, primary.parse().expect("an entity id")))
}

/// World-proto's primary frame declares what one step is worth, fractional by design (0.1 mm by a site's default):
/// read as stated, never rounded to a whole millimetre (reviews-impl/astra-t14.md, the ninth P1).
#[test]
fn a_fractional_unit_reads_as_it_is_stated() {
    for stated in [0.1, 1.0] {
        let (world, primary) = made_l_shape(Some(json!(stated))).expect("the made L-Shape loads");
        let (unit, _) = frames(&world, &primary).expect("a frame stating its unit");
        assert_eq!(unit.mm(), stated);
    }
}

/// A primary frame that states no step a frame can have is refused, by foundry's reader or else here, never read as
/// one millimetre.
#[test]
fn a_unit_that_cannot_be_read_is_refused_rather_than_taken_as_one_millimetre() {
    for stated in [None, Some(json!("a tenth")), Some(json!(0)), Some(json!(-0.1))] {
        let read = made_l_shape(stated.clone()).and_then(|(world, primary)| frames(&world, &primary));
        assert!(read.is_err(), "{stated:?} read as {} mm a step", read.map_or(0.0, |(unit, _)| unit.mm()));
    }
}
