//! A U-turn is two declared curved links separated by a presentation cell.
//! Reservations still protect cells and opposing uses of a directed link;
//! these tests make no claim about swept bodies on unrelated crossing links.

use std::f64::consts::PI;

use desgraph::{CircularArc, WorldPos};

use super::prelude::*;
use crate::Velocity;

fn u_turn(presentation_degrees: f64) -> WarehouseGraph {
    let radius = 586.7;
    let presentation_angle = PI + presentation_degrees.to_radians();
    let positions = [
        WorldPos { x_mm: -radius, y_mm: 800.0 },
        WorldPos { x_mm: radius * presentation_angle.cos(), y_mm: radius * presentation_angle.sin() },
        WorldPos { x_mm: radius, y_mm: 800.0 },
    ];
    let mut builder = WarehouseGraphBuilder::new();
    for (i, position) in positions.into_iter().enumerate() {
        let mut node = cell(i as u32, CellKind::Open, 0, 0);
        node.grid_pos = None;
        node.world_pos = Some(position);
        builder.add_cell(node);
    }
    let first = CircularArc {
        center: WorldPos { x_mm: 0.0, y_mm: 0.0 },
        clockwise: false,
        start: Some(WorldPos { x_mm: -radius, y_mm: 0.0 }),
        end: None,
    };
    let second = CircularArc { start: None, end: Some(WorldPos { x_mm: radius, y_mm: 0.0 }), ..first };
    for (i, arc) in [first, second].into_iter().enumerate() {
        let mut edge = Edge::uniform(arc.length_mm(positions[i], positions[i + 1]).round() as u64);
        edge.curve = Some(arc);
        builder.add_edge(c(i as u32), c(i as u32 + 1), edge);
    }
    builder.build()
}

fn config(motion_model: MotionModel) -> CollisionConfig {
    CollisionConfig {
        self_robot: r(1),
        sim_start: t(0),
        dest_dwell: Some(ms(1000)),
        max_wait: ms(30_000),
        max_iterations: 100_000,
        motion_model,
        predecessor: None,
        destination: None,
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
    }
}

#[test]
fn presentation_dwell_preserves_tangent_for_the_remaining_u_turn() {
    for presentation_degrees in [45.0, 135.0] {
        let graph = u_turn(presentation_degrees);
        let pf = pathfinder(&graph);
        let oracle = TableOracle::new(ReservationTable::default());
        let first_cfg = CollisionConfig { dest_dwell: None, ..config(MotionModel::ConstantSpeed) };
        let drive = &uniform_drive(RobotType::Htm, true);
        let first =
            pf.reserve_path_without_waiting(c(0), Orientation::North, c(1), drive, &oracle, &first_cfg).unwrap();
        let first_length = graph.edge_between(c(0), c(1)).unwrap().distance_mm;
        assert_eq!(first.path.travel_time, ms(first_length));
        assert!(first.path.waypoints.iter().all(|w| !matches!(w.action, WaypointAction::Rotate { .. })));
        let tangent = first.path.waypoints.last().unwrap().orientation;
        assert_eq!(tangent, Orientation::from_degrees(270.0 + presentation_degrees));
        let source = first.committed_reservations.iter().find(|res| res.cell() == c(0)).unwrap();
        assert_eq!(source.window().end(), Some(t(first_length)), "source is held for the whole curve");
        let from = graph.cell(c(0)).unwrap().world_pos.unwrap();
        let to = graph.cell(c(1)).unwrap().world_pos.unwrap();
        let chord = (from.x_mm - to.x_mm).hypot(from.y_mm - to.y_mm).ceil() as u64;
        assert!(first_length > chord);
        assert!(oracle.has_edge_conflict(c(1), c(0), t(chord), ms(1), Some(r(2))));
        assert!(!oracle.has_edge_conflict(c(1), c(0), t(first_length), ms(1), Some(r(2))));

        let presentation = first.committed_reservations.iter().find(|res| res.cell() == c(1)).unwrap().clone();
        let depart = t(first_length + 2000);
        let second_cfg = CollisionConfig {
            predecessor: Some(presentation),
            sim_start: depart,
            ..config(MotionModel::ConstantSpeed)
        };
        let drive = &uniform_drive(RobotType::Htm, true);
        let second = pf.reserve_path_without_waiting(c(1), tangent, c(2), drive, &oracle, &second_cfg).unwrap();
        let second_length = graph.edge_between(c(1), c(2)).unwrap().distance_mm;
        assert_eq!(second.path.travel_time, ms(second_length));
        assert_eq!(second.path.waypoints.last().unwrap().orientation, Orientation::South);
        assert!(second.path.waypoints.iter().all(|w| !matches!(w.action, WaypointAction::Rotate { .. })));
        assert_eq!(second.path.waypoints[1].start_time(), depart);
        let presentation_hold = oracle.reservations_for_cell(c(1));
        assert_eq!(presentation_hold[0].window().end(), Some(depart + ms(second_length)));
    }
}

