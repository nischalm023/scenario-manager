//! Within `studio-protocol`, `flow` is spec 06 section 3.4's payloads: the flow read (`GET /api/flow/{*location}`),
//! a site's flow graph at one read with what the Flow panel shows of each entity, the plan it compiles to and its
//! problems; the parameter list a sweep chooses from (`GET /api/flow-parameters/{*location}`); and the patches
//! `POST /api/flow-validate/{*location}` compiles with. Every `*_sim_ms` is simtime in whole milliseconds; a field the
//! panel shows as the component wrote it travels as that JSON.

use std::collections::BTreeMap;

use desunits::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::Clock;
use crate::grammar::{EntityId, Pin, ScopePath, Seq, TagName, Ulid};
use crate::problem::ProblemItem;
use crate::projection::DraftState;

/// `GET /api/flow/{*location}`'s answer: the flow scope at the pin read, every scope the read resolved, the graph,
/// and the plan it compiles to without a scenario (null while a problem is an error, or the site has no flow).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowRead {
    pub scope: ScopePath,
    pub pin: Pin,
    pub seqs: BTreeMap<ScopePath, Seq>, // ordered: user-visible JSON
    pub exact: bool,
    /// The draft drawn over this read; one no longer open is named with its state, and the read is the location's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub draft: Option<FlowDraft>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub segments: Vec<FlowSegment>,
    pub return_rules: Vec<FlowReturnRule>,
    pub plan: Option<FlowPlan>,
    pub problems: Vec<ProblemItem>,
    /// The template the site was last made or updated from, which a template-made entity's made-from line names.
    pub made_from: Option<FlowMadeFrom>,
}

/// Where a made site's template-made values come from (spec 05 section 3.7's newest made-from record): the template,
/// its tag once tags land, and its root's changeset that act read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowMadeFrom {
    pub template: ScopePath,
    pub tag: Option<TagName>,
    pub seq: Option<Seq>,
}

/// A schematic place: its members as a count (the map lists them), each sector with what it contributes, and what
/// the members offer each port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowNode {
    #[serde(rename = "ref")]
    pub ref_: EntityId,
    pub name: String,
    pub kind: String,
    pub sectors: Vec<FlowSector>,
    pub members: u32,
    pub ports: BTreeMap<String, u32>, // ordered: user-visible JSON
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub process: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub holds: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub demand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub work: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub all_in: Option<FlowAllIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub intake: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deposit: Option<String>,
    pub made_from_template: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<FlowDiff>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowSector {
    #[serde(rename = "ref")]
    pub ref_: EntityId,
    pub name: Option<String>,
    pub members: u32,
}

/// The rate a station reaches with its tote swap added (lean 12), beside the figure `field` names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowAllIn {
    pub field: String,
    pub value: f64,
    pub unit: String,
    pub text: String,
}

/// One movement of one carrier state, its policy as written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowEdge {
    #[serde(rename = "ref")]
    pub ref_: EntityId,
    pub label: String,
    pub from: EntityId,
    pub to: EntityId,
    pub moves: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub when: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub alternative_to: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bias: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional)]
    pub opens_at_sim_ms: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional)]
    pub until_sim_ms: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub storage_scope: Option<Vec<EntityId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub returns_by: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub induction: Option<Value>,
    pub served_by: FlowServedBy,
    pub made_from_template: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<FlowDiff>,
}

/// What serves an edge in a run (spec 06 section 3.2's root inference): `root`, the root task `task`; `clearance`,
/// the clearance each finished presentation spawns, which returns the tote from its station to storage;
/// `completion`, no transport, as an order leaving in its bin; or `nothing`, no registered root, which the compile
/// refuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "flow.ts")]
pub enum FlowServedBy {
    Root { task: String },
    Clearance,
    Completion,
    Nothing,
}

