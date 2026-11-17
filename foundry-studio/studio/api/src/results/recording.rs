//! Within `results`, `recording` is `GET /api/runs/{r}/recording?attempt=` (spec 08 section 3.7): the attempt's stored
//! `.worldview` recording as a download, named as the delivery package names one, "{scenario} - {site} - {variant} -
//! {the date its attempt started, wall clock}", so a file passed on says what produced it; a stub attempt's name adds
//! that its results are canned (ruling D110), and an attempt that stored none answers `no-recording`.

use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use run_invocation::Fault;
use runstore::RunStorage;
use serde::Deserialize;
use sqlx::Row;
use studio_protocol::{AttemptNumber, Manifest, Problem, ProblemType, RunSummary, ScopePath};
use wps_client::site::site_name;
use wps_client::wps::Wps;

use crate::GateState;
use crate::problem::{ProblemAnswer, internal, validation_failed};
use crate::runs::{no_run, run_in, summary_of};

#[derive(Deserialize)]
pub struct RecordingQuery {
    attempt: Option<AttemptNumber>,
}

/// `GET /api/runs/{r}/recording?attempt=`: the latest attempt's by default.
pub async fn recording<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<RecordingQuery>, QueryRejection>,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let Query(RecordingQuery { attempt }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let app = &state.app;
    let fault = |fault: Fault| internal("read the run's recording", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(|failure| fault(failure.into()))?;
    let summary =
        summary_of(&mut conn, &run).await.map_err(|failure| fault(failure.into()))?.ok_or_else(|| no_run(instance))?;
    let row = sqlx::query(
        "SELECT n, object_key, manifest_key, COALESCE(started_at, requested_at) AS started FROM attempts \
         WHERE run_id = $1 AND ($2::int IS NULL OR n = $2) ORDER BY n DESC LIMIT 1",
    )
    .bind(run.as_str())
    .bind(attempt.map(|n| i32::try_from(n.0).unwrap_or(i32::MAX)))
    .fetch_optional(&mut *conn)
    .await
    .map_err(|failure| fault(failure.into()))?;
    let Some(row) = row else {
        return Err(Problem::new(ProblemType::NotFound, "This run has no such attempt.").at(instance).into());
    };
    let n: i32 = row.try_get("n").map_err(|failure| fault(failure.into()))?;
    let Some(key) = row.try_get::<Option<String>, _>("object_key").map_err(|failure| fault(failure.into()))? else {
        let detail = format!("Attempt {n} of this run stored no recording, so it has none to give.");
        return Err(Problem::new(ProblemType::NoRecording, detail).at(instance).into());
    };
    let started: DateTime<Utc> = row.try_get("started").map_err(|failure| fault(failure.into()))?;
    let stub = match row.try_get::<Option<String>, _>("manifest_key").map_err(|failure| fault(failure.into()))? {
        Some(manifest) => {
            let bytes = app.runstore.get(&manifest).await.map_err(|failure| fault(failure.into()))?;
            serde_json::from_slice::<Manifest>(&bytes).map_err(|failure| fault(failure.into()))?.stub
        }
        None => false,
    };
    let bytes = app.runstore.get(&key).await.map_err(|failure| fault(failure.into()))?;
    let site = site_name_or_slug(&state, &summary.site).await;
    let name = download_name(&summary, &site, started, stub);
    let mut answer = Body::from(bytes).into_response();
    let headers = answer.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    let disposition = format!("attachment; filename=\"{}\"; filename*=UTF-8''{}", ascii(&name), encoded(&name));
    headers.insert(CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).map_err(|failure| fault(failure.into()))?);
    Ok(answer)
}

/// The site's name as the site list says it (`site_name`), else its root's last segment: a download is never refused
/// for a name world-proto-server could not give.
async fn site_name_or_slug<S: Wps>(state: &GateState<S>, site: &str) -> String {
    let slug = site.rsplit('/').next().unwrap_or(site).to_owned();
    let Ok(root) = site.parse::<ScopePath>() else { return slug };
    let named = async {
        let tree = state.world.subtree(&root).await?;
        match tree.get(&root) {
            Some(listed) => site_name(&state.world, listed).await,
            None => Ok(None),
        }
    };
    named.await.ok().flatten().unwrap_or(slug)
}

/// "{scenario} - {site} - {variant} - {date}.worldview", the variant the run's label after its scenario's name.
pub fn download_name(summary: &RunSummary, site: &str, started: DateTime<Utc>, stub: bool) -> String {
    let mut parts = vec![summary.scenario.name.clone(), site.to_owned()];
    let variant = summary.label.strip_prefix(&format!("{}, ", summary.scenario.name)).unwrap_or_default();
    if !variant.is_empty() {
        parts.push(variant.to_owned());
    }
    parts.push(started.format("%Y-%m-%d").to_string());
    if stub {
        parts.push("canned by the stub runner".to_owned());
    }
    let name = format!("{}.worldview", parts.join(" - "));
    name.chars()
        .map(|letter| if letter == '/' || letter == '\\' || letter.is_control() { '-' } else { letter })
        .collect()
}

/// The name for a reader that takes only ASCII in `filename`, its quotes made safe.
fn ascii(name: &str) -> String {
    name.chars().map(|letter| if letter.is_ascii() && letter != '"' { letter } else { '_' }).collect()
}

/// RFC 5987's encoding of `filename*`: its attribute characters as they are, every other byte as `%XX`.
fn encoded(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'&'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}
