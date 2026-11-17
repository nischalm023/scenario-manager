//! Spec 06 section 9, statement 6: one test per problem code of section 3.2, each breaking one thing in a clean
//! fixture and finding the problem on exactly that entity; and two pick nodes holding every pick station with
//! different times per pick give `consumer-conflict` naming both.

use dessim::composition::flowgraph::{Code, Compiled, Request, Severity, compile};
use dessim::worldmap::WorldMap;
use dessim::worldmap::flows::FlowScope;
use serde_json::{Value, json};
use site_model::ids::EntityId;

use crate::support::{self, L_SHAPE, LITMUS};

/// An id that sorts after every random one, for an entity a test adds where the later of two entities is named.
const LAST: &str = "ffffffff-ffff-4fff-bfff-ffffffffffff";

/// Compile `world`'s flows after `edit`, which finds entities by their labels in the clean compile.
fn after(world: &WorldMap, edit: impl FnOnce(&mut FlowScope, &dyn Fn(&str) -> EntityId)) -> Compiled {
    let mut flows = support::flows_of(world);
    let clean = compile(world, &flows, &Request::validation());
    assert!(
        clean.problems.iter().all(|p| p.severity == Severity::Warning || p.code == Code::GuardGap),
        "{:#?}",
        clean.problems
    );
    let pristine = flows.clone();
    edit(&mut flows, &|label| support::id_of(&clean, &pristine, label));
    compile(world, &flows, &Request::validation())
}

fn l_shape(edit: impl FnOnce(&mut FlowScope, &dyn Fn(&str) -> EntityId)) -> Compiled {
    after(&L_SHAPE, edit)
}

/// The problems of `code` name `entity`, and only it.
fn finds(compiled: &Compiled, code: Code, entity: &EntityId) {
    let named = support::named_by(compiled, code);
    assert!(!named.is_empty() && named.iter().all(|id| id == entity), "{code:?} on {entity}: {:#?}", compiled.problems);
}

fn component<'a>(flows: &'a mut FlowScope, id: &EntityId, name: &str) -> &'a mut Value {
    flows.get_mut(id).unwrap().components.get_mut(name).unwrap()
}

#[test]
fn schema_unknown() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let pick = id("Pick stations");
        component(flows, &pick, "flow.node")["work"]["pick_time_ms"] = json!(7000);
        target = Some(pick);
    });
    finds(&compiled, Code::SchemaUnknown, &target.unwrap());
}

/// Section 3.1's lower bounds, which `schema-unknown` refuses as values outside the registered schema: an admission
/// factor of zero, a backlog of zero, a step of zero (Astra's task-11 read, the compiler's forbidden values).
#[test]
fn schema_unknown_below_a_lower_bound() {
    for (label, name, pointer, value, words) in [
        ("Pick stations", "flow.node", "/intake/0/admission_factor", json!(0.0), "admission_factor is above zero"),
        ("Return rule", "flow.return_rule", "/min_backlog", json!(0), "min_backlog is a whole number from 1"),
        ("HTM fleet: relay to Pick stations", "flow.agent_segment", "/step", json!(0), "step is a whole number from 1"),
    ] {
        let mut target = None;
        let compiled = l_shape(|flows, id| {
            let entity = id(label);
            *component(flows, &entity, name).pointer_mut(pointer).unwrap() = value;
            target = Some(entity);
        });
        finds(&compiled, Code::SchemaUnknown, target.as_ref().unwrap());
        let text = &compiled.problems.iter().find(|p| p.code == Code::SchemaUnknown).unwrap().text;
        assert!(text.contains(words), "{label}: {text}");
        assert!(compiled.plan.is_none(), "{label}: nothing compiles");
    }
}

#[test]
fn ref_unresolved() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let edge = id("Put stations to Inventory storage: stock tote");
        component(flows, &edge, "flow.material_edge")["returns_by"] = json!("0b0d8b55-6a2d-4c3e-9f1a-2b3c4d5e6f70");
        target = Some(edge);
    });
    finds(&compiled, Code::RefUnresolved, &target.unwrap());
}

