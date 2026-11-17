//! Per-state expansion orchestration for the SIPP-A\* search.
//!
//! Owned by the [`super::search::Search`] runner. Defines the dispatch
//! through the three successor families (rotate / move / wait) and the
//! goal-acceptance hook. Per-family logic lives in `expand_rotate`,
//! `expand_move`, and `expand_wait`.

use std::collections::hash_map::Entry;

use desgraph::Edge;
use desgraph::WarehouseGraph;
use desgraph::types::{CellId, ZoneId};
use smallvec::SmallVec;

use super::super::{DriveModel, Orientation, Path};
use super::node::NodeMeta;
use super::pathfinder::ALL_ORIENTATIONS;
use super::reconstruct::edge_motion;
use super::search::Search;
use super::state::{SearchKey, SpeedBucket};
use crate::SimDuration;

#[derive(Clone, Debug, Default)]
pub(super) struct UsefulOrients {
    values: SmallVec<[Orientation; 4]>,
}

impl UsefulOrients {
    fn insert(&mut self, orient: Orientation) {
        if !self.contains(orient) {
            self.values.push(orient);
        }
    }

    pub(super) fn contains(&self, orient: Orientation) -> bool {
        self.values.contains(&orient)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = Orientation> + '_ {
        self.values.iter().copied()
    }
}

/// Orientations worth turning to at `current_cell` given the bot's current
/// facing and the set of traversable outgoing edges.
///
/// Always includes every "reachable" orientation (one that faces a
/// traversable neighbor). Includes perpendicular intermediates whenever the
/// bot may need to synthesize a 2-turn rotation: either because
/// `current_orient` does not face a reachable neighbor, or because the
/// *opposite* of `current_orient` is reachable (a U-turn might be needed —
/// e.g., a corridor cell with neighbors both east and west, where the bot
/// is facing east but the goal is west). Without the U-turn case, a bot in
/// a long bidi corridor facing the wrong way could never find a path back.
///
/// Shared by reservation-aware `expand_state` (this module) and the
/// spatial-only `find_path` in `super::interface`. Symmetric pruning
/// in both keeps the goal-undirected fan-out from blowing up.
pub(super) fn useful_orients(
    graph: &WarehouseGraph,
    neighbors: &[(CellId, &Edge)],
    current_cell: CellId,
    current_orient: Orientation,
    drive: &dyn DriveModel,
    to: CellId,
    dest_zone: ZoneId,
) -> UsefulOrients {
    let mut reachable_orients = UsefulOrients::default();
    for (nid, edge) in neighbors {
        if drive.may_traverse(edge, to, dest_zone) {
            if let Some(motion) = edge_motion(graph, current_cell, *nid, edge, drive) {
                reachable_orients.insert(motion.departure);
            }
        }
    }

    let current_not_reachable = !reachable_orients.contains(current_orient);
    let opposite_reachable =
        ALL_ORIENTATIONS.iter().any(|r| reachable_orients.contains(*r) && r.turns_to(current_orient) == 2);
    let need_intermediate = current_not_reachable || opposite_reachable;

    let mut useful = UsefulOrients::default();
    for orient in ALL_ORIENTATIONS {
        if reachable_orients.contains(orient)
            || (need_intermediate
                && ALL_ORIENTATIONS.iter().any(|r| reachable_orients.contains(*r) && r.turns_to(orient) == 1))
        {
            useful.insert(orient);
        }
    }
    for orient in reachable_orients.iter() {
        if matches!(orient, Orientation::Heading(_)) {
            useful.insert(orient);
        }
    }
    useful
}

