use desgraph::{RelayId, ZoneId};
use tokio::runtime::Builder;

use super::*;
use crate::events::TripEndpoint;
use crate::flows::fulfillment::pending_returns::ReturnWork;
use crate::model::ToteType;
use crate::model::conveyance::TotePhase;
use crate::model::conveyor::ConveyorId;
use crate::model::test_model::{TestModel, TestModelConfig, build_model};
use crate::pathfinding::Orientation;
use crate::simtime::TimeRange;

#[derive(Display)]
#[display("Collector")]
struct Collector {
    collect: Option<CollectTote>,
    exit: CellId,
}

#[async_trait(?Send)]
impl Task for Collector {
    type Proposal = EmptyProposal;
    type Schedule = (HtmCarryingTote, StationId, SimTime);
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let collect = self.collect.take().unwrap();
        let navigation = collect.navigation.clone();
        let bots = collect.bots.clone();
        let inventory = collect.inventory.clone();
        let events = collect.events.clone();
        let (mut trip, station) = ctx.add_task(collect).await;
        let loaded_at = trip.action_ready_at;
        let position = ctx
            .add_task(Navigate::new(
                HtmPosition {
                    cell_reservation: trip.cell_reservation,
                    action_ready_at: loaded_at,
                    orientation: trip.orientation,
                },
                self.exit,
                Some(trip.tote_id),
                navigation,
                bots,
                inventory,
                None,
                events,
                ctx.barrier().clock(),
            ))
            .await;
        trip.cell_reservation = position.cell_reservation;
        trip.action_ready_at = position.action_ready_at;
        trip.orientation = position.orientation;
        Ok((trip, station, loaded_at))
    }
}

fn return_work(model: &TestModel, service: &Conveyance, id: u32, ready: u64) -> (ReturnWork, RetainedHtm) {
    model
        .inventory
        .register_tote(
            ToteId(id),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::OnConveyance { conveyor_id: service.id() },
        )
        .unwrap();
    let (conveyed, retained) =
        service.try_admit(ToteId(id), RobotId(id), SimTime::ZERO).unwrap().unwrap().board(SimTime::ZERO).unwrap();
    let held =
        service.try_present(&conveyed, StationId(id), SimTime::ZERO).unwrap().unwrap().present(SimTime::ZERO).unwrap();
    let ready_at = SimTime::seconds(ready);
    held.clear(SimTime::ZERO, ready_at).unwrap();
    (
        ReturnWork {
            conveyed,
            ready_at,
            station: StationId(id),
            zone: ZoneId(id),
            source_relay: Some(RelayId(id)),
            presentation_id: PresentationId(id as u64),
            deliver_from: Some(TripEndpoint::Relay(RelayId(id))),
        },
        retained,
    )
}

#[test]
fn fifo_htms_collect_reverse_ready_totes_with_the_chosen_totes_metadata_and_full_dwell() {
    let mut floor_model = crate::model::test_model::reference_model();
    Arc::get_mut(&mut floor_model).unwrap().figures.conveyor_pickup_secs = Some(5.0);
    let (model, oracle) = build_model(TestModelConfig {
        floor_model: Some(floor_model),
        bot_htms: vec![(RobotId(1), CellId(13)), (RobotId(2), CellId(7))],
        ..Default::default()
    });
    let service = Conveyance::new(ConveyorId(0), 2, 2, [StationId(1), StationId(2)]).unwrap();
    let pending = PendingReturns::new();
    let (later, first) = return_work(&model, &service, 1, 100);
    let (earlier, second) = return_work(&model, &service, 2, 50);
    pending.publish(later).unwrap();
    pending.publish(earlier).unwrap();
    let collector = |retained: RetainedHtm, cell, unloaded, exit| Collector {
        collect: Some(CollectTote {
            position: Some(HtmPosition {
                cell_reservation: oracle
                    .table()
                    .reserve_cell(retained.robot(), cell, TimeRange::open_ended(SimTime::ZERO))
                    .unwrap(),
                action_ready_at: SimTime::seconds(unloaded),
                orientation: Orientation::East,
            }),
            returns: pending.clone(),
            retained,
            conveyance: service.clone(),
            collection: CellId(10),

            navigation: model.navigation.clone(),
            bots: model.bot_service.clone(),
            inventory: model.inventory.clone(),
            parking: None,
            events: model.events.clone(),
        }),
        exit,
    };
    let mut scheduler = Scheduler::new();
    // Register the later unload first: the HTM that reaches the collection
    // cell first claims first, whatever the task order.
    let second = scheduler.add_task(collector(second, CellId(7), 20, CellId(4)));
    let first = scheduler.add_task(collector(first, CellId(13), 10, CellId(14)));
    scheduler.run().unwrap();
    let ((first, first_station, first_load), (second, second_station, second_load)) =
        Builder::new_current_thread().build().unwrap().block_on(async { (first.await, second.await) });
    assert_eq!((first.tote_id, first.cell_reservation.robot()), (ToteId(2), RobotId(1)));
    assert_eq!((second.tote_id, second.cell_reservation.robot()), (ToteId(1), RobotId(2)));
    assert!(first_load < SimTime::seconds(100), "the first-unloaded HTM must not wait for its original tote");
    assert!(second_load >= SimTime::seconds(105));
    for (trip, station, loaded, source) in
        [(&first, first_station, first_load, 2), (&second, second_station, second_load, 1)]
    {
        assert_eq!(station, StationId(source));
        assert_eq!(trip.zone_id, ZoneId(source));
        assert_eq!(trip.source_relay_id, Some(RelayId(source)));
        assert_eq!(trip.presentation_id, PresentationId(source as u64));
        assert_eq!(trip.deliver_from, Some(TripEndpoint::Relay(RelayId(source))));
        assert!(trip.station_arrival_plan.is_none());
        let tote = model.inventory.tote(trip.tote_id).unwrap();
        assert_eq!(
            tote.location_at(loaded - SimDuration::ms(1)),
            ToteLocation::OnConveyance { conveyor_id: service.id() }
        );
        assert_eq!(tote.location_at(loaded), ToteLocation::WithRobot { robot_id: trip.cell_reservation.robot() });
        let holds = model.navigation.oracle.reservations_for_cell(CellId(10));
        let hold = holds.iter().find(|hold| hold.robot() == trip.cell_reservation.robot()).unwrap();
        assert!(loaded >= hold.window().start() + SimDuration::seconds(5));
    }
    assert_eq!(service.occupancy_at(first_load - SimDuration::ms(1)).conveyor, 2);
    assert_eq!(service.occupancy_at(first_load).conveyor, 1);
    assert_eq!(service.occupancy_at(second_load).conveyor, 0);
    assert_eq!(service.occupancy_at(first_load - SimDuration::ms(1)).retained_htms, 2);
    assert_eq!(service.occupancy_at(first_load).retained_htms, 1);
    assert_eq!(service.occupancy_at(second_load).retained_htms, 0);
    assert_eq!(service.unfinished(), 0);
    assert!(service.abandoned_custody().is_empty());
    assert!(service.visits().iter().all(|visit| matches!(visit.phase, TotePhase::Collected { .. })));
}
