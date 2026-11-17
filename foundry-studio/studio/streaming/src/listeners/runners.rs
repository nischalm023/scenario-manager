//! Within `listeners`, `runners` is a runner's socket on the internal port (`/runners`): a `hello` first, within its
//! bound, naming the attempt and presenting its token, which `admission` checks; then the socket owns the attempt's
//! stream, superseding any socket before it, and every message after it goes into the stream until the runner's ending
//! closes it. After `started`, every binary message is one frame of the picture, into the attempt's frames (live
//! frames); a segment the runner is sending counts as whole when the socket has sent no frame for `SEGMENT_QUIET`,
//! since the server cannot see a batch's end. The server pings the socket and takes one silent past its bound for
//! gone. A socket that goes without the ending leaves its runner the grace to come back (`streams`); one a newer
//! socket superseded is closed so, and the stream stays the newer one's.

use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::StreamExt;
use futures::stream::SplitStream;
use studio_protocol::{RunnerMessage, RunnerReply, STREAM_FORMAT, StreamRefusal};
use tokio::select;
use tokio::sync::watch::Receiver as OwnerReceiver;
use tokio::time::{Instant, interval_at, sleep_until, timeout};
use worldview::container::frame::{HEADER_LEN, MAX_PAYLOAD_LEN};

use super::{Listening, Sink, admission, refuse, send, send_text};
use crate::frames::AttemptFrames;
use crate::streams::{Stream, StreamKey};

/// The largest message a runner may send: one container frame, whose payload the format caps at 64 MiB; the runner's
/// longest text message, its ending, is far below it.
const MESSAGE_LIMIT: usize = HEADER_LEN + MAX_PAYLOAD_LEN as usize;
/// Wall clock without a binary frame after which the segment being sent is taken as whole; a runner sends a segment
/// as one batch, so a pause this long is the gap between segments.
pub const SEGMENT_QUIET: Duration = Duration::from_secs(1);

/// `GET /runners`: the upgrade.
pub async fn upgrade(upgrade: WebSocketUpgrade, State(listening): State<Listening>) -> impl IntoResponse {
    upgrade
        .max_message_size(MESSAGE_LIMIT)
        .max_frame_size(MESSAGE_LIMIT)
        .on_upgrade(move |socket| serve(socket, listening))
}

/// How a runner's socket ended.
enum Left {
    Ended,
    Superseded,
    Refused(StreamRefusal),
    /// It closed, failed or would not take a ping.
    Gone,
}

async fn serve(socket: WebSocket, listening: Listening) {
    let within = listening.bounds.write_within;
    let (mut sink, mut source) = socket.split();
    let (key, token) = match hello(&mut source, &listening).await {
        Ok(hello) => hello,
        Err(refusal) => return refuse(&mut sink, refusal, within).await,
    };
    let StreamKey { run, attempt } = &key;
    let named = format!("{run}/{}", attempt.0);
    if let Err(refusal) = admission::runner(&listening.pool, &key, &token).await {
        eprintln!("studio-streaming: {named} refused: {}", refusal.reason());
        return refuse(&mut sink, refusal, within).await;
    }
    let (stream, socket) = match listening.streams.take_over(key) {
        Ok(taken) => taken,
        Err(refusal) => return refuse(&mut sink, refusal, within).await,
    };
    let welcome = RunnerReply::Welcome { v: STREAM_FORMAT }.text().into();
    if !send_text(&mut sink, welcome, within).await {
        return listening.streams.runner_left(stream, socket);
    }
    eprintln!("studio-streaming: {named} streams on its socket {socket}");
    match follow(&mut sink, &mut source, &listening, &stream, socket).await {
        Left::Ended => {
            eprintln!("studio-streaming: {named} ended");
            send(&mut sink, Message::Close(None), within).await;
        }
        Left::Superseded => refuse(&mut sink, StreamRefusal::Superseded, within).await,
        Left::Refused(refusal) => {
            eprintln!("studio-streaming: {named}'s socket {socket} closed: {}", refusal.reason());
            // The grace starts from the refusal, not from when its close frame went out.
            listening.streams.runner_left(stream, socket);
            refuse(&mut sink, refusal, within).await;
        }
        Left::Gone => {
            eprintln!("studio-streaming: {named}'s socket {socket} went away before the ending");
            listening.streams.runner_left(stream, socket);
        }
    }
}

