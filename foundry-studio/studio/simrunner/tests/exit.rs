//! The runner's exit status says whether what it left reached the API: 0 once its report is taken, whatever the ending
//! says, and 1 when it is not, so the platform's view of a Job whose results never arrived says the runner failed,
//! while its ending stays in its folder. The door here refuses the attempt's pins, which ends it before simtime.

use std::fs;
use std::process::Command;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, put};
use studio_protocol::{AttemptNumber, Ending, Endpoint, FORMAT, JobSpec, RunState};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::task::spawn_blocking;

/// A door that refuses the attempt's pins, answers its report with `report` and lets its heartbeat be.
async fn door(report: StatusCode) -> String {
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", socket.local_addr().unwrap());
    let router = Router::new()
        .route("/runner/v1/attempts/{run}/{n}", get(|| async { (StatusCode::NOT_FOUND, r#"{"detail":"No pins."}"#) }))
        .route("/runner/v1/attempts/{run}/{n}/heartbeat", put(|| async { StatusCode::NO_CONTENT }))
        .route("/runner/v1/attempts/{run}/{n}/results", put(move || async move { (report, r#"{"ended":"wrote"}"#) }));
    tokio::spawn(async move { axum::serve(socket, router).await.unwrap() });
    endpoint
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_taken_exits_0_and_one_refused_exits_1_its_ending_left_in_the_folder() {
    for (report, exit) in [(StatusCode::OK, 0), (StatusCode::UNPROCESSABLE_ENTITY, 1)] {
        let folder = tempdir().unwrap();
        let spec = JobSpec {
            v: FORMAT,
            run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(),
            attempt: AttemptNumber(1),
            studio: Endpoint { endpoint: door(report).await },
            streaming: None,
        };
        let job = folder.path().join("job.json");
        fs::write(&job, serde_json::to_vec(&spec).unwrap()).unwrap();
        let out = folder.path().join("out");
        let mut runner = Command::new(env!("CARGO_BIN_EXE_simrunner"));
        runner.arg("--job").arg(&job).arg("--out").arg(&out).env("STUDIO_RUNNER_TOKEN", "exit-test-token");
        let status = spawn_blocking(move || runner.status()).await.unwrap().expect("the runner ran");
        assert_eq!(status.code(), Some(exit), "a report answered {report}");
        let ending: Ending = serde_json::from_slice(&fs::read(out.join("ending.json")).unwrap()).unwrap();
        assert_eq!(ending.state, RunState::Failed, "the refused pins ended it: {ending:?}");
    }
}
