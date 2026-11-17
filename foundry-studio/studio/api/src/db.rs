//! Within `studio-api`, `db` is the service's Postgres (spec 01 section 3.3): the pool every route shares, the
//! migrations embedded in the binary, which bring an empty database up to this build's tables before the first
//! request is served, and the conventions every table keeps: ids are ULIDs minted at random, and a value a column
//! promises but does not hold is a decode fault. Queries are runtime `sqlx::query`, as crucible's are, so the
//! workspace builds with no database in sight.

use std::error::Error;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::migrate::MigrateError;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::{EvaluationNumber, JournalSeq, Ulid};
use ulid::Ulid as RandomUlid;

use crate::config::Secret;

pub async fn connect(url: &Secret) -> Result<PgPool, SqlxError> {
    PgPoolOptions::new().connect(url.expose()).await
}

/// Apply every migration this build carries that the database has not had yet.
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/// A new object's id: a ULID with 80 random bits after its millisecond, never derived from anything.
pub fn mint_ulid() -> Ulid {
    RandomUlid::new().to_string().parse().expect("the ulid crate writes Crockford's base32 in upper case")
}

/// A value read back that does not hold what its column promises.
pub fn decode(why: impl Into<Box<dyn Error + Send + Sync>>) -> SqlxError {
    SqlxError::Decode(why.into())
}

/// A journal number as Postgres holds it (`BIGINT`), as the protocol's type.
pub fn seq_of(seq: i64) -> Result<JournalSeq, SqlxError> {
    u64::try_from(seq).map(JournalSeq::new).map_err(|_| decode(format!("journal number {seq} is below zero")))
}

/// A journal number as a bound on Postgres's `BIGINT`: one past its range, which only a client's cursor can be,
/// reads as its largest, past every entry.
pub fn seq_column(seq: JournalSeq) -> i64 {
    i64::try_from(seq.get()).unwrap_or(i64::MAX)
}

/// An evaluation's number from its `INTEGER` column, as the protocol's type; a negative one is a decode fault.
pub fn evaluation_of(n: i32) -> Result<EvaluationNumber, SqlxError> {
    u32::try_from(n).map(EvaluationNumber).map_err(|_| decode(format!("evaluation number {n} is below zero")))
}

/// An evaluation's number as its `INTEGER` column holds it; one past `i32::MAX`, beyond any draft's count, is
/// refused rather than wrapped.
pub fn evaluation_column(n: EvaluationNumber) -> Result<i32, SqlxError> {
    i32::try_from(n.0).map_err(|_| decode(format!("evaluation number {n} is past what its column holds")))
}

/// A protocol enumeration as its `TEXT` column holds it: its JSON word.
pub fn word<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(word)) => word,
        _ => unreachable!("a protocol enumeration serializes as its word"),
    }
}

/// A column's word read back as its protocol enumeration; a word it does not name is a decode fault.
pub fn read_word<T: DeserializeOwned>(text: &str) -> Result<T, SqlxError> {
    serde_json::from_value(Value::String(text.to_owned())).map_err(decode)
}
