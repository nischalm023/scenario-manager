//! Within `studio-protocol`, `runs` is what spec 08 section 3.7's run routes answer: a run as the Runs panel lists
//! it (`RunSummary`) and opens it (`RunDetail`, with every attempt), the queue, the deployment's run counts
//! (`RunStats`), a cancel's outcome, and the runner images a plan may name (ruling D78; one local image in the first
//! cut). The agent's run tools read the same shapes.
//! Every `_sim_ms` is simtime, every `_at` wall clock, every `_wall_ms` a wall-clock duration.

use std::collections::BTreeMap;

use desunits::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::WallClock;
use crate::grammar::{BranchName, Pin, ScopePath, Seq, Ulid};
use crate::plan::{Expectation, PlanRunner};
use crate::results::MetricRow;
use crate::run::{AttemptNumber, EndedBy, EndingCounts, Finding, FindingKind, ProgressValue, RunPosition, RunState};
use crate::scenario::{ReportWindow, RunCount, Seed, VersionNumber};

/// A wall-clock duration in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct WallMs(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunScenario {
    pub id: Ulid,
    pub version: VersionNumber,
    pub name: String,
}

/// One varied input as a run states it: the field's label and its value in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct VariedValue {
    pub label: String,
    pub value: String,
}

/// What a queued run waits for: a seat, with its place in the queue, or its runner, holding a seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "runs.ts")]
pub enum WaitingFor {
    Seat,
    Runner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct Waiting {
    #[serde(rename = "for")]
    pub for_: WaitingFor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub place_in_queue: Option<RunPosition>,
}

/// How far a run simulated, and its discovered running values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunProgressRead {
    #[ts(type = "number")]
    pub sim_ms: SimTime,
    pub values: Vec<ProgressValue>,
}

/// Whether the attempt's recording is stored, and how far it reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RecordingRead {
    pub stored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub live_edge_sim_ms: Option<SimTime>,
}

/// A run as a list shows it: its latest attempt's state stands for the run's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunSummary {
    pub id: Ulid,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub submission: Ulid,
    pub position: RunPosition,
    pub of: RunCount,
    pub label: String,
    pub scenario: RunScenario,
    pub site: String,
    pub variant: Vec<VariedValue>,
    pub seed: Seed,
    #[ts(type = "number")]
    pub duration_sim_ms: SimDuration,
    pub report_window: ReportWindow,
    pub state: RunState,
    pub attempt: AttemptNumber,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub waiting: Option<Waiting>,
    pub progress: RunProgressRead,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub started_at: Option<WallClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ended_at: Option<WallClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub elapsed_wall_ms: Option<WallMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ended_by: Option<EndedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stop_detail: Option<String>,
    pub recording: RecordingRead,
    /// The first headline metric over the run's report window (spec 08 section 3.9), once its attempt reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub headline: Option<MetricRow>,
    pub findings: Vec<FindingKind>,
}

/// The cut a run read, as its detail shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunCut {
    pub unit: ScopePath,
    pub requested: Pin,
    pub branch: BranchName,
}

/// The plan the attempt's runner compiled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct CompiledRead {
    pub digest: Option<String>,
    pub compiler: String,
}

/// One attempt of a run; what its manifest said once it arrived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts", optional_fields)]
pub struct AttemptRead {
    #[ts(optional = false)]
    pub n: AttemptNumber,
    #[ts(optional = false)]
    pub state: RunState,
    #[ts(optional = false)]
    pub requested_at: WallClock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<WallClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<WallClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<WallClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_stopped_at: Option<WallClock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_server: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_dirty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_by: Option<EndedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_detail: Option<String>,
    /// When the runner was last heard: its latest heartbeat, every 5 s of wall clock while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_at: Option<WallClock>,
    /// The platform's view of the attempt's Job as the reconciler last heard it, in words: `pending: ImagePullBackOff`,
    /// `running`, `ended: DeadlineExceeded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_status: Option<String>,
    /// A running attempt whose runner has not been heard for 30 s of wall clock, since its latest heartbeat or, before
    /// its first, since its manifest; read off the row, it ends nothing.
    #[serde(default)]
    #[ts(optional = false)]
    pub stalled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile: Option<CompiledRead>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walked: Option<String>,
    #[ts(optional = false)]
    pub findings: Vec<Finding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stub: Option<bool>,
}

