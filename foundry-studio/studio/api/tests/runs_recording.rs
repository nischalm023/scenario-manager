//! A runner's recording, against the database (spec 08 section 3.5): taken only whole and only once, and accepted on
//! the attempt's row while the attempt runs, whatever then becomes of its report; the API's own failures answered as
//! its own, a receipt that does not decode among them; and a recording its runner left in its folder read only while
//! its attempt still awaits one. A runner holds no credential for the run store (ruling D174): the API stores the
//! recording it sends.

mod support;

use std::convert::Infallible;
use std::env;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use run_invocation::{Ended, JobName, JobStatus, LoopSettings, Loops, RunStore};
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, recording_key};
use runstore::{RunStorage, RunStore as RunObjects};
use serde_json::Value;
use studio_api::runs::{JournalEvents, PgRunStore, Recording, Results, ResultsSource};
use studio_api::{AppState, runner_routes};
use studio_protocol::{Ending, Endpoint, RunState};
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;

use support::delivery::{accepted, cancelled, downloaded, finished, report, routes_of};
use support::results::{CANNED_RECORDING, OTHER_RECORDING};
use support::runs::{FakeInvoker, config, door, reserved, send, stack, until};

/// A body short of a whole recording, empty or cut off anywhere before its trailer's last byte, is refused, claims
/// nothing and stores nothing; once the attempt accepted the canned recording, neither such a body nor a whole
/// recording of other bytes replaces it.
#[tokio::test]
async fn an_empty_or_cut_off_recording_is_refused_and_never_replaces_the_accepted_one() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(stack.gate.app.clone());
    let sent = routes_of(&key).0;
    let half = &CANNED_RECORDING[..CANNED_RECORDING.len() / 2];
    let short = &CANNED_RECORDING[..CANNED_RECORDING.len() - 1];
    for body in [&[][..], half, short] {
        let (status, refused) = door(&runners, "PUT", &sent, Some(&token), Some(body.to_vec())).await;
        assert_eq!(
            (status, refused["type"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")),
            "{} bytes: {refused}",
            body.len()
        );
        let claimed: Option<String> = sqlx::query_scalar("SELECT recording_sha256 FROM attempts")
            .fetch_one(&scratch.pool)
            .await
            .expect("the attempt's row");
        assert_eq!((accepted(&stack, &key).await, claimed), ((None, None), None), "{} bytes", body.len());
    }
    let (status, stored) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let (status, refused) = door(&runners, "PUT", &sent, Some(&token), Some(half.to_vec())).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    let (status, refused) = door(&runners, "PUT", &sent, Some(&token), Some(OTHER_RECORDING.to_vec())).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("recording-differs")));
    let length = i64::try_from(CANNED_RECORDING.len()).unwrap();
    assert_eq!(accepted(&stack, &key).await, (stored["key"].as_str().map(str::to_owned), Some(length)));
    assert_eq!(downloaded(&stack, &key).await, (StatusCode::OK, CANNED_RECORDING.to_vec()), "the first one stands");
    scratch.close().await;
}

/// Two uploads of one attempt racing: of the same bytes, both are answered the one receipt; of other bytes, one is
/// taken and the other refused `recording-differs`, and the run store holds the bytes the attempt's row describes.
#[tokio::test(flavor = "multi_thread")]
async fn two_uploads_racing_take_one_recording() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runners = runner_routes(stack.gate.app.clone());
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let sent = routes_of(&key).0;
    let (first, second) = tokio::join!(
        door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())),
        door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())),
    );
    assert_eq!((first.0, second.0), (StatusCode::OK, StatusCode::OK), "{first:?} {second:?}");
    assert_eq!(first.1, second.1, "one receipt for the same bytes");
    assert_eq!(downloaded(&stack, &key).await.1, CANNED_RECORDING);
    let (_plan, key, token) = reserved(&stack, "Night shift").await;
    let sent = routes_of(&key).0;
    let (canned, other) = tokio::join!(
        door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())),
        door(&runners, "PUT", &sent, Some(&token), Some(OTHER_RECORDING.to_vec())),
    );
    let winner = match (canned.0, other.0) {
        (StatusCode::OK, StatusCode::CONFLICT) => (CANNED_RECORDING, other.1),
        (StatusCode::CONFLICT, StatusCode::OK) => (OTHER_RECORDING, canned.1),
        answers => panic!("one upload is taken and the other refused: {answers:?} {canned:?} {other:?}"),
    };
    assert_eq!(winner.1["type"], "recording-differs");
    let stored = stack.gate.app.runstore.get(&accepted(&stack, &key).await.0.expect("a recording")).await.unwrap();
    assert_eq!(stored.as_ref(), winner.0, "the store holds the bytes the row accepted");
    assert_eq!(accepted(&stack, &key).await.1, Some(i64::try_from(winner.0.len()).unwrap()));
    scratch.close().await;
}

