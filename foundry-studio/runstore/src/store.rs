//! The store the binaries actually construct, and the choice of backend behind
//! it.
//!
//! Relative to the crate: `runstore` is the run-output store,
//! [`storage`](crate::storage) states what such a store promises, and this
//! module keeps that promise over real storage — a Google Cloud Storage bucket
//! in production, a folder on disk in development. The two are reached through
//! the `object_store` crate, so this module is the adapter that narrows its
//! general object interface to the one the broker holds, and the place where the
//! backend is picked from the environment.
//!
//! The folder backend is not a simplification of the bucket: it publishes every
//! object by writing a staging file and renaming it into place, so an object
//! appears whole or not at all and a listing never names a half-written one,
//! exactly as a bucket behaves. It also syncs the file and the directory entry
//! before a write returns, because a bucket acknowledges a write only once it is
//! durable, and a broker that advances its cursor on a write that has been
//! acknowledged must not lose the chunk to a crash. That is what lets
//! development run without a cloud credential and still exercise what production
//! depends on.
//!
//! One thing the folder cannot promise: `object_store` follows symbolic links,
//! including ones pointing outside the folder, so a link planted inside it sends
//! writes and deletes elsewhere. The folder is trusted the way a bucket's access
//! policy is trusted — the development runner creates it and nothing else writes
//! there.

use std::env::{self, VarError};
use std::fmt;
use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use std::ops::Range;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::TryStreamExt;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use object_store::{
    Error as BackendError, MultipartUpload, ObjectStore, ObjectStoreExt, PutMode, PutOptions, PutPayload, PutResult,
};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::error::{StoreError, StoreResult};
use crate::key::{validate_key, validate_prefix};
use crate::layout::RUNS_PREFIX;
use crate::storage::{ObjectInfo, RunStorage};

/// Environment variable naming the folder that stands in for the bucket in
/// development. The development runner creates the folder and exports this.
pub const STORAGE_DIR_VAR: &str = "CRUCIBLE_STORAGE_DIR";

/// Environment variable naming the Google Cloud Storage bucket that holds run
/// output in production.
pub const STORAGE_BUCKET_VAR: &str = "CRUCIBLE_STORAGE_BUCKET";

/// How much of a file is in memory at once while it goes up as parts, and the
/// most a file may be to go up as one request instead. A bucket wants every
/// part but the last at 5 MiB or more; this is comfortably over, and the same
/// size the server fetches a recording back in.
pub const PART_BYTES: u64 = 8 * 1024 * 1024;

/// Run-output storage over a real backend: a bucket in production, a folder on
/// disk in development.
///
/// Cloning is cheap; clones share one backend.
#[derive(Debug, Clone)]
pub struct RunStore {
    backend: Arc<dyn ObjectStore>,
    origin: Origin,
}

/// Where a [`RunStore`]'s objects live, kept for the sake of saying so in logs
/// and errors.
#[derive(Debug, Clone)]
enum Origin {
    LocalFolder(PathBuf),
    #[cfg(feature = "gcs")]
    Bucket(String),
}

impl RunStore {
    /// Builds the store the environment asks for: the folder named by
    /// `CRUCIBLE_STORAGE_DIR`, or the bucket named by
    /// `CRUCIBLE_STORAGE_BUCKET`.
    ///
    /// Exactly one of the two is expected. Setting neither, both, or either to
    /// an empty value is a configuration failure naming what to set, because
    /// guessing would send a run's output somewhere the operator did not ask
    /// for.
    ///
    /// Bucket credentials come from the environment the way Google's own
    /// libraries take them: a service-account key named by `GOOGLE_*`, or the
    /// application default credentials of the host.
    pub fn from_env() -> StoreResult<Self> {
        let dir = setting(STORAGE_DIR_VAR, env::var(STORAGE_DIR_VAR))?;
        let bucket = setting(STORAGE_BUCKET_VAR, env::var(STORAGE_BUCKET_VAR))?;
        match chosen_backend(dir, bucket)? {
            Chosen::Folder(dir) => Self::local_folder(FsPath::new(&dir)),
            Chosen::Bucket(bucket) => Self::bucket(&bucket),
        }
    }

