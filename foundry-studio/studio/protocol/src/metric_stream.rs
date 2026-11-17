//! Within `studio-protocol`, `metric_stream` is the wire of one metric of one attempt, computed by the streaming
//! server from the attempt's WorldView recording and served on `ws://<host>/metrics/{run}/{attempt}`: what a viewer
//! asks, and what the server sends. One socket carries one metric; a page showing three metrics opens three. The
//! server computes every figure; a page only draws.
//!
//! Order on the socket: the viewer's `attach` first, within `FIRST_MESSAGE_WITHIN`, naming the metric and presenting
//! the attempt's watch ticket; the server's `attached`, carrying the metric's descriptor; then `rows`: every bucket
//! final by the attach, from simtime 0 on, in batches; each further bucket once the server has taken in the slice
//! that makes it final; for a metric stated over the whole run, its whole-run rows once the server has taken in the
//! run's end; `ended` after the last row of a run that is over, and the socket closes behind it. A row is sent once,
//! when it is final, and never again: a reader appends and never revises. `lagged` goes to a viewer that stopped
//! reading, which then attaches afresh and receives every row again. Every failure closes the socket with a refusal
//! as its close reason, one of `StreamRefusal`'s. Every `_sim_ms` is simtime, and a duration or a rate names its
//! clock; `FIRST_MESSAGE_WITHIN` is wall clock.

use desunits::{SimDuration, SimTime};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::DurationClock;
use crate::grammar::Ulid;
use crate::run::AttemptNumber;
use crate::run_stream::StreamRefusal;
use crate::runner::{Aggregation, FileFormat, MetricUnit};

/// The metric stream's format, 1 at its start; every message carries it as `v`, and both ends speak one format.
pub const METRIC_STREAM_FORMAT: FileFormat = FileFormat(1);

/// Viewer to server: `attach` first, and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export_to = "metric_stream.ts")]
pub enum MetricRequest {
    /// `metric` names one of the metrics the server computes (its `MetricDescriptor::metric`); `token` is the
    /// attempt's watch ticket (`POST /api/runs/{run}/watch`). `trailing` asks for the metric's buckets as trailing
    /// windows at a step instead of its own bucketing; left out, the metric's own. The server takes it only for a
    /// metric bucketed over time whose buckets are not running totals, and only when it fits the recording's slices
    /// (`TrailingSpec::fits`); it refuses any other `unsupported-trailing`, never widening or rounding it.
    Attach {
        v: FileFormat,
        metric: String,
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        trailing: Option<TrailingSpec>,
    },
}

/// Server to viewer: `attached`, then `rows` until the run's last row, then `ended`; `lagged` instead when the viewer
/// stopped reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export_to = "metric_stream.ts")]
pub enum MetricMessage {
    /// The metric as the server will send it: its descriptor's bucket is the one every row is cut by, an accepted
    /// `trailing` exactly as asked. `reached_sim_ms` is how far the server has read the recording at the attach, the
    /// end of the newest slice it has taken, which a copy read from its start may lie well short of. Every bucket final
    /// by then comes as history; one ending earlier can still come live, when the facts that make it final ride a
    /// later slice. `live` is false once the run is over, when `ended` follows the history.
    Attached {
        v: FileFormat,
        run: Ulid,
        attempt: AttemptNumber,
        descriptor: MetricDescriptor,
        #[ts(type = "number")]
        reached_sim_ms: SimTime,
        live: bool,
    },
    /// Rows, each final: the buckets in rising order of `to_sim_ms`, then, once the server has taken in the run's
    /// end, the whole-run rows of a metric stated over the whole run, marked `whole_run`. History comes in batches;
    /// live buckets come as the server takes in the slices that make them final, usually one per message.
    Rows { v: FileFormat, rows: Vec<MetricPoint> },
    /// Nothing more will come: the run is over and its last row is out. `outcome` is how its recording ends.
    Ended { v: FileFormat, outcome: RunOutcome },
    /// The viewer stopped reading, and the socket closes behind this; it attaches afresh to receive every row again.
    Lagged { v: FileFormat },
}

/// How a run's recording ends, as a stream says once it has read every frame: the reason the simulator stopped, which
/// the recording's end block states, or what keeps the copy the server read from stating one. The `/frames` socket's
/// `ended` says the same words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "metric_stream.ts")]
pub enum RunOutcome {
    /// Every order of the run was completed.
    Complete,
    /// A person stopped the run.
    UserStop,
    /// The simulator's watchdog stopped the run; the end block's detail says on what.
    Watchdog,
    /// The run reached its wall-clock bound.
    WallTimeout,
    /// The run reached the simtime it was to simulate.
    SimTimeLimit,
    /// The simulator stopped on an error; the end block's detail says which.
    Error,
    /// The copy the server read is the run's live segments, whose uploads never finished, so it stops short of the
    /// run's end whatever its last frame says.
    RecordingIncomplete,
    /// The recording ends without an end block, so it does not say how the run ended.
    NoEndBlock,
}