#[test]
fn source_reservation_cannot_end_after_the_chord_but_before_the_curve() {
    let graph = u_turn(135.0);
    let pf = pathfinder(&graph);
    let oracle = TableOracle::new(ReservationTable::default());
    let length = graph.edge_between(c(0), c(1)).unwrap().distance_mm;
    oracle
        .commit_path(r(2), &CommitPlan::single(ReservationRequest::spanning(c(0), t(length - 1), None)), None)
        .unwrap();
    let drive = &uniform_drive(RobotType::Htm, false);
    let result =
        pf.find_path_reserved(c(0), Orientation::North, c(1), drive, &oracle, &config(MotionModel::ConstantSpeed));
    assert!(result.is_err(), "the robot cannot clear the source before the blocking hold starts");
}

#[test]
fn shorter_curve_departure_fits_an_intermediate_window_without_phantom_hold_time() {
    let graph = u_turn(135.0);
    let pf = pathfinder(&graph);
    let oracle = TableOracle::new(ReservationTable::default());
    let first_length = graph.edge_between(c(0), c(1)).unwrap().distance_mm;
    let second_length = graph.edge_between(c(1), c(2)).unwrap().distance_mm;
    let finish = t(first_length + second_length);
    oracle.commit_path(r(2), &CommitPlan::single(ReservationRequest::spanning(c(1), finish, None)), None).unwrap();
    let drive = &uniform_drive(RobotType::Htm, true);
    let path = pf
        .reserve_path_without_waiting(
            c(0),
            Orientation::North,
            c(2),
            drive,
            &oracle,
            &config(MotionModel::ConstantSpeed),
        )
        .unwrap();
    assert_eq!(path.path.travel_time, finish - t(0));
    assert!(
        path.path
            .waypoints
            .iter()
            .all(|w| !matches!(w.action, WaypointAction::Rotate { .. } | WaypointAction::Wait { .. }))
    );
    let hold = path.committed_reservations.iter().find(|res| res.cell() == c(1)).unwrap();
    assert_eq!(hold.window().end(), Some(finish));
}

#[test]
fn curve_kinematics_and_cached_quote_use_full_arc_and_tangent_lead_length() {
    let graph = u_turn(45.0);
    let pf = pathfinder(&graph);
    let oracle = TableOracle::new(ReservationTable::default());
    let drive = &Trapezoidal::htm(&HtmKinematics::uniform(1000, SimDuration::seconds(1)), true);
    let (path, _) = pf
        .find_path_reserved(c(0), Orientation::North, c(1), drive, &oracle, &config(MotionModel::HtmKinematic))
        .unwrap();
    let length = graph.edge_between(c(0), c(1)).unwrap().distance_mm;
    // At 1000mm/s and 1000mm/s², the 500mm acceleration/deceleration legs
    // together add one second over constant-speed travel for this >1m move.
    assert!(path.travel_time.abs_diff(ms(length + 1000)) <= ms(1));
    let drive = &uniform_drive(RobotType::Htm, true);
    let spatial = pf.find_path(c(0), Orientation::North, c(1), drive, t(0)).unwrap();
    let drive = &Trapezoidal::htm(&HtmKinematics::uniform(1000, SimDuration::seconds(1)), true);
    assert_eq!(pf.kinematic_travel_time(&spatial, drive), path.travel_time);
    assert!(path.waypoints.iter().all(|w| !matches!(w.action, WaypointAction::Rotate { .. })));
    let final_move = path.waypoints.iter().find(|w| matches!(w.action, WaypointAction::Move { .. })).unwrap();
    assert!(matches!(final_move.action, WaypointAction::Move { exit_velocity_mm_s: 0.0, .. }));
}

#[test]
fn constant_quote_reprices_the_curve_and_tangent_leads_at_execution_speed() {
    let graph = u_turn(45.0);
    let pf = pathfinder(&graph);
    let estimate = uniform_drive(RobotType::Htm, true);
    let execution = Constant { speed: Velocity(3251.0), ..estimate };
    let spatial = pf.find_path(c(0), Orientation::North, c(2), &estimate, t(0)).unwrap();
    let quoted = pf.kinematic_travel_time(&spatial, &execution);
    let oracle = TableOracle::new(ReservationTable::default());
    let (reserved, _) = pf
        .find_path_reserved(c(0), Orientation::North, c(2), &execution, &oracle, &config(MotionModel::ConstantSpeed))
        .unwrap();
    let expected: u64 = [(c(0), c(1)), (c(1), c(2))]
        .into_iter()
        .map(|(from, to)| graph.edge_between(from, to).unwrap().distance_mm * 1000 / 3251)
        .sum();
    assert_eq!(quoted, ms(expected));
    assert_eq!(quoted, reserved.travel_time);
    assert_ne!(quoted, spatial.travel_time);
}
