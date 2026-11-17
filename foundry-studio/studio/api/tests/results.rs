//! Spec 08 section 8's statements 9 and 10 and the result routes (brief I1.10): on simrunner's canned fixture,
//! each additive metric's hourly values sum to its last running value as stored, no hourly value is made for another
//! class, and a window between two slice ends equals the series' difference exactly; no metric's name is a string
//! literal in the API's runs and results modules or in the Runs panel; and, against the database, a finished stub
//! attempt's metrics, series and headline metrics as the routes answer them, a window from 7,205,000 ms of simtime
//! refused as `window-not-recorded`, and the headline rule's three sources in order, the list's column included.
//! Astra's review of the results (2026-10-20) adds a test per finding the API answers: an early stop narrows the
//! default window (F3), an additive metric without running values has no window value (F4), the list follows the
//! `show` entry (F5) and a gauge's readings judge no period (F6). The end block's whole-run figures, which store no
//! series and which `metrics.json` marks the whole run's, answer the whole run's value over any window.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use desunits::{SimDuration, SimTime};
use rustc_hash::FxHashMap;
use serde_json::json;
use studio_api::config::RunConfig;
use studio_api::journal::NewEntry;
use studio_api::results::{Declared, Instants, Stored, periods, row, running_at, shaped};
use studio_metrics::SummarySeries;
use studio_protocol::{
    Aggregation, JournalKind, Panel, PanelAction, PanelCommandEntry, ReportWindow, SeriesKind, SeriesShape, SimWindow,
    ValueBasis,
};

use support::results::{canned, finished};
use support::runs::{PERSON, experiment, plan, scenario, send, stack, submit};

const HOUR: SimDuration = SimDuration::ms(3_600_000);

/// The canned fixture as the API's rows hold it: the declared metrics in descriptor order, each one's series, and the
/// instant its run ended.
fn canned_values() -> (Vec<Declared>, FxHashMap<String, Stored>, SimTime) {
    let (metrics, series) = canned();
    let declared = metrics
        .metrics
        .iter()
        .map(|metric| Declared {
            metric: metric.metric.clone(),
            label: metric.label.clone(),
            description: metric.description.clone(),
            unit: metric.unit,
            clock: metric.clock,
            aggregation: metric.aggregation,
            denominator: metric.denominator.clone(),
            value: metric.value,
            basis: metrics.basis,
            whole_run: metric.whole_run,
        })
        .collect();
    let stored = series
        .metrics
        .iter()
        .map(|one| (one.metric.clone(), Stored { kind: one.kind, points: one.points.clone() }))
        .collect();
    (declared, stored, series.end_sim_ms)
}

fn close(sum: f64, value: f64) -> bool {
    (sum - value).abs() <= 1e-9 * value.abs().max(1.0)
}

/// Statement 10: each additive metric's hourly values sum to its last running value, read straight from the stored
/// series rather than from a total this code computed, the hour the run's end cuts short included; no hourly value is
/// made for any other class, which reads as its readings.
#[test]
fn each_additive_metrics_hourly_values_sum_to_its_value_over_the_whole_run() {
    let (declared, stored, end) = canned_values();
    let instants = Instants::new(stored.values(), end);
    let whole = instants.whole();
    assert_eq!((whole.from_sim_ms, whole.to_sim_ms), (SimTime::ZERO, SimTime::ms(10_830_000)));
    let hours = periods(HOUR, end);
    let spans: Vec<(u64, u64)> = hours.iter().map(|hour| (hour.from_sim_ms.as_ms(), hour.to_sim_ms.as_ms())).collect();
    assert_eq!(spans, [(0, 3_600_000), (3_600_000, 7_200_000), (7_200_000, 10_800_000), (10_800_000, 10_830_000)]);
    assert!(
        hours.iter().all(|hour| instants.exact(hour.from_sim_ms).is_ok()),
        "every hour starts on a recorded instant"
    );
    let mut additive = 0;
    for metric in &declared {
        let Some(series) = stored.get(&metric.metric) else {
            assert!(is_figure(&metric.metric), "{} stored no series and is no whole-run figure", metric.metric);
            continue;
        };
        let (shape, points) = shaped(metric, series, &hours);
        if metric.aggregation != Aggregation::Additive {
            assert_eq!(shape, SeriesShape::Readings, "{} reads as its readings", metric.metric);
            assert_eq!(points.len(), series.points.len(), "{}'s readings are never thinned", metric.metric);
            continue;
        }
        additive += 1;
        assert_eq!(shape, SeriesShape::PerPeriod, "{} is drawn per hour", metric.metric);
        let stated: Vec<f64> = points.iter().filter_map(|point| point.value).collect();
        match series.points.iter().rev().find_map(|point| point.1) {
            None => assert!(stated.is_empty(), "{} stated nothing, so no hour holds a value", metric.metric),
            Some(last) => {
                let sum: f64 = stated.iter().sum();
                assert!(
                    close(sum, last),
                    "{}: the hours sum to {sum}, its last running value is {last}",
                    metric.metric
                );
            }
        }
    }
    assert!(additive > 80, "the fixture's additive metrics were checked: {additive}");
}

