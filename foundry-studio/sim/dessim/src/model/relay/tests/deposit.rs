//! Tests for deposit, finalize, headroom, schedule pressure, and which
//! RACKING a deposit is allowed to land on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use desgraph::{CellId, RelayId, ZoneId};

use crate::error::ReservationError;
use crate::model::RobotId;
use crate::model::SimTime;
use crate::model::relay::*;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::reservation::oracle::{CellStatus, ReservationOracle, SafeIntervalId, TableOracle};
use crate::model::reservation::table::ReservationConflict;
use crate::pathfinding::{CommitPlan, ReservationRequest};

use super::helpers::*;
use crate::model::test_model::block;
use pathfinder::ReservationOracle as SearchOracle;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// -- the load-spreading probe (putaway.random_slots) --

/// Deposit relays chosen over a run, with the probe seeded at `seed`.
fn probe_choices(seed: u64) -> Vec<RelayId> {
    let svc = two_zone_service();
    svc.set_random_deposit_slots(seed);
    (0..8)
        .filter_map(|i| {
            let r = block(svc.reserve_relay_for_deposit(
                CellId(12),
                Default::default(),
                SimTime::ms(i * 1_000),
                RobotId(0),
                None,
            ))
            .ok()?;
            let relay = r.relay_id;
            r.finalize(SimTime::ms(i * 1_000 + 500)).expect("release the slot for the next draw");
            Some(relay)
        })
        .collect()
}

/// The whole point of a seeded probe: same seed, same run. Seed 0 is the
/// configured default, so it is the one that has to hold.
#[test]
fn the_probe_replays_its_choices_for_a_seed() {
    assert_eq!(probe_choices(0), probe_choices(0), "seed 0 must replay exactly");
    assert_eq!(probe_choices(7), probe_choices(7));
}

/// It has to actually spread, or it measures nothing: the ranked selector
/// sends every deposit from x=12 to the same nearby relay, and the probe
/// must not.
#[test]
fn the_probe_spreads_deposits_the_ranked_selector_concentrates() {
    let svc = two_zone_service();
    let ranked: Vec<RelayId> = (0..8)
        .filter_map(|i| {
            let r = block(svc.reserve_relay_for_deposit(
                CellId(12),
                Default::default(),
                SimTime::ms(i * 1_000),
                RobotId(0),
                None,
            ))
            .ok()?;
            let relay = r.relay_id;
            r.finalize(SimTime::ms(i * 1_000 + 500)).expect("release");
            Some(relay)
        })
        .collect();
    let distinct = |choices: &[RelayId]| {
        let mut ids: Vec<u32> = choices.iter().map(|r| r.0).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    };
    assert_eq!(distinct(&ranked), 1, "the ranked selector concentrates on one relay");
    assert!(distinct(&probe_choices(0)) > 1, "the probe must reach more than one relay");
}

// -- deposit + finalize lifecycle --

#[test]
fn concurrent_buffer_deposits_hold_distinct_rack_slots() -> TestResult {
    let (svc, _) = wired_service_with_uses(&[(1, RackUse::OrderBuffer)]);
    let first =
        block(svc.reserve_buffer_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(1), None))?;
    let second =
        block(svc.reserve_buffer_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(2), None))?;
    assert_ne!(first.relay_id, second.relay_id);
    assert_ne!(first.recommendation, second.recommendation, "different relays must not claim the same rack slot");
    first.cancel();
    second.cancel();
    Ok(())
}

#[test]
fn deposit_claim_lasts_until_storage_and_abandoned_claims_release_capacity() -> TestResult {
    let (svc, inventory) = wired_service_with_uses(&[(1, RackUse::Inventory)]);
    let reserve =
        || block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(2), None));
    let abandoned = reserve()?;
    let slot = abandoned.recommendation;
    assert!(matches!(reserve(), Err(ReservationError::NoCandidates { .. })));
    abandoned.cancel();
    let committed = reserve()?;
    assert_eq!(committed.recommendation, slot);
    inventory
        .tote(ToteId(9001))?
        .move_to(ToteLocation::InRack { rack_id: slot.rack_id, z_index: slot.level }, SimTime::seconds(30))?;
    committed.finalize(SimTime::seconds(10))?;
    assert!(matches!(reserve(), Err(ReservationError::NoCandidates { .. })), "storage must outlive the relay hold");
    Ok(())
}

