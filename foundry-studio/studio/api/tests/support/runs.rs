//! What the run tests share (brief I1.09; spec 08): a stack over the mock world-proto-server whose site dessim loads,
//! with the first cut's one local image; a scenario and an experiment made through the routes; a plan prepared
//! (section 3.3, the submission plan) and pressed and executed (section 3.7, the run press) through the API's own
//! functions, as the routes' tasks run them; a run reserved as the dispatcher reserves one, and a request on the
//! runners' listener with its token; an invoker (section 3.5's) that only records what the loops ask of it and
//! answers the statuses a test sets; and the API's store (section 3.4's) with a pause after a reservation commits.

use std::env;
use std::mem::take;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use run_invocation::{
    AttemptKey, AttemptRow, Creator, Cursor, Ended, Fault, HeldAttempt, Invoker, JobName, JobRequest, JobStatus,
    Reserved, Round, RunStore, RunningProgress, Seats, Token, Transition,
};
use rustc_hash::FxHashMap;
use scenario_spec::{COMPILER, this_build_accepts};
use serde_json::{Value, from_slice, json};
use sqlx::{Connection, PgConnection, PgPool};
use studio_api::config::RunConfig;
use studio_api::identity::Identity;
use studio_api::projects::make_local_default;
use studio_api::runs::{PgRunStore, Runs, execute, prepare, press, start_plan};
use studio_api::scenarios::SiteReader;
use studio_api::{GateState, gate_routes};
use studio_protocol::{
    Ending, Experiment, Plan, PlanRequest, RunnerImage, ScopePath, StreamingServer, SubmitRequest, Ulid, WallClock,
};
use tokio::sync::Notify;
use tokio::time::sleep;
use wps_client::wps::Wps;
use wps_client::{Client, MockWps};

use super::sites::{fixture_site, world};

pub const SITE: &str = "customer/demo/site/lshape-6";
pub const PERSON: &str = "u-1";

/// The first cut's one image, as a test names it.
pub fn image() -> RunnerImage {
    RunnerImage {
        digest: "local:simrunner@test".to_owned(),
        tag: "test".to_owned(),
        commit: "0".repeat(40),
        branch: "main".to_owned(),
        registered_at: WallClock::now(),
        compiler: COMPILER.to_owned(),
        accepts: this_build_accepts(),
        default: true,
    }
}

pub struct RunStack<S: Wps = MockWps> {
    pub gate: GateState<S>,
    pub router: Router,
    pub site: ScopePath,
}

/// The run settings of a local service whose runners reach it at its default runners' listener.
pub fn config() -> RunConfig {
    RunConfig::local("http://127.0.0.1:3101", "studio-simrunner:dev")
}

/// A stack whose site loads, run settings at `config`, its runners reaching `runner_url`.
pub async fn stack(pool: &PgPool, config: RunConfig) -> RunStack {
    let world: Client<MockWps> = world().await;
    let site = fixture_site(&world, SITE, true).await;
    stack_over(pool, config, world, site).await
}

/// A stack over `world`, whose site `site` the test made, run settings at `config`.
pub async fn stack_over(pool: &PgPool, config: RunConfig, world: Client<MockWps>, site: ScopePath) -> RunStack {
    make_local_default(pool).await.expect("the default project");
    super::person(pool, PERSON).await;
    let mut runs = Runs::new(config, "http://127.0.0.1:7413");
    runs.image = Some(image().into());
    let app = super::state(pool.clone(), super::gate_rules()).with_runs(runs);
    let gate = GateState::new(app, world);
    RunStack { router: gate_routes(gate.clone()), gate, site }
}

pub fn identity() -> Identity {
    Identity {
        subject: PERSON.parse().expect("a subject"),
        email: format!("{PERSON}@example.com"),
        name: format!("Person {PERSON}"),
        developer: false,
    }
}

/// A request as the gate forwards it for `u-1`, a fresh `Idempotency-Key` on every POST.
pub async fn send(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = super::from_gate(method, uri, PERSON, "").header("content-type", "application/json");
    if method == "POST" {
        request = request.header("idempotency-key", super::ulid().to_string());
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, _, answer) = super::call(router, request.body(body).expect("a request")).await;
    (status, if answer.is_empty() { Value::Null } else { serde_json::from_slice(&answer).unwrap_or(Value::Null) })
}

