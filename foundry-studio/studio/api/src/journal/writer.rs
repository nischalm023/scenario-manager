//! Within `journal`, `writer` appends an entry to a conversation's record (spec 01 section 3.5, "Writers") inside the
//! caller's transaction: it locks the conversation's row, looks the entry's dedup key up, takes the next number with
//! `UPDATE … RETURNING last_seq` and inserts. The row lock orders a conversation's writers, so its numbers are gapless
//! and ordered, and a transaction that rolls back takes its number back with it. What it wrote reaches the hub only
//! once the caller has committed. An archived or discarded conversation takes no entry (`conversation-closed`), so
//! Dismiss writes its own last entry before it archives.
//!
//! The site form serves run events (spec 01 section 3.17's last paragraph): it locks the site's live conversation,
//! the row Dismiss locks to archive it, looks the key up across every conversation of the site, and appends to the
//! live one, so a run's entry lands once, and never in an archived conversation.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Error as SqlxError, Postgres, Transaction};
use studio_protocol::{
    ConversationState, EnvelopeVersion, JournalEntry, JournalKind, JournalSeq, Problem, ProblemType, SitePath, Ulid,
    WallClock,
};
use thiserror::Error;

use crate::db::{decode, seq_of};
use crate::problem::{ProblemAnswer, internal};

/// How many times the site form reads the site's live conversation again after missing it: a miss means a Dismiss
/// committed between the read and the lock, and every further miss needs another Dismiss in between.
const SITE_READS: usize = 3;

/// An entry to append: the envelope's fields its writer chooses. The number, the wall-clock `at` and the
/// conversation are the journal's.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntry {
    pub kind: JournalKind,
    pub body: Value,
    pub turn: Option<Ulid>,
    pub caused_by: Option<String>,
    /// A key the entry lands under once, however often its writer reports it (spec 08's run events).
    pub dedup_key: Option<String>,
}

impl NewEntry {
    pub fn new(kind: JournalKind, body: Value) -> Self {
        Self { kind, body, turn: None, caused_by: None, dedup_key: None }
    }
}

/// An entry a transaction wrote, for the hub once that transaction commits: a wake sent sooner would have a stream
/// read for a row it cannot see yet, or one a rollback took back.
#[must_use = "announce it to the hub once its transaction commits"]
#[derive(Debug, Clone, PartialEq)]
pub struct Written(JournalEntry);

impl Written {
    pub fn entry(&self) -> &JournalEntry {
        &self.0
    }

    pub fn into_entry(self) -> JournalEntry {
        self.0
    }
}

/// What an append did: wrote the entry, or found its dedup key held by the entry with this number, and wrote nothing.
#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub enum Appended {
    Wrote(Written),
    Held(JournalSeq),
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("there is no conversation {0}")]
    NotFound(Ulid),
    #[error("the conversation {conversation} is {state}, and takes no more entries")]
    Closed { conversation: Ulid, state: ConversationState },
    #[error("the site {0} has no live conversation: nobody has opened one")]
    NoLiveConversation(SitePath),
    #[error(transparent)]
    Database(#[from] SqlxError),
}

impl JournalError {
    /// This error as a route answers it for the request at `instance`.
    pub fn answer(&self, instance: &str) -> ProblemAnswer {
        let problem = match self {
            Self::NotFound(conversation) => {
                Problem::new(ProblemType::NotFound, format!("There is no conversation {conversation}."))
            }
            Self::Closed { state, .. } => Problem::new(
                ProblemType::ConversationClosed,
                format!("This conversation is {state}: it reads as history, and nothing in it can be done again."),
            ),
            Self::NoLiveConversation(site) => {
                Problem::new(ProblemType::NotFound, format!("Nobody has opened a conversation for {site} yet."))
            }
            Self::Database(fault) => return internal("write the conversation's journal", fault, instance),
        };
        problem.at(instance).into()
    }
}

/// Append `entry` to `conversation` inside `tx`. A dedup key the conversation already holds writes nothing.
pub async fn append(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    entry: NewEntry,
) -> Result<Appended, JournalError> {
    if let Some(seq) = held(tx, conversation, entry.dedup_key.as_deref()).await? {
        return Ok(Appended::Held(seq));
    }
    write(tx, conversation, entry).await.map(Appended::Wrote)
}

