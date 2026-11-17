//! Ruling D174's viewers, against the database: `POST /api/runs/{r}/watch` gives a person a ticket for the run's latest
//! attempt on the live streaming server the dispatcher pointed it at, naming its events socket and its frames socket
//! (live frames) beside each other, which that server then admits by the ticket's
//! SHA-256 until it expires, and deletes the expired tickets as it mints; once a restarted server's new process took
//! the stream up, the ticket names that one; a run whose attempt was never pointed at a server, or whose server's
//! heartbeat went stale while it runs, is `not-streamed`, and a run that is not there `not-found`; an attempt that
//! has ended and whose picture the store holds as a server fetches it is watched on one live server, which every
//! ticket names while it lives, and which fetches the picture from the store.

mod support;

use axum::http::StatusCode;
use futures::future::join_all;
use run_invocation::Token;
use serde_json::Value;
use sqlx::PgPool;
use studio_api::config::RunConfig;
use studio_protocol::Ulid;

use support::runs::{RunStack, experiment, plan, scenario, send, stack, submit};

const SERVER: &str = "01KZAVPQNES65HV4EZADPCSZZ1";

fn config() -> RunConfig {
    RunConfig::local("http://127.0.0.1:3101", "studio-simrunner:dev")
}

/// One run submitted on the stack's site, its attempt queued.
async fn submitted(stack: &RunStack) -> Ulid {
    let scenario = scenario(stack, "Peak hour").await;
    let made = experiment(stack, &scenario, 1, &[]).await;
    let (plan, hash) = plan(stack, &made.id).await;
    submit(stack, &plan, &hash).await;
    plan.runs[0].run.clone()
}

/// A streaming server registered with its heartbeat `age` old, and the run's attempt pointed at it.
async fn streamed_by(pool: &PgPool, run: &Ulid, age: &str) {
    sqlx::query(
        "INSERT INTO streaming_servers (id, runner_url, viewer_url, heartbeat_at) \
         VALUES ($1, 'ws://runners.example:3201/runners', 'ws://viewers.example:3202/streams', now() - $2::interval)",
    )
    .bind(SERVER)
    .bind(age)
    .execute(pool)
    .await
    .expect("a streaming server registered");
    sqlx::query("UPDATE attempts SET streaming_server = $2 WHERE run_id = $1")
        .bind(run.as_str())
        .bind(SERVER)
        .execute(pool)
        .await
        .expect("the attempt pointed at it");
}

#[tokio::test]
async fn a_run_on_a_live_server_gets_a_ticket_that_server_admits_until_it_expires() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let run = submitted(&stack).await;
    streamed_by(&scratch.pool, &run, "1 second").await;
    let expired = Token::mint();
    sqlx::query(
        "INSERT INTO stream_watch_tickets (token_sha256, run_id, n, subject, expires_at) \
         VALUES ($1, $2, 1, 'u-1', now() - interval '1 second')",
    )
    .bind(expired.sha256())
    .bind(run.as_str())
    .execute(&scratch.pool)
    .await
    .unwrap();
    let (status, ticket) = send(&stack.router, "POST", &format!("/api/runs/{run}/watch"), None).await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    assert_eq!(ticket["url"].as_str(), Some(format!("ws://viewers.example:3202/streams/{run}/1").as_str()));
    assert_eq!(ticket["frames_url"].as_str(), Some(format!("ws://viewers.example:3202/frames/{run}/1").as_str()));
    assert_eq!(ticket["attempt"].as_u64(), Some(1));
    let token = Token::presented(ticket["token"].as_str().expect("a token"));
    let admitted: Option<(String, i32)> =
        sqlx::query_as("SELECT run_id, n FROM live_watch_tickets WHERE token_sha256 = $1")
            .bind(token.sha256())
            .fetch_optional(&scratch.pool)
            .await
            .unwrap();
    assert_eq!(admitted, Some((run.to_string(), 1)), "the streaming server's view admits it, by its SHA-256 alone");
    let (lifetime, left): (f64, i64) = sqlx::query_as(
        "SELECT extract(epoch FROM expires_at - now())::float8, (SELECT count(*) FROM stream_watch_tickets) \
         FROM stream_watch_tickets WHERE token_sha256 = $1",
    )
    .bind(token.sha256())
    .fetch_one(&scratch.pool)
    .await
    .unwrap();
    assert!((50.0..=60.0).contains(&lifetime), "a minute to connect: {lifetime} s");
    assert_eq!(left, 1, "the expired ticket went as this one was minted");
    sqlx::query("UPDATE stream_watch_tickets SET expires_at = now() - interval '1 second'")
        .execute(&scratch.pool)
        .await
        .unwrap();
    let live: i64 =
        sqlx::query_scalar("SELECT count(*) FROM live_watch_tickets").fetch_one(&scratch.pool).await.unwrap();
    assert_eq!(live, 0, "an expired ticket admits no one");
    scratch.close().await;
}