/// A request on the runners' listener, the token as `Authorization: Bearer` when given.
pub async fn door(
    router: &Router,
    method: &str,
    uri: &str,
    token: Option<&Token>,
    body: Option<Vec<u8>>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {}", token.expose()));
    }
    let body = body.map_or_else(Body::empty, Body::from);
    let (status, _, answer) = super::call(router, request.body(body).expect("a request")).await;
    (status, from_slice(&answer).unwrap_or(Value::Null))
}

/// One submitted run of a new scenario `name`, reserved as the dispatcher reserves it: its plan, key and token.
pub async fn reserved(stack: &RunStack, name: &str) -> (Plan, AttemptKey, Token) {
    let scenario = scenario(stack, name).await;
    let made = experiment(stack, &scenario, 1, &[]).await;
    let (plan, hash) = plan(stack, &made.id).await;
    submit(stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let reserved = store.reserve(Seats(5), &Creator::mint()).await.expect("a reservation").expect("a waiting attempt");
    (plan, reserved.key, reserved.token)
}

/// Peak hour on the stack's site at its stated defaults, made through the route.
pub async fn scenario<S: Wps>(stack: &RunStack<S>, name: &str) -> Ulid {
    let body = json!({ "name": name, "site_scope": stack.site.to_string() });
    let (status, made) = send(&stack.router, "POST", "/api/scenarios", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["scenario"].as_str().expect("its id").parse().expect("a ULID")
}

/// Peak hour, cut to `minutes` of simtime, for the tests that run simrunner for real: the Rectangle at its defaults
/// simulates four hours in about 90 s of wall clock on a debug build, past `until`'s 30 s, and ten minutes in a few
/// seconds with picks in them. Stated at creation, so the version stays at revision 1 as the callers pin it.
pub async fn short_scenario<S: Wps>(stack: &RunStack<S>, name: &str, minutes: u64) -> Ulid {
    let body = json!({
        "name": name,
        "site_scope": stack.site.to_string(),
        "run_controls": {
            "duration_sim_ms": minutes * 60_000,
            "seed": 0,
            "floor_traffic": "reserved",
            "report_window": { "from_sim_ms": 0, "to_sim_ms": null, "before_name": "warm-up" },
        },
    });
    let (status, made) = send(&stack.router, "POST", "/api/scenarios", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["scenario"].as_str().expect("its id").parse().expect("a ULID")
}

/// An experiment of `scenario` at its version 1 and `revision`, sweeping orders per hour over `values` when given.
pub async fn experiment<S: Wps>(stack: &RunStack<S>, scenario: &Ulid, revision: u32, values: &[u32]) -> Experiment {
    let mut body = json!({ "scenario": scenario, "version": 1, "revision": revision });
    if !values.is_empty() {
        body["varies"] = json!({
            "target": format!("scenario:{scenario}"),
            "pointer": "/workload/generator/orders_per_hour",
            "values": values,
        });
    }
    let (status, made) = send(&stack.router, "POST", "/api/experiments", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    serde_json::from_value(made).expect("an experiment")
}

/// A plan of `experiment` prepared through `reader` as the route's task prepares it: its id, and once ready the plan
/// with its hash.
pub async fn plan_with<S: Wps, R: SiteReader>(
    stack: &RunStack<S>,
    reader: &R,
    experiment: &Ulid,
) -> (Ulid, Option<(Plan, String)>) {
    let request = PlanRequest { experiment: experiment.clone(), cut: None };
    let (id, fresh) = start_plan(&stack.gate.app, &identity(), &request, "/api/runs/plan").await.expect("a plan");
    assert!(fresh, "a new plan is prepared");
    prepare(&stack.gate.app, reader, &id).await;
    let (plan, hash, state, problem): (Option<Value>, Option<String>, String, Option<Value>) =
        sqlx::query_as("SELECT plan, plan_sha256, state, problem FROM plans WHERE id = $1")
            .bind(id.as_str())
            .fetch_one(&stack.gate.app.pool)
            .await
            .expect("the plan's row");
    let ready = plan.zip(hash).map(|(plan, hash)| (serde_json::from_value(plan).expect("a plan"), hash));
    if ready.is_none() {
        eprintln!("plan {id} is {state}: {}", problem.map(|problem| problem.to_string()).unwrap_or_default());
    }
    (id, ready)
}

pub async fn plan<S: Wps>(stack: &RunStack<S>, experiment: &Ulid) -> (Plan, String) {
    let (id, ready) = plan_with(stack, &*stack.gate.world, experiment).await;
    ready.unwrap_or_else(|| panic!("plan {id} is not ready; its state and problem are above"))
}

/// Press Run on `plan` as the person, then execute it as the route's task does, to its end; its approval.
pub async fn submit<S: Wps>(stack: &RunStack<S>, plan: &Plan, hash: &str) -> Ulid {
    let request = SubmitRequest { plan: plan.id.clone(), plan_sha256: hash.to_owned() };
    let approval = press(&stack.gate.app, &identity(), &request, "/api/runs").await.expect("the press is recorded").id;
    execute(stack.gate.app.clone(), approval.clone()).await;
    approval
}

/// A connection of the test's own on `pool`'s schema, outside the stack's pool, to hold a lock with.
pub async fn own_connection(pool: &PgPool) -> PgConnection {
    let schema: String = sqlx::query_scalar("SELECT current_schema()").fetch_one(pool).await.expect("the schema");
    let url = env::var(super::DATABASE_VAR).expect("the test database");
    let mut conn = PgConnection::connect(&url).await.expect("a connection of the test's own");
    sqlx::query(&format!("SET search_path TO {schema}")).execute(&mut conn).await.expect("the scratch schema");
    conn
}

/// Wait until `count` statements of this database that read `table` wait on a lock.
pub async fn waiting_on(pool: &PgPool, table: &str, count: i64) {
    until("the statements to wait on the lock", || async {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' \
             AND query LIKE '%FROM ' || $1 || '%'",
        )
        .bind(table)
        .fetch_one(pool)
        .await
        .expect("the waiting statements");
        waiting >= count
    })
    .await;
}

/// Rows of `table` in the stack's schema.
pub async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}")).fetch_one(pool).await.expect("a count")
}

/// Wait (wall clock, 120 s at most) until `done` holds. The bound only decides how soon a failing test says so: a
/// real ten-minute run of the Rectangle takes some 20 s of wall clock alone in a debug build and twice that when the
/// local suite's runners share the machine.
pub async fn until<F: Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done().await {
        assert!(Instant::now() < deadline, "{what} never happened");
        sleep(Duration::from_millis(20)).await;
    }
}

