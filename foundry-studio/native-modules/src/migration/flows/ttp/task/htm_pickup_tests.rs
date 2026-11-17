//! A relay pickup waits for a claimed HTM blocking its departure to execute
//! its own next leg, then reserves the route against the updated occupancy.

use super::*;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::relay::ToteSearchCriteria;
use crate::model::reservation::oracle::ReservationOracle;
use crate::model::station::{StationArrivalPlan, StationArrivalSource};
use crate::model::test_model::{
    EmittedFacts, TestModelConfig, bounded_scheduler, build_model, register_idle_htm, seed_empty_rack_tote,
};
use crate::scheduler::Scheduler;
use crate::simtime::TimeRange;
use desgraph::StationId;
use pathfinder::ReservationOracle as _;
use tokio::runtime::Builder;

#[test]
fn resident_pickup_preserves_the_registered_chassis_heading() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let robot = RobotId(60);
    register_idle_htm(&model, &oracle, robot, CellId(10), SimTime::ZERO);
    let mut entry = model.available_htm.take(robot).unwrap();
    entry.orientation = Orientation::North;
    model.available_htm.put_back(entry).unwrap();
    seed_empty_rack_tote(&model, ToteId(1), RackId(1));
    let reservation = model
        .relay_service
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(10),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(reservation.relay_id, RelayId(1));
    let deposit_time = reservation.start;
    model
        .inventory
        .tote(ToteId(1))
        .unwrap()
        .move_to(ToteLocation::AtRelay { relay_id: reservation.relay_id }, deposit_time)
        .unwrap();
    let mut scheduler = Scheduler::new();
    let completion = scheduler.add_task(HtmPickup {
        zone_id: reservation.zone_id,
        handoff: Some(RelayHandoff {
            reservation,
            deposit_time,
            estimated_htm_to_relay: SimDuration::ZERO,
            station_arrival_plan: None,
            presentation_id: PresentationId(1),
        }),
        clock: scheduler.barrier().clock(),
        navigation: model.navigation.clone(),
        relay: model.relay_service.clone(),
        bots: model.bot_service.clone(),
        inventory: model.inventory.clone(),
        available: model.available_htm.clone(),
        parking: None,
        events: model.events.clone(),

        headroom: None,
    });
    scheduler.run().unwrap();
    let handoff = Builder::new_current_thread().build().unwrap().block_on(completion);
    assert_eq!(handoff.orientation, Orientation::North, "a zero-distance pickup performs no turn");
    assert_eq!(handoff.cell_reservation.cell(), CellId(10));
    assert_eq!(model.inventory.tote(ToteId(1)).unwrap().location(), ToteLocation::WithRobot { robot_id: robot });
    seed_empty_rack_tote(&model, ToteId(2), RackId(2));
    let next_inbound = model
        .relay_service
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(20),
            SimTime::seconds(3),
        )
        .unwrap();
    assert!(
        !model.relay_service.can_admit_retrieve(ZoneId(1)),
        "the actual pickup's loaded tail plus one inbound board fill the two inbound credits"
    );
    assert_eq!(model.relay_service.unfinalized_count(), 1, "pickup tote custody is already closed");
    let mut departure = Scheduler::new();
    departure.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: handoff.cell_reservation,
            action_ready_at: handoff.action_ready_at,
            orientation: handoff.orientation,
        },
        CellId(5),
        Some(ToteId(1)),
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        departure.barrier().clock(),
    ));
    departure.run().unwrap();
    assert!(model.relay_service.can_admit_retrieve(ZoneId(1)), "actual loaded departure returns its inbound credit");
    next_inbound.finalize(SimTime::seconds(50)).unwrap();
}

