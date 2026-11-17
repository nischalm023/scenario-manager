//! Within `crucible`, `registry` is WHICH RUNS EXIST AND HOW EACH TRY AT ONE
//! ENDED: the PostgreSQL tables behind `list`, `status` and the queue, and the
//! only place in the server that speaks SQL.
//!
//! It exists so that a restart is not an amnesia. The worker pool, the
//! recording and the websocket all hold state in memory that dies with the
//! process; what a run WAS survives here, which is why the restart sweep can
//! say that an attempt found `running` ended without an ending.
//!
//! Two disciplines run through every write. Each transition is a conditional
//! `UPDATE` that reports whether it matched, so a message about an attempt that
//! is no longer the one running moves nothing. And the queue is claimed under
//! `FOR UPDATE SKIP LOCKED`, so two workers reaching for the same queued
//! attempt take two different ones rather than one of them waiting.
//!
//! The queries are runtime `sqlx::query`, not the compile-time-checked macros,
//! so the workspace builds with no database in sight and no `.sqlx` offline
//! directory to keep in step. Column types are asserted at each `try_get`.

use std::result::Result as StdResult;
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::migrate::MigrateError;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Error as SqlxError, PgPool, Row};
use thiserror::Error;

use crate::protocol::{AttemptState, AttemptSummary, Progress, RecordingExtent, RunSummary};

/// Which runs exist, and how each try at one ended.
#[derive(Clone)]
pub struct Registry {
    pool: PgPool,
}

/// An attempt the worker pool has taken off the queue and is about to run.
#[derive(Debug, Clone)]
pub struct ClaimedAttempt {
    /// The run it belongs to.
    pub run: String,
    /// Which try this is.
    pub n: u32,
    /// What the run is called; it becomes the recording's own name.
    pub name: String,
    /// Where the spec it runs is stored.
    pub spec_key: String,
}

