//! SIGTERM while the runner prepares its inputs is the platform ending the attempt, at once: the preparation is
//! abandoned, here a read of the pins the door never answers, and the runner delivers a `failed` ending by the
//! platform in its words well inside a Job's grace, then exits 0, its report taken.

use std::fs;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, put};
use serde_json::Value;
use studio_protocol::{AttemptNumber, Endpoint, FORMAT, JobSpec};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::spawn_blocking;
use tokio::time::timeout;

/// What the door saw: that the pins were asked for, and the report.
#[derive(Default)]
struct Seen {
    pins_asked: Notify,
    report: Mutex<Option<Value>>,
}

/// A door that never answers the pins, so the runner waits in its preparation, and takes the report.
async fn door(seen: Arc<Seen>) -> String {
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", socket.local_addr().unwrap());
    let asked = seen.clone();
    let reported = seen.clone();
    let router = Router::new()
        .route(
            "/runner/v1/attempts/{run}/{n}",
            get(move || async move {
                asked.pins_asked.notify_one();
                std::future::pending::<StatusCode>().await
            }),
        )
        .route("/runner/v1/attempts/{run}/{n}/heartbeat", put(|| async { StatusCode::NO_CONTENT }))
        .route(
            "/runner/v1/attempts/{run}/{n}/results",
            put(move |body: String| async move {
                *reported.report.lock().unwrap() = Some(serde_json::from_str(&body).unwrap());
                (StatusCode::OK, r#"{"ended":"wrote"}"#)
            }),
        );
    tokio::spawn(async move { axum::serve(socket, router).await.unwrap() });
    endpoint
}

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_while_the_inputs_are_read_ends_the_attempt_by_the_platform_at_once() {
    let seen = Arc::new(Seen::default());
    let folder = tempdir().unwrap();
    let spec = JobSpec {
        v: FORMAT,
        run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(),
        attempt: AttemptNumber(1),
        studio: Endpoint { endpoint: door(seen.clone()).await },
        streaming: None,
    };
    let job = folder.path().join("job.json");
    fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
    let mut runner = Command::new(env!("CARGO_BIN_EXE_simrunner"))
        .arg("--job")
        .arg(&job)
        .arg("--out")
        .arg(folder.path().join("out"))
        .env("STUDIO_RUNNER_TOKEN", "terminated-test-token")
        .stdout(Stdio::null())
        .spawn()
        .expect("the runner started");
    timeout(Duration::from_secs(30), seen.pins_asked.notified()).await.expect("the runner asked for its pins");
    let sent = Command::new("kill").args(["-TERM", &runner.id().to_string()]).status().expect("kill ran");
    assert!(sent.success(), "the runner was sent SIGTERM");
    let status = timeout(Duration::from_secs(20), spawn_blocking(move || runner.wait()))
        .await
        .expect("the runner ended inside the grace, its preparation abandoned")
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(0), "its report was taken");
    let report = seen.report.lock().unwrap().clone().expect("the runner reported");
    let ending = &report["ending"];
    assert_eq!((ending["state"].as_str(), ending["ended_by"].as_str()), (Some("failed"), Some("platform")));
    assert_eq!(ending["stop_reason"], "terminated by the platform", "{report}");
}
