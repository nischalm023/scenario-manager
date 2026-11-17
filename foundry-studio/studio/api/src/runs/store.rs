//! Within `runs`, `store` is the run loops' `RunStore` over the API's tables (spec 08 sections 3.4 and 3.6): a seat
//! reserved under one advisory lock so the count holds, the attempt's Job named with its creator's claim and its token
//! minted in the same transaction; the ending written only while the attempt is queued or running, so the first wins,
//! typed when its Job could not be created; the transitions log read from transactions that can no longer commit
//! behind it; and the platform's words on a held Job. Beside the loops' store, the Worldview service's part, which the
//! API does in the first cut: an attempt's results, its ending with its metrics and series (`results`), and its
//! recording (`recording`), which the runner sent already or left as a file.

mod recording;
mod results;

use std::time::Duration;

use chrono::{DateTime, Utc};
use desunits::SimTime;
use run_invocation::{
    AttemptCount, AttemptKey, AttemptRow, Creator, Cursor, Ended, Fault, HeldAttempt, JobName, Reserved, Round,
    RunStore, RunningProgress, Seats, Token, Transition, TransitionId, TxId,
};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{AttemptNumber, Ending, EndingCounts, RunCount, RunState, StreamingServer, Ulid, WallClock};

use crate::AppState;
use crate::db::{decode, read_word, word};

use super::rows::json;

pub use self::recording::RecordingFault;
pub use self::results::{Recording, Results, ResultsFault, ResultsSource};

/// The typed cause of an ending whose attempt's Job could not be created, which the run counts read.
pub const JOB_NOT_CREATED: &str = "job-not-created";

/// The advisory lock every reservation takes, so two never count the same free seat.
const RESERVING: i64 = 0x5354_5544_494f_5345;

/// The run loops' store: the API's own tables.
#[derive(Clone)]
pub struct PgRunStore {
    app: AppState,
}

impl PgRunStore {
    pub fn new(app: AppState) -> Self {
        Self { app }
    }

    /// Which of the Jobs `named` are of attempts whose seats are freed, which happens only once a Job is known stopped
    /// and, locally, its container's exit settled.
    pub async fn freed(&self, named: &[&str]) -> Result<Vec<JobName>, SqlxError> {
        let freed: Vec<String> =
            sqlx::query_scalar("SELECT job FROM attempts WHERE job = ANY($1) AND job_stopped_at IS NOT NULL")
                .bind(named)
                .fetch_all(&self.app.pool)
                .await?;
        Ok(freed.into_iter().map(JobName::stored).collect())
    }
}

