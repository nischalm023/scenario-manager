//! Within `studio-api`, `runs` are spec 08's simulation runs (brief I1.09): the submission plan prepared and checked
//! before anyone approves it (`plan`), a press's execution into a submission, its runs and their queued attempts
//! (`submit`), the run list, one run, the queue and an attempt's manifest (`read`), cancel (`cancel`), a ticket to
//! watch a run live on its streaming server (`watch`), the runner image (`image`), the runners' door on the second
//! listener with the findings a manifest raises (`door`, `findings`), and the run loops' store and events over the
//! API's tables and journal (`store`, `events`) with the local invoker, a container of the runner image per attempt
//! (`container`), or in a cluster the Jobs invoker (`kube-invoker`). Retry, the workbook and the result routes are
//! not here.

mod cancel;
mod container;
mod door;
mod events;
mod findings;
mod image;
mod plan;
mod read;
mod rows;
mod store;
mod submit;
mod watch;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use kube_invoker::{KubeInvoker, KubeSetup, own_namespace};
use run_invocation::{Fault, Invoker, LoopSettings, Loops, Resources, Running, Wakes};
use sqlx::postgres::PgListener;
use studio_protocol::{Endpoint, RunnerImage};
use tokio::time::sleep;
use wps_client::wps::Wps;

use crate::config::RunConfig;
use crate::idempotency::idempotent;
use crate::{AppState, GateState};

pub use container::{BakedImage, ContainerInvoker, ContainerSetup, HOST_ALIAS, find_engine, inspect_image};
pub use events::JournalEvents;
pub use findings::{Checked, findings};
pub use image::{checkout_source, cluster_image, local_image};
pub use plan::{plan_sha256, prepare, run_refusals, settle_unfinished, start as start_plan};
pub use read::submission_runs;
pub use read::{no_run, run_in};
pub use rows::{json, milliseconds, summary_of};
pub use store::{PgRunStore, Recording, Results, ResultsSource};
pub use submit::{execute, press, resume};

/// The channel migration 0017's trigger notifies on each committed change of an attempt's state.
const TRANSITIONS: &str = "studio_attempt_transitions";

/// What every Job in the cluster is told, from the run settings: the namespace `STUDIO_RUNNER_NAMESPACE` names, else
/// the one this process's pod runs in, the runner pod's service account, and the runner settings `STUDIO_RUNNER_PASSED`
/// names, which are all of this process's environment a runner is given (ruling D174).
pub fn cluster_setup(config: &RunConfig) -> Result<KubeSetup, String> {
    let namespace = config.runner_namespace.clone().or_else(own_namespace).ok_or(
        "STUDIO_RUNNER_NAMESPACE names no namespace, and this process runs in no pod whose namespace it could read",
    )?;
    let (service_account, passed) = (config.runner_service_account.clone(), config.runner_passed.clone());
    Ok(KubeSetup { service_account, passed, ..KubeSetup::new(namespace) })
}

/// Which invoker the loops run over, and what it is told: the local one, a container of the runner image per attempt,
/// or the cluster's, a Job per attempt.
#[derive(Clone)]
pub enum InvokerSetup {
    Container(Arc<ContainerSetup>),
    Kube(Arc<KubeSetup>),
}

/// What the run routes and loops share: the run settings, the image a plan names (none until an invoker is set up),
/// world-proto-server's address as a runner reaches it (`STUDIO_RUNNER_WPS_URL`, else the API's own), the loops'
/// wakes, and the invoker's setup.
#[derive(Clone)]
pub struct Runs {
    pub config: Arc<RunConfig>,
    pub image: Option<Arc<RunnerImage>>,
    pub wps: Arc<str>,
    pub wakes: Arc<Wakes>,
    pub invoker: Option<InvokerSetup>,
}

impl Runs {
    /// Over `wps`, world-proto-server's address as the API reaches it, which a runner reaches too unless the
    /// configuration names another.
    pub fn new(config: RunConfig, wps: &str) -> Self {
        let runner_wps = config.runner_wps_url.clone().unwrap_or_else(|| wps.to_owned());
        Self { config: Arc::new(config), image: None, wps: runner_wps.into(), wakes: Arc::default(), invoker: None }
    }

    /// With the one runner image and the local invoker, a container of it per attempt.
    pub fn container(self, image: RunnerImage, setup: ContainerSetup) -> Self {
        Self { image: Some(Arc::new(image)), invoker: Some(InvokerSetup::Container(Arc::new(setup))), ..self }
    }

