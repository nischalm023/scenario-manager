//! Within `provider`, `fetch` is ONE ATTEMPT FETCHED FROM THE RUN-OBJECT STORE INTO ITS CACHE FOLDER: the final
//! recording when the API's `attempts.object_key` names one, else the scenario block and the uploaded segments in
//! order, copied in ranged parts into a part file made for this fetch, and renamed into place once it is all in and
//! ends whole, so a copy cut short in the store is never kept. The fetch is a task of its own, so no reader waits for
//! it and none ends it by going away, and it publishes what has landed after each part is on disk (`FetchState`),
//! which every reader of the fetch follows. A failure is published once the part file is removed, as the store's or
//! the disk's fault, which another fetch may not meet, or as a damaged copy, which another fetch would copy again, and
//! both final steps, the rename and the removal, are taken under the folder's lock, so a reading never opens a part
//! file that is going away and a fetch started after a failure never has its own part file removed by the one before.
//! A copy of the live segments says beside it whether they were the whole run when it was fetched (`cache::vouch`),
//! so a later reader judges it by what it was copied from. A task that dies, panicking or cancelled, publishes
//! nothing more, and its fetch counts as landing no longer.

use std::fmt::{Display, Formatter, Result as FmtResult};
use std::fs::metadata;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use runstore::RunStorage;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, live_segment_key};
use runstore::store::PART_BYTES;
use tokio::fs::{File, OpenOptions, create_dir_all, remove_file, rename};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch::{Receiver as ProgressReceiver, Sender as ProgressSender, channel as progress_channel};
use tokio::task::spawn_blocking;
use worldview::{Error as WorldviewError, Reader, Tag};

use super::{Folder, Unreached};
use crate::cache::{Source, mark_cached, unvouch, vouch};
use crate::frames::{FrameStore, LIVE_FILE, RECORDING_FILE};
use crate::streams::StreamKey;
use crate::upload::Rows;

/// How far a fetch has got, as its readers follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchState {
    /// This many bytes are in the part file, flushed.
    Landing(u64),
    /// Every object is in and the part file is renamed into place.
    Done,
    /// The fetch failed, and its part file is removed.
    Failed(FetchFailure),
}

/// Why a fetch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    /// The store or the disk did not answer, in these words: another fetch may find them answering.
    Fault(String),
    /// The copy the store holds is no whole recording, in these words: it does not read, or it ends inside a frame.
    /// Another fetch would copy the same bytes.
    Damaged(String),
}

impl Display for FetchFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Fault(why) => f.write_str(why),
            Self::Damaged(why) => write!(f, "the store's copy is damaged: {why}"),
        }
    }
}

/// One fetch as the folder keeps it: the part file it writes, what it publishes, and whether what it copies stops short
/// of the run's end, live segments whose uploads had not all landed.
pub(super) struct Fetch {
    pub(super) part: PathBuf,
    progress: ProgressReceiver<FetchState>,
    pub(super) incomplete: bool,
}

impl Fetch {
    /// Whether the fetch still lands frames: it published neither its end nor its failure, and its task lives. A task
    /// that died dropped its sender, so its last `Landing` holds the folder no longer.
    pub(super) fn landing(&self) -> bool {
        self.progress.has_changed().is_ok() && matches!(*self.progress.borrow(), FetchState::Landing(_))
    }

    pub(super) fn progress(&self) -> ProgressReceiver<FetchState> {
        self.progress.clone()
    }

