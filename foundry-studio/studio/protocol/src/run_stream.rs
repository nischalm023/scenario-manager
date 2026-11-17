//! Within `studio-protocol`, `run_stream` is the wire of a run's live stream (ruling D174): what a runner says to the
//! streaming server on its internal port and what the server answers it, and what a viewer asks on the external port
//! and what the server sends it. Each direction is a type of its own, read only through this module, which refuses a
//! message that does not read or speaks another format than `STREAM_FORMAT`, so both ends keep one version rule; a
//! refusal is a socket's close reason, typed here too. Every message is one JSON text frame carrying `v` and `type`.
//! Values travel by name, never as fields of their own, so a viewer discovers what a run reports. Also here are the
//! wall-clock bounds both ends must agree on. Every `_sim_ms` is simtime.
//!
//! The binary rule, format 2 (live frames, 26 October 2026; ruling D184): after `started`, a binary message from the
//! runner is one bare worldview container frame, its header and payload exactly as `worldview`'s `Writer` writes
//! them, no file around it. The first is the scenario block. A keyframe opens a segment and its deltas follow on the
//! same socket with no other segment's frames between them: the runner sends the frames of the run's recording as
//! dessim writes them, each final once written, the end block last, behind the last segment's frames, and the index
//! never; an end block no keyframe went ahead of is passed over. After a lost connection it sends the scenario block
//! and the open segment from its keyframe again, its end block with it once that went. A binary message before
//! `started` is refused. The final recording does not travel here: the runner sends it to the API's door. Frames
//! reach a viewer on the server's `/frames` socket, in the viewer's own attach protocol, not as these messages.

use std::time::Duration;

use desunits::SimTime;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::grammar::Ulid;
use crate::run::{AttemptNumber, ProgressValue};
use crate::runner::{Ending, FileFormat};

/// The stream's format, 2 since live frames; every message carries it as `v`, and both ends speak one format.
pub const STREAM_FORMAT: FileFormat = FileFormat(2);

/// How long a socket's first message, a runner's `hello` or a viewer's `watch`, may take to come (wall clock).
pub const FIRST_MESSAGE_WITHIN: Duration = Duration::from_secs(5);
/// How often the server pings a socket; one that stays silent, pongs included, for `SILENT_AFTER` is taken for gone
/// (wall clock), which finds a peer that vanished without closing.
pub const PING_EVERY: Duration = Duration::from_secs(10);
pub const SILENT_AFTER: Duration = Duration::from_secs(30);
/// How long a runner whose socket closed without its ending has to come back before the server tells the viewers it is
/// lost (wall clock). One that comes back later still takes its stream up again.
pub const RUNNER_GRACE: Duration = Duration::from_secs(30);
/// The longest a runner waits between two tries to reconnect (wall clock): well inside the grace, so an outage the
/// grace covers sees several tries.
pub const RETRY_AT_MOST: Duration = Duration::from_secs(8);

/// Runner to server, on the internal port: `hello` first, then `started` once the run is prepared, its `progress` as it
/// goes, and `ended` last, the picture's frames as binary messages between them by the binary rule above. After a
/// reconnect the runner says `hello`, `started`, the scenario block and the open segment again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export_to = "run_stream.ts")]
pub enum RunnerMessage {
    /// The runner names its attempt and presents the attempt's token.
    Hello {
        v: FileFormat,
        run: Ulid,
        attempt: AttemptNumber,
        token: String,
    },
    /// The run is about to begin, its manifest accepted and its harness prepared.
    Started {
        v: FileFormat,
    },
    Progress(StreamProgress),
    Ended(StreamEnded),
}

/// Server to runner: `welcome` once a hello is admitted; a refusal is the close frame's reason instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export_to = "run_stream.ts")]
pub enum RunnerReply {
    Welcome { v: FileFormat },
}

/// Viewer to server, on the external port: `watch` first, with the ticket the API minted for the attempt
/// (`POST /api/runs/{run}/watch`). A viewer says nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export_to = "run_stream.ts")]
pub enum ViewerRequest {
    Watch { v: FileFormat, token: String },
}

