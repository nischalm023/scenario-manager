//! Within `studio-streaming`, `metric_streams` is the server's half of the metric stream protocol
//! (`studio/protocol/src/metric_stream.rs`) on `/metrics/{run}/{attempt}`, once the listener has admitted the attach
//! and named the metric: the socket joins the hub that follows the attempt's recording for every metric socket of the
//! attempt (`hub`), opens its ask's view there once the scenario block is in, and says what the view makes known:
//! `attached` with the descriptor, every row so far in batches, each later batch as it becomes final, then `ended`. A
//! failure, of the hub or of the view alone, closes the socket with its refusal as the close reason, and a socket that
//! stops taking what comes is told it lagged.

mod hub;
#[cfg(test)]
mod tests;

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use studio_metrics::{Ask, Unservable};
use studio_protocol::{METRIC_STREAM_FORMAT, MetricMessage, MetricPoint, StreamRefusal};
use tokio::select;
use tokio::time::timeout;

use crate::attach::Attach;
use crate::cache::opened;
use crate::provider::{Fault, Unreached};
use crate::streams::StreamKey;

use hub::member::{Lagged, Member, Told};
use hub::registry::Joined;
use hub::{HubEnd, HubFault};

pub use hub::registry::Hubs;

/// Rows per `rows` message while the rows so far go out.
const BATCH: usize = 256;

type Sink = SplitSink<WebSocket, Message>;

/// Why a socket stops before `ended`.
enum Stopped {
    /// The viewer went away.
    Gone,
    /// The viewer stopped taking what comes.
    Lagged,
    /// The hub or the view failed, so the socket closes with this refusal.
    Refused(StreamRefusal),
}

/// The attempt's metric, as `ask` cuts it, to an admitted client, by the module doc's rule.
pub async fn serve_metric(sink: Sink, mut source: SplitStream<WebSocket>, key: StreamKey, ask: Ask, attach: Attach) {
    let known = opened(&attach.pool, &key).await.map_err(|fault| fault.to_string());
    let Joined { over, mut member } = attach.hubs.join(&key, ask, known);
    let mut socket = Socket { sink, lag: attach.lag };
    let followed = async {
        let viewed = loop {
            match member.view().await.map_err(|fault| Stopped::Refused(refusal(&fault)))? {
                Some(viewed) => break viewed.map_err(|why| Stopped::Refused(view_refusal(&why)))?,
                None => {
                    if let Told::Ended(end) = told(&mut member, &mut source).await? {
                        return Ok(end);
                    }
                }
            }
        };
        let attached = MetricMessage::Attached {
            v: METRIC_STREAM_FORMAT,
            run: key.run.clone(),
            attempt: key.attempt,
            descriptor: viewed.descriptor,
            reached_sim_ms: viewed.reached,
            live: viewed.live,
        };
        socket.say(&attached).await?;
        socket.rows(&viewed.rows).await?;
        if let Some(outcome) = over {
            return Ok(HubEnd::Over(outcome));
        }
        loop {
            match told(&mut member, &mut source).await? {
                Told::Rows(rows) => socket.rows(&rows).await?,
                Told::Failed(why) => return Err(Stopped::Refused(view_refusal(&why))),
                Told::Ended(end) => return Ok(end),
                Told::Described => {}
            }
        }
    };
    let stopped = match followed.await {
        Ok(HubEnd::Over(outcome)) => match socket.say(&MetricMessage::Ended { v: METRIC_STREAM_FORMAT, outcome }).await
        {
            Ok(()) => return socket.close(None).await,
            Err(stopped) => stopped,
        },
        Ok(HubEnd::Failed(fault)) => Stopped::Refused(refusal(&fault)),
        Err(stopped) => stopped,
    };
    match stopped {
        Stopped::Gone => {}
        Stopped::Lagged => {
            let _ = socket.say(&MetricMessage::Lagged { v: METRIC_STREAM_FORMAT }).await;
            socket.close(None).await;
        }
        Stopped::Refused(refusal) => {
            socket.close(Some(CloseFrame { code: 1008, reason: refusal.reason().into() })).await;
        }
    }
}

/// What the hub tells the member next, the viewer's pings passed over: a viewer that closes or says anything else has
/// gone, and one that fell behind the hub's broadcast has lagged.
async fn told(member: &mut Member, source: &mut SplitStream<WebSocket>) -> Result<Told, Stopped> {
    loop {
        select! {
            told = member.next() => return told.map_err(|Lagged| Stopped::Lagged),
            from_client = source.next() => match from_client {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return Err(Stopped::Gone),
            },
        }
    }
}

/// The refusal a hub's failure closes every socket of the attempt with: worth another try when the picture is not there
/// yet or something the server needs did not answer, final when no copy will ever be, or the recording does not read
/// or panics the follower, which another try would repeat.
fn refusal(fault: &HubFault) -> StreamRefusal {
    match fault {
        HubFault::Unreached(Unreached::NotYet) => StreamRefusal::NotHeld,
        HubFault::Unreached(Unreached::Nowhere) => StreamRefusal::NoRecording,
        HubFault::Unreached(Unreached::Database(_) | Unreached::Disk(_)) => StreamRefusal::Unavailable,
        HubFault::Unreached(Unreached::Unopened(fault)) | HubFault::Read(fault) => copy_refusal(fault),
        HubFault::Recording(_) | HubFault::Panicked(_) => StreamRefusal::RecordingUnreadable,
    }
}

/// The refusal for an attempt's copy that failed as it was opened or read.
fn copy_refusal(fault: &Fault) -> StreamRefusal {
    match fault {
        Fault::Fetch(_) | Fault::Disk(_) | Fault::Gone => StreamRefusal::Unavailable,
        Fault::Unreadable(_) => StreamRefusal::RecordingUnreadable,
    }
}

/// The refusal a view that cannot be served closes its sockets with.
fn view_refusal(why: &Unservable) -> StreamRefusal {
    match why {
        Unservable::UnsupportedTrailing(_) => StreamRefusal::UnsupportedTrailing,
        Unservable::NotRecorded(_) => StreamRefusal::NotRecorded,
        Unservable::Unreadable(_) => StreamRefusal::RecordingUnreadable,
    }
}

/// One metric socket's sending half and how long one message may take to go.
struct Socket {
    sink: Sink,
    lag: Duration,
}

impl Socket {
    async fn rows(&mut self, rows: &[MetricPoint]) -> Result<(), Stopped> {
        for batch in rows.chunks(BATCH) {
            self.say(&MetricMessage::Rows { v: METRIC_STREAM_FORMAT, rows: batch.to_vec() }).await?;
        }
        Ok(())
    }

    async fn say(&mut self, message: &MetricMessage) -> Result<(), Stopped> {
        match timeout(self.lag, self.sink.send(Message::Text(message.text().into()))).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(Stopped::Gone),
            Err(_) => Err(Stopped::Lagged),
        }
    }

    async fn close(&mut self, frame: Option<CloseFrame>) {
        let _ = timeout(self.lag, self.sink.send(Message::Close(frame))).await;
    }
}
