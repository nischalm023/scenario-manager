//! Spec 07 section 8 against the database and the world-proto-server client's in-memory mock: statements 3 (create,
//! edit, the seal, the next version a copy, an older version's edit refused, a stale plan sealing nothing), 4 (the
//! field merge), 6 (experiments), 1 (one `resolve_cut` per readiness; a scope that does not resolve named) and 10
//! (the reader's refusal, in its words; a check rerun only when its key changes; a release the image refuses), with
//! duplicate, delete, the diff and spec 01 section 3.6's panel events. The site's world is dessim's own small relay
//! fixture, `minimal_putaway.json`, staged into a made site's map scope, with a flow that brings stock in. The races
//! are forced, never left to timing: a connection of the test's own holds a row lock the racing requests meet in a
//! known order, and the test lets go once Postgres shows each request waiting where the race needs it.

mod support;

use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};
use studio_api::projects::make_local_default;
use studio_api::scenarios::{
    ReadinessCache, SealError, Sealed, SiteReader, catalogue, readiness, seal, validate_variants,
};
use studio_api::{GateState, gate_routes};
use studio_protocol::{
    DeclaredId, Experiment, Readiness, Revision, ScenarioContent, ScenarioRead, SitePath, Ulid, VersionNumber,
    VersionState,
};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use wps_client::draft::Op;
use wps_client::{BlockingCompute, Client, CutExport, MockWps, UnitCut, WpsError};

use support::sites::{below, components, fixture_site, flow, world};

const SITE: &str = "customer/demo/site/lshape-6";
const NO_RACKS: &str = "customer/demo/site/no-racks";
struct Stack {
    gate: GateState<MockWps>,
    router: Router,
}

async fn stack(pool: &PgPool) -> Stack {
    make_local_default(pool).await.expect("the default project");
    let world = world().await;
    fixture_site(&world, SITE, true).await;
    fixture_site(&world, NO_RACKS, false).await;
    let gate = GateState::new(support::state(pool.clone(), support::gate_rules()), world);
    Stack { router: gate_routes(gate.clone()), gate }
}

/// A request as the gate forwards it for `u-1`, a fresh `Idempotency-Key` on every POST.
async fn send(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = support::from_gate(method, uri, "u-1", "").header("content-type", "application/json");
    if method == "POST" {
        request = request.header("idempotency-key", support::ulid().to_string());
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, _, answer) = support::call(router, request.body(body).expect("a request")).await;
    (status, if answer.is_empty() { Value::Null } else { serde_json::from_slice(&answer).expect("a JSON answer") })
}

async fn create(router: &Router, name: &str, extra: Value) -> Ulid {
    let mut body = json!({ "name": name, "site_scope": SITE });
    body.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
    let (status, made) = send(router, "POST", "/api/scenarios", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["scenario"].as_str().expect("the scenario's id").parse().expect("a ULID")
}

async fn read(router: &Router, id: &Ulid, version: Option<u32>) -> ScenarioRead {
    let query = version.map_or_else(String::new, |n| format!("?version={n}"));
    let (status, answer) = send(router, "GET", &format!("/api/scenarios/{id}{query}"), None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    serde_json::from_value(answer).expect("a scenario read")
}

async fn edit(router: &Router, id: &Ulid, version: u32, revision: u32, set: Value) -> (StatusCode, Value) {
    let body = json!({ "version": version, "revision": revision, "set": set });
    send(router, "PATCH", &format!("/api/scenarios/{id}"), Some(body)).await
}

async fn experiment(router: &Router, body: Value) -> (StatusCode, Value) {
    send(router, "POST", "/api/experiments", Some(body)).await
}

/// `send` on a task of its own, so the test can hold a lock while the request waits on it.
fn spawned(router: &Router, method: &'static str, uri: String, body: Value) -> JoinHandle<(StatusCode, Value)> {
    let router = router.clone();
    tokio::spawn(async move { send(&router, method, &uri, Some(body)).await })
}

/// A connection of the test's own on the scratch schema, outside the router's pool, so a lock it holds never takes a
/// connection a racing request needs; and its backend's process id, which Postgres names its waiters by.
async fn own_connection(pool: &PgPool) -> (PgConnection, i32) {
    let schema: String = sqlx::query_scalar("SELECT current_schema()").fetch_one(pool).await.expect("the schema");
    let url = env::var(support::DATABASE_VAR).expect("the test database");
    let mut conn = PgConnection::connect(&url).await.expect("a connection of the test's own");
    sqlx::query(&format!("SET search_path TO {schema}")).execute(&mut conn).await.expect("the scratch schema");
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut conn).await.expect("its process id");
    (conn, pid)
}

/// Wait (wall clock, 30 s at most) until `count` backends wait on `holder`'s locks, directly or behind a backend
/// that does.
async fn waiting_behind(pool: &PgPool, holder: i32, count: i64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity a WHERE $1 = ANY(pg_blocking_pids(a.pid)) OR EXISTS ( \
                 SELECT 1 FROM pg_stat_activity b \
                 WHERE $1 = ANY(pg_blocking_pids(b.pid)) AND b.pid = ANY(pg_blocking_pids(a.pid)))",
        )
        .bind(holder)
        .fetch_one(pool)
        .await
        .expect("the backends waiting");
        if waiting >= count {
            return;
        }
        assert!(Instant::now() < deadline, "{waiting} of {count} requests came to wait behind the test's lock");
        sleep(Duration::from_millis(20)).await;
    }
}

