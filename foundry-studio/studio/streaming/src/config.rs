//! Within `studio-streaming`, `config` is what the process is told through its environment: the database, the two
//! addresses it listens on, where a runner and a viewer reach it, how often it says it is alive, how many closed
//! streams it keeps replayable, and, for the picture (live frames), where its disk copies of the frames live, how long
//! and how large that cache may grow, and how many frame bytes one open attempt may hold in memory. Read once at
//! start; a value that does not read refuses the start in words, and so does a listener bound to every interface with
//! no address saying how it is reached, since the bind address would then be registered and reach nobody. The
//! run-object store's own variables (`CRUCIBLE_STORAGE_DIR` or `CRUCIBLE_STORAGE_BUCKET`) are read by `runstore`, not
//! here.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

pub const DATABASE_URL_VAR: &str = "STUDIO_DATABASE_URL";
pub const RUNNERS_BIND_VAR: &str = "STUDIO_STREAMING_RUNNERS_BIND";
pub const VIEWERS_BIND_VAR: &str = "STUDIO_STREAMING_VIEWERS_BIND";
pub const RUNNER_URL_VAR: &str = "STUDIO_STREAMING_RUNNER_URL";
pub const VIEWER_URL_VAR: &str = "STUDIO_STREAMING_VIEWER_URL";
pub const HEARTBEAT_SECS_VAR: &str = "STUDIO_STREAMING_HEARTBEAT_SECS";
pub const KEEP_VAR: &str = "STUDIO_STREAMING_KEEP";
pub const CACHE_DIR_VAR: &str = "STUDIO_STREAMING_CACHE_DIR";
pub const CACHE_TTL_SECS_VAR: &str = "STUDIO_STREAMING_CACHE_TTL_SECS";
pub const CACHE_MAX_BYTES_VAR: &str = "STUDIO_STREAMING_CACHE_MAX_BYTES";
pub const CACHE_SWEEP_SECS_VAR: &str = "STUDIO_STREAMING_CACHE_SWEEP_SECS";
pub const OPEN_MAX_BYTES_VAR: &str = "STUDIO_STREAMING_OPEN_MAX_BYTES";

/// A day of wall clock: how long an ended attempt's disk copy outlives its last open.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 86_400;
/// 20 GiB: the disk cache's size past which the oldest copies go first.
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 21_474_836_480;
/// Five minutes of wall clock between sweeps of the disk cache.
pub const DEFAULT_CACHE_SWEEP_SECS: u64 = 300;
/// 2 GiB: the frames one open attempt may hold in memory before its runner's socket is closed `frame-too-large`.
pub const DEFAULT_OPEN_MAX_BYTES: u64 = 2_147_483_648;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub database_url: String,
    /// The internal port, where runners connect (`/runners`).
    pub runners_bind: SocketAddr,
    /// The external port, where viewers connect (`/streams/{run}/{attempt}`, `/healthz`).
    pub viewers_bind: SocketAddr,
    /// The runners' websocket as a runner reaches it, which the dispatcher writes into a job spec.
    pub runner_url: String,
    /// The viewers' websocket base as a viewer reaches it, to which a watch ticket's `/{run}/{attempt}` is added.
    pub viewer_url: String,
    /// How often the registration row's heartbeat is refreshed (wall clock).
    pub heartbeat_every: Duration,
    /// How many closed streams stay replayable.
    pub keep: usize,
    /// Where the disk copies of the frames live, one folder per attempt under `<run>/<n>/`.
    pub cache_dir: PathBuf,
    /// Wall clock an ended attempt's disk copy outlives its last open before the sweeper may delete it.
    pub cache_ttl: Duration,
    /// Bytes the disk cache may hold before the sweeper deletes ended attempts' copies, oldest first.
    pub cache_max_bytes: u64,
    /// Wall clock between sweeps of the disk cache.
    pub cache_sweep_every: Duration,
    /// Bytes of frames one open attempt may hold in memory.
    pub open_max_bytes: u64,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0} is not set, and the server cannot start without it")]
    Missing(&'static str),
    #[error("{var} is {value:?}, which is not {expected}")]
    Unreadable { var: &'static str, value: String, expected: &'static str },
    #[error("{bind_var} binds every interface, so {var} must say how its listener is reached")]
    Unreachable { bind_var: &'static str, var: &'static str },
}

