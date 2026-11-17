//! Generic movement carries real reservations and curved-link headings through
//! a dwell, and resumes a blocked target when its previous holder departs.

use std::path::Path;
use std::sync::{Arc, mpsc};

use desgraph::CellId;
use dessim::config::{HtmConfig, SimConfig};
use dessim::events::{EventJournal, SimEvent};
use dessim::model::bot::LegSpec;
use dessim::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use dessim::model::navigation::Navigation;
use dessim::model::reservation::oracle::{GhostOracle, ReservationOracle, TableOracle};
use dessim::model::{BotKind, BotService, InventoryService, RobotId, ToteDesignation, ToteId, ToteLocation, ToteType};
use dessim::pathfinding::{CommitPlan, HtmKinematics, MotionModel, Orientation, ReservationRequest, WaypointAction};
use dessim::robots::reference_htm;
use dessim::robots::runtime::FloorRobotModel;
use dessim::scheduler::Scheduler;
use dessim::{FHSippPathfinder, SimDuration, SimTime, worldmap};
use native_modules::migration::flows::transport::{HtmPosition, Navigate};
use tokio::runtime::Builder;

fn curved_route() -> (Arc<Navigation>, [CellId; 3], [Orientation; 3]) {
    curved_route_with(Arc::new(TableOracle::default()), MotionModel::ConstantSpeed)
}

fn curved_route_with(
    oracle: Arc<dyn ReservationOracle>,
    motion_model: MotionModel,
) -> (Arc<Navigation>, [CellId; 3], [Orientation; 3]) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop.json");
    let world = worldmap::load(&file).unwrap();
    let mut arcs: Vec<_> = world.map.graph.edges().filter(|(_, _, edge)| edge.curve.is_some()).collect();
    arcs.sort_by_key(|&(from, to, _)| (from, to));
    let &(source, presentation, first) =
        arcs.iter().find(|(_, to, _)| arcs.iter().any(|(from, _, _)| from == to)).unwrap();
    let &(_, exit, second) = arcs.iter().find(|(from, _, _)| *from == presentation).unwrap();
    let point = |cell| world.map.graph.cell(cell).unwrap().world_pos.unwrap();
    let heading = |[dx, dy]: [f64; 2]| Orientation::from_degrees(dy.atan2(dx).to_degrees());
    let orientations = [
        heading(first.curve.unwrap().start_tangent(point(source))),
        heading(first.curve.unwrap().end_tangent(point(presentation))),
        heading(second.curve.unwrap().end_tangent(point(exit))),
    ];
    let mut sweeps = [
        first.curve.unwrap().sweep(point(source), point(presentation)).abs().to_degrees().round() as u32,
        second.curve.unwrap().sweep(point(presentation), point(exit)).abs().to_degrees().round() as u32,
    ];
    sweeps.sort();
    assert_eq!(sweeps, [45, 135]);
    let pf = Arc::new(FHSippPathfinder::new(&world.map, 1000, SimDuration::seconds(1), Default::default()));
    let figures = test_kinematics();
    let navigation =
        Arc::new(Navigation::new(Arc::new(world.map), pf, BotService::new(test_model(figures, motion_model)), oracle));
    (navigation, [source, presentation, exit], orientations)
}

fn test_kinematics() -> HtmKinematics {
    HtmKinematics { loaded_constant_speed_mm_s: 6000.0, empty_constant_speed_mm_s: 6000.0, ..reference_htm() }
}

fn test_model(figures: HtmKinematics, motion_model: MotionModel) -> Arc<FloorRobotModel> {
    let seconds = |duration: SimDuration| duration.as_ms() as f64 / 1000.0;
    let config = SimConfig {
        motion_model,
        htm: HtmConfig {
            loaded_speed: figures.loaded.max_velocity_mm_s / 1000.0,
            empty_speed: figures.unloaded.max_velocity_mm_s / 1000.0,
            loaded_constant_speed: figures.loaded_constant_speed_mm_s / 1000.0,
            empty_constant_speed: figures.empty_constant_speed_mm_s / 1000.0,
            loaded_accel: figures.loaded.acceleration_mm_s2 / 1000.0,
            empty_accel: figures.unloaded.acceleration_mm_s2 / 1000.0,
            loaded_rotate: seconds(figures.loaded_turn),
            empty_rotate: seconds(figures.empty_turn),
            loaded_rotate_180: figures.loaded_reversal.map(seconds),
            empty_rotate_180: figures.empty_reversal.map(seconds),
            ..Default::default()
        },
        ..Default::default()
    };
    Arc::new(FloorRobotModel::legacy(&config))
}

