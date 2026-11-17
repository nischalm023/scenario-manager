//! Within `studio-protocol`, `runner` is the runner contract (spec 08 section 3.5), written once for the API and
//! simrunner alike, so a runner and the API cannot disagree on a file: the variables a runner's environment may hold,
//! the job spec a runner starts from, the pins it reads on the runners' listener, the manifest it hands in before
//! simtime starts, the heartbeat it sends while it runs, the three files it leaves in its output folder
//! (`ending.json`, `metrics.json`, `series.json`), and once the run is over the recording the API stores for it and the
//! report it delivers them in. Every `_sim_ms` is simtime, every `_at` wall clock.

use std::collections::BTreeMap;
use std::time::Duration;

use desunits::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::{DurationClock, WallClock};
use crate::experiment::Patch;
use crate::grammar::{ScopePath, Seq, Ulid};
use crate::metric_stream::is_false;
use crate::plan::Expectation;
use crate::run::{AttemptNumber, EndedBy, Finding, FindingPart, ProgressValue, RunState};
use crate::scenario::{ReportWindow, Revision, Seed, VersionNumber, VersionRead};
use crate::workload::{GenerateRequest, UnitCut};

/// A runner file's format, 1 until release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct FileFormat(pub u32);

pub const FORMAT: FileFormat = FileFormat(1);

/// The runner's token, which its invoker sets in its environment and never in a job spec or on a command line.
pub const TOKEN_VAR: &str = "STUDIO_RUNNER_TOKEN";
/// dessim's executor threads, which the invoker sets from the CPUs it gives the runner.
pub const THREADS_VAR: &str = "STUDIO_RUNNER_THREADS";
/// How long the runner holds between its manifest's acceptance and the simulation, in milliseconds of wall clock. It
/// reaches a runner as a deployment's settings do, by name, and only tests set it, to catch a runner mid-run.
pub const HOLD_VAR: &str = "STUDIO_RUNNER_HOLD_MS";

/// The variables a runner reads that a deployment may hand every runner by name, and the only ones it may: dessim's
/// progress line and event digests, and the hold. The rest of what a runner reads is its invoker's (its token and
/// thread count) or its image's (its build's identity), so no other variable of the API's reaches a runner, and with
/// it no database URL or store credential (ruling D174: the simulation runner has no access to Postgres).
pub const RUNNER_SETTINGS: [&str; 4] = ["DES_HEARTBEAT", "DES_DIGEST", "DES_DIGEST_SORTED", HOLD_VAR];

/// How long a runner goes on delivering one thing once its run is over, its recording or its report, its tries, the
/// waits between them and their answers included (wall clock). A report the API took whose answer never arrived is
/// asked again within it, so its runner may still be at work for this long after the API took the report.
pub const DELIVERY_BUDGET: Duration = Duration::from_secs(240);

/// A size in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct ByteCount(pub u64);

/// Where the runners' listener answers, as a runner reaches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct Endpoint {
    pub endpoint: String,
}

/// The streaming server a dispatcher picked for an attempt (ruling D174): its registration's id, and the websocket
/// address of its runners' port as a runner reaches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct StreamingServer {
    pub server: Ulid,
    pub endpoint: String,
}

/// The job spec: the run, the attempt and where to read the rest. The token rides apart, never in here. `streaming`
/// is written only when the dispatcher found a streaming server alive, so a runner image that knows no streaming
/// server, refusing a field it does not know, still reads the spec of an attempt that runs unstreamed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct JobSpec {
    pub v: FileFormat,
    pub run: Ulid,
    pub attempt: AttemptNumber,
    pub studio: Endpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub streaming: Option<StreamingServer>,
}

/// The sealed scenario version a run reads, by number and the revision it was sealed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct PinnedScenario {
    pub id: Ulid,
    pub version: VersionNumber,
    pub revision: Revision,
}

/// The image an attempt runs, by digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct PinnedImage {
    pub image: String,
}

