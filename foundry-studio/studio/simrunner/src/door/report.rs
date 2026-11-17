//! Within `door`, `report` is the runner delivering its own run once it is over, because a Job's pod keeps no folder
//! the API could read: first its recording, sent to the door, which stores it in the run store so the runner holds no
//! credential for the store (ruling D174); then its report, the ending, the two files and who the runner is, naming
//! the recording by the receipt the door answered. What it delivers is what the run left as it ended, never read back
//! from the folder, which keeps a copy for the local invoker's fallback. A recording the door did not take turns the
//! ending into a `failed` one by the platform, so the report never names a recording the API does not hold; a report
//! the door never took is an error, with which the runner exits non-zero.

use std::env::var;

use studio_protocol::{
    EndedBy, Ending, FORMAT, JobSpec, RecordingStored, ResultsAccepted, ResultsEnded, ResultsReport, RunState,
};

use crate::attempt::Stop;
use crate::simulate::Left;

use super::{Carried, Door};

/// Deliver what the attempt `spec` names left: its recording, then its report.
pub async fn deliver(door: &Door, spec: &JobSpec, left: Left) -> Result<ResultsEnded, Stop> {
    let Left { mut ending, recording, metrics, series } = left;
    let attempt = format!("/runner/v1/attempts/{}/{}", spec.run, spec.attempt.0);
    let stored = match recording {
        Some(path) => {
            match door.deliver::<RecordingStored>(&format!("{attempt}/recording"), Carried::File(&path)).await {
                Ok(stored) => Some(stored),
                Err(unstored) => {
                    eprintln!("simrunner: its recording was not stored: {unstored}");
                    ending = Ending {
                        state: RunState::Failed,
                        ended_by: EndedBy::Platform,
                        stop_reason: format!("its recording could not be stored: {unstored}"),
                        stop_detail: None,
                        sim_ms: ending.sim_ms,
                    };
                    None
                }
            }
        }
        None => None,
    };
    let report = ResultsReport {
        v: FORMAT,
        ending,
        metrics,
        series,
        recording: stored,
        runner_commit: var("DESVIS_RUN_COMMIT").ok(),
        runner_dirty: var("DESVIS_RUN_DIRTY").ok().map(|dirty| dirty == "true"),
    };
    let body = serde_json::to_vec(&report).expect("a report is JSON");
    let accepted: ResultsAccepted = door.deliver(&format!("{attempt}/results"), Carried::Json(&body)).await?;
    Ok(accepted.ended)
}
