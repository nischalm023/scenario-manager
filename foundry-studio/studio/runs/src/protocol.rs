//! Within `crucible`, `protocol` is WHAT A BROWSER AND THE SERVER SAY TO EACH
//! OTHER: the JSON control messages, and nothing about how they travel.
//!
//! It is one module in the server rather than a crate of its own because there
//! is one server and one client shape. The client's half is generated from
//! these types by a test (`tests/ts_bindings.rs`) into
//! `studio/viewer/js/src/protocol.ts`, so the browser cannot drift from
//! what the server accepts: changing a type here without regenerating fails
//! that test.
//!
//! Every request carries an id and every reply carries the id it answers, so a
//! reply and an attachment's frames share one socket without a client having to
//! guess which is which. Frames travel as binary messages and are not described
//! here — they are worldview frames, which `worldview/worldview` decodes on both
//! ends; the only one the server builds rather than forwards is the scenario
//! block, and even that is the same record with fewer fields filled in.

use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use ts_rs::TS;

/// What a client called this request. Echoed on the reply; the server reads
/// nothing into it.
pub type RequestId = String;

/// What a browser sends.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "protocol.ts")]
pub enum Request {
    /// Start a run from a spec and its files. The reply names the run; nothing
    /// is streamed, because watching is a separate attach.
    Submit { id: RequestId, spec: SubmittedSpec },
    /// Every run, newest first.
    List { id: RequestId },
    /// One run: its attempts, its progress, how it ended.
    Status { id: RequestId, run: String },
    /// Stop a run. Its recording is completed from the model as it stands.
    Cancel { id: RequestId, run: String },
    /// Watch a run from an instant. The reply names the attempt being read,
    /// and the frames follow it as binary messages.
    Attach {
        id: RequestId,
        run: String,
        /// A simulated instant in milliseconds, or `"live"`.
        #[ts(type = "number | \"live\"")]
        from: AttachFrom,
        /// Which sections of each frame to send; empty means the default set.
        #[serde(default)]
        sections: Vec<String>,
    },
    /// Stop watching. The frames stop; the socket stays open.
    Detach { id: RequestId },
    /// Each named metric's points from the run's start to `until`. Naming none
    /// asks for every metric the run declared.
    Series {
        id: RequestId,
        run: String,
        metrics: Vec<String>,
        /// A simulated instant in milliseconds; absent means as far as the
        /// recording goes.
        #[serde(default)]
        until: Option<u64>,
    },
}

/// Where an attachment starts: a simulated instant in milliseconds, or the
/// literal `"live"` for as far as the recording has got.
///
/// Written by hand rather than derived, because the two spellings are a number
/// and one particular word: an untagged enum over a `String` would read
/// `{"from": "banana"}` as the live edge, and a unit variant would put `null`
/// on the wire. The TypeScript says `number | "live"` for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachFrom {
    /// A simulated instant in milliseconds.
    At(u64),
    /// As far as the recording has got.
    Live,
}

/// The word the live edge is spelled with, on the wire and in the TypeScript.
const LIVE: &str = "live";

impl Serialize for AttachFrom {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::At(ms) => serializer.serialize_u64(*ms),
            Self::Live => serializer.serialize_str(LIVE),
        }
    }
}

impl<'de> Deserialize<'de> for AttachFrom {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Number(number) => number
                .as_u64()
                .map(Self::At)
                .ok_or_else(|| D::Error::custom(format!("`{number}` is not a simulated instant in ms"))),
            Value::String(word) if word == LIVE => Ok(Self::Live),
            other => Err(D::Error::custom(format!(
                "`{other}` is not where an attachment starts; give an instant in ms or `\"{LIVE}\"`"
            ))),
        }
    }
}

/// A spec and every file it names, as a client that shares no filesystem with
/// the server has to send them.
///
/// `spec` is the world spec's own JSON text, verbatim — the server writes those
/// exact bytes back out and loads them, so what a recording says it was run
/// from is what was submitted. `files` is keyed by the reference as the spec
/// writes it (`"cencora_v3.json"`, `"../../items.csv"`), because that is the
/// key a recording stores its embedded copies under, and a path resolved on
/// the client's machine would mean nothing here.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct SubmittedSpec {
    /// The spec file's own text.
    pub spec: String,
    /// Every file the spec names: the reference as written, and the file's
    /// bytes in standard base64.
    pub files: Vec<SubmittedFile>,
    /// What to call the run. Absent means the spec's own file name.
    #[serde(default)]
    pub name: Option<String>,
}

