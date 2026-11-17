//! What the result tests share (brief I1.10): simrunner's canned results, three hours of simtime and thirty
//! seconds from a repository world, read from its fixture; and runs submitted on the run stack, each reserved, handed
//! in by a faithful stub's manifest through the runners' door and ended `finished` with the canned results, as the
//! local invoker settles simrunner's exit.

use std::collections::BTreeMap;
use std::env;
use std::fs;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use run_invocation::{Creator, RunStore, Seats, Token};
use serde_json::{Value, json};
use studio_api::runner_routes;
use studio_api::runs::{PgRunStore, Recording, Results, ResultsSource};
use studio_protocol::{
    EndedBy, Ending, Expectation, MetricsFile, Plan, RunState, ScopePath, Seq, SeriesFile, Ulid, WallClock,
};
use wps_client::wps::Wps;

use super::runs::{RunStack, experiment, plan, scenario, send, submit};

pub const CANNED_METRICS: &[u8] = include_bytes!("../../../simrunner/fixtures/canned/metrics.json");
pub const CANNED_SERIES: &[u8] = include_bytes!("../../../simrunner/fixtures/canned/series.json");
pub const CANNED_RECORDING: &[u8] = include_bytes!("../../../simrunner/fixtures/canned/recording.worldview");
/// Another whole recording, a viewer test's, for the tests that need two of other bytes.
pub const OTHER_RECORDING: &[u8] =
    include_bytes!("../../../viewer/tests/fixtures/minimal_easyloop_flow/recording.worldview");
pub const STUB_SENTENCE: &str = "stub runner: canned results; no simulation ran";

pub fn canned() -> (MetricsFile, SeriesFile) {
    let metrics = serde_json::from_slice(CANNED_METRICS).expect("the canned metrics.json reads");
    let series = serde_json::from_slice(CANNED_SERIES).expect("the canned series.json reads");
    (metrics, series)
}

/// Runs of a new scenario `name` on the stack's site, a sweep of orders per hour over `values` when given, each
/// ended `finished` with the canned results behind a stub's manifest; in position order.
pub async fn finished<S: Wps>(stack: &RunStack<S>, name: &str, values: &[u32]) -> (Plan, Vec<Ulid>) {
    let scenario = scenario(stack, name).await;
    let made = experiment(stack, &scenario, 1, values).await;
    let (plan, hash) = plan(stack, &made.id).await;
    submit(stack, &plan, &hash).await;
    let store = PgRunStore::new(stack.gate.app.clone());
    let runners = runner_routes(stack.gate.app.clone());
    let (metrics, series) = canned();
    let recording = env::temp_dir().join(format!("studio-canned-{}.worldview", super::ulid()));
    fs::write(&recording, CANNED_RECORDING).expect("the canned recording, written where the store reads it");
    let mut runs = Vec::new();
    for _ in &plan.runs {
        let reserved =
            store.reserve(Seats(12), &Creator::mint()).await.expect("a reservation").expect("a waiting attempt");
        let key = reserved.key;
        let manifest = stub_manifest(stack, &runners, &plan, &key.run, &reserved.token).await;
        let uri = format!("/runner/v1/attempts/{}/1/manifest", key.run);
        let (status, answer) = door(&runners, "PUT", &uri, &reserved.token, Some(manifest)).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let ending = Ending {
            state: RunState::Finished,
            ended_by: EndedBy::Simulator,
            stop_reason: "SimTimeLimit".to_owned(),
            stop_detail: Some(STUB_SENTENCE.to_owned()),
            sim_ms: series.end_sim_ms,
        };
        let results = Results {
            ending,
            metrics: Some(metrics.clone()),
            series: Some(series.clone()),
            recording: Some(Recording::File { path: recording.clone() }),
            runner_commit: Some("c331dee0f".to_owned()),
            runner_dirty: Some(false),
            source: ResultsSource::Folder,
        };
        store.take_results(&key, results).await.expect("the canned results stored");
        runs.push(key.run);
    }
    (plan, runs)
}

/// A request on the runners' listener with the attempt's token.
async fn door(router: &Router, method: &str, uri: &str, token: &Token, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", token.expose()));
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, _, answer) = super::call(router, request.body(body).expect("a request")).await;
    (status, serde_json::from_slice(&answer).unwrap_or(Value::Null))
}

/// The manifest simrunner would hand in for `run`: what its pins name, the check's digests, `stub` set.
async fn stub_manifest<S: Wps>(stack: &RunStack<S>, runners: &Router, plan: &Plan, run: &Ulid, token: &Token) -> Value {
    let (status, pins) = door(runners, "GET", &format!("/runner/v1/attempts/{run}/1"), token, None).await;
    assert_eq!(status, StatusCode::OK, "{pins}");
    let expected: Expectation = serde_json::from_value(pins["expected"].clone()).expect("the expectation");
    // ordered: the exports are listed in scope order, as the manifest lists its reads.
    let reads: BTreeMap<ScopePath, Seq> = serde_json::from_value(pins["reads"].clone()).expect("reads");
    let exports: Vec<Value> = reads
        .iter()
        .map(|(scope, seq)| json!({ "scope": scope, "seq": seq, "sha256": "e".repeat(64), "bytes": 1, "exact": true }))
        .collect();
    let (_, version) = send(&stack.router, "GET", &format!("/api/scenarios/{}", plan.scenario.id), None).await;
    json!({
        "v": 1, "run": run, "attempt": 1, "plan": plan.id, "experiment": plan.experiment,
        "scenario": pins["scenario"], "cut": pins["cut"], "reads": reads, "exports": exports,
        "world": {
            "sha256": expected.world_sha256, "scopes": reads, "walked": "annotations", "exact": true,
            "exact_reason": null, "assembled_by": "server", "derived_by": "server", "primary_frame": null,
        },
        "variant": [],
        "workload": {
            "spec": { "v": 1, "generator": version["version"]["content"]["workload"]["generator"],
                      "duration_sim_ms": pins["duration_sim_ms"], "slots": 0, "brings_stock_in": true },
            "seed": 0, "generator": "0.1.0", "sha256": expected.workload_sha256, "bytes": 1,
        },
        "compile": { "v": 1, "compiler": expected.compiler, "digest": expected.compile_sha256, "expected": expected,
                     "differs": [] },
        "seed": pins["seed"], "duration_sim_ms": pins["duration_sim_ms"], "report_window": pins["report_window"],
        "runner_image": pins["runner"]["image"], "fetched_at": WallClock::now(), "stub": true,
    })
}
