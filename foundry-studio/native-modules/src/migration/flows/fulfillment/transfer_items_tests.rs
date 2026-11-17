use std::sync::{Arc, mpsc};

use desgraph::{RelayId, StationId, WorldPos, ZoneId};
use tokio::runtime::Builder;

use super::*;
use crate::events::{InitialPositionsEvent, TripEndpoint, TripPurpose, TripStartEvent};
use crate::model::ToteType;
use crate::model::conveyance::{ConveyedTote, RetainedHtm};
use crate::model::conveyor::ConveyorId;
use crate::model::reservation::ReservationTable;
use crate::observer::SimObserver;
use crate::observer::time_slice::MetricsConfig;
use crate::pathfinding::{Orientation, ReservationRequest};
use crate::simtime::TimeRange;

const SKU: SkuId = SkuId(7);
const LEFT: StationId = StationId(1);
const RIGHT: StationId = StationId(2);

fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

struct Harness {
    inventory: InventoryService,
    bots: BotService,
    conveyance: Conveyance,
    completion: OrderCompletionTracker,
    events: EventJournal,
    received: mpsc::Receiver<SimEvent>,
    table: ReservationTable,
}

impl Harness {
    fn new() -> Self {
        let (sender, received) = mpsc::channel();
        let sink = Arc::new(sender);
        Self {
            inventory: InventoryService::with_event_sink(Some(sink.clone())),
            bots: BotService::new(crate::model::test_model::reference_model()),
            conveyance: Conveyance::new(ConveyorId(1), 8, 8, [LEFT, RIGHT]).unwrap(),
            completion: OrderCompletionTracker::new(),
            events: EventJournal::new(Some(sink)),
            received,
            table: ReservationTable::new(SimDuration::ZERO),
        }
    }

    fn present(
        &self,
        id: u32,
        side: StationId,
        ready: SimTime,
        qty: u32,
    ) -> (ConveyedTote, RetainedHtm, InventoryPresentation) {
        self.inventory
            .register_tote(
                ToteId(id),
                ToteType::Tote,
                ToteDesignation::Inventory,
                ToteLocation::OnConveyance { conveyor_id: self.conveyance.id() },
            )
            .unwrap();
        self.inventory.tote(ToteId(id)).unwrap().add_item(SKU, qty, 0).unwrap();
        for _ in 0..qty {
            self.inventory.record_inventory_added(SKU, t(0));
        }
        let (tote, retained) =
            self.conveyance.try_admit(ToteId(id), RobotId(id), t(0)).unwrap().unwrap().board(t(5)).unwrap();
        let presentation = self.conveyance.try_present(&tote, side, ready).unwrap().unwrap().present(ready).unwrap();
        (tote, retained, presentation)
    }

    fn order(&self, id: u32, cell: CellId, ready: SimTime) -> HtmCarryingTote {
        let robot = RobotId(id);
        let tote = ToteId(id);
        self.inventory
            .register_tote(tote, ToteType::Tote, ToteDesignation::Order, ToteLocation::WithRobot { robot_id: robot })
            .unwrap();
        self.bots.register(robot, BotKind::Htm, cell).unwrap();
        self.bots
            .commit_task(
                robot,
                TaskKind::Deliver,
                &[LegSpec {
                    leg: Leg::StationApproach,
                    requests: &[ReservationRequest::new(cell, TimeRange::open_ended(ready))],
                    action_ready_arrival: ready,
                    wp_kind: WaypointKind::Present,
                    expect: BotExpectation::empty(),
                    carrying_after: Some(tote),
                }],
                &self.inventory,
                None,
            )
            .unwrap();
        HtmCarryingTote {
            tote_id: tote,
            zone_id: ZoneId(1),
            cell_reservation: self.table.reserve_cell(robot, cell, TimeRange::open_ended(ready)).unwrap(),
            action_ready_at: ready,
            orientation: Orientation::from_degrees(135.0),
            station_arrival_plan: None,
            presentation_id: PresentationId(id as u64),
            source_relay_id: None,
            deliver_from: None,
        }
    }

    fn transfer(
        &self,
        order: HtmCarryingTote,
        presented: &InventoryPresentation,
        qty: u32,
        order_id: OrderId,
    ) -> TransferItems {
        assert!(self.inventory.reserve_for_order(&[(SKU, qty)], t(0)));
        self.completion.register(order_id, presented.station(), qty).unwrap();
        self.transfer_task(order, presented, qty, order_id)
    }

    fn transfer_task(
        &self,
        order: HtmCarryingTote,
        presented: &InventoryPresentation,
        qty: u32,
        order_id: OrderId,
    ) -> TransferItems {
        let cell = order.cell_reservation.cell();
        TransferItems::new(
            order,
            presented.clone(),
            self.conveyance.clone(),
            cell,
            TotePresentation {
                order_id,
                tote_id: presented.tote(),
                source_zone_id: ZoneId(3),
                source_relay_id: RelayId(4),
                presentation_id: PresentationId(55),
            },
            SKU,
            qty,
            SimDuration::ms(10),
            self.inventory.clone(),
            self.bots.clone(),
            self.events.clone(),
            self.completion.clone(),
            None,
        )
        .unwrap()
    }
}

