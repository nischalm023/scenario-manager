//! Within `agent`, `follow` is the test writer's stand-in for the loop (brief I1.03 section 2): an entry written
//! through `POST /api/dev/conversations/{c}/entries` makes the change to `turns` the loop makes when it journals that
//! entry, so the table and the journal agree and every reader of a turn's state reads the table. `message.user` and
//! `turn.started` make the turn's row (the loop makes it before both), `turn.waiting`, `turn.resumed` and
//! `turn.finished` move it, and any other entry must name a turn of its own conversation. Cards it writes make no row,
//! so acting on one answers `not-found`.

use serde_json::Value;
use sqlx::{Error as SqlxError, Postgres, Transaction};
use studio_protocol::{JournalKind, Ulid};
use thiserror::Error;

use crate::journal::NewEntry;

#[derive(Debug, Error)]
pub enum FollowError {
    #[error("names the turn {0}, which another conversation holds; give each conversation turns of its own")]
    Foreign(Ulid),
    #[error(
        "names the turn {0}, which this conversation has not started: write its message.user or turn.started first"
    )]
    Unknown(Ulid),
    #[error("would leave two turns of the conversation running or waiting, or two turns numbered alike: {0}")]
    Refused(String),
    #[error(transparent)]
    Database(#[from] SqlxError),
}

/// Inside `tx`, before `entry` is appended to `conversation`: the change to `turns` the loop makes with it.
pub async fn follow(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    entry: &NewEntry,
) -> Result<(), FollowError> {
    let Some(turn) = &entry.turn else { return Ok(()) };
    let held: Option<String> = sqlx::query_scalar("SELECT conversation FROM turns WHERE id = $1")
        .bind(turn.as_str())
        .fetch_optional(&mut **tx)
        .await?;
    match held.as_deref() {
        Some(other) if other != conversation.as_str() => return Err(FollowError::Foreign(turn.clone())),
        None if !matches!(entry.kind, JournalKind::MessageUser | JournalKind::TurnStarted) => {
            return Err(FollowError::Unknown(turn.clone()));
        }
        _ => {}
    }
    let body = &entry.body;
    let text = |pointer: &str| body.pointer(pointer).and_then(Value::as_str).map(str::to_owned);
    let changed = match entry.kind {
        JournalKind::MessageUser | JournalKind::TurnStarted if held.is_none() => {
            let next: i64 = sqlx::query_scalar("SELECT last_seq + 1 FROM conversations WHERE id = $1 FOR UPDATE")
                .bind(conversation.as_str())
                .fetch_one(&mut **tx)
                .await?;
            sqlx::query(
                "INSERT INTO turns (id, conversation, n, started_by, attended, browser_tab, state, message_seq) \
                 SELECT $1, $2, COALESCE($3, MAX(n) + 1, 1), $4, COALESCE($5, true), $6, 'running', COALESCE($7, $8) \
                 FROM turns WHERE conversation = $2",
            )
            .bind(turn.as_str())
            .bind(conversation.as_str())
            .bind(body.get("n").and_then(Value::as_i64).filter(|_| entry.kind == JournalKind::TurnStarted).map(|n| n as i32))
            .bind(text("/author/subject").unwrap_or_else(|| "dev".to_owned()))
            .bind(body.get("attended").and_then(Value::as_bool))
            .bind(text("/browser_tab"))
            .bind(body.get("message_seq").and_then(Value::as_i64))
            .bind(next)
            .execute(&mut **tx)
            .await
        }
        JournalKind::TurnStarted => {
            sqlx::query(
                "UPDATE turns SET n = COALESCE($2, n), started_by = COALESCE($3, started_by), \
                     attended = COALESCE($4, attended), browser_tab = $5, message_seq = COALESCE($6, message_seq) \
                 WHERE id = $1",
            )
            .bind(turn.as_str())
            .bind(body.get("n").and_then(Value::as_i64).map(|n| n as i32))
            .bind(text("/author/subject"))
            .bind(body.get("attended").and_then(Value::as_bool))
            .bind(text("/browser_tab"))
            .bind(body.get("message_seq").and_then(Value::as_i64))
            .execute(&mut **tx)
            .await
        }
        JournalKind::TurnWaiting => {
            sqlx::query(
                "UPDATE turns SET state = 'waiting', waiting_on = $2, waiting_id = $3, waiting_since = now(), \
                     lease_owner = NULL, lease_expires_at = NULL WHERE id = $1",
            )
            .bind(turn.as_str())
            .bind(text("/on"))
            .bind(text("/id"))
            .execute(&mut **tx)
            .await
        }
        JournalKind::TurnResumed => {
            sqlx::query(
                "UPDATE turns SET state = 'running', waiting_on = NULL, waiting_id = NULL, waiting_since = NULL \
                 WHERE id = $1 AND ended_at IS NULL",
            )
            .bind(turn.as_str())
            .execute(&mut **tx)
            .await
        }
        JournalKind::TurnFinished => {
            sqlx::query(
                "UPDATE turns SET state = $2, end_reason = $3, budget = $4, ended_at = now(), waiting_on = NULL, \
                     waiting_id = NULL, waiting_since = NULL, lease_owner = NULL, lease_expires_at = NULL WHERE id = $1",
            )
            .bind(turn.as_str())
            .bind(text("/outcome"))
            .bind(text("/end_reason"))
            .bind(text("/budget"))
            .execute(&mut **tx)
            .await
        }
        _ => return Ok(()),
    };
    match changed {
        Ok(_) => Ok(()),
        Err(fault) if fault.as_database_error().is_some_and(|fault| fault.constraint().is_some()) => {
            Err(FollowError::Refused(fault.to_string()))
        }
        Err(fault) => Err(FollowError::Database(fault)),
    }
}
