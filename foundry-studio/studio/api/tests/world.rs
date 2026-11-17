//! The service's world-proto-server client as the service drives it (spec 04 sections 3.2 and 3.10): the routes read
//! the world graph through it, every route test through the client's in-memory mock, serving keeps its schema cache
//! fresh at the client's own cadence, and each of its errors reaches a person as spec 01 section 3.14's problem
//! (spec 04 section 3.8).

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::to_bytes;
use axum::response::IntoResponse;
use serde_json::{Value, json};
use studio_api::problem::wps_problem;
use studio_api::{GateState, serve};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq};
use tokio::net::TcpListener;
use tokio::time::sleep;
use wps_client::refusal::Unresolved;
use wps_client::{
    BlockingCompute, Capability, Client, ClientConfig, GeneratorSet, MockWps, Problem, ProblemKind, WorldStore,
    WpsError,
};

const COMPONENT: &str = "template.description";

fn latest(world: &Client<MockWps>) -> Option<String> {
    world.registry().snapshot().components.get(COMPONENT)?.latest.as_ref().map(ToString::to_string)
}

/// Spec 04 section 3.10: the client loads the schema cache when it starts, and the service, once it serves, has it
/// revalidated at the client's cadence, so a version published after the start reaches the cache with no refusal.
#[tokio::test]
async fn serving_revalidates_the_schema_cache_at_the_clients_cadence() {
    let mock = MockWps::phase_3();
    let open = json!({ "type": "object" });
    mock.publish(COMPONENT, "0.1.0", open.clone(), Vec::new());
    let every = Duration::from_millis(50);
    let config =
        ClientConfig { preload: vec![COMPONENT.to_owned()], revalidate_every: every, ..ClientConfig::default() };
    let world = Client::start(mock, config, Arc::new(BlockingCompute)).await.expect("a client of the mock");
    assert_eq!(latest(&world).as_deref(), Some("0.1.0"), "loaded at the start");
    let state = GateState::new(support::state(support::unconnected_pool(), support::gate_rules()), world);
    let world = state.world.clone();

    world.wps().publish(COMPONENT, "0.2.0", open, Vec::new());
    sleep(every * 4).await;
    assert_eq!(latest(&world).as_deref(), Some("0.1.0"), "nothing revalidates before the service serves");

    let gate = TcpListener::bind("127.0.0.1:0").await.expect("a port on loopback");
    let runners = TcpListener::bind("127.0.0.1:0").await.expect("another port on loopback");
    tokio::spawn(serve(gate, runners, state));
    let began = Instant::now();
    while latest(&world).as_deref() != Some("0.2.0") {
        assert!(began.elapsed() < Duration::from_secs(5), "the published version never reached the cache");
        sleep(every).await;
    }
    assert!(world.capabilities().await.is_ok(), "the client still answers");
}

fn scope(text: &str) -> ScopePath {
    text.parse().expect("a scope path")
}

/// The answer `error` makes: its status, its `Retry-After` and its body.
async fn answered(error: &WpsError) -> (u16, Option<String>, Value) {
    let answer = wps_problem(error, "/api/test").into_response();
    let status = answer.status().as_u16();
    let wait = answer.headers().get("retry-after").map(|wait| wait.to_str().expect("text").to_owned());
    let body = to_bytes(answer.into_body(), usize::MAX).await.expect("the body reads");
    (status, wait, serde_json::from_slice(&body).expect("a problem body"))
}

