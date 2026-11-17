//! Spec 08 section 8's statement 7 on the local invoker and simrunner, a platform SIGTERM at each stage of a runner's
//! attempt, which is never a person's cancel: while the manifest is made and during its delivery, each of which it
//! abandons; after a person's cancel, during the preparation's computing, which it abandons too; and after the
//! preparation, where it stops the simulation, which keeps its recording. Each time the runner reports in its own
//! words, which only its own report does, and exits by itself well inside a Job's grace. What the tests need is in
//! `support::containers`; any of it missing, they say they are skipped.

mod support;

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use std::time::{Duration, Instant};

use studio_protocol::HOLD_VAR;
use tokio::time::{sleep, timeout};

use support::containers::{
    DoorGate, DoorRequest, RemovedAtEnd, cancel, container_runs, first_job, local, local_holding, loops, needed,
    out_of, run_of, state_is, submitted,
};
use support::runs::until;

/// Well inside a Job's 90 s termination grace, and long before a request's own five-minute try would give up.
const PROMPTLY: Duration = Duration::from_secs(30);

/// The platform's SIGTERM, sent to the container of `name` alone, never followed by a SIGKILL.
fn terminate(engine: &Path, name: &str) {
    let sent = StdCommand::new(engine).args(["kill", "-s", "TERM", name]).status().expect("the engine runs");
    assert!(sent.success(), "the engine sent the container its SIGTERM");
}

/// Whether the runner in the container of `name` has said `words` in its log.
fn said(engine: &Path, name: &str, words: &str) -> bool {
    let answer = StdCommand::new(engine).args(["logs", name]).output();
    answer.is_ok_and(|answer| String::from_utf8_lossy(&answer.stderr).contains(words))
}

/// SIGTERM while the runner makes its manifest, here asking for its pins, which the API holds unanswered, abandons the
/// making at once.
#[tokio::test]
async fn sigterm_while_the_manifest_is_made_abandons_it() {
    sigterm_while_held_abandons_the_step(DoorRequest::Pins).await;
}

/// SIGTERM while the runner's manifest is on its way, which the API holds unanswered, abandons the delivery at once.
#[tokio::test]
async fn sigterm_during_the_manifests_delivery_abandons_it() {
    sigterm_while_held_abandons_the_step(DoorRequest::Manifest).await;
}

/// SIGTERM while the runner waits on the request the API holds unanswered abandons the step it is in: the runner
/// reports `failed` by the platform in its own words, the attempt never having run, and exits.
async fn sigterm_while_held_abandons_the_step(request: DoorRequest) {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let gate = DoorGate::holding(request);
    let local = local_holding(&scratch.pool, &needed, &needed.image, gate.clone()).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let job = first_job(&runs[0]);
    timeout(Duration::from_secs(120), gate.arrived.notified()).await.expect("the runner's request, held");
    let terminated = Instant::now();
    terminate(&needed.engine, job.as_str());
    until("the runner's report", || state_is(stack, &runs[0], "failed")).await;
    until("the runner to exit", || async { !container_runs(&needed.engine, job.as_str()) }).await;
    assert!(terminated.elapsed() < PROMPTLY, "it exited {:?} after its SIGTERM", terminated.elapsed());
    let read = run_of(stack, &runs[0]).await;
    let ending = (read["ended_by"].as_str(), read["stop_reason"].as_str());
    assert_eq!(ending, (Some("platform"), Some("terminated by the platform")), "{read}");
    assert!(read["attempts"][0]["runner_commit"].is_string(), "its own report, which alone names its build: {read}");
    let path: Vec<String> = sqlx::query_scalar("SELECT \"to\" FROM attempt_transitions ORDER BY tx, id")
        .fetch_all(&scratch.pool)
        .await
        .expect("the transitions");
    assert_eq!(path, ["queued", "failed"], "its manifest never reached the API");
    drop(running);
    scratch.close().await;
}

/// A person's cancel lets the preparation go on, and a SIGTERM after it still abandons the preparation where it
/// computes, here the simulator's reading of the run, which waits on a spec that is a pipe no one writes. The runner
/// reports the attempt `cancelled` by the person, its words naming the platform's stop, and exits at once, the
/// abandoned step left waiting on its thread.
#[tokio::test]
async fn sigterm_after_a_cancel_abandons_the_preparation_still_computing() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let gate = DoorGate::holding(DoorRequest::Manifest);
    let local = local_holding(&scratch.pool, &needed, &needed.image, gate.clone()).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let job = first_job(&runs[0]);
    timeout(Duration::from_secs(120), gate.arrived.notified()).await.expect("the manifest on its way, held");
    // The run's folder is laid out before its manifest goes; the spec the preparation reads first becomes a pipe.
    let spec = out_of(stack, &runs[0]).join("inputs/spec.json");
    fs::remove_file(&spec).expect("the runner's spec");
    let piped = StdCommand::new("mkfifo").arg(&spec).status().expect("mkfifo runs");
    assert!(piped.success(), "the spec is a pipe");
    gate.release.notify_one();
    until("the manifest taken, the attempt running", || state_is(stack, &runs[0], "running")).await;
    sleep(Duration::from_secs(1)).await;
    assert!(container_runs(&needed.engine, job.as_str()), "the preparation waits on its spec");
    assert_eq!(cancel(stack, &runs[0]).await["outcome"], "stopping");
    // The SIGTERM waits until the runner says it has heard the cancel, so the cancel is its first stop.
    until("the runner to hear its cancel", || async { said(&needed.engine, job.as_str(), "a person cancelled it") })
        .await;
    let terminated = Instant::now();
    terminate(&needed.engine, job.as_str());
    until("the runner's report", || state_is(stack, &runs[0], "cancelled")).await;
    until("the runner to exit", || async { !container_runs(&needed.engine, job.as_str()) }).await;
    assert!(terminated.elapsed() < PROMPTLY, "it exited {:?} after its SIGTERM", terminated.elapsed());
    let read = run_of(stack, &runs[0]).await;
    let words = "cancelled before the simulation, its preparation abandoned: terminated by the platform";
    assert_eq!((read["ended_by"].as_str(), read["stop_reason"].as_str()), (Some("person"), Some(words)), "{read}");
    assert!(read["attempts"][0]["runner_commit"].is_string(), "its own report, which alone names its build: {read}");
    drop(running);
    scratch.close().await;
}

/// After its preparation, while it holds before its simulation, the runner catches SIGTERM, stops the simulation,
/// keeps the recording of what ran and reports `failed` by the platform in its own words; only a runner lost
/// unwatched is partial.
#[tokio::test]
async fn sigterm_after_the_preparation_stops_the_simulation_which_keeps_its_recording() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &needed.image, &[(HOLD_VAR, "30000")], 1).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let runs = submitted(stack, "Peak hour", 1).await;
    let _removed = RemovedAtEnd::of(&needed.engine, &runs);
    let job = first_job(&runs[0]);
    until("the runner to hold, prepared", || async { said(&needed.engine, job.as_str(), "holding") }).await;
    terminate(&needed.engine, job.as_str());
    until("the platform ending", || state_is(stack, &runs[0], "failed")).await;
    let read = run_of(stack, &runs[0]).await;
    assert_eq!(read["ended_by"], "platform", "{read}");
    assert!(read["stop_reason"].as_str().unwrap().contains("terminated by the platform"), "{read}");
    assert!(read["attempts"][0]["runner_commit"].is_string(), "its own report, which alone names its build: {read}");
    assert_eq!(read["recording"]["stored"], true, "the recording of what ran is stored: {read}");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT cancel_requested_at IS NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    );
    drop(running);
    scratch.close().await;
}
