//! Within `checks`, `readiness` refuses what a consumer that reads no flows yet would do differently from the graph
//! (spec 06 section 3.2): `consumer-not-ready` for a node holding less than every station, machine or racking of
//! its use while its consumer still takes them all, a closed-order routing other than today's fixed one, or a value
//! no consumer reads where it is stated; and `consumer-conflict` for one held value stated twice differently. Each
//! sentence names the consumer, so the refusal goes away the day it reads its flows.

use site_model::ids::EntityId;

use super::Ctx;
use super::guards::{admits, universe, words};
use crate::composition::flowgraph::components::{Container, Deposit, Holds, Moves, NodeKind, Operation};
use crate::composition::flowgraph::graph::{Edge, Graph, Node};
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::roles::{self, Root, Served};
use crate::model::ExitPath;
use crate::scenario_gen::RackUse;

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    for node in &ctx.graph.nodes {
        if let Some(text) = partial(ctx, node) {
            problems.push(ctx.error(Code::ConsumerNotReady, &node.id, "/flow.node/sectors", text));
        }
        for field in unread_node_fields(node) {
            let text = format!(
                "{} states {field}, which nothing the run starts reads on a node like it; the run would ignore it.",
                ctx.label(&node.id)
            );
            problems.push(ctx.error(Code::ConsumerNotReady, &node.id, &field, text));
        }
        if roles::is_pick_loop(node) {
            routing(ctx, node, problems);
        }
    }
    for edge in &ctx.graph.edges {
        let served = roles::served_by(ctx.graph, edge);
        let pick_feed = matches!(served, Served::Root(Root::PickDemand))
            || served == Served::Root(Root::FulfillOrders) && edge.spec.moves == Moves::StockTote;
        let rack_supply = served == Served::Root(Root::PutawaySupplyDemand) && edge.spec.alternative_to.is_none();
        for (field, stated, read) in [
            ("opens_at_sim_ms", edge.spec.opens_at_sim_ms.is_some(), pick_feed),
            ("until_sim_ms", edge.spec.until_sim_ms.is_some(), rack_supply),
        ] {
            if stated && !read {
                let text = format!(
                    "{} states {field}, which PickDemand, FulfillOrders and PutawaySupplyDemand read only on the edges that feed them from storage; the run would ignore it.",
                    ctx.label(&edge.id)
                );
                problems.push(ctx.error(
                    Code::ConsumerNotReady,
                    &edge.id,
                    &format!("/flow.material_edge/{field}"),
                    text,
                ));
            }
        }
    }
    problems.extend(ctx.consumers.conflicts(ctx.labels));
}

/// A node holding some, not all, of what its consumer takes: every station of an operation (and of an extraction
/// role), every belt machine, every bay the run seeds for one use (`PutawaySupply` takes the putaway stations;
/// FillInventory seeds racking by use).
fn partial(ctx: &Ctx, node: &Node) -> Option<String> {
    let site = ctx.site;
    let members = ctx.resolved.node(&node.id).map(|node| &node.members).filter(|members| !members.is_empty())?;
    let (held, every, what, consumer): (Vec<EntityId>, Vec<EntityId>, String, &str) =
        match (node.spec.kind, roles::operation(node)) {
            (NodeKind::Stations, Some(operation)) => {
                let (kinds, consumer): (&[&str], &str) = match operation {
                    Operation::Put => (&["put", "putaway"], "PutawayInduction and PutawaySupplyDemand"),
                    Operation::Pick => (&["pick"], "PickDemand"),
                    Operation::Extract => (&["extract"], "FulfillOrders' outbound handoffs"),
                    Operation::CreateContainer => (&["induct"], "FulfillOrders' box induction"),
                };
                let mut roles: Vec<&str> = members.iter().filter_map(|member| site.station(member)?.1).collect();
                roles.sort();
                roles.dedup();
                let every = match (operation, roles.as_slice()) {
                    (Operation::Extract, [role]) => site.stations(kinds, Some(role)),
                    _ => site.stations(kinds, None),
                };
                (members.clone(), every, format!("{} stations", operation.word()), consumer)
            }
            (NodeKind::Conveyance, Some(Operation::Pick)) => {
                (members.clone(), site.machines().to_vec(), "belt machines".into(), "FulfillOrders")
            }
            (NodeKind::Storage, _) => {
                let uses = match node.spec.holds? {
                    Holds::InventoryTote => RackUse::Inventory,
                    Holds::OrderContainer => RackUse::OrderDepot,
                    Holds::ClosedOrder => RackUse::OrderBuffer,
                };
                let mut bays: Vec<EntityId> = members
                    .iter()
                    .flat_map(|member| site.bays_of(member))
                    .filter(|bay| site.bay_use(bay).is_some())
                    .collect();
                bays.sort();
                bays.dedup();
                (
                    bays,
                    site.bays_used_as(uses),
                    format!(
                        "bays the run seeds for {}",
                        match uses {
                            RackUse::Inventory => "inventory",
                            RackUse::OrderDepot => "the empty order totes",
                            RackUse::OrderBuffer => "the order buffer",
                        }
                    ),
                    "FillInventory and the relay service",
                )
            }
            _ => return None,
        };
    (held != every).then(|| {
        format!(
            "{} holds {} of the {} {what} on the map; {consumer} still takes every one until it reads its flows.",
            ctx.label(&node.id),
            held.iter().filter(|id| every.contains(id)).count(),
            every.len()
        )
    })
}

