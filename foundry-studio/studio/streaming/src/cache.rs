//! Within `studio-streaming`, `cache` is the disk copy of each attempt's picture under `STUDIO_STREAMING_CACHE_DIR`,
//! one folder per attempt: which copy the folder holds and whether its writer vouched for it as the whole run
//! (`DiskCopy`); where the tables say the run-object store holds an attempt that is not here (the final recording the
//! API's `attempts.object_key` names, else the uploaded scenario block and segments), which the provider fetches it
//! from (`provider`); what the row says about the copy (`cached_at`, `last_opened_at`); and the sweeper that empties
//! the cache. The sweeper runs every `STUDIO_STREAMING_CACHE_SWEEP_SECS` of wall clock: it deletes an ended
//! attempt's folder once its last open (or its caching) is older than the TTL, then the oldest first while the cache's
//! total passes the size cap. Three things it never deletes: an open attempt's folder, a copy that may be the only
//! one, which is any attempt with an upload still queued or in flight on this server (a copy fetched from the store,
//! or one whose uploads all landed, has its twin there), and a folder a reading or a fetch holds. The ages are read
//! from the row where there is one, from the folder's own modification time where there is not.

use std::cmp::Reverse;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustc_hash::FxHashMap;
use sqlx::{Error as SqlError, PgPool, Row as SqlRow};
use studio_protocol::{AttemptNumber, Ulid};
use tokio::fs::{metadata, read_dir, remove_dir, remove_file, write};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::frames::{LIVE_FILE, RECORDING_FILE, WHOLE_MARK};
use crate::provider::Provider;
use crate::streams::{StreamKey, Streams};
use crate::upload::{Rows, upsert};

/// What the tables say of an attempt a viewer asks for: the API's `attempts.object_key` for the final recording, which
/// the runner sends to the API's door and the API stores, and `attempt_streams` for the live segments, when a server
/// streamed the attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Known {
    pub recording_key: Option<String>,
    pub scenario_key: Option<String>,
    pub segments_uploaded: u32,
    /// Whether the attempt ended and its streaming server's uploads all landed (`uploads_finished_at`): only then do
    /// the live segments hold the whole run.
    pub uploads_finished: bool,
    /// Whether the attempt has ended (`attempts.ended_at`): a picture nowhere to be found then never will be.
    pub ended: bool,
}

/// Where a cold attempt is fetched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The final recording, whole.
    Recording(String),
    /// The scenario block and segments `0..segments`, in order; the whole run once their uploads `finished`.
    Live { scenario_key: String, segments: u32, finished: bool },
}

impl Source {
    /// Whether a copy of the source holds the whole run.
    pub fn whole(&self) -> bool {
        match self {
            Self::Recording(_) => true,
            Self::Live { finished, .. } => *finished,
        }
    }
}

impl Known {
    pub fn source(&self) -> Option<Source> {
        if let Some(key) = &self.recording_key {
            return Some(Source::Recording(key.clone()));
        }
        let scenario_key = self.scenario_key.clone()?;
        Some(Source::Live { scenario_key, segments: self.segments_uploaded, finished: self.uploads_finished })
    }
}

/// What the tables say of the attempt, its `attempt_streams` row's `last_opened_at` set to the moment of asking where
/// it has one; `None` when there is no such attempt.
pub async fn opened(pool: &PgPool, key: &StreamKey) -> Result<Option<Known>, SqlError> {
    let row = sqlx::query(
        "WITH touched AS (UPDATE attempt_streams SET last_opened_at = now() WHERE run_id = $1 AND n = $2 \
                          RETURNING scenario_key, segments_uploaded, uploads_finished_at) \
         SELECT a.object_key AS recording_key, t.scenario_key, t.segments_uploaded, \
                t.uploads_finished_at IS NOT NULL AS uploads_finished, a.ended_at IS NOT NULL AS ended \
         FROM attempts a LEFT JOIN touched t ON true WHERE a.run_id = $1 AND a.n = $2",
    )
    .bind(key.run.as_str())
    .bind(i32::try_from(key.attempt.0).unwrap_or(i32::MAX))
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| Known {
        recording_key: row.get("recording_key"),
        scenario_key: row.get("scenario_key"),
        segments_uploaded: row
            .get::<Option<i32>, _>("segments_uploaded")
            .map_or(0, |segments| u32::try_from(segments).unwrap_or(0)),
        uploads_finished: row.get("uploads_finished"),
        ended: row.get("ended"),
    }))
}

/// The attempt's disk copy was written: `cached_at` says so.
pub async fn mark_cached(rows: &Rows, key: &StreamKey) {
    let written = sqlx::query(&upsert(", cached_at", ", now()", "cached_at = now()"))
        .bind(key.run.as_str())
        .bind(i32::try_from(key.attempt.0).unwrap_or(i32::MAX))
        .bind(rows.server.as_str())
        .execute(&rows.pool)
        .await;
    if let Err(fault) = written {
        eprintln!("studio-streaming: {}/{}: cached_at was not written: {fault}", key.run, key.attempt.0);
    }
}

