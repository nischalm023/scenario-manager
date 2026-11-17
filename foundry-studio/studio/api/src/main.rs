use std::env;
use std::io::stderr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use runstore::{RunStore, STORAGE_BUCKET_VAR, STORAGE_DIR_VAR};
use studio_api::agent::{self, Agent};
use studio_api::compute::ComputePool;
use studio_api::config::{Config, Deployment, WpsRoutes};
use studio_api::drafts;
use studio_api::projects::{LOCAL_DEFAULT_CUSTOMER, LOCAL_DEFAULT_NAME, make_local_default};
use studio_api::runs::{
    ContainerSetup, Runs, checkout_source, cluster_image, cluster_setup, find_engine, inspect_image, local_image,
    resume,
};
use studio_api::templates::recover;
use studio_api::{AppState, GateState, db, floors, serve};
use tokio::net::TcpListener;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry;
use tracing_subscriber::util::SubscriberInitExt;
use url::Url;
use wps_client::routes::{RouteShape, Routes};
use wps_client::{Client, ClientConfig, Compute, HttpConfig, HttpWps};

#[tokio::main]
async fn main() -> ExitCode {
    // The API's own lines at info and up (an evaluation's timing among them), every other crate's at warn and up.
    let lines = Targets::new().with_target("studio_api", Level::INFO).with_default(Level::WARN);
    registry().with(layer().with_writer(stderr).with_ansi(false)).with(lines).init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("studio-api: {failure}");
            ExitCode::FAILURE
        }
    }
}

/// Read the configuration, bring the database up to this build's tables (locally with a default project), start the
/// compute pool and the world-proto-server client, whose probe refuses a server it cannot use, open the run-object
/// store, bind both listeners, settle what a stopped process left (template work, the drafts it left being applied,
/// the submissions it left unsettled and the turns it left running), and serve. The settling comes before any request
/// is accepted: this one API process then runs every write it finds (LEAD-DECISIONS entry 54).
async fn run() -> Result<(), String> {
    let config = Config::from_env().map_err(|failure| failure.to_string())?;
    let pool = db::connect(&config.database_url)
        .await
        .map_err(|failure| format!("cannot reach the database STUDIO_DATABASE_URL names: {failure}"))?;
    db::migrate(&pool).await.map_err(|failure| format!("cannot bring the database up to this build: {failure}"))?;
    if config.deployment == Deployment::Local
        && make_local_default(&pool).await.map_err(|failure| format!("cannot make the default project: {failure}"))?
    {
        eprintln!(
            "studio-api made the default project, {LOCAL_DEFAULT_NAME}, for the customer {LOCAL_DEFAULT_CUSTOMER}"
        );
    }
    let compute: Arc<dyn Compute> =
        Arc::new(ComputePool::new(config.compute).map_err(|fault| format!("cannot start the compute pool: {fault}"))?);
    let world = world(&config, compute.clone()).await?;
    let runstore = RunStore::from_env().map_err(|fault| {
        format!("cannot open the run-object store that {STORAGE_DIR_VAR} or {STORAGE_BUCKET_VAR} names: {fault}")
    })?;
    let (gate, runners) = (bind(config.bind).await?, bind(config.runner_bind).await?);
    let (gate_address, runner_address) = (local(&gate)?, local(&runners)?);
    let runs = runs(&config).await?;
    if let Some(image) = &runs.image {
        eprintln!("studio-api runs the runner image {}, built from {}", image.digest, image.commit);
    }
    let agent = Agent::from_config(config.agent.clone(), &config.model)?;
    eprintln!("studio-api's agent runs on {:?}'s {} ({})", agent.provider, agent.model_id, config.model.label);
    let state = AppState::new(pool, config.identity, config.deployment, &config.model.label, compute, runstore);
    let state = state.with_runs(runs).with_agent(agent);
    resume(&state)
        .await
        .map_err(|failure| format!("cannot resume the submissions a stopped process left: {failure}"))?;
    let state = GateState::new(state, world);
    recover(&state).await.map_err(|failure| format!("cannot settle what a stopped process left: {failure}"))?;
    drafts::recover(&state)
        .await
        .map_err(|failure| format!("cannot resume the drafts left being applied: {failure}"))?;
    agent::sweep(&state.app)
        .await
        .map_err(|failure| format!("cannot interrupt the turns a stopped process left running: {failure}"))?;
    eprintln!("studio-api answers people on http://{gate_address} and runners on http://{runner_address}");
    eprintln!("studio-api keys its floors by its build, {}", floors::build_commit());
    serve(gate, runners, state).await.map_err(|failure| failure.to_string())
}