    /// Builds a store over a folder that already exists. The development runner
    /// creates the folder; a missing one is a configuration failure rather than
    /// something to create here, so that a mistyped path does not quietly
    /// collect a run's output.
    pub fn local_folder(dir: &FsPath) -> StoreResult<Self> {
        let backend = LocalFileSystem::new_with_prefix(dir)
            .map_err(|source| {
                StoreError::Config(format!("{STORAGE_DIR_VAR} folder `{}` cannot be used: {source}", dir.display()))
            })?
            // A bucket acknowledges a write only when it is durable; the folder
            // says so too, at the cost of the sync.
            .with_fsync(true);
        Ok(Self { backend: Arc::new(backend), origin: Origin::LocalFolder(dir.to_path_buf()) })
    }

    /// Builds a store over a Google Cloud Storage bucket.
    #[cfg(feature = "gcs")]
    pub fn bucket(bucket: &str) -> StoreResult<Self> {
        use object_store::gcp::GoogleCloudStorageBuilder;

        let backend = GoogleCloudStorageBuilder::from_env().with_bucket_name(bucket).build().map_err(|source| {
            StoreError::Config(format!("{STORAGE_BUCKET_VAR} bucket `{bucket}` cannot be used: {source}"))
        })?;
        Ok(Self { backend: Arc::new(backend), origin: Origin::Bucket(bucket.to_string()) })
    }

    /// Reports that this build has no bucket backend.
    #[cfg(not(feature = "gcs"))]
    pub fn bucket(bucket: &str) -> StoreResult<Self> {
        Err(StoreError::Config(format!(
            "bucket `{bucket}` was asked for, but this binary was not built with the `gcs` feature"
        )))
    }

    /// Builds a store over a bucket served by a Google Cloud Storage emulator
    /// such as `fake-gcs-server`, at `base_url`, with no credential at all.
    ///
    /// This is how the bucket backend is tested without a cloud in reach;
    /// nothing deployed calls it. A service-account key is the one place
    /// `object_store` reads an alternative endpoint and the no-token flag
    /// from, so an otherwise empty one carries them.
    #[cfg(feature = "gcs")]
    pub fn emulated_bucket(bucket: &str, base_url: &str) -> StoreResult<Self> {
        use object_store::gcp::GoogleCloudStorageBuilder;
        use serde_json::json;

        let key = json!({
            "gcs_base_url": base_url,
            "disable_oauth": true,
            "client_email": "",
            "private_key_id": "",
            "private_key": "",
        })
        .to_string();
        let backend =
            GoogleCloudStorageBuilder::new().with_bucket_name(bucket).with_service_account_key(key).build().map_err(
                |source| {
                    StoreError::Config(format!("emulated bucket `{bucket}` at {base_url} cannot be used: {source}"))
                },
            )?;
        Ok(Self { backend: Arc::new(backend), origin: Origin::Bucket(bucket.to_string()) })
    }
}

impl fmt::Display for RunStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.origin {
            Origin::LocalFolder(dir) => write!(f, "local folder {}", dir.display()),
            #[cfg(feature = "gcs")]
            Origin::Bucket(bucket) => write!(f, "gs://{bucket}"),
        }
    }
}

#[async_trait]
impl RunStorage for RunStore {
    async fn put(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        let path = object_path(key)?;
        let size = bytes.len() as u64;
        let written = self
            .backend
            .put(&path, PutPayload::from_bytes(bytes))
            .await
            .map_err(|source| translate("put", key, source))?;
        Ok(written_object(key, size, written))
    }

