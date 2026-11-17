//! Within `studio-api`, `dev` is the test writer (brief I1.04 section 5, lead decided): with `STUDIO_ENV=local`,
//! `POST /api/dev/conversations/{c}/entries` hands a developer's entry to the journal's writer and the hub, the path
//! the agent loop takes, or a live item to the hub alone, so the browser tests replay the journal fixtures before
//! task 3's loop exists. A body the projection could not read is refused, so nothing written leaves the fold
//! guessing, and each entry makes the change to the turn's row the loop makes with it (`agent::follow`), so the table
//! and the journal agree; an entry whose dedup key the conversation holds is answered under the conversation's lock
//! before any row changes, as the journal answers it. A cluster never mounts it.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::Value;
use studio_protocol::{ConversationState, DevEntry, DevLive, DevWritten};
use wps_client::wps::Wps;

use crate::agent::{FollowError, follow};
use crate::conversations::conversation_in;
use crate::journal::{Appended, JournalError, NewEntry, append, held, readable};
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};
use crate::{AppState, GateState};

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new().route("/api/dev/conversations/{conversation}/entries", post(write))
}

async fn write(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let conversation = conversation_in(path, instance)?;
    let written: Value = json_body(&body, instance)?;
    let refused = |why: String| validation_failed(why, instance);
    let fault = |fault| internal("write the conversation's journal", fault, instance);
    if written.get("live").is_some() {
        let DevLive { live } = serde_json::from_value(written).map_err(|why| refused(why.to_string()))?;
        let state_now: Option<String> = sqlx::query_scalar("SELECT state FROM conversations WHERE id = $1")
            .bind(conversation.as_str())
            .fetch_optional(&state.pool)
            .await
            .map_err(fault)?;
        match state_now.as_deref().map(str::parse::<ConversationState>) {
            None => return Err(JournalError::NotFound(conversation).answer(instance)),
            Some(Ok(ConversationState::Live)) => {}
            Some(Ok(closed)) => return Err(JournalError::Closed { conversation, state: closed }.answer(instance)),
            Some(Err(why)) => return Err(internal("read the conversation's state", why, instance)),
        }
        state.hub.live(&conversation, live).map_err(|why| refused(why.to_string()))?;
        return Ok(StatusCode::ACCEPTED.into_response());
    }
    let DevEntry { kind, body, turn, caused_by, dedup_key } =
        serde_json::from_value(written).map_err(|why| refused(why.to_string()))?;
    readable(kind, &body, turn.as_ref()).map_err(|why| refused(format!("this {} body {why}", kind.as_str())))?;
    let mut tx = state.pool.begin().await.map_err(fault)?;
    if let Some(held) = held(&mut tx, &conversation, dedup_key.as_deref()).await.map_err(|why| why.answer(instance))? {
        return Ok((StatusCode::OK, Json(DevWritten::Held { held })).into_response());
    }
    let entry = NewEntry { kind, body, turn, caused_by, dedup_key };
    follow(&mut tx, &conversation, &entry).await.map_err(|why| match why {
        FollowError::Database(fault) => internal("follow the entry's turn", fault, instance),
        refusal => refused(format!("this {} entry {refusal}", kind.as_str())),
    })?;
    let appended = append(&mut tx, &conversation, entry).await.map_err(|fault| fault.answer(instance))?;
    tx.commit().await.map_err(fault)?;
    Ok(match appended {
        Appended::Wrote(written) => {
            let entry = Box::new(written.entry().clone());
            state.hub.announce([written]);
            (StatusCode::CREATED, Json(DevWritten::Wrote { entry })).into_response()
        }
        Appended::Held(held) => (StatusCode::OK, Json(DevWritten::Held { held })).into_response(),
    })
}
