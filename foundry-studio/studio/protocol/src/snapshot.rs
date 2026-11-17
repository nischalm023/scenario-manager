//! Within `studio-protocol`, `snapshot` is how a tab reads a conversation (spec 01 section 3.6; spec 02 section 3.3,
//! step 1): the snapshot it loads, read in one transaction at one journal number, and the pages of earlier entries it
//! reads back, each with the projection of what its entries name.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::conversation::{Conversation, WaitingOn};
use crate::grammar::Ulid;
use crate::journal::{JournalEntry, JournalSeq};
use crate::projection::{Projection, TurnNumber, TurnState};
use crate::stream::StreamingBlock;

/// The questions still open, the approvals still pending and the drafts still open, by id into the projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "snapshot.ts")]
pub struct Pending {
    pub questions: Vec<Ulid>,
    pub approvals: Vec<Ulid>,
    pub drafts: Vec<Ulid>,
}

/// The turn running or waiting as a snapshot is read, with the blocks still streaming and their text so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "snapshot.ts")]
pub struct ActiveTurn {
    pub id: Ulid,
    pub n: TurnNumber,
    pub state: TurnState,
    pub waiting_on: Option<WaitingOn>,
    pub streaming: Vec<StreamingBlock>,
}

/// `GET /api/conversations/{c}`'s answer, read in one transaction at `last_seq`: the conversation, its last 200
/// entries oldest first, and the projection of what they name with every pending question, approval and open draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "snapshot.ts")]
pub struct Snapshot {
    pub conversation: Conversation,
    pub entries: Vec<JournalEntry>,
    pub projection: Projection,
    pub pending: Pending,
    pub active_turn: Option<ActiveTurn>,
    pub last_seq: JournalSeq,
}

/// `GET /api/conversations/{c}/entries?after=&limit=`'s answer: at most 500 entries after `after`, the projection of
/// what they name, and `next`, the `after` that reads on, when more remain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "snapshot.ts")]
pub struct EntriesPage {
    pub entries: Vec<JournalEntry>,
    pub projection: Projection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next: Option<JournalSeq>,
}
