//! Within `runs`, `rows` reads a run as the Runs panel and the run tools see it (spec 08 section 3.7's `RunSummary`
//! and `RunDetail`): its row with its submission's size, its scenario's name and site, and its latest attempt, whose
//! state is the run's; a waiting attempt's place in the queue; and every attempt of one run. Callers hold the
//! connection; a value a column promises but does not hold is a decode fault.

use std::error::Error;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use desunits::{SimDuration, SimTime};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{
    AttemptNumber, AttemptRead, EndedBy, Finding, ObjectKey, ObjectKind, Patch, ProgressValue, RecordingRead,
    ReportWindow, RunCount, RunPosition, RunProgressRead, RunScenario, RunState, RunSummary, Seed, Ulid, VariedValue,
    VersionNumber, Waiting, WaitingFor, WallClock, WallMs,
};

use crate::db::{decode, read_word, word};

/// A run with its latest attempt; the filter goes after it on `r`, `s`, `sc` or `l`, and the order newest submission
/// first, then position.
const SUMMARY: &str = "WITH waiting AS ( \
         SELECT a.run_id, a.n, row_number() OVER (ORDER BY a.requested_at, r.position, a.n) AS place \
         FROM attempts a JOIN runs r ON r.id = a.run_id WHERE a.state = 'queued' AND a.job IS NULL) \
     SELECT r.id, r.submission, r.position, r.variant, r.seed, r.duration_sim_ms, r.report_window, r.scenario, \
         r.scenario_version, sc.name AS scenario_name, sc.site_scope, s.submitted_at, \
         (SELECT count(*) FROM runs o WHERE o.submission = r.submission) AS of, \
         (SELECT count(*) FROM runs t WHERE t.submission = r.submission AND t.variant = r.variant) > 1 AS repeated, \
         l.n, l.state, l.job, l.started_at, l.ended_at, l.ended_by, l.stop_reason, l.stop_detail, l.progress_sim_ms, \
         l.progress_values, l.object_key, l.recording_live_edge_ms, l.findings, w.place \
     FROM runs r JOIN submissions s ON s.id = r.submission JOIN scenarios sc ON sc.id = r.scenario \
     JOIN LATERAL (SELECT * FROM attempts a WHERE a.run_id = r.id ORDER BY a.n DESC LIMIT 1) l ON true \
     LEFT JOIN waiting w ON w.run_id = l.run_id AND w.n = l.n";

/// A running attempt whose runner was last heard longer ago than this (wall clock) reads as stalled: six missed beats
/// of 5 s.
pub const STALLED_AFTER: Duration = Duration::from_secs(30);

/// Whether an attempt of `attempts` reads as stalled, as SQL whose parameter `seconds` binds `STALLED_AFTER`: running,
/// and its runner not heard for that long, since its latest heartbeat or, before its first, since the manifest's
/// arrival that made it running, so a runner whose beats never land stalls too.
pub fn stalled(seconds: &str) -> String {
    format!("(state = 'running' AND COALESCE(heartbeat_at, started_at) < now() - make_interval(secs => {seconds}))")
}

/// The list's order, which a page's cursor continues: by (submitted_at, submission, -position), descending.
const ORDER: &str = "ORDER BY s.submitted_at DESC, r.submission DESC, r.position";

/// What a list may be narrowed to, and the run the page starts after in the list's order.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub site: Option<String>,
    pub submitted_by: Option<String>,
    pub scenario: Option<Ulid>,
    pub submission: Option<Ulid>,
    pub state: Option<RunState>,
    pub runs: Option<Vec<Ulid>>,
    pub after: Option<Ulid>,
    pub limit: i64,
}

