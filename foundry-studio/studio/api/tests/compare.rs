//! Spec 08 section 8's statement 11 on synthetic rows (a seed difference inside one experiment is the experiment's; a
//! changeset difference across submissions is not, warns, and names the inputs read from that scope that differ;
//! runs over different report windows combine no window value), section 3.12's provenance that never warns, inputs
//! classified on their canonical values (Astra's review of the results, F2), and, against the database, a run's
//! inputs as values with its generated workload (F14), the comparison of a sweep's five finished stub runs (only
//! orders per hour differs, by design), an unfinished run left uncompared, and the recording download's name.

mod support;

use axum::body::Body;
use axum::http::StatusCode;
use desunits::SimTime;
use serde_json::json;
use studio_api::config::RunConfig;
use studio_api::results::{Design, compare_inputs, compare_metrics};
use studio_protocol::{
    Aggregation, DurationClock, InputRow, InputSection, MetricRow, MetricUnit, RunInputs, ScopePath, SimWindow, Ulid,
    ValueBasis,
};

use support::results::{CANNED_RECORDING, finished};
use support::runs::{PERSON, experiment, plan, scenario, send, stack, submit};

const FLOW: &str = "customer/demo/site/lshape-6/flow";
const RATE: &str = "/workload/generator/orders_per_hour";

/// A row saying `value`, its canonical value the words themselves, as the API's would be but for rounding; a run
/// without a `plan-differs` finding states none, as the API's row does.
fn row(section: InputSection, field: &str, label: &str, value: &str, scope: Option<&str>) -> InputRow {
    let none = field == "plan-differs" && value == "no";
    InputRow {
        section,
        field: field.to_owned(),
        label: label.to_owned(),
        value: value.to_owned(),
        canonical: if none { "null".to_owned() } else { json!(value).to_string() },
        all_in: None,
        from: "a test".to_owned(),
        held: None,
        varied: false,
        scope: scope.map(|scope| scope.parse::<ScopePath>().expect("a scope")),
        clock: None,
        unit: None,
        number: value.split(' ').next().and_then(|first| first.parse().ok()),
        changeset: None,
    }
}

/// A run's inputs: the seed, a flow parameter read from the flow scope, the flow scope's changeset, its exactness,
/// its compiler, and whether it differs from its check.
fn inputs(run: &Ulid, seed: &str, pick: &str, flow: &str, exact: &str, differs: &str) -> RunInputs {
    let rows = vec![
        row(InputSection::Scenario, "/run_controls/seed", "Simulation seed", seed, None),
        row(InputSection::Flow, "pick/pick_sim_ms", "Pick time", pick, Some(FLOW)),
        row(InputSection::Provenance, &format!("read:{FLOW}"), FLOW, flow, Some(FLOW)),
        row(InputSection::Provenance, "exact", "Read exact", exact, None),
        row(InputSection::Provenance, "compiler", "Compiler", "0.1.0", None),
        row(InputSection::Provenance, "plan-differs", "Differs from the check", differs, None),
    ];
    RunInputs { run: run.clone(), complete: false, rows, workload: None, text: String::new() }
}

/// Statement 11: two repeats of one experiment differ in their seed alone, which is the experiment's design and warns
/// of nothing; the rest is the same.
#[test]
fn a_seed_difference_inside_one_experiment_is_the_experiments() {
    let (a, b) = (support::ulid(), support::ulid());
    let first = inputs(&a, "0", "7.5 s", "changeset 2", "yes", "no");
    let second = inputs(&b, "1", "7.5 s", "changeset 2", "yes", "no");
    let design = Design { varied: Vec::new(), repeats: true };
    let (differs, same) = compare_inputs(&[(a.clone(), &first), (b.clone(), &second)], Some(&design));
    assert_eq!(differs.len(), 1, "{differs:?}");
    assert_eq!(
        (differs[0].field.as_str(), differs[0].by_experiment, differs[0].warns),
        ("/run_controls/seed", true, false)
    );
    let values: Vec<Option<&str>> = differs[0].values.iter().map(|value| value.value.as_deref()).collect();
    assert_eq!(values, [Some("0"), Some("1")]);
    assert_eq!(same.len(), 5);
}

