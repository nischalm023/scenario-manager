//! Within `journal`, `stream` is a conversation's server-sent events (spec 01 section 3.5's stream rules; brief I1.04
//! section 4). A connection subscribes to the hub first, then replays the journal after its cursor in batches of 500,
//! then the blocks still streaming as deltas from offset 0, then `stream.live`, then tails: each wake reads the
//! journal past the cursor up to its own number, so Postgres stays the record, the hub only a hint, and entries and
//! live items leave in the order the hub heard them, never a delta after its block's `text.done`. The cursor is
//! `Last-Event-ID`
//! when the browser sends one, else `?after=`, since a reconnecting `EventSource` reuses its first address with its
//! stale `after`. Journaled entries carry `id:`; nothing else does. A named `ping` every 15 s of wall clock is the
//! heartbeat, since `EventSource` hands a page no comment line. A cursor past the conversation's last entry is reset
//! as `unknown-cursor`; one below its first entry replays from that entry, numbers running on across a site's
//! conversations; a client more than 1,000 items behind is reset as `slow-client`.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::HeaderName;
use axum::http::{HeaderMap, HeaderValue, Uri};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::{
    BlockId, JournalEntry, JournalKind, JournalSeq, LiveKind, Ping, Problem, ProblemType, ResetReason, StreamKind,
    StreamLive, StreamReset, TextDelta, TextOffset, Ulid, WallClock,
};
use tokio::sync::mpsc::{Sender, channel};
use tokio::time::{Instant, interval_at};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use super::hub::{HubItem, HubRecvError, Subscription};
use super::read::entries_after;
use crate::AppState;
use crate::conversations::{conversation_in, load};
use crate::db::seq_of;
use crate::problem::{ProblemAnswer, internal, validation_failed};

/// The heartbeat's interval in wall clock (spec 01 section 3.5, "Heartbeat").
pub const PING_EVERY: Duration = Duration::from_secs(15);
/// How long a browser waits before it reconnects, sent as the stream's first line.
const RETRY: Duration = Duration::from_millis(2_000);
const REPLAY_BATCH: u32 = 500;
/// Events made but not yet taken by the connection; past them the stream waits on its client, and the hub's buffer
/// fills instead.
const OUTBOX: usize = 16;
const LAST_EVENT_ID: HeaderName = HeaderName::from_static("last-event-id");
const ACCEL_BUFFERING: HeaderName = HeaderName::from_static("x-accel-buffering");

#[derive(Deserialize)]
pub struct StreamQuery {
    after: Option<u64>,
}

/// `GET /api/conversations/{c}/stream`.
pub async fn stream(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<StreamQuery>, QueryRejection>,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let conversation = conversation_in(path, instance)?;
    let cursor = cursor(&headers, query, instance)?;
    let mut conn = state.pool.acquire().await.map_err(|fault| internal("reach the database", fault, instance))?;
    let found = load(&mut conn, &conversation, &state.model_label).await;
    if found.map_err(|fault| internal("read the conversation", fault, instance))?.is_none() {
        let detail = format!("There is no conversation {conversation}.");
        return Err(Problem::new(ProblemType::NotFound, detail).at(instance).into());
    }
    drop(conn);
    let subscription = state.hub.subscribe(&conversation);
    let (out, events) = channel(OUTBOX);
    let streamer = Streamer { pool: state.pool.clone(), conversation, cursor, subscription, out };
    tokio::spawn(streamer.run());
    let body = Sse::new(ReceiverStream::new(events).map(Ok::<Event, Infallible>));
    Ok(([(ACCEL_BUFFERING, HeaderValue::from_static("no"))], body).into_response())
}