#[test]
fn node_empty() {
    let empty = "5e0b1a7c-3d2e-4f6a-8b9c-0d1e2f3a4b5c";
    let world = support::l_shape_with(|document| {
        let entities = document["entities"].as_array_mut().unwrap();
        entities.push(json!({ "id": empty, "name": "Nowhere", "components": { "sector": { "members": [] } } }));
        let put = entities
            .iter_mut()
            .find(|e| e["name"] == "Put stations" && e["components"]["flow.node"].is_object())
            .unwrap();
        put["components"]["flow.node"]["sectors"] = json!([empty]);
    });
    let compiled = compile(&world, &support::flows_of(&world), &Request::validation());
    let put = support::id_of(&compiled, &support::flows_of(&world), "Put stations");
    finds(&compiled, Code::NodeEmpty, &put);
}

#[test]
fn member_unfit() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let (put, pick) = (id("Put stations"), id("Pick stations"));
        let picks = component(flows, &pick, "flow.node")["sectors"].clone();
        component(flows, &put, "flow.node")["sectors"] = picks;
        target = Some(put);
    });
    finds(&compiled, Code::MemberUnfit, &target.unwrap());
}

#[test]
fn holds_mismatch() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let storage = id("Inventory storage");
        component(flows, &storage, "flow.node")["holds"] = json!("order_container");
        target = Some(storage);
    });
    finds(&compiled, Code::HoldsMismatch, &target.unwrap());
}

#[test]
fn overlap_exclusive() {
    let compiled = l_shape(|flows, id| {
        let sectors = component(flows, &id("Inventory storage"), "flow.node")["sectors"].clone();
        let node = json!({ "kind": "storage", "sectors": sectors, "holds": "order_container" });
        support::add(flows, LAST, Some("Order tote row"), "flow.node", node);
    });
    finds(&compiled, Code::OverlapExclusive, &LAST.parse().unwrap());
}

#[test]
fn port_invalid() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let segment = id("the aisle VTMs: slot to relay");
        component(flows, &segment, "flow.agent_segment")["from"]["port"] = json!("presentation");
        target = Some(segment);
    });
    finds(&compiled, Code::PortInvalid, &target.unwrap());
}

#[test]
fn edge_unrealized() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        support::remove(flows, &id("orders leave in their bins"));
        target = Some(id("Pick stations to Outbound orders: order in its bin"));
    });
    finds(&compiled, Code::EdgeUnrealized, &target.unwrap());
}

#[test]
fn chain_broken() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        component(flows, &id("HTM fleet: relay to Pick stations"), "flow.agent_segment")["step"] = json!(3);
        target = Some(id("Inventory storage to Pick stations: stock tote"));
    });
    finds(&compiled, Code::ChainBroken, &target.unwrap());
}

#[test]
fn custody_gap() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let segment = id("the aisle VTMs: relay to slot");
        component(flows, &segment, "flow.agent_segment")["at_end"] = json!("keeps_through_work");
        target = Some(segment);
    });
    finds(&compiled, Code::CustodyGap, &target.unwrap());
}

#[test]
fn executor_unresolved() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let segment = id("HTM fleet: relay to Put stations");
        component(flows, &segment, "flow.agent_segment")["executor"] =
            json!({ "cohort": "1c9e0f4b-7a6d-4e5c-8b3a-2f1e0d9c8b7a" });
        target = Some(segment);
    });
    finds(&compiled, Code::ExecutorUnresolved, &target.unwrap());
}

#[test]
fn executor_cannot_take() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let segment = id("HTM fleet: relay to Pick stations");
        component(flows, &segment, "flow.agent_segment")["executor"] = json!({ "assigned": "from" });
        target = Some(segment);
    });
    finds(&compiled, Code::ExecutorCannotTake, &target.unwrap());
}

