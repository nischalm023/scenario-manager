//! Within `tools`, `runs` are the agent's reads of a run's results (spec 01 section 3.9; spec 08 sections 3.7 to
//! 3.10): `run.metrics`, every metric the run's attempt declared over a window of simtime, with the headline metrics,
//! as `GET /api/runs/{r}/metrics` answers it; and `run.series`, one metric by simulated period, as
//! `GET /api/runs/{r}/series` answers it. Each calls the route the Runs panel calls, so a figure the agent quotes is
//! one the panel shows; the metrics are the stored ones, the metric streams being the run page's live charts. A
//! metric's name travels percent-encoded, so a name holding `&`, `#` or a space reaches the route whole, and a metric
//! the agent names that the attempt did not declare is refused by name, never dropped.

use agent_loop::{Class, Tool, ToolFuture};
use axum::extract::{Path, Query, State};
use desunits::{SimDuration, SimTime};
use serde::Deserialize;
use serde_json::{Value, json};
use studio_protocol::{AttemptNumber, MetricsReply, Problem, ProblemType, Ulid};
use wps_client::wps::Wps;

use super::{ToolContext, encoded, input, uri};
use crate::problem::ProblemAnswer;
use crate::results::metrics::{metrics, series};

struct RunMetrics;

#[derive(Deserialize)]
struct MetricsInput {
    run: Ulid,
    attempt: Option<AttemptNumber>,
    from_sim_ms: Option<SimTime>,
    to_sim_ms: Option<SimTime>,
    metrics: Option<Vec<String>>,
}

/// The query `GET /api/runs/{r}/metrics` reads, naming only what was given.
fn metrics_query(attempt: Option<AttemptNumber>, from: Option<SimTime>, to: Option<SimTime>) -> String {
    let pairs = [
        attempt.map(|attempt| format!("attempt={}", attempt.0)),
        from.map(|from| format!("from_sim_ms={}", from.as_ms())),
        to.map(|to| format!("to_sim_ms={}", to.as_ms())),
    ];
    pairs.into_iter().flatten().collect::<Vec<_>>().join("&")
}

/// The reply as the agent reads it, its items kept to the `wanted` ones when it names some, every one of which the
/// attempt must have declared.
fn answer(reply: MetricsReply, wanted: Option<&[String]>) -> Result<Value, ProblemAnswer> {
    if let Some(wanted) = wanted {
        let unknown: Vec<&str> = wanted
            .iter()
            .filter(|name| !reply.items.iter().any(|item| item.metric == **name))
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            let detail =
                format!("Attempt {} of this run declared no metric named {}.", reply.attempt.0, unknown.join(", "));
            return Err(Problem::new(ProblemType::NotFound, detail).into());
        }
    }
    let items: Vec<Value> = reply
        .items
        .iter()
        .filter(|item| wanted.is_none_or(|wanted| wanted.contains(&item.metric)))
        .map(|item| {
            json!({
                "metric": item.metric, "label": item.label, "unit": item.unit, "clock": item.clock,
                "aggregation": item.aggregation, "value": item.value, "window": item.window, "basis": item.basis,
            })
        })
        .collect();
    Ok(json!({ "attempt": reply.attempt, "window": reply.window, "headlines": reply.headlines, "items": items }))
}

