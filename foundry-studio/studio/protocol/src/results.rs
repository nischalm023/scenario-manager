//! Within `studio-protocol`, `results` is what an ended run's result routes answer (spec 08 sections 3.7, 3.9, 3.10
//! and 3.12): its declared metrics over a window of simtime, one metric by simulated period, its inputs as values,
//! a comparison of 2 to 12 runs, and the headline metrics a project pins. The Runs panel and the agent's run tools
//! read the same shapes, so the agent never quotes a figure the panel does not show. Every `_sim_ms` is simtime.

use desunits::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::{Clock, DurationClock};
use crate::grammar::{ScopePath, Seq, Ulid};
use crate::run::{AttemptNumber, RunState};
use crate::runner::{Aggregation, MetricUnit, WorkloadRead};

/// A span of simtime between two instants the recording holds values for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct SimWindow {
    #[ts(type = "number")]
    pub from_sim_ms: SimTime,
    #[ts(type = "number")]
    pub to_sim_ms: SimTime,
}

/// Where a metric's value came from: the recording's end block, values rebuilt from its slices, or an additive
/// metric's running value at the window's end less at its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "results.ts")]
pub enum ValueBasis {
    EndBlock,
    Rebuilt,
    WindowDifference,
}

/// One declared metric's value over `window`, the span it actually covers: the window asked for, or the whole run for
/// a class whose window value is not exact. A value the run never stated is null, never 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct MetricRow {
    pub metric: String,
    pub label: String,
    pub description: String,
    pub unit: MetricUnit,
    /// The clock a duration or a rate's hour counts in, as the attempt's `metrics.json` declared it: so the run's
    /// wall-clock figure is never read as simtime. Absent for a count, a distance or a ratio, and for every metric of
    /// an attempt stored before results named their clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub denominator: Option<String>,
    pub value: Option<f64>,
    pub window: SimWindow,
    pub basis: ValueBasis,
}

/// `GET /api/runs/{r}/metrics`: every metric the attempt declared, in descriptor order, over `window`; `headlines`
/// names the run's headline metrics in order (section 3.9's rule), which the tiles and charts show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct MetricsReply {
    pub attempt: AttemptNumber,
    pub window: SimWindow,
    pub items: Vec<MetricRow>,
    pub headlines: Vec<String>,
}

/// How a series reads: an additive metric's value per period, or any other class's readings as stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "results.ts")]
pub enum SeriesShape {
    PerPeriod,
    Readings,
}

/// One period's value, or one reading at an instant (`from_sim_ms` equal to `to_sim_ms`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct PeriodPoint {
    #[ts(type = "number")]
    pub from_sim_ms: SimTime,
    #[ts(type = "number")]
    pub to_sim_ms: SimTime,
    pub value: Option<f64>,
}

/// `GET /api/runs/{r}/series`: one metric by simulated period from the run's start, never thinned; the last period
/// ends at the run's end, which may cut it short. `window` is the report window, which a chart shades outside.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct SeriesReply {
    pub run: Ulid,
    pub attempt: AttemptNumber,
    pub metric: String,
    pub label: String,
    pub unit: MetricUnit,
    /// The clock a duration or a rate's hour counts in, as on the metric's row (`MetricRow::clock`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    pub kind: SeriesShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub period_sim_ms: Option<SimDuration>,
    pub points: Vec<PeriodPoint>,
    pub window: SimWindow,
}

/// Which part of a run's inputs a row belongs to; `provenance` rows say where the rest came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "results.ts")]
pub enum InputSection {
    Scenario,
    Flow,
    Fleet,
    Map,
    Settings,
    Provenance,
}