/// `GET /api/metric-streams`: every metric the streaming server computes, each in the bucketing it is served in
/// unless an attach asks for another, so a page can offer them before any socket is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "metric_stream.ts")]
pub struct MetricCatalogue {
    pub metrics: Vec<MetricDescriptor>,
}

/// One metric as the server computes it: what it is called, what it measures, how its buckets are cut, and where in
/// the recording its facts come from, which says what makes a bucket final.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "metric_stream.ts")]
pub struct MetricDescriptor {
    /// The stable name a viewer attaches by, `snake_case`, never shown; `label` is what a page shows.
    pub metric: String,
    pub label: String,
    pub description: String,
    pub unit: MetricUnit,
    /// The clock a unit that measures time counts in, simtime or the wall clock: a `duration_ms` metric's
    /// milliseconds, or the hour a `rate_per_hour` metric is per. Absent for a count, a distance or a ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clock: Option<DurationClock>,
    pub aggregation: Aggregation,
    /// What a ratio's value is a fraction of, for a page's axis or tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub denominator: Option<String>,
    pub source: MetricSource,
    pub bucket: Bucket,
    /// The dimensions a row's `keys` name, in order; empty for a metric with one row per bucket.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
}

/// Where in the recording a metric's facts ride, which says when a bucket can be final: a `scalars` metric's bucket
/// closes with its slice; a `robot_state` metric's, read from the robot state channel, once the server has taken in the
/// slice past its end; a `journals` metric's once the barrier has passed the end of every journal row that overlaps it,
/// which can be later in simtime than the bucket; an `end_block` metric is stated once, over the whole run, at its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "metric_stream.ts")]
pub enum MetricSource {
    Scalars,
    RobotState,
    Journals,
    EndBlock,
}

/// How a metric's buckets are cut, in simtime: one per slice of the recording, trailing windows at a step, or no
/// bucket but the one spanning the whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "metric_stream.ts")]
pub enum Bucket {
    Slice {
        #[ts(type = "number")]
        width_sim_ms: SimDuration,
    },
    Trailing(TrailingSpec),
    Overall,
}

/// Causal trailing windows `(t - window, t]` at every multiple of `step` from the first instant one whole window fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "metric_stream.ts")]
pub struct TrailingSpec {
    #[ts(type = "number")]
    pub window_sim_ms: SimDuration,
    #[ts(type = "number")]
    pub step_sim_ms: SimDuration,
}

impl TrailingSpec {
    /// Whether these windows are what a recording of slices `slice_width` wide is cut into exactly: a window and a
    /// step each a whole number of slices, neither none. Any other would state windows a reader could not tell from
    /// the ones computed, so a server refuses it rather than computing its nearest fit.
    pub fn fits(self, slice_width: SimDuration) -> bool {
        let whole = |span: SimDuration| span > SimDuration::ZERO && span.as_ms().is_multiple_of(slice_width.as_ms());
        slice_width > SimDuration::ZERO && whole(self.window_sim_ms) && whole(self.step_sim_ms)
    }
}

/// One axis a metric's rows are split along, a pick station or a robot state, named for a page's legend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "metric_stream.ts")]
pub struct Dimension {
    pub name: String,
    pub description: String,
}

/// One row's figure: the span it covers, its position along each declared dimension, and its value, null where the
/// run stated nothing in the span. A ratio carries its parts too, so a page can show them exactly, but `value` is
/// already their quotient: no page divides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "metric_stream.ts")]
pub struct MetricPoint {
    #[ts(type = "number")]
    pub from_sim_ms: SimTime,
    #[ts(type = "number")]
    pub to_sim_ms: SimTime,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub numerator: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub denominator: Option<f64>,
    /// Whether the row states the metric over the whole run, from simtime 0 to the run's end, rather than over one
    /// of its buckets: what a page tables beside the chart, never a point on it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub whole_run: bool,
}

pub(crate) fn is_false(flag: &bool) -> bool {
    !flag
}

#[derive(Deserialize)]
struct VersionOnly {
    v: FileFormat,
}

