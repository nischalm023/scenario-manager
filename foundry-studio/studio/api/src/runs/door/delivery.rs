//! Within `door`, `delivery` is what a runner sends the API about its own run, beside what it reads to prepare it: its
//! heartbeat every 5 s of wall clock while it runs and, once the run is over, its recording, which the API takes into
//! the run store so no runner holds a credential for the store (ruling D174), then its results, written as the
//! attempt's ending. Each opens with the attempt's own token, as every door route does; what is taken, and when, is
//! the store's to say (`store`), and these routes answer it.

use std::env::temp_dir;
use std::fmt::Display;
use std::fs::remove_file;
use std::io::ErrorKind;
use std::path::PathBuf;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use futures::StreamExt;
use run_invocation::{AttemptKey, Ended};
use serde_json::json;
use studio_protocol::{
    ByteCount, Heartbeat, Problem, ProblemType, RecordingStored, ResultsAccepted, ResultsEnded, ResultsReport,
};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::AppState;
use crate::db::mint_ulid;
use crate::problem::{ProblemAnswer, internal, json_body};

use super::super::store::{PgRunStore, Recording, RecordingFault, Results, ResultsFault, ResultsSource};
use super::own;

/// The largest results report: a long run's series file passes the listener's 1 MiB; the recording never comes here.
pub(super) const REPORT_LIMIT: usize = 32 << 20;

/// The largest recording a runner may send, which waits on this process's disk until it is stored: a 16-hour run of
/// the DFW6 site records 669 MB.
const RECORDING_LIMIT: u64 = 8 << 30;

