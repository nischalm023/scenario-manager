//! Within `site-template`, `layout` is an evaluation's steps 4 to 8 (spec 05 section 4.3): presence, bindings and
//! the selective expansion of placements, which hands foundry's stamp instantiation one single-instance stamp per
//! placement instance and keeps what it instantiates as products; it then strips every `template.` component and
//! routes each made entity to its made scope. Real stamps, tilings and cohorts among the products stay what they
//! are, for world-proto-server to derive in the made site.

use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value, json};
use studio_protocol::DeclaredId;
use site_model::components::product::Template as Piece;
use site_model::entity::Entity;
use site_model::ids::{DeclaredEntityId, EntityId as ReaderId};
use site_model::pipeline::expand::instantiate_stamp;
use site_model::units::{Coord, Degrees, Steps};

use crate::count::Count;
use crate::evaluation::{Problem, ProblemCode, RelativeScope};
use crate::expression::{Dim, EvalFault};
use crate::knob::{KnobForm, KnobKey, KnobUnit};
use crate::source_key::SourceKey;
use crate::template::{
    BINDING, DESCRIPTION, KNOB, MAX_DEPTH, PLACEMENT, PREFIX, PRESENCE, TARGET_SCOPE, Template, bindings, read_piece,
    target_dim,
};
use crate::values::Knobs;

/// A made entity before it has an id: what made it, where it lands, its label and its components.
pub(crate) struct Made {
    pub key: SourceKey,
    pub scope: RelativeScope,
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

/// A placement as laid out: which handles stand for it in a list, and its direct products in source-key order.
pub(crate) struct Placed {
    pub stands_for: Option<Vec<String>>,
    pub products: Vec<SourceKey>,
}

/// A piece with its internals' own presence and bindings applied, once per evaluation.
struct Prepared {
    entity: Entity,
    absent: FxHashSet<String>,
    /// Per internal handle, the count knobs its repeat count binds, for the census.
    count_knobs: FxHashMap<String, Vec<KnobKey>>,
    has_exclusions: bool,
    binds_lengths: bool,
}

pub(crate) struct Layout<'t> {
    template: &'t Template<'t>,
    knobs: &'t Knobs<'t>,
    pub made: Vec<Made>,
    pub absent: FxHashSet<String>,
    pub placements: FxHashMap<String, Placed>,
    pub by_knob: BTreeMap<KnobKey, Count>, // ordered: the census is user-visible
    pub problems: Vec<Problem>,
    prepared: FxHashMap<DeclaredId, Option<Prepared>>,
    /// Present top-level entities' components after bindings, for the frames a length binding lands in.
    bound: FxHashMap<DeclaredId, Map<String, Value>>,
}

impl<'t> Layout<'t> {
    pub fn new(template: &'t Template<'t>, knobs: &'t Knobs<'t>) -> Self {
        Layout {
            template,
            knobs,
            made: Vec::new(),
            absent: FxHashSet::default(),
            placements: FxHashMap::default(),
            by_knob: BTreeMap::new(),
            problems: Vec::new(),
            prepared: FxHashMap::default(),
            bound: FxHashMap::default(),
        }
    }

    /// Steps 4 to 8 over the template's own entities, in the template's order.
    pub fn lay_out(&mut self) {
        let mut placements = Vec::new();
        let mut length_bound = Vec::new();
        for id in &self.template.order {
            let held = &self.template.entities[id];
            let components = &held.entity.components;
            let stripped = [KNOB, DESCRIPTION].iter().any(|name| components.contains_key(*name));
            if stripped || self.template.pieces.contains(id) {
                continue;
            }
            let key = SourceKey::entity(*id);
            if !self.present(components, &key) {
                self.absent.insert(key.to_string());
                continue;
            }
            let mut bound = components.clone();
            length_bound.extend(self.bind(&mut bound, &key).into_iter().map(|pointer| (*id, pointer)));
            self.bound.insert(*id, bound.clone());
            if bound.contains_key(PLACEMENT) {
                let counted = self.count_knobs_of(components);
                placements.push((key, held.scope.clone(), bound, counted));
            } else {
                self.made.push(Made {
                    key,
                    scope: held.scope.clone(),
                    name: held.entity.name.clone(),
                    components: strip(bound),
                });
            }
        }
        for (id, pointer) in length_bound {
            if let Some(why) = self.scaled_frame_under(&self.bound[&id], &pointer) {
                self.problems.push(Problem::error(ProblemCode::BindingInScaledFrame, why).at(id));
            }
        }
        for (key, scope, components, counted) in placements {
            self.place(&key, &scope, &components, &counted, &[]);
        }
    }