/// Statement 11: runs of two submissions that read the flow at different changesets differ outside any experiment,
/// which warns, and the changeset's row names the input read from that scope that differs; a read's exactness and
/// the compiler are provenance and never warn; a run that differs from its check warns even beside one that does not.
#[test]
fn a_changeset_difference_across_submissions_warns_and_names_what_it_changed() {
    let (a, b) = (support::ulid(), support::ulid());
    let first = inputs(&a, "0", "7.5 s", "changeset 2", "yes", "no");
    let mut second =
        inputs(&b, "0", "8 s", "changeset 3", "no: read below a moved head", "yes: it compiled another plan");
    second.rows.retain(|row| row.field != "compiler");
    let (differs, same) = compare_inputs(&[(a.clone(), &first), (b.clone(), &second)], None);
    let by_field =
        |field: &str| differs.iter().find(|differ| differ.field == field).unwrap_or_else(|| panic!("{field} differs"));
    let read = by_field(&format!("read:{FLOW}"));
    assert_eq!((read.by_experiment, read.warns), (false, true));
    assert_eq!(read.changed, ["Pick time"], "the flow changeset changed the pick time");
    assert!(by_field("pick/pick_sim_ms").warns);
    assert_eq!((by_field("exact").warns, by_field("compiler").warns), (false, false), "provenance never warns");
    assert_eq!(by_field("compiler").values[1].value, None, "a row one run lacks reads as not recorded");
    assert!(by_field("plan-differs").warns);
    assert!(differs.iter().any(|differ| differ.warns), "the heading warns");
    assert_eq!(same.iter().map(|same| same.field.as_str()).collect::<Vec<_>>(), ["/run_controls/seed"]);
}

/// Astra's review of the results, F2 (spec 08 section 3.12, a row is the same "only if every run has an equal value";
/// spec 07 section 3.2's numeric fields): two runs whose rates both say 3 orders an hour, rounded, still differ, as
/// their canonical values do, and the comparison then says each exactly; equal canonical values are the same.
#[test]
fn inputs_that_round_alike_still_differ_and_are_said_exactly() {
    let (a, b) = (support::ulid(), support::ulid());
    let rate = |run: &Ulid, canonical: &str| {
        let mut said = row(InputSection::Scenario, RATE, "Orders per hour", "3 orders an hour", None);
        said.canonical = canonical.to_owned();
        RunInputs { run: run.clone(), complete: false, rows: vec![said], workload: None, text: String::new() }
    };
    let (first, second) = (rate(&a, "3.0001"), rate(&b, "3.0002"));
    let (differs, same) = compare_inputs(&[(a.clone(), &first), (b.clone(), &second)], None);
    assert!(same.is_empty(), "{same:?}");
    let values: Vec<Option<&str>> = differs[0].values.iter().map(|value| value.value.as_deref()).collect();
    assert_eq!(values, [Some("3.0001 orders an hour"), Some("3.0002 orders an hour")]);
    let again = rate(&b, "3.0001");
    let (differs, same) = compare_inputs(&[(a.clone(), &first), (b, &again)], None);
    assert_eq!((differs.len(), same[0].value.as_str()), (0, "3 orders an hour"));
}

fn metric(name: &str, aggregation: Aggregation, from: u64, to: u64, value: f64) -> MetricRow {
    MetricRow {
        metric: name.to_owned(),
        label: name.to_owned(),
        description: String::new(),
        unit: MetricUnit::Count,
        clock: None,
        aggregation,
        denominator: None,
        value: Some(value),
        window: SimWindow { from_sim_ms: SimTime::ms(from), to_sim_ms: SimTime::ms(to) },
        basis: ValueBasis::WindowDifference,
    }
}

/// A duration's clock rides the comparison, so the page states the run's wall clock as the wall clock's; runs that
/// state one metric on different clocks combine no value, saying why.
#[test]
fn a_duration_is_compared_on_its_clock_and_only_on_one() {
    let (a, b, c) = (support::ulid(), support::ulid(), support::ulid());
    let timed = |clock, value| MetricRow {
        unit: MetricUnit::DurationMs,
        clock: Some(clock),
        ..metric("run_wall_clock", Aggregation::Interval, 0, 10_800_000, value)
    };
    let names = vec!["run_wall_clock".to_owned()];
    let both =
        [(a.clone(), Ok(vec![timed(DurationClock::Wall, 600.0)])), (b, Ok(vec![timed(DurationClock::Wall, 700.0)]))];
    let compared = compare_metrics(&names, &both);
    assert_eq!((compared[0].comparable, compared[0].clock), (true, Some(DurationClock::Wall)));
    let mixed = [(a, Ok(vec![timed(DurationClock::Wall, 600.0)])), (c, Ok(vec![timed(DurationClock::Sim, 600.0)]))];
    let compared = compare_metrics(&names, &mixed);
    assert!(!compared[0].comparable);
    assert_eq!(compared[0].reason.as_deref(), Some("the runs state it on different clocks"));
}

