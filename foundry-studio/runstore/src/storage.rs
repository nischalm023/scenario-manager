//! The interface the rest of the system holds run-output storage through.
//!
//! Relative to the crate: `runstore` is the run-output store; this module is the
//! contract, [`store`](crate::store) is the one implementation of it that talks
//! to real storage, and [`layout`](crate::layout) supplies the keys that cross
//! it.
//!
//! The contract exists so that nothing above it knows whether a run's bytes land
//! in a bucket or in a folder on a developer's disk. It is deliberately narrow:
//! the operations a broker actually performs, and no more. Everything in it
//! holds for both backends, so behaviour a caller relies on locally is behaviour
//! it still gets in production:
//!
//! - An object appears whole or not at all. A reader never sees a partly written
//!   object, and a listing never names one.
//! - There is no append. A written object is final; a later write under the same
//!   key replaces it whole, and [`RunStorage::create`] refuses even that.
//! - A listing is complete the moment a write returns: an object that has been
//!   put is in the next listing of its prefix.
//! - A listing comes back in lexical key order, which the run layout arranges to
//!   be the order chunks were produced.
//! - No key is a folder of another key. A bucket would hold both `a` and `a/b`,
//!   but the folder backend cannot — `a` would have to be a file and a directory
//!   at once. The layout never produces such a pair; a caller that writes one
//!   anyway gets [`StoreError::Backend`] from the folder and silent acceptance
//!   from a bucket, because catching it would cost a listing on every write.
//!
//! Keys are drawn from one alphabet, which this crate checks before a backend
//! sees them (see [`key`](crate::key)): a key is one or more segments separated
//! by single `/`, each segment non-empty, made of letters, digits, `.`, `_`, and
//! `-`, and never `.` or `..`; no leading or trailing `/`. A key outside that
//! shape is refused with [`StoreError::InvalidKey`] naming it as given, never
//! quietly rewritten into one that fits — the backends disagree about exactly
//! those keys, so refusing them is what keeps the two interchangeable. A prefix
//! is any leading substring of such a key, so it may be empty, may end in `/`,
//! and may stop part way through a segment.

use std::fmt::Debug;
use std::ops::Range;
use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::fs::read;

use crate::error::{StoreError, StoreResult};
use crate::layout::RUNS_PREFIX;

/// What storage knows about one object besides its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// The key the object is stored under.
    pub key: String,
    /// Size of the object in bytes.
    pub size: u64,
    /// The backend's entity tag for these exact bytes, where it has one. The
    /// local folder derives one from the file's metadata; a bucket returns the
    /// service's.
    pub etag: Option<String>,
    /// The backend's generation for this version of the object, where the
    /// backend versions objects at all. A bucket reports one; the local folder
    /// does not, and reports [`None`].
    pub generation: Option<String>,
}

/// Run-output storage: whole objects, addressed by key, that appear atomically
/// and are never appended to.
///
/// Implementors are held behind `Arc<dyn RunStorage>` by servers that share one
/// store across handlers, so the trait stays object-safe.
#[async_trait]
pub trait RunStorage: Debug + Send + Sync + 'static {
    /// Writes an object whole, replacing whatever was under the key.
    ///
    /// What comes back names the version just written — the size, and the
    /// backend's tag and generation for it — so a writer holds the token of its
    /// own write without a [`head`](RunStorage::head) that another writer could
    /// win in between.
    async fn put(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo>;

    /// Writes a file on disk as one object, replacing whatever was under the
    /// key, without holding the whole file in memory. There is no create-only
    /// form of this: a caller that must not clobber uses [`create`](RunStorage::create)
    /// and holds the bytes itself.
    ///
    /// This is how a recording goes up: it was written to disk by the run and
    /// may be hundreds of megabytes, and the worker uploading it shares the
    /// machine with every other run in flight. The object appears whole or not
    /// at all, as with [`put`](RunStorage::put). This default reads the file
    /// whole and calls `put`, which is what a test double can afford; the real
    /// store streams it up in parts.
    async fn put_file(&self, key: &str, path: &Path) -> StoreResult<ObjectInfo> {
        let bytes = read(path).await.map_err(|source| StoreError::Backend {
            operation: "put_file",
            key: key.to_string(),
            source: Box::new(source),
        })?;
        self.put(key, Bytes::from(bytes)).await
    }

    /// Proves the store can be listed before anything is served from it: the
    /// identity's token is good, the bucket or folder exists, and a listing
    /// under the runs prefix is allowed. Reading an object is not proved,
    /// because a store with nothing in it has nothing to read; the one role a
    /// deployment grants carries both rights, so a listing stands for both.
    ///
    /// A server calls this once at startup, so a deployment whose identity has
    /// no rights on its bucket stops with the reason in its log rather than
    /// accepting runs it cannot store. Write rights are not proved, because
    /// proving them means writing; the first upload is that test, and its
    /// failure names the key. This default lists the prefix whole, which a
    /// test double can afford; the real store asks for one page.
    async fn probe(&self) -> StoreResult<()> {
        self.list(RUNS_PREFIX).await.map(|_| ())
    }

    /// Writes an object whole, but only if the key is free; reports
    /// [`StoreError::AlreadyExists`] otherwise.
    ///
    /// This is the precondition a writer uses when a stale twin of itself may
    /// still be running: whoever gets there first owns the object, and the
    /// loser learns it lost instead of overwriting the winner. What comes back
    /// names the version just written, as it does for
    /// [`put`](RunStorage::put).
    async fn create(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo>;

    /// Reads an object whole.
    ///
    /// A key whose own segments run through an object — `<chunk key>/inner`,
    /// say — holds nothing, and reads as missing rather than as a failure.
    async fn get(&self, key: &str) -> StoreResult<Bytes>;

    /// Reads a byte range of an object.
    ///
    /// A range whose end is past the object returns the bytes that exist, so a
    /// reader chasing a writer gets a short read. A range whose start is at or
    /// past the end holds no bytes at all, and says so as
    /// [`StoreError::PastTheEnd`] carrying the object's size, which is how such
    /// a reader learns it has caught up rather than that storage failed. A range
    /// with nothing in it (`end` at or below `start`) returns empty bytes
    /// without asking storage anything.
    async fn get_range(&self, key: &str, range: Range<u64>) -> StoreResult<Bytes>;

    /// Lists every object whose key starts with `prefix`, in lexical key order.
    ///
    /// The prefix is matched as text rather than as a folder, so `runs/` names
    /// every run's objects, `runs` scans the whole store for keys starting with
    /// those four letters, and the empty prefix lists everything. The layout
    /// helpers hand back prefixes that already end in `/`.
    async fn list(&self, prefix: &str) -> StoreResult<Vec<ObjectInfo>>;

    /// Removes an object.
    ///
    /// Removing a key that holds no object succeeds — whether nothing was ever
    /// written there, or the key names a level of the layout rather than an
    /// object — so a cleanup that runs twice is not a failure the second time.
    async fn delete(&self, key: &str) -> StoreResult<()>;

    /// Reports an object's size and, where the backend has them, its entity tag
    /// and generation, without reading its bytes.
    async fn head(&self, key: &str) -> StoreResult<ObjectInfo>;

    /// Reports whether an object is stored under the key.
    async fn exists(&self, key: &str) -> StoreResult<bool> {
        match self.head(key).await {
            Ok(_) => Ok(true),
            Err(StoreError::NotFound { .. }) => Ok(false),
            Err(other) => Err(other),
        }
    }
}
