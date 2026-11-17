//! The L-Shape made at 6 aisles and 54,400 mm as Create site writes a site, which the flow and draft tests read
//! through the API's router against the client's mock seeded by the development seed; beside it a site whose flow
//! sub-scope is empty and one that has none.

use std::fs;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use serde_json::{Value, json};
use site_template::{
    DocId, KnobValues, Mint, Prior, Prologue, RefAnnotation, RegisteredComponent, RegistrySnapshot, RelativeScope,
    ScopeSnapshot, TemplateRead, evaluate,
};
use sqlx::PgPool;
use studio_api::projects::make_local_default;
use studio_api::{GateState, gate_routes};
use studio_protocol::{DeclaredId, ScopePath, Seq};
use wps_client::site::ensure_prefixes;
use wps_client::wps::Wps;
use wps_client::{Client, MockWps, Op, WorldStore};

use super::templates::{registered, repo, seeded};

pub const SITE: &str = "customer/demo/site/lshape-6";

/// A site with a map and a fleet and no flow sub-scope at all.
pub const FLOWLESS: &str = "customer/demo/site/flowless";

pub fn json_file(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()))
}

/// The registry the engine validates against, as site-template's own tests hold it.
pub fn registry() -> RegistrySnapshot {
    let fixtures = repo().join("studio/site-template/tests/fixtures");
    let mut files: Vec<PathBuf> =
        fs::read_dir(fixtures.join("vocabulary")).unwrap().map(|entry| entry.unwrap().path()).collect();
    files.sort();
    let mut components: Vec<RegisteredComponent> = (files.iter().map(|path| json_file(path)))
        .map(|definition| RegisteredComponent {
            name: definition["component"].as_str().unwrap().to_owned(),
            version: "0.1.0".into(),
            schema: Some(definition["schema"].clone()),
            refs: serde_json::from_value::<Vec<RefAnnotation>>(definition["refs"].clone()).unwrap(),
        })
        .collect();
    let core = json_file(&fixtures.join("registry/core.json"));
    for (name, entry) in core["components"].as_object().unwrap() {
        components.push(RegisteredComponent {
            name: name.clone(),
            version: entry["version"].as_str().unwrap().to_owned(),
            schema: entry.get("schema").cloned(),
            refs: serde_json::from_value(entry["refs"].clone()).unwrap(),
        });
    }
    RegistrySnapshot::new(components, Some(&json_file(&fixtures.join("registry/server-defs.json")))).unwrap()
}

/// The L-Shape template read whole from the development seed, each scope at changeset 1, as the seeded mock holds it.
pub fn l_shape() -> TemplateRead {
    let seed = repo().join("data/seed/studio");
    let root: ScopePath = "template/site/l-shape".parse().unwrap();
    let manifest = json_file(&seed.join("manifest.json"));
    let entries: Vec<(ScopePath, Value)> = (manifest["entries"].as_array().unwrap().iter())
        .map(|e| {
            (e["scope"].as_str().unwrap().parse().unwrap(), json_file(&seed.join(e["document"].as_str().unwrap())))
        })
        .collect();
    let snapshot = |(scope, document): &(ScopePath, Value)| {
        ScopeSnapshot::from_document(scope.clone(), Seq::new(1), document).unwrap()
    };
    let scopes: Vec<ScopeSnapshot> = entries.iter().filter(|(scope, _)| root.holds(scope)).map(snapshot).collect();
    let top = scopes.iter().find(|snapshot| snapshot.scope == root).unwrap();
    let wanted = top.prologue.as_ref().unwrap().depends_on.clone();
    let catalog = (entries.iter().filter(|(scope, _)| scope.as_str().starts_with("catalog/")))
        .map(snapshot)
        .filter(|snapshot| wanted.contains(&snapshot.prologue.as_ref().unwrap().doc))
        .collect();
    TemplateRead { template: root, tag: None, scopes, catalog, registry: registry() }
}

