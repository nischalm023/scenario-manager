//! Within `site-template`, `template` is an evaluation's step 2 (spec 05 section 4.3): the template read whole and
//! held to section 3.4's authoring rules and to the registered schemas before any knob is read, so that a template
//! fault is `template-invalid`, naming the entity, and never a strange site.

use std::fmt::Display;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value};
use studio_protocol::{DeclaredId, EntityRef, Pin, RefPin, ScopePath};
use site_model::components::product::Template as Piece;

use crate::evaluation::{Problem, ProblemCode, RelativeScope};
use crate::expression::{Dim, Expr};
use crate::knob::{self, DeclFault, KnobDecl};
use crate::read::{DeclaredEntity, Record, RegistrySnapshot, ScopeSnapshot, TemplateRead};
use crate::references::{self, Position, Rewrite};

pub(crate) const KNOB: &str = "template.knob";
pub(crate) const DESCRIPTION: &str = "template.description";
pub(crate) const PLACEMENT: &str = "template.placement";
pub(crate) const BINDING: &str = "template.knob_binding";
pub(crate) const PRESENCE: &str = "template.presence";
pub(crate) const TARGET_SCOPE: &str = "template.target_scope";
pub(crate) const PREFIX: &str = "template.";

/// A template read and checked: its scopes, its entities by id, its knobs, its title and its primary frame.
pub(crate) struct Template<'r> {
    pub read: &'r TemplateRead,
    pub scopes: Vec<(RelativeScope, &'r ScopeSnapshot)>,
    pub entities: FxHashMap<DeclaredId, Held<'r>>,
    /// The template's own entities in a stable order: scopes as read, then as each scope lists them.
    pub order: Vec<DeclaredId>,
    /// Every template entity a placement names, at any depth: none of them is made.
    pub pieces: FxHashSet<DeclaredId>,
    pub knobs: Vec<KnobDecl>,
    pub title: String,
    pub primary_frame: DeclaredId,
    pub unit_mm: f64,
    catalog: FxHashSet<DeclaredId>,
}

pub(crate) struct Held<'r> {
    pub scope: RelativeScope,
    pub entity: &'r DeclaredEntity,
}

/// What a reference names, by the UUID it opens with.
pub(crate) enum Names {
    Template,
    Catalog,
    Elsewhere,
}

impl<'r> Template<'r> {
    pub fn read(read: &'r TemplateRead) -> Result<Template<'r>, Vec<Problem>> {
        let mut faults = Faults::default();
        let scopes = relative_scopes(&read.template, &read.scopes, &mut faults);
        let mut entities = FxHashMap::default();
        let mut order = Vec::new();
        for (relative, snapshot) in &scopes {
            for entity in &snapshot.entities {
                if entities.insert(entity.id, Held { scope: relative.clone(), entity }).is_some() {
                    faults.at(entity.id, "two template scopes declare this entity");
                }
                order.push(entity.id);
            }
        }
        let Some((primary_frame, unit_mm)) = primary_frame(&scopes, &entities) else {
            faults.0.push(invalid(
                "the template's prologue must name its primary frame, a frame of its map scope stating `unit_mm`",
            ));
            return Err(faults.0);
        };
        let catalog = read.catalog.iter().flat_map(|scope| scope.entities.iter().map(|entity| entity.id)).collect();
        let mut template = Template {
            read,
            scopes,
            entities,
            order,
            pieces: FxHashSet::default(),
            knobs: Vec::new(),
            title: String::new(),
            primary_frame,
            unit_mm,
            catalog,
        };
        template.read_root(&mut faults);
        template.read_entities(&mut faults);
        template.read_records(&mut faults);
        if faults.0.is_empty() { Ok(template) } else { Err(faults.0) }
    }

    pub fn names(&self, id: DeclaredId) -> Names {
        if self.entities.contains_key(&id) {
            Names::Template
        } else if self.catalog.contains(&id) {
            Names::Catalog
        } else {
            Names::Elsewhere
        }
    }

    pub fn registry(&self) -> &'r RegistrySnapshot {
        &self.read.registry
    }