#[test]
fn process_incomplete() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let pick = id("Pick stations");
        component(flows, &pick, "flow.node")["process"].as_object_mut().unwrap().remove("gives_by");
        target = Some(pick);
    });
    finds(&compiled, Code::ProcessIncomplete, &target.unwrap());
}

/// Section 3.1's synchronization: an input that ends elsewhere, or brings another carrier than its side takes, is no
/// input of the process (Astra's task-11 read: a process input's edge must end there with the right carrier).
#[test]
fn process_incomplete_on_an_input_that_does_not_bring_its_carrier_here() {
    let cases = [
        ("Pick stations", "/process/gives_by/0", "Pick stations to Inventory storage: tote after the pick", "ends at"),
        ("Put stations", "/process/receives_by/0", "Inventory storage to Pick stations: stock tote", "ends at"),
        ("Put stations", "/process/receives_by/0", "Inbound units to Put stations: units", "not what a put puts"),
        (
            "Put stations",
            "/process/gives_by/0",
            "Inventory storage to Put stations: empty tote",
            "not what a put takes",
        ),
    ];
    for (node, pointer, edge, words) in cases {
        let mut target = None;
        let compiled = l_shape(|flows, id| {
            let (holder, named) = (id(node), id(edge));
            *component(flows, &holder, "flow.node").pointer_mut(pointer).unwrap() = json!(named.to_string());
            target = Some(holder);
        });
        finds(&compiled, Code::ProcessIncomplete, target.as_ref().unwrap());
        let found = compiled.problems.iter().find(|p| p.code == Code::ProcessIncomplete).unwrap();
        assert_eq!(found.field.as_deref(), Some(&*format!("/flow.node{pointer}")), "{node} {pointer}");
        assert!(found.text.contains(words), "{node} {pointer}: {}", found.text);
    }
}

#[test]
fn guard_gap() {
    let mut target = None;
    let compiled = after(&LITMUS, |flows, id| {
        support::remove(flows, &id("EasyLoop machines to Sealer drop-off: closed order"));
        target = Some(id("EasyLoop machines"));
    });
    finds(&compiled, Code::GuardGap, &target.unwrap());
}

#[test]
fn guard_overlap() {
    let mut target = None;
    let compiled = after(&LITMUS, |flows, id| {
        let bagging = id("EasyLoop machines to Bagging: closed order");
        component(flows, &bagging, "flow.material_edge")["when"].as_object_mut().unwrap().remove("container");
        target = Some(id("EasyLoop machines"));
    });
    finds(&compiled, Code::GuardOverlap, &target.unwrap());
}

#[test]
fn alternative_invalid() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let direct = id("Pick stations to Put stations: empty tote");
        let primary = id("Put stations to Inventory storage: stock tote");
        component(flows, &direct, "flow.material_edge")["alternative_to"] = json!(primary.to_string());
        target = Some(direct);
    });
    finds(&compiled, Code::AlternativeInvalid, &target.unwrap());
}

#[test]
fn work_missing() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let pick = id("Pick stations");
        component(flows, &pick, "flow.node")["work"].as_object_mut().unwrap().remove("pick_sim_ms");
        target = Some(pick);
    });
    finds(&compiled, Code::WorkMissing, &target.unwrap());
}

#[test]
fn work_disagrees_with_map() {
    let mut target = None;
    let compiled = after(&LITMUS, |flows, id| {
        let lane = id("EasyLoop machines");
        component(flows, &lane, "flow.node")["work"].as_object_mut().unwrap().remove("map_figures");
        target = Some(lane);
    });
    finds(&compiled, Code::WorkDisagreesWithMap, &target.unwrap());
}

#[test]
fn unsupported_value() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let pick = id("Pick stations");
        component(flows, &pick, "flow.node")["work"]["sequencing"] = json!("strict");
        target = Some(pick);
    });
    finds(&compiled, Code::UnsupportedValue, &target.unwrap());
    let text = &compiled.problems.iter().find(|p| p.code == Code::UnsupportedValue).unwrap().text;
    assert!(text.contains("all_totes and sequenced"), "{text}");
}

