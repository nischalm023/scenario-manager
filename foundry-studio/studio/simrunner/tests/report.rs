//! The runner's own delivery (`door::report::deliver`): its recording sent to the door first, whole, and then the
//! report naming it as the door stored it, with the ending and the two files the run left as the run left them, whatever
//! its folder holds by then; a listener that fails once is asked again; a recording the door does not store turns the
//! ending into the platform's failure; and a report the door refuses is not delivered.

use std::fs;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::put;
use dessim::harness::{RunOverrides, prepare};
use dessim::sim_types::Counters;
use serde_json::{Value, json};
use simrunner::door::Door;
use simrunner::door::report::deliver;
use simrunner::simulate::{Left, simulate};
use simrunner::stop::RunnerStop;
use studio_protocol::{AttemptNumber, Endpoint, FORMAT, JobSpec, ResultsEnded};
use tempfile::{TempDir, tempdir};
use tokio::net::TcpListener;
use tokio::task::spawn_blocking;

const RUN: &str = "01KZAVPQNES65HV4EZADPCS7E8";
const TOKEN: &str = "report-test-token";

/// The runners' listener as these tests stand in for it: what it answers and what it received.
#[derive(Default)]
struct Listener {
    /// Answered to the next report, then 200.
    report_first: AtomicU16,
    /// Answered to every recording; 0 is 200.
    recording_answer: AtomicU16,
    recording: Mutex<Option<Vec<u8>>>,
    report: Mutex<Option<Value>>,
}

fn stored_answer(bytes: usize) -> Value {
    json!({ "key": format!("runs/{RUN}/attempts/1/recording.worldview"), "live_edge_sim_ms": 300_000, "frames": 7,
        "bytes": bytes })
}

async fn recording(
    State(listener): State<Arc<Listener>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, String) {
    assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(uri.path(), format!("/runner/v1/attempts/{RUN}/1/recording"));
    let answer = listener.recording_answer.load(Ordering::SeqCst);
    if answer != 0 {
        return (StatusCode::from_u16(answer).unwrap(), r#"{"detail":"The body is no recording."}"#.to_owned());
    }
    *listener.recording.lock().unwrap() = Some(body.to_vec());
    (StatusCode::OK, stored_answer(body.len()).to_string())
}

async fn results(
    State(listener): State<Arc<Listener>>,
    headers: HeaderMap,
    uri: Uri,
    body: String,
) -> (StatusCode, String) {
    assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(uri.path(), format!("/runner/v1/attempts/{RUN}/1/results"));
    let first = listener.report_first.swap(200, Ordering::SeqCst);
    if first != 0 && first != 200 {
        return (StatusCode::from_u16(first).unwrap(), r#"{"detail":"Not this time."}"#.to_owned());
    }
    *listener.report.lock().unwrap() = Some(serde_json::from_str(&body).expect("a JSON report"));
    (StatusCode::OK, r#"{"ended":"wrote"}"#.to_owned())
}

async fn served() -> (JobSpec, Arc<Listener>) {
    let listener = Arc::new(Listener::default());
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let spec = JobSpec {
        v: FORMAT,
        run: RUN.parse().unwrap(),
        attempt: AttemptNumber(1),
        studio: Endpoint { endpoint: format!("http://{}", socket.local_addr().unwrap()) },
        streaming: None,
    };
    let router = Router::new()
        .route("/runner/v1/attempts/{run}/{n}/recording", put(recording))
        .route("/runner/v1/attempts/{run}/{n}/results", put(results))
        .with_state(listener.clone());
    tokio::spawn(async move { axum::serve(socket, router).await.unwrap() });
    (spec, listener)
}

/// A five-minute run of the canned fixture: its folder, and what it left.
async fn simulated(spec: &JobSpec) -> (TempDir, Left) {
    let spec = spec.clone();
    spawn_blocking(move || {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned/spec.json");
        let overrides = RunOverrides {
            sim_duration: Some(Duration::from_secs(300)),
            num_threads: Some(1),
            ..RunOverrides::default()
        };
        let prepared = prepare(&fixture, &overrides).expect("the fixture prepares");
        let out = tempdir().unwrap();
        let left = simulate(prepared, out.path(), &spec.run, spec.attempt, RunnerStop::new(), Counters::new(), None)
            .expect("simulates");
        (out, left)
    })
    .await
    .unwrap()
}

/// The metrics and series the report carries are the ones the run left in memory, never read back from the folder,
/// whose copies here are deleted and corrupted before the delivery.
#[tokio::test(flavor = "multi_thread")]
async fn the_recording_goes_first_whole_and_the_report_names_it_with_what_the_run_left() {
    let (spec, listener) = served().await;
    listener.report_first.store(503, Ordering::SeqCst);
    let (out, left) = simulated(&spec).await;
    fs::remove_file(out.path().join("metrics.json")).expect("the folder's metrics deleted");
    fs::write(out.path().join("series.json"), "not the series").expect("the folder's series corrupted");
    let recording = fs::read(left.recording.as_ref().expect("the run's recording")).unwrap();
    let ended = deliver(&Door::new(&spec.studio.endpoint, TOKEN), &spec, left.clone()).await.expect("delivered");
    assert_eq!(ended, ResultsEnded::Wrote);
    assert_eq!(listener.recording.lock().unwrap().as_deref(), Some(recording.as_slice()), "the recording, whole");
    let report = listener.report.lock().unwrap().clone().expect("the report arrived, on its second try");
    assert_eq!(report["v"], 1);
    assert_eq!(report["recording"], stored_answer(recording.len()), "the recording as the door stored it");
    assert_eq!(report["ending"], serde_json::to_value(&left.ending).unwrap(), "{report}");
    assert_eq!(report["metrics"], serde_json::to_value(&left.metrics).unwrap(), "the metrics the run derived");
    assert_eq!(report["series"], serde_json::to_value(&left.series).unwrap(), "the series the run derived");
}

/// A recording the door does not store leaves the API nothing to name, so the run is the platform's failure, with
/// how far it got and the files it derived.
#[tokio::test(flavor = "multi_thread")]
async fn a_recording_the_door_does_not_store_fails_the_run_by_the_platform() {
    let (spec, listener) = served().await;
    listener.recording_answer.store(422, Ordering::SeqCst);
    let (_out, left) = simulated(&spec).await;
    deliver(&Door::new(&spec.studio.endpoint, TOKEN), &spec, left.clone()).await.expect("delivered");
    let report = listener.report.lock().unwrap().clone().expect("the report");
    assert!(report["recording"].is_null(), "{report}");
    let ending = &report["ending"];
    assert_eq!((ending["state"].as_str(), ending["ended_by"].as_str()), (Some("failed"), Some("platform")));
    let words = ending["stop_reason"].as_str().unwrap();
    assert!(words.starts_with("its recording could not be stored") && words.contains("no recording"), "{words}");
    assert_eq!(ending["sim_ms"], serde_json::to_value(left.ending.sim_ms).unwrap(), "how far it got");
    assert!(report["metrics"].is_object(), "{report}");
}

/// A report the door refuses is final and not delivered, which `run` answers as an error and the runner exits
/// non-zero with.
#[tokio::test(flavor = "multi_thread")]
async fn a_report_the_door_refuses_is_not_delivered() {
    let (spec, listener) = served().await;
    listener.report_first.store(422, Ordering::SeqCst);
    let (_out, left) = simulated(&spec).await;
    let refused = deliver(&Door::new(&spec.studio.endpoint, TOKEN), &spec, left).await.expect_err("refused");
    assert!(refused.to_string().contains("refused"), "{refused}");
    assert!(listener.report.lock().unwrap().is_none());
}
