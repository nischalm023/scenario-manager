//! Within `scenario-spec`, `variant` is how one of an experiment's variants changes a run's inputs (spec 07 section
//! 3.6), the same in the check and in the runner: a world patch sets one field of one entity's component in the
//! flagged export the runner fetched, a spec patch one field of the version's content, and each says what
//! the field held before (`was`), so a manifest can say "350, where the scenario has 300". Nothing is written to WPS.

use serde_json::{Map, Value};
use studio_protocol::{AppliedTo, Patch, ScenarioContent, Variant};
use thiserror::Error;

/// A patch applied, with the value it replaced; null where the field was not stated.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub patch: Patch,
    pub was: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VariantError {
    #[error("{label} names {target}, which this world does not hold")]
    TargetMissing { target: String, label: String },
    #[error("{label} sets {pointer} of {target}, a field whose parent this world does not hold")]
    FieldMissing { target: String, pointer: String, label: String },
    #[error("{label} cannot set {pointer}: {why}")]
    Refused { pointer: String, label: String, why: String },
}

/// Apply `variant` to `world`, the flagged export's JSON, and to `content`, in the patches' order.
pub fn apply_variant(
    variant: &Variant,
    world: &mut Value,
    content: &mut ScenarioContent,
) -> Result<Vec<Applied>, VariantError> {
    let mut applied = Vec::with_capacity(variant.patches.len());
    for patch in &variant.patches {
        let was = match patch.applied_to {
            AppliedTo::World => patch_world(world, patch)?,
            AppliedTo::Spec => patch_content(content, patch)?,
        };
        applied.push(Applied { patch: patch.clone(), was });
    }
    Ok(applied)
}

/// The entity whose id is the target, bare; its components hold the pointer's first segment.
fn patch_world(world: &mut Value, patch: &Patch) -> Result<Value, VariantError> {
    let missing = || VariantError::TargetMissing { target: patch.target.clone(), label: patch.label.clone() };
    let entities = world.get_mut("entities").and_then(Value::as_array_mut).ok_or_else(missing)?;
    let entity = entities
        .iter_mut()
        .find(|entity| entity.get("id").and_then(Value::as_str) == Some(patch.target.as_str()))
        .ok_or_else(missing)?;
    let components = entity.get_mut("components").ok_or_else(missing)?;
    set(components, &patch.pointer, patch.value.clone()).ok_or_else(|| VariantError::FieldMissing {
        target: patch.target.clone(),
        pointer: patch.pointer.clone(),
        label: patch.label.clone(),
    })
}

/// The content as JSON, patched, read back strictly, so a value its type refuses is refused here.
fn patch_content(content: &mut ScenarioContent, patch: &Patch) -> Result<Value, VariantError> {
    let refused =
        |why: String| VariantError::Refused { pointer: patch.pointer.clone(), label: patch.label.clone(), why };
    let mut json = serde_json::to_value(&*content).map_err(|fault| refused(fault.to_string()))?;
    let was = set(&mut json, &patch.pointer, patch.value.clone())
        .ok_or_else(|| refused("no field of a scenario has that pointer".to_owned()))?;
    *content = serde_json::from_value(json).map_err(|fault| refused(fault.to_string()))?;
    Ok(was)
}

/// Set the field at the RFC 6901 `pointer` under `root`, its parent an existing object; the value it held, or null.
pub fn set(root: &mut Value, pointer: &str, value: Value) -> Option<Value> {
    let (parent, last) = pointer.rsplit_once('/')?;
    let field = last.replace("~1", "/").replace("~0", "~");
    let object: &mut Map<String, Value> = root.pointer_mut(parent)?.as_object_mut()?;
    Some(object.insert(field, value).unwrap_or(Value::Null))
}
