//! Within `conversations`, `rows` reads a `conversations` row (spec 01 section 3.3) as the protocol's `Conversation`.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{Conversation, SitePath, Subject, Ulid, WallClock};

use crate::db::{decode, seq_of};

pub const COLUMNS: &str =
    "id, project, site, state, title, created_by, created_at, bound_at, ended_at, ended_by, first_seq, last_seq";

pub async fn load(conn: &mut PgConnection, id: &Ulid, model_label: &str) -> Result<Option<Conversation>, SqlxError> {
    let sql = format!("SELECT {COLUMNS} FROM conversations WHERE id = $1");
    let row = sqlx::query(&sql).bind(id.as_str()).fetch_optional(conn).await?;
    row.map(|row| from_row(&row, model_label)).transpose()
}

/// The live conversation of `site`, if it has one.
pub async fn live_of_site(
    conn: &mut PgConnection,
    site: &SitePath,
    model_label: &str,
) -> Result<Option<Conversation>, SqlxError> {
    let sql = format!("SELECT {COLUMNS} FROM conversations WHERE site = $1 AND state = 'live'");
    let row = sqlx::query(&sql).bind(site.as_str()).fetch_optional(conn).await?;
    row.map(|row| from_row(&row, model_label)).transpose()
}

/// The live new-site thread `person` made, if they have one.
pub async fn live_thread_of(
    conn: &mut PgConnection,
    person: &Subject,
    model_label: &str,
) -> Result<Option<Conversation>, SqlxError> {
    let sql = format!("SELECT {COLUMNS} FROM conversations WHERE created_by = $1 AND site IS NULL AND state = 'live'");
    let row = sqlx::query(&sql).bind(person.as_str()).fetch_optional(conn).await?;
    row.map(|row| from_row(&row, model_label)).transpose()
}

pub fn from_row(row: &PgRow, model_label: &str) -> Result<Conversation, SqlxError> {
    let wall = |column: &str| row.try_get::<Option<DateTime<Utc>>, _>(column).map(|at| at.map(WallClock::from));
    let site: Option<String> = row.try_get("site")?;
    let ended_by: Option<String> = row.try_get("ended_by")?;
    Ok(Conversation {
        id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        project: row.try_get::<String, _>("project")?.parse().map_err(decode)?,
        site: site.map(|site| site.parse()).transpose().map_err(decode)?,
        state: row.try_get::<String, _>("state")?.parse().map_err(decode)?,
        title: row.try_get("title")?,
        created_by: row.try_get::<String, _>("created_by")?.parse().map_err(decode)?,
        created_at: WallClock::from(row.try_get::<DateTime<Utc>, _>("created_at")?),
        bound_at: wall("bound_at")?,
        ended_at: wall("ended_at")?,
        ended_by: ended_by.map(|subject| subject.parse()).transpose().map_err(decode)?,
        first_seq: seq_of(row.try_get("first_seq")?)?,
        last_seq: seq_of(row.try_get("last_seq")?)?,
        model_label: model_label.to_owned(),
    })
}
