//! Within `reservation`, `table` is the LIVE RECORD of who has claimed which
//! cell, and which directed edge, over which stretch of time.
//!
//! Every claim in the simulator is made here and nowhere else: a planner
//! searches against the safe intervals this table reports and then commits a
//! whole path into it under one write lock, so a plan is admitted entirely or
//! refused entirely. Two rules give the structure its shape. A robot's last
//! reservation stays open-ended until the plan that moves it away closes it,
//! which is how a standing robot keeps its cell without anyone having to know
//! in advance how long it will stand there. And a commit validates everything
//! before it changes anything — plan shape, then the predecessor's hold, then
//! each cell, then each edge — because a half-applied path would leave two
//! robots believing they own one square.

use std::future::Future;
use std::sync::Arc;

use parking_lot::RwLock;
use parking_lot::RwLockReadGuard;
use rustc_hash::FxHashMap;

use desgraph::CellId;

use crate::RobotId;
use crate::{CommitPlan, ReservationRequest};

use super::accumulator::CellDutyAggregate;
use super::cell_reservation::{CellReservation, ReservationId};
use super::commit_wake::CommitWake;
use super::diagnostics::CellDutyStats;
use crate::{SimDuration, SimTime, TimeRange};

/// Conflict found during reservation commit — diagnostic info.
#[derive(Debug)]
pub struct ReservationConflict {
    pub cell: CellId,
    pub proposed: TimeRange,
    pub existing_robot: RobotId,
    pub existing_window: TimeRange,
}

/// A single reservation operation for the trace log.
#[derive(Clone, Debug)]
pub enum TraceOp {
    /// A cell window was committed (upserted) with a fresh id.
    Commit { id: ReservationId, cell: CellId, robot: RobotId, window: TimeRange },
    /// An open-ended reservation had its end finalized.
    Finalize { id: ReservationId, cell: CellId, robot: RobotId, new_window: TimeRange },
    /// An open-ended reservation's start moved right (destination-row
    /// subsume: the owner's committed path landed at its actual arrival).
    Subsume { id: ReservationId, cell: CellId, robot: RobotId, new_start: SimTime },
    // Release/ClearRobot variants intentionally omitted — we're testing the
    // theory that those operations aren't needed under the new model.
}

/// A reservation on a directed edge (from→to) for a time window.
/// Prevents head-on swaps where bot A traverses X→Y while bot B
/// traverses Y→X at the same instant.
#[derive(Clone, Debug)]
struct EdgeReservation {
    robot: RobotId,
    window: TimeRange,
}

/// Per-cell reservations, struct-of-arrays. Hot-path binary searches probe the
/// dense `u32` key arrays instead of the 40-byte `CellReservation`, so a
/// ≤16-entry list searches within one 64-byte cache line. The three vecs are
/// index-aligned and same-length; mutations keep them in lockstep and
/// `assert_lane_invariants` is the debug-build check.
#[derive(Default)]
struct CellLane {
    /// End-ms keys, sorted ascending; open-ended (`window.end == None`) →
    /// `u32::MAX`. Search key for `has_conflict` / `earliest_free` /
    /// `robot_has_reservation`. Sorted because reservations are non-overlapping
    /// with a gap (`end[i] <= start[i+1]`) and open-ended is always last.
    ends: Vec<u32>,
    /// Start-ms keys, sorted ascending. Search key for `entry_intervals` and
    /// for sorted insertion.
    starts: Vec<u32>,
    /// Full reservations, index-aligned with `starts`/`ends`. Touched only in
    /// the post-search scan and in standalone returns.
    res: Vec<CellReservation>,
}

/// `end` key for open-ended reservations — a value no finite end can take, so
/// it's an unambiguous sentinel that also sorts last (`u32::MAX` ms ≈ 49.7 days).
const OPEN_ENDED_END_KEY: u32 = u32::MAX;

/// Sim time → `u32` ms search key. `u32::MAX` ms ≈ 49.7 days, beyond any
/// warehouse sim; the debug-assert guards against a silent truncation that
/// would corrupt the binary search.
#[inline]
fn ms_to_key(t: SimTime) -> u32 {
    let ms = t.as_ms();
    debug_assert!(ms < OPEN_ENDED_END_KEY as u64, "reservation time {ms}ms exceeds u32 key range (~49.7 days)");
    ms as u32
}

/// `end` key for a window: finite → `ms_to_key`, open-ended → the sentinel
/// (never `<= X`, so open-ended is never skipped by an `ends` search).
#[inline]
fn end_key(window: &TimeRange) -> u32 {
    window.end().map(ms_to_key).unwrap_or(OPEN_ENDED_END_KEY)
}

/// Query bound → `u32` ms key, saturating so an out-of-range bound still
/// compares monotonically (always in range in practice).
#[inline]
fn query_key(t: SimTime) -> u32 {
    t.as_ms().min(u32::MAX as u64) as u32
}

/// Entries to probe from the tail before falling back to a full binary search.
/// Forward-from-now queries almost always land in the recent tail of the key
/// array (which grows unboundedly over a long sim with no prune), so probing a
/// small contiguous window avoids scattered cold-cache probes across the whole
/// array. 75 is comfortable headroom over the observed working set
/// (cencora_phase2: per-cell p90 ≈ 16, frontier lead ≲ 63 s).
const TAIL_HINT: usize = 75;

/// `keys.partition_point(|&k| k <= bound)` over a sorted slice, biased to the
/// recent tail: if the target is within the last `TAIL_HINT` entries, search
/// only that contiguous (cache-friendly) window; otherwise full search. Result
/// is identical to `partition_point` — the tail probe is sound because the
/// slice is sorted ascending (`keys[lo] <= bound` ⇒ all of `[0, lo]` qualify).
#[inline]
fn tail_hinted_partition(keys: &[u32], bound: u32) -> usize {
    let lo = keys.len().saturating_sub(TAIL_HINT);
    if lo > 0 && keys[lo] <= bound {
        lo + keys[lo..].partition_point(|&k| k <= bound)
    } else {
        keys.partition_point(|&k| k <= bound)
    }
}

struct Inner {
    reservations: FxHashMap<CellId, CellLane>,
    /// Directed edge reservations: key = (from, to), prevents counter-flow.
    edge_reservations: FxHashMap<(CellId, CellId), Vec<EdgeReservation>>,
    /// Per-cell aggregate of finalized intervals that have been
    /// dropped by `prune_finalized_below_watermark`. Combined with a
    /// walk of the still-live list at report time to produce
    /// `CellDutyStats` — see `accumulator.rs`. The hot path
    /// (`create_reservation` / `finalize`) does NOT touch this; it's
    /// only updated when entries are pruned.
    dropped_stats: FxHashMap<CellId, CellDutyAggregate>,
    gap: SimDuration,
    next_reservation_id: u64,
    trace: Vec<TraceOp>,
    trace_enabled: bool,
}

