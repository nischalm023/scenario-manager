//! What the Map Creator adapter says about itself before a host hands it
//! anything.

use mapcreator_adapter_tests::mapcreator;
use worldadapt_host::Manifest;

/// The name a registry holds it under, its version, the format in words, both
/// directions, and declared entities only.
#[test]
fn the_module_says_what_it_is_and_that_it_translates_both_ways() {
    let manifest = mapcreator().manifest().clone();
    assert_eq!(
        manifest,
        Manifest {
            name: "mapcreator".into(),
            version: "0.1.0".into(),
            format: "Map Creator site export".into(),
            projects: true,
            ingests: true,
            reads_derived: false,
        }
    );
}