/// Statement 10: a window between two slice ends equals the difference of the series' values at those ends, read
/// here straight from the stored points; and a gauge's value is its whole run's, marked by the whole run's window.
#[test]
fn a_window_between_two_slice_ends_equals_the_series_difference() {
    let (declared, stored, end) = canned_values();
    let instants = Instants::new(stored.values(), end);
    let window = instants.window(SimTime::ms(3_610_000), Some(SimTime::ms(9_000_000))).expect("two slice ends");
    let mut checked = 0;
    for metric in &declared {
        let Some(series) = stored.get(&metric.metric) else {
            assert!(is_figure(&metric.metric), "{} stored no series and is no whole-run figure", metric.metric);
            continue;
        };
        let value = row(metric, Some(series), window, instants.whole());
        if metric.aggregation != Aggregation::Additive {
            assert_eq!(
                (value.window, value.value),
                (instants.whole(), metric.value),
                "{} is a whole-run value",
                metric.metric
            );
            continue;
        }
        let at = |instant: u64| series.points.iter().find(|point| point.0.as_ms() == instant).and_then(|point| point.1);
        if let (Some(from), Some(to)) = (at(3_610_000), at(9_000_000)) {
            assert_eq!(value.value, Some(to - from), "{} over 1 h 10 s to 2 h 30 min", metric.metric);
            assert_eq!(value.window, window);
            checked += 1;
        }
    }
    assert!(checked > 80, "{checked} metrics held values at both ends");
    let gap = instants.window(SimTime::ms(7_205_000), None).expect_err("7,205,000 ms is no slice end");
    assert_eq!((gap.before, gap.after), (Some(SimTime::ms(7_200_000)), Some(SimTime::ms(7_210_000))));
    assert_eq!(running_at(&[], SimTime::ms(1)), None, "nothing stated is no value, never 0");
}

fn span(from: u64, to: u64) -> SimWindow {
    SimWindow { from_sim_ms: SimTime::ms(from), to_sim_ms: SimTime::ms(to) }
}

/// Whether `metric` is one of the end block's whole-run figures, which store a declared value and no series.
fn is_figure(metric: &str) -> bool {
    SummarySeries::ALL.iter().any(|series| series.name() == metric)
}

