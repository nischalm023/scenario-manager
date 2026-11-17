//! Integration tests for `MotionModel::HtmKinematic` end-to-end through the
//! full SIPP-IP search.
//!
//! The unit tests in `kinematics.rs` cover `trapezoidal_cost` arithmetic in
//! isolation. This file validates that the kinematic motion model produces
//! sensible search behavior when wired through `find_path_reserved`:
//! - Constant-speed and kinematic modes produce different timings on the
//!   same fixture (kinematic strictly slower due to accel/decel ramps).
//! - Loaded vs unloaded HTM kinematic params produce different timings.
//! - VTMs ignore the kinematic toggle and stay on constant speed.
//! - The L1 heuristic remains admissible under the kinematic move costs.
//! - Search expansion count grows under kinematic mode (two move successors
//!   per neighbor — cruise vs decelerate-to-zero — vs one in constant-speed).
//! - Rotate-bracketing paths cost more than straight-line paths even with
//!   the same number of cells, because the rotate forces a decel-to-zero
//!   on the inbound move and an accel-from-zero on the outbound.
//!
//! Time assertions allow ~50 ms tolerance to absorb f64 sqrt rounding in the
//! trapezoidal profile math.

use super::prelude::*;
use crate::reservation::oracle::GhostOracle;

/// Build a `CollisionConfig` with the requested motion model and otherwise
/// boilerplate values lifted from the corridor tests.
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

/// `(actual - expected).abs() < ms(50)` — the trapezoidal profile uses f64
/// sqrt internally, so per-cell rounding can compound to ~10 ms over the
/// full corridor.
fn approx_eq(actual: SimDuration, expected: SimDuration, tolerance: SimDuration) -> bool {
    let diff = actual.abs_diff(expected);
    diff <= tolerance
}

#[test]
fn ghost_reservation_preserves_kinematic_physics_without_contention() {
    let graph = corridor(8);
    let htm =
        HtmKinematics { loaded_constant_speed_mm_s: 6000.0, empty_constant_speed_mm_s: 6000.0, ..reference_htm() };
    let pf = pathfinder_kinematics(&graph);
    let ghost = GhostOracle;
    assert_eq!(ghost.entry_intervals(c(1), t(3), t(30), None), ghost.entry_intervals(c(1), t(10), t(30), None));
    assert!(ghost.entry_intervals(c(1), t(31), t(30), None).is_empty());
    for carrying in [false, true] {
        let config = config(MotionModel::HtmKinematic);
        let drive = &Trapezoidal::htm(&htm, carrying);
        let ghost_path =
            pf.reserve_path_without_waiting(c(0), Orientation::East, c(7), drive, &ghost, &config).unwrap();
        let drive = &Trapezoidal::htm(&htm, carrying);
        let table_path = pf
            .reserve_path_without_waiting(c(0), Orientation::East, c(7), drive, &TableOracle::default(), &config)
            .unwrap();
        assert_eq!(ghost_path.path.waypoints, table_path.path.waypoints);
        let drive = &Constant::htm(&htm, carrying);
        let spatial = pf.find_path(c(0), Orientation::East, c(7), drive, t(0)).unwrap();
        assert!(ghost_path.path.travel_time > spatial.travel_time);
        // A second robot may occupy the same route at the same time in Ghost.
        let second_config = CollisionConfig { self_robot: r(2), ..config };
        let drive = &Trapezoidal::htm(&htm, carrying);
        let second =
            pf.reserve_path_without_waiting(c(0), Orientation::East, c(7), drive, &ghost, &second_config).unwrap();
        assert_eq!(second.path.waypoints, ghost_path.path.waypoints);
    }
}

// ---------------------------------------------------------------------------
// Test 1: Kinematic mode is slower than constant-speed on a corridor that
// requires accel/decel at the ends.
// ---------------------------------------------------------------------------

