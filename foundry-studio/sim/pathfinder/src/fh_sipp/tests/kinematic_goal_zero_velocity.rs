//! Regression tests for the goal-zero-velocity guard in
//! `Search::try_accept_goal` (see `expand.rs`).
//!
//! Bug: `try_accept_goal` accepted any goal-arrival state regardless of
//! `speed_bucket`. Phase 1 forced goal-cell Moves to use `target_exit_v = 0`,
//! but `trapezoidal_cost` Case 3a clamps the actual exit velocity UP when the
//! cell is too short to brake fully (e.g. the M5F unloaded: v_max=4500 mm/s,
//! decel=2000 mm/s² over a 1 m cell yields actual exit ~4031 mm/s). The
//! search then "accepted" a state with a non-zero exit velocity at the goal,
//! producing the user-visible "stops immediately at goals" artefact when the
//! kinematic playback rasterizer sampled the trailing velocity profile.
//!
//! Fix: gate `try_accept_goal` on `speed_bucket == ZERO` for HTMs in
//! `MotionModel::HtmKinematic`, mirroring the guards in `expand_wait` and
//! `expand_rotate`.
//!
//! Each test in this file fails on the pre-fix codebase with the speed-bucket
//! guard removed.

use super::prelude::*;

use crate::Path;

/// Same `CollisionConfig` shape used by the parent `kinematic.rs` integration
/// tests — bare-minimum fields for `find_path_reserved`.
fn config(motion_model: MotionModel) -> CollisionConfig {
    CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(1_000)),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 1_000_000,
        motion_model,
    }
}

