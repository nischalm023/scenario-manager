//! Within `conversations`, `snapshot` is how a tab reads a conversation (spec 01 section 3.6; spec 02 section 3.3,
//! step 1): the snapshot it loads, its last 200 entries with the projection of what they name and every pending
//! card, and the pages of earlier entries it reads back, each read in one transaction at one journal number.

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use rustc_hash::FxHashSet;
use serde::Deserialize;
use sqlx::{Error as SqlxError, PgPool, Postgres, Transaction};
use studio_protocol::{
    ActiveTurn, Conversation, EntriesPage, JournalKind, JournalSeq, Problem, ProblemType, Snapshot, Ulid,
};

use super::conversation_in;
use super::rows::load;
use crate::AppState;
use crate::journal::{FOLDED_KINDS, Fold, Named, entries_after, entries_of_kinds};
use crate::problem::{ProblemAnswer, internal, validation_failed};

const SNAPSHOT_ENTRIES: u32 = 200;
const PAGE_DEFAULT: u32 = 200;
const PAGE_MOST: u32 = 500;

/// `GET /api/conversations/{c}`: the snapshot, read in one transaction at the conversation's `last_seq`.
pub async fn snapshot(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<Snapshot>, ProblemAnswer> {
    let instance = uri.path();
    let id = conversation_in(path, instance)?;
    let fault = |fault| internal("read the conversation", fault, instance);
    let mut tx = reading(&state.pool).await.map_err(fault)?;
    let conversation = found(load(&mut tx, &id, &state.model_label).await.map_err(fault)?, &id, instance)?;
    let last_seq = conversation.last_seq;
    let from = last_seq.get().saturating_sub(u64::from(SNAPSHOT_ENTRIES)).max(conversation.first_seq.get() - 1);
    let entries = entries_after(&mut tx, &id, JournalSeq::new(from), None, SNAPSHOT_ENTRIES).await.map_err(fault)?;
    let fold = Fold::of(&entries_of_kinds(&mut tx, &id, &FOLDED_KINDS).await.map_err(fault)?);
    tx.commit().await.map_err(fault)?;
    report_unread(&id, &fold);
    // A block whose `text.done` the entries hold no longer streams, though its writer may not have said so yet.
    let done: FxHashSet<&str> = entries
        .iter()
        .filter(|entry| entry.kind == JournalKind::TextDone)
        .filter_map(|entry| entry.body.get("block").and_then(|block| block.as_str()))
        .collect();
    let active_turn = fold.open_turn().map(|turn| ActiveTurn {
        id: turn.id.clone(),
        n: turn.n,
        state: turn.state,
        waiting_on: turn.waiting_on,
        streaming: state.hub.streaming(&id).into_iter().filter(|block| !done.contains(block.block.as_str())).collect(),
    });
    let projection = fold.projection(&Named::of(&entries), true);
    Ok(Json(Snapshot { conversation, entries, projection, pending: fold.pending(), active_turn, last_seq }))
}

#[derive(Deserialize)]
pub struct PageQuery {
    after: Option<u64>,
    limit: Option<u32>,
}

/// `GET /api/conversations/{c}/entries?after=&limit=`: at most 500 entries after `after`, from the conversation's
/// first when `after` is below it, with the projection of what they name.
pub async fn entries(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<EntriesPage>, ProblemAnswer> {
    let instance = uri.path();
    let id = conversation_in(path, instance)?;
    let Query(PageQuery { after, limit }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let limit = limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MOST);
    let fault = |fault| internal("read the conversation's entries", fault, instance);
    let mut tx = reading(&state.pool).await.map_err(fault)?;
    let conversation = found(load(&mut tx, &id, &state.model_label).await.map_err(fault)?, &id, instance)?;
    let after = JournalSeq::new(after.unwrap_or(0).max(conversation.first_seq.get() - 1));
    let mut entries = entries_after(&mut tx, &id, after, None, limit + 1).await.map_err(fault)?;
    let fold = Fold::of(&entries_of_kinds(&mut tx, &id, &FOLDED_KINDS).await.map_err(fault)?);
    tx.commit().await.map_err(fault)?;
    report_unread(&id, &fold);
    let more = entries.len() > limit as usize;
    entries.truncate(limit as usize);
    let next = entries.last().filter(|_| more).map(|entry| entry.seq);
    let projection = fold.projection(&Named::of(&entries), false);
    Ok(Json(EntriesPage { entries, projection, next }))
}

/// A transaction whose every read sees the database as its first did, so entries and projection agree.
async fn reading(pool: &PgPool) -> Result<Transaction<'static, Postgres>, SqlxError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY").execute(&mut *tx).await?;
    Ok(tx)
}

fn found(conversation: Option<Conversation>, id: &Ulid, instance: &str) -> Result<Conversation, ProblemAnswer> {
    conversation.ok_or_else(|| {
        Problem::new(ProblemType::NotFound, format!("There is no conversation {id}.")).at(instance).into()
    })
}

/// Say in the log which entries the fold could not read: they draw nothing, which a writer's fault explains.
fn report_unread(conversation: &Ulid, fold: &Fold) {
    let unread = fold.unread();
    if !unread.is_empty() {
        let numbers: Vec<u64> = unread.iter().map(|seq| seq.get()).collect();
        eprintln!("studio-api: conversation {conversation}: entries {numbers:?} do not read as their kinds' bodies");
    }
}