/// `PUT /runner/v1/attempts/{run}/{n}/heartbeat`: the runner is alive and this far; kept on the row with the instant it
/// came, while the attempt is queued or running, `attempt-ended` after. Simtime only moves forward.
pub(super) async fn heartbeat(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
    body: Bytes,
) -> Result<StatusCode, ProblemAnswer> {
    let instance = uri.path();
    let key = own(&app, &headers, path, instance).await?;
    let beat: Heartbeat = json_body(&body, instance)?;
    let kept = sqlx::query(
        "UPDATE attempts SET heartbeat_at = now(), progress_sim_ms = GREATEST(progress_sim_ms, $3), \
             progress_values = $4 \
         WHERE run_id = $1 AND n = $2 AND state IN ('queued', 'running')",
    )
    .bind(key.run.as_str())
    .bind(i32::try_from(key.n.0).unwrap_or(i32::MAX))
    .bind(i64::try_from(beat.sim_ms.as_ms()).unwrap_or(i64::MAX))
    .bind(json!(beat.values))
    .execute(&app.pool)
    .await
    .map_err(|fault| internal("keep the heartbeat", fault, instance))?;
    if kept.rows_affected() == 0 {
        return Err(ended("This attempt has ended; its heartbeat is not kept.", instance));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /runner/v1/attempts/{run}/{n}/recording`: the runner's recording once its run is over, taken by the API as the
/// attempt's (`store::recording`). The body goes to a file of this process's own as it comes, at most
/// `RECORDING_LIMIT`, and is taken from there; the answer is the receipt the runner's report then names. A body that is
/// no whole recording is `validation-failed`, a recording after the attempt's end `attempt-ended`, other bytes than the
/// attempt took first `recording-differs`; this process's own disk or the run store failing is `internal`, which the
/// runner asks again.
pub(super) async fn recording(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
    body: Body,
) -> Result<Json<RecordingStored>, ProblemAnswer> {
    let instance = uri.path();
    let key = own(&app, &headers, path, instance).await?;
    let fault = |fault: &dyn Display| internal("store the runner's recording", fault, instance);
    let refused =
        |detail: String| -> ProblemAnswer { Problem::new(ProblemType::ValidationFailed, detail).at(instance).into() };
    let store = PgRunStore::new(app.clone());
    // A cheap refusal before the body is read; the one that holds is the store's, where the recording is accepted.
    if !store.may_take_recording(&key).await.map_err(|f| fault(&f))? {
        return Err(ended("This attempt has ended; its recording is not kept.", instance));
    }
    let upload = Upload::of(&key);
    let mut file = File::create(&upload.0).await.map_err(|f| fault(&f))?;
    let (mut chunks, mut received) = (body.into_data_stream(), 0_u64);
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|f| refused(format!("The recording did not arrive whole: {f}")))?;
        received += chunk.len() as u64;
        if received > RECORDING_LIMIT {
            return Err(refused(format!(
                "The recording passes {} GiB, more than any run records.",
                RECORDING_LIMIT >> 30
            )));
        }
        file.write_all(&chunk).await.map_err(|f| fault(&f))?;
    }
    file.flush().await.map_err(|f| fault(&f))?;
    drop(file);
    let stored = match store.take_recording(&key, &upload.0).await {
        Ok(stored) => stored,
        Err(RecordingFault::AttemptEnded) => {
            return Err(ended("This attempt has ended; its recording is not kept.", instance));
        }
        Err(RecordingFault::Differs) => {
            let detail = "This attempt took a recording of other bytes already, which stands; this one is not kept.";
            return Err(Problem::new(ProblemType::RecordingDiffers, detail).at(instance).into());
        }
        Err(RecordingFault::Store(failed)) => return Err(fault(&failed)),
        Err(bad) => return Err(refused(format!("The body {bad}; nothing of it is kept."))),
    };
    Ok(Json(RecordingStored {
        key: stored.key,
        live_edge_sim_ms: stored.live_edge,
        frames: stored.frames,
        bytes: ByteCount(stored.bytes),
    }))
}

/// `PUT /runner/v1/attempts/{run}/{n}/results`: the runner's own delivery of what it left, its recording sent first
/// through the recording route, written through the store as the attempt's ending; the first ending wins, and a
/// report after it answers `already-ended` and changes nothing. A report naming a recording the attempt did not accept
/// is `validation-failed`.
pub(super) async fn results(
    State(app): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    path: Result<Path<(String, u32)>, PathRejection>,
    body: Bytes,
) -> Result<Json<ResultsAccepted>, ProblemAnswer> {
    let instance = uri.path();
    let key = own(&app, &headers, path, instance).await?;
    let report: ResultsReport = json_body(&body, instance)?;
    if !report.ending.state.ended() {
        let detail = "The report's ending is not an ending: its state is still queued or running.";
        return Err(Problem::new(ProblemType::ValidationFailed, detail).at(instance).into());
    }
    let recording = report.recording.map(|named| Recording::Stored {
        key: named.key,
        live_edge: named.live_edge_sim_ms,
        frames: named.frames,
        bytes: named.bytes.0,
    });
    let results = Results {
        ending: report.ending,
        metrics: report.metrics,
        series: report.series,
        recording,
        runner_commit: report.runner_commit,
        runner_dirty: report.runner_dirty,
        source: ResultsSource::Report,
    };
    let ended = match PgRunStore::new(app.clone()).take_results(&key, results).await {
        Ok(ended) => ended,
        Err(ResultsFault::NotAccepted) => {
            let detail = "The report names a recording this attempt did not accept through its recording route.";
            return Err(Problem::new(ProblemType::ValidationFailed, detail).at(instance).into());
        }
        // A report names its recording by its receipt and brings no file, so no folder's recording can be refused here.
        Err(unwhole @ ResultsFault::LeftUnwhole(_)) => {
            return Err(internal("store the runner's results", unwhole, instance));
        }
        Err(ResultsFault::Store(failed)) => return Err(internal("store the runner's results", failed, instance)),
    };
    app.runs.wakes.reconcile();
    let ended = match ended {
        Ended::Wrote => ResultsEnded::Wrote,
        Ended::AlreadyEnded => ResultsEnded::AlreadyEnded,
    };
    Ok(Json(ResultsAccepted { ended }))
}

fn ended(detail: &str, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::AttemptEnded, detail).at(instance).into()
}

/// Where a recording waits to be stored: a file of this process's own, removed however the route answers.
struct Upload(PathBuf);

impl Upload {
    fn of(key: &AttemptKey) -> Self {
        Self(temp_dir().join(format!("studio-recording-{}-{}-{}.worldview", key.run, key.n.0, mint_ulid())))
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        if let Err(fault) = remove_file(&self.0)
            && fault.kind() != ErrorKind::NotFound
        {
            eprintln!("studio-api: {} was not removed: {fault}", self.0.display());
        }
    }
}
