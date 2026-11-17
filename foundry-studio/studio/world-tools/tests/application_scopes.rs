//! The scopes the command line hands a module, read from a layered scenario.
//!
//! A catalog layer is a scope of its own, which the site pins, and every other
//! layer is the site's own, as the server's seed splits them. The meddler
//! probe changes whatever entity a file tells it to, so it is the module that
//! shows which of the two each entity reached it as.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use worldadapt_testkit::build_component_path;

#[path = "../../test-support/wps_source.rs"]
mod wps_source;

/// Walmart Dallas: a scenario of five layers, its catalog among them.
fn scenario() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/seed/application-tests/lenses/fixtures/walmart-dallas/scenario.world.json")
}

/// The catalog layer's identity, and one template it declares.
const CATALOG: &str = "5a9f3a20-1849-44aa-a6cd-a77b2e1a29e8";
const RACK_TEMPLATE: &str = "5d03cfed-c642-428f-b963-95224752359e";

/// A frame the world layer declares.
const FLOOR: &str = "edd0316d-a9bc-48f4-bb37-a1594ced8cd1";

/// Ask the meddler, through the command line, to patch `id` in the scenario.
fn patch(scratch: &Path, id: &str) -> Output {
    let meddler = wps_source::root().join("adapter/host/probes/meddler");
    let module = build_component_path(&meddler).expect("the meddler probe builds");
    let instruction = scratch.join("patch");
    fs::write(&instruction, id).expect("the meddler's instruction");
    Command::new(env!("CARGO_BIN_EXE_foundry-world"))
        .args([
            "adapter".as_ref(),
            "ingest".as_ref(),
            module.as_os_str(),
            scenario().as_os_str(),
            instruction.as_os_str(),
        ])
        .output()
        .expect("worldadapt starts")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// A change to an entity the catalog layer declares is refused as a change to
/// a pinned scope, named by the catalog's identity; one to an entity of the
/// world layer is the site's own, and passes.
#[test]
fn a_catalog_layer_is_a_pinned_scope_and_every_other_layer_the_sites_own() {
    let scratch = TempDir::new().expect("a scratch directory");

    let refused = patch(scratch.path(), RACK_TEMPLATE);
    assert!(!refused.status.success(), "a change to a catalog entity was accepted: {}", said(&refused));
    assert!(
        said(&refused).contains(&format!("{RACK_TEMPLATE}, which belongs to the pinned scope {CATALOG}")),
        "{}",
        said(&refused)
    );

    let accepted = patch(scratch.path(), FLOOR);
    assert!(accepted.status.success(), "{}", said(&accepted));
    let changes: Value = serde_json::from_slice(&accepted.stdout).expect("the changes are JSON");
    assert_eq!(changes[0]["op"], "patch");
    assert_eq!(changes[0]["id"], FLOOR);
}
