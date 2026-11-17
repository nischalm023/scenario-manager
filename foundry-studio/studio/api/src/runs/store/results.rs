//! Within `store`, `results` is the Worldview service's part, which the API does in the first cut: an attempt's
//! results, its ending with its metrics, series and recording, written as the attempt's ending whoever brings them, its
//! runner's own report over the door or the local invoker's settle from its folder, so that the first ending wins with
//! what it brought and the attempt's row names only the recording the attempt accepted.

use std::path::PathBuf;

use desunits::SimTime;
use run_invocation::{AttemptKey, Ended, Fault};
use sqlx::{Error as SqlxError, PgConnection};
use studio_protocol::{Ending, MetricsFile, SeriesFile};
use thiserror::Error;

use crate::db::word;

use super::recording::{self, RecordingFault, StoredRecording};
use super::{PgRunStore, end, n_of};

/// What an attempt's runner left: its ending, its two files and its recording, who it says it is, and how they came.
/// The runner delivers them itself over the door once the run is over; the local invoker's settle also reads them off
/// the runner's folder when it left no report.
#[derive(Debug, Clone, PartialEq)]
pub struct Results {
    pub ending: Ending,
    pub metrics: Option<MetricsFile>,
    pub series: Option<SeriesFile>,
    pub recording: Option<Recording>,
    pub runner_commit: Option<String>,
    pub runner_dirty: Option<bool>,
    pub source: ResultsSource,
}

/// How an attempt's results reached the API: its runner's own report over the door, after which the runner exits by
/// itself, or its folder, read once its container exited without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsSource {
    Report,
    Folder,
}

/// An attempt's recording: a file on this machine's disk, written by a local runner, still to be taken as the
/// attempt's; or the one its runner sent the API already, as the API's receipt for it named it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recording {
    File { path: PathBuf },
    Stored { key: String, live_edge: Option<SimTime>, frames: u64, bytes: u64 },
}

/// Why a runner's results were not taken: a report names a recording the attempt did not accept, or the recording a
/// runner left in its folder is no whole one, neither of which a retry mends; or the store failed, which a retry may.
#[derive(Debug, Error)]
pub enum ResultsFault {
    #[error("the report names a recording this attempt did not accept through its recording route")]
    NotAccepted,
    /// The folder's recording stops before its trailer or does not read as one.
    #[error("the runner's recording.worldview {0}")]
    LeftUnwhole(RecordingFault),
    #[error("the results could not be stored: {0}")]
    Store(Fault),
}

impl From<SqlxError> for ResultsFault {
    fn from(fault: SqlxError) -> Self {
        Self::Store(fault.into())
    }
}

impl PgRunStore {
    /// The guarded ending with what the runner left. A recording left as a file is taken as the attempt's first, and
    /// read only while the attempt still awaits one; one the runner sent already must be the one the attempt accepted,
    /// as its receipt names it. Whoever ends the attempt, its accepted recording stays on its row; a report marks when
    /// it ended it.
    pub async fn take_results(&self, key: &AttemptKey, results: Results) -> Result<Ended, ResultsFault> {
        if let Some(Recording::File { path }) = &results.recording
            && self.awaits_recording(key).await?
        {
            match self.take_recording(key, path).await {
                // An attempt that ended meanwhile keeps the recording it has, if any; the ending below then answers
                // that it ended.
                Ok(_) | Err(RecordingFault::AttemptEnded) => {}
                Err(RecordingFault::Differs) => {
                    eprintln!("studio-api: {key}'s runner left a recording other than the one it sent, which stands");
                }
                Err(RecordingFault::Store(fault)) => return Err(ResultsFault::Store(fault)),
                Err(unwhole @ (RecordingFault::Unclosed | RecordingFault::Unreadable(_))) => {
                    return Err(ResultsFault::LeftUnwhole(unwhole));
                }
            }
        }
        let mut tx = self.app.pool.begin().await?;
        if let Some(Recording::Stored { key: named, live_edge, frames, bytes }) = &results.recording {
            let named = StoredRecording { key: named.clone(), live_edge: *live_edge, frames: *frames, bytes: *bytes };
            if recording::accepted(&mut tx, key).await? != Some(named) {
                return Err(ResultsFault::NotAccepted);
            }
        }
        if end(&mut tx, key, &results.ending, None).await? == Ended::AlreadyEnded {
            return Ok(Ended::AlreadyEnded);
        }
        sqlx::query(
            "UPDATE attempts SET runner_commit = $3, runner_dirty = $4, reported_at = CASE WHEN $5 THEN now() END \
             WHERE run_id = $1 AND n = $2",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .bind(&results.runner_commit)
        .bind(results.runner_dirty)
        .bind(results.source == ResultsSource::Report)
        .execute(&mut *tx)
        .await?;
        if let Some(metrics) = &results.metrics {
            insert_metrics(&mut tx, key, metrics).await?;
            if let Some(series) = &results.series {
                insert_series(&mut tx, key, series).await?;
            }
        }
        tx.commit().await?;
        Ok(Ended::Wrote)
    }
}

async fn insert_metrics(conn: &mut PgConnection, key: &AttemptKey, file: &MetricsFile) -> Result<(), SqlxError> {
    for (position, metric) in file.metrics.iter().enumerate() {
        sqlx::query(
            "INSERT INTO attempt_metrics (run_id, n, position, metric, label, description, unit, clock, \
                 aggregation, denominator, value, basis, whole_run) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .bind(i32::try_from(position + 1).unwrap_or(i32::MAX))
        .bind(&metric.metric)
        .bind(&metric.label)
        .bind(&metric.description)
        .bind(word(metric.unit))
        .bind(metric.clock.map(word))
        .bind(word(metric.aggregation))
        .bind(&metric.denominator)
        .bind(metric.value)
        .bind(word(file.basis))
        .bind(metric.whole_run)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn insert_series(conn: &mut PgConnection, key: &AttemptKey, file: &SeriesFile) -> Result<(), SqlxError> {
    for series in &file.metrics {
        let at: Vec<i64> =
            series.points.iter().map(|point| i64::try_from(point.0.as_ms()).unwrap_or(i64::MAX)).collect();
        let value: Vec<Option<f64>> = series.points.iter().map(|point| point.1).collect();
        sqlx::query(
            "INSERT INTO attempt_series (run_id, n, metric, kind, at_sim_ms, value) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .bind(&series.metric)
        .bind(word(series.kind))
        .bind(at)
        .bind(value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