impl<S: Wps> Tool<ToolContext<S>> for RunMetrics {
    fn name(&self) -> &'static str {
        "run.metrics"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "Read a run's metrics: every metric its attempt declared, with its value over a window of simtime (the run's \
         report window unless `from_sim_ms` and `to_sim_ms` name one), and which are the headline metrics. `metrics` \
         keeps only the named ones. Returns { attempt, window: { from_sim_ms, to_sim_ms }, headlines: [name], items: \
         [{ metric, label, unit, clock, aggregation, value, window, basis }] }; `clock` is the clock a duration or a \
         rate's hour counts in, `sim` or `wall`, and null for any other unit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object", "additionalProperties": false, "required": ["run"],
            "properties": {
                "run": { "type": "string", "description": "the run's id" },
                "attempt": { "type": "integer", "minimum": 1, "description": "the attempt; the latest unless named" },
                "from_sim_ms": { "type": "integer", "minimum": 0 },
                "to_sim_ms": { "type": "integer", "minimum": 0 },
                "metrics": {
                    "type": "array", "items": { "type": "string" }, "description": "only these metrics, by name",
                },
            },
        })
    }

    fn describe(&self, input: &Value) -> String {
        format!("Reading the metrics of run {}", input["run"].as_str().unwrap_or("named"))
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        match result["items"].as_array().map_or(0, Vec::len) {
            1 => "Read 1 metric".to_owned(),
            n => format!("Read {n} metrics"),
        }
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let MetricsInput { run, attempt, from_sim_ms, to_sim_ms, metrics: wanted } = input(&given)?;
            let address = uri(&format!("/api/runs/{run}/metrics?{}", metrics_query(attempt, from_sim_ms, to_sim_ms)))?;
            let asked = Query::try_from_uri(&address);
            let reply = metrics(State(cx.gate.app.clone()), address, Ok(Path(run.to_string())), asked).await?.0;
            answer(reply, wanted.as_deref()).map_err(Problem::from)
        })
    }
}

struct RunSeries;

#[derive(Deserialize)]
struct SeriesInput {
    run: Ulid,
    metric: String,
    attempt: Option<AttemptNumber>,
    period_sim_ms: Option<SimDuration>,
}

/// The query `GET /api/runs/{r}/series` reads: the metric's name percent-encoded, then what else was given.
fn series_query(metric: &str, attempt: Option<AttemptNumber>, period: Option<SimDuration>) -> String {
    let pairs = [
        Some(format!("metric={}", encoded(metric))),
        attempt.map(|attempt| format!("attempt={}", attempt.0)),
        period.map(|period| format!("period_sim_ms={}", period.as_ms())),
    ];
    pairs.into_iter().flatten().collect::<Vec<_>>().join("&")
}

impl<S: Wps> Tool<ToolContext<S>> for RunSeries {
    fn name(&self) -> &'static str {
        "run.series"
    }

    fn class(&self) -> Class {
        Class::Read
    }

    fn description(&self) -> &'static str {
        "Read one metric of a run as a series over simtime: an additive metric's value per period (an hour unless \
         `period_sim_ms` names another), any other metric's readings as stated. Returns { metric, label, unit, \
         clock?, aggregation, kind, period_sim_ms?, window, points: [{ from_sim_ms, to_sim_ms, value }] }; `clock` \
         is `sim` or `wall` for a duration or a rate, as on run.metrics."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object", "additionalProperties": false, "required": ["run", "metric"],
            "properties": {
                "run": { "type": "string", "description": "the run's id" },
                "metric": { "type": "string", "description": "the metric's name, as run.metrics lists it" },
                "attempt": { "type": "integer", "minimum": 1 },
                "period_sim_ms": {
                    "type": "integer", "minimum": 1, "description": "the period, in milliseconds of simtime",
                },
            },
        })
    }

    fn describe(&self, input: &Value) -> String {
        format!(
            "Reading {} of run {} over simtime",
            input["metric"].as_str().unwrap_or("a metric"),
            input["run"].as_str().unwrap_or("named")
        )
    }

    fn summarize(&self, _input: &Value, result: &Value) -> String {
        let points = result["points"].as_array().map_or(0, Vec::len);
        format!("Read {} as {points} points", result["label"].as_str().unwrap_or("the metric"))
    }

    fn execute<'a>(&'a self, cx: &'a ToolContext<S>, given: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let SeriesInput { run, metric, attempt, period_sim_ms } = input(&given)?;
            let address = uri(&format!("/api/runs/{run}/series?{}", series_query(&metric, attempt, period_sim_ms)))?;
            let asked = Query::try_from_uri(&address);
            let reply = series(State(cx.gate.app.clone()), address, Ok(Path(run.to_string())), asked).await?.0;
            Ok(json!(reply))
        })
    }
}