/// Statement 11: runs whose report windows differ combine no window value, each saying why; a whole-run value over
/// the same span stays comparable; a run whose window was not recorded makes every metric not comparable.
#[test]
fn runs_over_different_report_windows_combine_no_window_value() {
    let (a, b, c) = (support::ulid(), support::ulid(), support::ulid());
    let names = vec!["picks".to_owned(), "orders".to_owned(), "fill".to_owned()];
    let first = vec![
        metric("picks", Aggregation::Additive, 0, 10_800_000, 3000.0),
        metric("orders", Aggregation::Additive, 0, 10_800_000, 1000.0),
        metric("fill", Aggregation::SnapshotGauge, 0, 10_800_000, 0.8),
    ];
    let second = vec![
        metric("picks", Aggregation::Additive, 1_800_000, 10_800_000, 2600.0),
        metric("orders", Aggregation::Additive, 1_800_000, 10_800_000, 860.0),
        metric("fill", Aggregation::SnapshotGauge, 0, 10_800_000, 0.7),
    ];
    let compared = compare_metrics(&names, &[(a.clone(), Ok(first.clone())), (b.clone(), Ok(second))]);
    let comparable: Vec<(&str, bool)> = compared.iter().map(|one| (one.metric.as_str(), one.comparable)).collect();
    assert_eq!(comparable, [("picks", false), ("orders", false), ("fill", true)]);
    assert!(
        compared[0].reason.as_deref().is_some_and(|reason| reason.contains("different windows")),
        "{:?}",
        compared[0].reason
    );
    let unrecorded = compare_metrics(&names, &[(a, Ok(first)), (c, Err("its window is not recorded".to_owned()))]);
    assert!(unrecorded.iter().all(|one| !one.comparable));
}

fn config() -> RunConfig {
    RunConfig::local("http://127.0.0.1:3101", "studio-simrunner:dev")
}

/// Section 3.12's inputs of one run of a sweep: every scenario field as the run read it, its rate varied by the sweep
/// and marked so, then the provenance, the generated workload's request, seed and digest as its manifest names them
/// (Astra's review of the results, F14); `complete: false`; and the same rows as plain text.
#[tokio::test]
async fn a_runs_inputs_name_each_field_its_value_and_where_it_came_from() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_, runs) = finished(&stack, "Peak hour", &[200, 250, 300]).await;
    let (status, inputs) = send(&stack.router, "GET", &format!("/api/runs/{}/inputs", runs[1]), None).await;
    assert_eq!(status, StatusCode::OK, "{inputs}");
    assert_eq!(inputs["complete"], false);
    let rows = inputs["rows"].as_array().expect("the rows");
    let field = |field: &str| {
        rows.iter().find(|row| row["field"] == field).unwrap_or_else(|| panic!("{field} is a row")).clone()
    };
    let rate = field("/workload/generator/orders_per_hour");
    assert_eq!(
        (rate["value"].as_str(), rate["varied"].as_bool(), rate["number"].as_f64()),
        (Some("250 orders an hour"), Some(true), Some(250.0))
    );
    assert_eq!(
        (rate["from"].as_str(), rate["held"].as_str()),
        (Some("varied by the sweep, value 2 of 3"), Some("300 orders an hour"))
    );
    let duration = field("/run_controls/duration_sim_ms");
    assert_eq!((duration["value"].as_str(), duration["clock"].as_str()), (Some("4 hours of simtime"), Some("sim")));
    assert_eq!(field("/run_controls/report_window/to_sim_ms")["value"], "the end of the run");
    for provenance in
        ["scenario", "exact", "workload", "compile", "compiler", "plan-differs", "runner.image", "runner.commit"]
    {
        assert_eq!(field(provenance)["section"], "provenance");
    }
    assert_eq!(field("plan-differs")["value"], "no");
    assert_eq!(field("runner.commit")["value"], "c331dee0f, clean");
    let workload = &inputs["workload"];
    assert_eq!((workload["seed"].as_u64(), workload["generator"].as_str()), (Some(0), Some("0.1.0")));
    assert_eq!(workload["sha256"].as_str().map(str::len), Some(64), "the digest whole");
    assert_eq!(workload["spec"]["v"], 1, "the generator request the runner sent: {workload}");
    assert!(
        rows.iter().any(|row| row["field"].as_str().is_some_and(|field| field.starts_with("read:"))),
        "every scope read"
    );
    let request = support::from_gate("GET", &format!("/api/runs/{}/inputs?format=text", runs[1]), PERSON, "")
        .body(Body::empty())
        .expect("a request");
    let (status, headers, text) = support::call(&stack.router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    let text = String::from_utf8(text).expect("text");
    assert!(text.contains("Orders per hour: 250 orders an hour (varied by the sweep, value 2 of 3;"), "{text}");
    assert!(text.contains("Generator request: {\"brings_stock_in\":true,"), "{text}");
    assert_eq!(text, inputs["text"].as_str().expect("the same text"));
    scratch.close().await;
}