/// `text` read as `T`: `unsupported-version` when it states another format than `METRIC_STREAM_FORMAT`, whether or
/// not the rest reads; `unreadable` when it is in this format and does not read.
fn read<T: DeserializeOwned>(text: &str, version: fn(&T) -> FileFormat) -> Result<T, StreamRefusal> {
    match serde_json::from_str::<T>(text) {
        Ok(message) if version(&message) == METRIC_STREAM_FORMAT => Ok(message),
        Ok(_) => Err(StreamRefusal::UnsupportedVersion),
        Err(_) => match serde_json::from_str::<VersionOnly>(text) {
            Ok(VersionOnly { v }) if v != METRIC_STREAM_FORMAT => Err(StreamRefusal::UnsupportedVersion),
            _ => Err(StreamRefusal::Unreadable),
        },
    }
}

impl MetricRequest {
    pub fn read(text: &str) -> Result<Self, StreamRefusal> {
        read(text, |message| match message {
            Self::Attach { v, .. } => *v,
        })
    }

    pub fn text(&self) -> String {
        serde_json::to_string(self).expect("a metric stream message is JSON")
    }
}

impl MetricMessage {
    pub fn read(text: &str) -> Result<Self, StreamRefusal> {
        read(text, |message| match message {
            Self::Attached { v, .. } | Self::Rows { v, .. } | Self::Ended { v, .. } | Self::Lagged { v } => *v,
        })
    }

