//! What the unit tests share: a client over the in-memory mock, its registry holding the components the tests write
//! with the annotations WPS's demo seed gives them, and a site of the sub-scope layout to write into.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use studio_protocol::{DeclaredId, Pin, ScopePath, Seq};
use wps_client::draft::Op;
use wps_client::site::{SUB_SCOPES, ensure_prefixes};
use wps_client::wire::{RefAnnotation, WireChangeset};
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client, ClientConfig, Compute, MockWps, UnitSnapshot, WorldStore};

pub fn path(text: &str) -> ScopePath {
    text.parse().expect("a scope path")
}

pub fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

fn refs(paths: &[&str]) -> Vec<RefAnnotation> {
    paths.iter().map(|path| RefAnnotation { path: (*path).to_owned(), expects: None }).collect()
}

/// The mock's registry: an open object schema for each, with the reference annotations WPS's demo seed states.
pub fn register(mock: &MockWps) {
    let open = json!({ "type": "object" });
    mock.publish("frame", "0.7.0", open.clone(), refs(&["/parent"]));
    mock.publish("pose", "0.7.0", open.clone(), refs(&["/frame"]));
    mock.publish("cell", "0.7.0", open.clone(), Vec::new());
    mock.publish("sector", "0.7.0", open.clone(), refs(&["/members/*"]));
    mock.publish("stitch", "0.7.0", open.clone(), refs(&["/a", "/b"]));
    mock.publish("template", "0.7.0", open.clone(), Vec::new());
    mock.publish("stamp", "0.7.0", open.clone(), refs(&["/template"]));
    mock.publish("robot_class", "0.7.0", open.clone(), Vec::new());
    mock.publish("cohort", "0.7.0", open.clone(), refs(&["/robot_class"]));
    mock.publish("external_ids", "0.7.0", open.clone(), Vec::new());
    mock.publish("flow.node", "0.1.0", open.clone(), refs(&["/sectors/*"]));
    mock.publish("flow.return_rule", "0.1.0", open, Vec::new());
    let description = json!({
        "type": "object",
        "required": ["title", "text"],
        "properties": { "title": { "type": "string", "maxLength": 80 }, "text": { "type": "string" } },
        "additionalProperties": false
    });
    mock.publish("template.description", "0.1.0", description, Vec::new());
    mock.publish_generator("tiling", "1.0.0", "digest-tiling-1");
}

/// A test's configuration: the schemas the tests write preloaded, and a lost event stream reconnected at once.
pub fn config() -> ClientConfig {
    ClientConfig {
        preload: vec!["template.description".to_owned(), "flow.node".to_owned()],
        reconnect_first: Duration::from_millis(50),
        reconnect_last: Duration::from_millis(200),
        ..ClientConfig::default()
    }
}

pub async fn client_of(mock: MockWps) -> Client<MockWps> {
    register(&mock);
    Client::start(mock, config(), Arc::new(BlockingCompute)).await.expect("the mock starts a client")
}

/// A client of the mock whose computations run on `pool`.
pub async fn client_on(pool: Arc<dyn Compute>) -> Client<MockWps> {
    let mock = MockWps::phase_3();
    register(&mock);
    Client::start(mock, config(), pool).await.expect("the mock starts a client")
}

pub async fn client() -> Client<MockWps> {
    client_of(MockWps::phase_3()).await
}

/// A site's scopes, empty at changeset 0, made by name as a fixture: its root `customer/c/site/{name}` and the root's
/// `map`, `flow` and `fleet`. Create site itself makes and writes them in one changeset (`site::create_site`).
pub async fn site(client: &Client<MockWps>, name: &str) -> ScopePath {
    let root = path(&format!("customer/c/site/{name}"));
    ensure_prefixes(client, &root).await.expect("the prefixes");
    client.ensure_scope(&root, true).await.expect("the root");
    for sub in SUB_SCOPES {
        client.ensure_scope(&path(&format!("{root}/{sub}")), false).await.expect("a sub-scope");
    }
    root
}

/// A catalog item's one scope, `catalog/agent/{name}`.
pub async fn catalog(client: &Client<MockWps>, name: &str) -> ScopePath {
    let item = path(&format!("catalog/agent/{name}"));
    for prefix in ["catalog", "catalog/agent"] {
        client.ensure_scope(&path(prefix), false).await.expect("a catalog prefix");
    }
    client.ensure_scope(&item, true).await.expect("the catalog item");
    item
}

/// Write `ops` into `scope` directly, at its head, as another writer would.
pub async fn write(client: &Client<MockWps>, scope: &ScopePath, ops: Vec<Op>) -> Seq {
    let head = client
        .list_scopes(scope)
        .await
        .expect("a listing")
        .into_iter()
        .find(|s| &s.path == scope)
        .expect("the scope")
        .head;
    let changeset = WireChangeset::of_one_scope(scope.clone(), head, "someone-else", "written by hand", ops);
    client.wps().post_changeset(&changeset).await.expect("the write lands").seq.expect("one scope written")
}

pub fn declare(id: DeclaredId, components: Value) -> Op {
    Op::Declare { id, name: None, components: object(components) }
}

/// The site's scopes read at their heads.
pub async fn read_site(client: &Client<MockWps>, root: &ScopePath) -> UnitSnapshot {
    let cut = client.resolve_cut(root, &Pin::Branch("main".parse().expect("main"))).await.expect("a cut");
    client.read_unit(&cut).await.expect("the site's scopes")
}

pub fn bases(snapshot: &UnitSnapshot) -> BTreeMap<ScopePath, Seq> {
    snapshot.cut.seqs.clone()
}
