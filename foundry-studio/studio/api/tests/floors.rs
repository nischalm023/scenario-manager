//! `GET /api/floors/{key}` (spec 03 section 3.6): a site's floor from the client's floor cache and an evaluation's
//! from the run-object store, each as the floor document's media type and immutable, and nothing else of the store's.
//! The route is called without the identity layer, which needs the database and is not this route's.

mod support;

use std::fs;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use runstore::RunStorage;
use serde_json::Value;
use studio_api::GateState;
use studio_api::floors::{build_commit, floor_key, key_text, routes, site_floor};
use wps_client::{MockWps, WorldStore};

fn made_l_shape() -> Bytes {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/tests/fixtures/l_shape/world.json");
    Bytes::from(fs::read(path).expect("the viewer's fixture is checked in"))
}

async fn floors() -> (GateState<MockWps>, Router) {
    let gate =
        GateState::new(support::state(support::unconnected_pool(), support::gate_rules()), support::world().await);
    (gate.clone(), routes().with_state(gate))
}

async fn get(router: &Router, uri: &str) -> (StatusCode, Option<String>, Option<String>, Vec<u8>) {
    let request = Request::builder().uri(uri).body(Body::empty()).expect("a request");
    let (status, headers, body) = support::call(router, request).await;
    let header = |name: &str| headers.get(name).map(|value| value.to_str().expect("text").to_owned());
    (status, header("content-type"), header("cache-control"), body)
}

/// A site's floor, once compiled under its key, is the floor document under that key, immutable.
#[tokio::test]
async fn a_held_site_floor_is_served_by_its_key_and_never_changes() {
    let (gate, router) = floors().await;
    let key = floor_key("9f86d081884c7d65", build_commit(), None);
    let compiled = site_floor(gate.world.floors(), gate.app.compute.clone(), key.clone(), made_l_shape())
        .await
        .expect("the made L-Shape compiles");
    let (status, kind, cache, body) = get(&router, &format!("/api/floors/{}", key_text(&key))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind.as_deref(), Some("application/vnd.worldview-floor+msgpack"));
    assert!(cache.as_deref().is_some_and(|cache| cache.contains("immutable")), "{cache:?}");
    assert_eq!(body, compiled.to_vec());
}

/// An evaluation's floor, and a template card's default one (ruling D121: the card's picture is drawn from it), are
/// read from where spec 05 keeps them.
#[tokio::test]
async fn an_evaluations_floor_is_served_from_the_run_object_store() {
    let (gate, router) = floors().await;
    for key in [
        "template-drafts/01KN0TJZANP5N0PGJD8F7A3FDB/3/floor.msgpack",
        "template-thumbnails/01KN0TJZANP5N0PGJD8F7A3FDB/floor.msgpack",
    ] {
        gate.app.runstore.put(key, Bytes::from(key.to_owned())).await.expect("kept");
        let (status, kind, _, body) = get(&router, &format!("/api/floors/{key}")).await;
        assert_eq!(
            (status, kind.as_deref()),
            (StatusCode::OK, Some("application/vnd.worldview-floor+msgpack")),
            "{key}"
        );
        assert_eq!(body, key.as_bytes());
    }
}

/// A key no summary named, or any other object of the store, is no floor.
#[tokio::test]
async fn a_key_naming_no_floor_is_not_found() {
    let (gate, router) = floors().await;
    gate.app.runstore.put("template-thumbnails/7/plan.json", Bytes::from_static(b"a plan")).await.unwrap();
    let unheld = key_text(&floor_key("ab12", build_commit(), None));
    for path in [unheld.as_str(), "template-thumbnails/7/plan.json", "template-drafts/x/1/floor.msgpack", "nothing"] {
        let (status, kind, _, body) = get(&router, &format!("/api/floors/{path}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(kind.as_deref(), Some("application/problem+json"));
        let problem: Value = serde_json::from_slice(&body).expect("a problem");
        assert_eq!(problem["type"], "not-found", "{path}");
    }
}

/// The compiler's identity in a key is one dot-free segment, whatever the describe or the environment said.
#[test]
fn the_build_commit_is_one_segment_of_a_key() {
    let build = build_commit();
    assert!(!build.is_empty() && build.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{build}");
}