/// The copy a replay of an attempt's folder reads, and whether it holds the whole run as the copy itself says, not as
/// the tables say when it is read: the final recording always does; the live file does once its writer vouched for it
/// with `WHOLE_MARK` beside it, which this server's stream does for an attempt it took from its scenario block to its
/// ending, and a fetch for segments whose uploads had all landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskCopy {
    pub path: PathBuf,
    pub whole: bool,
}

/// The folder's copy: the recording when it is there, else the live file; `None` for neither.
pub async fn disk_copy(folder: &Path) -> Option<DiskCopy> {
    let recording = folder.join(RECORDING_FILE);
    if is_file(&recording).await {
        return Some(DiskCopy { path: recording, whole: true });
    }
    let live = folder.join(LIVE_FILE);
    if !is_file(&live).await {
        return None;
    }
    Some(DiskCopy { path: live, whole: is_file(&folder.join(WHOLE_MARK)).await })
}

/// The folder's live file said to hold the whole run.
pub async fn vouch(folder: &Path) -> Result<(), IoError> {
    write(folder.join(WHOLE_MARK), b"").await
}

/// What the folder's live file was said to hold taken back, ahead of another copy taking its place.
pub async fn unvouch(folder: &Path) -> Result<(), IoError> {
    match remove_file(folder.join(WHOLE_MARK)).await {
        Err(fault) if fault.kind() != ErrorKind::NotFound => Err(fault),
        _ => Ok(()),
    }
}

async fn is_file(path: &Path) -> bool {
    metadata(path).await.is_ok_and(|meta| meta.is_file())
}

/// What the sweeper is told.
#[derive(Debug, Clone, Copy)]
pub struct Sweep {
    pub ttl: Duration,
    pub max_bytes: u64,
    pub every: Duration,
}

/// One attempt's folder as the sweeper sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copy {
    pub key: StreamKey,
    pub folder: PathBuf,
    pub bytes: u64,
    /// When it was last opened, else cached, else written.
    pub age_from: SystemTime,
    /// Whether the never-rules allow deleting it at all.
    pub deletable: bool,
}

/// Which copies go: every deletable one past the TTL, then the oldest deletable ones while the total passes the cap.
/// Indices into `copies`, in deletion order.
pub fn choose(copies: &[Copy], now: SystemTime, ttl: Duration, max_bytes: u64) -> Vec<usize> {
    let age = |copy: &Copy| now.duration_since(copy.age_from).unwrap_or_default();
    let mut total: u64 = copies.iter().map(|copy| copy.bytes).sum();
    let mut chosen = Vec::new();
    for (index, copy) in copies.iter().enumerate() {
        if copy.deletable && age(copy) >= ttl {
            chosen.push(index);
            total -= copy.bytes;
        }
    }
    let mut by_age: Vec<usize> =
        (0..copies.len()).filter(|index| copies[*index].deletable && !chosen.contains(index)).collect();
    by_age.sort_by_key(|index| Reverse(age(&copies[*index])));
    for index in by_age {
        if total <= max_bytes {
            break;
        }
        chosen.push(index);
        total -= copies[index].bytes;
    }
    chosen
}

/// The sweeper, every `sweep.every`; dropping the handle stops it.
pub fn sweeper(pool: PgPool, streams: Streams, provider: Provider, sweep: Sweep) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(sweep.every).await;
            sweep_once(&pool, &streams, &provider, sweep.ttl, sweep.max_bytes).await;
        }
    })
}

/// One pass: the folders measured, the rows read, the never-rules applied, the chosen folders deleted unless the
/// provider found them taken meanwhile, and their `cached_at` cleared. How many went.
pub async fn sweep_once(pool: &PgPool, streams: &Streams, provider: &Provider, ttl: Duration, max_bytes: u64) -> usize {
    let frames = streams.frames();
    let known = match rows_cached(pool).await {
        Ok(known) => known,
        Err(fault) => {
            eprintln!("studio-streaming: the sweeper could not read attempt_streams, so nothing is swept: {fault}");
            return 0;
        }
    };
    let mut copies = Vec::new();
    for (key, folder) in folders(frames.cache_dir()).await {
        let (bytes, written) = measure(&folder).await;
        let open = streams.is_open(&key) || frames.is_open(&key) || provider.in_use(&key);
        let only_copy = frames.uploading(&key);
        let age_from = known.get(&key).and_then(|row| row.age_from).unwrap_or(written);
        copies.push(Copy { key, folder, bytes, age_from, deletable: !open && !only_copy });
    }
    let mut deleted = 0;
    for index in choose(&copies, SystemTime::now(), ttl, max_bytes) {
        let copy = &copies[index];
        match provider.sweep(&copy.key).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(fault) => {
                eprintln!("studio-streaming: {} was not deleted: {fault}", copy.folder.display());
                continue;
            }
        }
        if let Some(run_folder) = copy.folder.parent() {
            let _ = remove_dir(run_folder).await;
        }
        let cleared = sqlx::query("UPDATE attempt_streams SET cached_at = NULL WHERE run_id = $1 AND n = $2")
            .bind(copy.key.run.as_str())
            .bind(i32::try_from(copy.key.attempt.0).unwrap_or(i32::MAX))
            .execute(pool)
            .await;
        if let Err(fault) = cleared {
            eprintln!("studio-streaming: {}/{}: cached_at was not cleared: {fault}", copy.key.run, copy.key.attempt.0);
        }
        deleted += 1;
    }
    provider.forget_idle();
    deleted
}