    async fn put_file(&self, key: &str, path: &FsPath) -> StoreResult<ObjectInfo> {
        let object = object_path(key)?;
        let on_disk = |source: IoError| StoreError::Backend {
            operation: "put_file",
            key: key.to_string(),
            source: Box::new(source),
        };
        let mut file = File::open(path).await.map_err(on_disk)?;
        let size = file.metadata().await.map_err(on_disk)?.len();
        if size <= PART_BYTES {
            let mut whole = Vec::with_capacity(size as usize);
            file.read_to_end(&mut whole).await.map_err(on_disk)?;
            return self.put(key, Bytes::from(whole)).await;
        }

        // Parts go up one after another; the object exists only once `complete`
        // has been answered, and an upload that fails anywhere is aborted so a
        // bucket is not left holding parts it bills for and never lists.
        let mut upload =
            self.backend.put_multipart(&object).await.map_err(|source| translate("put_file", key, source))?;
        let mut sent = 0u64;
        while sent < size {
            let want = (size - sent).min(PART_BYTES) as usize;
            let mut part = vec![0u8; want];
            if let Err(source) = file.read_exact(&mut part).await {
                return Err(abandoned(upload, key, on_disk(source)).await);
            }
            if let Err(source) = upload.put_part(PutPayload::from(part)).await {
                return Err(abandoned(upload, key, translate("put_file", key, source)).await);
            }
            sent += want as u64;
        }
        if file.read(&mut [0u8]).await.map_err(on_disk)? != 0 {
            let grew = IoError::new(IoErrorKind::InvalidData, "the file grew while it was being uploaded");
            return Err(abandoned(upload, key, on_disk(grew)).await);
        }
        match upload.complete().await {
            Ok(written) => Ok(written_object(key, size, written)),
            Err(source) => Err(abandoned(upload, key, translate("put_file", key, source)).await),
        }
    }

    async fn probe(&self) -> StoreResult<()> {
        // One page of the listing, not the listing: a bucket of ten thousand
        // runs answers in one request, and a missing bucket or a refused token
        // fails on that same request.
        let scope = ObjectPath::from(RUNS_PREFIX.trim_end_matches('/'));
        let mut listing = self.backend.list(Some(&scope));
        listing.try_next().await.map(|_| ()).map_err(|source| translate("probe", RUNS_PREFIX, source))
    }

    async fn create(&self, key: &str, bytes: Bytes) -> StoreResult<ObjectInfo> {
        let path = object_path(key)?;
        let size = bytes.len() as u64;
        let options = PutOptions { mode: PutMode::Create, ..PutOptions::default() };
        let written = self
            .backend
            .put_opts(&path, PutPayload::from_bytes(bytes), options)
            .await
            .map_err(|source| translate("create", key, source))?;
        Ok(written_object(key, size, written))
    }

