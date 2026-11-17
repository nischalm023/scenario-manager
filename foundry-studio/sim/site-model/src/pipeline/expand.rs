//! Step 3 of SPEC §8: running every generator, so the entities nobody wrote by
//! hand come into existence.
//!
//! Expansion is the format's answer to drawing a graph by hand. A region with a
//! pitch becomes cells carrying whatever its tiling says its cells carry; a rack
//! becomes one relay per position of every relay level and one slot per storage
//! position; a stamp becomes instances and, inside each, the template's
//! internals — which are themselves generators, so the whole thing recurses.
//! Every one of those entities is named by the address it was produced at rather
//! than by a position in an array, which is what lets a record written against
//! `instance:0/local:rack` still mean the same rack after the row grows by two —
//! and what lets a second implementation name the same rack without agreeing on
//! anything but the chain of segments. A tiling's own exclusions are subtracted
//! here as well, because a cell a tiling carves out of itself must never have
//! existed for the graph steps that follow.
//!
//! Nothing here binds a relay to the floor. That waits for step 5, where the
//! cells that survived suppression are known (SPEC §8).

use crate::components::fleet::{CarrierPool, Cohort};
use crate::components::product::{Stamp, Template};
use crate::components::spatial::{Exclusion, Pose, Region, Tiling};
use crate::components::structure::Rack;
use crate::derivation::KeySegment;
use crate::entity::Entity;
use crate::error::LoadError;
use crate::frames::FrameGraph;
use crate::ids::{DeclaredEntityId, DerivedEntityId, EntityId};
use crate::pipeline::merge::Merged;
use crate::placeholders;
use crate::units::{Coord, Steps};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Everything expansion produced, and everything it took back out.
#[derive(Debug, Clone)]
pub struct Expansion {
    /// Declared entities and surviving derived ones, together.
    pub entities: BTreeMap<EntityId, Entity>,
    /// Every address a generator produced, including the ones exclusions
    /// removed. A record addressing one of these is not dangling — the
    /// derivation did produce it — it simply has nothing left to act on.
    pub produced: BTreeSet<EntityId>,
    /// Cells exclusions removed, each paired with the tiling entity whose own
    /// `exclusions` took it out, and kept so the bake can record them as the
    /// suppressions SPEC §5 says they are equivalent to.
    pub excluded: Vec<(Entity, EntityId)>,
    /// Surviving derived entities per declared generator, for the check report.
    pub counts: BTreeMap<DeclaredEntityId, usize>,
    pub frames: FrameGraph,
}

/// Expand every generator in the merged set, depth first, then subtract the
/// exclusions.
pub fn expand(merged: &Merged) -> Result<Expansion, LoadError> {
    for entity in merged.entities.values() {
        placeholders::refuse(&entity.id, &entity.components)?;
    }
    let frames = FrameGraph::build(&merged.entities, merged.primary_frame.clone())?;

    let mut derived: Vec<Entity> = Vec::new();
    for id in merged.entities.keys().cloned().collect::<Vec<_>>() {
        let entity = &merged.entities[&id];
        expand_entity(entity, &merged.entities, &mut derived)?;
    }

    let mut entities = merged.entities.clone();
    let mut produced = BTreeSet::new();
    for entity in &derived {
        // Two generators cannot collide — their addresses start at different
        // roots — and a declared identity cannot collide with a derived one at
        // all, since a UUID is not an address. What is left to catch is one
        // generator emitting one segment twice, which would be a bug in a
        // generator rather than in a document.
        if entities.contains_key(&entity.id) {
            return Err(LoadError::DerivedIdCollision {
                address: entity.id.to_string(),
            });
        }
        produced.insert(entity.id.clone());
        entities.insert(entity.id.clone(), entity.clone());
    }

    frames.refuse_geometry_in_lattice_frames(&entities)?;
    let excluded = subtract_exclusions(&mut entities)?;

    let mut counts: BTreeMap<DeclaredEntityId, usize> = BTreeMap::new();
    for entity in entities.values() {
        if entity.is_derived() {
            *counts.entry(entity.id.root()).or_default() += 1;
        }
    }

    Ok(Expansion {
        entities,
        produced,
        excluded,
        counts,
        frames,
    })
}

/// Dispatch one entity to whichever generators its components make it.
fn expand_entity(
    entity: &Entity,
    declared: &BTreeMap<EntityId, Entity>,
    out: &mut Vec<Entity>,
) -> Result<(), LoadError> {
    if entity.has("region") && entity.has("tiling") {
        expand_tiling(entity, out)?;
    }
    if entity.has("rack") {
        expand_rack(entity, out)?;
    }
    if entity.has("stamp") {
        expand_stamp(entity, declared, out)?;
    }
    if entity.has("cohort") {
        expand_cohort(entity, out)?;
    }
    if entity.has("carrier_pool") {
        expand_carrier_pool(entity, out)?;
    }
    Ok(())
}

/// One entity a generator produced: its identity is its parent's, one segment
/// longer.
fn child(
    parent: &Entity,
    kind: &str,
    payload: impl Into<String>,
    components: BTreeMap<String, Value>,
) -> Result<Entity, LoadError> {
    let segment = KeySegment::parse(&format!("{kind}:{}", payload.into()))?;
    let mut entity = Entity::derived(parent.id.derive(segment), parent.id.clone());
    entity.components = components;
    Ok(entity)
}

// ---------------------------------------------------------------------------
// Cell grids
// ---------------------------------------------------------------------------

/// A region with a pitch derives `cols × rows` cells, each anchored at its own
/// centre and carrying, verbatim, every component the tiling's `derives` names.
/// SPEC §5 requires each extent to be an exact whole multiple of its pitch — no
/// silent remainder strip along the far edge.
fn expand_tiling(entity: &Entity, out: &mut Vec<Entity>) -> Result<(), LoadError> {
    let region: Region = entity.require("region")?;
    let tiling: Tiling = entity.require("tiling")?;
    let frame = region.frame.ok_or_else(|| LoadError::Unsupported {
        entity: entity.id.clone(),
        message: "a tiled region must state its frame; inside a template, a stamp supplies it"
            .into(),
    })?;

    let cols =
        region.size[0]
            .whole_tiles(tiling.pitch[0])
            .ok_or(LoadError::TilingNotDivisible {
                entity: entity.id.clone(),
                axis: "x",
                size: region.size[0],
                pitch: tiling.pitch[0],
            })?;
    let rows =
        region.size[1]
            .whole_tiles(tiling.pitch[1])
            .ok_or(LoadError::TilingNotDivisible {
                entity: entity.id.clone(),
                axis: "y",
                size: region.size[1],
                pitch: tiling.pitch[1],
            })?;

    let derives = tiling.derives.clone().unwrap_or_default();
    // A derived cell's pose is this tiling's own output — the centre of the
    // tile it was produced at — so a `derives` entry of that name would replace
    // it on every cell at once, and nothing downstream could tell the
    // substitute from the real one. The subtraction below reads that pose as a
    // position in the region's own frame, which a hand-written one need not be,
    // so a silent replacement moves which cells a carve-out removes too.
    if derives.contains_key("pose") {
        return Err(LoadError::Unsupported {
            entity: entity.id.clone(),
            message: "a tiling's `derives` may not name `pose`: each derived cell is posed at \
                      the centre of its own tile, and a derived pose would silently stand in \
                      for that"
                .into(),
        });
    }
    for col in 0..cols {
        for row in 0..rows {
            let centre = [
                centre_of(region.origin[0], col, tiling.pitch[0]),
                centre_of(region.origin[1], row, tiling.pitch[1]),
            ];
            let mut components = BTreeMap::new();
            components.insert(
                "pose".to_string(),
                json!({ "frame": frame, "position": centre, "heading_deg": 0.0 }),
            );
            for (name, value) in &derives {
                // A `$comment` among the derived components is an annotation on
                // the rule, not a component the cells carry (SPEC §2).
                if name.starts_with('$') {
                    continue;
                }
                components.insert(name.clone(), strip_annotations(value));
            }
            // Every cell answers to being a cell, whether or not `derives` says
            // anything about one (SPEC §5).
            components
                .entry("cell".to_string())
                .or_insert_with(|| json!({}));
            out.push(child(
                entity,
                "cell",
                format!("{col},{row}"),
                components,
            )?);
        }
    }
    Ok(())
}

/// `origin + (index + 0.5) · pitch`, on the step lattice: half a pitch is where
/// a tile's centre sits, and an odd pitch lands between steps, which is SPEC
/// §5's one rounding rule arriving in the commonest place there is.
fn centre_of(origin: Steps, index: usize, pitch: Steps) -> Steps {
    Steps::round_half_up(origin.0 as f64 + (index as f64 + 0.5) * pitch.0 as f64)
}

