//! Within `provider`, `reading` is ONE READER'S VIEW OF AN ATTEMPT'S RECORDING: the frames its copy held when it was
//! opened (`history`), each with its tag, its instant and where its bytes rest, then each frame after them as the copy
//! grows, until the copy is whole: an open attempt's frames from the reader's own queue until it ends, however far
//! behind a burst leaves the reader, a fetch's as it publishes what has landed until it is done, nothing more for a
//! disk copy already whole. A fetch that fails and a copy that does not read are faults, never the copy's end. A frame
//! resting in a file is read only when asked for (`bytes`), so a reader that sends a few frames of a long copy reads
//! only those. Every scan and read of a file runs on a blocking thread holding the file, so a reader whose wait is
//! given up loses nothing it was about to take.
//!
//! TODO(overstuffed): three concerns share this file, each with its soundness argument: the reader's walk over its copy
//! (`Reading`, `Rest`), why a reading stops short (`Fault`), and the file held open on blocking threads (`OnDisk`). The
//! last two want child modules of their own.

use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io::{Error as IoError, Read, Seek, SeekFrom};
use std::iter::once;
use std::mem;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use desunits::SimTime;
use tokio::sync::mpsc::UnboundedReceiver as LiveReceiver;
use tokio::sync::watch::Receiver as ProgressReceiver;
use tokio::task::spawn_blocking;
use worldview::container::index::IndexRecord;
use worldview::{Error as WorldviewError, Reader, Tag};

use super::fetch::{Fetch, FetchFailure, FetchState};
use super::{Counted, Unreached};
use crate::frames::{FrameEvent, Held, Snapshot};

pub struct Reading {
    history: Vec<Stamped>,
    rest: Rest,
    /// The copy stops short of the run, whatever its last frame says: live segments whose uploads had not all landed,
    /// or frames a process took up after an earlier one.
    incomplete: bool,
}

/// One frame of a copy: its tag, its instant and where its bytes rest.
#[derive(Debug, Clone)]
pub struct Stamped {
    pub tag: Tag,
    pub at: SimTime,
    rests: Rests,
}

#[derive(Debug, Clone)]
enum Rests {
    Memory(Bytes),
    File(IndexRecord),
}

/// What comes after the history.
enum Rest {
    /// The reader's queue of an open attempt's frames, until the attempt ends.
    Live(LiveReceiver<FrameEvent>),
    /// A fetch's part file, as the fetch publishes what has landed: `taken` of its frames handed on, `ready` scanned
    /// and not yet handed on.
    Landing { disk: OnDisk, progress: ProgressReceiver<FetchState>, taken: usize, ready: VecDeque<IndexRecord> },
    /// A disk copy that is whole.
    Whole(OnDisk),
    /// An attempt in memory that has ended.
    Over,
}

/// Why a reading stopped short of its copy's end, or its copy on disk did not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The fetch from the store failed, in these words, the store or the disk not answering; what it wrote is not whole
    /// and is gone.
    Fetch(String),
    /// The disk did not answer as the copy was read, which another try may find it doing.
    Disk(String),
    /// The copy's bytes are not a recording: damaged, as on disk or as the store holds it, or not a `.worldview` at all.
    Unreadable(String),
    /// The open attempt's frames went away without its ending.
    Gone,
}

impl Fault {
    /// A failure of the worldview reader: the disk's when the reader's I/O failed, else the copy's.
    fn reading(fault: WorldviewError) -> Self {
        match fault {
            WorldviewError::Io { .. } => Self::Disk(fault.to_string()),
            fault => Self::Unreadable(fault.to_string()),
        }
    }

    fn disk(fault: IoError) -> Self {
        Self::Disk(fault.to_string())
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(why) => write!(f, "the attempt could not be fetched from the run-object store: {why}"),
            Self::Disk(why) => write!(f, "the disk did not answer as the attempt's copy was read: {why}"),
            Self::Unreadable(why) => write!(f, "the attempt's copy does not read: {why}"),
            Self::Gone => f.write_str("the attempt's frames went away before its ending"),
        }
    }
}

impl Stamped {
    fn held(held: Held) -> Self {
        Self { tag: held.tag, at: held.at, rests: Rests::Memory(held.bytes) }
    }

