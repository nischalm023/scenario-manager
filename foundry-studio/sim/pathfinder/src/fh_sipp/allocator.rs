//! Allocator: `reserve_path` commits a journey in windows of its path, each
//! one SIPP-A\* search and one atomic commit (cells + edges + predecessor
//! close under one write lock).
//!
//! Most journeys are one window: the whole path, driven through any queue on
//! the way without a stop. When another robot holds the target, or a queue
//! cell feeding it ahead of the robot, open-ended, the journey may be
//! foreshortened (`super::foreshortening`), and search decides how: the target
//! first, then each free queue cell feeding it (one no other robot holds
//! open-ended), nearest the target first. Of these, none is searched when it
//! cannot succeed: the target while another robot holds it open-ended and the
//! stay asked for is open-ended, and any cell while such holds seal off every
//! way into it. The first that a search reaches is committed; a queue cell is
//! held open-ended from the start of the robot's move onto it (simtime). Short
//! of the target, the call awaits (scheduler time) a commit touching a held
//! cell on the robot's route, keeping its origin if no queue cell was reached
//! (unless the caller refuses that wait, `WindowSink::allows_origin_wait`: the
//! call then returns `OriginWaitRefused` having committed nothing), and starts
//! again from where the robot stands; with no held cell on the route, it starts
//! again if this pass reached a queue cell, else ends with the target's search.
//! Every search starts (simtime) no earlier than the caller's barrier, and one
//! from a queue cell no earlier than the robot's arrival there. When the
//! target's search fails in a way a hold can explain, it runs once more from a
//! later start toward the clearance of an awaited cell on the robot's route,
//! when there is one, so the wait does not spend the horizon. A window ends on
//! the target or a queue cell only.
//!
//! Known limits, beyond those `Pathfinder::reserve_path` states: a queue cell
//! is accepted on the search reaching it, not on the target being reachable
//! from it in the orientation the window ends in, so a cell that forbids
//! rotation can strand the robot. An await watches the held cells on one
//! empty-floor route, so a commit that frees only another approach does not
//! wake it. The target's retry tries only the latest start (simtime) toward an
//! awaited cell's clearance on that route, so it can miss a journey that only
//! an earlier start, or a cell's clearance on another approach, would let
//! through: the robot can then arrive later than it could have, or the call
//! can return its error with the robot holding its last window's cell.
//!
//! Commit conflicts are search/commit drift (a bug class), not soft
//! failures — they propagate as `PathfinderError::CommitConflict`.

use crate::MotionKind;
use desgraph::types::{CellId, CellKind};
use smallvec::SmallVec;

use super::FHSippPathfinder;
use super::pathfinder::is_holding_cell;
use crate::SearchStats;
use crate::TimeRange;
use crate::WindowSink;
use crate::path::{CommitPlan, MoveOp, ReservationRequest};
use crate::reservation::oracle::ReservationOracle;
use crate::{CollisionConfig, DriveModel, Orientation, Pathfinder, PathfinderError, Waypoint, WaypointAction};
use crate::{Path, ReservedPath};
use crate::{SimDuration, SimTime};