impl RunStore for PgRunStore {
    async fn reserve(&self, limit: Seats, creator: &Creator) -> Result<Option<Reserved>, Fault> {
        let mut tx = self.app.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(RESERVING).execute(&mut *tx).await?;
        let held: i64 =
            sqlx::query_scalar("SELECT count(*) FROM attempts WHERE job IS NOT NULL AND job_stopped_at IS NULL")
                .fetch_one(&mut *tx)
                .await?;
        if held >= i64::from(limit.0) {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT a.run_id, a.n, r.runner_image FROM attempts a JOIN runs r ON r.id = a.run_id \
             WHERE a.state = 'queued' AND a.job IS NULL AND a.cancel_requested_at IS NULL \
             ORDER BY a.requested_at, r.position, a.n LIMIT 1 FOR UPDATE OF a SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let key = key_of(&row, "run_id")?;
        let job = JobName::of(&key);
        sqlx::query(
            "UPDATE attempts SET job = $3, dispatched_at = now(), job_creator = $4, job_claimed_at = now() \
             WHERE run_id = $1 AND n = $2",
        )
        .bind(key.run.as_str())
        .bind(n_of(&key))
        .bind(job.as_str())
        .bind(creator.as_str())
        .execute(&mut *tx)
        .await?;
        let token = mint_token(&mut tx, &key).await?;
        tx.commit().await?;
        Ok(Some(Reserved { key, job, image: row.try_get("runner_image")?, token }))
    }

    async fn release(&self, key: &AttemptKey, creator: &Creator) -> Result<(), Fault> {
        sqlx::query("UPDATE attempts SET job_creator = NULL WHERE run_id = $1 AND n = $2 AND job_creator = $3")
            .bind(key.run.as_str())
            .bind(n_of(key))
            .bind(creator.as_str())
            .execute(&self.app.pool)
            .await?;
        Ok(())
    }

    async fn attempt(&self, key: &AttemptKey) -> Result<Option<AttemptRow>, Fault> {
        let row = sqlx::query(
            "SELECT state, job, cancel_requested_at IS NOT NULL AS cancelled FROM attempts WHERE run_id = $1 AND n = $2",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .fetch_optional(&self.app.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(AttemptRow {
            state: read_word(&row.try_get::<String, _>("state")?)?,
            job: row.try_get::<Option<String>, _>("job")?.map(JobName::stored),
            cancel_requested: row.try_get("cancelled")?,
        }))
    }

    async fn end(&self, key: &AttemptKey, ending: &Ending) -> Result<Ended, Fault> {
        let mut tx = self.app.pool.begin().await?;
        let ended = end(&mut tx, key, ending, None).await?;
        tx.commit().await?;
        Ok(ended)
    }

    async fn end_uncreated(&self, key: &AttemptKey, ending: &Ending) -> Result<Ended, Fault> {
        let mut tx = self.app.pool.begin().await?;
        let ended = end(&mut tx, key, ending, Some(JOB_NOT_CREATED)).await?;
        tx.commit().await?;
        Ok(ended)
    }

    async fn job_stopped(&self, key: &AttemptKey) -> Result<(), Fault> {
        sqlx::query(
            "UPDATE attempts SET job_stopped_at = now() \
             WHERE run_id = $1 AND n = $2 AND job IS NOT NULL AND job_stopped_at IS NULL",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .execute(&self.app.pool)
        .await?;
        Ok(())
    }

    async fn held(&self) -> Result<Vec<HeldAttempt>, Fault> {
        let rows = sqlx::query(
            "SELECT a.run_id, a.n, a.job, a.state, a.dispatched_at, a.cancel_requested_at, a.job_creator, \
                 a.job_claimed_at, a.reported_at, r.runner_image \
             FROM attempts a JOIN runs r ON r.id = a.run_id WHERE a.job IS NOT NULL AND a.job_stopped_at IS NULL \
             ORDER BY a.dispatched_at",
        )
        .fetch_all(&self.app.pool)
        .await?;
        let at = |row: &PgRow, column: &str| {
            row.try_get::<Option<DateTime<Utc>>, _>(column).map(|at| at.map(WallClock::from))
        };
        let mut held = Vec::with_capacity(rows.len());
        for row in &rows {
            held.push(HeldAttempt {
                key: key_of(row, "run_id")?,
                job: JobName::stored(row.try_get("job")?),
                image: row.try_get("runner_image")?,
                state: read_word(&row.try_get::<String, _>("state")?)?,
                dispatched_at: at(row, "dispatched_at")?.ok_or_else(|| decode("a held attempt was dispatched"))?,
                cancel_requested_at: at(row, "cancel_requested_at")?,
                creator: row.try_get::<Option<String>, _>("job_creator")?.map(Creator::stored),
                claimed_at: at(row, "job_claimed_at")?.ok_or_else(|| decode("a held attempt's creation began"))?,
                reported_at: at(row, "reported_at")?,
            });
        }
        Ok(held)
    }

    async fn reclaim(&self, key: &AttemptKey, creator: &Creator, grace: Duration) -> Result<Option<Token>, Fault> {
        let mut tx = self.app.pool.begin().await?;
        // One UPDATE takes the row's lock and judges its conditions on the row as it stands once the lock is held, so
        // a cancel committed meanwhile leaves nothing claimed.
        let claimed = sqlx::query(
            "UPDATE attempts SET job_creator = $3, job_claimed_at = now() \
             WHERE run_id = $1 AND n = $2 AND state = 'queued' AND cancel_requested_at IS NULL AND job IS NOT NULL \
                 AND job_stopped_at IS NULL AND job_creator IS DISTINCT FROM $3 \
                 AND job_claimed_at <= now() - make_interval(secs => $4)",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .bind(creator.as_str())
        .bind(grace.as_secs_f64())
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() == 0 {
            return Ok(None);
        }
        let token = mint_token(&mut tx, key).await?;
        tx.commit().await?;
        Ok(Some(token))
    }

    async fn cursor(&self) -> Result<Cursor, Fault> {
        let (tx, id): (i64, i64) =
            sqlx::query_as("SELECT last_tx, last_transition FROM run_event_cursor").fetch_one(&self.app.pool).await?;
        Ok(Cursor { tx: TxId(tx), id: TransitionId(id) })
    }

    async fn transitions_after(&self, cursor: Cursor) -> Result<Vec<Transition>, Fault> {
        // Only transactions older than every one still running: none of them can commit a row behind these any more.
        let rows = sqlx::query(
            "SELECT id, tx, run, n, \"from\", \"to\" FROM attempt_transitions \
             WHERE (tx, id) > ($1, $2) AND tx < pg_snapshot_xmin(pg_current_snapshot())::text::bigint \
             ORDER BY tx, id LIMIT 500",
        )
        .bind(cursor.tx.0)
        .bind(cursor.id.0)
        .fetch_all(&self.app.pool)
        .await?;
        let mut transitions = Vec::with_capacity(rows.len());
        for row in &rows {
            transitions.push(Transition {
                at: Cursor { tx: TxId(row.try_get("tx")?), id: TransitionId(row.try_get("id")?) },
                key: key_of(row, "run")?,
                from: row.try_get::<Option<String>, _>("from")?.map(|from| read_word(&from)).transpose()?,
                to: read_word(&row.try_get::<String, _>("to")?)?,
            });
        }
        Ok(transitions)
    }

    async fn advance(&self, cursor: Cursor) -> Result<(), Fault> {
        sqlx::query(
            "UPDATE run_event_cursor SET last_tx = $1, last_transition = $2 WHERE (last_tx, last_transition) < ($1, $2)",
        )
        .bind(cursor.tx.0)
        .bind(cursor.id.0)
        .execute(&self.app.pool)
        .await?;
        Ok(())
    }

    async fn progress_of_running(&self) -> Result<Vec<RunningProgress>, Fault> {
        let rows =
            sqlx::query("SELECT run_id, n, progress_sim_ms, progress_values FROM attempts WHERE state = 'running'")
                .fetch_all(&self.app.pool)
                .await?;
        let mut running = Vec::with_capacity(rows.len());
        for row in &rows {
            running.push(RunningProgress {
                key: key_of(row, "run_id")?,
                sim_ms: SimTime::ms(u64::try_from(row.try_get::<i64, _>("progress_sim_ms")?).unwrap_or(0)),
                values: json(row.try_get("progress_values")?)?,
            });
        }
        Ok(running)
    }

    async fn round_ended(&self, at: &Transition) -> Result<Option<Round>, Fault> {
        // Each run's state as the log stood at `at`: its latest transition up to it, in the watcher's (tx, id) order.
        let row = sqlx::query(
            "WITH sub AS (SELECT submission FROM runs WHERE id = $1), \
                  upto AS (SELECT t.run, t.\"from\", t.\"to\", t.tx, t.id FROM attempt_transitions t \
                      JOIN runs r ON r.id = t.run WHERE r.submission = (SELECT submission FROM sub) \
                      AND (t.tx, t.id) <= ($2, $3)), \
                  latest AS (SELECT DISTINCT ON (run) run, \"to\" FROM upto ORDER BY run, tx DESC, id DESC) \
             SELECT (SELECT submission FROM sub) AS submission, \
                 (SELECT count(*) FROM runs WHERE submission = (SELECT submission FROM sub)) AS runs, \
                 (SELECT count(*) FROM upto WHERE \"from\" IS NULL) AS attempts, \
                 count(*) AS seen, count(*) FILTER (WHERE \"to\" IN ('queued', 'running')) AS open, \
                 count(*) FILTER (WHERE \"to\" = 'finished') AS finished, \
                 count(*) FILTER (WHERE \"to\" = 'cancelled') AS cancelled, \
                 count(*) FILTER (WHERE \"to\" = 'failed') AS failed, \
                 count(*) FILTER (WHERE \"to\" = 'partial') AS partial \
             FROM latest",
        )
        .bind(at.key.run.as_str())
        .bind(at.at.tx.0)
        .bind(at.at.id.0)
        .fetch_one(&self.app.pool)
        .await?;
        let number = |column: &str| -> Result<u32, Fault> { Ok(u32::try_from(row.try_get::<i64, _>(column)?)?) };
        if number("seen")? != number("runs")? || number("open")? > 0 {
            return Ok(None);
        }
        let counts = EndingCounts {
            finished: RunCount(number("finished")?),
            cancelled: RunCount(number("cancelled")?),
            failed: RunCount(number("failed")?),
            partial: RunCount(number("partial")?),
        };
        let submission =
            row.try_get::<Option<String>, _>("submission")?.ok_or("the transition's run is in no submission")?;
        let attempts = AttemptCount(number("attempts")?);
        Ok(Some(Round { submission: submission.parse().map_err(decode)?, attempts, counts }))
    }

    async fn stream_to(&self, key: &AttemptKey, stale_after: Duration) -> Result<Option<StreamingServer>, Fault> {
        // The heartbeats are the database's clock, so their age is too.
        let row = sqlx::query(
            "WITH picked AS ( \
                 SELECT id, runner_url FROM streaming_servers \
                 WHERE heartbeat_at > now() - make_interval(secs => $3) ORDER BY random() LIMIT 1 \
             ), recorded AS ( \
                 UPDATE attempts SET streaming_server = (SELECT id FROM picked) WHERE run_id = $1 AND n = $2 \
                 RETURNING n \
             ) \
             SELECT picked.id, picked.runner_url FROM picked, recorded",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .bind(stale_after.as_secs_f64())
        .fetch_optional(&self.app.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(StreamingServer {
            server: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
            endpoint: row.try_get("runner_url")?,
        }))
    }

    async fn platform_status(&self, key: &AttemptKey, words: &str) -> Result<(), Fault> {
        sqlx::query("UPDATE attempts SET platform_status = $3 WHERE run_id = $1 AND n = $2")
            .bind(key.run.as_str())
            .bind(n_of(key))
            .bind(words)
            .execute(&self.app.pool)
            .await?;
        Ok(())
    }
}

/// A fresh token for the attempt inside `tx`, its hash over any older one's, which dies with it (spec 01 section 3.16).
async fn mint_token(tx: &mut Transaction<'_, Postgres>, key: &AttemptKey) -> Result<Token, SqlxError> {
    let token = Token::mint();
    sqlx::query(
        "INSERT INTO runner_tokens (run, n, token_sha256) VALUES ($1, $2, $3) \
         ON CONFLICT (run, n) DO UPDATE SET token_sha256 = EXCLUDED.token_sha256, minted_at = now()",
    )
    .bind(key.run.as_str())
    .bind(n_of(key))
    .bind(token.sha256())
    .execute(&mut **tx)
    .await?;
    Ok(token)
}

/// The guarded ending inside `tx`, typed with `cause` where a count reads it: written only while the attempt is queued
/// or running.
async fn end(
    tx: &mut Transaction<'_, Postgres>,
    key: &AttemptKey,
    ending: &Ending,
    cause: Option<&str>,
) -> Result<Ended, SqlxError> {
    let failed = ending.state == RunState::Failed;
    let wrote = sqlx::query(
        "UPDATE attempts SET state = $3, ended_at = now(), ended_by = $4, stop_reason = $5, stop_detail = $6, \
             error = CASE WHEN $7 THEN $5 END, progress_sim_ms = GREATEST(progress_sim_ms, $8), ending_cause = $9 \
         WHERE run_id = $1 AND n = $2 AND state IN ('queued', 'running')",
    )
    .bind(key.run.as_str())
    .bind(n_of(key))
    .bind(word(ending.state))
    .bind(word(ending.ended_by))
    .bind(&ending.stop_reason)
    .bind(&ending.stop_detail)
    .bind(failed)
    .bind(i64::try_from(ending.sim_ms.as_ms()).unwrap_or(i64::MAX))
    .bind(cause)
    .execute(&mut **tx)
    .await?;
    Ok(if wrote.rows_affected() == 1 { Ended::Wrote } else { Ended::AlreadyEnded })
}

fn key_of(row: &PgRow, run: &str) -> Result<AttemptKey, SqlxError> {
    let id: Ulid = row.try_get::<String, _>(run)?.parse().map_err(decode)?;
    let n: i32 = row.try_get("n")?;
    Ok(AttemptKey { run: id, n: AttemptNumber(u32::try_from(n).map_err(decode)?) })
}

/// An attempt number as its `INTEGER` column holds it.
pub fn n_of(key: &AttemptKey) -> i32 {
    i32::try_from(key.n.0).unwrap_or(i32::MAX)
}
