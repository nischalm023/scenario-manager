//! Within `runs`, `watch` is a person's ticket to watch a run live (ruling D174): its latest attempt, the one streaming
//! server that serves that attempt, where a viewer reaches that server for the run's messages, for its picture (live
//! frames: the same port, `/frames` beside `/streams`) and for its metrics (`/metrics` beside them), and a token the
//! server admits the viewer of that attempt by on any of them, kept only as its SHA-256 and alive for a minute of wall
//! clock (migration 0022's `stream_watch_tickets`), so the server's viewer port opens to no one the API did not vouch
//! for. Each mint deletes the expired tickets, so the table holds only live ones. Every ticket for an attempt names the
//! same server while that server lives, so the picture and every chart read the attempt through one copy: the server
//! that holds its stream (the one that last wrote its figures, which is a restarted server's new process once the
//! runner reconnected to it, else the one the dispatcher pointed the attempt at), else, once the attempt has ended and
//! the store holds what a server fetches, the server recorded as its replay server (migration 0024).

use std::time::Duration;

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use run_invocation::Token;
use sqlx::Row;
use studio_protocol::{AttemptNumber, Problem, ProblemType, WatchTicket};

use crate::AppState;
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal};

use super::read::{no_run, run_in};

/// How long a ticket opens the stream for (wall clock, the database's): long enough to connect, not to keep.
const LIFETIME: Duration = Duration::from_secs(60);

/// `POST /api/runs/{run}/watch`: a ticket for the run's latest attempt, naming the server that holds its stream; for
/// an attempt that has ended and whose picture the store holds as a server fetches it (its final recording, or its
/// uploaded scenario block and live segments), its replay server when that one is gone, the first such ticket picking
/// one with a fresh heartbeat and recording it; else `not-streamed`: it was never pointed at a streaming server, its
/// server's heartbeat is stale while it runs, or nothing a server could fetch it from is stored. The attempt's row is
/// locked while the ticket is minted, so tickets minted at once name one replay server.
pub async fn ticket(
    State(app): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<WatchTicket>, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let fault = |fault| internal("mint a watch ticket", fault, instance);
    let mut tx = app.pool.begin().await.map_err(fault)?;
    sqlx::query("DELETE FROM stream_watch_tickets WHERE expires_at <= now()").execute(&mut *tx).await.map_err(fault)?;
    // The attempt's row is locked by a statement of its own, so a ticket minted meanwhile waits for this one to commit
    // and then, in its next statement, reads the replay server this one recorded.
    let n: i32 = sqlx::query_scalar("SELECT n FROM attempts WHERE run_id = $1 ORDER BY n DESC LIMIT 1 FOR UPDATE")
        .bind(run.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(fault)?
        .ok_or_else(|| no_run(instance))?;
    let stale_after = app.runs.config.streaming_stale_after.as_secs_f64();
    // `stored` is the streaming server's own rule for what it can fetch (`studio-streaming`'s `cache::Known::source`).
    let latest = sqlx::query(
        "SELECT own.viewer_url AS own_url, replay.viewer_url AS replay_url, \
                (a.ended_at IS NOT NULL AND (a.object_key IS NOT NULL OR f.scenario_key IS NOT NULL)) AS stored \
         FROM attempts a \
         LEFT JOIN attempt_streams f ON f.run_id = a.run_id AND f.n = a.n \
         LEFT JOIN streaming_servers own ON own.id = COALESCE(f.server_id, a.streaming_server) \
             AND own.heartbeat_at > now() - make_interval(secs => $3) \
         LEFT JOIN streaming_servers replay ON replay.id = a.replay_server \
             AND replay.heartbeat_at > now() - make_interval(secs => $3) \
         WHERE a.run_id = $1 AND a.n = $2",
    )
    .bind(run.as_str())
    .bind(n)
    .bind(stale_after)
    .fetch_one(&mut *tx)
    .await
    .map_err(fault)?;
    let attempt = AttemptNumber(u32::try_from(n).map_err(|bad| internal("read an attempt's number", bad, instance))?);
    let url = |column: &str| latest.try_get::<Option<String>, _>(column).map_err(fault);
    let viewer_url = match (url("own_url")?, latest.try_get::<bool, _>("stored").map_err(fault)?) {
        (Some(own), _) => Some(own),
        (None, true) => match url("replay_url")? {
            Some(replay) => Some(replay),
            None => sqlx::query_scalar(
                "WITH picked AS (SELECT id, viewer_url FROM streaming_servers \
                                 WHERE heartbeat_at > now() - make_interval(secs => $3) ORDER BY random() LIMIT 1) \
                 UPDATE attempts SET replay_server = picked.id FROM picked WHERE run_id = $1 AND n = $2 \
                 RETURNING picked.viewer_url",
            )
            .bind(run.as_str())
            .bind(n)
            .bind(stale_after)
            .fetch_optional(&mut *tx)
            .await
            .map_err(fault)?,
        },
        (None, false) => None,
    };
    let Some(viewer_url) = viewer_url else {
        let detail = format!(
            "Attempt {n} of this run has no live stream: no streaming server was alive when it was dispatched, or \
             the one it streams to is no longer, and no server is up to serve it from the store."
        );
        return Err(Problem::new(ProblemType::NotStreamed, detail).at(instance).into());
    };
    let token = Token::mint();
    let expires_at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO stream_watch_tickets (token_sha256, run_id, n, subject, expires_at) \
         VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5)) RETURNING expires_at",
    )
    .bind(token.sha256())
    .bind(run.as_str())
    .bind(n)
    .bind(identity.subject.to_string())
    .bind(LIFETIME.as_secs_f64())
    .fetch_one(&mut *tx)
    .await
    .map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    let streams = viewer_url.trim_end_matches('/');
    // The server registers where its `/streams` are reached; its `/frames` and `/metrics` are beside them on the
    // same port.
    let beside = |route: &str| {
        streams.strip_suffix("/streams").map_or_else(|| format!("{streams}/{route}"), |base| format!("{base}/{route}"))
    };
    let (frames, metrics) = (beside("frames"), beside("metrics"));
    Ok(Json(WatchTicket {
        url: format!("{streams}/{run}/{}", attempt.0),
        frames_url: format!("{frames}/{run}/{}", attempt.0),
        metrics_url: format!("{metrics}/{run}/{}", attempt.0),
        run,
        attempt,
        token: token.expose().to_owned(),
        expires_at: expires_at.into(),
    }))
}
