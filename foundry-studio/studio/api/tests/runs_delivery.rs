//! What a runner delivers of its own run besides its recording, against the database (spec 08 section 3.5): its
//! heartbeat, kept on the row with its instant; and its report, written as the attempt's ending, naming only the
//! recording the attempt accepted (`runs_recording` holds the recording's own tests). Every delivery opens with the
//! attempt's own token.

mod support;

use axum::http::StatusCode;
use run_invocation::{RunStore, Token};
use runstore::RunStorage;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, recording_key};
use serde_json::json;
use studio_api::runner_routes;
use studio_api::runs::PgRunStore;

use support::delivery::{accepted, cancelled, downloaded, report, routes_of};
use support::results::CANNED_RECORDING;
use support::runs::{config, door, reserved, send, stack};

/// The runner's heartbeat and report: a beat while queued is kept on the row with its instant and read back with the
/// run; a report naming a recording the attempt never accepted is refused; the report naming the one it accepted ends
/// the attempt, marks that its runner reported and keeps who the runner is, a second report answers `already-ended`,
/// and a beat after it `attempt-ended`.
#[tokio::test]
async fn a_runner_heartbeats_and_reports_its_results_which_the_rows_then_answer() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(stack.gate.app.clone());
    let (sent, results) = routes_of(&key);
    let heartbeat = format!("/runner/v1/attempts/{}/1/heartbeat", key.run);
    let beat = json!({ "sim_ms": 60_000, "values": [{ "metric": "picks", "label": "Picks", "value": 3.0 }] });
    let (status, _) = door(&runners, "PUT", &heartbeat, Some(&token), Some(beat.to_string().into_bytes())).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let detail = format!("/api/runs/{}", key.run);
    let (_, read) = send(&stack.router, "GET", &detail, None).await;
    assert!(read["attempts"][0]["heartbeat_at"].is_string(), "{read}");
    assert_eq!(
        (read["progress"]["sim_ms"].as_u64(), read["attempts"][0]["stalled"].as_bool()),
        (Some(60_000), Some(false))
    );
    assert_eq!(read["progress"]["values"][0]["metric"], "picks");
    let own = recording_key(&RunId::new(key.run.as_str()).unwrap(), StoredAttempt::new(1).unwrap());
    let never = json!({ "key": own, "live_edge_sim_ms": 600_000, "frames": 61, "bytes": 2 });
    let (status, refused) = door(&runners, "PUT", &results, Some(&token), Some(report(&never, Some("abc123")))).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    let (status, stored) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let body = report(&stored, Some("abc123"));
    let (status, ended) = door(&runners, "PUT", &results, Some(&token), Some(body.clone())).await;
    assert_eq!((status, ended["ended"].as_str()), (StatusCode::OK, Some("wrote")));
    let (status, ended) = door(&runners, "PUT", &results, Some(&token), Some(body)).await;
    assert_eq!((status, ended["ended"].as_str()), (StatusCode::OK, Some("already-ended")));
    let (_, read) = send(&stack.router, "GET", &detail, None).await;
    assert_eq!(read["state"], "finished", "{read}");
    assert_eq!(read["recording"]["stored"], true, "{read}");
    assert_eq!(read["recording"]["live_edge_sim_ms"], stored["live_edge_sim_ms"]);
    assert_eq!(read["attempts"][0]["runner_commit"], "abc123");
    let reported: bool = sqlx::query_scalar("SELECT reported_at IS NOT NULL FROM attempts")
        .fetch_one(&scratch.pool)
        .await
        .expect("the attempt's row");
    assert!(reported, "the runner's own report ended it");
    let (status, refused) = door(&runners, "PUT", &heartbeat, Some(&token), Some(beat.to_string().into_bytes())).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("attempt-ended")));
    scratch.close().await;
}

/// A runner's recording goes to the API, which takes it as the attempt's: a body that is no recording is refused and
/// nothing of it is kept; the canned recording is answered its receipt, which the report then names and the run reads
/// back and downloads; a report naming other figures is refused; and once the attempt has ended, the same bytes again
/// are answered the same receipt, while an attempt that ended with none takes none.
#[tokio::test]
async fn a_runner_sends_its_recording_which_the_api_takes_and_its_report_names() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(stack.gate.app.clone());
    let (sent, results) = routes_of(&key);
    let object = recording_key(&RunId::new(key.run.as_str()).unwrap(), StoredAttempt::new(1).unwrap());
    let (status, refused) = door(&runners, "PUT", &sent, Some(&token), Some(b"no recording".to_vec())).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    assert!(!stack.gate.app.runstore.exists(&object).await.unwrap(), "nothing of a body that is no recording is kept");
    let (status, stored) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(
        (stored["key"].as_str(), stored["bytes"].as_u64()),
        (Some(object.as_str()), Some(CANNED_RECORDING.len() as u64))
    );
    assert!(stored["frames"].as_u64().is_some_and(|frames| frames > 0), "{stored}");
    let mut misnamed = stored.clone();
    misnamed["frames"] = json!(1);
    let (status, refused) = door(&runners, "PUT", &results, Some(&token), Some(report(&misnamed, None))).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    let (status, ended) = door(&runners, "PUT", &results, Some(&token), Some(report(&stored, None))).await;
    assert_eq!((status, ended["ended"].as_str()), (StatusCode::OK, Some("wrote")));
    let (_, read) = send(&stack.router, "GET", &format!("/api/runs/{}", key.run), None).await;
    assert_eq!(read["recording"]["stored"], true, "{read}");
    assert_eq!(read["recording"]["live_edge_sim_ms"], stored["live_edge_sim_ms"]);
    assert_eq!(downloaded(&stack, &key).await, (StatusCode::OK, CANNED_RECORDING.to_vec()), "as the runner sent it");
    let (status, again) = door(&runners, "PUT", &sent, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!((status, &again), (StatusCode::OK, &stored), "the same bytes after the end, the same receipt");
    let (_plan, other, token) = reserved(&stack, "Night shift").await;
    PgRunStore::new(stack.gate.app.clone()).end(&other, &cancelled()).await.expect("the ending");
    let (status, refused) =
        door(&runners, "PUT", &routes_of(&other).0, Some(&token), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("attempt-ended")));
    assert_eq!(accepted(&stack, &other).await, (None, None));
    scratch.close().await;
}

/// A token is needed for every delivery, as for every door route.
#[tokio::test]
async fn a_delivery_without_the_attempts_token_is_refused() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_plan, key, _token) = reserved(&stack, "Peak hour").await;
    let runners = runner_routes(stack.gate.app.clone());
    let stranger = Token::mint();
    let (status, _) = door(&runners, "PUT", &routes_of(&key).0, Some(&stranger), Some(CANNED_RECORDING.to_vec())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(accepted(&stack, &key).await, (None, None));
    scratch.close().await;
}