/// Inside `tx`: the conversation's row locked, as every writer of it locks it first, a conversation that takes no entry
/// refused; and the number of the entry holding `key`, when one does. The test writer asks before it changes a turn's
/// row for an entry, so a held one changes nothing.
pub async fn held(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    key: Option<&str>,
) -> Result<Option<JournalSeq>, JournalError> {
    let state: Option<String> = sqlx::query_scalar("SELECT state FROM conversations WHERE id = $1 FOR UPDATE")
        .bind(conversation.as_str())
        .fetch_optional(&mut **tx)
        .await?;
    match state.as_deref().map(str::parse::<ConversationState>) {
        None => return Err(JournalError::NotFound(conversation.clone())),
        Some(Ok(ConversationState::Live)) => {}
        Some(Ok(state)) => return Err(JournalError::Closed { conversation: conversation.clone(), state }),
        Some(Err(why)) => return Err(decode(why).into()),
    }
    let Some(key) = key else { return Ok(None) };
    let held: Option<i64> = sqlx::query_scalar("SELECT seq FROM journal WHERE conversation = $1 AND dedup_key = $2")
        .bind(conversation.as_str())
        .bind(key)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(held.map(seq_of).transpose()?)
}

/// Append `entry` to the live conversation of `site` inside `tx`, unless a conversation of the site, archived ones
/// included, holds its dedup key. A site nobody has opened has no live conversation; open one first.
pub async fn append_to_site(
    tx: &mut Transaction<'_, Postgres>,
    site: &SitePath,
    entry: NewEntry,
) -> Result<Appended, JournalError> {
    let conversation = lock_live_conversation(tx, site).await?;
    if let Some(key) = &entry.dedup_key {
        let held: Option<i64> = sqlx::query_scalar(
            "SELECT j.seq FROM journal j JOIN conversations c ON c.id = j.conversation \
             WHERE j.dedup_key = $1 AND c.site = $2 LIMIT 1",
        )
        .bind(key)
        .bind(site.as_str())
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(seq) = held {
            return Ok(Appended::Held(seq_of(seq)?));
        }
    }
    write(tx, &conversation, entry).await.map(Appended::Wrote)
}

/// Lock the site's live conversation's row. A `FOR UPDATE` that waited on a Dismiss finds that row archived and
/// returns nothing, and its snapshot predates the next conversation, so it reads again.
async fn lock_live_conversation(tx: &mut Transaction<'_, Postgres>, site: &SitePath) -> Result<Ulid, JournalError> {
    for _ in 0..SITE_READS {
        let live: Option<String> =
            sqlx::query_scalar("SELECT id FROM conversations WHERE site = $1 AND state = 'live' FOR UPDATE")
                .bind(site.as_str())
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = live {
            return Ok(id.parse().map_err(decode)?);
        }
        let opened: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conversations WHERE site = $1)")
            .bind(site.as_str())
            .fetch_one(&mut **tx)
            .await?;
        if !opened {
            break;
        }
    }
    Err(JournalError::NoLiveConversation(site.clone()))
}

/// Take the conversation's next number and insert the entry under it; the caller holds the conversation's row lock.
async fn write(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    entry: NewEntry,
) -> Result<Written, JournalError> {
    let seq: i64 =
        sqlx::query_scalar("UPDATE conversations SET last_seq = last_seq + 1 WHERE id = $1 RETURNING last_seq")
            .bind(conversation.as_str())
            .fetch_one(&mut **tx)
            .await?;
    let at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO journal (conversation, seq, turn, kind, body, caused_by, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING at",
    )
    .bind(conversation.as_str())
    .bind(seq)
    .bind(entry.turn.as_ref().map(Ulid::as_str))
    .bind(entry.kind.as_str())
    .bind(&entry.body)
    .bind(&entry.caused_by)
    .bind(&entry.dedup_key)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Written(JournalEntry {
        v: EnvelopeVersion,
        seq: seq_of(seq)?,
        at: WallClock::from(at),
        conversation: conversation.clone(),
        turn: entry.turn,
        kind: entry.kind,
        caused_by: entry.caused_by,
        body: entry.body,
    }))
}
