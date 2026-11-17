//! Within `studio-streaming`, `frames` is one attempt's picture as this server holds it: the frames of the run's
//! worldview recording, which the runner sends as dessim writes them (live frames, 26 October 2026), its end block
//! last. Per open attempt the store keeps the scenario frame and every frame after it in memory, in the order taken in,
//! grouped into segments (a keyframe and the deltas behind it, the end block in the last; a run stopped before its
//! first slice has one, its end block alone), and appends each to `<cache>/<run>/<n>/live.worldview` as it is taken in,
//! so the file is an unfinished container file `Reader::open` reads by scanning. A process that takes an attempt up
//! after an earlier one, a restarted server's or one that had forgotten the attempt, holds only the frames sent to it:
//! it numbers its segments on from those the earlier process put in the store rather than overwrite them, and its
//! copies, in memory and on disk, stop short of the run's start (`Earlier`). A segment is closed, and so handed to the
//! upload queue, when the next keyframe is taken in, when the socket goes quiet after a batch, or when the attempt
//! ends; a closed segment that grows again is simply closed again. After a lost connection the runner sends the
//! scenario block and the open segment from its keyframe again: a keyframe at or before the open segment's own starts
//! that segment over from its keyframe, in memory and on disk, keeping the declarations ahead of it, which the runner
//! does not send again, and a scenario block already held is passed over. A recording with no keyframe, a run stopped
//! before its first slice, is sent again from just past its scenario block, and a frame of it held already, its end
//! block among them, is passed over. A declaration belongs to the segment it is sent with: dessim states one ahead of
//! the keyframe stamped with the same instant, so declarations after a closed segment open the next one. Each reader
//! following the attempt in memory is sent every frame taken through a queue of its own, so a burst, as a runner sends
//! everything it wrote while its connection was lost, leaves no reader behind. After the attempt ends the memory is
//! released once its uploads are done. The one bound: an open attempt's frames in memory may not pass `open_max_bytes`.
//! A reader's queue holds only the frames it has yet to take, whose bytes the store holds too.
//!
//! TODO(overstuffed): two things live here that want their own modules: one attempt's live frames
//! (`AttemptFrames`) and the map of every attempt (`FrameStore`).

use std::fmt;
use std::io::{Error as IoError, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use desunits::{SimDuration, SimTime};
use rustc_hash::FxHashMap;
use studio_protocol::StreamRefusal;
use tokio::fs::{File, create_dir_all, try_exists};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::{UnboundedReceiver as LiveReceiver, UnboundedSender as LiveSender, unbounded_channel};
use worldview::container::index::OrderCheck;
use worldview::{Frame, Payload, Scenario, Tag};

use crate::cache::vouch;
use crate::floor::Scenes;
use crate::streams::StreamKey;
use crate::upload::{Rows, Uploader, uploaded};

/// The unfinished container file the frames taken are appended to, under the attempt's cache folder.
pub const LIVE_FILE: &str = "live.worldview";
/// The final recording's file in an attempt's folder: fetched from the run-object store under the key the API's
/// `attempts.object_key` names (the runner sends the recording to the API's door), never written by a runner here.
pub const RECORDING_FILE: &str = "recording.worldview";
/// The empty file beside the live file that says it holds the whole run, written by whatever wrote the live file and
/// knew it whole (`cache::DiskCopy`); a live file without it stops short of the run's end for all a reader knows.
pub const WHOLE_MARK: &str = "live.whole";

/// One frame as held and sent to the readers: its tag and instant off the header, and the whole frame's bytes.
#[derive(Debug, Clone)]
pub struct Held {
    pub tag: Tag,
    pub at: SimTime,
    pub bytes: Bytes,
}

/// What a reader following an open attempt in memory is sent after its snapshot: each frame taken, then the ending.
#[derive(Debug, Clone)]
pub enum FrameEvent {
    Frame(Held),
    Ended,
}

/// An attempt's frames in memory as one reader takes them: the scenario block, every frame after it so far, whether
/// the attempt has ended, and whether the frames stop short of the run's start, taken up after an earlier process.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub scenario: Held,
    pub frames: Vec<Held>,
    pub ended: bool,
    pub incomplete: bool,
}

/// What a reader finds of an attempt this store holds.
#[derive(Debug)]
pub enum InMemory {
    /// The frames so far and the reader's queue of everything after them; an ended attempt's stays empty.
    Held(Snapshot, LiveReceiver<FrameEvent>),
    /// The attempt is open and its scenario block has not come yet.
    NotYet,
    /// The attempt ended and its memory was given back once its uploads were done: its disk copy serves it, as whole as
    /// it was vouched for.
    Released,
}

/// A segment the store closed: its number from 0 and the simtime instant the picture then reaches, which is the end
/// of its last slice, or the run's end the end block states in a run stopped before its first slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosedSegment {
    pub segment: u32,
    pub locked_sim_ms: SimTime,
}

/// What a push did besides keeping the frame: the scenario block opened the stream (and goes up first), a frame was
/// taken, a keyframe closed the segment before it, or a frame already held was passed over: the scenario block, or
/// after the end block a frame of the last segment sent again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushed {
    Scenario,
    Taken,
    Closed(ClosedSegment),
    Repeated,
}

/// Why a frame is refused, with the words for the log; the runner's close frame carries the protocol's refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NoScenario,
    FrameOrder(String),
    TooLarge,
    Disk(String),
    /// The database did not say what an earlier process put in the store, so the segments cannot be numbered safely.
    Database(String),
}

