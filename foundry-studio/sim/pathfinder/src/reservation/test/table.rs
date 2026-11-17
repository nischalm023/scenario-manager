//! Unit tests for `ReservationTable::reserve_path` and friends.
//!
//! Covers the single-shot atomic commit contract (spec §6) and the
//! predecessor / CommitPlan structural invariants validated before any
//! mutation.

use desgraph::CellId;

use crate::RobotId;
use crate::{CommitPlan, MoveOp, ReservationRequest};

use super::super::cell_reservation::CellReservation;
use super::super::table::ReservationTable;
use crate::{SimDuration, SimTime, TimeRange};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn table() -> ReservationTable {
    ReservationTable::new(SimDuration::ZERO)
}
fn cell(n: u32) -> CellId {
    CellId(n)
}
fn robot(n: u32) -> RobotId {
    RobotId(n)
}
fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}
fn d(ms: u64) -> SimDuration {
    SimDuration::ms(ms)
}
fn tw(start: u64, end: u64) -> TimeRange {
    TimeRange::new(t(start), t(end))
}

fn req(c: CellId, start: SimTime, end: Option<SimTime>) -> ReservationRequest {
    ReservationRequest::spanning(c, start, end)
}

fn move_op(src: CellId, dst: CellId, start: SimTime, end: SimTime) -> MoveOp {
    MoveOp { source_cell: src, dest_cell: dst, window: TimeRange::new(start, end) }
}

fn reserve_cell(tbl: &ReservationTable, r: RobotId, c: CellId, w: TimeRange) -> CellReservation {
    tbl.reserve_cell(r, c, w).expect("single-cell reserve should succeed")
}

fn assert_free(tbl: &ReservationTable, c: CellId, at: u64, dur: u64) {
    assert_eq!(
        tbl.earliest_free(c, t(at), d(dur), None),
        Some(t(at)),
        "expected cell {:?} free at {} for {}ms",
        c,
        at,
        dur
    );
}

fn assert_busy(tbl: &ReservationTable, c: CellId, at: u64, dur: u64) {
    assert_ne!(
        tbl.earliest_free(c, t(at), d(dur), None),
        Some(t(at)),
        "expected cell {:?} busy at {} for {}ms",
        c,
        at,
        dur
    );
}

// ---------------------------------------------------------------------------
// Basic reserve_path / earliest_free
// ---------------------------------------------------------------------------

#[test]
fn empty_table_always_free() {
    let tbl = table();
    assert_free(&tbl, cell(0), 0, 1000);
    assert_eq!(tbl.active_count(), 0);
}

#[test]
fn non_overlapping_same_cell() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(0, 500));
    assert_free(&tbl, cell(0), 700, 300);
}

#[test]
fn overlapping_same_cell() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(0, 1000));
    assert_busy(&tbl, cell(0), 500, 1000);
}

#[test]
fn gap_enforcement() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(0, 500));
    assert_free(&tbl, cell(0), 500, 300);
    assert_busy(&tbl, cell(0), 400, 400);
}

#[test]
fn predecessor_exclusion_earliest_free() {
    let tbl = table();
    let r = reserve_cell(&tbl, robot(0), cell(0), tw(0, 1000));
    assert_eq!(tbl.earliest_free(cell(0), t(0), d(500), None), Some(t(1000)));
    assert_eq!(tbl.earliest_free(cell(0), t(0), d(500), Some(&r)), Some(t(0)));
}

#[test]
fn conflicting_commit_rejected() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(500, 1000));
    let plan = CommitPlan::single(req(cell(0), t(600), Some(t(900))));
    let result = tbl.reserve_path(robot(1), &plan, None);
    assert!(result.is_err(), "overlapping commit should be rejected");
    assert_eq!(tbl.active_count(), 1, "no side effects on rejected commit");
}

#[test]
fn multiple_robots_different_cells() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(0, 1000));
    reserve_cell(&tbl, robot(1), cell(1), tw(0, 1000));
    assert_busy(&tbl, cell(0), 0, 1000);
    assert_busy(&tbl, cell(1), 0, 1000);
}

