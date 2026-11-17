//! Spec 01 section 3.13, idempotency, against a route the test mounts behind the identity layer: section 8's
//! statement 5, the section's other rules, and the lease on an in-flight key (lead decided).

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::routing::{MethodRouter, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use studio_api::AppState;
use studio_api::idempotency::idempotent;
use studio_api::identity;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};

/// A write route mounted as a real one would be: behind the identity layer, and made idempotent.
fn with_route(state: AppState, route: MethodRouter<AppState>) -> Router {
    Router::new()
        .route("/api/things", idempotent(&state, route))
        .layer(from_fn_with_state(state.clone(), identity::resolve))
        .with_state(state)
}

/// A route that counts its runs and answers 201 with the run's number and the body it read.
fn counting(state: AppState, runs: Arc<AtomicU32>) -> Router {
    with_route(
        state,
        post(move |body: String| async move {
            let run = runs.fetch_add(1, Ordering::SeqCst) + 1;
            (StatusCode::CREATED, Json(json!({ "run": run, "body": body })))
        }),
    )
}

fn write(subject: &str, key: Option<&str>, body: &str) -> Request<Body> {
    let request = support::from_gate("POST", "/api/things", subject, "").header("content-type", "application/json");
    let request = match key {
        Some(key) => request.header("idempotency-key", key),
        None => request,
    };
    request.body(Body::from(body.to_owned())).expect("a request")
}

fn problem_type(body: &[u8]) -> String {
    let problem: Value = serde_json::from_slice(body).expect("a problem body");
    problem["type"].as_str().expect("a problem names its type").to_owned()
}

