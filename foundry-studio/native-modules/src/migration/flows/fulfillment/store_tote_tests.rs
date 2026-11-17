//! Storage waits for external progress, including a committed departure that
//! only changes floor reservations. A failed parking relief must not wake itself.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use desgraph::{CellId, RelayId, ZoneId};

use super::*;
use crate::events::SimEvent;
use crate::flows::transport::{HtmPosition, Navigate};
use crate::model::htm::registry::{AvailableHtm, Observer};
use crate::model::reservation::oracle::ReservationOracle;
use crate::model::test_model::block;
use crate::model::test_model::{
    TestModelConfig, bounded_scheduler, build_model, register_idle_htm, register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::Orientation;
use pathfinder::ReservationOracle as _;

#[test]
fn storage_selection_wakes_when_an_assigned_robot_only_changes_oracle_reservations() {
    storage_selection_wakes(false);
}

#[test]
fn failed_relief_does_not_wake_storage_with_its_own_restored_htms() {
    storage_selection_wakes(true);
}

struct FailedReliefs(AtomicUsize);

impl Observer for FailedReliefs {
    fn on_register(&self, entry: &AvailableHtm) {
        if matches!(entry.robot(), RobotId(62..=64)) {
            let count = self.0.fetch_add(1, Ordering::Relaxed) + 1;
            // Three idle blockers, with no intervening floor change. Fail
            // promptly if a regression spins inside a single task poll.
            assert!(count <= 3, "storage retried a failed relief before external progress");
        }
    }

    fn on_take(&self, _: &AvailableHtm) {}
}

fn storage_selection_wakes(failed_relief: bool) {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (tx, rx) = mpsc::channel();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let storing = RobotId(60);
    let tote = ToteId(1);
    model
        .inventory
        .register_tote(
            tote,
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
        )
        .unwrap();
    model.inventory.tote(tote).unwrap().move_to(ToteLocation::WithRobot { robot_id: storing }, SimTime::ZERO).unwrap();
    register_loaded_htm(&model, &oracle, storing, CellId(5), tote);
    let source = model.available_htm.take(storing).unwrap();
    let mut blockers = Vec::new();
    for index in 1..=4 {
        let robot = RobotId(60 + index);
        register_idle_htm(&model, &oracle, robot, CellId(index * 10), SimTime::ZERO);
        if index == 1 || !failed_relief {
            blockers.push(model.available_htm.take(robot).unwrap());
        }
    }
    assert!(matches!(
        block(model.relay_service.reserve_relay_for_deposit(
            CellId(5),
            Default::default(),
            SimTime::ZERO,
            storing,
            None
        )),
        Err(ReservationError::AccessHeld { .. })
    ));
    let reliefs = Arc::new(FailedReliefs(AtomicUsize::new(0)));
    if failed_relief {
        model.available_htm.add_observer(reliefs.clone());
        // Observer registration replays the current fleet before live changes.
        reliefs.0.store(0, Ordering::Relaxed);
    }
    let mut scheduler = bounded_scheduler();
    scheduler.add_task(StoreTote {
        commit_observer: None,
        handoff: Some(HtmCarryingTote {
            tote_id: tote,
            zone_id: ZoneId(1),
            cell_reservation: source.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
            station_arrival_plan: None,
            presentation_id: PresentationId(1),
            source_relay_id: None,
            deliver_from: None,
        }),
        pool: RackUse::Inventory,
        station: StationId(0),
        navigation: model.navigation.clone(),
        relay: model.relay_service.clone(),
        bots: model.bot_service.clone(),
        inventory: model.inventory.clone(),
        available: model.available_htm.clone(),
        parking: failed_relief.then(|| ParkingRegistry::from_map(&model.navigation.map)),
        events: model.events.clone(),
    });
    let release_at = SimTime::seconds(30);
    scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: blockers.remove(0).cell_reservation,
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
    ));
    scheduler.run().unwrap();
    if failed_relief {
        assert_eq!(reliefs.0.load(Ordering::Relaxed), 3, "the fixture must exercise failed relief");
    }
    assert!(!scheduler.stopped(), "relay selection missed the oracle-only change and reached the test deadline");
    let nav = rx
        .try_iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == storing => Some(nav),
            _ => None,
        })
        .unwrap();
    assert!(nav.time >= release_at);
    assert!(matches!(model.inventory.tote(tote).unwrap().location(), ToteLocation::InRack { .. }));
    assert_eq!(model.relay_service.unfinalized_count(), 0);
    assert_eq!(model.relay_service.htm_access_holder(RelayId(2)), Some(RobotId(62)));
    assert!(oracle.find_overlaps().is_empty());
}
