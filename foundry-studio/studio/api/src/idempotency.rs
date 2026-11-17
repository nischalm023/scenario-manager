//! Within `studio-api`, `idempotency` is the layer that makes a write safe to repeat (spec 01 section 3.13, after
//! draft-ietf-httpapi-idempotency-key-header-07). A route wrapped by `idempotent` takes an `Idempotency-Key`, which
//! the browser draws fresh per press and the agent loop sets to the tool call's id. The first request under a
//! person's key runs and its answer is stored; a repeat with the same method, route and body gets that answer again,
//! marked `Idempotent-Replayed: true`, and runs nothing. Another request under the key is refused, and so is a repeat
//! while the first still runs. A 5xx answer is not stored, so its retry runs again. A key lasts 24 hours of wall
//! clock.
//!
//! The route runs in a task of its own, so a caller that hangs up mid-request leaves it to finish and store its
//! answer for the retry. "Still runs" is a lease (lead decided; spec 01 states no recovery from a crash): the request
//! holding a key renews its claim while the route runs, and a claim left unrenewed for a minute of wall clock, its
//! process dead or its answer unstored, is taken over by a repeat with the same digest, which runs again as after a
//! 5xx. Each taking of a key mints an owner token, and a request stores or releases the key only under its own, so a
//! request whose claim was taken over never settles the claim that replaced it.

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Error as SqlxError, PgPool, Row};
use studio_protocol::{Problem, ProblemType, Subject};
use tokio::task::{JoinError, JoinHandle};
use tokio::time::{Instant, interval_at};

use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal};
use crate::{AppState, BODY_LIMIT};

/// The canonical JSON a request's body is digested in, which the API's other digests take too.
pub(crate) use studio_protocol::write_canonical;

const KEY_HEADER: HeaderName = HeaderName::from_static("idempotency-key");
const REPLAYED_HEADER: HeaderName = HeaderName::from_static("idempotent-replayed");
const MAX_KEY_CHARS: usize = 255;
/// How long a key is kept, as a Postgres interval: spec 01 section 3.13's 24 hours of wall clock.
const KEPT_FOR: &str = "24 hours";
const PURGE_EVERY: Duration = Duration::from_secs(60 * 60);
/// A claim's lease, and how often (wall clock) its request renews it while the route runs: three renewals to a
/// lease, so one slow round trip to the database never lets a live claim lapse.
const LEASE: Duration = Duration::from_secs(60);
const RENEW_EVERY: Duration = Duration::from_secs(20);
/// How many times a claim looks again when the key it found changed hands between its two statements, released or
/// its lease run out; past that, the key keeps changing hands, which is a press still in flight.
const CLAIM_ATTEMPTS: usize = 3;

/// Make `route` a write that takes an `Idempotency-Key`, whatever state its router carries: `AppState`, or a
/// `GateState` for a route that also reads the world. It must sit behind the identity layer, since a key is a person's.
pub fn idempotent<T: Clone + Send + Sync + 'static>(state: &AppState, route: MethodRouter<T>) -> MethodRouter<T> {
    route.route_layer(from_fn_with_state(state.clone(), take_key))
}

async fn take_key(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    let refuse = |type_: ProblemType, detail: String| {
        ProblemAnswer::from(Problem::new(type_, detail).at(path.clone())).into_response()
    };
    let Some(Identity { subject, .. }) = parts.extensions.get::<Identity>().cloned() else {
        return internal("tell who is asking", "the idempotency layer ran outside the identity layer", &path)
            .into_response();
    };
    let Some(key) = key_of(&parts.headers) else {
        let detail =
            "A write here takes an Idempotency-Key header: 1 to 255 of A-Z, a-z, 0-9, _ and -, fresh per press.";
        return refuse(ProblemType::IdempotencyKeyMissing, detail.to_owned());
    };
    let Ok(body) = to_bytes(body, BODY_LIMIT).await else {
        let detail = "The body could not be read whole; a body is at most 1 MiB.";
        return refuse(ProblemType::ValidationFailed, detail.to_owned());
    };
    let route = parts.uri.path_and_query().map_or(path.as_str(), |route| route.as_str()).to_owned();
    let held = Held { pool: state.pool.clone(), subject, digest: request_digest(&parts.method, &route, &body), key };
    match held.claim(&parts.method, &route).await {
        Ok(Claim::Fresh(token)) => {
            Owner { held, token }.run(Request::from_parts(parts, Body::from(body)), next, path).await
        }
        Ok(Claim::Replay(stored)) => stored.replay(&path),
        Ok(Claim::InFlight) => {
            let detail =
                format!("The request first sent with the key {} has not finished; ask again once it has.", held.key);
            refuse(ProblemType::IdempotencyKeyInFlight, detail)
        }
        Ok(Claim::Reused) => {
            let detail =
                format!("The key {} was first sent with another request; a new request takes a new key.", held.key);
            refuse(ProblemType::IdempotencyKeyReused, detail)
        }
        Err(fault) => internal("look up the Idempotency-Key", fault, &path).into_response(),
    }
}

