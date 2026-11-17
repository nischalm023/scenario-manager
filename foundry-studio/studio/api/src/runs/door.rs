//! Within `runs`, `door` is the runners' door (spec 01 section 3.16; spec 08 section 3.7; spec 07 section 3.8): the
//! routes the second listener serves, which exist because a Job holds at most 1 MB of JSON, so a runner fetches its
//! own inputs after it starts (ruling D77), and delivers its own results, since a Job's pod keeps no folder the API
//! could read. Each opens only with an attempt's own token, sent as `Authorization: Bearer`, matched by its SHA-256 and
//! dead once the attempt's Job is known stopped: the attempt's pins; its manifest, handed in before simtime starts,
//! which the API compares with the check, journaling each finding, and whose arrival writes `running` in the first
//! cut; the sealed scenario version its pins name; the workload generator (ruling D96); and what the runner sends of
//! its run (`delivery`): its heartbeat, its recording and its results. No identity header is read here.

mod delivery;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Display;
use std::mem::take;

use axum::body::{Body, Bytes};
use axum::extract::rejection::PathRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use desunits::SimDuration;
use run_invocation::{AttemptKey, Token};
use runstore::RunStorage;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, attempt_prefix};
use scenario_spec::{Violation, WorkloadLines, check_request, facts_of, ndjson_line};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Error as SqlxError, Row};
use studio_protocol::{
    AttemptControl, AttemptNumber, Endpoint, Expectation, GenerateRequest, JournalKind, Manifest, ManifestAccepted,
    PinnedImage, PinnedScenario, Pins, Problem, ProblemItem, ProblemType, Revision, ScopePath, SealedVersion, Seed,
    Seq, Severity, SitePath, Subject, Ulid, UnitCut, VersionNumber,
};
use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use wps_client::WpsError;

use crate::db::{decode, word};
use crate::journal::NewEntry;
use crate::problem::{ProblemAnswer, internal, json_body, not_found, wps_problem};
use crate::scenarios::sealed_version;
use crate::{AppState, BODY_LIMIT};

use self::delivery::{REPORT_LIMIT, heartbeat, recording, results};
use super::events::journal_on_site;
use super::findings::{Checked, findings};
use super::rows::json as from_json;

/// The generator's stream goes out in chunks of about this many bytes, at most this many of them made and not yet
/// sent: what the route holds of a workload, however large.
const CHUNK_BYTES: usize = 16 * 1024;
const CHUNKS_WAITING: usize = 8;

pub fn routes(app: AppState) -> Router {
    Router::new()
        .route("/runner/v1/attempts/{run}/{n}", get(pins))
        .route("/runner/v1/attempts/{run}/{n}/control", get(control))
        .route("/runner/v1/attempts/{run}/{n}/manifest", put(manifest))
        .route("/runner/v1/attempts/{run}/{n}/heartbeat", put(heartbeat))
        // The recording streams to a file, which its own limit bounds, rather than into memory.
        .route("/runner/v1/attempts/{run}/{n}/recording", put(recording).layer(DefaultBodyLimit::disable()))
        .route("/runner/v1/attempts/{run}/{n}/results", put(results).layer(DefaultBodyLimit::max(REPORT_LIMIT)))
        .route("/runner/v1/scenarios/{scenario}/versions/{version}", get(version))
        .route("/runner/v1/workloads/generate", post(generate))
        .fallback(not_found)
        .method_not_allowed_fallback(not_found)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(app)
}

/// The attempt a presented token opens, among the attempts whose Job is not known stopped.
async fn bearer(app: &AppState, headers: &HeaderMap, instance: &str) -> Result<AttemptKey, ProblemAnswer> {
    let refused = || -> ProblemAnswer {
        let detail = "This token opens no attempt the runners' listener serves: it is wrong, or its Job has stopped.";
        Problem::new(ProblemType::RunnerTokenInvalid, detail).at(instance).into()
    };
    let presented = headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| {
        value.strip_prefix("Bearer ").map(str::trim).filter(|token| !token.is_empty()).map(Token::presented)
    });
    let Some(token) = presented else { return Err(refused()) };
    // The rule is migration 0022's view, which a streaming server admitting a runner reads too.
    let row = sqlx::query("SELECT run, n FROM live_runner_tokens WHERE token_sha256 = $1")
        .bind(token.sha256())
        .fetch_optional(&app.pool)
        .await
        .map_err(|fault| internal("read a runner's token", fault, instance))?
        .ok_or_else(refused)?;
    let run: String = row.try_get("run").map_err(|fault| internal("read a runner's token", fault, instance))?;
    let n: i32 = row.try_get("n").map_err(|fault| internal("read a runner's token", fault, instance))?;
    Ok(AttemptKey {
        run: run.parse().map_err(|_| refused())?,
        n: AttemptNumber(u32::try_from(n).map_err(|_| refused())?),
    })
}