/// The dispatcher pointed the attempt at a process that is gone; the server restarted at the same address, its runner
/// reconnected to the new process, which wrote the stream's figures: the ticket names the new process's viewers.
#[tokio::test]
async fn a_stream_a_restarted_server_took_up_is_watched_on_that_server() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let run = submitted(&stack).await;
    streamed_by(&scratch.pool, &run, "1 hour").await;
    let restarted = "01KZAVPQNES65HV4EZADPCSZZ2";
    sqlx::query(
        "INSERT INTO streaming_servers (id, runner_url, viewer_url, heartbeat_at) \
         VALUES ($1, 'ws://runners.example:3201/runners', 'ws://restarted.example:3202/streams', now())",
    )
    .bind(restarted)
    .execute(&scratch.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO attempt_streams (run_id, n, server_id, revision, opened_at, sim_ms, progress_values) \
         VALUES ($1, 1, $2, 1, now(), 0, '[]')",
    )
    .bind(run.as_str())
    .bind(restarted)
    .execute(&scratch.pool)
    .await
    .unwrap();
    let (status, ticket) = send(&stack.router, "POST", &format!("/api/runs/{run}/watch"), None).await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    assert_eq!(ticket["url"].as_str(), Some(format!("ws://restarted.example:3202/streams/{run}/1").as_str()));
    scratch.close().await;
}

#[tokio::test]
async fn a_run_with_no_live_stream_is_not_streamed_and_a_run_not_there_not_found() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let run = submitted(&stack).await;
    let watch = format!("/api/runs/{run}/watch");
    let (status, refused) = send(&stack.router, "POST", &watch, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("not-streamed")), "never pointed");
    streamed_by(&scratch.pool, &run, "1 hour").await;
    let (status, refused) = send(&stack.router, "POST", &watch, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("not-streamed")), "its server is dead");
    let (status, refused) = send(&stack.router, "POST", "/api/runs/01KZAVPQNES65HV4EZADPCS7E8/watch", None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")));
    scratch.close().await;
}

/// A streaming server registered at `viewers` with its heartbeat fresh.
async fn live_server(pool: &PgPool, id: &str, viewers: &str) {
    sqlx::query(
        "INSERT INTO streaming_servers (id, runner_url, viewer_url, heartbeat_at) \
         VALUES ($1, 'ws://runners.example:3201/runners', $2, now())",
    )
    .bind(id)
    .bind(viewers)
    .execute(pool)
    .await
    .expect("a live streaming server");
}

/// The run's attempt ended by the simulator, its final recording stored under `object_key` when there is one.
async fn ended(pool: &PgPool, run: &Ulid, object_key: Option<&str>) {
    sqlx::query(
        "UPDATE attempts SET state = 'finished', ended_at = now(), ended_by = 'simulator', object_key = $2 \
         WHERE run_id = $1",
    )
    .bind(run.as_str())
    .bind(object_key)
    .execute(pool)
    .await
    .expect("the attempt ended");
}

/// `count` tickets for the run minted at once, as a page's picture and charts ask: each one's three sockets.
async fn named(stack: &RunStack, run: &Ulid, count: usize) -> Vec<[String; 3]> {
    let watch = format!("/api/runs/{run}/watch");
    let minted = join_all((0..count).map(|_| send(&stack.router, "POST", &watch, None))).await;
    let socket = |ticket: &Value, field: &str| ticket[field].as_str().expect("a socket").to_owned();
    minted
        .into_iter()
        .map(|(status, ticket)| {
            assert_eq!(status, StatusCode::OK, "{ticket}");
            ["url", "frames_url", "metrics_url"].map(|field| socket(&ticket, field))
        })
        .collect()
}

