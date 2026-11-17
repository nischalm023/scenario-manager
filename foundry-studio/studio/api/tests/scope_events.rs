//! Spec 01 section 3.6's `GET /api/scope-events` against the client's in-memory mock: a changeset on a followed
//! scope reaches the page as `scope.moved` within 2 s of wall clock (spec 01 statement 2's second half); one that
//! lands before the client's own first read of the head is still told, with `gap` when more than one went untold
//! (spec 04 section 3.6); the request names one to 16 scopes world-proto-server holds; and a `ping` arrives within
//! 15 s.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use bytes::Bytes;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::journal::PING_EVERY;
use studio_api::scope_events::MOST_SCOPES;
use studio_api::{GateState, gate_routes};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq};
use support::sse::{EventReader, SseEvent};
use tower::ServiceExt;
use wps_client::draft::Op;
use wps_client::value::LogEntry;
use wps_client::wire::{
    EntityQuery, ExportFlags, RawExport, Resolution, WireApplied, WireChangeset, WireComponent, WireDefinition,
    WireEntity, WireGenerator, WireGenerators, WireImport, WireListed, WireProtocol, WireReferrer, WireScope,
    WireTrail,
};
use wps_client::wps::{EventStream, PostFailure, Wps};
use wps_client::{BlockingCompute, Client, ClientConfig, MockWps, WpsError};

const MAP: &str = "customer/demo/site/lshape-6/map";
const ROOT: &str = "customer/demo/site/lshape-6";
const WITHIN: Duration = Duration::from_secs(2);

async fn follow(router: &Router, query: &str) -> (StatusCode, EventReader) {
    let request = support::from_gate("GET", &format!("/api/scope-events{query}"), "u-1", "")
        .header("accept", "text/event-stream")
        .body(Body::empty())
        .expect("a request");
    let answer = router.clone().oneshot(request).await.expect("an answer");
    let (parts, body) = answer.into_parts();
    if parts.status == StatusCode::OK {
        assert_eq!(parts.headers.get("content-type").and_then(|v| v.to_str().ok()), Some("text/event-stream"));
    }
    (parts.status, EventReader::new(body))
}

fn frame() -> Op {
    Op::Declare {
        id: DeclaredId::mint(),
        name: None,
        components: json!({ "frame": {} }).as_object().cloned().expect("an object"),
    }
}

fn scope(text: &str) -> ScopePath {
    text.parse().expect("a scope path")
}

/// A `scope.moved` with no `id:` line, and its body.
fn moved(event: &SseEvent) -> Value {
    assert_eq!((event.kind(), event.id.as_ref()), ("scope.moved", None), "{event:?}");
    event.json()
}

#[tokio::test]
async fn a_changeset_reaches_the_stream_within_two_seconds() {
    let scratch = scratch_or_skip!();
    let gate = support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await;
    let router = gate_routes(gate.clone());
    let (status, mut stream) = follow(&router, &format!("?scope={MAP}&scope={}", ROOT.replace('/', "%2F"))).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(support::stage(&gate.world, &scope(MAP), vec![frame()], None).await, Seq::new(2));
    let event = stream.next(WITHIN).await.expect("the stream stays open");
    assert_eq!(moved(&event), json!({ "scope": MAP, "seq": 2, "gap": false }));
    support::stage(&gate.world, &scope(ROOT), vec![frame()], None).await;
    let event = stream.next(WITHIN).await.expect("the stream stays open");
    assert_eq!(moved(&event), json!({ "scope": ROOT, "seq": 2, "gap": false }));
    scratch.close().await;
}

/// A world-proto-server on which the changesets `landing` holds land as the client subscribes to a scope, before the
/// client reads the scope's head: the window a new watch leaves open (spec 04 section 3.6).
struct Racing {
    inner: MockWps,
    landing: Mutex<Vec<WireChangeset>>,
}

