//! What the run and scenario tests share (briefs I1.08 and I1.09): a site whose world dessim's own reader loads,
//! staged into a mock world-proto-server of world-proto cd4923516 whose registry defines every component it carries:
//! dessim's small relay fixture `minimal_putaway.json` in the map scope, the template's configured flow, and the root's
//! prologue last.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use studio_protocol::{DeclaredId, ScopePath};
use wps_client::draft::Op;
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client, ClientConfig, MockWps};

/// The fixture's rack run, its access, sector, VTM cohort, bay template and VTM class, by name: without them no rack
/// stands.
pub fn rack_part(name: &str) -> bool {
    ["rack run", "access:", "sector:", "VTMs", "relay bay", "VTM A5"].iter().any(|part| name.contains(part))
}

/// The fixture, every declared identity in it renamed to a fresh one unless `as_written`, since an identity belongs to
/// the scope that first declared it and two sites cannot both declare it.
pub fn fixture(as_written: bool) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_putaway.json");
    let mut text = std::fs::read_to_string(path).expect("the fixture reads");
    let doc: Value = serde_json::from_str(&text).expect("JSON");
    if !as_written {
        for id in doc["entities"].as_array().expect("entities").iter().filter_map(|entry| entry["id"].as_str()) {
            text = text.replace(id, &DeclaredId::mint().to_string());
        }
    }
    let mut document: Value = serde_json::from_str(&text).expect("JSON");
    // Preserve the legacy fixture's model and its legacy constant-mode execution cruise explicitly.
    for entity in document["entities"].as_array_mut().expect("entities") {
        if let Some(drive) = entity["components"].get_mut("floor_drive") {
            drive["kind"] = json!("trapezoidal");
            for load in ["loaded", "empty"] {
                drive[load]["constant_speed_mm_s"] = json!(6000);
            }
        }
    }
    document
}

