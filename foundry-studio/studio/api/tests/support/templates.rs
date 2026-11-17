//! What the template tests share (brief I1.07b): a mock world-proto-server of scopes phase 3 whose registry defines
//! every component the development seed carries (each schema left open, since the mock's definitions document is
//! empty, and each with the annotations the engine's fixtures hold), seeded from `data/seed/studio/manifest.json` by the seed
//! itself; a server, mock or live, that holds every changeset creating or writing a scope the test names until the
//! test lets them go; and the waits and reads the tests repeat.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use bytes::Bytes;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::projects::make_local_default;
use studio_api::{GateState, gate_routes};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq};
use tokio::sync::Notify;
use tokio::time::{Instant, sleep};
use wps_client::seed::run as seed;
use wps_client::value::LogEntry;
use wps_client::wire::{
    EntityQuery, ExportFlags, RawExport, RefAnnotation, Resolution, WireApplied, WireChangeset, WireComponent,
    WireDefinition, WireEntity, WireGenerator, WireGenerators, WireImport, WireListed, WireProtocol, WireReferrer,
    WireScope, WireTrail,
};
use wps_client::wps::{EventStream, PostFailure, Wps};
use wps_client::{BlockingCompute, Client, ClientConfig, Compute, MockWps, WpsError};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json_file(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()))
}

fn refs(of: &Value) -> Vec<RefAnnotation> {
    serde_json::from_value(of.clone()).expect("reference annotations")
}

/// A mock of world-proto cd4923516 generating with the native expansion, whose registry defines Agent Studio's eleven
/// components at 0.1.0, the protocol components the seed carries, each with its annotations, and `rack`, which the
/// relay bay's stamp makes in every shell's site, so a made site's export closes.
pub fn registered() -> MockWps {
    let mock = MockWps::phase_3();
    super::generation::native(&mock);
    let fixtures = repo().join("studio/site-template/tests/fixtures");
    for entry in fs::read_dir(fixtures.join("vocabulary")).expect("the vocabulary fixtures") {
        let definition = json_file(&entry.expect("a file").path());
        let name = definition["component"].as_str().expect("a component name");
        mock.publish(name, "0.1.0", json!({}), refs(&definition["refs"]));
    }
    let core = json_file(&fixtures.join("registry/core.json"));
    for (name, entry) in core["components"].as_object().expect("the core components") {
        let version = entry["version"].as_str().unwrap_or("0.7.0");
        mock.publish(name, version, json!({}), refs(&entry["refs"]));
    }
    mock.publish("rack", "0.7.0", json!({}), Vec::new());
    mock
}

/// A client of `wps` with the development seed written into it.
pub async fn seeded<S: Wps>(wps: S) -> Client<S> {
    let client = Client::start(wps, ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("the client");
    let manifest = repo().join("data/seed/studio/manifest.json");
    seed(&client, &manifest, None, "tests").await.expect("the development seed writes into the mock");
    client
}

/// A server that holds every changeset creating or writing the scope `hold` names, unanswered, until `open` lets them
/// go; that counts the changesets posted under each operation identifier; that can fail every schema read; and that
/// can hold every resolution of a reference, which the client's check asks for each reference a draft does not
/// declare itself. Two of them can speak for one world (`sharing`), as a restarted API's client and its stopped
/// predecessor's do.
pub struct Held<W = MockWps> {
    pub inner: Arc<W>,
    held: Mutex<Option<ScopePath>>,
    gate: Notify,
    posted: Mutex<Vec<Option<String>>>,
    schemas_fail: AtomicBool,
    schema_reads: AtomicUsize,
    resolves: ResolveHold,
}

/// The hold on resolutions: whether it is on, what lets it go, and how many it held in all and holds now.
#[derive(Default)]
struct ResolveHold {
    on: AtomicBool,
    gate: Notify,
    held: AtomicUsize,
    waiting: AtomicUsize,
}

/// A hold on resolutions, which lets every held one go, and holds no more, when it is dropped.
pub struct ResolvesHeld<'a>(&'a ResolveHold);

impl Drop for ResolvesHeld<'_> {
    fn drop(&mut self) {
        self.0.on.store(false, Ordering::SeqCst);
        self.0.gate.notify_waiters();
    }
}