fn check_blocked_pickup(blocker_cell: CellId, boxed_in: bool) {
    let (model, oracle) = build_model(TestModelConfig::default());
    let picker = RobotId(60);
    let blocker = RobotId(61);
    register_idle_htm(&model, &oracle, picker, CellId(5), SimTime::ZERO);
    seed_empty_rack_tote(&model, ToteId(1), RackId(1));
    let reservation = model
        .relay_service
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(10),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(reservation.relay_id, RelayId(1));
    register_idle_htm(&model, &oracle, blocker, blocker_cell, SimTime::ZERO);
    let entry = model.available_htm.take(blocker).unwrap();
    if boxed_in {
        oracle.table().reserve_cell(RobotId(62), CellId(4), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    }
    let deposit_time = reservation.start;
    // Stage the VTM's completed boundary; the task under test is the HTM leg.
    model
        .inventory
        .tote(ToteId(1))
        .unwrap()
        .move_to(ToteLocation::AtRelay { relay_id: reservation.relay_id }, deposit_time)
        .unwrap();
    let mut scheduler = Scheduler::new();
    let clock = scheduler.barrier().clock();
    scheduler.add_task(HtmPickup {
        zone_id: reservation.zone_id,
        handoff: Some(RelayHandoff {
            reservation,
            deposit_time,
            estimated_htm_to_relay: SimDuration::seconds(5),
            station_arrival_plan: None,
            presentation_id: PresentationId(1),
        }),
        clock: clock.clone(),
        navigation: model.navigation.clone(),
        relay: model.relay_service.clone(),
        bots: model.bot_service.clone(),
        inventory: model.inventory.clone(),
        available: model.available_htm.clone(),
        parking: None,
        events: model.events.clone(),

        headroom: None,
    });
    let blocker_depart = SimTime::seconds(30);
    scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: entry.cell_reservation,
            action_ready_at: blocker_depart,
            orientation: Orientation::East,
        },
        CellId(20),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        clock,
    ));
    scheduler.run().unwrap();
    assert_eq!(model.relay_service.unfinalized_count(), 0);
    let pickup_done =
        model.bot_service.all_availability(BotKind::Htm).into_iter().find(|(id, _, _)| *id == picker).unwrap();
    assert_eq!(pickup_done.2, CellId(10));
    assert!(pickup_done.1 > blocker_depart.as_ms());
    assert_eq!(model.inventory.tote(ToteId(1)).unwrap().location(), ToteLocation::WithRobot { robot_id: picker });
    let blocked = oracle.reservations_for_cell(blocker_cell);
    let before = blocked.iter().find(|hold| hold.robot() == blocker).unwrap();
    let after = blocked.iter().find(|hold| hold.robot() == picker).unwrap();
    assert!(before.window().end().is_some_and(|end| end <= after.window().start()));
}

#[test]
fn pickup_waits_for_an_interior_holder_to_commit_its_departure() {
    check_blocked_pickup(CellId(6), false);
}

#[test]
fn pickup_reselection_wakes_when_a_destination_holder_only_changes_oracle_reservations() {
    check_blocked_pickup(CellId(10), false);
}

#[test]
fn boxed_in_pickup_reselection_wakes_when_an_exit_holder_commits_departure() {
    check_blocked_pickup(CellId(6), true);
}

#[test]
fn pickup_skips_a_nearer_htm_whose_exits_are_booked_beyond_the_route_horizon() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(60), CellId(5), SimTime::ZERO);
    register_idle_htm(&model, &oracle, RobotId(61), CellId(16), SimTime::ZERO);
    seed_empty_rack_tote(&model, ToteId(1), RackId(1));
    let reservation = model
        .relay_service
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(10),
            SimTime::ZERO,
        )
        .unwrap();
    let relay = reservation.relay_id;
    let deposit_time = reservation.start;
    for cell in [CellId(4), CellId(6)] {
        for i in 0..40 {
            oracle
                .table()
                .reserve_cell(
                    RobotId(100 + i),
                    cell,
                    TimeRange::new(SimTime::seconds(i as u64 * 10), SimTime::seconds((i as u64 + 1) * 10)),
                )
                .unwrap();
        }
    }
    let mut pickup = HtmPickup {
        zone_id: reservation.zone_id,
        handoff: Some(RelayHandoff {
            reservation,
            deposit_time,
            estimated_htm_to_relay: SimDuration::ZERO,
            station_arrival_plan: None,
            presentation_id: PresentationId(1),
        }),
        clock: Mut::new(SimTime::ZERO),
        navigation: model.navigation.clone(),
        relay: model.relay_service.clone(),
        bots: model.bot_service.clone(),
        inventory: model.inventory.clone(),
        available: model.available_htm.clone(),
        parking: None,
        events: model.events.clone(),

        headroom: None,
    };
    let (pass, _) = pickup
        .select_once(relay, CellId(10), model.navigation.map.cell_world_pos(CellId(10)).unwrap(), deposit_time)
        .unwrap();
    let SelectionPass::Selected(selection) = pass else { panic!("the farther HTM has a free approach") };
    assert_eq!(selection.htm.robot(), RobotId(61), "finite traffic can make the nearest HTM unable to depart");
    assert_eq!(model.available_htm.len(), 2, "selection spends neither HTM");
    assert!(
        oracle.reservations_for_cell(CellId(5)).iter().any(|hold| hold.robot() == RobotId(60) && hold.is_open_ended())
    );
    let _other = model.available_htm.take(RobotId(61)).unwrap();
    model.inventory.tote(ToteId(1)).unwrap().move_to(ToteLocation::AtRelay { relay_id: relay }, deposit_time).unwrap();
    let mut scheduler = Scheduler::new();
    pickup.clock = scheduler.barrier().clock();
    let runtime = Builder::new_current_thread().build().unwrap();
    let proposal = runtime.block_on(Task::propose(&pickup, ProposalId { task: TaskId(999), attempt: 0 })).unwrap();
    assert_eq!(
        ProposedReservation::start_time(&proposal),
        Some(SimTime::seconds(400)),
        "a known finite departure opening schedules a future candidacy"
    );
    assert!(model.available_htm.contains(RobotId(60)), "the future candidacy leaves its HTM available to earlier work");
    drop(proposal);
    let completion = scheduler.add_task(pickup);
    scheduler.run().expect("a finite departure opening needs no unrelated oracle mutation to wake pickup");
    let handoff = runtime.block_on(completion);
    assert_eq!(handoff.cell_reservation.robot(), RobotId(60));
    assert!(handoff.action_ready_at > SimTime::seconds(400));
    assert!(oracle.find_overlaps().is_empty());
}

