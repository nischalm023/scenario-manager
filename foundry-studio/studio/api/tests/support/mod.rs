//! What the service's tests share: a scratch schema in the test database, brought up by the service's own embedded
//! migrations (spec 01 section 3.3's tables) as `db::migrate` brings up a deployment's; the service's state over it,
//! its world-proto-server the client's in-memory mock (spec 04 section 3.2); and a way to call a router without a
//! socket.
//!
//! The database is the one `STUDIO_TEST_DATABASE_URL` names. Each test that needs it gets a schema of its own,
//! created, migrated and dropped here, so the tests run in parallel and none sees another's rows. With the variable
//! unset, those tests say they are skipped and pass; a variable naming a database the tests cannot reach fails, since
//! a gate that skips an unreachable database passes for the wrong reason.

#![allow(dead_code)] // each test file uses its own part of this module

pub mod agent;
pub mod containers;
pub mod delivery;
pub mod generation;
pub mod l_shape;
pub mod loops;
pub mod made;
pub mod plan_site;
pub mod results;
pub mod runs;
pub mod sites;
pub mod sse;
pub mod templates;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::request::Builder;
use axum::http::{HeaderMap, Request, StatusCode};
use runstore::RunStore;
use rustc_hash::FxHashSet;
use serde_json::{Value, json};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, PgPool};
use studio_api::agent::follow;
use studio_api::config::Deployment;
use studio_api::identity::{IdentityHeaders, IdentityRules};
use studio_api::journal::{Appended, NewEntry, append};
use studio_api::{AppState, GateState};
use studio_protocol::{DeclaredId, JournalSeq, ScopePath, Seq, Ulid};
use tower::ServiceExt;
use wps_client::draft::Op;
use wps_client::site::{SUB_SCOPES, ensure_prefixes};
use wps_client::wire::{RefAnnotation, WireChangeset};
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client, ClientConfig, Compute, DocId, MockWps, Prologue, WorldStore};

pub const DATABASE_VAR: &str = "STUDIO_TEST_DATABASE_URL";

/// A schema of the test's own, migrated to this build's tables.
pub struct Scratch {
    pub pool: PgPool,
    schema: String,
}

impl Scratch {
    /// Open one, or `None` when no test database is named.
    pub async fn open() -> Option<Self> {
        let scratch = Self::bare().await?;
        studio_api::db::migrate(&scratch.pool).await.expect("the migrations apply to an empty schema");
        Some(scratch)
    }

    /// Open one with no tables yet, for a test that brings it up a migration at a time; `None` as for `open`.
    pub async fn bare() -> Option<Self> {
        let url = env::var(DATABASE_VAR).ok()?;
        let schema = format!("studio_test_{}", unique_name());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap_or_else(|failure| panic!("{DATABASE_VAR} names a database this test cannot reach: {failure}"));
        admin.execute(format!("CREATE SCHEMA {schema}").as_str()).await.expect("a schema of the test's own");
        admin.close().await;
        let pool = schema_pool(&schema).await;
        Some(Self { pool, schema })
    }

    /// Drop the schema; explicit rather than a `Drop`, since dropping awaits.
    pub async fn close(self) {
        let schema = self.schema;
        self.pool.execute(format!("DROP SCHEMA {schema} CASCADE").as_str()).await.expect("the schema drops");
        self.pool.close().await;
    }
}