#[test]
fn transfer_uses_actual_readiness_and_clock_and_changes_contents_only_at_pick_end() {
    let h = Harness::new();
    let (_tote, _retained, presented) = h.present(1, LEFT, t(30), 3);
    let order = h.order(100, CellId(11), t(20));
    let reservation = order.cell_reservation.id();
    let task = h.transfer(order, &presented, 2, OrderId(9));
    let mut scheduler = Scheduler::new().with_barrier_enabled(true);
    scheduler.barrier().advance_to(t(40));
    let result = scheduler.add_task(task);
    scheduler.run().unwrap();
    let (order, pair) = Builder::new_current_thread().build().unwrap().block_on(result);
    assert_eq!((pair.start, pair.end), (t(40), t(50)));
    assert_eq!((pair.inventory_tote, pair.order_tote), (ToteId(1), ToteId(100)));
    let stock = h.inventory.tote(ToteId(1)).unwrap();
    let destination = h.inventory.tote(ToteId(100)).unwrap();
    assert_eq!((stock.sku_qty_at(SKU, t(49)), destination.sku_qty_at(SKU, t(49))), (3, 0));
    assert_eq!((stock.sku_qty_at(SKU, t(50)), destination.sku_qty_at(SKU, t(50))), (1, 2));
    assert!(destination.location_is_through(t(20), t(500), ToteLocation::WithRobot { robot_id: RobotId(100) }));
    assert_eq!(order.action_ready_at, pair.end);
    assert_eq!(order.orientation, Orientation::from_degrees(135.0));
    assert_eq!(order.cell_reservation.id(), reservation);
    assert!(order.cell_reservation.is_open_ended());
    assert_eq!(h.conveyance.occupancy_at(t(51)).presentations, 1);
    assert_eq!(h.conveyance.occupancy_at(t(51)).conveyor, 1);
    assert_eq!(h.inventory.sku_inventory_at(SKU, t(49)), 3);
    assert_eq!(h.inventory.sku_inventory_at(SKU, t(50)), 1);
    assert_eq!(h.inventory.sku_reserved_at(SKU, t(50)), 0);
    assert!(
        h.inventory.find_sku_availability(SKU, &Default::default(), t(50)).is_empty(),
        "order cargo is not future pick stock"
    );
    let events: Vec<_> = h.received.try_iter().collect();
    assert!(events.iter().any(
        |event| matches!(event, SimEvent::OrderComplete(done) if done.order_id == OrderId(9) && done.sim_time_ms == 50)
    ));
    assert!(events.iter().any(|event| matches!(event, SimEvent::CycleComplete(cycle) if cycle.sim_time_ms == 40 && cycle.dwell_ms == 10 && cycle.tote_presentation.unwrap().tote_id == ToteId(1))));
    let cycle = events
        .iter()
        .find_map(|event| match event {
            SimEvent::CycleComplete(cycle) => Some(cycle),
            _ => None,
        })
        .unwrap();
    assert_eq!(cycle.presentation_id, Some(PresentationId(100)), "operator dwell belongs to the order HTM trip");
    assert_eq!(
        cycle.tote_presentation.unwrap().presentation_id,
        PresentationId(55),
        "inventory trip metadata remains separate"
    );
    let deltas: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::InventoryDelta(delta) if delta.at == t(50) => Some((delta.tote_id, delta.delta)),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, [(ToteId(1), -2)], "stock chart cannot add picked order contents back");
    assert!(presented.clear(t(49), t(60)).is_err());
    presented.clear(t(50), t(60)).unwrap();
}

#[test]
fn repeated_presentation_events_close_their_own_dwell_in_the_observer() {
    let h = Harness::new();
    let (_tote, _retained, presented) = h.present(1, LEFT, t(10), 2);
    let mut order = h.order(100, CellId(11), t(20));
    order.presentation_id = PresentationId(55);
    assert!(h.inventory.reserve_for_order(&[(SKU, 2)], t(0)));
    h.completion.register(OrderId(9), LEFT, 2).unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    for _ in 0..2 {
        let mut scheduler = Scheduler::new();
        let completion = scheduler.add_task(h.transfer_task(order, &presented, 1, OrderId(9)));
        scheduler.run().unwrap();
        (order, _) = runtime.block_on(completion);
    }
    let (tx, rx) = mpsc::channel();
    tx.send(SimEvent::InitialPositions(InitialPositionsEvent {
        htm_headings_rad: Vec::new(),
        positions: vec![(RobotId(100), CellId(11), RobotType::Htm, WorldPos::default())],
    }))
    .unwrap();
    tx.send(SimEvent::TripStart(TripStartEvent {
        robot_id: RobotId(100),
        robot_type: RobotType::Htm,
        purpose: TripPurpose::ToStation,
        presentation_id: Some(order.presentation_id),
        from: None,
        to: TripEndpoint::Station(LEFT),
        sim_time_ms: 0,
    }))
    .unwrap();
    for event in h.received.try_iter() {
        tx.send(event).unwrap();
    }
    drop(tx);
    let output =
        SimObserver::new(MetricsConfig::default(), rx).run().expect("the facts break no rule of the event stream");
    assert_eq!(output.cumulative_scalars.picks_completed, 2);
    assert_eq!(output.cumulative_scalars.orders_completed, 1);
    assert_eq!(order.action_ready_at, t(40));
}