/// The L-Shape at 6 aisles and 54,400 mm written as Create site writes a site: its scopes made, each made scope's
/// declarations staged, the root's prologue last.
pub async fn made_l_shape<S: Wps>(world: &Client<S>) {
    let (stages, prologue) = made_stages(&Prior::new(DocId::mint()));
    let root: ScopePath = SITE.parse().unwrap();
    super::made_scopes(world, &root).await;
    let frame: DeclaredId = prologue.primary_frame.as_ref().unwrap().to_string().parse().unwrap();
    for (scope, ops) in stages {
        let scope = if scope == "." { root.clone() } else { root.join(&scope.parse().unwrap()).unwrap() };
        let stated = (scope == root).then(|| super::prologue(NAME, frame));
        super::stage(world, &scope, ops, stated).await;
    }
}

const NAME: &str = "Small L-shape, 6 aisles";

/// The L-Shape at 6 aisles and 54,400 mm as Create site stages it, each scope's declarations in its staging order
/// (map, fleet, flow, then the root), and the root's prologue; each made entity keeps the id `prior` gives its key.
pub fn made_stages(prior: &Prior) -> (Vec<(&'static str, Vec<Op>)>, Prologue) {
    let read = l_shape();
    let knobs = KnobValues::new().with("aisles", 6).with("width", 54400);
    let evaluation = evaluate(&read, &knobs, prior, &mut Mint);
    assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
    let ops = |scope: &str| -> Vec<Op> {
        let plan = evaluation.plan.get(&RelativeScope::new(scope));
        (plan.into_iter().flat_map(|plan| &plan.declarations))
            .map(|made| Op::Declare { id: made.id, name: made.name.clone(), components: made.components.clone() })
            .collect()
    };
    let stages = ["map", "fleet", "flow", "."].map(|scope| (scope, ops(scope))).into();
    (stages, Prologue { name: Some(NAME.into()), ..evaluation.prologue })
}

/// A site made with its root, map and fleet, and no flow sub-scope: a frame in its map, the root's prologue naming it.
async fn made_without_flow<S: Wps>(world: &Client<S>) {
    let root: ScopePath = FLOWLESS.parse().unwrap();
    ensure_prefixes(world, &root).await.expect("the root's prefixes");
    world.ensure_scope(&root, true).await.expect("the root");
    for sub in ["map", "fleet"] {
        world.ensure_scope(&root.join(&sub.parse().unwrap()).unwrap(), false).await.expect("a sub-scope");
    }
    let frame = DeclaredId::mint();
    let components = json!({ "frame": {} }).as_object().cloned().unwrap();
    let map = root.join(&"map".parse().unwrap()).unwrap();
    super::stage(world, &map, vec![Op::Declare { id: frame, name: None, components }], None).await;
    super::stage(world, &root, Vec::new(), Some(super::prologue("No flow scope", frame))).await;
}

/// The service over the mock holding the made L-Shape, the bare site and the flowless one, and its gate-facing
/// router.
pub async fn ready(pool: &PgPool) -> (GateState<MockWps>, Router) {
    make_local_default(pool).await.expect("the default project");
    let mock = registered();
    // site-template's registry fixture leaves out the protocol's `rack`, which every bay of an expanded site carries.
    mock.publish("rack", "0.7.0", json!({}), Vec::new());
    let world = seeded(mock).await;
    made_l_shape(&world).await;
    super::made_site(&world, "customer/demo/site/bare", "No flow yet").await;
    made_without_flow(&world).await;
    let gate = GateState::new(super::state(pool.clone(), super::gate_rules()), world);
    (gate.clone(), gate_routes(gate))
}

/// A request as the person `subject`, with a fresh Idempotency-Key, answered as JSON.
pub async fn send_as(
    router: &Router,
    subject: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = super::from_gate(method, uri, subject, "")
        .header("content-type", "application/json")
        .header("idempotency-key", format!("k{}", DeclaredId::mint()));
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, _, answer) = super::call(router, request.body(body).unwrap()).await;
    (status, serde_json::from_slice(&answer).unwrap_or(Value::Null))
}

/// A request as the person `dev`.
pub async fn call(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_as(router, "dev", method, uri, body).await
}