/// What a person's key holds when a request arrives with it. `Fresh` is this request's own claim on the key, taken
/// new, past its 24 hours, or over a claim on the same request whose lease ran out.
enum Claim {
    Fresh(OwnerToken),
    Replay(Stored),
    InFlight,
    Reused,
}

struct OwnerToken(String);

/// A person's key, and the digest of the request that came with it.
struct Held {
    pool: PgPool,
    subject: Subject,
    key: String,
    digest: String,
}

impl Held {
    /// Take the key for this request, or learn what it holds: a stored answer, a request still running, or another
    /// request.
    async fn claim(&self, method: &Method, route: &str) -> Result<Claim, SqlxError> {
        for _ in 0..CLAIM_ATTEMPTS {
            let taken = sqlx::query_scalar::<_, String>(
                "INSERT INTO idempotency_keys \
                     (subject, key, method, route, request_digest, state, owner_token, lease_expires_at) \
                 VALUES ($1, $2, $3, $4, $5, 'in_flight', gen_random_uuid(), now() + $7) \
                 ON CONFLICT (subject, key) DO UPDATE SET method = EXCLUDED.method, route = EXCLUDED.route, \
                     request_digest = EXCLUDED.request_digest, state = 'in_flight', status = NULL, \
                     content_type = NULL, response = NULL, created_at = now(), \
                     owner_token = EXCLUDED.owner_token, lease_expires_at = EXCLUDED.lease_expires_at \
                 WHERE idempotency_keys.created_at < now() - $6::interval \
                     OR (idempotency_keys.state = 'in_flight' AND idempotency_keys.lease_expires_at < now() \
                         AND idempotency_keys.request_digest = EXCLUDED.request_digest) \
                 RETURNING owner_token::text",
            )
            .bind(self.subject.as_str())
            .bind(&self.key)
            .bind(method.as_str())
            .bind(route)
            .bind(&self.digest)
            .bind(KEPT_FOR)
            .bind(LEASE)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(token) = taken {
                return Ok(Claim::Fresh(OwnerToken(token)));
            }
            let held = sqlx::query(
                "SELECT request_digest, state, lease_expires_at < now() AS lapsed, status, content_type, response \
                 FROM idempotency_keys WHERE subject = $1 AND key = $2",
            )
            .bind(self.subject.as_str())
            .bind(&self.key)
            .fetch_optional(&self.pool)
            .await?;
            let Some(row) = held else { continue };
            if row.try_get::<String, _>("request_digest")? != self.digest {
                return Ok(Claim::Reused);
            }
            if row.try_get::<String, _>("state")? == "in_flight" {
                if row.try_get::<bool, _>("lapsed")? {
                    continue;
                }
                return Ok(Claim::InFlight);
            }
            let (status, content_type, body) =
                (row.try_get("status")?, row.try_get("content_type")?, row.try_get("response")?);
            return Ok(Claim::Replay(Stored { status, content_type, body }));
        }
        Ok(Claim::InFlight)
    }
}

/// The request whose claim holds a key in flight, with the token its claim was minted with, a random v4 Postgres
/// mints each time a request takes a key: everything it does to the key names that token, so it acts only while the
/// claim is still its own.
struct Owner {
    held: Held,
    token: OwnerToken,
}

impl Owner {
    /// Run the route in a task of its own, renewing the lease while it runs, and settle the key by its answer.
    async fn run(self, request: Request, next: Next, path: String) -> Response {
        let instance = path.clone();
        let task = tokio::spawn(async move {
            let answer = self.renew_until_answered(tokio::spawn(next.run(request))).await;
            self.settle(answer, &path).await
        });
        task.await.unwrap_or_else(|fault| internal("answer the request", fault, &instance).into_response())
    }

    async fn renew_until_answered(&self, mut route: JoinHandle<Response>) -> Result<Response, JoinError> {
        let mut renewals = interval_at(Instant::now() + RENEW_EVERY, RENEW_EVERY);
        let mut ours = true;
        loop {
            tokio::select! {
                answer = &mut route => return answer,
                _ = renewals.tick(), if ours => ours = self.renew().await,
            }
        }
    }

    /// Push the lease a whole `LEASE` past now; false once the claim is no longer this request's, which ends the
    /// renewals. A database fault keeps the claim, renewed again at the next tick.
    async fn renew(&self) -> bool {
        let renewed = sqlx::query(
            "UPDATE idempotency_keys SET lease_expires_at = now() + $4 \
             WHERE subject = $1 AND key = $2 AND owner_token = $3::uuid AND state = 'in_flight'",
        )
        .bind(self.held.subject.as_str())
        .bind(&self.held.key)
        .bind(&self.token.0)
        .bind(LEASE)
        .execute(&self.held.pool)
        .await;
        match renewed {
            Ok(done) if done.rows_affected() == 0 => {
                let key = &self.held.key;
                eprintln!(
                    "studio-api: the claim on Idempotency-Key {key} lapsed while its route ran, and was taken over"
                );
                false
            }
            Ok(_) => true,
            Err(fault) => {
                eprintln!("studio-api: the lease on Idempotency-Key {} was not renewed: {fault}", self.held.key);
                true
            }
        }
    }