impl Search<'_> {
    /// Expand a single popped state. Calls the three successor families in
    /// order: rotate, move, wait. Goal-acceptance is handled separately by
    /// the loop driver because it returns a path.
    pub(super) fn expand_state(&mut self, current: SearchKey, current_g: SimDuration, current_meta: NodeMeta) {
        let current_g_t = self.config.sim_start + current_g;
        let current_cell = current.cell();
        let current_orient = current.orientation();
        let current_interval = current.interval();

        let neighbors = self.pf.inner.graph().neighbors(current_cell);
        // Per-search lazy cache. `useful_orients` is deterministic in
        // `(current_cell, current_orient)` given the per-search
        // constants: the drive, `to` and `dest_zone`.
        // SIPP-IP dedup expands multiple SearchKeys per (cell, orient)
        // pair (distinct intervals / speed buckets / wait labels), so
        // the cache earns hits from the second visit onward.
        let useful_orients = match self.scratch.useful_orients_cache.entry((current_cell, current_orient)) {
            Entry::Occupied(occupied) => occupied.get().clone(),
            Entry::Vacant(vacant) => {
                let computed = useful_orients(
                    self.pf.inner.graph(),
                    &neighbors,
                    current_cell,
                    current_orient,
                    self.drive,
                    self.bound_for,
                    self.dest_zone,
                );
                vacant.insert(computed).clone()
            }
        };

        self.try_rotate_successors(
            current,
            current_g,
            current_g_t,
            current_cell,
            current_orient,
            current_interval,
            useful_orients,
        );
        self.try_move_successors(
            current,
            current_g_t,
            current_cell,
            current_orient,
            current_interval,
            current_meta.exit_velocity,
            &neighbors,
        );
        self.try_wait_successors(current, current_g_t, current_interval);
    }

    /// Goal acceptance. Validates the terminal reservation per `dest_dwell`
    /// against the goal-cell safe interval. Returns `Some(path)` if the
    /// goal state is admissible; `None` if rejected (search continues — a
    /// later goal state in another safe interval may still satisfy).
    ///
    /// Per-goal-state rejection counts are not tracked: classification of
    /// `DestinationExceedsMaxWait` uses the structural
    /// `terminal_dwell_possible_in_horizon` flag computed from
    /// `goal_intervals` at search start, not per-popped-state failures.
    pub(super) fn try_accept_goal(
        &mut self,
        current: SearchKey,
        current_g: SimDuration,
        current_meta: NodeMeta,
    ) -> Option<Path> {
        // Kinematic constraint: an HTM in HtmKinematic mode must arrive at
        // the goal stationary. `expand_move` forces goal-cell moves to use
        // `target_exit_v = 0`, but `trapezoidal_cost` Case 3a clamps the
        // ACTUAL exit velocity upward when the cell is too short to brake
        // fully (e.g. v_max=4500, decel=2000 over 1m yields ~4031 mm/s).
        // Such states are physically invalid as a final stop, so reject
        // them — the search will route through additional decel cells
        // before the goal, or report no-path if no such route fits the
        // horizon. Mirrors the guards in `expand_wait` / `expand_rotate`.
        // TODO: ask the drive whether it can do this (and rotate, wait) while moving.
        if current.sipp.speed_bucket != SpeedBucket::ZERO {
            return None;
        }
        let m_in_start = current_meta.action_start;
        let m_in_end = current_meta.action_end;
        // Per spec §4: goal action-ready time must stay inside the
        // planning horizon. Defense-in-depth — upstream successor gates
        // already reject states whose g would exceed horizon_t.
        if m_in_end > self.horizon_t {
            self.stats.action_ready_past_horizon += 1;
            return None;
        }
        let current_interval = current.interval();
        let admissible = match self.config.dest_dwell {
            None => current_interval.1.is_none(),
            Some(d) if d == SimDuration::MAX => current_interval.1.is_none(),
            Some(d) => {
                let end = m_in_end.saturating_add(d);
                let interval_end_ok = current_interval.1.map(|ie| end <= ie).unwrap_or(true);
                self.stats.reservation_queries += 1;
                let dur = end - m_in_start;
                interval_end_ok
                    && self.oracle.earliest_available(self.to, m_in_start, dur, self.config.exclusion_for(self.to))
                        == Some(m_in_start)
            }
        };
        if !admissible {
            return None;
        }
        Some(self.build_accepted_path(current, current_g))
    }
}
