//! Within `streams`, a `stream` is one attempt's: the replay a viewer attaching is sent, the broadcast its viewers
//! follow live, the runner socket that owns it, and whether it is open, closed by this server (its runner lost, or the
//! server stopping) or ended by its runner. Every change is made under one lock that also covers the broadcast, so a
//! viewer attaching gets each message exactly once, in its replay or live, and a message from a socket a newer one
//! superseded is refused in the step that would have kept it. Only `streams` changes a stream, and it loses or reopens
//! one only while holding its map's lock, so a stream sits among the closed ones `streams` may forget only while it is
//! not open.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::ws::Utf8Bytes;
use desunits::SimTime;
use studio_protocol::{ProgressValue, RunnerMessage, STREAM_FORMAT, StreamRefusal, ViewerMessage, WallClock};
use tokio::sync::broadcast::{Receiver as LiveReceiver, Sender as LiveSender, channel as live_channel};
use tokio::sync::watch::{Receiver as OwnerReceiver, Sender as OwnerSender};

use super::StreamKey;
use super::replay::Replay;
use crate::stats::{Closed, ClosedBy, Snapshot};

/// Live messages a viewer may fall behind by before it is told it lagged and is dropped.
const LIVE_BACKLOG: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Open,
    Closed,
    Ended,
}

pub struct Stream {
    pub key: StreamKey,
    revisions: Arc<AtomicU64>,
    state: Mutex<State>,
    /// The number of the runner socket that owns the stream, which each socket watches to learn it was superseded.
    owner: OwnerSender<u64>,
}

struct State {
    phase: Phase,
    socket: u64,
    replay: Replay,
    /// None once the stream ended or was forgotten, which ends every viewer's following.
    live: Option<LiveSender<Utf8Bytes>>,
    opened_at: WallClock,
    sim_ms: SimTime,
    progress: Vec<ProgressValue>,
    closed: Option<Closed>,
}

/// What a runner's message did to its stream: the figures to write when they changed, and whether the stream ended.
pub(super) struct Took {
    pub snapshot: Option<Snapshot>,
    pub ended: bool,
}

pub(super) struct TakenOver {
    pub socket: u64,
    pub reopened: bool,
}

/// What a viewer attaching is sent first (`attached`, then the replay), and the broadcast it follows after, none once
/// the stream ended.
pub struct Attachment {
    pub first: Vec<Utf8Bytes>,
    pub live: Option<LiveReceiver<Utf8Bytes>>,
}

impl Stream {
    pub(super) fn new(key: StreamKey, revisions: Arc<AtomicU64>) -> Self {
        let state = State {
            phase: Phase::Open,
            socket: 0,
            replay: Replay::default(),
            live: Some(live_channel(LIVE_BACKLOG).0),
            opened_at: WallClock::now(),
            sim_ms: SimTime::ZERO,
            progress: Vec::new(),
            closed: None,
        };
        Self { key, revisions, state: Mutex::new(state), owner: OwnerSender::new(0) }
    }

    pub fn owner(&self) -> OwnerReceiver<u64> {
        self.owner.subscribe()
    }

    /// A runner socket takes the stream over, superseding the one before it; a closed stream opens again, its viewers
    /// no longer told its runner is lost. An ended stream takes no socket.
    pub(super) fn take_over(&self) -> Result<TakenOver, StreamRefusal> {
        let mut state = self.state.lock().expect("the stream");
        if state.phase == Phase::Ended {
            return Err(StreamRefusal::AttemptEnded);
        }
        let reopened = state.phase == Phase::Closed;
        if reopened {
            state.phase = Phase::Open;
            state.closed = None;
            state.replay.last_word(None);
        }
        state.socket += 1;
        self.owner.send_replace(state.socket);
        Ok(TakenOver { socket: state.socket, reopened })
    }

