//! Application layer composition for the generic adapter host.
//! Catalog membership follows the existing authored-document convention.

use site_model::LoadError;
use std::collections::BTreeMap;
use std::path::Path;
use worldadapt_cli::CliError;
use worldadapt_host::{Entity, Manifest, Site};
use site_model::document::Record;
use site_model::pipeline::{layers, merge};
use site_model::EntityId;

const CATALOG: &str = "catalog";

pub fn site(document: &Path, manifest: &Manifest) -> Result<Site, CliError> {
    let reading =
        |source: LoadError| CliError::Site { document: document.into(), reason: source.to_string() };
    let (set, entities) = if manifest.reads_derived {
        let (set, world) = site_model::load(document).map_err(reading)?;
        (set, world.entities)
    } else {
        let set = layers::resolve(document).map_err(reading)?;
        let merged = merge::merge(&set).map_err(reading)?;
        (set, merged.entities)
    };
    let own = set.scenario.meta.id.to_string();
    let mut pinned: BTreeMap<EntityId, String> = BTreeMap::default();
    for catalog in set.layers.iter().filter(|layer| layer.meta.kind.as_deref() == Some(CATALOG)) {
        let scope = catalog.meta.id.to_string();
        for record in &catalog.records {
            if let Record::Declaration(declared) = record {
                pinned.insert(EntityId::from(declared.id), scope.clone());
            }
        }
    }
    let entities = entities
        .into_values()
        .map(|entity| Entity {
            scope: pinned.get(&entity.id).cloned().unwrap_or_else(|| own.clone()),
            id: entity.id.to_string(),
            derived: entity.is_derived(),
            name: entity.name,
            components: entity.components.into_iter().collect(),
        })
        .collect();
    Ok(Site { scope: own, entities })
}
