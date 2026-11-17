//! Within `listeners`, `frames` is a viewer's socket for an attempt's picture on the external port
//! (`/frames/{run}/{attempt}`; live frames, 26 October 2026), in the viewer's own attach protocol (`attach`): an
//! `attach` first, within the bound, carrying the same watch ticket the API minted for the attempt, which `admission`
//! checks as it checks a `watch`; then the picture, read through the provider, as `attach` serves it. A refusal is the
//! close frame's reason, as on the other sockets.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use studio_protocol::{AttemptNumber, StreamRefusal};
use tokio::time::timeout;

use super::{Listening, admission, refuse};
use crate::attach::{Request, serve_attached};
use crate::streams::StreamKey;

/// The largest message a viewer may send: its attach, a ticket and a few words.
const MESSAGE_LIMIT: usize = 4 << 10;

/// `GET /frames/{run}/{attempt}`: the upgrade, or 404 for a path that names no attempt.
pub async fn upgrade(
    upgrade: WebSocketUpgrade,
    State(listening): State<Listening>,
    Path((run, attempt)): Path<(String, u32)>,
) -> Response {
    let Ok(run) = run.parse() else { return (StatusCode::NOT_FOUND, "no such run").into_response() };
    let key = StreamKey { run, attempt: AttemptNumber(attempt) };
    upgrade.max_message_size(MESSAGE_LIMIT).on_upgrade(move |socket| serve(socket, listening, key)).into_response()
}

async fn serve(socket: WebSocket, listening: Listening, key: StreamKey) {
    let within = listening.bounds.write_within;
    let (mut sink, mut source) = socket.split();
    let request = match timeout(listening.bounds.first_message, source.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<Request>(&text) {
            Ok(request) => request,
            Err(_) => return refuse(&mut sink, StreamRefusal::Unreadable, within).await,
        },
        Ok(Some(Ok(Message::Binary(_)))) => return refuse(&mut sink, StreamRefusal::Binary, within).await,
        _ => return refuse(&mut sink, StreamRefusal::NoGreeting, within).await,
    };
    let Request::Attach { id, from, token, .. } = request;
    let Some(token) = token else { return refuse(&mut sink, StreamRefusal::TokenInvalid, within).await };
    if let Err(refusal) = admission::viewer(&listening.pool, &key, &token).await {
        return refuse(&mut sink, refusal, within).await;
    }
    serve_attached(sink, source, key, id, from, listening.attach()).await;
}
