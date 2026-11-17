use super::prelude::*;
use crate::reservation::oracle::GhostOracle;

fn relay_approach() -> WarehouseGraph {
    let mut builder = WarehouseGraphBuilder::new();
    builder.add_cell(cell(0, CellKind::Open, 0, 0));
    let mut relay = cell(1, CellKind::Relay, 1, 0);
    relay.htm_rotation_forbidden = true;
    builder.add_cell(relay);
    builder.add_cell(cell(2, CellKind::Open, 0, 0));
    builder.add_edge(c(0), c(1), Edge::uniform(1000));
    builder.add_edge(c(1), c(2), Edge { htm_drive: HtmDriveDirection::Reverse, ..Edge::uniform(1000) });
    builder.build()
}

#[test]
fn htm_enters_forward_and_leaves_in_reverse_without_rotating_at_relay() {
    let graph = relay_approach();
    let pf = pathfinder(&graph);
    let drive = &uniform_drive(RobotType::Htm, false);
    let path = pf.find_path(c(0), Orientation::East, c(2), drive, t(0)).unwrap();

    assert_waypoints!(
        path,
        [
            path_start(c(0), Orientation::East, t(0)),
            mv(c(1), Orientation::East, t(0), t(1_000)),
            mv(c(2), Orientation::East, t(1_000), t(2_000)),
            path_end(c(2), Orientation::East, t(2_000)),
        ]
    );
}

#[test]
fn forbidden_relay_rotation_makes_wrong_heading_unreachable_for_htm() {
    let graph = relay_approach();
    let pf = pathfinder(&graph);
    let drive = &uniform_drive(RobotType::Htm, false);
    assert!(matches!(
        pf.find_path(c(1), Orientation::North, c(2), drive, t(0)),
        Err(PathfinderError::Unreachable { .. })
    ));
}

#[test]
fn kinematic_reverse_edge_is_rest_to_rest() {
    let graph = relay_approach();
    let pf = pathfinder(&graph);
    let config = CollisionConfig {
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
        motion_model: MotionModel::HtmKinematic,
    };
    let drive = &Trapezoidal::htm(&HtmKinematics::uniform(1000, SimDuration::seconds(1)), false);
    let reserved =
        pf.reserve_path_without_waiting(c(0), Orientation::East, c(2), drive, &GhostOracle, &config).unwrap();
    let moves: Vec<_> = reserved
        .path
        .waypoints
        .iter()
        .filter_map(|waypoint| match waypoint.action {
            WaypointAction::Move { entry_velocity_mm_s, exit_velocity_mm_s, .. } => {
                Some((entry_velocity_mm_s, exit_velocity_mm_s))
            }
            _ => None,
        })
        .collect();
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].1, 0.0, "forward approach must stop before changing drive direction");
    assert_eq!(moves[1], (0.0, 0.0), "reverse translation must be rest-to-rest");
}

#[test]
fn arrival_orientation_uses_directed_edge_motion_and_robot_type() {
    let graph = relay_approach();
    let (htm, vtm) = (uniform_drive(RobotType::Htm, false), uniform_drive(RobotType::Vtm, false));
    assert_eq!(crate::arrival_orientation(&graph, c(1), c(2), &htm), Some(Orientation::East));
    assert_eq!(crate::arrival_orientation(&graph, c(1), c(2), &vtm), Some(Orientation::West));
    assert_eq!(crate::arrival_orientation(&graph, c(2), c(1), &htm), None);
}
