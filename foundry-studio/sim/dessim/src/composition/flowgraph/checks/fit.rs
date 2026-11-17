//! Within `checks`, `fit` refuses a valid graph the runner cannot execute as written (spec 06 section 3.2, lean
//! 10): `unsupported-value` for a value the chosen runner image or this build does not accept, `unsupported-pattern`
//! for a shape no registered root executes today; and warns `node-unused` for a node no edge touches.

use rustc_hash::FxHashMap;
use site_model::ids::EntityId;

use super::Ctx;
use crate::composition::flowgraph::components::{Holds, Moves, NodeKind, Operation, Phase};
use crate::composition::flowgraph::graph::Edge;
use crate::composition::flowgraph::label;
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::roles::{self, Served};

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    let graph = ctx.graph;
    for node in &graph.nodes {
        let work = &node.spec.work;
        if let Some(mode) = work.sequencing.as_ref().filter(|mode| !ctx.accepts.allows_sequencing(mode)) {
            let text = format!(
                "{} asks for {mode} sequencing, which the simulator image your runs use does not accept; it accepts {}.",
                ctx.label(&node.id),
                ctx.accepts.sequencing().join(" and ")
            );
            problems.push(ctx.error(Code::UnsupportedValue, &node.id, "/flow.node/work/sequencing", text));
        }
        for (field, stated) in [("fairness", work.fairness.is_some()), ("layer_size", work.layer_size.is_some())] {
            if stated {
                let text = format!(
                    "{} states {field}, which only the OrderPick sequencers read, and the simulator image your runs use does not carry them.",
                    ctx.label(&node.id)
                );
                problems.push(ctx.error(Code::UnsupportedValue, &node.id, &format!("/flow.node/work/{field}"), text));
            }
        }
        if matches!(node.spec.kind, NodeKind::Source | NodeKind::Sink) && !node.spec.sectors.is_empty() {
            let text = format!(
                "{} is a boundary and names sectors; a boundary stands for no place on the map.",
                ctx.label(&node.id)
            );
            problems.push(ctx.error(Code::UnsupportedPattern, &node.id, "/flow.node/sectors", text));
        }
        let belt_fed = ctx.resolved.node(&node.id).is_some_and(|resolved| {
            resolved
                .members
                .iter()
                .any(|member| ctx.site.entity(member).is_some_and(|entity| entity.has("go.easyloop")))
        });
        if roles::is_stations(node, Operation::Pick) && belt_fed {
            let text = format!(
                "{} is a group of belt-fed pick stations; LoopDeposit and LoopCollect are not inferred, so make it a conveyance node.",
                ctx.label(&node.id)
            );
            problems.push(ctx.error(Code::UnsupportedPattern, &node.id, "/flow.node/kind", text));
        }
        if !graph.edges.iter().any(|edge| edge.spec.from.id == node.id || edge.spec.to.id == node.id) {
            let text = format!("No edge touches {}.", ctx.label(&node.id));
            problems.push(Problem::warning(Code::NodeUnused, Some(&node.id), None, text));
        }
    }
    for edge in &graph.edges {
        if let Some(why) = unsupported(ctx, edge) {
            let text = format!("{} {why}.", ctx.label(&edge.id));
            problems.push(ctx.error(Code::UnsupportedPattern, &edge.id, "/flow.material_edge", text));
        }
    }
    two_operations(ctx, problems);
}

/// Why no registered root executes this edge as written, if none does.
fn unsupported(ctx: &Ctx, edge: &Edge) -> Option<String> {
    let graph = ctx.graph;
    let spec = &edge.spec;
    let (from, to) = graph.ends(edge)?;
    let served = roles::served_by(graph, edge);
    let leftover = spec.moves == Moves::EmptyTote
        && roles::is_stations(from, Operation::Put)
        && roles::is_storage(to, Holds::InventoryTote);
    let phased = spec.when.as_ref().is_some_and(|when| when.phase == Some(Phase::InboundEnded));
    if spec.moves == Moves::ClosedOrder && roles::is_stations(from, Operation::Pick) {
        Some("sends closed orders from pick stations; OrderCloseOut is not inferred".into())
    } else if served == Served::Nothing {
        Some(format!(
            "moves {} from a {} node to a {} node, which no registered root executes",
            label::moves(spec.moves),
            from.spec.kind.word(),
            to.spec.kind.word()
        ))
    } else if spec.storage_fallback.is_some() {
        Some(
            "states storage tiers beyond the first, which wait for the relay service to take an ordered storage scope"
                .into(),
        )
    } else if leftover != phased {
        Some("must return empty totes from put stations once inbound ends (phase inbound_ended), as a put station's clearance does, and only those".into())
    } else if spec.returns_by.is_some() && served != Served::Clearance {
        Some("names a return rule, which only edges from stations into storage share".into())
    } else if let Some(scope) = &spec.storage_scope {
        let store = [from, to].into_iter().find(|node| node.spec.kind == NodeKind::Storage)?;
        let mut stated: Vec<&EntityId> = scope.iter().map(|sector| &sector.id).collect();
        let mut whole: Vec<&EntityId> = store.spec.sectors.iter().map(|sector| &sector.id).collect();
        stated.sort();
        stated.dedup();
        whole.sort();
        whole.dedup();
        (stated != whole).then(|| {
            format!(
                "scopes {} to some of its sectors; a storage tier other than the whole node waits",
                ctx.label(&store.id)
            )
        })
    } else {
        None
    }
}

/// A station in two station-group nodes with different operations: dessim gives each station one type.
fn two_operations(ctx: &Ctx, problems: &mut Vec<Problem>) {
    let mut seen: FxHashMap<&EntityId, (Operation, &EntityId)> = FxHashMap::default();
    for node in ctx.graph.nodes.iter().filter(|node| node.spec.kind == NodeKind::Stations) {
        let (Some(operation), Some(resolved)) = (roles::operation(node), ctx.resolved.node(&node.id)) else { continue };
        let clash = resolved.members.iter().find_map(|member| match seen.get(member) {
            Some((other, by)) if *other != operation => Some((member, *by)),
            _ => None,
        });
        if let Some((member, by)) = clash {
            let text = format!(
                "{} runs {} at {}, which {} already runs another operation at; a station runs one operation in a run.",
                ctx.label(&node.id),
                operation.word(),
                ctx.name(member),
                ctx.label(by)
            );
            problems.push(ctx.error(Code::UnsupportedPattern, &node.id, "/flow.node/process/operation", text));
        }
        for member in &resolved.members {
            seen.entry(member).or_insert((operation, &node.id));
        }
    }
}
