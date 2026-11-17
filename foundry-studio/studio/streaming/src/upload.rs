//! Within `studio-streaming`, `upload` is the second arm of the T: each attempt's locked picture put into the run-object
//! store as it arrives, in the order it arrived. One queue per attempt, worked by one task, so the scenario block goes
//! up before segment 0, segment 0 before segment 1; a failed put is tried again on a backoff from 1 s doubling to
//! 30 s of wall clock, without end while the server lives, and `attempt_streams` is told after each success what is
//! in the store (`scenario_key`, `segments_uploaded`, `locked_sim_ms`; migration 0023). The final recording is not
//! uploaded here: the runner sends it to the API's door, which stores it. A segment closed twice is put twice, the
//! second time with more frames; the object is replaced whole, so a reader never sees a torn one. The bytes are read
//! from the attempt's frames at upload time, so an ended attempt's last job, behind every upload, says in the row
//! that its uploads finished (`uploads_finished_at`, migration 0025) and only then releases the memory. A server that
//! dies first leaves the column empty, which is how a reader of the segments knows they stop short; so does a process
//! that took the attempt up after an earlier one, whose segments follow that one's and never pass for the whole run.

use std::num::TryFromIntError;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use desunits::SimTime;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, live_scenario_key, live_segment_key};
use runstore::{RunStorage, RunStore};
use rustc_hash::FxHashMap;
use sqlx::{Error as SqlError, PgPool, Row as SqlRow};
use studio_protocol::Ulid;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch::{Sender as PendingSender, channel as pending_channel};
use tokio::time::sleep;

use crate::frames::{AttemptFrames, ClosedSegment, Earlier};
use crate::streams::StreamKey;

/// The first wait after a failed put, and the longest (wall clock).
pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Where the uploads are written down: the database and the id of this server, which the row names.
#[derive(Clone)]
pub struct Rows {
    pub pool: PgPool,
    pub server: Ulid,
}

/// The upload queues of every attempt, shared by the runner sockets and the frame store.
#[derive(Clone)]
pub struct Uploader {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<dyn RunStorage>,
    /// `None` only in tests without a database: the objects go up and nothing is written down.
    rows: Option<Rows>,
    backoff: (Duration, Duration),
    queues: Mutex<FxHashMap<StreamKey, Queue>>,
}

struct Queue {
    jobs: UnboundedSender<Job>,
    /// How many jobs are queued or in flight.
    pending: PendingSender<usize>,
}

enum Job {
    Upload(Upload),
    /// The attempt ended and every upload ahead of this job has landed.
    Finished(Arc<AttemptFrames>),
}

/// An object to put into the store.
enum Upload {
    Scenario(Arc<AttemptFrames>),
    Segment(Arc<AttemptFrames>, ClosedSegment),
}

impl Uploader {
    pub fn new(store: Arc<dyn RunStorage>, rows: Option<Rows>) -> Self {
        let inner = Inner { store, rows, backoff: (BACKOFF_FIRST, BACKOFF_MAX), queues: Mutex::default() };
        Self { inner: Arc::new(inner) }
    }

    /// The backoff shortened, for tests.
    pub fn with_backoff(self, first: Duration, max: Duration) -> Self {
        let inner =
            Arc::try_unwrap(self.inner).unwrap_or_else(|_| panic!("the backoff is set before the uploader is shared"));
        Self { inner: Arc::new(Inner { backoff: (first, max), ..inner }) }
    }

    /// The folder backend over `objects`, no database, a 10 ms backoff: what the tests run on.
    pub fn for_tests(objects: &Path) -> Self {
        let store = RunStore::local_folder(objects).expect("a folder store over a folder that exists");
        Self::new(Arc::new(store), None).with_backoff(Duration::from_millis(10), Duration::from_millis(50))
    }

    pub fn storage(&self) -> Arc<dyn RunStorage> {
        self.inner.store.clone()
    }

    pub fn rows(&self) -> Option<&Rows> {
        self.inner.rows.as_ref()
    }