/// One resolution the hold keeps waiting, counted until it is let go or dropped unanswered.
struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<W: Wps> Held<W> {
    pub fn new(inner: W) -> Self {
        Self::over(Arc::new(inner))
    }

    fn over(inner: Arc<W>) -> Self {
        Self {
            inner,
            held: Mutex::new(None),
            gate: Notify::new(),
            posted: Mutex::new(Vec::new()),
            schemas_fail: AtomicBool::new(false),
            schema_reads: AtomicUsize::new(0),
            resolves: ResolveHold::default(),
        }
    }

    /// Another server over the same world, holding nothing and counting afresh.
    pub fn sharing(&self) -> Self {
        Self::over(self.inner.clone())
    }

    /// Hold the changesets that create or write `scope` from now on.
    pub fn hold(&self, scope: &ScopePath) {
        *self.held.lock().expect("the hold") = Some(scope.clone());
    }

    /// Let every held changeset go, and hold no more.
    pub fn open(&self) {
        *self.held.lock().expect("the hold") = None;
        self.gate.notify_waiters();
    }

    /// How many changesets naming `operation` were posted through this server, held ones included.
    pub fn posted(&self, operation: &str) -> usize {
        self.posted.lock().expect("the count").iter().filter(|named| named.as_deref() == Some(operation)).count()
    }

    /// Answer every schema read from now on as a server that cannot be reached.
    pub fn fail_schema_reads(&self) {
        self.schemas_fail.store(true, Ordering::SeqCst);
    }

    /// How many schema reads were asked of this server since it failed them.
    pub fn schema_reads(&self) -> usize {
        self.schema_reads.load(Ordering::SeqCst)
    }

    /// Hold every resolution of a reference from now on, unanswered, until the hold this answers is dropped: a failing
    /// test's unwinding drops it too, so no thread is left waiting on a hold nobody will let go.
    pub fn hold_resolves(&self) -> ResolvesHeld<'_> {
        self.resolves.on.store(true, Ordering::SeqCst);
        ResolvesHeld(&self.resolves)
    }

    /// How many resolutions the hold has kept waiting, in all.
    pub fn resolves_held(&self) -> usize {
        self.resolves.held.load(Ordering::SeqCst)
    }

    /// How many resolutions wait in the hold now: one its asker gave up on stops waiting.
    pub fn resolves_waiting(&self) -> usize {
        self.resolves.waiting.load(Ordering::SeqCst)
    }
}

impl<W: Wps> Wps for Held<W> {
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
        if self.resolves.on.load(Ordering::SeqCst) {
            self.resolves.held.fetch_add(1, Ordering::SeqCst);
            self.resolves.waiting.fetch_add(1, Ordering::SeqCst);
            let _waiting = Waiting(&self.resolves.waiting);
            loop {
                let opened = self.resolves.gate.notified();
                if !self.resolves.on.load(Ordering::SeqCst) {
                    break;
                }
                opened.await;
            }
        }
        self.inner.resolve(scope, reference).await
    }
    async fn changesets(&self, scope: &ScopePath, before: Option<Seq>, limit: u16) -> Result<Vec<LogEntry>, WpsError> {
        self.inner.changesets(scope, before, limit).await
    }
    async fn changeset_named(&self, scope: &ScopePath, operation: &str) -> Result<Option<LogEntry>, WpsError> {
        self.inner.changeset_named(scope, operation).await
    }
    async fn post_changeset(&self, changeset: &WireChangeset) -> Result<WireApplied, PostFailure> {
        self.posted.lock().expect("the count").push(changeset.operation.clone());
        loop {
            let waiting = self.gate.notified();
            let held = self.held.lock().expect("the hold").clone();
            let touches = |scope: &ScopePath| {
                changeset.scope == *scope
                    || changeset.create.iter().any(|new| new.path == *scope)
                    || changeset.ops.iter().any(|op| op.scope.as_ref() == Some(scope))
            };
            if !held.as_ref().is_some_and(touches) {
                break;
            }
            waiting.await;
        }
        self.inner.post_changeset(changeset).await
    }
    async fn import(&self, scope: &ScopePath, base: Seq, document: &Value) -> Result<WireImport, WpsError> {
        self.inner.import(scope, base, document).await
    }
    async fn components(&self) -> Result<Vec<WireDefinition>, WpsError> {
        self.inner.components().await
    }
    async fn component(&self, name: &str) -> Result<Option<WireComponent>, WpsError> {
        if self.schemas_fail.load(Ordering::SeqCst) {
            self.schema_reads.fetch_add(1, Ordering::SeqCst);
            let last = format!("the test's server answers 503 to the schema read of {name}");
            return Err(WpsError::Unavailable { attempts: 3, last });
        }
        self.inner.component(name).await
    }
    async fn generators(&self) -> Result<WireGenerators, WpsError> {
        self.inner.generators().await
    }
    async fn generator(&self, name: &str) -> Result<WireGenerator, WpsError> {
        self.inner.generator(name).await
    }
    async fn subscribe(&self, scope: &ScopePath) -> Result<EventStream, WpsError> {
        self.inner.subscribe(scope).await
    }
}

