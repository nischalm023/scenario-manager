//! Within `studio-api`, `conversations` are a site's conversations (spec 01 sections 3.6 and 3.17, ruling D94): one
//! live conversation per site, and before a site exists one new-site thread per person. This module mounts the
//! routes that open one, list a site's, read one (its snapshot, its pages of entries and its stream) and dismiss
//! one. Only a site world-proto-server lists has a conversation to open.

mod lifecycle;
mod panel_events;
mod rows;
mod snapshot;

use axum::body::Bytes;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::Row;
use studio_protocol::{
    ConversationList, ConversationState, ConversationSummary, Dismissed, JournalSeq, OpenConversation,
    OpenedConversation, Problem, ProblemType, SitePath, TurnState, Ulid, WallClock,
};
use wps_client::wps::Wps;

pub use lifecycle::{ConversationError, bind_thread, dismiss, open_site, open_thread};
pub use panel_events::routes as panel_event_routes;
pub use rows::{live_of_site, load};

use crate::db::seq_column;
use crate::identity::Identity;
use crate::journal::{self, open_turn};
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};
use crate::sites::is_listed;
use crate::{AppState, GateState};

const LIST_DEFAULT: u32 = 20;
const LIST_MOST: u32 = 100;

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new()
        .route("/api/conversations", post(open).get(list))
        .route("/api/conversations/{conversation}", get(snapshot::snapshot))
        .route("/api/conversations/{conversation}/entries", get(snapshot::entries))
        .route("/api/conversations/{conversation}/stream", get(journal::stream))
        .route("/api/conversations/{conversation}/dismiss", post(dismiss_route))
}

/// A conversation's id from a route's path; one that is no ULID names nothing.
pub fn conversation_in(path: Result<Path<String>, PathRejection>, instance: &str) -> Result<Ulid, ProblemAnswer> {
    let nothing = || Problem::new(ProblemType::NotFound, "No conversation has that id.").at(instance).into();
    let Ok(Path(id)) = path else { return Err(nothing()) };
    id.parse().map_err(|_| nothing())
}

/// `POST /api/conversations`: the site's live conversation, or the person's new-site thread, `201` when this request
/// made it; `not-found` for a site world-proto-server does not list (brief I1.06b, lead decided).
async fn open<S: Wps>(
    State(state): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<OpenedConversation>), ProblemAnswer> {
    let OpenConversation { site } = json_body(&body, uri.path())?;
    if let Some(site) = &site
        && !is_listed(&state, site, uri.path()).await?
    {
        let detail = format!("world-proto-server lists no site {site}, so it has no conversation to open.");
        return Err(Problem::new(ProblemType::NotFound, detail).at(uri.path()).into());
    }
    let state = state.app;
    let mut conn = state.pool.acquire().await.map_err(|fault| internal("reach the database", fault, uri.path()))?;
    let opened = match &site {
        Some(site) => open_site(&mut conn, site, &identity.subject, &state.model_label).await,
        None => open_thread(&mut conn, &identity.subject, &state.model_label).await,
    };
    let (conversation, made) = opened.map_err(|fault| fault.answer(uri.path()))?;
    Ok((if made { StatusCode::CREATED } else { StatusCode::OK }, Json(OpenedConversation { conversation })))
}

#[derive(Deserialize)]
struct ListQuery {
    site: SitePath,
    before: Option<u64>,
    limit: Option<u32>,
}

/// `GET /api/conversations?site=&before=&limit=`: the site's live conversation, then its archived ones, newest first
/// by the first number each holds, which `before` pages by.
async fn list(
    State(state): State<AppState>,
    uri: Uri,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<ConversationList>, ProblemAnswer> {
    let instance = uri.path();
    let Query(ListQuery { site, before, limit }) =
        query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let limit = limit.unwrap_or(LIST_DEFAULT).clamp(1, LIST_MOST);
    let fault = |fault| internal("list the site's conversations", fault, instance);
    let mut conn = state.pool.acquire().await.map_err(fault)?;
    let sql = format!(
        "SELECT {}, \
             (SELECT at FROM journal j WHERE j.conversation = c.id ORDER BY j.seq LIMIT 1) AS first_at, \
             (SELECT at FROM journal j WHERE j.conversation = c.id ORDER BY j.seq DESC LIMIT 1) AS last_at \
         FROM conversations c WHERE c.site = $1 AND ($2::BIGINT IS NULL OR c.first_seq < $2) \
         ORDER BY c.first_seq DESC LIMIT $3",
        rows::COLUMNS
    );
    let before = before.map(|seq| seq_column(JournalSeq::new(seq)));
    let found = sqlx::query(&sql)
        .bind(site.as_str())
        .bind(before)
        .bind(i64::from(limit) + 1)
        .fetch_all(&mut *conn)
        .await
        .map_err(fault)?;
    let mut items = Vec::new();
    for row in found.iter().take(limit as usize) {
        let conversation = rows::from_row(row, &state.model_label).map_err(fault)?;
        let at = |column: &str| row.try_get::<Option<DateTime<Utc>>, _>(column).map(|at| at.map(WallClock::from));
        let (first_at, last_at) = (at("first_at").map_err(fault)?, at("last_at").map_err(fault)?);
        let waiting = match conversation.state {
            ConversationState::Live => open_turn(&mut conn, &conversation.id)
                .await
                .map_err(fault)?
                .filter(|turn| turn.state == TurnState::Waiting)
                .and_then(|turn| turn.waiting_on),
            _ => None,
        };
        items.push(ConversationSummary { conversation, waiting, first_at, last_at });
    }
    let next = (found.len() > limit as usize).then(|| items.last().map(|item| item.conversation.first_seq)).flatten();
    Ok(Json(ConversationList { items, next }))
}

/// `POST /api/conversations/{c}/dismiss` (spec 01 section 3.17).
async fn dismiss_route(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<Dismissed>, ProblemAnswer> {
    let conversation = conversation_in(path, uri.path())?;
    dismiss(&state, &conversation, &identity).await.map(Json).map_err(|fault| fault.answer(uri.path()))
}
