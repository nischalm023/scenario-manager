//! Within `journal`, `read` turns the journal's rows back into the envelopes the writer made (spec 01 section 3.5):
//! a page of entries after a number, and the entries of the kinds a fold reads.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{EnvelopeVersion, JournalEntry, JournalKind, JournalSeq, Ulid, WallClock};

use crate::db::{decode, seq_column, seq_of};

const COLUMNS: &str = "conversation, seq, at, turn, kind, body, caused_by";

/// At most `limit` entries of `conversation` numbered after `after`, and up to `until` when it is given, in order.
pub async fn entries_after(
    conn: &mut PgConnection,
    conversation: &Ulid,
    after: JournalSeq,
    until: Option<JournalSeq>,
    limit: u32,
) -> Result<Vec<JournalEntry>, SqlxError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM journal WHERE conversation = $1 AND seq > $2 AND ($3::BIGINT IS NULL OR seq <= $3) \
         ORDER BY seq LIMIT $4"
    );
    let rows = sqlx::query(&sql)
        .bind(conversation.as_str())
        .bind(seq_column(after))
        .bind(until.map(seq_column))
        .bind(i64::from(limit))
        .fetch_all(conn)
        .await?;
    rows.iter().map(entry_of).collect()
}

/// Every entry of `conversation` whose kind is one of `kinds`, in order.
pub async fn entries_of_kinds(
    conn: &mut PgConnection,
    conversation: &Ulid,
    kinds: &[JournalKind],
) -> Result<Vec<JournalEntry>, SqlxError> {
    let kinds: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
    let sql = format!("SELECT {COLUMNS} FROM journal WHERE conversation = $1 AND kind = ANY($2) ORDER BY seq");
    let rows = sqlx::query(&sql).bind(conversation.as_str()).bind(&kinds).fetch_all(conn).await?;
    rows.iter().map(entry_of).collect()
}

fn entry_of(row: &PgRow) -> Result<JournalEntry, SqlxError> {
    let kind: String = row.try_get("kind")?;
    let turn: Option<String> = row.try_get("turn")?;
    Ok(JournalEntry {
        v: EnvelopeVersion,
        seq: seq_of(row.try_get("seq")?)?,
        at: WallClock::from(row.try_get::<DateTime<Utc>, _>("at")?),
        conversation: row.try_get::<String, _>("conversation")?.parse().map_err(decode)?,
        turn: turn.map(|turn| turn.parse()).transpose().map_err(decode)?,
        kind: JournalKind::ALL
            .iter()
            .copied()
            .find(|known| known.as_str() == kind)
            .ok_or_else(|| decode(format!("`{kind}` is no journaled kind")))?,
        caused_by: row.try_get("caused_by")?,
        body: row.try_get("body")?,
    })
}