#[test]
fn open_ended_hold_blocks_forever() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(0)));
    assert_eq!(tbl.earliest_free(cell(0), t(1_000_000), d(1_000_000), None), None);
}

#[test]
fn clone_shares_state() {
    let tbl = table();
    let tbl2 = tbl.clone();
    reserve_cell(&tbl, robot(0), cell(0), tw(0, 1000));
    assert_busy(&tbl2, cell(0), 500, 300);
    assert_eq!(tbl2.active_count(), 1);
}

#[test]
fn earliest_free_finds_slot_after_reservation() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(1000, 5000));
    assert_eq!(tbl.earliest_free(cell(0), t(0), d(2000), None), Some(t(5000)));
    assert_eq!(tbl.earliest_free(cell(0), t(0), d(500), None), Some(t(0)));
}

#[test]
fn time_window_overlaps() {
    assert!(tw(0, 100).overlaps(tw(50, 150), d(0)));
    assert!(tw(50, 150).overlaps(tw(0, 100), d(0)));
    assert!(!tw(0, 100).overlaps(tw(100, 200), d(0)));
    assert!(!tw(100, 200).overlaps(tw(0, 100), d(0)));
    assert!(tw(0, 100).overlaps(tw(0, 100), d(0)));
    assert!(tw(0, 200).overlaps(tw(50, 150), d(0)));
    assert!(tw(50, 150).overlaps(tw(0, 200), d(0)));
}

#[test]
fn open_ended_lists_every_dangling_hold() {
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), tw(0, 1000));
    reserve_cell(&tbl, robot(1), cell(1), TimeRange::open_ended(t(500)));
    reserve_cell(&tbl, robot(2), cell(2), TimeRange::open_ended(t(600)));
    let open = tbl.open_ended();
    assert_eq!(open.len(), 2);
    let cells: Vec<CellId> = open.iter().map(|r| r.cell()).collect();
    assert!(cells.contains(&cell(1)));
    assert!(cells.contains(&cell(2)));
}

// ---------------------------------------------------------------------------
// Predecessor finalize — happy path
// ---------------------------------------------------------------------------

#[test]
fn finalize_sets_end_on_open_ended_predecessor() {
    let tbl = table();
    // Upstream produces an open-ended reservation at cell 0.
    let pred = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    assert!(pred.is_open_ended());

    // Plan: a Move out of cell 0 (predecessor) into cell 1, with the
    // close time bound to the move-end. Per the new contract the
    // predecessor's own cell is NOT in plan.requests; only the new
    // destination is.
    let plan = CommitPlan {
        requests: vec![req(cell(1), t(2000), Some(t(3000)))],
        move_ops: vec![Some(move_op(cell(0), cell(1), t(2000), t(3000)))],
        close_predecessor_at: Some(t(3000)),
    };

    let out = tbl.reserve_path(robot(0), &plan, Some(&pred)).expect("predecessor close should succeed");
    // Result vec contains only newly created reservations (matches
    // TableOracle contract); the predecessor finalize is a side effect.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].cell(), cell(1));
    assert_eq!(out[0].window(), TimeRange::new(t(2000), t(3000)));

    // Verify the side effect: predecessor's reservation is finalized.
    let resv0 = tbl.reservations_for_cell(cell(0));
    assert_eq!(resv0.len(), 1);
    assert_eq!(resv0[0].id(), pred.id());
    assert_eq!(resv0[0].window(), TimeRange::new(t(1000), t(3000)));
    assert!(!resv0[0].is_open_ended());
    // No remaining open-ended dangling holds (cell 1's end is finite).
    assert_eq!(tbl.open_ended().len(), 0);
}

// ---------------------------------------------------------------------------
// Predecessor invariants — Phase 1a/1b/1c/1d rejections
// ---------------------------------------------------------------------------

