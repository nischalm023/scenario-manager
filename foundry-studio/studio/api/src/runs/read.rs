//! Within `runs`, `read` is spec 08 section 3.7's run reads, which the Runs panel and the agent's run tools share so the
//! agent never quotes a figure the panel does not show: the list (a site's runs by default, newest submission first),
//! one run with every attempt and what its manifest said, the queue across the deployment, the deployment's run
//! counts for whoever watches it, and an attempt's manifest as its runner handed it in. None writes, and none asks the
//! platform: what the attempts' rows hold is what is answered.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use runstore::RunStorage;
use serde::Deserialize;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{
    CompiledRead, EndingCounts, Expectation, JournalSeq, Manifest, PlanRunner, Problem, ProblemType, RunCount, RunCut,
    RunDetail, RunList, RunQueue, RunState, RunStats, RunSummary, ScopePath, SeatsRead, Seq, Ulid, UnitCut, WallClock,
};

use crate::AppState;
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, validation_failed};
use crate::results::fill_headlines;

use super::rows::{self, Filter, STALLED_AFTER, json};
use super::store::JOB_NOT_CREATED;

/// How far back `GET /api/runs/stats` counts endings, Job creations that failed and runners lost: 24 h of wall clock.
const STATS_WINDOW: Duration = Duration::from_secs(24 * 3600);

const LIST_DEFAULT: i64 = 50;
const LIST_MOST: i64 = 200;

#[derive(Deserialize)]
pub struct ListQuery {
    site: Option<String>,
    mine: Option<bool>,
    scenario: Option<Ulid>,
    submission: Option<Ulid>,
    state: Option<RunState>,
    after: Option<Ulid>,
    limit: Option<i64>,
    highlights: Option<JournalSeq>,
}

