//! Phase 4 of kinematic-playback-viz: rasterization tests for
//! `nav_handler`'s HTM `Move` → trapezoidal-curve micro-`BotStep`s.
//!
//! Each test runs the observer with a `MetricsConfig` configured for
//! `MotionModel::HtmKinematic`, feeds a single Nav event with one Move
//! waypoint covering one cell, and verifies the emitted `BotStep` chain.
//!
//! The constant-speed control test confirms the rasterization gate is
//! mode-based (not velocity-based): with `motion_model == ConstantSpeed`,
//! one Move waypoint produces exactly one BotStep regardless of the
//! waypoint's velocity values.

use std::sync::Arc;
use std::sync::mpsc;

use desgraph::{CellId, CircularArc, RobotType, WorldPos};
use rustc_hash::FxHashMap;

use super::test_models::legacy_models;
use crate::config::SimConfig;
use crate::events::*;
use crate::model::{RobotId, SimDuration, SimTime};
use crate::observer::output::MetricsOutput;
use crate::observer::time_slice::{BotLocation, BotStepKind, MetricsConfig, TimeSliceData};
use crate::pathfinding::{MotionModel, Orientation, Velocity, Waypoint, WaypointAction, trapezoidal_profile};

use super::SimObserver;
use crate::robots::reference_htm;
use crate::robots::runtime::{FloorRobotModel, RobotModelId};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `MetricsConfig` for kinematic playback with sane defaults for
/// rasterization tests. `cell_spacing_mm = 1000` (1 m cell), 100 ms sample
/// period, 60 s slice width.
fn kinematic_config(motion_model: MotionModel, cell_positions: Vec<WorldPos>) -> MetricsConfig {
    let reference = reference_htm();
    let mut settings = SimConfig::default();
    settings.motion_model = motion_model;
    settings.htm.loaded_speed = reference.loaded.max_velocity_mm_s / 1000.0;
    settings.htm.empty_speed = reference.unloaded.max_velocity_mm_s / 1000.0;
    settings.htm.loaded_accel = reference.loaded.acceleration_mm_s2 / 1000.0;
    settings.htm.empty_accel = reference.unloaded.acceleration_mm_s2 / 1000.0;
    MetricsConfig {
        slice_width_ms: 60_000,
        speed_mm_per_sec: 1500.0,
        cell_spacing_mm: 1000.0,
        htm_models: legacy_models(&settings, [RobotId(0)]),
        motion_step_ms: 100,
        cell_positions: Arc::new(cell_positions),
        ..MetricsConfig::default()
    }
}

