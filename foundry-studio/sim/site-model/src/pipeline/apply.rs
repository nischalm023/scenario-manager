//! Step 4 of SPEC §8: letting hand-written records have the last word on
//! entities expansion produced.
//!
//! Overrides run first and suppressions after, both in layer order, and both are
//! refused outright when they address something no generator produces. That
//! refusal is the whole point: OpenUSD's `over` against a missing target
//! contributes nothing and says nothing, and a pinned cell that quietly
//! evaporates when a pitch changes is the failure this format was written
//! against. The `expect` pin is the same idea one level finer — a tiling key is
//! ordinal, so it keeps pointing *somewhere* after a pitch change, and only the
//! pin can tell whether it still points at the floor the author meant.

use crate::components::spatial::Pose;
use crate::document::Expect;
use crate::entity::Entity;
use crate::error::{LoadError, Warning};
use crate::ids::EntityId;
use crate::pipeline::expand::Expansion;
use crate::pipeline::merge::Merged;
use crate::placeholders;
use crate::units::Steps;
use serde_json::Value;
use std::collections::BTreeMap;

/// The entity set after every record has had its say, plus the records
/// themselves in the resolved form a bake writes back out.
#[derive(Debug, Clone)]
pub struct Applied {
    pub entities: BTreeMap<EntityId, Entity>,
    /// One entry per overridden target, components folded in layer order.
    pub overrides: BTreeMap<EntityId, BTreeMap<String, Value>>,
    /// Every target that ended up deleted, authored and exclusion-driven alike.
    pub suppressions: Vec<SuppressedTarget>,
    /// The `expect` pin each record carried, kept so the expanded form can write
    /// it back. A pin is authored data — it says where the author believed an
    /// ordinal address pointed — and an artifact that drops it hands the next
    /// reader a record that can be re-aimed silently.
    pub expects: BTreeMap<EntityId, Expect>,
    pub warnings: Vec<Warning>,
}

/// A deletion, and why it happened.
#[derive(Debug, Clone)]
pub struct SuppressedTarget {
    pub target: EntityId,
    /// The tiling whose own `exclusions` removed it, when it was not an authored
    /// suppression. SPEC §5 makes an exclusion "equivalent to a suppression of
    /// each" removed cell, and a bake records them as such.
    pub excluded_by: Option<EntityId>,
}

pub fn apply(merged: &Merged, expansion: &Expansion) -> Result<Applied, LoadError> {
    let mut entities = expansion.entities.clone();
    let mut warnings = Vec::new();
    let mut folded: BTreeMap<EntityId, BTreeMap<String, Value>> = BTreeMap::new();
    let mut expects: BTreeMap<EntityId, Expect> = BTreeMap::new();

    for record in &merged.overrides {
        let target: EntityId = record.address.clone().into();
        if !expansion.produced.contains(&target) {
            return Err(LoadError::dangling("an override", &record.address));
        }
        if let Some(stated) = &record.stated_id {
            if stated != &target {
                return Err(LoadError::DerivationIdMismatch {
                    doc: record.doc,
                    index: record.index,
                    stated: stated.clone(),
                    address: record.address.to_string(),
                });
            }
        }
        verify_expect(
            record.expect.as_ref(),
            &target,
            &record.address.to_string(),
            &entities,
            expansion,
        )?;
        if addresses_a_tiled_cell(&target, expansion) && record.expect.is_none() {
            warnings.push(Warning::UnpinnedTiledCellRecord {
                kind: "override",
                address: record.address.to_string(),
            });
        }

        if let Some(pin) = &record.expect {
            expects.insert(target.clone(), pin.clone());
        }
        let entry = folded.entry(target.clone()).or_default();
        placeholders::refuse(&target, &record.components)?;
        for (name, value) in &record.components {
            if crate::components::GENERATOR_INPUTS.contains(&name.as_str()) {
                warnings.push(Warning::OverrideReplacesGeneratorInput {
                    address: record.address.to_string(),
                    component: name.clone(),
                });
            }
            entry.insert(name.clone(), value.clone());
            // A cell an exclusion already removed is simply not here to change;
            // the record is still legitimate, because its derivation did produce
            // the address.
            if let Some(entity) = entities.get_mut(&target) {
                entity.components.insert(name.clone(), value.clone());
            }
        }
    }

    let mut suppressions = Vec::new();
    for (entity, tiling) in &expansion.excluded {
        suppressions.push(SuppressedTarget {
            target: entity.id.clone(),
            excluded_by: Some(tiling.clone()),
        });
    }

    for record in &merged.suppressions {
        let target = &record.target;
        match target {
            // An address a generator produced is reachable even after an
            // exclusion emptied it; a declared entity has to still be here.
            EntityId::Derived(address) => {
                if !expansion.produced.contains(target) {
                    return Err(LoadError::dangling("a suppression", address));
                }
                verify_expect(
                    record.expect.as_ref(),
                    target,
                    &address.to_string(),
                    &entities,
                    expansion,
                )?;
                if addresses_a_tiled_cell(target, expansion) && record.expect.is_none() {
                    warnings.push(Warning::UnpinnedTiledCellRecord {
                        kind: "suppression",
                        address: address.to_string(),
                    });
                }
            }
            EntityId::Declared(_) => {
                if !entities.contains_key(target) {
                    return Err(LoadError::DanglingDerivation {
                        kind: "a suppression",
                        address: format!("entity {target}"),
                    });
                }
            }
        }
        entities.remove(target);
        if let Some(pin) = &record.expect {
            expects.insert(target.clone(), pin.clone());
        }
        suppressions.push(SuppressedTarget {
            target: target.clone(),
            excluded_by: None,
        });
    }

    Ok(Applied {
        entities,
        overrides: folded,
        suppressions,
        expects,
        warnings,
    })
}