#[test]
fn deposit_waits_for_the_previous_occupant_to_leave_its_rack_slot() -> TestResult {
    let (svc, inventory) = wired_service_with_uses(&[(1, RackUse::Inventory)]);
    let tote = inventory.tote(ToteId(9001))?;
    let rack = ToteLocation::InRack { rack_id: RackId(1), z_index: 0 };
    tote.move_to(rack, SimTime::seconds(10))?;
    tote.move_to(ToteLocation::Reserved { rack_id: RackId(1), z_index: 0 }, SimTime::seconds(20))?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(9001) }, SimTime::seconds(90))?;
    let deposit =
        block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::seconds(30), RobotId(2), None))?;
    assert!(deposit.start >= SimTime::seconds(90), "a future vacancy cannot be used before its pickup");
    deposit.cancel();
    Ok(())
}

#[test]
fn deposit_reserves_relay() -> TestResult {
    let svc = two_zone_service();
    let r = block(svc.reserve_relay_for_deposit(CellId(12), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(r.zone_id, ZoneId(1));
    // Zone-1 relays sit at x=10/20/30/40; the loaded HTM is scored from the
    // deposit cell x=12 facing East (the reference M5F, ConstantSpeed). Reaching R1
    // (x=10) means a laden U-turn (East→West = two 90° rotations); at the M5F
    // loaded turn (2.1 s, field-calibrated to empty) that costs 2×222 + 2×2100
    // = 4644 ms, whereas driving straight forward to R2 (x=20, 8 cells, no
    // turn) is 8×222 = 1776 ms — so the nearest-by-time relay is R2. The U-turn
    // rotation cost dominates, so R2 wins across the loaded tuning (it also won
    // pre-calibration at 1.5 m/s / 2.6 s: 6532 vs 5328 ms).
    assert_eq!(r.relay_id, RelayId(2));
    assert_eq!(svc.unfinalized_count(), 1);
    r.finalize(SimTime::ms(1000))?;
    Ok(())
}

#[test]
fn deposit_near_zone2_picks_zone2() -> TestResult {
    let svc = two_zone_service();
    let r = block(svc.reserve_relay_for_deposit(CellId(55), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(r.zone_id, ZoneId(2));
    r.finalize(SimTime::ms(1000))?;
    Ok(())
}

#[test]
fn finalize_frees_relay() -> TestResult {
    let svc = two_zone_service();
    let r = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(svc.unfinalized_count(), 1);
    r.finalize(SimTime::ms(1000))?;
    assert_eq!(svc.unfinalized_count(), 0);
    Ok(())
}

#[test]
fn finalized_relay_reusable() -> TestResult {
    let svc = two_zone_service();
    let r1 = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    r1.finalize(SimTime::ms(500))?;
    let r2 = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ms(600), RobotId(0), None))?;
    assert_eq!(r2.relay_id, RelayId(1));
    r2.finalize(SimTime::ms(1000))?;
    Ok(())
}

// -- headroom --

#[test]
fn headroom_limits_direction() -> TestResult {
    let svc = test_service(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])]);
    let r1 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r2 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r3 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None));
    assert!(r3.is_err(), "expected rejection when headroom full");
    r1.finalize(SimTime::ms(100))?;
    r2.finalize(SimTime::ms(200))?;
    Ok(())
}

#[test]
fn headroom_allows_opposing_direction() -> TestResult {
    let svc = two_zone_service();
    let r1 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r2 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r = block(svc.reserve_relay_for_deposit(CellId(55), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(r.zone_id, ZoneId(2));
    r1.finalize(SimTime::ms(100))?;
    r2.finalize(SimTime::ms(200))?;
    r.finalize(SimTime::ms(300))?;
    Ok(())
}

#[test]
fn headroom_finalize_frees_direction_slot() -> TestResult {
    let svc = test_service(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])]);
    let r1 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r2 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    r1.finalize(SimTime::ms(500))?;
    let r3 = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ms(600), RobotId(0), None))?;
    assert!(r3.relay_id.0 > 0); // got a valid relay
    r2.finalize(SimTime::ms(700))?;
    r3.finalize(SimTime::ms(800))?;
    Ok(())
}