#[test]
fn close_predecessor_before_hold_start_is_rejected_not_panicked() {
    // Phase 1d ordering check. `close_at` precedes the predecessor hold's own
    // start, so the close window is not representable as a TimeRange. That is
    // a malformed plan and must surface as the same Err the `!has_match` arm
    // returns — NOT as a panic from inside TimeRange::new, which would abort
    // the run from within the branch whose job is to diagnose plan drift.
    let tbl = table();
    let pred = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    let active_before = tbl.active_count();

    let plan = CommitPlan {
        requests: vec![req(cell(1), t(2000), Some(t(3000)))],
        move_ops: vec![Some(move_op(cell(0), cell(1), t(2000), t(3000)))],
        // Before the predecessor's start of t(1000).
        close_predecessor_at: Some(t(500)),
    };

    let result = tbl.reserve_path(robot(0), &plan, Some(&pred));
    assert!(result.is_err(), "a close preceding the hold start must be rejected, not panic");
    assert_eq!(tbl.active_count(), active_before, "no side effects on the rejected path");
    // The predecessor must still be open — Phase 4's finalize never ran.
    let resv0 = tbl.reservations_for_cell(cell(0));
    assert_eq!(resv0.len(), 1);
    assert!(resv0[0].is_open_ended(), "predecessor hold must be untouched");
}

#[test]
fn close_predecessor_without_predecessor_rejected() {
    // Phase 1a: close_predecessor_at = Some, predecessor = None ⇒ Err,
    // no side effects.
    let tbl = table();
    reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    let active_before = tbl.active_count();
    let plan = CommitPlan {
        requests: vec![req(cell(1), t(2000), Some(t(3000)))],
        move_ops: vec![Some(move_op(cell(0), cell(1), t(2000), t(3000)))],
        close_predecessor_at: Some(t(3000)),
    };
    let result = tbl.reserve_path(robot(0), &plan, None);
    assert!(result.is_err(), "close without predecessor must be rejected");
    assert_eq!(tbl.active_count(), active_before, "no side effects");
}

#[test]
fn missing_predecessor_returns_err_not_panic() {
    // Phase 1b: predecessor reservation does not exist in the table —
    // legacy code would have panicked inside finalize. Now Err with no
    // mutation.
    let tbl = table();
    // Construct a fake predecessor that was never committed.
    let phantom = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    let other_tbl = table();
    let active_before = other_tbl.active_count();
    let plan = CommitPlan {
        requests: vec![req(cell(1), t(2000), Some(t(3000)))],
        move_ops: vec![Some(move_op(cell(0), cell(1), t(2000), t(3000)))],
        close_predecessor_at: Some(t(3000)),
    };
    let result = other_tbl.reserve_path(robot(0), &plan, Some(&phantom));
    assert!(result.is_err(), "missing predecessor must be Err");
    assert_eq!(other_tbl.active_count(), active_before, "no side effects on rejection");
}

#[test]
fn wrong_robot_predecessor_returns_err() {
    // Phase 1b: predecessor reservation belongs to a different robot.
    let tbl = table();
    let pred = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    let active_before = tbl.active_count();
    let plan = CommitPlan {
        requests: vec![req(cell(1), t(2000), Some(t(3000)))],
        move_ops: vec![Some(move_op(cell(0), cell(1), t(2000), t(3000)))],
        close_predecessor_at: Some(t(3000)),
    };
    // robot(1) trying to commit using robot(0)'s predecessor.
    let result = tbl.reserve_path(robot(1), &plan, Some(&pred));
    assert!(result.is_err(), "wrong-robot predecessor must be Err");
    assert_eq!(tbl.active_count(), active_before, "no side effects on rejection");
    // Predecessor untouched.
    assert!(tbl.reservations_for_cell(cell(0))[0].is_open_ended());
}