/// Run observer over a list of events, returning the final output.
fn run_with_events(events: Vec<SimEvent>, config: MetricsConfig) -> MetricsOutput {
    let (tx, rx) = mpsc::channel();
    for ev in events {
        tx.send(ev).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run().expect("the facts break no rule of the event stream")
}

#[test]
fn one_observer_preserves_distinct_robot_models_and_load_profiles() {
    let mut settings = SimConfig::default();
    settings.motion_model = MotionModel::HtmKinematic;
    settings.htm.empty_speed = 2.0;
    settings.htm.empty_accel = 1.0;
    settings.htm.loaded_speed = 1.0;
    settings.htm.loaded_accel = 0.25;
    let mut model = FloorRobotModel::legacy(&settings);
    model.id = RobotModelId(1);
    let model = Arc::new(model);

    let mut config = kinematic_config(MotionModel::ConstantSpeed, axis_cells_3());
    let models = Arc::make_mut(&mut config.htm_models);
    models.insert(RobotId(1), model.clone());
    models.insert(RobotId(2), model);

    let mut events = vec![SimEvent::InitialPositions(InitialPositionsEvent {
        positions: (0..3).map(|id| ip(id, 0, RobotType::Htm, WorldPos::default())).collect(),
        htm_headings_rad: Vec::new(),
    })];
    // Rest-to-rest over 1 m: the empty trapezoid takes 2 s, the loaded one
    // 4 s. The constant robot's committed 1 s move remains a single segment.
    for (robot, duration, loaded) in [(0, 1, false), (1, 2, false), (2, 4, true)] {
        events.push(SimEvent::Nav(NavEventData {
            robot_id: RobotId(robot),
            robot_type: RobotType::Htm,
            time: SimTime::ZERO,
            distance_mm: 1000,
            carrying_cargo: loaded,
            waypoints: move_chain_one_cell(SimTime::ZERO, SimDuration::seconds(duration), 0.0, 0.0),
        }));
    }
    let output = run_with_events(events, config);
    let locations = &output.time_slices[0].bot_locations;
    let moves =
        |id| locations[&id].steps.iter().filter(|step| matches!(step.kind, BotStepKind::Move)).collect::<Vec<_>>();
    assert_eq!(moves(0).len(), 1);
    let empty = moves(1);
    let loaded = moves(2);
    assert_eq!(empty.len(), 20);
    assert_eq!(loaded.len(), 40);
    assert!((empty[4].pos.x_mm - 125.0).abs() < 0.001);
    assert!((loaded[4].pos.x_mm - 31.25).abs() < 0.001);
    assert_eq!(empty.last().unwrap().pos.x_mm, 1000.0);
    assert_eq!(loaded.last().unwrap().pos.x_mm, 1000.0);
}

#[test]
#[should_panic(expected = "observer has no motion model for HTM")]
fn navigation_refuses_an_unbound_htm_instead_of_using_default_motion() {
    run_with_events(
        vec![SimEvent::Nav(NavEventData {
            robot_id: RobotId(91),
            robot_type: RobotType::Htm,
            time: SimTime::ZERO,
            distance_mm: 1000,
            carrying_cargo: false,
            waypoints: move_chain_one_cell(SimTime::ZERO, SimDuration::seconds(1), 0.0, 0.0),
        })],
        MetricsConfig::default(),
    );
}

/// Three cells on a horizontal axis: `c0 = (0, 0)`, `c1 = (1000, 0)`,
/// `c2 = (2000, 0)`. Cell IDs are array indices.
fn axis_cells_3() -> Vec<WorldPos> {
    vec![WorldPos { x_mm: 0.0, y_mm: 0.0 }, WorldPos { x_mm: 1000.0, y_mm: 0.0 }, WorldPos { x_mm: 2000.0, y_mm: 0.0 }]
}

/// Build a Waypoint chain `PathStart(c0) → Move(c0→c1, dur, entry, exit) →
/// PathEnd(c1)` for an HTM Move test fixture.
fn move_chain_one_cell(t0: SimTime, dur: SimDuration, entry_v: f64, exit_v: f64) -> Vec<Waypoint> {
    let c0 = CellId(0);
    let c1 = CellId(1);
    vec![
        Waypoint { cell: c0, orientation: Orientation::East, action: WaypointAction::PathStart { time: t0 } },
        Waypoint {
            cell: c1,
            orientation: Orientation::East,
            action: WaypointAction::Move {
                start: t0,
                end: t0 + dur,
                entry_velocity_mm_s: entry_v,
                exit_velocity_mm_s: exit_v,
            },
        },
        Waypoint { cell: c1, orientation: Orientation::East, action: WaypointAction::PathEnd { time: t0 + dur } },
    ]
}

/// Initial-positions tuple builder.
fn ip(robot_id: u32, cell: u32, robot_type: RobotType, pos: WorldPos) -> (RobotId, CellId, RobotType, WorldPos) {
    (RobotId(robot_id), CellId(cell), robot_type, pos)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Kinematic Move from rest (entry=0) to clamped exit. The peak is reached
/// during the cell so the position curve is quadratic in the accel phase.
/// We assert (a) we got >= 2 micro-BotSteps, (b) the early micro-step is
/// well below the linear-interpolation expectation (because position grows
/// as 0.5 * a * t² in the accel phase), and (c) the last micro-step ends at
/// the destination cell center.
#[test]
fn kinematic_accel_from_rest_emits_micro_steps_along_accel_curve() {
    // 1 m cell, the reference M5F's unloaded params (a = 2000 mm/s²): pure accel reaches
    // sqrt(2 * 2000 * 1000) = ~2000 mm/s by exit. Total time ~1 s.
    let dur = SimDuration::ms(1000);
    let nav = NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        waypoints: move_chain_one_cell(SimTime::ZERO, dur, 0.0, 2000.0),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: false, // → the reference M5F's unloaded params
    };
    let config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, WorldPos { x_mm: 0.0, y_mm: 0.0 })],
                htm_headings_rad: vec![(RobotId(0), 0.7)],
            }),
            SimEvent::Nav(nav),
        ],
        config,
    );

    let loc = output.time_slices[0].bot_locations.get(&0).expect("bot locations present");
    let move_steps: Vec<_> = loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    assert!(move_steps.len() >= 2, "rasterization gate must fire (>=2 micro-steps), got {}", move_steps.len());

    // Algo-expert M1: assert exact micro-step count. The rasterizer's loop
    // predicate is `t_ms < move_duration_ms` with `t_ms += step_ms`, which
    // yields `ceil(move_duration_ms / step_ms)` iterations. For dur=1000ms
    // at motion_step_ms=100ms this is exactly 10. (`move_duration_ms` is a
    // clean multiple of `step_ms`, so the formula reduces to plain division.)
    let move_duration_ms: u64 = 1000;
    let step_ms: u64 = 100;
    let expected_count = move_duration_ms.div_ceil(step_ms) as usize;
    assert_eq!(
        move_steps.len(),
        expected_count,
        "micro-step count must equal ceil(move_duration_ms / motion_step_ms) = {}",
        expected_count,
    );

    // Last micro-step lands at the destination cell center.
    let last = move_steps.last().unwrap();
    assert_eq!(last.cell_id, 1);
    assert!((last.pos.x_mm - 1000.0).abs() < 1e-6, "last pos.x = {}, expected 1000", last.pos.x_mm);

    // The micro-step ending at t = 100ms should be well below the linear
    // expectation. Linear: pos at t=100ms (10% of 1s) would be 100 mm.
    // Trapezoid (pure-accel-clamp): pos = 0.5 * 2000 * (0.1)² = 10 mm.
    // Use a very loose assertion: the early position must be far less than
    // half the linear-interpolation midpoint.
    let first = move_steps.first().unwrap();
    assert!(
        first.pos.x_mm < 50.0,
        "first micro-step (t≈100ms) pos.x = {} too far for accel-from-rest curve (expected ~10mm)",
        first.pos.x_mm,
    );

    // Algo-expert M2: within-Move position monotonicity. This is a single
    // Move on a single slice (60 s width >> 1 s move), so every micro-step
    // belongs to the same un-split Move. Position along the source→dest
    // axis must be non-decreasing — guards against a regression in
    // `sample_world_pos` (e.g. swapping source/dest or sign-flipping the
    // projection fraction).
    let mut prev_x = 0.0_f64; // source_pos.x_mm == 0
    for step in &move_steps {
        assert!(
            step.pos.x_mm + 1e-9 >= prev_x,
            "non-monotonic position within Move: prev={}, current={}",
            prev_x,
            step.pos.x_mm,
        );
        prev_x = step.pos.x_mm;
    }
}