    pub fn scenario(&self, frames: &Arc<AttemptFrames>) {
        self.enqueue(&frames.key, Job::Upload(Upload::Scenario(frames.clone())));
    }

    pub fn segment(&self, frames: &Arc<AttemptFrames>, closed: ClosedSegment) {
        self.enqueue(&frames.key, Job::Upload(Upload::Segment(frames.clone(), closed)));
    }

    /// The ended attempt's last job: once every upload ahead of it has landed, the row says its uploads finished and
    /// the attempt's memory is released.
    pub fn finished(&self, frames: &Arc<AttemptFrames>) {
        self.enqueue(&frames.key, Job::Finished(frames.clone()));
    }

    /// Jobs queued or in flight for the attempt.
    pub fn pending(&self, key: &StreamKey) -> usize {
        self.inner.queues.lock().expect("the upload queues").get(key).map_or(0, |queue| *queue.pending.borrow())
    }

    /// Wait until the attempt's queue is empty.
    pub async fn drain(&self, key: &StreamKey) {
        let watched =
            self.inner.queues.lock().expect("the upload queues").get(key).map(|queue| queue.pending.subscribe());
        if let Some(mut pending) = watched {
            let _ = pending.wait_for(|pending| *pending == 0).await;
        }
    }

    /// Wait until every attempt's queue is empty: what the server does before it stops, within a bound.
    pub async fn drain_all(&self) {
        let keys: Vec<StreamKey> = self.inner.queues.lock().expect("the upload queues").keys().cloned().collect();
        for key in keys {
            self.drain(&key).await;
        }
    }

    /// A drained queue of a forgotten attempt goes; one still working stays until it is done.
    pub fn forget(&self, key: &StreamKey) {
        let mut queues = self.inner.queues.lock().expect("the upload queues");
        if queues.get(key).is_some_and(|queue| *queue.pending.borrow() == 0) {
            queues.remove(key);
        }
    }

    fn enqueue(&self, key: &StreamKey, job: Job) {
        let mut queues = self.inner.queues.lock().expect("the upload queues");
        let queue = queues.entry(key.clone()).or_insert_with(|| {
            let (jobs, incoming) = unbounded_channel();
            let (pending, _) = pending_channel(0);
            tokio::spawn(work(self.inner.clone(), key.clone(), incoming, pending.clone()));
            Queue { jobs, pending }
        });
        queue.pending.send_modify(|pending| *pending += 1);
        let _ = queue.jobs.send(job);
    }
}

/// One attempt's queue, worked in order until the sender is dropped.
async fn work(inner: Arc<Inner>, key: StreamKey, mut incoming: UnboundedReceiver<Job>, pending: PendingSender<usize>) {
    while let Some(job) = incoming.recv().await {
        match &job {
            Job::Upload(upload) => match object_key(&key, upload) {
                Ok(object) => {
                    until_put(&inner, &key, &object, upload).await;
                    if let Some(rows) = &inner.rows {
                        write_down(rows, &key, &object, upload).await;
                    }
                }
                Err(why) => eprintln!("studio-streaming: {}/{}: an upload is dropped: {why}", key.run, key.attempt.0),
            },
            // Released before the count drops, so whoever waits for the queue to drain finds the memory already gone.
            Job::Finished(frames) => {
                if let Some(rows) = &inner.rows
                    && !frames.incomplete()
                {
                    write_finished(rows, &key, frames.segments()).await;
                }
                frames.release().await;
            }
        }
        pending.send_modify(|pending| *pending = pending.saturating_sub(1));
    }
}

fn object_key(key: &StreamKey, upload: &Upload) -> Result<String, String> {
    let run = RunId::new(key.run.as_str()).map_err(|fault| fault.to_string())?;
    let attempt = StoredAttempt::new(key.attempt.0).map_err(|fault| fault.to_string())?;
    Ok(match upload {
        Upload::Scenario(_) => live_scenario_key(&run, attempt),
        Upload::Segment(_, closed) => live_segment_key(&run, attempt, closed.segment),
    })
}