#[test]
fn duplicate_predecessor_cell_request_rejected() {
    // Phase 1c: a plan request that overlaps the predecessor's still-open hold
    // is rejected — it would duplicate that hold or false-conflict against it.
    // The request below starts at t(2000) and the hold does not close until
    // t(3500), so the two overlap.
    let tbl = table();
    let pred = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    let active_before = tbl.active_count();
    // Malformed plan: includes a request on cell(0), which is the
    // predecessor's cell.
    let plan = CommitPlan {
        requests: vec![req(cell(0), t(2000), Some(t(3000))), req(cell(1), t(2500), Some(t(3500)))],
        move_ops: vec![None, Some(move_op(cell(0), cell(1), t(2500), t(3500)))],
        close_predecessor_at: Some(t(3500)),
    };
    let result = tbl.reserve_path(robot(0), &plan, Some(&pred));
    assert!(result.is_err(), "duplicate predecessor-cell request must be rejected");
    assert_eq!(tbl.active_count(), active_before, "no side effects on rejection");
    assert!(tbl.reservations_for_cell(cell(0))[0].is_open_ended(), "predecessor untouched");
    assert!(tbl.reservations_for_cell(cell(1)).is_empty(), "destination cell untouched");
}

/// A robot that steps aside and comes back claims its old cell twice, and both
/// claims are true.
///
/// This is the manoeuvre a search makes when the cell it is heading for is busy
/// for a stretch: leave, stand out of the way, return, carry on. It is the
/// ordinary shape wherever the destination sits in a traffic lane — which is
/// what a handoff shelf's standing place became once racks acquired a depth and
/// the shelf stopped being worked from the steel itself. Refusing the re-entry
/// refuses the wait, and the run aborts over a manoeuvre the search chose on
/// purpose.
#[test]
fn a_path_that_leaves_and_returns_claims_its_start_cell_again() {
    let tbl = table();
    let pred = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    // Out to cell 1, stand there, back to cell 0, on to cell 2.
    let plan = CommitPlan {
        requests: vec![
            req(cell(1), t(2000), Some(t(6000))),
            req(cell(0), t(5000), Some(t(8000))),
            req(cell(2), t(7000), None),
        ],
        move_ops: vec![
            Some(move_op(cell(0), cell(1), t(2000), t(3000))),
            Some(move_op(cell(1), cell(0), t(5000), t(6000))),
            Some(move_op(cell(0), cell(2), t(7000), t(8000))),
        ],
        close_predecessor_at: Some(t(3000)),
    };

    let out = tbl.reserve_path(robot(0), &plan, Some(&pred)).expect("a re-entry after the close commits");
    assert_eq!(out.len(), 3);

    // The first hold ends where the robot left; the second starts where it came
    // back. One cell, two windows, no overlap.
    let resv0 = tbl.reservations_for_cell(cell(0));
    assert_eq!(resv0.len(), 2);
    assert_eq!(resv0[0].window(), tw(1000, 3000));
    assert_eq!(resv0[1].window(), tw(5000, 8000));
    assert!(resv0.iter().all(|r| !r.is_open_ended()));
}

/// Another robot standing on that cell while this one is away still decides.
/// The predecessor's own hold is excluded from the conflict check on its own
/// cell; nobody else's is.
#[test]
fn a_return_into_another_robots_window_is_still_refused() {
    let tbl = table();
    // The hold this path leaves behind runs out exactly when the robot leaves,
    // which is what lets the other robot stand there while it is away.
    let pred = reserve_cell(&tbl, robot(0), cell(0), tw(1000, 3000));
    reserve_cell(&tbl, robot(1), cell(0), tw(4000, 9000));
    let active_before = tbl.active_count();

    let plan = CommitPlan {
        requests: vec![
            req(cell(1), t(2000), Some(t(6000))),
            req(cell(0), t(5000), Some(t(8000))),
            req(cell(2), t(7000), None),
        ],
        move_ops: vec![
            Some(move_op(cell(0), cell(1), t(2000), t(3000))),
            Some(move_op(cell(1), cell(0), t(5000), t(6000))),
            Some(move_op(cell(0), cell(2), t(7000), t(8000))),
        ],
        close_predecessor_at: Some(t(3000)),
    };

    let result = tbl.reserve_path(robot(0), &plan, Some(&pred));
    assert!(result.is_err(), "another robot holds the cell when this one returns");
    assert_eq!(tbl.active_count(), active_before, "no side effects on rejection");
    assert_eq!(tbl.reservations_for_cell(cell(0))[0].window(), tw(1000, 3000), "predecessor untouched");
}