/// An invoker that records every start and stop, answers each Job's status as the test sets it (running once
/// started, absent before; a stop is only asked for, so a stopped Job runs until the test ends it), and can hold a
/// named start, or the first status asked of a named Job, until the test lets it return (`asked` says the status
/// waits).
#[derive(Default)]
pub struct FakeInvoker {
    pub starts: Mutex<Vec<JobRequest>>,
    pub stops: Mutex<Vec<JobName>>,
    pub statuses: Mutex<FxHashMap<JobName, JobStatus>>,
    pub held: Mutex<Option<(JobName, Arc<Notify>)>>,
    pub held_status: Mutex<Option<(JobName, Arc<Notify>)>>,
    pub asked: Notify,
}

impl FakeInvoker {
    pub fn set(&self, name: &JobName, status: JobStatus) {
        self.statuses.lock().expect("the statuses").insert(name.clone(), status);
    }

    pub fn started(&self) -> Vec<JobRequest> {
        self.starts.lock().expect("the starts").clone()
    }

    pub fn stopped(&self) -> Vec<JobName> {
        self.stops.lock().expect("the stops").clone()
    }
}

impl Invoker for FakeInvoker {
    async fn start(&self, request: JobRequest) -> Result<(), Fault> {
        let held = self.held.lock().expect("the hold").clone();
        if let Some((name, gate)) = held
            && name == request.name
        {
            gate.notified().await;
        }
        self.statuses.lock().expect("the statuses").entry(request.name.clone()).or_insert(JobStatus::Running);
        self.starts.lock().expect("the starts").push(request);
        Ok(())
    }

