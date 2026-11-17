//! ReservationOracle trait — injectable reservation checking + committing.
//!
//! The pathfinder calls `earliest_available` / `entry_intervals` during A*
//! to check cell conflicts, and `commit_path` after planning to atomically
//! commit the computed path (cells + edges + predecessor finalize) under
//! a single write lock.
//!
//! Implementations:
//! - `TableOracle` wraps `ReservationTable` for real collision checking
//!   (production CollisionDes mode).
//! - `GhostOracle` reports every cell/interval as free and no-ops commits
//!   (Fidelity::Ghost — drives bots through the real pathfinder with
//!   contention removed, the theoretical-max baseline).
//! - Test-only oracles live in `test/helpers.rs`.
//!
//! ## Commit semantics, per window
//!
//! Each window of a `reserve_path` call is one SIPP-A\* search that
//! validates every cell + edge against the oracle, then `commit_path`
//! validates and commits that window's plan all-or-nothing. There is no required-prefix /
//! optimistic-tail distinction; if any validation fails, no mutations are
//! applied. Edge reservations are part of the same atomic write-lock pass.
//!
//! A post-search commit conflict means search/commit drift — a bug, not a
//! soft failure. The allocator surfaces it as
//! `PathfinderError::CommitConflict`.

use std::future::pending;
use std::pin::Pin;

use desgraph::CellId;

use crate::CommitPlan;
use crate::RobotId;

use super::cell_reservation::CellReservation;
use super::diagnostics::CellDutyStats;
use super::table::{ReservationConflict, ReservationTable, TraceOp};
use crate::{SimDuration, SimTime, TimeRange};

/// Canonical identity of a safe interval on a cell's reservation timeline.
/// `(free_start, free_end)`. Trailing open interval has `free_end = None`.
pub type SafeIntervalId = (SimTime, Option<SimTime>);

/// Planning status of a cell from the planner's perspective.
#[derive(Clone, Debug)]
pub enum CellStatus {
    /// No open-ended reservation. A* can schedule arrivals at this cell.
    Schedulable,
    /// An open-ended reservation exists — some chain was planned to this
    /// point but its continuation hasn't been scheduled yet.
    PartiallyPlanned { robot: RobotId },
}