#[test]
fn insufficient_items_fail_before_bot_contents_or_operator_are_committed() {
    let h = Harness::new();
    let (_tote, _retained, presented) = h.present(1, LEFT, t(30), 1);
    // The order has reserved enough overall stock; the selected tote cannot
    // supply both units. Refusal must precede any partial transfer.
    let (_other_tote, _other_retained, _other_presentation) = h.present(2, RIGHT, t(30), 1);
    let order = h.order(100, CellId(11), t(20));
    let task = h.transfer(order, &presented, 2, OrderId(9));
    let mut scheduler = Scheduler::new();
    scheduler.add_task(task);
    assert!(scheduler.run().unwrap_err().to_string().contains("insufficient quantity"));
    assert_eq!(h.inventory.tote(ToteId(1)).unwrap().sku_qty_at(SKU, t(100)), 1);
    assert!(h.inventory.tote(ToteId(100)).unwrap().is_empty_at(t(100)));
    assert!(h.conveyance.transfers().is_empty());
    assert_eq!(h.conveyance.occupancy_at(t(35)).operator, 0);
    h.bots.for_each(BotKind::Htm, |_, plan| assert_eq!(plan.availability().unwrap().0, t(20)));
    h.completion.validate_pick(OrderId(9), 2).unwrap();
}

#[test]
fn two_presentations_share_one_operator() {
    let h = Harness::new();
    let (_left_tote, _left_retained, left) = h.present(1, LEFT, t(10), 1);
    let (_right_tote, _right_retained, right) = h.present(2, RIGHT, t(15), 1);
    let mut scheduler = Scheduler::new();
    let first = scheduler.add_task(h.transfer(h.order(100, CellId(11), t(20)), &left, 1, OrderId(10)));
    let second = scheduler.add_task(h.transfer(h.order(101, CellId(12), t(25)), &right, 1, OrderId(11)));
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    let (_, first) = runtime.block_on(first);
    let (_, second) = runtime.block_on(second);
    assert!(first.end <= second.start || second.end <= first.start);
    assert!(first.start >= t(20) && second.start >= t(25));
    for ms in 0..100 {
        assert!(h.conveyance.occupancy_at(t(ms)).operator <= 1);
    }
}

#[test]
fn matching_custody_at_endpoints_does_not_hide_an_interrupted_presentation() {
    let h = Harness::new();
    let (_tote, _retained, presented) = h.present(1, LEFT, t(30), 1);
    let order = h.order(100, CellId(11), t(20));
    let tote = h.inventory.tote(ToteId(100)).unwrap();
    tote.move_to(ToteLocation::AtRelay { relay_id: RelayId(8) }, t(35)).unwrap();
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(100) }, t(39)).unwrap();
    let mut scheduler = Scheduler::new();
    scheduler.add_task(h.transfer(order, &presented, 1, OrderId(9)));
    assert!(scheduler.run().unwrap_err().to_string().contains("leaves its HTM"));
    assert!(h.conveyance.transfers().is_empty());
    assert_eq!(h.inventory.tote(ToteId(1)).unwrap().sku_qty_at(SKU, t(100)), 1);
}

#[test]
fn wrong_order_cell_or_conveyor_custody_cannot_present_a_pair() {
    for wrong_cell in [true, false] {
        let h = Harness::new();
        let (_tote, _retained, presented) = h.present(1, LEFT, t(30), 1);
        let order = h.order(100, CellId(11), t(20));
        let mut task = h.transfer(order, &presented, 1, OrderId(9));
        if wrong_cell {
            task.order_cell = CellId(999);
        } else {
            h.inventory
                .tote(ToteId(1))
                .unwrap()
                .move_to(ToteLocation::OnConveyance { conveyor_id: ConveyorId(999) }, t(0))
                .unwrap();
        }
        let mut scheduler = Scheduler::new();
        scheduler.add_task(task);
        assert!(scheduler.run().is_err());
        assert!(h.conveyance.transfers().is_empty());
        assert_eq!(h.conveyance.occupancy_at(t(35)).operator, 0);
    }
}