fn position(navigation: &Navigation, robot: RobotId, cell: CellId, orientation: Orientation) -> HtmPosition {
    let cell_reservation = navigation
        .oracle
        .commit_path(robot, &CommitPlan::single(ReservationRequest::spanning(cell, SimTime::ZERO, None)), None)
        .unwrap()
        .pop()
        .unwrap();
    HtmPosition { cell_reservation, action_ready_at: SimTime::ZERO, orientation }
}

#[test]
fn navigation_preserves_curved_heading_and_tote_across_a_dwell() {
    check_curved_navigation(false);
}

#[test]
fn ghost_navigation_uses_loaded_and_empty_kinematics_through_a_curved_dwell() {
    check_curved_navigation(true);
}

fn check_curved_navigation(ghost: bool) {
    for carrying in [None, Some(ToteId(1))] {
        let (navigation, [source, presentation, exit], [start_angle, present_angle, exit_angle]) =
            if ghost { curved_route_with(Arc::new(GhostOracle), MotionModel::HtmKinematic) } else { curved_route() };
        let bots = navigation.bots.clone();
        let inventory = InventoryService::new();
        inventory.set_bot_service(bots.clone());
        let robot = RobotId(1);
        bots.register(robot, BotKind::Htm, source).unwrap();
        if let Some(tote) = carrying {
            inventory
                .register_tote(
                    tote,
                    ToteType::Tote,
                    ToteDesignation::Order,
                    ToteLocation::WithRobot { robot_id: robot },
                )
                .unwrap();
            bots.commit_task(
                robot,
                TaskKind::Deliver,
                &[LegSpec {
                    leg: Leg::StationWork { cell: source },
                    requests: &[],
                    action_ready_arrival: SimTime::ZERO,
                    wp_kind: WaypointKind::Travel,
                    expect: BotExpectation::empty(),
                    carrying_after: carrying,
                }],
                &inventory,
                None,
            )
            .unwrap();
        }
        let start = position(&navigation, robot, source, start_angle);
        let start_hold = start.cell_reservation.clone();
        let (events_tx, events_rx) = mpsc::channel();
        let events = EventJournal::new(Some(Arc::new(events_tx)));
        let mut scheduler = Scheduler::new();
        let first = scheduler.add_task(Navigate::new(
            start,
            presentation,
            carrying,
            navigation.clone(),
            bots.clone(),
            inventory.clone(),
            None,
            events.clone(),
            scheduler.barrier().clock(),
        ));
        scheduler.run().unwrap();
        let mut presented = Builder::new_current_thread().build().unwrap().block_on(first);
        assert_eq!(presented.orientation, present_angle);
        assert_eq!(presented.cell_reservation.cell(), presentation);
        assert!(
            presented.cell_reservation.window().start() < presented.action_ready_at,
            "readiness must include the entire move"
        );
        if !ghost {
            let live_start = navigation
                .oracle
                .reservations_for_cell(source)
                .into_iter()
                .find(|hold| hold.id() == start_hold.id())
                .unwrap();
            assert_eq!(live_start.window().end(), Some(presented.action_ready_at));
        }
        let dwell_end = presented.action_ready_at + SimDuration::seconds(7);
        presented.action_ready_at = dwell_end;

        let mut scheduler = Scheduler::new();
        let last = scheduler.add_task(Navigate::new(
            presented,
            exit,
            carrying,
            navigation.clone(),
            bots.clone(),
            inventory.clone(),
            None,
            events,
            scheduler.barrier().clock(),
        ));
        scheduler.run().unwrap();
        let returned = Builder::new_current_thread().build().unwrap().block_on(last);
        assert_eq!(returned.orientation, exit_angle);
        assert_eq!(returned.cell_reservation.cell(), exit);
        assert!(returned.cell_reservation.is_open_ended());
        assert!(navigation.oracle.find_overlaps().is_empty());
        assert_eq!(bots.all_availability(BotKind::Htm), [(robot, returned.action_ready_at.as_ms(), exit)]);
        if let Some(tote) = carrying {
            assert_eq!(
                inventory.tote(tote).unwrap().location_at(returned.action_ready_at),
                ToteLocation::WithRobot { robot_id: robot }
            );
        }
        let nav: Vec<_> = events_rx
            .try_iter()
            .filter_map(|event| match event {
                SimEvent::Nav(nav) => Some(nav),
                _ => None,
            })
            .collect();
        assert_eq!(nav.len(), 2);
        assert_eq!(nav[1].time, dwell_end);
        for leg in nav {
            assert_eq!(leg.carrying_cargo, carrying.is_some());
            assert!(
                leg.waypoints.iter().all(|waypoint| !matches!(waypoint.action, WaypointAction::Rotate { .. })),
                "a split U-turn must keep its tangent through the dwell"
            );
            if ghost {
                let params = if carrying.is_some() { test_kinematics().loaded } else { test_kinematics().unloaded };
                let mut moves = 0;
                for pair in leg.waypoints.windows(2) {
                    if let WaypointAction::Move { start, end, entry_velocity_mm_s, exit_velocity_mm_s } = pair[1].action
                    {
                        let distance =
                            navigation.map.graph.edge_between(pair[0].cell, pair[1].cell).unwrap().distance_mm;
                        assert_eq!(entry_velocity_mm_s, 0.0, "each half-turn departs a stopped robot");
                        assert_eq!(exit_velocity_mm_s, 0.0, "the presentation dwell requires a full stop");
                        let inverse_accel = 1.0 / params.acceleration_mm_s2 + 1.0 / params.deceleration_mm_s2;
                        let peak = (2.0 * distance as f64 / inverse_accel).sqrt().min(params.max_velocity_mm_s);
                        let cruise_distance = distance as f64 - peak * peak * inverse_accel / 2.0;
                        let seconds = peak * inverse_accel + cruise_distance / peak;
                        assert!(((end - start).as_ms() as f64 - seconds * 1000.0).abs() <= 1.0);
                        moves += 1;
                    }
                }
                assert_eq!(moves, 1);
            }
        }
    }
}

