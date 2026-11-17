//! Within `listeners`, `admission` is who may open a socket: a runner by its attempt's token, under the runners' door's
//! own rule (the view `live_runner_tokens`: a token opens its attempt while the attempt's Job is not known stopped),
//! and a viewer by a watch ticket the API minted for the attempt (the view `live_watch_tickets`: until it expires, by
//! the database's clock), each matched by its SHA-256 as the API keeps it. Both rules are the API's, kept once in its
//! migrations; this server only asks. A database that cannot answer refuses `unavailable`, which a runner takes as
//! worth another try.

use run_invocation::Token;
use sqlx::PgPool;
use studio_protocol::StreamRefusal;

use crate::streams::StreamKey;

/// Whether `token` opens the runner's attempt `key`.
pub async fn runner(pool: &PgPool, key: &StreamKey, token: &str) -> Result<(), StreamRefusal> {
    admitted(pool, "SELECT 1 FROM live_runner_tokens WHERE token_sha256 = $1 AND run = $2 AND n = $3", key, token).await
}

/// Whether `token` is a live watch ticket for the attempt `key`.
pub async fn viewer(pool: &PgPool, key: &StreamKey, token: &str) -> Result<(), StreamRefusal> {
    let query = "SELECT 1 FROM live_watch_tickets WHERE token_sha256 = $1 AND run_id = $2 AND n = $3";
    admitted(pool, query, key, token).await
}

async fn admitted(pool: &PgPool, query: &str, key: &StreamKey, token: &str) -> Result<(), StreamRefusal> {
    let found = sqlx::query_scalar::<_, i32>(query)
        .bind(Token::presented(token).sha256())
        .bind(key.run.as_str())
        .bind(i32::try_from(key.attempt.0).unwrap_or(i32::MAX))
        .fetch_optional(pool)
        .await;
    match found {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(StreamRefusal::TokenInvalid),
        Err(fault) => {
            let StreamKey { run, attempt } = key;
            eprintln!("studio-streaming: {run}/{}'s token could not be checked: {fault}", attempt.0);
            Err(StreamRefusal::Unavailable)
        }
    }
}
