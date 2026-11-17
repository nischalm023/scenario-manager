//! Within `results`, `metrics` is spec 08 section 3.7's two result routes over one attempt's stored values: `GET
//! /api/runs/{r}/metrics`, every metric the attempt declared over a window of simtime, the run's report window unless
//! the request names one, with the run's headline metrics (section 3.9); and `GET /api/runs/{r}/series`, one metric by
//! simulated period from the run's start (section 3.10). The default window is the report window as far as the
//! attempt got, so an attempt that stopped early still shows what it reached; a window the request names, or an
//! additive series' period boundary, that falls between the instants the recording holds values for is refused,
//! `window-not-recorded` naming the nearest ones.

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use desunits::{SimDuration, SimTime};
use serde::Deserialize;
use studio_protocol::{
    Aggregation, AttemptNumber, JournalSeq, MetricsReply, PeriodPoint, Problem, ProblemItem, ProblemType, SeriesKind,
    SeriesReply, SeriesShape, Severity, SimWindow,
};

use crate::AppState;
use crate::problem::{ProblemAnswer, internal, validation_failed};
use crate::runs::{no_run, run_in};

use super::headline::{choose, pins, shown};
use super::stored::{Attempt, attempt, run_row};
use super::values::{Instants, Unrecorded, periods, row, shaped};
use super::words::{sim, sim_exact};

/// The period a series is cut into unless the request names another: an hour of simtime.
const HOUR: SimDuration = SimDuration::ms(3_600_000);

#[derive(Deserialize)]
pub struct MetricsQuery {
    attempt: Option<AttemptNumber>,
    from_sim_ms: Option<SimTime>,
    to_sim_ms: Option<SimTime>,
    highlights: Option<JournalSeq>,
}

/// `GET /api/runs/{r}/metrics?attempt=&from_sim_ms=&to_sim_ms=&highlights=`.
pub async fn metrics(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<MetricsQuery>, QueryRejection>,
) -> Result<Json<MetricsReply>, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let Query(query) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let fault = |fault| internal("read the run's metrics", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    let run_row = run_row(&mut conn, &run).await.map_err(fault)?.ok_or_else(|| no_run(instance))?;
    let attempt = attempt(&mut conn, &run, query.attempt)
        .await
        .map_err(fault)?
        .ok_or_else(|| no_attempt(query.attempt, instance))?;
    let instants = attempt.instants();
    // An attempt that reported no metric (one queued, running, or ended before storing any) has no window to judge.
    if attempt.declared.is_empty() {
        return Ok(Json(MetricsReply {
            attempt: attempt.n,
            window: instants.whole(),
            items: Vec::new(),
            headlines: Vec::new(),
        }));
    }
    let window = match (query.from_sim_ms, query.to_sim_ms) {
        (None, None) => {
            let reached = instants.reached(&run_row.report_window);
            let window = instants.window(reached.from_sim_ms, Some(reached.to_sim_ms));
            window.map_err(|gap| unrecorded("report_window", gap, instance))?
        }
        (from, to) => self::window(&instants, from.unwrap_or(SimTime::ZERO), to, instance)?,
    };
    let whole = instants.whole();
    let items =
        attempt.declared.iter().map(|metric| row(metric, attempt.stored.get(&metric.metric), window, whole)).collect();
    let pinned = pins(&mut conn, &run_row.project).await.map_err(fault)?;
    let shown = match query.highlights {
        Some(seq) => shown(&mut conn, &run_row.site, seq).await.map_err(fault)?,
        None => None,
    };
    let headlines = choose(&attempt.declared, &pinned, shown.as_deref());
    Ok(Json(MetricsReply { attempt: attempt.n, window, items, headlines }))
}

#[derive(Deserialize)]
pub struct SeriesQuery {
    metric: Option<String>,
    attempt: Option<AttemptNumber>,
    period_sim_ms: Option<SimDuration>,
}