/// One file of a submission.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct SubmittedFile {
    /// The reference exactly as the spec writes it.
    pub reference: String,
    /// The file's bytes, standard base64 with padding.
    pub bytes: String,
}

/// What the server sends back.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "protocol.ts")]
pub enum Reply {
    /// What this server takes in one message, said as soon as the socket opens
    /// and answering no request.
    ///
    /// A submission carries a whole warehouse design inline, and a browser that
    /// learnt the ceiling by exceeding it would learn it as a closed socket:
    /// the transport refuses an oversized message before any of this server's
    /// own code sees it. Told up front, a form can name the number while
    /// somebody is still choosing files.
    Hello { max_message_bytes: u64 },
    /// The run exists and is queued. Nothing has been simulated yet.
    Submitted { id: RequestId, run: String },
    /// Every run, newest first.
    List { id: RequestId, runs: Vec<RunSummary> },
    /// One run: its id, its attempts and how far it has got, side by side as
    /// SPEC-phase2 §3 spells them. Flat, so a browser written against §3 reads
    /// `reply.attempts` rather than reaching through a nested object; `name`
    /// and `created_at` stand beside them because a reader showing a run shows
    /// both and `list` already carries them.
    Status {
        id: RequestId,
        run: String,
        name: String,
        created_at: String,
        attempts: Vec<AttemptSummary>,
        progress: Progress,
    },
    /// The run was asked to stop. Its recording is completed from where it got
    /// to; `status` says when that has happened.
    Cancelled { id: RequestId, run: String },
    /// The attachment is open and the frames are on their way: which attempt is
    /// being read, the instant its first keyframe stands at or before, and how
    /// far the recording goes.
    Attached { id: RequestId, run: String, attempt: u32, from_ms: u64, live_edge_ms: u64 },
    /// The attachment's frames are all sent — the end frame was the last one —
    /// and why the run stopped.
    ///
    /// `outcome` is the RECORDING's own wording, out of the end block a browser
    /// has just been sent, not the registry's summary of it: it is the sentence
    /// a viewer shows, and every other reader of that file sees the same one.
    Ended { id: RequestId, run: String, outcome: String },
    /// The attachment is closed at the browser's own asking.
    Detached { id: RequestId },
    /// The socket stopped accepting frames for longer than the server waits, so
    /// the attachment was dropped. A browser re-attaches at this instant,
    /// throws away the state it was holding, and is sent a fresh keyframe.
    ///
    /// `id` is the attach's, when the attachment had got as far as being told
    /// one; nothing else on the wire answers a request nobody made.
    Lagged {
        #[serde(default)]
        id: Option<RequestId>,
        resume_from_ms: u64,
    },
    /// The points each asked-for metric had, from the run's start to `until`.
    Series { id: RequestId, run: String, metrics: Vec<MetricSeries> },
    /// The request could not be answered, and why. `id` is absent when the
    /// message was not readable enough to carry one.
    Error {
        #[serde(default)]
        id: Option<RequestId>,
        message: String,
    },
}

/// A run as `list` and `status` report it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct RunSummary {
    /// The run's id.
    pub run: String,
    /// What it is called.
    pub name: String,
    /// When it was submitted, RFC 3339.
    pub created_at: String,
    /// Every try at it, oldest first.
    pub attempts: Vec<AttemptSummary>,
    /// How far the run has got: its newest attempt's progress.
    pub progress: Progress,
}

/// One try at a run.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct AttemptSummary {
    /// Which try this is, counting from one.
    pub n: u32,
    /// Where it stands.
    pub state: AttemptState,
    /// When the simulation started, RFC 3339; absent while queued.
    #[serde(default)]
    pub started_at: Option<String>,
    /// When it ended, RFC 3339; absent while it is still going.
    #[serde(default)]
    pub ended_at: Option<String>,
    /// The simulator's own word for why it stopped.
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// Where the recording is stored, once it is.
    #[serde(default)]
    pub object_key: Option<String>,
    /// Why it failed, in the simulator's words.
    #[serde(default)]
    pub error: Option<String>,
    /// How far it had got, last time anyone looked.
    pub progress: Progress,
    /// What its recording holds, once there is one. Written down when the
    /// attempt ends, so `list` carries it and a browser deciding where to
    /// attach asks once rather than once per run.
    #[serde(default)]
    pub recording: Option<RecordingExtent>,
}