/// `GET /runner/v1/attempts/{run}/{n}`: what one attempt must read, all fixed before anyone approved it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct Pins {
    pub run: Ulid,
    pub attempt: AttemptNumber,
    pub plan: Ulid,
    pub experiment: Ulid,
    pub cut: UnitCut,
    // ordered: reads are listed in scope order.
    pub reads: BTreeMap<ScopePath, Seq>,
    pub scenario: PinnedScenario,
    pub variant: Vec<Patch>,
    pub seed: Seed,
    #[ts(type = "number")]
    pub duration_sim_ms: SimDuration,
    pub report_window: ReportWindow,
    pub runner: PinnedImage,
    pub expected: Expectation,
    pub wps: Endpoint,
}

/// `GET /runner/v1/attempts/{run}/{n}/control`: the attempt row's live control, apart from its fixed pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct AttemptControl {
    pub cancel_requested: bool,
}

/// One scope's export as world-proto-server answered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct ExportRead {
    pub scope: ScopePath,
    pub seq: Seq,
    pub sha256: String,
    pub bytes: ByteCount,
    pub exact: bool,
}

/// The world a run read: the export before the variant, by its hash, every scope it holds entities from, how far its
/// closure was vouched for (spec 04's walk, `annotations` or `annotations-and-list:{digest}`) and its exactness; who
/// assembled it and who generated its derived entities, `server` for both since WPS writes the site as its flagged
/// export; and the primary frame of the world the runner loaded, the frame that states `unit_mm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct WorldRead {
    pub sha256: String,
    // ordered: reads are listed in scope order.
    pub scopes: BTreeMap<ScopePath, Seq>,
    pub walked: String,
    pub exact: bool,
    pub exact_reason: Option<String>,
    pub assembled_by: String,
    pub derived_by: String,
    pub primary_frame: Option<String>,
}

/// One patch the runner applied, with what the field held before and the artifact it changed, hashed either side of
/// it: the world (`world/site.world.json`), the generator request (`workload.spec`, this manifest's own field, hashed
/// as its RFC 8785 canonical JSON, the bytes sent), or the run's `spec.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct AppliedPatch {
    #[serde(flatten)]
    pub patch: Patch,
    pub was: Value,
    pub file: String,
    pub sha256_before: String,
    pub sha256_after: String,
}

/// The workload the generator made for the run: the body sent, the generator's seed and version, and what came back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct WorkloadRead {
    pub spec: GenerateRequest,
    pub seed: Seed,
    pub generator: String,
    pub sha256: String,
    pub bytes: ByteCount,
}

/// The plan the runner compiled, against the expectation it read with its pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct CompileRead {
    pub v: FileFormat,
    pub compiler: String,
    pub digest: Option<String>,
    pub expected: Expectation,
    pub differs: Vec<FindingPart>,
}

/// `manifest.json`: what one attempt's runner fetched, generated and compiled, handed in before simtime starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct Manifest {
    pub v: FileFormat,
    pub run: Ulid,
    pub attempt: AttemptNumber,
    pub plan: Ulid,
    pub experiment: Ulid,
    pub scenario: PinnedScenario,
    pub cut: UnitCut,
    // ordered: reads are listed in scope order.
    pub reads: BTreeMap<ScopePath, Seq>,
    pub exports: Vec<ExportRead>,
    pub world: WorldRead,
    pub variant: Vec<AppliedPatch>,
    pub workload: WorkloadRead,
    pub compile: CompileRead,
    pub seed: Seed,
    #[ts(type = "number")]
    pub duration_sim_ms: SimDuration,
    pub report_window: ReportWindow,
    pub runner_image: String,
    pub fetched_at: WallClock,
    /// Whether the results that follow are canned: true for every attempt before simrunner ran dessim (ruling D110,
    /// until 21 October 2026), false since; every surface showing a stub attempt's numbers says so.
    pub stub: bool,
}

/// `200` to `PUT /runner/v1/attempts/{run}/{n}/manifest`: what the API found comparing it with the check; the run goes
/// on whatever it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct ManifestAccepted {
    pub findings: Vec<Finding>,
}

/// `PUT /runner/v1/attempts/{run}/{n}/heartbeat`, every 5 s of wall clock from the runner's start: how far the run is
/// in simtime and its counters so far, zeros before simtime starts. The API keeps the latest with when it came.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct Heartbeat {
    #[ts(type = "number")]
    pub sim_ms: SimTime,
    pub values: Vec<ProgressValue>,
}