    /// The template's scopes relative to its top scope, as they would be made.
    pub fn has_scope(&self, relative: &str) -> bool {
        self.scopes.iter().any(|(scope, _)| scope.as_str() == relative)
    }

    fn read_root(&mut self, faults: &mut Faults) {
        let mut decls = Vec::new();
        let mut titles = Vec::new();
        for id in &self.order {
            let held = &self.entities[id];
            let components = &held.entity.components;
            let is_root = held.scope == RelativeScope::root();
            let card = [KNOB, DESCRIPTION].iter().filter(|name| components.contains_key(**name)).count();
            if is_root && card != 1 {
                faults.at(*id, "the top scope holds knob entities and one description entity, nothing else");
            }
            if !is_root && card != 0 {
                faults.at(*id, "a knob or a description stands only in the template's top scope");
            }
            if let Some(component) = components.get(KNOB) {
                match KnobDecl::read(*id, component) {
                    Ok(decl) => decls.push(decl),
                    Err(why) => faults.at(*id, why),
                }
            }
            if let Some(card) = components.get(DESCRIPTION) {
                titles.push(card.get("title").and_then(Value::as_str).unwrap_or_default().to_owned());
            }
        }
        match knob::read_all(decls) {
            Ok(decls) => self.knobs = decls,
            Err(DeclFault::Invalid(why)) => faults.0.push(invalid(why)),
            Err(DeclFault::Dimension(why)) => faults.0.push(Problem::error(ProblemCode::DimensionMismatch, why)),
        }
        match titles.as_slice() {
            [title] => self.title = title.clone(),
            _ => faults.0.push(invalid(format!("the top scope holds {} description entities, not one", titles.len()))),
        }
    }

    fn read_entities(&mut self, faults: &mut Faults) {
        for id in self.order.clone() {
            let entity = self.entities[&id].entity;
            if entity.components.contains_key(KNOB) || entity.components.contains_key(DESCRIPTION) {
                continue;
            }
            self.check_entity(id, &entity.components, false, faults);
            if entity.components.contains_key(TARGET_SCOPE) {
                faults.at(id, "`template.target_scope` stands only on a piece's internal");
            }
            if let Some(placement) = entity.components.get(PLACEMENT) {
                if entity.components.get("pose").and_then(|pose| pose.get("frame")).is_none() {
                    faults.at(id, "a placement stands beside a `pose` that names its frame");
                }
                self.read_placement(id, placement, 0, faults);
            }
            if let Some(found) = marker_outside_pieces(&entity.components) {
                faults.at(id, format!("`{found}` appears only inside a piece (rule 1)"));
            }
        }
    }

    /// A placement's piece, read and checked once, and the pieces its own internals place, depth first.
    fn read_placement(&mut self, by: DeclaredId, placement: &Value, depth: usize, faults: &mut Faults) {
        let piece_ref = placement.get("piece").and_then(Value::as_str).unwrap_or_default();
        let Some(piece_id) = piece_ref.parse::<DeclaredId>().ok().filter(|id| self.entities.contains_key(id)) else {
            faults.at(by, format!("`{piece_ref}` is no template entity: a placement's piece is one, named bare"));
            return;
        };
        if depth > MAX_DEPTH {
            faults.at(by, "placements nest deeper than eight");
            return;
        }
        let piece = match read_piece(self.entities[&piece_id].entity) {
            Ok(piece) => piece,
            Err(why) => {
                faults.at(piece_id, why);
                return;
            }
        };
        let handles: Vec<&str> = piece.entities.iter().map(|internal| internal.local.as_str()).collect();
        for handle in placement.get("stands_for").and_then(Value::as_array).into_iter().flatten() {
            if !handle.as_str().is_some_and(|handle| handles.contains(&handle)) {
                faults.at(by, format!("`stands_for` names {handle}, which the piece has no internal for"));
            }
        }
        if !self.pieces.insert(piece_id) {
            return;
        }
        for internal in &piece.entities {
            let components: Map<String, Value> = internal.components.clone().into_iter().collect();
            self.check_entity(piece_id, &components, true, faults);
            if let Some(scope) = components.get(TARGET_SCOPE).and_then(|target| target.get("scope"))
                && !scope.as_str().is_some_and(|scope| self.has_scope(scope))
            {
                faults.at(piece_id, format!("`{}` names {scope}, a scope the template lacks", internal.local));
            }
            if let Some(nested) = components.get(PLACEMENT) {
                self.read_placement(piece_id, nested, depth + 1, faults);
            }
        }
    }

