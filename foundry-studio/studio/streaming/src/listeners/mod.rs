//! Within `studio-streaming`, `listeners` are the server's two ports (ruling D174): the internal one, where a runner
//! streams its run (`runners`), and the external one, where viewers watch its messages (`viewers`), its picture
//! (`frames`, live frames) and its metrics one at a time (`metrics`); who may open a socket on any of them
//! (`admission`); and the wall-clock bounds all keep every socket within, so a peer that goes silent, stops reading or
//! never greets costs the server nothing for long.

mod admission;
mod frames;
mod metrics;
mod runners;
mod viewers;

use std::time::Duration;

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket};
use axum::routing::get;
use futures::SinkExt;
use futures::stream::SplitSink;
use sqlx::PgPool;
use studio_protocol::StreamRefusal;
use studio_protocol::run_stream::{FIRST_MESSAGE_WITHIN, PING_EVERY, SILENT_AFTER};
use tokio::time::timeout;

use crate::attach::Attach;
use crate::metric_streams::Hubs;
use crate::provider::Provider;
use crate::streams::Streams;

/// What both listeners share: the API's database, for admission, the streams, the provider every viewer reads an
/// attempt's recording through, and the hubs computing its metrics.
#[derive(Clone)]
pub struct Listening {
    pub pool: PgPool,
    pub streams: Streams,
    pub provider: Provider,
    pub hubs: Hubs,
    pub bounds: SocketBounds,
}

impl Listening {
    /// What a viewer socket's route reads with.
    fn attach(&self) -> Attach {
        Attach {
            streams: self.streams.clone(),
            pool: self.pool.clone(),
            provider: self.provider.clone(),
            hubs: self.hubs.clone(),
            lag: self.bounds.write_within,
        }
    }
}

/// The wall-clock bounds a socket is kept within; the protocol's, which tests shorten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketBounds {
    /// How long the first message, a hello or a watch, may take.
    pub first_message: Duration,
    pub ping_every: Duration,
    /// How long a socket may stay silent, pongs included, before it is taken for gone.
    pub silent_after: Duration,
    /// How long one message may take to go out before its peer is taken for too slow.
    pub write_within: Duration,
}

impl Default for SocketBounds {
    fn default() -> Self {
        Self {
            first_message: FIRST_MESSAGE_WITHIN,
            ping_every: PING_EVERY,
            silent_after: SILENT_AFTER,
            write_within: Duration::from_secs(10),
        }
    }
}

/// The internal port's routes: `GET /runners`, a runner's socket.
pub fn internal(listening: Listening) -> Router {
    Router::new().route("/runners", get(runners::upgrade)).with_state(listening)
}

/// The external port's routes: `GET /streams/{run}/{attempt}`, a viewer's socket for the run's messages,
/// `GET /frames/{run}/{attempt}`, one for its picture, `GET /metrics/{run}/{attempt}`, one for one of its metrics,
/// and `GET /healthz`.
pub fn external(listening: Listening) -> Router {
    Router::new()
        .route("/streams/{run}/{attempt}", get(viewers::upgrade))
        .route("/frames/{run}/{attempt}", get(frames::upgrade))
        .route("/metrics/{run}/{attempt}", get(metrics::upgrade))
        .route("/healthz", get(healthz))
        .with_state(listening)
}

async fn healthz() -> &'static str {
    "ok"
}

type Sink = SplitSink<WebSocket, Message>;

/// One message on `sink` within the bound; `false` when it did not go, the peer gone or too slow.
async fn send(sink: &mut Sink, message: Message, within: Duration) -> bool {
    matches!(timeout(within, sink.send(message)).await, Ok(Ok(())))
}

/// One text message on `sink` within the bound.
async fn send_text(sink: &mut Sink, text: Utf8Bytes, within: Duration) -> bool {
    send(sink, Message::Text(text), within).await
}

/// The socket closed with `refusal` as its reason, best effort.
async fn refuse(sink: &mut Sink, refusal: StreamRefusal, within: Duration) {
    let frame = CloseFrame { code: 1008, reason: refusal.reason().into() };
    send(sink, Message::Close(Some(frame)), within).await;
}
