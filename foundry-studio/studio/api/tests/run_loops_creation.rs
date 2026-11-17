//! Spec 08 section 8's statement 1 at a Job's creation, against the database: the run loops of `run-invocation` over
//! the API's store and journal with an invoker that only records what it is asked, its create held where a test needs
//! to race it. A cancel between a reservation and its create, and an attempt ended while its Job still runs, leave no
//! running Job outside the count; a runner that hands in its manifest, or reports, or is cancelled while its create is
//! still returning is the Job at work, left to exit by itself or to its runner's control; a reconciler pass between a
//! reservation's commit and its create leaves the creation to its creator; and a reservation whose create the API
//! never made is created by the reconciler under a fresh token, unless a cancel commits first.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use run_invocation::{Creator, JobName, JobStatus, RunStore, Seats};
use studio_api::runner_routes;
use studio_api::runs::PgRunStore;
use studio_protocol::RunState;
use tokio::sync::Notify;
use tokio::time::sleep;

use support::loops::{
    create_held, ending, held, key, loops, loops_with, manifest_arrived, reported, submitted, token_hash,
};
use support::runs::{FakeInvoker, PausingStore, config, send, stack, until};

/// Statement 1: a cancel between a reservation and its create leaves no Job running outside the count, the reconciler
/// stopping the Job as soon as its create returns and lets its claim go, and the seat staying held until the Job is
/// known stopped; an attempt ended while its Job still runs keeps its seat until the reconciler has stopped it.
#[tokio::test]
async fn a_cancel_racing_a_create_and_an_ending_before_the_job_stops_leave_no_job_outside_the_count() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 2).await;
    let invoker = Arc::new(FakeInvoker::default());
    let gate = Arc::new(Notify::new());
    *invoker.held.lock().unwrap() = Some((JobName::of(&key(&runs[0])), gate.clone()));
    let running = loops(&stack, invoker.clone());
    until("both reserved", || async { held(&scratch.pool).await == 2 }).await;
    let (status, cancelled) = send(&stack.router, "POST", &format!("/api/runs/{}/cancel", runs[0]), None).await;
    assert_eq!((status, cancelled["outcome"].as_str()), (StatusCode::ACCEPTED, Some("cancelled")), "{cancelled}");
    assert_eq!(held(&scratch.pool).await, 2, "the cancelled run's seat is held while its create is in flight");
    gate.notify_one();
    let first = JobName::of(&key(&runs[0]));
    until("the created Job asked to stop", || async { invoker.stopped().contains(&first) }).await;
    sleep(Duration::from_millis(150)).await;
    assert_eq!(held(&scratch.pool).await, 2, "its seat is held while its Job still runs");
    invoker.set(&first, JobStatus::Ended { reason: Some("deleted".to_owned()) });
    until("its seat freed once the Job is known stopped", || async { held(&scratch.pool).await == 1 }).await;
    // The second run ends while its Job still runs: the reconciler stops the Job, and only then does the seat free.
    let store = PgRunStore::new(stack.gate.app.clone());
    let second = JobName::of(&key(&runs[1]));
    until("the second started", || async { invoker.started().iter().any(|request| request.name == second) }).await;
    invoker.set(&second, JobStatus::Running);
    store.end(&key(&runs[1]), &ending(RunState::Failed)).await.expect("an ending");
    until("the reconciler stops the ended attempt's Job", || async { invoker.stopped().contains(&second) }).await;
    sleep(Duration::from_millis(150)).await;
    assert_eq!(held(&scratch.pool).await, 1, "an ended attempt whose Job still runs keeps its seat");
    invoker.set(&second, JobStatus::Ended { reason: Some("deleted".to_owned()) });
    until("its seat frees once stopped", || async { held(&scratch.pool).await == 0 }).await;
    drop(running);
    scratch.close().await;
}