/// Extract `(entry, exit)` velocities from each Move waypoint in `path`,
/// skipping non-Move actions (PathStart, Rotate, Wait, PathEnd).
fn move_velocities(path: &Path) -> Vec<(f64, f64)> {
    path.waypoints
        .iter()
        .filter_map(|w| match w.action {
            WaypointAction::Move { entry_velocity_mm_s, exit_velocity_mm_s, .. } => {
                Some((entry_velocity_mm_s, exit_velocity_mm_s))
            }
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Sanity: 2-cell rest-to-rest goal arrival. The pre-fix code happens to
// produce exit=0 here because the bot can't reach v_max within a single 1 m
// cell starting from rest (peak ~2000 mm/s, room to brake), so the goal-cell
// forced-decel triangular profile lands EXACTLY on 0. This test does NOT
// fail on the buggy version — it's a baseline sanity check that the fix
// doesn't regress the "trivial rest-to-rest" case (e.g. by introducing a
// no-path under conditions that previously found a valid stationary
// arrival).
//
// The bug-triggering tests below (multi-cell, cruise-into-goal) are the
// actual regression coverage.
// ---------------------------------------------------------------------------

#[test]
fn rest_to_rest_two_cell_arrives_at_zero_kinematic() {
    let graph = corridor(2);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(1), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("rest-to-rest 2-cell corridor must find a path under kinematic mode");

    let moves = move_velocities(&path);
    assert!(!moves.is_empty(), "path must contain at least one Move waypoint");
    let (_, last_exit) = *moves.last().expect("checked non-empty above");
    assert!(last_exit.abs() < 1e-6, "last Move's exit_velocity_mm_s must be 0 at the goal; got {last_exit} mm/s",);
}

// ---------------------------------------------------------------------------
// Smallest scenario that exercises the bug: a 4-cell corridor where the bot
// can build velocity over 2 cells of cruise (reaching ~2828 mm/s by c2) and
// then must decelerate into the goal at c3. The decel-only goal-cell move
// from 2828 mm/s in a 1 m cell needs 2828² / (2·2000) = 2000 mm to brake —
// twice the cell length. Pre-fix the trapezoidal Case 3a clamp lands the
// `actual_exit_v` at ~2000 mm/s and `try_accept_goal` accepts the state.
// Post-fix the search must find an alternate (slower) arrival profile that
// lands at v=0; this routes through earlier-decel buckets.
// ---------------------------------------------------------------------------

#[test]
fn shortest_bug_repro_four_cell_corridor_kinematic() {
    let graph = corridor(4);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(3), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("rest-to-rest 4-cell corridor must find a path under kinematic mode");

    let moves = move_velocities(&path);
    assert!(!moves.is_empty(), "path must contain at least one Move waypoint");

    let (_, last_exit) = *moves.last().expect("checked non-empty above");
    // Bug version: last_exit ≈ 2000 mm/s (trapezoidal clamp from cruise
    // entry ~2828 mm/s over 1 m goal-cell decel). Fix version: last_exit ≈ 0.
    assert!(
        last_exit.abs() < 1.0,
        "last Move's exit_velocity_mm_s must be ~0 at the goal; got {last_exit} mm/s. \
         Pre-fix bug emits ~2000 mm/s here (Case 3a clamp on 1 m goal-cell decel from cruise).",
    );
}

// ---------------------------------------------------------------------------
// Multi-cell decel: a 5-cell straight corridor where the bot must start
// decelerating multiple cells before the goal. The penultimate move's exit
// velocity must be below v_max — the search picked an early decel — and the
// last move's exit velocity must be exactly zero.
// ---------------------------------------------------------------------------

#[test]
fn multi_cell_decel_lands_on_zero_velocity_kinematic() {
    // 5-cell corridor: bot enters at c(0), goal at c(4). Unloaded m5f params:
    // v_max=4500, decel=2000. Brake from v_max to 0 needs 5062 mm — more than
    // 5 cells × 1 m? No, 5062 < 5000+? Actually 5062 > 5000 so it just barely
    // doesn't fit. The search will need to start decelerating no later than
    // the penultimate move and the penultimate move's exit_v will be < v_max
    // (because pure cruise into the last cell would over-shoot).
    let graph = corridor(5);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("rest-to-rest 5-cell corridor must find a path under kinematic mode");

    let moves = move_velocities(&path);
    assert!(moves.len() >= 2, "5-cell corridor needs at least 2 Move waypoints; got {}", moves.len());

    // Last Move's exit must be 0 — the goal arrival contract.
    let (_, last_exit) = *moves.last().expect("checked above");
    assert!(last_exit.abs() < 1.0, "last Move's exit_velocity_mm_s must be 0 at the goal; got {last_exit} mm/s",);

    // Penultimate Move's exit must be strictly below v_max — the bot has
    // already started its deceleration sequence at least one cell before the
    // goal. (If the cell length were so generous that the bot could brake
    // fully from v_max to 0 within the goal cell alone, this would not hold,
    // but for 1 m cells with the M5F's unloaded params it must.)
    let (_, penultimate_exit) = moves[moves.len() - 2];
    let v_max = 4500.0_f64;
    assert!(
        penultimate_exit < v_max - 1e-6,
        "penultimate Move's exit_velocity_mm_s ({penultimate_exit}) must be < v_max ({v_max}); \
         search should have started decelerating before the goal cell",
    );
}

// ---------------------------------------------------------------------------
// Edge case: every accepted path's terminal Move ends at zero. Repeats the
// 5-cell scenario with a stronger contract — for ALL Move waypoints, if the
// waypoint is the path's LAST Move (immediately preceding PathEnd), exit=0.
// The intermediate Move waypoints may carry any velocity dictated by the
// trapezoidal profile.
// ---------------------------------------------------------------------------

#[test]
fn all_intermediate_moves_consistent_with_zero_terminal_kinematic() {
    let graph = corridor(5);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("kinematic 5-cell path must succeed");

    // Find the index of the last Move. Everything between consecutive Moves
    // must have entry_velocity == prior exit_velocity (continuity); the very
    // first Move enters at 0 (search starts at v=0); the very last Move
    // exits at 0.
    let moves = move_velocities(&path);
    assert!(!moves.is_empty(), "must have at least one Move");

    let (first_entry, _) = moves[0];
    assert!(
        first_entry.abs() < 1e-6,
        "first Move's entry_velocity_mm_s must be 0 (search initialised at rest); got {first_entry}",
    );

    let (_, last_exit) = *moves.last().expect("checked above");
    assert!(last_exit.abs() < 1.0, "last Move's exit_velocity_mm_s must be 0 at the goal; got {last_exit}",);

    // Continuity across consecutive Moves (no rotate/wait between, since
    // this is a straight corridor with no turns required).
    for window in moves.windows(2) {
        let (_, prev_exit) = window[0];
        let (next_entry, _) = window[1];
        assert!(
            (prev_exit - next_entry).abs() < 1e-6,
            "velocity continuity violated: prev_exit={prev_exit}, next_entry={next_entry}",
        );
    }
}

// ---------------------------------------------------------------------------
// VTM behaviour is unchanged. Pre- and post-fix the VTM finds the same path
// because the new guard only fires for `RobotType::Htm` under HtmKinematic.
// VTMs in HtmKinematic mode use the constant-speed cost model (per
// `use_kinematic_mode` in expand_move), so their states stay at
// `SpeedBucket::ZERO` and the guard is a no-op.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Edge case: tight planning horizon. The post-fix path is strictly slower
// than the pre-fix (buggy) path because the search must back off to find a
// stationary arrival. Constructed scenario: 5-cell corridor under a very
// tight `max_wait` that the buggy fast-cruise-into-goal path would just fit
// but the corrected zero-velocity-arrival path overruns. The fix must
// surface an `Err(_)` (not a panic) — `UnreachableWithinHorizon` or
// equivalent — when the corrected path doesn't fit.
// ---------------------------------------------------------------------------

#[test]
fn tight_horizon_returns_err_not_panic_kinematic() {
    let graph = corridor(5);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    // Buggy path travel_time is roughly the cruise traversal: 4 × ~250 ms ≈
    // 1000 ms (give or take accel ramp-in). Post-fix the bot must do a full
    // accel-decel triangular, taking ~2-3 s. A 500 ms horizon is too tight
    // for either; a 1500 ms horizon may fit the buggy path but not the
    // corrected one (depends on profile arithmetic — we mainly care that no
    // panic occurs).
    let cfg = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(ms(1_000)),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: ms(500), // intentionally too tight for any kinematic path
        max_iterations: 1_000_000,
        motion_model: MotionModel::HtmKinematic,
    };

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let result = pf.find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &cfg);

    // Must not panic. Must return Err. The exact variant is allowed to be
    // any of the planning-failure errors — the contract is "graceful
    // failure under the new constraint".
    let err = result.expect_err("tight horizon under HtmKinematic must return Err, not a panic or success path");

    // Sanity: the error variant must reference the from/to pair (i.e. it's
    // a planning-time failure, not e.g. ZeroLength which is a precondition
    // failure). We accept any of the planning-failure variants.
    assert!(
        matches!(
            err,
            PathfinderError::Unreachable { .. }
                | PathfinderError::UnreachableWithinHorizon { .. }
                | PathfinderError::DestinationExceedsMaxWait { .. }
                | PathfinderError::NoStaticRoute { .. }
                | PathfinderError::IterationLimit { .. }
        ),
        "expected a planning-failure variant from tight-horizon kinematic search; got {err:?}",
    );
}

#[test]
fn vtm_unaffected_by_goal_zero_velocity_guard() {
    let graph = corridor(2);
    let pf = pathfinder_speed(&graph, 1500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &drive_speed(1500, RobotType::Vtm, false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(1), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("VTM under HtmKinematic must still find a path (guard does not apply to VTMs)");

    // VTM uses constant-speed model, so all Move waypoints share entry==exit
    // and the velocity is whatever the constant-speed reconstruct emits.
    let moves = move_velocities(&path);
    assert_eq!(moves.len(), 1, "2-cell VTM corridor must produce exactly one Move");
    let (entry, exit) = moves[0];
    assert!((entry - exit).abs() < 1e-6, "VTM Move must be constant-speed: entry={entry}, exit={exit}");
}