/// One span of custody, or an empty move (`carries: "nothing"`, no edge or step).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowSegment {
    #[serde(rename = "ref")]
    pub ref_: EntityId,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub edge: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<u32>,
    pub from: FlowPlace,
    pub to: FlowPlace,
    pub executor: FlowExecutor,
    pub carries: String,
    pub at_end: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub retained_for: Option<Vec<EntityId>>,
    pub made_from_template: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<FlowDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowPlace {
    pub node: EntityId,
    pub port: String,
}

/// Who carries: `cohort` (with its reference and count), `assigned` (a kind of cohort, named schematically with no
/// reference or count, ruling D105), `conveyance` (the node whose belt carries) or `external`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowExecutor {
    pub kind: String,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ref_: Option<EntityId>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowReturnRule {
    #[serde(rename = "ref")]
    pub ref_: EntityId,
    pub label: String,
    pub min_backlog: u32,
    pub destination: String,
    pub made_from_template: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<FlowDiff>,
}

/// What the graph compiles to (spec 06 section 3.3): the plan's format and compiler versions, its digest, the setup
/// in run mode, each root task with what it waits for and its configuration, and the settings block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowPlan {
    pub v: u32,
    pub compiler: String,
    pub digest: String,
    pub setup: Option<FlowSetup>,
    pub roots: Vec<FlowRoot>,
    pub settings: Vec<FlowSetting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowRoot {
    #[serde(rename = "type")]
    pub type_: String,
    pub after: Vec<String>,
    pub config: Value,
}

/// One setting the plan states, with the flow entity and field it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowSetting {
    pub setting: String,
    pub value: Value,
    pub from: EntityId,
    pub field: String,
}

/// A scenario's setup (spec 06 section 3.2, run mode): the fraction of stock at the start and the order trays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowSetup {
    pub stock_at_start: f64,
    pub order_trays: u64,
}

/// `GET /api/flow-parameters/{*location}`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowParameters {
    pub seqs: BTreeMap<ScopePath, Seq>, // ordered: user-visible JSON
    pub parameters: Vec<FlowParameter>,
}

/// One scalar field of a flow entity: its words, value, unit and clock, whether a sweep may vary it, and a closed
/// field's options in the customer's words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowParameter {
    #[serde(rename = "ref")]
    pub ref_: EntityId,
    pub pointer: String,
    pub label: String,
    pub short_label: String,
    pub value: Value,
    pub unit: Option<String>,
    pub clock: Option<Clock>,
    pub sweepable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<FlowChoice>>", optional)]
    pub options: Vec<FlowChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub all_in: Option<FlowAllIn>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowChoice {
    pub value: String,
    pub label: String,
}

/// `POST /api/flow-validate/{*location}`'s body: patches applied in memory, and in run mode the setup and the order
/// kinds the workload holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowValidate {
    #[serde(default)]
    pub patches: Vec<FlowPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub setup: Option<FlowSetup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub order_kinds: Option<Vec<FlowOrderKind>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowPatch {
    pub target: EntityId,
    pub pointer: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowOrderKind {
    pub container: String,
    pub exit_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub routing: Option<String>,
}

/// `POST /api/flow-validate/{*location}`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowValidated {
    pub plan: Option<FlowPlan>,
    pub problems: Vec<ProblemItem>,
}

/// The draft a read names: its state and revision, and while it is drawn, the plan its stored cut compiles to without
/// it (null when that does not compile), which the footer compares the drawn plan against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowDraft {
    #[serde(rename = "ref")]
    pub ref_: Ulid,
    pub state: DraftState,
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base: Option<FlowBase>,
}

/// What a drawn draft's stored cut compiles to without the draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowBase {
    pub plan: Option<FlowPlan>,
}

/// What a draft does to one entity (`changed` while drafts hold patches only), with was and becomes for each field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowDiff {
    pub op: String,
    pub fields: Vec<FlowFieldDiff>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "flow.ts")]
pub struct FlowFieldDiff {
    pub pointer: String,
    pub was: Value,
    pub becomes: Value,
}
