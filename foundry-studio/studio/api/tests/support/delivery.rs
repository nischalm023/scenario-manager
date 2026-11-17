//! What the delivery tests share (spec 08 section 3.5): the routes a runner delivers its recording and its report on,
//! the report it sends, what the attempt's row accepted and what the run page downloads, and the endings they write.

use axum::body::Body;
use axum::http::StatusCode;
use desunits::SimTime;
use run_invocation::AttemptKey;
use serde_json::{Value, json};
use studio_protocol::{EndedBy, Ending, RunState};

use super::runs::RunStack;

/// The paths of the attempt `key`'s recording route and results route.
pub fn routes_of(key: &AttemptKey) -> (String, String) {
    let attempt = format!("/runner/v1/attempts/{}/{}", key.run, key.n.0);
    (format!("{attempt}/recording"), format!("{attempt}/results"))
}

/// A report of a finished run naming `recording`, as a runner of the build `commit` sends it.
pub fn report(recording: &Value, commit: Option<&str>) -> Vec<u8> {
    json!({
        "v": 1,
        "ending": { "state": "finished", "ended_by": "simulator", "stop_reason": "SimTimeLimit", "stop_detail": null,
            "sim_ms": 600_000 },
        "metrics": null, "series": null, "recording": recording, "runner_commit": commit, "runner_dirty": false,
    })
    .to_string()
    .into_bytes()
}

/// The bytes the run's latest attempt stored as its recording, as the run page downloads them.
pub async fn downloaded(stack: &RunStack, key: &AttemptKey) -> (StatusCode, Vec<u8>) {
    let download = super::from_gate("GET", &format!("/api/runs/{}/recording", key.run), "u-1", "");
    let (status, _, bytes) = super::call(&stack.router, download.body(Body::empty()).unwrap()).await;
    (status, bytes)
}

/// What the attempt's row says it accepted: its key and size, none before it accepts one.
pub async fn accepted(stack: &RunStack, key: &AttemptKey) -> (Option<String>, Option<i64>) {
    sqlx::query_as("SELECT object_key, recording_size_bytes FROM attempts WHERE run_id = $1 AND n = $2")
        .bind(key.run.as_str())
        .bind(i32::try_from(key.n.0).unwrap())
        .fetch_one(&stack.gate.app.pool)
        .await
        .expect("the attempt's row")
}

/// A finished ending, as a runner's simulator writes one.
pub fn finished() -> Ending {
    Ending {
        state: RunState::Finished,
        ended_by: EndedBy::Simulator,
        stop_reason: "SimTimeLimit".to_owned(),
        stop_detail: None,
        sim_ms: SimTime::ms(600_000),
    }
}

/// The ending the reconciler writes for a cancel its runner never confirmed.
pub fn cancelled() -> Ending {
    let stop_reason = "cancelled; the runner did not confirm".to_owned();
    Ending {
        state: RunState::Cancelled,
        ended_by: EndedBy::Person,
        stop_reason,
        stop_detail: None,
        sim_ms: SimTime::ZERO,
    }
}