/// The runs `filter` names, newest submission first.
pub async fn summaries(conn: &mut PgConnection, filter: &Filter) -> Result<Vec<RunSummary>, SqlxError> {
    let sql = format!(
        "{SUMMARY} WHERE ($1::text IS NULL OR sc.site_scope = $1) AND ($2::text IS NULL OR s.submitted_by = $2) \
         AND ($3::text IS NULL OR r.scenario = $3) AND ($4::text IS NULL OR r.submission = $4) \
         AND ($5::text IS NULL OR l.state = $5) AND ($6::text[] IS NULL OR r.id = ANY($6)) \
         AND ($8::text IS NULL OR (s.submitted_at, r.submission, -r.position) < \
             (SELECT a.submitted_at, b.submission, -b.position FROM runs b JOIN submissions a ON a.id = b.submission \
              WHERE b.id = $8)) \
         {ORDER} LIMIT $7"
    );
    let runs: Option<Vec<String>> = filter.runs.as_ref().map(|runs| runs.iter().map(|run| run.to_string()).collect());
    let rows = sqlx::query(&sql)
        .bind(filter.site.as_deref())
        .bind(filter.submitted_by.as_deref())
        .bind(filter.scenario.as_ref().map(Ulid::as_str))
        .bind(filter.submission.as_ref().map(Ulid::as_str))
        .bind(filter.state.map(word))
        .bind(runs)
        .bind(filter.limit)
        .bind(filter.after.as_ref().map(Ulid::as_str))
        .fetch_all(conn)
        .await?;
    rows.iter().map(summary).collect()
}

/// One run's summary; `None` when no run has that id.
pub async fn summary_of(conn: &mut PgConnection, run: &Ulid) -> Result<Option<RunSummary>, SqlxError> {
    let filter = Filter { runs: Some(vec![run.clone()]), limit: 1, ..Filter::default() };
    Ok(summaries(conn, &filter).await?.into_iter().next())
}

fn summary(row: &PgRow) -> Result<RunSummary, SqlxError> {
    let id: Ulid = parse(row.try_get::<String, _>("id")?)?;
    let name: String = row.try_get("scenario_name")?;
    let patches: Vec<Patch> = json(row.try_get("variant")?)?;
    let seed = Seed(u64::try_from(row.try_get::<i64, _>("seed")?).map_err(decode)?);
    let variant: Vec<VariedValue> =
        patches.iter().map(|patch| VariedValue { label: patch.label.clone(), value: shown(&patch.value) }).collect();
    let mut label = name.clone();
    for varied in &variant {
        label.push_str(&format!(", {} {}", varied.value, varied.label.to_lowercase()));
    }
    if row.try_get::<bool, _>("repeated")? {
        label.push_str(&format!(", seed {}", seed.0));
    }
    let state: RunState = read_word(&row.try_get::<String, _>("state")?)?;
    let waiting = (state == RunState::Queued).then(|| -> Result<Waiting, SqlxError> {
        let place: Option<i64> = row.try_get("place")?;
        Ok(match place {
            Some(place) => Waiting { for_: WaitingFor::Seat, place_in_queue: Some(RunPosition(count(place)?)) },
            None => Waiting { for_: WaitingFor::Runner, place_in_queue: None },
        })
    });
    let at = |column: &str| row.try_get::<Option<DateTime<Utc>>, _>(column).map(|at| at.map(WallClock::from));
    let (started_at, ended_at) = (at("started_at")?, at("ended_at")?);
    let elapsed = started_at.map(|started| {
        let until = ended_at.unwrap_or_else(WallClock::now);
        WallMs(u64::try_from((until.instant() - started.instant()).num_milliseconds()).unwrap_or(0))
    });
    let findings: Vec<Finding> = json(row.try_get("findings")?)?;
    let progress_values: Vec<ProgressValue> = json(row.try_get("progress_values")?)?;
    Ok(RunSummary {
        ref_: ObjectKey::new(ObjectKind::Run, id.clone()).to_string(),
        id,
        submission: parse(row.try_get::<String, _>("submission")?)?,
        position: RunPosition(count(i64::from(row.try_get::<i32, _>("position")?))?),
        of: RunCount(count(row.try_get::<i64, _>("of")?)?),
        label,
        scenario: RunScenario {
            id: parse(row.try_get::<String, _>("scenario")?)?,
            version: VersionNumber(count(i64::from(row.try_get::<i32, _>("scenario_version")?))?),
            name,
        },
        site: row.try_get("site_scope")?,
        variant,
        seed,
        duration_sim_ms: SimDuration::ms(milliseconds(row.try_get("duration_sim_ms")?)?),
        report_window: json::<ReportWindow>(row.try_get("report_window")?)?,
        state,
        attempt: AttemptNumber(count(i64::from(row.try_get::<i32, _>("n")?))?),
        waiting: waiting.transpose()?,
        progress: RunProgressRead {
            sim_ms: SimTime::ms(milliseconds(row.try_get("progress_sim_ms")?)?),
            values: progress_values,
        },
        started_at,
        ended_at,
        elapsed_wall_ms: elapsed,
        ended_by: row.try_get::<Option<String>, _>("ended_by")?.map(|by| read_word::<EndedBy>(&by)).transpose()?,
        stop_reason: row.try_get("stop_reason")?,
        stop_detail: row.try_get("stop_detail")?,
        recording: RecordingRead {
            stored: row.try_get::<Option<String>, _>("object_key")?.is_some(),
            live_edge_sim_ms: row
                .try_get::<Option<i64>, _>("recording_live_edge_ms")?
                .map(milliseconds)
                .transpose()?
                .map(SimTime::ms),
        },
        headline: None,
        findings: findings.iter().map(|finding| finding.kind).collect(),
    })
}