impl Config {
    /// The environment read; defaults are loopback ports 3201 (runners) and 3202 (viewers), a 2 s heartbeat, 50 closed
    /// streams kept replayable, a day's cache
    /// TTL, a 20 GiB cache swept every 5 minutes, and 2 GiB per open attempt. The database and the cache folder have
    /// no default.
    pub fn from_env() -> Result<Self, ConfigError> {
        let runners_bind = address(RUNNERS_BIND_VAR, "127.0.0.1:3201")?;
        let viewers_bind = address(VIEWERS_BIND_VAR, "127.0.0.1:3202")?;
        Ok(Self {
            database_url: env::var(DATABASE_URL_VAR).map_err(|_| ConfigError::Missing(DATABASE_URL_VAR))?,
            runners_bind,
            viewers_bind,
            runner_url: reached(RUNNER_URL_VAR, RUNNERS_BIND_VAR, runners_bind, |at| format!("ws://{at}/runners"))?,
            viewer_url: reached(VIEWER_URL_VAR, VIEWERS_BIND_VAR, viewers_bind, |at| format!("ws://{at}/streams"))?,
            heartbeat_every: Duration::from_secs(number(HEARTBEAT_SECS_VAR, 2)?),
            keep: whole(number(KEEP_VAR, 50)?),
            cache_dir: PathBuf::from(required(CACHE_DIR_VAR)?),
            cache_ttl: Duration::from_secs(number(CACHE_TTL_SECS_VAR, DEFAULT_CACHE_TTL_SECS)?),
            cache_max_bytes: number(CACHE_MAX_BYTES_VAR, DEFAULT_CACHE_MAX_BYTES)?,
            cache_sweep_every: Duration::from_secs(number(CACHE_SWEEP_SECS_VAR, DEFAULT_CACHE_SWEEP_SECS)?),
            open_max_bytes: number(OPEN_MAX_BYTES_VAR, DEFAULT_OPEN_MAX_BYTES)?,
        })
    }
}

/// A variable with no default; empty counts as unset, since an empty folder names nothing.
fn required(var: &'static str) -> Result<String, ConfigError> {
    match env::var(var) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(ConfigError::Missing(var)),
    }
}

/// How a listener bound at `bind` is reached: `var` when set, else the bind address as `own` spells it into a URL,
/// which is refused when the bind is every interface's.
fn reached(
    var: &'static str,
    bind_var: &'static str,
    bind: SocketAddr,
    own: impl FnOnce(SocketAddr) -> String,
) -> Result<String, ConfigError> {
    match env::var(var) {
        Ok(value) => Ok(value),
        Err(_) if bind.ip().is_unspecified() => Err(ConfigError::Unreachable { bind_var, var }),
        Err(_) => Ok(own(bind)),
    }
}

fn address(var: &'static str, default: &str) -> Result<SocketAddr, ConfigError> {
    let value = env::var(var).unwrap_or_else(|_| default.to_owned());
    value.parse().map_err(|_| ConfigError::Unreadable {
        var,
        value,
        expected: "an address and port, like 0.0.0.0:3201",
    })
}

fn whole(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

fn number(var: &'static str, default: u64) -> Result<u64, ConfigError> {
    match env::var(var) {
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(n) if n >= 1 => Ok(n),
            _ => Err(ConfigError::Unreadable { var, value, expected: "a whole number of at least 1" }),
        },
        Err(_) => Ok(default),
    }
}
