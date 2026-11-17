//! Spec 08 section 8's statement 7 on the local invoker and simrunner, how a run in a container stops: a run waiting
//! for a seat is cancelled without the invoker, a running one's cancel reaches it over attempt control and an ended
//! one's is left as it is; a run whose runner has not connected is cancelled at once, its seat held until its container
//! stopped; an unconfirmed stop ends `cancelled` once the grace has passed and not before; and a container past its
//! deadline, new or adopted, is stopped even when the engine refuses the first stop. A platform SIGTERM's tests are
//! `runs_container_terminations`. What the tests need is in `support::containers`; any of it missing, they say they
//! are skipped.

mod support;

use std::env;
use std::slice::from_ref;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use run_invocation::{Creator, Invoker, JobStatus, RunStore, Seats};
use runstore::RunStore as RunObjects;
use studio_api::AppState;
use studio_api::config::{Deployment, RunConfig};
use studio_api::db::mint_ulid;
use studio_api::runs::{ContainerInvoker, PgRunStore};
use studio_protocol::HOLD_VAR;
use tokio::time::sleep;
use wps_client::BlockingCompute;

use support::containers::{
    RemovedAtEnd, cancel, container_exists, container_runs, engine, first_job, left_container, local, loops, needed,
    refusing_engine, request_alone, run_of, script, setup, state_is, submitted,
};
use support::runs::{experiment, plan, scenario, stack, submit, until};

const UNCONFIRMED: &str = "cancelled; the runner did not confirm";

/// Statement 7, three of the four cases on a real runner, held after its manifest: a run waiting for a seat is
/// cancelled without the invoker; the attempt control interrupts the running one's hold and retains a recording of
/// what ran in its folder; an ended one answers `already-ended`.
#[tokio::test]
async fn a_waiting_run_is_cancelled_a_running_one_stops_and_an_ended_one_is_left_as_it_is() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let runner = &needed.image;
    let local = local(&scratch.pool, &needed, runner, &[(HOLD_VAR, "30000")], 1).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 2).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    until("the first to run and hold", || state_is(stack, &runs[0], "running")).await;
    let waiting = cancel(stack, &runs[1]).await;
    assert_eq!((waiting["outcome"].as_str(), waiting["state"].as_str()), (Some("cancelled"), Some("cancelled")));
    let job: Option<String> = sqlx::query_scalar("SELECT job FROM attempts WHERE run_id = $1")
        .bind(runs[1].as_str())
        .fetch_one(&scratch.pool)
        .await
        .unwrap();
    assert_eq!(job, None, "a run that never had a seat never reached the invoker");
    let stopping = cancel(stack, &runs[0]).await;
    assert_eq!((stopping["outcome"].as_str(), stopping["state"].as_str()), (Some("stopping"), Some("running")));
    until("the running one to end", || state_is(stack, &runs[0], "cancelled")).await;
    let read = run_of(stack, &runs[0]).await;
    assert_eq!(read["ended_by"], "person", "{read}");
    assert_ne!(read["stop_reason"], UNCONFIRMED, "the runner confirms the attempt-row cancellation");
    assert!(read["attempts"][0]["runner_commit"].is_string(), "its own report, which alone names its build: {read}");
    assert_eq!(read["recording"]["stored"], true, "the recording of what ran is stored: {read}");
    assert_eq!(read["attempts"][0]["stub"], false, "{read}");
    let ended = cancel(stack, &runs[0]).await;
    assert_eq!((ended["outcome"].as_str(), ended["state"].as_str()), (Some("already-ended"), Some("cancelled")));
    drop(running);
    scratch.close().await;
}

