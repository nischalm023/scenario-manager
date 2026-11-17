//! Within `crucible`, `config` is WHAT THE SERVER WAS TOLD before it started:
//! the address to answer on, the folder its recordings are written in while
//! they are being written, the database its registry lives in, and how many
//! simulations it may run at once.
//!
//! Everything is read from the process environment once, at startup, and
//! nothing here is read again — a server that would change what it was told
//! mid-flight is a server whose logs cannot be trusted about what it did.
//!
//! Where the finished recordings go is `runstore`'s own two variables
//! (`CRUCIBLE_STORAGE_DIR` or `CRUCIBLE_STORAGE_BUCKET`), read by the store
//! rather than restated here.

use std::env::{self, VarError};
use std::path::PathBuf;
use std::thread::available_parallelism;
use std::time::Duration;

use thiserror::Error;

/// The address the websocket and the health check answer on.
pub const BIND_VAR: &str = "BIND";
/// The database holding the run registry.
pub const DATABASE_URL_VAR: &str = "DATABASE_URL";
/// The folder recordings are written into before they are stored.
pub const TEMP_DIR_VAR: &str = "CRUCIBLE_TEMP_DIR";
/// How many simulations may run at once.
pub const RUNS_AT_ONCE_VAR: &str = "CRUCIBLE_RUNS_AT_ONCE";
/// The largest control message the server will accept.
pub const MAX_MESSAGE_BYTES_VAR: &str = "CRUCIBLE_MAX_MESSAGE_BYTES";
/// How long a frame may wait for a browser's socket before the attachment is
/// dropped.
pub const ATTACH_LAG_SECONDS_VAR: &str = "CRUCIBLE_ATTACH_LAG_SECONDS";
/// The most points one metric's series is allowed to come back as.
pub const SERIES_POINTS_VAR: &str = "CRUCIBLE_SERIES_POINTS";
/// How much of the temporary folder the fetched recordings and their compiled
/// floors may take up.
pub const CACHE_BYTES_VAR: &str = "CRUCIBLE_CACHE_BYTES";

/// A submitted spec carries a warehouse design and its workload inline, so the
/// ceiling is generous; it exists so that a truncated or runaway message is
/// refused rather than held in memory.
const DEFAULT_MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

/// Long enough that a browser busy drawing a frame is not thrown off, short
/// enough that a browser that has gone away stops costing a reader.
const DEFAULT_ATTACH_LAG_SECONDS: usize = 10;

/// More points than a chart a few hundred pixels wide can show, and few enough
/// that one reply is one message.
const DEFAULT_SERIES_POINTS: usize = 2_000;

/// Room for a few hundred ten-minute recordings, or a dozen long ones, beside
/// the floors compiled from them.
const DEFAULT_CACHE_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// What the server was told.
#[derive(Debug, Clone)]
pub struct Config {
    /// Host and port to listen on.
    pub bind: String,
    /// The registry's database.
    pub database_url: String,
    /// Where a recording is written while the run that produces it is being
    /// recorded. The layout under it mirrors the storage keys, so the file a
    /// run wrote and the object it became have the same name.
    pub temp_dir: PathBuf,
    /// How many simulations run at once; submissions beyond it queue.
    pub runs_at_once: usize,
    /// The largest control message accepted.
    pub max_message_bytes: usize,
    /// How long one frame may wait for a browser's socket before its
    /// attachment is dropped and told where to resume.
    pub attach_lag: Duration,
    /// The most points one metric's series comes back as; more slices than
    /// that are averaged into that many buckets.
    pub series_points: usize,
    /// How much the fetched recordings and their compiled floors may take up
    /// before the ones nobody has read for longest are thrown away.
    pub cache_bytes: u64,
}

/// What the environment can fail to say.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0} is not set, and the server has nowhere to keep its runs")]
    Missing(&'static str),
    #[error("{key} is `{value}`, which is not a number")]
    NotANumber { key: &'static str, value: String },
    #[error("{key} is 0; the server has to be allowed to run at least one simulation")]
    NoRuns { key: &'static str },
    #[error("{key} is 0, which would end every attachment on its first frame")]
    NoWait { key: &'static str },
    #[error("{0} does not hold text")]
    NotText(&'static str),
}

impl Config {
    /// Read the environment, refusing anything it cannot make sense of rather
    /// than guessing a value the operator did not choose.
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            bind: stated(BIND_VAR)?.unwrap_or_else(|| "127.0.0.1:3001".to_string()),
            database_url: stated(DATABASE_URL_VAR)?.ok_or(ConfigError::Missing(DATABASE_URL_VAR))?,
            temp_dir: stated(TEMP_DIR_VAR)?.map(PathBuf::from).unwrap_or_else(|| env::temp_dir().join("crucible")),
            runs_at_once: number(RUNS_AT_ONCE_VAR, default_runs_at_once())?,
            max_message_bytes: number(MAX_MESSAGE_BYTES_VAR, DEFAULT_MAX_MESSAGE_BYTES)?,
            attach_lag: lag_wait(number(ATTACH_LAG_SECONDS_VAR, DEFAULT_ATTACH_LAG_SECONDS)?, ATTACH_LAG_SECONDS_VAR)?,
            series_points: number(SERIES_POINTS_VAR, DEFAULT_SERIES_POINTS)?.max(1),
            cache_bytes: number(CACHE_BYTES_VAR, DEFAULT_CACHE_BYTES)? as u64,
        })
    }
}

/// One core fewer than the machine has, so a run at full concurrency still
/// leaves the websocket and the registry a core to answer on.
fn default_runs_at_once() -> usize {
    available_parallelism().map(|cores| cores.get().saturating_sub(1)).unwrap_or(1).max(1)
}

/// What one variable holds: a value, nothing, or a mistake worth naming.
///
/// An empty value is a mistake rather than an absence: whoever wrote the
/// variable's name meant to name something with it.
fn stated(name: &'static str) -> Result<Option<String>, ConfigError> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Err(ConfigError::Missing(name)),
        Ok(value) => Ok(Some(value.trim().to_string())),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(ConfigError::NotText(name)),
    }
}

/// How long a frame may wait for a socket. Zero is refused by name: it would
/// end every attachment on its first frame, and an operator who wrote it meant
/// something else.
fn lag_wait(seconds: usize, key: &'static str) -> Result<Duration, ConfigError> {
    match seconds {
        0 => Err(ConfigError::NoWait { key }),
        seconds => Ok(Duration::from_secs(seconds as u64)),
    }
}

/// A count, or the default when the variable says nothing.
fn number(name: &'static str, fallback: usize) -> Result<usize, ConfigError> {
    let Some(value) = stated(name)? else { return Ok(fallback) };
    let parsed: usize = value.parse().map_err(|_| ConfigError::NotANumber { key: name, value: value.clone() })?;
    match parsed {
        0 if name == RUNS_AT_ONCE_VAR => Err(ConfigError::NoRuns { key: name }),
        _ => Ok(parsed),
    }
}

#[cfg(test)]
mod tests {
    use super::{RUNS_AT_ONCE_VAR, default_runs_at_once};

    #[test]
    fn at_least_one_run_is_allowed_however_small_the_machine() {
        // A single-core machine would otherwise be told it may run none.
        assert!(default_runs_at_once() >= 1, "{RUNS_AT_ONCE_VAR} defaults to a workable count");
    }
}
