//! Within `studio-protocol`, `projection` is the state of what a conversation's entries name, as of a journal number
//! (spec 01 section 3.6, "The projection"): a tab that holds only the newest entries draws from it what a full replay
//! would, since its first entry may sit inside a turn whose header, tool calls or cards came earlier. Every object
//! carries `as_of_seq`, the last entry that changed it, so a tab keeps per id the one with the larger.

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::conversation::WaitingOn;
use crate::grammar::Ulid;
use crate::journal::JournalSeq;
use crate::person::Person;
use crate::stream::BlockId;

/// A turn's number in its conversation, 1 for the first, as "Agent, turn 4, done" reads it (spec 01 section 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct TurnNumber(u32);

impl TurnNumber {
    pub const fn new(n: u32) -> Self {
        Self(n)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A turn's state, spec 01 section 3.4's six (`.turn[data-state]`): `running` and `waiting` while it is open, the
/// other four once `turn.finished` names its outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "projection.ts")]
pub enum TurnState {
    Running,
    Waiting,
    Done,
    Failed,
    Stopped,
    Interrupted,
}

impl TurnState {
    /// Whether the turn still runs or waits, which keeps a conversation from being dismissed.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

/// A turn as its `turn.*` entries leave it: `turn.started`'s fields, then the state, what it waits on, and how it
/// ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct ProjectedTurn {
    pub id: Ulid,
    pub n: TurnNumber,
    pub state: TurnState,
    pub author: Person,
    pub attended: bool,
    /// The browser tab that sent the turn's message (spec 03 section 3.3), which alone navigates on its commands.
    pub browser_tab: Option<String>,
    pub message_seq: JournalSeq,
    pub waiting_on: Option<WaitingOn>,
    /// The problem slug a failed turn ended on (spec 01 section 3.14).
    pub end_reason: Option<String>,
    /// The reply the announcer reads: the last text block before a wait, or the one `turn.finished` names.
    pub reply_block: Option<BlockId>,
    /// The per-turn budget a `done` turn reached (spec 01 section 3.4).
    pub budget: Option<String>,
    pub as_of_seq: JournalSeq,
}

/// A tool call, by the ULID the API mints for it, since a provider may send none (spec 01 section 3.3's
/// `tool_calls.id`; ruling D122).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(type = "string", export_to = "projection.ts")]
pub struct CallId(String);

impl CallId {
    pub fn new(call: impl Into<String>) -> Self {
        Self(call.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CallId {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

/// A tool call's state (spec 01 section 3.3's `tool_calls.state`); `waiting` while an approval it asked for waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "projection.ts")]
pub enum CallState {
    Running,
    Waiting,
    Ok,
    Failed,
    Denied,
    Stopped,
    Interrupted,
}

/// A tool call as its `tool.*` entries and its approval's leave it: what a ToolRow draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct ProjectedCall {
    pub call: CallId,
    pub turn: Option<Ulid>,
    pub tool: String,
    pub class: String,
    pub state: CallState,
    pub summary: String,
    /// The latest `tool.progress` note while the call runs.
    pub note: Option<String>,
    pub problem: Option<Value>,
    pub as_of_seq: JournalSeq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "projection.ts")]
pub enum QuestionState {
    Open,
    Answered,
    Expired,
}

/// A question as its entries leave it: `asked` is its first entry's body, `answered` the answer's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct ProjectedQuestion {
    pub id: Ulid,
    pub call: Option<CallId>,
    pub turn: Option<Ulid>,
    pub state: QuestionState,
    pub asked: Value,
    pub answered: Option<Value>,
    pub as_of_seq: JournalSeq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "projection.ts")]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// How an approved act's execution stands (spec 01 section 3.3's `approvals.execution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "projection.ts")]
pub enum Execution {
    NotStarted,
    Running,
    Ok,
    Failed,
    Interrupted,
    Unknown,
}

/// An approval as its entries leave it: `requested` is its first entry's body; `decided`, `executed` and `noted`
/// the latest `approval.decided`, `approval.execution` and `approval.noted` bodies, what an ApprovalCard draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct ProjectedApproval {
    pub id: Ulid,
    /// Null for a button's approval, which no tool call asked for.
    pub call: Option<CallId>,
    pub turn: Option<Ulid>,
    pub state: ApprovalState,
    pub execution: Option<Execution>,
    pub requested: Value,
    pub decided: Option<Value>,
    pub executed: Option<Value>,
    pub noted: Option<Value>,
    pub as_of_seq: JournalSeq,
}

/// A draft changeset's state (spec 01 section 3.11): `applying` while an approval has claimed its checked revision and
/// its one changeset is being written, which no edit or discard interrupts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "projection.ts")]
pub enum DraftState {
    Drafting,
    Ready,
    Applying,
    Applied,
    Discarded,
    Stale,
}

impl DraftState {
    /// Whether the draft can still be applied or discarded, which a snapshot's `pending` lists.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Drafting | Self::Ready)
    }
}

/// A draft changeset as its entries leave it: `updated` is its latest `draft.updated` body, `ended` the
/// `draft.applied`, `draft.discarded` or `draft.stale` body that closed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct ProjectedDraft {
    pub id: Ulid,
    pub turn: Option<Ulid>,
    pub state: DraftState,
    pub updated: Option<Value>,
    pub ended: Option<Value>,
    pub as_of_seq: JournalSeq,
}

/// A run as its entries leave it, until spec 08's `RunSummary`: its latest `run.status` body and its findings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct ProjectedRun {
    pub run: Ulid,
    pub status: Value,
    pub findings: Vec<Value>,
    pub as_of_seq: JournalSeq,
}

/// What a range of entries names, as of the journal number the read stopped at; each list in the order its objects
/// first appeared.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "projection.ts")]
pub struct Projection {
    pub turns: Vec<ProjectedTurn>,
    pub calls: Vec<ProjectedCall>,
    pub questions: Vec<ProjectedQuestion>,
    pub approvals: Vec<ProjectedApproval>,
    pub drafts: Vec<ProjectedDraft>,
    pub runs: Vec<ProjectedRun>,
}
