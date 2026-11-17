//! What the run-loop tests share (spec 08 sections 3.4 and 3.6): the loops of `run-invocation` over a stack's tables
//! and journal with an invoker that only records what it is asked, every wait short; runs submitted and the seats they
//! hold; an attempt's key, an ending and a runner's report of one; and a create the invoker holds until the test lets
//! it return.

use std::sync::Arc;
use std::time::Duration;

use desunits::SimTime;
use run_invocation::{AttemptKey, JobName, LoopSettings, Loops, RunStore, Running};
use sqlx::PgPool;
use studio_api::runs::{JournalEvents, PgRunStore, Results, ResultsSource};
use studio_protocol::{AttemptNumber, EndedBy, Ending, Endpoint, RunState, Ulid};
use tokio::sync::Notify;

use super::runs::{FakeInvoker, RunStack, experiment, plan, scenario, submit, until};

/// The loops over the stack's tables and journal with `invoker`, every wait short so a test runs in seconds.
pub fn loops(stack: &RunStack, invoker: Arc<FakeInvoker>) -> Running {
    loops_with(stack, Arc::new(PgRunStore::new(stack.gate.app.clone())), invoker)
}

/// The same loops over `store`.
pub fn loops_with<S: RunStore>(stack: &RunStack, store: Arc<S>, invoker: Arc<FakeInvoker>) -> Running {
    loops_told(stack, store, invoker, short_waits())
}

/// The deployment's settings, every wait short.
pub fn short_waits() -> LoopSettings {
    let mut settings = LoopSettings::new(Endpoint { endpoint: "http://127.0.0.1:3101".to_owned() });
    settings.create_retries = vec![Duration::from_millis(10)];
    settings.dispatch_every = Duration::from_millis(50);
    settings.watch_every = Duration::from_millis(50);
    settings.reconcile_every = Duration::from_millis(50);
    settings.lost_grace = Duration::from_millis(200);
    settings.exit_grace = Duration::from_secs(1);
    settings.create_grace = Duration::from_millis(100);
    settings
}

/// The loops over `store`, told `settings`.
pub fn loops_told<S: RunStore>(
    stack: &RunStack,
    store: Arc<S>,
    invoker: Arc<FakeInvoker>,
    settings: LoopSettings,
) -> Running {
    let app = &stack.gate.app;
    Loops::new(store, Arc::new(JournalEvents::new(app.clone())), invoker, settings, app.runs.wakes.clone()).spawn()
}

/// The hash of the token the attempt of `run` holds now.
pub async fn token_hash(pool: &PgPool, run: &Ulid) -> String {
    sqlx::query_scalar("SELECT token_sha256 FROM runner_tokens WHERE run = $1")
        .bind(run.as_str())
        .fetch_one(pool)
        .await
        .expect("the attempt's token")
}

/// `count` runs of one submission, queued: a sweep of orders per hour.
pub async fn submitted(stack: &RunStack, name: &str, count: u32) -> Vec<Ulid> {
    let scenario = scenario(stack, name).await;
    let values: Vec<u32> = (0..count).map(|n| 100 + 10 * n).collect();
    let made = experiment(stack, &scenario, 1, if count > 1 { &values } else { &[] }).await;
    let (plan, hash) = plan(stack, &made.id).await;
    submit(stack, &plan, &hash).await;
    plan.runs.iter().map(|run| run.run.clone()).collect()
}

/// The seats held: attempts with a Job not known stopped.
pub async fn held(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM attempts WHERE job IS NOT NULL AND job_stopped_at IS NULL")
        .fetch_one(pool)
        .await
        .expect("the seats held")
}

/// An ending the simulator wrote, of `state`.
pub fn ending(state: RunState) -> Ending {
    Ending {
        state,
        ended_by: EndedBy::Simulator,
        stop_reason: "SimTimeLimit".to_owned(),
        stop_detail: None,
        sim_ms: SimTime::ms(1),
    }
}

/// The first attempt of `run`.
pub fn key(run: &Ulid) -> AttemptKey {
    AttemptKey { run: run.clone(), n: AttemptNumber(1) }
}

/// One run submitted, its create held by the invoker until the test lets the returned `Notify` go, and the loops
/// running, its seat reserved.
pub async fn create_held(stack: &RunStack, invoker: &Arc<FakeInvoker>) -> (Ulid, Arc<Notify>, Running) {
    let runs = submitted(stack, "Peak hour", 1).await;
    let gate = Arc::new(Notify::new());
    *invoker.held.lock().unwrap() = Some((JobName::of(&key(&runs[0])), gate.clone()));
    let running = loops(stack, invoker.clone());
    until("the seat reserved", || async { held(&stack.gate.app.pool).await == 1 }).await;
    (runs[0].clone(), gate, running)
}

/// The manifest's arrival, as the runners' door writes it.
pub async fn manifest_arrived(pool: &PgPool, run: &Ulid) {
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now() WHERE run_id = $1")
        .bind(run.as_str())
        .execute(pool)
        .await
        .expect("running");
}

/// A runner's own report of an ending of `state`, as the results route takes it, naming no recording.
pub fn reported(state: RunState) -> Results {
    Results {
        ending: ending(state),
        metrics: None,
        series: None,
        recording: None,
        runner_commit: None,
        runner_dirty: None,
        source: ResultsSource::Report,
    }
}