/// One input as a value: `field` names it across runs (a scenario field's pointer, or a provenance key), `value` says
/// it for a reader, rounded, `from` says where it came from, and `scope` names the scope it was read from when it is a
/// world's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct InputRow {
    pub section: InputSection,
    pub field: String,
    pub label: String,
    pub value: String,
    /// The value as the run read it, unrounded and untruncated, as RFC 8785 canonical JSON (a scenario field's JSON at
    /// its pointer, a provenance row's every figure): two runs read one value exactly when these are equal, whatever
    /// `value`'s rounding says, so a comparison classifies on this and formats only after.
    pub canonical: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub all_in: Option<String>,
    pub from: String,
    /// The scenario's own value, where the run read another: a sweep's, a repeat's or an override's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub held: Option<String>,
    pub varied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scope: Option<ScopePath>,
    /// The clock the value counts in, when it is a time, so a page tags it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<Clock>,
    /// The unit a number is shown in, the field catalogue's, so a comparison's axis and headings name it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unit: Option<String>,
    /// The value as a number in that unit, where the field holds one, for a comparison's chart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub number: Option<f64>,
    /// The changeset a scope was read at, on a read's provenance row, so a page links that scope's history there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub changeset: Option<Seq>,
}

/// `GET /api/runs/{r}/inputs`: the run's inputs as values, then their provenance; `complete` is false until the
/// recording's resolved settings are read; `workload` is the generated workload as the runner's manifest names it, the
/// generator request it sent with its seed and digest, none before the manifest arrives (spec 08 section 3.12's
/// provenance); `text` holds the same rows, and the workload's request, as plain lines, for a copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct RunInputs {
    pub run: Ulid,
    pub complete: bool,
    pub rows: Vec<InputRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workload: Option<WorkloadRead>,
    pub text: String,
}

/// A compared run, and whether its attempt's numbers are simrunner's canned ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct ComparedRun {
    pub id: Ulid,
    pub label: String,
    pub state: RunState,
    pub attempt: AttemptNumber,
    pub stub: bool,
}

/// One run's value of an input; null where that run did not record it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct InputValue {
    pub run: Ulid,
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub number: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub changeset: Option<Seq>,
}

/// An input that differs between the compared runs: by the experiment's design or outside it, where it `warns` that
/// the results cannot be read as the varied input's effect alone, but for a read's exactness and the compiler's
/// version, which are provenance and never warn; a scope read at other changesets lists in `changed` the labels of the
/// differing inputs read from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct InputField {
    pub field: String,
    pub section: InputSection,
    pub label: String,
    pub by_experiment: bool,
    pub warns: bool,
    pub values: Vec<InputValue>,
    pub changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<Clock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct SameField {
    pub field: String,
    pub section: InputSection,
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<Clock>,
}

/// A run left out of the comparison, since its latest attempt did not finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct NotCompared {
    pub run: Ulid,
    pub state: RunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct ComparedValue {
    pub run: Ulid,
    pub value: Option<f64>,
    pub window: SimWindow,
}

/// One metric across the compared runs: combined only when every run reports it over one window with one unit,
/// clock, aggregation and denominator, else not comparable, saying why; its aggregation says what its window holds,
/// an additive metric's report window, a gauge's last reading or any other class's whole run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct ComparedMetric {
    pub metric: String,
    pub label: String,
    pub unit: MetricUnit,
    /// The clock of a duration or a rate's hour, as `MetricRow::clock`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    pub comparable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    pub values: Vec<ComparedValue>,
}

/// `GET /api/runs/compare`: the inputs that differ first, then what is the same, then results, each measured against
/// `reference`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "results.ts")]
pub struct CompareReply {
    pub runs: Vec<ComparedRun>,
    pub differs: Vec<InputField>,
    pub same: Vec<SameField>,
    pub not_compared: Vec<NotCompared>,
    pub reference: Ulid,
    pub metrics: Vec<ComparedMetric>,
}

/// `PUT /api/projects/{p}/metric-pins`: a project's headline metrics by name, in order, at most eight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "results.ts")]
pub struct MetricPins {
    pub metrics: Vec<String>,
}