#[test]
fn navigation_waits_for_a_target_hold_to_be_bounded() {
    let (navigation, [source, target, exit], [start_angle, target_angle, _]) = curved_route();
    let bots = navigation.bots.clone();
    let inventory = InventoryService::new();
    let follower = RobotId(1);
    let leader = RobotId(2);
    bots.register(follower, BotKind::Htm, source).unwrap();
    bots.register(leader, BotKind::Htm, target).unwrap();
    let first = position(&navigation, follower, source, start_angle);
    let mut blocker = position(&navigation, leader, target, target_angle);
    blocker.action_ready_at = SimTime::seconds(5);
    let initial_version = navigation.oracle.version().unwrap().seen();
    let (events_tx, events_rx) = mpsc::channel();
    let events = EventJournal::new(Some(Arc::new(events_tx)));
    let mut scheduler = Scheduler::new();
    let following = scheduler.add_task(Navigate::new(
        first,
        target,
        None,
        navigation.clone(),
        bots.clone(),
        inventory.clone(),
        None,
        events.clone(),
        scheduler.barrier().clock(),
    ));
    let leaving = scheduler.add_task(Navigate::new(
        blocker,
        exit,
        None,
        navigation.clone(),
        bots.clone(),
        inventory,
        None,
        events,
        scheduler.barrier().clock(),
    ));
    scheduler.run().unwrap();
    let (arrived, left) =
        Builder::new_current_thread().build().unwrap().block_on(async { (following.await, leaving.await) });
    assert!(
        arrived.action_ready_at > left.action_ready_at,
        "the following robot must wait until the destination is clear"
    );
    assert_eq!(navigation.oracle.version().unwrap().seen(), initial_version + 2);
    assert!(navigation.oracle.find_overlaps().is_empty());
    let follower_nav = events_rx
        .try_iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == follower => Some(nav),
            _ => None,
        })
        .unwrap();
    assert!(
        follower_nav.time >= SimTime::seconds(5),
        "the target's release was scheduled only after the barrier advanced to five seconds"
    );
    assert!(follower_nav.waypoints.iter().all(|waypoint| waypoint.action.start_time() >= follower_nav.time));
}

#[path = "navigation_tasks/alternatives.rs"]
mod alternatives;
