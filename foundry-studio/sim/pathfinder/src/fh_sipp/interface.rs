//! `Pathfinder` trait implementation for [`FHSippPathfinder`] — the
//! public-interface boundary.
//!
//! - `find_path` performs spatial-only A\* (no reservation checks). Used
//!   for travel-time estimates, ghost mode, and `Cached`
//!   lookups. The sentinel `SPATIAL_INTERVAL = (ZERO, None)` collapses
//!   all SIPP intervals at a `(cell, orient)` to a single dedup entry —
//!   appropriate for spatial-only search.
//! - `find_path_reserved` runs reservation-aware SIPP without committing.
//! - `reserve_path` delegates to the allocator, which commits the path in
//!   windows, each composed and committed atomically through the oracle.

use async_trait::async_trait;
use rustc_hash::FxHashMap;

use desgraph::types::{CellId, ZoneId};

#[cfg(any(feature = "pathfinder-trace", test))]
use super::super::pathfinder_trace::prelude::*;
use super::super::pathfinder_trace::trace_emit;
use super::super::{
    CollisionConfig, DriveModel, MotionKind, MotionModel, Orientation, Path, Pathfinder, PathfinderError, ReservedPath,
    WindowSink,
};
use super::expand::useful_orients;
use super::heap::BucketQueue;
use super::node::{NodeActionKind, NodeMeta};
use super::pathfinder::{FHSippPathfinder, Inner, open_bucket_capacity, rotation_permitted};
use super::reconstruct::{constant_walk_time, edge_motion, kinematic_walk_time, reconstruct_waypoints};
use super::search::{NODE_TABLE_INITIAL_CAPACITY, SearchEntry, relax_node};
use super::state::{SafeIntervalId, SearchKey, SippState, SpeedBucket};
use crate::reservation::oracle::ReservationOracle;
use crate::{SimDuration, SimTime, Velocity};

/// Sentinel safe interval for spatial-only `find_path`: "no temporal
/// constraint." All states at a `(cell, orient)` collapse to a single entry.
const SPATIAL_INTERVAL: SafeIntervalId = (SimTime::ZERO, None);