/// The put, tried until it lands.
async fn until_put(inner: &Inner, key: &StreamKey, object: &str, upload: &Upload) {
    let (mut wait, longest) = inner.backoff;
    let mut tries = 0u32;
    loop {
        tries += 1;
        let outcome = match upload {
            Upload::Scenario(frames) => match frames.scenario() {
                Some(bytes) => inner.store.put(object, bytes).await.map(|_| ()),
                None => {
                    return eprintln!("studio-streaming: {}/{}: no scenario block to upload", key.run, key.attempt.0);
                }
            },
            Upload::Segment(frames, closed) => match frames.segment(closed.segment) {
                Some(held) => {
                    let mut whole = Vec::with_capacity(held.iter().map(|held| held.bytes.len()).sum());
                    held.iter().for_each(|held| whole.extend_from_slice(&held.bytes));
                    inner.store.put(object, Bytes::from(whole)).await.map(|_| ())
                }
                None => {
                    return eprintln!(
                        "studio-streaming: {}/{}: segment {} is gone",
                        key.run, key.attempt.0, closed.segment
                    );
                }
            },
        };
        match outcome {
            Ok(()) if tries > 1 => {
                return eprintln!("studio-streaming: {}/{}: {object} landed on try {tries}", key.run, key.attempt.0);
            }
            Ok(()) => return,
            Err(fault) if tries == 1 => eprintln!(
                "studio-streaming: {}/{}: {object} did not land, trying again in {wait:?}: {fault}",
                key.run, key.attempt.0
            ),
            Err(_) => {}
        }
        sleep(wait).await;
        wait = (wait * 2).min(longest);
    }
}

/// What the row says once the object is in the store; a write that fails is said, not retried, since the object is up.
async fn write_down(rows: &Rows, key: &StreamKey, object: &str, upload: &Upload) {
    let run = key.run.as_str();
    let n = i32::try_from(key.attempt.0).unwrap_or(i32::MAX);
    let server = rows.server.as_str();
    let written = match upload {
        Upload::Scenario(_) => {
            sqlx::query(&upsert(", scenario_key", ", $4", "scenario_key = EXCLUDED.scenario_key"))
                .bind(run)
                .bind(n)
                .bind(server)
                .bind(object)
                .execute(&rows.pool)
                .await
        }
        Upload::Segment(_, closed) => {
            sqlx::query(&upsert(
                ", segments_uploaded, locked_sim_ms",
                ", $4, $5",
                "segments_uploaded = GREATEST(attempt_streams.segments_uploaded, EXCLUDED.segments_uploaded), \
                 locked_sim_ms = GREATEST(attempt_streams.locked_sim_ms, EXCLUDED.locked_sim_ms)",
            ))
            .bind(run)
            .bind(n)
            .bind(server)
            .bind(i32::try_from(closed.segment.saturating_add(1)).unwrap_or(i32::MAX))
            .bind(i64::try_from(closed.locked_sim_ms.as_ms()).unwrap_or(i64::MAX))
            .execute(&rows.pool)
            .await
        }
    };
    if let Err(fault) = written {
        eprintln!("studio-streaming: {run}/{n}: {object} is up but attempt_streams was not told: {fault}");
    }
}