#[test]
fn predecessor_close_without_matching_move_op_rejected() {
    // Phase 1d.i: close_predecessor_at = Some(t), but no MoveOp in the
    // plan has source_cell == pred.cell() and move_end == t.
    let tbl = table();
    let pred = reserve_cell(&tbl, robot(0), cell(0), TimeRange::open_ended(t(1000)));
    let active_before = tbl.active_count();
    // MoveOp source_cell mismatches pred.cell() (claims a Move out of
    // cell(2) instead of cell(0)).
    let plan = CommitPlan {
        requests: vec![req(cell(1), t(2000), Some(t(3000)))],
        move_ops: vec![Some(move_op(cell(2), cell(1), t(2000), t(3000)))],
        close_predecessor_at: Some(t(3000)),
    };
    let result = tbl.reserve_path(robot(0), &plan, Some(&pred));
    assert!(result.is_err(), "close without matching MoveOp must be rejected");
    assert_eq!(tbl.active_count(), active_before, "no side effects on rejection");
    assert!(tbl.reservations_for_cell(cell(0))[0].is_open_ended(), "predecessor untouched");
}

#[test]
fn predecessor_close_widening_rejected() {
    // Phase 1d.ii: close narrows or matches the LIVE predecessor end —
    // never widens. (Exercises the case where the live reservation is
    // already finite, e.g., a station-dwell hold.)
    let tbl = table();
    // Stage a finite predecessor (e.g., dwell ending at 5000).
    let pred = reserve_cell(&tbl, robot(0), cell(0), tw(1000, 5000));
    let active_before = tbl.active_count();
    // Close at 6000 would WIDEN the existing 5000 end.
    let plan = CommitPlan {
        requests: vec![req(cell(1), t(5000), Some(t(6000)))],
        move_ops: vec![Some(move_op(cell(0), cell(1), t(5000), t(6000)))],
        close_predecessor_at: Some(t(6000)),
    };
    let result = tbl.reserve_path(robot(0), &plan, Some(&pred));
    assert!(result.is_err(), "widening close must be rejected");
    assert_eq!(tbl.active_count(), active_before, "no side effects on rejection");
    let resv0 = tbl.reservations_for_cell(cell(0));
    assert_eq!(resv0[0].window(), tw(1000, 5000), "predecessor end unchanged");
}

// ---------------------------------------------------------------------------
// CommitPlan structural invariants — Phase 0p1 / 0p2
// ---------------------------------------------------------------------------

#[test]
fn commit_plan_move_ops_length_mismatch_rejected() {
    // Phase 0p1: move_ops.len() must equal requests.len().
    let tbl = table();
    let active_before = tbl.active_count();
    let plan = CommitPlan {
        requests: vec![req(cell(0), t(0), Some(t(1000))), req(cell(1), t(1000), Some(t(2000)))],
        move_ops: vec![None], // length 1, requests length 2 — malformed.
        close_predecessor_at: None,
    };
    let result = tbl.reserve_path(robot(0), &plan, None);
    assert!(result.is_err(), "length mismatch must be rejected");
    assert_eq!(tbl.active_count(), active_before, "no side effects");
}

#[test]
fn commit_plan_move_op_dest_cell_mismatch_rejected() {
    // Phase 0p2: every Some(op) at index i must have op.dest_cell == requests[i].cell.
    let tbl = table();
    let active_before = tbl.active_count();
    // Request lands cell(1) but the MoveOp claims dest_cell = cell(2).
    let plan = CommitPlan {
        requests: vec![req(cell(0), t(0), Some(t(1000))), req(cell(1), t(1000), Some(t(2000)))],
        move_ops: vec![None, Some(move_op(cell(0), cell(2), t(1000), t(2000)))],
        close_predecessor_at: None,
    };
    let result = tbl.reserve_path(robot(0), &plan, None);
    assert!(result.is_err(), "dest_cell mismatch must be rejected");
    assert_eq!(tbl.active_count(), active_before, "no side effects");
}

