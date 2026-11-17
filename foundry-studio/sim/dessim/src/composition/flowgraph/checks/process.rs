//! Within `checks`, `process` holds what nodes do to what edges bring (spec 06 section 3.2): `process-incomplete`
//! for a process missing a synchronization input, naming as one an edge that does not end at it or does not bring
//! the carrier its side takes, or lacking an edge for what its work leaves behind; `work-missing` for a required
//! figure, intake entry, induction or return rule that is absent; `alternative-invalid` for an alternative that is
//! not the direct-empty shape today's supply query runs.

use site_model::ids::EntityId;

use super::Ctx;
use crate::composition::flowgraph::components::{Container, Holds, Moves, NodeKind, Operation, Process};
use crate::composition::flowgraph::graph::{Edge, Node};
use crate::composition::flowgraph::label::moves as moved;
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::roles::{self, Root, Served};

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    for node in &ctx.graph.nodes {
        incomplete(ctx, node, problems);
        missing_work(ctx, node, problems);
    }
    for edge in &ctx.graph.edges {
        let served = roles::served_by(ctx.graph, edge);
        let missing = match served {
            Served::Root(Root::PutawayInduction) if edge.spec.induction.is_none() => {
                Some(("induction", "how units are inducted"))
            }
            Served::Clearance if edge.spec.returns_by.is_none() => Some(("returns_by", "its return rule")),
            _ => None,
        };
        if let Some((field, what)) = missing {
            let text = format!("{} states no {field}: {what}.", ctx.label(&edge.id));
            problems.push(ctx.error(Code::WorkMissing, &edge.id, &format!("/flow.material_edge/{field}"), text));
        }
        alternative(ctx, edge, problems);
    }
}

fn incomplete(ctx: &Ctx, node: &Node, problems: &mut Vec<Problem>) {
    let graph = ctx.graph;
    if !matches!(node.spec.kind, NodeKind::Stations | NodeKind::Conveyance) {
        return;
    }
    let Some(process) = &node.spec.process else {
        let text = format!("{} states no process, so nothing runs there.", ctx.label(&node.id));
        problems.push(ctx.error(Code::ProcessIncomplete, &node.id, "/flow.node/process", text));
        return;
    };
    let synchronized =
        !process.gives_by.is_empty() && (!process.receives_by.is_empty() || process.receives_in.is_some());
    if matches!(process.operation, Operation::Pick | Operation::Put) && !synchronized {
        let text = format!(
            "{}'s {} process needs both what units come out of (gives_by) and what they go into (receives_by or receives_in).",
            ctx.label(&node.id),
            process.operation.word()
        );
        problems.push(ctx.error(Code::ProcessIncomplete, &node.id, "/flow.node/process", text));
    }
    inputs(ctx, node, process, problems);
    let receives = |moves: Moves| graph.edges_into(&node.id).any(|edge| edge.spec.moves == moves);
    let sends = |moves: &[Moves]| graph.edges_out_of(&node.id).any(|edge| moves.contains(&edge.spec.moves));
    let returnable = graph.edges_into(&node.id).any(|edge| {
        edge.spec.moves == Moves::ClosedOrder
            && edge
                .spec
                .when
                .as_ref()
                .and_then(|when| when.container.as_ref())
                .is_none_or(|kinds| kinds.iter().any(|k| *k != Container::Carton))
    });
    let leaves = match process.operation {
        Operation::Pick if receives(Moves::StockTote) && !sends(&[Moves::ToteAfterPick, Moves::EmptyTote]) => {
            Some("the tote after the pick")
        }
        Operation::Pick
            if (receives(Moves::EmptyOrderContainer) || receives(Moves::NewOrderContainer))
                && !sends(&[Moves::ClosedOrder, Moves::OrderInBin]) =>
        {
            Some("the closed order")
        }
        Operation::Put if receives(Moves::EmptyTote) && !sends(&[Moves::StockTote]) => Some("the filled stock tote"),
        Operation::Extract if returnable && !sends(&[Moves::EmptyOrderContainer]) => {
            Some("the emptied order container")
        }
        _ => None,
    };
    if let Some(what) = leaves {
        let text = format!(
            "{} receives carriers and has no edge for what its work leaves behind: {what}.",
            ctx.label(&node.id)
        );
        problems.push(ctx.error(Code::ProcessIncomplete, &node.id, "/flow.node/process", text));
    }
}

/// The carriers a process's synchronization inputs bring (section 3.1): what units come out of (`gives_by`) and
/// what they go into (`receives_by`). The spec names none for extraction or box induction, whose inputs are held only
/// to ending at the node.
fn carriers(operation: Operation) -> Option<(&'static [Moves], &'static [Moves])> {
    match operation {
        Operation::Pick => Some((&[Moves::StockTote], &[Moves::EmptyOrderContainer, Moves::NewOrderContainer])),
        Operation::Put => Some((&[Moves::Units], &[Moves::EmptyTote])),
        Operation::Extract | Operation::CreateContainer => None,
    }
}