/// What the registry can fail to do.
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("the registry refused a query: {0}")]
    Db(#[from] SqlxError),
    #[error("the registry's schema could not be brought up to date: {0}")]
    Migrate(#[from] MigrateError),
    #[error("the registry holds `{0}`, which this build does not understand")]
    UnknownValue(String),
    #[error("attempt {attempt} of run `{run}` is not running, so this ending was not written")]
    NotRunning { run: String, attempt: u32 },
}

type Result<T> = StdResult<T, RegistryError>;

impl Registry {
    /// Open the registry and bring its schema up to date. Both are startup
    /// preconditions: a server that cannot record its runs does not start.
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self> {
        let pool = PgPoolOptions::new().max_connections(max_connections).connect(database_url).await?;
        Self::from_pool(pool).await
    }

    /// Wrap a pool somebody else opened and migrate it. This is the door the
    /// tests come in through, so they get the schema the server gets.
    pub async fn from_pool(pool: PgPool) -> Result<Self> {
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// Record a submitted run. It has no attempt yet, so nothing can claim it:
    /// `submitted {run}` reaches the client before a worker could start, which
    /// is what the design means by the reply coming "before any work starts".
    pub async fn submit(&self, run: &str, name: &str, spec_key: &str) -> Result<()> {
        sqlx::query("INSERT INTO runs (id, name, spec_key) VALUES ($1, $2, $3)")
            .bind(run)
            .bind(name)
            .bind(spec_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Give a recorded run its first attempt, queued — the moment it becomes
    /// work a worker can take.
    pub async fn queue_first_attempt(&self, run: &str) -> Result<()> {
        sqlx::query("INSERT INTO attempts (run_id, n, state) VALUES ($1, 1, 'queued') ON CONFLICT DO NOTHING")
            .bind(run)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Every attempt a running server left behind, for a restart to account
    /// for one by one.
    pub async fn running_attempts(&self) -> Result<Vec<(String, u32)>> {
        let rows = sqlx::query("SELECT run_id, n FROM attempts WHERE state = 'running' ORDER BY run_id, n")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get::<String, _>("run_id")?, row.try_get::<i32, _>("n")? as u32)))
            .collect()
    }

    /// Take the oldest queued attempt and mark it running.
    ///
    /// `SKIP LOCKED` is what makes this safe to call from every worker at once:
    /// a worker that finds a row locked moves to the next one instead of
    /// waiting behind it. `r.id` breaks the tie between two runs submitted
    /// inside one transaction's timestamp — a ULID sorts by the instant it was
    /// minted, so two such runs still start in the order they were submitted —
    /// and the outer `UPDATE` re-checks `queued` as layered checking.
    pub async fn claim_queued(&self) -> Result<Option<ClaimedAttempt>> {
        let row = sqlx::query(
            "UPDATE attempts SET state = 'running', started_at = now()
             WHERE state = 'queued' AND (run_id, n) = (
                 SELECT a.run_id, a.n FROM attempts a JOIN runs r ON r.id = a.run_id
                 WHERE a.state = 'queued'
                 ORDER BY r.created_at, r.id, a.n
                 FOR UPDATE OF a SKIP LOCKED
                 LIMIT 1
             )
             RETURNING run_id, n,
                       (SELECT name FROM runs WHERE runs.id = attempts.run_id) AS name,
                       (SELECT spec_key FROM runs WHERE runs.id = attempts.run_id) AS spec_key",
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            Ok(ClaimedAttempt {
                run: row.try_get::<String, _>("run_id")?,
                n: row.try_get::<i32, _>("n")? as u32,
                name: row.try_get::<String, _>("name")?,
                spec_key: row.try_get::<String, _>("spec_key")?,
            })
        })
        .transpose()
    }

    /// Note how far a running attempt has got. Guarded on `running`, so a
    /// reading that arrives after the run ended does not reopen it.
    pub async fn record_progress(&self, run: &str, n: u32, progress: Progress) -> Result<()> {
        sqlx::query(
            "UPDATE attempts SET progress_sim_ms = $3, progress_picks = $4
             WHERE run_id = $1 AND n = $2 AND state = 'running'",
        )
        .bind(run)
        .bind(n as i32)
        .bind(progress.sim_ms as i64)
        .bind(progress.picks as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Close an attempt: the state it ended in, why it stopped, where its
    /// recording is, and what went wrong if anything did.
    ///
    /// Only an attempt still `running` is closed, and the report says whether
    /// one was. Whoever writes an ending first wins, and nobody writes a second
    /// one over it: a restart sweeping a live attempt to `partial` is the case
    /// that matters, because the server still running that attempt would
    /// otherwise write `finished` back over the sweep and a run nobody can
    /// account for would read as a clean one.
    /// `extent` is what the recording just written holds. It is stored here
    /// rather than read off the file later because this is the one moment the
    /// file is certainly on this machine, and because `list` — asked once a
    /// second by every open browser — cannot fetch an object per run.
    pub async fn attempt_ended(
        &self,
        run: &str,
        n: u32,
        state: AttemptState,
        stop_reason: Option<&str>,
        object_key: Option<&str>,
        error: Option<&str>,
        extent: Option<RecordingExtent>,
    ) -> Result<()> {
        let closed = sqlx::query(
            "UPDATE attempts SET state = $3, ended_at = now(),
                                 stop_reason = $4, object_key = $5, error = $6,
                                 recording_live_edge_ms = $7, recording_frames = $8,
                                 recording_size_bytes = $9
             WHERE run_id = $1 AND n = $2 AND state = 'running'
             RETURNING n",
        )
        .bind(run)
        .bind(n as i32)
        .bind(state.as_str())
        .bind(stop_reason)
        .bind(object_key)
        .bind(error)
        .bind(extent.map(|held| held.live_edge_ms as i64))
        .bind(extent.map(|held| held.frames as i64))
        .bind(extent.map(|held| held.size_bytes as i64))
        .fetch_optional(&self.pool)
        .await?;
        match closed.is_some() {
            true => Ok(()),
            false => Err(RegistryError::NotRunning { run: run.to_string(), attempt: n }),
        }
    }

    /// End a queued attempt outright, which is what cancelling a run nothing
    /// has started yet amounts to. Reports whether it was still queued.
    pub async fn cancel_queued(&self, run: &str) -> Result<bool> {
        let ended = sqlx::query(
            "UPDATE attempts SET state = 'cancelled', ended_at = now(), stop_reason = 'cancelled before it started'
             WHERE run_id = $1 AND state = 'queued'
             RETURNING n",
        )
        .bind(run)
        .fetch_optional(&self.pool)
        .await?;
        Ok(ended.is_some())
    }

    /// Close one interrupted attempt as `finished` with the recording that was
    /// found stored for it: the upload landed and the ending did not, and the
    /// object is the run.
    pub async fn attempt_reconciled(&self, run: &str, n: u32, object_key: &str) -> Result<()> {
        sqlx::query(
            "UPDATE attempts SET state = 'finished', ended_at = COALESCE(ended_at, now()),
                                 stop_reason = 'the recording was stored before the server stopped',
                                 object_key = $3
             WHERE run_id = $1 AND n = $2 AND state = 'running'",
        )
        .bind(run)
        .bind(n as i32)
        .bind(object_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark every attempt left `running` as `partial`, and report how many.
    ///
    /// This is what a restart does first. The process died mid-run, so how far
    /// those runs got is not known: nothing is synthesised for them, and their
    /// recordings — whatever prefix reached the temporary folder — are not
    /// stored, because a prefix nobody can vouch for is not a recording.
    pub async fn sweep_interrupted(&self) -> Result<u64> {
        let swept = sqlx::query(
            "UPDATE attempts SET state = 'partial', ended_at = COALESCE(ended_at, now()),
                                 stop_reason = 'the server stopped while this run was going'
             WHERE state = 'running'",
        )
        .execute(&self.pool)
        .await?;
        Ok(swept.rows_affected())
    }

    /// Every run, newest first — all of them. A cap here would drop the oldest
    /// run without saying so; if a listing ever needs paging it gets a field in
    /// the protocol that says which page it is.
    pub async fn list(&self) -> Result<Vec<RunSummary>> {
        let rows = sqlx::query("SELECT id, name, created_at FROM runs ORDER BY created_at DESC, id DESC")
            .fetch_all(&self.pool)
            .await?;
        let mut runs = Vec::with_capacity(rows.len());
        for row in rows {
            runs.push(self.with_attempts(row).await?);
        }
        Ok(runs)
    }

    /// One run, or nothing if no such run was ever submitted.
    pub async fn summary(&self, run: &str) -> Result<Option<RunSummary>> {
        let row = sqlx::query("SELECT id, name, created_at FROM runs WHERE id = $1")
            .bind(run)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => Ok(Some(self.with_attempts(row).await?)),
            None => Ok(None),
        }
    }

    /// Fill in a run row's attempts.
    async fn with_attempts(&self, row: PgRow) -> Result<RunSummary> {
        let run: String = row.try_get("id")?;
        let created: DateTime<Utc> = row.try_get("created_at")?;
        let attempts = sqlx::query(
            "SELECT n, state, started_at, ended_at, stop_reason, object_key, error,
                    progress_sim_ms, progress_picks,
                    recording_live_edge_ms, recording_frames, recording_size_bytes
             FROM attempts WHERE run_id = $1 ORDER BY n",
        )
        .bind(&run)
        .fetch_all(&self.pool)
        .await?;
        let attempts = attempts.into_iter().map(attempt_summary).collect::<Result<Vec<_>>>()?;
        // The run's progress is its newest attempt's; a run with no attempt has
        // got nowhere, which is what the default says.
        let progress = attempts.last().map(|attempt| attempt.progress).unwrap_or_default();
        Ok(RunSummary { run, name: row.try_get("name")?, created_at: rfc3339(created), attempts, progress })
    }
}

/// Read one attempt row.
fn attempt_summary(row: PgRow) -> Result<AttemptSummary> {
    let state: String = row.try_get("state")?;
    Ok(AttemptSummary {
        n: row.try_get::<i32, _>("n")? as u32,
        state: AttemptState::from_str(&state).map_err(RegistryError::UnknownValue)?,
        started_at: row.try_get::<Option<DateTime<Utc>>, _>("started_at")?.map(rfc3339),
        ended_at: row.try_get::<Option<DateTime<Utc>>, _>("ended_at")?.map(rfc3339),
        stop_reason: row.try_get("stop_reason")?,
        object_key: row.try_get("object_key")?,
        error: row.try_get("error")?,
        progress: Progress {
            sim_ms: row.try_get::<i64, _>("progress_sim_ms")? as u64,
            picks: row.try_get::<i64, _>("progress_picks")? as u64,
        },
        recording: recording_extent(&row)?,
    })
}

/// What the attempt's recording holds, as the ending wrote it down. All three
/// columns are written together, so one of them standing alone is a row no
/// version of this server produced and is read as no recording.
fn recording_extent(row: &PgRow) -> Result<Option<RecordingExtent>> {
    let edge: Option<i64> = row.try_get("recording_live_edge_ms")?;
    let frames: Option<i64> = row.try_get("recording_frames")?;
    let size: Option<i64> = row.try_get("recording_size_bytes")?;
    Ok(match (edge, frames, size) {
        (Some(edge), Some(frames), Some(size)) => {
            Some(RecordingExtent { live_edge_ms: edge as u64, frames: frames as u64, size_bytes: size as u64 })
        }
        _ => None,
    })
}

/// One spelling of an instant everywhere it crosses the wire.
fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}