    /// A message from the runner's socket `socket`: kept in the replay, each message encoded once, and broadcast to
    /// the viewers following. Refused when another socket owns the stream, when the stream ended, and for a second
    /// hello on one socket.
    pub(super) fn take(&self, socket: u64, message: RunnerMessage) -> Result<Took, StreamRefusal> {
        let mut state = self.state.lock().expect("the stream");
        if state.socket != socket {
            return Err(StreamRefusal::Superseded);
        }
        if state.phase == Phase::Ended {
            return Err(StreamRefusal::AttemptEnded);
        }
        let (changed, ended) = match message {
            RunnerMessage::Hello { .. } => return Err(StreamRefusal::Unreadable),
            RunnerMessage::Started { v } => {
                let encoded = encode(&ViewerMessage::Started { v });
                state.replay.started(encoded.clone());
                state.publish(encoded);
                (false, false)
            }
            RunnerMessage::Progress(progress) => {
                state.sim_ms = progress.sim_ms;
                state.progress.clone_from(&progress.values);
                let encoded = encode(&ViewerMessage::Progress(progress));
                state.replay.progress(encoded.clone());
                state.publish(encoded);
                (true, false)
            }
            RunnerMessage::Ended(ended) => {
                state.sim_ms = ended.ending.sim_ms;
                let encoded = encode(&ViewerMessage::Ended(ended));
                state.replay.last_word(Some(encoded.clone()));
                state.publish(encoded);
                // The broadcast closes behind the ending, so each viewer leaves once it has it.
                state.live = None;
                state.phase = Phase::Ended;
                state.closed = Some(Closed { at: WallClock::now(), by: ClosedBy::Runner });
                (true, true)
            }
        };
        let snapshot = changed.then(|| self.snapshot_of(&state));
        Ok(Took { snapshot, ended })
    }

    /// The grace after socket `socket` closed without the ending has passed: unless a newer socket took over or the
    /// stream is no longer open, its runner is lost, which its viewers are told; the stream stays theirs to follow if
    /// the runner comes back. The figures to write say so.
    pub(super) fn lose(&self, socket: u64) -> Option<Snapshot> {
        let mut state = self.state.lock().expect("the stream");
        if state.socket != socket || state.phase != Phase::Open {
            return None;
        }
        let encoded = encode(&ViewerMessage::RunnerLost { v: STREAM_FORMAT });
        state.replay.last_word(Some(encoded.clone()));
        state.publish(encoded);
        state.close();
        Some(self.snapshot_of(&state))
    }

    /// The server is stopping: an open stream closes, and the figures to write say so.
    pub(super) fn stop(&self) -> Option<Snapshot> {
        let mut state = self.state.lock().expect("the stream");
        if state.phase != Phase::Open {
            return None;
        }
        state.close();
        Some(self.snapshot_of(&state))
    }

    /// What a viewer attaching gets, taken under the lock every message is kept and broadcast under, so none is
    /// both replayed and followed live.
    pub fn attach(&self) -> Attachment {
        let state = self.state.lock().expect("the stream");
        let replay = state.replay.messages();
        let attached = ViewerMessage::Attached {
            v: STREAM_FORMAT,
            run: self.key.run.clone(),
            attempt: self.key.attempt,
            replayed: u32::try_from(replay.len()).unwrap_or(u32::MAX),
        };
        let first = [encode(&attached)].into_iter().chain(replay).collect();
        Attachment { first, live: state.live.as_ref().map(LiveSender::subscribe) }
    }

    /// The figures as they stand, written as a new revision.
    pub(super) fn snapshot(&self) -> Snapshot {
        self.snapshot_of(&self.state.lock().expect("the stream"))
    }

    pub(super) fn forget(&self) {
        let mut state = self.state.lock().expect("the stream");
        state.live = None;
        state.replay.clear();
    }

    pub(super) fn is_open(&self) -> bool {
        self.state.lock().expect("the stream").phase == Phase::Open
    }

    /// Whether the runner socket `socket` owns the stream.
    pub(super) fn owned_by(&self, socket: u64) -> bool {
        self.state.lock().expect("the stream").socket == socket
    }

    /// The server locked one more segment of the picture: said to the viewers following, never kept in the replay.
    pub(super) fn edge(&self, locked_sim_ms: SimTime) {
        let state = self.state.lock().expect("the stream");
        state.publish(encode(&ViewerMessage::Edge { v: STREAM_FORMAT, locked_sim_ms }));
    }

    /// The figures in `state` under a revision drawn from the process's one counter, while the stream's lock is held,
    /// so a later snapshot of the stream always has a greater one, even when the stream was forgotten and opened anew.
    fn snapshot_of(&self, state: &State) -> Snapshot {
        Snapshot {
            key: self.key.clone(),
            revision: self.revisions.fetch_add(1, Ordering::Relaxed) + 1,
            opened_at: state.opened_at,
            sim_ms: state.sim_ms,
            progress: state.progress.clone(),
            closed: state.closed,
        }
    }
}

impl State {
    /// One message to every viewer following; none following is no fault.
    fn publish(&self, encoded: Utf8Bytes) {
        if let Some(live) = &self.live {
            let _ = live.send(encoded);
        }
    }

    fn close(&mut self) {
        self.phase = Phase::Closed;
        self.closed = Some(Closed { at: WallClock::now(), by: ClosedBy::Server });
    }
}

fn encode(message: &ViewerMessage) -> Utf8Bytes {
    message.text().into()
}
