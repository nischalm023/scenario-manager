//! The ground a backend test stands on: one store, a run of its own to write
//! under, where that is on disk when there is a disk, and what the backend
//! leaves out of the service it stands in for.
//!
//! One contract suite runs over every backend a build can reach, so each test
//! takes a `Ground` rather than making a store. The folder is always there and
//! thrown away afterwards. A bucket is shared by every test at once, so each
//! ground writes under a run id nobody else has, asks nothing of the bucket as
//! a whole, and sweeps what it wrote when its test is over. Two buckets are
//! known: an emulator's, named by `RUNSTORE_GCS_EMULATOR`, which misses parts
//! of the service and says so; and a real one, named by `RUNSTORE_GCS_BUCKET`
//! alone, reached with the host's own credentials and missing nothing.

#![allow(dead_code)]

use std::fmt;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use runstore::layout::{AttemptNumber, RunId, attempt_prefix, run_prefix, spec_key};
use runstore::{RunStorage, RunStore};
use tempfile::TempDir;

/// Names the emulator a bucket ground talks to, as `http://host:port`.
pub const EMULATOR_VAR: &str = "RUNSTORE_GCS_EMULATOR";
/// The bucket to test against, which has to exist already: in the emulator
/// when one is named, otherwise a real one reached with the host's own
/// credentials (`gcloud auth application-default login`, or a deployment's
/// identity). With neither variable set, no bucket is tested and the suite
/// says so.
pub const BUCKET_VAR: &str = "RUNSTORE_GCS_BUCKET";
const DEFAULT_EMULATED_BUCKET: &str = "runstore-test";

static MINTED: AtomicU32 = AtomicU32::new(0);

/// A part of the service an emulator leaves out. A test of exactly that part
/// is skipped on such a ground, and says so, rather than failing for a reason
/// that is not the backend's. Neither gap exists against a real bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// `x-goog-if-generation-match: 0`, what `create` refuses with, is not
    /// honoured: a second write under the key goes through.
    /// <https://github.com/fsouza/fake-gcs-server/issues/994>
    CreatePrecondition,
    /// XML multipart uploads, what `put_file` streams a large file up with,
    /// are not implemented.
    /// <https://github.com/fsouza/fake-gcs-server/issues/852>
    Multipart,
    /// A write's response carries no `x-goog-generation` header, so the token
    /// a write hands back names no generation although a `head` of the same
    /// object does. The service sends it on every write.
    GenerationOnWrite,
}

pub struct Ground {
    pub store: RunStore,
    pub run: RunId,
    backend: &'static str,
    folder: Option<TempDir>,
    gaps: &'static [Gap],
}

impl Ground {
    /// A store over a folder that lives as long as the ground.
    pub fn folder() -> Self {
        let folder = TempDir::new().expect("a temporary folder");
        let store = RunStore::local_folder(folder.path()).expect("a store over a fresh folder");
        Self { store, run: fresh_run(), backend: "folder", folder: Some(folder), gaps: &[] }
    }

    /// A store over a bucket, or nothing when none is named. Only absent
    /// variables are a skip: a bucket that is named and cannot be used FAILS,
    /// because a suite that reports a cheerful skip when its bucket is
    /// unreachable is a suite that passes for the wrong reason.
    #[cfg(feature = "gcs")]
    pub fn bucket() -> Option<Self> {
        let emulator = std::env::var(EMULATOR_VAR).ok();
        let bucket = std::env::var(BUCKET_VAR).ok();
        match (emulator, bucket) {
            (Some(base_url), bucket) => {
                let bucket = bucket.unwrap_or_else(|| DEFAULT_EMULATED_BUCKET.to_string());
                let store = RunStore::emulated_bucket(&bucket, &base_url).unwrap_or_else(|failure| {
                    panic!("{EMULATOR_VAR} names an emulator this test cannot use: {failure}")
                });
                let gaps: &'static [Gap] = &[Gap::CreatePrecondition, Gap::Multipart, Gap::GenerationOnWrite];
                Some(Self { store, run: fresh_run(), backend: "emulated bucket", folder: None, gaps })
            }
            (None, Some(bucket)) => {
                let store = RunStore::bucket(&bucket)
                    .unwrap_or_else(|failure| panic!("{BUCKET_VAR} names a bucket this test cannot use: {failure}"));
                Some(Self { store, run: fresh_run(), backend: "real bucket", folder: None, gaps: &[] })
            }
            (None, None) => {
                eprintln!("no bucket is tested: neither {EMULATOR_VAR} nor {BUCKET_VAR} is set");
                None
            }
        }
    }

    /// Whether this ground lacks `gap`, for a test that asserts around it.
    pub fn lacks(&self, gap: Gap) -> bool {
        self.gaps.contains(&gap)
    }

    /// Whether this ground lacks `gap`, saying so on stderr when it does, so a
    /// test can return early in one line and the run shows what was not tried.
    pub fn skips(&self, gap: Gap) -> bool {
        let skipped = self.lacks(gap);
        if skipped {
            eprintln!("  skipped on the {}: it does not implement {gap:?}", self.backend);
        }
        skipped
    }

    /// Where the objects are on disk, for the assertions only a folder can be
    /// asked. A bucket has no disk.
    pub fn on_disk(&self) -> Option<&Path> {
        self.folder.as_ref().map(TempDir::path)
    }

    pub fn run_prefix(&self) -> String {
        run_prefix(&self.run)
    }

    pub fn spec_key(&self) -> String {
        spec_key(&self.run)
    }

    /// A level below an attempt's prefix, and the many objects under it that
    /// the listing promises need. The run layout puts one recording under an
    /// attempt, which is too few to show that a listing is lexical and
    /// prefix-exact, so the tests spell keys of their own inside the alphabet.
    pub fn parts_prefix(&self, attempt: AttemptNumber) -> String {
        format!("{}parts/", attempt_prefix(&self.run, attempt))
    }

    pub fn part_of(&self, attempt: AttemptNumber, sequence: u32) -> String {
        format!("{}{sequence:06}", self.parts_prefix(attempt))
    }

    pub fn part(&self, sequence: u32) -> String {
        self.part_of(AttemptNumber::FIRST, sequence)
    }

    /// Remove everything this ground's run wrote, so a bucket is left as it
    /// was found. The folder is thrown away whole regardless.
    pub async fn sweep(&self) {
        for object in self.store.list(&self.run_prefix()).await.expect("the store is readable") {
            self.store.delete(&object.key).await.expect("what a test wrote is deletable");
        }
    }
}

impl fmt::Display for Ground {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, run {}", self.backend, self.run)
    }
}

/// Every backend this build can test: the folder, and a bucket when the `gcs`
/// feature is on and one is named.
pub fn grounds() -> Vec<Ground> {
    let mut grounds = vec![Ground::folder()];
    #[cfg(feature = "gcs")]
    grounds.extend(Ground::bucket());
    grounds
}

/// A run id no other test has, in this process or any other sharing the
/// bucket: the process, the instant and a count, spelled in the key alphabet.
fn fresh_run() -> RunId {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.subsec_nanos()).unwrap_or(0);
    let id = format!("T{:X}-{:X}-{:X}", process::id(), nanos, MINTED.fetch_add(1, Ordering::Relaxed));
    RunId::new(id).expect("letters, digits and dashes are one key segment")
}
