//! Within `journal`, `turns` folds each turn's state (spec 01 section 3.4's six) from the journal's `turn.*` entries,
//! as a snapshot's projection and its active turn draw them; the conversation's open turn, which Dismiss's
//! `turn-running` and a conversation's `waiting` read, comes from task 3's `turns` table (`open_turn`). The agent loop,
//! and the test writer standing in for it, change a turn's row in the transaction that journals the change, so the
//! table and the fold agree.

use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Error as SerdeError, Value};
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{
    BlockId, JournalEntry, JournalKind, JournalSeq, Person, ProjectedTurn, TurnNumber, TurnState, Ulid, WaitingOn,
};

use crate::db::{decode, read_word, seq_of};

/// The kinds a turn's state follows.
pub const TURN_KINDS: [JournalKind; 4] =
    [JournalKind::TurnStarted, JournalKind::TurnWaiting, JournalKind::TurnResumed, JournalKind::TurnFinished];

/// The turns of a conversation as its entries, applied in order, leave them; `unread` names each entry the fold
/// could not read, which a route logs.
#[derive(Debug, Default)]
pub struct TurnFold {
    turns: Vec<ProjectedTurn>,
    index: FxHashMap<Ulid, usize>,
    pub unread: Vec<JournalSeq>,
}

#[derive(Deserialize)]
struct Started {
    n: TurnNumber,
    author: Person,
    message_seq: JournalSeq,
    attended: bool,
    #[serde(default)]
    browser_tab: Option<String>,
}

#[derive(Deserialize)]
struct Waiting {
    on: WaitingOn,
    #[serde(default)]
    reply_block: Option<BlockId>,
}

#[derive(Deserialize)]
struct Finished {
    outcome: TurnState,
    #[serde(default)]
    end_reason: Option<String>,
    #[serde(default)]
    reply_block: Option<BlockId>,
    #[serde(default)]
    budget: Option<String>,
}

impl TurnFold {
    /// Apply one entry; kinds other than `turn.*` change nothing.
    pub fn apply(&mut self, entry: &JournalEntry) {
        if !TURN_KINDS.contains(&entry.kind) {
            return;
        }
        if !entry.turn.as_ref().is_some_and(|turn| self.change(entry, turn)) {
            self.unread.push(entry.seq);
        }
    }

    fn change(&mut self, entry: &JournalEntry, turn: &Ulid) -> bool {
        if entry.kind == JournalKind::TurnStarted {
            let Some(started) = read::<Started>(&entry.body) else { return false };
            self.index.insert(turn.clone(), self.turns.len());
            self.turns.push(ProjectedTurn {
                id: turn.clone(),
                n: started.n,
                state: TurnState::Running,
                author: started.author,
                attended: started.attended,
                browser_tab: started.browser_tab,
                message_seq: started.message_seq,
                waiting_on: None,
                end_reason: None,
                reply_block: None,
                budget: None,
                as_of_seq: entry.seq,
            });
            return true;
        }
        let Some(projected) = self.index.get(turn).map(|n| &mut self.turns[*n]) else { return false };
        match entry.kind {
            JournalKind::TurnWaiting => {
                let Some(waiting) = read::<Waiting>(&entry.body) else { return false };
                (projected.state, projected.waiting_on) = (TurnState::Waiting, Some(waiting.on));
                projected.reply_block = waiting.reply_block.or(projected.reply_block.take());
            }
            JournalKind::TurnResumed => (projected.state, projected.waiting_on) = (TurnState::Running, None),
            _ => {
                let Some(finished) = read::<Finished>(&entry.body).filter(|done| !done.outcome.is_open()) else {
                    return false;
                };
                (projected.state, projected.waiting_on) = (finished.outcome, None);
                (projected.end_reason, projected.budget) = (finished.end_reason, finished.budget);
                projected.reply_block = finished.reply_block.or(projected.reply_block.take());
            }
        }
        projected.as_of_seq = entry.seq;
        true
    }

    /// The turn still running or waiting, the latest started if a faulty writer left several open.
    pub fn open(&self) -> Option<&ProjectedTurn> {
        self.turns.iter().rev().find(|turn| turn.state.is_open())
    }

    pub fn get(&self, turn: &Ulid) -> Option<&ProjectedTurn> {
        self.index.get(turn).map(|n| &self.turns[*n])
    }

    /// Every turn, in the order they started.
    pub fn turns(&self) -> &[ProjectedTurn] {
        &self.turns
    }
}

/// The conversation's turn still running or waiting, read from the `turns` table.
pub async fn open_turn(conn: &mut PgConnection, conversation: &Ulid) -> Result<Option<ProjectedTurn>, SqlxError> {
    let row = sqlx::query(
        "SELECT t.id, t.n, t.state, t.started_by, p.name, t.attended, t.browser_tab, t.message_seq, t.waiting_on \
         FROM turns t LEFT JOIN people p ON p.subject = t.started_by \
         WHERE t.conversation = $1 AND t.state IN ('running', 'waiting')",
    )
    .bind(conversation.as_str())
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let subject: String = row.try_get("started_by")?;
    let message_seq = seq_of(row.try_get("message_seq")?)?;
    Ok(Some(ProjectedTurn {
        id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        n: TurnNumber::new(row.try_get::<i32, _>("n")? as u32),
        state: read_word(&row.try_get::<String, _>("state")?)?,
        author: Person {
            name: row.try_get::<Option<String>, _>("name")?.unwrap_or_else(|| subject.clone()),
            subject: subject.parse().map_err(decode)?,
        },
        attended: row.try_get("attended")?,
        browser_tab: row.try_get("browser_tab")?,
        message_seq,
        waiting_on: row.try_get::<Option<String>, _>("waiting_on")?.map(|on| read_word(&on)).transpose()?,
        end_reason: None,
        reply_block: None,
        budget: None,
        as_of_seq: message_seq,
    }))
}

/// Whether the fold reads `body` as a `turn.*` entry written in `turn`; every one names its turn in the envelope.
pub fn readable(kind: JournalKind, body: &Value, turn: Option<&Ulid>) -> Result<(), String> {
    if turn.is_none() {
        return Err("names no turn, and a turn's entry names its turn in the envelope's `turn`".to_owned());
    }
    let fault = |fault: SerdeError| fault.to_string();
    match kind {
        JournalKind::TurnStarted => Started::deserialize(body).map(drop).map_err(fault),
        JournalKind::TurnWaiting => Waiting::deserialize(body).map(drop).map_err(fault),
        JournalKind::TurnFinished => match Finished::deserialize(body).map_err(fault)?.outcome {
            outcome if outcome.is_open() => {
                Err("ends no turn: its outcome is done, failed, stopped or interrupted".into())
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// A body as the type that reads it, or `None` when it does not hold what that type needs.
pub fn read<T: DeserializeOwned>(body: &Value) -> Option<T> {
    T::deserialize(body).ok()
}
