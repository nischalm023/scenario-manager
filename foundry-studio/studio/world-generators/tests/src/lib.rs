//! Shared inputs for Foundry's application module tests. Modules are built by the
//! explicit SDK wrapper; expected application semantics come from Foundry's reader.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use worldgen_host::{EntityView, Object};
use site_model::LoadError;

pub fn expanded_form(path: &Path) -> Result<Value, LoadError> {
    site_model::load(path).map(|(set, world)| site_model::expanded::emit(&set, &world))
}
pub fn anchor_object(id: &str, components: Value) -> Object {
    Object { anchor: EntityView { id: id.into(), name: None, components: components.to_string() }, fields: vec![] }
}
pub fn stamp_object(id: &str, components: Value, template: Option<(&str, Value)>) -> Object {
    let mut object = anchor_object(id, components);
    object.fields.push(("template".into(), template.into_iter().map(|(id, components)|
        EntityView { id: id.into(), name: None, components: components.to_string() }).collect()));
    object
}
pub fn tiling_object(id: &str, components: Value) -> Object { anchor_object(id, components) }
fn module(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(std::env::var_os("FOUNDRY_PARITY_MODULES").expect("module directory")).join(name)).unwrap()
}
pub fn tiling_module() -> &'static [u8] {
    static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
    MODULE.get_or_init(|| module("tiling.wasm"))
}
pub fn stamp_module() -> &'static [u8] {
    static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
    MODULE.get_or_init(|| module("stamp.wasm"))
}
pub fn rack_module() -> &'static [u8] {
    static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
    MODULE.get_or_init(|| module("rack.wasm"))
}

pub fn cohort_module() -> &'static [u8] {
    static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
    MODULE.get_or_init(|| module("cohort.wasm"))
}

pub fn carrier_pool_module() -> &'static [u8] {
    static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
    MODULE.get_or_init(|| module("carrier_pool.wasm"))
}