/// The attempt's state is checked where a recording is accepted, not only before its body is read: an upload whose
/// attempt is ended while its body arrives is refused `attempt-ended` and leaves nothing named; a recording accepted
/// before its attempt's unconfirmed cancel stays on the row and downloads.
#[tokio::test]
async fn a_recording_whose_attempt_ends_as_it_arrives_is_refused_and_one_accepted_before_stays() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let runners = runner_routes(stack.gate.app.clone());
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    // One piece waits at a time, so the second is sent only once the route has read the first, its early check passed.
    let (sender, receiver) = channel::<Result<Bytes, Infallible>>(1);
    let request = Request::builder()
        .method("PUT")
        .uri(routes_of(&key).0)
        .header("authorization", format!("Bearer {}", token.expose()))
        .body(Body::from_stream(ReceiverStream::new(receiver)))
        .unwrap();
    let upload = tokio::spawn({
        let runners = runners.clone();
        async move { support::call(&runners, request).await }
    });
    let (head, rest) = CANNED_RECORDING.split_at(CANNED_RECORDING.len() / 3);
    let (middle, tail) = rest.split_at(rest.len() / 2);
    sender.send(Ok(Bytes::from_static(head))).await.unwrap();
    sender.send(Ok(Bytes::from_static(middle))).await.unwrap();
    store.end(&key, &cancelled()).await.expect("the reconciler's unconfirmed cancel");
    sender.send(Ok(Bytes::from_static(tail))).await.unwrap();
    drop(sender);
    let (status, _, answer) = upload.await.unwrap();
    let refused: Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("attempt-ended")));
    assert_eq!(accepted(&stack, &key).await, (None, None), "an ended attempt accepts nothing");
    let object = recording_key(&RunId::new(key.run.as_str()).unwrap(), StoredAttempt::new(1).unwrap());
    assert!(!stack.gate.app.runstore.exists(&object).await.unwrap(), "and the store keeps nothing of it");
    let (_plan, key, token) = reserved(&stack, "Night shift").await;
    let (status, stored) =
        door(&runners, "PUT", &routes_of(&key).0, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    store.end(&key, &cancelled()).await.expect("the reconciler's unconfirmed cancel");
    let (_, read) = send(&stack.router, "GET", &format!("/api/runs/{}", key.run), None).await;
    assert_eq!((read["state"].as_str(), read["recording"]["stored"].as_bool()), (Some("cancelled"), Some(true)));
    assert_eq!(downloaded(&stack, &key).await, (StatusCode::OK, CANNED_RECORDING.to_vec()));
    scratch.close().await;
}

/// A recording accepted while its attempt runs stays on the row when the runner's report never arrives and another
/// API process's reconciler ends the attempt as lost: the download still answers the bytes the runner sent.
#[tokio::test]
async fn an_accepted_recording_outlives_a_lost_report_and_an_api_restart() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now()").execute(&scratch.pool).await.unwrap();
    let runners = runner_routes(stack.gate.app.clone());
    let (status, stored) =
        door(&runners, "PUT", &routes_of(&key).0, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    // The report never comes, and the API process is replaced: the next one's loops, over the same tables and store,
    // find the runner's Job gone.
    let next = AppState::new(
        scratch.pool.clone(),
        support::gate_rules(),
        stack.gate.app.deployment,
        support::MODEL_LABEL,
        stack.gate.app.compute.clone(),
        stack.gate.app.runstore.clone(),
    )
    .with_runs(stack.gate.app.runs.clone());
    let invoker = Arc::new(FakeInvoker::default());
    invoker.set(&JobName::of(&key), JobStatus::Ended { reason: Some("Error".to_owned()) });
    let mut settings = LoopSettings::new(Endpoint { endpoint: "http://127.0.0.1:3101".to_owned() });
    settings.reconcile_every = Duration::from_millis(50);
    settings.lost_grace = Duration::from_millis(200);
    // The stopped process's claim on the Job's creation, which the reservation took, is long past.
    settings.create_grace = Duration::from_millis(100);
    let running = Loops::new(
        Arc::new(PgRunStore::new(next.clone())),
        Arc::new(JournalEvents::new(next.clone())),
        invoker,
        settings,
        next.runs.wakes.clone(),
    )
    .spawn();
    until("the attempt ended as lost", || async {
        PgRunStore::new(next.clone()).attempt(&key).await.unwrap().is_some_and(|row| row.state == RunState::Partial)
    })
    .await;
    drop(running);
    let (_, read) = send(&stack.router, "GET", &format!("/api/runs/{}", key.run), None).await;
    assert_eq!(read["recording"]["stored"], true, "{read}");
    assert_eq!(read["recording"]["live_edge_sim_ms"], stored["live_edge_sim_ms"]);
    assert_eq!(downloaded(&stack, &key).await, (StatusCode::OK, CANNED_RECORDING.to_vec()));
    scratch.close().await;
}