    /// Store the route's answer under the key; or, when the route failed on the service's side (a 5xx, a panic),
    /// release the key so that a retry runs again.
    async fn settle(self, answer: Result<Response, JoinError>, path: &str) -> Response {
        let response = match answer {
            Ok(response) => response,
            Err(fault) => {
                self.release().await;
                return internal("answer the request", fault, path).into_response();
            }
        };
        let (parts, body) = response.into_parts();
        let body = match to_bytes(body, usize::MAX).await {
            Ok(body) => body,
            Err(fault) => {
                self.release().await;
                return internal("read the route's answer", fault, path).into_response();
            }
        };
        if parts.status.is_server_error() {
            self.release().await;
        } else {
            let content_type = parts.headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok());
            self.store(parts.status, content_type, &body).await;
        }
        Response::from_parts(parts, Body::from(body))
    }

    async fn store(&self, status: StatusCode, content_type: Option<&str>, body: &[u8]) {
        let stored = sqlx::query(
            "UPDATE idempotency_keys SET state = 'done', status = $4, content_type = $5, response = $6 \
             WHERE subject = $1 AND key = $2 AND owner_token = $3::uuid AND state = 'in_flight'",
        )
        .bind(self.held.subject.as_str())
        .bind(&self.held.key)
        .bind(&self.token.0)
        .bind(i32::from(status.as_u16()))
        .bind(content_type)
        .bind(body)
        .execute(&self.held.pool)
        .await;
        let key = &self.held.key;
        match stored {
            Ok(done) if done.rows_affected() == 0 => {
                eprintln!(
                    "studio-api: the answer under Idempotency-Key {key} went unstored: the key is no longer its claim"
                );
            }
            Ok(_) => {}
            Err(fault) => eprintln!(
                "studio-api: the answer under Idempotency-Key {key} went unstored; a repeat runs again once its lease \
                 runs out: {fault}"
            ),
        }
    }

    async fn release(&self) {
        let released = sqlx::query(
            "DELETE FROM idempotency_keys \
             WHERE subject = $1 AND key = $2 AND owner_token = $3::uuid AND state = 'in_flight'",
        )
        .bind(self.held.subject.as_str())
        .bind(&self.held.key)
        .bind(&self.token.0)
        .execute(&self.held.pool)
        .await;
        if let Err(fault) = released {
            let key = &self.held.key;
            eprintln!(
                "studio-api: Idempotency-Key {key} was not released; a retry runs once its lease runs out: {fault}"
            );
        }
    }
}

/// A first answer as its key keeps it.
struct Stored {
    status: i32,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Stored {
    fn replay(self, path: &str) -> Response {
        let Some(status) = u16::try_from(self.status).ok().and_then(|code| StatusCode::from_u16(code).ok()) else {
            return internal("replay the stored answer", format!("status {} is not HTTP's", self.status), path)
                .into_response();
        };
        let mut response = (status, self.body).into_response();
        let headers = response.headers_mut();
        headers.remove(CONTENT_TYPE);
        if let Some(content_type) = self.content_type.and_then(|text| HeaderValue::from_str(&text).ok()) {
            headers.insert(CONTENT_TYPE, content_type);
        }
        headers.insert(REPLAYED_HEADER, HeaderValue::from_static("true"));
        response
    }
}

/// The request's key, when its `Idempotency-Key` header holds one: 1 to 255 of `A-Z`, `a-z`, `0-9`, `_` and `-`.
fn key_of(headers: &HeaderMap) -> Option<String> {
    let key = headers.get(KEY_HEADER)?.to_str().ok()?;
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-';
    ((1..=MAX_KEY_CHARS).contains(&key.len()) && key.bytes().all(allowed)).then(|| key.to_owned())
}

/// The digest a key holds: SHA-256 of the method, the route and the body, the body canonical when it is JSON (object
/// keys sorted, no whitespace), so a repeat that encodes the same body differently is still the same request.
fn request_digest(method: &Method, route: &str, body: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(method.as_str());
    digest.update(b"\n");
    digest.update(route);
    digest.update(b"\n");
    match serde_json::from_slice::<Value>(body) {
        Ok(json) => {
            let mut canonical = Vec::new();
            write_canonical(&json, &mut canonical);
            digest.update(&canonical);
        }
        Err(_) => digest.update(body),
    }
    format!("{:x}", digest.finalize())
}

/// Delete the keys older than 24 hours, once an hour (wall clock), for as long as the service runs. A claim already
/// takes an older key afresh, so this only keeps the table small.
pub async fn purge_hourly(pool: PgPool) {
    let mut hours = interval_at(Instant::now() + PURGE_EVERY, PURGE_EVERY);
    loop {
        hours.tick().await;
        if let Err(fault) = purge_expired(&pool).await {
            eprintln!("studio-api: expired Idempotency-Keys were not purged: {fault}");
        }
    }
}

pub async fn purge_expired(pool: &PgPool) -> Result<u64, SqlxError> {
    let purged = sqlx::query("DELETE FROM idempotency_keys WHERE created_at < now() - $1::interval")
        .bind(KEPT_FOR)
        .execute(pool)
        .await?;
    Ok(purged.rows_affected())
}
