//! Within site-model, a `flagged_export` is a whole site as WPS generated it, with every entity it references: the
//! file world-proto-server writes at `GET /scopes/<path>/-/export?nested=1&derived=1&referenced=1` (Agent Studio's
//! ruling D185), after wps-client has checked it and stripped its pins. This module reads that file into the same
//! [`World`] the native pipeline derives, so a run builds from the site exactly as WPS generated it and nothing in
//! Foundry generates the site a second time (ruling D175).
//!
//! The file lists every entity: the declared ones, and the generated ones with their overrides already laid over and
//! the suppressed ones gone. So the steps of SPEC §8 that make or remove entities — merging layers, the native
//! expansion and applying records — have nothing left to do: this reader builds the frame graph the entities stand
//! in and hands them to the steps that read entities as they stand, the ones the native pipeline ends with
//! (`pipeline::finish`): each cell's geometry, the arcs, relay binding and the checks.
//!
//! The file states no primary frame and no prologue. The primary frame is the one frame whose `frame` component
//! states `unit_mm` (ruling D177).

use crate::components::spatial::Frame;
use crate::derivation::DerivationKey;
use crate::entity::Entity;
use crate::error::{spoken, LoadError};
use crate::frames::FrameGraph;
use crate::ids::{DerivedEntityId, EntityId};
use crate::pipeline::apply::Applied;
use crate::pipeline::{self, World};
use crate::placeholders;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::iter;
use std::path::Path;

// The keys that tell the two files apart: a flagged export holds those of the first beside `entities` (the scope its
// request named, its three flags and the scopes it holds entities from), and no authored document does; an authored
// document's prologue holds those of the second, and no flagged export does. A world is built only from a flagged
// export stating each of the third: the site whole with its nested scopes, as WPS generated it, with every entity it
// references.
const FLAGGED_KEYS: [&str; 5] = ["derived", "nested", "referenced", "scope", "scopes"];
const PROLOGUE_KEYS: [&str; 3] = ["doc", "primary_frame", "proto_version"];
const FLAGS: [&str; 3] = ["nested", "derived", "referenced"];

/// Whether `file` is a flagged export, which this module reads, rather than an authored document, which the native
/// pipeline reads: a flagged export names the scope it exports under `scope`, as a string, beside its three flags,
/// and holds none of an authored document's prologue. A file marked both ways is refused rather than read one way,
/// which would lose what the other way reads, and so is a `scope` that names no scope.
pub fn is_flagged_export(path: &Path, file: &Value) -> Result<bool, LoadError> {
    let flagged = named_keys(file, &FLAGGED_KEYS);
    if flagged.is_empty() {
        return Ok(false);
    }
    let authored = named_keys(file, &PROLOGUE_KEYS);
    if !authored.is_empty() {
        return Err(LoadError::FlaggedBesideAuthored { path: path.to_path_buf(), flagged, authored });
    }
    match file.get("scope") {
        Some(Value::String(_)) => Ok(true),
        named => {
            let named = named.map_or_else(|| "nothing".to_owned(), Value::to_string);
            let message = format!("`scope` names the scope a flagged export exports, as a string, not {named}");
            Err(LoadError::Shape { path: path.to_path_buf(), message })
        }
    }
}

fn named_keys(file: &Value, keys: &[&str]) -> String {
    let held: Vec<&str> = keys.iter().copied().filter(|key| file.get(*key).is_some()).collect();
    spoken(&held)
}

/// Read `file` and build the world it holds.
pub fn world(path: &Path, file: &Value) -> Result<World, LoadError> {
    FlaggedExport::read(path, file)?.into_world()
}

/// One entity as the file writes it. Its `scope` is for wps-client and the Studio API to read; a world needs none.
#[derive(Deserialize)]
struct Row {
    id: EntityId,
    #[serde(default)]
    name: Option<String>,
    components: BTreeMap<String, Value>,
}

/// A flagged export's entities, each read and checked, its generated entities with their parents.
pub struct FlaggedExport {
    // ordered: they become the world's entities, a B-tree by identity, and two frames that state `unit_mm` are named
    // in identity order.
    entities: BTreeMap<EntityId, Entity>,
}