/// The API's own failures are no fault of the recording's, and each is answered `internal`, which a runner asks again:
/// this process's disk refusing to read back the upload it has just written, and the run store failing once the bytes
/// are claimed. The same bytes, asked again once both answer, are taken.
#[tokio::test]
async fn the_apis_own_failures_are_internal_and_the_retry_is_taken() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let objects = support::runstore_folder();
    let app = AppState {
        runstore: RunObjects::local_folder(&objects).expect("a run store of the test's own"),
        ..stack.gate.app.clone()
    };
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(app);
    let sent = routes_of(&key).0;
    let (sender, receiver) = channel::<Result<Bytes, Infallible>>(1);
    let request = Request::builder()
        .method("PUT")
        .uri(&sent)
        .header("authorization", format!("Bearer {}", token.expose()))
        .body(Body::from_stream(ReceiverStream::new(receiver)))
        .unwrap();
    let upload = tokio::spawn({
        let runners = runners.clone();
        async move { support::call(&runners, request).await }
    });
    let (head, tail) = CANNED_RECORDING.split_at(CANNED_RECORDING.len() / 2);
    sender.send(Ok(Bytes::from_static(head))).await.unwrap();
    // The upload's own file, once the route has made it, keeps what is written into it but refuses to be read back.
    let named = format!("studio-recording-{}-{}-", key.run, key.n.0);
    let made = || {
        let listed = fs::read_dir(env::temp_dir()).expect("the temporary folder lists");
        listed
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(&named)))
    };
    until("the upload's file made", || async { made().is_some() }).await;
    let written = made().expect("the upload's file");
    fs::set_permissions(&written, Permissions::from_mode(0o000)).expect("the upload's file made unreadable");
    sender.send(Ok(Bytes::from_static(tail))).await.unwrap();
    drop(sender);
    let (status, _, answer) = upload.await.unwrap();
    let failed: Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!((status, failed["type"].as_str()), (StatusCode::INTERNAL_SERVER_ERROR, Some("internal")), "{failed}");
    assert!(!written.exists(), "the upload's file is removed however the route answers");
    assert_eq!(accepted(&stack, &key).await, (None, None));
    fs::set_permissions(&objects, Permissions::from_mode(0o555)).expect("the store's folder made read-only");
    let (status, failed) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    fs::set_permissions(&objects, Permissions::from_mode(0o755)).expect("the store's folder writable again");
    assert_eq!((status, failed["type"].as_str()), (StatusCode::INTERNAL_SERVER_ERROR, Some("internal")));
    assert_eq!(accepted(&stack, &key).await, (None, None));
    let (status, stored) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(accepted(&stack, &key).await.0.as_deref(), stored["key"].as_str());
    scratch.close().await;
}

/// A recording its runner left in its folder is read whole and hashed only while its attempt still awaits one: a
/// settle after the runner exits, which follows every run's own upload and report, reads nothing of the folder's file
/// (unreadable here) once the runner's upload was accepted, and writes the folder's ending only where the attempt has
/// none.
#[tokio::test]
async fn a_folder_recording_is_read_only_while_its_attempt_awaits_one() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let runners = runner_routes(stack.gate.app.clone());
    let left = env::temp_dir().join(format!("studio-folder-recording-{}.worldview", support::ulid()));
    fs::write(&left, CANNED_RECORDING).expect("the folder's recording");
    fs::set_permissions(&left, Permissions::from_mode(0o000)).expect("the folder's recording made unreadable");
    let settled = |ending: Ending| Results {
        ending,
        metrics: None,
        series: None,
        recording: Some(Recording::File { path: left.clone() }),
        runner_commit: None,
        runner_dirty: None,
        source: ResultsSource::Folder,
    };
    // Its runner's upload accepted, the runner gone before its report.
    let (_plan, uploaded, token) = reserved(&stack, "Peak hour").await;
    let (status, sent) =
        door(&runners, "PUT", &routes_of(&uploaded).0, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let ended = store.take_results(&uploaded, settled(finished())).await.expect("the folder's ending, its file unread");
    assert_eq!(ended, Ended::Wrote);
    // Its runner's upload accepted and its report taken.
    let (_plan, reported, token) = reserved(&stack, "Night shift").await;
    let (sent, results) = routes_of(&reported);
    let (status, stored) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let (status, taken) = door(&runners, "PUT", &results, Some(&token), Some(report(&stored, Some("c0ffee")))).await;
    assert_eq!(status, StatusCode::OK, "{taken}");
    let ended = store.take_results(&reported, settled(finished())).await.expect("no ending, its file unread");
    assert_eq!(ended, Ended::AlreadyEnded);
    fs::set_permissions(&left, Permissions::from_mode(0o644)).expect("readable again, to go");
    fs::remove_file(&left).expect("the folder's recording removed");
    scratch.close().await;
}

/// A receipt is never made up: an accepted recording whose row has lost one of its counts does not decode, so a report
/// naming it is answered `internal` rather than held against a recording of no frames.
#[tokio::test]
async fn a_receipt_missing_its_counts_is_internal() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let runners = runner_routes(stack.gate.app.clone());
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let (sent, results) = routes_of(&key);
    let (status, stored) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    sqlx::query("UPDATE attempts SET recording_frames = NULL").execute(&scratch.pool).await.expect("a count lost");
    let (status, failed) = door(&runners, "PUT", &results, Some(&token), Some(report(&stored, None))).await;
    assert_eq!((status, failed["type"].as_str()), (StatusCode::INTERNAL_SERVER_ERROR, Some("internal")), "{failed}");
    scratch.close().await;
}