/// What the row says an earlier process put in the store of the attempt's picture, which a process taking the attempt
/// up after it numbers its segments on from; `None` when no scenario block went up.
pub async fn uploaded(rows: &Rows, key: &StreamKey) -> Result<Option<Earlier>, SqlError> {
    let row = sqlx::query(
        "SELECT segments_uploaded, locked_sim_ms FROM attempt_streams \
         WHERE run_id = $1 AND n = $2 AND scenario_key IS NOT NULL",
    )
    .bind(key.run.as_str())
    .bind(i32::try_from(key.attempt.0).unwrap_or(i32::MAX))
    .fetch_optional(&rows.pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let decoded = |fault: TryFromIntError| SqlError::Decode(Box::new(fault));
    let segments = u32::try_from(row.try_get::<i32, _>("segments_uploaded")?).map_err(decoded)?;
    let locked = u64::try_from(row.try_get::<i64, _>("locked_sim_ms")?).map_err(decoded)?;
    Ok(Some(Earlier { segments, locked: SimTime::ms(locked) }))
}

/// What the row says once an ended attempt's every upload has landed: its uploads finished, and the `segments` it took
/// are all in the store, which a reader of the segments then trusts as the whole run. A write that fails is said, not
/// retried, and leaves the segments read as the incomplete recording they may be.
async fn write_finished(rows: &Rows, key: &StreamKey, segments: u32) {
    let run = key.run.as_str();
    let n = i32::try_from(key.attempt.0).unwrap_or(i32::MAX);
    let written = sqlx::query(&upsert(
        ", segments_uploaded, uploads_finished_at",
        ", $4, now()",
        "segments_uploaded = GREATEST(attempt_streams.segments_uploaded, EXCLUDED.segments_uploaded), \
         uploads_finished_at = EXCLUDED.uploads_finished_at",
    ))
    .bind(run)
    .bind(n)
    .bind(rows.server.as_str())
    .bind(i32::try_from(segments).unwrap_or(i32::MAX))
    .execute(&rows.pool)
    .await;
    if let Err(fault) = written {
        eprintln!("studio-streaming: {run}/{n}: the uploads finished but attempt_streams was not told: {fault}");
    }
}

/// An upsert of this server's columns on the attempt's `attempt_streams` row, `$1` the run, `$2` the attempt and `$3`
/// this server: the row made at revision 0 with `columns` set to `values` when the stats writer has not made it yet
/// (its first write then takes the row over, its revision being greater), else `set` applied to the row that is
/// there. `columns` and `values` each start with a comma.
pub fn upsert(columns: &str, values: &str, set: &str) -> String {
    format!(
        "INSERT INTO attempt_streams (run_id, n, server_id, revision, opened_at, sim_ms, progress_values{columns}) \
         VALUES ($1, $2, $3, 0, now(), 0, '[]'{values}) \
         ON CONFLICT (run_id, n) DO UPDATE SET {set}"
    )
}

#[cfg(test)]
mod tests {
    use runstore::layout::{AttemptNumber as StoredAttempt, RunId, live_scenario_key, live_segment_key};
    use tempfile::TempDir;
    use worldview::Tag;

    use crate::frames::tests::{frame, frames_in, key};

    #[tokio::test]
    async fn the_scenario_and_each_closed_segment_go_up_in_order_and_a_regrown_segment_is_replaced() {
        let dir = TempDir::new().unwrap();
        let (store, frames) = frames_in(&dir, u64::MAX);
        let uploader = store.uploader().clone();
        frames.push(frame(Tag::Scenario, 0, 10)).await.unwrap();
        uploader.scenario(&frames);
        frames.push(frame(Tag::Keyframe, 0, 10)).await.unwrap();
        frames.push(frame(Tag::Delta, 0, 10)).await.unwrap();
        let first = frames.settle().expect("segment 0 closes");
        uploader.segment(&frames, first);
        uploader.drain(&key(1)).await;
        let run = RunId::new(key(1).run.as_str()).unwrap();
        let objects = uploader.storage();
        let seg0 = live_segment_key(&run, StoredAttempt::FIRST, 0);
        let short = objects.get(&seg0).await.expect("segment 0 is up").len();
        assert!(objects.get(&live_scenario_key(&run, StoredAttempt::FIRST)).await.is_ok(), "the scenario is up");

        frames.push(frame(Tag::Delta, 10_000, 10)).await.unwrap();
        let again = frames.settle().expect("segment 0 grew, so it closes again");
        assert_eq!(again.segment, 0);
        uploader.segment(&frames, again);
        uploader.drain(&key(1)).await;
        assert!(objects.get(&seg0).await.unwrap().len() > short, "the object is replaced with the longer segment");
        assert_eq!(uploader.pending(&key(1)), 0);
    }
}
