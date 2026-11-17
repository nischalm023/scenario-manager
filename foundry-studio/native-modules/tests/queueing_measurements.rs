//! Small committed conveyor histories exercise the benchmark measurement
//! boundaries without running a warehouse workload.

#[path = "common/queueing.rs"]
mod queueing;

use desgraph::{CellId, StationId};
use dessim::model::conveyance::{Conveyance, TotePhase};
use dessim::model::conveyor::ConveyorId;
use dessim::model::{InventoryService, RobotId, SimTime, ToteDesignation, ToteId, ToteLocation, ToteType};

fn service() -> Conveyance {
    Conveyance::new(ConveyorId(0), 8, 8, [StationId(1), StationId(2)]).unwrap()
}

fn visit(machine: &Conveyance, tote: u32, side: u32, times: [u64; 5]) {
    let [admitted, boarded, start, end, collected] = times.map(SimTime::ms);
    let mut admission = machine.try_admit(ToteId(tote), RobotId(tote), admitted).unwrap().unwrap();
    let (inventory, retained) = admission.board(boarded).unwrap();
    let mut slot = machine.try_present(&inventory, StationId(side), start).unwrap().unwrap();
    let presented = slot.present(start).unwrap();
    let mut transfer =
        machine.try_pair(&presented, ToteId(tote + 100), RobotId(tote + 100), start, start, end - start).unwrap();
    let pair = transfer.commit().unwrap();
    assert_eq!((pair.start, pair.end), (start, end));
    presented.clear(end, end).unwrap();
    inventory.collect(&retained, collected).unwrap();
}

#[test]
fn middle_window_clips_both_incomplete_boundary_visits() {
    let machine = service();
    visit(&machine, 1, 1, [0, 0, 2, 3, 10]);
    visit(&machine, 2, 2, [4, 4, 8, 9, 20]);
    let stats = queueing::measure(&machine, 20);
    assert_eq!(stats.full.physical_area_tote_ms, 26);
    assert_eq!(stats.full.mean_physical_wip, 1.3);
    assert_eq!(stats.full_cohort_little_wip, 1.3);
    assert_eq!(stats.mean_boarded_to_collected_secs, 0.013);
    assert_eq!(stats.mean_boarded_to_pick_start_secs, 0.003);
    assert_eq!(stats.mean_pick_end_to_collected_secs, 0.009);
    assert_eq!((stats.middle.start_ms, stats.middle.end_ms), (5, 15));
    assert_eq!((stats.middle.wip_at_start, stats.middle.wip_at_end), (2, 1));
    assert_eq!((stats.middle.boardings, stats.middle.collections), (0, 1));
    assert_eq!(stats.middle.physical_area_tote_ms, 15);
    assert_eq!(stats.middle.clipped_visit_tote_ms, 15);
    assert_eq!(stats.middle.mean_physical_wip, 1.5);
    assert_eq!(stats.middle.operator_busy_ms, 1);
    assert_eq!(stats.middle.operator_utilization, 0.1);
}

#[test]
fn future_commits_repeated_totes_and_delayed_boarding_use_physical_time() {
    let machine = service();
    visit(&machine, 2, 2, [6, 6, 8, 9, 10]);
    visit(&machine, 1, 1, [0, 2, 3, 4, 6]);
    visit(&machine, 1, 1, [6, 6, 7, 8, 11]);
    let cancelled = machine.try_admit(ToteId(3), RobotId(3), SimTime::ZERO).unwrap().unwrap();
    drop(cancelled);
    let stats = queueing::measure(&machine, 12);
    assert_eq!(machine.occupancy_at(SimTime::ms(1)).conveyor, 0);
    assert_eq!(stats.visits.len(), 3);
    assert_eq!(stats.visits.iter().filter(|v| v.inventory_tote == 1).count(), 2);
    assert_eq!(stats.visits[0].admitted_at_ms, 0);
    assert_eq!(stats.visits[0].boarded_at_ms, 2);
    assert_eq!(stats.full.physical_area_tote_ms, 13);
    assert_eq!(stats.full.operator_busy_ms, 3);
    assert_eq!(stats.full.operator_utilization, 0.25);
    assert_eq!(
        machine.occupancy_change_times(),
        [2, 3, 4, 6, 7, 8, 9, 10, 11].map(SimTime::ms),
        "cancelled and delayed provisional starts must not survive in the calendar"
    );
}

#[test]
fn a_boundary_pick_is_clipped_without_counting_its_entire_dwell() {
    let machine = service();
    visit(&machine, 1, 1, [0, 0, 2, 8, 20]);
    let stats = queueing::measure(&machine, 20);
    assert_eq!(stats.full.operator_busy_ms, 6);
    assert_eq!(stats.middle.operator_busy_ms, 3);
    assert_eq!(stats.middle.collections, 0);
    assert_eq!(stats.middle.mean_physical_wip, 1.0);
    assert_eq!(stats.mean_boarded_to_collected_secs, 0.02);
}

