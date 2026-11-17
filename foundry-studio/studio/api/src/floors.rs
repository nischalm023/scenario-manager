//! Within `studio-api`, `floors` is spec 03 section 3.6's floor (BACKLOG task 11): a self-contained world document,
//! a site's flagged export or a template evaluation's document, loaded with `dessim::worldmap::load_from_bytes`,
//! compiled with `worldview_floor::compile` and written as the one floor document a browser draws, `{ floor_version,
//! floor, roll }` (`worldview_render::FloorAndRoll`), as crucible keeps a recording's floor.
//!
//! It exists so the Map panel and a run of the same changesets draw one floor: the compile is the one crucible and
//! desvis run, over the exact bytes a run loads, keyed by their sha256 (spec 04 section 3.11's `FloorKey`), and it
//! runs on spec 01's compute pool, never on a thread serving a route, a stream or a turn. A site's floor is held in
//! the client's floor cache, which owns the compile: it runs once however many ask, whoever stops waiting, and every
//! asker reads its one outcome. Where an evaluation's floor is kept is the evaluation's own store (spec 05).
//! `GET /api/floors/{key}` serves both (`route`); the map summary that names a site's key is `map`'s.

mod route;

use std::env;
use std::fmt;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use dessim::worldmap::{WorldMap, load_from_bytes};
use rustc_hash::FxHashMap;
use studio_protocol::{Problem, ProblemType};
use worldview_floor::{CompileError, Compiled};
use worldview_render::{FLOOR_VERSION, FloorAndRoll};
use wps_client::cache::{FloorCache, FloorKey};
use wps_client::compute::{self, Compute};
use wps_client::{UnitCut, WpsError};

use crate::problem::{ProblemAnswer, wps_problem};

pub use route::routes;

pub const BUILD_COMMIT_VAR: &str = "STUDIO_BUILD_COMMIT";

/// Why a document has no floor to draw: what the person reads after "This map cannot be drawn: " (spec 03 section
/// 3.8, `floor-compile-failed`). A document that loads still has its counts, so the two are told apart.
#[derive(Debug)]
pub enum FloorError {
    /// The simulator's reader refuses the document, a prologue without an id or a primary frame among the reasons.
    Load(String),
    /// The document loads and states no floor: no grid bounds, or conveyor geometry that will not place.
    Compile(CompileError),
    /// The floor would not encode, which is a fault of ours rather than the document's.
    Encode(String),
}

impl fmt::Display for FloorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(why) => write!(f, "the site's document will not load: {why}"),
            Self::Compile(why) => write!(f, "{why}"),
            Self::Encode(why) => write!(f, "its floor would not encode: {why}"),
        }
    }
}

/// Load a self-contained document as the simulator does, for its floor and for what it counts.
pub fn load(document: &[u8]) -> Result<WorldMap, FloorError> {
    load_from_bytes(document).map_err(|failure| FloorError::Load(failure.to_string()))
}

pub fn compile(map: &WorldMap) -> Result<Compiled, FloorError> {
    worldview_floor::compile(map).map_err(FloorError::Compile)
}

/// How large a floor document is, in bytes, as a map summary says it (`FloorRef.bytes`, a number in its JSON).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloorBytes(u64);

