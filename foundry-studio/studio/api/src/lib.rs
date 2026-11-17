//! The Agent Studio API service (spec 01, the API service and the agent loop): the one door through which a person in
//! the browser and the agent act on Agent Studio's data. It answers on two listeners (spec 01 section 3.16): the
//! gate-facing one, reached only through the login gate, which reads the person from the gate's headers; and the
//! runners' one, which reads no identity and serves only the runners' door, each route opened by an attempt's token
//! (`runs`), whose loops the service runs beside its routes. What
//! the service keeps lives in Postgres, whose tables its embedded migrations create (`db`); a conversation's record
//! is its journal, which the in-process hub relays to the conversation's streams (`journal`). The world graph lives
//! in world-proto-server, reached only through `wps-client` (spec 04), over whichever server the service is built
//! with: HTTP when it serves, the client's in-memory mock in the tests. CPU-bound work runs on the compute pool
//! (`compute`), and what a computation leaves (an evaluation's plan and floor) goes to the run-object store.

pub mod agent;
pub mod approvals;
pub mod compute;
pub mod config;
pub mod conversations;
pub mod db;
pub mod dev;
pub mod drafts;
pub mod floors;
pub mod flows;
pub mod idempotency;
pub mod identity;
pub mod journal;
pub mod map;
pub mod problem;
pub mod projects;
pub mod results;
pub mod runs;
pub mod scenarios;
pub mod scope_events;
pub mod sites;
pub mod templates;
pub mod tools;

use std::future::IntoFuture;
use std::io;
use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, FromRef, Request, State};
use axum::http::Method;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use axum::routing::get;
use native_modules as _;
use runstore::RunStore;
use sqlx::PgPool;
use tokio::net::TcpListener;
use wps_client::wps::Wps;
use wps_client::{Client, Compute};

use crate::agent::Agent;
use crate::approvals::Executing;
use crate::config::{Deployment, RunConfig};
use crate::floors::Origins;
use crate::identity::IdentityRules;
use crate::journal::Hub;
use crate::map::Kept as KeptMaps;
use crate::runs::Runs;
use crate::templates::Kept;

/// The largest body any route takes: spec 01 section 3.6's 1 MiB of JSON, unless a spec states more.
pub const BODY_LIMIT: usize = 1 << 20;

const HEALTH_PATH: &str = "/healthz";

/// What every route reads: the database, how to tell who is asking, the hub that relays each conversation's journal
/// to its streams, where the service runs, and the model's name as a conversation's header shows it; the compute pool
/// the world client shares (spec 01 section 3.15) and the run-object store; the template reads kept per template
/// state, with the thumbnails being computed; the approvals this process's workers are executing;
/// what the run routes and loops share; and the agent loop's model, settings and running turns.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub identity: Arc<IdentityRules>,
    pub hub: Arc<Hub>,
    pub deployment: Deployment,
    pub model_label: Arc<str>,
    pub compute: Arc<dyn Compute>,
    pub runstore: RunStore,
    pub templates: Arc<Kept>,
    pub executing: Arc<Executing>,
    /// The cut each site floor key was minted for (spec 03 section 3.6).
    pub floor_origins: Arc<Origins>,
    /// Each site read the Map panel's routes loaded, by its floor key.
    pub maps: Arc<KeptMaps>,
    pub runs: Runs,
    pub agent: Arc<Agent>,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        identity: IdentityRules,
        deployment: Deployment,
        model_label: &str,
        compute: Arc<dyn Compute>,
        runstore: RunStore,
    ) -> Self {
        Self {
            pool,
            identity: Arc::new(identity),
            hub: Hub::new(),
            deployment,
            model_label: model_label.into(),
            compute,
            runstore,
            templates: Arc::default(),
            executing: Arc::default(),
            floor_origins: Arc::default(),
            maps: Arc::default(),
            runs: Runs::new(RunConfig::local("http://127.0.0.1:3101", "studio-simrunner:dev"), "http://127.0.0.1:7413"),
            agent: Arc::new(Agent::unconfigured()),
        }
    }

    /// The same state with `agent` as the agent loop's model and settings.
    pub fn with_agent(self, agent: Agent) -> Self {
        Self { agent: Arc::new(agent), ..self }
    }

    /// The same state with `runs` as what the run routes and loops share.
    pub fn with_runs(self, runs: Runs) -> Self {
        Self { runs, ..self }
    }
}

