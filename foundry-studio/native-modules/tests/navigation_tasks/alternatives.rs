//! Equivalent physical destinations are selected at departure and reconsidered
//! after occupancy changes; cargo and trip metadata follow the committed port.

use std::thread;
use std::time::Duration;

use desgraph::graph::Edge;
use desgraph::{Cell, CellKind, GridPos, StationId, WarehouseGraphBuilder, WorldPos, ZoneId};
use desmap::{SystemType, WarehouseMapBuilder};
use dessim::events::{TripEndpoint, TripPurpose};
use dessim::sim_types::{StopReason, StopSignal};

use super::*;

fn routes() -> Arc<Navigation> {
    let mut graph = WarehouseGraphBuilder::new();
    for (id, x, y) in [(0, 0, 0), (1, 1, 0), (2, 0, 3), (3, 0, 4), (4, 2, 0), (5, 10, 10)] {
        graph.add_cell(Cell {
            id: CellId(id),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x, y }),
            world_pos: Some(WorldPos { x_mm: x as f64 * 1000.0, y_mm: y as f64 * 1000.0 }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
    }
    for (from, to, length) in [(0, 1, 1000), (0, 2, 3000), (2, 3, 1000), (1, 4, 1000)] {
        graph.add_edge_bidi(CellId(from), CellId(to), Edge::uniform(length));
    }
    let map = WarehouseMapBuilder::new(SystemType::Relay).graph(graph.build()).build();
    let pathfinder = Arc::new(FHSippPathfinder::new(&map, 1000, SimDuration::seconds(1), Default::default()));
    let bots = BotService::new(test_model(reference_htm(), MotionModel::ConstantSpeed));
    Arc::new(Navigation::new(Arc::new(map), pathfinder, bots, Arc::new(TableOracle::default())))
}

fn check_alternatives(both_blocked: bool, finite_nearest: bool) {
    let navigation = routes();
    let bots = navigation.bots.clone();
    let inventory = InventoryService::new();
    inventory.set_bot_service(bots.clone());
    let moving = RobotId(1);
    let tote = ToteId(1);
    bots.register(moving, BotKind::Htm, CellId(0)).unwrap();
    inventory
        .register_tote(tote, ToteType::Tote, ToteDesignation::Order, ToteLocation::WithRobot { robot_id: moving })
        .unwrap();
    bots.commit_task(
        moving,
        TaskKind::Deliver,
        &[LegSpec {
            leg: Leg::StationWork { cell: CellId(0) },
            requests: &[],
            action_ready_arrival: SimTime::ZERO,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation::empty(),
            carrying_after: Some(tote),
        }],
        &inventory,
        None,
    )
    .unwrap();
    // The nearest port remains occupied; its holder has already been assigned
    // elsewhere, so it cannot create a fleet-availability notification.
    navigation
        .oracle
        .commit_path(
            RobotId(2),
            &CommitPlan::single(ReservationRequest::spanning(
                CellId(1),
                SimTime::ZERO,
                finite_nearest.then_some(SimTime::seconds(100)),
            )),
            None,
        )
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let events = EventJournal::new(Some(Arc::new(tx)));
    let stop = StopSignal::new();
    let mut scheduler = Scheduler::new().with_stop_signal(stop.clone());
    // A second of wall time, then the run stops: a selection that never wakes
    // ends the test instead of hanging it.
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        stop.stop(StopReason::WallTimeout);
    });
    let arrived = scheduler.add_task(
        Navigate::new(
            position(&navigation, moving, CellId(0), Orientation::East),
            CellId(1),
            Some(tote),
            navigation.clone(),
            bots.clone(),
            inventory.clone(),
            None,
            events.clone(),
            scheduler.barrier().clock(),
        )
        .with_destinations(vec![
            (CellId(5), TripEndpoint::Station(StationId(5))), // unreachable alternative
            (CellId(1), TripEndpoint::Station(StationId(1))),
            (CellId(2), TripEndpoint::Station(StationId(2))),
        ])
        .with_trip(TripPurpose::ToStation, None, TripEndpoint::Station(StationId(1))),
    );
    let ready = SimTime::seconds(30);
    if both_blocked {
        let leaving = RobotId(3);
        bots.register(leaving, BotKind::Htm, CellId(2)).unwrap();
        let mut held = position(&navigation, leaving, CellId(2), Orientation::North);
        held.action_ready_at = ready;
        scheduler.add_task(Navigate::new(
            held,
            CellId(3),
            None,
            navigation.clone(),
            bots.clone(),
            inventory.clone(),
            None,
            events.clone(),
            scheduler.barrier().clock(),
        ));
    }
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "alternative selection failed to wake on an oracle-only change");
    let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
    assert_eq!(arrived.cell_reservation.cell(), CellId(2));
    assert_eq!(bots.position_at(moving, arrived.action_ready_at), Some(CellId(2)));
    assert!(inventory.tote(tote).unwrap().location_is_through(
        SimTime::ZERO,
        arrived.action_ready_at,
        ToteLocation::WithRobot { robot_id: moving }
    ));
    let events: Vec<_> = rx.try_iter().collect();
    let trip = events
        .iter()
        .find_map(|event| match event {
            SimEvent::TripStart(trip) if trip.robot_id == moving => Some(trip),
            _ => None,
        })
        .unwrap();
    assert_eq!(trip.to, TripEndpoint::Station(StationId(2)));
    let nav = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == moving => Some(nav),
            _ => None,
        })
        .unwrap();
    assert!(nav.carrying_cargo);
    assert_eq!(nav.waypoints.last().unwrap().cell, CellId(2));
    assert_eq!(nav.waypoints.last().unwrap().action.end_time(), arrived.action_ready_at);
    assert_eq!(SimTime::ms(trip.sim_time_ms), nav.time);
    if both_blocked {
        assert!(nav.time >= ready, "departure must be re-floored after waiting for physical clearance");
    } else {
        assert_eq!(nav.time, SimTime::ZERO, "a free alternative must not wait for the busy nearest port");
    }
    assert!(nav.waypoints.iter().all(|waypoint| waypoint.action.start_time() >= nav.time));
    assert!(navigation.oracle.find_overlaps().is_empty());
}