/// The attempt a route's path names, which must be the one its token opens.
async fn own(
    app: &AppState,
    headers: &HeaderMap,
    path: Result<Path<(String, u32)>, PathRejection>,
    instance: &str,
) -> Result<AttemptKey, ProblemAnswer> {
    let key = bearer(app, headers, instance).await?;
    let named = path.ok().and_then(|Path((run, n))| run.parse::<Ulid>().ok().map(|run| (run, n)));
    match named {
        Some((run, n)) if run == key.run && n == key.n.0 => Ok(key),
        _ => {
            let detail = "This token opens another attempt's routes, not this one's.";
            Err(Problem::new(ProblemType::RunnerTokenInvalid, detail).at(instance).into())
        }
    }
}

/// `GET /runner/v1/attempts/{run}/{n}`: the attempt's pins, all fixed before anyone approved it.
async fn pins(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
) -> Result<Json<Pins>, ProblemAnswer> {
    let instance = uri.path();
    let key = own(&app, &headers, path, instance).await?;
    let fault = |fault| internal("read the attempt's pins", fault, instance);
    let row = sqlx::query(
        "SELECT r.cut, r.reads, r.scenario, r.scenario_version, r.variant, r.seed, r.duration_sim_ms, \
             r.report_window, r.runner_image, r.expected, r.experiment, s.plan, e.revision \
         FROM runs r JOIN submissions s ON s.id = r.submission JOIN experiments e ON e.id = r.experiment \
         WHERE r.id = $1",
    )
    .bind(key.run.as_str())
    .fetch_one(&app.pool)
    .await
    .map_err(fault)?;
    let read = || -> Result<Pins, SqlxError> {
        let parse =
            |column: &str| -> Result<Ulid, SqlxError> { row.try_get::<String, _>(column)?.parse().map_err(decode) };
        Ok(Pins {
            run: key.run.clone(),
            attempt: key.n,
            plan: parse("plan")?,
            experiment: parse("experiment")?,
            cut: from_json(row.try_get("cut")?)?,
            reads: from_json(row.try_get("reads")?)?,
            scenario: PinnedScenario {
                id: parse("scenario")?,
                version: VersionNumber(u32::try_from(row.try_get::<i32, _>("scenario_version")?).unwrap_or(0)),
                revision: Revision(u32::try_from(row.try_get::<i32, _>("revision")?).unwrap_or(0)),
            },
            variant: from_json(row.try_get("variant")?)?,
            seed: Seed(u64::try_from(row.try_get::<i64, _>("seed")?).unwrap_or(0)),
            duration_sim_ms: SimDuration::ms(u64::try_from(row.try_get::<i64, _>("duration_sim_ms")?).unwrap_or(0)),
            report_window: from_json(row.try_get("report_window")?)?,
            runner: PinnedImage { image: row.try_get("runner_image")? },
            expected: from_json(row.try_get("expected")?)?,
            // world-proto-server as a runner reaches it, which from a container or a pod is not the API's loopback.
            wps: Endpoint { endpoint: app.runs.wps.to_string() },
        })
    };
    read().map(Json).map_err(fault)
}

/// `GET /runner/v1/attempts/{run}/{n}/control`: a cancellation is the attempt row's request, never a process signal.
async fn control(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
) -> Result<Json<AttemptControl>, ProblemAnswer> {
    let instance = uri.path();
    let key = own(&app, &headers, path, instance).await?;
    let cancel_requested =
        sqlx::query_scalar("SELECT cancel_requested_at IS NOT NULL FROM attempts WHERE run_id = $1 AND n = $2")
            .bind(key.run.as_str())
            .bind(i32::try_from(key.n.0).unwrap_or(i32::MAX))
            .fetch_one(&app.pool)
            .await
            .map_err(|fault| internal("read the attempt's control", fault, instance))?;
    Ok(Json(AttemptControl { cancel_requested }))
}

