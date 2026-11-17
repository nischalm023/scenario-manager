//! Within `flowgraph`, the `plan` is stage 5 of spec 06 section 3.2 and section 3.3's flow plan: what a run starts
//! with (the composition and the settings block) beside the resolved flows, the read's provenance and the versions.
//! It carries no digest: a caller that compares plans hashes the composition and the settings as the runner writes
//! them into its `spec.json` (`settings_object`), which neither a label, a display-only figure, the read's provenance
//! nor the compiler version reaches.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
use site_model::ids::EntityId;

use super::components::{
    AtEnd, Carries, Demand, Destination, EDGE, EdgeSpec, End, Holds, NODE, NodeKind, Operation, Place, Port, RULE,
    SCHEMA, SEGMENT,
};
use super::graph::{Graph, Segment};
use super::quantity::{Backlog, Step};
use super::resolve::{Resolved, ResolvedExecutor};
use super::roots::Inference;
use super::{COMPILER, Changeset, Mode, Patch, Request, RunSetup};
use crate::composition::Definition;

/// The plan's format, which the check and the runner both produce.
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub v: u32,
    pub compiler: String,
    pub schemas: BTreeMap<String, String>, // ordered: the plan's bytes
    pub seqs: BTreeMap<String, Changeset>, // ordered: the plan's bytes
    pub exact: bool,
    pub variant: Vec<Patch>,
    pub setup: Option<RunSetup>,
    pub composition: Definition,
    pub settings: Vec<SettingEntry>,
    pub flows: PlanFlows,
}

/// One setting a value the run also reads elsewhere compiles to, and the flow entity and field it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingEntry {
    pub setting: String,
    pub value: Value,
    pub from: EntityId,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanFlows {
    pub nodes: Vec<PlanNode>,
    pub edges: Vec<PlanEdge>,
    pub empty_moves: Vec<PlanSegment>,
    pub return_rules: Vec<PlanRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanNode {
    #[serde(rename = "ref")]
    pub reference: EntityId,
    pub kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holds: Option<Holds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demand: Option<Demand>,
    pub members: Vec<EntityId>,
    pub ports: BTreeMap<Port, u32>, // ordered: the plan's bytes
}

/// An edge with its policy as written and its segments in step order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanEdge {
    #[serde(rename = "ref")]
    pub reference: EntityId,
    #[serde(flatten)]
    pub policy: EdgeSpec,
    pub segments: Vec<PlanSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanSegment {
    #[serde(rename = "ref")]
    pub reference: EntityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<Step>,
    pub from: Place,
    pub to: Place,
    pub executor: PlanExecutor,
    pub carries: Carries,
    pub at_end: AtEnd,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retained_for: Vec<EntityId>,
}

/// Who carries, by cohort identity: an `assigned` executor names, per member of its place, the cohorts the fleet
/// hard-assigns there (ruling D105).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanExecutor {
    Cohort { cohort: EntityId },
    Assigned { at: End, by_member: Vec<MemberCohorts> },
    Conveyance { node: EntityId },
    External,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemberCohorts {
    pub member: EntityId,
    pub cohorts: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanRule {
    #[serde(rename = "ref")]
    pub reference: EntityId,
    pub min_backlog: Backlog,
    pub destination: Destination,
}

/// The settings block as the runner writes it into `spec.json`'s `settings`: each dotted name nested.
pub fn settings_object(settings: &[SettingEntry]) -> Value {
    let mut root = Map::new();
    for entry in settings {
        let mut at = &mut root;
        let mut parts = entry.setting.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                at.insert(part.to_owned(), entry.value.clone());
            } else {
                let next = at.entry(part).or_insert_with(|| Value::Object(Map::new()));
                at = next.as_object_mut().expect("a setting's prefix names a group, never a value");
            }
        }
    }
    Value::Object(root)
}

fn segment(graph: &Graph, resolved: &Resolved, segment: &Segment) -> PlanSegment {
    let at = graph.segments.binary_search_by(|s| s.id.cmp(&segment.id)).expect("a segment of the graph");
    let executor = match &resolved.executors[at] {
        ResolvedExecutor::Cohort(cohort) => PlanExecutor::Cohort { cohort: cohort.clone() },
        ResolvedExecutor::Assigned { at, by_member } => PlanExecutor::Assigned {
            at: *at,
            by_member: by_member
                .iter()
                .map(|(member, cohorts)| MemberCohorts { member: member.clone(), cohorts: cohorts.clone() })
                .collect(),
        },
        ResolvedExecutor::Conveyance(node) => PlanExecutor::Conveyance { node: node.clone() },
        ResolvedExecutor::External => PlanExecutor::External,
    };
    let spec = &segment.spec;
    PlanSegment {
        reference: segment.id.clone(),
        step: spec.step,
        from: spec.from.clone(),
        to: spec.to.clone(),
        executor,
        carries: spec.carries,
        at_end: spec.at_end,
        retained_for: spec.retained_for.iter().map(|next| next.id.clone()).collect(),
    }
}

pub(super) fn emit(graph: &Graph, resolved: &Resolved, inference: Inference, request: &Request) -> Plan {
    let nodes = (graph.nodes.iter().zip(&resolved.nodes))
        .map(|(node, resolved)| PlanNode {
            reference: node.id.clone(),
            kind: node.spec.kind,
            operation: node.spec.operation(),
            holds: node.spec.holds,
            demand: node.spec.demand,
            members: resolved.members.clone(),
            ports: resolved.ports.clone(),
        })
        .collect();
    let edges = (graph.edges.iter())
        .map(|edge| PlanEdge {
            reference: edge.id.clone(),
            policy: edge.spec.clone(),
            segments: graph.chain(&edge.id).into_iter().map(|s| segment(graph, resolved, s)).collect(),
        })
        .collect();
    let empty_moves =
        (graph.segments.iter()).filter(|s| s.spec.is_empty_move()).map(|s| segment(graph, resolved, s)).collect();
    let return_rules = (graph.rules.iter())
        .map(|rule| PlanRule {
            reference: rule.id.clone(),
            min_backlog: rule.spec.min_backlog,
            destination: rule.spec.destination,
        })
        .collect();
    let schemas = [NODE, EDGE, SEGMENT, RULE]
        .into_iter()
        .map(|component| {
            (component.to_owned(), request.schemas.get(component).map_or(SCHEMA, String::as_str).to_owned())
        })
        .collect();
    Plan {
        v: FORMAT,
        compiler: COMPILER.to_owned(),
        schemas,
        seqs: request.seqs.clone(),
        exact: request.exact,
        variant: request.variant.clone(),
        setup: match &request.mode {
            Mode::Run { setup, .. } => Some(*setup),
            Mode::Validation => None,
        },
        composition: inference.definition,
        settings: inference.settings,
        flows: PlanFlows { nodes, edges, empty_moves, return_rules },
    }
}