/// How many undeleted scenarios of the site hold `name`, compared as the unique index compares it.
async fn holding(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM scenarios WHERE lower(name) = lower($1) AND deleted_at IS NULL")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("the names")
}

#[tokio::test]
async fn create_states_every_field_and_a_name_the_site_uses_is_refused_with_its_id() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let read = read(&router, &id, None).await;
    assert_eq!((read.version.n, read.version.revision), (VersionNumber(1), Revision(1)));
    assert_eq!(read.version.content, scenario_spec::stated_defaults());
    assert_eq!(read.version.set_by.len(), catalogue().items.len(), "every field is set");
    assert!(read.version.set_by.values().all(|set| set.author.name == "Person u-1"));
    let (status, taken) =
        send(&router, "POST", "/api/scenarios", Some(json!({ "name": "PEAK HOUR", "site_scope": SITE }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(taken["type"], "name-taken");
    assert_eq!(taken["problems"][0]["ref"], format!("scenario:{id}"));
    let elsewhere = json!({ "name": "Peak hour", "site_scope": "customer/demo/site/nowhere" });
    assert_eq!(send(&router, "POST", "/api/scenarios", Some(elsewhere)).await.0, StatusCode::NOT_FOUND);
    scratch.close().await;
}

/// Statement 3: create (open, revision 1); edit (revision 2); a submission seals; the next edit opens version 2 as
/// a copy; an edit of version 1 is then refused; an experiment prepared at revision 1 of version 2, after an edit to
/// revision 2, is refused `plan-stale` and seals nothing.
/// Astra's name race: two creations of one name both pass the name check before either commits. The test holds the
/// default project's row, so the first waits at its foreign key with its name already in the unique index and the
/// second waits on that entry; let go, the first commits and the index refuses the second, which answers
/// `name-taken` linking the winner, as the check's own collision does. One scenario holds the name.
#[tokio::test]
async fn two_creations_racing_for_a_name_leave_one_and_the_loser_links_the_winner() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let (mut holder, pid) = own_connection(&scratch.pool).await;
    let mut held = holder.begin().await.expect("the test's transaction");
    sqlx::query("SELECT id FROM projects FOR UPDATE").execute(&mut *held).await.expect("the project's row held");
    let body = json!({ "name": "Peak hour", "site_scope": SITE });
    let first = spawned(&router, "POST", "/api/scenarios".to_owned(), body.clone());
    waiting_behind(&scratch.pool, pid, 1).await;
    let second = spawned(&router, "POST", "/api/scenarios".to_owned(), body);
    waiting_behind(&scratch.pool, pid, 2).await;
    held.rollback().await.expect("the project's row let go");
    let (won, lost) = (first.await.expect("the first request"), second.await.expect("the second request"));
    assert_eq!(won.0, StatusCode::CREATED, "{}", won.1);
    assert_eq!((lost.0, lost.1["type"].clone()), (StatusCode::CONFLICT, json!("name-taken")), "{}", lost.1);
    assert_eq!(lost.1["problems"][0]["ref"], format!("scenario:{}", won.1["scenario"].as_str().expect("its id")));
    assert_eq!(holding(&scratch.pool, "Peak hour").await, 1);
    scratch.close().await;
}

/// The same race on a rename: a creation's name waits uncommitted in the index while a rename of another scenario
/// to that name passes its check and waits on it; let go, the rename answers `name-taken` linking the creation, and
/// the renamed scenario keeps its name.
#[tokio::test]
async fn a_rename_racing_a_creation_for_a_name_links_the_creation() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let night = create(&router, "Night shift", json!({})).await;
    let (mut holder, pid) = own_connection(&scratch.pool).await;
    let mut held = holder.begin().await.expect("the test's transaction");
    sqlx::query("SELECT id FROM projects FOR UPDATE").execute(&mut *held).await.expect("the project's row held");
    let made =
        spawned(&router, "POST", "/api/scenarios".to_owned(), json!({ "name": "Peak hour", "site_scope": SITE }));
    waiting_behind(&scratch.pool, pid, 1).await;
    let rename = json!({ "version": 1, "revision": 1, "set": [{ "pointer": "/name", "value": "Peak hour" }] });
    let renamed = spawned(&router, "PATCH", format!("/api/scenarios/{night}"), rename);
    waiting_behind(&scratch.pool, pid, 2).await;
    held.rollback().await.expect("the project's row let go");
    let (made, renamed) = (made.await.expect("the creation"), renamed.await.expect("the rename"));
    assert_eq!(made.0, StatusCode::CREATED, "{}", made.1);
    assert_eq!((renamed.0, renamed.1["type"].clone()), (StatusCode::CONFLICT, json!("name-taken")), "{}", renamed.1);
    assert_eq!(renamed.1["problems"][0]["ref"], format!("scenario:{}", made.1["scenario"].as_str().expect("its id")));
    assert_eq!(read(&router, &night, None).await.scenario.name, "Night shift");
    assert_eq!(holding(&scratch.pool, "Peak hour").await, 1);
    scratch.close().await;
}