/// The end block's whole-run figures, desvis's Status rows such as the run's total picks, store a declared value and
/// no series, and `metrics.json` marks each the whole run's: over any window, the whole run's or a narrower one after
/// a warm-up, each answers that value marked by the whole run's window, as the end block's. Unmarked, the run's total
/// picks would be an additive metric with no running values, which answers no narrower window.
#[test]
fn a_whole_run_figure_answers_its_whole_run_value_over_any_window() {
    let (declared, stored, end) = canned_values();
    let whole = Instants::new(stored.values(), end).whole();
    let figures: Vec<&Declared> = declared.iter().filter(|metric| !stored.contains_key(&metric.metric)).collect();
    assert_eq!(figures.len(), 16, "desvis's Status figures: {figures:?}");
    assert!(figures.iter().all(|figure| figure.whole_run), "each figure is marked the whole run's");
    assert_eq!(declared.iter().filter(|metric| metric.whole_run).count(), 16, "and no other metric is");
    let after_warm_up = span(1_800_000, end.as_ms());
    for figure in &figures {
        for window in [whole, span(3_600_000, 7_200_000), after_warm_up] {
            let value = row(figure, None, window, whole);
            assert_eq!((value.value, value.window, value.basis), (figure.value, whole, ValueBasis::EndBlock));
            assert!(value.value.is_some(), "{} answers the whole run", figure.metric);
        }
    }
    let picks = figures.iter().find(|metric| metric.metric == "run_total_picks").expect("the run's total picks");
    assert_eq!(picks.aggregation, Aggregation::Additive);
    assert!(picks.value.is_some_and(|picks| picks > 0.0), "{picks:?}");
    let unmarked = Declared { whole_run: false, ..(*picks).clone() };
    let narrower = row(&unmarked, None, after_warm_up, whole);
    assert_eq!((narrower.value, narrower.window, narrower.basis), (None, after_warm_up, ValueBasis::WindowDifference));
}

/// Astra's review of the results, F4 (brief I1.10 section 1; spec 08 section 3.9's additive window rule): an additive
/// metric that stored no running values has no value over a window, marked as the window difference it could not
/// take, never its whole run's number in their place; nor has one over a window that holds no span.
#[test]
fn an_additive_metric_without_running_values_has_no_window_value() {
    let (declared, stored, end) = canned_values();
    let instants = Instants::new(stored.values(), end);
    let counted = declared
        .iter()
        .find(|metric| metric.aggregation == Aggregation::Additive && metric.value.is_some_and(|value| value > 0.0))
        .expect("an additive metric with a final value");
    let window = span(3_600_000, 7_200_000);
    let readings = Stored { kind: SeriesKind::Readings, points: stored[&counted.metric].points.clone() };
    for stored in [None, Some(&readings)] {
        let value = row(counted, stored, window, instants.whole());
        assert_eq!((value.value, value.window, value.basis), (None, window, ValueBasis::WindowDifference));
    }
    let empty = span(7_200_000, 7_200_000);
    assert_eq!(row(counted, Some(&stored[&counted.metric]), empty, instants.whole()).value, None);
}

/// Astra's review of the results, F3 (spec 08 section 3.9, unfinished runs): the report window as far as an attempt
/// got ends at the attempt's stop when it stopped before the window's planned end, and holds no span when it stopped
/// before the window began, so its default window is never refused for the part it did not reach.
#[test]
fn the_report_window_resolves_against_the_attempts_stop() {
    let (_, stored, end) = canned_values();
    let instants = Instants::new(stored.values(), end);
    let report = |from: u64, to: Option<u64>| ReportWindow {
        from_sim_ms: SimTime::ms(from),
        to_sim_ms: to.map(SimTime::ms),
        before_name: "warm-up".to_owned(),
    };
    assert_eq!(instants.reached(&report(1_800_000, Some(14_400_000))), span(1_800_000, 10_830_000));
    assert_eq!(instants.reached(&report(1_800_000, None)), span(1_800_000, 10_830_000));
    assert_eq!(instants.reached(&report(1_800_000, Some(3_600_000))), span(1_800_000, 3_600_000));
    let early = Instants::new(stored.values(), SimTime::ms(1_200_000));
    assert_eq!(early.reached(&report(1_800_000, None)), span(1_200_000, 1_200_000));
}

/// The source files statement 9 covers: the API's runs and results modules and the Runs panel, its tests aside.
fn covered() -> Vec<PathBuf> {
    let api = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [api.join("src/runs"), api.join("src/results"), api.join("../ui/src/panels/runs")];
    let mut files = Vec::new();
    let mut walking: Vec<PathBuf> = roots.to_vec();
    while let Some(path) = walking.pop() {
        if path.is_dir() {
            walking.extend(fs::read_dir(&path).expect("a folder lists").map(|entry| entry.expect("an entry").path()));
        } else if !path.to_string_lossy().contains(".test.") {
            files.push(path);
        }
    }
    files
}