    async fn get(&self, key: &str) -> StoreResult<Bytes> {
        let path = object_path(key)?;
        let object = self.backend.get(&path).await.map_err(|source| translate("get", key, source))?;
        object.bytes().await.map_err(|source| translate("get", key, source))
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> StoreResult<Bytes> {
        let path = object_path(key)?;
        if range.end <= range.start {
            return Ok(Bytes::new());
        }
        let start = range.start;
        match self.backend.get_range(&path, range).await {
            Ok(bytes) => Ok(bytes),
            Err(source) => {
                // Each backend refuses a start past the end in its own untyped
                // way: the folder with an out-of-range error, the bucket with an
                // HTTP status. The object's size is what separates that from
                // storage failing, and a reader chasing a writer has to be able
                // to tell the two apart.
                if let Ok(meta) = self.backend.head(&path).await
                    && start >= meta.size
                {
                    return Err(StoreError::PastTheEnd { key: key.to_string(), start, size: meta.size });
                }
                Err(translate("get_range", key, source))
            }
        }
    }

    async fn list(&self, prefix: &str) -> StoreResult<Vec<ObjectInfo>> {
        validate_prefix(prefix).map_err(|source| StoreError::InvalidKey { key: prefix.to_string(), source })?;
        // The backend lists whole path segments, so it is asked for the deepest
        // folder the prefix names and the remainder is matched here. That keeps
        // a partial final segment meaning the same thing on both backends.
        let folder = match prefix.rfind('/') {
            Some(last) => &prefix[..last],
            None => "",
        };
        let scope = if folder.is_empty() { None } else { Some(ObjectPath::from(folder)) };

        let listed: Vec<_> = self
            .backend
            .list(scope.as_ref())
            .try_collect()
            .await
            .map_err(|source| translate("list", prefix, source))?;

        let mut objects: Vec<ObjectInfo> = listed
            .into_iter()
            .map(|meta| ObjectInfo {
                key: meta.location.as_ref().to_string(),
                size: meta.size,
                etag: meta.e_tag,
                generation: meta.version,
            })
            .filter(|object| object.key.starts_with(prefix))
            .collect();
        objects.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(objects)
    }

    async fn delete(&self, key: &str) -> StoreResult<()> {
        let path = object_path(key)?;
        match self.backend.delete(&path).await {
            Ok(()) | Err(BackendError::NotFound { .. }) => Ok(()),
            Err(source) => {
                // A key that holds no object is deletable. A bucket says so
                // outright; the folder refuses to unlink the directory carrying
                // the key's name, and refuses differently again when a segment
                // of the key is itself an object. Asking what the key holds
                // settles all three the same way.
                match self.head(key).await {
                    Err(StoreError::NotFound { .. }) => Ok(()),
                    _ => Err(translate("delete", key, source)),
                }
            }
        }
    }

    async fn head(&self, key: &str) -> StoreResult<ObjectInfo> {
        let path = object_path(key)?;
        let meta = self.backend.head(&path).await.map_err(|source| translate("head", key, source))?;
        Ok(ObjectInfo {
            key: meta.location.as_ref().to_string(),
            size: meta.size,
            etag: meta.e_tag,
            generation: meta.version,
        })
    }
}

/// Names the version a write just created: the key it was asked for, the size it
/// was handed, and whatever the backend now calls those bytes.
/// Abort an upload that failed, and say so if the abort failed too: parts a
/// bucket keeps after a failed abort are billed and listed by nothing, so the
/// one error that goes back names both failures rather than hiding the second.
async fn abandoned(mut upload: Box<dyn MultipartUpload>, key: &str, failure: StoreError) -> StoreError {
    match upload.abort().await {
        Ok(()) => failure,
        Err(abort) => StoreError::Backend {
            operation: "put_file",
            key: key.to_string(),
            source: format!("{failure}; the abort failed too, so parts may be left in the bucket: {abort}").into(),
        },
    }
}

fn written_object(key: &str, size: u64, written: PutResult) -> ObjectInfo {
    ObjectInfo { key: key.to_string(), size, etag: written.e_tag, generation: written.version }
}

/// Which backend the environment names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Chosen {
    /// The folder named by [`STORAGE_DIR_VAR`].
    Folder(String),
    /// The bucket named by [`STORAGE_BUCKET_VAR`].
    Bucket(String),
}

/// Reads what one variable holds, apart from the process it was read from: a
/// value, nothing, or a mistake worth naming.
///
/// An empty value is a mistake rather than an absence: an operator who wrote the
/// variable's name meant to name something with it, and silently falling through
/// to the other backend would send a run's output somewhere nobody asked for.
fn setting(name: &str, value: Result<String, VarError>) -> StoreResult<Option<String>> {
    match value {
        Ok(value) if value.trim().is_empty() => Err(StoreError::Config(format!("{name} is set but empty"))),
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(StoreError::Config(format!("{name} does not hold text"))),
    }
}

/// Decides which backend two settings name, refusing to guess when they name
/// none or both.
fn chosen_backend(dir: Option<String>, bucket: Option<String>) -> StoreResult<Chosen> {
    match (dir, bucket) {
        (Some(dir), None) => Ok(Chosen::Folder(dir)),
        (None, Some(bucket)) => Ok(Chosen::Bucket(bucket)),
        (Some(_), Some(_)) => Err(StoreError::Config(format!(
            "{STORAGE_DIR_VAR} and {STORAGE_BUCKET_VAR} are both set; set exactly one of them"
        ))),
        (None, None) => Err(StoreError::Config(format!(
            "set {STORAGE_DIR_VAR} to a folder for local development, \
             or {STORAGE_BUCKET_VAR} to a Google Cloud Storage bucket"
        ))),
    }
}

/// Reads a key and turns it into the backend's own path.
///
/// This is the one gate: no key reaches a backend without passing it. Every key
/// it lets through is a subset of what `object_store` addresses without
/// rewriting, so the path carries the caller's own spelling and a listing hands
/// that same spelling back.
fn object_path(key: &str) -> StoreResult<ObjectPath> {
    validate_key(key).map_err(|source| StoreError::InvalidKey { key: key.to_string(), source })?;
    Ok(ObjectPath::from(key))
}

