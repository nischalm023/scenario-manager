//! Exercise every declared order-presentation curve in the reviewed maps,
//! including the mirrored stations, through the reservation-aware pathfinder.

use std::error::Error;
use std::future::Future;
use std::path::Path;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use dessim::model::RobotId;
use dessim::model::reservation::oracle::TableOracle;
use dessim::model::reservation::{CellReservation, ReservationTable};
use dessim::pathfinding::{
    CollisionConfig, Constant, HtmKinematics, MotionModel, Orientation, ReservedPath, WaypointAction, WindowSink,
};
use dessim::worldmap;
use dessim::{FHSippPathfinder, SimDuration, SimTime};

#[test]
fn every_loaded_presentation_link_travels_its_curve_without_a_spin() {
    for (fixture, expected_curves) in [("minimal_easyloop.json", 4), ("walmart_dfw6.json", 16)] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let world = worldmap::load(&path).expect("reviewed map loads");
        let pf = FHSippPathfinder::new(&world.map, 1000, SimDuration::seconds(1), Default::default());
        let laden = Constant::htm(&HtmKinematics::uniform(1000, SimDuration::seconds(1)), true);
        let mut count = 0;
        for (from, to, edge) in world.map.graph.edges() {
            let Some(arc) = edge.curve else { continue };
            count += 1;
            let source = world.map.graph.cell(from).unwrap().world_pos.unwrap();
            let destination = world.map.graph.cell(to).unwrap().world_pos.unwrap();
            let [dx, dy] = arc.start_tangent(source);
            let departure = Orientation::from_degrees(dy.atan2(dx).to_degrees());
            let [dx, dy] = arc.end_tangent(destination);
            let arrival = Orientation::from_degrees(dy.atan2(dx).to_degrees());
            let config = CollisionConfig {
                self_robot: RobotId(1),
                predecessor: None,
                destination: None,
                sim_start: SimTime::ZERO,
                dest_dwell: Some(SimDuration::seconds(2)),
                collision_depth: 4,
                window_size: 0,
                replan_interval: 0,
                max_wait: SimDuration::seconds(30),
                max_iterations: 100_000,
                motion_model: MotionModel::ConstantSpeed,
            };
            let oracle = TableOracle::new(ReservationTable::default());
            let reserved =
                at_once(pf.reserve_path(from, departure, to, &laden, &oracle, &config, &mut NoWindow)).unwrap();
            assert_eq!(reserved.path.travel_time, SimDuration::ms(edge.distance_mm), "{fixture}: {from:?} → {to:?}");
            assert_eq!(reserved.path.waypoints.last().unwrap().orientation, arrival);
            assert_eq!(
                reserved.path.waypoints.iter().filter(|w| matches!(w.action, WaypointAction::Move { .. })).count(),
                1
            );
            assert!(reserved.path.waypoints.iter().all(|w| !matches!(w.action, WaypointAction::Rotate { .. })));
            let source_hold = reserved.committed_reservations.iter().find(|res| res.cell() == from).unwrap();
            assert_eq!(source_hold.window().end(), Some(SimTime::ms(edge.distance_mm)));
        }
        assert_eq!(count, expected_curves, "the fixture must actually exercise both halves of each U-turn");
    }
}

/// A free floor's journeys are one window each: any other stops the test.
struct NoWindow;

impl WindowSink for NoWindow {
    fn barrier(&self) -> SimTime {
        SimTime::ZERO
    }

    fn committed(
        &mut self,
        _window: &ReservedPath,
        _closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        Err("a journey on a free floor stopped short".into())
    }
}

/// The journey's result, polled once: a free floor never makes it wait.
fn at_once<F: Future>(future: F) -> F::Output {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("a journey on a free floor waits"),
    }
}
