//! Within `studio-streaming`, `registry` is this server's row in `streaming_servers` (migration 0022, the API's), which
//! is how the dispatcher finds a live streaming server for an attempt (ruling D174): written at start with where a
//! runner and a viewer reach the server, written again by every heartbeat with how many streams it holds open, so a row
//! someone deleted comes back at the next beat, and deleted at a clean stop. Each write also deletes the rows whose
//! heartbeat stopped `EXPIRES_AFTER` ago, so the rows of servers that died without a clean stop do not pile up.

use std::time::Duration;

use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::Ulid;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::config::Config;
use crate::streams::Streams;

/// How long after its last heartbeat a registration is a dead server's and is deleted (wall clock, the database's):
/// far past the dispatcher's staleness bound (`STUDIO_STREAMING_STALE_SECS`, 10 s), which already passes it over.
pub const EXPIRES_AFTER: Duration = Duration::from_secs(600);

/// This server as it registers: its id, minted at each start, and where a runner and a viewer reach it.
#[derive(Debug, Clone)]
pub struct Registration {
    pub id: Ulid,
    pub runner_url: String,
    pub viewer_url: String,
}

impl Registration {
    pub fn new(id: Ulid, config: &Config) -> Self {
        Self { id, runner_url: config.runner_url.clone(), viewer_url: config.viewer_url.clone() }
    }
}

/// Register this server, or say why not: a database without the table has not had the API's migration 0022.
pub async fn register(pool: &PgPool, registration: &Registration) -> Result<(), String> {
    write(pool, registration, 0).await.map_err(|fault| {
        format!("cannot register in streaming_servers (does the database have the API's migration 0022?): {fault}")
    })
}

/// Refresh the heartbeat every `every`, with the count of open streams; a refresh that fails is said once and tried
/// again next time. Aborting the handle stops the beats.
pub fn beat(pool: PgPool, registration: Registration, streams: Streams, every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut missed = false;
        loop {
            sleep(every).await;
            let open = i32::try_from(streams.open_count()).unwrap_or(i32::MAX);
            match write(&pool, &registration, open).await {
                Ok(()) if missed => {
                    eprintln!("studio-streaming: the heartbeat lands again");
                    missed = false;
                }
                Err(fault) if !missed => {
                    eprintln!("studio-streaming: the heartbeat did not land: {fault}; beating on");
                    missed = true;
                }
                _ => {}
            }
        }
    })
}

/// The row stamped alive by the database's clock, inserted when it is not there, and the expired rows of other, dead
/// servers deleted; never this server's own, which a process asleep past the expiry would otherwise delete and upsert
/// in one statement.
async fn write(pool: &PgPool, registration: &Registration, open_streams: i32) -> Result<(), SqlxError> {
    sqlx::query(
        "WITH expired AS ( \
             DELETE FROM streaming_servers WHERE heartbeat_at < now() - make_interval(secs => $5) AND id <> $1 \
         ) \
         INSERT INTO streaming_servers (id, runner_url, viewer_url, heartbeat_at, open_streams) \
         VALUES ($1, $2, $3, now(), $4) \
         ON CONFLICT (id) DO UPDATE SET heartbeat_at = now(), open_streams = EXCLUDED.open_streams",
    )
    .bind(registration.id.as_str())
    .bind(&registration.runner_url)
    .bind(&registration.viewer_url)
    .bind(open_streams)
    .bind(EXPIRES_AFTER.as_secs_f64())
    .execute(pool)
    .await?;
    Ok(())
}

/// Whether the database has the API's migrations 0023 and 0025, whose columns on `attempt_streams` this server writes
/// for the picture; without them the start is refused in words, as it is without 0022's tables.
pub async fn require_live_frames(pool: &PgPool) -> Result<(), String> {
    sqlx::query(
        "SELECT scenario_key, segments_uploaded, locked_sim_ms, cached_at, last_opened_at, uploads_finished_at \
         FROM attempt_streams LIMIT 0",
    )
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|fault| {
        format!(
            "attempt_streams lacks the live-frame columns (does the database have the API's migrations \
             0023_live_frames and 0025_uploads_finished?): {fault}"
        )
    })
}

/// A clean stop takes the row with it; a crash leaves it to go stale and then expire.
pub async fn unregister(pool: &PgPool, id: &Ulid) {
    if let Err(fault) = sqlx::query("DELETE FROM streaming_servers WHERE id = $1").bind(id.as_str()).execute(pool).await
    {
        eprintln!("studio-streaming: our row in streaming_servers was not deleted, and will expire: {fault}");
    }
}
