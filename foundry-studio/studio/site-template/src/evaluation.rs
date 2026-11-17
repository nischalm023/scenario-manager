//! Within `site-template`, an `evaluation` is what one evaluation of a template at given knob values produces (spec
//! 05 section 3.6): per made scope the entities and records a made site would declare, the prologue, the problems,
//! the notes and the census, and the self-contained world document the API loads, compiles and validates.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use serde_json::{Map, Value, json};
use studio_protocol::{DeclaredId, EntityId};

use crate::count::Count;
use crate::knob::{KnobKey, KnobValues, Severity};
use crate::read::{Prologue, Record, TemplateRead};
use crate::references::{self, Position, Rewrite};
use crate::source_key::SourceKey;

pub const PROTO_VERSION: &str = "0.7.0"; // the minor foundry's reader reads

/// A made scope, named relative to the made site's root: `.` for the root, else `map`, `flow`, `fleet`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct RelativeScope(String);

impl RelativeScope {
    pub fn root() -> Self {
        Self(".".into())
    }

    pub fn new(relative: &str) -> Self {
        Self(relative.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RelativeScope {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

/// One evaluation's result. A `template-invalid` problem leaves the plan empty.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// Every declared knob's value: a set one as set, every other at its default, an expression's worked out.
    pub knobs: KnobValues,
    /// The knobs nobody set whose value their default expression gave (D120).
    pub derived: FxHashSet<KnobKey>,
    /// The made document's `doc` block and primary frame. Its `name` is the template's title: the API names it
    /// after the evaluation, or after the site on update.
    pub prologue: Prologue,
    /// Per made scope, what a made site declares there. Never iterate it into output unsorted.
    pub plan: FxHashMap<RelativeScope, ScopePlan>,
    pub problems: Vec<Problem>,
    pub notes: Vec<Note>,
    pub census: Census,
    pub engine: EngineStamp,
}

/// What one made scope declares, in source-key order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScopePlan {
    pub declarations: Vec<Declaration>,
    pub records: Vec<Record>,
}

/// One made entity: the id its source key has for the draft's life, its label and its components, the source key
/// written into its `external_ids` beside the copy kept here.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub id: DeclaredId,
    pub key: SourceKey,
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProblemCode {
    ValidationFailed,
    CheckFailed,
    TemplateInvalid,
    ExpressionOverflow,
    DivisionByZero,
    DimensionMismatch,
    BindingInScaledFrame,
    KnobNotWholeSteps,
    PlacementInSingleReference,
    ReferenceToAbsentEntity,
    ReferenceLeavesTemplate,
    DerivedOutOfRange,
}

/// What the person or the agent reads about an evaluation (spec 05 section 3.11), naming the knob to change or
/// the template entity at fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub severity: Severity,
    pub code: ProblemCode,
    pub knob: Option<KnobKey>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub text: String,
}

impl Problem {
    pub(crate) fn error(code: ProblemCode, text: impl Into<String>) -> Self {
        Self { severity: Severity::Error, code, knob: None, reference: None, text: format!("Error: {}", text.into()) }
    }

    pub(crate) fn at(self, reference: impl Display) -> Self {
        Self { reference: Some(reference.to_string()), ..self }
    }

    pub(crate) fn on(self, knob: &KnobKey) -> Self {
        Self { knob: Some(knob.clone()), ..self }
    }
}

/// Something the evaluation left out without fault, such as a record whose generator a knob made absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub text: String,
}

/// What an evaluation made, counted as discovered (spec 05 section 3.6): the instances each count knob's
/// placements made and the made entities per component. `derived` is the API's to fill from the floor compile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Census {
    pub by_knob: BTreeMap<KnobKey, Count>, // ordered: user-visible JSON
    pub declared: BTreeMap<String, Count>, // ordered: user-visible JSON
    pub derived: BTreeMap<String, Count>,  // ordered: user-visible JSON
}