    async fn stop(&self, name: &JobName) -> Result<(), Fault> {
        self.stops.lock().expect("the stops").push(name.clone());
        Ok(())
    }

    async fn status(&self, name: &JobName) -> Result<JobStatus, Fault> {
        let hold = {
            let mut held = self.held_status.lock().expect("the status hold");
            if held.as_ref().is_some_and(|(held, _)| held == name) { held.take() } else { None }
        };
        if let Some((_, gate)) = hold {
            self.asked.notify_one();
            gate.notified().await;
        }
        Ok(self.statuses.lock().expect("the statuses").get(name).cloned().unwrap_or(JobStatus::Absent))
    }
}

/// The API's store, except that its first reservation, once committed, waits for the test before the dispatcher hears
/// of it (`paused` says it waits; `go` lets it on), so a reconciler pass falls between a reservation's commit and its
/// create.
pub struct PausingStore {
    pub inner: PgRunStore,
    pub paused: Notify,
    pub go: Notify,
    armed: Mutex<bool>,
}

impl PausingStore {
    pub fn new(inner: PgRunStore) -> Self {
        Self { inner, paused: Notify::new(), go: Notify::new(), armed: Mutex::new(true) }
    }
}

impl RunStore for PausingStore {
    async fn reserve(&self, limit: Seats, creator: &Creator) -> Result<Option<Reserved>, Fault> {
        let reserved = self.inner.reserve(limit, creator).await?;
        let pause = reserved.is_some() && take(&mut *self.armed.lock().expect("the pause"));
        if pause {
            self.paused.notify_one();
            self.go.notified().await;
        }
        Ok(reserved)
    }

    async fn release(&self, key: &AttemptKey, creator: &Creator) -> Result<(), Fault> {
        self.inner.release(key, creator).await
    }

    async fn attempt(&self, key: &AttemptKey) -> Result<Option<AttemptRow>, Fault> {
        self.inner.attempt(key).await
    }

    async fn end(&self, key: &AttemptKey, ending: &Ending) -> Result<Ended, Fault> {
        self.inner.end(key, ending).await
    }

    async fn end_uncreated(&self, key: &AttemptKey, ending: &Ending) -> Result<Ended, Fault> {
        self.inner.end_uncreated(key, ending).await
    }

    async fn job_stopped(&self, key: &AttemptKey) -> Result<(), Fault> {
        self.inner.job_stopped(key).await
    }

    async fn held(&self) -> Result<Vec<HeldAttempt>, Fault> {
        self.inner.held().await
    }

    async fn reclaim(&self, key: &AttemptKey, creator: &Creator, grace: Duration) -> Result<Option<Token>, Fault> {
        self.inner.reclaim(key, creator, grace).await
    }

    async fn cursor(&self) -> Result<Cursor, Fault> {
        self.inner.cursor().await
    }

    async fn transitions_after(&self, cursor: Cursor) -> Result<Vec<Transition>, Fault> {
        self.inner.transitions_after(cursor).await
    }

    async fn advance(&self, cursor: Cursor) -> Result<(), Fault> {
        self.inner.advance(cursor).await
    }

    async fn progress_of_running(&self) -> Result<Vec<RunningProgress>, Fault> {
        self.inner.progress_of_running().await
    }

    async fn round_ended(&self, at: &Transition) -> Result<Option<Round>, Fault> {
        self.inner.round_ended(at).await
    }

    async fn platform_status(&self, key: &AttemptKey, words: &str) -> Result<(), Fault> {
        self.inner.platform_status(key, words).await
    }

    async fn stream_to(&self, key: &AttemptKey, stale_after: Duration) -> Result<Option<StreamingServer>, Fault> {
        self.inner.stream_to(key, stale_after).await
    }
}