    fn filed(record: IndexRecord) -> Self {
        Self { tag: record.tag, at: SimTime::ms(record.t.0), rests: Rests::File(record) }
    }
}

impl Reading {
    pub(super) fn memory(snapshot: Snapshot, live: LiveReceiver<FrameEvent>) -> Self {
        let history = once(snapshot.scenario).chain(snapshot.frames).map(Stamped::held).collect();
        let rest = if snapshot.ended { Rest::Over } else { Rest::Live(live) };
        Self { history, rest, incomplete: snapshot.incomplete }
    }

    /// The frames the copy held when it was opened, in file order.
    pub fn history(&self) -> &[Stamped] {
        &self.history
    }

    /// Whether an open attempt's frames follow the history as they are taken.
    pub fn live(&self) -> bool {
        matches!(self.rest, Rest::Live(_))
    }

    /// Whether the copy ends short of the run's end, which `provider::ended_outcome` says in place of its ending.
    pub fn incomplete(&self) -> bool {
        self.incomplete
    }

    pub fn landing(&self) -> bool {
        matches!(self.rest, Rest::Landing { .. })
    }

    /// The bytes of `frames`, frames of this reading, in order: a frame in memory as it is held, one in a file read
    /// from it.
    pub async fn bytes(&self, frames: &[Stamped]) -> Result<Vec<Bytes>, Fault> {
        let filed: Vec<IndexRecord> = frames
            .iter()
            .filter_map(|frame| match frame.rests {
                Rests::File(record) => Some(record),
                Rests::Memory(_) => None,
            })
            .collect();
        let mut read = match (&self.rest, filed.is_empty()) {
            (_, true) => Vec::new(),
            (Rest::Landing { disk, .. } | Rest::Whole(disk), false) => disk.read(filed).await?,
            (Rest::Live(_) | Rest::Over, false) => {
                return Err(Fault::Unreadable("a frame of a file was asked of a copy in memory".to_owned()));
            }
        }
        .into_iter();
        frames
            .iter()
            .map(|frame| match &frame.rests {
                Rests::Memory(bytes) => Ok(bytes.clone()),
                Rests::File(_) => read.next().ok_or_else(|| Fault::Unreadable("a frame was not read".to_owned())),
            })
            .collect()
    }

    /// The next frame after the history and every frame `next` gave before: `Some` once the copy holds it, `None` once
    /// the copy is whole.
    pub async fn next(&mut self) -> Result<Option<Stamped>, Fault> {
        loop {
            match &mut self.rest {
                Rest::Whole(_) | Rest::Over => return Ok(None),
                Rest::Live(live) => {
                    return match live.recv().await {
                        Some(FrameEvent::Frame(held)) => Ok(Some(Stamped::held(held))),
                        Some(FrameEvent::Ended) => {
                            self.rest = Rest::Over;
                            Ok(None)
                        }
                        None => Err(Fault::Gone),
                    };
                }
                Rest::Landing { disk, progress, taken, ready } => {
                    if let Some(record) = ready.pop_front() {
                        *taken += 1;
                        return Ok(Some(Stamped::filed(record)));
                    }
                    let state = progress.borrow_and_update().clone();
                    match state {
                        FetchState::Failed(FetchFailure::Fault(why)) => return Err(Fault::Fetch(why)),
                        FetchState::Failed(FetchFailure::Damaged(why)) => return Err(Fault::Unreadable(why)),
                        _ => {}
                    }
                    ready.extend(disk.scanned_from(*taken).await?);
                    if !ready.is_empty() {
                        continue;
                    }
                    if state == FetchState::Done {
                        break;
                    }
                    if progress.changed().await.is_err() {
                        return Err(Fault::Fetch("the fetch stopped before it finished".to_owned()));
                    }
                }
            }
        }
        if let Rest::Landing { disk, .. } = mem::replace(&mut self.rest, Rest::Over) {
            self.rest = Rest::Whole(disk);
        }
        Ok(None)
    }

