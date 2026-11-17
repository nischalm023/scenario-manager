//! Within `journal`, `hub` is the in-process relay between a conversation's writers and its open streams (spec 01
//! section 3.5): a wake naming each committed entry's number, which a stream answers by reading the journal past its
//! cursor (Postgres is the record, the hub only a hint); the live-only items, which exist nowhere else; and each
//! streaming block's text so far, so a stream that joins mid-block starts it with one delta from offset 0. Deltas
//! reach subscribers coalesced, one event per block per 50 ms of wall clock. A subscriber's buffer holds exactly
//! 1,000 items (a bounded channel per subscriber, since tokio's broadcast rounds its capacity up to a power of two),
//! and one that falls further behind lags, which its stream ends as `slow-client`. One process serves the PoC, so no
//! `LISTEN`/`NOTIFY` is needed; a second replica would need it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rustc_hash::FxHashMap;
use serde_json::Value;
use studio_protocol::{
    BlockId, JournalKind, JournalSeq, LiveItem, LiveKind, StreamingBlock, TextDelta, TextOffset, Ulid,
};
use thiserror::Error;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::time::{Instant, sleep_until};

use super::writer::Written;

/// How many items a subscriber may have waiting before it lags (spec 01 section 3.5, "Delivery").
pub const SUBSCRIBER_BUFFER: usize = 1_000;
/// The wall-clock interval at which one block's deltas reach subscribers (spec 01 section 3.5, "Text deltas").
const COALESCE: Duration = Duration::from_millis(50);

/// What a subscriber hears: an entry committed under this number, or a live-only item.
#[derive(Debug, Clone, PartialEq)]
pub enum HubItem {
    Wake(JournalSeq),
    Delta(TextDelta),
    Live(LiveItem),
}

/// Why a subscriber hears nothing more: it fell more than `SUBSCRIBER_BUFFER` items behind, or the hub is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HubRecvError {
    #[error("more than {SUBSCRIBER_BUFFER} items went unread")]
    Lagged,
    #[error("the hub has shut down")]
    Closed,
}

/// A delta the hub cannot place, or a live item whose body is not its kind's.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveRefused {
    #[error("the block {block} holds {have} UTF-16 code units, so a delta from {offset} would leave a gap")]
    Gap { block: BlockId, have: u64, offset: u64 },
    #[error("{offset} falls inside a character of the block {block}")]
    Split { block: BlockId, offset: u64 },
    #[error("a text.delta's body is {{ block, offset, delta }}: {0}")]
    Body(String),
}

/// The relay, one per process, shared by every route and the agent loop.
#[derive(Debug, Default)]
pub struct Hub {
    channels: Mutex<FxHashMap<Ulid, Channel>>,
    next_subscriber: AtomicU64,
}

#[derive(Debug, Default)]
struct Channel {
    subscribers: Vec<Subscriber>,
    blocks: Vec<Block>,
}

/// A subscriber's end of its bounded channel, and the flag its overflow raises before the hub lets it go.
#[derive(Debug)]
struct Subscriber {
    id: u64,
    sender: Sender<HubItem>,
    lagged: Arc<AtomicBool>,
}

/// A streaming block: its text so far, how far subscribers have it (`sent`), both in UTF-16 code units, and the
/// coalescing state.
#[derive(Debug)]
struct Block {
    id: BlockId,
    text: String,
    units: u64,
    sent: u64,
    flushed_at: Option<Instant>,
    flush_due: bool,
}

/// One stream's hold on a conversation's hub: what it hears from its joining on, and the blocks streaming then.
#[derive(Debug)]
pub struct Subscription {
    hub: Arc<Hub>,
    conversation: Ulid,
    id: u64,
    receiver: Receiver<HubItem>,
    lagged: Arc<AtomicBool>,
    streaming: Vec<StreamingBlock>,
}