/// Server to viewer: `attached` first, then the stream so far (its `started`, the newest `progress`, and its last word,
/// `ended` or `runner-lost`, when it has one), then the runner's messages as they come, with an `edge` after each
/// segment of the picture the server locked. `lagged` comes before a viewer that stopped reading is dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export_to = "run_stream.ts")]
pub enum ViewerMessage {
    /// How many messages the server replays before the live ones.
    Attached {
        v: FileFormat,
        run: Ulid,
        attempt: AttemptNumber,
        replayed: u32,
    },
    Started {
        v: FileFormat,
    },
    Progress(StreamProgress),
    Ended(StreamEnded),
    /// The server locked one more segment of the picture: the simtime instant the live picture reaches, so the run
    /// page can say how far behind the simulator it trails. Live only, never replayed: a viewer attaching to the
    /// frames socket is told the edge in its `attached`.
    Edge {
        v: FileFormat,
        #[ts(type = "number")]
        locked_sim_ms: SimTime,
    },
    /// The runner's socket closed without its ending and the runner did not come back within the grace; if it comes
    /// back, its messages follow. How the run ended is the API's to say.
    RunnerLost {
        v: FileFormat,
    },
    Lagged {
        v: FileFormat,
    },
}

/// How far the run is: the simtime it reached and its counters so far by name, as the runner's heartbeat reads them.
/// The newest supersedes every one before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "run_stream.ts")]
pub struct StreamProgress {
    pub v: FileFormat,
    #[ts(type = "number")]
    pub sim_ms: SimTime,
    pub values: Vec<ProgressValue>,
}

/// The run is over: the ending the attempt left, sent before the runner delivers its recording and report. The report
/// carries the same ending unless the recording cannot be stored, when it says the platform failed the attempt; how
/// the attempt ended is the API's to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "run_stream.ts")]
pub struct StreamEnded {
    pub v: FileFormat,
    pub ending: Ending,
}

/// Why one end closed a socket, as the close frame's reason spells it, so the other end tells a refusal it should take
/// as final from one worth another try: a runner by `is_final_for_runner`, a viewer by `is_final_for_viewer`, which
/// the bindings carry to the pages as `VIEWER_FINAL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "run_stream.ts")]
pub enum StreamRefusal {
    /// No `hello` or `watch` that reads came within `FIRST_MESSAGE_WITHIN`.
    NoGreeting,
    /// A message that does not read as its direction's, or one out of its place, as a second `hello`.
    Unreadable,
    /// A message in another format than `STREAM_FORMAT`.
    UnsupportedVersion,
    /// A binary frame before `started`, or from a viewer.
    #[serde(rename = "binary-refused")]
    Binary,
    /// A binary frame that is not one container frame, or one out of the binary rule's order: a frame before the
    /// scenario block, a second scenario block, a delta or the end block before any keyframe, an instant running
    /// backwards, an index frame, a frame behind the end block but its open segment sent again, a frame after the
    /// ending.
    FrameOrder,
    /// The attempt's frames held in memory would pass the server's bound.
    FrameTooLarge,
    /// The token opens no attempt whose Job runs (a runner's), or is no live watch ticket for the attempt (a viewer's).
    TokenInvalid,
    /// The attempt's stream has ended, and takes nothing more.
    AttemptEnded,
    /// The server holds no picture of the attempt yet: its runner has not sent the scenario block here. Another try
    /// may find one.
    NotHeld,
    /// No server can serve the attempt: the server holds no stream of it, and no recording of it is anywhere the
    /// server reads, on its disk or in the run store.
    NoRecording,
    /// A metric stream's attach names a metric the server does not compute.
    UnknownMetric,
    /// A metric stream's attach asks for trailing windows the server does not cut that metric into: for a metric
    /// whose buckets are running totals or that has no buckets over time, or windows that are not whole slices of the
    /// recording (`TrailingSpec::fits`).
    UnsupportedTrailing,
    /// The attempt's recording lacks a fact the metric is computed from, as a scalar it does not declare, being
    /// written before that fact was recorded or since it was renamed; refused rather than charted as no value.
    NotRecorded,
    /// The attempt's recording does not read, or states facts that contradict each other, so the metric cannot be
    /// computed from it.
    RecordingUnreadable,
    /// Something the server needs did not answer: its database, as it checks the token, or its disk or the run
    /// store, as it reads the attempt's recording. Another try may succeed.
    Unavailable,
    /// A newer socket of the same runner took the stream over.
    Superseded,
    /// Nothing was heard for `SILENT_AFTER`, pongs included.
    Silent,
}