/// Statement 9: no metric's name is a string literal where the runs and results are read and shown, so metrics are
/// discovered from what a run declares and never enumerated. The names are every metric the canned run declared.
#[test]
fn no_metric_name_is_a_string_literal_in_the_runs_and_results_code_or_the_runs_panel() {
    let (metrics, _) = canned();
    let files = covered();
    assert!(files.len() > 20, "the covered files were found: {}", files.len());
    let mut found = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("a source file reads");
        for metric in &metrics.metrics {
            for quote in ['"', '\'', '`'] {
                if text.contains(&format!("{quote}{}{quote}", metric.metric)) {
                    found.push(format!("{}: {}", file.display(), metric.metric));
                }
            }
        }
    }
    assert_eq!(found, Vec::<String>::new(), "metric names written as literals");
}

fn config() -> RunConfig {
    RunConfig::local("http://127.0.0.1:3101", "studio-simrunner:dev")
}

/// The database half of statement 5's window rule on a finished attempt: a window from 7,205,000 ms of simtime falls
/// between two slice ends and is refused, `window-not-recorded` naming the nearest recorded instants either side; so
/// is a window ending there, and an additive metric's period whose boundaries leave the slice ends; a gauge's readings
/// judge no period, so that period answers them all (Astra's review of the results, F6; spec 08 section 3.10).
#[tokio::test]
async fn a_window_from_7_205_000_ms_of_simtime_is_refused_as_not_recorded() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_, runs) = finished(&stack, "Peak hour", &[]).await;
    let run = &runs[0];
    for (query, field) in [("from_sim_ms=7205000", "from_sim_ms"), ("from_sim_ms=0&to_sim_ms=7205000", "to_sim_ms")] {
        let (status, problem) = send(&stack.router, "GET", &format!("/api/runs/{run}/metrics?{query}"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["type"], "validation-failed");
        let item = &problem["problems"][0];
        assert_eq!((item["code"].as_str(), item["field"].as_str()), (Some("window-not-recorded"), Some(field)));
        let text = item["text"].as_str().expect("the item's words");
        assert!(text.contains("7,200,000 ms") && text.contains("7,210,000 ms"), "{text}");
    }
    let (metrics, series) = canned();
    let additive = metrics.metrics.iter().find(|metric| metric.aggregation == Aggregation::Additive).expect("additive");
    let uri = format!("/api/runs/{run}/series?metric={}&period_sim_ms=3605000", additive.metric);
    let (status, problem) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem["problems"][0]["code"], "window-not-recorded");
    let gauge = metrics.metrics.iter().find(|metric| metric.aggregation == Aggregation::SnapshotGauge).expect("gauge");
    let uri = format!("/api/runs/{run}/series?metric={}&period_sim_ms=3605000", gauge.metric);
    let (status, readings) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{readings}");
    let stored = series.metrics.iter().find(|one| one.metric == gauge.metric).expect("the gauge's series");
    assert_eq!(
        (readings["kind"].as_str(), readings["points"].as_array().map(Vec::len)),
        (Some("readings"), Some(stored.points.len()))
    );
    scratch.close().await;
}