/// Kinematic Move braking to rest (entry=v_max-clamped, exit=0). Position
/// curve is concave (decel), so positions grow faster early than linear.
/// Mirror of the accel-from-rest test.
#[test]
fn kinematic_decel_to_rest_emits_micro_steps_along_decel_curve() {
    // 1 m cell, the reference M5F unloaded (a = dec = 2000): "decel-3a" branch — entry =
    // 2000 mm/s, target = 0, distance 1000 mm. Brake distance d_d =
    // (2000²) / (2 * 2000) = 1000 — exactly fits. Decel time ~1 s.
    let dur = SimDuration::ms(1000);
    let nav = NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        waypoints: move_chain_one_cell(SimTime::ZERO, dur, 2000.0, 0.0),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: false,
    };
    let config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, WorldPos { x_mm: 0.0, y_mm: 0.0 })],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::Nav(nav),
        ],
        config,
    );

    let loc = output.time_slices[0].bot_locations.get(&0).expect("bot locations present");
    let move_steps: Vec<_> = loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    assert!(move_steps.len() >= 2, "rasterization gate must fire (>=2 micro-steps)");

    let last = move_steps.last().unwrap();
    assert!((last.pos.x_mm - 1000.0).abs() < 1e-6, "last pos.x = {}", last.pos.x_mm);

    // First micro-step (t ≈ 100ms) under pure decel from 2000 mm/s with
    // dec = 2000 mm/s²: pos = v*t - 0.5*a*t² = 200 - 10 = 190 mm. Linear
    // would be 100 mm. So the early position is well above linear.
    let first = move_steps.first().unwrap();
    assert!(
        first.pos.x_mm > 150.0,
        "first micro-step (t≈100ms) pos.x = {} too low for decel-to-rest curve (expected ~190mm)",
        first.pos.x_mm,
    );
}

