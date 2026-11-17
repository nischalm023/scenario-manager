//! Within `conversations`, `panel_events` is spec 01 section 3.6's `POST /api/conversations/{c}/panel-events`: a
//! panel's own act told to the agent as a `panel.event` entry in the conversation's journal, which starts no turn
//! and which the next turn reads with its message (spec 01 section 3.5). Its first poster is the Scenario panel
//! (spec 07 sections 4.2 and 4.3; brief I1.08, lead decided); each kind belongs to the spec whose panel posts it.

use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::routing::post;
use axum::{Json, Router};
use studio_protocol::{JournalKind, PanelEvent, PanelEventWritten};
use wps_client::wps::Wps;

use crate::idempotency::idempotent;
use crate::journal::{Appended, NewEntry, append};
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};
use crate::{AppState, GateState};

use super::conversation_in;

/// The longest sentence a panel event carries for the agent to read.
const TEXT_MOST: usize = 2_000;

pub fn routes<S: Wps>(app: &AppState) -> Router<GateState<S>> {
    Router::new().route("/api/conversations/{conversation}/panel-events", idempotent(app, post(post_event)))
}

async fn post_event(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<(StatusCode, Json<PanelEventWritten>), ProblemAnswer> {
    let instance = uri.path();
    let conversation = conversation_in(path, instance)?;
    let event: PanelEvent = json_body(&body, instance)?;
    if event.text.trim().is_empty() || event.text.chars().count() > TEXT_MOST {
        return Err(validation_failed(
            format!("`text` is 1 to {TEXT_MOST} characters for the agent to read"),
            instance,
        ));
    }
    if !event.body.is_object() {
        return Err(validation_failed("`body` is an object of the event kind's own fields", instance));
    }
    let fault = |fault| internal("write the panel event", fault, instance);
    let entry = NewEntry::new(JournalKind::PanelEvent, serde_json::to_value(&event).expect("a panel event is JSON"));
    let mut tx = state.pool.begin().await.map_err(fault)?;
    let appended = append(&mut tx, &conversation, entry).await.map_err(|fault| fault.answer(instance))?;
    tx.commit().await.map_err(fault)?;
    let Appended::Wrote(written) = appended else {
        return Err(internal("write the panel event", "an entry with no dedup key was held", instance));
    };
    let seq = written.entry().seq;
    state.hub.announce([written]);
    Ok((StatusCode::CREATED, Json(PanelEventWritten { seq })))
}