/// Spec 08 section 3.4, step 3 (Astra's run-chain review, the second finding): a runner that hands in its manifest
/// while its Job's create is still returning has moved its attempt to running, which is the Job at work, so the Job
/// runs on once its create returns.
#[tokio::test]
async fn an_attempt_its_runner_moved_to_running_during_the_create_is_not_stopped() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let invoker = Arc::new(FakeInvoker::default());
    let (run, gate, running) = create_held(&stack, &invoker).await;
    manifest_arrived(&scratch.pool, &run).await;
    gate.notify_one();
    until("the create to return", || async { !invoker.started().is_empty() }).await;
    sleep(Duration::from_millis(300)).await;
    assert!(invoker.stopped().is_empty(), "a Job whose runner already runs is not stopped: {:?}", invoker.stopped());
    drop(running);
    scratch.close().await;
}

/// A runner that hands in its manifest and then reports while its Job's create is still returning has ended its
/// attempt by its own report, and then exits by itself: the Job is not stopped when the create returns, but left to the
/// reconciler once the creation's claim is let go, which leaves it to exit and stops it only past the exit grace.
#[tokio::test]
async fn an_attempt_its_runner_reported_during_the_create_is_left_to_exit_by_itself() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let invoker = Arc::new(FakeInvoker::default());
    let (run, gate, running) = create_held(&stack, &invoker).await;
    manifest_arrived(&scratch.pool, &run).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    store.take_results(&key(&run), reported(RunState::Finished)).await.expect("the runner's report, during the create");
    gate.notify_one();
    until("the creation claim released", || async {
        sqlx::query_scalar::<_, bool>("SELECT job_creator IS NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    })
    .await;
    sleep(Duration::from_millis(300)).await;
    assert!(invoker.stopped().is_empty(), "a reported Job exits by itself: {:?}", invoker.stopped());
    let job = JobName::of(&key(&run));
    until("a reported Job still running past the grace stopped", || async { invoker.stopped().contains(&job) }).await;
    drop(running);
    scratch.close().await;
}

/// A cancellation after the runner connects, but before create returns, belongs to runner control. The reconciler does
/// not delete that Job before its cancellation grace.
#[tokio::test]
async fn a_running_cancel_during_create_waits_for_the_runner_to_confirm() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let invoker = Arc::new(FakeInvoker::default());
    let (run, gate, running) = create_held(&stack, &invoker).await;
    let job = JobName::of(&key(&run));
    manifest_arrived(&scratch.pool, &run).await;
    let (_, cancelled) = send(&stack.router, "POST", &format!("/api/runs/{run}/cancel"), None).await;
    assert_eq!(cancelled["outcome"], "stopping");
    gate.notify_one();
    until("the creation claim released", || async {
        sqlx::query_scalar::<_, bool>("SELECT job_creator IS NULL FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .unwrap()
    })
    .await;
    sleep(Duration::from_millis(300)).await;
    assert!(invoker.stopped().is_empty(), "running cancellation must not delete the Job before its grace");
    assert_eq!(held(&scratch.pool).await, 1);
    // Only the grace's unconfirmed ending permits process cleanup when the runner sends no ending.
    sqlx::query("UPDATE attempts SET cancel_requested_at = now() - interval '61 seconds'")
        .execute(&scratch.pool)
        .await
        .unwrap();
    stack.gate.app.runs.wakes.reconcile();
    until("the unconfirmed attempt's Job stopped", || async { invoker.stopped().contains(&job) }).await;
    let state: String = sqlx::query_scalar("SELECT state FROM attempts").fetch_one(&scratch.pool).await.unwrap();
    assert_eq!(state, "cancelled");
    drop(running);
    scratch.close().await;
}

/// Statement 1's creation ownership (Astra's run-chain review, the first finding): reconciler passes that fall
/// between a reservation's commit and its create, the creation grace long past, neither create the Job nor mint it a
/// new token, since the reservation row carries its creator's claim; the creator's one create then carries the token
/// whose hash is stored.
#[tokio::test]
async fn a_reconciler_pass_between_a_reservation_and_its_create_leaves_the_creation_to_its_creator() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    let store = Arc::new(PausingStore::new(PgRunStore::new(stack.gate.app.clone())));
    let invoker = Arc::new(FakeInvoker::default());
    let running = loops_with(&stack, store.clone(), invoker.clone());
    store.paused.notified().await;
    let reserved = token_hash(&scratch.pool, &runs[0]).await;
    // The Job is absent from the invoker throughout, and the grace (100 ms) passes four times over.
    sleep(Duration::from_millis(400)).await;
    assert!(invoker.started().is_empty(), "the reconciler created nothing while its creator held the reservation");
    assert_eq!(token_hash(&scratch.pool, &runs[0]).await, reserved, "and minted no token over the creator's");
    store.go.notify_one();
    until("the creator's create", || async { !invoker.started().is_empty() }).await;
    sleep(Duration::from_millis(300)).await;
    let started = invoker.started();
    assert_eq!(started.len(), 1, "one create, its creator's");
    assert_eq!(started[0].token.sha256(), reserved, "under the token the reservation minted");
    drop(running);
    scratch.close().await;
}

/// Spec 08 section 3.6's re-creation, only while queued and not cancelled (Astra's run-chain review, the third
/// finding): a cancel that commits while the reconciler asks the invoker about a recorded Job it does not know leaves
/// that Job uncreated and its token unminted, the attempt re-read under its row lock before any re-creation.
#[tokio::test]
async fn a_cancel_while_the_reconciler_asks_about_a_lost_create_leaves_the_job_uncreated() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    // A dispatcher that reserved and died before its create.
    let store = PgRunStore::new(stack.gate.app.clone());
    let reserved = store.reserve(Seats(5), &Creator::mint()).await.expect("a reservation").expect("the run waits");
    let invoker = Arc::new(FakeInvoker::default());
    let gate = Arc::new(Notify::new());
    *invoker.held_status.lock().unwrap() = Some((reserved.job.clone(), gate.clone()));
    let running = loops(&stack, invoker.clone());
    // The reconciler has read the attempt queued, past the grace, and waits on the invoker's answer: absent.
    invoker.asked.notified().await;
    let (status, cancelled) = send(&stack.router, "POST", &format!("/api/runs/{}/cancel", runs[0]), None).await;
    assert_eq!((status, cancelled["outcome"].as_str()), (StatusCode::ACCEPTED, Some("cancelled")), "{cancelled}");
    gate.notify_one();
    until("its seat freed", || async { held(&scratch.pool).await == 0 }).await;
    assert!(invoker.started().is_empty(), "a cancelled run's Job is not created again: {:?}", invoker.started());
    assert_eq!(token_hash(&scratch.pool, &runs[0]).await, reserved.token.sha256(), "and no token is minted for it");
    drop(running);
    scratch.close().await;
}