    /// What every template entity and piece internal is held to: no `external_ids` (rule 7), bindings and presence
    /// that parse and name declared knobs, references that carry the pins rule 6 allows, and, outside a piece,
    /// registered schemas, a piece's `template` among them as world-proto-server validates it, placeholders and
    /// all (rule 1); inside a piece, component names the registry defines, the values validated once
    /// instantiation has replaced their placeholders.
    fn check_entity(
        &self,
        id: impl Display + Copy,
        components: &Map<String, Value>,
        inside_piece: bool,
        faults: &mut Faults,
    ) {
        if components.contains_key("external_ids") {
            faults.at(id, "no template entity carries `external_ids`: the engine writes the source key (rule 7)");
        }
        for (pointer, expr) in bindings(components).unwrap_or_else(|why| {
            faults.at(id, why);
            Vec::new()
        }) {
            if let Err((code, why)) = self.check_binding(components, &pointer, &expr) {
                faults.coded(code, id, why);
            }
        }
        if let Some(presence) = components.get(PRESENCE) {
            match presence.get("when").and_then(Value::as_str).map(Expr::parse) {
                Some(Ok(expr)) => {
                    if let Err((code, why)) = self.check_expression(&expr, Some(Dim::Count)) {
                        faults.coded(code, id, why);
                    }
                }
                Some(Err(fault)) => faults.at(id, fault.to_string()),
                None => faults.at(id, "`template.presence` holds its condition in `when`"),
            }
        }
        let mut copy = components.clone();
        references::rewrite(&mut copy, self.registry(), &mut |position: Position<'_>| {
            if let Some(why) = self.pin_fault(position.value) {
                faults.at(id, format!("`{}` at `{}`: {why}", position.component, position.path));
            }
            Rewrite::Keep
        });
        for (name, value) in components {
            if !inside_piece {
                if let Err(why) = self.registry().validate(name, value) {
                    faults.at(id, why);
                }
            } else if !self.registry().defines(name) {
                faults.at(id, format!("`{name}` inside the piece is no component the registry defines"));
            }
        }
    }

    /// Rule 6: a reference to a template entity carries no pin; one to the catalog carries a changeset or a tag.
    pub fn pin_fault(&self, value: &Value) -> Option<String> {
        let reference: EntityRef = value.as_str()?.parse().ok()?;
        match (self.names(reference.id().declared()), reference.pin()) {
            (Names::Template, Some(pin)) => {
                Some(format!("`{reference}` names the template's own entity with pin `{pin}`"))
            }
            (Names::Catalog, Some(RefPin::Pin(Pin::Changeset(_) | Pin::Tag(_)))) => None,
            (Names::Catalog, _) => Some(format!("`{reference}` names the catalog without a changeset or a tag")),
            _ => None,
        }
    }

    fn check_binding(
        &self,
        components: &Map<String, Value>,
        pointer: &str,
        expr: &Expr,
    ) -> Result<(), (ProblemCode, String)> {
        let invalid = |why: String| (ProblemCode::TemplateInvalid, why);
        let component = pointer.split('/').nth(1).unwrap_or_default();
        if [BINDING, PRESENCE, "external_ids"].contains(&component) {
            return Err(invalid(format!("the binding `{pointer}` points into `{component}`")));
        }
        let held = Value::Object(components.clone());
        if !held.pointer(pointer).is_some_and(|value| value.is_i64()) {
            return Err(invalid(format!(
                "the binding `{pointer}` names no integer the entity holds at the default knobs"
            )));
        }
        if is_reference_position(self.registry(), pointer) {
            return Err(invalid(format!("the binding `{pointer}` points at a reference")));
        }
        self.check_expression(expr, target_dim(pointer))
            .map_err(|(code, why)| (code, format!("the binding `{pointer}`: {why}")))
    }

    /// An expression names declared knobs only, and its dimensions meet where it is read as `want`.
    pub fn check_expression(&self, expr: &Expr, want: Option<Dim>) -> Result<(), (ProblemCode, String)> {
        if let Some(unknown) = expr.knobs().into_iter().find(|name| !knob::declared(&self.knobs, name)) {
            return Err((ProblemCode::TemplateInvalid, format!("`{unknown}` is no declared knob")));
        }
        expr.check_dims(&|name| knob::dim_of(&self.knobs, name), want)
            .map_err(|fault| (ProblemCode::DimensionMismatch, fault.to_string()))
    }

    /// Rule 3 for records: each aims beneath one of the template's own entities in its own scope, never beneath a
    /// knob, a description, a piece or a placement, from which world-proto-server derives nothing; an override is
    /// held as an entity is, `template.presence` the one template component it may hold.
    fn read_records(&self, faults: &mut Faults) {
        for (scope, snapshot) in &self.scopes {
            for record in &snapshot.records {
                let (Record::Override { target, .. } | Record::Suppress { target }) = record;
                match self.entities.get(&target.declared()) {
                    None => faults.at(target, "the record addresses no generator of the template (rule 3)"),
                    Some(held) if held.scope != *scope => faults.at(
                        target,
                        format!("the record stands in `{scope}` and its generator in `{}` (rule 3)", held.scope),
                    ),
                    Some(held) if DERIVE_NOTHING.iter().any(|name| held.entity.components.contains_key(*name)) => {
                        faults.at(
                            target,
                            "the record aims beneath a knob, a description, a piece or a placement (rule 3)",
                        )
                    }
                    Some(_) => {}
                }
                let Record::Override { components, .. } = record else { continue };
                if let Some(name) = components.keys().find(|name| name.starts_with(PREFIX) && *name != PRESENCE) {
                    faults.at(
                        target,
                        format!("a record holds no template component but `template.presence`, not `{name}`"),
                    );
                }
                if let Some(found) = marker_outside_pieces(components) {
                    faults.at(target, format!("`{found}` appears only inside a piece (rule 1)"));
                }
                self.check_entity(target, components, false, faults);
            }
        }
    }
}

