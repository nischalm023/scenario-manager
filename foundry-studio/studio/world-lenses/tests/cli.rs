//! The Foundry command retains native application loading and layer composition.
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[test]
fn native_cli_composes_authored_application_layers_before_querying() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_foundry-worldlens-check"))
        .args(["--lens", "rack_bay", "--count"])
        .arg(root.join("data/seed/application-tests/lenses/fixtures/walmart-dallas/scenario.world.json"))
        .arg(root.join("data/seed/application-tests/lenses/legacy-wps/world.lens"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "20\n");
}

#[test]
fn native_cli_projects_the_minimal_application_machine() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_foundry-worldlens-check"))
        .args(["--lens", "easyloop_station", "--array"])
        .arg(root.join("data/seed/application-tests/lenses/fixtures/minimal-easyloop/minimal_easyloop.world.json"))
        .arg(root.join("data/seed/application-tests/lenses/legacy-wps/world.lens"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let objects: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(objects.as_array().unwrap().len(), 1);
    for (field, expected) in [("cells", 28), ("ports", 2), ("pick_places", 2), ("waiting", 12)] {
        assert_eq!(objects[0][field].as_array().unwrap().len(), expected, "{field}");
    }
}