impl FlaggedExport {
    /// Read every entity of `file`, a flagged export with all three of its flags set, refusing a placeholder left
    /// outside a template and an identity listed twice, and give each generated entity its nearest ancestor in the
    /// file as its parent. WPS takes a generated entity's parent to be one step up its identity, and the native
    /// expansion the entity that generated it; the two differ where one generation step emits a key of several
    /// segments, and the nearest ancestor present is the one entity both readings can name.
    pub fn read(path: &Path, file: &Value) -> Result<FlaggedExport, LoadError> {
        let shape = |message: String| LoadError::Shape { path: path.to_path_buf(), message };
        refuse_unset_flags(path, file)?;
        let rows = file
            .get("entities")
            .and_then(Value::as_array)
            .ok_or_else(|| shape("a flagged export lists its entities under `entities`".into()))?;
        let mut entities = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            let Row { id, name, components } =
                Row::deserialize(row).map_err(|fault| shape(format!("entity #{index} does not read: {fault}")))?;
            placeholders::refuse(&id, &components)?;
            let entity = Entity { id: id.clone(), name, components, parent: None };
            if entities.insert(id.clone(), entity).is_some() {
                return Err(LoadError::ListedTwice { entity: id });
            }
        }
        let mut parents = Vec::new();
        for id in entities.keys() {
            let Some(derived) = id.as_derived() else { continue };
            let parent = nearest_ancestor(derived, &entities).ok_or_else(|| LoadError::GeneratedWithoutGenerator {
                entity: id.clone(),
                root: derived.root().into(),
            })?;
            parents.push((id.clone(), parent));
        }
        for (id, parent) in parents {
            if let Some(entity) = entities.get_mut(&id) {
                entity.parent = Some(parent);
            }
        }
        Ok(FlaggedExport { entities })
    }

    /// The one frame that states `unit_mm`, or none when no frame does; a caller that needs a primary frame refuses
    /// the file then.
    pub fn primary_frame(&self) -> Result<Option<EntityId>, LoadError> {
        let mut primary: Option<&EntityId> = None;
        for (id, entity) in &self.entities {
            let Some(frame) = entity.component::<Frame>("frame")? else { continue };
            if frame.unit_mm.is_none() {
                continue;
            }
            if let Some(first) = primary {
                return Err(LoadError::UnitOnTwoFrames { first: first.clone(), second: id.clone() });
            }
            primary = Some(id);
        }
        Ok(primary.cloned())
    }

    /// The world these entities make: the frame graph they stand in, then the pipeline's steps that read entities as
    /// they stand, as the native pipeline runs them once its records are laid.
    pub fn into_world(self) -> Result<World, LoadError> {
        let primary = self.primary_frame()?;
        let entities = self.entities;
        let frames = FrameGraph::build(&entities, primary)?;
        frames.refuse_geometry_in_lattice_frames(&entities)?;
        let declared: BTreeMap<EntityId, Entity> = entities
            .iter()
            .filter(|(id, _)| !id.is_derived())
            .map(|(id, entity)| (id.clone(), entity.clone()))
            .collect();
        // What a queue path may name: every generated entity, each of which survived its generation.
        let produced: BTreeSet<EntityId> = entities.keys().filter(|id| id.is_derived()).cloned().collect();
        // WPS laid every record over what it generated, so none is left to apply.
        let applied = Applied {
            entities,
            overrides: BTreeMap::new(),
            suppressions: Vec::new(),
            expects: BTreeMap::new(),
            warnings: Vec::new(),
        };
        pipeline::finish(applied, frames, &produced, declared)
    }
}

/// Refuse a flagged export that does not state each of [`FLAGS`] as true, naming each it leaves unset and the request
/// that sets them: one without `nested` may leave a scope of the site out, one without `derived` holds none of what
/// WPS generated, and one without `referenced` may name an entity it does not hold. A flag stated as neither `true`
/// nor `false` is refused by name.
fn refuse_unset_flags(path: &Path, file: &Value) -> Result<(), LoadError> {
    let mut unset = Vec::new();
    for flag in FLAGS {
        match file.get(flag) {
            Some(Value::Bool(true)) => {}
            None | Some(Value::Bool(false)) => unset.push(flag),
            Some(other) => {
                let message = format!("a flagged export states `{flag}` as `true` or `false`, not {other}");
                return Err(LoadError::Shape { path: path.to_path_buf(), message });
            }
        }
    }
    match unset.is_empty() {
        true => Ok(()),
        false => Err(LoadError::ExportFlagsUnset { path: path.to_path_buf(), unset }),
    }
}

/// The longest prefix of `id` the file holds: one step up its key first, the generator it starts at last.
fn nearest_ancestor(id: &DerivedEntityId, entities: &BTreeMap<EntityId, Entity>) -> Option<EntityId> {
    let segments = id.key().segments();
    (1..segments.len())
        .rev()
        .map(|end| EntityId::from(DerivedEntityId::new(id.root(), DerivationKey::new(segments[..end].to_vec()))))
        .chain(iter::once(EntityId::Declared(id.root())))
        .find(|candidate| entities.contains_key(candidate))
}

/// `world` written out as WPS writes a site in `scope`, held whole at changeset 1 with all three flags set: each entity
/// once, with its components as they stand, except a relay's `access_cell` that no declaration or override stated,
/// which only this crate's binding writes. For tests that stand in for WPS, so one rule says what such a file holds.
#[cfg(feature = "test-support")]
pub fn written_out(world: &World, scope: &str) -> Value {
    let stated = |id: &EntityId| {
        let declared = world.declared.get(id).is_some_and(|entity| entity.has("access_cell"));
        declared || world.overrides.get(id).is_some_and(|components| components.contains_key("access_cell"))
    };
    let entities: Vec<Value> = world
        .entities
        .values()
        .map(|entity| {
            let mut components = entity.components.clone();
            if entity.has("relay") && !stated(&entity.id) {
                components.remove("access_cell");
            }
            let mut row = serde_json::json!({ "components": components, "id": entity.id, "scope": scope });
            if let Some(name) = &entity.name {
                row["name"] = Value::String(name.clone());
            }
            row
        })
        .collect();
    serde_json::json!({
        "derived": true,
        "entities": entities,
        "nested": true,
        "referenced": true,
        "scope": scope,
        "scopes": [{ "changeset": 1, "scope": scope, "whole": true }],
    })
}