/// One metric's points: the instant in milliseconds, and the value the run
/// held there.
///
/// A metric this run never declared comes back named and empty rather than
/// left out, so a chart can say which of the things it asked for the run has
/// nothing to say about.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct MetricSeries {
    pub metric: String,
    pub points: Vec<(u64, f64)>,
}

/// What a finished attempt's recording holds, for a browser deciding where to
/// attach and how much there is to scrub through.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct RecordingExtent {
    /// The last frame's simulated instant: as far as the recording goes.
    pub live_edge_ms: u64,
    /// How many frames it holds.
    pub frames: u64,
    pub size_bytes: u64,
}

/// Where an attempt stands.
///
/// `partial` is what a restart writes over an attempt it found `running`: the
/// process died mid-run, so how far that run got is not known and nothing is
/// invented for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "protocol.ts")]
pub enum AttemptState {
    Queued,
    Running,
    Finished,
    Cancelled,
    Failed,
    Partial,
}

/// How far a run has got, in the two numbers that mean something while it is
/// still going.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
#[ts(export_to = "protocol.ts")]
pub struct Progress {
    /// The simulated clock, in milliseconds.
    pub sim_ms: u64,
    /// Picks completed.
    pub picks: u64,
}

impl Request {
    /// What this request called itself, which its answer echoes back.
    pub fn id(&self) -> &str {
        match self {
            Self::Submit { id, .. }
            | Self::List { id }
            | Self::Status { id, .. }
            | Self::Cancel { id, .. }
            | Self::Attach { id, .. }
            | Self::Detach { id }
            | Self::Series { id, .. } => id,
        }
    }
}

impl Reply {
    /// Which request this reply answers, for a client picking its own answer
    /// out of everything one socket carries.
    ///
    /// `None` answers no request at all: the greeting is sent before anything
    /// is asked for, a lag can arrive before the attach it belongs to was
    /// given an id, and a message too broken to read carries no id to echo.
    pub fn answers(&self) -> Option<&str> {
        match self {
            Self::Hello { .. } => None,
            Self::Submitted { id, .. }
            | Self::List { id, .. }
            | Self::Status { id, .. }
            | Self::Cancelled { id, .. }
            | Self::Attached { id, .. }
            | Self::Ended { id, .. }
            | Self::Detached { id }
            | Self::Series { id, .. } => Some(id),
            Self::Lagged { id, .. } | Self::Error { id, .. } => id.as_deref(),
        }
    }
}

impl AttemptState {
    /// The word this state is stored and sent as.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }

    /// Whether nothing more will happen to this attempt.
    pub fn is_over(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

impl FromStr for AttemptState {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        match word {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "finished" => Ok(Self::Finished),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "partial" => Ok(Self::Partial),
            other => Err(format!("`{other}` is not a state an attempt can be in")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AttachFrom, AttemptState, Request};

    #[test]
    fn a_request_is_tagged_by_its_type() {
        let text = r#"{"type":"status","id":"7","run":"01J8Z5K3QW"}"#;
        let request: Request = serde_json::from_str(text).expect("a status request");
        assert!(matches!(request, Request::Status { ref run, .. } if run == "01J8Z5K3QW"));
        assert_eq!(serde_json::to_string(&request).expect("re-encodes"), text);
    }

    #[test]
    fn an_attachment_starts_at_an_instant_or_at_the_live_edge_and_nowhere_else() {
        let at: AttachFrom = serde_json::from_str("600000").expect("an instant");
        assert!(matches!(at, AttachFrom::At(600_000)));
        let live: AttachFrom = serde_json::from_str(r#""live""#).expect("the live edge");
        assert!(matches!(live, AttachFrom::Live));
        assert_eq!(serde_json::to_string(&live).expect("re-encodes"), r#""live""#);
        // Any other word is not a place to start.
        assert!(serde_json::from_str::<AttachFrom>(r#""banana""#).is_err());
    }

    #[test]
    fn every_state_reads_back_as_itself() {
        for state in [
            AttemptState::Queued,
            AttemptState::Running,
            AttemptState::Finished,
            AttemptState::Cancelled,
            AttemptState::Failed,
            AttemptState::Partial,
        ] {
            assert_eq!(state.as_str().parse(), Ok(state));
        }
        assert!("lost".parse::<AttemptState>().is_err());
    }
}