/// Wait, up to 60 s of wall clock, until `read` answers `Some`.
pub async fn until<T, F: Future<Output = Option<T>>>(what: &str, mut read: impl FnMut() -> F) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(found) = read().await {
            return found;
        }
        assert!(Instant::now() < deadline, "waited 60 s for {what}");
        sleep(Duration::from_millis(50)).await;
    }
}

/// The draft as `GET /api/template-drafts/{d}` answers it, once its newest evaluation has ended.
pub async fn settled(router: &Router, draft: &str) -> Value {
    until("the draft's newest evaluation to end", || async {
        let (_, read) = super::json(router, "GET", &format!("/api/template-drafts/{draft}"), "dev", None).await;
        let state = read["latest"]["state"].as_str().unwrap_or_default().to_owned();
        (state != "in-progress").then_some(read)
    })
    .await
}

/// The draft once its write has ended: `created`, or `open` again with the write's problem.
pub async fn written(router: &Router, draft: &str) -> Value {
    until("the draft's write to end", || async {
        let (_, read) = super::json(router, "GET", &format!("/api/template-drafts/{draft}"), "dev", None).await;
        let state = read["state"].as_str().unwrap_or_default().to_owned();
        (state != "writing").then_some(read)
    })
    .await
}

/// The service with its default project over `registered()` seeded by the development seed, held by `Held`.
pub async fn ready(pool: &PgPool) -> (GateState<Held>, Router) {
    ready_on(pool, Arc::new(BlockingCompute)).await
}

/// `ready`, its computations on `compute`.
pub async fn ready_on(pool: &PgPool, compute: Arc<dyn Compute>) -> (GateState<Held>, Router) {
    make_local_default(pool).await.expect("the default project");
    let world = seeded(Held::new(registered())).await;
    let gate = GateState::new(super::state_on(pool.clone(), super::gate_rules(), compute), world);
    (gate.clone(), gate_routes(gate))
}

/// A request as the person `dev`, with `key` or a fresh Idempotency-Key, answered as JSON with its headers.
pub async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    key: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let key = key.map(str::to_owned).unwrap_or_else(|| format!("k{}", DeclaredId::mint()));
    let request = super::from_gate(method, uri, "dev", "")
        .header("content-type", "application/json")
        .header("idempotency-key", key);
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, headers, answer) = super::call(router, request.body(body).expect("a request")).await;
    let answer = if answer.is_empty() { Value::Null } else { serde_json::from_slice(&answer).expect("a JSON answer") };
    (status, headers, answer)
}

pub async fn get(router: &Router, uri: &str) -> Value {
    let (status, _, answer) = send(router, "GET", uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {answer}");
    answer
}

/// `dev`'s live New site conversation, which a draft runs in and Create site binds.
pub async fn thread(router: &Router) -> String {
    let (status, _, opened) = send(router, "POST", "/api/conversations", Some(json!({})), None).await;
    assert!(status.is_success(), "{opened}");
    opened["conversation"]["id"].as_str().expect("a conversation id").to_owned()
}

/// A draft of `template/site/{shell}` at its head, its first evaluation ended; with the New site conversation when
/// `conversation` names one.
pub async fn draft(router: &Router, shell: &str, conversation: Option<&str>) -> (String, Value) {
    let mut body = json!({ "template": format!("template/site/{shell}@main") });
    if let Some(conversation) = conversation {
        body["conversation"] = json!(conversation);
    }
    let (status, _, started) = send(router, "POST", "/api/template-drafts", Some(body), None).await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    assert_eq!(started["evaluation"], json!({ "n": 1, "state": "in-progress" }));
    let id = started["draft"].as_str().expect("a draft id").to_owned();
    let read = settled(router, &id).await;
    (id, read)
}

/// Press Create site on `draft` for `customer/demo/site/{slug}` from evaluation `n`, under `key` or a fresh one.
pub async fn press(
    router: &Router,
    draft: &str,
    slug: &str,
    n: u32,
    key: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let body = json!({ "customer": "demo", "slug": slug, "name": format!("Site {slug}"), "evaluation": n });
    send(router, "POST", &format!("/api/template-drafts/{draft}/site"), Some(body), key).await
}