/// Restates a backend failure in this crate's vocabulary, keeping the two
/// failures callers decide on apart from the rest.
fn translate(operation: &'static str, key: &str, source: BackendError) -> StoreError {
    match source {
        BackendError::NotFound { .. } => StoreError::NotFound { key: key.to_string() },
        BackendError::AlreadyExists { .. } => StoreError::AlreadyExists { key: key.to_string() },
        other if runs_through_an_object(&other) => StoreError::NotFound { key: key.to_string() },
        other => StoreError::Backend { operation, key: key.to_string(), source: Box::new(other) },
    }
}

/// Whether the failure is the folder saying a segment of the key is itself an
/// object, so the key can hold nothing.
///
/// A bucket has no folders and answers that such a key is missing. The folder's
/// operating system answers that the segment is not a directory, which arrives
/// untyped at the top of the failure but keeps its kind further down the chain,
/// so the two are made to agree here instead of in every caller.
fn runs_through_an_object(source: &BackendError) -> bool {
    let mut failure: Option<&(dyn std::error::Error + 'static)> = Some(source);
    while let Some(current) = failure {
        if let Some(io) = current.downcast_ref::<IoError>()
            && io.kind() == IoErrorKind::NotADirectory
        {
            return true;
        }
        failure = current.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    #[cfg(not(feature = "gcs"))]
    use super::RunStore;
    use super::{Chosen, STORAGE_BUCKET_VAR, STORAGE_DIR_VAR, VarError, chosen_backend, setting};
    use crate::error::StoreError;

    /// Whether a failure says what to do about itself.
    fn config_failure_naming(error: StoreError, expected: &[&str]) -> bool {
        let StoreError::Config(message) = error else {
            return false;
        };
        expected.iter().all(|wanted| message.contains(wanted))
    }

    #[test]
    fn a_variable_holds_a_value_nothing_or_a_mistake() {
        assert_eq!(setting("X", Ok("/tmp/runs".to_string())).ok(), Some(Some("/tmp/runs".to_string())));
        assert_eq!(setting("X", Err(VarError::NotPresent)).ok(), Some(None));

        // An empty or blank value is somebody meaning to name something and
        // failing, not somebody leaving the variable alone.
        for empty in ["", "   "] {
            let refused = setting("X", Ok(empty.to_string())).expect_err("an empty value is a mistake");
            assert!(config_failure_naming(refused, &["X", "empty"]));
        }

        let refused = setting("X", Err(VarError::NotUnicode(OsString::from("runs")))).expect_err("not text");
        assert!(config_failure_naming(refused, &["X"]));
    }

    #[test]
    fn the_environment_names_exactly_one_backend() {
        let folder = chosen_backend(Some("/tmp/runs".to_string()), None);
        assert_eq!(folder.ok(), Some(Chosen::Folder("/tmp/runs".to_string())));
        let bucket = chosen_backend(None, Some("gm-runs".to_string()));
        assert_eq!(bucket.ok(), Some(Chosen::Bucket("gm-runs".to_string())));

        // Neither and both are both a guess this crate will not make, and each
        // failure names the two variables the operator has to choose between.
        let neither = chosen_backend(None, None).expect_err("neither is set");
        assert!(config_failure_naming(neither, &[STORAGE_DIR_VAR, STORAGE_BUCKET_VAR]));
        let both =
            chosen_backend(Some("/tmp/runs".to_string()), Some("gm-runs".to_string())).expect_err("both are set");
        assert!(config_failure_naming(both, &[STORAGE_DIR_VAR, STORAGE_BUCKET_VAR]));
    }

    #[cfg(not(feature = "gcs"))]
    #[test]
    fn a_build_without_the_gcs_feature_says_so() {
        let refused = RunStore::bucket("gm-runs").expect_err("this build has no bucket backend");
        assert!(config_failure_naming(refused, &["gm-runs", "gcs"]));
    }
}
