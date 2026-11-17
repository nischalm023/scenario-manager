//! Tests for retrieve (empty + SKU) via RelayService.

use desgraph::{CellId, ZoneId};

use crate::model::ToteDesignation;
use crate::model::ids::{RobotId, ToteId};
use crate::model::location::{ToteLocation, ToteType};
use crate::model::relay::*;
use crate::model::{RackId, SimTime, SkuId};

use super::helpers::*;

const fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ---------------------------------------------------------------------------
// Empty tote retrieval
// ---------------------------------------------------------------------------

#[test]
fn retrieve_empty_finds_tote() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(15),
        SimTime::ZERO,
    )?;
    assert_eq!(r.recommendation, ToteId(1));
    assert_eq!(r.zone_id, ZoneId(1));
    r.finalize(SimTime::ms(100))?;
    Ok(())
}

#[test]
fn retrieve_empty_ignores_non_empty_tote() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.tote(ToteId(1))?.add_item(SkuId(10), 1, 0)?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(15),
        SimTime::ZERO,
    );
    assert!(r.is_err(), "tote with SKU should not appear in empty retrieve");
    Ok(())
}

#[test]
fn retrieve_empty_ignores_tote_moved_out() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.tote(ToteId(1))?.move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, SimTime::ms(500))?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(15),
        SimTime::ZERO,
    );
    assert!(r.is_err(), "tote moved out of rack should not appear");
    Ok(())
}

#[test]
fn retrieve_empty_finds_returned_tote() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    // Dispatch and return.
    inv.tote(ToteId(1))?.move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, SimTime::ms(500))?;
    inv.tote(ToteId(1))?.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 0 }, SimTime::ms(2000))?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(25),
        SimTime::ZERO,
    )?;
    assert_eq!(r.recommendation, ToteId(1));
    r.finalize(SimTime::ms(100))?;
    Ok(())
}

#[test]
fn retrieve_empty_prefers_lower_z_index() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 5 },
    )?;
    inv.register_tote(
        ToteId(2),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    assert_eq!(r.recommendation, ToteId(2), "lower z-index should be preferred");
    r.finalize(SimTime::ms(100))?;
    Ok(())
}

#[test]
fn retrieve_empty_prefers_closer_rack() -> TestResult {
    let (svc, inv) = wired_service();
    // Rack 1 at x=12, rack 4 at x=42. Relay 1 at x=10.
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.register_tote(
        ToteId(2),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(4), z_index: 0 },
    )?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    assert_eq!(r.recommendation, ToteId(1), "closer rack should be preferred");
    r.finalize(SimTime::ms(100))?;
    Ok(())
}

#[test]
fn retrieve_empty_returns_err_when_no_totes() -> TestResult {
    let (svc, _inv) = wired_service();
    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    );
    assert!(r.is_err());
    Ok(())
}

#[test]
fn retrieve_empty_reserves_relay() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.register_tote(
        ToteId(2),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(2), z_index: 0 },
    )?;

    let r1 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    assert_eq!(svc.unfinalized_count(), 1);

    let r2 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    assert_ne!(r1.relay_id, r2.relay_id, "second retrieve should use different relay");
    assert_eq!(svc.unfinalized_count(), 2);
    r1.finalize(SimTime::ms(100))?;
    r2.finalize(SimTime::ms(200))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// SKU retrieval
// ---------------------------------------------------------------------------

#[test]
fn retrieve_sku_finds_tote() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.tote(ToteId(1))?.add_item(SkuId(10), 3, 0)?;

    let r =
        svc.reserve_relay_for_retrieve(ToteSearchCriteria::ContainsSku { sku: SkuId(10) }, CellId(15), SimTime::ZERO)?;
    assert_eq!(r.recommendation, ToteId(1));
    r.finalize(SimTime::ms(100))?;
    Ok(())
}

#[test]
fn retrieve_sku_wrong_sku_not_found() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.tote(ToteId(1))?.add_item(SkuId(10), 1, 0)?;

    let r =
        svc.reserve_relay_for_retrieve(ToteSearchCriteria::ContainsSku { sku: SkuId(99) }, CellId(15), SimTime::ZERO);
    assert!(r.is_err());
    Ok(())
}

