//! Per-robot duty cycle accounting.
//!
//! Records the intervals a robot spent inside a scheduled task (a DA50's rail
//! legs from `VtmNav`, a D30's work from the intervals `activity` builds) and
//! attributes their durations to slices: a DA50's as its leg is read, a D30's
//! as B passes each slice (`activity`'s `slice_duty`). Idle is the implicit
//! complement: a gap in a robot's interval list is time in no task at all.
//!
//! Consumed by the observer to populate per-slice `duty_per_robot` totals,
//! cumulative `robot_duty_totals`, and (optionally) raw per-bot timelines.

use std::collections::HashMap;

use desgraph::RobotType;
use serde::{Deserialize, Serialize};

use crate::model::{RobotId, SimDuration, SimTime};
use crate::observer::activity::ActivityKind;
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::ScalarCounters;

/// A contiguous interval a robot spent doing one kind of work.
///
/// Idle is implicit: gaps between intervals (or before the first / after the
/// last) count as idle time. Recorded only when the `MetricsConfig`
/// `record_robot_timelines` flag is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusyInterval {
    pub start: SimTime,
    pub end: SimTime,
    pub kind: ActivityKind,
}

impl BusyInterval {
    pub fn duration(&self) -> SimDuration {
        self.end - self.start
    }
}

/// A contiguous operator-busy interval at a single station.
///
/// Stations are binary (busy / idle), so unlike `BusyInterval` no kind is
/// needed. Recorded for every `CycleCompleteEvent`; idle is
/// the implicit complement (gaps between intervals = operator-cell idle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationBusyInterval {
    pub start: SimTime,
    pub end: SimTime,
}

impl StationBusyInterval {
    pub fn duration(&self) -> SimDuration {
        self.end - self.start
    }
}

/// A robot-keyed map split into its two tracked fleets. Both maps cover the
/// FULL fleet — never-moved robots included — because they are built from the
/// `robot_types` roster rather than from the activity map (see
/// [`partition_by_fleet`]).
#[derive(Debug, Clone, Default)]
pub struct FleetMaps<T> {
    pub vtm: HashMap<RobotId, T>,
    pub htm: HashMap<RobotId, T>,
}

/// Partition a robot-keyed map by robot type over the WHOLE fleet.
///
/// `roster` — the observer's `robot_types`, seeded from `InitialPositions` — is
/// the source of truth for which robots exist. `source` (per-robot duty totals)
/// holds only robots that emitted activity, so iterating it would silently drop
/// robots that never moved and inflate fleet utilization. Instead we walk the
/// roster: every VTM/HTM gets an entry, taking its value from `source` when
/// present and `T::default()` (an all-idle `DutyBreakdown`) otherwise. RTPs and
/// any other type are skipped — no fleet metrics exist for them yet.
///
/// A robot present in `source` but absent from `roster` is ignored: we only
/// walk the roster, so an orphaned activity entry is never added. That means a
/// broken upstream seeding contract passes silently here. Callers that must
/// catch the breakage (the final summary) use [`try_partition_by_fleet`].
pub fn partition_by_fleet<T: Clone + Default>(
    source: &HashMap<RobotId, T>,
    roster: &HashMap<RobotId, RobotType>,
) -> FleetMaps<T> {
    let mut maps = FleetMaps::default();
    for (&robot_id, &robot_type) in roster {
        let bucket = match robot_type {
            RobotType::Vtm => &mut maps.vtm,
            RobotType::Htm => &mut maps.htm,
            _ => continue,
        };
        bucket.insert(robot_id, source.get(&robot_id).cloned().unwrap_or_default());
    }
    maps
}

/// [`partition_by_fleet`], but first verifies every robot in `source` is in the
/// `roster`. A robot with recorded activity but no roster entry means the
/// upstream `InitialPositions` seeding contract is broken; this returns that
/// robot's id as `Err` so the caller can surface a hard error rather than
/// silently dropping it. The final summary uses this (it runs on the UI thread
/// and prefers a clean error to a crash).
pub fn try_partition_by_fleet<T: Clone + Default>(
    source: &HashMap<RobotId, T>,
    roster: &HashMap<RobotId, RobotType>,
) -> Result<FleetMaps<T>, RobotId> {
    if let Some(&orphan) = source.keys().find(|id| !roster.contains_key(id)) {
        return Err(orphan);
    }
    Ok(partition_by_fleet(source, roster))
}