/// Algo-expert M1: rest-to-rest with peak in the middle is the canonical
/// rasterization case. `entry == exit == 0` MUST still trigger
/// rasterization (the discriminator is mode + robot type + distance > 0,
/// NOT velocity equality). This is the spec's motivating decel-into-goal /
/// accel-from-rotate scenario.
#[test]
fn kinematic_rest_to_rest_with_peak_rasterizes() {
    // 1 m cell, the reference M5F unloaded: triangular peak — v_p = sqrt(a*d) =
    // sqrt(2000*1000) ≈ 1414 mm/s, total time = 2 * v_p / a ≈ 1.414 s.
    // Use the full canonical duration so trapezoidal_profile produces the
    // same shape internally.
    let dur = SimDuration::ms(1414);
    let nav = NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        waypoints: move_chain_one_cell(SimTime::ZERO, dur, 0.0, 0.0),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: false,
    };
    let config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, WorldPos { x_mm: 0.0, y_mm: 0.0 })],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::Nav(nav),
        ],
        config,
    );

    let loc = output.time_slices[0].bot_locations.get(&0).expect("bot locations present");
    let move_steps: Vec<_> = loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    assert!(
        move_steps.len() >= 2,
        "rest-to-rest with peak MUST rasterize (M1 algo-expert gate); got {} steps",
        move_steps.len(),
    );

    // Final micro-step lands at the destination cell center.
    let last = move_steps.last().unwrap();
    assert!((last.pos.x_mm - 1000.0).abs() < 1e-3, "last pos.x = {}", last.pos.x_mm);

    // Symmetric profile (a == dec): the curve has the symmetry
    // pos(t) + pos(total - t) == total_distance. Sample two micro-steps
    // ending at the same offset from each end and verify the relation.
    // (Asserting position-at-midpoint exactly is brittle under 100 ms
    // rasterization granularity — the nearest sample lands ~90 ms past
    // true midpoint for a 1414 ms move, giving a ~120 mm offset.)
    let total_ms: u64 = move_steps.iter().map(|s| s.delta_ms).sum();
    let mut acc = 0u64;
    let mut early_pos: Option<f64> = None;
    let mut late_pos: Option<f64> = None;
    let probe_offset_ms = 200u64;
    for s in &move_steps {
        let end = acc + s.delta_ms;
        if early_pos.is_none() && end >= probe_offset_ms {
            early_pos = Some(s.pos.x_mm);
        }
        if late_pos.is_none() && end >= total_ms.saturating_sub(probe_offset_ms) {
            late_pos = Some(s.pos.x_mm);
        }
        acc = end;
    }
    let early = early_pos.expect("early probe found");
    let late = late_pos.expect("late probe found");
    // Symmetric profile: pos(t) + pos(total - t) == total_distance.
    // Allow ~80 mm slack for the 100 ms-step bracketing on either side.
    assert!(
        (early + late - 1000.0).abs() < 80.0,
        "symmetric trapezoidal: early({:.1}) + late({:.1}) = {:.1}, expected ≈ 1000",
        early,
        late,
        early + late,
    );
}

/// Constant-speed mode: the rasterization gate is OFF regardless of the
/// waypoint's velocity values. One Move waypoint = one BotStep.
#[test]
fn constant_speed_mode_emits_one_botstep_per_move() {
    let dur = SimDuration::ms(700);
    let nav = NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        waypoints: move_chain_one_cell(SimTime::ZERO, dur, 0.0, 0.0),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: false,
    };
    let config = kinematic_config(MotionModel::ConstantSpeed, axis_cells_3());
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, WorldPos { x_mm: 0.0, y_mm: 0.0 })],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::Nav(nav),
        ],
        config,
    );

    let loc = output.time_slices[0].bot_locations.get(&0).expect("bot locations present");
    let move_steps: Vec<_> = loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    assert_eq!(
        move_steps.len(),
        1,
        "ConstantSpeed mode must emit exactly one BotStep per Move (gate is mode-based, not velocity-based)",
    );
    assert!((move_steps[0].pos.x_mm - 1000.0).abs() < 1e-6);
    assert_eq!(move_steps[0].delta_ms, 700);
}

