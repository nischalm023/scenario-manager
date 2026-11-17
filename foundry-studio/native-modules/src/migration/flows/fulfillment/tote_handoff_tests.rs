//! Claim identity and physical custody must agree before a tote handoff writes
//! either service. Failed provisional claims roll back their own calendars.

use std::sync::{Arc, mpsc};

use desgraph::{CellId, StationId, ZoneId};
use tokio::runtime::Builder;

use super::*;
use crate::model::ToteType;
use crate::model::conveyance::{Conveyance, TotePhase};
use crate::model::reservation::ReservationTable;
use crate::pathfinding::{Orientation, ReservationRequest};
use crate::simtime::TimeRange;

fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

struct Harness {
    inventory: InventoryService,
    bots: BotService,
    conveyor: Conveyance,
    table: ReservationTable,
    events: EventJournal,
    received: mpsc::Receiver<SimEvent>,
}

impl Harness {
    fn new(location: ToteLocation) -> Self {
        let (sender, received) = mpsc::channel();
        let sender = Arc::new(sender);
        let inventory = InventoryService::with_event_sink(Some(sender.clone()));
        for id in [1, 2] {
            inventory.register_tote(ToteId(id), ToteType::Tote, ToteDesignation::Inventory, location).unwrap();
        }
        Self {
            inventory,
            bots: BotService::new(crate::model::test_model::reference_model()),
            conveyor: Conveyance::new(ConveyorId(1), 8, 8, [StationId(1), StationId(2)]).unwrap(),
            table: ReservationTable::new(SimDuration::ZERO),
            events: EventJournal::new(Some(sender)),
            received,
        }
    }

    fn position(&self, robot: RobotId, carrying: Option<ToteId>) -> HtmPosition {
        let cell = CellId(10);
        self.bots.register(robot, BotKind::Htm, cell).unwrap();
        self.bots
            .commit_task(
                robot,
                TaskKind::Navigate,
                &[LegSpec {
                    leg: Leg::StationApproach,
                    requests: &[ReservationRequest::new(cell, TimeRange::open_ended(t(20)))],
                    action_ready_arrival: t(20),
                    wp_kind: WaypointKind::Present,
                    expect: BotExpectation::empty(),
                    carrying_after: carrying,
                }],
                &self.inventory,
                None,
            )
            .unwrap();
        HtmPosition {
            cell_reservation: self.table.reserve_cell(robot, cell, TimeRange::open_ended(t(20))).unwrap(),
            action_ready_at: t(20),
            orientation: Orientation::East,
        }
    }

    fn plan(&self) -> String {
        let mut plan = String::new();
        self.bots.for_each(BotKind::Htm, |_, bot| {
            plan.push_str(&format!(
                "{:?} {:?} {:?} {:?}",
                bot.tasks(),
                bot.legs(),
                bot.iter().collect::<Vec<_>>(),
                bot.carrying_tote()
            ));
        });
        plan
    }

    fn locations(&self) -> Vec<ToteLocation> {
        [1, 2]
            .into_iter()
            .flat_map(|id| {
                let tote = self.inventory.tote(ToteId(id)).unwrap();
                (0..100).map(move |ms| tote.location_at(t(ms)))
            })
            .collect()
    }

    fn returning(&self) -> (ConveyedTote, RetainedHtm, CollectionClaim) {
        let (tote, htm) = self.conveyor.try_admit(ToteId(1), RobotId(1), t(0)).unwrap().unwrap().board(t(0)).unwrap();
        let presentation =
            self.conveyor.try_present(&tote, StationId(1), t(1)).unwrap().unwrap().present(t(1)).unwrap();
        presentation.clear(t(2), t(10)).unwrap();
        let claim = self.conveyor.try_collect(&tote, &htm, t(10)).unwrap().unwrap();
        (tote, htm, claim)
    }
}

