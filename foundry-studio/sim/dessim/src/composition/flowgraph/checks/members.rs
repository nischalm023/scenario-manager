//! Within `checks`, `members` holds a node's members to what the node needs of them (spec 06 section 3.1's member
//! table): `node-empty`, `member-unfit`, `holds-mismatch`, `overlap-exclusive` where a process needs exclusive
//! ownership (ruling D68: no other overlap is checked), and `work-disagrees-with-map` against the figures the
//! members themselves state.

use std::slice;

use rustc_hash::FxHashSet;
use serde_json::Value;
use site_model::ids::EntityId;

use super::Ctx;
use crate::composition::flowgraph::components::{Holds, NodeKind, Operation};
use crate::composition::flowgraph::consumers::seconds;
use crate::composition::flowgraph::graph::Node;
use crate::composition::flowgraph::problem::{Code, Problem};
use crate::composition::flowgraph::roles;

/// What a member of a stations node must be, by the node's operation (`worldmap/stations.rs` reads `station.kind`).
fn station_kinds(operation: Operation) -> &'static [&'static str] {
    match operation {
        Operation::Put => &["put", "putaway"],
        Operation::Pick => &["pick"],
        Operation::Extract => &["extract"],
        Operation::CreateContainer => &["induct"],
    }
}

const EXTRACT_ROLES: [&str; 4] = ["route_dispense", "sealer_dropoff", "bagging", "flex"];

/// Why a member does not fit its node, if it does not.
fn unfit(ctx: &Ctx, node: &Node, member: &EntityId) -> Option<String> {
    let site = ctx.site;
    if site.entity(member).is_none() {
        return Some("names nothing at this read".into());
    }
    match (node.spec.kind, roles::operation(node)) {
        (NodeKind::Stations, Some(operation)) => {
            let Some((kind, role)) = site.station(member) else { return Some("is not a station".into()) };
            if !station_kinds(operation).contains(&kind) {
                return Some(format!("is a {kind} station, which cannot run this operation"));
            }
            let presents = site.presentations(member);
            match operation {
                Operation::Put | Operation::Pick
                    if !presents.iter().any(|f| f.as_deref() == Some("inventory_tote")) =>
                {
                    Some("has no presentation point for an inventory tote".into())
                }
                Operation::Extract if !role.is_some_and(|role| EXTRACT_ROLES.contains(&role)) => {
                    Some("states no extraction role (route_dispense, sealer_dropoff, bagging or flex)".into())
                }
                Operation::Extract if presents.is_empty() => Some("has no presentation point".into()),
                _ => None,
            }
        }
        (NodeKind::Conveyance, _) => {
            let Some(machine) = site.machine(member) else { return Some("is not a belt machine (go.easyloop)".into()) };
            if machine.sinks == 0 || machine.sources == 0 {
                Some("lacks an inventory sink or source port".into())
            } else if machine.order_presentations != 2 {
                Some(format!("has {} order presentations, and FulfillOrders runs two", machine.order_presentations))
            } else {
                None
            }
        }
        (NodeKind::Storage, _) if site.bays_of(member).is_empty() => Some("is not a rack row or bay".into()),
        _ => None,
    }
}

/// The holds a storage node's bays must state (`storage.holds`): inventory for inventory totes, order otherwise.
fn bay_holds(holds: Holds) -> &'static str {
    match holds {
        Holds::InventoryTote => "inventory",
        Holds::OrderContainer | Holds::ClosedOrder => "order",
    }
}