impl Hub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Hear `conversation` from this call on. The blocks streaming at this call are taken under the same lock, so the
    /// subscriber's first delta of each continues the text it was handed.
    pub fn subscribe(self: &Arc<Self>, conversation: &Ulid) -> Subscription {
        let (sender, receiver) = channel(SUBSCRIBER_BUFFER);
        let (id, lagged) = (self.next_subscriber.fetch_add(1, Ordering::Relaxed), Arc::new(AtomicBool::new(false)));
        let mut channels = self.lock();
        let channel = channels.entry(conversation.clone()).or_default();
        channel.subscribers.push(Subscriber { id, sender, lagged: Arc::clone(&lagged) });
        let streaming = channel.streaming();
        Subscription { hub: Arc::clone(self), conversation: conversation.clone(), id, receiver, lagged, streaming }
    }

    /// The blocks of `conversation` streaming at this call, with their text so far, as a snapshot's active turn shows
    /// them.
    pub fn streaming(&self, conversation: &Ulid) -> Vec<StreamingBlock> {
        self.lock().get(conversation).map(Channel::streaming).unwrap_or_default()
    }

    /// Wake the streams of each entry's conversation; call it once the transaction that wrote them has committed.
    /// A `text.done` ends its block's streaming, and a `turn.finished` every block's.
    pub fn announce(&self, written: impl IntoIterator<Item = Written>) {
        let mut channels = self.lock();
        for entry in written.into_iter().map(Written::into_entry) {
            let Some(channel) = channels.get_mut(&entry.conversation) else { continue };
            match entry.kind {
                JournalKind::TextDone => {
                    let done = entry.body.get("block").and_then(Value::as_str);
                    channel.blocks.retain(|block| Some(block.id.as_str()) != done);
                }
                JournalKind::TurnFinished => channel.blocks.clear(),
                _ => {}
            }
            channel.send(&HubItem::Wake(entry.seq));
            if channel.idle() {
                channels.remove(&entry.conversation);
            }
        }
    }

    /// Place `delta` in its block's text, and send what subscribers lack at once, or, when they were last sent some
    /// less than 50 ms of wall clock ago, 50 ms after that sending.
    pub fn text_delta(self: &Arc<Self>, conversation: &Ulid, delta: TextDelta) -> Result<(), LiveRefused> {
        let mut channels = self.lock();
        let channel = channels.entry(conversation.clone()).or_default();
        let blocks = &mut channel.blocks;
        let block = match blocks.iter().position(|block| block.id == delta.block) {
            Some(n) => &mut blocks[n],
            None => {
                blocks.push(Block::new(delta.block.clone()));
                blocks.last_mut().expect("just pushed")
            }
        };
        block.place(delta.offset, &delta.delta)?;
        let now = Instant::now();
        match block.flushed_at {
            _ if block.flush_due => {}
            Some(at) if now < at + COALESCE => {
                block.flush_due = true;
                let (hub, conversation, id) = (Arc::clone(self), conversation.clone(), delta.block);
                tokio::spawn(async move {
                    sleep_until(at + COALESCE).await;
                    hub.flush(&conversation, &id);
                });
            }
            _ => {
                if let Some(unsent) = block.unsent(now) {
                    channel.send(&HubItem::Delta(unsent));
                }
            }
        }
        Ok(())
    }

    /// Hand subscribers a live-only item; a `text.delta` is placed in its block first. With nobody subscribed, an
    /// item other than a delta goes nowhere, as live-only items do.
    pub fn live(self: &Arc<Self>, conversation: &Ulid, item: LiveItem) -> Result<(), LiveRefused> {
        if item.kind == LiveKind::TextDelta {
            let delta = serde_json::from_value(item.body).map_err(|fault| LiveRefused::Body(fault.to_string()))?;
            return self.text_delta(conversation, delta);
        }
        if let Some(channel) = self.lock().get_mut(conversation) {
            channel.send(&HubItem::Live(item));
        }
        Ok(())
    }

    fn flush(&self, conversation: &Ulid, id: &BlockId) {
        let mut channels = self.lock();
        let Some(channel) = channels.get_mut(conversation) else { return };
        let block = channel.blocks.iter_mut().find(|block| &block.id == id);
        if let Some(unsent) = block.and_then(|block| block.unsent(Instant::now())) {
            channel.send(&HubItem::Delta(unsent));
        }
    }

    fn unsubscribe(&self, conversation: &Ulid, id: u64) {
        let mut channels = self.lock();
        let Some(channel) = channels.get_mut(conversation) else { return };
        channel.subscribers.retain(|subscriber| subscriber.id != id);
        if channel.idle() {
            channels.remove(conversation);
        }
    }

    /// The channels, also after a panic elsewhere left the lock poisoned: they hold nothing a half-done change could
    /// make unsafe to read.
    fn lock(&self) -> MutexGuard<'_, FxHashMap<Ulid, Channel>> {
        self.channels.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Channel {
    /// Hand `item` to every subscriber; one whose buffer is full is marked lagged and let go, so its stream resets.
    fn send(&mut self, item: &HubItem) {
        self.subscribers.retain(|subscriber| match subscriber.sender.try_send(item.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                subscriber.lagged.store(true, Ordering::Release);
                false
            }
            Err(TrySendError::Closed(_)) => false,
        });
    }

    fn streaming(&self) -> Vec<StreamingBlock> {
        self.blocks.iter().map(|block| StreamingBlock { block: block.id.clone(), text: block.text.clone() }).collect()
    }

    fn idle(&self) -> bool {
        self.subscribers.is_empty() && self.blocks.is_empty()
    }
}

