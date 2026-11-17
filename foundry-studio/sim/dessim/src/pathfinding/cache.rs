//! Within `pathfinding`, `cache` is the MEMO: it wraps any `Pathfinder` and
//! remembers each spatial (from, orientation, to, cargo) result for an HTM, so
//! repeated "how far is X from Y" questions cost a map lookup instead of an A\*
//! search.
//!
//! Spatial only — no reservations, no time. That is what makes the answers
//! cacheable: they depend on graph topology and the HTM's drives, neither of
//! which changes during a run. Callers wanting reservation-aware paths must go
//! to `fh_sipp`, whose results are time-dependent and never cached here.
//! Used for travel-time estimates (demand-side quoting) and ghost mode.

use std::sync::Arc;

use dashmap::DashMap;
use desgraph::types::CellId;

use crate::model::SimDuration;

use super::path::Path;
use super::{DriveModel, Orientation, Pathfinder, PathfinderError};

/// Cache key for spatial path lookups. The memo is per load state because the
/// HTM's drive differs by load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathCacheKey {
    pub from: CellId,
    pub from_orientation: Orientation,
    pub to: CellId,
    pub carrying_cargo: bool,
}

/// Concurrent spatial path cache backed by `DashMap`.
///
/// Each cache is bound to one immutable robot model and warehouse graph.
/// Within that binding, paths depend on the key. Lazily populated on
/// first query. Thread-safe — no &mut self required.
///
/// # Drives
///
/// Both pairs are `[empty, laden]` and stored at construction. Each search
/// runs at the HTM's constant estimate drive for the key's load, and
/// [`travel_time`](Self::travel_time) converts the found path with the run's
/// drive for that load — so callers asking "how long does it take to get
/// there?" get an answer that matches the dispatched chain's actual cost under
/// both ConstantSpeed and HtmKinematic. Separate model caches also keep
/// speed/turn-dependent path choices from being reused by another model.
pub struct Cached {
    cache: DashMap<PathCacheKey, Path>,
    pathfinder: Arc<dyn Pathfinder>,
    htm_estimate_drives: [Arc<dyn DriveModel>; 2],
    htm_drives: [Arc<dyn DriveModel>; 2],
}

impl Cached {
    pub fn new(
        pathfinder: impl Pathfinder + Send + Sync + 'static,
        htm_estimate_drives: [Arc<dyn DriveModel>; 2],
        htm_drives: [Arc<dyn DriveModel>; 2],
    ) -> Self {
        Self { cache: DashMap::new(), pathfinder: Arc::new(pathfinder), htm_estimate_drives, htm_drives }
    }

    pub fn from_arc(
        pathfinder: Arc<dyn Pathfinder>,
        htm_estimate_drives: [Arc<dyn DriveModel>; 2],
        htm_drives: [Arc<dyn DriveModel>; 2],
    ) -> Self {
        Self { cache: DashMap::new(), pathfinder, htm_estimate_drives, htm_drives }
    }

    pub fn find_path(&self, key: PathCacheKey) -> Result<Path, PathfinderError> {
        if let Some(entry) = self.cache.get(&key) {
            #[cfg(feature = "perf")]
            tracing::info!(cache_hit = true);
            return Ok(entry.clone());
        }
        #[cfg(feature = "perf")]
        tracing::info!(cache_hit = false);

        use crate::model::SimTime;
        let path = self.pathfinder.find_path(
            key.from,
            key.from_orientation,
            key.to,
            self.htm_estimate_drives[key.carrying_cargo as usize].as_ref(),
            SimTime::ZERO,
        )?;
        self.cache.insert(key, path.clone());
        Ok(path)
    }

    /// Traversal time with the run's drive for `key.carrying_cargo`. A
    /// trapezoidal drive walks the cached spatial path's waypoints under its
    /// kinematic profile; a constant drive reprices its moves and turns with
    /// the execution figures. Single source of truth so demand-side
    /// quotes can't accidentally diverge from execution costs.
    pub fn travel_time(&self, key: PathCacheKey) -> Result<SimDuration, PathfinderError> {
        let path = self.find_path(key)?;
        Ok(self.pathfinder.kinematic_travel_time(&path, self.htm_drives[key.carrying_cargo as usize].as_ref()))
    }

    pub(crate) fn minimum_travel_time(&self, distance_mm: f64, carrying: bool) -> SimDuration {
        self.htm_estimate_drives[carrying as usize].min_travel_time(distance_mm.max(0.0).floor() as u64)
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn clear(&self) {
        self.cache.clear();
    }
}
