//! Within `crucible`, `floor` is WHAT A BROWSER DRAWS THE RUN ON: the world
//! document a recording carries, compiled once into the scene the renderer
//! draws and the roll of which cells each entity turned into.
//!
//! The server does this on the browser's behalf because turning a document's
//! entities into a floor of cells is the simulator's map compiler, and the
//! browser has no simulator in it. The answer goes into the scenario block
//! every attachment is sent, in the two fields a file leaves nil.
//!
//! It is done ONCE per run: the compile takes seconds on a site-sized document
//! and a run's world never changes, so the answer is kept beside the recording
//! it came out of. Once, and not once per attachment — two browsers attaching
//! together take turns on one guard rather than compiling the same world twice,
//! and the kept copy is published by rename, so a third never reads a
//! half-written one.

use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dessim::worldmap;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as Turn;
use tracing::warn;
use worldview::{Error as WorldviewError, Opaque, Reader, Scenario};
use worldview_floor::{CompileError, FLOOR_VERSION};

/// What one run's document compiled to, in the form the wire carries: two
/// MessagePack documents `worldview/worldview` ferries and never reads.
///
/// The two stamps are what make a kept copy safe to believe: the floor layout
/// the compile wrote, and the size of the recording it was compiled from. A
/// build that changed either compiles again rather than handing a browser a
/// scene of a shape it does not read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub floor_version: u32,
    pub of_bytes: u64,
    pub floor: Opaque,
    pub census: Opaque,
}

/// Why a recording's world document could not become a floor.
#[derive(Debug, thiserror::Error)]
pub enum FloorError {
    #[error("the recording could not be read: {0}")]
    Recording(#[from] WorldviewError),
    #[error("the embedded world document will not load: {0}")]
    Document(String),
    #[error("the document states no floor: {0}")]
    NoFloor(#[from] CompileError),
    #[error("the compiled floor could not be encoded: {0}")]
    Encode(String),
}

/// One turn per recording, so two first attachments compile one world once.
///
/// A map of async mutexes kept for the life of the process: one entry per run
/// this server has served, each a pointer, and a run's floor is asked for again
/// every time a browser opens it.
#[derive(Default)]
pub struct Compiling {
    turns: Mutex<FxHashMap<PathBuf, Arc<Turn<()>>>>,
}

impl Compiling {
    /// The compiled floor and census for this recording, compiling them if this
    /// is the first time anyone has asked.
    ///
    /// The turn is held across the blocking work, so the second caller waits
    /// and then finds the answer kept rather than doing it again.
    pub async fn of(&self, recording: &Path) -> Result<Scene, FloorError> {
        let turn = self.turn(recording);
        let _held = turn.lock().await;
        let path = recording.to_path_buf();
        tokio::task::spawn_blocking(move || of(&path)).await.map_err(|joined| FloorError::Encode(joined.to_string()))?
    }

    fn turn(&self, recording: &Path) -> Arc<Turn<()>> {
        let mut turns = self.turns.lock().expect("no compile panics holding this");
        match turns.entry(recording.to_path_buf()) {
            Entry::Occupied(held) => Arc::clone(held.get()),
            Entry::Vacant(free) => Arc::clone(free.insert(Arc::new(Turn::new(())))),
        }
    }
}

/// The compiled floor and census for this recording, read back or worked out.
/// Blocking; [`Compiling::of`] is the door a caller on a runtime comes in by.
pub fn of(recording: &Path) -> Result<Scene, FloorError> {
    let kept = beside(recording);
    let of_bytes = std::fs::metadata(recording).map(|held| held.len()).unwrap_or(0);
    if let Ok(bytes) = std::fs::read(&kept)
        && let Ok(scene) = rmp_serde::from_slice::<Scene>(&bytes)
        && (scene.floor_version, scene.of_bytes) == (FLOOR_VERSION, of_bytes)
    {
        return Ok(scene);
    }
    let scene = compile(recording, of_bytes)?;
    keep(&kept, &scene);
    Ok(scene)
}

/// Write the compiled scene beside its recording, by rename so a reader finds
/// the whole thing or nothing.
///
/// A failure costs the next attachment a recompile and nothing else, so it is
/// logged rather than raised — but it IS logged, because a folder that cannot
/// be written to makes every attachment pay for the same world for as long as
/// nobody notices.
fn keep(kept: &Path, scene: &Scene) {
    let encoded = match rmp_serde::to_vec(scene) {
        Ok(bytes) => bytes,
        Err(failure) => return warn!(path = %kept.display(), %failure, "the compiled floor would not encode"),
    };
    let arriving = kept.with_extension(format!("scene.{}.arriving", std::process::id()));
    let written = std::fs::write(&arriving, encoded).and_then(|()| std::fs::rename(&arriving, kept));
    if let Err(failure) = written {
        let _ = std::fs::remove_file(&arriving);
        warn!(path = %kept.display(), %failure, "the compiled floor could not be kept; the next reader compiles again");
    }
}

/// Where a recording's compiled floor is kept: beside the recording, under a
/// name derived from it, so removing the one folder removes both.
pub fn beside(recording: &Path) -> PathBuf {
    let mut name = recording.file_name().unwrap_or_default().to_os_string();
    name.push(".scene");
    recording.with_file_name(name)
}

fn compile(recording: &Path, of_bytes: u64) -> Result<Scene, FloorError> {
    let mut reader = Reader::open(recording)?;
    let scenario = Scenario::read_from(&mut reader)?;
    let world =
        worldmap::load_from_bytes(&scenario.world.0).map_err(|failure| FloorError::Document(failure.to_string()))?;
    let compiled = worldview_floor::compile(&world)?;
    Ok(Scene {
        floor_version: FLOOR_VERSION,
        of_bytes,
        floor: Opaque::of(&compiled.floor, "the compiled floor").map_err(|e| FloorError::Encode(e.to_string()))?,
        census: Opaque::of(&compiled.census, "the entity census").map_err(|e| FloorError::Encode(e.to_string()))?,
    })
}
