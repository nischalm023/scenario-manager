//! Application regression inputs preserved from World Proto's lens suite.
//! Their authored JSON and layer closure now live in Foundry's seed test data.
//! These are native-reader inputs, not exports or WPS snapshots.

// Each test binary compiles this module and calls the part of it that binary
// needs; what the others call is not dead, it is somebody else's.
#![allow(dead_code)]

use site_model::pipeline::World;
use site_model::LayerSet;
use std::path::{Path, PathBuf};

pub struct Fixture {
    pub name: &'static str,
    pub layers: LayerSet,
    pub world: World,
}

/// The scenario file each fixture is entered through. Every count table in this
/// suite carries one column per row of this list, in this order, so a document
/// joins the suite as one line here and one number per pattern there.
///
pub fn paths() -> [(&'static str, PathBuf); 4] {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let reader = crate_dir.join("../../data/seed/application-tests/lenses/fixtures");
    [
        ("cencora-v3", reader.join("cencora-v3/cencora_v3.world.json")),
        ("walmart-dallas", reader.join("walmart-dallas/scenario.world.json")),
        ("minimal-putaway2", reader.join("minimal-putaway2/minimal_putaway2.world.json")),
        ("minimal-easyloop", reader.join("minimal-easyloop/minimal_easyloop.world.json")),
    ]
}

pub fn documents() -> Vec<Fixture> {
    paths()
        .into_iter()
        .map(|(name, path)| {
            let (layers, world) = site_model::load(&path).unwrap_or_else(|error| panic!("{name} loads: {error}"));
            Fixture { name, layers, world }
        })
        .collect()
}

/// The synthetic application document preserved with the site inputs: the corner cases that decide a
/// rule of the language and that no surveyed site happens to contain — an empty
/// list, one dotted path two component names both claim, one entity that names
/// others in every spelling the format has and three that name nothing, and one
/// stamp whose template internal carries a `frame`, which is how a frame nobody
/// declared comes to exist.
///
/// It still states two presentation points equidistant from a station and a
/// region edged on two cell centres, which decided the tie-break and the closed
/// interval before those relations were repealed (2026-09-16). The document is
/// left as it stands rather than trimmed: one of those points is what the
/// sector membership case names, and a hand-written fixture that two tests read
/// is cheaper to keep whole than to re-cut.
pub fn corners() -> Fixture {
    let (name, path) = ("corner-cases", corners_path());
    let (layers, world) = site_model::load(&path).expect("the corner-case fixture loads");
    Fixture { name, layers, world }
}

pub fn corners_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/seed/application-tests/lenses/fixtures/corner-cases.world.json")
}

/// One fixture by name, for a test that only needs the one.
pub fn document(want: &str) -> Fixture {
    let (name, path) = paths().into_iter().find(|(name, _)| *name == want).expect("a known fixture name");
    let (layers, world) = site_model::load(&path).unwrap_or_else(|error| panic!("{name} loads: {error}"));
    Fixture { name, layers, world }
}