/// `GET /api/runs/{r}/series?metric=&attempt=&period_sim_ms=3600000`.
pub async fn series(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<SeriesQuery>, QueryRejection>,
) -> Result<Json<SeriesReply>, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let Query(query) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let Some(metric) = query.metric.filter(|metric| !metric.is_empty()) else {
        return Err(validation_failed("`metric` names the metric whose series is asked for", instance));
    };
    let period = query.period_sim_ms.unwrap_or(HOUR);
    if period.as_ms() == 0 {
        return Err(validation_failed("`period_sim_ms` is a span of simtime longer than 0 ms", instance));
    }
    let fault = |fault| internal("read the run's series", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    let run_row = run_row(&mut conn, &run).await.map_err(fault)?.ok_or_else(|| no_run(instance))?;
    let attempt = attempt(&mut conn, &run, query.attempt)
        .await
        .map_err(fault)?
        .ok_or_else(|| no_attempt(query.attempt, instance))?;
    if attempt.stored.is_empty() && attempt.recording.is_none() {
        return Err(no_recording(&attempt, instance));
    }
    let Some(declared) = attempt.declared.iter().find(|declared| declared.metric == metric) else {
        let detail = format!("Attempt {} of this run declared no metric named {metric}.", attempt.n.0);
        return Err(Problem::new(ProblemType::NotFound, detail).at(instance).into());
    };
    let instants = attempt.instants();
    let stored = attempt.stored.get(&metric);
    // Only an additive metric's running values are cut into periods, so only theirs have boundaries to judge; any
    // other class answers its readings as stated, whatever period was asked.
    let by_period =
        declared.aggregation == Aggregation::Additive && stored.is_none_or(|stored| stored.kind == SeriesKind::Running);
    let (kind, points) = if by_period {
        let periods = periods(period, instants.end());
        for boundary in &periods {
            instants.exact(boundary.from_sim_ms).map_err(|gap| unrecorded("period_sim_ms", gap, instance))?;
        }
        let none = |window: &SimWindow| PeriodPoint {
            from_sim_ms: window.from_sim_ms,
            to_sim_ms: window.to_sim_ms,
            value: None,
        };
        match stored {
            Some(stored) => shaped(declared, stored, &periods),
            None => (SeriesShape::PerPeriod, periods.iter().map(none).collect()),
        }
    } else {
        stored.map_or((SeriesShape::Readings, Vec::new()), |stored| shaped(declared, stored, &[]))
    };
    Ok(Json(SeriesReply {
        run,
        attempt: attempt.n,
        metric: declared.metric.clone(),
        label: declared.label.clone(),
        unit: declared.unit,
        clock: declared.clock,
        aggregation: declared.aggregation,
        period_sim_ms: (kind == SeriesShape::PerPeriod).then_some(period),
        kind,
        points,
        window: instants.reached(&run_row.report_window),
    }))
}

/// The window from `from` to `to`, the run's end when `to` is none, once both are recorded instants and it is not
/// empty.
pub fn window(
    instants: &Instants,
    from: SimTime,
    to: Option<SimTime>,
    instance: &str,
) -> Result<SimWindow, ProblemAnswer> {
    let window = instants.window(from, to).map_err(|gap| {
        let field = if gap.asked == from { "from_sim_ms" } else { "to_sim_ms" };
        unrecorded(field, gap, instance)
    })?;
    if window.from_sim_ms >= window.to_sim_ms {
        let detail = format!(
            "the window from {} to {} of simtime holds no span",
            sim(window.from_sim_ms),
            sim(window.to_sim_ms)
        );
        return Err(validation_failed(detail, instance));
    }
    Ok(window)
}

/// `validation-failed` with spec 08's item `window-not-recorded`, naming the recorded instants either side.
pub fn unrecorded(field: &str, gap: Unrecorded, instance: &str) -> ProblemAnswer {
    let nearest = match (gap.before, gap.after) {
        (Some(before), Some(after)) => format!("the nearest are {} and {}", sim_exact(before), sim_exact(after)),
        (Some(before), None) => format!("the run's values end at {}", sim_exact(before)),
        (None, Some(after)) => format!("the first is {}", sim_exact(after)),
        (None, None) => "it holds none".to_owned(),
    };
    let text = format!(
        "{} of simtime falls between the instants this run's recording holds values for; {nearest} of simtime.",
        sim_exact(gap.asked)
    );
    let mut problem =
        Problem::new(ProblemType::ValidationFailed, format!("The request does not read: {text}")).at(instance);
    problem.problems = vec![ProblemItem {
        code: "window-not-recorded".to_owned(),
        severity: Severity::Error,
        ref_: None,
        scope: None,
        field: Some(field.to_owned()),
        knob: None,
        text,
    }];
    problem.into()
}

pub fn no_attempt(n: Option<AttemptNumber>, instance: &str) -> ProblemAnswer {
    let detail =
        n.map_or_else(|| "This run has no attempt yet.".to_owned(), |n| format!("This run has no attempt {}.", n.0));
    Problem::new(ProblemType::NotFound, detail).at(instance).into()
}

pub fn no_recording(attempt: &Attempt, instance: &str) -> ProblemAnswer {
    let detail = format!("Attempt {} of this run stored no recording, so it has none to give.", attempt.n.0);
    Problem::new(ProblemType::NoRecording, detail).at(instance).into()
}
