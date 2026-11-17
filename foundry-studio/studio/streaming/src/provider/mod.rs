//! Within `studio-streaming`, `provider` is THE ONE WAY TO AN ATTEMPT'S RECORDING: whatever serves an attempt's
//! frames, the picture on `/frames` and the metric streams on `/metrics` alike, asks it for a `Reading` and never looks
//! for a file itself. The first copy found is read: the attempt's frames in memory while it is open, its disk copy
//! once it has ended, else the run-object store, fetched onto disk once however many readers ask (`fetch`). Each
//! attempt's cache folder has one lock, under which a reading picks its copy, a fetch starts and later renames its
//! part file into place or removes it, and the sweeper deletes the folder: so every reader of a fetch, the one whose
//! request started it included, follows the one part file as the fetch publishes what has landed (`reading`), and no
//! reading opens a file that is being deleted or renamed. A reading counts on its folder until it is dropped, and the
//! sweeper leaves a counted folder alone. A disk copy is judged by its own word on whether it holds the whole run
//! (`cache::DiskCopy`), never by the tables as they stand when it is read: one that cannot be vouched for gives way to
//! a fetch of a whole one the tables name, the final recording or segments whose uploads all landed, and otherwise is
//! served as far as it goes and ended as incomplete, never as the run's own ending, so a reader never takes a
//! truncated history for the whole run. So are the frames in memory of an attempt this process took up after an
//! earlier one (`frames::Earlier`).
//!
//! TODO(overstuffed): what a copy's end says (`end_block_outcome`, `ended_outcome`), which `/frames` and the metric
//! hubs both ask once a reading is over, is no part of finding the copy and wants a module of its own beside `reading`.

mod fetch;
mod reading;
#[cfg(test)]
mod tests;

use std::fmt;
use std::io::Error as IoError;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use studio_metrics::run_outcome;
use studio_protocol::RunOutcome;
use tokio::fs::remove_dir_all;
use tokio::sync::Mutex as AsyncMutex;
use worldview::{End, Frame, Payload};

use crate::cache::{DiskCopy, Known, Source, disk_copy};
use crate::frames::{FrameStore, InMemory};
use crate::streams::StreamKey;

use fetch::Fetch;

pub use fetch::{FetchFailure, FetchState};
pub use reading::{Fault, Reading, Stamped};

/// The provider every viewer socket reads through, shared with the sweeper.
#[derive(Clone)]
pub struct Provider {
    inner: Arc<Inner>,
}

struct Inner {
    frames: FrameStore,
    /// Every attempt folder read or fetched into since the sweeper last found it idle.
    folders: Mutex<FxHashMap<StreamKey, Arc<Folder>>>,
}

/// One attempt's cache folder as the provider keeps it.
struct Folder {
    path: PathBuf,
    /// The newest fetch into the folder, behind the lock every reading, fetch and sweep takes to touch its files.
    fetch: AsyncMutex<Option<Fetch>>,
    /// How many readings read the folder's copy.
    readers: AtomicUsize,
}

/// A reading's count on its attempt's folder, from before it picks its copy until it is dropped.
struct Counted {
    folder: Arc<Folder>,
}

impl Counted {
    fn on(folder: &Arc<Folder>) -> Self {
        folder.readers.fetch_add(1, Ordering::AcqRel);
        Self { folder: folder.clone() }
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.folder.readers.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Why an attempt has no copy this server can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreached {
    /// The attempt is still going and has no picture yet: its runner has not sent its scenario block, here or to any
    /// server whose uploads the tables name.
    NotYet,
    /// The attempt has ended, or is no attempt the tables know, and no copy of it is anywhere this server can reach:
    /// in memory, on disk, or in the store.
    Nowhere,
    /// The database did not say where the store holds the attempt.
    Database(String),
    /// The cache folder could not be written to fetch the attempt into.
    Disk(String),
    /// The disk copy does not open: the disk did not answer, or the file is not a recording.
    Unopened(Fault),
}

impl fmt::Display for Unreached {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotYet => f.write_str("the attempt's runner has not sent its picture yet"),
            Self::Nowhere => f.write_str("this attempt's picture is nowhere this server can reach"),
            Self::Database(why) => write!(f, "the database did not say where the attempt is stored: {why}"),
            Self::Disk(why) => write!(f, "the attempt could not be fetched onto this server's disk: {why}"),
            Self::Unopened(fault) => fault.fmt(f),
        }
    }
}

impl Provider {
    pub fn new(frames: FrameStore) -> Self {
        Self { inner: Arc::new(Inner { frames, folders: Mutex::default() }) }
    }

