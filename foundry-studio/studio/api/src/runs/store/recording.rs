//! Within `store`, `recording` is an attempt's one recording as the run store holds it, which exists so the attempt's
//! row never names bytes the store does not hold: a recording is taken only whole, one its run never closed (an empty
//! or cut-off body) refused; its bytes are the attempt's from the moment the first upload of them begins storing them,
//! so a retry of those bytes is answered the same receipt and other bytes are refused; and it is accepted on the
//! attempt's row, with its key, size, frames and reach in simtime, only while the attempt is queued or running, whether
//! or not a report follows. No ending written later clears it.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use desunits::SimTime;
use run_invocation::{AttemptKey, Fault};
use runstore::RunStorage;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, recording_key};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::RunState;
use thiserror::Error;
use tokio::task::spawn_blocking;
use worldview::Error as WorldviewError;
use worldview::container::recover::load;

use crate::db::{decode, read_word};

use super::{PgRunStore, n_of};

/// The recording an attempt accepted: its key in the run store, how far it reaches in simtime, its frames and its size,
/// as its row keeps them and the recording route answers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecording {
    pub key: String,
    pub live_edge: Option<SimTime>,
    pub frames: u64,
    pub bytes: u64,
}

/// Why a recording was not taken: its bytes are no whole recording; its attempt ended first, or took other bytes
/// first; or the API's own disk or the run store failed, which says nothing of the recording and may pass.
#[derive(Debug, Error)]
pub enum RecordingFault {
    #[error("stops before a recording's trailer, as an empty or cut-off one does")]
    Unclosed,
    #[error("does not read as a recording: {0}")]
    Unreadable(WorldviewError),
    #[error("came after its attempt ended")]
    AttemptEnded,
    #[error("is not the recording its attempt took first")]
    Differs,
    #[error("could not be stored: {0}")]
    Store(Fault),
}

impl From<SqlxError> for RecordingFault {
    fn from(fault: SqlxError) -> Self {
        Self::Store(fault.into())
    }
}

/// What a whole recording on this machine's disk says of itself: its bytes' SHA-256 and size, its frames and its reach.
struct Whole {
    sha256: String,
    bytes: u64,
    frames: u64,
    live_edge: Option<SimTime>,
}