pub(super) fn check(ctx: &Ctx, problems: &mut Vec<Problem>) {
    for node in &ctx.graph.nodes {
        let Some(resolved) = ctx.resolved.node(&node.id) else { continue };
        if !matches!(node.spec.kind, NodeKind::Stations | NodeKind::Storage | NodeKind::Conveyance) {
            continue;
        }
        let unresolved = node.spec.sectors.iter().any(|s| !s.at_head() || ctx.site.sector(&s.id).is_none());
        if resolved.members.is_empty() && !unresolved {
            let text =
                format!("{}'s sectors have no members, so it stands for nothing on the map.", ctx.label(&node.id));
            problems.push(ctx.error(Code::NodeEmpty, &node.id, "/flow.node/sectors", text));
            continue;
        }
        let mut reasons: Vec<(String, Vec<EntityId>)> = Vec::new();
        for member in &resolved.members {
            if let Some(reason) = unfit(ctx, node, member) {
                match reasons.iter_mut().find(|(seen, _)| *seen == reason) {
                    Some((_, members)) => members.push(member.clone()),
                    None => reasons.push((reason, vec![member.clone()])),
                }
            }
        }
        for (reason, members) in reasons {
            let text = format!("{}'s member {} {reason}.", ctx.label(&node.id), ctx.some_of(&members));
            problems.push(ctx.error(Code::MemberUnfit, &node.id, "/flow.node/sectors", text));
        }
        if let (NodeKind::Storage, Some(holds)) = (node.spec.kind, node.spec.holds) {
            let want = bay_holds(holds);
            let wrong: Vec<EntityId> = (resolved.members.iter())
                .filter(|member| ctx.site.bays_of(member).iter().any(|bay| ctx.site.bay_holds(bay) != want))
                .cloned()
                .collect();
            if !wrong.is_empty() {
                let text = format!(
                    "{} holds {}, and its member {} holds something else.",
                    ctx.label(&node.id),
                    holds.word(),
                    ctx.some_of(&wrong)
                );
                problems.push(ctx.error(Code::HoldsMismatch, &node.id, "/flow.node/holds", text));
            }
        }
        disagrees_with_map(ctx, node, &resolved.members, problems);
    }
    overlaps(ctx, problems);
}

/// One rack row or bay in two storage nodes holding different things (a slot holds one kind of carrier), or one
/// belt machine in two conveyance nodes (one owner of its admission and its order presentations).
fn overlaps(ctx: &Ctx, problems: &mut Vec<Problem>) {
    let owned = |node: &Node| -> FxHashSet<EntityId> {
        let members = ctx.resolved.node(&node.id).map(|n| n.members.as_slice()).unwrap_or(&[]);
        match node.spec.kind {
            NodeKind::Storage => members.iter().flat_map(|member| ctx.site.bays_of(member)).collect(),
            _ => members.iter().cloned().collect(),
        }
    };
    let nodes = &ctx.graph.nodes;
    for (at, later) in nodes.iter().enumerate() {
        for earlier in &nodes[..at] {
            let exclusive = match (earlier.spec.kind, later.spec.kind) {
                (NodeKind::Storage, NodeKind::Storage) => earlier.spec.holds != later.spec.holds,
                (NodeKind::Conveyance, NodeKind::Conveyance) => true,
                _ => false,
            };
            if exclusive && !owned(earlier).is_disjoint(&owned(later)) {
                let text = format!(
                    "{} shares racking or machines with {}, which a process owns alone.",
                    ctx.label(&later.id),
                    ctx.label(&earlier.id)
                );
                problems.push(ctx.error(Code::OverlapExclusive, &later.id, "/flow.node/sectors", text));
            }
        }
    }
}

/// A time per pick the node states against the one its members' machines state (`go.easyloop.time_per_pick_secs`),
/// unless the node marks the map's figures display only.
fn disagrees_with_map(ctx: &Ctx, node: &Node, members: &[EntityId], problems: &mut Vec<Problem>) {
    let work = &node.spec.work;
    let (Some(pick), None) = (work.pick_sim_ms, work.map_figures) else { return };
    for member in members {
        let stated = ctx
            .site
            .entity(member)
            .and_then(|entity| entity.raw("go.easyloop"))
            .and_then(|belt| belt.get("time_per_pick_secs"))
            .and_then(Value::as_f64);
        if let Some(secs) = stated.filter(|secs| (secs * 1000.0 - pick.as_ms() as f64).abs() >= 1.0) {
            let text = format!(
                "{} states {} per pick and its member {} states {secs} s; mark the map's figures display only \
                 (work.map_figures) to keep the node's.",
                ctx.label(&node.id),
                seconds(pick),
                ctx.some_of(slice::from_ref(member)),
            );
            problems.push(ctx.error(Code::WorkDisagreesWithMap, &node.id, "/flow.node/work/pick_sim_ms", text));
            return;
        }
    }
}