    /// The attempt's recording for one reader, by the module doc's rule. `known` is what the tables say of the
    /// attempt: where the store holds it, which a fetch reads, and whether that copy is the whole run. A disk copy
    /// vouched for needs none of it; one that is not needs the tables to know whether a whole one is to be had.
    pub async fn open(&self, key: &StreamKey, known: Result<Option<Known>, String>) -> Result<Reading, Unreached> {
        if let Some(frames) = self.inner.frames.get(key) {
            match frames.attach() {
                InMemory::Held(snapshot, live) => return Ok(Reading::memory(snapshot, live)),
                InMemory::NotYet if !ended(&known) => return Err(Unreached::NotYet),
                InMemory::NotYet => return Err(Unreached::Nowhere),
                InMemory::Released => {}
            }
        }
        let folder = self.folder(key);
        let counted = Counted::on(&folder);
        let mut newest = folder.fetch.lock().await;
        if let Some(landing) = newest.as_ref().filter(|fetch| fetch.landing()) {
            return reading::landing(landing, counted).await;
        }
        let copy = disk_copy(&folder.path).await;
        if let Some(DiskCopy { path, whole: true }) = &copy {
            return reading::whole(path, counted, false).await;
        }
        let known = known.map_err(Unreached::Database)?;
        let source = known.as_ref().and_then(Known::source);
        if let Some(copy) = copy
            && !source.as_ref().is_some_and(Source::whole)
        {
            return reading::whole(&copy.path, counted, true).await;
        }
        let Some(source) = source else {
            return Err(if known.is_some_and(|known| !known.ended) { Unreached::NotYet } else { Unreached::Nowhere });
        };
        let started = Fetch::start(&self.inner.frames, key, &source, &folder).await?;
        let reading = reading::landing(&started, counted).await;
        *newest = Some(started);
        reading
    }

    /// Whether a reading or a fetch holds the attempt's folder, which the sweeper leaves alone.
    pub fn in_use(&self, key: &StreamKey) -> bool {
        let folder = self.inner.folders.lock().expect("the provider's folders").get(key).cloned();
        folder.is_some_and(|folder| {
            folder.readers.load(Ordering::Acquire) > 0
                || folder.fetch.try_lock().map_or(true, |newest| newest.as_ref().is_some_and(Fetch::landing))
        })
    }

    /// The attempt's folder deleted unless a reading or a fetch holds it by the time its lock is taken: whether it
    /// went. The lock is held through the deletion, so a reading arriving meanwhile finds the folder empty.
    pub async fn sweep(&self, key: &StreamKey) -> Result<bool, IoError> {
        let folder = self.folder(key);
        let Ok(newest) = folder.fetch.try_lock() else { return Ok(false) };
        if folder.readers.load(Ordering::Acquire) > 0 || newest.as_ref().is_some_and(Fetch::landing) {
            return Ok(false);
        }
        remove_dir_all(&folder.path).await?;
        Ok(true)
    }

    /// The folders nobody holds are forgotten, so the map keeps only the attempts being read or fetched.
    pub fn forget_idle(&self) {
        self.inner.folders.lock().expect("the provider's folders").retain(|_, folder| Arc::strong_count(folder) > 1);
    }

    fn folder(&self, key: &StreamKey) -> Arc<Folder> {
        let mut folders = self.inner.folders.lock().expect("the provider's folders");
        folders
            .entry(key.clone())
            .or_insert_with(|| {
                let path = self.inner.frames.folder(key);
                Arc::new(Folder { path, fetch: AsyncMutex::new(None), readers: AtomicUsize::new(0) })
            })
            .clone()
    }
}

/// How an end block, read off its frame's bytes, says the run ended.
pub fn end_block_outcome(bytes: &[u8]) -> Result<RunOutcome, String> {
    let (frame, _) = Frame::decode_at(bytes, 0).map_err(|fault| fault.to_string())?;
    let body = frame.contents(0).map_err(|fault| fault.to_string())?;
    let end = End::from_bytes(&body, 0).map_err(|fault| fault.to_string())?;
    run_outcome(&end.stop_reason)
        .ok_or_else(|| format!("the end block's stop reason `{}` is none this server knows", end.stop_reason))
}

fn ended(known: &Result<Option<Known>, String>) -> bool {
    known.as_ref().is_ok_and(|known| known.as_ref().is_some_and(|known| known.ended))
}

/// What `ended` says once a copy is read to its end, on `/frames` and `/metrics` alike: an incomplete copy says so in
/// place of whatever its last frame says, so a page never takes a truncated history for the whole run; a whole copy
/// says what its end block states, or that it carried none.
pub fn ended_outcome(reading: &Reading, end_block: Option<RunOutcome>) -> RunOutcome {
    match (reading.incomplete(), end_block) {
        (true, _) => RunOutcome::RecordingIncomplete,
        (false, Some(outcome)) => outcome,
        (false, None) => RunOutcome::NoEndBlock,
    }
}