pub(crate) const MAX_DEPTH: usize = 8;

/// The components of a template entity that world-proto-server derives nothing from, so no record aims beneath it.
const DERIVE_NOTHING: [&str; 4] = [KNOB, DESCRIPTION, PLACEMENT, "template"];

/// Every `(pointer, expression)` of an entity's `template.knob_binding`, `$comment` left out.
pub(crate) fn bindings(components: &Map<String, Value>) -> Result<Vec<(String, Expr)>, String> {
    let Some(binding) = components.get(BINDING) else { return Ok(Vec::new()) };
    let Some(map) = binding.as_object() else { return Err("`template.knob_binding` is an object".into()) };
    let mut out = Vec::new();
    for (pointer, text) in map {
        if pointer == "$comment" {
            continue;
        }
        let text = text.as_str().ok_or_else(|| format!("the binding `{pointer}` holds no expression"))?;
        out.push((pointer.clone(), Expr::parse(text).map_err(|fault| fault.to_string())?));
    }
    Ok(out)
}

/// The piece a template entity's `template` component packages, as foundry's reader reads it.
pub(crate) fn read_piece(entity: &DeclaredEntity) -> Result<Piece, String> {
    let component = entity.components.get("template").ok_or("a placement's piece carries `template`")?;
    serde_json::from_value(component.clone()).map_err(|fault| format!("the piece does not read: {fault}"))
}

/// What a binding's target measures, read off the protocol field it names; `None` where the engine has no rule.
pub(crate) fn target_dim(pointer: &str) -> Option<Dim> {
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let numbered = |at: usize| segments.get(at).is_some_and(|s| s.parse::<u32>().is_ok());
    Some(match segments.as_slice() {
        ["frame", "transform_to_parent", "translation", _] => Dim::Length,
        ["frame", "transform_to_parent", "scale", _] => Dim::Count,
        ["pose", "position", _] | ["region", "origin" | "size", _] | ["tiling", "pitch", _] => Dim::Length,
        ["tiling", "exclusions", _, "origin" | "size", _] if numbered(2) => Dim::Length,
        ["pose", "heading_deg"] => Dim::Angle,
        ["stamp", "repeat", "count"] | ["cohort" | "carrier_pool", "count"] => Dim::Count,
        ["stamp", "repeat", "step", _] | ["stitch", "tolerance"] => Dim::Length,
        ["template.placement", "repeat", "count", _] => Dim::Count,
        ["template.placement", "repeat", "pitch", _] => Dim::Length,
        _ => return None,
    })
}