/// Full-fleet timeline partition for the Gantt view, refusing a roster orphan
/// as [`try_partition_by_fleet`] does.
///
/// It is gated on the timeline feature actually being active: when `source`
/// is empty (`record_robot_timelines` was off, so nothing recorded) it returns
/// empty maps, preserving the Gantt's "no timeline data yet" state. When any
/// timeline exists, every VTM/HTM gets a lane — never-moved robots get an
/// empty one — so the Gantt shows the whole fleet rather than only the robots
/// that moved.
pub fn try_partition_timelines_by_fleet(
    source: &HashMap<RobotId, Vec<BusyInterval>>,
    roster: &HashMap<RobotId, RobotType>,
) -> Result<FleetMaps<Vec<BusyInterval>>, RobotId> {
    if source.is_empty() {
        return Ok(FleetMaps::default());
    }
    try_partition_by_fleet(source, roster)
}

/// Time-integrated breakdown of a robot's activity by `ActivityKind`.
///
/// The field names are the exported metric keys (`htm_duty_traveling_ms` and
/// its siblings) that profiles, the xlsx sheets and pinned baselines are keyed
/// by: `traveling` is `Travel`, `loading_unloading` is `ForkOp`, `processing`
/// is `Presentation`.
///
/// Named fields (not a `HashMap`) so consumers don't have to handle missing
/// keys. A gap is implicit (`busy()` is the sum of the three below; the
/// caller computes idle = window − busy).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DutyBreakdown {
    pub traveling: SimDuration,
    pub loading_unloading: SimDuration,
    pub processing: SimDuration,
}

impl DutyBreakdown {
    /// Add `dur` to the bucket for `kind`. `Gap` is a no-op.
    pub fn add(&mut self, kind: ActivityKind, dur: SimDuration) {
        match kind {
            ActivityKind::Gap => {}
            ActivityKind::Travel => self.traveling += dur,
            ActivityKind::ForkOp => self.loading_unloading += dur,
            ActivityKind::Presentation => self.processing += dur,
        }
    }

    /// Sum of the time spent inside any interval.
    pub fn busy(&self) -> SimDuration {
        self.traveling + self.loading_unloading + self.processing
    }

    /// `busy / window` as a fraction in [0, 1]. Returns `0.0` when window is
    /// zero (avoids divide-by-zero on degenerate runs).
    pub fn duty_pct_global(&self, sim_elapsed: SimDuration) -> f64 {
        let total = sim_elapsed.as_ms();
        if total == 0 { 0.0 } else { self.busy().as_ms() as f64 / total as f64 }
    }
}

impl SimObserverState {
    /// Record one interval of a D30's work, once `activity` has built it, in
    /// the run's own accumulators: first/last-seen markers, cumulative
    /// `robot_duty_totals`, and the raw `robot_timelines` entry when
    /// `MetricsConfig::record_robot_timelines` is true. Its share of each
    /// slice is derived as B passes the slice, from the same work.
    ///
    /// Zero-duration intervals (`end <= start`) are dropped — they would be
    /// no-ops in every accumulator and pollute the timeline.
    pub(super) fn record_interval(&mut self, robot_id: RobotId, interval: BusyInterval) {
        if interval.end <= interval.start || interval.kind == ActivityKind::Gap {
            return;
        }
        self.record_markers_totals(robot_id, interval);
        if self.config.record_robot_timelines {
            self.output.robot_timelines.entry(robot_id).or_default().push(interval);
        }
    }

    /// First/last-seen markers and cumulative `robot_duty_totals`. Caller
    /// guarantees a non-empty interval of work, not a gap.
    fn record_markers_totals(&mut self, robot_id: RobotId, interval: BusyInterval) {
        self.output.robot_first_seen.entry(robot_id).or_insert(interval.start);
        self.output.robot_last_seen.insert(robot_id, interval.end);
        self.output.robot_duty_totals.entry(robot_id).or_default().add(interval.kind, interval.duration());
    }