// ---------------------------------------------------------------------------
// Racks
// ---------------------------------------------------------------------------

/// A rack derives its shelves: one `relay` per position of every relay level,
/// keyed `relay:{level},{i}`, and one `slot` per storage position, keyed
/// `slot:{level},{col},{depth}`. Both index the rack's full `levels` list, so a
/// level number names the same physical shelf whichever kind of entry sits below
/// it (SPEC §5).
fn expand_rack(entity: &Entity, out: &mut Vec<Entity>) -> Result<(), LoadError> {
    let rack: Rack = entity.require("rack")?;
    check_levels(&entity.id, &rack)?;

    for (level_index, level) in rack.relay_levels() {
        let relay = level
            .relay()
            .expect("check_levels refuses an incomplete relay level");
        for (index, offset) in relay.positions.iter().enumerate() {
            let mut components = BTreeMap::new();
            components.insert(
                "relay".to_string(),
                json!({
                    "rack": entity.id,
                    "level": level_index,
                    "index": index,
                    "height": level.height,
                    "offset": offset,
                }),
            );
            out.push(child(
                entity,
                "relay",
                format!("{level_index},{index}"),
                components,
            )?);
        }
    }

    for (level_index, _) in rack.storage_levels() {
        for col in 0..rack.slots_wide {
            for depth in 0..rack.slots_deep {
                let mut components = BTreeMap::new();
                components.insert(
                    "slot".to_string(),
                    json!({ "rack": entity.id, "level": level_index, "col": col, "depth": depth }),
                );
                out.push(child(
                    entity,
                    "slot",
                    format!("{level_index},{col},{depth}"),
                    components,
                )?);
            }
        }
    }
    Ok(())
}

/// The two things SPEC §5 demands of a `levels` list, checked before anything
/// hangs an identity off it: heights increase down the list, so that "the lowest
/// level" and "level 3" each mean one thing; and a relay level states how far
/// its relays reach, without which they could not be bound to a cell. Where the
/// shelves stand needs no separate check — a position is a position.
fn check_levels(entity: &EntityId, rack: &Rack) -> Result<(), LoadError> {
    for (index, level) in rack.levels.iter().enumerate() {
        if index > 0 {
            let previous = &rack.levels[index - 1];
            if level.height.0 <= previous.height.0 {
                return Err(LoadError::RackLevelsNotIncreasing {
                    entity: entity.clone(),
                    index,
                    height: level.height,
                    previous: index - 1,
                    previous_height: previous.height,
                });
            }
        }
        if !level.is_relay() {
            continue;
        }
        if level.reach.is_none() {
            return Err(LoadError::RelayLevelHasNoReach {
                entity: entity.clone(),
                index,
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Stamps and templates
// ---------------------------------------------------------------------------

/// A stamp places a template. The `instance:{i}` level always exists — a plain
/// stamp is instance 0 — so adding a `repeat` later cannot silently rename every
/// id beneath it.
fn expand_stamp(
    entity: &Entity,
    declared: &BTreeMap<EntityId, Entity>,
    out: &mut Vec<Entity>,
) -> Result<(), LoadError> {
    let reading = StampReading::read(entity, declared)?;

    let (count, step) = match &reading.stamp.repeat {
        None => (1usize, [Coord(0), Coord(0)]),
        Some(repeat) => {
            if repeat.count == 0 {
                return Err(LoadError::Unsupported {
                    entity: entity.id.clone(),
                    message: "`repeat.count` must be a positive integer".into(),
                });
            }
            (repeat.count, repeat.step)
        }
    };
    reading.check_handles()?;

    for index in 0..count {
        // The step is expressed in the stamp pose's frame and is deliberately
        // not rotated by the heading, so a row rotated 180° still marches along
        // the same world axis and spans the same extent (SPEC §6).
        let position = [
            Coord(reading.anchor[0].0 + index as i64 * step[0].0),
            Coord(reading.anchor[1].0 + index as i64 * step[1].0),
        ];
        let Instantiation {
            instance,
            internals,
        } = reading.instantiate(&index.to_string(), position)?;

        out.push(instance);
        for internal in internals {
            // Depth first: a stamped internal that is itself a generator — the
            // EasyLoop's tiled queue, a row's rack — expands under its own id.
            expand_entity(&internal, declared, out)?;
            out.push(internal);
        }
    }
    Ok(())
}

/// One instance of a stamp as the instantiation step leaves it, before
/// anything inside it expands: the instance and the template's internals.
#[derive(Debug, Clone)]
pub struct Instantiation {
    /// `{stamp}/instance:{payload}`, carrying the instance's pose and nothing
    /// else.
    pub instance: Entity,
    /// `{stamp}/instance:{payload}/local:{handle}` for each internal, in the
    /// template's order: parameters bound, local references rewritten, and
    /// geometry composed into the instance's pose. An internal that is itself a
    /// generator is returned unexpanded.
    pub internals: Vec<Entity>,
}

/// Instantiate the single instance a stamp without `repeat` makes, under the
/// instance payload `instance` (`"0"` is the one [`expand`] gives it), at the
/// stamp's own pose: parameters bound, local references rewritten and geometry
/// composed exactly as [`expand`] does for each of its instances, and nothing
/// expanded inside. The template evaluation engine of Agent Studio calls this
/// once per placement instance with a payload such as `"3,0"`, so a placement
/// and a stamp share one implementation of stamping. A stamp with `repeat` is
/// refused: its instances are [`expand`]'s to number.
pub fn instantiate_stamp(
    stamp: &Entity,
    declared: &BTreeMap<EntityId, Entity>,
    instance: &str,
) -> Result<Instantiation, LoadError> {
    let reading = StampReading::read(stamp, declared)?;
    if reading.stamp.repeat.is_some() {
        return Err(LoadError::Unsupported {
            entity: stamp.id.clone(),
            message: "a stamp with `repeat` makes several instances; instantiate one stamp \
                      per instance, each without `repeat`"
                .into(),
        });
    }
    if instance.is_empty() || instance.contains('/') {
        return Err(LoadError::Unsupported {
            entity: stamp.id.clone(),
            message: format!(
                "`{instance}` cannot be an `instance:` payload: a key segment's payload is \
                 never empty and holds no `/`"
            ),
        });
    }
    reading.check_handles()?;
    reading.instantiate(instance, reading.anchor)
}

/// What every instance of one stamp shares, read once: the stamp, where it is
/// posed, its template and the parameters the stamp binds.
struct StampReading<'a> {
    entity: &'a Entity,
    stamp: Stamp,
    pose: Pose,
    frame: EntityId,
    anchor: [Coord; 2],
    template_entity: &'a Entity,
    template: Template,
    bindings: BTreeMap<String, Value>,
}

impl<'a> StampReading<'a> {
    fn read(
        entity: &'a Entity,
        declared: &'a BTreeMap<EntityId, Entity>,
    ) -> Result<StampReading<'a>, LoadError> {
        let stamp: Stamp = entity.require("stamp")?;
        let pose: Pose = entity.require("pose")?;
        let frame = pose.frame.clone().ok_or_else(|| LoadError::Unsupported {
            entity: entity.id.clone(),
            message: "a stamp must be posed in a frame".into(),
        })?;
        let anchor = pose.xy().ok_or_else(|| LoadError::Unsupported {
            entity: entity.id.clone(),
            message: "a stamp's pose needs at least a planar position".into(),
        })?;

        let template_entity =
            declared
                .get(&stamp.template)
                .ok_or_else(|| LoadError::DanglingReference {
                    entity: entity.id.clone(),
                    component: "stamp".into(),
                    field: "template".into(),
                    target: stamp.template.clone(),
                })?;
        let template: Template = template_entity.require("template")?;
        let bindings = bind_params(&template, &stamp, &entity.id, &template_entity.id)?;
        Ok(StampReading {
            entity,
            stamp,
            pose,
            frame,
            anchor,
            template_entity,
            template,
            bindings,
        })
    }

    fn check_handles(&self) -> Result<(), LoadError> {
        let entities = &self.template.entities;
        for handle in entities.iter().map(|e| &e.local) {
            if handle.contains('/') {
                return Err(LoadError::SlashInLocalHandle {
                    template: self.template_entity.id.clone(),
                    handle: handle.clone(),
                });
            }
            if entities.iter().filter(|e| &e.local == handle).count() > 1 {
                return Err(LoadError::DuplicateLocalHandle {
                    template: self.template_entity.id.clone(),
                    handle: handle.clone(),
                });
            }
        }
        Ok(())
    }

    /// The instance under `payload` standing at `position`, and its internals.
    fn instantiate(&self, payload: &str, position: [Coord; 2]) -> Result<Instantiation, LoadError> {
        let (frame, heading) = (&self.frame, self.pose.heading_deg);
        let instance_pose = Pose {
            frame: Some(frame.clone()),
            position: vec![position[0], position[1]],
            heading_deg: heading,
        };
        let mut instance_components = BTreeMap::new();
        instance_components.insert(
            "pose".to_string(),
            json!({ "frame": frame, "position": position, "heading_deg": heading }),
        );
        let instance = child(
            self.entity,
            "instance",
            payload,
            instance_components,
        )?;

        let handles: BTreeMap<String, EntityId> = self
            .template
            .entities
            .iter()
            .map(|te| {
                Ok((
                    te.local.clone(),
                    EntityId::from(instance.id.derive(KeySegment::parse(&format!("local:{}", te.local))?)),
                ))
            })
            .collect::<Result<_, LoadError>>()?;

        let template_id = &self.template_entity.id;
        let mut internals = Vec::new();
        for te in &self.template.entities {
            let mut rewritten = BTreeMap::new();
            for (name, value) in &te.components {
                let substituted = substitute_params(value, &self.bindings, template_id)?;
                rewritten.insert(name.clone(), rewrite_local_refs(&substituted, &handles, template_id)?);
            }
            placeholders::refuse(&handles[&te.local], &rewritten)?;
            let mut components = BTreeMap::new();
            for (name, value) in rewritten {
                let composed = compose_into_instance(&name, value, &instance_pose, frame, &handles[&te.local])?;
                components.insert(name, composed);
            }
            let mut internal = child(&instance, "local", te.local.clone(), components)?;
            internal.name = te.name.clone();
            internals.push(internal);
        }
        Ok(Instantiation {
            instance,
            internals,
        })
    }
}

/// Bind every parameter a template declares, and refuse a stamp that binds one
/// it does not: a mistyped parameter name that silently does nothing is exactly
/// the failure the patch rule exists to prevent one layer up.
fn bind_params(
    template: &Template,
    stamp: &Stamp,
    stamp_id: &EntityId,
    template_id: &EntityId,
) -> Result<BTreeMap<String, Value>, LoadError> {
    let mut bindings = BTreeMap::new();
    for (name, decl) in &template.params {
        let bound = stamp
            .params
            .get(name)
            .cloned()
            .or_else(|| decl.default.clone())
            .ok_or_else(|| LoadError::UnboundParam {
                stamp: stamp_id.clone(),
                template: template_id.clone(),
                param: name.clone(),
            })?;
        bindings.insert(name.clone(), bound);
    }
    for name in stamp.params.keys() {
        if !template.params.contains_key(name) {
            return Err(LoadError::UnexpectedParam {
                stamp: stamp_id.clone(),
                template: template_id.clone(),
                param: name.clone(),
            });
        }
    }
    Ok(bindings)
}

/// The one-key object form `{"$param": "name"}` standing as a whole value.
/// Partial-string interpolation does not exist in this format, so nothing here
/// looks inside strings.
fn param_reference(map: &Map<String, Value>) -> Option<&str> {
    single_marker(map, "$param")
}

fn local_reference(map: &Map<String, Value>) -> Option<&str> {
    single_marker(map, "local_ref")
}

/// The address form `{"local_ref": handle, "key": "cell:0,0"}`: a derivation
/// address whose generator is a template internal. Returns the handle and the
/// key it addresses beneath it.
fn local_address(map: &Map<String, Value>) -> Option<(&str, &str)> {
    let mut data_keys: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "$comment")
        .collect();
    data_keys.sort_unstable();
    if data_keys != ["key", "local_ref"] {
        return None;
    }
    let handle = map.get("local_ref")?.as_str()?;
    Some((handle, map.get("key")?.as_str()?))
}

fn single_marker<'a>(map: &'a Map<String, Value>, marker: &str) -> Option<&'a str> {
    let mut data_keys = map.keys().filter(|k| k.as_str() != "$comment");
    let only = data_keys.next()?;
    if data_keys.next().is_some() || only != marker {
        return None;
    }
    map.get(marker).and_then(Value::as_str)
}

