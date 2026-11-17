//! Within `results`, `stored` reads what one attempt of a run left, as the result routes need it: the run's report
//! window and project, the attempt's state and how far it got, the metrics it declared in descriptor order (the
//! sidecar's rows) and each one's stored series. Callers hold the connection; a value a column promises but does
//! not hold is a decode fault.

use desunits::SimTime;
use rustc_hash::FxHashMap;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{AttemptNumber, ReportWindow, SeriesPoint, Ulid};

use crate::db::{decode, read_word};
use crate::runs::{json, milliseconds};

use super::values::{Declared, Instants, Stored};

/// A run's row as the result routes read it, with its scenario's project and site.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub project: Ulid,
    pub site: String,
    pub report_window: ReportWindow,
}

/// One attempt and what it left: how far it got in simtime, its recording's key, and its rows.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub n: AttemptNumber,
    pub reached: SimTime,
    pub recording: Option<String>,
    pub declared: Vec<Declared>,
    pub stored: FxHashMap<String, Stored>,
}

impl Attempt {
    pub fn instants(&self) -> Instants {
        Instants::new(self.stored.values(), self.reached)
    }
}

pub async fn run_row(conn: &mut PgConnection, run: &Ulid) -> Result<Option<RunRow>, SqlxError> {
    let row = sqlx::query(
        "SELECT r.report_window, sc.project, sc.site_scope FROM runs r JOIN scenarios sc ON sc.id = r.scenario \
         WHERE r.id = $1",
    )
    .bind(run.as_str())
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(RunRow {
        project: row.try_get::<String, _>("project")?.parse().map_err(decode)?,
        site: row.try_get("site_scope")?,
        report_window: json(row.try_get("report_window")?)?,
    }))
}

/// Attempt `n` of `run`, the latest when `n` is none; none when the run has no such attempt.
pub async fn attempt(
    conn: &mut PgConnection,
    run: &Ulid,
    n: Option<AttemptNumber>,
) -> Result<Option<Attempt>, SqlxError> {
    let row = sqlx::query(
        "SELECT n, progress_sim_ms, object_key FROM attempts WHERE run_id = $1 AND ($2::int IS NULL OR n = $2) \
         ORDER BY n DESC LIMIT 1",
    )
    .bind(run.as_str())
    .bind(n.map(|n| i32::try_from(n.0).unwrap_or(i32::MAX)))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let n: i32 = row.try_get("n")?;
    let declared = declared(conn, run, n).await?;
    let stored = series(conn, run, n, None).await?;
    Ok(Some(Attempt {
        n: AttemptNumber(u32::try_from(n).map_err(decode)?),
        reached: SimTime::ms(milliseconds(row.try_get("progress_sim_ms")?)?),
        recording: row.try_get("object_key")?,
        declared,
        stored,
    }))
}

/// Every metric attempt `n` of `run` declared, in descriptor order.
pub async fn declared(conn: &mut PgConnection, run: &Ulid, n: i32) -> Result<Vec<Declared>, SqlxError> {
    let rows = sqlx::query(
        "SELECT metric, label, description, unit, clock, aggregation, denominator, value, basis, whole_run \
         FROM attempt_metrics WHERE run_id = $1 AND n = $2 ORDER BY position",
    )
    .bind(run.as_str())
    .bind(n)
    .fetch_all(conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Declared {
                metric: row.try_get("metric")?,
                label: row.try_get("label")?,
                description: row.try_get("description")?,
                unit: read_word(&row.try_get::<String, _>("unit")?)?,
                clock: row.try_get::<Option<String>, _>("clock")?.as_deref().map(read_word).transpose()?,
                aggregation: read_word(&row.try_get::<String, _>("aggregation")?)?,
                denominator: row.try_get("denominator")?,
                value: row.try_get("value")?,
                basis: read_word(&row.try_get::<String, _>("basis")?)?,
                whole_run: row.try_get("whole_run")?,
            })
        })
        .collect()
}

/// The stored series of attempt `n` of `run`, by metric: every one, or only `only`'s.
pub async fn series(
    conn: &mut PgConnection,
    run: &Ulid,
    n: i32,
    only: Option<&str>,
) -> Result<FxHashMap<String, Stored>, SqlxError> {
    let rows = sqlx::query(
        "SELECT metric, kind, at_sim_ms, value FROM attempt_series WHERE run_id = $1 AND n = $2 \
         AND ($3::text IS NULL OR metric = $3)",
    )
    .bind(run.as_str())
    .bind(n)
    .bind(only)
    .fetch_all(conn)
    .await?;
    rows.iter()
        .map(|row| {
            let at: Vec<i64> = row.try_get("at_sim_ms")?;
            let value: Vec<Option<f64>> = row.try_get("value")?;
            let points = at
                .into_iter()
                .zip(value)
                .map(|(at, value)| Ok(SeriesPoint(SimTime::ms(milliseconds(at)?), value)))
                .collect::<Result<Vec<_>, SqlxError>>()?;
            let stored = Stored { kind: read_word(&row.try_get::<String, _>("kind")?)?, points };
            Ok((row.try_get("metric")?, stored))
        })
        .collect()
}