pub fn tools<S: Wps>() -> Vec<Box<dyn Tool<ToolContext<S>>>> {
    vec![Box::new(RunMetrics), Box::new(RunSeries)]
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;
    use desunits::{SimDuration, SimTime};
    use serde::Deserialize;
    use studio_protocol::{
        Aggregation, AttemptNumber, DurationClock, MetricRow, MetricUnit, MetricsReply, ProblemType, SimWindow,
        ValueBasis,
    };

    use super::{answer, metrics_query, series_query};
    use crate::tools::uri;

    /// The series route's query as its own parser reads it.
    #[derive(Debug, Deserialize, PartialEq)]
    struct Asked {
        metric: String,
        attempt: Option<u32>,
        period_sim_ms: Option<u64>,
    }

    /// A metric's name holding the query's own syntax, a space, a percent sign or letters past ASCII reaches the route
    /// as it was named, beside the attempt and the period.
    #[test]
    fn a_metric_name_reaches_the_series_route_whole_whatever_it_holds() {
        let names = ["picks_per_hour", "a&metric=b", "with space", "hash#tag", "per%cent", "path/and?query", "Zähler"];
        for metric in names {
            let query = series_query(metric, Some(AttemptNumber(2)), Some(SimDuration::ms(60_000)));
            let address = uri(&format!("/api/runs/01KZAVPQNES65HV4EZADPCS7E8/series?{query}")).expect("an address");
            let Query(asked) = Query::<Asked>::try_from_uri(&address).expect("the route's query reads");
            assert_eq!(asked, Asked { metric: metric.to_owned(), attempt: Some(2), period_sim_ms: Some(60_000) });
        }
    }

    #[test]
    fn the_metrics_query_names_only_what_was_given() {
        assert_eq!(metrics_query(None, None, None), "");
        let window = metrics_query(Some(AttemptNumber(3)), Some(SimTime::ms(3_600_000)), Some(SimTime::ms(7_200_000)));
        assert_eq!(window, "attempt=3&from_sim_ms=3600000&to_sim_ms=7200000");
    }

    fn row(metric: &str) -> MetricRow {
        let whole = SimWindow { from_sim_ms: SimTime::ZERO, to_sim_ms: SimTime::ms(10_800_000) };
        MetricRow {
            metric: metric.to_owned(),
            label: metric.to_owned(),
            description: String::new(),
            unit: MetricUnit::Count,
            clock: None,
            aggregation: Aggregation::Additive,
            denominator: None,
            value: Some(1.0),
            window: whole,
            basis: ValueBasis::EndBlock,
        }
    }

    fn reply() -> MetricsReply {
        let whole = SimWindow { from_sim_ms: SimTime::ZERO, to_sim_ms: SimTime::ms(10_800_000) };
        let items = ["picks", "orders", "putaways"].map(row).to_vec();
        MetricsReply { attempt: AttemptNumber(1), window: whole, items, headlines: vec!["picks".to_owned()] }
    }

    /// The metrics the agent names are kept, in the reply's order; all of them when it names none.
    #[test]
    fn the_named_metrics_are_kept_in_the_replys_order() {
        let named = ["putaways".to_owned(), "picks".to_owned()];
        let kept = answer(reply(), Some(&named)).expect("both declared");
        let items = kept["items"].as_array().expect("the items");
        let metrics: Vec<&str> = items.iter().map(|item| item["metric"].as_str().unwrap()).collect();
        assert_eq!(metrics, ["picks", "putaways"]);
        assert_eq!(answer(reply(), None).expect("every one")["items"].as_array().map(Vec::len), Some(3));
    }

    /// A duration's clock rides the answer, so the agent never reads the run's wall clock as simtime.
    #[test]
    fn a_durations_clock_rides_the_answer() {
        let mut timed = reply();
        timed.items[0] = MetricRow { unit: MetricUnit::DurationMs, clock: Some(DurationClock::Wall), ..row("wall") };
        let items = answer(timed, None).expect("every one")["items"].clone();
        assert_eq!((items[0]["clock"].as_str(), items[1]["clock"].as_str()), (Some("wall"), None));
    }

    /// A metric the attempt did not declare is refused by name rather than dropped, among names it did declare.
    #[test]
    fn a_metric_the_attempt_did_not_declare_is_refused_by_name() {
        let named = ["picks".to_owned(), "pick_rate".to_owned(), "orders".to_owned(), "lunch".to_owned()];
        let refused = *answer(reply(), Some(&named)).expect_err("two undeclared").0;
        assert_eq!(refused.type_, ProblemType::NotFound);
        assert_eq!(refused.detail, "Attempt 1 of this run declared no metric named pick_rate, lunch.");
    }
}
