//! Within `agent`, `locks` are the one order in which every writer of a turn takes its rows' locks (spec 01 sections
//! 3.4 and 3.10): the conversation's row, then the turn's, and only then its question's or approval's, so a message,
//! an answer, a decision, a press, a wait and an ending cannot deadlock one another.

use sqlx::{Error as SqlxError, Postgres, Transaction};
use studio_protocol::{TurnState, Ulid};

use crate::db::{decode, read_word};

/// A turn and its conversation as a writer reads them under their locks.
pub(super) struct Locked {
    pub conversation: Ulid,
    /// The conversation's own state (`live` takes entries), its site, who made it and the provider it began with.
    pub lived: Option<String>,
    pub site: Option<String>,
    pub created_by: Option<String>,
    pub provider: Option<String>,
    pub state: TurnState,
    pub waiting_on: Option<String>,
    pub waiting_id: Option<String>,
}

/// Inside `tx`: the locks every writer of a turn takes, in one order (the conversation's row, then the turn's, and only
/// then its question's or approval's), so a message, an answer, a decision, a press, a wait and an ending cannot
/// deadlock; `None` when no turn has that id. A turn's conversation never changes, so it is read before the locks.
pub(super) async fn lock_turn(tx: &mut Transaction<'_, Postgres>, turn: &str) -> Result<Option<Locked>, SqlxError> {
    let conversation: Option<String> =
        sqlx::query_scalar("SELECT conversation FROM turns WHERE id = $1").bind(turn).fetch_optional(&mut **tx).await?;
    let Some(conversation) = conversation else { return Ok(None) };
    let (lived, site, created_by, provider): (Option<String>, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as("SELECT state, site, created_by, provider FROM conversations WHERE id = $1 FOR UPDATE")
            .bind(&conversation)
            .fetch_one(&mut **tx)
            .await?;
    let (state, waiting_on, waiting_id): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT state, waiting_on, waiting_id FROM turns WHERE id = $1 FOR UPDATE")
            .bind(turn)
            .fetch_one(&mut **tx)
            .await?;
    Ok(Some(Locked {
        conversation: conversation.parse().map_err(decode)?,
        lived,
        site,
        created_by,
        provider,
        state: read_word(&state)?,
        waiting_on,
        waiting_id,
    }))
}