/// Statement 7's second case: a run holding a seat whose runner has not connected is cancelled at once and its Job
/// stopped; its container, a shell that runs out the 5 s stop grace, keeps the seat held while it runs, and the seat
/// frees once the container is known stopped.
#[tokio::test]
async fn a_run_holding_a_seat_before_its_runner_connects_is_cancelled_and_its_job_stopped() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &script(&needed.engine, "sleep 30"), &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let job = first_job(&runs[0]);
    until("its container running", || async { container_runs(&needed.engine, job.as_str()) }).await;
    let answer = cancel(stack, &runs[0]).await;
    assert_eq!((answer["outcome"].as_str(), answer["state"].as_str()), (Some("cancelled"), Some("cancelled")));
    let freed = || async {
        sqlx::query_scalar::<_, bool>("SELECT job_stopped_at IS NOT NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    };
    sleep(Duration::from_secs(2)).await;
    assert!(
        container_runs(&needed.engine, job.as_str()) && !freed().await,
        "its seat is held while its container runs"
    );
    until("the Job stopped and the seat freed", freed).await;
    until("its container gone", || async { !container_exists(&needed.engine, job.as_str()) }).await;
    drop(running);
    scratch.close().await;
}

/// Statement 7's unconfirmed stop: a running attempt whose runner, in a live container, ignores the stop ends
/// `cancelled` once the 5 s grace has passed and not before, and without waiting for its container's kill; its
/// container is then stopped and its seat freed.
#[tokio::test]
async fn an_unconfirmed_stop_ends_cancelled_after_the_grace_and_not_before() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &script(&needed.engine, "trap '' TERM; sleep 30"), &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let job = first_job(&runs[0]);
    until("its container running", || async { container_runs(&needed.engine, job.as_str()) }).await;
    // The runner says nothing; the test stands in for its manifest's arrival.
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now()").execute(&scratch.pool).await.unwrap();
    let answer = cancel(stack, &runs[0]).await;
    assert_eq!(answer["outcome"], "stopping");
    until("the unconfirmed stop to end it", || state_is(stack, &runs[0], "cancelled")).await;
    let read = run_of(stack, &runs[0]).await;
    assert_eq!(read["stop_reason"], UNCONFIRMED);
    let waited: f64 =
        sqlx::query_scalar("SELECT extract(epoch FROM ended_at - cancel_requested_at)::float8 FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap();
    let grace = stack.gate.app.runs.config.cancel_grace.as_secs_f64();
    assert!(waited >= grace, "ended after {waited} s, before its {grace} s grace");
    assert!(waited < grace + 5.0, "ended after {waited} s, waiting for the container's kill");
    until("the Job stopped and the seat freed", || async {
        sqlx::query_scalar::<_, bool>("SELECT job_stopped_at IS NOT NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    })
    .await;
    assert!(!container_runs(&needed.engine, job.as_str()), "no container of it runs once its seat is free");
    drop(running);
    scratch.close().await;
}

/// A container past its deadline is stopped, and a stop the engine refused is asked again until the engine says the
/// container stopped, its status reading running, which holds its seat, until then. The engine here refuses the first
/// stop as an engine whose daemon is away does; the container would run ten minutes and its deadline is 2 s. An
/// attempt no row holds settles as already ended, so no run is needed.
#[tokio::test]
async fn a_deadline_stop_the_engine_refused_is_asked_again_until_the_container_stops() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let folder = env::temp_dir().join(format!("studio-deadline-{}", mint_ulid()));
    let stand_in = refusing_engine(&engine, &folder, &["stop"]);
    let objects = support::runstore_folder();
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let app = AppState::new(
        scratch.pool.clone(),
        support::gate_rules(),
        Deployment::Local,
        support::MODEL_LABEL,
        Arc::new(BlockingCompute),
        RunObjects::local_folder(&objects).expect("the run store"),
    );
    let invoker = ContainerInvoker::new(app, Arc::new(setup(&stand_in, objects, out, &[])));
    let mut request = request_alone(&script(&engine, "sleep 600"));
    request.deadline = Duration::from_secs(2);
    let name = request.name.clone();
    invoker.start(request).await.expect("a start");
    until("the first stop at the deadline refused", || async { folder.join("stop").exists() }).await;
    assert_eq!(invoker.status(&name).await.expect("a status"), JobStatus::Running, "its seat is held meanwhile");
    let stopped = Instant::now();
    until("the container stopped and removed", || async { !container_exists(&engine, name.as_str()) }).await;
    assert!(stopped.elapsed() < Duration::from_secs(60), "stopped by the stop asked again, not by its own end");
    scratch.close().await;
}

/// The same for a container an API restart adopted: its deadline, which its labels keep, passes under the next process,
/// whose first stop the engine refuses; the container is stopped all the same and its attempt failed for its deadline.
#[tokio::test]
async fn an_adopted_containers_deadline_stop_the_engine_refused_is_asked_again() {
    let Some(engine) = engine() else { return };
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, RunConfig::local("http://127.0.0.1:9", "studio-simrunner:dev")).await;
    let made = experiment(&stack, &scenario(&stack, "Peak hour").await, 1, &[]).await;
    let (plan, hash) = plan(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let held = store.reserve(Seats(5), &Creator::mint()).await.expect("a reservation").expect("a waiting attempt");
    left_container(&engine, held.job.as_str(), Some((Utc::now().timestamp() + 2, 3_600)), "sleep 600");
    let folder = env::temp_dir().join(format!("studio-deadline-{}", mint_ulid()));
    let stand_in = refusing_engine(&engine, &folder, &["stop"]);
    let out = env::temp_dir().join(format!("studio-runner-out-{}", mint_ulid()));
    let invoker =
        ContainerInvoker::new(stack.gate.app.clone(), Arc::new(setup(&stand_in, support::runstore_folder(), out, &[])));
    assert_eq!(invoker.adopt().await.expect("the container adopted"), from_ref(&held.job));
    until("the first stop at the deadline refused", || async { folder.join("stop").exists() }).await;
    until("the passed deadline to end the attempt", || async {
        store.attempt(&held.key).await.expect("the attempt").is_some_and(|row| row.state.ended())
    })
    .await;
    let reason: String = sqlx::query_scalar("SELECT stop_reason FROM attempts WHERE run_id = $1")
        .bind(held.key.run.as_str())
        .fetch_one(&scratch.pool)
        .await
        .expect("the ending");
    assert_eq!(reason, "ran past its wall-clock deadline of 1 h");
    until("the container removed", || async { !container_exists(&engine, held.job.as_str()) }).await;
    scratch.close().await;
}
