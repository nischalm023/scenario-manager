//! # runstore
//!
//! Run-output storage: the objects a simulation run leaves behind — the spec it
//! was run from and the `.worldview` recording its frames were written to — and
//! the single interface everything above reads and writes them through.
//!
//! The interface exists so that no caller knows which storage it has. In
//! production that is a Google Cloud Storage bucket; in development it is a
//! folder on disk, so a developer runs the whole system without a cloud
//! credential. The folder keeps the semantics the run format depends on: an
//! object appears whole or not at all, nothing is appended to, and a listing is
//! complete the moment a write returns.
//!
//! - [`storage`] — the interface: [`RunStorage`] and what it promises.
//! - [`key`] — what a key may be spelled with, checked before any backend
//!   sees it.
//! - [`store`] — [`RunStore`], the implementation over real storage, and the
//!   choice of backend from the environment.
//! - [`layout`] — the keys a run's objects live under, and the reading of a key
//!   back into the run and attempt it names.
//! - [`error`] — what a storage operation reports when it does not deliver.
//!
//! ```no_run
//! use bytes::Bytes;
//! use runstore::layout::{AttemptNumber, RunId, recording_key};
//! use runstore::{RunStorage, RunStore};
//!
//! # async fn upload_an_attempts_recording() -> Result<(), Box<dyn std::error::Error>> {
//! let store = RunStore::from_env()?;
//! let run = RunId::new("01J8Z5K3QW")?;
//! let key = recording_key(&run, AttemptNumber::FIRST);
//! store.put(&key, Bytes::from_static(b"...")).await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod key;
pub mod layout;
pub mod storage;
pub mod store;

pub use error::{StoreError, StoreResult};
pub use key::KeyError;
pub use storage::{ObjectInfo, RunStorage};
pub use store::{RunStore, STORAGE_BUCKET_VAR, STORAGE_DIR_VAR};
