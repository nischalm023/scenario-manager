//! `MotionModel::ConstantSpeed` per-robot cruise speed.
//!
//! HTMs cruise at the Quicktron M5F chassis max for their load state even in
//! constant-speed mode — 4500 mm/s loaded AND unloaded (loaded was field-
//! calibrated to the empty value; see robots/quicktron_m5f.world.json) — sourced from the
//! kinematic params on the pathfinder, NOT the shared `speed_mm_per_sec` (which
//! the pathfinder is constructed with for VTMs, e.g. 6000 mm/s). VTMs keep the
//! shared configured speed.
//!
//! Per-1000mm-cell cost is `cell_length_mm * 1000 / speed` truncated to whole
//! ms, matching the `Constant` drive model's integer division:
//! - loaded HTM:   1_000_000 / 4500 = 222 ms  (field-calibrated to empty)
//! - unloaded HTM: 1_000_000 / 4500 = 222 ms
//! - VTM @ 6000:   1_000_000 / 6000 = 166 ms

use super::prelude::*;

use crate::{Path, Velocity};

/// `CollisionConfig` for a reservation-aware ConstantSpeed search on an empty
/// table — straight corridor, generous horizon, open terminal dwell.
fn constant_config() -> CollisionConfig {
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
        motion_model: MotionModel::ConstantSpeed,
    }
}

/// Sum the durations of all Move waypoints in a path.
fn total_move_ms(path: &Path) -> u64 {
    path.waypoints
        .iter()
        .filter_map(|wp| match wp.action {
            WaypointAction::Move { start, end, .. } => Some((end - start).as_ms()),
            _ => None,
        })
        .sum()
}

// The pathfinder is built at the VTM-style shared speed (6000 mm/s) to prove
// HTMs do NOT pick that up in constant mode — they resolve their own M5F speed.
const SHARED_VTM_SPEED_MM_S: u64 = 6000;

#[test]
fn unloaded_htm_cell_cost_is_m5f_empty_speed() {
    let graph = corridor(3);
    let pf = pathfinder_speed(&graph, SHARED_VTM_SPEED_MM_S);

    let drive = &drive_speed(SHARED_VTM_SPEED_MM_S, RobotType::Htm, false);
    let path = pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO).expect("unloaded htm path");

    // Two 1000mm cells at the unloaded M5F max (4500 mm/s) = 2 × 222 ms.
    assert_eq!(total_move_ms(&path), 2 * (1_000 * 1_000 / 4_500));
    // Waypoint velocity substitution reports the same M5F cruise speed.
    let first_move = path.waypoints.iter().find_map(|wp| match wp.action {
        WaypointAction::Move { entry_velocity_mm_s, exit_velocity_mm_s, .. } => {
            Some((entry_velocity_mm_s, exit_velocity_mm_s))
        }
        _ => None,
    });
    assert_eq!(first_move, Some((4_500.0, 4_500.0)));
}

#[test]
fn loaded_htm_cell_cost_is_m5f_loaded_speed() {
    let graph = corridor(3);
    let pf = pathfinder_speed(&graph, SHARED_VTM_SPEED_MM_S);

    let drive = &drive_speed(SHARED_VTM_SPEED_MM_S, RobotType::Htm, true);
    let path = pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO).expect("loaded htm path");

    // Two 1000mm cells at the loaded M5F max (4500 mm/s, field-calibrated to
    // the empty value) = 2 × 222 ms.
    assert_eq!(total_move_ms(&path), 2 * (1_000 * 1_000 / 4_500));
}

#[test]
fn vtm_cell_cost_is_shared_configured_speed() {
    let graph = corridor(3);
    let pf = pathfinder_speed(&graph, SHARED_VTM_SPEED_MM_S);

    let drive = &drive_speed(SHARED_VTM_SPEED_MM_S, RobotType::Vtm, false);
    let path = pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO).expect("vtm path");

    // VTMs keep the shared constructor speed: 2 × (1_000_000 / 6000) ms.
    assert_eq!(total_move_ms(&path), 2 * (1_000 * 1_000 / SHARED_VTM_SPEED_MM_S));
}

#[test]
fn reservation_aware_unloaded_htm_matches_m5f_empty_speed() {
    // Exercises the expand_move + search reconstruct paths (not just the
    // spatial-only find_path), proving the constant-mode cost resolution is
    // consistent across both search entry points.
    let graph = corridor(3);
    let pf = pathfinder_speed(&graph, SHARED_VTM_SPEED_MM_S);
    let oracle = TableOracle::new(ReservationTable::default());
    let config = constant_config();

    let drive = &drive_speed(SHARED_VTM_SPEED_MM_S, RobotType::Htm, false);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::East, c(2), drive, &oracle, &config)
        .expect("reservation-aware unloaded htm path");

    assert_eq!(total_move_ms(&path), 2 * (1_000 * 1_000 / 4_500));
}

#[test]
fn loaded_htm_constant_cost_uses_constant_speed_not_v_max() {
    // ConstantSpeed mode charges the configured per-load CONSTANT speed, which
    // is separate from the kinematic v_max. Set them to distinct values and
    // confirm constant-mode cost follows the constant speed (2500 mm/s →
    // 1_000_000 / 2500 = 400 ms/cell), NOT the v_max (1500 → would be 666).
    let graph = corridor(3);
    let mut htm = reference_htm();
    htm.loaded_constant_speed_mm_s = 2_500.0; // constant-mode cruise
    htm.loaded.max_velocity_mm_s = 1_500.0; // kinematic v_max — must be ignored here
    let pf = pathfinder_kinematics(&graph);

    let drive = &Constant::htm(&htm, true);
    let path =
        pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO).expect("loaded htm constant-mode path");

    assert_eq!(total_move_ms(&path), 2 * (1_000 * 1_000 / 2_500));
}

#[test]
fn constant_quotes_reprice_distance_and_turns_with_the_execution_drive() {
    let graph = corridor(3);
    let pf = pathfinder_kinematics(&graph);
    for carrying in [false, true] {
        let mut figures = reference_htm();
        figures.empty_constant_speed_mm_s = 6_000.0;
        figures.loaded_constant_speed_mm_s = 3_251.9;
        figures.loaded_turn = ms(1741);
        figures.loaded_reversal = Some(ms(3123));
        let execution = Constant::htm(&figures, carrying);
        let estimate = Constant {
            speed: Velocity(if carrying { 4_200.0 } else { 4_500.0 }),
            turn_duration: ms(123),
            reversal_duration: Some(ms(200)),
            ..execution
        };
        for orientation in [Orientation::East, Orientation::North, Orientation::West] {
            let spatial = pf.find_path(c(0), orientation, c(2), &estimate, t(0)).unwrap();
            let quoted = pf.kinematic_travel_time(&spatial, &execution);
            let oracle = TableOracle::new(ReservationTable::default());
            let (reserved, _) =
                pf.find_path_reserved(c(0), orientation, c(2), &execution, &oracle, &constant_config()).unwrap();
            let turn = match orientation {
                Orientation::East => SimDuration::ZERO,
                Orientation::North => execution.rotation_cost(90_000),
                _ => execution.reversal_duration_for().unwrap_or_else(|| execution.rotation_cost(180_000)),
            };
            let moves = 2 * (1_000 * 1_000 / execution.max_speed().mm_per_sec() as u64);
            assert_eq!(quoted, ms(moves) + turn);
            assert_eq!(quoted, reserved.travel_time, "quotes use the reserved drive's per-edge rounding and turns");
            assert_ne!(quoted, spatial.travel_time, "the search estimate is not the execution quote");
        }
    }
}
