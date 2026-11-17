//! Within `studio-protocol`, `journal` is spec 01 section 3.5's envelope: one entry of a conversation's record as the
//! journal keeps it and the stream sends it, on one `data:` line under `id: <seq>` and `event: <kind>`. Each kind's
//! body is typed by the task that writes that kind; until then it travels as JSON, `panel.command`'s already being
//! `PanelCommandEntry`. The stream's live-only items (`text.delta`, `stream.live`, `ping` and the rest) are not
//! entries and are not here.

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::WallClock;
use crate::grammar::Ulid;

/// The envelope's `v`, always 1: the format is unreleased, so it stays at version 1 through every wire change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, TS)]
#[ts(type = "1", export_to = "journal.ts")]
pub struct EnvelopeVersion;

const ENVELOPE_VERSION: u8 = 1;

impl Serialize for EnvelopeVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(ENVELOPE_VERSION)
    }
}

impl<'de> Deserialize<'de> for EnvelopeVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u64::deserialize(deserializer)? {
            version if version == u64::from(ENVELOPE_VERSION) => Ok(Self),
            version => Err(D::Error::custom(format!("envelope version {version} is not {ENVELOPE_VERSION}"))),
        }
    }
}

/// An entry's place in its site's one journal sequence (spec 01 section 3.17): the numbers run on across the site's
/// conversations, so one number names one entry of the site and `highlights={seq}` needs no conversation id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "journal.ts")]
pub struct JournalSeq(u64);

impl JournalSeq {
    pub const fn new(seq: u64) -> Self {
        Self(seq)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Writes `JournalKind` from one list, so its wire names, `as_str` and `ALL` cannot disagree.
macro_rules! journal_kinds {
    ($($kind:ident = $wire:literal),+ $(,)?) => {
        /// The kinds a journal entry has: the rows of spec 01 section 3.5's table that are journaled.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export_to = "journal.ts")]
        pub enum JournalKind {
            $(#[serde(rename = $wire)] $kind),+
        }

        impl JournalKind {
            pub const ALL: &'static [JournalKind] = &[$(JournalKind::$kind),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(JournalKind::$kind => $wire),+
                }
            }
        }
    };
}

journal_kinds! {
    MessageUser = "message.user",
    TurnStarted = "turn.started",
    TextDone = "text.done",
    ToolStarted = "tool.started",
    ToolProgress = "tool.progress",
    ToolFinished = "tool.finished",
    QuestionAsked = "question.asked",
    QuestionAnswered = "question.answered",
    QuestionExpired = "question.expired",
    ApprovalRequested = "approval.requested",
    ApprovalDecided = "approval.decided",
    ApprovalExpired = "approval.expired",
    ApprovalNoted = "approval.noted",
    ApprovalExecution = "approval.execution",
    DraftUpdated = "draft.updated",
    DraftApplied = "draft.applied",
    DraftDiscarded = "draft.discarded",
    DraftStale = "draft.stale",
    PanelCommand = "panel.command",
    PanelEvent = "panel.event",
    RunStatus = "run.status",
    SubmissionEnded = "submission.ended",
    RunFinding = "run.finding",
    TurnWaiting = "turn.waiting",
    TurnResumed = "turn.resumed",
    TurnFinished = "turn.finished",
    SystemNote = "system.note",
    ConversationChanged = "conversation.changed",
}

/// One journal entry, the envelope of spec 01 section 3.5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "journal.ts")]
pub struct JournalEntry {
    pub v: EnvelopeVersion,
    pub seq: JournalSeq,
    pub at: WallClock,
    pub conversation: Ulid,
    /// Null for an entry no turn wrote, such as a panel event.
    pub turn: Option<Ulid>,
    pub kind: JournalKind,
    /// The tool call (the id the API minted for it), question or approval the entry answers, or null.
    pub caused_by: Option<String>,
    pub body: Value,
}
