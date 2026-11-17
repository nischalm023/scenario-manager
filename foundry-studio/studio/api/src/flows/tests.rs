//! The flow read's body over the L-Shape made at 6 aisles, dessim's own fixture (spec 06 section 6.1): the counts
//! the Flow panel draws, the executors as it names them, what serves each edge, and the plan's seven roots.

use std::fs;
use std::path::Path;

use std::collections::BTreeMap;

use bytes::Bytes;
use dessim::composition::flowgraph::{Patch, Request, compile};
use dessim::worldmap::flows::read as read_flows;
use dessim::worldmap::load_from_bytes;
use serde_json::json;
use studio_protocol::{FlowSegment, FlowServedBy, Key, ScopePath, Seq, Severity};
use wps_client::{CutExport, Exactness, Walked};

use super::answer::{Body, item};
use super::{compiled_body, keepable};

fn fixture() -> Bytes {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json");
    Bytes::from(fs::read(&path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display())))
}

fn flow_scope() -> ScopePath {
    "customer/demo/site/lshape-6/flow".parse().expect("a flow scope")
}

fn segment<'a>(body: &'a Body, label: &str) -> &'a FlowSegment {
    body.segments.iter().find(|segment| segment.label == label).unwrap_or_else(|| panic!("{label} is drawn"))
}

/// Spec 06 section 6.1: 5 nodes, 8 edges, 13 segments and one return rule, compiling to the seven roots; a return
/// edge is served by each finished presentation's clearance, not a root; each node counts its members and ports, an
/// assigned executor is named as a kind, never referenced or counted.
#[test]
fn the_made_l_shape_reads_as_section_6_1_counts_it() {
    let body = compiled_body(&fixture(), &flow_scope());
    assert!(body.problems.is_empty(), "{:#?}", body.problems);
    let counts = (body.nodes.len(), body.edges.len(), body.segments.len(), body.return_rules.len());
    assert_eq!(counts, (5, 8, 13, 1));
    let plan = body.plan.as_ref().expect("a plan");
    let roots: Vec<&str> = plan.roots.iter().map(|root| root.type_.as_str()).collect();
    assert_eq!(
        roots,
        [
            "InitializeFleet",
            "FillInventory",
            "LoadOrders",
            "FleetConservation",
            "PutawaySupplyDemand",
            "PutawayInduction",
            "PickDemand"
        ]
    );
    let served =
        |label: &str| &body.edges.iter().find(|edge| edge.label == label).expect("the edge is drawn").served_by;
    let picked = FlowServedBy::Root { task: "PickDemand".into() };
    assert_eq!(served("Inventory storage to Pick stations: stock tote"), &picked);
    assert_eq!(served("Put stations to Inventory storage: stock tote"), &FlowServedBy::Clearance, "a return");
    assert_eq!(served("Pick stations to Outbound orders: order in its bin"), &FlowServedBy::Completion, "an order");
    let pick = body.nodes.iter().find(|node| node.name == "Pick stations").expect("the pick node");
    assert_eq!((pick.members, pick.sectors.len(), pick.sectors[0].members), (4, 1, 4));
    assert_eq!(pick.ports.get("presentation"), Some(&4));
    assert_eq!(pick.work.as_ref().map(|work| &work["pick_sim_ms"]), Some(&json!(7000)));
    let all_in = pick.all_in.as_ref().map(|all_in| all_in.text.as_str());
    assert_eq!(all_in, Some("7.0 s operator time + 2.0 s tote swap = 9.0 s, 400 an hour per station, all-in"));
    let storage = body.nodes.iter().find(|node| node.name == "Inventory storage").expect("the storage node");
    assert_eq!(storage.members, 12, "two rack rows in each of six aisles");
    let htm = &segment(&body, "HTM fleet: relay to Pick stations").executor;
    assert_eq!((htm.kind.as_str(), htm.name.as_str(), htm.count), ("cohort", "HTM fleet", Some(24)));
    let vtm = &segment(&body, "the aisle VTMs: slot to relay").executor;
    assert_eq!((vtm.kind.as_str(), vtm.name.as_str()), ("assigned", "the VTMs assigned there"));
    assert_eq!((vtm.ref_.as_ref(), vtm.count), (None, None), "the flow names no aisle's cohort");
    assert!(body.nodes.iter().all(|node| node.made_from_template), "the template made every flow entity");
}

/// A document the simulator's reader refuses compiles to no graph: `world-unreadable` when it states a flow, and
/// `flow-empty` when it states none, whatever its map.
#[test]
fn a_document_the_reader_refuses_answers_one_problem_and_no_plan() {
    let flowing = br#"{ "entities": [{ "id": "0b0d8b55-6a2d-4c3e-9f1a-2b3c4d5e6f70", "components": { "flow.node": { "kind": "sink" } } }] }"#;
    for (document, code, severity) in [
        (Bytes::from_static(flowing), "world-unreadable", Severity::Error),
        (Bytes::from_static(b"{}"), "flow-empty", Severity::Warning),
    ] {
        let body = compiled_body(&document, &flow_scope());
        let found: Vec<(&str, Severity)> = body.problems.iter().map(|p| (p.code.as_str(), p.severity)).collect();
        assert_eq!(found, [(code, severity)]);
        assert!(body.plan.is_none() && body.nodes.is_empty());
    }
}

/// A problem is spec 01's item: its code, the flow entity it names as a key, and the flow scope that holds it.
#[test]
fn a_problem_names_its_entity_in_the_flow_scope() {
    let world = load_from_bytes(&fixture()).expect("the fixture loads");
    let flows = read_flows(&world.world);
    let pick = flows.entities.iter().find(|entity| entity.name.as_deref() == Some("Pick stations")).unwrap();
    let strict =
        Patch { target: pick.id.clone(), pointer: "/flow.node/work/sequencing".into(), value: json!("strict") };
    let compiled = compile(&world, &flows, &Request { variant: vec![strict], ..Request::validation() });
    let items: Vec<_> = compiled.problems.iter().map(|finding| item(finding, &flow_scope())).collect();
    let refused = items.iter().find(|item| item.code == "unsupported-value").expect("strict is refused");
    assert_eq!(refused.ref_, Some(Key::Entity(pick.id.to_string().parse().unwrap())));
    assert_eq!(refused.scope, Some(flow_scope()));
    assert_eq!(refused.field.as_deref(), Some("/flow.node/work/sequencing"));
    assert!(refused.text.starts_with("Error: Pick stations asks for strict sequencing"));
}

/// Spec 06 section 3.4: an inexact read is not kept. The export's own exactness decides, which folds in each catalog
/// scope it holds below its head, never the cut's alone.
#[test]
fn only_an_exact_export_is_kept() {
    let catalog: ScopePath = "catalog/agent/d30".parse().unwrap();
    let export = |exact| CutExport {
        bytes: fixture(),
        sha256: "0".repeat(64),
        scopes: BTreeMap::from([(catalog.clone(), Seq::new(1))]),
        tag: None,
        exact,
        walked: Walked::Annotations,
        holders: Default::default(),
    };
    assert!(keepable(&export(Exactness::Exact)));
    assert!(!keepable(&export(Exactness::below_head(&catalog, Seq::new(1)))));
}