/// `Last-Event-ID` when the browser sends one, else `?after=`; neither replays from the conversation's first entry.
fn cursor(
    headers: &HeaderMap,
    query: Result<Query<StreamQuery>, QueryRejection>,
    instance: &str,
) -> Result<Option<JournalSeq>, ProblemAnswer> {
    let last = headers.get(LAST_EVENT_ID).map(|value| value.to_str().map(str::trim));
    match last {
        Some(Ok("")) | None => {}
        Some(Ok(last)) => {
            let refused = |_| validation_failed(format!("Last-Event-ID `{last}` is no journal number"), instance);
            return last.parse().map(|seq| Some(JournalSeq::new(seq))).map_err(refused);
        }
        Some(Err(_)) => return Err(validation_failed("Last-Event-ID is not text", instance)),
    }
    let Query(StreamQuery { after }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    Ok(after.map(JournalSeq::new))
}

/// One connection's stream: what it sent up to (`cursor`), what it hears, and where its events go.
struct Streamer {
    pool: PgPool,
    conversation: Ulid,
    cursor: Option<JournalSeq>,
    subscription: Subscription,
    out: Sender<Event>,
}

/// Why a stream ended early: its client went away, or the database failed it, which the client's reconnect retries.
enum Ended {
    Gone,
    Fault(SqlxError),
}

impl From<SqlxError> for Ended {
    fn from(fault: SqlxError) -> Self {
        Self::Fault(fault)
    }
}

impl Streamer {
    async fn run(mut self) {
        if let Err(Ended::Fault(fault)) = self.serve().await {
            eprintln!("studio-api: the stream of conversation {} ended on a fault: {fault}", self.conversation);
        }
    }

    async fn serve(&mut self) -> Result<(), Ended> {
        self.send(Event::default().retry(RETRY)).await?;
        let numbers: Option<(i64, i64)> = sqlx::query_as("SELECT first_seq, last_seq FROM conversations WHERE id = $1")
            .bind(self.conversation.as_str())
            .fetch_optional(&self.pool)
            .await?;
        let Some((first, last)) = numbers else { return Ok(()) };
        let (first, last) = (seq_of(first)?, seq_of(last)?);
        if self.cursor.is_some_and(|cursor| cursor > last) {
            return self
                .send(control(StreamKind::StreamReset, &StreamReset { reason: ResetReason::UnknownCursor }))
                .await;
        }
        let mut cursor = self.cursor.unwrap_or(JournalSeq::new(0)).max(JournalSeq::new(first.get() - 1));
        let done = self.replay(&mut cursor, None).await?;
        let streaming: Vec<_> = self.subscription.streaming().to_vec();
        for block in streaming.into_iter().filter(|block| !done.contains(&block.block)) {
            let whole = TextDelta { block: block.block, offset: TextOffset::new(0), delta: block.text };
            self.send(control_live(LiveKind::TextDelta, &whole)).await?;
        }
        self.send(control(StreamKind::StreamLive, &StreamLive { last_seq: cursor })).await?;
        let mut pings = interval_at(Instant::now() + PING_EVERY, PING_EVERY);
        loop {
            tokio::select! {
                heard = self.subscription.recv() => match heard {
                    Ok(HubItem::Wake(seq)) if seq <= cursor => {}
                    Ok(HubItem::Wake(seq)) => drop(self.replay(&mut cursor, Some(seq)).await?),
                    Ok(HubItem::Delta(delta)) => self.send(control_live(LiveKind::TextDelta, &delta)).await?,
                    Ok(HubItem::Live(item)) => self.send(control_live(item.kind, &item.body)).await?,
                    Err(HubRecvError::Lagged) => {
                        let reset = StreamReset { reason: ResetReason::SlowClient };
                        return self.send(control(StreamKind::StreamReset, &reset)).await;
                    }
                    Err(HubRecvError::Closed) => return Ok(()),
                },
                _ = pings.tick() => self.send(control(StreamKind::Ping, &Ping { at: WallClock::now() })).await?,
            }
        }
    }

    /// Send every entry after `cursor`, up to `until` when given, a batch at a time, moving `cursor` on; the blocks
    /// whose `text.done` it sent.
    async fn replay(
        &mut self,
        cursor: &mut JournalSeq,
        until: Option<JournalSeq>,
    ) -> Result<FxHashSet<BlockId>, Ended> {
        let mut done = FxHashSet::default();
        loop {
            let mut conn = self.pool.acquire().await?;
            let batch = entries_after(&mut conn, &self.conversation, *cursor, until, REPLAY_BATCH).await?;
            drop(conn);
            for entry in &batch {
                if entry.kind == JournalKind::TextDone
                    && let Some(block) = entry.body.get("block").and_then(|block| block.as_str())
                {
                    done.insert(BlockId::new(block));
                }
                self.send(journaled(entry)).await?;
                *cursor = entry.seq;
            }
            if batch.len() < REPLAY_BATCH as usize {
                return Ok(done);
            }
        }
    }

    async fn send(&self, event: Event) -> Result<(), Ended> {
        self.out.send(event).await.map_err(|_| Ended::Gone)
    }
}

/// A journaled entry: `id: <seq>`, `event: <kind>`, and the envelope on one `data:` line.
fn journaled(entry: &JournalEntry) -> Event {
    data(Event::default().id(entry.seq.get().to_string()).event(entry.kind.as_str()), entry)
}

/// An item the stream makes itself, without an `id:` line.
fn control(kind: StreamKind, body: &impl Serialize) -> Event {
    data(Event::default().event(kind.as_str()), body)
}

/// A live-only item from the hub, without an `id:` line.
fn control_live(kind: LiveKind, body: &impl Serialize) -> Event {
    data(Event::default().event(kind.as_str()), body)
}

fn data(event: Event, body: &impl Serialize) -> Event {
    event.json_data(body).expect("a protocol body is plain JSON and serializes")
}
