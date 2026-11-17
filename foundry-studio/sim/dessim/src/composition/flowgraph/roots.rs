//! Within `flowgraph`, `roots` is stage 4 of spec 06 section 3.2, root inference: one instance per root type in the
//! table's order, with the `after` lists of `compositions/cencora.json` and `walmart.json`, each configured from the
//! flows it serves; and the settings block for every value the run also reads as a setting. Every value is stated,
//! a default included, so no setting stands in for one (ruling D93, "no fallback to settings").

use serde_json::{Map, Value, json};
use site_model::ids::EntityId;

use super::components::{Holds, Moves};
use super::consumers::{Consumers, Feed};
use super::graph::Graph;
use super::plan::SettingEntry;
use super::roles::{self, Root, Served};
use super::{Mode, RunSetup};
use crate::composition::{Definition, RootTask};
use crate::simtime::{SimDuration, SimTime};
use crate::task::TaskDefaults;

fn secs(duration: SimDuration) -> f64 {
    duration.as_ms() as f64 / 1000.0
}

fn at_secs(at: SimTime) -> f64 {
    at.as_ms() as f64 / 1000.0
}

/// The roots the graph's shape calls for, in table order.
fn inferred(graph: &Graph) -> Vec<Root> {
    let serves = |root: Root| graph.edges.iter().any(|edge| roles::served_by(graph, edge) == Served::Root(root));
    Root::ORDER
        .into_iter()
        .filter(|&root| match root {
            Root::InitializeFleet | Root::FillInventory | Root::LoadOrders | Root::FleetConservation => true,
            Root::FulfillOrders => graph.nodes.iter().any(roles::is_pick_loop),
            Root::PutawaySupplyDemand => graph.edges.iter().any(|edge| {
                edge.spec.alternative_to.is_none()
                    && roles::served_by(graph, edge) == Served::Root(Root::PutawaySupplyDemand)
            }),
            other => serves(other),
        })
        .collect()
}

pub(super) struct Inference {
    pub definition: Definition,
    pub settings: Vec<SettingEntry>,
}

/// Write each present feed's first statement into `config` under `key`.
fn put<T>(config: &mut Map<String, Value>, key: &str, feed: &Feed<T>, value: impl Fn(&T) -> Value) {
    if let Some(stated) = feed.first() {
        config.insert(key.to_owned(), value(&stated.value));
    }
}

pub(super) fn infer(graph: &Graph, consumers: &Consumers, mode: &Mode) -> Inference {
    let roots = inferred(graph);
    let tasks = roots
        .iter()
        .map(|&root| RootTask {
            task_type: root.name().to_owned(),
            config: Value::Object(config(root, consumers, mode)),
            after: root.after().iter().map(|after| after.name().to_owned()).collect(),
        })
        .collect();
    let mut settings = settings(&roots, graph, consumers);
    settings.sort_by(|a, b| a.setting.cmp(&b.setting));
    Inference { definition: Definition { tasks, task_defaults: TaskDefaults::default() }, settings }
}

fn config(root: Root, c: &Consumers, mode: &Mode) -> Map<String, Value> {
    let mut config = Map::new();
    match root {
        Root::FillInventory => {
            if let Mode::Run { setup: RunSetup { stock_at_start, order_trays }, .. } = mode {
                config.insert("order_trays".into(), json!(order_trays));
                config.insert("preload_fraction".into(), json!(stock_at_start));
            }
        }
        Root::FulfillOrders => {
            config
                .insert("bypass_box_induction".into(), json!(c.bypass_box_induction.first().is_some_and(|s| s.value)));
            config.insert("bypass_extraction".into(), json!(c.bypass_extraction.first().is_some_and(|s| s.value)));
            put(&mut config, "extraction_secs", &c.extraction, |&time| json!(secs(time)));
            put(&mut config, "buffer_dwell_secs", &c.buffer_dwell, |&dwell| json!(secs(dwell)));
            put(&mut config, "queue_factor", &c.loop_admission, |&factor| json!(factor));
            if let Some(Some(cap)) = c.max_open_orders.first().map(|s| s.value) {
                config.insert("max_active_orders_per_machine".into(), json!(cap));
            }
        }
        Root::PutawaySupplyDemand => {
            if let Some(Some(bias)) = c.direct_bias.first().map(|s| s.value) {
                config.insert("direct_bias_secs".into(), json!(bias.as_ms() as f64 / 1000.0));
            }
        }
        Root::PutawayInduction => {
            put(&mut config, "max_presentations_per_hour", &c.fill_pace, |pace| json!(pace));
            put(&mut config, "sequence", &c.induction_sequence, |sequence| json!(sequence));
            put(&mut config, "units_per_tote", &c.units_per_tote, |units| json!(units));
            if let Some(Some(lead)) = c.lead_time.first().map(|s| s.value) {
                config.insert("lead_time_secs".into(), json!(secs(lead)));
            }
        }
        Root::PickDemand => {
            put(&mut config, "embargo_secs", &c.pick_opens, |&at| json!(at_secs(at)));
            put(&mut config, "sequencing", &c.sequencing, |mode| json!(mode));
            put(&mut config, "pace_to_station", &c.pace_to_station, |pace| json!(pace));
            put(&mut config, "queue_factor", &c.pick_admission, |factor| json!(factor));
            if let Some(Some(cap)) = c.max_inflight.first().map(|s| s.value) {
                config.insert("max_inflight_picks".into(), json!(cap));
            }
        }
        Root::InitializeFleet | Root::LoadOrders | Root::FleetConservation => {}
    }
    config
}

