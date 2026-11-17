//! Within `scenario-spec`, `site` is what a scenario reads of its site from dessim's own loaded world (spec 07
//! section 3.2): the inventory tote slots the fill is a share of, and the flow facts needed before generating
//! and compiling a run: whether it brings stock in, and each pick node's sequencing,
//! read from the `flow.` entities as section 3.2 words them. It also names the closed values this build accepts,
//! which the local runner image's registered `accepts` repeats (spec 08 section 3.5).

use std::collections::BTreeMap;

use dessim::config::PickSequencing;
use dessim::worldmap::WorldMap;
use rustc_hash::FxHashSet;
use serde::Serialize;
use serde_json::Value;
use studio_protocol::{Release, SlotCount};

/// The setting names of the closed values a runner image accepts.
pub const PICK_SEQUENCING: &str = "pick.sequencing";
pub const RELEASE: &str = "workload.release";

/// The two facts of the site at the cut the generator reads: its inventory tote slots, and whether its flow brings
/// stock in (a `units` edge into a put node, which spec 06 infers as PutawayInduction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiteFacts {
    pub slots: SlotCount,
    pub brings_stock_in: bool,
}

/// The site's inventory tote slots and whether its flow brings stock in.
pub fn site_facts(world: &WorldMap) -> SiteFacts {
    let racking = &world.racking;
    let racks = racking.columns.iter().filter(|rack| rack.uses.takes_stock()).count() as u64;
    let slots = SlotCount(racks * u64::from(racking.vertical_tote_capacity));
    let puts = nodes_doing(world, "put");
    let brings_stock_in = flow_entities(world, "flow.material_edge").any(|(_, edge)| {
        edge.get("moves").and_then(Value::as_str) == Some("units")
            && edge.get("to").and_then(Value::as_str).is_some_and(|to| puts.contains(bare(to)))
    });
    SiteFacts { slots, brings_stock_in }
}

/// Each pick node's sequencing, by its entity id in id order; a node that states none takes spec 06's default.
pub fn pick_sequencing(world: &WorldMap) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = flow_entities(world, "flow.node")
        .filter(|(_, node)| operation(node) == Some("pick"))
        .map(|(id, node)| {
            let stated = node.pointer("/work/sequencing").and_then(Value::as_str).unwrap_or("all_totes");
            (id, stated.to_owned())
        })
        .collect();
    found.sort();
    found
}

/// The closed values this build compiles and loads, by setting: the sequencing values dessim enumerates, and only
/// the release every order at the start until dessim reads an order's arrival (BACKLOG task 46).
pub fn this_build_accepts() -> BTreeMap<String, Vec<String>> {
    // ordered: the accepts map is answered in setting order.
    let mut accepts = BTreeMap::new();
    let mut sequencing: Vec<String> = PickSequencing::ALL.iter().map(word_of).collect();
    sequencing.sort();
    accepts.insert(PICK_SEQUENCING.to_owned(), sequencing);
    accepts.insert(RELEASE.to_owned(), vec![word_of(&Release::AllAtStart)]);
    accepts
}

/// A closed value as its JSON spells it.
pub fn word_of<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

fn nodes_doing(world: &WorldMap, doing: &str) -> FxHashSet<String> {
    flow_entities(world, "flow.node").filter(|(_, node)| operation(node) == Some(doing)).map(|(id, _)| id).collect()
}

fn operation(node: &Value) -> Option<&str> {
    node.pointer("/process/operation").and_then(Value::as_str)
}

/// Every entity carrying `component`, with its id, from the world dessim's reader built.
fn flow_entities<'w>(world: &'w WorldMap, component: &'w str) -> impl Iterator<Item = (String, &'w Value)> + 'w {
    world
        .world
        .entities
        .values()
        .filter_map(move |entity| Some((entity.id.to_string(), entity.components.get(component)?)))
}

/// A reference's identity without its pin, as an export holds same-scope references bare.
fn bare(reference: &str) -> &str {
    reference.split_once('@').map_or(reference, |(id, _)| id)
}