#[tokio::test]
async fn versions_open_seal_and_copy_and_a_stale_plan_seals_nothing() {
    let scratch = scratch_or_skip!();
    let Stack { router, gate } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let duration = json!([{ "pointer": "/run_controls/duration_sim_ms", "value": 7_200_000 }]);
    let (status, edited) = edit(&router, &id, 1, 1, duration).await;
    assert_eq!((status, edited), (StatusCode::OK, json!({ "version": 1, "revision": 2, "opened": false })));
    let (status, prepared) = experiment(&router, json!({ "scenario": id, "version": 1, "revision": 2 })).await;
    assert_eq!(status, StatusCode::CREATED, "{prepared}");
    let prepared: Experiment = serde_json::from_value(prepared).expect("an experiment");

    let mut tx = gate.app.pool.begin().await.expect("a transaction");
    let sealed = seal(&mut tx, &id, VersionNumber(1), Revision(2), &prepared.id).await.expect("the seal");
    tx.commit().await.expect("a commit");
    assert_eq!(sealed, Sealed::Now);
    let rate = json!([{ "pointer": "/workload/generator/orders_per_hour", "value": 350 }]);
    let (status, edited) = edit(&router, &id, 1, 2, rate.clone()).await;
    assert_eq!((status, edited), (StatusCode::OK, json!({ "version": 2, "revision": 1, "opened": true })));
    let (one, two) = (read(&router, &id, Some(1)).await, read(&router, &id, Some(2)).await);
    assert_eq!(one.version.content.workload.generator.orders_per_hour.0, 300.0, "version 1 stays as its runs read it");
    assert_eq!(two.version.content.workload.generator.orders_per_hour.0, 350.0);
    assert_eq!(two.version.content.run_controls, one.version.content.run_controls, "a copy of version 1");
    let (status, refused) = edit(&router, &id, 1, 2, rate).await;
    assert_eq!((status, refused["type"].clone()), (StatusCode::CONFLICT, json!("scenario-changed")));

    let (_, stale) = experiment(&router, json!({ "scenario": id, "version": 2, "revision": 1 })).await;
    let stale: Experiment = serde_json::from_value(stale).expect("an experiment");
    edit(&router, &id, 2, 1, json!([{ "pointer": "/workload/generator/sku_count", "value": 500 }])).await;
    let mut tx = gate.app.pool.begin().await.expect("a transaction");
    let refused = seal(&mut tx, &id, VersionNumber(2), Revision(1), &stale.id).await;
    assert!(matches!(refused, Err(SealError::PlanStale { planned: 1, now: 2, .. })), "{refused:?}");
    drop(tx);
    let two = read(&router, &id, Some(2)).await;
    assert_eq!(two.version.state, VersionState::Open, "nothing was sealed");
    scratch.close().await;
}

/// Each version of `scenario` as stored, oldest first: its state, revision, SKU count and the experiment that sealed it.
async fn stored_versions(pool: &PgPool, scenario: &Ulid) -> Vec<(String, i32, i64, Option<String>)> {
    sqlx::query_as(
        "SELECT state, revision, (workload #>> '{generator,sku_count}')::bigint, sealed_by FROM scenario_versions \
         WHERE scenario = $1 ORDER BY version",
    )
    .bind(scenario.as_str())
    .fetch_all(pool)
    .await
    .expect("the versions")
}