#[test]
fn a_pickup_that_awaits_a_held_queue_cell_on_its_way_announces_its_assignment_at_its_window_s_start() {
    // Cells 6..=9 are a queue feeding relay 1's access cell 10; the blocker
    // holds 6 open-ended until its departure is scheduled at 30 s, so the
    // pickup's call awaits inside the pathfinder and commits its one window
    // after the barrier has passed the start the task asked for.
    let (mut model, oracle) = build_model(TestModelConfig {
        queue_cells: vec![CellId(6), CellId(7), CellId(8), CellId(9)],
        ..Default::default()
    });
    let scheduler = bounded_scheduler();
    let emitted = Arc::new(EmittedFacts::new(scheduler.barrier().clock()));
    let journal = EventJournal::new(Some(emitted.clone()));
    // The journal hears B advance, as the runner wires it.
    let mut scheduler = scheduler.with_advance_listener({
        let journal = journal.clone();
        Box::new(move |at| journal.barrier_advanced(at))
    });
    Arc::get_mut(&mut model).unwrap().events = journal;
    let picker = RobotId(60);
    register_idle_htm(&model, &oracle, picker, CellId(5), SimTime::ZERO);
    seed_empty_rack_tote(&model, ToteId(1), RackId(1));
    let reservation = model
        .relay_service
        .reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            CellId(10),
            SimTime::ZERO,
        )
        .unwrap();
    register_idle_htm(&model, &oracle, RobotId(61), CellId(6), SimTime::ZERO);
    let blocker = model.available_htm.take(RobotId(61)).unwrap();
    let deposit_time = reservation.start;
    model
        .inventory
        .tote(ToteId(1))
        .unwrap()
        .move_to(ToteLocation::AtRelay { relay_id: reservation.relay_id }, deposit_time)
        .unwrap();
    let clock = scheduler.barrier().clock();
    let plan = StationArrivalPlan::new(StationId(1), StationArrivalSource::PickFromRack, deposit_time, deposit_time);
    scheduler.add_task(HtmPickup {
        zone_id: reservation.zone_id,
        handoff: Some(RelayHandoff {
            reservation,
            deposit_time,
            estimated_htm_to_relay: SimDuration::seconds(5),
            station_arrival_plan: Some(plan),
            presentation_id: PresentationId(1),
        }),
        clock: clock.clone(),
        navigation: model.navigation.clone(),
        relay: model.relay_service.clone(),
        bots: model.bot_service.clone(),
        inventory: model.inventory.clone(),
        available: model.available_htm.clone(),
        parking: None,
        events: model.events.clone(),
        headroom: None,
    });
    let blocker_depart = SimTime::seconds(30);
    scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: blocker.cell_reservation,
            action_ready_at: blocker_depart,
            orientation: Orientation::East,
        },
        CellId(20),
        None,
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        clock,
    ));
    scheduler.run().unwrap();

    let windows = emitted.windows_of(picker);
    assert_eq!(windows.len(), 1, "one window, to the relay: {windows:?}");
    assert!(windows[0].1 > SimTime::ZERO, "the call awaited the queue cell's hold: {windows:?}");
    assert_eq!(emitted.behind_the_barrier(), Vec::<String>::new());
}