impl Refusal {
    /// The refusal as the protocol spells it: an order fault, the bound, or the disk or the database, which another
    /// try may find answering.
    pub fn refusal(&self) -> StreamRefusal {
        match self {
            Self::NoScenario | Self::FrameOrder(_) => StreamRefusal::FrameOrder,
            Self::TooLarge => StreamRefusal::FrameTooLarge,
            Self::Disk(_) | Self::Database(_) => StreamRefusal::Unavailable,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoScenario => f.write_str("a frame before the scenario block"),
            Self::FrameOrder(detail) => write!(f, "frame order: {detail}"),
            Self::TooLarge => f.write_str("the open attempt's frames would pass the memory bound"),
            Self::Disk(detail) => write!(f, "the disk copy could not be written: {detail}"),
            Self::Database(detail) => write!(f, "the earlier uploads could not be read: {detail}"),
        }
    }
}

struct Segment {
    /// Index into `frames` of the segment's first frame, its keyframe or a declaration ahead of it.
    start: usize,
    /// Index into `frames` of the frame that opened the segment: its keyframe, or the end block of a run stopped
    /// before its first slice.
    opened_at: usize,
    /// How many of `frames` the last close of this segment covered; past it the segment has grown since.
    closed_through: usize,
    /// The bytes held, which is the live file's length, and the order check, as they stood at `opened_at`: what a
    /// segment started over goes back to. The runner sends the segment again from its keyframe, so the declarations
    /// ahead of it, which it does not send again, are kept.
    bytes_before: u64,
    order_before: OrderCheck,
}

/// What an earlier process left of an attempt this one takes up after it: the frames it took, which are not held here,
/// and its segments in the store, `segments` of them, the last reaching `locked`, which this process numbers on from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Earlier {
    pub segments: u32,
    pub locked: SimTime,
}

impl Earlier {
    /// The number of this process's first segment, opened at `at`. The runner sends its open segment again from its
    /// keyframe: when `at` falls inside the earlier process's last segment, that segment is started over, else the
    /// next one opens.
    fn first_segment(&self, at: SimTime) -> u32 {
        if at < self.locked { self.segments.saturating_sub(1) } else { self.segments }
    }
}

struct State {
    ended: bool,
    /// Set once a frame was refused for size; every later frame is refused too, so a reconnecting runner learns the
    /// same answer rather than appending past the bound.
    refused: bool,
    scenario: Option<Held>,
    /// The slice width the scenario block states, which turns a frame's instant into the end of its slice.
    slice: Option<SimDuration>,
    frames: Vec<Held>,
    segments: Vec<Segment>,
    /// What an earlier process left of the attempt, when this one took it up after it.
    earlier: Option<Earlier>,
    /// The number of `segments[0]` in the store.
    numbered_from: u32,
    bytes: u64,
    order: OrderCheck,
    /// The queue of each reader following the attempt in memory, until the attempt ends or the reader goes.
    readers: Vec<LiveSender<FrameEvent>>,
}

/// What `admit` makes of a frame: taken at its tag and instant, starting the open segment over when `restart`; or a
/// frame already held, passed over.
enum Admitted {
    Frame { tag: Tag, at: SimTime, restart: bool, slice: Option<SimDuration> },
    Repeated,
}

/// One attempt's frames.
pub struct AttemptFrames {
    pub key: StreamKey,
    /// `<cache>/<run>/<n>`.
    pub folder: PathBuf,
    open_max_bytes: u64,
    /// Where an earlier process's uploads are read from.
    rows: Option<Rows>,
    state: Mutex<State>,
    live_file: AsyncMutex<Option<File>>,
}

impl AttemptFrames {
    fn new(key: StreamKey, folder: PathBuf, open_max_bytes: u64, rows: Option<Rows>) -> Self {
        let state = State {
            ended: false,
            refused: false,
            scenario: None,
            slice: None,
            frames: Vec::new(),
            segments: Vec::new(),
            earlier: None,
            numbered_from: 0,
            bytes: 0,
            order: OrderCheck::default(),
            readers: Vec::new(),
        };
        Self { key, folder, open_max_bytes, rows, state: Mutex::new(state), live_file: AsyncMutex::new(None) }
    }

    pub fn live_path(&self) -> PathBuf {
        self.folder.join(LIVE_FILE)
    }

