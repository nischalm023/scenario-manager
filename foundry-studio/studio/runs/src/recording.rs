//! Within `crucible`, `recording` is A RUN'S RECORDING AS A FILE ON THIS
//! MACHINE: which attempt of a run has one, fetching it out of storage the
//! first time anyone asks for it, and what it says about its own extent.
//!
//! It exists because everything that serves a recording — an attachment's
//! reader, a series reply, the floor compile — wants a `.worldview` it can seek
//! around in, and storage hands over whole objects. The fetched copy lands
//! under the temporary folder at the key it is stored under, so the object and
//! the file have one name, and every later reader finds it already there.
//!
//! A recording is final, so a copy of it can only be right; what bounds the
//! folder is a ceiling rather than a delete on the way out. When a fetch takes
//! the copies past it, the runs nobody has read for longest go, whole — the
//! recording and the compiled floor beside it — so a reader never finds one
//! without the other.
//!
//! Two requests for the same uncached recording take turns rather than both
//! fetching: they arrive together whenever a browser asks to attach and for its
//! charts at once, and a run's recording is megabytes.

use std::collections::hash_map::Entry;
use std::fs::{File, create_dir_all, metadata, read_dir, remove_file, rename};
use std::io::{Error as IoError, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use runstore::layout::{AttemptNumber, RunId, recording_key};
use runstore::{RunStorage, StoreError};
use rustc_hash::FxHashMap;
use tokio::sync::Mutex as Turn;
use tracing::{debug, info, warn};
use worldview::{Error as WorldviewError, Reader, Tag};

use crate::floor::beside;
use crate::protocol::{RecordingExtent, RunSummary};
use crate::registry::{Registry, RegistryError};

/// How much of an object is held in memory at once while it is written to
/// disk. `runstore` hands over whole objects or byte ranges and no reader, so
/// a fetch asks for ranges: a four-hour recording is tens of megabytes and
/// none of it needs to be resident.
const CHUNK: u64 = 8 * 1024 * 1024;

/// One attempt's recording, on this machine.
#[derive(Debug, Clone)]
pub struct Held {
    /// Which attempt of the run this recording belongs to.
    pub attempt: AttemptNumber,
    /// Why the run stopped, in the simulator's own words.
    pub outcome: String,
    pub path: PathBuf,
}

/// Why a run's recording could not be put in front of a reader.
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("there is no run `{0}`")]
    NoSuchRun(String),
    #[error("run `{0}` has no recording yet: no attempt at it has ended")]
    NoRecording(String),
    #[error("{0}")]
    Registry(#[from] RegistryError),
    #[error("the recording could not be fetched: {0}")]
    Storage(#[from] StoreError),
    #[error("the recording could not be written to {path}: {source}")]
    Write { path: PathBuf, source: IoError },
    #[error("{0}")]
    Damaged(#[from] WorldviewError),
    #[error("the fetch did not finish: {0}")]
    Joined(String),
}

/// Where an attempt's recording sits while this machine holds a copy: the
/// storage key, under the temporary folder, so a file and the object it came
/// from (or became) have one name.
pub fn local_path(temp_dir: &Path, run: &RunId, attempt: AttemptNumber) -> PathBuf {
    temp_dir.join(recording_key(run, attempt))
}

/// What this machine holds: one turn per recording, so two requests for the
/// same uncached one fetch it once, and the ceiling the folder is kept under.
pub struct Cache {
    turns: Mutex<FxHashMap<PathBuf, Arc<Turn<()>>>>,
    ceiling_bytes: u64,
}

impl Cache {
    pub fn new(ceiling_bytes: u64) -> Self {
        Self { turns: Mutex::new(FxHashMap::default()), ceiling_bytes }
    }

    /// Put a run's recording on this machine and say where it is and how it
    /// ended.
    pub async fn hold(
        &self,
        registry: &Registry,
        storage: &dyn RunStorage,
        temp_dir: &Path,
        run: &str,
    ) -> Result<Held, RecordingError> {
        let summary = registry.summary(run).await?.ok_or_else(|| RecordingError::NoSuchRun(run.to_string()))?;
        let stored = stored_attempt(&summary).ok_or_else(|| RecordingError::NoRecording(run.to_string()))?;
        let id = RunId::new(run).map_err(|_| RecordingError::NoSuchRun(run.to_string()))?;
        let path = self.fetch(storage, temp_dir, &id, stored.attempt, stored.size_bytes).await?;
        Ok(Held { attempt: stored.attempt, outcome: stored.outcome, path })
    }

    /// Fetch one attempt's recording, or report the copy already here. Two
    /// callers asking together do it once: the second waits and then finds it.
    ///
    /// `expected` is the object's size as the registry wrote it down when the
    /// attempt ended. A copy here of exactly that size is the object, because
    /// an object is written whole under a key that names one attempt and is
    /// never appended to; so it is served without asking storage. Without a
    /// size to go on, storage is asked what it holds.
    pub async fn fetch(
        &self,
        storage: &dyn RunStorage,
        temp_dir: &Path,
        run: &RunId,
        attempt: AttemptNumber,
        expected: Option<u64>,
    ) -> Result<PathBuf, RecordingError> {
        let key = recording_key(run, attempt);
        let path = local_path(temp_dir, run, attempt);
        let turn = self.turn(&path);
        let _held = turn.lock().await;
        let held = metadata(&path).ok().map(|copy| copy.len());
        if held.is_some() && held == expected {
            debug!(key = %key, "served from the copy here, at the size the registry holds");
            return Ok(path);
        }
        let stored = storage.head(&key).await?;
        if held == Some(stored.size) {
            debug!(key = %key, "served from the copy here, at the size storage holds");
            return Ok(path);
        }
        let started = Instant::now();
        stream_to_disk(storage, &key, stored.size, &path).await?;
        info!(key = %key, bytes = stored.size, wall_ms = started.elapsed().as_millis() as u64, "fetched from storage");
        under_ceiling(temp_dir, self.ceiling_bytes, &path);
        Ok(path)
    }

    fn turn(&self, path: &Path) -> Arc<Turn<()>> {
        let mut turns = self.turns.lock().expect("no fetch panics holding this");
        match turns.entry(path.to_path_buf()) {
            Entry::Occupied(held) => Arc::clone(held.get()),
            Entry::Vacant(free) => Arc::clone(free.insert(Arc::new(Turn::new(())))),
        }
    }
}

/// The attempt a browser should read, and what the registry says of its
/// recording: the size it was stored at, when the ending wrote one down.
struct Stored {
    attempt: AttemptNumber,
    outcome: String,
    size_bytes: Option<u64>,
}

/// The attempt of this run a browser should read: the last one that stored a
/// recording at all, because a re-run supersedes what came before it. Anyone
/// wanting an earlier one has every attempt from `status`.
fn stored_attempt(summary: &RunSummary) -> Option<Stored> {
    let attempt = summary.attempts.iter().rev().find(|attempt| attempt.object_key.is_some())?;
    let outcome = attempt.stop_reason.clone().unwrap_or_else(|| attempt.state.as_str().to_string());
    Some(Stored {
        attempt: AttemptNumber::new(attempt.n).ok()?,
        outcome,
        size_bytes: attempt.recording.map(|held| held.size_bytes),
    })
}

/// Write a stored object to `path`, a range at a time, and publish it by
/// rename — so a reader opening the key while a fetch is under way sees the
/// whole file or no file, and never a prefix that is neither.
///
/// A copy already at that path is trusted when it is the size storage says the
/// object is: an object is written whole and never appended to, so the same key
/// at the same size is the same bytes.
async fn stream_to_disk(storage: &dyn RunStorage, key: &str, size: u64, path: &Path) -> Result<(), RecordingError> {
    let parent = path.parent().unwrap_or(path).to_path_buf();
    let write = |path: PathBuf, source| RecordingError::Write { path, source };
    let arriving = parent.join(format!(".arriving.{}.{}", std::process::id(), name_of(path)));
    let made = tokio::task::spawn_blocking({
        let (parent, arriving) = (parent.clone(), arriving.clone());
        move || create_dir_all(&parent).and_then(|()| File::create(&arriving))
    })
    .await
    .map_err(|joined| RecordingError::Joined(joined.to_string()))?;
    let mut file = made.map_err(|source| write(arriving.clone(), source))?;

    let mut at = 0u64;
    while at < size {
        let bytes = storage.get_range(key, at..(at + CHUNK).min(size)).await?;
        at += bytes.len() as u64;
        let (written, handed_back) = tokio::task::spawn_blocking(move || (file.write_all(&bytes), file))
            .await
            .map_err(|joined| RecordingError::Joined(joined.to_string()))?;
        file = handed_back;
        written.map_err(|source| write(arriving.clone(), source))?;
    }

    let published = tokio::task::spawn_blocking({
        let (arriving, path) = (arriving.clone(), path.to_path_buf());
        move || file.sync_all().and_then(|()| rename(&arriving, &path))
    })
    .await
    .map_err(|joined| RecordingError::Joined(joined.to_string()))?;
    if published.is_err() {
        let _ = remove_file(&arriving);
    }
    published.map_err(|source| write(path.to_path_buf(), source))
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().to_string()
}

/// Fill in what the run's recording holds, where the registry cannot say it.
///
/// An attempt's ending writes its extent down, so this answers for the one
/// attempt that has no ending of its own: an interrupted run whose object a
/// restart found and reconciled. Nothing is said when there is no recording
/// yet, and a recording that cannot be read leaves the attempt's own facts —
/// its state, its progress, why it stopped — standing, with a line in the log
/// saying why the extent is not there.
///
/// `status` calls this and `list` never does: `list` is asked once a second by
/// every open browser, and fetching an object per run to answer it is what the
/// stored extent exists to avoid.
pub async fn describe(cache: &Cache, storage: &dyn RunStorage, temp_dir: &Path, summary: &mut RunSummary) {
    let Some(stored) = stored_attempt(summary) else { return };
    let attempt = stored.attempt;
    if summary.attempts.iter().any(|row| row.n == attempt.get() && row.recording.is_some()) {
        return;
    }
    // The registry has no size for this one, by definition; storage is asked.
    let read = match RunId::new(&summary.run) {
        Ok(id) => cache.fetch(storage, temp_dir, &id, attempt, None).await.and_then(|path| extent(&path)),
        Err(_) => return,
    };
    match read {
        Ok(held) => {
            if let Some(row) = summary.attempts.iter_mut().find(|row| row.n == attempt.get()) {
                row.recording = Some(held);
            }
        }
        Err(failure) => warn!(run = %summary.run, %failure, "the recording's extent could not be read"),
    }
}

/// What a recording holds, read from its own index: how far it goes, how many
/// frames it is, and how big the file is.
pub fn extent(path: &Path) -> Result<RecordingExtent, RecordingError> {
    let reader = Reader::open(path)?;
    let index = reader.index();
    Ok(RecordingExtent {
        live_edge_ms: index.live_edge().map_or(0, |edge| edge.0),
        // The index block describes the file rather than the run, so counting
        // it would answer a different question from the one a browser asked.
        frames: index.records().iter().filter(|record| record.tag != Tag::Index).count() as u64,
        size_bytes: std::fs::metadata(path).map(|held| held.len()).unwrap_or(0),
    })
}

/// Bring the folder back under its ceiling, oldest read first.
///
/// A run's recording and its compiled floor sit under that run's own folder, so
/// whatever removes the folder — this, or a restart clearing an interrupted
/// run — removes both together and never leaves one without the other.
///
/// A run goes whole — its recording and the compiled floor beside it — because
/// a floor without the recording it describes is of no use to anyone, and a
/// recording whose floor has gone is simply compiled again. The copy just
/// fetched is never the one evicted: whoever asked for it is about to read it.
pub fn under_ceiling(temp_dir: &Path, ceiling_bytes: u64, keeping: &Path) {
    let mut held = cached(&temp_dir.join("runs"));
    let mut total: u64 = held.iter().map(|copy| copy.bytes).sum();
    if total <= ceiling_bytes {
        return;
    }
    // Oldest read first. A filesystem that does not keep read times leaves
    // every copy at its write time, which is the next best order.
    held.sort_by_key(|copy| copy.last_read);
    for copy in held {
        if total <= ceiling_bytes {
            return;
        }
        if copy.recording == keeping {
            continue;
        }
        let freed = copy.bytes;
        let _ = remove_file(&copy.recording);
        let _ = remove_file(beside(&copy.recording));
        total = total.saturating_sub(freed);
        info!(path = %copy.recording.display(), freed, "a cached recording was evicted to stay under the ceiling");
    }
}

/// One cached recording: where it is, how big, and when it was last read.
struct Cached {
    recording: PathBuf,
    bytes: u64,
    last_read: SystemTime,
}

/// Every recording under the cache root, with the floor beside each counted
/// against the same run.
fn cached(runs: &Path) -> Vec<Cached> {
    let mut held = Vec::new();
    for recording in walk(runs) {
        if recording.extension().is_none_or(|kind| kind != "worldview") {
            continue;
        }
        let Ok(about) = metadata(&recording) else { continue };
        let scene = metadata(beside(&recording)).map(|about| about.len()).unwrap_or(0);
        let last_read = about.accessed().or_else(|_| about.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        held.push(Cached { recording, bytes: about.len() + scene, last_read });
    }
    held
}

fn walk(at: &Path) -> Vec<PathBuf> {
    let Ok(entries) = read_dir(at) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        match path.is_dir() {
            true => found.extend(walk(&path)),
            false => found.push(path),
        }
    }
    found
}
