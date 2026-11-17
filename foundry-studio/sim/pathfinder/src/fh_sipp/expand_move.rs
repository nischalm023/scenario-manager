//! Move-successor expansion. Each candidate neighbor cell × each safe
//! interval admitting an entry that fits the implied composed reservation
//! window. Single-shot: every Move is reservation-validated AND
//! edge-validated; there is no spatial-only fallback.
//!
//! Branching on the drive's [`MotionKind`]:
//! - `Constant` emits a single move successor per (neighbor, interval) at the
//!   drive's cell traversal time. Velocities stay at `Velocity::ZERO` and
//!   `SpeedBucket::ZERO` everywhere, so the search dedups on cell, facing and
//!   interval alone.
//! - `Kinematic` (an HTM's trapezoid under `MotionModel::HtmKinematic`)
//!   emits TWO move successors per (neighbor, interval): one for cruise
//!   (target exit = `v_max`) and one for decelerate-to-stop (target
//!   exit = `ZERO`). Each carries its own `actual_exit_velocity` from the
//!   trapezoidal profile and gets a distinct `SpeedBucket` so the SIPP-IP
//!   dedup keeps them separate. Required for completeness under bounded
//!   acceleration (Ali & Yakovlev, AAAI 2023).
//!
//! Curves use arc length and endpoint tangents. Profiles constrain longitudinal
//! acceleration only; they add no lateral-acceleration or steering-speed limit.
//! Collision checks protect graph cells and directed edges, not swept bodies.

use desgraph::Edge;
use desgraph::types::CellId;
use smallvec::SmallVec;

use super::super::Orientation;
#[cfg(any(feature = "pathfinder-trace", test))]
use super::super::pathfinder_trace::prelude::*;
use super::super::pathfinder_trace::trace_emit;
use super::node::{NodeActionKind, NodeMeta};
use super::reconstruct::edge_motion;
use super::search::Search;
use super::state::{SafeIntervalId, SearchKey, SippState, SpeedBucket};
use crate::{MotionKind, SimDuration, SimTime, Velocity};