/// The runner's attempt and token, from a hello within the bound.
async fn hello(
    source: &mut SplitStream<WebSocket>,
    listening: &Listening,
) -> Result<(StreamKey, String), StreamRefusal> {
    match timeout(listening.bounds.first_message, source.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match RunnerMessage::read(&text)? {
            RunnerMessage::Hello { run, attempt, token, .. } => Ok((StreamKey { run, attempt }, token)),
            _ => Err(StreamRefusal::NoGreeting),
        },
        Ok(Some(Ok(Message::Binary(_)))) => Err(StreamRefusal::Binary),
        _ => Err(StreamRefusal::NoGreeting),
    }
}

/// Every message of socket `socket` into `stream` until the ending, a refusal, the socket's going, or a newer socket.
async fn follow(
    sink: &mut Sink,
    source: &mut SplitStream<WebSocket>,
    listening: &Listening,
    stream: &Stream,
    socket: u64,
) -> Left {
    let bounds = listening.bounds;
    let mut owner: OwnerReceiver<u64> = stream.owner();
    if *owner.borrow_and_update() != socket {
        return Left::Superseded;
    }
    let mut pings = interval_at(Instant::now() + bounds.ping_every, bounds.ping_every);
    let mut heard = Instant::now();
    // The attempt's picture, from `started` on (or from an earlier socket's), and when the segment being sent counts
    // as whole, absent while none is being sent.
    let mut frames: Option<Arc<AttemptFrames>> = listening.streams.frames().get(&stream.key);
    let mut quiet_at: Option<Instant> = None;
    loop {
        let quiet = async {
            match quiet_at {
                Some(at) => sleep_until(at).await,
                None => pending().await,
            }
        };
        select! {
            changed = owner.changed() => {
                if changed.is_err() || *owner.borrow_and_update() != socket {
                    return Left::Superseded;
                }
            }
            _ = pings.tick() => {
                if heard.elapsed() >= bounds.silent_after {
                    return Left::Refused(StreamRefusal::Silent);
                }
                if !send(sink, Message::Ping(Bytes::new()), bounds.write_within).await {
                    return Left::Gone;
                }
            }
            () = quiet => {
                quiet_at = None;
                if let Some(frames) = &frames {
                    listening.streams.settle(stream, frames);
                }
            }
            received = source.next() => {
                heard = Instant::now();
                match received {
                    Some(Ok(Message::Text(text))) => {
                        let message = match RunnerMessage::read(&text) {
                            Ok(message) => message,
                            Err(refusal) => return Left::Refused(refusal),
                        };
                        if let RunnerMessage::Started { .. } = &message {
                            frames = Some(listening.streams.frames().open(&stream.key));
                        }
                        if let RunnerMessage::Ended(_) = &message {
                            quiet_at = None;
                        }
                        match listening.streams.take(stream, socket, message) {
                            Ok(true) => return Left::Ended,
                            Ok(false) => {}
                            Err(StreamRefusal::Superseded) => return Left::Superseded,
                            Err(refusal) => return Left::Refused(refusal),
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let Some(open) = &frames else { return Left::Refused(StreamRefusal::Binary) };
                        match listening.streams.frame(stream, socket, open, bytes.to_vec()).await {
                            Ok(()) => quiet_at = Some(Instant::now() + SEGMENT_QUIET),
                            Err(StreamRefusal::Superseded) => return Left::Superseded,
                            Err(refusal) => return Left::Refused(refusal),
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Left::Gone,
                }
            }
        }
    }
}
