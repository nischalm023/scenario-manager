//! Within `checks`, `guards` is `guard-gap` and `guard-overlap` (spec 06 section 3.2): the closed-order edges
//! leaving a node where orders close (a pick conveyance node) or rest (closed-order storage) must split the order
//! kinds that reach it exclusively, and leave none with nowhere to go, as FulfillOrders refuses an order with no
//! handoff today (`flows/fulfillment/outbound.rs`, `for_order`). With a scenario the kinds are its demand's and a gap
//! is an error; without one they are every container and exit path, and a gap is a warning.

use super::Ctx;
use crate::composition::flowgraph::components::{Container, Guard, Holds, Moves};
use crate::composition::flowgraph::graph::{Edge, Node};
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::roles;
use crate::composition::flowgraph::{Mode, OrderKind};
use crate::model::ExitPath;

/// Whether a guard admits an order kind; an absent field admits all.
pub(in crate::composition::flowgraph) fn admits(guard: Option<&Guard>, kind: &OrderKind) -> bool {
    let Some(guard) = guard else { return true };
    guard.container.as_ref().is_none_or(|containers| containers.contains(&kind.container))
        && guard.exit_path.as_ref().is_none_or(|exits| exits.contains(&kind.exit_path))
        && guard.routing.as_ref().is_none_or(|routings| kind.routing.as_ref().is_some_and(|r| routings.contains(r)))
}

/// The kinds a node must route: in run mode the demand's, else every container on every exit path.
pub(in crate::composition::flowgraph) fn universe(mode: &Mode) -> Vec<OrderKind> {
    match mode {
        Mode::Run { order_kinds, .. } => order_kinds.clone(),
        Mode::Validation => [Container::Tote, Container::Tray, Container::Carton]
            .into_iter()
            .flat_map(|container| {
                [ExitPath::Buffer, ExitPath::Sorter, ExitPath::Dock].map(|exit_path| OrderKind {
                    container,
                    exit_path,
                    routing: None,
                })
            })
            .collect(),
    }
}

pub(in crate::composition::flowgraph) fn words(kind: &OrderKind) -> String {
    let routing = kind.routing.as_ref().map(|r| format!(" routed {r}")).unwrap_or_default();
    let (container, exit) = (format!("{:?}", kind.container), format!("{:?}", kind.exit_path));
    format!("{} orders leaving by {}{routing}", container.to_lowercase(), exit.to_lowercase())
}

fn closed(edge: &Edge) -> bool {
    edge.spec.moves == Moves::ClosedOrder
}

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    let graph = ctx.graph;
    let all = universe(ctx.mode);
    for node in &graph.nodes {
        let reaching: Vec<&OrderKind> = if roles::is_pick_loop(node) {
            all.iter().collect()
        } else if roles::is_storage(node, Holds::ClosedOrder) {
            let incoming: Vec<&Edge> = graph.edges_into(&node.id).filter(|edge| closed(edge)).collect();
            all.iter().filter(|kind| incoming.iter().any(|edge| admits(edge.spec.when.as_ref(), kind))).collect()
        } else {
            continue;
        };
        route(ctx, node, &reaching, problems);
    }
}

fn route(ctx: &Ctx, node: &Node, reaching: &[&OrderKind], problems: &mut Vec<Problem>) {
    let leaving: Vec<&Edge> = ctx.graph.edges_out_of(&node.id).filter(|edge| closed(edge)).collect();
    let taking = |kind: &OrderKind| leaving.iter().filter(|edge| admits(edge.spec.when.as_ref(), kind)).count();
    let stranded: Vec<String> = reaching.iter().filter(|kind| taking(kind) == 0).map(|kind| words(kind)).collect();
    let doubled: Vec<String> = reaching.iter().filter(|kind| taking(kind) > 1).map(|kind| words(kind)).collect();
    let field = "/flow.node/process";
    if !stranded.is_empty() {
        let text =
            format!("{}'s closed-order edges leave {} with nowhere to go.", ctx.label(&node.id), stranded.join(", "));
        problems.push(match ctx.mode {
            Mode::Run { .. } => ctx.error(Code::GuardGap, &node.id, field, text),
            Mode::Validation => Problem::warning(Code::GuardGap, Some(&node.id), Some(field.into()), text),
        });
    }
    if !doubled.is_empty() {
        let text = format!(
            "{}'s closed-order edges each take {}; one edge must take each kind.",
            ctx.label(&node.id),
            doubled.join(", ")
        );
        problems.push(ctx.error(Code::GuardOverlap, &node.id, field, text));
    }
}