/// Every attempt of `run`, oldest first, as the detail shows it before its manifest's facts are read.
pub async fn attempts(conn: &mut PgConnection, run: &Ulid) -> Result<Vec<(AttemptRead, Option<String>)>, SqlxError> {
    let query = format!(
        "SELECT n, state, requested_at, dispatched_at, started_at, ended_at, job, job_stopped_at, streaming_server, \
             runner_commit, runner_dirty, ended_by, stop_reason, stop_detail, findings, manifest_key, heartbeat_at, \
             platform_status, {} IS TRUE AS stalled \
         FROM attempts WHERE run_id = $1 ORDER BY n",
        stalled("$2"),
    );
    let rows = sqlx::query(&query).bind(run.as_str()).bind(STALLED_AFTER.as_secs_f64()).fetch_all(conn).await?;
    rows.iter()
        .map(|row| {
            let at = |column: &str| row.try_get::<Option<DateTime<Utc>>, _>(column).map(|at| at.map(WallClock::from));
            let read = AttemptRead {
                n: AttemptNumber(count(i64::from(row.try_get::<i32, _>("n")?))?),
                state: read_word(&row.try_get::<String, _>("state")?)?,
                requested_at: WallClock::from(row.try_get::<DateTime<Utc>, _>("requested_at")?),
                dispatched_at: at("dispatched_at")?,
                started_at: at("started_at")?,
                ended_at: at("ended_at")?,
                job: row.try_get("job")?,
                job_stopped_at: at("job_stopped_at")?,
                streaming_server: row
                    .try_get::<Option<String>, _>("streaming_server")?
                    .map(|server| server.parse().map_err(decode))
                    .transpose()?,
                runner_commit: row.try_get("runner_commit")?,
                runner_dirty: row.try_get("runner_dirty")?,
                ended_by: row.try_get::<Option<String>, _>("ended_by")?.map(|by| read_word(&by)).transpose()?,
                stop_reason: row.try_get("stop_reason")?,
                stop_detail: row.try_get("stop_detail")?,
                heartbeat_at: at("heartbeat_at")?,
                platform_status: row.try_get("platform_status")?,
                stalled: row.try_get("stalled")?,
                compile: None,
                exact: None,
                exact_reason: None,
                walked: None,
                findings: json(row.try_get("findings")?)?,
                stub: None,
            };
            Ok((read, row.try_get("manifest_key")?))
        })
        .collect()
}

/// A variant's value as a run states it: a number as written, a closed value as its word.
pub fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && float.abs() < 1e15 => format!("{float:.0}"),
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

pub fn json<T: DeserializeOwned>(value: Value) -> Result<T, SqlxError> {
    serde_json::from_value(value).map_err(decode)
}

pub fn parse<T: FromStr>(text: String) -> Result<T, SqlxError>
where
    T::Err: Error + Send + Sync + 'static,
{
    text.parse().map_err(decode)
}

pub fn count(value: i64) -> Result<u32, SqlxError> {
    u32::try_from(value).map_err(decode)
}

pub fn milliseconds(value: i64) -> Result<u64, SqlxError> {
    u64::try_from(value).map_err(decode)
}
