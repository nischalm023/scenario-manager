//! What run-output storage reports when an operation does not deliver: the
//! failures callers act on told apart from the ones they can only report.
//!
//! Relative to the crate: `runstore` is the run-output store, and this module is
//! the vocabulary its whole interface answers in. Three failures drive decisions
//! and so are named rather than lumped in with the rest: a missing object is how
//! a reader learns it has caught up with the writer between chunks, a read
//! starting past the end of an object is how it learns the same thing inside
//! one, and an object already present is how the writer of an attempt learns a
//! stale twin got there first. Each of the three is one arm here because the two
//! backends word them differently and no caller should have to know that.
//!
//! Every failure says why in its own message. What the disk, the network, or the
//! service said is carried as the source and repeated in the display, because a
//! log line that reads "put failed" and drops the reason is what turns a full
//! disk into an afternoon.

use thiserror::Error;

use crate::key::KeyError;

/// The result of every run-output storage operation.
pub type StoreResult<T> = Result<T, StoreError>;

/// A run-output storage operation that did not deliver.
#[derive(Debug, Error)]
pub enum StoreError {
    /// No object is stored under this key.
    #[error("no object at `{key}`")]
    NotFound {
        /// The key asked for.
        key: String,
    },
    /// An object is already stored under this key, and the write refused to
    /// replace it.
    #[error("an object already exists at `{key}`")]
    AlreadyExists {
        /// The key the write was refused for.
        key: String,
    },
    /// The key or prefix is not spelled the way this store requires.
    #[error("`{key}` is not a usable object key: {source}")]
    InvalidKey {
        /// The key or prefix exactly as the caller gave it.
        key: String,
        /// Which part of the spelling rule it broke.
        #[source]
        source: KeyError,
    },
    /// The store could not be built from what the environment says.
    #[error("run-output storage is not configured: {0}")]
    Config(String),
    /// The read starts at or past the end of the object, so there is nothing
    /// there to read.
    #[error("`{key}` holds {size} bytes, so a read starting at {start} is past its end")]
    PastTheEnd {
        /// The object read.
        key: String,
        /// Where the read would have started.
        start: u64,
        /// The object's size when the read was refused.
        size: u64,
    },
    /// The backend failed for a reason of its own: the network, the disk, a
    /// permission, a service error.
    #[error("{operation} failed for `{key}`: {source}")]
    Backend {
        /// Which operation failed, named as the interface names it.
        operation: &'static str,
        /// The key or prefix it was working on.
        key: String,
        /// What the backend said.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
