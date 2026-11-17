//! A picked-up tote's inbound unit follows a journey's windows whatever task
//! made the journey. On the test line, cells 7, 8 and 9 are the queue feeding
//! station 1's cell 10, which an idle HTM holds open-ended until its departure
//! is scheduled at 30 s.

use std::cell::RefCell;
use std::rc::Rc;

use desgraph::{CellId, StationId, ZoneId};

use crate::flows::transport::windows::{PathCommitObserver, RouteWindow};
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::relay::tests::helpers::{count_open_pickup_units, record_pickup_unit};
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::test_model::{
    TestModel, TestModelConfig, bounded_scheduler, build_model, register_idle_htm, register_loaded_htm,
};
use crate::model::{RobotId, SimTime, ToteId};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::{Orientation, Path};
use crate::scheduler::Scheduler;

fn navigate(model: &TestModel, scheduler: &Scheduler, from: CellReservation, at: SimTime, to: CellId) -> Navigate {
    let carrying = (from.robot() == RobotId(60)).then_some(ToteId(60));
    Navigate::new(
        HtmPosition { cell_reservation: from, action_ready_at: at, orientation: Orientation::East },
        to,
        carrying,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    )
}

#[test]
fn a_navigate_after_a_pickup_keeps_its_unit_on_the_queue_cell_its_first_window_ends_on() {
    let (model, oracle) = build_model(TestModelConfig {
        stations: vec![(StationId(1), CellId(10))],
        queue_cells: vec![CellId(7), CellId(8), CellId(9)],
        ..Default::default()
    });
    let mut scheduler = bounded_scheduler();
    register_idle_htm(&model, &oracle, RobotId(61), CellId(10), SimTime::ZERO);
    let holder = model.available_htm.take(RobotId(61)).unwrap();
    model
        .inventory
        .register_tote(
            ToteId(60),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::WithRobot { robot_id: RobotId(60) },
        )
        .unwrap();
    register_loaded_htm(&model, &oracle, RobotId(60), CellId(3), ToteId(60));
    let loaded = model.available_htm.take(RobotId(60)).unwrap();
    // The unit its pickup left riding the robot's hold.
    record_pickup_unit(&model.navigation, ZoneId(1), loaded.cell_reservation.clone());

    let counted = Rc::new(RefCell::new(Vec::new()));
    let observer: PathCommitObserver = {
        let (navigation, relay, counted) = (model.navigation.clone(), model.relay_service.clone(), counted.clone());
        Box::new(move |_: &Path, window: RouteWindow| {
            let units = count_open_pickup_units(&navigation, &relay, ZoneId(1));
            counted.borrow_mut().push((window, units));
            Ok(())
        })
    };
    scheduler.add_task(
        navigate(&model, &scheduler, loaded.cell_reservation, SimTime::ZERO, CellId(10)).with_commit_observer(observer),
    );
    scheduler.add_task(navigate(&model, &scheduler, holder.cell_reservation, SimTime::seconds(30), CellId(59)));
    scheduler.run().unwrap();

    assert_eq!(
        *counted.borrow(),
        vec![(RouteWindow::ShortOfTarget, 1), (RouteWindow::Last, 0)],
        "the unit rides the queue cell's hold, and the window onto the station releases it"
    );
}