/// What the run routes and loops share. The one runner image is `STUDIO_RUNNER_IMAGE` (spec 08 section 3.5), locally
/// run as a container per attempt by this machine's engine, which must hold it built from this checkout's source, in a
/// cluster as a Job per attempt in the namespace `STUDIO_RUNNER_NAMESPACE` names, else this pod's own; either way
/// each runner is handed the runner settings of this environment the configuration names, and no other variable.
async fn runs(config: &Config) -> Result<Runs, String> {
    let wps = config.wps.url.to_string();
    let runs = Runs::new(config.runs.clone(), wps.trim_end_matches('/'));
    let reference = &config.runs.runner_image;
    if config.deployment == Deployment::Local {
        let engine = find_engine(config.runs.container_engine.as_deref())
            .ok_or("no container engine: docker or podman on PATH, or STUDIO_CONTAINER_ENGINE naming one")?;
        let image = local_image(reference, &inspect_image(&engine, reference).await?, &checkout_source()?)?;
        let objects = env::var(STORAGE_DIR_VAR).map(PathBuf::from).map_err(|_| {
            format!("the container invoker mounts each runner's job spec from {STORAGE_DIR_VAR}'s folder: set it")
        })?;
        let out = config.runs.runner_out.clone().unwrap_or_else(|| env::temp_dir().join("studio-runner-out"));
        let setup = ContainerSetup {
            passed: config.runs.runner_passed.clone(),
            user: config.runs.container_user.clone(),
            stack: config.runs.container_stack.clone(),
            ..ContainerSetup::new(engine, objects, out)
        };
        return Ok(runs.container(image, setup));
    }
    let branch = config.runs.build_branch.as_deref().unwrap_or_default();
    let image = cluster_image(reference, floors::build_commit(), branch);
    Ok(runs.kube(image, cluster_setup(&config.runs)?))
}

/// The client of the world-proto-server the configuration names, started: probed, writing nothing, and its schema
/// cache loaded (spec 04 sections 3.12 and 3.10), its exports checked on the compute pool (spec 01 section 3.15).
async fn world(config: &Config, compute: Arc<dyn Compute>) -> Result<Client<HttpWps>, String> {
    let url = &config.wps.url;
    let base = Url::parse(&url.to_string()).map_err(|fault| format!("STUDIO_WPS_URL={url}: {fault}"))?;
    let shape = match config.wps.routes {
        WpsRoutes::Path => RouteShape::Path,
    };
    let routes = Routes::new(base, shape)?;
    let wps = HttpWps::new(HttpConfig::new(routes)).map_err(|fault| fault.to_string())?;
    let client = ClientConfig { revision: config.wps.revision.clone(), ..ClientConfig::default() };
    Client::start(wps, client, compute)
        .await
        .map_err(|fault| format!("cannot start against the world-proto-server at {url}: {fault}"))
}

async fn bind(address: SocketAddr) -> Result<TcpListener, String> {
    TcpListener::bind(address).await.map_err(|failure| format!("cannot listen on {address}: {failure}"))
}

fn local(listener: &TcpListener) -> Result<SocketAddr, String> {
    listener.local_addr().map_err(|failure| failure.to_string())
}
