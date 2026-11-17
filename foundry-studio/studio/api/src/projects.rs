//! Within `studio-api`, `projects` are what a conversation belongs to (spec 01 section 3.3). Brief I1.04 pulls the
//! table forward from task 24, whose routes list and make projects; until then a conversation takes the default
//! project, which the service makes at start with `STUDIO_ENV=local`.

use sqlx::{Error as SqlxError, PgConnection, PgPool};
use studio_protocol::Ulid;

use crate::db::{decode, mint_ulid};

/// The default project the service makes locally: "Demo", for the customer `demo`.
pub const LOCAL_DEFAULT_NAME: &str = "Demo";
pub const LOCAL_DEFAULT_CUSTOMER: &str = "demo";

/// Make the local default project unless a project is the default already; true when this call made it.
pub async fn make_local_default(pool: &PgPool) -> Result<bool, SqlxError> {
    let made = sqlx::query(
        "INSERT INTO projects (id, name, customer, is_default) VALUES ($1, $2, $3, true) \
         ON CONFLICT (is_default) WHERE is_default DO NOTHING",
    )
    .bind(mint_ulid().as_str())
    .bind(LOCAL_DEFAULT_NAME)
    .bind(LOCAL_DEFAULT_CUSTOMER)
    .execute(pool)
    .await?;
    Ok(made.rows_affected() == 1)
}

/// The default project, if one is.
pub async fn default_project(conn: &mut PgConnection) -> Result<Option<Ulid>, SqlxError> {
    let id: Option<String> =
        sqlx::query_scalar("SELECT id FROM projects WHERE is_default").fetch_optional(conn).await?;
    id.map(|id| id.parse().map_err(decode)).transpose()
}
