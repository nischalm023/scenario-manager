//! Within `studio-streaming`, `stats` is each stream's top-level figures as `attempt_streams` keeps them (migration
//! 0022; ruling D174): how far its run's progress reached in simtime and its counters by name, when the stream opened
//! and when and by whom it closed. Every change a stream takes is offered here as a snapshot with a revision, and one
//! writer task writes the newest snapshot of each stream every 5 s of wall clock, at once when a stream closes. A
//! snapshot waits here until its write lands, whatever the streams forget meanwhile, and the row takes a write only
//! with a revision newer than its own, so an older snapshot never overwrites a newer one.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use desunits::SimTime;
use rustc_hash::FxHashMap;
use sqlx::types::Json;
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::{ProgressValue, Ulid, WallClock};
use tokio::select;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::streams::StreamKey;

/// How often the writer writes what changed (wall clock).
pub const WRITE_EVERY: Duration = Duration::from_secs(5);

/// One stream's figures at one revision.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub key: StreamKey,
    /// Drawn from the process's one counter, so a later snapshot of the stream has a greater one.
    pub revision: u64,
    pub opened_at: WallClock,
    pub sim_ms: SimTime,
    pub progress: Vec<ProgressValue>,
    pub closed: Option<Closed>,
}

/// When a stream closed, and who closed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed {
    pub at: WallClock,
    pub by: ClosedBy,
}

/// Who closed a stream: its runner, with its ending, or this server, its runner lost past the grace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedBy {
    Runner,
    Server,
}

impl ClosedBy {
    fn word(self) -> &'static str {
        match self {
            Self::Runner => "runner",
            Self::Server => "server",
        }
    }
}

/// The snapshots waiting to be written, shared by the streams that offer them and the task that writes them.
#[derive(Clone, Default)]
pub struct Stats(Arc<Waiting>);

#[derive(Default)]
struct Waiting {
    snapshots: Mutex<FxHashMap<StreamKey, Snapshot>>,
    hurry: Notify,
}

impl Stats {
    /// A stream's figures to be written, taking the place of an older waiting snapshot of it, never of a newer.
    pub fn offer(&self, snapshot: Snapshot) {
        let mut waiting = self.0.snapshots.lock().expect("the waiting snapshots");
        if waiting.get(&snapshot.key).is_none_or(|older| older.revision < snapshot.revision) {
            waiting.insert(snapshot.key.clone(), snapshot);
        }
    }

    /// A pass at once rather than at the writer's next one: a stream closed.
    pub fn hurry(&self) {
        self.0.hurry.notify_one();
    }

    /// The writer: every `every`, or when hurried, the waiting snapshots written as `server`'s.
    pub fn writer(&self, pool: PgPool, server: Ulid, every: Duration) -> JoinHandle<()> {
        let stats = self.clone();
        tokio::spawn(async move {
            loop {
                select! {
                    () = sleep(every) => {}
                    () = stats.0.hurry.notified() => {}
                }
                stats.write_waiting(&pool, &server).await;
            }
        })
    }

    /// Every waiting snapshot written once as `server`'s, each left waiting until its write lands, so a pass cut
    /// short (a stop aborting the writer) loses none; one whose write fails is said and waits for the next pass.
    pub async fn write_waiting(&self, pool: &PgPool, server: &Ulid) {
        let batch: Vec<Snapshot> = self.0.snapshots.lock().expect("the waiting snapshots").values().cloned().collect();
        for snapshot in batch {
            match write(pool, server, &snapshot).await {
                Ok(()) => self.written(&snapshot),
                Err(fault) => {
                    let StreamKey { run, attempt } = &snapshot.key;
                    eprintln!("studio-streaming: {run}/{}'s stats were not written, and wait: {fault}", attempt.0);
                }
            }
        }
    }

    /// `snapshot` landed: it no longer waits, unless a newer one of its stream came while it was written.
    fn written(&self, snapshot: &Snapshot) {
        let mut waiting = self.0.snapshots.lock().expect("the waiting snapshots");
        if waiting.get(&snapshot.key).is_some_and(|newest| newest.revision == snapshot.revision) {
            waiting.remove(&snapshot.key);
        }
    }
}

/// One snapshot written as `server`'s: the row inserted, or updated when this snapshot is newer than the row's own or
/// another server process wrote the row (a restarted server counts its revisions from one again).
pub async fn write(pool: &PgPool, server: &Ulid, snapshot: &Snapshot) -> Result<(), SqlxError> {
    sqlx::query(
        "INSERT INTO attempt_streams (run_id, n, server_id, revision, opened_at, sim_ms, progress_values, closed_at, \
             closed_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (run_id, n) DO UPDATE SET server_id = EXCLUDED.server_id, revision = EXCLUDED.revision, \
             written_at = now(), sim_ms = EXCLUDED.sim_ms, progress_values = EXCLUDED.progress_values, \
             closed_at = EXCLUDED.closed_at, closed_by = EXCLUDED.closed_by \
         WHERE attempt_streams.revision < EXCLUDED.revision OR attempt_streams.server_id <> EXCLUDED.server_id",
    )
    .bind(snapshot.key.run.as_str())
    .bind(i32::try_from(snapshot.key.attempt.0).unwrap_or(i32::MAX))
    .bind(server.as_str())
    .bind(i64::try_from(snapshot.revision).unwrap_or(i64::MAX))
    .bind(snapshot.opened_at.instant())
    .bind(i64::try_from(snapshot.sim_ms.as_ms()).unwrap_or(i64::MAX))
    .bind(Json(&snapshot.progress))
    .bind(snapshot.closed.map(|closed| closed.at.instant()))
    .bind(snapshot.closed.map(|closed| closed.by.word()))
    .execute(pool)
    .await?;
    Ok(())
}
