//! Within `studio-api`, `scope_events` is spec 01 section 3.6's `GET /api/scope-events`, which exists so that a page
//! showing a scope at its head reads it again when it moves, though world-proto-server is not reachable from the
//! browser. It relays the client's one upstream stream per scope, shared in the process (spec 04 section 3.6), as
//! `scope.moved`, with a `ping` every 15 s of wall clock. Each named scope is followed from its head as the request
//! read it, so a changeset that lands before the client's own first read of the head is still told.

use std::convert::Infallible;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::header::HeaderName;
use axum::http::{HeaderValue, Uri};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rustc_hash::FxHashMap;
use serde::Serialize;
use studio_protocol::{
    GrammarError, Ping, Problem, ProblemType, ScopeEventKind, ScopeMoved, ScopePath, Seq, WallClock,
};
use tokio::sync::mpsc::{Sender, channel};
use tokio::time::{Instant, interval_at};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{StreamExt, StreamMap};
use url::form_urlencoded;
use wps_client::wps::Wps;
use wps_client::{ScopeEvent, ScopeEvents, WorldStore};

use crate::GateState;
use crate::journal::PING_EVERY;
use crate::problem::{ProblemAnswer, validation_failed, wps_problem};

/// How many scopes one stream follows at most (spec 01 section 3.6).
pub const MOST_SCOPES: usize = 16;
/// Events made but not yet taken by the connection; past them the relay waits on its client.
const OUTBOX: usize = 16;
const ACCEL_BUFFERING: HeaderName = HeaderName::from_static("x-accel-buffering");

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new().route("/api/scope-events", get(scope_events))
}

/// `GET /api/scope-events?scope={path}&scope={path}`.
async fn scope_events<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    RawQuery(query): RawQuery,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let scopes = named(query.as_deref().unwrap_or_default()).map_err(|why| validation_failed(why, instance))?;
    let tree = state.world.tree().await.map_err(|error| wps_problem(&error, instance))?;
    let (mut told, mut moves) = (FxHashMap::default(), StreamMap::new());
    for scope in scopes {
        let Some(head) = tree.head(&scope) else {
            let detail = format!("world-proto-server holds no scope {scope}.");
            return Err(Problem::new(ProblemType::NotFound, detail).at(instance).into());
        };
        moves.insert(scope.clone(), state.world.events_since(&scope, head));
        told.insert(scope, head);
    }
    let (out, events) = channel(OUTBOX);
    tokio::spawn(relay(moves, told, out));
    let body = Sse::new(ReceiverStream::new(events).map(Ok::<Event, Infallible>));
    Ok(([(ACCEL_BUFFERING, HeaderValue::from_static("no"))], body).into_response())
}

/// The scopes `query` names as `scope=`, each once, in the order named: at least one and at most `MOST_SCOPES`.
fn named(query: &str) -> Result<Vec<ScopePath>, String> {
    let mut scopes: Vec<ScopePath> = Vec::new();
    for (key, value) in form_urlencoded::parse(query.as_bytes()).filter(|(key, _)| key == "scope") {
        let scope: ScopePath = value.parse().map_err(|fault: GrammarError| format!("`{key}`: {fault}"))?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    match scopes.len() {
        0 => Err("name at least one scope to follow, as ?scope={path}".to_owned()),
        named if named > MOST_SCOPES => {
            Err(format!("a stream follows at most {MOST_SCOPES} scopes, and this request names {named}"))
        }
        _ => Ok(scopes),
    }
}

/// Relay each move as `scope.moved`, and a `ping` every `PING_EVERY`, until the page goes away. `told` holds, per
/// scope, the last changeset the page was told of, its head as the request read it to begin with.
async fn relay(mut moves: StreamMap<ScopePath, ScopeEvents>, mut told: FxHashMap<ScopePath, Seq>, out: Sender<Event>) {
    let mut pings = interval_at(Instant::now() + PING_EVERY, PING_EVERY);
    loop {
        let event = tokio::select! {
            heard = moves.next() => match heard {
                Some((_, heard)) => match moved(heard, &mut told) {
                    Some(moved) => event(ScopeEventKind::ScopeMoved, &moved),
                    None => continue,
                },
                None => return,
            },
            _ = pings.tick() => event(ScopeEventKind::Ping, &Ping { at: WallClock::now() }),
        };
        if out.send(event).await.is_err() {
            return;
        }
    }
}

/// What the page is told of `heard`, if anything new: the client's `Moved`, or its `Resynced` after a reconnect or
/// a changeset that landed before its first read, with `gap` set when changesets since the last one told go untold.
fn moved(heard: ScopeEvent, told: &mut FxHashMap<ScopePath, Seq>) -> Option<ScopeMoved> {
    let (scope, seq, gap) = match heard {
        ScopeEvent::Moved { scope, seq, gap, .. } => (scope, seq, gap),
        ScopeEvent::Resynced { scope, head } => (scope, head, false),
    };
    let last = told.get(&scope).copied().unwrap_or(Seq::new(0));
    if seq <= last {
        return None;
    }
    told.insert(scope.clone(), seq);
    Some(ScopeMoved { gap: gap || seq.get() > last.get() + 1, scope, seq })
}

fn event(kind: ScopeEventKind, body: &impl Serialize) -> Event {
    Event::default().event(kind.as_str()).json_data(body).expect("a protocol body is plain JSON and serializes")
}
