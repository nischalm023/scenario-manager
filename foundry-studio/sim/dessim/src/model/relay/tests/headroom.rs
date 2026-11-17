//! Admission credits follow the exact physical pickup tail, while speculative
//! sources leave room for an admitted consumer's next retrieval.

use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use desgraph::{CellId, RelayId, ZoneId};
use tokio::runtime::Builder;

use super::helpers::{build_test_env, empty_htm_registry, test_drives};
use crate::config::VtmConfig;
use crate::events::EventJournal;
use crate::model::ids::TaskId;
use crate::model::location::{ToteDesignation, ToteLocation, ToteType};
use crate::model::relay::{RelayRetrieveSelection, RelayService, RetrieveAdmission, ToteSearchCriteria};
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::reservation::oracle::{ReservationOracle, TableOracle};
use crate::model::test_model::OneWindow;
use crate::model::{RackId, RobotId, SimDuration, SimTime, ToteId};
use crate::pathfinding::{CollisionConfig, FHSippPathfinder, MotionModel, Orientation};
use crate::scheduler::task::proposal::{AcceptError, ProposalId, ProposedReservation};
use crate::signals::Version;
use crate::simtime::TimeRange;
use pathfinder::ReservationOracle as _;

const CRITERIA: ToteSearchCriteria = ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory };

fn fixture() -> (RelayService, Arc<TableOracle>, Arc<FHSippPathfinder>, RelayRetrieveSelection) {
    let (data, inventory, pathfinder) =
        build_test_env(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40), (5, 50), (6, 60)])], &[(1, 12, 1)]);
    inventory
        .register_tote(
            ToteId(1),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
        )
        .unwrap();
    let oracle = Arc::new(TableOracle::default());
    let registry = empty_htm_registry(&data, &inventory);
    let service = RelayService::new(
        &data,
        inventory,
        super::helpers::test_navigation(&data, pathfinder.clone(), oracle.clone()),
        &registry,
        &VtmConfig::default(),
    )
    .unwrap();
    let selection = service.select_relay_for_retrieve(CRITERIA, CellId(15), SimTime::ZERO).unwrap();
    (service, oracle, pathfinder, selection)
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

/// A `reserve_path` whose way is free completes when first polled.
fn at_once<F: Future>(future: F) -> F::Output {
    match poll_once(pin!(future)) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the way is free, yet the call awaits"),
    }
}

fn owner(attempt: u32) -> ProposalId {
    ProposalId { task: TaskId(1), attempt }
}

/// Commit `source`'s robot one cell on from `start`; its hold there.
fn depart(
    pathfinder: &FHSippPathfinder,
    oracle: &TableOracle,
    source: &CellReservation,
    start: SimTime,
) -> CellReservation {
    let drives = test_drives();
    let config = CollisionConfig {
        self_robot: source.robot(),
        predecessor: Some(source.clone()),
        destination: None,
        sim_start: start,
        dest_dwell: None,
        collision_depth: 8,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(100),
        max_iterations: 10_000,
        motion_model: MotionModel::ConstantSpeed,
    };
    let reserved = at_once(pathfinder.reserve_path(
        source.cell(),
        Orientation::East,
        CellId(source.cell().0 + 1),
        drives[1].as_ref(),
        oracle,
        &config,
        &mut OneWindow(&EventJournal::new(None)),
    ))
    .unwrap();
    assert!(
        !oracle
            .reservations_for_cell(source.cell())
            .iter()
            .find(|hold| hold.id() == source.id())
            .unwrap()
            .is_open_ended()
    );
    reserved.committed_reservations.last().unwrap().clone()
}