    /// Record one VTM leg's two clocks (see [`VtmNavEventData`]): the instant
    /// window feeds the wall-clock-bounded surfaces (first/last-seen,
    /// per-slice duty + fleet aggregates, interim `robot_duty_totals`); the
    /// geometric window feeds the gantt timeline and, when the leg diverges
    /// (Teleport), the `vtm_geo_intervals` side-channel consumed by
    /// [`Self::finalize_vtm_union_duty`]. Identical to
    /// [`Self::record_interval`] when the windows are equal.
    ///
    /// [`VtmNavEventData`]: crate::events::VtmNavEventData
    pub(super) fn record_vtm_leg(&mut self, robot_id: RobotId, instant: BusyInterval, geo: BusyInterval) {
        debug_assert_eq!(instant.kind, geo.kind, "one leg, one kind: instant and geo must agree");
        let diverged = instant.start != geo.start || instant.end != geo.end;
        if instant.end > instant.start && instant.kind != ActivityKind::Gap {
            self.record_markers_totals(robot_id, instant);
            // The leg is read no earlier than B, so every slice it touches is
            // still open.
            let kind = instant.kind;
            self.for_each_slice_clip(instant.start, instant.end, |scalars, _slice_index, clipped| {
                book_slice_duty(scalars, robot_id, RobotType::Vtm, kind, clipped);
            });
            if !diverged && self.config.record_robot_timelines {
                self.output.robot_timelines.entry(robot_id).or_default().push(instant);
            }
        }
        if !diverged {
            return;
        }
        if geo.end > geo.start && geo.kind != ActivityKind::Gap {
            if self.config.record_robot_timelines {
                self.output.robot_timelines.entry(robot_id).or_default().push(geo);
            }
            self.vtm_geo_intervals.entry(robot_id).or_default().push(geo);
        }
    }

    /// Rewrite each side-channel VTM's duty from its geometric intervals
    /// (Teleport only): clamp to `[0, latest_sim_time]` (the duty
    /// denominator — the clamp is what caps duty at 100%), then `busy()` =
    /// union of the clamped windows, split across buckets in proportion to
    /// the per-kind sums. Replaces the interim instant-clock totals; call
    /// once at finalize.
    pub(super) fn finalize_vtm_union_duty(&mut self) {
        let horizon = self.latest_sim_time;
        let by_vtm = std::mem::take(&mut self.vtm_geo_intervals);
        for (robot_id, mut intervals) in by_vtm {
            // Clamp to the observed window; drop what lies wholly past it.
            for iv in &mut intervals {
                iv.end = iv.end.min(horizon);
            }
            intervals.retain(|iv| iv.end > iv.start);
            if intervals.is_empty() {
                continue;
            }
            intervals.sort_unstable_by_key(|iv| iv.start);

            // Union sweep + per-kind sums in one pass.
            let mut union = SimDuration::ZERO;
            let mut sums = DutyBreakdown::default();
            let mut open: Option<(SimTime, SimTime)> = None;
            for iv in &intervals {
                sums.add(iv.kind, iv.duration());
                match open {
                    None => open = Some((iv.start, iv.end)),
                    Some((os, oe)) if iv.start <= oe => open = Some((os, oe.max(iv.end))),
                    Some((os, oe)) => {
                        union += oe - os;
                        open = Some((iv.start, iv.end));
                    }
                }
            }
            if let Some((os, oe)) = open {
                union += oe - os;
            }

            let sum = sums.busy();
            if sum == SimDuration::ZERO {
                continue;
            }
            // Split the union across buckets in the per-kind sum proportions.
            // Bucket-agnostic so `busy()` == the union whatever kinds the DA50
            // was in (today only travel and fork operations).
            let scale = union.as_ms() as f64 / sum.as_ms() as f64;
            let scaled = |d: SimDuration| (d.as_ms() as f64 * scale).round() as u64;
            let mut trav = scaled(sums.traveling);
            let mut load = scaled(sums.loading_unloading);
            let mut proc = scaled(sums.processing);
            // Absorb rounding drift on the largest bucket so the three sum to the
            // union exactly and no real-but-small bucket is zeroed.
            let residual = union.as_ms() as i64 - (trav + load + proc) as i64;
            let largest = trav.max(load).max(proc);
            let adjust = |b: &mut u64| *b = (*b as i64 + residual).max(0) as u64;
            if trav == largest {
                adjust(&mut trav);
            } else if load == largest {
                adjust(&mut load);
            } else {
                adjust(&mut proc);
            }
            *self.output.robot_duty_totals.entry(robot_id).or_default() = DutyBreakdown {
                traveling: SimDuration::ms(trav),
                loading_unloading: SimDuration::ms(load),
                processing: SimDuration::ms(proc),
            };
        }
    }