/// Spec 01 section 8, statement 5: the same key and body replay the first answer with `Idempotent-Replayed`.
#[tokio::test]
async fn the_same_key_and_body_replay_the_first_answer_and_run_nothing() {
    let scratch = scratch_or_skip!();
    let runs = Arc::new(AtomicU32::new(0));
    let router = counting(support::state(scratch.pool.clone(), support::gate_rules()), runs.clone());

    let (status, headers, first) = support::call(&router, write("u-1", Some("k-1"), r#"{"a":1,"b":[2,3]}"#)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(!headers.contains_key("idempotent-replayed"));

    let same_body_spelled_otherwise = "{ \"b\": [2, 3],\n  \"a\": 1 }";
    let (status, headers, again) = support::call(&router, write("u-1", Some("k-1"), same_body_spelled_otherwise)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(headers["idempotent-replayed"], "true");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(again, first, "the stored answer, byte for byte");
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scratch.close().await;
}

/// Spec 01 section 8, statement 5: another body under the key gives 422, and a missing key 400.
#[tokio::test]
async fn another_body_under_the_key_is_refused_and_a_write_without_a_key_too() {
    let scratch = scratch_or_skip!();
    let runs = Arc::new(AtomicU32::new(0));
    let router = counting(support::state(scratch.pool.clone(), support::gate_rules()), runs.clone());
    support::call(&router, write("u-1", Some("k-1"), r#"{"a":1}"#)).await;

    let (status, _, body) = support::call(&router, write("u-1", Some("k-1"), r#"{"a":2}"#)).await;
    assert_eq!((status, problem_type(&body).as_str()), (StatusCode::UNPROCESSABLE_ENTITY, "idempotency-key-reused"));
    for key in [None, Some("two words"), Some("")] {
        let (status, _, body) = support::call(&router, write("u-1", key, r#"{"a":1}"#)).await;
        assert_eq!((status, problem_type(&body).as_str()), (StatusCode::BAD_REQUEST, "idempotency-key-missing"));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    scratch.close().await;
}

/// Spec 01 section 8, statement 5: a concurrent repeat gives 409, and once the first has answered, its answer.
#[tokio::test]
async fn a_repeat_while_the_first_still_runs_is_refused() {
    let scratch = scratch_or_skip!();
    let (entered, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let route = {
        let (entered, release) = (entered.clone(), release.clone());
        post(move || async move {
            entered.notify_one();
            release.notified().await;
            (StatusCode::CREATED, Json(json!({ "done": true })))
        })
    };
    let router = with_route(support::state(scratch.pool.clone(), support::gate_rules()), route);

    let first = tokio::spawn({
        let router = router.clone();
        async move { support::call(&router, write("u-1", Some("k-1"), "{}")).await }
    });
    timeout(Duration::from_secs(10), entered.notified()).await.expect("the first request reaches the route");
    let (status, _, body) = support::call(&router, write("u-1", Some("k-1"), "{}")).await;
    assert_eq!((status, problem_type(&body).as_str()), (StatusCode::CONFLICT, "idempotency-key-in-flight"));

    release.notify_one();
    let (status, _, _) = first.await.expect("the first request finishes");
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, _) = support::call(&router, write("u-1", Some("k-1"), "{}")).await;
    assert_eq!((status, headers.contains_key("idempotent-replayed")), (StatusCode::CREATED, true));
    scratch.close().await;
}

/// Spec 01 section 3.13: a 5xx answer is not stored, so a retry executes again.
#[tokio::test]
async fn a_server_error_is_not_stored_so_its_retry_runs_again() {
    let scratch = scratch_or_skip!();
    let runs = Arc::new(AtomicU32::new(0));
    let route = {
        let runs = runs.clone();
        post(move || async move {
            match runs.fetch_add(1, Ordering::SeqCst) {
                0 => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "try": "again" }))),
                _ => (StatusCode::CREATED, Json(json!({ "done": true }))),
            }
        })
    };
    let router = with_route(support::state(scratch.pool.clone(), support::gate_rules()), route);

    let statuses = [
        support::call(&router, write("u-1", Some("k-1"), "{}")).await.0,
        support::call(&router, write("u-1", Some("k-1"), "{}")).await.0,
        support::call(&router, write("u-1", Some("k-1"), "{}")).await.0,
    ];
    assert_eq!(statuses, [StatusCode::SERVICE_UNAVAILABLE, StatusCode::CREATED, StatusCode::CREATED]);
    assert_eq!(runs.load(Ordering::SeqCst), 2, "the 503 ran again; the 201 was replayed");
    scratch.close().await;
}

/// Spec 01 section 3.13: keys are stored per subject and key, so another person's identical key runs anew.
#[tokio::test]
async fn a_key_is_its_persons_own() {
    let scratch = scratch_or_skip!();
    let runs = Arc::new(AtomicU32::new(0));
    let router = counting(support::state(scratch.pool.clone(), support::gate_rules()), runs.clone());
    for subject in ["u-1", "u-2"] {
        let (status, headers, _) = support::call(&router, write(subject, Some("k-1"), "{}")).await;
        assert_eq!((status, headers.contains_key("idempotent-replayed")), (StatusCode::CREATED, false));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    scratch.close().await;
}

/// The lease (lead decided, beside spec 01 section 3.13): a claim whose lease ran out is taken over by a repeat with
/// the same body, which runs again; and the request whose claim lapsed, answering afterwards, neither stores nor
/// releases the claim that replaced it, whether it answers 201 or 503.
#[tokio::test]
async fn a_lapsed_claim_is_taken_over_and_its_first_request_settles_nothing() {
    let scratch = scratch_or_skip!();
    for first_answer in [StatusCode::CREATED, StatusCode::SERVICE_UNAVAILABLE] {
        // Each of the two runs waits at its own gate until the test lets it answer, run 1 with `first_answer`.
        let runs = Arc::new(AtomicUsize::new(0));
        let entered: Arc<[Notify; 2]> = Arc::new([Notify::new(), Notify::new()]);
        let gates: Arc<[Notify; 2]> = Arc::new([Notify::new(), Notify::new()]);
        let route = {
            let (runs, entered, gates) = (runs.clone(), entered.clone(), gates.clone());
            post(move || async move {
                let run = runs.fetch_add(1, Ordering::SeqCst);
                entered[run].notify_one();
                gates[run].notified().await;
                (if run == 0 { first_answer } else { StatusCode::CREATED }, Json(json!({ "run": run + 1 })))
            })
        };
        let router = with_route(support::state(scratch.pool.clone(), support::gate_rules()), route);
        let key = format!("k-{}", first_answer.as_u16());
        let send = || {
            let (router, key) = (router.clone(), key.clone());
            tokio::spawn(async move { support::call(&router, write("u-1", Some(&key), "{}")).await })
        };

        let first = send();
        timeout(Duration::from_secs(10), entered[0].notified()).await.expect("the first request reaches the route");
        sqlx::query("UPDATE idempotency_keys SET lease_expires_at = now() - interval '1 second' WHERE key = $1")
            .bind(&key)
            .execute(&scratch.pool)
            .await
            .expect("the first claim's lease runs out, as it would a minute after its process died");
        let second = send();
        timeout(Duration::from_secs(10), entered[1].notified()).await.expect("the repeat takes the key over");

        gates[0].notify_one();
        assert_eq!(first.await.expect("the first request answers").0, first_answer, "its own caller hears it");
        let (status, _, body) = support::call(&router, write("u-1", Some(&key), "{}")).await;
        assert_eq!(status, StatusCode::CONFLICT, "after {first_answer}, the repeat's claim still holds the key");
        assert_eq!(problem_type(&body), "idempotency-key-in-flight");

        gates[1].notify_one();
        assert_eq!(second.await.expect("the repeat answers").0, StatusCode::CREATED);
        let (status, headers, body) = support::call(&router, write("u-1", Some(&key), "{}")).await;
        assert_eq!((status, headers.contains_key("idempotent-replayed")), (StatusCode::CREATED, true));
        assert_eq!(serde_json::from_slice::<Value>(&body).expect("JSON"), json!({ "run": 2 }), "the repeat's answer");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
    scratch.close().await;
}

/// The lease (lead decided, beside spec 01 section 3.13): a request whose route still runs renews its lease, 20 s of
/// wall clock after taking the key, so a slow write is never taken over. This test waits 21 s for that renewal.
#[tokio::test]
async fn a_running_request_renews_its_lease() {
    let scratch = scratch_or_skip!();
    let (entered, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let route = {
        let (entered, release) = (entered.clone(), release.clone());
        post(move || async move {
            entered.notify_one();
            release.notified().await;
            (StatusCode::CREATED, Json(json!({ "done": true })))
        })
    };
    let router = with_route(support::state(scratch.pool.clone(), support::gate_rules()), route);
    let first = tokio::spawn({
        let router = router.clone();
        async move { support::call(&router, write("u-1", Some("k-1"), "{}")).await }
    });
    timeout(Duration::from_secs(10), entered.notified()).await.expect("the request reaches the route");

    sleep(Duration::from_secs(21)).await;
    let left: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM lease_expires_at - now())::float8 FROM idempotency_keys WHERE key = 'k-1'",
    )
    .fetch_one(&scratch.pool)
    .await
    .expect("the claim is still in the table");
    assert!(left > 45.0, "unrenewed, a 60 s lease would hold 39 s after 21 s; it holds {left:.1} s");

    release.notify_one();
    assert_eq!(first.await.expect("the request answers").0, StatusCode::CREATED);
    scratch.close().await;
}