/// The work fields and intake entries a node states that no consumer reads on a node of its kind and operation.
fn unread_node_fields(node: &Node) -> Vec<String> {
    let (spec, work) = (&node.spec, &node.spec.work);
    let pick_stations = roles::is_stations(node, Operation::Pick);
    let mut unread = Vec::new();
    for (field, stated, read) in [
        ("pick_sim_ms", work.pick_sim_ms.is_some(), roles::is_pick(node)),
        ("pallet_swap_sim_ms", work.pallet_swap_sim_ms.is_some(), pick_stations),
        ("sequencing", work.sequencing.is_some(), pick_stations),
        ("pace_to_station", work.pace_to_station.is_some(), pick_stations),
        ("presentations_per_hour", work.presentations_per_hour.is_some(), roles::is_stations(node, Operation::Put)),
        ("extraction_sim_ms", work.extraction_sim_ms.is_some(), roles::is_stations(node, Operation::Extract)),
        (
            "isolated",
            work.isolated.is_some(),
            matches!(spec.operation(), Some(Operation::Extract | Operation::CreateContainer)),
        ),
        ("dwell_sim_ms", work.dwell_sim_ms.is_some(), roles::is_storage(node, Holds::ClosedOrder)),
        ("max_open_orders", work.max_open_orders.is_some(), roles::is_pick_loop(node)),
    ] {
        if stated && !read {
            unread.push(format!("/flow.node/work/{field}"));
        }
    }
    for (at, entry) in spec.intake.iter().enumerate() {
        let read = match entry.moves {
            Moves::EmptyTote => roles::is_stations(node, Operation::Put),
            Moves::StockTote => roles::is_pick(node),
            _ => false,
        };
        if !read || entry.max_inflight.is_some() && !pick_stations {
            unread.push(format!("/flow.node/intake/{at}"));
        }
    }
    if spec.deposit == Some(Deposit::Random) && !roles::is_storage(node, Holds::InventoryTote) {
        unread.push("/flow.node/deposit".into());
    }
    unread
}

fn closed_out_of<'g>(graph: &'g Graph, from: &EntityId) -> Vec<&'g Edge> {
    graph.edges_out_of(from).filter(|edge| edge.spec.moves == Moves::ClosedOrder).collect()
}

/// Today's fixed closed-order routing (`flows/fulfillment/outbound.rs` `role_for`, and the buffer decision in
/// `flows/fulfillment/orders.rs`): through the buffer exactly when the exit path is the buffer and a buffer edge
/// exists, then to sealing for a carton, flex for a tray, route dispense for a buffered tote and bagging otherwise.
fn routing(ctx: &Ctx, node: &Node, problems: &mut Vec<Problem>) {
    let graph = ctx.graph;
    let closed = |from: &EntityId| closed_out_of(graph, from);
    let buffers = closed(&node.id)
        .iter()
        .any(|edge| graph.node(&edge.spec.to.id).is_some_and(|to| roles::is_storage(to, Holds::ClosedOrder)));
    let routed = graph.edges.iter().any(|edge| edge.spec.when.as_ref().is_some_and(|when| when.routing.is_some()));
    let mut differ = Vec::new();
    for kind in universe(ctx.mode) {
        let buffered = kind.exit_path == ExitPath::Buffer && buffers;
        let role = match (kind.container, buffered) {
            (Container::Carton, _) => "sealer_dropoff",
            (Container::Tray, _) => "flex",
            (Container::Tote, true) => "route_dispense",
            (Container::Tote, false) => "bagging",
        };
        let next = |from: &EntityId| -> Option<EntityId> {
            let mut taking = closed(from).into_iter().filter(|edge| admits(edge.spec.when.as_ref(), &kind));
            let edge = taking.next().filter(|_| taking.next().is_none())?;
            Some(edge.spec.to.id.clone())
        };
        let Some(first) = next(&node.id) else { continue };
        let through_buffer = graph.node(&first).is_some_and(|to| roles::is_storage(to, Holds::ClosedOrder));
        let Some(handoff) = (if through_buffer { next(&first) } else { Some(first) }) else { continue };
        let roles_there: Vec<&str> = (ctx.resolved.node(&handoff).map(|n| n.members.as_slice()).unwrap_or(&[]).iter())
            .filter_map(|member| ctx.site.station(member)?.1)
            .collect();
        if through_buffer != buffered || roles_there.is_empty() || roles_there.iter().any(|r| *r != role) {
            differ.push(words(&kind));
        }
    }
    if routed || !differ.is_empty() {
        let what = if routed { "routing designations".to_owned() } else { differ.join(", ") };
        let text = format!(
            "{}'s closed-order edges route {what} other than FulfillOrders' fixed routing does; the guards wait until it reads its flows.",
            ctx.label(&node.id)
        );
        problems.push(ctx.error(Code::ConsumerNotReady, &node.id, "/flow.node/process", text));
    }
}