#[test]
fn extraction_preserves_contents_through_dwell_and_only_removes_cartons() {
    for physical_type in [ToteType::Carton, ToteType::Tote, ToteType::Tray] {
        let robot = RobotId(1);
        let h = Harness::new(ToteLocation::Uninitialized);
        let tote_id = h
            .inventory
            .create_tote(physical_type, ToteDesignation::Order, ToteLocation::WithRobot { robot_id: robot }, t(0))
            .unwrap();
        let tote = h.inventory.tote(tote_id).unwrap();
        tote.add_item(SkuId(7), 3, 0).unwrap();
        let position = h.position(robot, Some(tote_id));
        let mut scheduler = Scheduler::new();
        let completion = scheduler.add_task(ExtractOrder {
            order: Some(HtmCarryingTote {
                tote_id,
                zone_id: ZoneId(0),
                cell_reservation: position.cell_reservation,
                action_ready_at: position.action_ready_at,
                orientation: position.orientation,
                station_arrival_plan: None,
                presentation_id: PresentationId(55),
                source_relay_id: None,
                deliver_from: None,
            }),
            order_id: OrderId(1),
            station: Some(StationId(1)),
            dwell: SimDuration::ms(10),
            inventory: h.inventory.clone(),
            bots: h.bots.clone(),
            events: h.events.clone(),
        });
        scheduler.run().unwrap();
        let outcome = Builder::new_current_thread().build().unwrap().block_on(completion);
        assert_eq!(tote.contents_at(t(29)), vec![(SkuId(7), 3)]);
        assert_eq!(tote.location_at(t(29)), ToteLocation::WithRobot { robot_id: robot });
        assert!(tote.is_empty_at(t(30)));
        let carton = physical_type == ToteType::Carton;
        match outcome {
            ExtractedOrder::Removed(position) => {
                assert!(carton);
                assert_eq!(position.action_ready_at, t(30));
                assert!(position.cell_reservation.is_open_ended());
            }
            ExtractedOrder::Reusable(order) => {
                assert!(!carton);
                assert_eq!(order.tote_id, tote_id);
                assert_eq!(order.action_ready_at, t(30));
            }
        }
        assert_eq!(
            tote.location_at(t(30)),
            if carton { ToteLocation::Extracted } else { ToteLocation::WithRobot { robot_id: robot } }
        );
        h.bots.for_each(BotKind::Htm, |_, plan| assert_eq!(plan.carrying_tote(), (!carton).then_some(tote_id)));
        let events: Vec<_> = h.received.try_iter().collect();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SimEvent::ToteMoved(moved)
                if moved.tote_id == tote_id && moved.new_state == ToteVisualState::Extracted && moved.time == t(30)))
                .count(),
            usize::from(carton)
        );
    }
}

#[test]
fn unload_rejects_mismatched_claims_and_interrupted_custody_before_mutation() {
    for wrong in ["tote", "robot", "conveyor", "custody", "interrupted", "spent"] {
        let h = Harness::new(ToteLocation::WithRobot { robot_id: RobotId(1) });
        let position = h.position(RobotId(1), Some(ToteId(1)));
        let claim_robot = if wrong == "robot" { RobotId(2) } else { RobotId(1) };
        let mut admission = h.conveyor.try_admit(ToteId(1), claim_robot, t(10)).unwrap().unwrap();
        let _boarded = if wrong == "spent" { Some(admission.board(t(10)).unwrap()) } else { None };
        if wrong == "custody" || wrong == "interrupted" {
            let tote = h.inventory.tote(ToteId(1)).unwrap();
            let leave = if wrong == "interrupted" { 25 } else { 20 };
            tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(9) }, t(leave)).unwrap();
            if wrong == "interrupted" {
                tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(29)).unwrap();
            }
        }
        let plan = h.plan();
        let locations = h.locations();
        h.received.try_iter().for_each(drop);
        let mut scheduler = Scheduler::new();
        scheduler.add_task(UnloadTote {
            position: Some(position),
            tote: ToteId(if wrong == "tote" { 2 } else { 1 }),
            conveyor: ConveyorId(if wrong == "conveyor" { 2 } else { 1 }),
            admission: Some(admission),
            dwell: SimDuration::ms(10),
            inventory: h.inventory.clone(),
            bots: h.bots.clone(),
            events: h.events.clone(),
        });
        scheduler.run().expect_err(wrong);
        assert_eq!(h.plan(), plan, "{wrong} changed the bot plan");
        assert_eq!(h.locations(), locations, "{wrong} changed tote custody");
        assert_eq!(h.received.try_iter().count(), 0, "{wrong} emitted physical work");
        assert_eq!(h.conveyor.occupancy_at(t(100)).conveyor, usize::from(wrong == "spent"));
    }
}