/// `GET /api/runs?site=&mine=&scenario=&submission=&state=&after=&limit=&highlights=`: newest submission first, a page
/// at a time; `next` names the page's last run, which `after` continues from, while more runs follow it; `highlights`
/// names the agent's `show` entry whose metrics headline a run whose project pins none, as on the run's own page.
pub async fn list(
    State(app): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<RunList>, ProblemAnswer> {
    let instance = uri.path();
    let Query(query) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let limit = query.limit.unwrap_or(LIST_DEFAULT).clamp(1, LIST_MOST);
    let filter = Filter {
        site: query.site,
        submitted_by: query.mine.unwrap_or(false).then(|| identity.subject.to_string()),
        scenario: query.scenario,
        submission: query.submission,
        state: query.state,
        runs: None,
        after: query.after,
        limit: limit + 1,
    };
    let fault = |fault| internal("list the runs", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    if let Some(after) = &filter.after
        && rows::summary_of(&mut conn, after).await.map_err(fault)?.is_none()
    {
        return Err(validation_failed(format!("`after` names {after}, which is no run"), instance));
    }
    let mut items = rows::summaries(&mut conn, &filter).await.map_err(fault)?;
    let more = items.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    fill_headlines(&mut conn, &mut items, query.highlights).await.map_err(fault)?;
    let next = more.then(|| items.last().map(|run| run.id.to_string())).flatten();
    Ok(Json(RunList { items, next }))
}

/// Every run of `submission`, in position order, as the list shows each.
pub async fn submission_runs(conn: &mut PgConnection, submission: &Ulid) -> Result<Vec<RunSummary>, SqlxError> {
    let filter = Filter { submission: Some(submission.clone()), limit: i64::from(u32::MAX), ..Filter::default() };
    rows::summaries(conn, &filter).await
}

/// `GET /api/runs/queue`: what holds a seat and what waits, in the order the waiting start.
pub async fn queue(State(app): State<AppState>, uri: Uri) -> Result<Json<RunQueue>, ProblemAnswer> {
    let instance = uri.path();
    let fault = |fault| internal("read the queue", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    let held: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT run_id FROM attempts WHERE job IS NOT NULL AND job_stopped_at IS NULL")
            .fetch_all(&mut *conn)
            .await
            .map_err(fault)?;
    let waiting: Vec<String> = sqlx::query_scalar(
        "SELECT a.run_id FROM attempts a JOIN runs r ON r.id = a.run_id WHERE a.state = 'queued' AND a.job IS NULL \
         ORDER BY a.requested_at, r.position, a.n",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(fault)?;
    let of = |ids: &[String]| -> Vec<Ulid> { ids.iter().filter_map(|id| id.parse().ok()).collect() };
    let filter = |runs: Vec<Ulid>| Filter { limit: runs.len().max(1) as i64, runs: Some(runs), ..Filter::default() };
    let going = rows::summaries(&mut conn, &filter(of(&held))).await.map_err(fault)?;
    let mut waiting_runs = rows::summaries(&mut conn, &filter(of(&waiting))).await.map_err(fault)?;
    waiting_runs.sort_by_key(|run| run.waiting.and_then(|waiting| waiting.place_in_queue));
    Ok(Json(RunQueue { limit: RunCount(app.runs.config.at_once.0), going, waiting: waiting_runs }))
}

/// `GET /api/runs/stats`: the deployment's runs counted off the attempts' rows in one query, a Job creation that failed
/// by its ending's typed cause.
pub async fn stats(State(app): State<AppState>, uri: Uri) -> Result<Json<RunStats>, ProblemAnswer> {
    let instance = uri.path();
    let fault = |fault| internal("count the runs", fault, instance);
    let query = format!(
        "SELECT count(*) FILTER (WHERE job IS NOT NULL AND job_stopped_at IS NULL) AS held, \
             count(*) FILTER (WHERE state = 'queued' AND job IS NULL) AS waiting, \
             count(*) FILTER (WHERE state = 'running') AS running, \
             count(*) FILTER (WHERE {stalled}) AS stalled, \
             count(*) FILTER (WHERE state = 'finished' AND ended_at > now() - make_interval(secs => $2)) AS finished, \
             count(*) FILTER (WHERE state = 'cancelled' AND ended_at > now() - make_interval(secs => $2)) AS cancelled, \
             count(*) FILTER (WHERE state = 'failed' AND ended_at > now() - make_interval(secs => $2)) AS failed, \
             count(*) FILTER (WHERE state = 'partial' AND ended_at > now() - make_interval(secs => $2)) AS partial, \
             count(*) FILTER (WHERE ending_cause = $3 AND ended_at > now() - make_interval(secs => $2)) \
                 AS create_failures, \
             min(requested_at) FILTER (WHERE state = 'queued' AND job IS NULL) AS oldest_waiting \
         FROM attempts",
        stalled = rows::stalled("$1"),
    );
    let row = sqlx::query(&query)
        .bind(STALLED_AFTER.as_secs_f64())
        .bind(STATS_WINDOW.as_secs_f64())
        .bind(JOB_NOT_CREATED)
        .fetch_one(&app.pool)
        .await
        .map_err(fault)?;
    let count = |column: &str| -> Result<RunCount, ProblemAnswer> {
        let number: i64 = row.try_get(column).map_err(fault)?;
        Ok(RunCount(u32::try_from(number).unwrap_or(u32::MAX)))
    };
    let oldest: Option<DateTime<Utc>> = row.try_get("oldest_waiting").map_err(fault)?;
    Ok(Json(RunStats {
        seats: SeatsRead { limit: RunCount(app.runs.config.at_once.0), held: count("held")? },
        waiting: count("waiting")?,
        running: count("running")?,
        stalled: count("stalled")?,
        ended_last_24h: EndingCounts {
            finished: count("finished")?,
            cancelled: count("cancelled")?,
            failed: count("failed")?,
            partial: count("partial")?,
        },
        create_failures_last_24h: count("create_failures")?,
        oldest_waiting_since: oldest.map(WallClock::from),
    }))
}

/// `GET /api/runs/{r}`.
pub async fn detail(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<RunDetail>, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let fault = |fault| internal("read the run", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    let summary: RunSummary =
        rows::summary_of(&mut conn, &run).await.map_err(fault)?.ok_or_else(|| no_run(instance))?;
    let row = sqlx::query(
        "SELECT r.cut, r.reads, r.runner_image, r.expected, p.plan -> 'runner' AS runner, s.approval, s.conversation, \
             pe.name AS submitted_by \
         FROM runs r JOIN submissions s ON s.id = r.submission JOIN plans p ON p.id = s.plan \
         JOIN people pe ON pe.subject = s.submitted_by WHERE r.id = $1",
    )
    .bind(run.as_str())
    .fetch_one(&mut *conn)
    .await
    .map_err(fault)?;
    let cut: UnitCut = json(row.try_get("cut").map_err(fault)?).map_err(fault)?;
    let reads: BTreeMap<ScopePath, Seq> = json(row.try_get("reads").map_err(fault)?).map_err(fault)?;
    let runner: PlanRunner = json(row.try_get("runner").map_err(fault)?).map_err(fault)?;
    let expected: Expectation = json(row.try_get("expected").map_err(fault)?).map_err(fault)?;
    let mut attempts = Vec::new();
    for (mut attempt, manifest_key) in rows::attempts(&mut conn, &run).await.map_err(fault)? {
        if let Some(key) = manifest_key
            && let Ok(bytes) = app.runstore.get(&key).await
            && let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes)
        {
            attempt.compile =
                Some(CompiledRead { digest: manifest.compile.digest, compiler: manifest.compile.compiler });
            attempt.exact = Some(manifest.world.exact);
            attempt.exact_reason = manifest.world.exact_reason;
            attempt.walked = Some(manifest.world.walked);
            attempt.stub = Some(manifest.stub);
        }
        attempts.push(attempt);
    }
    let parse = |column: &str| -> Result<Option<Ulid>, ProblemAnswer> {
        let text: Option<String> = row.try_get(column).map_err(fault)?;
        Ok(text.and_then(|text| text.parse().ok()))
    };
    Ok(Json(RunDetail {
        cut: RunCut { unit: cut.unit, requested: cut.requested, branch: cut.branch },
        reads,
        runner,
        expected,
        submitted_by: row.try_get("submitted_by").map_err(fault)?,
        approval: parse("approval")?,
        conversation: parse("conversation")?,
        attempts,
        summary,
    }))
}

#[derive(Deserialize)]
pub struct AttemptQuery {
    attempt: Option<u32>,
}

/// `GET /api/runs/{r}/manifest?attempt=`: the manifest exactly as the runner handed it in, the latest attempt's by
/// default; `not-fetched` before it arrives.
pub async fn manifest(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<AttemptQuery>, QueryRejection>,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let Query(AttemptQuery { attempt }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let fault = |fault| internal("read the manifest", fault, instance);
    let row = sqlx::query(
        "SELECT n, manifest_key FROM attempts WHERE run_id = $1 AND ($2::int IS NULL OR n = $2) ORDER BY n DESC LIMIT 1",
    )
    .bind(run.as_str())
    .bind(attempt.map(|n| i32::try_from(n).unwrap_or(i32::MAX)))
    .fetch_optional(&app.pool)
    .await
    .map_err(fault)?
    .ok_or_else(|| no_run(instance))?;
    let key: Option<String> = row.try_get("manifest_key").map_err(fault)?;
    let Some(key) = key else {
        let detail = "This attempt's runner has not handed in what it read yet.";
        return Err(Problem::new(ProblemType::NotFetched, detail).at(instance).into());
    };
    let bytes = app.runstore.get(&key).await.map_err(|fault| internal("read the manifest", fault, instance))?;
    let mut answer = Body::from(bytes).into_response();
    answer.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(answer)
}

pub fn run_in(path: Result<Path<String>, PathRejection>, instance: &str) -> Result<Ulid, ProblemAnswer> {
    let Ok(Path(id)) = path else { return Err(no_run(instance)) };
    id.parse().map_err(|_| no_run(instance))
}

pub fn no_run(instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::NotFound, "No run has that id.").at(instance).into()
}
