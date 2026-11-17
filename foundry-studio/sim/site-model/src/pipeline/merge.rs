//! Step 2 of SPEC §8: folding the layers' declarations into one entity set,
//! before anything expands.
//!
//! The order matters more than the mechanism. Merging first is what lets a
//! policy overlay change a lane's direction, a stamp's parameters, or a whole
//! template and have the *generator* compile the change, rather than patching up
//! cells after they exist. Replacement is per component and wholesale — no deep
//! merge — so a merged entity is always some layer's exact statement about that
//! component, and a diff between two runs reads as a list of components rather
//! than a tree of field edits.

use crate::document::{Declaration, Override, Record, Suppression};
use crate::entity::Entity;
use crate::error::LoadError;
use crate::ids::EntityId;
use crate::pipeline::layers::LayerSet;
use std::collections::BTreeMap;

/// The merged declarations, plus the records that will not be applied until
/// after expansion.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    pub entities: BTreeMap<EntityId, Entity>,
    /// In layer order, so a later layer's override of the same component wins.
    pub overrides: Vec<Override>,
    pub suppressions: Vec<Suppression>,
    pub primary_frame: Option<EntityId>,
}

/// Union the declared entities across layers by id, replacing per component.
pub fn merge(set: &LayerSet) -> Result<Merged, LoadError> {
    let mut merged = Merged::default();
    for document in set.documents() {
        if let Some(frame) = &document.primary_frame {
            merged.primary_frame = Some(frame.clone());
        }
        for record in &document.records {
            match record {
                Record::Declaration(d) => apply_declaration(&mut merged.entities, d),
                Record::Patch(d) => {
                    if !merged.entities.contains_key(&EntityId::from(d.id)) {
                        return Err(LoadError::PatchMissesTarget {
                            doc: document.meta.id,
                            entity: d.id.into(),
                        });
                    }
                    apply_declaration(&mut merged.entities, d);
                }
                Record::Override(o) => merged.overrides.push(o.clone()),
                Record::Suppression(s) => merged.suppressions.push(s.clone()),
            }
        }
    }
    Ok(merged)
}

fn apply_declaration(entities: &mut BTreeMap<EntityId, Entity>, d: &Declaration) {
    let entity = entities
        .entry(d.id.into())
        .or_insert_with(|| Entity::new(d.id));
    if d.name.is_some() {
        entity.name = d.name.clone();
    }
    for (name, value) in &d.components {
        entity.components.insert(name.clone(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::pipeline::layers::LayerSet;
    use serde_json::{json, Value};
    use std::path::Path;
    use uuid::Uuid;

    fn document(name: &str, entities: Value) -> Document {
        let value = json!({
            "proto_version": "0.7.0",
            "doc": { "id": format!("da000000-0000-4000-8000-00000000000{name}") },
            "entities": entities
        });
        Document::from_value(Path::new(name), &value).unwrap()
    }

    fn merge_layers(layers: Vec<Document>) -> Result<Merged, LoadError> {
        let scenario = document("9", json!([]));
        merge(&LayerSet { scenario, layers })
    }

    const AISLE: &str = "a0000000-0000-4000-8000-000000000002";

    #[test]
    fn a_later_layer_replaces_one_component_and_leaves_the_rest() {
        let world = document(
            "1",
            json!([{ "id": AISLE, "name": "aisle 1", "components": {
                "region": { "origin": [2000, 5300], "size": [16200, 1800] },
                "tiling": { "pitch": [900, 900] },
                "flow": { "rule": "one_way", "direction": "+y" }
            }}]),
        );
        let policy = document(
            "2",
            json!([{ "id": AISLE, "patch": true,
                     "components": { "flow": { "rule": "one_way", "direction": "+x" } } }]),
        );
        let merged = merge_layers(vec![world, policy]).unwrap();
        let entity = &merged.entities[&AISLE.parse::<Uuid>().map(EntityId::declared).unwrap()];
        assert_eq!(entity.name.as_deref(), Some("aisle 1"));
        assert_eq!(
            entity.components["flow"],
            json!({ "rule": "one_way", "direction": "+x" })
        );
        assert!(entity.components.contains_key("region"));
        assert!(entity.components.contains_key("tiling"));
    }

    #[test]
    fn a_patch_that_hits_nothing_fails_the_load() {
        let policy = document(
            "2",
            json!([{ "id": AISLE, "patch": true,
                     "components": { "flow": { "rule": "two_way" } } }]),
        );
        let err = merge_layers(vec![policy]).unwrap_err();
        assert!(matches!(err, LoadError::PatchMissesTarget { .. }), "{err}");
    }

    #[test]
    fn a_plain_declaration_in_a_later_layer_needs_no_earlier_one() {
        let later = document(
            "2",
            json!([{ "id": AISLE, "components": { "cell": { "blocked": true } } }]),
        );
        assert!(merge_layers(vec![later]).is_ok());
    }
}