    /// With the cluster's one runner image and the Jobs invoker.
    pub fn kube(self, image: RunnerImage, setup: KubeSetup) -> Self {
        Self { image: Some(Arc::new(image)), invoker: Some(InvokerSetup::Kube(Arc::new(setup))), ..self }
    }

    /// The container invoker's setup, when that is the invoker.
    pub fn container_setup(&self) -> Option<&Arc<ContainerSetup>> {
        match &self.invoker {
            Some(InvokerSetup::Container(setup)) => Some(setup),
            _ => None,
        }
    }

    /// The loops' settings from the run configuration.
    pub fn loop_settings(&self) -> LoopSettings {
        let config = &self.config;
        let mut settings = LoopSettings::new(Endpoint { endpoint: config.runner_url.clone() });
        settings.seats = config.at_once;
        settings.deadline = config.deadline;
        settings.resources = Resources { cpu_millis: config.cpu_millis, memory_mib: config.memory_mib };
        settings.start_timeout = config.start_timeout;
        settings.cancel_grace = config.cancel_grace;
        settings.streaming_stale_after = config.streaming_stale_after;
        settings
    }
}

/// The gate-facing run routes (spec 08 section 3.7) this build serves.
pub fn routes<S: Wps>(app: &AppState) -> Router<GateState<S>> {
    Router::new()
        .route("/api/runs/plan", idempotent(app, post(plan::ask::<S>)))
        .route("/api/runs/plan/{plan}", get(plan::read))
        .route("/api/runs", idempotent(app, post(submit::submit::<S>)).get(read::list))
        .route("/api/runs/queue", get(read::queue))
        .route("/api/runs/stats", get(read::stats))
        .route("/api/runs/{run}", get(read::detail))
        .route("/api/runs/{run}/manifest", get(read::manifest))
        .route("/api/runs/{run}/cancel", idempotent(app, post(cancel::run)))
        .route("/api/runs/{run}/watch", post(watch::ticket))
        .route("/api/submissions/{submission}/cancel", idempotent(app, post(cancel::submission)))
        .route("/api/runner-images", get(image::list))
}

/// The runners' listener's routes: the runners' door alone.
pub fn runner_routes(app: AppState) -> Router {
    door::routes(app)
}

/// Wake the watcher whenever a transaction that changed an attempt's state commits (the trigger's `NOTIFY`), for as
/// long as the service runs; its own tick reads the log anyway, so a lost connection only slows it down.
pub async fn wake_on_transitions(app: AppState) {
    loop {
        match PgListener::connect_with(&app.pool).await {
            Ok(mut listener) => {
                if listener.listen(TRANSITIONS).await.is_ok() {
                    while listener.recv().await.is_ok() {
                        app.runs.wakes.watch();
                    }
                }
            }
            Err(fault) => eprintln!("studio-api: the watcher hears no transitions until it reconnects: {fault}"),
        }
        sleep(Duration::from_secs(5)).await;
    }
}

/// Start the run loops over the API's tables and journal with the invoker set up, if one is: the local one once it
/// has adopted the containers a stopped process left running, which must succeed (a runner left unadopted would read
/// as lost and its seat free while it runs), or the cluster's, which adopts nothing because the cluster is the record.
/// The loops run until the answer is dropped.
pub async fn start_loops(app: &AppState) -> Result<Option<Running>, Fault> {
    match app.runs.invoker.clone() {
        None => Ok(None),
        Some(InvokerSetup::Container(setup)) => {
            let invoker = ContainerInvoker::new(app.clone(), setup);
            let adopted = invoker.adopt().await?;
            if !adopted.is_empty() {
                let names: Vec<String> = adopted.iter().map(ToString::to_string).collect();
                eprintln!("studio-api adopted the runners' containers a stopped process left: {}", names.join(", "));
            }
            Ok(Some(spawn_loops(app, invoker)))
        }
        Some(InvokerSetup::Kube(setup)) => {
            let invoker = KubeInvoker::from_env((*setup).clone()).await?;
            eprintln!("studio-api runs its attempts as Jobs in the namespace {}", setup.namespace);
            Ok(Some(spawn_loops(app, invoker)))
        }
    }
}

fn spawn_loops<I: Invoker>(app: &AppState, invoker: I) -> Running {
    Loops::new(
        Arc::new(PgRunStore::new(app.clone())),
        Arc::new(JournalEvents::new(app.clone())),
        Arc::new(invoker),
        app.runs.loop_settings(),
        app.runs.wakes.clone(),
    )
    .spawn()
}