/// VTM Moves are NEVER rasterized even under HtmKinematic mode (the gate
/// requires `robot_type == Htm`). One Move waypoint = one BotStep.
#[test]
fn vtm_move_not_rasterized_under_kinematic_mode() {
    let dur = SimDuration::ms(800);
    let nav = NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Vtm,
        waypoints: move_chain_one_cell(SimTime::ZERO, dur, 0.0, 0.0),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: false,
    };
    let config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Vtm, WorldPos { x_mm: 0.0, y_mm: 0.0 })],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::Nav(nav),
        ],
        config,
    );

    let loc = output.time_slices[0].bot_locations.get(&0).expect("vtm bot loc");
    let move_steps: Vec<_> = loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    assert_eq!(move_steps.len(), 1, "VTM moves must not rasterize (HTM-only gate)");
}

/// Slice-boundary behavior. A 1.5 s kinematic Move with `slice_width_ms =
/// 1000` straddles the boundary at t = 1000 ms. The rasterizer emits
/// micro-intervals every 100 ms with canonical end positions; the
/// slice-splitting layer redistributes them into the two slices.
///
/// We assert (a) both slices contain Move BotSteps for the bot, (b) the
/// micro-step counts roughly match the slice durations, and (c) positions
/// are monotonically non-decreasing across the boundary — i.e. the
/// trapezoidal sampling stays consistent across the split.
#[test]
fn kinematic_move_splits_at_slice_boundary() {
    let dur = SimDuration::ms(1500);
    let nav = NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        waypoints: move_chain_one_cell(SimTime::ZERO, dur, 0.0, 0.0),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: false,
    };
    // Tight slice width: 1 s. Cell spans 0..1500 ms — boundary at 1000 ms.
    let mut config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
    config.slice_width_ms = 1000;

    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, WorldPos { x_mm: 0.0, y_mm: 0.0 })],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::Nav(nav),
        ],
        config,
    );

    assert!(output.time_slices.len() >= 2, "expected >= 2 slices, got {}", output.time_slices.len());
    let slice0_loc = output.time_slices[0].bot_locations.get(&0).expect("slice 0 has bot");
    let slice1_loc = output.time_slices[1].bot_locations.get(&0).expect("slice 1 has bot");

    // Both slices contain Move steps.
    let s0_moves: Vec<_> = slice0_loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    let s1_moves: Vec<_> = slice1_loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).collect();
    assert!(!s0_moves.is_empty(), "slice 0 should contain Move steps");
    assert!(!s1_moves.is_empty(), "slice 1 should contain Move steps");

    // Position monotonicity across the boundary: the last step of slice 0
    // and the steps of slice 1 should produce non-decreasing positions
    // (allow tiny float jitter from the linearization-within-micro-step
    // when a 100 ms micro-interval is split mid-step).
    let mut last_x = 0.0_f64;
    for s in slice0_loc.steps.iter().chain(slice1_loc.steps.iter()) {
        if matches!(s.kind, BotStepKind::Move) {
            assert!(
                s.pos.x_mm + 1e-3 >= last_x,
                "position regressed across slice boundary: {} < {}",
                s.pos.x_mm,
                last_x,
            );
            last_x = s.pos.x_mm;
        }
    }
    // Final position must reach the destination cell center.
    assert!((last_x - 1000.0).abs() < 1e-3, "final pos.x = {}, expected 1000", last_x);
}

