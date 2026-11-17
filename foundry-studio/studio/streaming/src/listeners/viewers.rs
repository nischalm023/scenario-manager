//! Within `listeners`, `viewers` is a viewer's socket on the external port (`/streams/{run}/{attempt}`): a `watch`
//! first, within its bound, with a ticket `admission` checks for that very attempt; then `attached`, the stream's
//! replay, and its messages live as they come, each sent as it was encoded once for every viewer. A viewer that does
//! not take a message within the write bound or falls further behind the live broadcast than its backlog is told
//! `lagged` and dropped, and one silent past its bound, pongs included, is dropped too: nothing is buffered for it
//! beyond that. Its following ends when the stream ends or is forgotten.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use futures::stream::SplitStream;
use studio_protocol::{AttemptNumber, STREAM_FORMAT, StreamRefusal, ViewerMessage, ViewerRequest};
use tokio::select;
use tokio::sync::broadcast::Receiver as LiveReceiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Instant, interval_at, timeout};

use super::{Listening, Sink, admission, refuse, send, send_text};
use crate::streams::{Attachment, StreamKey};

/// How long the `lagged` said to a viewer being dropped may take (wall clock).
const LAGGED_WITHIN: Duration = Duration::from_secs(1);
/// The largest message a viewer may send: its watch, a ticket and a few words.
const MESSAGE_LIMIT: usize = 4 << 10;

/// `GET /streams/{run}/{attempt}`: the upgrade, or 404 for a path that names no attempt.
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
    let token = match watch(&mut source, &listening).await {
        Ok(token) => token,
        Err(refusal) => return refuse(&mut sink, refusal, within).await,
    };
    if let Err(refusal) = admission::viewer(&listening.pool, &key, &token).await {
        return refuse(&mut sink, refusal, within).await;
    }
    let Some(stream) = listening.streams.get(&key) else {
        return refuse(&mut sink, StreamRefusal::NotHeld, within).await;
    };
    let Attachment { first, live } = stream.attach();
    drop(stream);
    for message in first {
        if !send_text(&mut sink, message, within).await {
            return lagged(&mut sink).await;
        }
    }
    match live {
        Some(live) => follow(&mut sink, &mut source, &listening, live).await,
        // The stream had ended: its replay was all of it.
        None => {
            send(&mut sink, Message::Close(None), within).await;
        }
    }
}

/// The ticket, from a watch within the bound.
async fn watch(source: &mut SplitStream<WebSocket>, listening: &Listening) -> Result<String, StreamRefusal> {
    match timeout(listening.bounds.first_message, source.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match ViewerRequest::read(&text)? {
            ViewerRequest::Watch { token, .. } => Ok(token),
        },
        Ok(Some(Ok(Message::Binary(_)))) => Err(StreamRefusal::Binary),
        _ => Err(StreamRefusal::NoGreeting),
    }
}

/// The live messages until the stream ends or is forgotten, the viewer falls behind or goes, or says what v1 viewers
/// do not.
async fn follow(
    sink: &mut Sink,
    source: &mut SplitStream<WebSocket>,
    listening: &Listening,
    mut live: LiveReceiver<Utf8Bytes>,
) {
    let bounds = listening.bounds;
    let mut pings = interval_at(Instant::now() + bounds.ping_every, bounds.ping_every);
    let mut heard = Instant::now();
    loop {
        select! {
            received = live.recv() => match received {
                Ok(message) => {
                    if !send_text(sink, message, bounds.write_within).await {
                        return lagged(sink).await;
                    }
                }
                Err(RecvError::Lagged(_)) => return lagged(sink).await,
                Err(RecvError::Closed) => {
                    send(sink, Message::Close(None), bounds.write_within).await;
                    return;
                }
            },
            _ = pings.tick() => {
                if heard.elapsed() >= bounds.silent_after {
                    return refuse(sink, StreamRefusal::Silent, bounds.write_within).await;
                }
                if !send(sink, Message::Ping(Bytes::new()), bounds.write_within).await {
                    return;
                }
            }
            from_viewer = source.next() => {
                heard = Instant::now();
                match from_viewer {
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_) | Message::Binary(_))) => {
                        return refuse(sink, StreamRefusal::Unreadable, bounds.write_within).await;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                }
            }
        }
    }
}

/// The viewer fell behind: told so, best effort, and dropped.
async fn lagged(sink: &mut Sink) {
    let lagged = ViewerMessage::Lagged { v: STREAM_FORMAT }.text().into();
    send_text(sink, lagged, LAGGED_WITHIN).await;
}
