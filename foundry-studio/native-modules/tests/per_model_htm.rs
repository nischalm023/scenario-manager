//! Deployed HTM identities select immutable motion models and separate route
//! caches, including when two robots ask for the same spatial journey.

use std::sync::Arc;

use desgraph::{Cell, CellId, CellKind, Edge, GridPos, WarehouseGraphBuilder, WorldPos, ZoneId};
use desmap::{SystemType, WarehouseMapBuilder};
use dessim::config::{HtmConfig, SimConfig};
use dessim::events::{EventJournal, TripEndpoint};
use dessim::model::navigation::Navigation;
use dessim::model::reservation::oracle::TableOracle;
use dessim::model::{BotKind, BotService, InventoryService, RobotId};
use dessim::pathfinding::{CommitPlan, MotionModel, Orientation, PathCacheKey, ReservationRequest};
use dessim::robots::runtime::{FloorRobotModel, RobotModelId, RuntimeRobots};
use dessim::scheduler::Scheduler;
use dessim::{FHSippPathfinder, SimDuration, SimTime};
use native_modules::migration::flows::transport::{HtmPosition, Navigate};

fn model(id: u32, kind: MotionModel) -> Arc<FloorRobotModel> {
    let htm = match kind {
        MotionModel::ConstantSpeed => HtmConfig {
            empty_speed: 6.0,
            loaded_speed: 5.0,
            empty_constant_speed: 2.0,
            loaded_constant_speed: 1.0,
            ..Default::default()
        },
        MotionModel::HtmKinematic => {
            HtmConfig { empty_speed: 4.0, loaded_speed: 3.0, empty_accel: 2.0, loaded_accel: 1.0, ..Default::default() }
        }
    };
    let mut model = FloorRobotModel::legacy(&SimConfig { htm, motion_model: kind, ..Default::default() });
    model.id = RobotModelId(id);
    Arc::new(model)
}

fn navigation() -> Navigation {
    let mut graph = WarehouseGraphBuilder::new();
    for id in 0..=1 {
        graph.add_cell(Cell {
            id: CellId(id),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: id * 12, y: 0 }),
            world_pos: Some(WorldPos { x_mm: id as f64 * 12000.0, y_mm: 0.0 }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
    }
    graph.add_edge_bidi(CellId(0), CellId(1), Edge::uniform(12000));
    let map = Arc::new(WarehouseMapBuilder::new(SystemType::Relay).graph(graph.build()).build());
    let pathfinder = Arc::new(FHSippPathfinder::new(&map, 1000, SimDuration::seconds(1), Default::default()));
    let constant = model(1, MotionModel::ConstantSpeed);
    let trapezoidal = model(2, MotionModel::HtmKinematic);
    let mut runtime = RuntimeRobots::legacy(&SimConfig::default(), 0, 2);
    runtime.htm = vec![constant.clone(), trapezoidal, constant];
    runtime.validate_counts(3, 2).unwrap();
    let bots = BotService::from_runtime(&runtime, 2);
    Navigation::new(map, pathfinder, bots, Arc::new(TableOracle::default()))
}

#[test]
fn same_route_keeps_model_and_load_specific_estimates_and_execution_quotes() {
    let navigation = navigation();
    let constant = RobotId(2);
    let trapezoidal = RobotId(3);
    let same_constant = RobotId(4);
    let first = navigation.cache(constant).unwrap();
    let second = navigation.cache(trapezoidal).unwrap();
    assert!(std::ptr::eq(first, navigation.cache(same_constant).unwrap()));
    assert!(!std::ptr::eq(first, second));
    assert_eq!(navigation.models().len(), 2);

    for (carrying_cargo, constant_ms, trapezoidal_ms, constant_estimate_ms, trapezoidal_estimate_ms) in
        [(false, 6000, 5000, 2000, 3000), (true, 12000, 7000, 2400, 4000)]
    {
        let key = PathCacheKey { from: CellId(0), from_orientation: Orientation::East, to: CellId(1), carrying_cargo };
        assert_eq!(first.find_path(key).unwrap().travel_time, SimDuration::ms(constant_estimate_ms));
        assert_eq!(second.find_path(key).unwrap().travel_time, SimDuration::ms(trapezoidal_estimate_ms));
        assert_eq!(first.travel_time(key).unwrap(), SimDuration::ms(constant_ms));
        assert_eq!(second.travel_time(key).unwrap(), SimDuration::ms(trapezoidal_ms));
        assert_eq!(first.travel_time(key).unwrap(), SimDuration::ms(constant_ms));
        assert_eq!(navigation.unassigned_travel_time(key).unwrap(), SimDuration::ms(constant_ms));
    }
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    first.clear();
    assert!(first.is_empty());
    assert_eq!(second.len(), 2, "one model's cache must not evict another model's routes");
}

#[test]
fn registration_keeps_runtime_bindings_and_refuses_duplicate_or_unknown_ids() {
    let navigation = navigation();
    let bots = &navigation.bots;
    for (robot, model_id) in [(RobotId(2), RobotModelId(1)), (RobotId(3), RobotModelId(2))] {
        let model = bots.model(robot).unwrap();
        assert_eq!(model.id, model_id);
        bots.register(robot, BotKind::Htm, CellId(0)).unwrap();
        for carrying in [false, true] {
            assert!(Arc::ptr_eq(&bots.drive(robot, carrying).unwrap(), &model.execution[carrying as usize]));
        }
        assert!(bots.register(robot, BotKind::Htm, CellId(1)).unwrap_err().to_string().contains("already registered"));
        assert_eq!(bots.position_at(robot, SimTime::ZERO), Some(CellId(0)));
    }
    for robot in [RobotId(0), RobotId(1), RobotId(5)] {
        assert!(bots.model(robot).is_err());
        assert!(navigation.cache(robot).is_err());
        assert!(bots.register(robot, BotKind::Htm, CellId(0)).is_err());
        assert!(bots.drive(robot, false).is_err());
        assert_eq!(bots.position_at(robot, SimTime::ZERO), None);
    }
}

#[test]
fn unknown_robot_cannot_complete_a_zero_distance_journey() {
    for alternatives in [false, true] {
        let navigation = Arc::new(navigation());
        let robot = RobotId(99);
        let hold = navigation
            .oracle
            .commit_path(robot, &CommitPlan::single(ReservationRequest::spanning(CellId(0), SimTime::ZERO, None)), None)
            .unwrap()
            .pop()
            .unwrap();
        let hold_id = hold.id();
        let mut scheduler = Scheduler::new();
        let mut task = Navigate::new(
            HtmPosition { cell_reservation: hold, action_ready_at: SimTime::ZERO, orientation: Orientation::East },
            CellId(0),
            None,
            navigation.clone(),
            navigation.bots.clone(),
            InventoryService::new(),
            None,
            EventJournal::new(None),
            scheduler.barrier().clock(),
        );
        if alternatives {
            task = task.with_destinations(vec![(CellId(0), TripEndpoint::Parking)]);
        }
        scheduler.add_task(task);
        let error = scheduler.run().unwrap_err().to_string();
        assert!(error.contains("no model binding") || error.contains("not registered"), "{error}");
        assert_eq!(navigation.bots.position_at(robot, SimTime::ZERO), None);
        assert!(
            navigation.oracle.reservations_for_cell(CellId(0)).iter().any(|r| r.id() == hold_id && r.is_open_ended())
        );
    }
}