#[async_trait(?Send)]
impl Pathfinder for FHSippPathfinder {
    /// Spatial-only A\*. Ignores the reservation table; returns timing on
    /// the static graph for travel-time estimates and ghost mode.
    ///
    /// Plans at constant speed only: its states carry no speed, so it
    /// refuses a kinematic drive.
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    fn find_path(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        start: SimTime,
    ) -> Result<Path, PathfinderError> {
        assert_eq!(
            drive.kind(),
            MotionKind::Constant,
            "find_path plans constant-speed routes only; it cannot price a kinematic drive"
        );
        if from == to {
            return Err(PathfinderError::ZeroLength { cell: from });
        }

        let start_sipp = SippState {
            cell: from,
            orientation: from_orientation,
            interval: SPATIAL_INTERVAL,
            speed_bucket: SpeedBucket::ZERO,
        };
        let start_key = SearchKey::ordinary(start_sipp);

        let mut nodes: FxHashMap<SearchKey, SearchEntry> =
            FxHashMap::with_capacity_and_hasher(NODE_TABLE_INITIAL_CAPACITY, Default::default());
        let mut open = BucketQueue::with_capacity(open_bucket_capacity(self.inner.move_jump_mm, drive));

        nodes.insert(
            start_key,
            SearchEntry {
                key: start_key,
                g: SimDuration::ZERO,
                parent: None,
                meta: NodeMeta {
                    action_kind: NodeActionKind::Start,
                    action_start: start,
                    action_end: start,
                    source_cell: None,
                    interval: SPATIAL_INTERVAL,
                    entry_velocity: Velocity::ZERO,
                    exit_velocity: Velocity::ZERO,
                },
            },
        );
        let h = self.heuristic_for(from, to, drive);
        open.push(start_key, h, SimDuration::ZERO);

        let dest_zone = self.inner.graph().cell(to).map(|c| c.zone).unwrap_or(ZoneId(0));

        let mut iterations: u64 = 0;
        while let Some((current, current_g)) = open.pop() {
            iterations += 1;
            if iterations > 5_000_000 {
                return Err(PathfinderError::IterationLimit { from, to });
            }

            if nodes.get(&current).is_some_and(|e| current_g > e.g) {
                continue;
            }

            let current_cell = current.cell();
            let current_orient = current.orientation();

            trace_emit!(self.inner, Expand { cell: current_cell, orientation: current_orient, g: current_g, step: 0 });

            if current_cell == to {
                // Spatial-only `find_path` is always ConstantSpeed — no
                // kinematic search occurs here, so velocity substitution
                // unconditionally uses the drive's cruise speed.
                let waypoints = reconstruct_waypoints(
                    |k| nodes.get(&k).cloned(),
                    current,
                    drive.max_speed(),
                    MotionModel::ConstantSpeed,
                );
                return Ok(Path { waypoints, travel_time: current_g });
            }

            let current_g_t = start + current_g;

            // Compute neighbors once per popped state. Reused for the
            // useful-orients pruning gate below and the move successor
            // emission. Symmetric with `expand::expand_state` so the
            // spatial-only and reservation-aware searches don't fan out
            // on different shapes.
            let neighbors = self.inner.graph().neighbors(current_cell);
            let useful =
                useful_orients(self.inner.graph(), &neighbors, current_cell, current_orient, drive, to, dest_zone);

            // Rotate successors, over the same `useful_orients` and timed by
            // the same rule (`Inner::rotate_cost`) as the reservation-aware
            // search's in `expand_rotate.rs`, so a travel estimate gives a
            // rotate the same time the reserved plan does.
            if rotation_permitted(self.inner.graph(), current_cell, drive) {
                for orient in useful.iter() {
                    let Some(turn) = Inner::rotate_cost(drive, current_orient, orient) else {
                        continue;
                    };
                    let action_start = current_g_t;
                    let action_end = action_start.saturating_add(turn);
                    let next_g = current_g + turn;
                    let next_sipp = SippState {
                        cell: current_cell,
                        orientation: orient,
                        interval: SPATIAL_INTERVAL,
                        speed_bucket: SpeedBucket::ZERO,
                    };
                    let next_key = SearchKey::ordinary(next_sipp);
                    let candidate = SearchEntry {
                        key: next_key,
                        g: next_g,
                        parent: Some(current),
                        meta: NodeMeta {
                            action_kind: NodeActionKind::Rotate,
                            action_start,
                            action_end,
                            source_cell: None,
                            interval: SPATIAL_INTERVAL,
                            entry_velocity: Velocity::ZERO,
                            exit_velocity: Velocity::ZERO,
                        },
                    };
                    if relax_node(&mut nodes, next_key, candidate) {
                        let h = self.heuristic_for(current_cell, to, drive);
                        open.push(next_key, next_g + h, next_g);
                        trace_emit!(
                            self.inner,
                            Rotate(current_cell, current_orient, orient, next_g, SimDuration::ZERO)
                        );
                    }
                }
            }

            // Move successors.
            for &(neighbor_id, edge) in neighbors.iter() {
                if !drive.may_traverse(edge, to, dest_zone) {
                    continue;
                }
                let Some(motion) = edge_motion(self.inner.graph(), current_cell, neighbor_id, edge, drive) else {
                    continue;
                };
                if current_orient != motion.departure {
                    // Spatial-only mode: turns roll into the move time but
                    // we model them as separate states for path fidelity.
                    continue;
                }
                let (traverse_cost, _) = drive.kinematic_cost(Velocity::ZERO, Velocity::ZERO, motion.distance_mm);
                let action_start = current_g_t;
                let action_end = action_start.saturating_add(traverse_cost);
                let next_g = current_g + traverse_cost;
                let next_sipp = SippState {
                    cell: neighbor_id,
                    orientation: motion.arrival,
                    interval: SPATIAL_INTERVAL,
                    speed_bucket: SpeedBucket::ZERO,
                };
                let next_key = SearchKey::ordinary(next_sipp);
                let candidate = SearchEntry {
                    key: next_key,
                    g: next_g,
                    parent: Some(current),
                    meta: NodeMeta {
                        action_kind: NodeActionKind::Move,
                        action_start,
                        action_end,
                        source_cell: Some(current_cell),
                        interval: SPATIAL_INTERVAL,
                        entry_velocity: Velocity::ZERO,
                        exit_velocity: Velocity::ZERO,
                    },
                };
                if relax_node(&mut nodes, next_key, candidate) {
                    let h = self.heuristic_for(neighbor_id, to, drive);
                    open.push(next_key, next_g + h, next_g);
                    trace_emit!(self.inner, Move(current_cell, neighbor_id, next_g, SimDuration::ZERO));
                }
            }
        }

        Err(PathfinderError::Unreachable { from, to })
    }

    async fn reserve_path(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
        windows: &mut dyn WindowSink,
    ) -> Result<ReservedPath, PathfinderError> {
        FHSippPathfinder::reserve_path(self, from, from_orientation, to, drive, oracle, config, windows).await
    }

    fn traverse_duration(&self) -> SimDuration {
        FHSippPathfinder::traverse_duration(self)
    }

    // TODO: delete once quotes come from the drive model.
    fn kinematic_travel_time(&self, path: &Path, drive: &dyn DriveModel) -> SimDuration {
        match drive.kind() {
            MotionKind::Constant => constant_walk_time(&path.waypoints, drive, self.inner.graph()),
            MotionKind::Kinematic => kinematic_walk_time(&path.waypoints, drive, self.inner.graph()),
        }
    }
}