#[test]
fn small_zone_no_headroom_panic() {
    let svc = test_service(&[(1, &[(1, 10)])]);
    let r = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None));
    assert!(r.is_err());
}

#[test]
fn two_relay_zone_allows_one_per_direction() {
    let svc = test_service(&[(1, &[(1, 10), (2, 20)])]);
    let r = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None));
    assert!(r.is_err());
}

// -- relay selection --

#[test]
fn deposit_skips_occupied_relay() -> TestResult {
    let svc = two_zone_service();
    let r1 = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(r1.relay_id, RelayId(1));
    let r2 = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_ne!(r1.relay_id, r2.relay_id, "second deposit should use different relay");
    r1.finalize(SimTime::ms(100))?;
    r2.finalize(SimTime::ms(200))?;
    Ok(())
}

#[test]
fn deposit_skips_to_other_zone_when_headroom_full() -> TestResult {
    let svc = two_zone_service();
    let r1 = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r2 = block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    let r = block(svc.reserve_relay_for_deposit(CellId(15), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(r.zone_id, ZoneId(2));
    r1.finalize(SimTime::ms(100))?;
    r2.finalize(SimTime::ms(200))?;
    r.finalize(SimTime::ms(300))?;
    Ok(())
}

/// Bug: b3469d59 - deposit relay-slot starts must respect HtmStore's 300s
/// action-ready horizon, not only access-cell availability from the slot start.
#[test]
fn bug_b3469d59_deposit_rejects_slot_start_after_store_horizon() -> TestResult {
    let svc = test_service(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])]);
    let after = SimTime::ZERO;
    let blocked_until = after + DEPOSIT_ACCESS_MAX_WAIT + SimDuration::seconds(1);

    // Finalized prior reservations are not headroom pressure, but they do push
    // every relay slot's timeline frontier beyond HtmStore's path horizon.
    for _ in 0..4 {
        let reservation =
            block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), after, RobotId(0), None))?;
        assert!(
            reservation.start <= blocked_until,
            "test setup should not create a prior reservation after the intended frontier"
        );
        reservation.finalize(blocked_until)?;
    }

    let states = svc.debug_relay_states().0;
    assert!(
        states.iter().all(|(_, _, unfinalized, available_at, _)| !*unfinalized && *available_at == blocked_until),
        "test setup should push every relay slot frontier to {blocked_until:?}: {states:?}"
    );

    match block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), after, RobotId(99), None)) {
        Err(err @ ReservationError::BeyondAccessHorizon { operation: "deposit", opens_at, .. }) => {
            assert_eq!(opens_at, blocked_until, "the first relay's opening");
            let stated = err.to_string();
            assert!(stated.contains(&format!("{DEPOSIT_ACCESS_MAX_WAIT} access horizon")), "{stated}");
            Ok(())
        }
        Err(err) => panic!("expected BeyondAccessHorizon once all relay slots start beyond 300s, got {err:?}"),
        Ok(reservation) => {
            let delay = reservation.start - after;
            assert!(
                delay <= DEPOSIT_ACCESS_MAX_WAIT,
                "deposit reservation for {:?} starts at {:?}, delay {:?} exceeds {:?}",
                reservation.relay_id,
                reservation.start,
                delay,
                DEPOSIT_ACCESS_MAX_WAIT
            );
            reservation.cancel();
            Ok(())
        }
    }
}

struct ReliefBypassOracle {
    block_access: Arc<AtomicBool>,
    blocker: RobotId,
}

impl ReservationOracle for ReliefBypassOracle {}

impl SearchOracle for ReliefBypassOracle {
    fn earliest_available(
        &self,
        _cell: CellId,
        after: SimTime,
        _duration: SimDuration,
        _predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        Some(after)
    }

    fn commit_path(
        &self,
        _robot: RobotId,
        _plan: &CommitPlan,
        _predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        Ok(Vec::new())
    }