    /// Walk every slice that `[start, end)` touches, clipping the window to
    /// each slice boundary, and invoke `f` with that slice's `ScalarCounters`
    /// and the clipped sub-duration, writing through `slice_to_write`. The
    /// single authoritative implementation of per-slice interval clipping —
    /// shared by duty accounting and the rotation carve-out so the two never
    /// drift apart. No-op for a zero/negative window.
    pub(super) fn for_each_slice_clip(
        &mut self,
        start: SimTime,
        end: SimTime,
        mut f: impl FnMut(&mut ScalarCounters, usize, SimDuration),
    ) {
        if end <= start {
            return;
        }
        let slice_width_ms = self.config.slice_width_ms;
        let start_slice_index = (start.as_ms() / slice_width_ms) as usize;
        let end_slice_index = (end.as_ms().saturating_sub(1) / slice_width_ms) as usize;
        for slice_index in start_slice_index..=end_slice_index {
            let slice_start = SimTime::ms((slice_index as u64) * slice_width_ms);
            let slice_end = SimTime::ms(((slice_index as u64) + 1) * slice_width_ms);
            let clipped = end.min(slice_end) - start.max(slice_start);
            f(&mut self.slice_to_write(slice_index).scalars, slice_index, clipped);
        }
    }

    /// Add an HTM duty overlay window `[start, end)` to `select`'s counter in
    /// each slice it overlaps, clipped to the slice: the rotation carve-out
    /// and the three loaded-time KPIs (`htm_duty_wait_loaded_ms`,
    /// `htm_duty_dwell_loaded_ms`, `htm_duty_pick_at_station_ms`) are all
    /// booked here, alike. Overlays intentionally overlap a coarse bucket
    /// and stay out of the HtmIdle busy-sum. No-op for a zero/negative window.
    pub(super) fn add_htm_duty_overlay(
        &mut self,
        start: SimTime,
        end: SimTime,
        select: impl Fn(&mut ScalarCounters) -> &mut u64,
    ) {
        if end <= start {
            return;
        }
        self.for_each_slice_clip(start, end, |scalars, _slice_index, clipped| {
            *select(scalars) += clipped.as_ms();
        });
    }
}

/// Book `share` of `kind` work by `robot_id` into one slice: its per-robot
/// `duty_per_robot` and the fleet aggregate (`*_duty_*_ms`) the line charts
/// read. RTPs and other untracked types populate the per-robot map but no
/// fleet aggregate; none exists for them yet.
pub(super) fn book_slice_duty(
    scalars: &mut ScalarCounters,
    robot_id: RobotId,
    robot_type: RobotType,
    kind: ActivityKind,
    share: SimDuration,
) {
    scalars.duty_per_robot.entry(robot_id.0).or_default().add(kind, share);
    bump_fleet_aggregate(scalars, robot_type, kind, share.as_ms());
}

/// Add `clipped_ms` to the slice's per-type, per-kind fleet aggregate.
/// Unknown robot types (e.g., RTP) or `Gap` are no-ops — the bookers filter
/// gaps, and fleet aggregates exist only for VTM/HTM today.
fn bump_fleet_aggregate(scalars: &mut ScalarCounters, robot_type: RobotType, kind: ActivityKind, clipped_ms: u64) {
    match (robot_type, kind) {
        (RobotType::Vtm, ActivityKind::Travel) => scalars.vtm_duty_traveling_ms += clipped_ms,
        (RobotType::Vtm, ActivityKind::ForkOp) => scalars.vtm_duty_loading_unloading_ms += clipped_ms,
        (RobotType::Htm, ActivityKind::Travel) => scalars.htm_duty_traveling_ms += clipped_ms,
        (RobotType::Htm, ActivityKind::ForkOp) => scalars.htm_duty_loading_unloading_ms += clipped_ms,
        (RobotType::Htm, ActivityKind::Presentation) => scalars.htm_duty_processing_ms += clipped_ms,
        _ => {}
    }
}