#[test]
fn retrieve_sku_depleted_disappears() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.tote(ToteId(1))?.add_item(SkuId(10), 1, 0)?;
    inv.tote(ToteId(1))?.remove_item(SkuId(10), 1, 1000)?;

    let r =
        svc.reserve_relay_for_retrieve(ToteSearchCriteria::ContainsSku { sku: SkuId(10) }, CellId(15), SimTime::ZERO);
    assert!(r.is_err(), "depleted SKU should not match");
    Ok(())
}

#[test]
fn retrieve_sku_multiple_totes_picks_closer() -> TestResult {
    let (svc, inv) = wired_service();
    // Tote 1 at rack 1 (x=12), tote 2 at rack 4 (x=42). Both have SKU 10.
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inv.register_tote(
        ToteId(2),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(4), z_index: 0 },
    )?;
    inv.tote(ToteId(1))?.add_item(SkuId(10), 1, 0)?;
    inv.tote(ToteId(2))?.add_item(SkuId(10), 1, 0)?;

    // Station near relay 1 (x=10). Tote 1 at rack 1 (x=12) is closer.
    let r =
        svc.reserve_relay_for_retrieve(ToteSearchCriteria::ContainsSku { sku: SkuId(10) }, CellId(10), SimTime::ZERO)?;
    assert_eq!(r.recommendation, ToteId(1));
    r.finalize(SimTime::ms(100))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Full lifecycle: putaway → pick
// ---------------------------------------------------------------------------

#[test]
fn full_lifecycle_putaway_then_pick() -> TestResult {
    let (svc, inv) = wired_service();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    // 1. Retrieve empty tote for putaway.
    let r1 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    assert_eq!(r1.recommendation, ToteId(1));

    // 2. Tote dispatched.
    inv.tote(ToteId(1))?.move_to(ToteLocation::WithRobot { robot_id: RobotId(0) }, SimTime::ms(100))?;
    r1.finalize(SimTime::ms(200))?;

    // 3. Putaway fills tote, returns to rack.
    inv.tote(ToteId(1))?.add_item(SkuId(42), 5, 1000)?;
    inv.tote(ToteId(1))?.move_to(ToteLocation::InRack { rack_id: RackId(3), z_index: 2 }, SimTime::ms(2000))?;

    // 4. Should NOT be in empty set.
    inv.register_tote(
        ToteId(2),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(2), z_index: 0 },
    )?;
    let r2 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    assert_eq!(r2.recommendation, ToteId(2), "tote 1 has SKU, should not be in empty set");

    // 5. Should be in SKU set.
    let r3 =
        svc.reserve_relay_for_retrieve(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, CellId(10), SimTime::ZERO)?;
    assert_eq!(r3.recommendation, ToteId(1));

    // 6. Pick consumes SKU — tote re-enters empty set.
    inv.tote(ToteId(1))?.remove_item(SkuId(42), 5, 5000)?;

    // Finalize previous reservations to free relays.
    r2.finalize(SimTime::ms(3000))?;
    r3.finalize(SimTime::ms(4000))?;

    // Tote 1 is at rack 3 (x=32), tote 2 is at rack 2 (x=22).
    // Retrieve from CellId(35) — near relay 3 (x=30), where tote 1 is cheapest.
    let r4 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(35),
        SimTime::ms(5000),
    )?;
    assert_eq!(r4.recommendation, ToteId(1), "tote 1 should be retrievable as empty after SKU consumed");
    r4.finalize(SimTime::ms(6000))?;
    Ok(())
}

#[test]
fn retrieve_headroom_exhausted() -> TestResult {
    let (svc, inv) = wired_service();
    for i in 0..4u32 {
        inv.register_tote(
            ToteId(i),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(1), z_index: i as u16 },
        )?;
    }
    // Headroom for 4-relay zone: N-2 = 2.
    let r1 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;
    let r2 = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    )?;

    let r = svc.reserve_relay_for_retrieve(
        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
        CellId(10),
        SimTime::ZERO,
    );
    assert!(r.is_err(), "should not find relay when headroom exhausted");
    r1.finalize(SimTime::ms(100))?;
    r2.finalize(SimTime::ms(200))?;
    Ok(())
}