    /// Up to `at_most` of the frames `next` would give next that need no wait: those a burst left in an open attempt's
    /// queue, or those of a fetch's part file already scanned. A queue's ending is taken too, so `next` then says the
    /// copy is whole.
    pub fn ready(&mut self, at_most: usize) -> Vec<Stamped> {
        let mut queued = Vec::new();
        while queued.len() < at_most {
            match &mut self.rest {
                Rest::Live(live) => match live.try_recv() {
                    Ok(FrameEvent::Frame(held)) => queued.push(Stamped::held(held)),
                    Ok(FrameEvent::Ended) => self.rest = Rest::Over,
                    Err(_) => break,
                },
                Rest::Landing { taken, ready, .. } => match ready.pop_front() {
                    Some(record) => {
                        *taken += 1;
                        queued.push(Stamped::filed(record));
                    }
                    None => break,
                },
                Rest::Whole(_) | Rest::Over => break,
            }
        }
        queued
    }
}

/// A disk copy at `path` that is all on disk, every frame of it the history; `incomplete` when it is the live segments
/// of an attempt whose uploads never finished.
pub(super) async fn whole(path: &Path, counted: Counted, incomplete: bool) -> Result<Reading, Unreached> {
    let (disk, records) = OnDisk::open(path, counted).await.map_err(Unreached::Unopened)?;
    Ok(Reading { history: records.into_iter().map(Stamped::filed).collect(), rest: Rest::Whole(disk), incomplete })
}

/// `fetch`'s part file, the frames landed so far the history and the rest followed as they land.
pub(super) async fn landing(fetch: &Fetch, counted: Counted) -> Result<Reading, Unreached> {
    let (disk, records) = OnDisk::open(&fetch.part, counted).await.map_err(Unreached::Unopened)?;
    let taken = records.len();
    let rest = Rest::Landing { disk, progress: fetch.progress(), taken, ready: VecDeque::new() };
    Ok(Reading { history: records.into_iter().map(Stamped::filed).collect(), rest, incomplete: fetch.incomplete })
}

/// A copy on disk as one reading holds it open: a reader scanning its frames and a handle reading their bytes, behind
/// one lock the blocking threads take, and the reading's count on its folder.
struct OnDisk {
    open: Arc<Mutex<Open>>,
    _counted: Counted,
}

struct Open {
    scanned: Reader<File>,
    raw: File,
}

impl OnDisk {
    async fn open(path: &Path, counted: Counted) -> Result<(Self, Vec<IndexRecord>), Fault> {
        let path = path.to_owned();
        let opened = spawn_blocking(move || -> Result<(Open, Vec<IndexRecord>), Fault> {
            let scanned = Reader::open(&path).map_err(Fault::reading)?;
            let raw = File::open(&path).map_err(Fault::disk)?;
            let records = scanned.index().records().to_vec();
            Ok((Open { scanned, raw }, records))
        })
        .await
        .map_err(|fault| Fault::Unreadable(fault.to_string()))??;
        Ok((Self { open: Arc::new(Mutex::new(opened.0)), _counted: counted }, opened.1))
    }

    async fn read(&self, records: Vec<IndexRecord>) -> Result<Vec<Bytes>, Fault> {
        self.blocking(move |open| {
            records
                .iter()
                .map(|record| {
                    let mut bytes = vec![0; record.len as usize];
                    open.raw.seek(SeekFrom::Start(record.offset))?;
                    open.raw.read_exact(&mut bytes)?;
                    Ok(Bytes::from(bytes))
                })
                .collect::<Result<_, IoError>>()
                .map_err(Fault::disk)
        })
        .await
    }

    /// The frames the file holds whole from its `from`th on, the file scanned again for what grew.
    async fn scanned_from(&self, from: usize) -> Result<Vec<IndexRecord>, Fault> {
        self.blocking(move |open| {
            open.scanned.refresh().map_err(Fault::reading)?;
            Ok(open.scanned.index().records().get(from..).map(<[IndexRecord]>::to_vec).unwrap_or_default())
        })
        .await
    }

    /// `work` on a blocking thread holding the file. A thread that panicked or was cancelled took the copy with it,
    /// which reads as a copy that does not read.
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Open) -> Result<T, Fault> + Send + 'static,
    ) -> Result<T, Fault> {
        let open = self.open.clone();
        spawn_blocking(move || work(&mut open.lock().expect("a copy's file")))
            .await
            .map_err(|fault| Fault::Unreadable(fault.to_string()))?
    }
}