impl Search<'_> {
    pub(super) fn try_move_successors(
        &mut self,
        current: SearchKey,
        current_g_t: SimTime,
        current_cell: CellId,
        current_orient: Orientation,
        current_interval: SafeIntervalId,
        entry_velocity: Velocity,
        neighbors: &[(CellId, &Edge)],
    ) {
        for (neighbor_id, edge) in neighbors {
            let neighbor_id = *neighbor_id;
            if !self.drive.may_traverse(edge, self.bound_for, self.dest_zone) {
                self.stats.moves_filtered_traversability += 1;
                continue;
            }
            let Some(motion) = edge_motion(self.pf.inner.graph(), current_cell, neighbor_id, edge, self.drive) else {
                continue;
            };
            if current_orient != motion.departure {
                self.stats.turns_skipped_timed += 1;
                continue;
            }
            let move_out_distance_mm = if neighbor_id == self.to {
                0
            } else {
                self.pf
                    .inner
                    .graph()
                    .neighbors(neighbor_id)
                    .iter()
                    .filter_map(|(next, next_edge)| {
                        self.drive
                            .may_traverse(next_edge, self.bound_for, self.dest_zone)
                            .then(|| edge_motion(self.pf.inner.graph(), neighbor_id, *next, next_edge, self.drive))
                            .flatten()
                            .map(|next_motion| next_motion.distance_mm)
                    })
                    .min()
                    .unwrap_or(0)
            };

            // Build the per-variant emission list. A constant drive: one variant.
            // A kinematic drive: two variants (cruise vs decel-to-zero).
            // Each variant carries (target_exit_v, computed_traverse_cost,
            // actual_exit_v). The target is what we asked for; actual may be
            // clamped by the kinematic profile. `SmallVec<[_; 2]>` keeps the
            // 1- or 2-element list stack-resident; profiling pinned the
            // previous `Vec` drop at 5.5% of total wall time per call.
            let variants: SmallVec<[(Velocity, SimDuration, Velocity); 2]> = match self.drive.kind() {
                MotionKind::Constant => {
                    let (cost, _) = self.drive.kinematic_cost(Velocity::ZERO, Velocity::ZERO, motion.distance_mm);
                    let mut v = SmallVec::new();
                    v.push((Velocity::ZERO, cost, Velocity::ZERO));
                    v
                }
                MotionKind::Kinematic => {
                    // TODO: ask the drive whether it can start a reverse while moving.
                    if motion.reverse && entry_velocity != Velocity::ZERO {
                        continue;
                    }
                    let cruise_target = self.drive.max_speed();
                    let decel_target = Velocity::ZERO;
                    let (decel_cost, decel_actual) =
                        self.drive.kinematic_cost(entry_velocity, decel_target, motion.distance_mm);
                    let mut v: SmallVec<[(Velocity, SimDuration, Velocity); 2]> = SmallVec::new();
                    // Goal-cell moves MUST terminate at zero velocity — the
                    // robot has to be physically stopped on arrival to honor
                    // dest_dwell semantics. Suppress the cruise variant for
                    // the goal so the search can't pick a faster path that
                    // arrives at v_max. Non-goal cells emit both variants
                    // (cruise + decel) so downstream actions like rotate
                    // (which need SpeedBucket::ZERO) can still be reached.
                    if neighbor_id != self.to && !motion.reverse {
                        let (cruise_cost, cruise_actual) =
                            self.drive.kinematic_cost(entry_velocity, cruise_target, motion.distance_mm);
                        v.push((cruise_target, cruise_cost, cruise_actual));
                        // Skip the decel variant if it would produce the same
                        // actual_exit_v as cruise (would dedup on the same
                        // SpeedBucket and just produce a longer-time copy).
                        if SpeedBucket::from_velocity(decel_actual) != SpeedBucket::from_velocity(cruise_actual) {
                            v.push((decel_target, decel_cost, decel_actual));
                        }
                    } else {
                        v.push((decel_target, decel_cost, decel_actual));
                    }
                    v
                }
            };

            self.stats.entry_intervals_calls += 1;
            // Take the per-search scratch buffer out of `self` so the
            // subsequent `&mut self` calls to `try_emit_move_into_interval`
            // can borrow self exclusively. The scratch is restored after
            // the inner loop. On panic mid-loop the field is left as
            // `Vec::new()`; functionally that's a clean restart of the
            // scratch on the next search, not data loss.
            let mut intervals_scratch = std::mem::take(&mut self.scratch.entry_intervals_scratch);
            self.oracle.entry_intervals_into(
                neighbor_id,
                current_g_t,
                self.horizon_t,
                self.config.exclusion_for(neighbor_id),
                &mut intervals_scratch,
            );

            trace_emit!(
                self.pf.inner,
                EntryIntervals { cell: neighbor_id, intervals: intervals_scratch.clone() },
                self.oracle
            );

            if intervals_scratch.is_empty() {
                self.stats.reservation_permanent += 1;
                self.scratch.entry_intervals_scratch = intervals_scratch;
                continue;
            }

            for &(free_start, free_end) in &intervals_scratch {
                for &(target_exit_v, traverse_cost, actual_exit_v) in &variants {
                    self.try_emit_move_into_interval(
                        current,
                        current_g_t,
                        current_cell,
                        current_interval,
                        neighbor_id,
                        motion.arrival,
                        traverse_cost,
                        move_out_distance_mm,
                        free_start,
                        free_end,
                        entry_velocity,
                        target_exit_v,
                        actual_exit_v,
                    );
                }
            }
            self.scratch.entry_intervals_scratch = intervals_scratch;
        }
    }

    /// Emit a single Move successor into one (free_start, free_end) interval
    /// on the neighbor. Validates source-hold for any wait, planning horizon,
    /// exact admission, and the directed edge before pushing.
    ///
    /// `target_exit_v` is the requested exit velocity at the neighbor's
    /// center; `actual_exit_v` is what the trapezoidal profile actually
    /// achieved (may differ when distance was insufficient). The new state's
    /// `SpeedBucket` derives from `actual_exit_v`.
    #[allow(clippy::too_many_arguments)]
    fn try_emit_move_into_interval(
        &mut self,
        current: SearchKey,
        current_g_t: SimTime,
        current_cell: CellId,
        current_interval: SafeIntervalId,
        neighbor_id: CellId,
        required_orient: Orientation,
        traverse_cost: SimDuration,
        move_out_distance_mm: u64,
        free_start: SimTime,
        free_end: Option<SimTime>,
        entry_velocity: Velocity,
        _target_exit_v: Velocity,
        actual_exit_v: Velocity,
    ) {
        let m_in_start = std::cmp::max(free_start, current_g_t);
        let m_in_end = m_in_start.saturating_add(traverse_cost);

        // Per spec §4: Move action-ready time at neighbor (m_in_end) must
        // stay inside the planning horizon. This subsumes the older
        // wait-at-source horizon check, since m_in_end >= m_in_start.
        if m_in_end > self.horizon_t {
            self.stats.action_ready_past_horizon += 1;
            return;
        }

        // Lower bound from the shortest traversable outgoing link, omitting
        // any required rotation. Reusing the incoming length would falsely
        // reject a 135° arc followed by a shorter 45° exit. The actual successor
        // still validates its complete source hold and directed-edge window.
        let move_out_target = match self.drive.kind() {
            MotionKind::Kinematic => self.drive.max_speed(),
            MotionKind::Constant => Velocity::ZERO,
        };
        let move_out_cost = self.drive.kinematic_cost(actual_exit_v, move_out_target, move_out_distance_mm).0;

        // Implied minimum composed reservation window on neighbor.
        let implied_min_end = if neighbor_id == self.to {
            match self.config.dest_dwell {
                None => SimTime::EOT,
                Some(d) if d == SimDuration::MAX => SimTime::EOT,
                Some(d) => m_in_end.saturating_add(d),
            }
        } else {
            m_in_end.saturating_add(move_out_cost)
        };

        let fits_interval = match free_end {
            None => true,
            Some(end) => implied_min_end <= end,
        };
        if !fits_interval {
            return;
        }

        // Goal cell with open dwell needs an open-ended interval.
        if neighbor_id == self.to {
            let needs_open = matches!(self.config.dest_dwell, None) || self.config.dest_dwell == Some(SimDuration::MAX);
            if needs_open && free_end.is_some() {
                return;
            }
        }

        // Source-cell hold: physically the robot occupies the current cell
        // until the move completes (center-on-next-cell), not merely until
        // the move starts. Validate [current_g_t, m_in_end) against both the
        // SIPP interval and the live oracle so the commit plan cannot later
        // reject a source-cell close at m_in_end.
        let source_hold = if m_in_end > current_g_t { m_in_end - current_g_t } else { SimDuration::ZERO };
        if source_hold > SimDuration::ZERO {
            let interval_end_ok = current_interval.1.map(|ie| m_in_end <= ie).unwrap_or(true);
            if !interval_end_ok {
                self.stats.source_hold_blocked += 1;
                return;
            }
            self.stats.reservation_queries += 1;
            if self.oracle.earliest_available(
                current_cell,
                current_g_t,
                source_hold,
                self.config.exclusion_for(current_cell),
            ) != Some(current_g_t)
            {
                self.stats.source_hold_blocked += 1;
                return;
            }
        }

        // Exact admission check (debug_assert that locally-built window
        // matches what the oracle would admit).
        let check_dur = if neighbor_id == self.to {
            match self.config.dest_dwell {
                None => traverse_cost,
                Some(d) if d == SimDuration::MAX => traverse_cost,
                Some(d) => traverse_cost + d,
            }
        } else {
            traverse_cost + move_out_cost
        };
        self.stats.reservation_queries += 1;
        if self.oracle.earliest_available(neighbor_id, m_in_start, check_dur, self.config.exclusion_for(neighbor_id))
            != Some(m_in_start)
        {
            return;
        }
        debug_assert!(
            m_in_start >= free_start && free_end.map(|e| m_in_start < e).unwrap_or(true),
            "m_in_start {:?} not in safe interval ({:?}, {:?})",
            m_in_start,
            free_start,
            free_end,
        );

        // Edge validation: directed edge (current_cell → neighbor_id) must
        // be free over [m_in_start, m_in_end) for THIS robot's crossing.
        if self.oracle.has_edge_conflict(
            current_cell,
            neighbor_id,
            m_in_start,
            traverse_cost,
            Some(self.config.self_robot),
        ) {
            self.stats.edge_conflicts_rejected += 1;
            return;
        }

        let wait_at_source = if m_in_start > current_g_t { m_in_start - current_g_t } else { SimDuration::ZERO };
        if free_start <= current_g_t {
            self.stats.reservation_free += 1;
        } else {
            self.stats.reservation_delayed += 1;
        }

        let next_g = m_in_end - self.config.sim_start;
        let next_interval: SafeIntervalId = (free_start, free_end);
        let next_sipp = SippState {
            cell: neighbor_id,
            orientation: required_orient,
            interval: next_interval,
            speed_bucket: SpeedBucket::from_velocity(actual_exit_v),
        };
        let next_key = SearchKey::ordinary(next_sipp);
        let meta = NodeMeta {
            action_kind: NodeActionKind::Move,
            action_start: m_in_start,
            action_end: m_in_end,
            source_cell: Some(current_cell),
            interval: next_interval,
            entry_velocity,
            exit_velocity: actual_exit_v,
        };
        if self.relax(next_key, next_g, current, meta) {
            trace_emit!(self.pf.inner, Move(current_cell, neighbor_id, next_g, wait_at_source), self.oracle);
            self.stats.successors_emitted += 1;
            if wait_at_source > SimDuration::ZERO {
                self.stats.pause_nodes_emitted += 1;
            }
        }
    }
}