impl StreamRefusal {
    /// Every refusal, in the order the type declares them.
    pub const ALL: [StreamRefusal; 17] = [
        Self::NoGreeting,
        Self::Unreadable,
        Self::UnsupportedVersion,
        Self::Binary,
        Self::FrameOrder,
        Self::FrameTooLarge,
        Self::TokenInvalid,
        Self::AttemptEnded,
        Self::NotHeld,
        Self::NoRecording,
        Self::UnknownMetric,
        Self::UnsupportedTrailing,
        Self::NotRecorded,
        Self::RecordingUnreadable,
        Self::Unavailable,
        Self::Superseded,
        Self::Silent,
    ];

    /// The close frame's reason.
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoGreeting => "no-greeting",
            Self::Unreadable => "unreadable",
            Self::UnsupportedVersion => "unsupported-version",
            Self::Binary => "binary-refused",
            Self::FrameOrder => "frame-order",
            Self::FrameTooLarge => "frame-too-large",
            Self::TokenInvalid => "token-invalid",
            Self::AttemptEnded => "attempt-ended",
            Self::NotHeld => "not-held",
            Self::NoRecording => "no-recording",
            Self::UnknownMetric => "unknown-metric",
            Self::UnsupportedTrailing => "unsupported-trailing",
            Self::NotRecorded => "not-recorded",
            Self::RecordingUnreadable => "recording-unreadable",
            Self::Unavailable => "unavailable",
            Self::Superseded => "superseded",
            Self::Silent => "silent",
        }
    }

    /// The refusal a close frame's reason spells, if it spells one.
    pub fn from_reason(reason: &str) -> Option<Self> {
        serde_json::from_value(Value::from(reason)).ok()
    }

    /// Whether a runner so refused stops trying for its attempt: its token opens nothing, its stream has ended, it
    /// speaks another format, or another runner of the attempt holds the stream, which a runner's own reconnection
    /// never meets, since it opens a socket only once it has given up the one before. Anything else is worth another
    /// connection.
    pub fn is_final_for_runner(self) -> bool {
        matches!(self, Self::TokenInvalid | Self::AttemptEnded | Self::UnsupportedVersion | Self::Superseded)
    }

    /// Whether a viewer so refused stops attaching: another attach would meet the same refusal, because the server
    /// refused what the viewer said, as a message that does not read, another format, a binary frame, a metric it
    /// does not compute or windows it does not cut; or because of the attempt itself, which has no recording or one
    /// the metric cannot be computed from. Anything else, a ticket gone stale or a picture not there yet among them,
    /// is worth another attach.
    pub fn is_final_for_viewer(self) -> bool {
        matches!(
            self,
            Self::Unreadable
                | Self::UnsupportedVersion
                | Self::Binary
                | Self::NoRecording
                | Self::UnknownMetric
                | Self::UnsupportedTrailing
                | Self::NotRecorded
                | Self::RecordingUnreadable
        )
    }
}

/// A message's format, which every direction's type states in every variant.
trait Versioned {
    fn v(&self) -> FileFormat;
}

/// Only `v`, read off a message that does not read whole, to tell another format from a fault.
#[derive(Deserialize)]
struct VersionOnly {
    v: FileFormat,
}

/// `text` read as a message of `T`: refused `unsupported-version` when it is in another format than `STREAM_FORMAT`,
/// whether or not the rest reads, and `unreadable` when it is in this format and does not read.
fn read<T: DeserializeOwned + Versioned>(text: &str) -> Result<T, StreamRefusal> {
    match serde_json::from_str::<T>(text) {
        Ok(message) if message.v() == STREAM_FORMAT => Ok(message),
        Ok(_) => Err(StreamRefusal::UnsupportedVersion),
        Err(_) => match serde_json::from_str::<VersionOnly>(text) {
            Ok(VersionOnly { v }) if v != STREAM_FORMAT => Err(StreamRefusal::UnsupportedVersion),
            _ => Err(StreamRefusal::Unreadable),
        },
    }
}

fn text<T: Serialize>(message: &T) -> String {
    serde_json::to_string(message).expect("a stream message is JSON")
}

impl Versioned for RunnerMessage {
    fn v(&self) -> FileFormat {
        match self {
            Self::Hello { v, .. } | Self::Started { v } => *v,
            Self::Progress(progress) => progress.v,
            Self::Ended(ended) => ended.v,
        }
    }
}

impl Versioned for RunnerReply {
    fn v(&self) -> FileFormat {
        match self {
            Self::Welcome { v } => *v,
        }
    }
}

impl Versioned for ViewerRequest {
    fn v(&self) -> FileFormat {
        match self {
            Self::Watch { v, .. } => *v,
        }
    }
}

