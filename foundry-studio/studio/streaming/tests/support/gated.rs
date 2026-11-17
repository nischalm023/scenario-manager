//! Within the streaming server's test support, `gated` is THE RUN-OBJECT STORE A TEST LETS READ ONE OBJECT AT A TIME:
//! the integration tests' cold fetches and the provider's own unit tests (`src/provider/tests.rs`, which takes this
//! file by path) share it, so both drive a fetch's interleavings through one store.

use std::fs::create_dir_all;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use runstore::{ObjectInfo, RunStorage, RunStore, StoreError, StoreResult};
use tokio::sync::Semaphore;

/// A folder store whose ranged reads each wait for a permit the test releases, and fail once `fail_from` of them have
/// been asked for, or panic, taking the reading task with them, once `panic_from` have: a cold fetch whose every object
/// lands when, and only when, the test says.
#[derive(Debug)]
pub struct Gated {
    pub inner: RunStore,
    permits: Semaphore,
    pub reads: AtomicUsize,
    pub fail_from: AtomicUsize,
    pub panic_from: AtomicUsize,
}

impl Gated {
    pub fn over(folder: &Path) -> Self {
        create_dir_all(folder).expect("the objects folder");
        Self {
            inner: RunStore::local_folder(folder).expect("a folder store"),
            permits: Semaphore::new(0),
            reads: AtomicUsize::new(0),
            fail_from: AtomicUsize::new(usize::MAX),
            panic_from: AtomicUsize::new(usize::MAX),
        }
    }

    /// `reads` more ranged reads may go.
    pub fn release(&self, reads: usize) {
        self.permits.add_permits(reads);
    }
}

#[async_trait]
impl RunStorage for Gated {
    async fn put(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        self.inner.put(key, bytes).await
    }

    async fn create(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        self.inner.create(key, bytes).await
    }

    async fn get(&self, key: &str) -> StoreResult<Bytes> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> StoreResult<Bytes> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        self.permits.acquire().await.expect("the test's permits").forget();
        assert!(read < self.panic_from.load(Ordering::SeqCst), "the test's store panics on read {read}");
        if read >= self.fail_from.load(Ordering::SeqCst) {
            return Err(StoreError::Config(format!("the test's store refused read {read}")));
        }
        self.inner.get_range(key, range).await
    }

    async fn list(&self, prefix: &str) -> StoreResult<Vec<ObjectInfo>> {
        self.inner.list(prefix).await
    }

    async fn delete(&self, key: &str) -> StoreResult<()> {
        self.inner.delete(key).await
    }

    async fn head(&self, key: &str) -> StoreResult<ObjectInfo> {
        self.inner.head(key).await
    }
}