/// Loaded vs unloaded param dispatch. Two identical Moves except for
/// `carrying_cargo`: the reference M5F's loaded params against its unloaded
/// ones, which were v_max=1500, a=1000 against v_max=4500, a=2000. For a 1 s rest-to-rest move on a 1 m cell, the
/// loaded profile is "decel-3a-style" partial brake (slower) while the
/// unloaded profile is pure accel (faster ramp). The early-position
/// difference between the two BotStep chains exercises the cargo branch.
#[test]
fn carrying_cargo_selects_loaded_kinematic_params() {
    let dur = SimDuration::ms(1000);
    let waypoints = move_chain_one_cell(SimTime::ZERO, dur, 0.0, 0.0);

    let make_nav = |carrying: bool| NavEventData {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        waypoints: waypoints.clone(),
        time: SimTime::ZERO,
        distance_mm: 1000,
        carrying_cargo: carrying,
    };

    let config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
    let cells = axis_cells_3();
    let init = SimEvent::InitialPositions(InitialPositionsEvent {
        positions: vec![ip(0, 0, RobotType::Htm, cells[0])],
        htm_headings_rad: Vec::new(),
    });

    let loaded_out = run_with_events(vec![init.clone(), SimEvent::Nav(make_nav(true))], config.clone());
    let unloaded_out = run_with_events(vec![init, SimEvent::Nav(make_nav(false))], config);

    let loaded_loc = loaded_out.time_slices[0].bot_locations.get(&0).expect("loaded loc");
    let unloaded_loc = unloaded_out.time_slices[0].bot_locations.get(&0).expect("unloaded loc");

    // Both should rasterize.
    assert!(loaded_loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).count() >= 2);
    assert!(unloaded_loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).count() >= 2);

    // Take the position at t≈500ms (the 5th micro-step) under both profiles.
    // Loaded chassis kinematics were field-calibrated to the empty values, so
    // cargo dispatch now selects params identical to unloaded — the rasterized
    // midpoints match. (Pre-calibration loaded a=1000 vs unloaded a=2000 gave
    // distinct midpoints; see robots/quicktron_m5f.world.json.)
    let pos_at_idx = |loc: &BotLocation, idx: usize| -> f64 {
        loc.steps.iter().filter(|s| matches!(s.kind, BotStepKind::Move)).nth(idx).map(|s| s.pos.x_mm).unwrap_or(-1.0)
    };
    let loaded_mid = pos_at_idx(loaded_loc, 4);
    let unloaded_mid = pos_at_idx(unloaded_loc, 4);
    assert!(
        (loaded_mid - unloaded_mid).abs() < 1.0,
        "loaded ({}) vs unloaded ({}) midpoints should now match (loaded field-calibrated to empty)",
        loaded_mid,
        unloaded_mid,
    );
}

#[test]
fn declared_curve_playback_follows_the_arc_under_both_motion_models() {
    for motion_model in [MotionModel::ConstantSpeed, MotionModel::HtmKinematic] {
        let source = WorldPos { x_mm: -600.0, y_mm: 0.0 };
        let destination = WorldPos { x_mm: 600.0, y_mm: 0.0 };
        let arc = CircularArc { center: WorldPos::default(), clockwise: false, start: None, end: None };
        let distance = arc.length_mm(source, destination).round() as u64;
        let (duration, entry_velocity, exit_velocity) = if motion_model == MotionModel::HtmKinematic {
            let profile =
                trapezoidal_profile(Velocity(0.0), Velocity(0.0), distance, &reference_htm().unloaded).unwrap();
            (SimDuration::ms((profile.total_time_s * 1000.0).ceil() as u64), 0.0, 0.0)
        } else {
            (SimDuration::ms(distance), 1000.0, 1000.0)
        };
        let mut config = kinematic_config(motion_model, vec![source, destination]);
        config.link_curves = Arc::new([((CellId(0), CellId(1)), arc)].into_iter().collect());
        let mut waypoints = move_chain_one_cell(SimTime::ZERO, duration, entry_velocity, exit_velocity);
        waypoints[0].orientation = Orientation::North;
        waypoints[1].orientation = Orientation::South;
        waypoints[2].orientation = Orientation::South;
        let output = run_with_events(
            vec![
                SimEvent::InitialPositions(InitialPositionsEvent {
                    positions: vec![ip(0, 0, RobotType::Htm, source)],
                    htm_headings_rad: Vec::new(),
                }),
                SimEvent::Nav(NavEventData {
                    robot_id: RobotId(0),
                    robot_type: RobotType::Htm,
                    waypoints,
                    time: SimTime::ZERO,
                    distance_mm: distance,
                    carrying_cargo: false,
                }),
            ],
            config,
        );
        let location = output.time_slices[0].bot_locations.get(&0).unwrap();
        let moves: Vec<_> = location.steps.iter().filter(|step| step.kind == BotStepKind::Move).collect();
        assert!(moves.len() >= 36, "tight curves need sufficiently fine geometric sampling");
        assert_eq!(moves.iter().map(|step| step.delta_ms).sum::<u64>(), duration.as_ms());
        assert!(moves.iter().all(|step| (step.pos.x_mm.hypot(step.pos.y_mm) - 600.0).abs() < 1e-6));
        assert!(moves.iter().any(|step| step.pos.y_mm < -599.0), "the centreline reaches the bottom of the U-turn");
        assert_eq!(moves.last().unwrap().pos, destination);
        assert!(location.steps.iter().all(|step| step.kind != BotStepKind::Rotate));
        let mut traffic = FxHashMap::default();
        for span in location.motion.as_ref().unwrap() {
            span.accumulate(0, duration.as_ms(), &mut traffic);
        }
        assert!((traffic.values().map(|row| row.distance_mm).sum::<f64>() - distance as f64).abs() < 1e-6);
        assert!((traffic.values().map(|row| row.moving_ms).sum::<f64>() - duration.as_ms() as f64).abs() < 1e-6);
    }
}

