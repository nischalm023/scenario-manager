//! Within `runs`, `cancel` is spec 08 section 3.6's cancel, which never asks (ruling D61: it only stops spending) and
//! answers what was actually done: a run waiting for a seat is cancelled without the invoker; one holding a seat
//! before its runner connected is cancelled and its Job stopped by the reconciler, the seat freeing once the Job is
//! known stopped; a running one is asked to stop (`stopping`) and ends when its runner confirms or the grace passes;
//! an ended one is left as it is. A submission's cancel does this for each of its runs.

use axum::Json;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{
    CancelAnswer, CancelOutcome, Problem, ProblemType, RunCancelled, RunState, SubmissionCancelled, Ulid,
};

use crate::AppState;
use crate::db::read_word;
use crate::problem::{ProblemAnswer, internal};

use super::read::{no_run, run_in};

/// `POST /api/runs/{r}/cancel`.
pub async fn run(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<(StatusCode, Json<CancelAnswer>), ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let fault = |fault| internal("cancel the run", fault, instance);
    let mut tx = app.pool.begin().await.map_err(fault)?;
    let answer = cancel(&mut tx, &run).await.map_err(fault)?.ok_or_else(|| no_run(instance))?;
    tx.commit().await.map_err(fault)?;
    app.runs.wakes.reconcile();
    app.runs.wakes.dispatch();
    Ok((StatusCode::ACCEPTED, Json(answer)))
}

/// `POST /api/submissions/{s}/cancel`: each run of the submission not yet ended, in position order.
pub async fn submission(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<(StatusCode, Json<SubmissionCancelled>), ProblemAnswer> {
    let instance = uri.path();
    let nothing =
        || ProblemAnswer::from(Problem::new(ProblemType::NotFound, "No submission has that id.").at(instance));
    let Ok(Path(id)) = path else { return Err(nothing()) };
    let submission: Ulid = id.parse().map_err(|_| nothing())?;
    let fault = |fault| internal("cancel the submission", fault, instance);
    let mut tx = app.pool.begin().await.map_err(fault)?;
    let runs: Vec<String> = sqlx::query_scalar("SELECT id FROM runs WHERE submission = $1 ORDER BY position")
        .bind(submission.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(fault)?;
    if runs.is_empty() {
        return Err(nothing());
    }
    let mut outcomes = Vec::with_capacity(runs.len());
    for run in runs {
        let run: Ulid = run.parse().map_err(|fault| internal("cancel the submission", fault, instance))?;
        if let Some(answer) = cancel(&mut tx, &run).await.map_err(fault)? {
            outcomes.push(RunCancelled { run, outcome: answer.outcome, state: answer.state });
        }
    }
    tx.commit().await.map_err(fault)?;
    app.runs.wakes.reconcile();
    app.runs.wakes.dispatch();
    Ok((StatusCode::ACCEPTED, Json(SubmissionCancelled { outcomes })))
}

/// Cancel `run`'s latest attempt inside `tx`, under its row's lock; `None` when no run has that id.
async fn cancel(tx: &mut Transaction<'_, Postgres>, run: &Ulid) -> Result<Option<CancelAnswer>, SqlxError> {
    let row = sqlx::query("SELECT n, state, job FROM attempts WHERE run_id = $1 ORDER BY n DESC LIMIT 1 FOR UPDATE")
        .bind(run.as_str())
        .fetch_optional(&mut **tx)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let n: i32 = row.try_get("n")?;
    let state: RunState = read_word(&row.try_get::<String, _>("state")?)?;
    let job: Option<String> = row.try_get("job")?;
    let ended = |stop_reason: &str| {
        sqlx::query(
            "UPDATE attempts SET state = 'cancelled', cancel_requested_at = now(), ended_at = now(), \
                 ended_by = 'person', stop_reason = $3 \
             WHERE run_id = $1 AND n = $2 AND state = 'queued'",
        )
        .bind(run.as_str())
        .bind(n)
        .bind(stop_reason.to_owned())
    };
    let answer = match (state, job) {
        (RunState::Queued, None) => {
            ended("cancelled before it started").execute(&mut **tx).await?;
            CancelAnswer { outcome: CancelOutcome::Cancelled, state: RunState::Cancelled }
        }
        (RunState::Queued, Some(_)) => {
            ended("cancelled before its runner started").execute(&mut **tx).await?;
            CancelAnswer { outcome: CancelOutcome::Cancelled, state: RunState::Cancelled }
        }
        (RunState::Running, _) => {
            sqlx::query(
                "UPDATE attempts SET cancel_requested_at = COALESCE(cancel_requested_at, now()) \
                 WHERE run_id = $1 AND n = $2",
            )
            .bind(run.as_str())
            .bind(n)
            .execute(&mut **tx)
            .await?;
            CancelAnswer { outcome: CancelOutcome::Stopping, state: RunState::Running }
        }
        (state, _) => CancelAnswer { outcome: CancelOutcome::AlreadyEnded, state },
    };
    Ok(Some(answer))
}
