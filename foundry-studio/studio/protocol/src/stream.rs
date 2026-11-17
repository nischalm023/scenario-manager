//! Within `studio-protocol`, `stream` is what the conversation stream sends that the journal never keeps (spec 01
//! section 3.5): the live-only items a writer hands the API's hub, `text.delta`, `run.progress` and
//! `template.evaluation`, and the three the stream makes itself, `stream.live`, `ping` and `stream.reset`. None
//! carries an `id:` line, so a tab's last event id stays at the last journaled entry, and a tab that misses one loses
//! nothing the journal holds.

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::WallClock;
use crate::journal::JournalSeq;

/// The kinds of live-only item a writer hands the hub: the rows of spec 01 section 3.5's table marked "live only".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub enum LiveKind {
    #[serde(rename = "text.delta")]
    TextDelta,
    #[serde(rename = "run.progress")]
    RunProgress,
    #[serde(rename = "template.evaluation")]
    TemplateEvaluation,
}

impl LiveKind {
    pub const ALL: [LiveKind; 3] = [Self::TextDelta, Self::RunProgress, Self::TemplateEvaluation];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextDelta => "text.delta",
            Self::RunProgress => "run.progress",
            Self::TemplateEvaluation => "template.evaluation",
        }
    }
}

/// A text block of a turn, as `text.delta`, `text.done` and `turn.finished`'s `reply_block` name it; the agent loop
/// mints it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(type = "string", export_to = "stream.ts")]
pub struct BlockId(String);

impl BlockId {
    pub fn new(block: impl Into<String>) -> Self {
        Self(block.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for BlockId {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

/// A position in a streaming block's text, counted in UTF-16 code units as a JavaScript string's `length` counts
/// them, so a page places a delta with `slice` (spec 01 section 3.5 says "character offset"; this is the reading the
/// browser can apply without counting code points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub struct TextOffset(u64);

impl TextOffset {
    pub const fn new(units: u64) -> Self {
        Self(units)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// `text.delta`'s body: `delta` is written over the block's text from `offset` for its own length, any text beyond
/// it kept, so a delta placed twice changes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "stream.ts")]
pub struct TextDelta {
    pub block: BlockId,
    pub offset: TextOffset,
    pub delta: String,
}

/// A live-only item as a writer hands it to the hub. The bodies are their owners' (`run.progress` spec 08's,
/// `template.evaluation` spec 05's); a `text.delta`'s body is a `TextDelta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "stream.ts")]
pub struct LiveItem {
    pub kind: LiveKind,
    pub body: Value,
}

/// A block still streaming and its text so far: what a stream that joins mid-block sends first as one delta from
/// offset 0, and what a snapshot's active turn shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub struct StreamingBlock {
    pub block: BlockId,
    pub text: String,
}

/// The kinds the stream makes itself, each sent without an `id:` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub enum StreamKind {
    #[serde(rename = "stream.live")]
    StreamLive,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "stream.reset")]
    StreamReset,
}

impl StreamKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StreamLive => "stream.live",
            Self::Ping => "ping",
            Self::StreamReset => "stream.reset",
        }
    }
}

/// `stream.live`'s body, sent once per connection after the replayed entries: the last journaled number it sent,
/// so a tab tells catching up from live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub struct StreamLive {
    pub last_seq: JournalSeq,
}

/// `ping`'s body, every 15 s of wall clock: an event a page's `EventSource` hands on, which a comment line is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub struct Ping {
    pub at: WallClock,
}

/// Why a stream ended with `stream.reset`: its cursor names no entry this conversation has reached, or its client fell
/// more than 1,000 items behind. Either way the tab reads the snapshot again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "stream.ts")]
pub enum ResetReason {
    UnknownCursor,
    SlowClient,
}

/// `stream.reset`'s body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "stream.ts")]
pub struct StreamReset {
    pub reason: ResetReason,
}