#[test]
fn load_rejects_wrong_tote_robot_or_conveyor_before_mutation() {
    for wrong in ["tote", "robot", "conveyor", "interrupted", "spent"] {
        let h = Harness::new(ToteLocation::OnConveyance { conveyor_id: ConveyorId(1) });
        let robot = RobotId(if wrong == "robot" { 2 } else { 1 });
        let position = h.position(robot, None);
        let (_tote, _htm, mut claim) = h.returning();
        if wrong == "conveyor" || wrong == "interrupted" {
            let tote = h.inventory.tote(ToteId(1)).unwrap();
            let leave = if wrong == "interrupted" { 25 } else { 20 };
            tote.move_to(ToteLocation::OnConveyance { conveyor_id: ConveyorId(2) }, t(leave)).unwrap();
            if wrong == "interrupted" {
                tote.move_to(ToteLocation::OnConveyance { conveyor_id: ConveyorId(1) }, t(29)).unwrap();
            }
        }
        if wrong == "spent" {
            claim.commit(t(10)).unwrap();
        }
        let plan = h.plan();
        let locations = h.locations();
        h.received.try_iter().for_each(drop);
        let mut scheduler = Scheduler::new();
        scheduler.add_task(LoadTote {
            position: Some(position),
            tote: ToteId(if wrong == "tote" { 2 } else { 1 }),
            claim: Some(claim),
            dwell: SimDuration::ms(10),
            inventory: h.inventory.clone(),
            bots: h.bots.clone(),
            events: h.events.clone(),
        });
        scheduler.run().expect_err(wrong);
        assert_eq!(h.plan(), plan, "{wrong} changed the bot plan");
        assert_eq!(h.locations(), locations, "{wrong} changed tote custody");
        assert_eq!(h.received.try_iter().count(), 0, "{wrong} emitted physical work");
        assert_eq!(h.conveyor.occupancy_at(t(100)).conveyor, usize::from(wrong != "spent"));
        if wrong != "spent" {
            assert_eq!(h.conveyor.returning().len(), 1, "failed claim did not release its selection");
        }
    }
}

#[test]
fn valid_unload_and_load_transfer_the_named_tote_only_at_dwell_end() {
    let h = Harness::new(ToteLocation::WithRobot { robot_id: RobotId(1) });
    let position = h.position(RobotId(1), Some(ToteId(1)));
    let admission = h.conveyor.try_admit(ToteId(1), RobotId(1), t(10)).unwrap().unwrap();
    let mut scheduler = Scheduler::new();
    scheduler.barrier().advance_to(t(30));
    let done = scheduler.add_task(UnloadTote {
        position: Some(position),
        tote: ToteId(1),
        conveyor: ConveyorId(1),
        admission: Some(admission),
        dwell: SimDuration::ms(10),
        inventory: h.inventory.clone(),
        bots: h.bots.clone(),
        events: h.events.clone(),
    });
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    let (position, conveyed, retained, unloaded) = runtime.block_on(done);
    assert_eq!(unloaded, t(40));
    let tote = h.inventory.tote(ToteId(1)).unwrap();
    assert_eq!(tote.location_at(t(39)), ToteLocation::WithRobot { robot_id: RobotId(1) });
    assert_eq!(tote.location_at(t(40)), ToteLocation::OnConveyance { conveyor_id: ConveyorId(1) });
    h.bots.for_each(BotKind::Htm, |_, plan| assert_eq!(plan.carrying_tote(), None));
    let presentation = h.conveyor.try_present(&conveyed, StationId(1), t(45)).unwrap().unwrap().present(t(45)).unwrap();
    presentation.clear(t(50), t(60)).unwrap();
    let claim = h.conveyor.try_collect(&conveyed, &retained, t(60)).unwrap().unwrap();
    let mut scheduler = Scheduler::new();
    let done = scheduler.add_task(LoadTote {
        position: Some(position),
        tote: ToteId(1),
        claim: Some(claim),
        dwell: SimDuration::ms(10),
        inventory: h.inventory.clone(),
        bots: h.bots.clone(),
        events: h.events.clone(),
    });
    scheduler.run().unwrap();
    let position = runtime.block_on(done);
    assert_eq!(position.action_ready_at, t(70));
    assert_eq!(tote.location_at(t(69)), ToteLocation::OnConveyance { conveyor_id: ConveyorId(1) });
    assert_eq!(tote.location_at(t(70)), ToteLocation::WithRobot { robot_id: RobotId(1) });
    assert_eq!(conveyed.phase(), TotePhase::Collected { at: t(70), robot: RobotId(1) });
    assert_eq!(h.conveyor.unfinished(), 0);
    assert!(h.conveyor.abandoned_custody().is_empty());
}