/// The seal/edit race, the seal first (Astra's point (c)): a submission's seal holds the scenario's and the version's
/// rows while an edit based on the same revision waits on them; once the seal commits, the edit finds the version
/// sealed and opens version 2 as a copy with its change, and version 1 stays as the seal left it.
#[tokio::test]
async fn an_edit_racing_a_seal_that_holds_the_version_opens_the_next_version() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let (_, prepared) = experiment(&router, json!({ "scenario": id, "version": 1, "revision": 1 })).await;
    let prepared: Ulid = prepared["id"].as_str().expect("the experiment").parse().expect("a ULID");
    let (mut holder, pid) = own_connection(&scratch.pool).await;
    let mut sealing = holder.begin().await.expect("the submission's transaction");
    let sealed = seal(&mut sealing, &id, VersionNumber(1), Revision(1), &prepared).await.expect("the seal");
    assert_eq!(sealed, Sealed::Now);
    let sku =
        json!({ "version": 1, "revision": 1, "set": [{ "pointer": "/workload/generator/sku_count", "value": 500 }] });
    let edit = spawned(&router, "PATCH", format!("/api/scenarios/{id}"), sku);
    waiting_behind(&scratch.pool, pid, 1).await;
    sealing.commit().await.expect("the seal commits");
    let (status, edited) = edit.await.expect("the edit");
    assert_eq!((status, edited), (StatusCode::OK, json!({ "version": 2, "revision": 1, "opened": true })));
    let stored = stored_versions(&scratch.pool, &id).await;
    let sealed_by = Some(prepared.to_string());
    assert_eq!(stored, [("sealed".to_owned(), 1, 400, sealed_by), ("open".to_owned(), 1, 500, None)]);
    assert_eq!(read(&router, &id, None).await.scenario.latest_version, VersionNumber(2));
    scratch.close().await;
}

/// The seal/edit race, the edit first: the edit holds the scenario's row and waits on the version's, which the test
/// holds, while a seal of the revision it read waits on the edit; let go, the edit saves revision 2 and the seal,
/// finding revision 2 where its plan read 1, refuses `plan-stale` and seals nothing.
#[tokio::test]
async fn a_seal_racing_an_edit_that_holds_the_scenario_is_plan_stale_and_seals_nothing() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let (_, prepared) = experiment(&router, json!({ "scenario": id, "version": 1, "revision": 1 })).await;
    let prepared: Ulid = prepared["id"].as_str().expect("the experiment").parse().expect("a ULID");
    let (mut holder, pid) = own_connection(&scratch.pool).await;
    let mut held = holder.begin().await.expect("the test's transaction");
    sqlx::query("SELECT version FROM scenario_versions WHERE scenario = $1 AND version = 1 FOR UPDATE")
        .bind(id.as_str())
        .execute(&mut *held)
        .await
        .expect("the version's row held");
    let sku =
        json!({ "version": 1, "revision": 1, "set": [{ "pointer": "/workload/generator/sku_count", "value": 500 }] });
    let edit = spawned(&router, "PATCH", format!("/api/scenarios/{id}"), sku);
    waiting_behind(&scratch.pool, pid, 1).await;
    let (mut submitter, _) = own_connection(&scratch.pool).await;
    let scenario = id.clone();
    let sealing = tokio::spawn(async move {
        let mut tx = submitter.begin().await.expect("the submission's transaction");
        let sealed = seal(&mut tx, &scenario, VersionNumber(1), Revision(1), &prepared).await;
        tx.rollback().await.expect("the submission rolls back");
        sealed
    });
    waiting_behind(&scratch.pool, pid, 2).await;
    held.rollback().await.expect("the version's row let go");
    let (status, edited) = edit.await.expect("the edit");
    assert_eq!((status, edited), (StatusCode::OK, json!({ "version": 1, "revision": 2, "opened": false })));
    let refused = sealing.await.expect("the seal");
    assert!(matches!(refused, Err(SealError::PlanStale { planned: 1, now: 2, .. })), "{refused:?}");
    assert_eq!(stored_versions(&scratch.pool, &id).await, [("open".to_owned(), 2, 500, None)]);
    scratch.close().await;
}