impl PgRunStore {
    /// Whether a recording may still come for the attempt: while it is queued or running, or once it accepted one,
    /// whose bytes again are answered their receipt.
    pub async fn may_take_recording(&self, key: &AttemptKey) -> Result<bool, SqlxError> {
        let row = sqlx::query(
            "SELECT state IN ('queued', 'running') OR object_key IS NOT NULL FROM attempts WHERE run_id = $1 AND n = $2",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .fetch_optional(&self.app.pool)
        .await?;
        row.map_or(Ok(false), |row| row.try_get(0))
    }

    /// Whether the attempt still awaits its recording: queued or running, none accepted yet. Only then is a recording
    /// its runner left as a file worth reading whole and hashing; once the runner's own upload was accepted, or the
    /// attempt ended, the file changes nothing.
    pub(super) async fn awaits_recording(&self, key: &AttemptKey) -> Result<bool, SqlxError> {
        let row = sqlx::query(
            "SELECT state IN ('queued', 'running') AND object_key IS NULL FROM attempts WHERE run_id = $1 AND n = $2",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .fetch_optional(&self.app.pool)
        .await?;
        row.map_or(Ok(false), |row| row.try_get(0))
    }

    /// Take the recording at `path` as the attempt's: read whole, its bytes claimed, stored under the attempt's key and
    /// accepted on its row; a retry of bytes accepted already is answered their receipt and stores nothing.
    pub async fn take_recording(&self, key: &AttemptKey, path: &Path) -> Result<StoredRecording, RecordingFault> {
        let read = path.to_path_buf();
        let whole =
            spawn_blocking(move || whole(&read)).await.map_err(|fault| RecordingFault::Store(fault.into()))??;
        if let Some(accepted) = self.claim(key, &whole.sha256).await? {
            return Ok(accepted);
        }
        let object = object_key(key).map_err(RecordingFault::Store)?;
        self.app.runstore.put_file(&object, path).await.map_err(|fault| RecordingFault::Store(fault.into()))?;
        let stored =
            StoredRecording { key: object, live_edge: whole.live_edge, frames: whole.frames, bytes: whole.bytes };
        self.accept(key, stored).await
    }

    /// Claim the attempt's recording for the bytes `sha256` names, under its row's lock: their receipt when they were
    /// accepted already; none when this upload is to store them, from now on the only bytes the attempt takes.
    async fn claim(&self, key: &AttemptKey, sha256: &str) -> Result<Option<StoredRecording>, RecordingFault> {
        let mut tx = self.app.pool.begin().await?;
        let Some(row) = locked(&mut tx, key).await? else { return Err(RecordingFault::AttemptEnded) };
        let claimed: Option<String> = row.try_get("recording_sha256")?;
        match claimed.as_deref() {
            Some(claimed) if claimed != sha256 => return Err(RecordingFault::Differs),
            Some(_) => {
                if let Some(accepted) = receipt(&row)? {
                    return Ok(Some(accepted));
                }
            }
            None => {}
        }
        if read_word::<RunState>(&row.try_get::<String, _>("state")?)?.ended() {
            return Err(RecordingFault::AttemptEnded);
        }
        if claimed.is_none() {
            sqlx::query("UPDATE attempts SET recording_sha256 = $3 WHERE run_id = $1 AND n = $2")
                .bind(key.run.as_str())
                .bind(n_of(key))
                .bind(sha256)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(None)
    }

    /// Accept `stored`, its bytes the ones claimed, on the attempt's row under its lock, while the attempt is queued or
    /// running; an earlier try's receipt when it was accepted first. Once the attempt has ended nothing is accepted,
    /// and the object no row will name is removed.
    async fn accept(&self, key: &AttemptKey, stored: StoredRecording) -> Result<StoredRecording, RecordingFault> {
        let mut tx = self.app.pool.begin().await?;
        let Some(row) = locked(&mut tx, key).await? else { return Err(RecordingFault::AttemptEnded) };
        if let Some(accepted) = receipt(&row)? {
            return Ok(accepted);
        }
        if read_word::<RunState>(&row.try_get::<String, _>("state")?)?.ended() {
            drop(tx);
            if let Err(fault) = self.app.runstore.delete(&stored.key).await {
                eprintln!("studio-api: {key}'s recording, stored after its attempt ended, was not removed: {fault}");
            }
            return Err(RecordingFault::AttemptEnded);
        }
        sqlx::query(
            "UPDATE attempts SET object_key = $3, recording_live_edge_ms = $4, recording_frames = $5, \
                 recording_size_bytes = $6 \
             WHERE run_id = $1 AND n = $2",
        )
        .bind(key.run.as_str())
        .bind(n_of(key))
        .bind(&stored.key)
        .bind(stored.live_edge.map(|edge| saturating(edge.as_ms())))
        .bind(saturating(stored.frames))
        .bind(saturating(stored.bytes))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stored)
    }
}

/// The recording the attempt accepted, read under its row's lock inside `tx`; none when it accepted none.
pub(super) async fn accepted(
    tx: &mut Transaction<'_, Postgres>,
    key: &AttemptKey,
) -> Result<Option<StoredRecording>, SqlxError> {
    match locked(tx, key).await? {
        Some(row) => receipt(&row),
        None => Ok(None),
    }
}

/// The attempt's row as its recording reads it, locked inside `tx`; none when no attempt has that key.
async fn locked(tx: &mut Transaction<'_, Postgres>, key: &AttemptKey) -> Result<Option<PgRow>, SqlxError> {
    sqlx::query(
        "SELECT state, recording_sha256, object_key, recording_live_edge_ms, recording_frames, recording_size_bytes \
         FROM attempts WHERE run_id = $1 AND n = $2 FOR UPDATE",
    )
    .bind(key.run.as_str())
    .bind(n_of(key))
    .fetch_optional(&mut **tx)
    .await
}

/// The receipt a row keeps for the recording it accepted, none before one is; a count it keeps that is missing or
/// below zero does not decode.
fn receipt(row: &PgRow) -> Result<Option<StoredRecording>, SqlxError> {
    let Some(key) = row.try_get::<Option<String>, _>("object_key")? else { return Ok(None) };
    let count = |column: &str| -> Result<u64, SqlxError> {
        let kept: Option<i64> = row.try_get(column)?;
        kept.and_then(|kept| u64::try_from(kept).ok())
            .ok_or_else(|| decode(format!("{key}'s {column} is {kept:?}, which counts nothing")))
    };
    let live_edge = row.try_get::<Option<i64>, _>("recording_live_edge_ms")?;
    let live_edge = live_edge
        .map(|edge| u64::try_from(edge).map_err(|_| decode(format!("{key}'s live edge is {edge} ms, before simtime"))))
        .transpose()?;
    Ok(Some(StoredRecording {
        key: key.clone(),
        live_edge: live_edge.map(SimTime::ms),
        frames: count("recording_frames")?,
        bytes: count("recording_size_bytes")?,
    }))
}

/// The run store's key for the attempt's recording, the one `runstore`'s layout gives every attempt.
fn object_key(key: &AttemptKey) -> Result<String, Fault> {
    Ok(recording_key(&RunId::new(key.run.as_str())?, StoredAttempt::new(key.n.0)?))
}

/// The recording at `path` read whole: closed through its trailer, as every recording a run finishes is, its figures
/// read off its index, its bytes hashed. A fault reading this process's own file is the store's, never the recording's.
fn whole(path: &Path) -> Result<Whole, RecordingFault> {
    let loaded = load(path).map_err(|fault| match fault {
        WorldviewError::Io { .. } => RecordingFault::Store(fault.into()),
        fault => RecordingFault::Unreadable(fault),
    })?;
    if !loaded.through_trailer {
        return Err(RecordingFault::Unclosed);
    }
    let mut file = File::open(path).map_err(|fault| RecordingFault::Store(fault.into()))?;
    let (mut hash, mut chunk, mut bytes) = (Sha256::new(), vec![0; 1 << 20], 0_u64);
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                hash.update(&chunk[..read]);
                bytes += read as u64;
            }
            Err(fault) if fault.kind() == ErrorKind::Interrupted => {}
            Err(fault) => return Err(RecordingFault::Store(fault.into())),
        }
    }
    Ok(Whole {
        sha256: format!("{:x}", hash.finalize()),
        bytes,
        frames: loaded.index.len() as u64,
        live_edge: loaded.index.live_edge().map(|edge| SimTime::ms(edge.0)),
    })
}

/// A count as a column keeps it, at most `i64::MAX`.
fn saturating(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