/// What the gate-facing routes read: `AppState`, and world-proto-server through its one client (spec 04 section 3.2)
/// over the server `S`, so a route that reads the world graph is tested against the mock. A route needing no world
/// still takes `State<AppState>`.
pub struct GateState<S: Wps> {
    pub app: AppState,
    pub world: Arc<Client<S>>,
}

impl<S: Wps> GateState<S> {
    pub fn new(app: AppState, world: Client<S>) -> Self {
        Self { app, world: Arc::new(world) }
    }
}

// By hand: a derive would ask `S` itself to be `Clone`, which no server is.
impl<S: Wps> Clone for GateState<S> {
    fn clone(&self) -> Self {
        Self { app: self.app.clone(), world: self.world.clone() }
    }
}

impl<S: Wps> FromRef<GateState<S>> for AppState {
    fn from_ref(state: &GateState<S>) -> Self {
        state.app.clone()
    }
}

/// The gate-facing listener's routes (spec 01 section 3.6), with the test writer only where the service runs locally,
/// and `not-found` for every method and path they do not serve, `/runner/` paths included. Every request but
/// `GET /healthz`, the deployment's probe, passes the identity layer first, one bound for `not-found` included, so it
/// is refused without the gate's identity or records its person (spec 01 section 3.2). Routes join above the identity
/// layer's line, so that it covers them.
pub fn gate_routes<S: Wps>(state: GateState<S>) -> Router {
    let routes = Router::new()
        .route(HEALTH_PATH, get(|| async { "ok" }))
        .route("/api/me", get(identity::me))
        .merge(conversations::routes())
        .merge(sites::routes())
        .merge(scope_events::routes())
        .merge(templates::routes(&state.app))
        .merge(approvals::routes())
        .merge(floors::routes())
        .merge(map::routes())
        .merge(scenarios::routes(&state.app))
        .merge(runs::routes(&state.app))
        .merge(results::routes(&state.app))
        .merge(conversations::panel_event_routes(&state.app))
        .merge(flows::routes())
        .merge(drafts::routes(&state.app))
        .merge(agent::routes(&state.app));
    let routes = if state.app.deployment == Deployment::Local { routes.merge(dev::routes()) } else { routes };
    routes
        .fallback(problem::not_found)
        .method_not_allowed_fallback(problem::not_found)
        .layer(from_fn_with_state(state.app.clone(), identify_unless_health))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

async fn identify_unless_health(state: State<AppState>, request: Request, next: Next) -> Response {
    if request.method() == Method::GET && request.uri().path() == HEALTH_PATH {
        return next.run(request).await;
    }
    identity::resolve(state, request, next).await
}

/// The runners' listener's routes: the runners' door alone (spec 01 section 3.16), each route opened by an attempt's
/// token, every other request `not-found`; no identity header is read here.
pub fn runner_routes(state: AppState) -> Router {
    runs::runner_routes(state)
}

/// Serve both listeners until either fails, meanwhile purging expired idempotency keys and revalidating the WPS
/// client's schema cache at the client's own cadence (spec 04 section 3.10), which loaded the cache when it started;
/// failing the plans a stopped process left preparing; and running the run loops with their watcher woken by each
/// stored transition, once the runners it left running are adopted; and stopping each turn whose wait outlives its
/// time, once a minute. The process settles what a stopped one left (`templates::recover`, `drafts::recover`, and
/// `agent::sweep`, which ends interrupted every turn it left running) before it serves.
pub async fn serve<S: Wps>(gate: TcpListener, runners: TcpListener, state: GateState<S>) -> io::Result<()> {
    tokio::spawn(agent::sweep_waits_every_minute(state.app.clone()));
    tokio::spawn(idempotency::purge_hourly(state.app.pool.clone()));
    let unfinished = state.app.clone();
    tokio::spawn(async move {
        if let Err(fault) = runs::settle_unfinished(&unfinished).await {
            eprintln!("studio-api: the plans a stopped process left preparing were not settled: {fault}");
        }
    });
    let loops = runs::start_loops(&state.app)
        .await
        .map_err(|fault| io::Error::other(format!("cannot adopt the runners a stopped process left: {fault}")))?;
    if loops.is_some() {
        tokio::spawn(runs::wake_on_transitions(state.app.clone()));
    }
    state.world.keep_schemas_fresh();
    let runners = axum::serve(runners, runner_routes(state.app.clone())).into_future();
    let gate = axum::serve(gate, gate_routes(state)).into_future();
    let served = tokio::try_join!(gate, runners).map(|_| ());
    drop(loops);
    served
}