/// Statement 4: two edits from one revision on unrelated pointers both apply; on one pointer, or on `/workload`
/// after `/workload/generator/sku_count` (and the reverse), the second is refused naming the field; two disjoint
/// edits that together break a rule that joins fields (waves with no interval) leave the second refused
/// `validation-failed`, nothing saved.
#[tokio::test]
async fn the_field_merge_applies_disjoint_edits_and_refuses_overlapping_ones() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let id = create(&router, "Merge", json!({})).await;
    let set = |pointer: &str, value: Value| json!([{ "pointer": pointer, "value": value }]);
    assert_eq!(edit(&router, &id, 1, 1, set("/workload/generator/sku_count", json!(500))).await.0, StatusCode::OK);
    let (status, both) = edit(&router, &id, 1, 1, set("/run_controls/seed", json!(4))).await;
    assert_eq!((status, both["revision"].clone()), (StatusCode::OK, json!(3)), "unrelated fields both apply");
    let (status, one) = edit(&router, &id, 1, 1, set("/workload/generator/sku_count", json!(600))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(one["conflicts"][0]["pointer"], "/workload/generator/sku_count");
    assert_eq!(one["conflicts"][0]["value"], 500);
    assert_eq!(one["conflicts"][0]["set_by"]["revision"], 2);

    let workload = serde_json::to_value(scenario_spec::stated_defaults().workload).expect("JSON");
    let (status, whole) = edit(&router, &id, 1, 1, set("/workload", workload.clone())).await;
    assert_eq!(
        (status, whole["conflicts"][0]["pointer"].clone()),
        (StatusCode::CONFLICT, json!("/workload/generator/sku_count"))
    );
    assert_eq!(edit(&router, &id, 1, 3, set("/workload", workload)).await.0, StatusCode::OK);
    let (status, reverse) = edit(&router, &id, 1, 3, set("/workload/generator/sku_count", json!(700))).await;
    assert_eq!((status, reverse["conflicts"][0]["pointer"].clone()), (StatusCode::CONFLICT, json!("/workload")));

    let waves = create(
        &router,
        "Waves",
        json!({ "workload": {
        "generator": { "orders_per_hour": 300, "lines_per_order": 3, "sku_count": 400, "inventory_fill_pct": 80,
                       "units_per_stored_tote": 1, "release": "random", "wave_every_sim_ms": 600000, "seed": 0 },
        "order_trays": 0 } }),
    )
    .await;
    assert_eq!(
        edit(&router, &waves, 1, 1, set("/workload/generator/wave_every_sim_ms", Value::Null)).await.0,
        StatusCode::OK
    );
    let (status, joined) = edit(&router, &waves, 1, 1, set("/workload/generator/release", json!("waves"))).await;
    assert_eq!((status, joined["type"].clone()), (StatusCode::UNPROCESSABLE_ENTITY, json!("validation-failed")));
    assert_eq!(joined["problems"][0]["field"], "/workload/generator/wave_every_sim_ms");
    let kept = read(&router, &waves, None).await;
    assert_eq!(kept.version.revision, Revision(2), "nothing saved");
    assert_eq!(serde_json::to_value(kept.version.content.workload.generator.release).unwrap(), "random");
    scratch.close().await;
}

/// A reader that counts its cuts, over the mock's client.
struct Counting<'a> {
    client: &'a Client<MockWps>,
    cuts: AtomicUsize,
}

impl SiteReader for Counting<'_> {
    async fn resolve_cut(&self, site: &SitePath) -> Result<UnitCut, WpsError> {
        self.cuts.fetch_add(1, Ordering::SeqCst);
        self.client.resolve_cut(site).await
    }

    async fn export(&self, cut: &UnitCut) -> Result<CutExport, WpsError> {
        SiteReader::export(self.client, cut).await
    }
}

/// A reader whose site's fleet scope does not resolve on `main`.
struct FleetUnresolved;

impl SiteReader for FleetUnresolved {
    async fn resolve_cut(&self, site: &SitePath) -> Result<UnitCut, WpsError> {
        let scope = site.as_scope().join(&"fleet".parse().expect("a slug")).expect("the fleet scope");
        let text = format!("{scope} does not resolve on main");
        Err(WpsError::PinUnresolved { scope, text })
    }

    async fn export(&self, _: &UnitCut) -> Result<CutExport, WpsError> {
        unreachable!("nothing is exported without a cut")
    }
}

async fn ready(gate: &GateState<MockWps>, reader: &impl SiteReader, cache: &ReadinessCache, id: &Ulid) -> Readiness {
    readiness(&gate.app.pool, reader, Arc::new(BlockingCompute), cache, id, None, "/test").await.expect("a readiness")
}

/// Statement 1: one `resolve_cut` per readiness; a site whose fleet scope does not resolve is refused naming it.
/// Statement 10: the check runs once per key, again when the revision or the cut moves, and a release the runner
/// image refuses makes readiness an error.
#[tokio::test]
async fn readiness_reads_one_cut_reruns_only_on_a_new_key_and_names_what_refuses() {
    let scratch = scratch_or_skip!();
    let Stack { router, gate } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let (cache, reader) = (ReadinessCache::default(), Counting { client: &gate.world, cuts: AtomicUsize::new(0) });
    let first = ready(&gate, &reader, &cache, &id).await;
    assert!(first.ready, "{:?}", first.problems);
    assert_eq!(first.problems.iter().map(|problem| problem.code.as_str()).collect::<Vec<_>>(), ["not-checked"]);
    assert_eq!(
        first.problems[0].text, "This check does not establish that all generated stock fits the racks.",
        "what readiness does not check, in one sentence"
    );
    let seqs = &first.cut.as_ref().expect("the cut read").seqs;
    assert_eq!(seqs.len(), 4, "the root, its map, flow and fleet: {seqs:?}");
    assert!(first.workload.as_ref().is_some_and(|workload| workload.orders.0 == 1_200), "300 an hour for 4 h");
    ready(&gate, &reader, &cache, &id).await;
    assert_eq!((reader.cuts.load(Ordering::SeqCst), cache.computations()), (2, 1), "one cut a read; one check a key");

    let random = json!([{ "pointer": "/workload/generator/release", "value": "random" }]);
    assert_eq!(edit(&router, &id, 1, 1, random).await.0, StatusCode::OK);
    let refused = ready(&gate, &reader, &cache, &id).await;
    assert_eq!(cache.computations(), 2, "a new revision checks again");
    assert!(!refused.ready);
    let release =
        refused.problems.iter().find(|problem| problem.code == "unsupported-value").expect("the release refused");
    assert_eq!(release.field.as_deref(), Some("/workload/generator/release"));
    assert!(
        release.text.starts_with("Random arrivals need a runner image that reads arrival times"),
        "{}",
        release.text
    );

    let scope = below(&SITE.parse().unwrap(), "flow");
    support::stage(&gate.world, &scope, Vec::<Op>::new(), None).await;
    ready(&gate, &reader, &cache, &id).await;
    assert_eq!(cache.computations(), 3, "a moved scope checks again");

    let unresolved = ready(&gate, &FleetUnresolved, &cache, &id).await;
    assert!(!unresolved.ready && unresolved.cut.is_none());
    assert_eq!(unresolved.problems[0].code, "pin-unresolved");
    assert_eq!(unresolved.problems[0].scope.as_ref().map(ToString::to_string), Some(format!("{SITE}/fleet")));
    scratch.close().await;
}