/// Spec 04 section 3.8's table: every error of the client answers the spec 01 problem type and status it names.
#[tokio::test]
async fn every_client_error_answers_its_problem_type() {
    let map = scope("customer/demo/site/lshape-6/map");
    let refusal = Problem::new(ProblemKind::Dangling, "it names nothing");
    let cases = [
        (WpsError::HeadMoved { moved: vec![(map.clone(), Seq::new(3), Seq::new(4))] }, "head-moved", 409),
        (
            WpsError::GeneratorsMoved { expected: GeneratorSet::default(), found: GeneratorSet::default() },
            "generators-moved",
            409,
        ),
        (WpsError::RegistryMoved { scope: map.clone(), text: "kept moving".to_owned() }, "generators-moved", 409),
        (WpsError::Refused { problem: Box::new(refusal.clone()) }, "validation-failed", 422),
        (WpsError::Problems { problems: vec![refusal.clone()] }, "validation-failed", 422),
        (WpsError::TooLarge { bytes: 2, limit: 1 }, "validation-failed", 422),
        (WpsError::ExportNotClosed { problems: vec![refusal] }, "export-not-closed", 422),
        (WpsError::NotFound { text: "no such entity".to_owned() }, "not-found", 404),
        (WpsError::Unresolvable { unresolved: Unresolved::Undeclared, text: "no one".to_owned() }, "not-found", 404),
        (
            WpsError::Unresolvable {
                unresolved: Unresolved::Owned { owner: map.clone(), problem: None },
                text: "a pin within one unit".to_owned(),
            },
            "validation-failed",
            422,
        ),
        (WpsError::PinUnresolved { scope: map.clone(), text: "no changeset 12".to_owned() }, "pin-unresolved", 404),
        (WpsError::ScopeExists { scope: map.clone(), head: Seq::new(2) }, "site-exists", 409),
        (WpsError::Unsupported { capability: Capability::Tags }, "wps-not-ready", 501),
        (WpsError::Incompatible { text: "protocol 0.8".to_owned() }, "wps-not-ready", 501),
        (WpsError::Unavailable { attempts: 3, last: "refused".to_owned() }, "wps-unavailable", 503),
        (WpsError::OutcomeUnknown { scopes: vec![map.clone()] }, "wps-outcome-unknown", 502),
        (WpsError::ServerFault { status: 500, text: "a module fell over".to_owned() }, "wps-fault", 502),
        (WpsError::Invalid { text: "a malformed request".to_owned() }, "internal", 500),
        (
            WpsError::OperationReused {
                operation: "01JAZ0".to_owned(),
                initiated: map.clone(),
                named: vec![(map.clone(), Seq::new(2))],
            },
            "internal",
            500,
        ),
        (
            WpsError::Decode {
                entity: EntityId::from(DeclaredId::mint()),
                component: "frame".to_owned(),
                detail: "x".to_owned(),
            },
            "internal",
            500,
        ),
        (WpsError::ComputeBusy { retry_after: Duration::from_millis(1500) }, "compute-busy", 503),
        (WpsError::ComputeLost, "internal", 500),
    ];
    for (error, slug, status) in cases {
        let (answered_status, _, body) = answered(&error).await;
        assert_eq!(
            (answered_status, body["type"].as_str(), body["status"].as_u64()),
            (status, Some(slug), Some(u64::from(status))),
            "{error:?}"
        );
        assert_eq!(body["instance"], "/api/test");
    }
}

/// Spec 01 section 3.14's extensions as the client fills them: `moved` with `head-moved`, each refusal an error item
/// naming its entity and field, the scopes whose write is unknown, and the wait on `compute-busy` in the body and in
/// `Retry-After`; a fault in the client itself keeps its internals out.
#[tokio::test]
async fn the_extensions_carry_what_the_client_knows() {
    let map = scope("customer/demo/site/lshape-6/map");
    let (_, _, moved) = answered(&WpsError::HeadMoved { moved: vec![(map.clone(), Seq::new(3), Seq::new(4))] }).await;
    assert_eq!(moved["moved"], json!([{ "scope": map.as_str(), "base": 3, "head": 4 }]));

    let entity = EntityId::from(DeclaredId::mint());
    let refusal = Problem::new(ProblemKind::Dangling, "flow node 3f21 names nothing")
        .on(entity.clone())
        .in_scope(&map)
        .at_field("flow.node", "/sectors/0");
    let (_, _, refused) = answered(&WpsError::Refused { problem: Box::new(refusal) }).await;
    let item = json!({ "code": "dangling", "severity": "error", "ref": entity.to_string(), "scope": map.as_str(), "field": "/flow.node/sectors/0", "text": "flow node 3f21 names nothing" });
    assert_eq!(refused["problems"], json!([item]));

    let (_, _, unknown) = answered(&WpsError::OutcomeUnknown { scopes: vec![map.clone()] }).await;
    assert_eq!(unknown["uncertain"], json!([map.as_str()]));

    let (_, wait, busy) = answered(&WpsError::ComputeBusy { retry_after: Duration::from_millis(1500) }).await;
    assert_eq!((wait.as_deref(), busy["retry_after_s"].as_u64()), (Some("2"), Some(2)), "rounded up to whole seconds");

    let (_, wait, fault) = answered(&WpsError::Invalid { text: "the secret internals".to_owned() }).await;
    assert!(wait.is_none() && !fault.to_string().contains("secret internals"), "{fault}");
    for absent in ["problems", "moved", "uncertain", "retry_after_s"] {
        assert!(fault.get(absent).is_none(), "{absent} is left out when empty");
    }
}