/// Injectable reservation checking and committing for the pathfinder.
///
/// The pathfinder calls `earliest_available` / `entry_intervals` /
/// `has_edge_conflict` during A* successor validation, and `commit_path`
/// once the full plan is built.
pub trait ReservationOracle: Send + Sync {
    /// Find the earliest time `cell` is free for `duration` at or after `after`.
    /// If `predecessor` is Some, that reservation is excluded from the check.
    /// Returns `None` if the cell has a permanent hold.
    fn earliest_available(
        &self,
        cell: CellId,
        after: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime>;

    /// Quick conflict check. Default impl delegates to earliest_available.
    fn has_conflict(
        &self,
        cell: CellId,
        start: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> bool {
        self.earliest_available(cell, start, duration, predecessor) != Some(start)
    }

    /// Check if a move from→to would conflict with a counter-flow move to→from.
    /// Edges are robot-scoped — pass the acting robot to exclude its direction.
    fn has_edge_conflict(
        &self,
        _from: CellId,
        _to: CellId,
        _start: SimTime,
        _duration: SimDuration,
        _acting_robot: Option<RobotId>,
    ) -> bool {
        false // default: no edge conflict tracking
    }

    /// Commit a directed edge reservation (from→to) during a traversal window.
    fn commit_edge(&self, _from: CellId, _to: CellId, _robot: RobotId, _window: TimeRange) {
        // default: no-op
    }

    /// Remove a specific edge reservation (extension rollback).
    fn release_edge_window(&self, _from: CellId, _to: CellId, _robot: RobotId, _window: TimeRange) {}

    /// Commit `plan` atomically (single-shot per spec §6).
    ///
    /// `plan.requests` are the new cell reservations to open.
    /// `plan.move_ops` carry the producing Move actions per request (None
    /// for direct holds and fresh-path start cells); each `Some(op)` is
    /// validated and committed as a directed-edge reservation in the same
    /// write lock.
    /// `plan.close_predecessor_at`, if Some, finalizes the predecessor's
    /// open hold to that time atomically with the rest of the commit.
    ///
    /// **Predecessor structural invariants** (validated before any cell or
    /// edge check; failure returns `Err` with no mutations):
    ///
    /// - `close_predecessor_at.is_some()` requires `predecessor.is_some()`.
    /// - The predecessor reservation must exist in the live table at
    ///   `predecessor.cell()` with matching id and `robot == committer`.
    /// - No `plan.requests` entry may target `predecessor.cell()` — the
    ///   predecessor IS the cell-0 hold; a parallel request would
    ///   duplicate or self-conflict.
    /// - When `close_predecessor_at` is `Some(t)`, the plan must contain a
    ///   `MoveOp` with `source_cell == predecessor.cell()` and `move_end
    ///   == t`. The close also narrows (or matches) the live predecessor
    ///   end — never widens it.
    ///
    /// All-or-nothing: if any predecessor, cell, or edge validation fails,
    /// `Err` is returned with no visible side effects. There is no
    /// required-prefix / optimistic-tail distinction.
    ///
    /// A post-search conflict signals search/commit drift (cell/edge
    /// validation, gap semantics, or predecessor exclusion are out of
    /// sync between search and table). The allocator treats this as a bug,
    /// not a soft failure.
    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict>;

    /// Find all overlapping reservations between different robots (diagnostic).
    fn find_overlaps(&self) -> Vec<(CellId, RobotId, TimeRange, RobotId, TimeRange)> {
        Vec::new()
    }

    /// Does this robot have a reservation on this cell at this time?
    /// Default: `true` — non-tracking oracles (GhostOracle, test mocks) tell
    /// callers the commit "happened" so downstream invariant checks pass.
    fn robot_has_reservation(&self, _robot: RobotId, _cell: CellId, _at: SimTime) -> bool {
        true
    }

    /// All reservations on a cell (diagnostic).
    fn reservations_for_cell(&self, _cell: CellId) -> Vec<CellReservation> {
        Vec::new()
    }

    /// Per-cell duty stats for the end-of-run report. `TableOracle`
    /// overrides to project the accumulator + live open-ended tail;
    /// default returns zeros for test oracles that don't accumulate.
    fn cell_duty_stats(&self, _cell: CellId, _horizon_ms: u64) -> CellDutyStats {
        CellDutyStats::default()
    }

    /// Planning status of a cell: is it schedulable, or does an unresolved
    /// open-ended reservation block further planning?
    fn cell_status(&self, _cell: CellId) -> CellStatus {
        CellStatus::Schedulable
    }

    /// Exact unresolved tail identity, without scanning finalized history in
    /// table-backed oracles. Used to keep resource admission tied to departure.
    fn open_ended_reservation(&self, cell: CellId) -> Option<CellReservation> {
        self.reservations_for_cell(cell).into_iter().find(CellReservation::is_open_ended)
    }

    /// All open-ended reservations — end-of-sim validation.
    fn open_ended(&self) -> Vec<CellReservation> {
        Vec::new()
    }

    /// Canonical safe intervals on `cell` overlapping `[after, horizon]`.
    /// Trailing open interval has `free_end = None`. Identity is the
    /// physical interval — NOT clipped by `after` (otherwise dedup would
    /// break across queries from different `after` values).
    ///
    /// No default impl: every implementor must provide canonical interval
    /// enumeration. A default impl pairing `entry_times` with `None` would
    /// silently widen finite intervals to open-ended ones and corrupt
    /// goal-validation logic that depends on `free_end`.
    fn entry_intervals(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId>;

    /// Same as `entry_intervals` but writes into a caller-provided `out`
    /// buffer. The buffer is `clear()`-ed before any work; hot callers
    /// reuse the same Vec across queries to avoid per-call heap allocation.
    ///
    /// Default implementation calls `entry_intervals` and extends `out`,
    /// preserving the per-call allocation. `TableOracle` overrides to call
    /// the table's direct path.
    fn entry_intervals_into(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
        out: &mut Vec<SafeIntervalId>,
    ) {
        out.clear();
        out.extend(self.entry_intervals(cell, after, horizon, predecessor));
    }

    /// Entry times for a cell: start of each safe interval clipped to
    /// `[after, horizon]` and to half-open finite-interval admission.
    ///
    /// Compatibility wrapper around `entry_intervals`. Implementors should
    /// rarely need to override.
    fn entry_times(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
    ) -> Vec<SimTime> {
        self.entry_intervals(cell, after, horizon, predecessor)
            .into_iter()
            .filter_map(|(free_start, free_end)| {
                let entry = std::cmp::max(free_start, after);
                if entry > horizon {
                    return None;
                }
                if free_end.is_some_and(|end| entry >= end) {
                    return None;
                }
                Some(entry)
            })
            .collect()
    }

    /// True if this oracle maintains a persistent, contended reservation
    /// table — i.e. commits persist and later queries observe them. The
    /// allocator branches on the negation: an oracle that does *not* track
    /// reservations (e.g. `GhostOracle`, always-free + fresh-per-call table)
    /// must bypass SIPP A* (which would expand unboundedly against always-
    /// free safe intervals) and skip the predecessor-close (a fresh per-call
    /// table can't satisfy the "predecessor exists in live table" invariant).
    /// Default: true (real oracles track reservations).
    fn tracks_reservations(&self) -> bool {
        true
    }

    /// A count that rises (scheduler time) with every commit this oracle
    /// accepts, for `reserve_path` to await a held cell's bounding. `None`
    /// from an oracle that cannot wake an await (the ghost oracle): a journey
    /// is then never foreshortened.
    fn commit_count(&self) -> Option<u64> {
        None
    }

    /// Resolves once a commit counted after `seen` (a value `commit_count`
    /// returned earlier) has touched any of `cells`. Level-triggered: a commit
    /// that left a cell's hold as it was still resolves it, and the caller
    /// looks again. Never resolves by default.
    fn commit_touching<'a>(&'a self, _cells: &'a [CellId], _seen: u64) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(pending())
    }

    /// Enable reservation tracing.
    fn enable_trace(&self) {}

    /// Drain the reservation trace log.
    fn drain_trace(&self) -> Vec<TraceOp> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Real oracle: wraps ReservationTable
// ---------------------------------------------------------------------------

/// Real oracle backed by ReservationTable. Used in production CollisionDes mode.
pub struct TableOracle {
    table: ReservationTable,
}

impl TableOracle {
    pub fn new(table: ReservationTable) -> Self {
        Self { table }
    }

    pub fn table(&self) -> &ReservationTable {
        &self.table
    }
}

impl Default for TableOracle {
    fn default() -> Self {
        Self::new(ReservationTable::default())
    }
}

impl ReservationOracle for TableOracle {
    fn earliest_available(
        &self,
        cell: CellId,
        after: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        self.table.earliest_free(cell, after, duration, predecessor)
    }

    fn has_conflict(
        &self,
        cell: CellId,
        start: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> bool {
        self.table.has_conflict(cell, start, duration, predecessor)
    }

    fn has_edge_conflict(
        &self,
        from: CellId,
        to: CellId,
        start: SimTime,
        duration: SimDuration,
        acting_robot: Option<RobotId>,
    ) -> bool {
        self.table.has_edge_conflict(from, to, start, duration, acting_robot)
    }

    fn commit_edge(&self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        self.table.commit_edge(from, to, robot, window);
    }

    fn release_edge_window(&self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        self.table.release_edge_window(from, to, robot, window);
    }

    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        self.table.reserve_path(robot, plan, predecessor)
    }

    fn commit_count(&self) -> Option<u64> {
        Some(self.table.commit_count())
    }

    fn commit_touching<'a>(&'a self, cells: &'a [CellId], seen: u64) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(self.table.commit_touching(cells, seen))
    }

    fn find_overlaps(&self) -> Vec<(CellId, RobotId, TimeRange, RobotId, TimeRange)> {
        self.table.find_overlaps()
    }

    fn robot_has_reservation(&self, robot: RobotId, cell: CellId, at: SimTime) -> bool {
        self.table.robot_has_reservation(robot, cell, at)
    }

    fn reservations_for_cell(&self, cell: CellId) -> Vec<CellReservation> {
        self.table.reservations_for_cell(cell)
    }

    fn cell_duty_stats(&self, cell: CellId, horizon_ms: u64) -> CellDutyStats {
        self.table.cell_duty_stats(cell, horizon_ms)
    }

    fn entry_intervals(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId> {
        self.table.entry_intervals(cell, after, horizon, predecessor)
    }

    fn entry_intervals_into(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
        out: &mut Vec<SafeIntervalId>,
    ) {
        self.table.entry_intervals_into(cell, after, horizon, predecessor, out);
    }

    fn open_ended_reservation(&self, cell: CellId) -> Option<CellReservation> {
        self.table.open_ended_reservation(cell)
    }

    fn cell_status(&self, cell: CellId) -> CellStatus {
        match self.table.open_ended_at(cell) {
            Some(robot) => CellStatus::PartiallyPlanned { robot },
            None => CellStatus::Schedulable,
        }
    }

    fn open_ended(&self) -> Vec<CellReservation> {
        self.table.open_ended()
    }

    fn enable_trace(&self) {
        self.table.enable_trace();
    }

    fn drain_trace(&self) -> Vec<TraceOp> {
        self.table.drain_trace()
    }
}

// ---------------------------------------------------------------------------
// GhostOracle: always-free oracle for Fidelity::Ghost
// ---------------------------------------------------------------------------

/// Always-free oracle for Fidelity::Ghost. Reports every cell as free,
/// every conflict check as "no conflict", and no-ops commits — each
/// `commit_path` call uses a fresh `ReservationTable` so the returned
/// `CellReservation` objects are well-formed for downstream consumers
/// (e.g. `AvailableHtmRegistry`) but never observable from any subsequent
/// oracle query.
///
/// Net effect: identical route whether there is 1 bot or hundreds — the
/// theoretical-max baseline.
///
/// The allocator's `reserve_path` shortcircuits to spatial-only A* when
/// `!oracle.tracks_reservations()` (see `pathfinding/fh_sipp/allocator.rs`)
/// and passes `close_predecessor_at = None`, because a fresh per-call table
/// cannot satisfy the "predecessor exists in live table" invariant on
/// `commit_path`.
///
/// ## Inherited trait defaults (audited, intentional)
///
/// These default impls are correct for ghost as-is — none are overridden:
///
/// - `commit_edge` / `release_edge_window` — no-op. Ghost never tracks
///   edges (spatial-only A* path).
/// - `find_overlaps` — empty. By construction no two ghost reservations
///   coexist (commits don't persist).
/// - `robot_has_reservation` — `true`. Callers (invariant checks) rely on
///   commits "having happened" from their POV even though nothing persists.
/// - `reservations_for_cell` — empty. Nothing persists.
/// - `cell_status` — `Schedulable`. No open-ended holds exist; relay-cell
///   `PartiallyPlanned` detection in `HtmPickup` never fires under ghost,
///   which is correct: there's no blocker to detect.
/// - `open_ended` — empty. No persistent reservations means no open-ended
///   ones for end-of-sim validation.
/// - `entry_intervals_into` — trait default delegates to `entry_intervals`.
/// - `cell_duty_stats` — zero (trait default). Station/relay reports
///   under ghost will show 0% utilization for traversed cells; accepted
///   tradeoff (ghost is a routing baseline, not a utilization sim).
pub struct GhostOracle;

impl ReservationOracle for GhostOracle {
    fn tracks_reservations(&self) -> bool {
        false
    }

    fn earliest_available(
        &self,
        _cell: CellId,
        after: SimTime,
        _duration: SimDuration,
        _predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        Some(after)
    }

    fn has_conflict(
        &self,
        _cell: CellId,
        _start: SimTime,
        _duration: SimDuration,
        _predecessor: Option<&CellReservation>,
    ) -> bool {
        false
    }

    fn has_edge_conflict(
        &self,
        _from: CellId,
        _to: CellId,
        _start: SimTime,
        _duration: SimDuration,
        _acting_robot: Option<RobotId>,
    ) -> bool {
        false
    }

    /// Fresh per-call `ReservationTable` — commits never conflict, nothing
    /// persists across calls. The returned `CellReservation` objects are
    /// well-formed (id, robot, cell, window) so `AvailableHtmRegistry` and
    /// other consumers that only read those fields keep working.
    ///
    /// Predecessor is intentionally ignored: the allocator must pass
    /// `close_predecessor_at = None` for ghost since the predecessor would
    /// not exist in the fresh table. ReservationIds reset per call, so IDs
    /// are not globally unique across ghost legs; the one ID-based filter
    /// (`station_gap_before`) only operates on `reservations_for_cell`
    /// results which are empty under ghost, so collisions are inert.
    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        _predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        let fresh = ReservationTable::default();
        fresh.reserve_path(robot, plan, None)
    }

    /// Every cell shares one canonical, always-free interval. Its identity
    /// must not depend on the requested arrival time: SIPP deduplicates
    /// states by interval, and `(after, ...)` invents another state on every
    /// revisit. The search enforces its horizon independently.
    fn entry_intervals(
        &self,
        _cell: CellId,
        after: SimTime,
        horizon: SimTime,
        _predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId> {
        if after > horizon { Vec::new() } else { vec![(SimTime::ZERO, None)] }
    }
}
