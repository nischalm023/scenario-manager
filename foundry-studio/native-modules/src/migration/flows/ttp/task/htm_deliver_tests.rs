//! A delivery to a station cell that no queue cell feeds, held by another
//! HTM: the delivering HTM keeps its source hold, and its drive starts no
//! earlier than the barrier when the holder's departure is scheduled.

use std::sync::{Arc, Mutex, mpsc};

use desgraph::ZoneId;

use super::*;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::reservation::oracle::ReservationOracle;
use crate::model::test_model::{
    TestModelConfig, bounded_scheduler, build_model, register_idle_htm, register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::Orientation;
use crate::utils::temporal_counter::TemporalCounter;
use pathfinder::ReservationOracle as _;

#[test]
fn a_delivery_to_a_held_station_cell_no_queue_cell_feeds_keeps_its_origin_until_the_holders_departure_is_scheduled() {
    let station = StationId(1);
    let (mut model, oracle) =
        build_model(TestModelConfig { stations: vec![(station, CellId(10))], ..Default::default() });
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let robot = RobotId(60);
    let tote = ToteId(1);
    model
        .inventory
        .register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, ToteLocation::WithRobot { robot_id: robot })
        .unwrap();
    register_loaded_htm(&model, &oracle, robot, CellId(5), tote);
    let source = model.available_htm.take(robot).unwrap();
    register_idle_htm(&model, &oracle, RobotId(61), CellId(10), SimTime::ZERO);
    let blocker = model.available_htm.take(RobotId(61)).unwrap();
    let mut scheduler = bounded_scheduler();
    scheduler.add_task(HtmDeliver::new(
        station,
        InflightSpan::open(Arc::new(Mutex::new(TemporalCounter::new())), SimTime::ZERO, Version::new()),
        HtmCarryingTote {
            tote_id: tote,
            zone_id: ZoneId(1),
            cell_reservation: source.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
            station_arrival_plan: None,
            presentation_id: PresentationId(1),
            source_relay_id: None,
            deliver_from: None,
        },
        scheduler.barrier().clock(),
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        model.station_service.clone(),
        model.station_arrivals.clone(),
        None,
        false,
        None,
    ));
    let release_at = SimTime::seconds(30);
    scheduler.add_task(ClearStation {
        journey: Some(Navigate::new(
            HtmPosition {
                cell_reservation: blocker.cell_reservation,
                action_ready_at: release_at,
                orientation: Orientation::East,
            },
            CellId(15),
            None,
            model.navigation.clone(),
            model.bot_service.clone(),
            model.inventory.clone(),
            None,
            model.events.clone(),
            scheduler.barrier().clock(),
        )),
        available: model.available_htm.clone(),
    });
    scheduler.run().unwrap();
    let navigation: Vec<_> = rx
        .try_iter()
        .filter_map(|event| match event {
            SimEvent::Nav(event) if event.robot_id == robot => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(navigation.len(), 1, "the delivery must wake and complete");
    assert!(navigation[0].time >= release_at, "the drive is planned behind the barrier: {:?}", navigation[0].time);
    let holds = oracle.reservations_for_cell(CellId(5));
    assert!(holds.iter().any(|hold| hold.robot() == robot && hold.window().end().is_some_and(|end| end > release_at)));
}

#[derive(Display)]
#[display("ClearStation")]
struct ClearStation {
    journey: Option<Navigate>,
    available: AvailableHtmRegistry,
}

#[async_trait(?Send)]
impl Task for ClearStation {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let position = ctx.add_task(self.journey.take().unwrap()).await;
        self.available
            .register(position.cell_reservation, position.action_ready_at, position.orientation, None)
            .map_err(|error| PlanError::TaskGraphInvariant { context: "ClearStation", detail: error.to_string() })?;
        Ok(())
    }
}