/// Section 3.12 on the five finished stub runs of a sweep: the inputs that differ come first, only orders per hour
/// among the inputs, by the experiment's design, with the workload it generated; nothing warns; the rest is the same;
/// each headline metric is comparable over the one report window; and a queued run is left uncompared.
#[tokio::test]
async fn a_sweeps_five_runs_differ_only_in_orders_per_hour_by_design() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_, runs) = finished(&stack, "Peak hour", &[200, 250, 300, 350, 400]).await;
    let ids: Vec<String> = runs.iter().map(ToString::to_string).collect();
    let (status, reply) = send(&stack.router, "GET", &format!("/api/runs/compare?ids={}", ids.join(",")), None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let differs = reply["differs"].as_array().expect("what differs");
    let inputs: Vec<&str> = differs
        .iter()
        .filter(|differ| differ["section"] != "provenance")
        .map(|differ| differ["label"].as_str().unwrap())
        .collect();
    assert_eq!(inputs, ["Orders per hour"]);
    assert!(differs.iter().all(|differ| differ["by_experiment"] == true && differ["warns"] == false), "{differs:?}");
    let numbers: Vec<f64> =
        differs[0]["values"].as_array().unwrap().iter().filter_map(|value| value["number"].as_f64()).collect();
    assert_eq!(numbers, [200.0, 250.0, 300.0, 350.0, 400.0]);
    assert!(reply["same"].as_array().is_some_and(|same| same.len() > 10));
    assert_eq!(reply["not_compared"], json!([]));
    assert_eq!(reply["reference"], ids[0]);
    assert!(reply["runs"].as_array().unwrap().iter().all(|run| run["stub"] == true));
    let metrics = reply["metrics"].as_array().expect("the metrics");
    assert_eq!(metrics.len(), 4, "the headline rule: the first four declared");
    assert!(metrics.iter().all(|metric| metric["comparable"] == true), "{metrics:?}");

    // A run still queued is listed apart and never combined.
    let scenario = scenario(&stack, "Off peak").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (queued, hash) = plan(&stack, &made.id).await;
    submit(&stack, &queued, &hash).await;
    let waiting = queued.runs[0].run.to_string();
    let uri = format!("/api/runs/compare?ids={},{},{waiting}&reference={waiting}", ids[0], ids[1]);
    let (status, reply) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["not_compared"], json!([{ "run": waiting, "state": "queued" }]));
    assert_eq!(reply["reference"], ids[0], "a reference that did not finish gives way to the first that did");
    let uri = format!("/api/runs/compare?ids={},{}&metrics=no_such_metric", ids[0], ids[1]);
    let (status, problem) = send(&stack.router, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert!(problem["detail"].as_str().is_some_and(|detail| detail.contains("no_such_metric")), "{problem}");
    for (uri, status) in [
        (format!("/api/runs/compare?ids={}", ids[0]), StatusCode::UNPROCESSABLE_ENTITY),
        (format!("/api/runs/compare?ids={},{}", ids[0], ids[0]), StatusCode::UNPROCESSABLE_ENTITY),
        (format!("/api/runs/compare?ids={},01KN0TJZANP5N0PGJD8F7A3FZZ", ids[0]), StatusCode::NOT_FOUND),
    ] {
        assert_eq!(send(&stack.router, "GET", &uri, None).await.0, status, "{uri}");
    }
    scratch.close().await;
}

/// Section 3.7's download: the attempt's recording as stored, named "{scenario} - {site} - {variant} - {date}", the
/// site by the name its root states, with the stub's canned results said; an attempt that stored none answers
/// `no-recording`.
#[tokio::test]
async fn the_recording_downloads_under_the_delivery_packages_name() {
    let scratch = scratch_or_skip!();
    let stack = stack(&scratch.pool, config()).await;
    let (_, runs) = finished(&stack, "Peak hour", &[200, 350]).await;
    let request = support::from_gate("GET", &format!("/api/runs/{}/recording", runs[1]), PERSON, "")
        .body(Body::empty())
        .expect("a request");
    let (status, headers, bytes) = support::call(&stack.router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, CANNED_RECORDING, "the recording as the runner left it");
    let disposition = headers["content-disposition"].to_str().expect("ASCII");
    assert!(disposition.starts_with("attachment; filename=\"Peak hour - L-Shape 6 - "), "{disposition}");
    assert!(
        disposition.contains(" - 350 orders per hour - ")
            && disposition.contains(" - canned by the stub runner.worldview\""),
        "{disposition}"
    );
    let scenario = scenario(&stack, "Off peak").await;
    let made = experiment(&stack, &scenario, 1, &[]).await;
    let (queued, hash) = plan(&stack, &made.id).await;
    submit(&stack, &queued, &hash).await;
    let (status, problem) =
        send(&stack.router, "GET", &format!("/api/runs/{}/recording", queued.runs[0].run), None).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::CONFLICT, Some("no-recording")));
    scratch.close().await;
}