    pub fn text(&self) -> String {
        serde_json::to_string(self).expect("a metric stream message is JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attach_reads_with_and_without_a_trailing_spec_and_refuses_another_format() {
        let plain = r#"{"type":"attach","v":1,"metric":"picks_per_hour","token":"t"}"#;
        let read = MetricRequest::read(plain).expect("reads");
        assert!(matches!(&read, MetricRequest::Attach { metric, trailing: None, .. } if metric == "picks_per_hour"));
        assert_eq!(MetricRequest::read(&read.text()), Ok(read));
        let trailing = r#"{"type":"attach","v":1,"metric":"da50_state","token":"t","trailing":{"window_sim_ms":900000,"step_sim_ms":300000}}"#;
        let read = MetricRequest::read(trailing).expect("reads");
        assert!(matches!(
            read,
            MetricRequest::Attach { trailing: Some(TrailingSpec { window_sim_ms, step_sim_ms }), .. }
                if window_sim_ms == SimDuration::ms(900_000) && step_sim_ms == SimDuration::ms(300_000)
        ));
        assert_eq!(
            MetricRequest::read(r#"{"type":"attach","v":2,"metric":"x","token":"t"}"#),
            Err(StreamRefusal::UnsupportedVersion)
        );
        assert_eq!(MetricRequest::read(r#"{"type":"attach","v":1}"#), Err(StreamRefusal::Unreadable));
    }

    /// A ratio's row carries its parts beside the quotient; a plain row leaves them out, as it leaves out `keys`
    /// when the metric has no dimension.
    #[test]
    fn a_row_spells_only_what_it_has() {
        let plain = MetricPoint {
            from_sim_ms: SimTime::ms(0),
            to_sim_ms: SimTime::ms(10_000),
            keys: Vec::new(),
            value: Some(3.0),
            numerator: None,
            denominator: None,
            whole_run: false,
        };
        let text = MetricMessage::Rows { v: METRIC_STREAM_FORMAT, rows: vec![plain.clone()] }.text();
        assert_eq!(text, r#"{"type":"rows","v":1,"rows":[{"from_sim_ms":0,"to_sim_ms":10000,"value":3.0}]}"#);
        let ratio = MetricPoint {
            keys: vec!["Waiting".into()],
            numerator: Some(1.0),
            denominator: Some(4.0),
            value: Some(0.25),
            ..plain
        };
        let text = MetricMessage::Rows { v: METRIC_STREAM_FORMAT, rows: vec![ratio.clone()] }.text();
        assert!(text.contains(r#""keys":["Waiting"],"value":0.25,"numerator":1.0,"denominator":4.0"#), "{text}");
        assert_eq!(MetricMessage::read(&text), Ok(MetricMessage::Rows { v: METRIC_STREAM_FORMAT, rows: vec![ratio] }));
    }

    /// A run no longer than one window has a whole-run row spanning exactly what a window would: the mark, not the
    /// span, is what says which it is.
    #[test]
    fn a_whole_run_row_is_marked_on_the_wire_and_a_bucket_is_not() {
        let window = MetricPoint {
            from_sim_ms: SimTime::ms(0),
            to_sim_ms: SimTime::ms(900_000),
            keys: Vec::new(),
            value: Some(4.0),
            numerator: None,
            denominator: None,
            whole_run: false,
        };
        let whole = MetricPoint { whole_run: true, ..window.clone() };
        let text = MetricMessage::Rows { v: METRIC_STREAM_FORMAT, rows: vec![window.clone(), whole.clone()] }.text();
        assert_eq!(text.matches(r#""whole_run":true"#).count(), 1, "{text}");
        let read = MetricMessage::read(&text).expect("reads");
        assert_eq!(read, MetricMessage::Rows { v: METRIC_STREAM_FORMAT, rows: vec![window, whole] });
    }

    #[test]
    fn a_duration_or_a_rate_names_its_clock_and_a_count_names_none() {
        let descriptor = MetricDescriptor {
            metric: "wall_clock_per_slice".to_owned(),
            label: "Wall clock/slice".to_owned(),
            description: String::new(),
            unit: MetricUnit::DurationMs,
            clock: Some(DurationClock::Wall),
            aggregation: Aggregation::Additive,
            denominator: None,
            source: MetricSource::Scalars,
            bucket: Bucket::Slice { width_sim_ms: SimDuration::ms(10_000) },
            dimensions: Vec::new(),
        };
        let text = serde_json::to_string(&descriptor).expect("JSON");
        assert!(text.contains(r#""unit":"duration_ms","clock":"wall""#), "{text}");
        assert_eq!(serde_json::from_str::<MetricDescriptor>(&text).expect("reads"), descriptor);
        let count = MetricDescriptor { unit: MetricUnit::Count, clock: None, ..descriptor };
        assert!(!serde_json::to_string(&count).expect("JSON").contains(r#""clock":"#), "a count is on no clock");
    }

    /// A window or a step that is not a whole number of slices would state windows other than the ones computed,
    /// and one of none would state no window at all.
    #[test]
    fn a_trailing_spec_fits_a_recording_only_in_whole_slices() {
        let spec = |window_ms, step_ms| TrailingSpec {
            window_sim_ms: SimDuration::ms(window_ms),
            step_sim_ms: SimDuration::ms(step_ms),
        };
        let slice = SimDuration::ms(10_000);
        assert!(spec(900_000, 300_000).fits(slice));
        assert!(spec(10_000, 30_000).fits(slice), "a step longer than the window is still whole slices");
        assert!(!spec(25_000, 10_000).fits(slice), "a 25 s window covers three slices, 30 s");
        assert!(!spec(30_000, 15_000).fits(slice), "a 15 s step would fire every 30 s");
        assert!(!spec(0, 10_000).fits(slice) && !spec(10_000, 0).fits(slice), "a window or a step of none");
        assert!(!spec(10_000, 10_000).fits(SimDuration::ZERO), "nor does anything fit slices of no width");
    }

    /// An outcome travels as one word, the simulator's stop reasons among them in the wire's own spelling, so a page
    /// words each one and a word no outcome spells does not read.
    #[test]
    fn an_ended_message_says_its_outcome_in_one_word() {
        let ended = |outcome| MetricMessage::Ended { v: METRIC_STREAM_FORMAT, outcome };
        assert_eq!(ended(RunOutcome::SimTimeLimit).text(), r#"{"type":"ended","v":1,"outcome":"sim-time-limit"}"#);
        let incomplete = r#"{"type":"ended","v":1,"outcome":"recording-incomplete"}"#;
        assert_eq!(MetricMessage::read(incomplete), Ok(ended(RunOutcome::RecordingIncomplete)));
        assert_eq!(
            MetricMessage::read(r#"{"type":"ended","v":1,"outcome":"SimTimeLimit"}"#),
            Err(StreamRefusal::Unreadable)
        );
    }

    #[test]
    fn a_bucket_is_tagged_by_kind() {
        let slice = serde_json::to_string(&Bucket::Slice { width_sim_ms: SimDuration::ms(10_000) }).expect("JSON");
        assert_eq!(slice, r#"{"kind":"slice","width_sim_ms":10000}"#);
        let overall = serde_json::to_string(&Bucket::Overall).expect("JSON");
        assert_eq!(overall, r#"{"kind":"overall"}"#);
        let trailing = Bucket::Trailing(TrailingSpec {
            window_sim_ms: SimDuration::ms(60_000),
            step_sim_ms: SimDuration::ms(10_000),
        });
        assert_eq!(
            serde_json::to_string(&trailing).expect("JSON"),
            r#"{"kind":"trailing","window_sim_ms":60000,"step_sim_ms":10000}"#
        );
    }
}