/// A pool over the test database every connection of which sees only `schema`, so the migrations build their tables
/// inside it; also the pool of an API process a test runs beside its own over the same schema, opened on that
/// process's runtime.
pub async fn schema_pool(schema: &str) -> PgPool {
    let url = env::var(DATABASE_VAR).unwrap_or_else(|_| panic!("{DATABASE_VAR} names the test database"));
    let options: PgConnectOptions = url.parse().expect("the test database's URL is a connection string");
    let search_path = format!("SET search_path TO {schema}");
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
            let search_path = search_path.clone();
            Box::pin(async move {
                connection.execute(search_path.as_str()).await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .unwrap_or_else(|failure| panic!("the test's own schema would not open: {failure}"))
}

/// A name no other test picks: the counter, not the clock, keeps two tests started in one tick apart.
fn unique_name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_nanos();
    format!("{nanos:x}_{:x}_{:x}", process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The scratch schema, or a skip said in words, from a test that needs the database.
#[macro_export]
macro_rules! scratch_or_skip {
    () => {
        match support::Scratch::open().await {
            Some(scratch) => scratch,
            None => {
                eprintln!("skipped: set {} to a PostgreSQL the tests may create schemas in", support::DATABASE_VAR);
                return;
            }
        }
    };
}

/// The rules the gate's default headers follow, with no local developers and no development identity.
pub fn gate_rules() -> IdentityRules {
    IdentityRules {
        headers: IdentityHeaders::default(),
        developer_role: "studio-developer".to_owned(),
        developers: FxHashSet::default(),
        dev_identity: None,
    }
}

/// A pool that never connects, for routes that answer before they touch the database.
pub fn unconnected_pool() -> PgPool {
    PgPoolOptions::new().connect_lazy("postgres://nobody@127.0.0.1:9/nothing").expect("a well-formed database URL")
}

/// The model's name as the tests' conversations carry it.
pub const MODEL_LABEL: &str = "Claude Sonnet 5.5";

/// The service's state over `pool`, telling who is asking by `rules`, as a developer's machine runs it: computations
/// on tokio's blocking threads, and a run-object store in a folder of the test's own.
pub fn state(pool: PgPool, rules: IdentityRules) -> AppState {
    state_on(pool, rules, Arc::new(BlockingCompute))
}

/// `state`, its computations on `compute`.
pub fn state_on(pool: PgPool, rules: IdentityRules, compute: Arc<dyn Compute>) -> AppState {
    AppState::new(pool, rules, Deployment::Local, MODEL_LABEL, compute, runstore())
}

/// A run-object store in a fresh folder of the test's own.
pub fn runstore() -> RunStore {
    RunStore::local_folder(&runstore_folder()).expect("a run-object store over the folder")
}

/// A fresh folder of the test's own for a run-object store.
pub fn runstore_folder() -> PathBuf {
    let objects = env::temp_dir().join(format!("studio-api-objects-{}", unique_name()));
    fs::create_dir_all(&objects).expect("a folder for the run-object store");
    objects
}

/// A fresh in-memory world-proto-server of world-proto cd4923516, which the client's probe needs at start, generating
/// with the native expansion, whose registry defines `frame`, the one component the tests' sites carry, so a site's
/// export can read its references.
pub fn mock() -> MockWps {
    let mock = MockWps::phase_3();
    generation::native(&mock);
    let parent = RefAnnotation { path: "/parent".to_owned(), expects: None };
    mock.publish("frame", "0.7.0", json!({ "type": "object" }), vec![parent]);
    mock
}

/// A client of a fresh in-memory world-proto-server.
pub async fn world() -> Client<MockWps> {
    Client::start(mock(), ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("the mock's client")
}

/// The sites the route tests open across files, each with its name: opening answers `not-found` for a site
/// world-proto-server does not list, so `gate` makes them.
pub const DEMO_SITES: [(&str, &str); 2] =
    [("customer/demo/site/lshape-6", "L-Shape 6"), ("customer/demo/site/ushape-4", "U-Shape 4")];

/// `app` with a fresh in-memory world-proto-server as its world, holding `DEMO_SITES`: what `gate_routes` and `serve`
/// take in the tests.
pub async fn gate(app: AppState) -> GateState<MockWps> {
    let world = world().await;
    for (root, name) in DEMO_SITES {
        made_site(&world, root, name).await;
    }
    GateState::new(app, world)
}

/// Post `ops` to `scope` at its head in a changeset of that scope alone, stating `prologue` when given; the
/// changeset it landed as.
pub async fn stage<S: Wps>(world: &Client<S>, scope: &ScopePath, ops: Vec<Op>, prologue: Option<Prologue>) -> Seq {
    let base = world.tree().await.expect("the scope tree").head(scope).expect("the scope exists");
    let mut changeset = WireChangeset::of_one_scope(scope.clone(), base, "tests", "a stage", ops);
    changeset.prologue = prologue;
    world.wps().post_changeset(&changeset).await.expect("the stage lands").seq.expect("the scope's new changeset")
}

/// A root's prologue naming the site `name`, its primary frame `frame`.
pub fn prologue(name: &str, frame: DeclaredId) -> Prologue {
    Prologue {
        doc: DocId::mint(),
        kind: Some("world".to_owned()),
        name: Some(name.to_owned()),
        primary_frame: Some(frame.into()),
        depends_on: Vec::new(),
    }
}

/// A site as the route tests need one, on a mock of any phase: its scopes made by name with their flags, a frame in
/// its `map`, and its root at changeset 1 stating the prologue that names it.
pub async fn made_site<S: Wps>(world: &Client<S>, root: &str, name: &str) -> ScopePath {
    let root: ScopePath = root.parse().expect("a site's root");
    made_scopes(world, &root).await;
    let frame = DeclaredId::mint();
    let components = json!({ "frame": {} }).as_object().cloned().expect("an object");
    let map = root.join(&"map".parse().expect("a slug")).expect("the site's map");
    stage(world, &map, vec![Op::Declare { id: frame, name: None, components }], None).await;
    stage(world, &root, Vec::new(), Some(prologue(name, frame))).await;
    root
}

/// `root`, made by name a boundary with its missing prefixes, and its `map`, `flow` and `fleet` flagless, all empty.
pub async fn made_scopes<S: Wps>(world: &Client<S>, root: &ScopePath) {
    ensure_prefixes(world, root).await.expect("the root's prefixes");
    world.ensure_scope(root, true).await.expect("the root");
    for sub in SUB_SCOPES {
        let below = root.join(&sub.parse().expect("a slug")).expect("a sub-scope");
        world.ensure_scope(&below, false).await.expect("a sub-scope");
    }
}

/// A request as the gate forwards it for a person: subject, email, name and roles in the default headers.
pub fn from_gate(method: &str, uri: &str, subject: &str, roles: &str) -> Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-auth-request-user", subject)
        .header("x-auth-request-email", format!("{subject}@example.com"))
        .header("x-auth-request-preferred-username", format!("Person {subject}"))
        .header("x-auth-request-groups", roles)
}

/// Send `request` through `router` and read its whole answer.
pub async fn call(router: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let answer = router.clone().oneshot(request).await.expect("a router always answers");
    let (parts, body) = answer.into_parts();
    let body = to_bytes(body, usize::MAX).await.expect("the answer's body reads");
    (parts.status, parts.headers, body.to_vec())
}

/// A fresh ULID for a test's rows: Crockford digits after a fixed prefix, unique within the process.
pub fn ulid() -> Ulid {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!("01KN0TJZANP5N0PGJD{:08}", NEXT.fetch_add(1, Ordering::Relaxed)).parse().expect("Crockford digits")
}

/// A person, as the identity layer records one.
pub async fn person(pool: &PgPool, subject: &str) {
    sqlx::query(
        "INSERT INTO people (subject, email, name) VALUES ($1, $1 || '@example.com', 'Person ' || $1) \
         ON CONFLICT DO NOTHING",
    )
    .bind(subject)
    .execute(pool)
    .await
    .expect("a person is recorded");
}

/// A conversation row as a test needs it, made by `by` (recorded first): `site` is `None` for a new-site thread, and
/// its journal numbers start at `first_seq`.
pub async fn conversation(pool: &PgPool, site: Option<&str>, state: &str, by: &str, first_seq: i64) -> Ulid {
    person(pool, by).await;
    let id = ulid();
    sqlx::query(
        "INSERT INTO projects (id, name) VALUES ('01KN0TJZANP5N0PGJD8F7A3FPR', 'Tests') ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("the tests' project is recorded");
    sqlx::query(
        "INSERT INTO conversations (id, project, site, state, created_by, first_seq, last_seq) \
         VALUES ($1, '01KN0TJZANP5N0PGJD8F7A3FPR', $2, $3, $4, $5, $5 - 1)",
    )
    .bind(id.as_str())
    .bind(site)
    .bind(state)
    .bind(by)
    .bind(first_seq)
    .execute(pool)
    .await
    .expect("a conversation is recorded");
    id
}

/// Send a request as the gate forwards it for `subject`, with `body` as JSON, and read the answer as JSON
/// (`Value::Null` when it has no body).
pub async fn json(router: &Router, method: &str, uri: &str, subject: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = from_gate(method, uri, subject, "").header("content-type", "application/json");
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, _, answer) = call(router, request.body(body).expect("a request")).await;
    let answer = if answer.is_empty() { Value::Null } else { serde_json::from_slice(&answer).expect("a JSON answer") };
    (status, answer)
}

/// Append `entry` to `conversation` in a transaction of its own, commit, and announce it, as a writer does; an entry
/// of a turn makes the change to the turn's row the loop makes with it (`agent::follow`), as the test writer's do.
pub async fn write(state: &AppState, conversation: &Ulid, entry: NewEntry) -> JournalSeq {
    let mut tx = state.pool.begin().await.expect("a transaction");
    follow(&mut tx, conversation, &entry).await.expect("the entry's turn follows");
    let Appended::Wrote(written) = append(&mut tx, conversation, entry).await.expect("an append") else {
        panic!("an entry without a dedup key is written")
    };
    tx.commit().await.expect("a commit");
    let seq = written.entry().seq;
    state.hub.announce([written]);
    seq
}

/// Append every entry to `conversation` in one transaction, commit, and announce them.
pub async fn write_all(state: &AppState, conversation: &Ulid, entries: Vec<NewEntry>) {
    let mut tx = state.pool.begin().await.expect("a transaction");
    let mut written = Vec::new();
    for entry in entries {
        follow(&mut tx, conversation, &entry).await.expect("the entry's turn follows");
        match append(&mut tx, conversation, entry).await.expect("an append") {
            Appended::Wrote(entry) => written.push(entry),
            Appended::Held(_) => {}
        }
    }
    tx.commit().await.expect("a commit");
    state.hub.announce(written);
}