    /// Step 4 for one entity: present unless its `template.presence` is below one; a fault leaves it out.
    fn present(&mut self, components: &Map<String, Value>, at: &SourceKey) -> bool {
        match self.knobs.present(components) {
            Ok(present) => present,
            Err(fault) => {
                self.problems.push(fault_problem(fault, "the presence condition").at(at));
                false
            }
        }
    }

    /// Step 5 for one entity: each binding's value replaces the default it points at. Answers the pointers that
    /// bound a length, for the scaled-frame rule.
    fn bind(&mut self, components: &mut Map<String, Value>, at: &SourceKey) -> Vec<String> {
        let mut lengths = Vec::new();
        let Ok(bindings) = bindings(components) else { return lengths };
        let mut whole = Value::Object(std::mem::take(components));
        for (pointer, expr) in bindings {
            match self.knobs.eval(&expr) {
                Ok(value) => {
                    if let Some(slot) = whole.pointer_mut(&pointer) {
                        *slot = json!(value);
                    }
                    if target_dim(&pointer) == Some(Dim::Length) {
                        lengths.push(pointer);
                    }
                }
                Err(fault) => self.problems.push(fault_problem(fault, &format!("the binding `{pointer}`")).at(at)),
            }
        }
        if let Value::Object(map) = whole {
            *components = map;
        }
        lengths
    }