fn entry(setting: &str, value: Value, from: &EntityId, field: &str) -> SettingEntry {
    SettingEntry { setting: setting.to_owned(), value, from: from.clone(), field: field.to_owned() }
}

/// A feed's first statement: the entity, its field and the value.
fn stated<T>(feed: &Feed<T>) -> Option<(&EntityId, &str, &T)> {
    feed.first().map(|stated| (&stated.entity, stated.field.as_str(), &stated.value))
}

fn settings(roots: &[Root], graph: &Graph, c: &Consumers) -> Vec<SettingEntry> {
    let mut out = Vec::new();
    if let Some((from, field, &time)) = stated(&c.time_per_pick) {
        out.push(entry("operator.time_per_pick_secs", json!(secs(time)), from, field));
    }
    if let Some((from, field, &random)) = stated(&c.random_slots) {
        out.push(entry("putaway.random_slots", json!(random), from, field));
    }
    if roots.contains(&Root::PickDemand)
        && let Some((from, field, &swap)) = stated(&c.pallet_swap)
    {
        out.push(entry("operator.pallet_swap_secs", json!(secs(swap)), from, field));
    }
    if roots.contains(&Root::PutawayInduction) {
        let inbound = graph.edges.iter().find(|e| roles::served_by(graph, e) == Served::Root(Root::PutawayInduction));
        if let Some(edge) = inbound {
            out.push(entry("putaway.enabled", json!(true), &edge.id, "/flow.material_edge/moves"));
        }
        if let Some((from, field, &gate)) = stated(&c.hour_gate) {
            out.push(entry("putaway.ignore_hour_gate", json!(!gate), from, field));
        }
    }
    if roots.contains(&Root::PutawaySupplyDemand) {
        if let Some((from, field, &factor)) = stated(&c.put_admission) {
            out.push(entry("putaway.htm_wip_factor", json!(factor), from, field));
        }
        if let Some((from, field, &cutoff)) = stated(&c.rack_cutoff) {
            out.push(entry("putaway.forbid_rack_after_secs", json!(cutoff.map(at_secs)), from, field));
        }
    }
    if roots.contains(&Root::FulfillOrders) {
        // Closed orders rest in the buffer exactly when an edge takes them there (`flows/fulfillment/orders.rs`).
        let buffered = graph.edges.iter().find(|edge| {
            edge.spec.moves == Moves::ClosedOrder
                && graph
                    .ends(edge)
                    .is_some_and(|(f, t)| roles::is_pick_loop(f) && roles::is_storage(t, Holds::ClosedOrder))
        });
        let pick_loop = graph.nodes.iter().find(|node| roles::is_pick_loop(node));
        match (buffered, pick_loop) {
            (Some(edge), _) => {
                out.push(entry("order.direct_extraction", json!(false), &edge.id, "/flow.material_edge/to"))
            }
            (None, Some(node)) => {
                out.push(entry("order.direct_extraction", json!(true), &node.id, "/flow.node/process"))
            }
            (None, None) => {}
        }
        if let Some((from, field, &opens)) = stated(&c.loop_opens) {
            out.push(entry("pick.embargo_secs", json!(at_secs(opens)), from, field));
        }
    }
    out
}