/// Astra's review of the results, F3 (spec 08 section 3.9, unfinished runs): a run whose report window was planned to
/// end at 4 h of simtime but whose attempt stopped at 3 h 30 s answers its metrics up to the stop by default, an
/// additive one as its running value there less at the window's start, and the list's column over that span; asking
/// for the planned end itself is still refused, as no instant there was recorded.
#[tokio::test]
async fn an_attempt_that_stopped_before_its_report_windows_end_answers_up_to_its_stop() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_, runs) = finished(&stack, "Peak hour", &[]).await;
    let run = &runs[0];
    sqlx::query("UPDATE runs SET report_window = $2::jsonb WHERE id = $1")
        .bind(run.as_str())
        .bind(json!({ "from_sim_ms": 1_800_000, "to_sim_ms": 14_400_000, "before_name": "warm-up" }).to_string())
        .execute(&scratch.pool)
        .await
        .expect("the run planned with a window to 4 h of simtime");
    let (status, read) = send(&stack.router, "GET", &format!("/api/runs/{run}/metrics"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["window"], json!({ "from_sim_ms": 1_800_000, "to_sim_ms": 10_830_000 }));
    let (metrics, series) = canned();
    let counted = metrics
        .metrics
        .iter()
        .find(|metric| metric.aggregation == Aggregation::Additive && metric.value > Some(100.0))
        .expect("a counted metric");
    let running = &series.metrics.iter().find(|one| one.metric == counted.metric).expect("its series").points;
    let at = |instant: u64| running.iter().find(|point| point.0.as_ms() == instant).and_then(|point| point.1);
    let item = read["items"].as_array().unwrap().iter().find(|item| item["metric"] == counted.metric.as_str()).unwrap();
    assert_eq!(item["value"].as_f64(), Some(at(10_830_000).unwrap() - at(1_800_000).unwrap()));
    // The run's total picks, a Status figure, answers the whole run's value after the warm-up too, marked as such.
    let picks = metrics.metrics.iter().find(|metric| metric.metric == "run_total_picks").expect("a Status figure");
    let item = read["items"].as_array().unwrap().iter().find(|item| item["metric"] == "run_total_picks").unwrap();
    assert_eq!(item["value"].as_f64(), picks.value);
    assert_eq!(item["window"], json!({ "from_sim_ms": 0, "to_sim_ms": 10_830_000 }));
    assert_eq!(item["basis"], "end_block");
    let uri = format!("/api/runs/{run}/metrics?from_sim_ms=1800000&to_sim_ms=14400000");
    let (status, problem) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem["problems"][0]["code"], "window-not-recorded");
    let (_, list) = send(&stack.router, "GET", &format!("/api/runs?site={}", stack.site), None).await;
    assert_eq!(list["items"][0]["headline"]["window"], json!({ "from_sim_ms": 1_800_000, "to_sim_ms": 10_830_000 }));
    scratch.close().await;
}