/// `PUT /runner/v1/attempts/{run}/{n}/manifest`: kept with the attempt, compared with the check, each finding
/// journaled once; the same bytes again answer the first outcome, other bytes `manifest-differs`.
async fn manifest(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
    body: Bytes,
) -> Result<Json<ManifestAccepted>, ProblemAnswer> {
    let instance = uri.path();
    let key = own(&app, &headers, path, instance).await?;
    let manifest: Manifest = json_body(&body, instance)?;
    if manifest.run != key.run || manifest.attempt != key.n {
        let detail = "The manifest names another attempt than the route.";
        return Err(Problem::new(ProblemType::ValidationFailed, detail).at(instance).into());
    }
    let digest = format!("{:x}", Sha256::digest(&body));
    let fault = |fault: &dyn Display| internal("keep the manifest", fault, instance);
    let mut tx = app.pool.begin().await.map_err(|f| fault(&f))?;
    let row = sqlx::query(
        "SELECT a.manifest_sha256, a.findings, r.expected, r.reads, r.cut, r.submission, r.position, \
             (SELECT count(*) FROM runs o WHERE o.submission = r.submission) AS of, sc.site_scope, s.submitted_by \
         FROM attempts a JOIN runs r ON r.id = a.run_id JOIN submissions s ON s.id = r.submission \
         JOIN scenarios sc ON sc.id = r.scenario WHERE a.run_id = $1 AND a.n = $2 FOR UPDATE OF a",
    )
    .bind(key.run.as_str())
    .bind(i32::try_from(key.n.0).unwrap_or(i32::MAX))
    .fetch_one(&mut *tx)
    .await
    .map_err(|f| fault(&f))?;
    let held: Option<String> = row.try_get("manifest_sha256").map_err(|f| fault(&f))?;
    if let Some(held) = held {
        if held != digest {
            let detail = "This attempt handed in another manifest already; a manifest is handed in once.";
            return Err(Problem::new(ProblemType::ManifestDiffers, detail).at(instance).into());
        }
        let found = from_json(row.try_get("findings").map_err(|f| fault(&f))?).map_err(|f| fault(&f))?;
        return Ok(Json(ManifestAccepted { findings: found }));
    }
    let expected: Expectation = from_json(row.try_get("expected").map_err(|f| fault(&f))?).map_err(|f| fault(&f))?;
    let reads: BTreeMap<ScopePath, Seq> =
        from_json(row.try_get("reads").map_err(|f| fault(&f))?).map_err(|f| fault(&f))?;
    let cut: UnitCut = from_json(row.try_get("cut").map_err(|f| fault(&f))?).map_err(|f| fault(&f))?;
    let checked = Checked { expected: &expected, reads: &reads, exact: cut.exact };
    let found = findings(&key.run, key.n, &checked, &manifest);
    let object = format!(
        "{}manifest.json",
        attempt_prefix(
            &RunId::new(key.run.as_str()).map_err(|f| fault(&f))?,
            StoredAttempt::new(key.n.0).map_err(|f| fault(&f))?
        )
    );
    app.runstore.put(&object, body.clone()).await.map_err(|f| fault(&f))?;
    sqlx::query(
        "UPDATE attempts SET manifest_key = $3, manifest_sha256 = $4, compile_sha256 = $5, findings = $6, \
             started_at = CASE WHEN state = 'queued' THEN now() ELSE started_at END, \
             state = CASE WHEN state = 'queued' THEN 'running' ELSE state END \
         WHERE run_id = $1 AND n = $2",
    )
    .bind(key.run.as_str())
    .bind(i32::try_from(key.n.0).unwrap_or(i32::MAX))
    .bind(&object)
    .bind(&digest)
    .bind(&manifest.compile.digest)
    .bind(json!(found))
    .execute(&mut *tx)
    .await
    .map_err(|f| fault(&f))?;
    let site: SitePath =
        row.try_get::<String, _>("site_scope").map_err(|f| fault(&f))?.parse().map_err(|f| fault(&f))?;
    let opener: Subject =
        row.try_get::<String, _>("submitted_by").map_err(|f| fault(&f))?.parse().map_err(|f| fault(&f))?;
    let submission: String = row.try_get("submission").map_err(|f| fault(&f))?;
    let position: i32 = row.try_get("position").map_err(|f| fault(&f))?;
    let of: i64 = row.try_get("of").map_err(|f| fault(&f))?;
    let mut written = Vec::new();
    for finding in &found {
        let body = json!({
            "run": key.run, "submission": submission, "position": position, "of": of, "attempt": key.n,
            "finding": finding,
        });
        let mut entry = NewEntry::new(JournalKind::RunFinding, body);
        let kind = word(finding.kind);
        entry.dedup_key = Some(format!("run.finding:{}:{}:{kind}", key.run, key.n.0));
        written.extend(journal_on_site(&app, &mut tx, &site, &opener, entry).await.map_err(|f| fault(&f))?);
    }
    tx.commit().await.map_err(|f| fault(&f))?;
    app.hub.announce(written);
    Ok(Json(ManifestAccepted { findings: found }))
}