/// Which engine made an evaluation. The API adds the foundry commit and the generator set it read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineStamp {
    pub version: String,
    pub registry: BTreeMap<String, String>, // ordered: user-visible JSON
}

/// A world document foundry's reader loads: `proto_version`, the `doc` block, the primary frame and the entities.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldDocument(Value);

impl WorldDocument {
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.0).expect("a JSON value serializes")
    }
}

impl Evaluation {
    /// The evaluation as one self-contained world document: the made scopes' declarations and records, then every
    /// catalog entity the template pins, every pin dropped from every reference, as world-proto-server hands its
    /// generators pinned references bare. No `depends_on`: the catalog rides inside.
    pub fn document(&self, read: &TemplateRead) -> WorldDocument {
        let mut entities = Vec::new();
        let mut scopes: Vec<(&RelativeScope, &ScopePlan)> = self.plan.iter().collect();
        scopes.sort_by(|a, b| a.0.cmp(b.0));
        for (_, plan) in &scopes {
            for declaration in &plan.declarations {
                entities.push(entity_json(declaration.id, &declaration.name, &declaration.components, read));
            }
        }
        for (_, plan) in &scopes {
            entities.extend(plan.records.iter().map(|record| record_json(record, read)));
        }
        let mut catalog: Vec<_> = read.catalog.iter().collect();
        catalog.sort_by(|a, b| a.scope.cmp(&b.scope));
        for scope in catalog {
            for entity in &scope.entities {
                entities.push(entity_json(entity.id, &entity.name, &entity.components, read));
            }
            entities.extend(scope.records.iter().map(|record| record_json(record, read)));
        }
        let mut doc = json!({ "id": self.prologue.doc.to_string(), "kind": self.prologue.kind });
        if let Some(name) = &self.prologue.name {
            doc["name"] = json!(name);
        }
        let mut document = json!({ "proto_version": PROTO_VERSION, "doc": doc, "entities": entities });
        if let Some(frame) = &self.prologue.primary_frame {
            document["primary_frame"] = json!(frame.to_string());
        }
        WorldDocument(document)
    }

    /// Every made entity's source key and id, in source-key order: what the draft records for its next evaluation.
    pub fn ids(&self) -> Vec<(SourceKey, DeclaredId)> {
        let mut out: Vec<(SourceKey, DeclaredId)> =
            self.plan.values().flat_map(|plan| plan.declarations.iter().map(|d| (d.key.clone(), d.id))).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn has_errors(&self) -> bool {
        self.problems.iter().any(|problem| problem.severity == Severity::Error)
    }
}

fn entity_json(id: DeclaredId, name: &Option<String>, components: &Map<String, Value>, read: &TemplateRead) -> Value {
    let mut out = json!({ "id": id.to_string(), "components": unpinned(components, read) });
    if let Some(name) = name {
        out["name"] = json!(name);
    }
    out
}

fn record_json(record: &Record, read: &TemplateRead) -> Value {
    let derivation = |target: &EntityId| {
        let text = target.to_string();
        let (generator, key) = text.split_at(36);
        json!({ "generator": generator, "key": &key[1..] })
    };
    match record {
        Record::Override { target, components } => {
            json!({ "derivation": derivation(target), "components": unpinned(components, read) })
        }
        Record::Suppress { target } => json!({ "derivation": derivation(target), "suppress": true }),
    }
}

/// The components with the pin dropped from every reference at every reference position, nested component maps
/// included.
fn unpinned(components: &Map<String, Value>, read: &TemplateRead) -> Map<String, Value> {
    let mut out = components.clone();
    references::rewrite(&mut out, &read.registry, &mut |position: Position<'_>| match position.value {
        Value::String(text) => match text.split_once('@') {
            Some((bare, _)) => Rewrite::Replace(Value::String(bare.to_owned())),
            None => Rewrite::Keep,
        },
        _ => Rewrite::Keep,
    });
    out
}