#[test]
fn valid_commit_plan_pairing_commits_cells_and_edges() {
    // Phase 0p2 happy path: matching dest_cell on each Some(op).
    // Verifies cells and edges land atomically under one write lock.
    let tbl = table();
    // Fresh path (no predecessor): start cell + transit.
    let plan = CommitPlan {
        requests: vec![req(cell(0), t(0), Some(t(1000))), req(cell(1), t(1000), Some(t(2000)))],
        // First request opens the start cell — no MoveOp.
        // Second request is a Move-into-cell(1) at [1000, 2000).
        move_ops: vec![None, Some(move_op(cell(0), cell(1), t(1000), t(2000)))],
        close_predecessor_at: None,
    };
    let out = tbl.reserve_path(robot(0), &plan, None).expect("valid plan should commit");
    assert_eq!(out.len(), 2, "two cells committed");
    assert_eq!(tbl.active_count(), 2);
    // Verify the edge reservation is in place: a counter-flow Move
    // (cell(1) → cell(0)) over the same window must conflict.
    assert!(
        tbl.has_edge_conflict(cell(1), cell(0), t(1000), d(1000), Some(robot(99))),
        "edge cell(0)→cell(1) should block the reverse during [1000, 2000)"
    );
}

// ---------------------------------------------------------------------------
// Cell-duty accumulator: golden equivalence vs history scan
// ---------------------------------------------------------------------------

/// Drive a sequence of reserve_cell + finalize calls through the live
/// table, then assert the accumulator-projected stats equal the
/// history-scan stats cell-by-cell. Production code paths are
/// append-only (`pos == list.len()`) — verified in
/// `model::relay::tests::scale::*` — so equality should be exact for
/// every stat including the documented monotonic-max fields.
#[test]
fn accumulator_matches_history_scan_for_chronological_ops() {
    use super::super::diagnostics::cell_duty_stats_from_history;

    let tbl = table();

    // Robot 1: open-ended hold on cell(0), then a finite traversal of
    // cell(0) → cell(1) → cell(2).
    let r1_hold = reserve_cell(&tbl, robot(1), cell(0), TimeRange::open_ended(t(0)));
    let r1_path = tbl.reserve_path(
        robot(1),
        &CommitPlan {
            requests: vec![req(cell(1), t(1000), Some(t(2000))), req(cell(2), t(2000), Some(t(3000)))],
            move_ops: vec![
                Some(move_op(cell(0), cell(1), t(1000), t(1000))),
                Some(move_op(cell(1), cell(2), t(2000), t(2000))),
            ],
            close_predecessor_at: Some(t(1000)),
        },
        Some(&r1_hold),
    );
    assert!(r1_path.is_ok(), "robot 1 path commit should succeed");

    // Robot 2: open-ended hold on cell(3), then traversal cell(3) → cell(2).
    // Arrives at cell(2) AFTER robot 1 vacates (t=3000).
    let r2_hold = reserve_cell(&tbl, robot(2), cell(3), TimeRange::open_ended(t(500)));
    let r2_path = tbl.reserve_path(
        robot(2),
        &CommitPlan {
            requests: vec![req(cell(2), t(3500), Some(t(4500)))],
            move_ops: vec![Some(move_op(cell(3), cell(2), t(3500), t(3500)))],
            close_predecessor_at: Some(t(3500)),
        },
        Some(&r2_hold),
    );
    assert!(r2_path.is_ok(), "robot 2 path commit should succeed");

    let horizon_ms = 10_000;
    for c in [cell(0), cell(1), cell(2), cell(3)] {
        let acc_stats = tbl.cell_duty_stats(c, horizon_ms);

        let history = tbl.reservations_for_cell(c);
        let history_stats = cell_duty_stats_from_history(history, horizon_ms);

        assert_eq!(acc_stats.reservations, history_stats.reservations, "cell {:?}: reservations", c);
        assert_eq!(acc_stats.open_ended, history_stats.open_ended, "cell {:?}: open_ended", c);
        assert_eq!(acc_stats.first_start_ms, history_stats.first_start_ms, "cell {:?}: first_start_ms", c);
        assert_eq!(acc_stats.frontier_ms, history_stats.frontier_ms, "cell {:?}: frontier_ms", c);
        assert_eq!(acc_stats.busy_ms, history_stats.busy_ms, "cell {:?}: busy_ms", c);
        assert_eq!(acc_stats.gap_ms, history_stats.gap_ms, "cell {:?}: gap_ms", c);
        assert_eq!(acc_stats.gap_count, history_stats.gap_count, "cell {:?}: gap_count", c);
        assert_eq!(acc_stats.longest_gap_ms, history_stats.longest_gap_ms, "cell {:?}: longest_gap_ms", c);
        assert_eq!(acc_stats.longest_hold_ms, history_stats.longest_hold_ms, "cell {:?}: longest_hold_ms", c);
        assert!(
            (acc_stats.start_interval_stddev_ms - history_stats.start_interval_stddev_ms).abs() < 1e-9,
            "cell {:?}: stddev {} vs {}",
            c,
            acc_stats.start_interval_stddev_ms,
            history_stats.start_interval_stddev_ms,
        );
    }
}

