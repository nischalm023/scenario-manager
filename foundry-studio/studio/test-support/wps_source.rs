//! Inspect the same WPS source that Cargo selected for the application tests.
//! This works with a local checkout now and a pinned Git dependency later.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

pub fn root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args(["metadata", "--format-version", "1", "--locked", "--offline"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("read Cargo's resolved WPS dependency");
        assert!(output.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&output.stderr));
        let metadata: Value = serde_json::from_slice(&output.stdout).expect("Cargo metadata is JSON");
        let packages: Vec<_> = metadata["packages"]
            .as_array()
            .expect("Cargo packages")
            .iter()
            .filter(|package| package["name"] == "worldprotocol-core")
            .collect();
        assert_eq!(packages.len(), 1, "the application must use one shared WPS identity crate");
        let manifest = PathBuf::from(packages[0]["manifest_path"].as_str().expect("WPS core manifest path"));
        manifest.parent().expect("core directory").join("../..").canonicalize().expect("WPS source root")
    })
}