/// Whether a JSON pointer lands on a field the registry annotates as a reference.
fn is_reference_position(registry: &RegistrySnapshot, pointer: &str) -> bool {
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let Some((component, rest)) = segments.split_first() else { return false };
    registry.refs(component).iter().any(|annotation| {
        let path: Vec<&str> = annotation.path.split('/').skip(1).collect();
        path.len() == rest.len() && path.iter().zip(rest).all(|(want, got)| *want == "*" || want == got)
    })
}

/// The first `$param` or `local_ref` outside a `template` component, which only a piece's internals may hold.
fn marker_outside_pieces(components: &Map<String, Value>) -> Option<&'static str> {
    fn find(value: &Value) -> Option<&'static str> {
        match value {
            Value::Object(map) if map.contains_key("$param") => Some("$param"),
            Value::Object(map) if map.contains_key("local_ref") => Some("local_ref"),
            Value::Object(map) => map.values().find_map(find),
            Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    components.iter().filter(|(name, _)| name.as_str() != "template").find_map(|(_, value)| find(value))
}

/// Rule 9: the top scope's prologue names the primary frame, a frame of the map scope stating `unit_mm`.
fn primary_frame(
    scopes: &[(RelativeScope, &ScopeSnapshot)],
    entities: &FxHashMap<DeclaredId, Held<'_>>,
) -> Option<(DeclaredId, f64)> {
    let (_, root) = scopes.iter().find(|(scope, _)| *scope == RelativeScope::root())?;
    let named = root.prologue.as_ref()?.primary_frame.as_ref()?;
    if named.segments().next().is_some() {
        return None;
    }
    let held = entities.get(&named.declared())?;
    let unit_mm = held.entity.components.get("frame")?.get("unit_mm")?.as_f64()?;
    (held.scope.as_str() == "map" && unit_mm > 0.0).then_some((named.declared(), unit_mm))
}

fn relative_scopes<'r>(
    root: &ScopePath,
    scopes: &'r [ScopeSnapshot],
    faults: &mut Faults,
) -> Vec<(RelativeScope, &'r ScopeSnapshot)> {
    let mut out: Vec<(RelativeScope, &'r ScopeSnapshot)> = Vec::new();
    for snapshot in scopes {
        let relative = if snapshot.scope == *root {
            Some(RelativeScope::root())
        } else {
            snapshot.scope.as_str().strip_prefix(&format!("{root}/")).map(RelativeScope::new)
        };
        match relative {
            Some(relative) if out.iter().all(|(seen, _)| *seen != relative) => out.push((relative, snapshot)),
            Some(relative) => faults.0.push(invalid(format!("the read holds the scope `{relative}` twice"))),
            None => faults.0.push(invalid(format!("`{}` is not a scope of `{root}`", snapshot.scope))),
        }
    }
    if out.iter().all(|(scope, _)| *scope != RelativeScope::root()) {
        faults.0.push(invalid(format!("the read holds no top scope `{root}`")));
    }
    out
}

fn invalid(why: impl Into<String>) -> Problem {
    Problem::error(ProblemCode::TemplateInvalid, format!("This template cannot be evaluated: {}", why.into()))
}

#[derive(Default)]
struct Faults(Vec<Problem>);

impl Faults {
    fn at(&mut self, at: impl Display, why: impl Into<String>) {
        self.0.push(invalid(why).at(at));
    }

    fn coded(&mut self, code: ProblemCode, at: impl Display, why: String) {
        match code {
            ProblemCode::TemplateInvalid => self.at(at, why),
            code => self.0.push(Problem::error(code, why).at(at)),
        }
    }
}