#[test]
fn traffic_is_independent_of_playback_sampling_and_profile_slice_boundaries() {
    let mut totals = Vec::new();
    for (sample_ms, slice_ms) in [(25, 157), (333, 60_000)] {
        let mut config = kinematic_config(MotionModel::HtmKinematic, axis_cells_3());
        config.motion_step_ms = sample_ms;
        config.slice_width_ms = slice_ms;
        let output = run_with_events(
            vec![
                SimEvent::InitialPositions(InitialPositionsEvent {
                    positions: vec![ip(0, 0, RobotType::Htm, WorldPos::default())],
                    htm_headings_rad: Vec::new(),
                }),
                SimEvent::Nav(NavEventData {
                    robot_id: RobotId(0),
                    robot_type: RobotType::Htm,
                    waypoints: move_chain_one_cell(SimTime::ZERO, SimDuration::ms(1000), 0.0, 2000.0),
                    time: SimTime::ZERO,
                    distance_mm: 1000,
                    carrying_cargo: false,
                }),
            ],
            config,
        );
        // Exercise the same named MessagePack representation used by profiles.
        let encoded = rmp_serde::to_vec_named(&output.time_slices).unwrap();
        let slices: Vec<TimeSliceData> = rmp_serde::from_slice(&encoded).unwrap();
        let mut traffic = FxHashMap::default();
        for slice in slices {
            for loc in slice.bot_locations.values() {
                for span in loc.motion.as_ref().unwrap() {
                    span.accumulate(slice.start_ms, (slice.start_ms + slice_ms).min(750), &mut traffic);
                }
            }
        }
        assert!((traffic.values().map(|row| row.distance_mm).sum::<f64>() - 562.5).abs() < 1e-6);
        assert!((traffic.values().map(|row| row.moving_ms).sum::<f64>() - 750.0).abs() < 1e-6);
        totals.push(traffic);
    }
    for cell in [0, 1] {
        for (a, b) in totals[0][&cell].speed_ms.iter().zip(totals[1][&cell].speed_ms) {
            assert!((a - b).abs() < 1e-6);
        }
    }
}

#[test]
fn reverse_link_is_journaled_as_translation_without_rotation() {
    let positions = axis_cells_3();
    let mut config = kinematic_config(MotionModel::ConstantSpeed, positions.clone());
    config.slice_width_ms = 500;
    config.htm_reverse_links = Arc::new([(CellId(0), CellId(1))].into_iter().collect());
    let waypoints = move_chain_one_cell(SimTime::ZERO, SimDuration::ms(1000), 1000.0, 1000.0);
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, positions[0])],
                htm_headings_rad: vec![(RobotId(0), 0.7)],
            }),
            SimEvent::Nav(NavEventData {
                robot_id: RobotId(0),
                robot_type: RobotType::Htm,
                waypoints,
                time: SimTime::ZERO,
                distance_mm: 1000,
                carrying_cargo: true,
            }),
        ],
        config,
    );
    assert_eq!(output.time_slices.len(), 2);
    for (index, slice) in output.time_slices.iter().enumerate() {
        let location = slice.bot_locations.get(&0).unwrap();
        if index == 0 {
            assert_eq!(location.start_heading_rad, Some(0.7));
        }
        assert_eq!(location.steps.iter().filter(|step| step.kind == BotStepKind::Reverse).count(), 1);
        assert!(location.steps.iter().all(|step| step.kind != BotStepKind::Rotate));
    }
}

