//! The deployment's run counts, `GET /api/runs/stats` (spec 08 section 3.7), against the database: what the attempts'
//! rows hold, the platform never asked; a running attempt not heard for 30 s of wall clock, which the run's read calls
//! stalled too; and the Job creations that failed, counted by their endings' typed cause, never by their words.

mod support;

use axum::http::StatusCode;
use desunits::SimTime;
use run_invocation::{Creator, RunStore, Seats};
use serde_json::json;
use studio_api::runner_routes;
use studio_api::runs::PgRunStore;
use studio_protocol::{EndedBy, Ending, RunState};

use support::runs::{config, door, experiment, plan, reserved, scenario, send, stack, submit};

fn ending(state: RunState, ended_by: EndedBy, stop_reason: &str) -> Ending {
    Ending { state, ended_by, stop_reason: stop_reason.to_owned(), stop_detail: None, sim_ms: SimTime::ZERO }
}

/// The counts read the rows: of three runs, one holds its seat though it finished (its Job not yet known stopped), one
/// runs and holds the other seat, and one waits, since when the stats say; the endings of the last day are counted by
/// state.
#[tokio::test]
async fn the_stats_count_what_the_rows_hold_no_platform_asked() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let made = experiment(&stack, &scenario(&stack, "Peak hour").await, 1, &[100, 150, 200]).await;
    let (plan, hash) = plan(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let finished = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("the first run waits");
    let running = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("the second run waits");
    store.end(&finished.key, &ending(RunState::Finished, EndedBy::Simulator, "SimTimeLimit")).await.unwrap();
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now() WHERE run_id = $1")
        .bind(running.key.run.as_str())
        .execute(&scratch.pool)
        .await
        .unwrap();
    let (status, stats) = send(&stack.router, "GET", "/api/runs/stats", None).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!((stats["seats"]["limit"].as_u64(), stats["seats"]["held"].as_u64()), (Some(5), Some(2)), "{stats}");
    assert_eq!(
        (stats["waiting"].as_u64(), stats["running"].as_u64(), stats["stalled"].as_u64()),
        (Some(1), Some(1), Some(0)),
        "{stats}"
    );
    assert!(stats["oldest_waiting_since"].is_string(), "{stats}");
    let ended = &stats["ended_last_24h"];
    assert_eq!(*ended, json!({ "finished": 1, "cancelled": 0, "failed": 0, "partial": 0 }), "{stats}");
    scratch.close().await;
}

/// A running attempt reads as stalled once its runner has not been heard for 30 s of wall clock, since its latest
/// heartbeat or, before its first, since the manifest's arrival that made it running, so a runner whose beats never land
/// stalls too; a beat clears it. The run's read and the stats count the same.
#[tokio::test]
async fn a_running_attempt_not_heard_for_30_s_reads_as_stalled_beat_or_none() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_plan, key, token) = reserved(&stack, "Peak hour").await;
    let stalled = || async {
        let (_, read) = send(&stack.router, "GET", &format!("/api/runs/{}", key.run), None).await;
        let (_, stats) = send(&stack.router, "GET", "/api/runs/stats", None).await;
        (read["attempts"][0]["stalled"].as_bool(), stats["stalled"].as_u64())
    };
    // Its manifest made it running a minute ago, and no beat of it ever landed.
    sqlx::query("UPDATE attempts SET state = 'running', started_at = now() - interval '1 minute'")
        .execute(&scratch.pool)
        .await
        .unwrap();
    assert_eq!(stalled().await, (Some(true), Some(1)), "a runner never heard since it started");
    let beat = json!({ "sim_ms": 60_000, "values": [] }).to_string().into_bytes();
    let runners = runner_routes(stack.gate.app.clone());
    let path = format!("/runner/v1/attempts/{}/1/heartbeat", key.run);
    assert_eq!(door(&runners, "PUT", &path, Some(&token), Some(beat)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(stalled().await, (Some(false), Some(0)), "heard just now");
    sqlx::query("UPDATE attempts SET heartbeat_at = now() - interval '31 seconds'")
        .execute(&scratch.pool)
        .await
        .unwrap();
    assert_eq!(stalled().await, (Some(true), Some(1)), "its last beat 31 s ago");
    scratch.close().await;
}

/// A Job creation that failed is counted by its ending's typed cause, whatever its words say: the dispatcher's ending,
/// reworded so that no words begin as the dispatcher's do, is counted alone, and another platform failure whose words
/// begin as the dispatcher's do adds a failure and no creation failure.
#[tokio::test]
async fn the_stats_count_a_job_creation_that_failed_by_its_typed_cause() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let made = experiment(&stack, &scenario(&stack, "Peak hour").await, 1, &[100, 150]).await;
    let (plan, hash) = plan(&stack, &made.id).await;
    submit(&stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let uncreated = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("the first run waits");
    let other = store.reserve(Seats(5), &Creator::mint()).await.unwrap().expect("the second run waits");
    let counted = || async {
        let (status, stats) = send(&stack.router, "GET", "/api/runs/stats", None).await;
        assert_eq!(status, StatusCode::OK, "{stats}");
        (stats["create_failures_last_24h"].as_u64(), stats["ended_last_24h"]["failed"].as_u64())
    };
    let refused = ending(RunState::Failed, EndedBy::Platform, "the cluster's admission webhook refused the Job");
    store.end_uncreated(&uncreated.key, &refused).await.expect("the dispatcher's ending");
    assert_eq!(counted().await, (Some(1), Some(1)), "the creation failure, whose words are not the dispatcher's");
    let lookalike = ending(RunState::Failed, EndedBy::Platform, "its Job could not be created, a runner wrote");
    store.end(&other.key, &lookalike).await.expect("another platform ending");
    assert_eq!(counted().await, (Some(1), Some(2)), "a failure whose words are the dispatcher's, and no creation's");
    scratch.close().await;
}