/// Watermark pruning drops finite reservations whose end is in the past,
/// preserves open-ended ones, and leaves the duty accumulator intact so
/// `cell_duty_stats` returns the same numbers before and after.
#[test]
fn prune_below_watermark_drops_past_finite_preserves_open_ended_and_stats() {
    let tbl = table();
    // Set up cell(0): three finite reservations ending at 1000, 2500,
    // 5000 (all "in the past" once `now` advances to >= 7000), plus an
    // open-ended hold starting at 5000.
    let _r1 = reserve_cell(&tbl, robot(1), cell(0), tw(0, 1000));
    let _r2 = reserve_cell(&tbl, robot(2), cell(0), tw(1500, 2500));
    let _r3 = reserve_cell(&tbl, robot(3), cell(0), tw(3000, 5000));
    let _hold = reserve_cell(&tbl, robot(4), cell(0), TimeRange::open_ended(t(5000)));

    let horizon_ms = 10_000;
    let pre_stats = tbl.cell_duty_stats(cell(0), horizon_ms);

    // Sanity: we have 4 reservations, 1 open-ended.
    assert_eq!(pre_stats.reservations, 4);
    assert_eq!(pre_stats.open_ended, 1);
    assert_eq!(tbl.active_count(), 4);

    // Prune at now=7000 with 1000ms margin → watermark=6000. Finite
    // reservations ending at 1000/2500/5000 all qualify; open-ended stays.
    tbl.prune_below_watermark(t(7000), d(1000));

    // Live table should now have only the open-ended.
    let remaining = tbl.reservations_for_cell(cell(0));
    assert_eq!(remaining.len(), 1, "expected only the open-ended to survive");
    assert!(remaining[0].is_open_ended(), "surviving entry should be open-ended");
    assert_eq!(tbl.active_count(), 1);

    // Accumulator + duty stats should be identical pre/post prune.
    let post_stats = tbl.cell_duty_stats(cell(0), horizon_ms);

    assert_eq!(pre_stats.reservations, post_stats.reservations);
    assert_eq!(pre_stats.open_ended, post_stats.open_ended);
    assert_eq!(pre_stats.first_start_ms, post_stats.first_start_ms);
    assert_eq!(pre_stats.frontier_ms, post_stats.frontier_ms);
    assert_eq!(pre_stats.busy_ms, post_stats.busy_ms);
    assert_eq!(pre_stats.gap_ms, post_stats.gap_ms);
    assert_eq!(pre_stats.gap_count, post_stats.gap_count);
    assert_eq!(pre_stats.longest_gap_ms, post_stats.longest_gap_ms);
    assert_eq!(pre_stats.longest_hold_ms, post_stats.longest_hold_ms);
}

