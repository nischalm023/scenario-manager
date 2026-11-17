//! Event-driven Wait-successor expansion. Per spec §4 "Wait successor
//! (event-driven)" + FHSippPathfinder draft §Event-Driven Wait Successors.
//!
//! Emitted only from ordinary SIPP states (`wait_label = None`); from a
//! wait state, an even-later wait is reachable directly from the original
//! predecessor and would dedup to the same key with equal-or-larger `g`.
//! Skipping wait-from-wait keeps the per-state set bounded by
//! `goal_event_times.len()`.

use super::node::{NodeActionKind, NodeMeta};
use super::search::Search;
use super::state::{SafeIntervalId, SearchKey, SpeedBucket};
use crate::{SimTime, Velocity};

impl Search<'_> {
    pub(super) fn try_wait_successors(
        &mut self,
        current: SearchKey,
        current_g_t: SimTime,
        current_interval: SafeIntervalId,
    ) {
        if current.wait_label.is_some() || self.goal_event_times.is_empty() {
            return;
        }
        // Kinematic constraint: an HTM in HtmKinematic mode cannot wait
        // (which is a stationary action) while still in motion. The bot
        // must reach SpeedBucket::ZERO via the explicit decelerate-to-stop
        // successor in expand_move before waiting becomes available.
        if current.sipp.speed_bucket != SpeedBucket::ZERO {
            return;
        }
        // Iterate a snapshot: the loop body calls `relax` (&mut self), which
        // conflicts with borrowing `self.goal_event_times` for the iterator.
        let goal_events = self.goal_event_times.clone();
        for &event_time in &goal_events {
            self.stats.wait_events_considered += 1;
            if event_time <= current_g_t {
                continue;
            }
            if event_time > self.horizon_t {
                self.stats.wait_successors_rejected_horizon += 1;
                continue;
            }
            if let Some(end) = current_interval.1 {
                if event_time > end {
                    self.stats.wait_successors_rejected_source_interval += 1;
                    continue;
                }
            }
            let next_g = event_time - self.config.sim_start;
            let next_key = SearchKey { sipp: current.sipp, wait_label: Some(event_time) };
            let meta = NodeMeta {
                action_kind: NodeActionKind::Wait,
                action_start: current_g_t,
                action_end: event_time,
                source_cell: None,
                interval: current_interval,
                entry_velocity: Velocity::ZERO,
                exit_velocity: Velocity::ZERO,
            };
            if self.relax(next_key, next_g, current, meta) {
                self.stats.successors_emitted += 1;
                self.stats.wait_successors_emitted += 1;
            }
        }
    }
}