#[test]
fn kinematic_corridor_includes_accel_and_decel() {
    let graph = corridor(5);
    let pf = pathfinder_speed(&graph, 4500); // unloaded m5f-equivalent v_max
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (kinematic_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("kinematic corridor should find a path");
    let drive = &drive_speed(4500, RobotType::Htm, false);
    let (constant_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::ConstantSpeed))
        .expect("constant-speed corridor should find a path");

    // Constant speed: 4 hops × (1000 mm / 4500 mm/s) = 4 × 222 ms = 888 ms.
    assert!(
        approx_eq(constant_path.travel_time, ms(888), ms(5)),
        "constant-speed time = {:?}, expected ~888 ms",
        constant_path.travel_time,
    );

    // Kinematic must be strictly slower (accel from 0 and final decel to 0
    // burn extra time vs the cruise-everywhere constant model).
    assert!(
        kinematic_path.travel_time > constant_path.travel_time,
        "kinematic time {:?} should exceed constant-speed time {:?}",
        kinematic_path.travel_time,
        constant_path.travel_time,
    );

    // Both paths reach the same goal in the same number of moves (5 cells:
    // PathStart + 4 Move + PathEnd = 6 waypoints).
    assert_eq!(constant_path.waypoints.len(), kinematic_path.waypoints.len());
    assert_eq!(kinematic_path.waypoints.len(), 6);
}

// ---------------------------------------------------------------------------
// Test 2: Decelerate-to-rotate costs more than continuing straight, even
// with the same number of cells.
// ---------------------------------------------------------------------------