#[test]
fn pickup_preserves_inbound_credit_until_actual_departure_and_wakes_waiting_source() {
    Builder::new_current_thread().build().unwrap().block_on(async {
        let (service, oracle, pathfinder, mut selection) = fixture();
        let drives = test_drives();
        let mut slots = Vec::new();
        // Four unresolved inbound deliveries consume R−2; custody is deliberately
        // outside this test, which exercises the slot-to-physical-hold handoff.
        for relay in 1..=4 {
            selection.relay_id = RelayId(relay);
            slots.push(service.reserve_selected_relay_for_retrieve(&selection).unwrap());
        }
        let picked = slots.pop().unwrap();
        let relay = picked.relay_id;
        let source = oracle
            .table()
            .reserve_cell(
                RobotId(60),
                service.htm_access_cell(relay).unwrap(),
                TimeRange::open_ended(SimTime::seconds(10)),
            )
            .unwrap();
        picked.finalize_pickup(SimTime::seconds(12), &source).unwrap();
        assert_eq!(service.unfinalized_count(), 3, "tote slot custody ended at pickup");
        assert!(!service.can_admit_retrieve(ZoneId(1)), "loaded HTM still occupies the fourth inbound credit");
        service.state.with(|state| {
            assert_eq!(state.timelines[&relay].available_at(), SimTime::seconds(12));
            assert_eq!(state.timelines[&relay].occupancy_intervals().last().unwrap().1, 12_000);
        });

        let aim_version = Version::new();
        let source_request = service.source_retrieve(
            CRITERIA,
            CellId(15),
            SimTime::ZERO,
            SimTime::ZERO,
            || None,
            &aim_version,
            owner(0),
        );
        let mut source_request = Box::pin(source_request);
        assert!(poll_once(source_request.as_mut()).is_pending());
        let onward = depart(&pathfinder, &oracle, &source, SimTime::seconds(100));
        assert!(service.can_admit_retrieve(ZoneId(1)), "a real future departure makes the unknown end finite");
        let proposed = match poll_once(source_request.as_mut()) {
            Poll::Ready(Ok(proposed)) => proposed,
            _ => panic!("oracle commit must wake source selection without inventory or relay mutation"),
        };
        assert!(proposed.admissible());
        assert_eq!(
            service.navigation.pickup_units.count(ZoneId(1), |_| true),
            0,
            "normal source registration prunes the completed physical credit"
        );
        drop(proposed);

        // A later visit by the same robot has a different hold identity; it
        // cannot resurrect the old pickup credit.
        let from = onward.cell();
        let config = CollisionConfig {
            self_robot: source.robot(),
            predecessor: Some(onward),
            destination: None,
            sim_start: SimTime::seconds(110),
            dest_dwell: None,
            collision_depth: 8,
            window_size: 0,
            replan_interval: 0,
            max_wait: SimDuration::seconds(100),
            max_iterations: 10_000,
            motion_model: MotionModel::ConstantSpeed,
        };
        let returning = at_once(pathfinder.reserve_path(
            from,
            Orientation::West,
            source.cell(),
            drives[1].as_ref(),
            oracle.as_ref(),
            &config,
            &mut OneWindow(&EventJournal::new(None)),
        ))
        .unwrap();
        let replacement = returning.committed_reservations.last().unwrap();
        assert_ne!(replacement.id(), source.id());
        assert!(!service.pickup_hold_is_open(&source));
        assert!(service.can_admit_retrieve(ZoneId(1)));
        for slot in slots {
            slot.finalize(SimTime::seconds(120)).unwrap();
        }
    });
}

#[test]
fn speculative_source_leaves_productive_credits_and_promotes_without_recreation() {
    Builder::new_current_thread().build().unwrap().block_on(async {
        let (service, _, _, mut selection) = fixture();
        let mut slots = Vec::new();
        for relay in 1..=2 {
            selection.relay_id = RelayId(relay);
            slots.push(service.reserve_selected_relay_for_retrieve(&selection).unwrap());
        }
        let productive = Arc::new(AtomicBool::new(false));
        let changed = Version::new();
        let policy = RetrieveAdmission::new(
            {
                let productive = productive.clone();
                move || productive.load(Ordering::Relaxed)
            },
            changed.clone(),
        );
        let aim_version = Version::new();
        let request = service.source_retrieve_with_admission(
            CRITERIA,
            CellId(15),
            SimTime::ZERO,
            SimTime::ZERO,
            || None,
            &aim_version,
            owner(0),
            Some(&policy),
        );
        let mut request = Box::pin(request);
        assert!(poll_once(request.as_mut()).is_pending(), "speculative work must leave the last two inbound credits");
        assert!(service.can_admit_retrieve(ZoneId(1)), "productive default retains access to those credits");
        productive.store(true, Ordering::Relaxed);
        changed.bump();
        let mut proposed = match poll_once(request.as_mut()) {
            Poll::Ready(Ok(proposed)) => proposed,
            _ => panic!("acquiring the consumer right must wake this same source request"),
        };
        assert!(proposed.admissible());
        productive.store(false, Ordering::Relaxed);
        changed.bump();
        assert!(!proposed.admissible());
        assert!(proposed.poisoned(), "a regressed quote must not pin the scheduler clock");
        assert!(matches!(proposed.accept(), Err(AcceptError::Poisoned { .. })));
        for slot in slots {
            slot.finalize(SimTime::seconds(120)).unwrap();
        }
    });
}