/// A mock of world-proto cd4923516 generating with the native expansion, whose registry defines every component the
/// sites carry, published before the client starts and loads it.
pub async fn world() -> Client<MockWps> {
    let mock = MockWps::phase_3();
    super::generation::native(&mock);
    let components = [
        "access",
        "access_cell",
        "carrier",
        "carrier_pool",
        "carrier_type",
        "cell",
        "cohort",
        "end_effector",
        "external_ids",
        "floor_drive",
        "frame",
        "lift",
        "port",
        "pose",
        "presentation_point",
        "queue",
        "rack",
        "rack_ladder",
        "region",
        "relay",
        "robot",
        "robot_class",
        "sector",
        "slot",
        "stamp",
        "station",
        "stitch",
        "storage",
        "template",
        "tiling",
        "flow.node",
        "flow.material_edge",
        "flow.agent_segment",
        "flow.return_rule",
        "successor",
    ];
    for component in components {
        mock.publish(component, "0.1.0", json!({ "type": "object" }), Vec::new());
    }
    Client::start(mock, ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("the mock's client")
}

pub fn below(root: &ScopePath, sub: &str) -> ScopePath {
    root.join(&sub.parse().expect("a slug")).expect("a sub-scope")
}

pub fn components(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

/// The flow's stock intake, as section 3.2 reads it: a `units` edge from the putaway source into a put node; with
/// `sequencing`, a pick node that sequences its picks so.
pub fn flow(sequencing: Option<&str>) -> Vec<Op> {
    let (source, put) = (DeclaredId::mint(), DeclaredId::mint());
    let mut ops = vec![
        Op::Declare {
            id: source,
            name: None,
            components: components(json!({ "flow.node": { "kind": "source", "demand": "putaway" } })),
        },
        Op::Declare {
            id: put,
            name: None,
            components: components(json!({ "flow.node": { "kind": "stations", "process": { "operation": "put" } } })),
        },
        Op::Declare {
            id: DeclaredId::mint(),
            name: None,
            components: components(
                json!({ "flow.material_edge": { "from": source.to_string(), "to": put.to_string(), "moves": "units" } }),
            ),
        },
    ];
    if let Some(sequencing) = sequencing {
        let node = json!({ "flow.node": { "kind": "stations", "process": { "operation": "pick" }, "work": { "sequencing": sequencing } } });
        ops.push(Op::Declare { id: DeclaredId::mint(), name: None, components: components(node) });
    }
    ops
}

/// A site made from the fixture: its scopes made by name, its world in `map`, its flow, and the root's prologue last.
pub async fn fixture_site<S: Wps>(world: &Client<S>, root: &str, racks: bool) -> ScopePath {
    let root: ScopePath = root.parse().expect("a site's root");
    super::made_scopes(world, &root).await;
    let doc = fixture(racks);
    let mut ops = Vec::new();
    for entry in doc["entities"].as_array().expect("entities") {
        let parts = components(entry["components"].clone());
        match entry.get("id").and_then(Value::as_str) {
            Some(_) if !racks && entry["name"].as_str().is_some_and(rack_part) => {}
            Some(id) => ops.push(Op::Declare {
                id: id.parse().expect("a declared id"),
                name: entry["name"].as_str().map(str::to_owned),
                components: parts,
            }),
            None => {
                let derivation = &entry["derivation"];
                let target =
                    format!("{}/{}", derivation["generator"].as_str().unwrap(), derivation["key"].as_str().unwrap());
                ops.push(Op::Override { target: target.parse().expect("a derived address"), components: parts });
            }
        }
    }
    super::stage(world, &below(&root, "map"), ops, None).await;
    super::stage(world, &below(&root, "flow"), configured_flow(&doc, racks), None).await;
    let frame: DeclaredId = doc["primary_frame"].as_str().unwrap().parse().expect("the primary frame's id");
    super::stage(world, &root, Vec::new(), Some(super::prologue("L-Shape 6", frame))).await;
    root
}

/// The template's stock, pick and return flow over the small fixture's stations and rack sector.
fn configured_flow(doc: &Value, racks: bool) -> Vec<Op> {
    let template: Value =
        serde_json::from_str(include_str!("../../../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json"))
            .unwrap();
    let mut flows: Vec<Value> = template["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["components"].as_object().unwrap().keys().any(|key| key.starts_with("flow.")))
        .cloned()
        .collect();
    let id = |component: &str, kind: Option<&str>| {
        doc["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                let held = &entry["components"][component];
                !held.is_null() && kind.is_none_or(|kind| held["kind"] == kind)
            })
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let htm = doc["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["components"]["cohort"]["assignment"]["kind"] == "free")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let mut ops = Vec::new();
    let mut sectors = Vec::new();
    for (name, member) in [
        ("Pick stations", id("station", Some("pick"))),
        ("Put stations", id("station", Some("put"))),
        ("Inventory storage", id("sector", None)),
    ] {
        let sector = DeclaredId::mint();
        let members = if name == "Inventory storage" && !racks { Vec::new() } else { vec![member] };
        ops.push(Op::Declare {
            id: sector,
            name: None,
            components: components(json!({ "sector": { "members": members } })),
        });
        sectors.push((name, sector.to_string()));
    }
    for entry in &mut flows {
        entry["components"].as_object_mut().unwrap().remove("external_ids");
        if entry["components"]["flow.agent_segment"]["executor"].get("cohort").is_some() {
            entry["components"]["flow.agent_segment"]["executor"]["cohort"] = json!(htm);
        }
        if let Some((_, sector)) = sectors.iter().find(|(name, _)| entry["name"] == *name) {
            entry["components"]["flow.node"]["sectors"] = json!([sector]);
        }
    }
    let mut text = serde_json::to_string(&flows).unwrap();
    for entry in &flows {
        text = text.replace(entry["id"].as_str().unwrap(), &DeclaredId::mint().to_string());
    }
    let flows: Vec<Value> = serde_json::from_str(&text).unwrap();
    ops.extend(flows.into_iter().map(|entry| Op::Declare {
        id: entry["id"].as_str().unwrap().parse().unwrap(),
        name: entry["name"].as_str().map(str::to_owned),
        components: components(entry["components"].clone()),
    }));
    ops
}