    /// The range count knobs a placement's repeat count bindings name, for the census.
    fn count_knobs_of(&self, components: &Map<String, Value>) -> Vec<KnobKey> {
        let mut out = Vec::new();
        for (pointer, expr) in bindings(components).unwrap_or_default() {
            if !pointer.starts_with("/template.placement/repeat/count/") {
                continue;
            }
            for name in expr.knobs() {
                let decl = self.template.knobs.iter().find(|decl| decl.key.as_str() == name);
                if let Some(decl) =
                    decl.filter(|d| d.unit == KnobUnit::Count && matches!(d.form, KnobForm::Range { .. }))
                {
                    out.push(decl.key.clone());
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Rule from spec 05 section 3.3: a length bound into a frame that scales that axis by anything but 1 or -1.
    fn scaled_frame_under(&self, components: &Map<String, Value>, pointer: &str) -> Option<String> {
        let segments: Vec<&str> = pointer.split('/').skip(1).collect();
        let frame = match segments.first().copied() {
            Some("frame") => components.get("frame").and_then(|frame| frame.get("parent")),
            Some("region" | "tiling") => components.get("region").and_then(|region| region.get("frame")),
            Some(_) => components.get("pose").and_then(|pose| pose.get("frame")),
            None => None,
        }?;
        let axis: usize = segments.last()?.parse().ok()?;
        let scale = self.frame_scale(frame.as_str()?.parse().ok()?, axis);
        (scale.abs() != 1.0)
            .then(|| format!("the binding `{pointer}` sets a length in a frame scaled by {scale} on that axis"))
    }

    /// The product of the scales along a frame's chain to the primary frame, on one axis.
    fn frame_scale(&self, frame: DeclaredId, axis: usize) -> f64 {
        let mut scale = 1.0;
        let mut at = Some(frame);
        let mut hops = 0;
        while let Some(id) = at.filter(|_| hops < 64) {
            let Some(frame) = self.bound.get(&id).and_then(|components| components.get("frame")) else { break };
            let transform = frame.get("transform_to_parent");
            scale *=
                transform.and_then(|t| t.get("scale")).and_then(|s| s.get(axis)).and_then(Value::as_f64).unwrap_or(1.0);
            at = frame.get("parent").and_then(Value::as_str).and_then(|parent| parent.parse().ok());
            hops += 1;
        }
        scale
    }

    /// Step 6 for one placement: each instance through foundry's instantiation step, its products made, nested
    /// placements laid out in turn.
    fn place(
        &mut self,
        key: &SourceKey,
        scope: &RelativeScope,
        components: &Map<String, Value>,
        counted: &[KnobKey],
        chain: &[DeclaredId],
    ) {
        let invalid = |why: String| {
            Problem::error(ProblemCode::TemplateInvalid, format!("This template cannot be evaluated: {why}")).at(key)
        };
        let placement = &components[PLACEMENT];
        let repeat = placement.get("repeat");
        let read = |field: &str, axis: usize| repeat?.get(field)?.get(axis)?.as_i64();
        let count = |axis: usize| read("count", axis).map_or(Some(Count::ONE), Count::from_json);
        // The placement as bound is held to its shape before a stamp is made of it: a count is zero or more (spec 05
        // section 3.2), where a binding can take it below zero at some knobs.
        let (Some(along_x), Some(along_y)) = (count(0), count(1)) else {
            let written = [read("count", 0), read("count", 1)].map(|count| count.unwrap_or(1));
            self.problems
                .push(invalid(format!("its repeat count is {written:?} at these knobs; a count is zero or more")));
            return;
        };
        let pitch = [Steps(read("pitch", 0).unwrap_or(0)), Steps(read("pitch", 1).unwrap_or(0))];
        if let Err(why) = self.template.registry().validate(PLACEMENT, placement) {
            self.problems.push(invalid(format!("the placement as bound at these knobs: {why}")));
            return;
        }
        let Some(piece_id) =
            placement.get("piece").and_then(Value::as_str).and_then(|id| id.parse::<DeclaredId>().ok())
        else {
            return;
        };
        if chain.contains(&piece_id) || chain.len() > MAX_DEPTH {
            self.problems
                .push(invalid(format!("the piece {piece_id} reaches itself, or placements nest deeper than eight")));
            return;
        }
        if !self.prepared.contains_key(&piece_id) {
            let prepared = self.prepare(piece_id);
            self.prepared.insert(piece_id, prepared);
        }
        let Some(prepared) = self.prepared.get(&piece_id).and_then(Option::as_ref) else { return };
        let pose = components.get("pose").cloned().unwrap_or(Value::Null);
        let heading = Degrees(pose.get("heading_deg").and_then(Value::as_f64).unwrap_or(0.0));
        if heading.quarter_turns() != Some(0) && prepared.has_exclusions {
            self.problems.push(invalid(format!("the piece {piece_id} carries a tiling with exclusions and is placed at heading {heading}, which only heading 0 allows (rule 4)")));
            return;
        }
        let frame = pose.get("frame").and_then(Value::as_str).and_then(|frame| frame.parse::<DeclaredId>().ok());
        let scaled = frame.is_some_and(|frame| (0..2).any(|axis| self.frame_scale(frame, axis).abs() != 1.0));
        if scaled && prepared.binds_lengths {
            self.problems.push(
                Problem::error(
                    ProblemCode::BindingInScaledFrame,
                    format!("the piece {piece_id} binds lengths and is placed in a scaled frame"),
                )
                .at(key),
            );
        }
        let position: Vec<Coord> = pose
            .get("position")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|axis| axis.as_i64().map(Coord))
            .collect();
        let (Some(&x), Some(&y)) = (position.first(), position.get(1)) else {
            self.problems.push(invalid("a placement's pose has a planar position".into()));
            return;
        };
        let piece_entity = prepared.entity.clone();
        let absent = prepared.absent.clone();
        let nested_counts = prepared.count_knobs.clone();
        let declared: BTreeMap<ReaderId, Entity> = [(piece_entity.id.clone(), piece_entity)].into_iter().collect();
        let Ok(stamp_id) = key.as_str().parse::<ReaderId>() else { return };
        let mut products = Vec::new();
        let mut chain = chain.to_vec();
        chain.push(piece_id);
        for i in along_x.indexes() {
            for j in along_y.indexes() {
                let mut stamp = Entity::new(stamp_id.clone());
                let at = [i.marched(x, pitch[0]), j.marched(y, pitch[1])];
                stamp
                    .components
                    .insert("pose".into(), json!({ "frame": pose["frame"], "position": at, "heading_deg": heading }));
                let params = placement.get("params").cloned().unwrap_or_else(|| json!({}));
                stamp.components.insert("stamp".into(), json!({ "template": piece_id.to_string(), "params": params }));
                let instantiation = match instantiate_stamp(&stamp, &declared, &format!("{i},{j}")) {
                    Ok(instantiation) => instantiation,
                    Err(fault) => {
                        self.problems.push(invalid(fault.to_string()));
                        return;
                    }
                };
                for internal in instantiation.internals {
                    let Ok(product) = internal.id.to_string().parse::<SourceKey>() else { continue };
                    let handle = product.handle().unwrap_or_default().to_owned();
                    if absent.contains(&handle) {
                        self.absent.insert(product.to_string());
                        continue;
                    }
                    let components: Map<String, Value> = internal.components.into_iter().collect();
                    // A nested placement is held to its shape when it is laid out, below.
                    let held = components.iter().filter(|(name, _)| name.starts_with(PREFIX) && *name != PLACEMENT);
                    for (name, value) in held {
                        if let Err(why) = self.template.registry().validate(name, value) {
                            self.problems.push(invalid(why));
                        }
                    }
                    let lands = components
                        .get(TARGET_SCOPE)
                        .and_then(|target| target.get("scope"))
                        .and_then(Value::as_str)
                        .map_or_else(|| scope.clone(), RelativeScope::new);
                    products.push(product.clone());
                    if components.contains_key(PLACEMENT) {
                        let counted = nested_counts.get(&handle).cloned().unwrap_or_default();
                        self.place(&product, &lands, &components, &counted, &chain);
                    } else {
                        self.made.push(Made {
                            key: product,
                            scope: lands,
                            name: internal.name,
                            components: strip(components),
                        });
                    }
                }
            }
        }
        let instances = along_x * along_y;
        for knob in counted {
            *self.by_knob.entry(knob.clone()).or_default() += instances;
        }
        products.sort();
        let stands_for = placement
            .get("stands_for")
            .and_then(Value::as_array)
            .map(|handles| handles.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        self.placements.insert(key.to_string(), Placed { stands_for, products });
    }

    /// A piece with its internals' presence and bindings applied, or `None` when it does not read.
    fn prepare(&mut self, piece_id: DeclaredId) -> Option<Prepared> {
        let mut piece: Piece = read_piece(self.template.entities[&piece_id].entity).ok()?;
        let at = SourceKey::entity(piece_id);
        let mut prepared = Prepared {
            entity: Entity::new(DeclaredEntityId(piece_id.as_uuid())),
            absent: FxHashSet::default(),
            count_knobs: FxHashMap::default(),
            has_exclusions: false,
            binds_lengths: false,
        };
        for internal in &mut piece.entities {
            let mut components: Map<String, Value> = std::mem::take(&mut internal.components).into_iter().collect();
            if !self.present(&components, &at) {
                prepared.absent.insert(internal.local.clone());
            } else {
                let counted = self.count_knobs_of(&components);
                prepared.count_knobs.insert(internal.local.clone(), counted);
                prepared.binds_lengths |= !self.bind(&mut components, &at).is_empty();
            }
            components.remove(PRESENCE);
            components.remove(BINDING);
            let exclusions =
                components.get("tiling").and_then(|tiling| tiling.get("exclusions")).and_then(Value::as_array);
            prepared.has_exclusions |= exclusions.is_some_and(|list| !list.is_empty());
            internal.components = components.into_iter().collect();
        }
        prepared.entity.components.insert("template".into(), serde_json::to_value(&piece).ok()?);
        Some(prepared)
    }
}

/// Step 7 for one entity: every `template.` component goes.
fn strip(mut components: Map<String, Value>) -> Map<String, Value> {
    components.retain(|name, _| !name.starts_with(PREFIX));
    components
}

pub(crate) fn fault_problem(fault: EvalFault, what: &str) -> Problem {
    let code = match fault {
        EvalFault::Overflow => ProblemCode::ExpressionOverflow,
        EvalFault::DivisionByZero => ProblemCode::DivisionByZero,
    };
    Problem::error(code, format!("{what} has no value: {fault}"))
}