    /// The fetch of `key` from `source` into `folder`, started: the part file made (a leftover of a fetch that died
    /// with its process removed first), then the copying spawned. The folder's lock is held by the caller.
    pub(super) async fn start(
        frames: &FrameStore,
        key: &StreamKey,
        source: &Source,
        folder: &Arc<Folder>,
    ) -> Result<Self, Unreached> {
        let disk = |what: &str, fault: IoError| Unreached::Disk(format!("{what}: {fault}"));
        let (name, objects) = objects(key, source)?;
        create_dir_all(&folder.path).await.map_err(|fault| disk("making the cache folder", fault))?;
        let part = folder.path.join(format!("{name}.part"));
        match remove_file(&part).await {
            Ok(()) => {}
            Err(fault) if fault.kind() == ErrorKind::NotFound => {}
            Err(fault) => return Err(disk("removing a leftover part file", fault)),
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .await
            .map_err(|fault| disk("making the part file", fault))?;
        let (published, progress) = progress_channel(FetchState::Landing(0));
        let job = Job {
            store: frames.uploader().storage(),
            rows: frames.uploader().rows().cloned(),
            key: key.clone(),
            objects,
            part: part.clone(),
            copy: folder.path.join(name),
            live_whole: matches!(source, Source::Live { .. }).then(|| source.whole()),
            folder: folder.clone(),
            published,
        };
        tokio::spawn(fetching(job, file));
        Ok(Self { part, progress, incomplete: !source.whole() })
    }
}

/// The file a fetch of `source` is renamed to, and the objects it is copied from, in order.
fn objects(key: &StreamKey, source: &Source) -> Result<(&'static str, Vec<String>), Unreached> {
    match source {
        Source::Recording(object) => Ok((RECORDING_FILE, vec![object.clone()])),
        Source::Live { scenario_key, segments, .. } => {
            // An attempt the store cannot spell a key for (attempt 0, say) has nothing there.
            let run = RunId::new(key.run.as_str()).map_err(|_| Unreached::Nowhere)?;
            let attempt = StoredAttempt::new(key.attempt.0).map_err(|_| Unreached::Nowhere)?;
            let mut objects = vec![scenario_key.clone()];
            objects.extend((0..*segments).map(|segment| live_segment_key(&run, attempt, segment)));
            Ok((LIVE_FILE, objects))
        }
    }
}

struct Job {
    store: Arc<dyn RunStorage>,
    rows: Option<Rows>,
    key: StreamKey,
    objects: Vec<String>,
    part: PathBuf,
    /// Where the part file is renamed to.
    copy: PathBuf,
    /// For a copy of the live segments, whether they hold the whole run, which the copy says beside it.
    live_whole: Option<bool>,
    folder: Arc<Folder>,
    published: ProgressSender<FetchState>,
}

async fn fetching(job: Job, mut file: File) {
    let copied = copy(job.store.as_ref(), &job.objects, &mut file, &job.published).await;
    let synced = match copied {
        Ok(()) => file.sync_data().await.map_err(|fault| format!("syncing the part file: {fault}")),
        Err(why) => Err(why),
    };
    drop(file);
    let synced = match synced {
        Ok(()) => ends_whole(&job.part).await,
        Err(why) => Err(FetchFailure::Fault(why)),
    };
    let locked = job.folder.fetch.lock().await;
    let ended = match synced {
        Ok(()) => placed(&job).await.map_err(FetchFailure::Fault),
        Err(why) => Err(why),
    };
    match ended {
        Ok(()) => {
            job.published.send_replace(FetchState::Done);
            drop(locked);
            if let Some(rows) = &job.rows {
                mark_cached(rows, &job.key).await;
            }
        }
        Err(why) => {
            eprintln!(
                "studio-streaming: {}/{}: the fetch from the store failed: {why}",
                job.key.run, job.key.attempt.0
            );
            if let Err(fault) = remove_file(&job.part).await
                && fault.kind() != ErrorKind::NotFound
            {
                eprintln!("studio-streaming: {} was not removed: {fault}", job.part.display());
            }
            job.published.send_replace(FetchState::Failed(why));
        }
    }
}

/// The part file renamed into place, a copy of the live segments saying beside it whether it holds the whole run: what
/// an earlier copy said is taken back first, so a failure leaves the copy unvouched, never wrongly vouched.
async fn placed(job: &Job) -> Result<(), String> {
    let said = |fault: IoError| format!("saying whether the copy is whole: {fault}");
    if job.live_whole.is_some() {
        unvouch(&job.folder.path).await.map_err(said)?;
    }
    rename(&job.part, &job.copy).await.map_err(|fault| format!("renaming the part file into place: {fault}"))?;
    if job.live_whole == Some(true) {
        vouch(&job.folder.path).await.map_err(said)?;
    }
    Ok(())
}

/// A fetched copy ends with its end block, or else exactly where its last whole frame ends; one that ends inside a
/// frame was cut short in the store, and one the reader does not open is no recording: either is damaged. A disk that
/// does not answer as the copy is read is the disk's fault.
async fn ends_whole(part: &Path) -> Result<(), FetchFailure> {
    let part = part.to_owned();
    spawn_blocking(move || {
        let reader = Reader::open(&part).map_err(|fault| match fault {
            WorldviewError::Io { .. } => FetchFailure::Fault(format!("reading the part file: {fault}")),
            fault => FetchFailure::Damaged(format!("the fetched copy does not read: {fault}")),
        })?;
        let records = reader.index().records();
        if records.last().is_some_and(|record| record.tag == Tag::End) {
            return Ok(());
        }
        let framed = records.last().map_or(0, |record| record.offset + u64::from(record.len));
        let measuring = |fault| FetchFailure::Fault(format!("measuring the part file: {fault}"));
        let length = metadata(&part).map_err(measuring)?.len();
        if framed == length {
            Ok(())
        } else {
            let past = length - framed;
            Err(FetchFailure::Damaged(format!(
                "the fetched copy ends inside a frame, {past} bytes past its last whole one"
            )))
        }
    })
    .await
    .map_err(|fault| FetchFailure::Fault(format!("checking the part file: {fault}")))?
}

/// Every object appended to `file` in parts, so a recording of hundreds of megabytes is never whole in memory, each
/// part flushed before it is published as landed.
async fn copy(
    store: &dyn RunStorage,
    objects: &[String],
    file: &mut File,
    published: &ProgressSender<FetchState>,
) -> Result<(), String> {
    let mut landed = 0u64;
    for object in objects {
        let size = store.head(object).await.map_err(|fault| format!("{object}: {fault}"))?.size;
        let mut at = 0u64;
        while at < size {
            let part = store
                .get_range(object, at..(at + PART_BYTES).min(size))
                .await
                .map_err(|fault| format!("{object}: {fault}"))?;
            if part.is_empty() {
                return Err(format!("{object}: the store answered no bytes at {at} of {size}"));
            }
            file.write_all(&part).await.map_err(|fault| format!("writing the part file: {fault}"))?;
            file.flush().await.map_err(|fault| format!("writing the part file: {fault}"))?;
            at += part.len() as u64;
            landed += part.len() as u64;
            published.send_replace(FetchState::Landing(landed));
        }
    }
    Ok(())
}