#[test]
#[should_panic(expected = "calendar and tote visit history differ at 6")]
fn calendar_changes_between_visit_milestones_cannot_escape_the_audit() {
    let machine = service();
    visit(&machine, 1, 1, [0, 0, 2, 3, 20]);
    let incomplete_history = queueing::measure(&machine, 20).visits;
    visit(&machine, 2, 2, [6, 6, 7, 8, 9]);

    // The second visit's entire occupancy lies between the first visit's
    // milestones. Sampling only the supplied history would miss the mismatch.
    queueing::window(&machine, &incomplete_history, 0, 20);
}

fn inventory_for_visits(machine: &Conveyance) -> InventoryService {
    let inventory = InventoryService::new();
    for visit in machine.visits() {
        let TotePhase::Collected { at, robot } = visit.phase else { panic!("test visit collected") };
        if inventory.tote(visit.tote).is_err() {
            inventory
                .register_tote(
                    visit.tote,
                    ToteType::Tote,
                    ToteDesignation::Inventory,
                    ToteLocation::WithRobot { robot_id: RobotId(99) },
                )
                .unwrap();
        }
        let tote = inventory.tote(visit.tote).unwrap();
        tote.move_to(ToteLocation::OnConveyance { conveyor_id: machine.id() }, visit.boarded_at.unwrap()).unwrap();
        tote.move_to(ToteLocation::WithRobot { robot_id: robot }, at).unwrap();
    }
    inventory
}

#[test]
fn inventory_custody_uses_half_open_visits_for_repeated_totes() {
    let machine = service();
    visit(&machine, 1, 1, [0, 2, 3, 4, 10]);
    visit(&machine, 1, 2, [10, 12, 13, 14, 20]);
    let inventory = inventory_for_visits(&machine);
    queueing::assert_inventory_custody(&machine, &inventory);
}

#[test]
#[should_panic(expected = "left its conveyor during")]
fn inventory_custody_rejects_a_departure_and_return_between_endpoints() {
    let machine = service();
    visit(&machine, 1, 1, [0, 2, 3, 4, 10]);
    let inventory = inventory_for_visits(&machine);
    let tote = inventory.tote(ToteId(1)).unwrap();
    tote.move_to(ToteLocation::AtCell { cell_id: CellId(42) }, SimTime::ms(6)).unwrap();
    tote.move_to(ToteLocation::OnConveyance { conveyor_id: machine.id() }, SimTime::ms(7)).unwrap();
    queueing::assert_inventory_custody(&machine, &inventory);
}

#[test]
#[should_panic(expected = "was not on its named collector")]
fn inventory_custody_rejects_collection_by_the_wrong_robot() {
    let machine = service();
    visit(&machine, 1, 1, [0, 2, 3, 4, 10]);
    let inventory = inventory_for_visits(&machine);
    inventory
        .tote(ToteId(1))
        .unwrap()
        .move_to(ToteLocation::WithRobot { robot_id: RobotId(7) }, SimTime::ms(10))
        .unwrap();
    queueing::assert_inventory_custody(&machine, &inventory);
}

#[test]
#[should_panic(expected = "was not with a robot immediately before boarding")]
fn inventory_custody_rejects_a_tote_that_boarded_before_its_visit() {
    let machine = service();
    visit(&machine, 1, 1, [0, 2, 3, 4, 10]);
    let inventory = inventory_for_visits(&machine);
    inventory
        .tote(ToteId(1))
        .unwrap()
        .move_to(ToteLocation::OnConveyance { conveyor_id: machine.id() }, SimTime::ms(1))
        .unwrap();
    queueing::assert_inventory_custody(&machine, &inventory);
}

#[test]
#[should_panic(expected = "completed cohort leaves no calendar occupancy at its final horizon")]
fn calendar_tail_at_the_horizon_must_not_look_drained() {
    let machine = service();
    visit(&machine, 1, 1, [0, 2, 3, 4, 20]);
    let completed = queueing::measure(&machine, 20);
    let _new_tail = machine.try_admit(ToteId(2), RobotId(2), SimTime::ms(20)).unwrap().unwrap();

    // The final endpoint has no width in [0,20), so its new occupancy does not
    // alter the integral. Draining must also check the calendar at that instant.
    let window = queueing::window(&machine, &completed.visits, 0, 20);
    assert_eq!(window.physical_area_tote_ms, completed.full.physical_area_tote_ms);
    queueing::assert_calendar_drained(&machine, 20);
}
