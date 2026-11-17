//! Bot plan append, carrying state and time-order checks with only their
//! actual dependencies: inventory and optional parking.

use super::plan::*;
use crate::model::inventory::InventoryService;
use crate::model::{RobotId, SimTime, ToteId};
use crate::pathfinding::ReservationRequest;
use desgraph::CellId;

fn cell(n: u32) -> CellId {
    CellId(n)
}
fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

/// 5-cell corridor leg: cells 10-14, each 1000ms, starting at `start_ms`.
fn corridor_windows(start: u64) -> Vec<ReservationRequest> {
    (0..5)
        .map(|i| {
            let enter = start + i * 1000;
            let exit = enter + 1000;
            ReservationRequest::spanning(cell(10 + i as u32), t(enter), Some(t(exit)))
        })
        .collect()
}

#[test]
fn initial_position_persists_through_idle_and_is_superseded_by_first_movement() {
    let mut plan = BotPlan::new_at(RobotId(1), cell(9));
    for at in [t(0), t(500), t(100_000)] {
        assert_eq!(plan.position_at(at).unwrap().cell, cell(9));
    }
    assert_eq!(plan.availability(), Some((t(0), cell(9))));
    let task = plan.register_task(TaskKind::Navigate);
    plan.append_leg(
        task,
        Leg::Travel,
        &corridor_windows(1000),
        t(6000),
        WaypointKind::Travel,
        BotExpectation::empty(),
        None,
        &InventoryService::new(),
        None,
    )
    .unwrap();
    assert_eq!(plan.position_at(t(999)).unwrap().cell, cell(9));
    assert_eq!(plan.position_at(t(1000)).unwrap().cell, cell(10));
    assert_eq!(plan.position_at(t(5999)).unwrap().cell, cell(14));
    assert!(plan.position_at(t(6000)).is_none(), "expired movement cannot reveal the old initial hold");
}

#[test]
fn append_leg_and_query_position() {
    let mut plan = BotPlan::new(RobotId(1));
    let task = plan.register_task(TaskKind::Park);
    plan.append_leg(
        task,
        Leg::Parking { cell: cell(14) },
        &corridor_windows(0),
        t(5000),
        WaypointKind::Travel,
        BotExpectation::empty(),
        None,
        &InventoryService::new(),
        None,
    )
    .expect("append");

    assert_eq!(plan.len(), 5);
    assert!(!plan.is_empty());

    // position_at returns the waypoint containing T.
    let wp = plan.position_at(t(500)).expect("mid cell 10");
    assert_eq!(wp.cell, cell(10));
    let wp = plan.position_at(t(2500)).expect("mid cell 12");
    assert_eq!(wp.cell, cell(12));
    assert!(plan.position_at(t(5500)).is_none(), "past plan end");

    // availability: Parking legs return the first waypoint of the leg
    // (when the bot becomes idle and available for retasking, before
    // it arrives at the parking cell).
    let (avail_t, avail_cell) = plan.availability().expect("not empty");
    assert_eq!(avail_t, t(0), "Parking availability = first wp enter (retaskable)");
    assert_eq!(avail_cell, cell(10));
}

#[test]
fn append_empty_leg_records_action_ready_without_waypoint() {
    let mut plan = BotPlan::new(RobotId(1));
    let task = plan.register_task(TaskKind::Deliver);
    let inventory = InventoryService::new();

    // Predecessor leg: bot drives onto cell 14 with an open-ended hold
    // ending only when the next move closes it. Mirrors the upstream
    // reservation a real predecessor would have created.
    let pre = vec![ReservationRequest::spanning(cell(14), t(0), None)];
    plan.append_leg(
        task,
        Leg::RelayApproach,
        &pre,
        t(0),
        WaypointKind::Travel,
        BotExpectation::empty(),
        None,
        &inventory,
        None,
    )
    .expect("predecessor leg");

    let waypoints_before = plan.len();
    let legs_before = plan.legs().len();

    // Empty/no-move leg — bot stays put, just records action_ready_arrival.
    let leg_id = plan
        .append_leg(
            task,
            Leg::RelayApproach,
            &[],
            t(5_000),
            WaypointKind::Travel,
            BotExpectation::empty(),
            Some(ToteId(7)),
            &inventory,
            None,
        )
        .expect("empty leg should append");

    assert_eq!(plan.len(), waypoints_before, "empty leg adds no waypoints");
    assert_eq!(plan.legs().len(), legs_before + 1, "leg meta still registered");
    let meta = plan.legs().iter().find(|l| l.id == leg_id).expect("leg meta");
    assert!(meta.wp_range.is_empty(), "wp_range is empty for no-move leg");
    assert_eq!(meta.action_ready_arrival, t(5_000));

    // Carrying state advanced.
    assert_eq!(plan.carrying_tote(), Some(ToteId(7)));

    // availability() reports the empty leg's action_ready_arrival paired
    // with the bot's last known cell (the predecessor's terminal).
    let (avail_t, avail_cell) = plan.availability().expect("not empty");
    assert_eq!(avail_t, t(5_000));
    assert_eq!(avail_cell, cell(14));
}

#[test]
fn append_rejects_backward_in_time_leg() {
    let mut plan = BotPlan::new(RobotId(1));
    let task = plan.register_task(TaskKind::Deliver);

    let windows: Vec<ReservationRequest> = (0..3)
        .map(|i| ReservationRequest::spanning(cell(i), t(5000 + i as u64 * 1000), Some(t(6000 + i as u64 * 1000))))
        .collect();
    let inventory = InventoryService::new();
    plan.append_leg(
        task,
        Leg::StationApproach,
        &windows,
        t(8000),
        WaypointKind::Travel,
        BotExpectation::empty(),
        None,
        &inventory,
        None,
    )
    .expect("first leg");

    let bad_windows = vec![ReservationRequest::spanning(cell(20), t(2000), Some(t(3000)))];
    let result = plan.append_leg(
        task,
        Leg::Parking { cell: cell(14) },
        &bad_windows,
        t(3000),
        WaypointKind::Park,
        BotExpectation::empty(),
        None,
        &inventory,
        None,
    );
    assert!(result.is_err(), "should reject backward-in-time leg");
}
