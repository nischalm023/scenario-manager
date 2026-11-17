//! Within the tests' support, `generation` is what the in-memory world-proto-server generates a site's entities with
//! when it writes a flagged export: site-model's native expansion, standing in for the WASM generators
//! world-proto-server runs (ruling D175), which the mock cannot run.

use std::path::Path;

use serde_json::{Value, json};
use site_model::Document;
use site_model::pipeline::layers::LayerSet;
use site_model::pipeline::{apply, expand, merge};
use wps_client::MockWps;

/// The components that make an entity a generator, each read by one of the native expansion's generators.
const GENERATORS: [&str; 5] = ["tiling", "rack", "stamp", "cohort", "carrier_pool"];

/// Give `mock` the native expansion as its generation.
pub fn native(mock: &MockWps) {
    mock.set_generation(Box::new(generate));
}

/// Every entity the native expansion generates from `entities`, a document's declarations and records, with the
/// records laid over: none where no entity is a generator, so a site of declarations alone reaches the reader as it
/// stands. The frame that states `unit_mm` is the document's primary frame (ruling D177).
fn generate(entities: Vec<Value>) -> Result<Vec<Value>, String> {
    let generates = |entry: &Value| {
        let components = entry.get("components").and_then(Value::as_object);
        components.is_some_and(|components| GENERATORS.iter().any(|name| components.contains_key(*name)))
    };
    if !entities.iter().any(generates) {
        return Ok(Vec::new());
    }
    let primary = entities.iter().find(|entry| entry.pointer("/components/frame/unit_mm").is_some());
    let mut document = json!({ "proto_version": "0.7.0", "doc": { "id": "da000000-0000-4000-8000-000000000001" } });
    if let Some(frame) = primary.and_then(|entry| entry.get("id")) {
        document["primary_frame"] = frame.clone();
    }
    document["entities"] = Value::Array(entities);
    let scenario = Document::from_value(Path::new("the mock's generation"), &document).map_err(|e| e.to_string())?;
    let set = LayerSet { scenario, layers: Vec::new() };
    let merged = merge::merge(&set).map_err(|e| e.to_string())?;
    let expansion = expand::expand(&merged).map_err(|e| e.to_string())?;
    let applied = apply::apply(&merged, &expansion).map_err(|e| e.to_string())?;
    let generated = applied.entities.into_values().filter(|entity| entity.is_derived());
    Ok(generated
        .map(|entity| {
            let mut entry = json!({ "id": entity.id, "components": entity.components });
            if let Some(name) = entity.name {
                entry["name"] = json!(name);
            }
            entry
        })
        .collect())
}