    /// One binary message from the runner: checked, appended to the live file, kept, and sent to every reader.
    pub async fn push(&self, bytes: Vec<u8>) -> Result<Pushed, Refusal> {
        let (tag, at, restart, slice) = match self.admit(&bytes)? {
            Admitted::Frame { tag, at, restart, slice } => (tag, at, restart, slice),
            Admitted::Repeated => return Ok(Pushed::Repeated),
        };
        if restart {
            let bytes_before = {
                let mut state = self.state.lock().expect("the attempt's frames");
                let last = state.segments.pop().expect("a restart has an open segment");
                state.frames.truncate(last.opened_at);
                state.bytes = last.bytes_before;
                state.order = last.order_before;
                last.bytes_before
            };
            self.truncate_live(bytes_before).await?;
        }
        let earlier = if tag == Tag::Scenario { self.earlier().await? } else { None };
        self.append_live(tag == Tag::Scenario, &bytes).await?;
        let held = Held { tag, at, bytes: Bytes::from(bytes) };
        let mut state = self.state.lock().expect("the attempt's frames");
        let pushed = match tag {
            Tag::Scenario => {
                state.bytes += held.bytes.len() as u64;
                state.scenario = Some(held.clone());
                state.slice = slice;
                state.earlier = earlier;
                Pushed::Scenario
            }
            // A keyframe opens a segment, and so does the end block of a run stopped before its first slice.
            _ if tag == Tag::Keyframe || (tag == Tag::End && state.segments.is_empty()) => {
                let start = match state.segments.last() {
                    None => 0,
                    Some(last) if only_declares(&state.frames[last.closed_through..]) => last.closed_through,
                    Some(_) => state.frames.len(),
                };
                let closed = if start == state.frames.len() { self.close_last(&mut state, false) } else { None };
                let (opened_at, bytes_before, order_before) = (state.frames.len(), state.bytes, state.order);
                state.bytes += held.bytes.len() as u64;
                state.frames.push(held.clone());
                if state.segments.is_empty() {
                    state.numbered_from = state.earlier.map_or(0, |earlier| earlier.first_segment(at));
                }
                state.segments.push(Segment { start, opened_at, closed_through: start, bytes_before, order_before });
                closed.map_or(Pushed::Taken, Pushed::Closed)
            }
            _ => {
                state.bytes += held.bytes.len() as u64;
                state.frames.push(held.clone());
                Pushed::Taken
            }
        };
        // Sent under the lock, so a reader whose snapshot holds this frame is not sent it too; a reader that went is
        // forgotten.
        state.readers.retain(|reader| reader.send(FrameEvent::Frame(held.clone())).is_ok());
        Ok(pushed)
    }

    /// The checks a frame passes before anything is written: it decodes whole (a scenario block as the block it is,
    /// which states the slice width), it comes where the wire format says, its instant does not run backwards (but
    /// for a keyframe starting the open segment over), and the memory bound holds.
    fn admit(&self, bytes: &[u8]) -> Result<Admitted, Refusal> {
        let order_fault = |detail: String| Refusal::FrameOrder(detail);
        let (frame, consumed) = Frame::decode_at(bytes, 0).map_err(|fault| order_fault(fault.to_string()))?;
        if consumed != bytes.len() {
            return Err(order_fault(format!(
                "a {}-byte message carries a {consumed}-byte frame and more",
                bytes.len()
            )));
        }
        let (tag, t) = (frame.header.tag, frame.header.t);
        let slice = match tag {
            Tag::Scenario => {
                let body = frame.contents(0).map_err(|fault| order_fault(fault.to_string()))?;
                let block = Scenario::from_bytes(&body, 0).map_err(|fault| order_fault(fault.to_string()))?;
                Some(block.slice_width)
            }
            _ => None,
        };
        let state = self.state.lock().expect("the attempt's frames");
        if state.refused {
            return Err(Refusal::TooLarge);
        }
        if state.ended {
            return Err(order_fault(format!("a {tag} frame after the attempt's end")));
        }
        match tag {
            Tag::Scenario if state.scenario.as_ref().is_some_and(|held| held.bytes.as_ref() == bytes) => {
                return Ok(Admitted::Repeated);
            }
            Tag::Scenario if state.scenario.is_some() => {
                return Err(order_fault("a second, different scenario block".to_owned()));
            }
            Tag::Scenario => {}
            _ if state.scenario.is_none() => return Err(Refusal::NoScenario),
            Tag::Index => return Err(order_fault("an index frame on the live stream".to_owned())),
            Tag::Delta if state.segments.is_empty() => {
                return Err(order_fault(format!("a {tag} frame before any keyframe")));
            }
            _ => {}
        }
        // A keyframe at or before the open segment's own: the runner sends that segment again, from its keyframe.
        let restart = tag == Tag::Keyframe
            && state.segments.last().is_some_and(|last| {
                state.frames[last.start..]
                    .iter()
                    .find(|held| held.tag == Tag::Keyframe)
                    .is_some_and(|k| t.0 <= k.at.as_ms())
            });
        // The end block is the recording's last frame: nothing follows it but the last segment sent again, from its
        // keyframe, which starts it over, or, in a recording with no keyframe, from just past the scenario block: a
        // frame of that segment, the end block itself among them, is held already.
        if !restart && state.frames.last().is_some_and(|held| held.tag == Tag::End) {
            let start = state.segments.last().map_or(0, |last| last.start);
            if state.frames[start..].iter().any(|held| held.bytes.as_ref() == bytes) {
                return Ok(Admitted::Repeated);
            }
            return Err(order_fault(format!("a {tag} frame after the end block")));
        }
        let (mut order, bytes_held) = match (restart, state.segments.last()) {
            (true, Some(last)) => (last.order_before, last.bytes_before),
            _ => (state.order, state.bytes),
        };
        order.admit(tag, t, bytes_held).map_err(|fault| order_fault(fault.to_string()))?;
        if bytes_held + bytes.len() as u64 > self.open_max_bytes {
            drop(state);
            self.state.lock().expect("the attempt's frames").refused = true;
            return Err(Refusal::TooLarge);
        }
        drop(state);
        self.state.lock().expect("the attempt's frames").order = order;
        Ok(Admitted::Frame { tag, at: SimTime::ms(t.0), restart, slice })
    }