    fn entry_intervals(
        &self,
        _cell: CellId,
        after: SimTime,
        horizon: SimTime,
        _predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId> {
        if after > horizon { Vec::new() } else { vec![(after, Some(horizon))] }
    }

    fn entry_times(
        &self,
        _cell: CellId,
        after: SimTime,
        _horizon: SimTime,
        _predecessor: Option<&CellReservation>,
    ) -> Vec<SimTime> {
        vec![after]
    }

    fn cell_status(&self, _cell: CellId) -> CellStatus {
        if self.block_access.load(Ordering::SeqCst) {
            CellStatus::PartiallyPlanned { robot: self.blocker }
        } else {
            CellStatus::Schedulable
        }
    }
}

/// Bug: a793ab22 - parking relief must not revive a blocked deposit candidate
/// whose relay slot starts outside HtmStore's 300s action-ready horizon.
#[test]
fn bug_a793ab22_deposit_relief_rejects_blocked_slot_after_store_horizon() -> TestResult {
    let block_access = Arc::new(AtomicBool::new(false));
    let oracle = Arc::new(ReliefBypassOracle { block_access: block_access.clone(), blocker: RobotId(7) });
    let svc = test_service_with_oracle(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])], oracle);
    let after = SimTime::ZERO;
    let blocked_until = after + DEPOSIT_ACCESS_MAX_WAIT + SimDuration::seconds(1);

    for _ in 0..4 {
        let reservation =
            block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), after, RobotId(0), None))?;
        reservation.finalize(blocked_until)?;
    }
    block_access.store(true, Ordering::SeqCst);

    let park_calls = AtomicUsize::new(0);
    let relief = ParkingRelief::new({
        let park_calls = &park_calls;
        move |_| {
            Box::pin(async move {
                park_calls.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
    });

    match block(svc.reserve_relay_for_deposit(CellId(10), Default::default(), after, RobotId(99), Some(&relief))) {
        Err(ReservationError::BeyondAccessHorizon { operation: "deposit", opens_at, .. }) => {
            assert_eq!(opens_at, blocked_until, "the first relay's opening");
            assert_eq!(park_calls.load(Ordering::SeqCst), 0, "relief must not run for over-horizon starts");
            Ok(())
        }
        Err(err) => panic!("expected BeyondAccessHorizon for over-horizon blocked relief candidate, got {err:?}"),
        Ok(reservation) => {
            let delay = reservation.start - after;
            reservation.cancel();
            panic!("relief committed a deposit after {:?}, beyond {:?}", delay, DEPOSIT_ACCESS_MAX_WAIT);
        }
    }
}

/// Open-ended access holds produce no entry times. Their blockers must still
/// reach relief; screening by entry time first would discard every candidate.
#[test]
fn deposit_requests_relief_when_every_access_cell_has_an_open_hold() -> TestResult {
    let oracle = Arc::new(TableOracle::default());
    let relays = [(1, 10), (2, 20), (3, 30), (4, 40)];
    let svc = test_service_with_oracle(&[(1, &relays)], oracle.clone());
    for &(id, cell) in &relays {
        oracle
            .commit_path(
                RobotId(id),
                &CommitPlan::single(ReservationRequest::spanning(CellId(cell), SimTime::ZERO, None)),
                None,
            )
            .unwrap();
        assert!(oracle.entry_times(CellId(cell), SimTime::ZERO, SimTime::seconds(300), None).is_empty());
    }
    let park_calls = AtomicUsize::new(0);
    let relief = ParkingRelief::new({
        let (park_calls, oracle) = (&park_calls, &oracle);
        move |target| {
            Box::pin(async move {
                park_calls.fetch_add(1, Ordering::SeqCst);
                let predecessor = oracle.reservations_for_cell(target.cell).pop().unwrap();
                assert_eq!(predecessor.robot(), target.robot);
                Ok(true)
            })
        }
    });
    let reservation = block(svc.reserve_relay_for_deposit(
        CellId(12),
        Default::default(),
        SimTime::ZERO,
        RobotId(99),
        Some(&relief),
    ))?;
    assert_eq!(park_calls.load(Ordering::SeqCst), 1);
    reservation.cancel();
    assert_eq!(svc.unfinalized_count(), 0);
    Ok(())
}

/// A caller that passes no relief moves a squatter itself (a clearance's
/// return runs `Parking`): when robots hold every candidate's access cell
/// open-ended, the decline names each held cell with its holder, earliest
/// relay first, and reserves nothing.
#[test]
fn a_deposit_without_relief_declines_naming_every_held_access_cell() -> TestResult {
    let oracle = Arc::new(TableOracle::default());
    let relays = [(1, 10), (2, 20), (3, 30), (4, 40)];
    let svc = test_service_with_oracle(&[(1, &relays)], oracle.clone());
    for &(id, cell) in &relays {
        oracle
            .commit_path(
                RobotId(id),
                &CommitPlan::single(ReservationRequest::spanning(CellId(cell), SimTime::ZERO, None)),
                None,
            )
            .unwrap();
    }
    match block(svc.reserve_relay_for_deposit(CellId(12), Default::default(), SimTime::ZERO, RobotId(99), None)) {
        Err(ReservationError::AccessHeld { operation: "deposit", held, earliest_admitted_departure: None }) => {
            assert_eq!(held, relays.map(|(id, cell)| (CellId(cell), RobotId(id))));
        }
        other => panic!("expected the held access cells, got {other:?}"),
    }
    assert_eq!(svc.unfinalized_count(), 0, "a decline reserves nothing");
    Ok(())
}

// -- racking: a deposit lands on steel that is for its own pool --

/// The row of empty order totes stands INSIDE the storage aisles, so no zone
/// filter can keep stock out of it. The slot frontier does: a deposit names
/// the racking it means and is only ever offered slots of that kind.
///
/// Without this, one round trip is all it takes to erase the row -- an order
/// tote comes back from extraction, lands in the nearest stock slot, and the
/// place close-out looks for empty order totes is somewhere else now.
#[test]
fn a_deposit_only_ever_lands_on_racking_of_its_own_kind() -> TestResult {
    use crate::scenario_gen::RackUse;

    // Racks 1 and 2 are stock, rack 3 is the order-tote row, rack 4 is
    // buffer -- all four in one zone, which is the whole point.
    let (svc, _inv) = wired_service_with_uses(&[
        (1, RackUse::Inventory),
        (2, RackUse::Inventory),
        (3, RackUse::OrderDepot),
        (4, RackUse::OrderBuffer),
    ]);
    let depot_slot = RackSlot { rack_id: RackId(3), level: 0 };

    // Stock goes to stock racking, whichever slot is nearest.
    for _ in 0..8 {
        let r = block(svc.reserve_relay_for_deposit(CellId(12), Default::default(), SimTime::ZERO, RobotId(0), None))?;
        assert_ne!(r.recommendation, depot_slot, "stock landed in the order-tote row");
        assert_ne!(r.recommendation.rack_id, RackId(4), "stock landed in the order buffer");
        r.finalize(SimTime::ms(1000))?;
    }

    // And an emptied order tote goes home to the row, which is the only
    // depot slot there is.
    let r =
        block(svc.reserve_depot_relay_for_deposit(CellId(12), Default::default(), SimTime::ZERO, RobotId(0), None))?;
    assert_eq!(r.recommendation, depot_slot, "an order tote went somewhere other than the row");
    r.finalize(SimTime::ms(1000))?;
    Ok(())
}

/// With no order racking at all -- every map before this row existed -- an
/// order-tote deposit has nowhere to go, and says so rather than falling back
/// on the nearest stock slot.
#[test]
fn a_depot_deposit_declines_when_the_map_states_no_row() -> TestResult {
    use crate::scenario_gen::RackUse;

    let (svc, _inv) = wired_service_with_uses(&[(1, RackUse::Inventory), (2, RackUse::Inventory)]);
    let r = block(svc.reserve_depot_relay_for_deposit(CellId(12), Default::default(), SimTime::ZERO, RobotId(0), None));
    assert!(matches!(r, Err(ReservationError::NoCandidates { .. })), "got {r:?}");
    Ok(())
}