#[test]
fn a_delivery_via_a_queue_cell_keeps_its_inbound_unit_until_its_window_onto_the_station_is_committed() {
    let (service, oracle, pathfinder, mut selection) = fixture();
    let mut slots = Vec::new();
    for relay in 1..=4 {
        selection.relay_id = RelayId(relay);
        slots.push(service.reserve_selected_relay_for_retrieve(&selection).unwrap());
    }
    let picked = slots.pop().unwrap();
    let access = service.htm_access_cell(picked.relay_id).unwrap();
    let source = oracle.table().reserve_cell(RobotId(60), access, TimeRange::open_ended(SimTime::seconds(10))).unwrap();
    picked.finalize_pickup(SimTime::seconds(12), &source).unwrap();
    assert!(!service.can_admit_retrieve(ZoneId(1)), "the loaded HTM holds the fourth inbound unit");

    // The delivery's first window leaves the relay and ends short of the
    // station, on a queue cell: the unit rides the robot's hold there.
    let queued = depart(&pathfinder, &oracle, &source, SimTime::seconds(100));
    service.navigation.pickup_units.follow(&source, &queued);
    assert!(!service.can_admit_retrieve(ZoneId(1)), "the delivery is short of the station's cell");

    // Its last window reaches the station's cell and closes that hold.
    depart(&pathfinder, &oracle, &queued, queued.window().start());
    assert!(service.can_admit_retrieve(ZoneId(1)), "the delivery's path reached the station's cell");
    for slot in slots {
        slot.finalize(SimTime::seconds(120)).unwrap();
    }
}

#[test]
fn a_robot_s_unit_from_a_pickup_in_a_second_zone_follows_its_windows_past_its_unpruned_unit_in_the_first() {
    // Both orders of the two zones: no order of the ledger may let the
    // earlier, closed unit stand in for the later one.
    for (earlier_zone, later_zone) in [(ZoneId(1), ZoneId(2)), (ZoneId(2), ZoneId(1))] {
        let (service, oracle, pathfinder, _) = fixture();
        let units = &service.navigation.pickup_units;
        let access = service.htm_access_cell(RelayId(1)).unwrap();
        let earlier =
            oracle.table().reserve_cell(RobotId(60), access, TimeRange::open_ended(SimTime::seconds(10))).unwrap();
        units.record(earlier_zone, earlier.clone());

        // The robot delivers that tote, closing the hold; no relay operation in
        // its zone prunes the unit. Its next pickup is in the other zone.
        let later = depart(&pathfinder, &oracle, &earlier, SimTime::seconds(100));
        units.record(later_zone, later.clone());
        assert_eq!(units.count(earlier_zone, |_| true), 0, "the later pickup replaces the robot's earlier unit");

        // That pickup's first window ends on a queue cell.
        let queued = depart(&pathfinder, &oracle, &later, SimTime::seconds(200));
        units.follow(&later, &queued);
        assert_eq!(
            units.count(later_zone, |hold| service.pickup_hold_is_open(hold)),
            1,
            "the later zone counts the unit on the queue cell's hold"
        );
    }
}

#[test]
fn a_relay_that_serves_a_second_pickup_while_the_first_unit_rides_a_queue_cell_counts_both_units() {
    let (service, oracle, pathfinder, mut selection) = fixture();
    let mut slots = Vec::new();
    for relay in 1..=4 {
        selection.relay_id = RelayId(relay);
        slots.push(service.reserve_selected_relay_for_retrieve(&selection).unwrap());
    }
    let picked = slots.pop().unwrap();
    let access = service.htm_access_cell(picked.relay_id).unwrap();
    let first = oracle.table().reserve_cell(RobotId(60), access, TimeRange::open_ended(SimTime::seconds(10))).unwrap();
    picked.finalize_pickup(SimTime::seconds(12), &first).unwrap();

    // The first HTM's window leaves the relay for a queue cell, its unit
    // riding the hold there; a slot is finalized and the relay serves again.
    let queued = depart(&pathfinder, &oracle, &first, SimTime::seconds(100));
    service.navigation.pickup_units.follow(&first, &queued);
    slots.remove(0).finalize(SimTime::seconds(120)).unwrap();
    assert!(service.can_admit_retrieve(ZoneId(1)), "one credit is free");
    selection.start = SimTime::seconds(150);
    let again = service.reserve_selected_relay_for_retrieve(&selection).unwrap();
    let second =
        oracle.table().reserve_cell(RobotId(61), access, TimeRange::open_ended(SimTime::seconds(200))).unwrap();
    again.finalize_pickup(SimTime::seconds(202), &second).unwrap();
    assert!(
        !service.can_admit_retrieve(ZoneId(1)),
        "two slots and two loaded HTMs' units, the first on its queue cell, fill the zone"
    );
    for slot in slots {
        slot.finalize(SimTime::seconds(300)).unwrap();
    }
}