    /// What an earlier process left of the attempt, asked as this one takes the scenario block: what the row says it
    /// put in the store, or else a live file it wrote on this disk; `None` when this process takes the attempt from
    /// its start.
    async fn earlier(&self) -> Result<Option<Earlier>, Refusal> {
        let uploaded = match &self.rows {
            Some(rows) => uploaded(rows, &self.key).await.map_err(|fault| Refusal::Database(fault.to_string()))?,
            None => None,
        };
        let written = try_exists(self.live_path()).await.map_err(|fault| Refusal::Disk(fault.to_string()))?;
        Ok(uploaded.or(written.then_some(Earlier::default())))
    }

    async fn append_live(&self, first: bool, bytes: &[u8]) -> Result<(), Refusal> {
        let disk = |fault: IoError| Refusal::Disk(fault.to_string());
        let mut file = self.live_file.lock().await;
        if first {
            create_dir_all(&self.folder).await.map_err(disk)?;
            *file = Some(File::create(self.live_path()).await.map_err(disk)?);
        }
        let Some(open) = file.as_mut() else { return Err(Refusal::NoScenario) };
        open.write_all(bytes).await.map_err(disk)?;
        open.flush().await.map_err(disk)
    }

    /// The live file cut back to `length`, the segment started over gone from it.
    async fn truncate_live(&self, length: u64) -> Result<(), Refusal> {
        let disk = |fault: IoError| Refusal::Disk(fault.to_string());
        let mut file = self.live_file.lock().await;
        let Some(open) = file.as_mut() else { return Err(Refusal::NoScenario) };
        open.set_len(length).await.map_err(disk)?;
        open.seek(SeekFrom::Start(length)).await.map_err(disk)?;
        Ok(())
    }

    /// The newest segment closed if it grew since its last close. A tail of declarations alone is left for the keyframe
    /// behind them to open the next segment with, unless the attempt is ending and no keyframe will come.
    fn close_last(&self, state: &mut State, ending: bool) -> Option<ClosedSegment> {
        let count = state.frames.len();
        let number = state.numbered_from + u32::try_from(state.segments.len().checked_sub(1)?).unwrap_or(u32::MAX);
        let segment = state.segments.last_mut()?;
        if segment.closed_through >= count || (!ending && only_declares(&state.frames[segment.closed_through..])) {
            return None;
        }
        segment.closed_through = count;
        let held = &state.frames[segment.start..];
        let locked_sim_ms = match held.iter().rev().find(|held| matches!(held.tag, Tag::Keyframe | Tag::Delta)) {
            Some(last) => last.at.saturating_add(
                state.slice.expect("a segment follows the scenario block, which states the slice width"),
            ),
            None => held.iter().find(|held| held.tag == Tag::End)?.at,
        };
        Some(ClosedSegment { segment: number, locked_sim_ms })
    }

    pub fn settle(&self) -> Option<ClosedSegment> {
        let mut state = self.state.lock().expect("the attempt's frames");
        self.close_last(&mut state, false)
    }

    /// The attempt ended: the newest segment closed, its readers told and let go, nothing more taken.
    pub fn end(&self) -> Option<ClosedSegment> {
        let mut state = self.state.lock().expect("the attempt's frames");
        state.ended = true;
        let closed = self.close_last(&mut state, true);
        for reader in state.readers.drain(..) {
            let _ = reader.send(FrameEvent::Ended);
        }
        closed
    }

    pub fn is_ended(&self) -> bool {
        self.state.lock().expect("the attempt's frames").ended
    }

    /// Whether the frames stop short of the run's start, the attempt taken up after an earlier process.
    pub fn incomplete(&self) -> bool {
        self.state.lock().expect("the attempt's frames").earlier.is_some()
    }

    /// The memory given back once an ended attempt's uploads are done, the disk copy first vouched for as the whole run
    /// when this process took the attempt from its start, so a reader that finds the memory gone finds the copy's word;
    /// replays read the disk copy from here on.
    pub async fn release(&self) {
        if !self.incomplete()
            && let Err(fault) = vouch(&self.folder).await
        {
            eprintln!(
                "studio-streaming: {}/{}: the disk copy was not vouched whole: {fault}",
                self.key.run, self.key.attempt.0
            );
        }
        let mut state = self.state.lock().expect("the attempt's frames");
        state.frames = Vec::new();
        state.scenario = None;
    }

    /// The frames so far and a queue of everything after them, taken under one lock so no frame falls between.
    pub fn attach(&self) -> InMemory {
        let mut state = self.state.lock().expect("the attempt's frames");
        let Some(scenario) = state.scenario.clone() else {
            return if state.ended { InMemory::Released } else { InMemory::NotYet };
        };
        let (reader, queue) = unbounded_channel();
        if !state.ended {
            state.readers.push(reader);
        }
        let snapshot = Snapshot {
            scenario,
            frames: state.frames.clone(),
            ended: state.ended,
            incomplete: state.earlier.is_some(),
        };
        InMemory::Held(snapshot, queue)
    }

    pub fn scenario(&self) -> Option<Bytes> {
        self.state.lock().expect("the attempt's frames").scenario.as_ref().map(|held| held.bytes.clone())
    }

    /// Segment `segment`'s frames as they stand, keyframe first; `None` for a segment that is not held here.
    pub fn segment(&self, segment: u32) -> Option<Vec<Held>> {
        let state = self.state.lock().expect("the attempt's frames");
        let index = usize::try_from(segment.checked_sub(state.numbered_from)?).ok()?;
        let start = state.segments.get(index)?.start;
        let end = state.segments.get(index + 1).map_or(state.frames.len(), |next| next.start);
        Some(state.frames[start..end].to_vec())
    }