/// Statement 1: killing the API between a reservation's commit and its create leaves the reconciler creating the Job
/// under a fresh token once the creation grace has passed, the old one refused.
#[tokio::test]
async fn a_reservation_the_api_never_created_is_created_by_the_reconciler_under_a_fresh_token() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runs = submitted(&stack, "Peak hour", 1).await;
    // A dispatcher that reserved and died before its create: the reservation committed, its token only in memory.
    let store = PgRunStore::new(stack.gate.app.clone());
    let reserved = store.reserve(Seats(5), &Creator::mint()).await.expect("a reservation").expect("the run waits");
    let invoker = Arc::new(FakeInvoker::default());
    let running = loops(&stack, invoker.clone());
    until("the reconciler creates the recorded Job", || async { !invoker.started().is_empty() }).await;
    let created = invoker.started()[0].clone();
    assert_eq!(created.name, JobName::of(&key(&runs[0])));
    assert_ne!(created.token.expose(), reserved.token.expose(), "a fresh token");
    let runners = runner_routes(stack.gate.app.clone());
    let pins = |token: String| {
        let request = Request::builder()
            .uri(format!("/runner/v1/attempts/{}/1", runs[0]))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("a request");
        support::call(&runners, request)
    };
    assert_eq!(pins(reserved.token.expose().to_owned()).await.0, StatusCode::UNAUTHORIZED, "the old token is dead");
    assert_eq!(pins(created.token.expose().to_owned()).await.0, StatusCode::OK);
    drop(running);
    scratch.close().await;
}
