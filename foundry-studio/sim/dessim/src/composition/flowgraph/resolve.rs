//! Within `flowgraph`, `resolve` is stage 2 of spec 06 section 3.2: each node's members, the union of its sectors'
//! members without repeats in identity order, a member that is itself a sector giving its own members one level
//! deep (lead decided, say if not); what those members offer each port on the map; and each segment's executor
//! resolved to cohorts, an `assigned` one per member of its place, so the plan names the per-aisle cohorts and the
//! flow names none (ruling D105).

use std::collections::BTreeMap;

use site_model::ids::EntityId;

use super::components::{End, Executor, NodeKind, Port};
use super::graph::Graph;
use super::problem::{Code, Problem};
use super::site::Site;

/// A node as the map resolves it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    pub id: EntityId,
    pub members: Vec<EntityId>,
    /// Each sector the node names, in its order, with the members it contributes.
    pub by_sector: Vec<(EntityId, Vec<EntityId>)>,
    pub ports: BTreeMap<Port, u32>, // ordered: the plan's JSON
}

/// Who carries a segment, resolved against the fleet.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedExecutor {
    Cohort(EntityId),
    /// Per member of the place's node, the hard-assigned cohorts that reach it.
    Assigned {
        at: End,
        by_member: Vec<(EntityId, Vec<EntityId>)>,
    },
    Conveyance(EntityId),
    External,
}

/// Stage 2's result, aligned with the graph: `nodes[i]` resolves `graph.nodes[i]`, `executors[i]` the executor of
/// `graph.segments[i]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub nodes: Vec<ResolvedNode>,
    pub executors: Vec<ResolvedExecutor>,
}

impl Resolved {
    pub fn node(&self, id: &EntityId) -> Option<&ResolvedNode> {
        self.nodes.binary_search_by(|node| node.id.cmp(id)).ok().map(|at| &self.nodes[at])
    }
}

pub(super) fn resolve(graph: &Graph, site: &Site, problems: &mut Vec<Problem>) -> Resolved {
    let nodes: Vec<ResolvedNode> = graph
        .nodes
        .iter()
        .map(|node| {
            let mut by_sector = Vec::new();
            for sector in node.spec.sectors.iter().filter(|sector| sector.at_head()) {
                let Some(direct) = site.sector(&sector.id) else { continue };
                let mut contributed = Vec::new();
                for member in direct {
                    match site.sector(&member) {
                        Some(inner) => contributed.extend(inner),
                        None => contributed.push(member),
                    }
                }
                contributed.sort();
                contributed.dedup();
                let (deep, kept): (Vec<EntityId>, Vec<EntityId>) =
                    contributed.into_iter().partition(|member| site.sector(member).is_some());
                if let Some(deep) = deep.first() {
                    let text = format!(
                        "{}'s sector {} reaches the sector {deep} two levels down; a node's sectors are read one \
                         level deep",
                        node.name.as_deref().unwrap_or("A node"),
                        site.name(&sector.id).unwrap_or("(unnamed)"),
                    );
                    problems.push(Problem::error(Code::MemberUnfit, &node.id, Some("/flow.node/sectors".into()), text));
                }
                by_sector.push((sector.id.clone(), kept));
            }
            let mut members: Vec<EntityId> =
                by_sector.iter().flat_map(|(_, members)| members.iter().cloned()).collect();
            members.sort();
            members.dedup();
            let ports = ports(node.spec.kind, &members, site);
            ResolvedNode { id: node.id.clone(), members, by_sector, ports }
        })
        .collect();
    let mut resolved = Resolved { nodes, executors: Vec::new() };
    resolved.executors = graph
        .segments
        .iter()
        .map(|segment| match &segment.spec.executor {
            Executor::Cohort(cohort) => ResolvedExecutor::Cohort(cohort.id.clone()),
            Executor::Conveyance(node) => ResolvedExecutor::Conveyance(node.id.clone()),
            Executor::External => ResolvedExecutor::External,
            Executor::Assigned(at) => {
                let place = match at {
                    End::From => &segment.spec.from,
                    End::To => &segment.spec.to,
                };
                let members = resolved.node(&place.node.id).map(|node| node.members.as_slice()).unwrap_or(&[]);
                let by_member = members
                    .iter()
                    .map(|member| (member.clone(), site.assigned_to(member).iter().map(|c| c.id.clone()).collect()))
                    .collect();
                ResolvedExecutor::Assigned { at: *at, by_member }
            }
        })
        .collect();
    resolved
}

/// What a node's members offer each of its ports on the map: presentation points naming a station; slots and
/// bound relays on a storage member's bays; a machine's sink and source ports and its order presentations.
fn ports(kind: NodeKind, members: &[EntityId], site: &Site) -> BTreeMap<Port, u32> {
    let mut ports = BTreeMap::new();
    let sum = |count: &dyn Fn(&EntityId) -> u32| members.iter().map(count).sum::<u32>();
    match kind {
        NodeKind::Stations => {
            ports.insert(Port::Presentation, sum(&|member| site.presentations(member).len() as u32));
        }
        NodeKind::Storage => {
            let bays: Vec<EntityId> = members.iter().flat_map(|member| site.bays_of(member)).collect();
            ports.insert(Port::Slot, bays.iter().map(|bay| site.slots_on(bay)).sum());
            ports.insert(Port::Relay, bays.iter().map(|bay| site.relays_on(bay)).sum());
        }
        NodeKind::Conveyance => {
            ports.insert(Port::Dropoff, sum(&|member| site.machine(member).map_or(0, |m| m.sinks)));
            ports.insert(Port::Collect, sum(&|member| site.machine(member).map_or(0, |m| m.sources)));
            ports.insert(
                Port::OrderPresentation,
                sum(&|member| site.machine(member).map_or(0, |m| m.order_presentations)),
            );
        }
        NodeKind::Source | NodeKind::Sink => {}
    }
    ports
}
