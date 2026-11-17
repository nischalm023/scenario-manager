//! Within `studio-protocol`, `turn` is what a turn journals about itself and its words (spec 01 section 3.5): how
//! it started, waited, resumed and ended, each text block the agent finished with the references it resolved, and a
//! note the service writes. A tool call's own entries are `tool`'s.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::conversation::WaitingOn;
use crate::grammar::Ulid;
use crate::journal::JournalSeq;
use crate::person::Person;
use crate::projection::{CallId, CallState, TurnNumber, TurnState};
use crate::stream::BlockId;

/// `turn.started`'s body; `browser_tab` is the browser tab whose message started it, null for an unattended turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct TurnStarted {
    pub n: TurnNumber,
    pub author: Person,
    pub message_seq: JournalSeq,
    pub attended: bool,
    pub browser_tab: Option<String>,
}

/// `turn.waiting`'s body: the card the turn waits on, and the last text block before the wait, which is the reply
/// spec 02 speaks (ruling D70).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct TurnWaiting {
    pub on: WaitingOn,
    pub id: Ulid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub reply_block: Option<BlockId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "turn.ts")]
pub enum ResumedBy {
    Answer,
    Decision,
    Message,
}

/// `turn.resumed`'s body: what ended the wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct TurnResumed {
    pub by: ResumedBy,
}

/// What a turn spent with the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct Usage {
    pub model_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
}

/// `turn.finished`'s body: one of the four ended states, the problem slug a failed turn ended on, the reply block
/// spec 02 speaks, and the per-turn budget a `done` turn reached (spec 01 section 3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct TurnFinished {
    pub outcome: TurnState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub end_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub reply_block: Option<BlockId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub budget: Option<String>,
    pub usage: Usage,
}

/// A text block's kind: the agent's words, or a note, the model's reasoning, which the service no longer journals and
/// the chat never draws (D117); only a journal written before that holds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "turn.ts")]
pub enum TextKind {
    Text,
    Note,
}

/// One `ref:` link of a text block, resolved at the turn's location by spec 03's rules: the key its chip carries, the
/// thing's kind and label as the chip shows them, and its route, null when it does not resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct ResolvedRef {
    pub key: String,
    pub kind: Option<String>,
    pub label: String,
    pub route: Option<String>,
}

/// `text.done`'s body: a block's final text, Markdown for a `text` block, with its resolved references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct TextDone {
    pub block: BlockId,
    pub kind: TextKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub refs: Option<Vec<ResolvedRef>>,
}

/// `system.note`'s body: the service's own sentence, a failed turn's reason among them (spec 01 section 3.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct SystemNote {
    pub text: String,
}

/// One call as the turn's details disclosure reads it (spec 01 section 3.6, `GET /api/turns/{t}/calls`): its input and
/// its result, the result trimmed to 4 KiB of JSON text, and a call still running has none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct CallRead {
    pub call: CallId,
    pub tool: String,
    pub input: Value,
    pub result: Option<Value>,
    pub outcome: CallState,
    pub summary: String,
}

/// `GET /api/turns/{t}/calls`'s answer: the turn's calls in the order they started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct TurnCalls {
    pub calls: Vec<CallRead>,
}

/// `POST /api/turns/{t}/stop`'s answer: `202` while the turn is being stopped, `200` for one already ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "turn.ts")]
pub struct StopAccepted {
    pub turn: Ulid,
}