#[test]
fn geometric_turns_use_whole_curve_sweep_and_round_nav_totals_once() {
    for split_nav in [false, true] {
        let radius = 600.0;
        let diagonal = radius / 2.0_f64.sqrt();
        let positions = vec![
            WorldPos { x_mm: -radius, y_mm: 0.0 },
            WorldPos { x_mm: -diagonal, y_mm: -diagonal },
            WorldPos { x_mm: radius, y_mm: 0.0 },
        ];
        let arc = CircularArc { center: WorldPos::default(), clockwise: false, start: None, end: None };
        let mut config = kinematic_config(MotionModel::ConstantSpeed, positions.clone());
        config.link_curves =
            Arc::new([((CellId(0), CellId(1)), arc), ((CellId(1), CellId(2)), arc)].into_iter().collect());
        let mut first = move_chain_one_cell(SimTime::ZERO, SimDuration::ms(500), 1000.0, 1000.0);
        first[0].orientation = Orientation::North;
        first[1].orientation = Orientation::from_degrees(315.0);
        first[2].orientation = first[1].orientation;
        let mut second = move_chain_one_cell(SimTime::ms(2500), SimDuration::ms(1500), 1000.0, 1000.0);
        for waypoint in &mut second {
            waypoint.cell.0 += 1;
        }
        second[0].orientation = first[1].orientation;
        second[1].orientation = Orientation::South;
        second[2].orientation = Orientation::South;
        let nav = |time, waypoints| {
            SimEvent::Nav(NavEventData {
                robot_id: RobotId(0),
                robot_type: RobotType::Htm,
                waypoints,
                time,
                distance_mm: 0,
                carrying_cargo: false,
            })
        };
        let mut events = vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, positions[0])],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::TripStart(TripStartEvent {
                robot_id: RobotId(0),
                robot_type: RobotType::Htm,
                purpose: TripPurpose::ToParking,
                presentation_id: None,
                from: Some(TripEndpoint::Parking),
                to: TripEndpoint::Parking,
                sim_time_ms: 0,
            }),
        ];
        if split_nav {
            events.extend([nav(SimTime::ZERO, first), nav(SimTime::ms(2500), second)]);
        } else {
            first.pop();
            first.push(Waypoint {
                cell: CellId(1),
                orientation: second[0].orientation,
                action: WaypointAction::Wait { start: SimTime::ms(500), end: SimTime::ms(2500) },
            });
            first.extend(second.into_iter().skip(1));
            events.push(nav(SimTime::ZERO, first));
        }
        let output = run_with_events(events, config);
        assert_eq!(
            output.htm_leg_log.iter().map(|row| row.turn_count).sum::<u32>(),
            2,
            "45° +135° describes a half-turn, including across a presentation dwell"
        );
        assert_eq!(output.cumulative_scalars.htm_duty_rotating_ms, 0, "steering is not an in-place rotation");
    }

    // A 270° arc has endpoint tangents only 90° apart by the short route.
    // The ledger must measure its declared sweep, not that shortcut.
    let positions = vec![WorldPos { x_mm: -600.0, y_mm: 0.0 }, WorldPos { x_mm: 0.0, y_mm: 600.0 }];
    let mut config = kinematic_config(MotionModel::ConstantSpeed, positions.clone());
    config.link_curves = Arc::new(
        [(
            (CellId(0), CellId(1)),
            CircularArc { center: WorldPos::default(), clockwise: false, start: None, end: None },
        )]
        .into_iter()
        .collect(),
    );
    let mut waypoints = move_chain_one_cell(SimTime::ZERO, SimDuration::ms(3000), 1000.0, 1000.0);
    waypoints[0].orientation = Orientation::North;
    waypoints[1].orientation = Orientation::West;
    waypoints[2].orientation = Orientation::West;
    let output = run_with_events(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![ip(0, 0, RobotType::Htm, positions[0])],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::TripStart(TripStartEvent {
                robot_id: RobotId(0),
                robot_type: RobotType::Htm,
                purpose: TripPurpose::ToParking,
                presentation_id: None,
                from: Some(TripEndpoint::Parking),
                to: TripEndpoint::Parking,
                sim_time_ms: 0,
            }),
            SimEvent::Nav(NavEventData {
                robot_id: RobotId(0),
                robot_type: RobotType::Htm,
                waypoints,
                time: SimTime::ZERO,
                distance_mm: 0,
                carrying_cargo: false,
            }),
        ],
        config,
    );
    assert_eq!(output.htm_leg_log.iter().map(|row| row.turn_count).sum::<u32>(), 3);
}