/// Each edge a process names as a synchronization input brings its carrier to this node; an edge naming nothing is
/// `ref-unresolved`'s.
fn inputs(ctx: &Ctx, node: &Node, process: &Process, problems: &mut Vec<Problem>) {
    let roles = carriers(process.operation);
    let sides = [
        ("gives_by", &process.gives_by, roles.map(|(gives, _)| gives), "takes units out of"),
        ("receives_by", &process.receives_by, roles.map(|(_, receives)| receives), "puts units into"),
    ];
    for (field, listed, wanted, does) in sides {
        for (at, reference) in listed.iter().enumerate() {
            let Some(edge) = ctx.graph.edge(&reference.id) else { continue };
            let fault = if edge.spec.to.id != node.id {
                format!("which ends at {}, not here", ctx.label(&edge.spec.to.id))
            } else if let Some(wanted) = wanted.filter(|wanted| !wanted.contains(&edge.spec.moves)) {
                let named: Vec<&str> = wanted.iter().map(|&carrier| moved(carrier)).collect();
                let operation = process.operation.word();
                format!(
                    "which moves {}, not what a {operation} {does} ({})",
                    moved(edge.spec.moves),
                    named.join(" or ")
                )
            } else {
                continue;
            };
            let text = format!("{}'s {field} names {}, {fault}.", ctx.label(&node.id), ctx.label(&edge.id));
            let pointer = format!("/flow.node/process/{field}/{at}");
            problems.push(ctx.error(Code::ProcessIncomplete, &node.id, &pointer, text));
        }
    }
}

/// The figures a consumer reads with no fallback (ruling D93), and the intake entries the admission laws read.
fn missing_work(ctx: &Ctx, node: &Node, problems: &mut Vec<Problem>) {
    let (spec, work) = (&node.spec, &node.spec.work);
    let mut absent = Vec::new();
    match (spec.kind, spec.operation()) {
        (_, Some(Operation::Pick)) if work.pick_sim_ms.is_none() => absent.push("/flow.node/work/pick_sim_ms"),
        (_, Some(Operation::Put)) if work.presentations_per_hour.is_none() => {
            absent.push("/flow.node/work/presentations_per_hour")
        }
        (_, Some(Operation::Extract)) if work.extraction_sim_ms.is_none() => {
            absent.push("/flow.node/work/extraction_sim_ms")
        }
        (NodeKind::Storage, _) if spec.holds.is_none() => absent.push("/flow.node/holds"),
        (NodeKind::Storage, _) if spec.holds == Some(Holds::ClosedOrder) && work.dwell_sim_ms.is_none() => {
            absent.push("/flow.node/work/dwell_sim_ms")
        }
        (NodeKind::Source, _) if spec.demand.is_none() => absent.push("/flow.node/demand"),
        _ => {}
    }
    let fed = |root: Root, moves: Moves| {
        ctx.graph
            .edges_into(&node.id)
            .any(|edge| edge.spec.moves == moves && roles::served_by(ctx.graph, edge) == Served::Root(root))
    };
    let admits = [
        (Root::PutawaySupplyDemand, Moves::EmptyTote),
        (Root::PickDemand, Moves::StockTote),
        (Root::FulfillOrders, Moves::StockTote),
    ];
    if admits.iter().any(|&(root, moves)| fed(root, moves) && spec.intake(moves).is_none()) {
        absent.push("/flow.node/intake");
    }
    for field in absent {
        let text =
            format!("{} needs {field}, which it does not state; no setting stands in for it.", ctx.label(&node.id));
        problems.push(ctx.error(Code::WorkMissing, &node.id, field, text));
    }
}

/// An alternative shares its primary's end and carrier and, in the proof of concept, is the direct-empty shape:
/// empty totes straight from every node where picks empty them, because today's supply query takes any HTM holding
/// an empty tote wherever it stands (`flows/ttp/demand/feed_putaway_stations.rs`).
fn alternative(ctx: &Ctx, edge: &Edge, problems: &mut Vec<Problem>) {
    let graph = ctx.graph;
    let spec = &edge.spec;
    let fault = match (&spec.alternative_to, spec.bias) {
        (None, Some(_)) => Some("states a bias with no edge it is an alternative to".to_owned()),
        (None, None) => None,
        (Some(primary), _) => match graph.edge(&primary.id) {
            None => None,
            Some(primary) if primary.spec.to.id != spec.to.id || primary.spec.moves != spec.moves => Some(format!(
                "is an alternative to {}, which does not bring the same carrier to the same node",
                ctx.label(&primary.id)
            )),
            Some(primary) => {
                let emptied: Vec<&EntityId> = (graph.nodes.iter())
                    .filter(|node| {
                        roles::is_pick(node) && graph.edges_into(&node.id).any(|e| e.spec.moves == Moves::StockTote)
                    })
                    .map(|node| &node.id)
                    .collect();
                let mut froms: Vec<&EntityId> = (graph.edges.iter())
                    .filter(|e| e.spec.alternative_to.as_ref().is_some_and(|p| p.id == primary.id))
                    .map(|e| &e.spec.from.id)
                    .collect();
                froms.sort();
                froms.dedup();
                let direct_empty = spec.moves == Moves::EmptyTote && froms == emptied;
                (!direct_empty).then(|| {
                    "is not the direct-empty shape: an empty-tote alternative from every node where picks empty totes".to_owned()
                })
            }
        },
    };
    if let Some(fault) = fault {
        let text = format!("{} {fault}.", ctx.label(&edge.id));
        problems.push(ctx.error(Code::AlternativeInvalid, &edge.id, "/flow.material_edge/alternative_to", text));
    }
}
