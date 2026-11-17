//! Within `flowgraph`, the `graph` is stage 1 of spec 06 section 3.2, the read: the four `flow.` components typed
//! into nodes, material edges, agent segments and return rules, each list in identity order. A component the
//! compiler cannot read whole (a schema version it does not know, a field it does not define, a value of the wrong
//! shape) is refused as `schema-unknown` naming its entity: a field carried unread would be a run that quietly does
//! something other than what the site says.

use std::collections::BTreeMap;

use rustc_hash::FxHashSet;
use serde::Deserialize;
use site_model::ids::EntityId;

use super::components::{EDGE, EdgeSpec, NODE, NodeSpec, RULE, RuleSpec, SCHEMA, SEGMENT, SegmentSpec};
use super::problem::{Code, Problem};
use crate::worldmap::flows::{FlowEntity, FlowScope};

/// One flow entity, typed: its identity, its label as written, and its component.
#[derive(Debug, Clone, PartialEq)]
pub struct Item<T> {
    pub id: EntityId,
    pub name: Option<String>,
    pub spec: T,
}

pub type Node = Item<NodeSpec>;
pub type Edge = Item<EdgeSpec>;
pub type Segment = Item<SegmentSpec>;
pub type ReturnRule = Item<RuleSpec>;

/// The typed flow sub-scope, each list in identity order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub segments: Vec<Segment>,
    pub rules: Vec<ReturnRule>,
    /// The entities refused as `schema-unknown`, which later checks leave to that problem.
    pub unread: FxHashSet<EntityId>,
}

fn find<'a, T>(items: &'a [Item<T>], id: &EntityId) -> Option<&'a Item<T>> {
    items.binary_search_by(|item| item.id.cmp(id)).ok().map(|at| &items[at])
}

impl Graph {
    pub fn node(&self, id: &EntityId) -> Option<&Node> {
        find(&self.nodes, id)
    }

    pub fn edge(&self, id: &EntityId) -> Option<&Edge> {
        find(&self.edges, id)
    }

    pub fn segment(&self, id: &EntityId) -> Option<&Segment> {
        find(&self.segments, id)
    }

    pub fn rule(&self, id: &EntityId) -> Option<&ReturnRule> {
        find(&self.rules, id)
    }

    pub fn edges_into(&self, node: &EntityId) -> impl Iterator<Item = &Edge> + '_ {
        let node = node.clone();
        self.edges.iter().filter(move |edge| edge.spec.to.id == node)
    }

    pub fn edges_out_of(&self, node: &EntityId) -> impl Iterator<Item = &Edge> + '_ {
        let node = node.clone();
        self.edges.iter().filter(move |edge| edge.spec.from.id == node)
    }

    /// The segments that carry `edge`, in step order.
    pub fn chain(&self, edge: &EntityId) -> Vec<&Segment> {
        let mut chain: Vec<&Segment> =
            self.segments.iter().filter(|segment| segment.spec.edge.as_ref().is_some_and(|e| &e.id == edge)).collect();
        chain.sort_by_key(|segment| (segment.spec.step, segment.id.clone()));
        chain
    }

    /// The node at either end of an edge, when the reference resolves to one.
    pub fn ends(&self, edge: &Edge) -> Option<(&Node, &Node)> {
        Some((self.node(&edge.spec.from.id)?, self.node(&edge.spec.to.id)?))
    }
}

fn typed<T>(entity: &FlowEntity, spec: T) -> Item<T> {
    Item { id: entity.id.clone(), name: entity.name.clone(), spec }
}

/// Whether this compiler reads a component at `version`: the same major, and for a 0.x schema the same minor.
fn known(version: &str) -> bool {
    let mut parts = version.split('.');
    matches!((parts.next(), parts.next()), (Some("0"), Some("1")))
}

/// Type every flow component of `flows`; `schemas` holds the registry version each was read at, where the caller
/// knows it (an absent name reads as this compiler's own).
pub(super) fn read(flows: &FlowScope, schemas: &BTreeMap<String, String>, problems: &mut Vec<Problem>) -> Graph {
    let mut graph = Graph::default();
    for entity in &flows.entities {
        let label = entity.name.clone().unwrap_or_else(|| entity.id.to_string());
        for (component, value) in entity.flow_components() {
            let refuse = |why: String| {
                let text = format!("{label}'s {component} cannot be read: {why}");
                Problem::error(Code::SchemaUnknown, &entity.id, Some(format!("/{component}")), text)
            };
            if let Some(version) = schemas.get(component).filter(|version| !known(version)) {
                problems.push(refuse(format!("schema {version} is not one this compiler reads (it reads {SCHEMA})")));
                graph.unread.insert(entity.id.clone());
                continue;
            }
            let read = match component {
                NODE => NodeSpec::deserialize(value).map(|spec| graph.nodes.push(typed(entity, spec))),
                EDGE => EdgeSpec::deserialize(value).map(|spec| graph.edges.push(typed(entity, spec))),
                SEGMENT => SegmentSpec::deserialize(value).map(|spec| graph.segments.push(typed(entity, spec))),
                RULE => RuleSpec::deserialize(value).map(|spec| graph.rules.push(typed(entity, spec))),
                _ => {
                    problems.push(refuse("it is not one of the four flow components".into()));
                    graph.unread.insert(entity.id.clone());
                    continue;
                }
            };
            if let Err(fault) = read {
                problems.push(refuse(fault.to_string()));
                graph.unread.insert(entity.id.clone());
            }
        }
    }
    graph
}