#[test]
fn rotate_in_middle_costs_more_than_straight() {
    // L-graph for the rotating path: c(0) → c(1) (East) → rotate → c(2) (South).
    //
    //   c(0) — c(1)
    //           |
    //          c(2)
    //
    // Straight graph: c(10) → c(11) → c(12), all East.
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 1, 1));
    b.add_edge_bidi(c(0), c(1), Edge::uniform(1000));
    b.add_edge_bidi(c(1), c(2), Edge::uniform(1000));

    b.add_cell(cell(10, CellKind::Open, 0, 5));
    b.add_cell(cell(11, CellKind::Open, 1, 5));
    b.add_cell(cell(12, CellKind::Open, 2, 5));
    b.add_edge_bidi(c(10), c(11), Edge::uniform(1000));
    b.add_edge_bidi(c(11), c(12), Edge::uniform(1000));

    let graph = b.build();
    let pf = pathfinder_speed(&graph, 1500); // m5f loaded v_max
    let oracle = TableOracle::new(ReservationTable::default());

    // L path: 2 moves + 1 rotate. HTM loaded for the conservative profile.
    let drive = &Trapezoidal::htm(&reference_htm(), true);
    let (l_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(2), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("L path with rotate should find a path");

    // Straight path: 2 moves, no rotate.
    let drive = &Trapezoidal::htm(&reference_htm(), true);
    let (straight_path, _) = pf
        .find_path_reserved(c(10), Orientation::East, c(12), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("straight path should find a path");

    // The L path forces a decelerate-to-zero on the move into c(1) (because
    // the next action is a rotate, which requires SpeedBucket::ZERO), then a
    // separate accel-from-zero out of c(1) into c(2). The straight path can
    // cruise from c(10) into c(11) and let velocity carry through to c(12).
    // The L path adds: rotate duration (1000 ms) + extra time spent at low
    // velocity vs the cruise.
    assert!(
        l_path.travel_time > straight_path.travel_time + ms(1_000),
        "L path travel_time {:?} should exceed straight path {:?} by at least the 1s rotate plus accel/decel overhead",
        l_path.travel_time,
        straight_path.travel_time,
    );

    // Sanity: the L path actually contains a rotate waypoint.
    let l_has_rotate = l_path.waypoints.iter().any(|w| matches!(w.action, WaypointAction::Rotate { .. }));
    assert!(l_has_rotate, "L path must include a Rotate waypoint");
}

// ---------------------------------------------------------------------------
// Test 3: Kinematic mode expands more states than constant-speed because of
// the cruise-vs-decel branching factor (proxy for SIPP-IP completeness).
// ---------------------------------------------------------------------------

#[test]
fn kinematic_mode_emits_two_move_variants_per_neighbor() {
    // SIPP-IP completeness claim: at each (cell, orient, interval) the
    // kinematic search keeps two arrivals — cruise (target_exit = v_max) and
    // decelerate-to-zero (target_exit = 0) — as distinct SearchKeys, so the
    // search can always satisfy a downstream zero-velocity requirement
    // (rotate, wait, goal-with-finite-dwell).
    //
    // We assert this structurally by counting Move trace events: kinematic
    // mode emits two Move events per (cell, neighbor) expanded (cruise +
    // decel), constant-speed emits one. Move events are emitted only when
    // a successor state actually pushes to the heap with a strictly better
    // g_score, so this also verifies the bucket-distinct-key dedup is
    // working.
    //
    // Speed is set to match kinematic v_max so the heuristic stays
    // admissible (h_per_step = 1000 mm / 4500 mm/s = 222 ms). The corridor
    // is 5 cells = 4 forward edges. In kinematic mode each expanded cruise
    // variant emits 2 moves to its forward neighbor; the decel variant
    // sits in the heap unpopped because its g is higher.
    let graph = corridor(5);
    let (pf, trace_kinematic) = (pathfinder_speed(&graph, 4500), TraceCollector::new());
    let pf_kinematic = pf.with_trace_hook(trace_kinematic.hook());
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (_, _) = pf_kinematic
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("kinematic corridor must succeed");

    let (pf2, trace_constant) = (pathfinder_speed(&graph, 4500), TraceCollector::new());
    let pf_constant = pf2.with_trace_hook(trace_constant.hook());
    let drive = &drive_speed(4500, RobotType::Htm, false);
    let (_, _) = pf_constant
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::ConstantSpeed))
        .expect("constant-speed corridor must succeed");

    let kinematic_move_events = trace_kinematic.events().iter().filter(|ev| matches!(ev, TraceEvent::Move(_))).count();
    let constant_move_events = trace_constant.events().iter().filter(|ev| matches!(ev, TraceEvent::Move(_))).count();

    // Constant-speed: 4 edges × 1 variant = 4 Move events.
    // Kinematic: 10 Move events. Pre-fix this was 8 (4 edges × cruise+decel).
    // The post-fix `try_accept_goal` rejects non-zero-velocity arrivals at
    // the goal under HtmKinematic, so the search must keep popping states
    // after the cruise-built-up state reaches c(4) at non-zero velocity,
    // re-expanding c(2)/c(3) at the v=0 (decel-route) speed bucket to find
    // a stationary arrival. The two extra Moves are the c(2)→c(3) cruise+
    // decel pair re-emitted from the v=0 entry bucket on c(2). The decel
    // variant from c(3)@v=0 into c(4) emits the same actual_exit_v as the
    // existing cruise variant from the higher-velocity c(3) bucket would
    // have, so dedup keeps total at 10 (not 12).
    assert_eq!(constant_move_events, 4, "constant-speed corridor should emit exactly one Move per edge");
    assert_eq!(
        kinematic_move_events, 10,
        "kinematic corridor with goal-zero-velocity guard should emit 10 Moves \
         (4 edges × 2 variants + 2 extra re-expansions through the v=0 bucket on c(2)/c(3))",
    );
    assert!(
        kinematic_move_events > constant_move_events,
        "kinematic Move events ({}) must exceed constant-speed ({}) — bucket variant property",
        kinematic_move_events,
        constant_move_events,
    );
}

// ---------------------------------------------------------------------------
// Test 4: Loaded HTM is strictly slower than unloaded HTM under kinematic
// mode.
// ---------------------------------------------------------------------------