/// Statement 10: missing storage is refused by the flow compiler before the workload is loaded; a sequencing
/// mode the runner image does not run also makes readiness an error.
#[tokio::test]
async fn readiness_reports_missing_storage_and_an_unsupported_sequencing() {
    let scratch = scratch_or_skip!();
    let Stack { router, gate } = stack(&scratch.pool).await;
    let body = json!({ "name": "Peak hour", "site_scope": NO_RACKS });
    let (status, made) = send(&router, "POST", "/api/scenarios", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id: Ulid = made["scenario"].as_str().unwrap().parse().unwrap();
    let (cache, reader) = (ReadinessCache::default(), Counting { client: &gate.world, cuts: AtomicUsize::new(0) });
    let refused = ready(&gate, &reader, &cache, &id).await;
    assert!(!refused.ready);
    let empty = refused
        .problems
        .iter()
        .find(|problem| problem.code == "node-empty")
        .unwrap_or_else(|| panic!("{:?}", refused.problems));
    assert!(empty.text.contains("Inventory storage"), "the compiler names the missing storage: {}", empty.text);

    let scope = below(&SITE.parse().unwrap(), "flow");
    support::stage(&gate.world, &scope, flow(Some("strict")), None).await;
    let peak = create(&router, "Peak hour", json!({})).await;
    let sequenced = ready(&gate, &reader, &cache, &peak).await;
    let unsupported = sequenced.problems.iter().find(|problem| problem.code == "unsupported-value").expect("refused");
    assert_eq!(unsupported.field.as_deref(), Some("/flow.node/work/sequencing"));
    assert!(!sequenced.ready);
    scratch.close().await;
}

/// Spec 07 section 3.7: an agent override no cohort names is a warning, and one whose item the world does not carry
/// is the reader's refusal (`resolve_successors`, run first by dessim's reader).
#[tokio::test]
async fn an_override_no_cohort_names_warns_and_one_of_nothing_is_refused() {
    let scratch = scratch_or_skip!();
    let Stack { router, gate } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let fleet = below(&SITE.parse().unwrap(), "fleet");
    let figures =
        json!({ "pick_secs": 7.1, "drop_secs": 5.3, "align_before_pick_secs": 1.7, "align_before_drop_secs": 1.7 });
    let unused =
        components(json!({ "successor": { "of": "d257c36b-53ea-5ba2-a0f3-6fd0346f00ac" }, "end_effector": figures }));
    support::stage(
        &gate.world,
        &fleet,
        vec![Op::Declare { id: DeclaredId::mint(), name: None, components: unused }],
        None,
    )
    .await;
    let (cache, reader) = (ReadinessCache::default(), Counting { client: &gate.world, cuts: AtomicUsize::new(0) });
    let warned = ready(&gate, &reader, &cache, &id).await;
    assert!(warned.ready, "{:?}", warned.problems);
    assert!(warned.problems.iter().any(|problem| problem.code == "override-unused"), "{:?}", warned.problems);

    let nowhere =
        components(json!({ "successor": { "of": "7b0c2d4e-6f8a-4b1c-9d3e-5f7a9b1c3d5e" }, "end_effector": figures }));
    support::stage(
        &gate.world,
        &fleet,
        vec![Op::Declare { id: DeclaredId::mint(), name: None, components: nowhere }],
        None,
    )
    .await;
    let refused = ready(&gate, &reader, &cache, &id).await;
    assert!(!refused.ready);
    let floor = refused.problems.iter().find(|problem| problem.code == "floor-compile-failed").expect("refused");
    assert!(floor.text.contains("does not carry"), "{}", floor.text);
    scratch.close().await;
}

/// Statement 6: five values with three repeats make 15 runs, the first five the five values at the first seed, each
/// value one variant whose repeats send one workload spec; a map entity, the duration, a field no sweep varies, two
/// varied fields and authored variants are refused; creating generates, compiles and loads nothing, and the plan's
/// check does all three once per variant.
#[tokio::test]
async fn experiments_expand_a_sweep_refuse_what_no_sweep_varies_and_check_only_at_the_plan() {
    let scratch = scratch_or_skip!();
    let Stack { router, gate } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    let target = format!("scenario:{id}");
    let (requests, cache) = (gate.world.wps().requests(), ReadinessCache::default());
    let sweep = json!({ "scenario": id, "version": 1, "revision": 1, "repeats": 3,
        "varies": { "target": target, "pointer": "/workload/generator/orders_per_hour", "values": [200, 250, 300, 350, 400] } });
    let (status, made) = experiment(&router, sweep).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let made: Experiment = serde_json::from_value(made).expect("an experiment");
    assert_eq!(gate.world.wps().requests(), requests, "creating read nothing of the world");
    assert_eq!(cache.computations(), 0, "and checked nothing");
    assert_eq!(made.run_order.len(), 15);
    let first: Vec<(u32, u64)> = made.run_order[..5].iter().map(|slot| (slot.variant.0, slot.seed.0)).collect();
    assert_eq!(first, [(1, 0), (2, 0), (3, 0), (4, 0), (5, 0)]);
    assert_eq!(made.variants[3].label, "rate 350");
    assert!(made.variants.iter().all(|variant| variant.patches.len() == 1));

    let refusals = [
        json!({ "target": "d48891a3-bc89-5e90-8efa-3015e94cccb1", "pointer": "/stamp/count", "values": [1, 2] }),
        json!({ "target": target, "pointer": "/run_controls/duration_sim_ms", "values": [3_600_000, 7_200_000] }),
        json!({ "target": "00000000-0000-4000-8000-000000000001", "pointer": "/flow.node/work/pick_sim_ms", "values": [1, 2] }),
        json!([{ "target": target, "pointer": "/workload/generator/sku_count", "values": [1, 2] },
               { "target": target, "pointer": "/workload/generator/orders_per_hour", "values": [1, 2] }]),
        json!({ "target": target, "pointer": "/workload/generator/release", "values": ["all_at_start", "random"] }),
        json!({ "target": target, "pointer": "/workload/generator/wave_every_sim_ms", "values": [600_000, 900_000] }),
    ];
    for varies in refusals {
        let (status, refused) =
            experiment(&router, json!({ "scenario": id, "version": 1, "revision": 1, "varies": varies })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{varies}: {refused}");
    }
    let authored = json!({ "scenario": id, "version": 1, "revision": 1, "variants": [] });
    assert_eq!(experiment(&router, authored).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    let content: ScenarioContent = read(&router, &id, None).await.version.content;
    let cut = SiteReader::resolve_cut(&*gate.world, &SITE.parse().unwrap()).await.expect("the cut");
    let checks =
        validate_variants(&*gate.world, Arc::new(BlockingCompute), &made, &content, &cut).await.expect("checks");
    assert_eq!(checks.len(), 5, "one check per variant");
    let digests: Vec<&String> = checks.iter().filter_map(|check| check.outcome.workload_sha256.as_ref()).collect();
    assert_eq!(digests.len(), 5);
    assert!((1..5).all(|n| digests[n] != digests[n - 1]), "each rate its own workload");
    scratch.close().await;
}

/// Spec 07 section 5.1: a closed field offers only the values this build runs, and a sweep may vary no field with
/// fewer than two of them, nor the wave interval, which a run reads only with waves (Astra's cuts: the release is
/// every order at the start until dessim reads arrivals).
#[test]
fn the_catalogue_offers_only_the_values_this_build_runs_and_sweeps_none_it_cannot() {
    let field = |pointer: &str| catalogue().items.iter().find(|entry| entry.pointer == pointer).expect("the field");
    let release = field("/workload/generator/release");
    let offered: Vec<(&str, bool)> =
        release.options.iter().map(|option| (option.value.as_str(), option.accepted)).collect();
    assert_eq!(offered, [("all_at_start", true), ("even", false), ("random", false), ("waves", false)]);
    assert!(!release.sweepable && !field("/workload/generator/wave_every_sim_ms").sweepable);
    let traffic = field("/run_controls/floor_traffic");
    assert!(traffic.sweepable && traffic.options.iter().all(|option| option.accepted));
    assert!(field("/workload/generator/orders_per_hour").sweepable);
    let spec_sweeps: Vec<&str> = catalogue()
        .items
        .iter()
        .filter(|entry| entry.sweepable && !entry.pointer.starts_with("/workload/generator/"))
        .map(|entry| entry.pointer.as_str())
        .collect();
    assert_eq!(
        spec_sweeps,
        ["/run_controls/floor_traffic"],
        "spec.json sweeps must keep the compiled flow and workload so the runner can hash their shared baseline"
    );
}

#[tokio::test]
async fn duplicate_copies_a_version_delete_keeps_it_read_only_and_the_diff_names_the_fields() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let id = create(&router, "Peak hour", json!({})).await;
    edit(&router, &id, 1, 1, json!([{ "pointer": "/workload/generator/sku_count", "value": 450 }])).await;
    let (status, copy) = send(&router, "POST", &format!("/api/scenarios/{id}/duplicate"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::CREATED, "{copy}");
    let copy: Ulid = copy["scenario"].as_str().unwrap().parse().unwrap();
    let copied = read(&router, &copy, None).await;
    assert_eq!(copied.scenario.name, "Peak hour (copy)");
    assert_eq!(copied.version.content.workload.generator.sku_count.0, 450);
    let (_, again) = send(&router, "POST", &format!("/api/scenarios/{id}/duplicate"), Some(json!({}))).await;
    let again: Ulid = again["scenario"].as_str().unwrap().parse().unwrap();
    assert_eq!(read(&router, &again, None).await.scenario.name, "Peak hour (copy 2)");

    let (status, gone) = send(&router, "DELETE", &format!("/api/scenarios/{id}"), None).await;
    assert_eq!((status, gone["deleted_by"]["name"].clone()), (StatusCode::OK, json!("Person u-1")));
    let (status, refused) = edit(&router, &id, 1, 2, json!([{ "pointer": "/run_controls/seed", "value": 1 }])).await;
    assert_eq!((status, refused["type"].clone()), (StatusCode::CONFLICT, json!("scenario-deleted")));
    let (_, listed) = send(&router, "GET", &format!("/api/scenarios?site={SITE}"), None).await;
    assert!(listed["items"].as_array().unwrap().iter().all(|item| item["id"] != json!(id.to_string())));

    let (_, rename) =
        edit(&router, &copy, 1, 1, json!([{ "pointer": "/run_controls/duration_sim_ms", "value": 7_200_000 }])).await;
    assert_eq!(rename["revision"], 2);
    let mut tx = scratch.pool.begin().await.unwrap();
    let (_, sealer) = experiment(&router, json!({ "scenario": copy, "version": 1, "revision": 2 })).await;
    let sealer: Ulid = sealer["id"].as_str().unwrap().parse().unwrap();
    seal(&mut tx, &copy, VersionNumber(1), Revision(2), &sealer).await.expect("sealed");
    tx.commit().await.unwrap();
    edit(&router, &copy, 1, 2, json!([{ "pointer": "/workload/generator/seed", "value": 7 }])).await;
    let (status, diff) = send(&router, "GET", &format!("/api/scenarios/{copy}/diff?from=1&to=2"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        diff,
        json!({ "changes": [{ "pointer": "/workload/generator/seed", "label": "Generator seed", "from": 0, "to": 7 }] })
    );
    scratch.close().await;
}

/// Spec 01 section 3.6: a panel event lands as one `panel.event` entry and starts no turn; the same key answers the
/// same seq, and a closed conversation takes none.
#[tokio::test]
async fn a_panel_event_lands_once_in_the_sites_conversation() {
    let scratch = scratch_or_skip!();
    let Stack { router, .. } = stack(&scratch.pool).await;
    let (_, opened) = send(&router, "POST", "/api/conversations", Some(json!({ "site": SITE }))).await;
    let conversation = opened["conversation"]["id"].as_str().expect("a conversation").to_owned();
    let event = json!({ "kind": "scenario.changed", "text": "You changed Duration in Peak hour from 3 h to 2 h of simtime",
        "body": { "scenario": "01KK8AJ7RYA6GNNQ3QTRQCW7Z8", "version": 1, "revision": 2 } });
    let uri = format!("/api/conversations/{conversation}/panel-events");
    let request = |key: &str| {
        support::from_gate("POST", &uri, "u-1", "")
            .header("content-type", "application/json")
            .header("idempotency-key", key)
            .body(Body::from(event.to_string()))
            .expect("a request")
    };
    let (status, _, first) = support::call(&router, request("k-panel")).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, again) = support::call(&router, request("k-panel")).await;
    assert_eq!((status, again), (StatusCode::CREATED, first.clone()));
    let seq = serde_json::from_slice::<Value>(&first).unwrap()["seq"].clone();
    let (_, snapshot) = send(&router, "GET", &format!("/api/conversations/{conversation}"), None).await;
    let entries = snapshot["entries"].as_array().expect("entries");
    let written = entries.iter().find(|entry| entry["seq"] == seq).expect("the entry");
    assert_eq!(written["kind"], "panel.event");
    assert_eq!(written["body"]["kind"], "scenario.changed");
    assert!(written["turn"].is_null(), "no turn started");
    scratch.close().await;
}