impl Wps for Racing {
    async fn healthz(&self) -> Result<String, WpsError> {
        self.inner.healthz().await
    }
    async fn protocol(&self) -> Result<WireProtocol, WpsError> {
        self.inner.protocol().await
    }
    async fn protocol_defs(&self) -> Result<Value, WpsError> {
        self.inner.protocol_defs().await
    }
    async fn scopes(&self) -> Result<Vec<WireScope>, WpsError> {
        self.inner.scopes().await
    }
    async fn scopes_under(&self, under: &ScopePath) -> Result<Vec<WireScope>, WpsError> {
        self.inner.scopes_under(under).await
    }
    async fn create_scope(&self, path: &ScopePath, boundary: bool) -> Result<WireScope, WpsError> {
        self.inner.create_scope(path, boundary).await
    }
    async fn export_scope(&self, scope: &ScopePath, at: Option<Seq>) -> Result<RawExport, WpsError> {
        self.inner.export_scope(scope, at).await
    }
    async fn export_flagged(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        at: &BTreeMap<ScopePath, Seq>,
    ) -> Result<Bytes, WpsError> {
        self.inner.export_flagged(scope, flags, at).await
    }
    async fn entity(&self, scope: &ScopePath, id: &EntityId, at: Option<Seq>) -> Result<WireEntity, WpsError> {
        self.inner.entity(scope, id, at).await
    }
    async fn trail(&self, scope: &ScopePath, id: &EntityId) -> Result<Option<WireTrail>, WpsError> {
        self.inner.trail(scope, id).await
    }
    async fn entities(
        &self,
        scope: &ScopePath,
        query: &EntityQuery,
    ) -> Result<(Vec<WireListed>, Option<String>), WpsError> {
        self.inner.entities(scope, query).await
    }
    async fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Option<Seq>,
        after: Option<String>,
    ) -> Result<(Vec<WireReferrer>, Option<String>), WpsError> {
        self.inner.referrers(scope, id, at, after).await
    }
    async fn resolve(&self, scope: &ScopePath, reference: &str) -> Result<Resolution, WpsError> {
        self.inner.resolve(scope, reference).await
    }
    async fn changesets(&self, scope: &ScopePath, before: Option<Seq>, limit: u16) -> Result<Vec<LogEntry>, WpsError> {
        self.inner.changesets(scope, before, limit).await
    }
    async fn changeset_named(&self, scope: &ScopePath, operation: &str) -> Result<Option<LogEntry>, WpsError> {
        self.inner.changeset_named(scope, operation).await
    }
    async fn post_changeset(&self, changeset: &WireChangeset) -> Result<WireApplied, PostFailure> {
        self.inner.post_changeset(changeset).await
    }
    async fn import(&self, scope: &ScopePath, base: Seq, document: &Value) -> Result<WireImport, WpsError> {
        self.inner.import(scope, base, document).await
    }
    async fn components(&self) -> Result<Vec<WireDefinition>, WpsError> {
        self.inner.components().await
    }
    async fn component(&self, name: &str) -> Result<Option<WireComponent>, WpsError> {
        self.inner.component(name).await
    }
    async fn generators(&self) -> Result<WireGenerators, WpsError> {
        self.inner.generators().await
    }
    async fn generator(&self, name: &str) -> Result<WireGenerator, WpsError> {
        self.inner.generator(name).await
    }
    async fn subscribe(&self, scope: &ScopePath) -> Result<EventStream, WpsError> {
        let landing: Vec<WireChangeset> = self.landing.lock().expect("the test's lock").drain(..).collect();
        for changeset in &landing {
            self.inner.post_changeset(changeset).await.expect("a changeset lands before the head is read");
        }
        self.inner.subscribe(scope).await
    }
}

async fn racing(pool: &PgPool, landing: u64) -> (GateState<Racing>, Router) {
    let racing = Racing { inner: support::mock(), landing: Mutex::new(Vec::new()) };
    let world = Client::start(racing, ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("a client");
    support::made_site(&world, ROOT, "L-Shape 6").await;
    let changesets = (1..=landing).map(|base| {
        WireChangeset::of_one_scope(
            scope(MAP),
            Seq::new(base),
            "someone-else",
            "landed as the watch began",
            vec![frame()],
        )
    });
    world.wps().landing.lock().expect("the test's lock").extend(changesets);
    let gate = GateState::new(support::state(pool.clone(), support::gate_rules()), world);
    (gate.clone(), gate_routes(gate))
}

#[tokio::test]
async fn a_changeset_landing_before_the_clients_first_head_read_is_told() {
    let scratch = scratch_or_skip!();
    for (landing, gap) in [(1, false), (2, true)] {
        let (gate, router) = racing(&scratch.pool, landing).await;
        let (status, mut stream) = follow(&router, &format!("?scope={MAP}")).await;
        assert_eq!(status, StatusCode::OK);
        let event = stream.next(WITHIN).await.expect("the stream stays open");
        assert_eq!(moved(&event), json!({ "scope": MAP, "seq": 1 + landing, "gap": gap }), "{landing} landed");
        assert!(gate.world.wps().landing.lock().expect("the test's lock").is_empty());
    }
    scratch.close().await;
}

#[tokio::test]
async fn the_request_names_one_to_sixteen_scopes_world_proto_server_holds() {
    let scratch = scratch_or_skip!();
    let router = gate_routes(support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await);
    let too_many: Vec<String> = (0..=MOST_SCOPES).map(|n| format!("scope={ROOT}/s{n}")).collect();
    for (query, status, slug) in [
        (String::new(), StatusCode::UNPROCESSABLE_ENTITY, "validation-failed"),
        ("?scopes=customer".to_owned(), StatusCode::UNPROCESSABLE_ENTITY, "validation-failed"),
        (format!("?{}", too_many.join("&")), StatusCode::UNPROCESSABLE_ENTITY, "validation-failed"),
        ("?scope=Not%20a%20scope".to_owned(), StatusCode::UNPROCESSABLE_ENTITY, "validation-failed"),
        ("?scope=customer/demo/site/nowhere".to_owned(), StatusCode::NOT_FOUND, "not-found"),
    ] {
        let (answered, problem) =
            support::json(&router, "GET", &format!("/api/scope-events{query}"), "u-1", None).await;
        assert_eq!((answered, problem["type"].as_str()), (status, Some(slug)), "{query}");
    }
    let twice: Vec<String> = (0..MOST_SCOPES).map(|_| format!("scope={MAP}&scope={ROOT}")).collect();
    let (status, _) = follow(&router, &format!("?{}", twice.join("&"))).await;
    assert_eq!(status, StatusCode::OK, "a scope named again counts once");
    scratch.close().await;
}

#[tokio::test]
async fn a_ping_arrives_within_15_s() {
    let scratch = scratch_or_skip!();
    let router = gate_routes(support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await);
    let (_, mut stream) = follow(&router, &format!("?scope={MAP}")).await;
    let ping = stream.next(PING_EVERY + Duration::from_secs(1)).await.expect("a ping");
    assert_eq!((ping.kind(), ping.id.as_ref()), ("ping", None));
    assert!(ping.json()["at"].is_string(), "{ping:?}");
    scratch.close().await;
}
