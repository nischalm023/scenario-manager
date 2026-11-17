//! Within `studio-protocol`, `template_evaluation` is one evaluation of a template draft as
//! `GET /api/template-drafts/{d}/evaluations/{n}` answers it (spec 05 section 3.6): its prologue, the engine and
//! generators it ran with, its census, what the knobs added or left out, its problems and notes, and its floor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::WallClock;
use crate::grammar::{EntityId, Ulid};
use crate::map::FrameAxes;
use crate::template::{Census, EvaluationNote, EvaluationProblem, KnobValue};
use crate::template_draft::{DraftMode, EvaluationNumber, EvaluationState, TemplateAt};

/// Where an evaluation took a knob's value from: set by a person or the agent, the template's integer default, or
/// derived from its default expression at the other knobs (D120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "template_evaluation.ts")]
pub enum KnobSource {
    Set,
    Default,
    Derived,
}

/// An evaluation's document prologue: the draft's document id, its kind, "{title}, evaluation {n}", and the made id
/// of the template's primary frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct EvaluationPrologue {
    pub doc: String,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub primary_frame: Option<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct GeneratorUsed {
    pub name: String,
    pub version: String,
    pub digest: String,
}

/// Which engine made an evaluation, with the registry versions and generators it read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct EvaluationEngine {
    pub version: String,
    pub expansion: String,
    // ordered: component versions are listed by name.
    pub registry: BTreeMap<String, String>,
    pub generators: Vec<GeneratorUsed>,
}

/// One made entity a knob change added or left out against the template's defaults: its source key, its made id
/// (null for one left out) and its label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct KeyChange {
    pub key: String,
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct KeyChanges {
    pub added: Vec<KeyChange>,
    pub removed: Vec<KeyChange>,
}

/// Where an evaluation's compiled floor is kept, by its key in the run-object store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct FloorRef {
    pub key: String,
    pub floor_version: u32,
    pub bytes: u64,
}

/// `GET /api/template-drafts/{d}/evaluations/{n}`'s answer (section 3.6), its floor drawn by the page's viewer.
/// `knobs` holds every knob's value once the evaluation has ended, the values set until then, and `sources` where
/// each came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_evaluation.ts")]
pub struct TemplateEvaluation {
    pub draft: Ulid,
    pub n: EvaluationNumber,
    pub state: EvaluationState,
    pub mode: DraftMode,
    pub revision: u32,
    // ordered: knobs are echoed in key order.
    pub knobs: BTreeMap<String, KnobValue>,
    pub sources: BTreeMap<String, KnobSource>,
    pub template: TemplateAt,
    pub prologue: Option<EvaluationPrologue>,
    pub engine: Option<EvaluationEngine>,
    pub census: Option<Census>,
    pub changes_from_template: Option<KeyChanges>,
    pub problems: Vec<EvaluationProblem>,
    pub notes: Vec<EvaluationNote>,
    pub diff: Option<Value>,
    pub floor: Option<FloorRef>,
    /// The frame the template's wall knobs name and where its axes point, which the stage draws in its corner and its
    /// text says (spec 05 section 6); none for an evaluation that made no floor, or one stored before it was kept.
    pub axes: Option<FrameAxes>,
    pub started_at: WallClock,
    pub finished_at: Option<WallClock>,
}