/// `GET /api/runs/{r}`: a run, what it read, and every attempt, the latest last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunDetail {
    #[serde(flatten)]
    pub summary: RunSummary,
    pub cut: RunCut,
    // ordered: reads are listed in scope order.
    pub reads: BTreeMap<ScopePath, Seq>,
    pub runner: PlanRunner,
    pub expected: Expectation,
    pub submitted_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approval: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conversation: Option<Ulid>,
    pub attempts: Vec<AttemptRead>,
}

/// `GET /api/runs`: the list, newest submission first, and where the next page starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunList {
    pub items: Vec<RunSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next: Option<String>,
}

/// `GET /api/runs/queue`: what holds a seat and what waits, across the deployment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunQueue {
    pub limit: RunCount,
    pub going: Vec<RunSummary>,
    pub waiting: Vec<RunSummary>,
}

/// The seats: how many may be held at once across the deployment, and how many are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct SeatsRead {
    pub limit: RunCount,
    pub held: RunCount,
}

/// `GET /api/runs/stats`: the deployment's runs counted off the attempts' rows, for whoever watches them: the seats,
/// what waits, runs and stalls (running with its runner not heard for 30 s of wall clock), how the last 24 h of wall
/// clock ended, a runner lost being the one `partial` ending, how many Jobs could not be created in them, and how long
/// the oldest waiting attempt has waited. Counters and gauges, so a metrics exposition can be laid over them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunStats {
    pub seats: SeatsRead,
    pub waiting: RunCount,
    pub running: RunCount,
    pub stalled: RunCount,
    pub ended_last_24h: EndingCounts,
    pub create_failures_last_24h: RunCount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub oldest_waiting_since: Option<WallClock>,
}

/// What a cancel actually did (spec 08 section 3.6): cancelled it, asked it to stop, or found it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "runs.ts")]
pub enum CancelOutcome {
    Cancelled,
    Stopping,
    AlreadyEnded,
}

/// `202` to `POST /api/runs/{r}/cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct CancelAnswer {
    pub outcome: CancelOutcome,
    pub state: RunState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunCancelled {
    pub run: Ulid,
    pub outcome: CancelOutcome,
    pub state: RunState,
}

/// `202` to `POST /api/submissions/{s}/cancel`: each run's outcome, in position order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct SubmissionCancelled {
    pub outcomes: Vec<RunCancelled>,
}

/// `200` to `POST /api/runs/{r}/watch`: a ticket to watch the run's latest attempt live (ruling D174): the
/// streaming server's events websocket for that attempt (`url`), its frames websocket (`frames_url`, the picture
/// in the viewer's attach protocol; live frames, 26 October 2026) and its metrics websocket (`metrics_url`, one
/// metric per socket in the metric stream protocol), the one token a viewer presents on any of them in its first
/// message, and when the ticket stops opening them. Each ask mints a ticket of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct WatchTicket {
    pub run: Ulid,
    pub attempt: AttemptNumber,
    pub url: String,
    pub frames_url: String,
    pub metrics_url: String,
    pub token: String,
    pub expires_at: WallClock,
}

/// A runner image a plan may name, with the closed values its build accepts, by setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunnerImage {
    pub digest: String,
    pub tag: String,
    pub commit: String,
    pub branch: String,
    pub registered_at: WallClock,
    pub compiler: String,
    // ordered: settings are listed by name.
    pub accepts: BTreeMap<String, Vec<String>>,
    pub default: bool,
}

/// `GET /api/runner-images`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runs.ts")]
pub struct RunnerImages {
    pub items: Vec<RunnerImage>,
}