/// The attempt ended, its recording is in the store, and the server it streamed to is gone: tickets minted at once
/// pick one live server, and every later one names it while it lives, its three sockets beside each other; once it is
/// gone too, the next tickets pick another, which they keep.
#[tokio::test]
async fn an_ended_attempt_whose_recording_is_stored_is_watched_on_one_live_server_while_it_lives() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let run = submitted(&stack).await;
    streamed_by(&scratch.pool, &run, "1 hour").await;
    ended(&scratch.pool, &run, Some("runs/r/attempts/1/recording.worldview")).await;
    let watch = format!("/api/runs/{run}/watch");
    let (status, refused) = send(&stack.router, "POST", &watch, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("not-streamed")), "no server is up");
    let servers = ["01KZAVPQNES65HV4EZADPCSZZ3", "01KZAVPQNES65HV4EZADPCSZZ4", "01KZAVPQNES65HV4EZADPCSZZ5"];
    for (id, host) in servers.iter().zip(["one", "two", "three"]) {
        live_server(&scratch.pool, id, &format!("ws://{host}.example:3202/streams")).await;
    }
    let first = named(&stack, &run, 12).await;
    let [url, frames, metrics] = &first[0];
    let base = url.strip_suffix(&format!("/streams/{run}/1")).expect("the attempt's events socket");
    assert_eq!((frames, metrics), (&format!("{base}/frames/{run}/1"), &format!("{base}/metrics/{run}/1")));
    assert!(first.iter().all(|one| *one == first[0]), "one server for tickets minted at once: {first:?}");
    assert!(named(&stack, &run, 6).await.iter().all(|later| *later == first[0]), "and for every later ticket");

    let picked: String = sqlx::query_scalar("SELECT replay_server FROM attempts WHERE run_id = $1")
        .bind(run.as_str())
        .fetch_one(&scratch.pool)
        .await
        .expect("the replay server recorded");
    sqlx::query("UPDATE streaming_servers SET heartbeat_at = now() - interval '1 hour' WHERE id = $1")
        .bind(&picked)
        .execute(&scratch.pool)
        .await
        .unwrap();
    let after = named(&stack, &run, 12).await;
    assert!(after[0][0] != *url, "a gone server is not named: {after:?}");
    assert!(after.iter().all(|later| *later == after[0]), "the next pick is kept: {after:?}");
    scratch.close().await;
}

/// Without a final recording, the uploaded scenario block and live segments are what a server fetches: an ended
/// attempt with them gets a ticket, and one whose stream uploaded nothing is `not-streamed`, as is an attempt still
/// running whose server is gone, though another server is up.
#[tokio::test]
async fn only_an_ended_attempt_a_server_can_fetch_is_watched_once_its_server_is_gone() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let run = submitted(&stack).await;
    streamed_by(&scratch.pool, &run, "1 hour").await;
    live_server(&scratch.pool, "01KZAVPQNES65HV4EZADPCSZZ3", "ws://another.example:3202/streams").await;
    sqlx::query(
        "INSERT INTO attempt_streams (run_id, n, server_id, revision, opened_at, sim_ms, progress_values) \
         VALUES ($1, 1, $2, 1, now(), 600000, '[]')",
    )
    .bind(run.as_str())
    .bind(SERVER)
    .execute(&scratch.pool)
    .await
    .expect("the stream's row");
    let watch = format!("/api/runs/{run}/watch");
    let (status, refused) = send(&stack.router, "POST", &watch, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("not-streamed")), "still running");
    ended(&scratch.pool, &run, None).await;
    let (status, refused) = send(&stack.router, "POST", &watch, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("not-streamed")), "nothing uploaded");
    sqlx::query(
        "UPDATE attempt_streams SET scenario_key = 'runs/r/attempts/1/live/scenario.worldview' WHERE run_id = $1",
    )
    .bind(run.as_str())
    .execute(&scratch.pool)
    .await
    .unwrap();
    let (status, ticket) = send(&stack.router, "POST", &watch, None).await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    assert_eq!(ticket["url"].as_str(), Some(format!("ws://another.example:3202/streams/{run}/1").as_str()));
    scratch.close().await;
}