impl Block {
    fn new(id: BlockId) -> Self {
        Self { id, text: String::new(), units: 0, sent: 0, flushed_at: None, flush_due: false }
    }

    /// Write `delta` over the text from `offset` for its own length, keeping any text beyond it, as a page places a
    /// delta; text that changes is sent again from where it changed.
    fn place(&mut self, offset: TextOffset, delta: &str) -> Result<(), LiveRefused> {
        let (at, end) = (offset.get(), offset.get() + utf16_len(delta));
        if at > self.units {
            return Err(LiveRefused::Gap { block: self.id.clone(), have: self.units, offset: at });
        }
        let split = || LiveRefused::Split { block: self.id.clone(), offset: at };
        let start = byte_at(&self.text, at).ok_or_else(split)?;
        let stop = if end < self.units { byte_at(&self.text, end).ok_or_else(split)? } else { self.text.len() };
        if self.text[start..stop] != *delta {
            self.text.replace_range(start..stop, delta);
            self.units = utf16_len(&self.text);
            self.sent = self.sent.min(at);
        }
        Ok(())
    }

    /// What subscribers lack, as one delta, which this marks sent.
    fn unsent(&mut self, now: Instant) -> Option<TextDelta> {
        self.flush_due = false;
        if self.sent >= self.units {
            return None;
        }
        let from = byte_at(&self.text, self.sent).expect("a block is sent up to a character's end");
        let delta =
            TextDelta { block: self.id.clone(), offset: TextOffset::new(self.sent), delta: self.text[from..].into() };
        (self.sent, self.flushed_at) = (self.units, Some(now));
        Some(delta)
    }
}

impl Subscription {
    /// The blocks streaming when this subscriber joined, each with its text so far.
    pub fn streaming(&self) -> &[StreamingBlock] {
        &self.streaming
    }

    /// The next item; `Lagged` as soon as more than `SUBSCRIBER_BUFFER` items went unread, whatever is still
    /// buffered, since a stream that lagged resets anyway.
    pub async fn recv(&mut self) -> Result<HubItem, HubRecvError> {
        if self.lagged.load(Ordering::Acquire) {
            return Err(HubRecvError::Lagged);
        }
        match self.receiver.recv().await {
            Some(item) => Ok(item),
            None if self.lagged.load(Ordering::Acquire) => Err(HubRecvError::Lagged),
            None => Err(HubRecvError::Closed),
        }
    }

    /// The next item if one waits, `Ok(None)` if none does.
    pub fn try_recv(&mut self) -> Result<Option<HubItem>, HubRecvError> {
        if self.lagged.load(Ordering::Acquire) {
            return Err(HubRecvError::Lagged);
        }
        match self.receiver.try_recv() {
            Ok(item) => Ok(Some(item)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) if self.lagged.load(Ordering::Acquire) => Err(HubRecvError::Lagged),
            Err(TryRecvError::Disconnected) => Err(HubRecvError::Closed),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.unsubscribe(&self.conversation, self.id);
    }
}

fn utf16_len(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

/// The byte index of the UTF-16 offset `units` in `text`, or `None` when it falls inside a character or past the end.
fn byte_at(text: &str, units: u64) -> Option<usize> {
    let mut count = 0;
    for (byte, character) in text.char_indices() {
        if count == units {
            return Some(byte);
        }
        count += character.len_utf16() as u64;
        if count > units {
            return None;
        }
    }
    (count == units).then_some(text.len())
}
