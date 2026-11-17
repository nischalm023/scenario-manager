//! What the engine's tests share: the registry as world-proto-server holds it (the eleven Agent Studio definitions
//! from design/vocabulary, the protocol components the seed uses, the server's definitions document), and a
//! template read from the development seed on disk the way the API reads one from the server.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use site_template::{RefAnnotation, RegisteredComponent, RegistrySnapshot, ScopeSnapshot, TemplateRead};
use studio_protocol::{ScopePath, Seq};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn seed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/studio")
}

/// The customer sites' seed, which a store is seeded from beside the development seed.
pub fn customer_seed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/customer")
}

pub fn json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()))
}

/// The eleven definitions of design/vocabulary, as copied into the fixtures, each a registry definition file.
pub fn vocabulary() -> Vec<Value> {
    let mut files: Vec<PathBuf> =
        fs::read_dir(fixtures().join("vocabulary")).unwrap().map(|entry| entry.unwrap().path()).collect();
    files.sort();
    files.iter().map(|path| json(path)).collect()
}

/// Agent Studio's definitions as dev.sh's seed registers them, from `data/seed/application/components`.
pub fn application_definitions() -> Vec<Value> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application/components");
    let mut files: Vec<PathBuf> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).collect();
    files.sort();
    files.iter().map(|path| json(path)).collect()
}

pub fn registry() -> RegistrySnapshot {
    registry_with(Vec::new())
}

/// The registry with `extra` defined beside what the fixtures hold.
pub fn registry_with(extra: Vec<RegisteredComponent>) -> RegistrySnapshot {
    let mut components = extra;
    for definition in vocabulary() {
        let refs = definition["refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|annotation| RefAnnotation {
                path: annotation["path"].as_str().unwrap().to_owned(),
                expects: annotation["expects"].as_str().map(str::to_owned),
            })
            .collect();
        components.push(RegisteredComponent {
            name: definition["component"].as_str().unwrap().to_owned(),
            version: "0.1.0".into(),
            schema: Some(definition["schema"].clone()),
            refs,
        });
    }
    let core = json(&fixtures().join("registry/core.json"));
    for (name, entry) in core["components"].as_object().unwrap() {
        components.push(RegisteredComponent {
            name: name.clone(),
            version: entry["version"].as_str().unwrap().to_owned(),
            schema: entry.get("schema").cloned(),
            refs: serde_json::from_value(entry["refs"].clone()).unwrap(),
        });
    }
    let defs = json(&fixtures().join("registry/server-defs.json"));
    RegistrySnapshot::new(components, Some(&defs)).expect("the fixtures' schemas compile")
}

/// Every entry of `data/seed/studio/manifest.json`: a scope and the document seeded into it.
pub fn manifest() -> Vec<(ScopePath, PathBuf)> {
    manifest_in(&seed_dir())
}

/// Every entry of the manifest in `seed`: a scope and the document seeded into it.
pub fn manifest_in(seed: &Path) -> Vec<(ScopePath, PathBuf)> {
    let manifest = json(&seed.join("manifest.json"));
    manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (entry["scope"].as_str().unwrap().parse().unwrap(), seed.join(entry["document"].as_str().unwrap()))
        })
        .collect()
}

fn snapshot(scope: &ScopePath, path: &Path) -> ScopeSnapshot {
    ScopeSnapshot::from_document(scope.clone(), Seq::new(1), &json(path)).unwrap()
}

/// The template `template/site/{slug}` read whole from the seed: its scopes, and the catalog scopes its top
/// scope's prologue depends on, each at changeset 1 as a freshly seeded server holds them.
pub fn read_template(slug: &str) -> TemplateRead {
    let root: ScopePath = format!("template/site/{slug}").parse().unwrap();
    let entries = manifest();
    let scopes: Vec<ScopeSnapshot> =
        entries.iter().filter(|(scope, _)| root.holds(scope)).map(|(scope, path)| snapshot(scope, path)).collect();
    let top = scopes.iter().find(|snapshot| snapshot.scope == root).expect("the template's top scope is seeded");
    let wanted = top.prologue.as_ref().unwrap().depends_on.clone();
    let catalog = entries
        .iter()
        .filter(|(scope, _)| scope.as_str().starts_with("catalog/"))
        .map(|(scope, path)| snapshot(scope, path))
        .filter(|snapshot| wanted.contains(&snapshot.prologue.as_ref().unwrap().doc))
        .collect();
    TemplateRead { template: root, tag: None, scopes, catalog, registry: registry() }
}