impl Versioned for ViewerMessage {
    fn v(&self) -> FileFormat {
        match self {
            Self::Attached { v, .. } | Self::Started { v } | Self::Edge { v, .. } | Self::RunnerLost { v } => *v,
            Self::Lagged { v } => *v,
            Self::Progress(progress) => progress.v,
            Self::Ended(ended) => ended.v,
        }
    }
}

impl RunnerMessage {
    pub fn read(text: &str) -> Result<Self, StreamRefusal> {
        read(text)
    }

    pub fn text(&self) -> String {
        text(self)
    }
}

impl RunnerReply {
    pub fn read(text: &str) -> Result<Self, StreamRefusal> {
        read(text)
    }

    pub fn text(&self) -> String {
        text(self)
    }
}

impl ViewerRequest {
    pub fn read(text: &str) -> Result<Self, StreamRefusal> {
        read(text)
    }

    pub fn text(&self) -> String {
        text(self)
    }
}

impl ViewerMessage {
    pub fn read(text: &str) -> Result<Self, StreamRefusal> {
        read(text)
    }

    pub fn text(&self) -> String {
        text(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{RETRY_AT_MOST, RUNNER_GRACE, RunnerMessage, RunnerReply, STREAM_FORMAT, StreamRefusal};

    #[test]
    fn a_message_in_another_format_is_refused_as_such_whether_or_not_it_reads() {
        assert_eq!(RunnerReply::read(r#"{"type":"welcome","v":2}"#), Ok(RunnerReply::Welcome { v: STREAM_FORMAT }));
        assert_eq!(RunnerReply::read(r#"{"type":"welcome","v":1}"#), Err(StreamRefusal::UnsupportedVersion));
        let changed = r#"{"type":"progress","v":3,"sim_ms":"a shape v2 never had"}"#;
        assert_eq!(RunnerMessage::read(changed), Err(StreamRefusal::UnsupportedVersion));
        assert_eq!(RunnerMessage::read(r#"{"type":"progress","v":2}"#), Err(StreamRefusal::Unreadable));
        assert_eq!(RunnerMessage::read("not json"), Err(StreamRefusal::Unreadable));
    }

    /// `ALL` names every refusal: the match below stops compiling until a new one is added to it, and then to `ALL`.
    #[test]
    fn every_refusal_reads_back_from_its_reason() {
        use StreamRefusal::*;
        for refusal in StreamRefusal::ALL {
            match refusal {
                NoGreeting | Unreadable | UnsupportedVersion | Binary | FrameOrder | FrameTooLarge | TokenInvalid
                | AttemptEnded | NotHeld | NoRecording | UnknownMetric | UnsupportedTrailing | NotRecorded
                | RecordingUnreadable | Unavailable | Superseded | Silent => {}
            }
            assert_eq!(StreamRefusal::from_reason(refusal.reason()), Some(refusal), "{refusal:?}");
        }
        let mut reasons: Vec<&str> = StreamRefusal::ALL.iter().map(|refusal| refusal.reason()).collect();
        reasons.sort_unstable();
        reasons.dedup();
        assert_eq!(reasons.len(), StreamRefusal::ALL.len(), "each refusal once, with a reason of its own");
        assert_eq!(StreamRefusal::from_reason("going away"), None);
    }

    /// A runner stops trying on four refusals, a viewer on eight; none is final for both but another format, which
    /// either end meets again whatever it does.
    #[test]
    fn a_runner_and_a_viewer_each_stop_on_the_refusals_another_try_cannot_change() {
        use StreamRefusal::*;
        let runner: Vec<StreamRefusal> = StreamRefusal::ALL.into_iter().filter(|r| r.is_final_for_runner()).collect();
        assert_eq!(runner, [UnsupportedVersion, TokenInvalid, AttemptEnded, Superseded]);
        let viewer: Vec<StreamRefusal> = StreamRefusal::ALL.into_iter().filter(|r| r.is_final_for_viewer()).collect();
        assert_eq!(
            viewer,
            [
                Unreadable,
                UnsupportedVersion,
                Binary,
                NoRecording,
                UnknownMetric,
                UnsupportedTrailing,
                NotRecorded,
                RecordingUnreadable
            ]
        );
    }

    /// A runner tries again at least three times within the grace, so a short outage never loses it its stream.
    #[test]
    fn a_runner_retries_well_inside_the_servers_grace() {
        assert!(RETRY_AT_MOST * 3 < RUNNER_GRACE, "{RETRY_AT_MOST:?} against {RUNNER_GRACE:?}");
    }
}