    /// How many segments the attempt's picture has, an earlier process's among them.
    pub fn segments(&self) -> u32 {
        let state = self.state.lock().expect("the attempt's frames");
        state.numbered_from + u32::try_from(state.segments.len()).unwrap_or(u32::MAX)
    }

    /// The simtime instant the newest frame states, the live edge a frame client is told.
    pub fn reach(&self) -> Option<SimTime> {
        let state = self.state.lock().expect("the attempt's frames");
        state.frames.last().or(state.scenario.as_ref()).map(|held| held.at)
    }

    pub fn bytes_held(&self) -> u64 {
        self.state.lock().expect("the attempt's frames").bytes
    }
}

/// Every attempt's frames this server holds, shared by the runner sockets, the frame clients, the upload queue and
/// the sweeper.
#[derive(Clone)]
pub struct FrameStore {
    inner: Arc<Inner>,
}

struct Inner {
    cache_dir: PathBuf,
    open_max_bytes: u64,
    uploader: Uploader,
    attempts: Mutex<FxHashMap<StreamKey, Arc<AttemptFrames>>>,
    scenes: Scenes,
}

impl FrameStore {
    pub fn new(cache_dir: PathBuf, open_max_bytes: u64, uploader: Uploader) -> Self {
        let inner =
            Inner { cache_dir, open_max_bytes, uploader, attempts: Mutex::default(), scenes: Scenes::default() };
        Self { inner: Arc::new(inner) }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.inner.cache_dir
    }

    pub fn folder(&self, key: &StreamKey) -> PathBuf {
        self.inner.cache_dir.join(key.run.as_str()).join(key.attempt.0.to_string())
    }

    pub fn uploader(&self) -> &Uploader {
        &self.inner.uploader
    }

    pub fn scenes(&self) -> &Scenes {
        &self.inner.scenes
    }

    /// The attempt's frames, made if this is its first `started`; a reconnecting runner gets the same ones.
    pub fn open(&self, key: &StreamKey) -> Arc<AttemptFrames> {
        let mut attempts = self.inner.attempts.lock().expect("the frame store");
        attempts
            .entry(key.clone())
            .or_insert_with(|| {
                let rows = self.inner.uploader.rows().cloned();
                Arc::new(AttemptFrames::new(key.clone(), self.folder(key), self.inner.open_max_bytes, rows))
            })
            .clone()
    }

    pub fn get(&self, key: &StreamKey) -> Option<Arc<AttemptFrames>> {
        self.inner.attempts.lock().expect("the frame store").get(key).cloned()
    }

    pub fn is_open(&self, key: &StreamKey) -> bool {
        self.get(key).is_some_and(|frames| !frames.is_ended())
    }

    /// Whether an upload of the attempt is queued or in flight here, in which case the disk copy may be the only one.
    pub fn uploading(&self, key: &StreamKey) -> bool {
        self.inner.uploader.pending(key) > 0
    }

    /// The attempt ended: its newest segment closed, its readers told, and the upload queue's last job queued, which
    /// says in the row that the uploads finished and releases the memory once every upload ahead of it has landed.
    pub fn close(&self, key: &StreamKey) -> Option<ClosedSegment> {
        let frames = self.get(key)?;
        let closed = frames.end();
        if let Some(closed) = closed {
            self.inner.uploader.segment(&frames, closed);
        }
        self.inner.uploader.finished(&frames);
        closed
    }

    /// The stream is forgotten: so are its frames and its drained queue.
    pub fn forget(&self, key: &StreamKey) {
        self.inner.attempts.lock().expect("the frame store").remove(key);
        self.inner.uploader.forget(key);
        self.inner.scenes.forget(key);
    }
}