#[test]
fn busy_nearest_port_uses_free_farther_port_with_actual_station_metadata_and_custody() {
    check_alternatives(false, false);
}

#[test]
fn all_ports_blocked_reconsiders_the_alternative_when_only_its_oracle_hold_changes() {
    check_alternatives(true, false);
}

#[test]
fn finite_destination_hold_is_included_in_the_arrival_ranking() {
    check_alternatives(false, true);
}

#[test]
fn empty_destinations_are_refused_and_preserve_the_source_hold() {
    let navigation = routes();
    let robot = RobotId(1);
    let bots = navigation.bots.clone();
    bots.register(robot, BotKind::Htm, CellId(0)).unwrap();
    let held = position(&navigation, robot, CellId(0), Orientation::East);
    let source_id = held.cell_reservation.id();
    let mut scheduler = Scheduler::new();
    scheduler.add_task(
        Navigate::new(
            held,
            CellId(1),
            None,
            navigation.clone(),
            bots,
            InventoryService::new(),
            None,
            EventJournal::new(None),
            scheduler.barrier().clock(),
        )
        .with_destinations(Vec::new()),
    );
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("destination alternatives are empty"));
    assert!(
        navigation
            .oracle
            .reservations_for_cell(CellId(0))
            .iter()
            .any(|hold| hold.id() == source_id && hold.is_open_ended())
    );
}

#[test]
fn the_current_cell_is_a_zero_distance_alternative() {
    let navigation = routes();
    let robot = RobotId(1);
    let bots = navigation.bots.clone();
    bots.register(robot, BotKind::Htm, CellId(0)).unwrap();
    let mut held = position(&navigation, robot, CellId(0), Orientation::East);
    held.action_ready_at = SimTime::seconds(12);
    let source_id = held.cell_reservation.id();
    let mut scheduler = Scheduler::new();
    let arrived = scheduler.add_task(
        Navigate::new(
            held,
            CellId(1),
            None,
            navigation.clone(),
            bots,
            InventoryService::new(),
            None,
            EventJournal::new(None),
            scheduler.barrier().clock(),
        )
        .with_destinations(vec![
            (CellId(1), TripEndpoint::Station(StationId(1))),
            (CellId(0), TripEndpoint::Station(StationId(0))),
        ]),
    );
    scheduler.run().unwrap();
    let arrived = Builder::new_current_thread().build().unwrap().block_on(arrived);
    assert_eq!(arrived.cell_reservation.id(), source_id);
    assert_eq!(arrived.action_ready_at, SimTime::seconds(12));
    assert!(navigation.oracle.find_overlaps().is_empty());
}
