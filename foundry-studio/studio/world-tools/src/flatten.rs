//! Merge a layered Foundry scenario into one authored document for WPS import.
//!
//! Application composition resolves layers before the server boundary. The
//! output retains declarations, derived overrides, authored suppressions and
//! labels in the existing import format. Suppressions produced by generation
//! (such as tiling exclusions) remain the responsibility of that generator.
//!
//! This preserves the former native client command's behavior. Document
//! metadata and import wire-format changes belong to the separate migration.

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use site_model::pipeline::layers::LayerSet;
use site_model::EntityId;
use site_model::World;
use std::path::Path;

/// The existing authored format version, also stamped on WPS exports.
const PROTO_VERSION: &str = "0.7.0";

pub fn run(scenario: &Path, out: Option<&Path>) -> Result<()> {
    let (set, world) = site_model::load(scenario).with_context(|| format!("loading {}", scenario.display()))?;
    let named = world.declared.values().filter(|entity| entity.name.is_some()).count();
    let suppressions = authored_suppressions(&world);

    let document = document(&set, &world, &suppressions);
    let entities = document["entities"].as_array().map_or(0, Vec::len);
    let text = serde_json::to_string_pretty(&document).context("writing the merged document")?;
    match out {
        Some(path) => std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?,
        None => println!("{text}"),
    }

    eprintln!(
        "{}: {entities} authored entries ({} declared, {} override records, {} suppressions)",
        scenario.display(),
        world.declared.len(),
        world.overrides.len(),
        suppressions.len(),
    );
    eprintln!("carried: {named} entity `name` labels");
    Ok(())
}

/// Only authored suppressions become document entries; generated exclusions
/// remain part of the application generator behavior.
fn authored_suppressions(world: &World) -> Vec<&EntityId> {
    world
        .suppressions
        .iter()
        .filter(|suppressed| suppressed.excluded_by.is_none())
        .map(|suppressed| &suppressed.target)
        .collect()
}

/// The merged authored tier of a loaded world as one document — the shape the
/// server's import boundary reads.
///
/// Shared by application callers and equivalence tests so the authored output
/// has one implementation.
///
/// The prologue is the scenario's own, so what comes out names the same
/// document the merge started from rather than a new one per run: an identity
/// minted here would make two flattens of one file two different documents.
/// The primary frame comes from the merged world, because a layer set states
/// it once and the merge is what decides which statement won.
pub fn merged_document(set: &LayerSet, world: &World) -> Value {
    document(set, world, &authored_suppressions(world))
}

fn document(set: &LayerSet, world: &World, suppressions: &[&EntityId]) -> Value {
    let mut entities: Vec<Value> =
        Vec::with_capacity(world.declared.len() + world.overrides.len() + suppressions.len());
    for entity in world.declared.values() {
        let mut entry = Map::new();
        entry.insert("id".to_string(), json!(entity.id.to_string()));
        if let Some(name) = &entity.name {
            entry.insert("name".to_string(), json!(name));
        }
        entry.insert("components".to_string(), Value::Object(components(&entity.components)));
        entities.push(Value::Object(entry));
    }
    for (target, components_of) in &world.overrides {
        // As a derivation and not as an `id`: an address in `id` reads to the
        // protocol's own loader as somebody declaring one, and it refuses the
        // file (`document::classify`). A record on a declared identity cannot
        // arise — an override addresses something a generator produced — so the
        // fallback arm writes the identity plainly and never runs.
        let mut entry = Map::new();
        match target {
            EntityId::Derived(address) => entry.insert(
                "derivation".to_string(),
                json!({
                    "generator": address.root().to_string(),
                    "key": address.key().to_string(),
                }),
            ),
            declared => entry.insert("id".to_string(), json!(declared.to_string())),
        };
        entry.insert("components".to_string(), Value::Object(components(components_of)));
        entities.push(Value::Object(entry));
    }
    for target in suppressions {
        entities.push(json!({ "id": target.to_string(), "suppress": true }));
    }
    let meta = &set.scenario.meta;
    let mut doc = Map::new();
    doc.insert("id".to_string(), json!(meta.id.to_string()));
    doc.insert("kind".to_string(), json!(meta.kind.clone().unwrap_or_else(|| "world".to_string())));
    if let Some(name) = &meta.name {
        doc.insert("name".to_string(), json!(name));
    }
    let mut out = Map::new();
    out.insert("proto_version".to_string(), json!(PROTO_VERSION));
    out.insert("doc".to_string(), Value::Object(doc));
    if let Some(primary) = world.frames.primary() {
        out.insert("primary_frame".to_string(), json!(primary.to_string()));
    }
    out.insert("entities".to_string(), Value::Array(entities));
    Value::Object(out)
}

/// A component map as JSON. The crate's `BTreeMap` and `serde_json`'s `Map` are
/// both key-ordered, so this is a re-key rather than a reordering.
fn components<'a, I>(components: I) -> Map<String, Value>
where
    I: IntoIterator<Item = (&'a String, &'a Value)>,
{
    components.into_iter().map(|(name, payload)| (name.clone(), payload.clone())).collect()
}