fn only_declares(tail: &[Held]) -> bool {
    tail.iter().all(|held| held.tag == Tag::Declare)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use desunits::SimTime;
    use studio_protocol::{AttemptNumber, StreamRefusal};
    use tempfile::TempDir;
    use worldview::{Frame, Ms, Reader, Tag};

    use super::{AttemptFrames, FrameEvent, FrameStore, InMemory, Pushed, Refusal, WHOLE_MARK};
    use crate::floor::tests::{scenario_frame, scenario_frame_of_run};
    use crate::streams::StreamKey;
    use crate::upload::Uploader;

    pub(crate) fn key(n: u32) -> StreamKey {
        StreamKey { run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(), attempt: AttemptNumber(n) }
    }

    /// A frame whose body is `fill` repeated `len` times, compressed as the writer compresses; a scenario block is a
    /// real one, since the store reads the slice width off it.
    pub(crate) fn frame(tag: Tag, at_ms: u64, len: usize) -> Vec<u8> {
        match tag {
            Tag::Scenario => scenario_frame(),
            _ => Frame::encode(tag, Ms(at_ms), &vec![(at_ms % 251) as u8; len], true).expect("a frame encodes"),
        }
    }

    pub(crate) fn frames_in(dir: &TempDir, open_max_bytes: u64) -> (FrameStore, Arc<AttemptFrames>) {
        let objects = dir.path().join("objects");
        std::fs::create_dir_all(&objects).unwrap();
        let uploader = Uploader::for_tests(&objects);
        let store = FrameStore::new(dir.path().join("cache"), open_max_bytes, uploader);
        let attempt = store.open(&key(1));
        (store, attempt)
    }

    /// A frame store over a scratch folder that lives as long as the process, for tests of the streams that never
    /// touch it.
    pub(crate) fn frame_store() -> FrameStore {
        let dir = TempDir::new().unwrap().keep();
        let objects = dir.join("objects");
        std::fs::create_dir_all(&objects).unwrap();
        FrameStore::new(dir.join("cache"), u64::MAX, Uploader::for_tests(&objects))
    }

    fn tags_on_disk(frames: &AttemptFrames) -> Vec<Tag> {
        let mut reader = Reader::open(&frames.live_path()).expect("an unfinished file reads by scanning");
        reader.frames_from(0).map(|read| read.unwrap().1.header.tag).collect()
    }

    #[tokio::test]
    async fn frames_are_taken_in_order_grouped_into_segments_and_mirrored_on_disk() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        assert_eq!(frames.push(frame(Tag::Delta, 0, 10)).await, Err(Refusal::NoScenario));
        assert_eq!(frames.push(frame(Tag::Scenario, 0, 10)).await, Ok(Pushed::Scenario));
        let refused = frames.push(frame(Tag::Delta, 0, 10)).await.expect_err("a delta before any keyframe");
        assert_eq!(refused.refusal(), StreamRefusal::FrameOrder);
        assert_eq!(frames.push(frame(Tag::Keyframe, 0, 10)).await, Ok(Pushed::Taken));
        for at in [0, 10_000, 20_000] {
            assert_eq!(frames.push(frame(Tag::Delta, at, 10)).await, Ok(Pushed::Taken));
        }
        assert_eq!(
            frames.settle().map(|closed| (closed.segment, closed.locked_sim_ms)),
            Some((0, SimTime::ms(30_000)))
        );
        assert!(frames.settle().is_none(), "nothing grew since the close");
        let closed = frames.push(frame(Tag::Keyframe, 30_000, 10)).await.expect("the next segment opens");
        assert_eq!(closed, Pushed::Taken, "segment 0 was already closed by the quiet");
        assert_eq!(frames.push(frame(Tag::Delta, 30_000, 10)).await, Ok(Pushed::Taken));
        let late = frames.push(frame(Tag::Delta, 20_000, 10)).await.expect_err("simtime does not run backwards");
        assert_eq!(late.refusal(), StreamRefusal::FrameOrder);
        assert_eq!(frames.segments(), 2);
        assert_eq!(frames.segment(0).map(|held| held.len()), Some(4));
        assert_eq!(frames.reach(), Some(SimTime::ms(30_000)));
        assert_eq!(
            tags_on_disk(&frames),
            [Tag::Scenario, Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Delta, Tag::Keyframe, Tag::Delta],
            "the disk copy holds every frame taken, in order"
        );
        let reader = Reader::open(&frames.live_path()).unwrap();
        assert_eq!(reader.live_edge(), Some(Ms(30_000)));
    }

    /// The runner sends a segment again after a lost connection: the keyframe at the open segment's own instant
    /// starts it over, in memory, on disk and in the byte count, and the scenario block said again is passed over.
    #[tokio::test]
    async fn a_segment_sent_again_starts_over_and_a_repeated_scenario_block_is_passed_over() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        let scenario = frame(Tag::Scenario, 0, 10);
        frames.push(scenario.clone()).await.unwrap();
        frames.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 10_000, 10)).await.unwrap();
        frames.push(frame(Tag::Keyframe, 20_000, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 20_000, 10)).await.unwrap();
        let before = frames.bytes_held();
        let partial = frames.settle().expect("the partial segment 1 closes on the quiet");
        assert_eq!((partial.segment, partial.locked_sim_ms), (1, SimTime::ms(30_000)));
        // The connection was lost mid-segment; the runner says the scenario block and segment 1 again, whole.
        assert_eq!(frames.push(scenario).await, Ok(Pushed::Repeated));
        assert_eq!(frames.push(frame(Tag::Keyframe, 20_000, 10)).await, Ok(Pushed::Taken), "segment 1 starts over");
        for at in [20_000, 30_000, 40_000] {
            frames.push(frame(Tag::Delta, at, 10)).await.unwrap();
        }
        assert_eq!(frames.segments(), 2, "segment 1 was started over, not opened again");
        let tags = |segment| frames.segment(segment).unwrap().iter().map(|held| held.tag).collect::<Vec<_>>();
        assert_eq!(tags(1), [Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Delta]);
        assert_eq!(frames.bytes_held(), before + 2 * frame(Tag::Delta, 30_000, 10).len() as u64);
        assert_eq!(
            tags_on_disk(&frames),
            [Tag::Scenario, Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::Delta],
            "the disk copy was cut back and holds the segment once"
        );
        let again = frames.settle().expect("segment 1 grew since its partial close");
        assert_eq!((again.segment, again.locked_sim_ms), (1, SimTime::ms(50_000)), "closed again, whole");
        let other = frames.push(scenario_frame_of_run("another-run")).await.expect_err("a different scenario block");
        assert_eq!(other.refusal(), StreamRefusal::FrameOrder);
    }

    #[tokio::test]
    async fn an_attempt_past_the_memory_bound_is_refused_for_good() {
        let dir = TempDir::new().unwrap();
        // Room for the scenario block and a little more, so the keyframe fits and the large delta does not.
        let (_store, frames) = frames_in(&dir, scenario_frame().len() as u64 + 200);
        assert!(frames.push(frame(Tag::Scenario, 0, 10)).await.is_ok());
        assert!(frames.push(frame(Tag::Keyframe, 0, 10)).await.is_ok());
        // Stored uncompressed, so the frame is as large on the wire as its body.
        let large = Frame::encode(Tag::Delta, Ms::ZERO, &[7u8; 1_000], false).unwrap();
        assert_eq!(frames.push(large).await, Err(Refusal::TooLarge));
        assert_eq!(frames.push(frame(Tag::Delta, 0, 1)).await, Err(Refusal::TooLarge), "sticky");
        assert_eq!(Refusal::TooLarge.refusal(), StreamRefusal::FrameTooLarge);
    }

    /// A segment sent again after a lost connection, whose declarations came ahead of its keyframe: the runner sends
    /// it again from the keyframe, so the declarations are kept, once, in memory, on disk and in the byte count.
    #[tokio::test]
    async fn a_segment_started_over_keeps_the_declarations_ahead_of_its_keyframe() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        let scenario = frame(Tag::Scenario, 0, 10);
        frames.push(scenario.clone()).await.unwrap();
        frames.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 0, 10)).await.unwrap();
        frames.settle().expect("segment 0 closes");
        frames.push(frame(Tag::Declare, 10_000, 10)).await.unwrap();
        frames.push(frame(Tag::Keyframe, 10_000, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 10_000, 10)).await.unwrap();
        let before = frames.bytes_held();
        assert_eq!(frames.push(scenario).await, Ok(Pushed::Repeated));
        assert_eq!(frames.push(frame(Tag::Keyframe, 10_000, 10)).await, Ok(Pushed::Taken), "segment 1 starts over");
        frames.push(frame(Tag::Delta, 10_000, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 20_000, 10)).await.unwrap();
        let tags = |segment| frames.segment(segment).unwrap().iter().map(|held| held.tag).collect::<Vec<_>>();
        assert_eq!(tags(1), [Tag::Declare, Tag::Keyframe, Tag::Delta, Tag::Delta], "its declaration kept");
        assert_eq!(frames.bytes_held(), before + frame(Tag::Delta, 20_000, 10).len() as u64);
        assert_eq!(
            tags_on_disk(&frames),
            [Tag::Scenario, Tag::Keyframe, Tag::Delta, Tag::Declare, Tag::Keyframe, Tag::Delta, Tag::Delta],
            "and on disk, once"
        );
    }

    #[tokio::test]
    async fn a_declaration_ahead_of_a_keyframe_opens_that_segment_and_one_behind_it_stays_in_it() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        frames.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        assert_eq!(frames.push(frame(Tag::Declare, 0, 10)).await, Ok(Pushed::Taken), "ahead of the first keyframe");
        frames.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Declare, 10_000, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 10_000, 10)).await.unwrap();
        let closed = frames.settle().expect("segment 0 closes");
        assert_eq!((closed.segment, closed.locked_sim_ms), (0, SimTime::ms(20_000)));
        frames.push(frame(Tag::Declare, 30_000, 10)).await.unwrap();
        assert!(frames.settle().is_none(), "a declaration alone does not reopen the closed segment");
        assert_eq!(frames.push(frame(Tag::Keyframe, 30_000, 10)).await, Ok(Pushed::Taken), "nothing to close");
        frames.push(frame(Tag::Delta, 30_000, 10)).await.unwrap();
        let tags = |segment: u32| -> Vec<Tag> { frames.segment(segment).unwrap().iter().map(|h| h.tag).collect() };
        assert_eq!(tags(0), [Tag::Declare, Tag::Keyframe, Tag::Delta, Tag::Declare, Tag::Delta]);
        assert_eq!(tags(1), [Tag::Declare, Tag::Keyframe, Tag::Delta]);
        let closed = frames.end().expect("the tail closes at the end");
        assert_eq!((closed.segment, closed.locked_sim_ms), (1, SimTime::ms(40_000)), "from the delta, not the declare");
    }

    /// The runner sends the end block last: it is taken once, into the last segment and onto the disk copy, the
    /// locked edge still the last slice's end; and nothing follows it but that segment sent again after a lost
    /// connection.
    #[tokio::test]
    async fn the_end_block_closes_the_last_segment_and_only_that_segment_sent_again_follows_it() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        frames.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        for (tag, at) in [(Tag::Keyframe, 0), (Tag::Delta, 0), (Tag::Delta, 10_000)] {
            frames.push(frame(tag, at, 10)).await.unwrap();
        }
        // dessim stamps the end block with the run's end, past the last slice's start.
        assert_eq!(frames.push(frame(Tag::End, 15_000, 10)).await, Ok(Pushed::Taken));
        let after = frames.push(frame(Tag::Delta, 20_000, 10)).await.expect_err("nothing after the end block");
        assert_eq!(after.refusal(), StreamRefusal::FrameOrder);
        let closed = frames.settle().expect("the segment with the end block closes");
        assert_eq!((closed.segment, closed.locked_sim_ms), (0, SimTime::ms(20_000)), "the last slice's end");
        let tags = || frames.segment(0).unwrap().iter().map(|held| held.tag).collect::<Vec<_>>();
        assert_eq!(tags(), [Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::End], "uploaded with the segment");
        // The connection was lost after the end block went: the open segment comes again, its end block with it.
        assert_eq!(frames.push(frame(Tag::Keyframe, 0, 10)).await, Ok(Pushed::Taken), "the segment starts over");
        for (tag, at) in [(Tag::Delta, 0), (Tag::Delta, 10_000), (Tag::End, 15_000)] {
            frames.push(frame(tag, at, 10)).await.unwrap();
        }
        assert_eq!(tags(), [Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::End], "held once");
        assert_eq!(tags_on_disk(&frames), [Tag::Scenario, Tag::Keyframe, Tag::Delta, Tag::Delta, Tag::End]);
    }

    /// A process that finds the live file an earlier one wrote for the attempt on this disk takes the attempt up as
    /// frames that stop short of the run's start, in memory and so on disk, whose copy is never vouched for whole.
    #[tokio::test]
    async fn an_attempt_taken_up_after_an_earlier_process_here_is_never_vouched_for_whole() {
        let dir = TempDir::new().unwrap();
        let (_earlier, frames) = frames_in(&dir, u64::MAX);
        frames.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        assert!(!frames.incomplete(), "taken from its start");
        let (store, again) = frames_in(&dir, u64::MAX);
        again.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        again.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        let InMemory::Held(snapshot, _) = again.attach() else { panic!("in memory") };
        assert!(snapshot.incomplete, "taken up after the earlier process");
        let mark = again.folder.join(WHOLE_MARK);
        store.close(&key(1));
        store.uploader().drain(&key(1)).await;
        assert!(matches!(again.attach(), InMemory::Released) && !mark.exists(), "released, never vouched for");
    }

    /// A run stopped before its first slice sends its end block with no keyframe ahead of it: the end block opens the
    /// only segment, the declarations ahead of it with it, which closes at the attempt's end at the instant the end
    /// block states; a delta before any keyframe is still out of order, and nothing follows the end block.
    #[tokio::test]
    async fn the_end_block_of_a_run_stopped_before_its_first_slice_is_its_only_segment() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        frames.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        let early = frames.push(frame(Tag::Delta, 0, 10)).await.expect_err("a delta before any keyframe");
        assert_eq!(early.refusal(), StreamRefusal::FrameOrder);
        frames.push(frame(Tag::Declare, 0, 10)).await.unwrap();
        assert_eq!(frames.push(frame(Tag::End, 4_000, 10)).await, Ok(Pushed::Taken));
        let after = frames.push(frame(Tag::Keyframe, 5_000, 10)).await.expect_err("nothing after the end block");
        assert_eq!(after.refusal(), StreamRefusal::FrameOrder);
        let closed = frames.end().expect("the end block's segment closes");
        assert_eq!((closed.segment, closed.locked_sim_ms), (0, SimTime::ms(4_000)));
        let held: Vec<Tag> = frames.segment(0).unwrap().iter().map(|held| held.tag).collect();
        assert_eq!(held, [Tag::Declare, Tag::End]);
        assert_eq!(tags_on_disk(&frames), [Tag::Scenario, Tag::Declare, Tag::End]);
    }

    /// A run stopped before its first slice has no keyframe, so after a lost connection the runner sends its scenario
    /// block and its end block again, the server having taken that end block or not: one it holds is passed over,
    /// once in memory and on disk, and another end block after it is still refused.
    #[tokio::test]
    async fn the_end_block_of_a_run_stopped_before_its_first_slice_sent_again_is_passed_over() {
        let dir = TempDir::new().unwrap();
        let (_store, frames) = frames_in(&dir, u64::MAX);
        let (scenario, end) = (frame(Tag::Scenario, 0, 10), frame(Tag::End, 4_000, 10));
        frames.push(scenario.clone()).await.unwrap();
        assert_eq!(frames.push(end.clone()).await, Ok(Pushed::Taken));
        let held = frames.bytes_held();
        assert_eq!(frames.push(scenario).await, Ok(Pushed::Repeated));
        assert_eq!(frames.push(end).await, Ok(Pushed::Repeated), "the end block it holds, sent again");
        assert_eq!(frames.bytes_held(), held, "held once");
        assert_eq!(tags_on_disk(&frames), [Tag::Scenario, Tag::End]);
        let other = frames.push(frame(Tag::End, 5_000, 10)).await.expect_err("an end block it does not hold");
        assert_eq!(other.refusal(), StreamRefusal::FrameOrder);
        let closed = frames.end().expect("the end block's segment closes at the end");
        assert_eq!((closed.segment, closed.locked_sim_ms), (0, SimTime::ms(4_000)));
    }

    #[tokio::test]
    async fn ending_closes_the_tail_and_releases_the_memory_when_nothing_is_pending() {
        let dir = TempDir::new().unwrap();
        let (store, frames) = frames_in(&dir, u64::MAX);
        frames.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 0, 10)).await.unwrap();
        let InMemory::Held(snapshot, mut live) = frames.attach() else { panic!("in memory") };
        assert_eq!(snapshot.frames.len(), 2);
        let closed = store.close(&key(1)).expect("the tail closes");
        assert_eq!((closed.segment, closed.locked_sim_ms), (0, SimTime::ms(10_000)));
        assert!(matches!(live.recv().await, Some(FrameEvent::Ended)));
        assert!(!store.is_open(&key(1)));
        store.uploader().drain(&key(1)).await;
        assert!(matches!(frames.attach(), InMemory::Released), "released once the upload queue drained");
        let refused = frames.push(frame(Tag::Delta, 10_000, 10)).await.expect_err("nothing after the end");
        assert_eq!(refused.refusal(), StreamRefusal::FrameOrder);
    }
}
