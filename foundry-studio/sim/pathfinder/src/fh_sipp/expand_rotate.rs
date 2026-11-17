//! Rotate-successor expansion. Same cell, same safe interval (a rotate
//! doesn't leave the cell, so the interval admitting `current` admits the
//! rotated state). Validates the rotation duration fits the source
//! interval and the cell is free for the rotate window.

use desgraph::types::CellId;

use super::super::Orientation;
#[cfg(any(feature = "pathfinder-trace", test))]
use super::super::pathfinder_trace::prelude::*;
use super::super::pathfinder_trace::trace_emit;
use super::expand::UsefulOrients;
use super::node::{NodeActionKind, NodeMeta};
use super::pathfinder::{Inner, rotation_permitted};
use super::search::Search;
use super::state::{SafeIntervalId, SearchKey, SippState, SpeedBucket};
use crate::{SimDuration, SimTime, Velocity};

impl Search<'_> {
    pub(super) fn try_rotate_successors(
        &mut self,
        current: SearchKey,
        current_g: SimDuration,
        current_g_t: SimTime,
        current_cell: CellId,
        current_orient: Orientation,
        current_interval: SafeIntervalId,
        useful_orients: UsefulOrients,
    ) {
        if !rotation_permitted(self.pf.inner.graph(), current_cell, self.drive) {
            return;
        }
        // Kinematic constraint: an HTM in HtmKinematic mode cannot rotate
        // while moving — it must reach SpeedBucket::ZERO first. The search
        // produces the explicit deceleration successor (target_exit_v = 0)
        // in expand_move; this gate prevents emitting impossible rotations
        // off non-zero-velocity states. VTMs and ConstantSpeed mode are
        // unaffected (their states stay at ZERO anyway).
        if current.sipp.speed_bucket != SpeedBucket::ZERO {
            return;
        }

        for orient in useful_orients.iter() {
            let Some(turn) = Inner::rotate_cost(self.drive, current_orient, orient) else {
                continue;
            };
            let action_start = current_g_t;
            let action_end = action_start.saturating_add(turn);
            // Per spec §4: rotate action-ready time must stay inside the
            // planning horizon.
            if action_end > self.horizon_t {
                self.stats.action_ready_past_horizon += 1;
                continue;
            }
            let interval_end_ok = current_interval.1.map(|ie| action_end <= ie).unwrap_or(true);
            if !interval_end_ok {
                self.stats.source_hold_blocked += 1;
                trace_emit!(
                    self.pf.inner,
                    RotateBlocked {
                        cell: current_cell,
                        from_orient: current_orient,
                        to_orient: orient,
                        hold_start: action_start,
                        hold_needed: turn,
                    },
                    self.oracle
                );
                continue;
            }
            self.stats.reservation_queries += 1;
            if self.oracle.earliest_available(current_cell, action_start, turn, self.config.exclusion_for(current_cell))
                != Some(action_start)
            {
                self.stats.source_hold_blocked += 1;
                continue;
            }
            let next_g = current_g + turn;
            let next_sipp = SippState {
                cell: current_cell,
                orientation: orient,
                interval: current_interval,
                speed_bucket: SpeedBucket::ZERO,
            };
            let next_key = SearchKey::ordinary(next_sipp);
            let meta = NodeMeta {
                action_kind: NodeActionKind::Rotate,
                action_start,
                action_end,
                source_cell: None,
                interval: current_interval,
                entry_velocity: Velocity::ZERO,
                exit_velocity: Velocity::ZERO,
            };
            if self.relax(next_key, next_g, current, meta) {
                trace_emit!(
                    self.pf.inner,
                    Rotate(current_cell, current_orient, orient, next_g, SimDuration::ZERO),
                    self.oracle
                );
                self.stats.successors_emitted += 1;
            }
        }
    }
}