/// `200` to `PUT /runner/v1/attempts/{run}/{n}/recording`: the runner's recording as the API stored it under the
/// attempt's key in the run store, which the runner's report then names: its key, how far it reaches in simtime, its
/// frames and its size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct RecordingStored {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub live_edge_sim_ms: Option<SimTime>,
    pub frames: u64,
    pub bytes: ByteCount,
}

/// `PUT /runner/v1/attempts/{run}/{n}/results`: what the runner left, delivered by the runner itself once the run is
/// over, since a Job's pod keeps no folder the API could read. The recording went up first, through
/// `PUT .../recording`, whose answer `recording` is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct ResultsReport {
    pub v: FileFormat,
    pub ending: Ending,
    pub metrics: Option<MetricsFile>,
    pub series: Option<SeriesFile>,
    pub recording: Option<RecordingStored>,
    pub runner_commit: Option<String>,
    pub runner_dirty: Option<bool>,
}

/// Whether a report's ending took: the first ending wins, and a later report changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "runner.ts")]
pub enum ResultsEnded {
    Wrote,
    AlreadyEnded,
}

/// `200` to `PUT /runner/v1/attempts/{run}/{n}/results`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct ResultsAccepted {
    pub ended: ResultsEnded,
}

/// `GET /runner/v1/scenarios/{s}/versions/{v}`: the sealed version a run's pins name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct SealedVersion {
    pub scenario: Ulid,
    pub version: VersionRead,
}

/// `ending.json`: how the attempt ended, who ended it, the simulator's words and how far it got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct Ending {
    pub state: RunState,
    pub ended_by: EndedBy,
    pub stop_reason: String,
    pub stop_detail: Option<String>,
    #[ts(type = "number")]
    pub sim_ms: SimTime,
}

/// A metric's base unit, as dessim's descriptors state it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "runner.ts")]
pub enum MetricUnit {
    Count,
    DurationMs,
    DistanceMm,
    Ratio,
    RatePerHour,
}

/// How a metric's values combine across periods: an additive one sums, a gauge is its newest reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "runner.ts")]
pub enum Aggregation {
    Additive,
    Interval,
    Ratio,
    SnapshotGauge,
    DistinctCount,
}

/// Whether a metric's final value came from the recording's end block or was rebuilt from its slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "runner.ts")]
pub enum MetricBasis {
    EndBlock,
    Rebuilt,
}

/// One metric the attempt declared and its final value, null where the run never stated one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct DeclaredMetric {
    pub metric: String,
    pub label: String,
    pub description: String,
    pub unit: MetricUnit,
    /// The clock a unit that measures time counts in, as the metric stream's descriptor names it: so the run's
    /// wall-clock figure is never read as simtime. Absent for a count, a distance or a ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    pub denominator: Option<String>,
    pub value: Option<f64>,
    /// Whether the value is the whole run's by definition, as each of the end block's Status figures is (the run's
    /// total picks): a reader answers it over any window, marked as the whole run's, where an additive metric with
    /// no running values otherwise answers no narrower window.
    #[serde(default, skip_serializing_if = "is_false")]
    pub whole_run: bool,
}

/// `metrics.json`: every metric the attempt declared, in descriptor order, and where their values came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct MetricsFile {
    pub v: FileFormat,
    pub basis: MetricBasis,
    pub metrics: Vec<DeclaredMetric>,
}

/// What a series holds: an accumulating metric's running value at each slice end from the run's start, or a
/// gauge's readings at each slice that stated one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "runner.ts")]
pub enum SeriesKind {
    Running,
    Readings,
}

/// One instant in simtime and the value there, null where the run had stated none yet.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct SeriesPoint(#[ts(type = "number")] pub SimTime, pub Option<f64>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "runner.ts")]
pub struct MetricSeries {
    pub metric: String,
    pub kind: SeriesKind,
    pub points: Vec<SeriesPoint>,
}

/// `series.json`: every declared metric's values at the instants the recording holds them, never thinned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "runner.ts")]
pub struct SeriesFile {
    pub v: FileFormat,
    #[ts(type = "number")]
    pub slice_sim_ms: SimDuration,
    #[ts(type = "number")]
    pub end_sim_ms: SimTime,
    pub metrics: Vec<MetricSeries>,
}