fn addresses_a_tiled_cell(target: &EntityId, expansion: &Expansion) -> bool {
    let entity = expansion
        .entities
        .get(target)
        .or_else(|| expansion.excluded.iter().find(|(entity, _)| &entity.id == target).map(|(entity, _)| entity));
    entity.is_some_and(|entity| {
        entity.has("cell")
            && entity
                .parent
                .as_ref()
                .and_then(|parent| expansion.entities.get(parent))
                .is_some_and(|parent| parent.has("tiling"))
    })
}

/// Check the pin: the entity must still sit within `tolerance` of the stated
/// position, measured in the steps of the generator's own region or pose frame —
/// which is the frame the derived pose is already expressed in, so no conversion
/// is involved and none can go wrong.
fn verify_expect(
    expect: Option<&Expect>,
    target: &EntityId,
    address: &str,
    entities: &BTreeMap<EntityId, Entity>,
    expansion: &Expansion,
) -> Result<(), LoadError> {
    let Some(expect) = expect else {
        return Ok(());
    };
    let entity = entities
        .get(target)
        .or_else(|| {
            expansion
                .excluded
                .iter()
                .find(|(e, _)| &e.id == target)
                .map(|(e, _)| e)
        })
        .ok_or_else(|| LoadError::DanglingDerivation {
            kind: "an `expect` pin",
            address: address.to_string(),
        })?;
    let Some(pose) = entity.component::<Pose>("pose")? else {
        return Err(LoadError::Unsupported {
            entity: target.clone(),
            message: format!("`expect` pins {address}, which carries no pose to check"),
        });
    };
    let Some(xy) = pose.xy() else {
        return Err(LoadError::Unsupported {
            entity: target.clone(),
            message: format!("`expect` pins {address}, whose pose has no planar position"),
        });
    };
    let actual = [Steps(xy[0].0), Steps(xy[1].0)];
    let expected = [Steps(expect.position[0].0), Steps(expect.position[1].0)];
    if Steps::beyond(actual, expected, expect.tolerance) {
        return Err(LoadError::ExpectFailed {
            address: address.to_string(),
            expected: [expected[0].0, expected[1].0],
            actual: [actual[0].0, actual[1].0],
            distance: Steps::distance(actual, expected),
            tolerance: expect.tolerance,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::FrameGraph;
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn tiled_cell_warning_uses_components_and_provenance_not_key_names() {
        let parent_id: EntityId = "a0000000-0000-4000-8000-000000000001".parse().unwrap();
        let mut parent = Entity::new(parent_id.clone());
        parent.components.insert("tiling".into(), json!({}));
        let real_id: EntityId = format!("{parent_id}/custom:floor").parse().unwrap();
        let misleading_id: EntityId = format!("{parent_id}/cell:label_only").parse().unwrap();
        let mut real = Entity::derived(real_id.clone(), parent_id.clone());
        real.components.insert("cell".into(), json!({}));
        let misleading = Entity::derived(misleading_id.clone(), parent_id.clone());
        let entities =
            BTreeMap::from([(parent_id.clone(), parent), (real_id.clone(), real), (misleading_id.clone(), misleading)]);
        let mut expansion = Expansion {
            frames: FrameGraph::build(&BTreeMap::new(), None).unwrap(),
            entities,
            produced: BTreeSet::new(),
            excluded: Vec::new(),
            counts: BTreeMap::new(),
        };
        assert!(addresses_a_tiled_cell(&real_id, &expansion));
        assert!(!addresses_a_tiled_cell(&misleading_id, &expansion));
        let removed = expansion.entities.remove(&real_id).unwrap();
        expansion.excluded.push((removed, parent_id));
        assert!(addresses_a_tiled_cell(&real_id, &expansion), "exclusions retain provenance");
    }
}