struct Cached {
    age_from: Option<SystemTime>,
}

/// Every row with a disk copy somewhere, by its key, with the wall-clock instant its age counts from.
async fn rows_cached(pool: &PgPool) -> Result<FxHashMap<StreamKey, Cached>, SqlError> {
    let rows = sqlx::query(
        "SELECT run_id, n, EXTRACT(EPOCH FROM COALESCE(last_opened_at, cached_at))::bigint AS age_epoch \
         FROM attempt_streams WHERE cached_at IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    let mut known = FxHashMap::default();
    for row in rows {
        let Ok(run) = row.get::<String, _>("run_id").parse() else { continue };
        let Ok(n) = u32::try_from(row.get::<i32, _>("n")) else { continue };
        let age_from = row
            .get::<Option<i64>, _>("age_epoch")
            .and_then(|epoch| u64::try_from(epoch).ok())
            .map(|epoch| UNIX_EPOCH + Duration::from_secs(epoch));
        known.insert(StreamKey { run, attempt: AttemptNumber(n) }, Cached { age_from });
    }
    Ok(known)
}

/// Every `<run>/<n>` folder under the cache, with its key; a folder not spelled as one is left alone.
pub async fn folders(cache_dir: &Path) -> Vec<(StreamKey, PathBuf)> {
    let mut found = Vec::new();
    let Ok(mut runs) = read_dir(cache_dir).await else { return found };
    while let Ok(Some(run_entry)) = runs.next_entry().await {
        let Ok(run) = run_entry.file_name().to_string_lossy().parse::<Ulid>() else { continue };
        let Ok(mut attempts) = read_dir(run_entry.path()).await else { continue };
        while let Ok(Some(attempt_entry)) = attempts.next_entry().await {
            let Ok(n) = attempt_entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            found.push((StreamKey { run: run.clone(), attempt: AttemptNumber(n) }, attempt_entry.path()));
        }
    }
    found.sort_by(|a, b| (a.0.run.as_str(), a.0.attempt.0).cmp(&(b.0.run.as_str(), b.0.attempt.0)));
    found
}

/// A folder's bytes and the newest modification time among its files.
async fn measure(folder: &Path) -> (u64, SystemTime) {
    let (mut bytes, mut newest) = (0, UNIX_EPOCH);
    if let Ok(mut entries) = read_dir(folder).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                bytes += meta.len();
                newest = newest.max(meta.modified().unwrap_or(UNIX_EPOCH));
            }
        }
    }
    (bytes, newest)
}

/// The cache's total, for `dev.sh --status` and the logs.
pub async fn total_bytes(cache_dir: &Path) -> u64 {
    let mut total = 0;
    for (_, folder) in folders(cache_dir).await {
        total += measure(&folder).await.0;
    }
    total
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use studio_protocol::AttemptNumber;

    use super::{Copy, choose};
    use crate::streams::StreamKey;

    fn copy(n: u32, bytes: u64, age_secs: u64, now: SystemTime, deletable: bool) -> Copy {
        Copy {
            key: StreamKey { run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(), attempt: AttemptNumber(n) },
            folder: PathBuf::from(format!("/cache/{n}")),
            bytes,
            age_from: now - Duration::from_secs(age_secs),
            deletable,
        }
    }

    #[test]
    fn the_ttl_goes_first_then_the_oldest_past_the_cap_and_the_never_rules_hold() {
        let now = SystemTime::now();
        let copies = [
            copy(1, 100, 10_000, now, true),
            copy(2, 100, 50, now, true),
            copy(3, 100, 500, now, true),
            copy(4, 100, 20_000, now, false),
            copy(5, 100, 900, now, true),
        ];
        let ttl = Duration::from_secs(3_600);
        assert_eq!(choose(&copies, now, ttl, 1_000), vec![0], "only the one past the TTL");
        assert_eq!(choose(&copies, now, ttl, 350), vec![0, 4], "then the oldest deletable until the total fits");
        assert_eq!(choose(&copies, now, ttl, 250), vec![0, 4, 2]);
        assert_eq!(choose(&copies, now, ttl, 0), vec![0, 4, 2, 1], "never the open or only copy, even past the cap");
    }
}