/// Sections 3.9 and 3.10 over a finished stub attempt: every declared metric over the report window, the whole run
/// here, an additive one as its window difference and a gauge as its whole-run value; a window between slice ends;
/// one metric per simulated hour, the last hour cut short by the run's end, and a gauge's readings; the headline rule
/// in its order (the first four declared, then the agent's `show` command's, then the project's pins, which win), and
/// the list's headline column; and the refusals: an undeclared metric, a project's ninth pin, an unknown project.
#[tokio::test]
async fn a_finished_stub_attempts_metrics_series_and_headlines() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_, runs) = finished(&stack, "Peak hour", &[]).await;
    let run = &runs[0];
    let (metrics, series) = canned();
    let names: Vec<String> = metrics.metrics.iter().map(|metric| metric.metric.clone()).collect();
    let additive = metrics
        .metrics
        .iter()
        .find(|metric| metric.aggregation == Aggregation::Additive && metric.value > Some(100.0))
        .expect("a counted metric");
    let gauge =
        metrics.metrics.iter().find(|metric| metric.aggregation == Aggregation::SnapshotGauge).expect("a gauge");
    let running = series.metrics.iter().find(|one| one.metric == additive.metric).expect("its series");
    assert_eq!(running.kind, SeriesKind::Running);

    let (status, read) = send(&stack.router, "GET", &format!("/api/runs/{run}/metrics"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["window"], json!({ "from_sim_ms": 0, "to_sim_ms": 10_830_000 }));
    let items = read["items"].as_array().expect("the items");
    assert_eq!(
        items.iter().map(|item| item["metric"].as_str().unwrap()).collect::<Vec<_>>(),
        names,
        "descriptor order"
    );
    assert_eq!(read["headlines"], json!(names[..4]), "no pins and no command: the first four declared");
    let item = |metric: &str| items.iter().find(|item| item["metric"] == metric).expect("the metric's row").clone();
    let last = running.points.last().and_then(|point| point.1).expect("a running value at the end");
    assert_eq!(
        (item(&additive.metric)["value"].as_f64(), item(&additive.metric)["basis"].as_str()),
        (Some(last), Some("window_difference"))
    );
    assert_eq!(item(&gauge.metric)["basis"], "end_block");
    let picks = metrics.metrics.iter().find(|metric| metric.metric == "run_total_picks").expect("a Status figure");
    assert_eq!(
        (item(&picks.metric)["value"].as_f64(), item(&picks.metric)["basis"].as_str()),
        (picks.value, Some("end_block")),
        "a whole-run figure answers the whole run"
    );
    // Each duration names the clock metrics.json declared for it, as stored: the run's wall clock the wall clock, the
    // simtime it reached simtime; a count names none.
    assert_eq!(item("run_wall_clock")["clock"], "wall");
    assert_eq!(item("run_sim_time")["clock"], "sim");
    assert_eq!(item(&picks.metric).get("clock"), None);

    let uri = format!("/api/runs/{run}/metrics?from_sim_ms=3600000&to_sim_ms=7200000");
    let (status, read) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let at = |instant: u64| {
        running.points.iter().find(|point| point.0.as_ms() == instant).and_then(|point| point.1).unwrap()
    };
    let hour_1 = read["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["metric"] == additive.metric.as_str())
        .unwrap()
        .clone();
    assert_eq!(hour_1["value"].as_f64(), Some(at(7_200_000) - at(3_600_000)));
    assert_eq!(hour_1["window"], json!({ "from_sim_ms": 3_600_000, "to_sim_ms": 7_200_000 }));
    let figure = read["items"].as_array().unwrap().iter().find(|item| item["metric"] == picks.metric.as_str()).unwrap();
    assert_eq!(figure["value"].as_f64(), picks.value, "and the whole run's over a narrower window");
    assert_eq!(figure["window"], json!({ "from_sim_ms": 0, "to_sim_ms": 10_830_000 }), "marked by the whole run's");

    let (status, hours) =
        send(&stack.router, "GET", &format!("/api/runs/{run}/series?metric={}", additive.metric), None).await;
    assert_eq!(status, StatusCode::OK, "{hours}");
    assert_eq!((hours["kind"].as_str(), hours["period_sim_ms"].as_u64()), (Some("per_period"), Some(3_600_000)));
    let points = hours["points"].as_array().expect("the hours");
    assert_eq!(points.len(), 4, "hours 0 to 2 and the 30 s of hour 3 the run reached");
    assert_eq!(points[3]["to_sim_ms"], 10_830_000);
    let sum: f64 = points.iter().filter_map(|point| point["value"].as_f64()).sum();
    assert!(close(sum, last), "{sum} against {last}");
    let (status, readings) =
        send(&stack.router, "GET", &format!("/api/runs/{run}/series?metric={}", gauge.metric), None).await;
    assert_eq!(status, StatusCode::OK, "{readings}");
    assert_eq!(readings["kind"], "readings");
    assert!(readings.get("period_sim_ms").is_none(), "readings have no period");
    let stored = series.metrics.iter().find(|one| one.metric == gauge.metric).expect("the gauge's series");
    assert_eq!(readings["points"].as_array().map(Vec::len), Some(stored.points.len()));
    // A series names its clock as the metric's row does: the slices' wall clock, a duty's simtime, a count none.
    let clocked =
        [("wall_clock_duration_ms", Some("wall")), ("htm_duty_traveling_ms", Some("sim")), ("picks_completed", None)];
    for (metric, clock) in clocked {
        let (status, timed) =
            send(&stack.router, "GET", &format!("/api/runs/{run}/series?metric={metric}"), None).await;
        assert_eq!(status, StatusCode::OK, "{timed}");
        assert_eq!(timed["clock"].as_str(), clock, "{metric}");
    }
    let (status, _) = send(&stack.router, "GET", &format!("/api/runs/{run}/series?metric=no_such_metric"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The agent's `show` command names the metrics when the project pins none.
    let site = stack.site.to_string();
    let live: Option<String> = sqlx::query_scalar("SELECT id FROM conversations WHERE site = $1 AND state = 'live'")
        .bind(&site)
        .fetch_optional(&scratch.pool)
        .await
        .expect("the site's live conversation, if one is open");
    let conversation = match live {
        Some(id) => id.parse().expect("a ULID"),
        None => support::conversation(&scratch.pool, Some(&site), "live", PERSON, 1).await,
    };
    let command = PanelCommandEntry {
        action: PanelAction::Show,
        panel: Panel::Runs,
        location: Some(format!("run:{run}").parse().expect("a run's key")),
        resolved: Vec::new(),
        select: None,
        layers: None,
        draft: None,
        sim_ms: None,
        compare: None,
        metrics: Some(vec![names[10].clone(), "never_declared".to_owned(), names[5].clone()]),
        highlight: None,
        route: format!("/runs/{run}"),
        as_it_was: None,
        announce: "Opened the run".to_owned(),
    };
    let entry = NewEntry::new(JournalKind::PanelCommand, serde_json::to_value(&command).expect("a body"));
    let seq = support::write(&stack.gate.app, &conversation, entry).await;
    let uri = format!("/api/runs/{run}/metrics?highlights={}", seq.get());
    let (_, read) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(read["headlines"], json!([names[10], names[5]]), "the command's metrics the run declared, in its order");
    // The list's column follows the same command, so a run's cell and its page's first tile name one metric (Astra's
    // review of the results, F5).
    let listed = format!("/api/runs?site={site}&highlights={}", seq.get());
    let (_, list) = send(&stack.router, "GET", &listed, None).await;
    assert_eq!(list["items"][0]["headline"]["metric"], names[10], "{list}");
    let (_, list) = send(&stack.router, "GET", &format!("/api/runs?site={site}"), None).await;
    assert_eq!(list["items"][0]["headline"]["metric"], names[0], "without the command, descriptor order");

    // The project's pins come first.
    let project: String =
        sqlx::query_scalar("SELECT project FROM scenarios").fetch_one(&scratch.pool).await.expect("the project");
    let pins = json!({ "metrics": [additive.metric, "never_declared", gauge.metric] });
    let (status, pinned) =
        send(&stack.router, "PUT", &format!("/api/projects/{project}/metric-pins"), Some(pins)).await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    let (_, read) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(read["headlines"], json!([additive.metric, gauge.metric]), "the pins the run declared, in pin order");
    let (_, list) = send(&stack.router, "GET", &format!("/api/runs?site={site}"), None).await;
    let headline = &list["items"][0]["headline"];
    assert_eq!(
        (headline["metric"].as_str(), headline["value"].as_f64()),
        (Some(additive.metric.as_str()), Some(last)),
        "{list}"
    );

    let nine = json!({ "metrics": (0..9).map(|n| names[n].clone()).collect::<Vec<_>>() });
    let (status, _) = send(&stack.router, "PUT", &format!("/api/projects/{project}/metric-pins"), Some(nine)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = send(
        &stack.router,
        "PUT",
        "/api/projects/01KN0TJZANP5N0PGJD8F7A3FZZ/metric-pins",
        Some(json!({ "metrics": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, cleared) =
        send(&stack.router, "PUT", &format!("/api/projects/{project}/metric-pins"), Some(json!({ "metrics": [] })))
            .await;
    assert_eq!((status, cleared), (StatusCode::OK, json!({ "metrics": [] })));
    scratch.close().await;
}

/// Section 3.13's results of an attempt that stored none: its metrics answer no item and no headline, never a window
/// refused for a run that reached nothing, and its series `no-recording`.
#[tokio::test]
async fn an_attempt_that_stored_no_results_answers_no_metrics_and_no_series() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let scenario = scenario(&stack, "Peak hour").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (queued, hash) = plan(&stack, &made.id).await;
    submit(&stack, &queued, &hash).await;
    let run = &queued.runs[0].run;
    let (status, read) = send(&stack.router, "GET", &format!("/api/runs/{run}/metrics"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!((read["items"].clone(), read["headlines"].clone()), (json!([]), json!([])));
    let metric = canned().0.metrics[0].metric.clone();
    let (status, problem) = send(&stack.router, "GET", &format!("/api/runs/{run}/series?metric={metric}"), None).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::CONFLICT, Some("no-recording")));
    scratch.close().await;
}