#[test]
fn loaded_htm_matches_unloaded_under_kinematic_mode() {
    let graph = corridor(10);
    let pf = pathfinder_speed(&graph, 1500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), true);
    let (loaded_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(9), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("loaded HTM path");
    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (unloaded_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(9), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("unloaded HTM path");

    // Loaded chassis kinematics were field-calibrated to the empty values
    // (m5f loaded == unloaded: v_max=4500, accel=2000), so a carried tote no
    // longer inflates the trapezoidal traversal — loaded and unloaded kinematic
    // times now match. (Pre-calibration the sheet modeled loaded at 1500/1000,
    // making it strictly slower; see robots/quicktron_m5f.world.json.)
    assert_eq!(
        loaded_path.travel_time, unloaded_path.travel_time,
        "loaded HTM kinematic time {:?} should now equal unloaded {:?}",
        loaded_path.travel_time, unloaded_path.travel_time,
    );
}

// ---------------------------------------------------------------------------
// Test 5: VTMs ignore the kinematic toggle (always constant-speed).
// ---------------------------------------------------------------------------

#[test]
fn vtm_ignores_kinematic_toggle() {
    let graph = corridor(5);
    let pf = pathfinder_speed(&graph, 1500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &drive_speed(1500, RobotType::Vtm, false);
    let (vtm_kinematic_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("VTM in HtmKinematic mode must still find a path");
    let drive = &drive_speed(1500, RobotType::Vtm, false);
    let (vtm_constant_path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::ConstantSpeed))
        .expect("VTM in ConstantSpeed mode must find a path");

    // Per the use_kinematic_mode() gate in expand_move.rs, VTMs always use
    // the constant-speed cost model regardless of the global toggle. The
    // travel times must match exactly.
    assert_eq!(
        vtm_kinematic_path.travel_time, vtm_constant_path.travel_time,
        "VTM travel time must be invariant under MotionModel toggle (kinematic={:?}, constant={:?})",
        vtm_kinematic_path.travel_time, vtm_constant_path.travel_time,
    );

    // Constant speed: 4 hops × (1000 mm / 1500 mm/s) = 4 × 666 ms = 2664 ms.
    assert!(
        approx_eq(vtm_constant_path.travel_time, ms(2_664), ms(5)),
        "VTM constant-speed time = {:?}, expected ~2664 ms",
        vtm_constant_path.travel_time,
    );
}

// ---------------------------------------------------------------------------
// Test 6: L1 heuristic remains admissible under the kinematic cost model.
// ---------------------------------------------------------------------------

#[test]
fn heuristic_admissible_under_kinematic_mode() {
    let graph = corridor(6);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(5), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("kinematic path");

    // The heuristic uses cell_spacing_mm / speed_mm_per_sec per Manhattan
    // step (constant-speed estimate), which is the optimistic minimum cost.
    // Under kinematic mode the actual move time is always >= that, so:
    //
    //     g(goal) = travel_time >= h(start, goal)
    let h = pf.heuristic(c(0), c(5));
    assert!(
        path.travel_time >= h,
        "heuristic admissibility violated: travel_time = {:?}, heuristic = {:?}",
        path.travel_time,
        h,
    );
}

// ---------------------------------------------------------------------------
// Test 7: Existing corridor traversal still produces a valid path under
// kinematic mode (smoke test for the Phase 1 trace shape under the new
// motion model).
// ---------------------------------------------------------------------------

#[test]
fn corridor_traverse_kinematic_smoke() {
    let graph = corridor(5);
    let pf = pathfinder_speed(&graph, 4500);
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, stats) = pf
        .find_path_reserved(c(0), Orientation::East, c(4), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("kinematic corridor must terminate with a valid path");

    // Smoke checks that the search produced a sane plan (no infinite loop,
    // path actually reaches the goal, has the right shape).
    assert!(stats.iterations < 1_000_000, "search exceeded reasonable iteration budget");
    assert_eq!(path.waypoints.first().map(|w| w.cell), Some(c(0)));
    assert_eq!(path.waypoints.last().map(|w| w.cell), Some(c(4)));
    let move_count = path.waypoints.iter().filter(|w| matches!(w.action, WaypointAction::Move { .. })).count();
    assert_eq!(move_count, 4, "5-cell corridor should produce 4 Move waypoints");

    // Under HtmKinematic mode, reconstruction MUST surface the trapezoidal
    // profile's actual endpoint velocities — NOT substitute the configured
    // cruise speed (4500 mm/s here). Concretely: a 5-cell rest-to-rest
    // corridor cannot stay at v_max for the full traverse; at minimum the
    // first and last moves carry sub-cruise endpoints (start at 0, end
    // ramp-down). This is a smoke check; the *sharp* regression guard for
    // the value-based-vs-mode-based substitution defect is
    // `goal_cell_decel_preserves_zero_velocity_kinematic` below.
    let cruise = 4500.0_f64;
    let any_non_cruise = path.waypoints.iter().any(|w| match w.action {
        WaypointAction::Move { entry_velocity_mm_s, exit_velocity_mm_s, .. } => {
            (entry_velocity_mm_s - cruise).abs() > 1e-6 || (exit_velocity_mm_s - cruise).abs() > 1e-6
        }
        _ => false,
    });
    assert!(
        any_non_cruise,
        "HtmKinematic reconstruction must emit real profile endpoints, not the cruise-speed substitute"
    );
}

// ---------------------------------------------------------------------------
// Test 8: Sharp regression guard for the value-based substitution defect.
//
// A 2-cell rest-to-rest corridor produces a single Move (start → goal) whose
// NodeMeta carries `(entry_velocity, exit_velocity) == (Velocity::ZERO,
// Velocity::ZERO)`:
//   - entry: search.rs initialises the start state at Velocity::ZERO.
//   - exit:  expand_move.rs forces decel-only variants for moves into the
//     goal cell, so the trapezoidal triangular-peak profile lands exactly
//     on zero.
//
// Pre-fix `reconstruct.rs` used a value predicate (`if entry == 0 && exit ==
// 0 { substitute cruise }`) that fired here even though the search had
// computed a real (0, 0) profile, smearing the configured cruise speed back
// onto the waypoint. The post-fix mode-gated substitution emits the
// NodeMeta values verbatim under `MotionModel::HtmKinematic`, preserving the
// genuine zeros.
//
// Reverting the predicate gate in reconstruct.rs (without touching anything
// else) flips both the entry and exit assertions below — the 5-cell smoke
// test above keeps passing, but this one fails. That's the missing
// regression-coverage hole H1 called out.
// ---------------------------------------------------------------------------

#[test]
fn goal_cell_decel_preserves_zero_velocity_kinematic() {
    // 2-cell corridor: c(0) start, c(1) goal. Single accepted Move is the
    // goal-cell move with `force_decel = ZERO`.
    let graph = corridor(2);
    let pf = pathfinder_speed(&graph, 4500); // unloaded m5f-equivalent v_max
    let oracle = TableOracle::new(ReservationTable::default());

    let drive = &Trapezoidal::htm(&reference_htm(), false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(1), drive, &oracle, &config(MotionModel::HtmKinematic))
        .expect("kinematic 2-cell corridor must find a path");

    // Shape sanity: PathStart, single Move, PathEnd.
    let move_waypoints: Vec<_> = path
        .waypoints
        .iter()
        .filter_map(|w| match w.action {
            WaypointAction::Move { entry_velocity_mm_s, exit_velocity_mm_s, .. } => {
                Some((entry_velocity_mm_s, exit_velocity_mm_s))
            }
            _ => None,
        })
        .collect();
    assert_eq!(move_waypoints.len(), 1, "2-cell corridor must produce exactly one Move waypoint");
    let (entry, exit) = move_waypoints[0];

    // Both endpoints must be exactly zero — no cruise-speed substitution.
    // The pre-fix bug (value-based predicate) would emit (4500.0, 4500.0)
    // here. Use a tight bound; trapezoidal_cost returns Velocity::ZERO
    // exactly for forced-decel triangular-peak profiles, no f64 jitter.
    assert!(
        entry.abs() < 1e-9,
        "entry velocity must be 0 for goal-cell forced-decel move under HtmKinematic; got {entry}",
    );
    assert!(
        exit.abs() < 1e-9,
        "exit velocity must be 0 for goal-cell forced-decel move under HtmKinematic; got {exit}",
    );
}