impl FloorBytes {
    pub fn of(document: &[u8]) -> Self {
        Self(document.len() as u64)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The floor document's bytes: what `GET /api/floors/{key}` answers, its roll saying which entity is a cell.
pub fn encode(compiled: &Compiled) -> Result<Bytes, FloorError> {
    let bytes = FloorAndRoll::write(&compiled.floor, &compiled.roll());
    bytes.map(Bytes::from).map_err(|failure| FloorError::Encode(failure.to_string()))
}

/// A document straight to its floor document. Blocking and CPU-bound: a caller on the runtime goes through
/// [`site_floor`], or runs this on the compute pool itself.
pub fn floor_document(document: &[u8]) -> Result<Bytes, FloorError> {
    encode(&compile(&load(document)?)?)
}

/// The key a floor is held under: the document's sha256, the compile's identity (`FLOOR_VERSION`, the one version of
/// the floor and its roll, and `build`, the commit the API was built from), and the digest of a draft applied in
/// memory, if one was. A changed prologue or label changes the bytes, so a later change makes a new key and never new
/// bytes under an old one.
pub fn floor_key(document_sha256: &str, build: &str, draft: Option<&str>) -> FloorKey {
    FloorKey {
        document: document_sha256.to_owned(),
        compiler: format!("floor{FLOOR_VERSION}@{build}"),
        draft: draft.map(str::to_owned),
    }
}

/// A key as one path segment, `{document}.{compiler}` or `{document}.{compiler}.{draft}`: what `MapSummary` names
/// and `GET /api/floors/{key}` is asked with. None of the three holds a dot.
pub fn key_text(key: &FloorKey) -> String {
    match &key.draft {
        Some(draft) => format!("{}.{}.{draft}", key.document, key.compiler),
        None => format!("{}.{}", key.document, key.compiler),
    }
}

/// The key a path segment names, or nil for a segment [`key_text`] never writes.
pub fn read_key(text: &str) -> Option<FloorKey> {
    let mut parts = text.split('.');
    let (document, compiler) = (parts.next()?, parts.next()?);
    let draft = parts.next();
    let hex = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_hexdigit());
    let fits = parts.next().is_none() && hex(document) && compiler.starts_with("floor") && draft.is_none_or(hex);
    fits.then(|| FloorKey {
        document: document.to_owned(),
        compiler: compiler.to_owned(),
        draft: draft.map(str::to_owned),
    })
}

/// What asking for a site's floor can end in besides its bytes, shared by everyone who asked during one compile.
#[derive(Debug, Clone)]
pub enum FloorFailure {
    /// The document has no floor (`floor-compile-failed`).
    Floor(Arc<FloorError>),
    /// The compute pool was full (`compute-busy`), or the computation ended without an answer.
    Compute(WpsError),
}

impl From<WpsError> for FloorFailure {
    fn from(failure: WpsError) -> Self {
        Self::Compute(failure)
    }
}

/// A site's floor document under `key`, compiled from `document` on `pool` when `floors` does not hold it yet. The
/// cache owns the compile: a caller asking while it runs waits for it rather than starting another, a caller that
/// stops waiting leaves it running, and its floor is held once done. A document with no floor is not held, so each
/// later ask says why again, and the cache only ever holds floors.
pub async fn site_floor(
    floors: &FloorCache,
    pool: Arc<dyn Compute>,
    key: FloorKey,
    document: Bytes,
) -> Result<Bytes, FloorFailure> {
    floors
        .get_or_spawn(key, move || async move {
            match compute::run(&*pool, move || floor_document(&document)).await {
                Ok(Ok(floor)) => Ok(floor),
                Ok(Err(failure)) => Err(FloorFailure::Floor(Arc::new(failure))),
                Err(failure) => Err(FloorFailure::Compute(failure)),
            }
        })
        .await
}

/// The commit this API was built from, as a floor key's compiler identity names it, read once at start:
/// `STUDIO_BUILD_COMMIT` when set, else `git describe --always --dirty` of the checkout the API runs in, which dev.sh
/// builds it from (spec 08 section 3.5's local image name takes the same describe). Only `[A-Za-z0-9-]` is kept, so
/// a key's text stays one dot-separated path segment.
///
/// Deliberately: a process that can say neither keys its floors by its own start, `unversioned-{ms}`, so that no
/// floor a browser cached under another build's key is ever taken for this build's.
pub fn build_commit() -> &'static str {
    static BUILD: OnceLock<String> = OnceLock::new();
    BUILD.get_or_init(|| {
        let described = || {
            let out = Command::new("git").args(["describe", "--always", "--dirty", "--abbrev=12"]).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        };
        let named = env::var(BUILD_COMMIT_VAR).ok().filter(|commit| !commit.trim().is_empty()).or_else(described);
        let kept: String = named
            .unwrap_or_default()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        if kept.is_empty() {
            let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
            format!("unversioned-{started}")
        } else {
            kept
        }
    })
}

/// Where each site floor key this process minted came from: the cut whose flagged export the key names, so a floor
/// the cache let go is compiled again from the same bytes rather than answered as gone.
#[derive(Default)]
pub struct Origins(Mutex<FxHashMap<FloorKey, UnitCut>>);

impl Origins {
    pub fn remember(&self, key: FloorKey, cut: UnitCut) {
        self.0.lock().expect("no panic holds the origins").insert(key, cut);
    }

    pub fn of(&self, key: &FloorKey) -> Option<UnitCut> {
        self.0.lock().expect("no panic holds the origins").get(key).cloned()
    }
}

/// A floor that could not be had, as the person meets it: `floor-compile-failed` with the reason after "This map
/// cannot be drawn: ", or the pool's `compute-busy`.
pub fn floor_problem(failure: &FloorFailure, instance: &str) -> ProblemAnswer {
    match failure {
        FloorFailure::Floor(why) => {
            let detail = format!("This map cannot be drawn: {why}.");
            Problem::new(ProblemType::FloorCompileFailed, detail).at(instance).into()
        }
        FloorFailure::Compute(error) => wps_problem(error, instance),
    }
}

#[cfg(test)]
mod tests;