fn substitute_params(
    value: &Value,
    bindings: &BTreeMap<String, Value>,
    template: &EntityId,
) -> Result<Value, LoadError> {
    match value {
        Value::Object(map) => {
            if let Some(name) = param_reference(map) {
                return bindings
                    .get(name)
                    .cloned()
                    .ok_or_else(|| LoadError::UnknownParam {
                        template: template.clone(),
                        param: name.to_string(),
                    });
            }
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k.clone(), substitute_params(v, bindings, template)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute_params(item, bindings, template)?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other.clone()),
    }
}

/// Turn `{"local_ref": "waiting_block"}` into the UUID that handle got in this
/// instance. This is what lets a template ship its own internal connectivity —
/// a station wires its own organs together instead of hoping the site author
/// does — and it reaches stitch endpoints and explicit pair addresses alike,
/// because it rewrites any value of that shape wherever it appears.
fn rewrite_local_refs(
    value: &Value,
    handles: &BTreeMap<String, EntityId>,
    template: &EntityId,
) -> Result<Value, LoadError> {
    match value {
        Value::Object(map) => {
            let resolve = |handle: &str| -> Result<EntityId, LoadError> {
                handles
                    .get(handle)
                    .cloned()
                    .ok_or_else(|| LoadError::UnknownLocalRef {
                        template: template.clone(),
                        handle: handle.to_string(),
                    })
            };
            if let Some(handle) = local_reference(map) {
                return Ok(Value::String(resolve(handle)?.to_string()));
            }
            if let Some((handle, key)) = local_address(map) {
                // Wire addresses start at a declared UUID. The local generator
                // is itself derived, so retain its chain in the key.
                let address: DerivedEntityId = format!("{}/{key}", resolve(handle)?).parse()?;
                return Ok(json!({ "generator": address.root(), "key": address.key().to_string() }));
            }
            // A `local_ref` in any other company is a shape nothing will rewrite,
            // and a handle that survives expansion resolves to nothing. Refusing
            // here names the template; letting it through would strand it.
            if map.contains_key("local_ref") {
                return Err(LoadError::MalformedLocalRef {
                    template: template.clone(),
                    found: map
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k.clone(), rewrite_local_refs(v, handles, template)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(rewrite_local_refs(item, handles, template)?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other.clone()),
    }
}

/// Carry a template internal's local geometry into the world the instance sits
/// in. Only the components that *are* geometry move: a pose rotates and
/// translates, a region's rectangle is re-cornered, and a tiling's pitch swaps
/// axes under a quarter turn. Every other component, known or not, passes
/// through untouched.
fn compose_into_instance(
    component: &str,
    value: Value,
    instance: &Pose,
    frame: &EntityId,
    entity: &EntityId,
) -> Result<Value, LoadError> {
    match component {
        "pose" => compose_pose(value, instance, frame, entity),
        "region" => compose_region(value, instance, frame, entity),
        "tiling" => compose_tiling(value, instance, entity),
        _ => Ok(value),
    }
}

fn compose_pose(
    mut value: Value,
    instance: &Pose,
    frame: &EntityId,
    entity: &EntityId,
) -> Result<Value, LoadError> {
    let local: Pose =
        serde_json::from_value(value.clone()).map_err(|source| LoadError::ComponentShape {
            entity: entity.clone(),
            component: "pose".into(),
            source,
        })?;
    // A template internal that names its own frame is already absolute; only a
    // frameless pose is local to the template.
    if local.frame.is_some() {
        return Ok(value);
    }
    let xy = local.xy().ok_or_else(|| LoadError::Unsupported {
        entity: entity.clone(),
        message: "a template internal's pose needs at least a planar position".into(),
    })?;
    let rotated = instance
        .heading_deg
        .rotate([xy[0].0 as f64, xy[1].0 as f64]);
    let anchor = instance.xy().expect("an instance pose is always planar");
    let object = value
        .as_object_mut()
        .ok_or_else(|| LoadError::Unsupported {
            entity: entity.clone(),
            message: "`pose` must be an object".into(),
        })?;
    object.insert("frame".into(), json!(frame));
    object.insert(
        "position".into(),
        json!([
            Steps::round_half_up(anchor[0].0 as f64 + rotated[0]),
            Steps::round_half_up(anchor[1].0 as f64 + rotated[1]),
        ]),
    );
    object.insert(
        "heading_deg".into(),
        json!(instance.heading_deg.compose(local.heading_deg)),
    );
    Ok(value)
}

fn compose_region(
    mut value: Value,
    instance: &Pose,
    frame: &EntityId,
    entity: &EntityId,
) -> Result<Value, LoadError> {
    let local: Region =
        serde_json::from_value(value.clone()).map_err(|source| LoadError::ComponentShape {
            entity: entity.clone(),
            component: "region".into(),
            source,
        })?;
    if local.frame.is_some() {
        return Ok(value);
    }
    // A region is an axis-aligned rectangle, so it survives a quarter turn and
    // nothing else. Refusing the general case is deliberate: rotating a rectangle
    // by 30° produces a shape v0 has no way to write down, and silently keeping
    // the unrotated extents would put cells where no floor is.
    if instance.heading_deg.quarter_turns().is_none() {
        return Err(LoadError::Unsupported {
            entity: entity.clone(),
            message: format!(
                "a template region cannot be stamped at {}: v0 regions are axis-aligned \
                 rectangles, so only quarter-turn headings are defined",
                instance.heading_deg
            ),
        });
    }
    let anchor = instance.xy().expect("an instance pose is always planar");
    let corners = [
        [local.origin[0].0 as f64, local.origin[1].0 as f64],
        [
            (local.origin[0].0 + local.size[0].0) as f64,
            (local.origin[1].0 + local.size[1].0) as f64,
        ],
    ]
    .map(|c| instance.heading_deg.rotate(c));
    let min = [
        Steps::round_half_up(corners[0][0].min(corners[1][0]) + anchor[0].0 as f64),
        Steps::round_half_up(corners[0][1].min(corners[1][1]) + anchor[1].0 as f64),
    ];
    let size = [
        Steps::round_half_up((corners[1][0] - corners[0][0]).abs()),
        Steps::round_half_up((corners[1][1] - corners[0][1]).abs()),
    ];
    let object = value
        .as_object_mut()
        .ok_or_else(|| LoadError::Unsupported {
            entity: entity.clone(),
            message: "`region` must be an object".into(),
        })?;
    object.insert("frame".into(), json!(frame));
    object.insert("origin".into(), json!(min));
    object.insert("size".into(), json!(size));
    Ok(value)
}

fn compose_tiling(
    mut value: Value,
    instance: &Pose,
    entity: &EntityId,
) -> Result<Value, LoadError> {
    let quarter = instance
        .heading_deg
        .quarter_turns()
        .ok_or_else(|| LoadError::Unsupported {
            entity: entity.clone(),
            message: format!(
                "a template tiling cannot be stamped at {}: only quarter-turn headings are \
                 defined for axis-aligned pitches",
                instance.heading_deg
            ),
        })?;
    // At heading 0 the region only translates, and a carve-out stated relative
    // to that region's origin translates with it: nothing to do, for the pitch
    // or for the exclusions.
    if quarter == 0 {
        return Ok(value);
    }
    let local: Tiling =
        serde_json::from_value(value.clone()).map_err(|source| LoadError::ComponentShape {
            entity: entity.clone(),
            component: "tiling".into(),
            source,
        })?;
    // Every other quarter turn — 180° included, which flips an origin-relative
    // rectangle onto the far corner — would need the region's extent to say
    // where the carve-out lands, and composing one component at a time cannot
    // see it. Refusing beats stamping a hole in the wrong place.
    if !local.exclusions.is_empty() {
        return Err(LoadError::Unsupported {
            entity: entity.clone(),
            message: format!(
                "a template tiling carrying `exclusions` cannot be stamped at {}: its \
                 carve-outs are stated from the region's origin and re-cornering them needs \
                 the region's extent, so v0 stamps such a tiling at 0° only",
                instance.heading_deg
            ),
        });
    }
    if quarter % 2 == 0 {
        return Ok(value);
    }
    let object = value
        .as_object_mut()
        .ok_or_else(|| LoadError::Unsupported {
            entity: entity.clone(),
            message: "`tiling` must be an object".into(),
        })?;
    object.insert("pitch".into(), json!([local.pitch[1], local.pitch[0]]));
    Ok(value)
}

// ---------------------------------------------------------------------------
// Fleet
// ---------------------------------------------------------------------------

fn expand_cohort(entity: &Entity, out: &mut Vec<Entity>) -> Result<(), LoadError> {
    let cohort: Cohort = entity.require("cohort")?;
    for index in 0..cohort.count {
        let mut components = BTreeMap::new();
        let mut body = json!({
            "robot_class": cohort.robot_class,
            "cohort": entity.id,
            "index": index,
        });
        if let Some(assignment) = &cohort.assignment {
            body["assignment"] = serde_json::to_value(assignment).expect("assignment serializes");
        }
        components.insert("robot".to_string(), body);
        out.push(child(
            entity,
            "robot",
            index.to_string(),
            components,
        )?);
    }
    Ok(())
}

fn expand_carrier_pool(entity: &Entity, out: &mut Vec<Entity>) -> Result<(), LoadError> {
    let pool: CarrierPool = entity.require("carrier_pool")?;
    for index in 0..pool.count {
        let mut components = BTreeMap::new();
        let mut body = json!({
            "carrier_type": pool.carrier_type,
            "pool": entity.id,
            "index": index,
            "designation": pool.designation,
        });
        if let Some(placement) = &pool.placement {
            body["placement"] = serde_json::to_value(placement).expect("placement serializes");
        }
        components.insert("carrier".to_string(), body);
        out.push(child(
            entity,
            "carrier",
            index.to_string(),
            components,
        )?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// A tiling's own carve-outs
// ---------------------------------------------------------------------------

/// Remove every derived cell whose centre falls strictly inside one of its own
/// tiling's exclusions, and refuse any entity still carrying the `exclusion`
/// component the format used to have.
///
/// A candidate cell is one whose `parent` is the tiling entity — a tiling's
/// cells point straight at it — so a carve-out reaches its own tiling's cells
/// and nothing else. Declared cells are therefore untouched without a rule
/// saying so: a hand-placed cell has no tiling above it.
///
/// The comparison is between the cell's stored pose integers and the region's
/// own origin, both in the region's frame, so no frame conversion happens here
/// at all. The stored integers are the point rather than a shortcut: an odd
/// pitch puts the exact centre at a half step and [`centre_of`] has already
/// rounded it half-up, so comparing anything else would move edge cases where a
/// carve-out's edge falls between the two.
///
/// One scope line on that same claim: the retired exclusion entity was tested
/// after projecting both rect and centre to the primary frame, so a document
/// migrated off it keeps its exact verdicts wherever the region's frame
/// reaches the primary without shrinking — identity or integer translation,
/// which is every shipped fixture. A shrinking scale could round two projected
/// values together that this frame-local test still tells apart.
fn subtract_exclusions(
    entities: &mut BTreeMap<EntityId, Entity>,
) -> Result<Vec<(Entity, EntityId)>, LoadError> {
    let mut carved: BTreeMap<EntityId, ([Steps; 2], Vec<Exclusion>)> = BTreeMap::new();
    for entity in entities.values() {
        // A declared one, or one a tiling's `derives` minted: either way its
        // carve-outs would go unread, so the load stops rather than quietly
        // leaving every cell it meant to remove standing.
        if entity.has("exclusion") {
            return Err(LoadError::ExclusionComponentRemoved {
                entity: entity.id.clone(),
            });
        }
        // A template's internals become entities only where a stamp mints them,
        // so the walk above reaches a carrier inside an unstamped template not
        // at all. That template still says it means to remove cells, and the
        // day something stamps it the refusal would arrive instead of the
        // removals — so it refuses now, named by the template that holds it.
        if template_internal_carries_exclusion(entity) {
            return Err(LoadError::ExclusionComponentRemoved {
                entity: entity.id.clone(),
            });
        }
        if !entity.has("region") || !entity.has("tiling") {
            continue;
        }
        let tiling: Tiling = entity.require("tiling")?;
        if tiling.exclusions.is_empty() {
            continue;
        }
        let region: Region = entity.require("region")?;
        carved.insert(entity.id.clone(), (region.origin, tiling.exclusions));
    }
    if carved.is_empty() {
        return Ok(Vec::new());
    }

    let mut removed = Vec::new();
    for entity in entities.values() {
        if !entity.is_derived() || !entity.has("cell") {
            continue;
        }
        let Some(tiling) = entity.parent.as_ref() else {
            continue;
        };
        let Some((origin, exclusions)) = carved.get(tiling) else {
            continue;
        };
        let Some(pose) = entity.component::<Pose>("pose")? else {
            continue;
        };
        let Some(centre) = pose.xy() else {
            continue;
        };
        // Every carve-out is read, with no early exit on the first hit: one
        // that cannot be computed is a refusal whichever of its siblings
        // already covered this centre.
        let mut inside = false;
        for exclusion in exclusions {
            let Some(covers) = exclusion.contains_strictly(*origin, centre) else {
                return Err(LoadError::ExclusionOutOfRange {
                    entity: tiling.clone(),
                });
            };
            inside |= covers;
        }
        if inside {
            removed.push((entity.clone(), tiling.clone()));
        }
    }
    for (entity, _) in &removed {
        entities.remove(&entity.id);
    }
    Ok(removed)
}

/// Whether any of a template's internals carries the retired `exclusion`
/// component.
///
/// Read off the raw value rather than through the typed [`Template`] view on
/// purpose: a template whose params or entities are shaped wrong is a different
/// refusal with a different message, and the retired name must not be traded
/// away for it.
fn template_internal_carries_exclusion(entity: &Entity) -> bool {
    let Some(template) = entity.raw("template") else {
        return false;
    };
    template
        .get("entities")
        .and_then(Value::as_array)
        .is_some_and(|internals| {
            internals.iter().any(|internal| {
                internal
                    .get("components")
                    .and_then(Value::as_object)
                    .is_some_and(|components| components.contains_key("exclusion"))
            })
        })
}

/// Component values are copied into every derived cell, and a `$comment`
/// in them is an annotation on the *rule*, not on each of the sixty-six cells it
/// produced.
fn strip_annotations(value: &Value) -> Value {
    crate::canonical::strip_comments(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::product::Queue;
    use crate::components::spatial::Frame;
    use serde_json::json;
    use std::str::FromStr;
    use uuid::Uuid;

    fn id(s: &str) -> EntityId {
        EntityId::from_str(s).expect("a literal identity")
    }

    /// The identity a generator derives at one key — written the way a document
    /// writes it, since that spelling is the identity.
    fn at(generator: &str, key: &str) -> EntityId {
        id(&format!("{generator}/{key}"))
    }

    fn root(s: &str) -> DeclaredEntityId {
        DeclaredEntityId(Uuid::from_str(s).expect("a literal UUID"))
    }

    fn entity(idstr: &str, components: Value) -> Entity {
        let mut e = Entity::new(id(idstr).as_declared().expect("a declared identity"));
        for (k, v) in components.as_object().unwrap() {
            e.components.insert(k.clone(), v.clone());
        }
        e
    }

    const FLOOR: &str = "f0000000-0000-4000-8000-000000000002";
    const HIGHWAY: &str = "a0000000-0000-4000-8000-000000000001";

    fn floor_frame() -> Entity {
        entity(
            FLOOR,
            json!({ "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } }),
        )
    }

    fn merged_of(entities: Vec<Entity>) -> Merged {
        let mut map = BTreeMap::new();
        for e in entities {
            map.insert(e.id.clone(), e);
        }
        Merged {
            entities: map,
            overrides: Vec::new(),
            suppressions: Vec::new(),
            primary_frame: Some(id(FLOOR)),
        }
    }

    #[test]
    fn tiled_centres_sit_half_a_pitch_in_from_the_origin() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 29700] },
                    "tiling": { "pitch": [900, 900],
                                "derives": { "cell": { "traversable_by": ["htm"] } } }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        assert_eq!(expansion.counts[&root(HIGHWAY)], 66);
        let cell = &expansion.entities[&at(HIGHWAY, "cell:1,7")];
        assert_eq!(
            cell.components["pose"]["position"],
            json!([1350, 6750]),
            "the pin world.json writes for this cell"
        );
        assert_eq!(
            cell.components["cell"],
            json!({ "traversable_by": ["htm"] })
        );
    }

    /// `derives` is the whole component map a cell_grid's cells carry, not one
    /// component's defaults — which is how "these cells have chargers" is said
    /// once for a strip instead of once per tile (SPEC §5).
    #[test]
    fn a_tiling_stamps_every_component_it_derives_onto_its_cells() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 900] },
                    "tiling": { "pitch": [900, 900], "derives": {
                        "cell": { "traversable_by": ["htm"] },
                        "charging": {},
                        "$comment": "an annotation on the rule, not a component on the cells"
                    } }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let cell = &expansion.entities[&at(HIGHWAY, "cell:0,0")];
        assert_eq!(cell.components["charging"], json!({}));
        assert_eq!(
            cell.components["cell"],
            json!({ "traversable_by": ["htm"] })
        );
        assert!(!cell.components.contains_key("$comment"));
    }

    /// A cell whose tiling names no `cell` component still answers to being a
    /// cell, because everything downstream asks that question by component name.
    #[test]
    fn a_cell_a_tiling_says_nothing_about_is_still_a_cell() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 900] },
                    "tiling": { "pitch": [900, 900], "derives": { "charging": {} } }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let cell = &expansion.entities[&at(HIGHWAY, "cell:0,0")];
        assert_eq!(cell.components["cell"], json!({}));
    }

    #[test]
    fn an_extent_the_pitch_does_not_divide_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 29650] },
                    "tiling": { "pitch": [900, 900] }
                }),
            ),
        ]);
        let err = expand(&merged).unwrap_err();
        assert!(
            matches!(err, LoadError::TilingNotDivisible { axis: "y", .. }),
            "{err}"
        );
    }

    #[test]
    fn an_exclusion_removes_the_cells_whose_centres_it_covers() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 29700] },
                    "tiling": { "pitch": [900, 900],
                                "exclusions": [{ "origin": [0, 26000], "size": [8000, 3000] }] }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        // Rows 29, 30 and 31 have centres at 26550, 27450 and 28350; row 28 sits
        // at 25650 and row 32 at 29250, both outside.
        assert_eq!(expansion.excluded.len(), 6);
        assert_eq!(expansion.counts[&root(HIGHWAY)], 60);
        assert!(!expansion.entities.contains_key(&at(HIGHWAY, "cell:0,29")));
        assert!(expansion.entities.contains_key(&at(HIGHWAY, "cell:0,32")));
        // Excluded coordinates were still produced, so a record addressing one
        // is not dangling.
        assert!(expansion.produced.contains(&at(HIGHWAY, "cell:0,29")));
        // And the removal is booked against the tiling that asked for it.
        assert!(expansion
            .excluded
            .iter()
            .all(|(_, tiling)| tiling == &id(HIGHWAY)));
    }

    /// The selectivity the old design could not express: two tilings over the
    /// same floor, one of which carves a hole, and the other keeps every cell.
    ///
    /// An `exclusion` entity subtracted from every generator at once, so this
    /// document could only be written by moving one of the two tilings out of
    /// the hole's way. A tiling that owns its carve-outs makes the two
    /// independent by construction.
    #[test]
    fn one_tilings_exclusion_leaves_an_overlapping_tilings_cells_alone() {
        const OVERLAY: &str = "a0000000-0000-4000-8000-000000000005";
        let square = |exclusions: Value| {
            json!({
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [2700, 2700] },
                "tiling": { "pitch": [900, 900], "exclusions": exclusions }
            })
        };
        let merged = merged_of(vec![
            floor_frame(),
            entity(HIGHWAY, square(json!([{ "origin": [900, 900], "size": [900, 900] }]))),
            entity(OVERLAY, square(json!([]))),
        ]);
        let expansion = expand(&merged).unwrap();
        // The middle tile of the carving tiling, and only it.
        assert_eq!(expansion.excluded.len(), 1);
        assert_eq!(expansion.counts[&root(HIGHWAY)], 8);
        assert!(!expansion.entities.contains_key(&at(HIGHWAY, "cell:1,1")));
        // The other tiling covers the very same square and keeps all nine.
        assert_eq!(expansion.counts[&root(OVERLAY)], 9);
        assert!(expansion.entities.contains_key(&at(OVERLAY, "cell:1,1")));
    }

    /// A template's own carve-out travels with the stamp: the exclusion is
    /// stated from the region's origin, so translating the region translates the
    /// hole and the same tile comes out in every instance.
    #[test]
    fn a_stamped_tilings_exclusion_travels_with_the_instance() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [
                    { "local": "floor", "components": {
                        "region": { "origin": [0, 0], "size": [2700, 2700] },
                        "tiling": { "pitch": [900, 900],
                                    "exclusions": [{ "origin": [900, 900],
                                                     "size": [900, 900] }] } } }
                ]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [10000, 20000] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let tiling = at(STAMP, "instance:0/local:floor");
        // The stamped region runs 10000..12700 by 20000..22700, so its middle
        // tile is centred at [11350, 21350] and the carve-out covers
        // 10900..11800 by 20900..21800 — that tile and no other.
        assert!(!expansion
            .entities
            .contains_key(&id(&format!("{tiling}/cell:1,1"))));
        let neighbour = &expansion.entities[&id(&format!("{tiling}/cell:1,2"))];
        assert_eq!(
            neighbour.components["pose"]["position"],
            json!([11350, 22250])
        );
        assert_eq!(expansion.excluded.len(), 1);
        assert_eq!(expansion.excluded[0].1, tiling);
    }

    /// The same template at a quarter turn refuses, because re-cornering an
    /// origin-relative rectangle needs the region's extent and per-component
    /// composition never sees it.
    #[test]
    fn a_template_tiling_with_exclusions_stamped_at_a_quarter_turn_fails_the_load() {
        let stamped_at = |heading: f64| {
            merged_of(vec![
                floor_frame(),
                param_template(json!({
                    "params": {},
                    "entities": [
                        { "local": "floor", "components": {
                            "region": { "origin": [0, 0], "size": [2700, 2700] },
                            "tiling": { "pitch": [900, 900],
                                        "exclusions": [{ "origin": [900, 900],
                                                         "size": [900, 900] }] } } }
                    ]
                })),
                entity(
                    STAMP,
                    json!({
                        "pose": { "frame": FLOOR, "position": [0, 0], "heading_deg": heading },
                        "stamp": { "template": TEMPLATE }
                    }),
                ),
            ])
        };
        for heading in [90.0, 180.0, 270.0] {
            let err = expand(&stamped_at(heading)).unwrap_err();
            assert!(
                matches!(&err, LoadError::Unsupported { message, .. }
                         if message.contains("`exclusions`")),
                "at {heading}°: {err}"
            );
        }
        // And 0° is the heading that does load, so the refusal is about the turn
        // rather than about the template.
        assert!(expand(&stamped_at(0.0)).is_ok());
    }

    /// The component that used to carve cells out of other entities' tilings is
    /// gone, and a document still carrying one is refused rather than loaded
    /// with every cell it meant to remove still standing.
    #[test]
    fn an_entity_carrying_the_retired_exclusion_component_fails_the_load() {
        const CHARGING: &str = "a0000000-0000-4000-8000-000000000004";
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 29700] },
                    "tiling": { "pitch": [900, 900] }
                }),
            ),
            entity(
                CHARGING,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 26000], "size": [8000, 3000] },
                    "exclusion": {}
                }),
            ),
        ]);
        let err = expand(&merged).unwrap_err();
        assert!(
            matches!(&err, LoadError::ExclusionComponentRemoved { entity } if entity == &id(CHARGING)),
            "{err}"
        );
        assert!(
            err.to_string().contains("`exclusions`"),
            "the refusal names the replacement: {err}"
        );

        // And a derived one is caught the same way: a tiling's `derives` can
        // mint the component onto every cell it produces, and those cells are
        // in hand by the time the subtraction walks them.
        let minted = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 900] },
                    "tiling": { "pitch": [900, 900], "derives": { "exclusion": {} } }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&minted).unwrap_err(),
            LoadError::ExclusionComponentRemoved { ref entity }
                if entity == &at(HIGHWAY, "cell:0,0")
        ));
    }

    /// A tiling computes each cell's pose, so `derives` may not also state one:
    /// the derived value would replace the computed one on every cell, and the
    /// subtraction reads that pose as a position in the region's own frame.
    #[test]
    fn a_tiling_that_derives_a_pose_onto_its_cells_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 1800] },
                    "tiling": { "pitch": [900, 900], "derives": {
                        "pose": { "frame": FLOOR, "position": [0, 0] } } }
                }),
            ),
        ]);
        let err = expand(&merged).unwrap_err();
        assert!(
            matches!(&err, LoadError::Unsupported { entity, message }
                     if entity == &id(HIGHWAY) && message.contains("`derives` may not name `pose`")),
            "{err}"
        );
    }

    /// A carve-out whose far corner does not fit in a step count is a refusal,
    /// not a rectangle that wrapped around — and not a panic in a debug build,
    /// which is what an unchecked `origin + size` was.
    #[test]
    fn an_exclusion_whose_far_corner_overflows_fails_the_load_instead_of_panicking() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 1800] },
                    "tiling": { "pitch": [900, 900], "exclusions": [
                        { "origin": [9_000_000_000_000_000_000i64, 0],
                          "size": [300_000_000_000_000_000i64, 900] }
                    ] }
                }),
            ),
        ]);
        let err = expand(&merged).unwrap_err();
        assert!(
            matches!(&err, LoadError::ExclusionOutOfRange { entity } if entity == &id(HIGHWAY)),
            "{err}"
        );
    }

    /// The retired component hiding where nothing has stamped it yet. A
    /// template's internals are not entities until a stamp mints them, so the
    /// scan over entities would find this one only once something stamped the
    /// template — and until then the document would load as if it said nothing.
    #[test]
    fn an_unstamped_template_internal_carrying_the_retired_component_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [
                    { "local": "hole", "components": {
                        "region": { "origin": [0, 0], "size": [900, 900] },
                        "exclusion": {} } }
                ]
            })),
        ]);
        let err = expand(&merged).unwrap_err();
        assert!(
            matches!(&err, LoadError::ExclusionComponentRemoved { entity }
                     if entity == &id(TEMPLATE)),
            "the refusal names the template that holds it: {err}"
        );
        // Nothing stamps it, so this is the scan reaching inside the template
        // rather than the one that walks minted entities.
        assert!(!merged.entities.values().any(|e| e.has("stamp")));
    }

    #[test]
    fn geometry_declared_in_a_lattice_frame_fails_the_load() {
        let lattice = "f0000000-0000-4000-8000-000000000003";
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                lattice,
                json!({ "frame": { "parent": FLOOR, "dims": 2, "units": "index" } }),
            ),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": lattice, "origin": [0, 0], "size": [1800, 1800] },
                    "tiling": { "pitch": [900, 900] }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&merged),
            Err(LoadError::GeometryInLatticeFrame { .. })
        ));
        let _ = Frame {
            parent: None,
            dims: 2,
            units: "mm".into(),
            unit_mm: Some(1.0),
            integer_only: false,
            vendor: None,
            transform_to_parent: None,
        };
    }

    const STAMP: &str = "b0000000-0000-4000-8000-000000000001";
    const TEMPLATE: &str = "c0000000-0000-4000-8000-000000000001";

    fn param_template(body: Value) -> Entity {
        entity(TEMPLATE, json!({ "template": body }))
    }

    #[test]
    fn a_parameter_substitutes_as_a_whole_value_of_its_own_type() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": { "loop_transit_secs": { "default": 45 } },
                "entities": [
                    { "local": "s", "components": {
                        "station": { "kind": "pick", "role": "easyloop" },
                        "go.easyloop": {
                            "loop_transit_secs": { "$param": "loop_transit_secs" } } } }
                ]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE, "params": { "loop_transit_secs": 60 } }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let s = &expansion.entities[&at(STAMP, "instance:0/local:s")];
        assert_eq!(s.components["go.easyloop"]["loop_transit_secs"], json!(60));
    }

    #[test]
    fn an_unbound_parameter_without_a_default_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": { "loop_transit_secs": {} },
                "entities": [{ "local": "q", "components": { "queue": {} } }]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&merged),
            Err(LoadError::UnboundParam { .. })
        ));
    }

    #[test]
    fn a_reference_to_an_undeclared_parameter_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [{ "local": "s", "components": {
                    "go.easyloop": { "loop_transit_secs": { "$param": "nope" } } } }]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&merged),
            Err(LoadError::UnknownParam { ref param, .. }) if param == "nope"
        ));
    }

    #[test]
    fn a_stamp_binding_a_parameter_the_template_never_declared_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": { "loop_transit_secs": { "default": 45 } },
                "entities": [{ "local": "q", "components": { "queue": {} } }]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE, "params": { "loop_transit_sesc": 60 } }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&merged),
            Err(LoadError::UnexpectedParam { ref param, .. }) if param == "loop_transit_sesc"
        ));
    }

    #[test]
    fn a_plain_stamp_still_expands_under_instance_zero() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [{ "local": "station", "components": {
                    "pose": { "position": [0, 0] }, "station": { "kind": "induct" } } }]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [100, 200] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        assert!(expansion.entities.contains_key(&at(STAMP, "instance:0")));
        let station = &expansion.entities[&at(STAMP, "instance:0/local:station")];
        assert_eq!(station.components["pose"]["position"], json!([100, 200]));
    }

    const ROW_TEMPLATE: &str = "c0000000-0000-4000-8000-000000000002";

    /// What the row inside the single-instance template stamps: one cell.
    fn row_template() -> Entity {
        entity(
            ROW_TEMPLATE,
            json!({ "template": { "entities": [{ "local": "cell", "components": {
                "pose": { "position": [0, 0] }, "cell": {} } }] } }),
        )
    }

    /// The template the single-instance tests stamp: a tiled floor patch, a
    /// row that is itself a stamp, and a port wired to the patch by its handle,
    /// with one parameter.
    fn single_instance_template() -> Entity {
        param_template(json!({
            "params": { "kind": { "default": "pick" } },
            "entities": [
                { "local": "patch", "components": {
                    "region": { "origin": [0, 0], "size": [2000, 1000] },
                    "tiling": { "pitch": [1000, 1000] } } },
                { "local": "row", "components": {
                    "pose": { "position": [0, 2000], "heading_deg": 0 },
                    "stamp": { "template": ROW_TEMPLATE, "repeat": { "count": 2, "step": [1000, 0] } } } },
                { "local": "station", "components": {
                    "pose": { "position": [500, 500] },
                    "station": { "kind": { "$param": "kind" } },
                    "port": { "direction": "sink", "into": { "local_ref": "patch" } } } }
            ]
        }))
    }

    /// The single-instance step makes the one instance under the payload it is
    /// given, binds, rewrites and composes as [`expand`] does, and expands
    /// nothing inside: the patch derives no cells and the row no instances.
    #[test]
    fn instantiating_one_stamp_instance_composes_its_internals_and_expands_nothing() {
        let template = single_instance_template();
        let stamp = entity(
            STAMP,
            json!({
                "pose": { "frame": FLOOR, "position": [10000, 0], "heading_deg": 90 },
                "stamp": { "template": TEMPLATE, "params": { "kind": "put" } }
            }),
        );
        let declared: BTreeMap<EntityId, Entity> =
            [(template.id.clone(), template)].into_iter().collect();
        let made = instantiate_stamp(&stamp, &declared, "3,0").unwrap();

        assert_eq!(made.instance.id, at(STAMP, "instance:3,0"));
        let ids: Vec<String> = made.internals.iter().map(|e| e.id.to_string()).collect();
        assert_eq!(
            ids,
            ["patch", "row", "station"]
                .map(|h| at(STAMP, &format!("instance:3,0/local:{h}")).to_string())
        );
        let [patch, row, station] = [&made.internals[0], &made.internals[1], &made.internals[2]];
        // A quarter turn re-corners the region and turns the pitch.
        assert_eq!(patch.components["region"]["origin"], json!([9000, 0]));
        assert_eq!(patch.components["region"]["size"], json!([1000, 2000]));
        assert_eq!(row.components["pose"]["position"], json!([8000, 0]));
        assert_eq!(row.components["stamp"]["repeat"]["count"], json!(2));
        assert_eq!(station.components["station"]["kind"], json!("put"));
        assert_eq!(
            station.components["port"]["into"],
            json!(patch.id.to_string())
        );
    }

    /// One implementation of stamping: what the single-instance step makes under
    /// `"0"` is what [`expand`] makes for a plain stamp's instance 0.
    #[test]
    fn one_instantiation_matches_what_expand_makes_for_instance_zero() {
        let stamp = entity(
            STAMP,
            json!({
                "pose": { "frame": FLOOR, "position": [500, 700], "heading_deg": 180 },
                "stamp": { "template": TEMPLATE }
            }),
        );
        let merged = merged_of(vec![
            floor_frame(),
            single_instance_template(),
            row_template(),
            stamp.clone(),
        ]);
        let expansion = expand(&merged).unwrap();
        let made = instantiate_stamp(&stamp, &merged.entities, "0").unwrap();
        for internal in &made.internals {
            assert_eq!(
                expansion.entities[&internal.id].components, internal.components,
                "{}",
                internal.id
            );
        }
    }

    #[test]
    fn instantiating_a_repeated_stamp_or_a_slashed_payload_is_refused() {
        let template = single_instance_template();
        let declared: BTreeMap<EntityId, Entity> =
            [(template.id.clone(), template)].into_iter().collect();
        let repeated = entity(
            STAMP,
            json!({
                "pose": { "frame": FLOOR, "position": [0, 0] },
                "stamp": { "template": TEMPLATE, "repeat": { "count": 2, "step": [1000, 0] } }
            }),
        );
        let err = instantiate_stamp(&repeated, &declared, "0").unwrap_err();
        assert!(matches!(err, LoadError::Unsupported { .. }), "{err}");

        let plain = entity(
            STAMP,
            json!({ "pose": { "frame": FLOOR, "position": [0, 0] }, "stamp": { "template": TEMPLATE } }),
        );
        let err = instantiate_stamp(&plain, &declared, "1/2").unwrap_err();
        assert!(matches!(err, LoadError::Unsupported { .. }), "{err}");
    }

    #[test]
    fn a_local_ref_becomes_the_stamped_uuid_of_that_handle() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [
                    { "local": "a", "components": { "pose": { "position": [0, 0] },
                                                    "cell": {} } },
                    { "local": "b", "components": { "port": { "direction": "sink",
                                                              "into": { "local_ref": "a" } } } }
                ]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let a = at(STAMP, "instance:0/local:a");
        let b = &expansion.entities[&at(STAMP, "instance:0/local:b")];
        assert_eq!(b.components["port"]["into"], json!(a.to_string()));
    }

    /// The two-key form `{"local_ref": h, "key": k}` becomes a real derivation
    /// address once a stamp has minted the handle's UUID — this is how a
    /// template's queue names its own tiling's cells before those cells exist.
    #[test]
    fn a_local_address_becomes_a_derivation_address_under_the_stamp() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [
                    { "local": "q", "components": {
                        "region": { "origin": [0, 0], "size": [2700, 1800] },
                        "tiling": { "pitch": [900, 900], "derives": { "cell": {} } },
                        "queue": { "path": [
                            { "local_ref": "q", "key": "cell:0,0" },
                            { "local_ref": "q", "key": "cell:2,1" }
                        ] } } }
                ]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let queue_owner = at(STAMP, "instance:0/local:q");
        let path = expansion.entities[&queue_owner].components["queue"]["path"]
            .as_array()
            .expect("a path");
        assert_eq!(path[0]["generator"], STAMP);
        assert_eq!(path[0]["key"], "instance:0/local:q/cell:0,0");
        assert_eq!(path[1]["generator"], STAMP);
        assert_eq!(path[1]["key"], "instance:0/local:q/cell:2,1");
        // The address must also decode as the queue consumer reads it.
        let _: Queue = expansion.entities[&queue_owner].require("queue").unwrap();
        // And both name cells the same expansion actually produced.
        for key in ["cell:0,0", "cell:2,1"] {
            assert!(expansion
                .entities
                .contains_key(&id(&format!("{queue_owner}/{key}"))));
        }
    }

    #[test]
    fn a_local_ref_in_unrecognized_company_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": {},
                "entities": [{ "local": "q", "components": {
                    "queue": { "path": [
                        { "local_ref": "q", "key": "cell:0,0", "index": 1 }
                    ] } } }]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&merged),
            Err(LoadError::MalformedLocalRef { .. })
        ));
    }

    /// One rack, both kinds of level, keyed off one list. The relay level sits
    /// second, so a reader who wants "the shelf at 700" asks for level 2 whether
    /// or not a relay level came before it.
    #[test]
    fn a_racks_levels_key_its_relays_and_its_slots_off_one_list() {
        const RACK: &str = "b0000000-0000-4000-8000-000000000003";
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                RACK,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "rack": { "slots_wide": 2, "slots_deep": 1,
                              "footprint": [2750, 1300],
                              "levels": [
                                  { "height": 350 },
                                  { "height": 400,
                                    "positions": [[340, 1300], [1030, 1300]],
                                    "reach": 600 },
                                  { "height": 700 }
                              ] }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        // 2 relays at level 1, plus 2 storage levels x 2 columns x 1 deep = 4.
        assert_eq!(expansion.counts[&root(RACK)], 6);
        assert_eq!(
            expansion.entities[&at(RACK, "relay:1,1")].components["relay"],
            json!({ "rack": RACK, "level": 1, "index": 1,
                    "height": 400, "offset": [1030, 1300] })
        );
        for key in ["slot:0,0,0", "slot:0,1,0", "slot:2,0,0", "slot:2,1,0"] {
            assert!(expansion.entities.contains_key(&at(RACK, key)), "{key}");
        }
        // Level 1 is the relay level, so nothing stores there.
        assert!(!expansion.entities.contains_key(&at(RACK, "slot:1,0,0")));
    }

    /// Stacked relay levels — two heights over the same offsets on the bay —
    /// are two relays per position, individually addressable, because two
    /// shelves are two places to put a tote (SPEC §5).
    #[test]
    fn stacked_relay_levels_derive_a_relay_each() {
        const RACK: &str = "b0000000-0000-4000-8000-000000000004";
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                RACK,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "rack": { "slots_wide": 1, "slots_deep": 1,
                              "footprint": [2140, 850],
                              "levels": [
                                  { "height": 394, "reach": 850,
                                    "positions": [[425, 850], [1070, 850], [1715, 850]] },
                                  { "height": 952, "reach": 850,
                                    "positions": [[425, 850], [1070, 850], [1715, 850]] },
                                  { "height": 1580 }
                              ] }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        // Six relays and one slot.
        assert_eq!(expansion.counts[&root(RACK)], 7);
        assert_eq!(
            expansion.entities[&at(RACK, "relay:0,0")].components["relay"]["height"],
            394
        );
        assert_eq!(
            expansion.entities[&at(RACK, "relay:1,0")].components["relay"]["height"],
            952
        );
        assert_eq!(
            expansion.entities[&at(RACK, "relay:1,0")].components["relay"]["offset"],
            json!([425, 850])
        );
    }

    #[test]
    fn levels_that_do_not_climb_fail_the_load() {
        const RACK: &str = "b0000000-0000-4000-8000-000000000005";
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                RACK,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "rack": { "slots_wide": 1, "slots_deep": 1,
                              "footprint": [1000, 1000],
                              "levels": [{ "height": 700 }, { "height": 350 }] }
                }),
            ),
        ]);
        assert!(matches!(
            expand(&merged),
            Err(LoadError::RackLevelsNotIncreasing { index: 1, .. })
        ));
    }

    #[test]
    fn a_relay_level_that_states_no_reach_fails_the_load() {
        const RACK: &str = "b0000000-0000-4000-8000-000000000006";
        let rack_of = |level: Value| {
            merged_of(vec![
                floor_frame(),
                entity(
                    RACK,
                    json!({
                        "pose": { "frame": FLOOR, "position": [0, 0] },
                        "rack": { "slots_wide": 1, "slots_deep": 1,
                                  "footprint": [1000, 1000], "levels": [level] }
                    }),
                ),
            ])
        };
        assert!(matches!(
            expand(&rack_of(
                json!({ "height": 400, "positions": [[500, 1000]] })
            )),
            Err(LoadError::RelayLevelHasNoReach { index: 0, .. })
        ));
        // The offsets alone are enough to place the shelves, so a level that
        // states them and a reach needs nothing else.
        assert!(expand(&rack_of(
            json!({ "height": 400, "positions": [[500, 1000]], "reach": 600 })
        ))
        .is_ok());
    }
    #[test]
    fn a_parameter_or_a_bare_handle_where_a_reference_or_a_cell_goes_becomes_an_identity() {
        let merged = merged_of(vec![
            floor_frame(),
            entity(
                HIGHWAY,
                json!({
                    "region": { "frame": FLOOR, "origin": [0, 0], "size": [6000, 1000] },
                    "tiling": { "pitch": [1000, 1000] }
                }),
            ),
            param_template(json!({
                "params": { "floor": {}, "queue": {}, "exit": {} },
                "entities": [
                    { "local": "patch", "components": {
                        "region": { "origin": [0, 0], "size": [2000, 1000] },
                        "tiling": { "pitch": [1000, 1000] } } },
                    { "local": "stand", "components": { "pose": { "position": [2500, 500] }, "cell": {} } },
                    { "local": "seam", "components": {
                        "stitch": { "a": { "local_ref": "patch" }, "b": { "$param": "floor" }, "rule": "contact" } } },
                    { "local": "lane", "components": { "queue": { "path": [
                        { "local_ref": "patch", "key": { "$param": "queue" } },
                        { "local_ref": "stand" },
                        { "$param": "exit" }
                    ] } } }
                ]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 1000] },
                    "stamp": { "template": TEMPLATE, "params": {
                        "floor": HIGHWAY, "queue": "cell:1,0", "exit": format!("{HIGHWAY}/cell:3,0") } }
                }),
            ),
        ]);
        let expansion = expand(&merged).unwrap();
        let stitch = &expansion.entities[&at(STAMP, "instance:0/local:seam")].components["stitch"];
        assert_eq!(stitch["a"], json!(at(STAMP, "instance:0/local:patch").to_string()));
        assert_eq!(stitch["b"], json!(HIGHWAY), "the floor the stamp bound");

        let lane = &expansion.entities[&at(STAMP, "instance:0/local:lane")];
        assert_eq!(
            lane.components["queue"]["path"],
            json!([
                { "generator": STAMP, "key": "instance:0/local:patch/cell:1,0" },
                at(STAMP, "instance:0/local:stand").to_string(),
                format!("{HIGHWAY}/cell:3,0"),
            ]),
            "the picked tile, the template's own cell, the floor cell the stamp named"
        );
        let queue: Queue = lane.require("queue").unwrap();
        assert_eq!(queue.path.len(), 3, "every step decodes as the queue consumer reads it");
        for step in
            [at(STAMP, "instance:0/local:patch/cell:1,0"), at(STAMP, "instance:0/local:stand"), at(HIGHWAY, "cell:3,0")]
        {
            assert!(expansion.entities.get(&step).is_some_and(|cell| cell.has("cell")), "{step} is a cell");
        }
    }

    #[test]
    fn a_key_parameter_bound_to_something_other_than_a_string_fails_the_load() {
        let merged = merged_of(vec![
            floor_frame(),
            param_template(json!({
                "params": { "tile": {} },
                "entities": [{ "local": "q", "components": {
                    "region": { "origin": [0, 0], "size": [1800, 900] },
                    "tiling": { "pitch": [900, 900] },
                    "queue": { "path": [{ "local_ref": "q", "key": { "$param": "tile" } }] } } }]
            })),
            entity(
                STAMP,
                json!({
                    "pose": { "frame": FLOOR, "position": [0, 0] },
                    "stamp": { "template": TEMPLATE, "params": { "tile": [1, 0] } }
                }),
            ),
        ]);
        let err = expand(&merged).unwrap_err();
        assert!(
            matches!(err, LoadError::MalformedLocalRef { ref found, .. } if found == "key, local_ref")
                && err.to_string().contains("each holding a string"),
            "{err}"
        );
    }

    #[test]
    #[ignore = "Known Foundry rotated-cell gap; fwd bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg (ownership preserves current geometry)"]
    fn a_template_cells_size_turns_with_the_stamp_as_a_pitch_does() {
        let stamped_at = |heading: f64| {
            let merged = merged_of(vec![
                floor_frame(),
                param_template(json!({
                    "params": {},
                    "entities": [
                        { "local": "stand", "components": {
                            "pose": { "position": [1500, 500] },
                            "cell": { "size": [3000, 1000], "traversable_by": ["htm"] } } },
                        { "local": "lane", "components": {
                            "region": { "origin": [0, 1000], "size": [3000, 1000] },
                            "tiling": { "pitch": [1500, 1000],
                                        "derives": { "cell": { "size": [1500, 500] } } } } },
                        { "local": "fixed", "components": {
                            "pose": { "frame": FLOOR, "position": [9000, 9000] },
                            "cell": { "size": [3000, 1000] } } }
                    ]
                })),
                entity(
                    STAMP,
                    json!({
                        "pose": { "frame": FLOOR, "position": [0, 0], "heading_deg": heading },
                        "stamp": { "template": TEMPLATE }
                    }),
                ),
            ]);
            let expansion = expand(&merged).unwrap();
            let component = |key: &str, name: &str| expansion.entities[&at(STAMP, key)].components[name].clone();
            (
                component("instance:0/local:stand", "cell")["size"].clone(),
                component("instance:0/local:lane", "tiling")["pitch"].clone(),
                component("instance:0/local:lane/cell:0,0", "cell")["size"].clone(),
                component("instance:0/local:fixed", "cell")["size"].clone(),
            )
        };
        let standing = (json!([3000, 1000]), json!([1500, 1000]), json!([1500, 500]), json!([3000, 1000]));
        let swapped = (json!([1000, 3000]), json!([1000, 1500]), json!([500, 1500]), json!([3000, 1000]));
        assert_eq!(stamped_at(0.0), standing);
        assert_eq!(stamped_at(90.0), swapped);
        assert_eq!(stamped_at(180.0), standing);
        assert_eq!(stamped_at(270.0), swapped);
    }

    #[test]
    #[ignore = "Known Foundry rotated-cell gap; fwd bJpVt6Au3gUIWjV0ApPZr3SBB0lXmw6Iung2OKSrzAg (ownership preserves current geometry)"]
    fn a_template_cell_stating_a_size_stamped_off_a_quarter_turn_fails_the_load() {
        let stamped_with = |cell: Value| {
            merged_of(vec![
                floor_frame(),
                param_template(json!({
                    "params": {},
                    "entities": [
                        { "local": "stand", "components": {
                            "pose": { "position": [1500, 500] }, "cell": cell } }
                    ]
                })),
                entity(
                    STAMP,
                    json!({
                        "pose": { "frame": FLOOR, "position": [0, 0], "heading_deg": 30.0 },
                        "stamp": { "template": TEMPLATE }
                    }),
                ),
            ])
        };
        let err = expand(&stamped_with(json!({ "size": [1000, 1000] }))).unwrap_err();
        assert!(
            matches!(&err, LoadError::Unsupported { message, .. }
                     if message.contains("a template cell stating a `size` cannot be stamped at 30°")),
            "{err}"
        );
        assert!(expand(&stamped_with(json!({ "traversable_by": ["htm"] }))).is_ok());
    }

}