impl FHSippPathfinder {
    /// Plan and commit a path from `from` to `to` against the live oracle,
    /// window by window, handing each window that ends short of `to` to
    /// `windows` as it commits and returning the one that ends on `to`. A `to`
    /// whose kind is not a journey end (`is_journey_end`) is refused before
    /// any search, as `NotAJourneyEnd`.
    ///
    /// An oracle with no commit count cannot wake an await (the ghost oracle,
    /// which tracks no holds): the journey is then one search. On A\* failure
    /// the search's `Unreachable*` / `IterationLimit` variant propagates; on
    /// commit conflict, `CommitConflict` (the search and the table disagreed —
    /// a bug to investigate, not a soft fallback).
    #[allow(clippy::too_many_arguments)]
    pub async fn reserve_path(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
        windows: &mut dyn WindowSink,
    ) -> Result<ReservedPath, PathfinderError> {
        if let Some(cell) = self.inner.graph().cell(to)
            && !is_journey_end(cell.kind)
        {
            return Err(PathfinderError::NotAJourneyEnd { cell: to, kind: cell.kind });
        }
        if from == to {
            return Err(PathfinderError::ZeroLength { cell: from });
        }
        if config.dest_dwell == Some(SimDuration::MAX) && !is_holding_cell(self, to) {
            return Err(PathfinderError::MaxDwellNonHolding { cell: to });
        }
        let mut journey = config.clone();
        let (mut at, mut facing) = (from, from_orientation);
        // An open-ended stay cannot fit on a target another robot holds
        // open-ended, so that search is not run while it does.
        let open_ended_stay = journey.dest_dwell.is_none_or(|dwell| dwell == SimDuration::MAX);
        let mut awaited: SmallVec<[CellId; 4]> = SmallVec::new();
        loop {
            journey.sim_start = journey.sim_start.max(windows.barrier());
            // The count is read before the holds are, so a commit between the
            // read and the await still wakes it.
            let seen = oracle.commit_count();
            let held_ahead = seen.zip(seen.and_then(|_| self.window_ends(at, to, drive, oracle, journey.self_robot)));
            // With nothing held ahead, the target's search is the journey's
            // last, and its error stands.
            let Some((seen, ends)) = held_ahead else {
                let (path, stats) = self.search_target(at, facing, to, &awaited, drive, oracle, &journey)?;
                return self.commit_window(path, stats, to, oracle, &journey);
            };
            let (robot, start) = (journey.self_robot, journey.sim_start);
            let sealed_off = |into| self.sealed_off(at, into, to, drive, oracle, robot, start);
            if !(ends.target_held && open_ended_stay) && !sealed_off(to) {
                match self.search_target(at, facing, to, &awaited, drive, oracle, &journey) {
                    Ok((path, stats)) => return self.commit_window(path, stats, to, oracle, &journey),
                    Err(error) if explained_by_a_hold(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            let queue_window = CollisionConfig { dest_dwell: None, destination: None, ..journey.clone() };
            let mut moved = false;
            for queue_cell in ends.queue_cells.into_iter().filter(|cell| !sealed_off(*cell)) {
                let (path, stats) = match self.search_window(at, facing, queue_cell, to, drive, oracle, &queue_window) {
                    Ok(found) => found,
                    Err(error)
                        if explained_by_a_hold(&error) || matches!(error, PathfinderError::NoStaticRoute { .. }) =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let window = self.commit_window(path, stats, queue_cell, oracle, &queue_window)?;
                windows.committed(&window, journey.predecessor.as_ref()).map_err(PathfinderError::WindowRefused)?;
                let (Some(end), Some(hold)) = (window.path.waypoints.last(), window.committed_reservations.last())
                else {
                    unreachable!("a window from {at:?} to {queue_cell:?} commits a hold on {queue_cell:?}");
                };
                (at, facing) = (queue_cell, end.orientation);
                journey.predecessor = Some(hold.clone());
                journey.sim_start = end.end_time();
                moved = true;
                break;
            }
            let held = self.held_on_route(at, facing, to, drive, oracle, &journey);
            if held.is_empty() {
                // From its new cell the target may be in reach: look again.
                if moved {
                    continue;
                }
                // Nothing to await: the target's own search names the failure.
                return self.reserve_window(at, facing, to, drive, oracle, &journey);
            }
            if at == from && !windows.allows_origin_wait() {
                return Err(PathfinderError::OriginWaitRefused { origin: from, held: held.into_vec() });
            }
            oracle.commit_touching(&held, seen).await;
            // Merged, not replaced: a cell whose hold a commit bounded before
            // a later await (scheduler time) can still move the target's
            // search later (simtime).
            for cell in held {
                if !awaited.contains(&cell) {
                    awaited.push(cell);
                }
            }
        }
    }

    /// The search to the target from `at`: first from `config.sim_start`
    /// (simtime); when that fails in a way a hold can explain, once more from
    /// the latest start toward the clearance of a cell of `awaited` on the
    /// robot's empty-floor route from `at`, when that is later, so the wait on
    /// `at` does not spend the horizon.
    #[allow(clippy::too_many_arguments)]
    fn search_target(
        &self,
        at: CellId,
        facing: Orientation,
        to: CellId,
        awaited: &[CellId],
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<(Path, SearchStats), PathfinderError> {
        let error = match self.search_window(at, facing, to, to, drive, oracle, config) {
            Ok(found) => return Ok(found),
            Err(error) if explained_by_a_hold(&error) => error,
            Err(error) => return Err(error),
        };
        let on_route = self.awaited_on_route(awaited, at, facing, to, drive, config);
        let later =
            on_route.into_iter().map(|cell| self.start_toward_clearance(at, facing, cell, to, drive, oracle, config));
        match later.max().filter(|later| *later > config.sim_start) {
            Some(later) => {
                let deferred = CollisionConfig { sim_start: later, ..config.clone() };
                self.search_window(at, facing, to, to, drive, oracle, &deferred)
            }
            None => Err(error),
        }
    }

    /// One window to `to`, the journey's target: one SIPP-A\* search, one
    /// atomic commit.
    pub(crate) fn reserve_window(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<ReservedPath, PathfinderError> {
        let (path, stats) = self.search_window(from, from_orientation, to, to, drive, oracle, config)?;
        self.commit_window(path, stats, to, oracle, config)
    }

    /// A window's search, committing nothing.
    #[allow(clippy::too_many_arguments)]
    fn search_window(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        bound_for: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<(Path, SearchStats), PathfinderError> {
        if from == to {
            return Err(PathfinderError::ZeroLength { cell: from });
        }
        // Ghost removes contention, but must preserve the selected physics.
        // The spatial shortcut only implements constant-speed travel.
        if !oracle.tracks_reservations() && drive.kind() == MotionKind::Constant {
            return Ok((self.find_path(from, from_orientation, to, drive, config.sim_start)?, SearchStats::default()));
        }
        crate::perf_span!("fh_sipp_find_path_reserved");
        self.find_window_reserved(from, from_orientation, to, bound_for, drive, oracle, config)
    }

    /// Commit a window's searched path to `to` in one atomic commit.
    fn commit_window(
        &self,
        path: Path,
        mut stats: SearchStats,
        to: CellId,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<ReservedPath, PathfinderError> {
        // Empty path: degenerate but legal (e.g., zero-Move plan). No commit.
        if path.waypoints.is_empty() {
            return Ok(ReservedPath { path, stats, committed_reservations: vec![] });
        }

        // Compose Waypoint chain into a CommitPlan per spec §3. Terminal
        // dwell flows here (it is a reservation property, not a path
        // property — paths always end with PathEnd).
        let tracking = oracle.tracks_reservations();
        let plan = {
            crate::perf_span!("fh_sipp_compose_commit_plan");
            compose_commit_plan(&path.waypoints, tracking && config.predecessor.is_some(), config.dest_dwell)
        };
        if plan.requests.is_empty() {
            return Ok(ReservedPath { path, stats, committed_reservations: vec![] });
        }

        // Single atomic commit (cells + predecessor close + edges, all under
        // one write lock per the table). A conflict signals search/commit
        // drift — surfaced as CommitConflict per spec §6 step 4.
        let committed = {
            crate::perf_span!("fh_sipp_commit_path");
            oracle
                .commit_path(config.self_robot, &plan, config.predecessor.as_ref().filter(|_| tracking))
                .map_err(|c| PathfinderError::CommitConflict { from: c.cell, to })?
        };

        stats.path_length = path.waypoints.len();
        stats.cells_committed = committed.len();

        Ok(ReservedPath { path, stats, committed_reservations: committed })
    }
}

/// A search failure an open-ended hold on the way can explain, so a shorter
/// window or an await may still get the journey there.
fn explained_by_a_hold(error: &PathfinderError) -> bool {
    matches!(
        error,
        PathfinderError::Unreachable { .. }
            | PathfinderError::UnreachableWithinHorizon { .. }
            | PathfinderError::DestinationExceedsMaxWait { .. }
    )
}

/// Whether a journey may end on a cell of `kind`, read off the kind alone.
/// The ends are where a task delivers a robot: an operator's presentation
/// cell, a handoff cell (a belt's port or a relay's access cell), a relay
/// shelf (where a legacy design's HTM stands beneath it), a charger, and open
/// floor (an order area's entry and waiting cells). The rest are on the way
/// (a queue, a highway, an aisle) or no place to stop (a rack, a
/// workstation's own cell, a junction node, a blank). No wildcard: a new kind
/// is ruled on here before anything can target it.
pub fn is_journey_end(kind: CellKind) -> bool {
    match kind {
        CellKind::Presentation | CellKind::Exchange | CellKind::Relay | CellKind::Charging | CellKind::Open => true,
        CellKind::QueueCell
        | CellKind::Highway
        | CellKind::Aisle
        | CellKind::Storage
        | CellKind::StationArea
        | CellKind::Junction
        | CellKind::Blank => false,
    }
}

/// Compose a Waypoint chain into a `CommitPlan` per spec §3.
///
/// One `ReservationRequest` per cell advance. Walks the chain tracking a
/// "currently open epoch" (the cell whose composition is in-progress).
/// A `Move` closes the open epoch (or sets `close_predecessor_at` if a
/// caller-supplied predecessor's hold covers the start cell) and opens a
/// new epoch at the destination. `PathEnd` leaves the last epoch
/// open-ended unless a finite `dest_dwell` is requested, in which case the
/// last epoch ends at `PathEnd.time + dest_dwell` — the dwell is a
/// reservation property, applied here, not encoded in the path.
fn compose_commit_plan(waypoints: &[Waypoint], has_predecessor: bool, dest_dwell: Option<SimDuration>) -> CommitPlan {
    if waypoints.is_empty() {
        return CommitPlan::empty();
    }

    let start_cell = waypoints[0].cell;
    let start_time = match &waypoints[0].action {
        WaypointAction::PathStart { time } => *time,
        other => panic!("compose_commit_plan: expected PathStart at index 0, got {:?}", other),
    };

    struct Epoch {
        cell: CellId,
        start: SimTime,
        end: Option<SimTime>,
        move_op: Option<MoveOp>,
    }
    let mut epochs: Vec<Epoch> = Vec::new();
    let mut close_predecessor_at: Option<SimTime> = None;

    // Open the start cell's epoch unless a predecessor's hold already covers it.
    if !has_predecessor {
        epochs.push(Epoch { cell: start_cell, start: start_time, end: None, move_op: None });
    }

    let mut prev_cell = start_cell;

    for wp in waypoints.iter().skip(1) {
        match &wp.action {
            WaypointAction::PathStart { .. } => {
                panic!("compose_commit_plan: PathStart at non-zero index");
            }
            WaypointAction::Move { start: m_start, end: m_end, .. } => {
                // Close the open epoch (or set predecessor close).
                if let Some(last) = epochs.last_mut() {
                    if last.end.is_none() {
                        last.end = Some(*m_end);
                    }
                } else {
                    debug_assert!(has_predecessor, "no epoch + no predecessor");
                    close_predecessor_at = Some(*m_end);
                }
                // Open new epoch at the destination cell.
                epochs.push(Epoch {
                    cell: wp.cell,
                    start: *m_start,
                    end: None,
                    move_op: Some(MoveOp {
                        source_cell: prev_cell,
                        dest_cell: wp.cell,
                        window: TimeRange::new(*m_start, *m_end),
                    }),
                });
                prev_cell = wp.cell;
            }
            WaypointAction::Rotate { .. } | WaypointAction::Wait { .. } => {
                // Same-cell action; epoch continues.
            }
            WaypointAction::PathEnd { time } => {
                // Apply finite terminal dwell to the open epoch. None and
                // Some(MAX) leave it open-ended (parking, indefinite
                // holds). Every other Some(d) — including
                // Some(SimDuration::ZERO) — is finite; the search treats
                // zero dwell as a finite goal hold ending at PathEnd.time
                // (see expand.rs::try_accept_goal), so the commit plan
                // must do the same.
                if let (Some(d), Some(last)) = (dest_dwell.filter(|d| *d != SimDuration::MAX), epochs.last_mut()) {
                    last.end = Some(time.saturating_add(d));
                }
            }
        }
    }

    let requests: Vec<ReservationRequest> =
        epochs.iter().map(|e| ReservationRequest::spanning(e.cell, e.start, e.end)).collect();
    let move_ops: Vec<Option<MoveOp>> = epochs.iter().map(|e| e.move_op).collect();

    CommitPlan { requests, move_ops, close_predecessor_at }
}