impl Inner {
    /// Verify the `CellLane` invariants that the hot-path binary searches
    /// depend on. Mutation sites (`create_reservation`, `finalize`, `prune*`)
    /// are responsible for preserving them; this is a query-time sanity check.
    ///
    /// 1. `starts.len() == ends.len() == res.len()`.
    /// 2. `starts` non-decreasing; `ends` non-decreasing.
    /// 3. Each key array mirrors the structs — `starts[i] ==
    ///    ms_to_key(res[i].start)` and `ends[i] == end_key(res[i])`. Catches
    ///    both desync (an out-of-lockstep mutation) and truncation.
    ///
    /// Wrapped in `debug_assertions` so release/perf builds pay nothing.
    #[cfg(debug_assertions)]
    #[inline]
    fn assert_lane_invariants(lane: &CellLane, cell: CellId, caller: &'static str) {
        let n = lane.res.len();
        assert!(
            lane.starts.len() == n && lane.ends.len() == n,
            "{caller}: cell {:?} lane length mismatch — starts={}, ends={}, res={}",
            cell,
            lane.starts.len(),
            lane.ends.len(),
            n,
        );
        for i in 1..n {
            assert!(
                lane.starts[i - 1] <= lane.starts[i],
                "{caller}: cell {:?} starts not sorted — starts[{}]={} > starts[{}]={}",
                cell,
                i - 1,
                lane.starts[i - 1],
                i,
                lane.starts[i],
            );
            assert!(
                lane.ends[i - 1] <= lane.ends[i],
                "{caller}: cell {:?} ends not sorted — ends[{}]={} > ends[{}]={}",
                cell,
                i - 1,
                lane.ends[i - 1],
                i,
                lane.ends[i],
            );
        }
        for i in 0..n {
            let window = lane.res[i].window();
            assert!(
                lane.starts[i] == ms_to_key(window.start()),
                "{caller}: cell {:?} starts[{}]={} desync from res start {:?}",
                cell,
                i,
                lane.starts[i],
                window.start(),
            );
            assert!(
                lane.ends[i] == end_key(&window),
                "{caller}: cell {:?} ends[{}]={} desync from res end {:?}",
                cell,
                i,
                lane.ends[i],
                window.end(),
            );
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn assert_lane_invariants(_lane: &CellLane, _cell: CellId, _caller: &'static str) {}

    /// Sorted-by-start invariant check for the edge-reservation lists.
    /// `upsert_edge` is responsible for sorted insertion.
    #[cfg(debug_assertions)]
    #[inline]
    fn assert_edges_sorted_by_start(list: &[EdgeReservation], key: (CellId, CellId), caller: &'static str) {
        if let Some(idx) = list.windows(2).position(|w| w[0].window.start() > w[1].window.start()) {
            panic!(
                "{caller}: edge {:?} reservations not sorted by start — \
                 idx {} (robot={:?}, start={:?}) > idx {} (robot={:?}, start={:?})",
                key,
                idx,
                list[idx].robot,
                list[idx].window.start(),
                idx + 1,
                list[idx + 1].robot,
                list[idx + 1].window.start(),
            );
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn assert_edges_sorted_by_start(_list: &[EdgeReservation], _key: (CellId, CellId), _caller: &'static str) {}

    fn has_conflict(
        &self,
        cell: CellId,
        proposed: TimeRange,
        predecessor: Option<&CellReservation>,
    ) -> Option<ReservationConflict> {
        let lane = self.reservations.get(&cell)?;
        Self::assert_lane_invariants(lane, cell, "has_conflict");
        let exclude_id = predecessor.map(|r| r.id());
        // Binary search the `ends` key array: skip reservations that end before
        // proposed starts. Open-ended ends are `u32::MAX`, never `<= start`, so
        // they're never skipped — matching the old `None => false` predicate.
        let proposed_start_key = query_key(proposed.start());
        let start_idx = tail_hinted_partition(&lane.ends, proposed_start_key);
        for r in &lane.res[start_idx..] {
            if Some(r.id()) == exclude_id {
                continue;
            }
            // Sorted by start: if this starts after proposed ends, all later do too.
            // Open-ended proposed (end=None) → no early exit (overlaps with everything).
            if let Some(proposed_end) = proposed.end() {
                if r.window().start() >= proposed_end.saturating_add(self.gap) {
                    break;
                }
            }
            if r.window().overlaps(proposed, self.gap) {
                return Some(ReservationConflict {
                    cell,
                    proposed,
                    existing_robot: r.robot(),
                    existing_window: r.window(),
                });
            }
        }
        None
    }

    /// Drop finite reservations whose `end < watermark_ms`. Open-ended
    /// reservations are unconditionally retained (they have no end).
    /// Empty per-cell lists are dropped from the map.
    ///
    /// Safety: the only past read in any query path is `entry_intervals`'s
    /// one-step backward lookup for the most-recent finite end, bounded
    /// by `self.gap`. Callers pick `watermark_ms = now - max(gap, margin)`
    /// so that bounded backward read stays inside the kept range.
    ///
    /// Each dropped finite interval is folded into the cell's
    /// `dropped_stats` aggregate before removal so `cell_duty_stats` can
    /// recover the full per-cell stats at report time by combining the
    /// aggregate with a walk of the still-live list.
    fn prune_finalized_below_watermark(&mut self, watermark_ms: u64) {
        let dropped_stats = &mut self.dropped_stats;
        self.reservations.retain(|cell, lane| {
            // Retain all three arrays in lockstep. `Vec::retain` over `res`
            // can't drive the parallel key arrays, so we rebuild from a
            // keep-mask walked in index order (cheap; prune is amortized rare).
            let mut keep_idx = 0usize;
            for i in 0..lane.res.len() {
                let window = lane.res[i].window();
                match window.end() {
                    // Finite and below the watermark: fold into the per-cell
                    // aggregate, then let compaction drop it.
                    Some(end) if end.as_ms() < watermark_ms => {
                        dropped_stats.entry(*cell).or_default().ingest_finalized(window.start().as_ms(), end.as_ms());
                    }
                    // Kept (open-ended, or ending at/after the watermark):
                    // compact in place toward `keep_idx`.
                    _ => {
                        if keep_idx != i {
                            lane.starts[keep_idx] = lane.starts[i];
                            lane.ends[keep_idx] = lane.ends[i];
                            lane.res.swap(keep_idx, i);
                        }
                        keep_idx += 1;
                    }
                }
            }
            lane.starts.truncate(keep_idx);
            lane.ends.truncate(keep_idx);
            lane.res.truncate(keep_idx);
            !lane.res.is_empty()
        });
        self.edge_reservations.retain(|_, list| {
            list.retain(|r| r.window.end().map(|e| e.as_ms() >= watermark_ms).unwrap_or(true));
            !list.is_empty()
        });
    }

    fn earliest_free(
        &self,
        cell: CellId,
        earliest: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        let Some(lane) = self.reservations.get(&cell) else { return Some(earliest) };
        Self::assert_lane_invariants(lane, cell, "earliest_free");
        let exclude_id = predecessor.map(|r| r.id());
        let earliest_key = query_key(earliest);
        let start_idx = tail_hinted_partition(&lane.ends, earliest_key);
        let mut candidate = earliest;
        loop {
            let proposed_end = candidate.saturating_add(duration);
            let proposed = TimeRange::new(candidate, proposed_end);
            let mut blocked = false;
            for r in &lane.res[start_idx..] {
                if Some(r.id()) == exclude_id {
                    continue;
                }
                if r.window().start() >= proposed_end.saturating_add(self.gap) {
                    break;
                }
                if r.window().overlaps(proposed, self.gap) {
                    // Open-ended blocker: the cell never frees again.
                    let Some(end) = r.window().end() else { return None };
                    candidate = end;
                    blocked = true;
                    break;
                }
            }
            if !blocked {
                return Some(candidate);
            }
        }
    }

    /// Canonical safe intervals on `cell` overlapping `[after, horizon]`.
    /// Returns `(free_start, free_end)` tuples; trailing open interval has
    /// `free_end = None`. Identity is the physical interval — NOT clipped by
    /// `after`.
    ///
    /// Gap semantics match `TimeRange::overlaps`: a free interval starts
    /// `gap` after the prior reservation's end.
    fn entry_intervals(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
    ) -> Vec<(SimTime, Option<SimTime>)> {
        let mut out = Vec::new();
        self.entry_intervals_into(cell, after, horizon, predecessor, &mut out);
        out
    }

    /// Same as `entry_intervals` but writes results into a caller-provided
    /// `out` buffer. `out` is `clear()`-ed before any work; the caller can
    /// reuse the same `Vec` across many queries to avoid per-call heap
    /// allocation (profile attributed the per-call drop at 1.9% of total
    /// wall time).
    fn entry_intervals_into(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
        out: &mut Vec<(SimTime, Option<SimTime>)>,
    ) {
        out.clear();
        let Some(lane) = self.reservations.get(&cell) else {
            out.push((SimTime::ZERO, None));
            return;
        };
        Self::assert_lane_invariants(lane, cell, "entry_intervals");
        let exclude_id = predecessor.map(|r| r.id());

        // Binary-search past the "all in the past" prefix over the `starts`
        // key array. Reservations with `start <= after` never push a gap (the
        // original gap-overlaps-query check needs `res_start > after`), so we
        // only need their cumulative effect on `free_start`. With this skip,
        // per-call work becomes O(log N + K) where K is the number of
        // reservations overlapping [after, horizon].
        let after_key = query_key(after);
        let i_after = tail_hinted_partition(&lane.starts, after_key);

        // Initial `free_start` = (last non-excluded finite reservation in the
        // past prefix).end + gap. By non-overlap + sorted-by-start, this is
        // the largest-end among non-excluded finite reservations in
        // `[0..i_after]`, reachable in O(1) amortized by walking back from
        // `i_after - 1` and stopping at the first non-excluded entry.
        //
        // A non-excluded open-ended entry in the past prefix occupies
        // [start, ∞), so nothing on this cell is ever free: leave `out` empty.
        let mut free_start = SimTime::ZERO;
        for r in lane.res[..i_after].iter().rev() {
            if Some(r.id()) == exclude_id {
                continue;
            }
            let Some(end) = r.window().end() else { return };
            free_start = end.saturating_add(self.gap);
            break;
        }

        // `free_start` is monotonically non-decreasing through the rest of
        // the loop; once it exceeds `horizon`, neither a gap push nor a
        // trailing-open push can succeed.
        if free_start > horizon {
            return;
        }

        // Reserve capacity for the worst case. `Vec::reserve` is a no-op
        // when the buffer already has enough headroom, so reuse across
        // calls keeps capacity at the steady-state max for free.
        out.reserve(lane.res.len().saturating_sub(i_after) + 1);

        // Walk the remaining (start > after) reservations. In this slice
        // `res_start > after` is guaranteed, so the original
        // `gap_overlaps_query` reduces to just `free_start <= horizon`.
        for r in lane.res[i_after..].iter().filter(|r| Some(r.id()) != exclude_id) {
            let res_start = r.window().start();
            if free_start < res_start && free_start <= horizon {
                out.push((free_start, Some(res_start)));
            }
            let Some(end) = r.window().end() else {
                return;
            };
            let next_free = end.saturating_add(self.gap);
            if next_free > free_start {
                free_start = next_free;
            }
            if free_start > horizon {
                return;
            }
        }

        if free_start <= horizon {
            out.push((free_start, None));
        }
    }

    /// Check if the REVERSE edge (to→from) is reserved during `window`.
    /// Edges are robot-scoped — pass the acting robot to exclude its own direction.
    fn has_edge_conflict(&self, from: CellId, to: CellId, proposed: TimeRange, acting_robot: Option<RobotId>) -> bool {
        let reverse = (to, from);
        let Some(list) = self.edge_reservations.get(&reverse) else { return false };
        Self::assert_edges_sorted_by_start(list, reverse, "has_edge_conflict");
        // Binary-search past edges that ended before proposed started.
        let start_idx = list.partition_point(|r| match r.window.end() {
            Some(end) => end <= proposed.start(),
            None => false,
        });
        for r in &list[start_idx..] {
            if Some(r.robot) == acting_robot {
                continue;
            }
            // Sorted by start: if this edge starts after proposed ends (+ gap),
            // all later edges do too.
            if let Some(proposed_end) = proposed.end() {
                if r.window.start() >= proposed_end.saturating_add(self.gap) {
                    break;
                }
            }
            if r.window.overlaps(proposed, self.gap) {
                return true;
            }
        }
        false
    }

    // Intentionally no release_window: testing the theory that we never need
    // to release a cell reservation outside of finalize (setting an end on
    // an open-ended reservation). If this turns out to be wrong, we'll add
    // one back with an explicit purpose.

    /// Finalize a reservation by setting its end, returning the updated row.
    /// Errs when the live table no longer holds `predecessor`'s row, when the
    /// row belongs to another robot, or when `new_window` would widen it.
    ///
    /// TODO(factoring): consider moving this onto `CellReservation` itself
    /// (with an Arc back to oracle internals) so the call site reads
    /// `predecessor.finalize(...)` rather than `table.finalize(&predecessor, ...)`.
    /// Pros: self-describing, could pair with an RAII `OpenEndedReservation`
    /// handle type. Cons: CellReservation stops being a pure value type —
    /// every clone bumps an Arc refcount, and the TTP pipeline's
    /// Box<dyn Any> / Mutex<Option<_>> handoffs become more entangled. For
    /// now we rely on an end-of-sim scan for leaked open-ended reservations.
    fn finalize(
        &mut self,
        predecessor: &CellReservation,
        new_window: TimeRange,
    ) -> Result<CellReservation, ReservationConflict> {
        let cell = predecessor.cell();
        let vanished = ReservationConflict {
            cell,
            proposed: new_window,
            existing_robot: predecessor.robot(),
            existing_window: predecessor.window(),
        };
        let Some(lane) = self.reservations.get_mut(&cell) else { return Err(vanished) };
        let Some(pos) = lane.res.iter().position(|r| r.id() == predecessor.id()) else { return Err(vanished) };
        // The new window must narrow (or equal) the live one: open-ended →
        // finite (normal), finite → narrower finite (station dwell). Widening,
        // re-opening, or a live row owned by another robot means the search
        // and the commit disagree — the same drift `reserve_path`'s validation
        // phases report, so it is reported the same way.
        let old_window = lane.res[pos].window();
        let narrows = match (old_window.end(), new_window.end()) {
            (Some(existing), Some(proposed)) => proposed <= existing,
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !narrows || lane.res[pos].robot() != predecessor.robot() {
            return Err(ReservationConflict {
                cell,
                proposed: new_window,
                existing_robot: lane.res[pos].robot(),
                existing_window: old_window,
            });
        }
        // Remove from all three arrays and re-insert at the start-sorted
        // position to preserve sort order if window.start changes. (In practice
        // start stays the same and the position is stable; defensive anyway.)
        // The end narrows, so the `ends` array stays sorted after reinsertion
        // (`end[i] <= start[i+1] <= end[i+1]` still holds for a narrowed end).
        lane.starts.remove(pos);
        lane.ends.remove(pos);
        let mut reservation = lane.res.remove(pos);
        reservation.set_window(new_window);
        let new_start_key = ms_to_key(new_window.start());
        let insert_at = lane.starts.partition_point(|&start| start < new_start_key);
        lane.starts.insert(insert_at, new_start_key);
        lane.ends.insert(insert_at, end_key(&new_window));
        lane.res.insert(insert_at, reservation.clone());
        if self.trace_enabled {
            self.trace.push(TraceOp::Finalize {
                id: reservation.id(),
                cell: reservation.cell(),
                robot: reservation.robot(),
                new_window,
            });
        }
        Ok(reservation)
    }

    /// Move an open-ended reservation's start RIGHT, preserving its id
    /// and open end — the destination-row subsume (task-model-v2 temporal
    /// model: a dwell/park destination is promoted at its not-earlier-than
    /// arrival; the owner's committed path later lands at the actual
    /// arrival and subsumes the row rather than closing and recreating
    /// it, so the claim's identity survives). Refusing `new_start` left
    /// of the current start IS the arrive-after guard: a committed
    /// landing earlier than the promoted start means the estimate/
    /// schedule query pair diverged — a bug, panicked here with the sim
    /// harness's disposition.
    ///
    /// TODO(zero-panic): that panic disposition is repealed. These guards
    /// become `Err(ReservationConflict)` the way `finalize`'s did, which also
    /// rewrites the two `#[should_panic]` cases in
    /// `model::reservation::test::table` — `subsume_refuses_leftward_motion`
    /// and `subsume_refuses_bounded_rows`.
    ///
    /// The lane's sort invariants survive without repositioning: an
    /// open-ended row is necessarily the lane's last entry (nothing can
    /// start after it without overlapping), and moving its start right
    /// keeps it there.
    fn subsume_start_right(&mut self, row: &CellReservation, new_start: SimTime) -> CellReservation {
        let cell = row.cell();
        let lane = self
            .reservations
            .get_mut(&cell)
            .unwrap_or_else(|| panic!("subsume: no reservations at cell {:?} for {}", cell, row.id()));
        let pos = lane
            .res
            .iter()
            .position(|r| r.id() == row.id())
            .unwrap_or_else(|| panic!("subsume: reservation {} missing from cell {:?}", row.id(), cell));
        let last = lane.res.len() - 1;
        let live = &mut lane.res[pos];
        assert!(
            live.window().is_open_ended(),
            "subsume: reservation {} is bounded ({}) — only open destination rows subsume",
            row.id(),
            live.window(),
        );
        assert_eq!(live.robot(), row.robot(), "subsume: reservation {} robot mismatch", row.id());
        assert!(
            new_start >= live.window().start(),
            "subsume: committed landing {} precedes the promoted start {} on {} — \
             the arrive-after guard (estimate/schedule divergence)",
            new_start,
            live.window().start(),
            row.id(),
        );
        assert_eq!(pos, last, "subsume: open-ended row must be the lane tail");
        live.set_window(TimeRange::open_ended(new_start));
        lane.starts[pos] = ms_to_key(new_start);
        let updated = live.clone();
        Self::assert_lane_invariants(lane, cell, "subsume_start_right");
        if self.trace_enabled {
            self.trace.push(TraceOp::Subsume { id: updated.id(), cell, robot: updated.robot(), new_start });
        }
        updated
    }

    fn upsert_edge(&mut self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        let key = (from, to);
        let list = self.edge_reservations.entry(key).or_default();
        // Sorted insertion by window.start — enables binary search in
        // has_edge_conflict instead of linear scan. Production code paths
        // commit in chronological start-order, so this is always append
        // (`pos == list.len()`); the sorted-insertion form supports
        // mid-list inserts only as a defensive fallback for the same
        // test scenarios documented on `create_reservation`.
        let pos = list.partition_point(|r| r.window.start() < window.start());
        list.insert(pos, EdgeReservation { robot, window });
    }

    /// Create a new reservation with a fresh id and insert it at the
    /// sorted position. Returns the inserted reservation for the caller.
    ///
    /// Production code paths append (`pos == list.len()`); two
    /// pathfinding tests pre-stage future blockers and produce mid-list
    /// inserts — `basic::dest_dwell_zero_holds_until_arrival` and
    /// `turn::bug_c891118_turn_waypoint_phantom_conflict`. Both are
    /// handled by `partition_point` insertion. No stats work happens
    /// here: duty stats are computed at report time from a walk of the
    /// live list combined with the per-cell drop-aggregate (see
    /// `accumulator.rs` and `Inner::cell_duty_stats`).
    fn create_reservation(&mut self, cell: CellId, robot: RobotId, window: TimeRange) -> CellReservation {
        let id = ReservationId::new(self.next_reservation_id);
        self.next_reservation_id += 1;
        let reservation = CellReservation::new(id, cell, robot, window);
        if self.trace_enabled {
            self.trace.push(TraceOp::Commit { id, cell, robot, window });
        }
        let lane = self.reservations.entry(cell).or_default();
        let start_key = ms_to_key(window.start());
        let pos = lane.starts.partition_point(|&start| start < start_key);
        lane.starts.insert(pos, start_key);
        lane.ends.insert(pos, end_key(&window));
        lane.res.insert(pos, reservation.clone());
        reservation
    }

    /// Combine the per-cell drop-aggregate with a walk of the still-live
    /// list to produce `CellDutyStats` at report time. Open-ended live
    /// entries contribute a synthetic finite interval ending at
    /// `horizon_ms`.
    fn cell_duty_stats(&self, cell: CellId, horizon_ms: u64) -> CellDutyStats {
        // Seed from the drop-aggregate (None ⇒ empty aggregate; not a
        // silent fallback — absence is the canonical "nothing dropped
        // for this cell yet" state).
        let mut aggregate = match self.dropped_stats.get(&cell) {
            Some(a) => a.clone(),
            None => CellDutyAggregate::default(),
        };

        // Walk the still-live list, folding each entry into the same
        // aggregate. Sorted-by-start invariant guarantees sequential
        // calls match the same boundary semantics that the prune-time
        // ingest used.
        let mut open_ended = 0usize;
        if let Some(lane) = self.reservations.get(&cell) {
            Self::assert_lane_invariants(lane, cell, "cell_duty_stats");
            for r in &lane.res {
                let start_ms = r.window().start().as_ms();
                let end_ms = match r.window().end() {
                    Some(end) => end.as_ms(),
                    None => {
                        open_ended += 1;
                        horizon_ms
                    }
                };
                aggregate.ingest_finalized(start_ms, end_ms);
            }
        }

        CellDutyStats {
            reservations: aggregate.reservations,
            open_ended,
            first_start_ms: aggregate.first_start_ms,
            frontier_ms: aggregate.frontier_ms,
            busy_ms: aggregate.busy_ms,
            gap_ms: aggregate.gap_ms,
            gap_count: aggregate.gap_count,
            longest_gap_ms: aggregate.longest_gap_ms,
            longest_hold_ms: aggregate.longest_hold_ms,
            start_interval_stddev_ms: aggregate.interval_stddev_ms(),
        }
    }
}

/// Shared reservation state. Clone is cheap (Arc bump).
/// Read lock for earliest_free (A*), write lock for reserve_path (atomic commit).
/// Every method that changes a cell's holds stamps the cell in `wake`, which
/// clones share, so no commit can change a hold without waking its awaits.
#[derive(Clone)]
pub struct ReservationTable {
    inner: Arc<RwLock<Inner>>,
    wake: Arc<CommitWake>,
}

/// Read-locked snapshot of the reservation table. Holds the read lock for its
/// lifetime, allowing many `earliest_free` calls without re-acquiring the lock.
pub struct ReservationSnapshot<'a> {
    guard: RwLockReadGuard<'a, Inner>,
}

impl Default for ReservationTable {
    fn default() -> Self {
        Self::new(SimDuration::ZERO)
    }
}

impl ReservationTable {
    pub fn new(gap: SimDuration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                reservations: FxHashMap::default(),
                edge_reservations: FxHashMap::default(),
                dropped_stats: FxHashMap::default(),
                gap,
                next_reservation_id: 1,
                trace: Vec::new(),
                trace_enabled: false,
            })),
            wake: Arc::default(),
        }
    }

    /// Enable reservation tracing. Call before plan_ahead.
    pub fn enable_trace(&self) {
        self.inner.write().trace_enabled = true;
    }

    /// Drain and return the trace log.
    pub fn drain_trace(&self) -> Vec<TraceOp> {
        let mut inner = self.inner.write();
        std::mem::take(&mut inner.trace)
    }

    /// Find the earliest time `>= earliest` when `cell` is free for `duration`.
    /// If `predecessor` is Some, that reservation is excluded from the check
    /// (used when the caller intends to finalize it rather than compete with it).
    /// Returns `None` if the cell has a permanent hold.
    pub fn earliest_free(
        &self,
        cell: CellId,
        earliest: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        self.inner.read().earliest_free(cell, earliest, duration, predecessor)
    }

    /// Canonical safe intervals on `cell` overlapping `[after, horizon]`.
    /// Trailing open interval has `free_end = None`.
    pub fn entry_intervals(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
    ) -> Vec<(SimTime, Option<SimTime>)> {
        self.inner.read().entry_intervals(cell, after, horizon, predecessor)
    }

    /// Same as `entry_intervals` but writes into a caller-provided `out`
    /// buffer. The buffer is `clear()`-ed and reused — hot callers
    /// (`fh_sipp::expand_move`) pass a per-search scratch Vec to avoid
    /// per-call heap allocation.
    pub fn entry_intervals_into(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
        out: &mut Vec<(SimTime, Option<SimTime>)>,
    ) {
        self.inner.read().entry_intervals_into(cell, after, horizon, predecessor, out);
    }

    /// Check if a proposed window on a cell conflicts with any existing
    /// reservation. `predecessor` is excluded from the check.
    pub fn has_conflict(
        &self,
        cell: CellId,
        start: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> bool {
        let window = TimeRange::from_duration(start, duration);
        self.inner.read().has_conflict(cell, window, predecessor).is_some()
    }

    /// Check if a move from→to conflicts with a counter-flow move to→from.
    /// Edge reservations are robot-scoped — pass the acting robot to exclude
    /// its own direction.
    pub fn has_edge_conflict(
        &self,
        from: CellId,
        to: CellId,
        start: SimTime,
        duration: SimDuration,
        acting_robot: Option<RobotId>,
    ) -> bool {
        let window = TimeRange::from_duration(start, duration);
        self.inner.read().has_edge_conflict(from, to, window, acting_robot)
    }

    /// Commit an edge reservation (from→to) during a traversal window.
    pub fn commit_edge(&self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        self.inner.write().upsert_edge(from, to, robot, window);
    }

    /// Acquire a read-locked snapshot for batch queries (e.g., A* search).
    pub fn snapshot(&self) -> ReservationSnapshot<'_> {
        ReservationSnapshot { guard: self.inner.read() }
    }

    // Intentionally no clear_robot / release / validate_path / reserve_cell:
    // testing the theory that the only legitimate mutations are reserve_path
    // (create new reservations, optionally finalizing a predecessor) and
    // cancel_forward (re-tasking). If any of those turn out to be needed we'll
    // add them back with an explicit purpose.

    /// Commit `plan` atomically under a single write lock per spec §6
    /// (single-shot semantics; the windowed required-prefix / optimistic-tail
    /// distinction is gone).
    ///
    /// Validation phases (no mutation; first failure aborts atomically):
    ///   0. CommitPlan shape:
    ///      0p1. `plan.move_ops.len() == plan.requests.len()`.
    ///      0p2. For every `Some(op)` at index `i`,
    ///           `op.dest_cell == plan.requests[i].cell` (provenance is
    ///           consistent — a stray MoveOp can't satisfy
    ///           predecessor-close or edge validation without naming the
    ///           committed destination).
    ///   1. Predecessor structural invariants:
    ///      1a. `close_predecessor_at` requires `predecessor.is_some()`.
    ///      1b. Identity in live table — `predecessor.cell()` exists, the
    ///          reservation with `predecessor.id()` is present and owned by
    ///          the committing robot.
    ///      1c. No plan request opens a NEW reservation on the predecessor's
    ///          cell while its hold is live (the predecessor IS the cell-0
    ///          hold; overlapping it would duplicate or self-conflict).
    ///      1d. If closing the predecessor, the plan must contain a `MoveOp`
    ///          with `source_cell == predecessor.cell()` and
    ///          `move_end == close_predecessor_at`. The close also narrows
    ///          (or matches) the live predecessor window — never widens it.
    ///   2. Every cell request is admissible against existing reservations
    ///      on its cell.
    ///   3. Every `MoveOp` is admissible on its directed edge.
    ///
    /// Phase 1c bites only while the hold is live. A request that starts at or
    /// after `close_predecessor_at` says the robot left the cell and came back
    /// to it, which is the path a search draws when it steps aside to let
    /// somebody through and then carries on; that request is admitted and
    /// decided on its merits in phase 2, against every other robot's claim on
    /// the cell.
    ///
    /// On success, all cells are inserted, the predecessor finalized (if
    /// applicable), and all edge reservations committed under the same
    /// write lock.
    ///
    /// A post-search commit conflict is treated as search/commit drift —
    /// not a normal partial-success path. The allocator surfaces it as
    /// `PathfinderError::CommitConflict`.
    pub fn reserve_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        let committed = self.commit_locked(robot, plan, predecessor)?;
        let closed = plan.close_predecessor_at.and(predecessor).map(CellReservation::cell);
        self.wake.touch(plan.requests.iter().map(|request| request.cell).chain(closed));
        Ok(committed)
    }

    /// A count that rises with every commit (scheduler time).
    pub fn commit_count(&self) -> u64 {
        self.wake.count()
    }

    /// Resolves once a commit counted after `seen` has changed any of `cells`.
    pub fn commit_touching<'a>(&'a self, cells: &'a [CellId], seen: u64) -> impl Future<Output = ()> + 'a {
        self.wake.touching(cells, seen)
    }

    /// `reserve_path`'s validation and mutation, under one write lock.
    fn commit_locked(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        let mut inner = self.inner.write();

        // Phase 0p1: parallel-vec invariant on the CommitPlan.
        if plan.move_ops.len() != plan.requests.len() {
            // Sentinel: requests[0].cell when present, MAX otherwise.
            let cell = plan.requests.first().map(|r| r.cell).unwrap_or(CellId(u32::MAX));
            let window = plan.requests.first().map(|r| r.window).unwrap_or(TimeRange::open_ended(SimTime::ZERO));
            return Err(ReservationConflict { cell, proposed: window, existing_robot: robot, existing_window: window });
        }

        // Phase 0p2: each Some(op) must name the committed destination.
        for (i, op) in plan.move_ops.iter().enumerate() {
            if let Some(op) = op.as_ref() {
                let req = &plan.requests[i];
                if op.dest_cell != req.cell {
                    let window = req.window;
                    return Err(ReservationConflict {
                        cell: req.cell,
                        proposed: window,
                        existing_robot: robot,
                        existing_window: op.window,
                    });
                }
            }
        }

        // Phase 1a: close_predecessor_at requires predecessor.
        if let Some(close_at) = plan.close_predecessor_at {
            if predecessor.is_none() {
                let window = TimeRange::open_ended(close_at);
                return Err(ReservationConflict {
                    cell: CellId(u32::MAX),
                    proposed: window,
                    existing_robot: robot,
                    existing_window: window,
                });
            }
        }

        // Phase 1b: predecessor identity in live table (cell + id + robot).
        // The live row is cloned once and kept paired with the caller's
        // snapshot, so later phases can read the live window without colliding
        // with the &mut inner needed for mutation — and so no phase can hold
        // one of the two without the other.
        let live_pred: Option<(&CellReservation, CellReservation)> = match predecessor {
            None => None,
            Some(pred) => {
                let found = inner
                    .reservations
                    .get(&pred.cell())
                    .and_then(|lane| lane.res.iter().find(|r| r.id() == pred.id()).cloned());
                match found {
                    Some(live) if live.robot() == robot => Some((pred, live)),
                    _ => {
                        return Err(ReservationConflict {
                            cell: pred.cell(),
                            proposed: pred.window(),
                            existing_robot: robot,
                            existing_window: pred.window(),
                        });
                    }
                }
            }
        };

        // Phase 1c: no plan request opens a new reservation on the
        // predecessor's cell *while the predecessor still holds it*. The
        // predecessor's hold IS the cell-0 reservation; a request overlapping
        // it would duplicate it or false-conflict with it.
        //
        // A request that begins at or after the close is the opposite
        // statement: the robot left the cell and came back to it. That is a
        // path that steps aside to let another robot pass and returns, which
        // is how the planner waits out a conflict on a cell it must reach —
        // and it is the ordinary shape now that a handoff shelf is worked from
        // a cell of the traffic lane rather than from a dead-end square of its
        // own. Refusing it refuses the wait, not the duplicate, and aborts a
        // run over a manoeuvre the search made on purpose.
        if let Some(pred) = predecessor {
            let reopens_at = plan.close_predecessor_at;
            let overlapping = plan
                .requests
                .iter()
                .find(|r| r.cell == pred.cell() && reopens_at.is_none_or(|close| r.window.start() < close));
            if let Some(req) = overlapping {
                let window = req.window;
                return Err(ReservationConflict {
                    cell: pred.cell(),
                    proposed: window,
                    existing_robot: pred.robot(),
                    existing_window: pred.window(),
                });
            }
        }

        // Phase 1d: close_predecessor_at must bind to a MoveOp and narrow
        // the live predecessor end (if finite).
        //
        // The validated close window is hoisted out so Phase 4 reuses it
        // instead of rebuilding it: one construction, one ordering check, and
        // no way for the commit to construct a range Phase 1d never saw.
        let mut pred_close_window: Option<TimeRange> = None;
        if let (Some((pred, live)), Some(close_at)) = (live_pred.as_ref(), plan.close_predecessor_at) {
            // Ordering first. A close that precedes the hold's own start is a
            // malformed plan, which is exactly what the `!has_match` arm below
            // already reports as an Err — so report it the same way rather
            // than panicking inside `TimeRange::new`. Panicking here would
            // abort the run from within the branch whose job is to *diagnose*
            // plan drift.
            let Ok(close_window) = TimeRange::try_new(pred.window().start(), close_at) else {
                return Err(ReservationConflict {
                    cell: pred.cell(),
                    // The requested close is not representable as a range
                    // (it precedes the start), so this stands in for it and
                    // carries `close_at`. `proposed` is diagnostic only.
                    proposed: TimeRange::empty_at(close_at),
                    existing_robot: pred.robot(),
                    existing_window: pred.window(),
                });
            };

            let has_match = plan
                .move_ops
                .iter()
                .filter_map(|o| o.as_ref())
                .any(|op| op.source_cell == pred.cell() && op.window.end() == Some(close_at));
            if !has_match {
                return Err(ReservationConflict {
                    cell: pred.cell(),
                    proposed: close_window,
                    existing_robot: pred.robot(),
                    existing_window: pred.window(),
                });
            }
            // Narrowing: validate against the LIVE end, not the snapshot
            // (the caller's CellReservation may be stale).
            if let Some(live_end) = live.window().end() {
                if close_at > live_end {
                    return Err(ReservationConflict {
                        cell: pred.cell(),
                        // Cannot invert: this arm requires `close_at > live_end`,
                        // and `live_end >= live.window().start()` holds for every
                        // constructed TimeRange, so `close_at > start`.
                        proposed: TimeRange::new(live.window().start(), close_at),
                        existing_robot: live.robot(),
                        existing_window: live.window(),
                    });
                }
            }
            pred_close_window = Some(close_window);
        }

        // Phase 2: validate every cell request. The predecessor's own hold is
        // excluded from self-conflict on its own cell — it is finalized
        // atomically below, and Phase 1c has already established that any
        // request landing there begins after that close. Every other robot's
        // reservation on that cell still decides.
        let pred_cell = predecessor.map(|p| p.cell());
        for req in &plan.requests {
            let window = req.window;
            let pred_for_check = if Some(req.cell) == pred_cell { predecessor } else { None };
            if let Some(conflict) = inner.has_conflict(req.cell, window, pred_for_check) {
                return Err(conflict);
            }
        }

        // Phase 3: validate every directed-edge reservation. Per spec §4
        // edge conflicts are caught during search; arrival here means
        // search/commit drift (a bug worth surfacing as ReservationConflict).
        for op in plan.move_ops.iter().filter_map(|o| o.as_ref()) {
            let move_window = op.window;
            if inner.has_edge_conflict(op.source_cell, op.dest_cell, move_window, Some(robot)) {
                return Err(ReservationConflict {
                    cell: op.source_cell,
                    proposed: move_window,
                    existing_robot: robot,
                    existing_window: move_window,
                });
            }
        }

        // Phase 4: commit cells + finalize predecessor (if any). Reuses the
        // window Phase 1d already validated, so this cannot construct (or
        // panic on) a range the validation did not see.
        if !plan.requests.is_empty() {
            if let (Some(pred), Some(new_window)) = (predecessor, pred_close_window) {
                inner.finalize(pred, new_window)?;
            }
        }
        let mut committed: Vec<CellReservation> = Vec::with_capacity(plan.requests.len());
        for req in &plan.requests {
            let window = req.window;
            let reservation = inner.create_reservation(req.cell, robot, window);
            committed.push(reservation);
        }

        // Phase 5: commit edges (same write lock; atomic with the cells).
        for op in plan.move_ops.iter().filter_map(|o| o.as_ref()) {
            let window = op.window;
            inner.upsert_edge(op.source_cell, op.dest_cell, robot, window);
        }

        Ok(committed)
    }

    /// Test helper: pre-populate a single cell reservation.
    ///
    /// Thin wrapper over `reserve_path` with a 1-request plan. Used by A\*
    /// tests to stage pre-existing reservations before exercising the
    /// pathfinder.
    pub fn reserve_cell(
        &self,
        robot: RobotId,
        cell: CellId,
        window: TimeRange,
    ) -> Result<CellReservation, ReservationConflict> {
        let plan = CommitPlan::single(ReservationRequest::new(cell, window));
        let mut reservations = self.reserve_path(robot, &plan, None)?;
        Ok(reservations.remove(0))
    }

    /// Move an open-ended reservation's start right, preserving id and
    /// open end — the destination-row subsume; refuses leftward motion
    /// (the arrive-after guard). See the inner op for the full contract.
    /// Dormant until family services author destination rows at
    /// promotion; the owner's path commit is the intended caller.
    pub fn subsume_start_right(&self, row: &CellReservation, new_start: SimTime) -> CellReservation {
        let subsumed = self.inner.write().subsume_start_right(row, new_start);
        self.wake.touch([row.cell()]);
        subsumed
    }

    /// Number of active reservations (for metrics).
    pub fn active_count(&self) -> usize {
        let inner = self.inner.read();
        inner.reservations.values().map(|l| l.res.len()).sum()
    }

    /// Diagnostic: (total_entries, unique_cells, max_per_cell, stale_count).
    pub fn diagnostics(&self, now: SimTime) -> (usize, usize, usize, usize) {
        let inner = self.inner.read();
        let total: usize = inner.reservations.values().map(|l| l.res.len()).sum();
        let cells = inner.reservations.len();
        let max = inner.reservations.values().map(|l| l.res.len()).max().unwrap_or(0);
        let stale: usize = inner
            .reservations
            .values()
            .map(|l| l.res.iter().filter(|r| r.window().end().map(|e| e < now).unwrap_or(false)).count())
            .sum();
        (total, cells, max, stale)
    }

    /// Remove a specific edge reservation by exact match.
    pub fn release_edge_window(&self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        let key = (from, to);
        let mut inner = self.inner.write();
        if let Some(list) = inner.edge_reservations.get_mut(&key) {
            list.retain(|r| !(r.robot == robot && r.window == window));
        }
    }

    // cancel_forward removed: the simulator doesn't cancel legs mid-flight.
    // If real-warehouse re-tasking ever needs to invalidate future reservations,
    // reintroduce this (or a replacement) with an explicit mid-flight re-task
    // story.

    /// Does this robot have a reservation on this cell covering this time?
    pub fn robot_has_reservation(&self, robot: RobotId, cell: CellId, at: SimTime) -> bool {
        let inner = self.inner.read();
        let Some(lane) = inner.reservations.get(&cell) else { return false };
        Inner::assert_lane_invariants(lane, cell, "robot_has_reservation");
        // Binary-search the `ends` key array past reservations that have already
        // finished by `at`. Open-ended ends are `u32::MAX`, never skipped.
        let at_key = query_key(at);
        let start_idx = tail_hinted_partition(&lane.ends, at_key);
        for r in &lane.res[start_idx..] {
            // Sorted by start: once start > at, no later reservation contains `at`.
            if r.window().start() > at {
                break;
            }
            if r.robot() == robot {
                return true;
            }
        }
        false
    }

    /// Check that no two different robots overlap on any cell.
    pub fn find_overlaps(&self) -> Vec<(CellId, RobotId, TimeRange, RobotId, TimeRange)> {
        let inner = self.inner.read();
        let mut violations = Vec::new();
        for (&cell, lane) in &inner.reservations {
            for i in 0..lane.res.len() {
                for j in (i + 1)..lane.res.len() {
                    let a = &lane.res[i];
                    let b = &lane.res[j];
                    if a.robot() != b.robot() && a.window().overlaps(b.window(), inner.gap) {
                        violations.push((cell, a.robot(), a.window(), b.robot(), b.window()));
                    }
                }
            }
        }
        violations
    }

    /// All reservations on a cell, sorted by start time.
    pub fn reservations_for_cell(&self, cell: CellId) -> Vec<CellReservation> {
        let inner = self.inner.read();
        // The underlying lane is maintained sorted by `window().start` via
        // sorted insertion in `create_reservation` / `finalize` — no need to
        // re-sort on the clone.
        match inner.reservations.get(&cell) {
            Some(lane) => {
                Inner::assert_lane_invariants(lane, cell, "reservations_for_cell");
                lane.res.clone()
            }
            None => Vec::new(),
        }
    }

    /// All open-ended reservations — diagnostic and end-of-sim validation.
    /// Nonzero at simulation teardown indicates a planning stall.
    pub fn open_ended(&self) -> Vec<CellReservation> {
        let inner = self.inner.read();
        let mut out = Vec::new();
        for lane in inner.reservations.values() {
            for r in &lane.res {
                if r.is_open_ended() {
                    out.push(r.clone());
                }
            }
        }
        out
    }

    /// Exact unresolved tail identity. Sorted, non-overlapping lanes can have
    /// only one open hold, at their tail.
    pub fn open_ended_reservation(&self, cell: CellId) -> Option<CellReservation> {
        let inner = self.inner.read();
        let lane = inner.reservations.get(&cell)?;
        Inner::assert_lane_invariants(lane, cell, "open_ended_reservation");
        lane.res.last().filter(|r| r.is_open_ended()).cloned()
    }

    pub fn open_ended_at(&self, cell: CellId) -> Option<RobotId> {
        let inner = self.inner.read();
        // By the non-overlap + sorted-by-start invariant, an open-ended
        // reservation (window [start, ∞)) must be the LAST entry — anything
        // after it would overlap. O(1) tail check instead of full scan.
        let lane = inner.reservations.get(&cell)?;
        Inner::assert_lane_invariants(lane, cell, "open_ended_at");
        lane.res.last().filter(|r| r.is_open_ended()).map(|r| r.robot())
    }

    /// Per-cell duty stats for the end-of-run report.
    ///
    /// Starts from the per-cell drop-aggregate (entries that have
    /// already been pruned, accumulated at drop time) and folds in
    /// every still-live entry by walking the sorted-by-start list.
    /// Open-ended entries contribute a synthetic finite interval
    /// `[start, horizon_ms]`. Sorted-by-start + non-overlap invariants
    /// make the walk strictly sequential, so `CellDutyAggregate::ingest_finalized`
    /// produces the same numbers it would for the entire pre-prune
    /// history.
    ///
    /// Single read-lock acquisition for both the aggregate and the
    /// live walk.
    pub fn cell_duty_stats(&self, cell: CellId, horizon_ms: u64) -> CellDutyStats {
        self.inner.read().cell_duty_stats(cell, horizon_ms)
    }

    /// Drop finite cell + edge reservations whose `end < now - margin`.
    /// Watermark = `now.as_ms() - max(gap, margin.as_ms())`. Accumulators
    /// are untouched — duty stats stay correct after pruning.
    ///
    /// Invoked only by explicit callers (tests, or an external end-of-batch
    /// trigger in a sim driver) — there is **no** auto-prune in `reserve_path`.
    /// The live headless path never prunes, so `reservations_for_cell` holds
    /// the full reservation history at end-of-run.
    /// Stamps nothing: it removes only finalized rows, so no cell's
    /// open-ended hold, which is what an await is for, changes.
    pub fn prune_below_watermark(&self, now: SimTime, margin: SimDuration) {
        let mut inner = self.inner.write();
        let margin_ms = inner.gap.as_ms().max(margin.as_ms());
        let watermark_ms = now.as_ms().saturating_sub(margin_ms);
        inner.prune_finalized_below_watermark(watermark_ms);
    }
}

impl<'a> ReservationSnapshot<'a> {
    /// Earliest time cell is free, with optional predecessor exclusion.
    pub fn earliest_free(
        &self,
        cell: CellId,
        earliest: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        self.guard.earliest_free(cell, earliest, duration, predecessor)
    }

    /// Conflict check with optional predecessor exclusion.
    pub fn has_conflict(
        &self,
        cell: CellId,
        start: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> bool {
        let window = TimeRange::from_duration(start, duration);
        self.guard.has_conflict(cell, window, predecessor).is_some()
    }

    /// Edge conflict check. Edges are robot-scoped.
    pub fn has_edge_conflict(
        &self,
        from: CellId,
        to: CellId,
        start: SimTime,
        duration: SimDuration,
        acting_robot: Option<RobotId>,
    ) -> bool {
        let window = TimeRange::from_duration(start, duration);
        self.guard.has_edge_conflict(from, to, window, acting_robot)
    }
}
