//! Within `studio-protocol`, `run` is a simulation run as the conversation hears of it (spec 08 section 3.6, through
//! spec 01's `RunEvents`): each change of a run's state, a finding on a run that does not stop it, a submission's
//! end, and a run's live progress; and the words those bodies share, a run's place in its submission, its attempt
//! number and who ended it. Every `sim_ms` is simtime.

use std::collections::BTreeMap;

use desunits::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::{ScopePath, Seq, Ulid};
use crate::scenario::RunCount;

/// A run's place in its submission and in the queue, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct RunPosition(pub u32);

/// An attempt's number within its run, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct AttemptNumber(pub u32);

/// A run's state; `cancelled`, `failed`, `partial` and `finished` end it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "run.ts")]
pub enum RunState {
    Queued,
    Running,
    Finished,
    Cancelled,
    Failed,
    Partial,
}

impl RunState {
    pub fn ended(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

/// Who ended an attempt, which decides whether a retry is offered (spec 08 section 3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "run.ts")]
pub enum EndedBy {
    Simulator,
    Person,
    Platform,
}

/// `run.status`'s body: one change of a run's state, its place in its submission, the attempt, the state it left,
/// how far it simulated, and, once it ended, who ended it, why, and the detail beneath its state words; with the
/// scenario's name and the run's duration, so a card can say "of Peak hour" and "of 3 h".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct RunStatus {
    pub run: Ulid,
    pub submission: Ulid,
    pub position: RunPosition,
    pub of: RunCount,
    pub attempt: AttemptNumber,
    pub state: RunState,
    pub from: Option<RunState>,
    #[ts(type = "number")]
    pub sim_ms: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ended_by: Option<EndedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stop_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scenario: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub duration_sim_ms: Option<SimDuration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "run.ts")]
pub enum FindingKind {
    PlanDiffers,
    ReadDiffers,
    CompilerDiffers,
}

/// What a finding compared: the compile digest, the world's or the workload's, the changesets read, the read's
/// exactness, or the compiler's version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "run.ts")]
pub enum FindingPart {
    Compile,
    World,
    Workload,
    Reads,
    Exact,
    Compiler,
}

/// One side of a finding: what the check recorded, or what the runner reported. A compile digest is null while no
/// flow compiler writes a composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct FindingSide {
    pub compile_sha256: Option<String>,
    pub world_sha256: String,
    pub workload_sha256: String,
    pub compiler: String,
    // ordered: reads are listed in scope order.
    pub reads: BTreeMap<ScopePath, Seq>,
    pub exact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exact_reason: Option<String>,
}

/// A discrepancy recorded on an attempt that does not stop it (spec 08 section 3.5), its parts, both sides and the
/// sentence spec 08 section 7 writes for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct Finding {
    pub kind: FindingKind,
    pub differs: Vec<FindingPart>,
    pub expected: FindingSide,
    pub got: FindingSide,
    pub sentence: String,
}

/// `run.finding`'s body: a discrepancy recorded on one attempt of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct RunFinding {
    pub run: Ulid,
    pub submission: Ulid,
    pub position: RunPosition,
    pub of: RunCount,
    pub attempt: AttemptNumber,
    pub finding: Finding,
}

/// How many runs ended in each way: a submission's once all of them have, or the deployment's over a span of wall
/// clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct EndingCounts {
    pub finished: RunCount,
    pub cancelled: RunCount,
    pub failed: RunCount,
    pub partial: RunCount,
}

/// `submission.ended`'s body: how many runs of the submission ended in each way, once all have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct SubmissionEnded {
    pub submission: Ulid,
    pub of: RunCount,
    pub counts: EndingCounts,
}

/// One metric's value so far, null before the run has one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct ProgressValue {
    pub metric: String,
    pub label: String,
    pub value: Option<f64>,
}

/// `run.progress`'s body, a live-only item at most once per run per 5 s of wall clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "run.ts")]
pub struct RunProgress {
    pub run: Ulid,
    pub attempt: AttemptNumber,
    #[ts(type = "number")]
    pub sim_ms: SimTime,
    pub values: Vec<ProgressValue>,
}