#[test]
fn unsupported_pattern() {
    let mut target = None;
    let compiled = l_shape(|flows, id| {
        let aisle =
            L_SHAPE.world.entities.values().find(|e| e.name.as_deref() == Some("aisle") && e.has("sector")).unwrap();
        let feed = id("Inventory storage to Pick stations: stock tote");
        component(flows, &feed, "flow.material_edge")["storage_scope"] = json!([aisle.id.to_string()]);
        target = Some(feed);
    });
    finds(&compiled, Code::UnsupportedPattern, &target.unwrap());
}

/// A station-group node holding two of the four pick stations, while PickDemand still takes every pick station.
#[test]
fn consumer_not_ready() {
    let half = "7a3c9e1f-5b2d-4c8e-9f0a-1b2c3d4e5f60";
    let world = support::l_shape_with(|document| {
        let entities = document["entities"].as_array_mut().unwrap();
        let mut picks: Vec<String> = (entities.iter())
            .filter(|e| e["components"]["station"]["kind"] == "pick")
            .map(|e| e["id"].as_str().unwrap().to_owned())
            .collect();
        picks.sort();
        entities.push(
            json!({ "id": half, "name": "Two pick stations", "components": { "sector": { "members": picks[..2] } } }),
        );
        let node = entities
            .iter_mut()
            .find(|e| e["name"] == "Pick stations" && e["components"]["flow.node"].is_object())
            .unwrap();
        node["components"]["flow.node"]["sectors"] = json!([half]);
    });
    let compiled = compile(&world, &support::flows_of(&world), &Request::validation());
    let pick = support::id_of(&compiled, &support::flows_of(&world), "Pick stations");
    finds(&compiled, Code::ConsumerNotReady, &pick);
    let text = &compiled.problems.iter().find(|p| p.code == Code::ConsumerNotReady).unwrap().text;
    assert!(text.contains("2 of the 4 pick stations") && text.contains("PickDemand"), "{text}");
}

/// Two pick nodes, each holding every pick station, stating different times per pick: operator.time_per_pick_secs
/// holds one, so the conflict names both.
#[test]
fn consumer_conflict() {
    let compiled = l_shape(|flows, id| {
        let mut second = component(flows, &id("Pick stations"), "flow.node").clone();
        second["work"]["pick_sim_ms"] = json!(7500);
        support::add(flows, LAST, Some("Pick stations south"), "flow.node", second);
    });
    finds(&compiled, Code::ConsumerConflict, &LAST.parse().unwrap());
    let conflicts: Vec<&str> =
        compiled.problems.iter().filter(|p| p.code == Code::ConsumerConflict).map(|p| p.text.as_str()).collect();
    assert!(
        conflicts.contains(
            &"Error: Pick stations and Pick stations south state different times per pick (7.0 s, 7.5 s); \
              operator.time_per_pick_secs holds one until PickDemand reads its flows."
        ),
        "{conflicts:#?}"
    );
}

#[test]
fn node_unused() {
    let compiled = l_shape(|flows, _| {
        support::add(flows, LAST, Some("Returns dock"), "flow.node", json!({ "kind": "sink" }));
    });
    finds(&compiled, Code::NodeUnused, &LAST.parse().unwrap());
    assert!(compiled.plan.is_some(), "a warning does not refuse the plan");
}

#[test]
fn flow_empty() {
    let compiled = compile(&L_SHAPE, &FlowScope::default(), &Request::validation());
    let codes: Vec<(Code, Severity)> = compiled.problems.iter().map(|p| (p.code, p.severity)).collect();
    assert_eq!(codes, [(Code::FlowEmpty, Severity::Warning)]);
    assert!(compiled.plan.is_none() && compiled.graph.nodes.is_empty());
}