#[cfg(test)]
mod fleet_partition_tests {
    use super::*;

    fn moved(ms: u64) -> DutyBreakdown {
        DutyBreakdown { traveling: SimDuration::ms(ms), ..Default::default() }
    }

    #[test]
    fn duty_partition_covers_never_moved_robots() {
        // Roster: 2 HTMs + 1 VTM. Only HTM 0 recorded activity.
        let roster =
            HashMap::from([(RobotId(0), RobotType::Htm), (RobotId(1), RobotType::Htm), (RobotId(2), RobotType::Vtm)]);
        let duty = HashMap::from([(RobotId(0), moved(5_000))]);

        let maps = partition_by_fleet(&duty, &roster);

        // Both HTMs present; the never-moved one defaults to all-idle.
        assert_eq!(maps.htm.len(), 2, "full HTM fleet, not just movers");
        assert_eq!(maps.htm[&RobotId(0)].busy(), SimDuration::ms(5_000));
        assert_eq!(maps.htm[&RobotId(1)], DutyBreakdown::default(), "never-moved HTM is all-idle");
        assert_eq!(maps.vtm.len(), 1);
        assert_eq!(maps.vtm[&RobotId(2)], DutyBreakdown::default());
    }

    #[test]
    fn timeline_partition_gated_on_feature() {
        let roster = HashMap::from([(RobotId(0), RobotType::Htm), (RobotId(1), RobotType::Htm)]);

        // Feature off (no timelines recorded): stays empty so the Gantt shows
        // its "no data" state rather than fabricated idle lanes.
        let empty: HashMap<RobotId, Vec<BusyInterval>> = HashMap::new();
        assert!(try_partition_timelines_by_fleet(&empty, &roster).expect("no orphan").htm.is_empty());

        // Feature on (HTM 0 has a lane): the never-moved HTM 1 gets an empty
        // lane so the Gantt renders the whole fleet.
        let recorded = HashMap::from([(
            RobotId(0),
            vec![BusyInterval { start: SimTime::ZERO, end: SimTime::ms(1_000), kind: ActivityKind::Travel }],
        )]);
        let maps = try_partition_timelines_by_fleet(&recorded, &roster).expect("no orphan");
        assert_eq!(maps.htm.len(), 2);
        assert_eq!(maps.htm[&RobotId(0)].len(), 1);
        assert!(maps.htm[&RobotId(1)].is_empty(), "never-moved HTM gets an empty lane, not a dropped one");
    }

    #[test]
    fn rtp_robots_land_in_neither_fleet() {
        // RTPs have no duty metrics yet — they must be skipped, even when they
        // recorded activity, and must not trip the orphan check.
        let roster = HashMap::from([(RobotId(0), RobotType::Htm), (RobotId(1), RobotType::Rtp)]);
        let duty = HashMap::from([(RobotId(0), moved(5_000)), (RobotId(1), moved(9_000))]);

        let maps = try_partition_by_fleet(&duty, &roster).expect("RTP in roster is not an orphan");

        assert_eq!(maps.htm.len(), 1);
        assert!(maps.vtm.is_empty());
        assert!(!maps.htm.contains_key(&RobotId(1)), "RTP is excluded from the HTM fleet");
    }

    #[test]
    fn try_partition_flags_activity_without_roster_entry() {
        // HTM 1 recorded activity but is absent from the roster — a broken
        // InitialPositions seeding contract. The fallible variant surfaces it.
        let roster = HashMap::from([(RobotId(0), RobotType::Htm)]);
        let duty = HashMap::from([(RobotId(0), moved(5_000)), (RobotId(1), moved(3_000))]);

        assert_eq!(try_partition_by_fleet(&duty, &roster).unwrap_err(), RobotId(1));

        // The infallible variant drops the orphan silently instead.
        let maps = partition_by_fleet(&duty, &roster);
        assert_eq!(maps.htm.len(), 1);
        assert!(!maps.htm.contains_key(&RobotId(1)));
    }
}