/// `GET /runner/v1/scenarios/{s}/versions/{v}`: the sealed version the token's run reads; any other, an open one
/// included, is `not-this-run`.
async fn version(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
) -> Result<Json<SealedVersion>, ProblemAnswer> {
    let instance = uri.path();
    let key = bearer(&app, &headers, instance).await?;
    let not_this = || -> ProblemAnswer {
        let detail = "This token's run reads another scenario version, or this one is not sealed.";
        Problem::new(ProblemType::NotThisRun, detail).at(instance).into()
    };
    let named = path.ok().and_then(|Path((scenario, n))| scenario.parse::<Ulid>().ok().map(|scenario| (scenario, n)));
    let Some((scenario, n)) = named else { return Err(not_this()) };
    let fault = |fault| internal("read the scenario version", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    let pinned: (String, i32) = sqlx::query_as("SELECT scenario, scenario_version FROM runs WHERE id = $1")
        .bind(key.run.as_str())
        .fetch_one(&mut *conn)
        .await
        .map_err(fault)?;
    if pinned.0 != scenario.as_str() || u32::try_from(pinned.1).ok() != Some(n) {
        return Err(not_this());
    }
    let read = sealed_version(&mut conn, &scenario, VersionNumber(n)).await.map_err(fault)?;
    read.map(|version| Json(SealedVersion { scenario, version })).ok_or_else(not_this)
}

/// `POST /runner/v1/workloads/generate`: a workload spec in, the generator's NDJSON out as the compute pool makes it
/// (spec 07 section 3.8), the same bytes for the same body from every build of the crate's version; nothing is stored
/// (ruling D96). The lines go through a bounded channel into the answer's body, so the generator waits on a slow
/// reader rather than holding the stream, and stops once the reader has gone.
async fn generate(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    bearer(&app, &headers, instance).await?;
    let request: GenerateRequest = json_body(&body, instance)?;
    let found = check_request(&request);
    if !found.is_empty() {
        return Err(refused(&found, instance));
    }
    let (sender, receiver) = channel::<Result<Bytes, Infallible>>(CHUNKS_WAITING);
    let made = app.compute.submit(Box::new(move || {
        let lines = WorkloadLines::new(&request.generator, request.duration_sim_ms, facts_of(&request));
        let mut chunk = Vec::with_capacity(CHUNK_BYTES);
        for line in lines {
            ndjson_line(&line, &mut chunk);
            if chunk.len() >= CHUNK_BYTES && sender.blocking_send(Ok(Bytes::from(take(&mut chunk)))).is_err() {
                return;
            }
        }
        if !chunk.is_empty() {
            let _ = sender.blocking_send(Ok(Bytes::from(chunk)));
        }
    }));
    made.map_err(|busy| wps_problem(&WpsError::ComputeBusy { retry_after: busy.retry_after }, instance))?;
    let mut answer = Body::from_stream(ReceiverStream::new(receiver)).into_response();
    answer.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/x-ndjson"));
    Ok(answer)
}

/// The generator's refusal of a body, each violation an item naming its field.
fn refused(found: &[Violation], instance: &str) -> ProblemAnswer {
    let items: Vec<ProblemItem> = found
        .iter()
        .map(|violation| ProblemItem {
            code: "validation-failed".to_owned(),
            severity: Severity::Error,
            ref_: None,
            scope: None,
            field: Some(violation.pointer.clone()),
            knob: None,
            text: format!("{} {}.", violation.pointer, violation.rule),
        })
        .collect();
    let detail = items.iter().map(|item| item.text.as_str()).collect::<Vec<_>>().join(" ");
    let mut problem = Problem::new(ProblemType::ValidationFailed, detail).at(instance);
    problem.problems = items;
    problem.into()
}