/// Regression guard: committing routes through `reserve_path` must never
/// silently drop an existing live reservation, no matter how many commits
/// happen or how far in the future their windows land.
#[test]
fn reserve_path_never_drops_a_live_reservation() {
    let tbl = table(); // gap = ZERO, matching the production default.

    // Robot 1 holds cell(99) over [10_000, 40_000].
    let _occupied = reserve_cell(&tbl, robot(1), cell(99), tw(10_000, 40_000));

    // Many unrelated commits — enough to trip the old 1000-commit auto-prune.
    for i in 0..1000 {
        let c = cell(1000 + i);
        let _ = reserve_cell(&tbl, robot(2), c, tw(0, 1));
    }

    // A far-future commit (a long route's tail cell). Under the old auto-prune
    // this anchored the watermark at 100_000 - 1000 = 99_000 and pruned
    // cell(99)'s [10_000, 40_000] hold even though it was entirely in the future.
    let _far = reserve_cell(&tbl, robot(3), cell(50), tw(100_000, 101_000));

    // cell(99) must still block a robot crossing it at t=20_000.
    assert!(
        tbl.has_conflict(cell(99), t(20_000), d(1000), None),
        "cell(99) reservation [10_000,40_000] was silently dropped — robots would route through robot 1",
    );
}

// ---------------------------------------------------------------------------
// subsume_start_right — the destination-row third shape's table op
// ---------------------------------------------------------------------------

#[test]
fn subsume_moves_an_open_start_right_preserving_id_and_open_end() {
    let tbl = table();
    let row = reserve_cell(&tbl, robot(1), cell(5), TimeRange::open_ended(t(1000)));
    let updated = tbl.subsume_start_right(&row, t(1500));
    assert_eq!(updated.id(), row.id(), "the claim's identity survives the subsume");
    assert_eq!(updated.window().start(), t(1500));
    assert!(updated.window().is_open_ended());
    // The table's live row moved too: the cell is now free before 1500.
    assert_eq!(tbl.earliest_free(cell(5), t(1000), d(100), None), Some(t(1000)));
    let live = tbl.reservations_for_cell(cell(5));
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].window().start(), t(1500));
}

#[test]
fn subsume_at_the_exact_promoted_start_is_legal() {
    let tbl = table();
    let row = reserve_cell(&tbl, robot(1), cell(5), TimeRange::open_ended(t(1000)));
    let updated = tbl.subsume_start_right(&row, t(1000));
    assert_eq!(updated.window().start(), t(1000));
}

/// The arrive-after guard: a committed landing earlier than the promoted
/// start is an estimate/schedule divergence, refused loudly.
#[test]
#[should_panic(expected = "arrive-after guard")]
fn subsume_refuses_leftward_motion() {
    let tbl = table();
    let row = reserve_cell(&tbl, robot(1), cell(5), TimeRange::open_ended(t(1000)));
    tbl.subsume_start_right(&row, t(999));
}

/// Only open destination rows subsume; a bounded row means the claim was
/// already closed.
#[test]
#[should_panic(expected = "only open destination rows subsume")]
fn subsume_refuses_bounded_rows() {
    let tbl = table();
    let row = reserve_cell(&tbl, robot(1), cell(5), tw(1000, 2000));
    tbl.subsume_start_right(&row, t(1500));
}

/// The subsumed row keeps excluding other robots after its start moved:
/// the open end still blocks everything at and after the new start.
#[test]
fn subsumed_row_still_excludes_rivals_after_new_start() {
    let tbl = table();
    let row = reserve_cell(&tbl, robot(1), cell(5), TimeRange::open_ended(t(1000)));
    tbl.subsume_start_right(&row, t(1500));
    // A rival can fit strictly before the new start...
    assert_eq!(tbl.earliest_free(cell(5), t(0), d(100), None), Some(t(0)));
    // ...but nothing fits at or after it (open-ended hold → None).
    assert_eq!(tbl.earliest_free(cell(5), t(1500), d(100), None), None);
}
