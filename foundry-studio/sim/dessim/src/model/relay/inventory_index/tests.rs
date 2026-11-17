//! Tests for RelayInventoryIndex — verifies event-driven index maintenance
//! and query correctness. Independent of RelayService.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use desgraph::{RelayId, ZoneId};
use desmap::BlockSide;
use rustc_hash::FxHashMap;

use crate::error::ReservationError;
use crate::model::ToteDesignation;
use crate::model::ids::RobotId;
use crate::model::ids::{RackSlot, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::inventory::events::InventoryObserver;
use crate::model::location::{ToteLocation, ToteType};
use crate::model::relay::ToteSearchCriteria;
use crate::model::relay::config::RelayZones;
use crate::model::relay::tests::helpers::build_test_env;
use crate::model::{RackId, SimTime, SkuId};
use crate::robots::vtm::motion::CycleTimes;
use crate::scenario_gen::RackUse;

use super::RelayInventoryIndex;

#[test]
fn buffered_filled_order_is_retrievable_until_reserved_for_early_and_late_indices() -> TestResult {
    let (early, inventory, _early_observer) = test_index();
    let order = ToteId(900);
    let rack = ToteLocation::InRack { rack_id: RackId(1), z_index: 0 };
    inventory.register_tote(order, ToteType::Tote, ToteDesignation::Order, rack)?;
    let tote = inventory.tote(order)?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, t(10))?;
    tote.add_item(SkuId(42), 3, 20)?;
    tote.move_to(rack, t(30))?;

    let zones: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10), (2, 20)])];
    let (data, _, _) = build_test_env(zones, &[(1, 12, 1), (2, 22, 1), (3, 15, 1)]);
    let late = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let _late_observer =
        inventory.add_observer_with_snapshot(Arc::new(late.clone()), |snapshot| late.initialize(snapshot))?;
    for index in [&early, &late] {
        let candidates =
            index.best_tote_per_relay(ToteSearchCriteria::BufferedOrder { tote_id: order }, &HashSet::new())?;
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| candidate.item == order && candidate.available_at == t(30)));
        assert!(
            index
                .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Order }, &HashSet::new())
                .is_err()
        );
        assert!(
            index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new()).is_err()
        );
    }
    tote.move_to(ToteLocation::Reserved { rack_id: RackId(1), z_index: 0 }, t(40))?;
    for index in [&early, &late] {
        assert!(
            index.best_tote_per_relay(ToteSearchCriteria::BufferedOrder { tote_id: order }, &HashSet::new()).is_err()
        );
    }
    Ok(())
}

const fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Build an index wired to an InventoryService via observer.
/// Zone 1: relays 1,2 at x=10,20. Racks 1,2,3 at x=12,22,15 in zone 1.
///
/// Returns the observer Arc as the third tuple element; tests must bind it
/// (typically as `_observer`) so the observer stays alive for the test body
/// — `add_observer` holds it weakly.
fn test_index() -> (RelayInventoryIndex, InventoryService, Arc<dyn InventoryObserver>) {
    let zone_spec: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10), (2, 20)])];
    let rack_spec: &[(u32, u32, u32)] = &[
        (1, 12, 1), // rack 1 at x=12, zone 1 — close to relay 1
        (2, 22, 1), // rack 2 at x=22, zone 1 — close to relay 2
        (3, 15, 1), // rack 3 at x=15, zone 1 — between relays
    ];
    let (data, inventory, _) = build_test_env(zone_spec, rack_spec);
    let index = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let observer = inventory.add_observer(Arc::new(index.clone()));
    (index, inventory, observer)
}

/// Two-zone setup. Zone 1: relay 1 at x=10, rack 1 at x=12.
/// Zone 2: relay 2 at x=50, rack 2 at x=52.
fn two_zone_index() -> (RelayInventoryIndex, InventoryService, Arc<dyn InventoryObserver>) {
    let zone_spec: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10)]), (2, &[(2, 50)])];
    let rack_spec: &[(u32, u32, u32)] = &[(1, 12, 1), (2, 52, 2)];
    let (data, inventory, _) = build_test_env(zone_spec, rack_spec);
    let index = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let observer = inventory.add_observer(Arc::new(index.clone()));
    (index, inventory, observer)
}

// -- Empty tote tests ---------------------------------------------------------

#[test]
fn empty_tote_registered_in_rack_appears_in_index() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert!(!candidates.is_empty(), "should have candidates");
    assert_eq!(candidates.len(), 2);
    for c in &candidates {
        assert_eq!(c.item, ToteId(1));
        assert_eq!(c.relay.zone_id, ZoneId(1));
    }
    Ok(())
}

#[test]
fn empty_tote_closest_relay_has_lower_cost() -> TestResult {
    let (index, inv, _observer) = test_index();
    // Rack 1 at x=12, relay 1 at x=10, relay 2 at x=20.
    // Rack 1 is closer to relay 1.
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    let r1 = candidates.iter().find(|c| c.relay.id == RelayId(1)).unwrap();
    let r2 = candidates.iter().find(|c| c.relay.id == RelayId(2)).unwrap();
    assert!(
        r1.vtm_cost < r2.vtm_cost,
        "relay 1 (x=10) should be cheaper for rack 1 (x=12) than relay 2 (x=20), got {} vs {}",
        r1.vtm_cost,
        r2.vtm_cost
    );
    Ok(())
}

#[test]
fn cheapest_empty_tote_wins_per_relay() -> TestResult {
    let (index, inv, _observer) = test_index();
    // Rack 1 at x=12 (close to relay 1), rack 2 at x=22 (close to relay 2).
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

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    let r1 = candidates.iter().find(|c| c.relay.id == RelayId(1)).unwrap();
    let r2 = candidates.iter().find(|c| c.relay.id == RelayId(2)).unwrap();
    // Relay 1's cheapest should be tote 1 (rack 1 at x=12, relay at x=10).
    assert_eq!(r1.item, ToteId(1));
    // Relay 2's cheapest should be tote 2 (rack 2 at x=22, relay at x=20).
    assert_eq!(r2.item, ToteId(2));
    Ok(())
}

#[test]
fn no_empty_totes_returns_no_candidates() -> TestResult {
    let (index, _inv, _observer) = test_index();
    assert!(matches!(
        index.best_tote_per_relay(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            &HashSet::new()
        ),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

#[test]
fn tote_leaving_rack_removed_from_empty_index() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    assert!(
        !index
            .best_tote_per_relay(
                ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
                &HashSet::new()
            )?
            .is_empty()
    );

    // Move tote out of rack.
    let tote = inv.tote(ToteId(1))?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(100))?;

    assert!(matches!(
        index.best_tote_per_relay(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            &HashSet::new()
        ),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

#[test]
fn tote_gaining_content_removed_from_empty_index() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    assert!(
        !index
            .best_tote_per_relay(
                ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
                &HashSet::new()
            )?
            .is_empty()
    );

    let tote = inv.tote(ToteId(1))?;
    tote.add_item(SkuId(42), 1, 100)?;

    assert!(matches!(
        index.best_tote_per_relay(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            &HashSet::new()
        ),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

// -- SKU tote tests -----------------------------------------------------------

#[test]
fn sku_tote_appears_after_add_item() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;
    tote.add_item(SkuId(42), 5, 100)?;

    let candidates = index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new())?;
    assert_eq!(candidates.len(), 2); // both relays in zone 1
    for c in &candidates {
        assert_eq!(c.item, ToteId(1));
    }

    // Different SKU returns nothing.
    assert!(matches!(
        index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(99) }, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

#[test]
fn sku_tote_removed_when_qty_reaches_zero() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;
    tote.add_item(SkuId(42), 1, 100)?;
    assert!(
        !index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new())?.is_empty()
    );

    tote.remove_item(SkuId(42), 1, 200)?;
    assert!(matches!(
        index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

#[test]
fn sku_tote_removed_when_leaving_rack() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;
    tote.add_item(SkuId(42), 5, 100)?;
    assert!(
        !index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new())?.is_empty()
    );

    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(200))?;
    assert!(matches!(
        index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

#[test]
fn tote_returning_to_rack_with_skus_appears_in_sku_index() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;
    tote.add_item(SkuId(42), 5, 100)?;

    // Leave rack.
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(200))?;
    assert!(matches!(
        index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));

    // Return to rack (possibly different slot).
    tote.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 3 }, t(300))?;
    let candidates = index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(42) }, &HashSet::new())?;
    assert_eq!(candidates.len(), 2);
    for c in &candidates {
        assert_eq!(c.item, ToteId(1));
    }
    Ok(())
}

// -- Empty slot tests ---------------------------------------------------------

#[test]
fn empty_slots_available_when_no_totes() -> TestResult {
    let (index, _inv, _observer) = test_index();
    // No totes registered → no slots occupied → no slots tracked.
    // Slots only enter the index when a tote departs them.
    assert!(matches!(
        index.best_empty_slot_per_relay(RackUse::Inventory, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

#[test]
fn slot_becomes_empty_when_tote_departs() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    // Slot is occupied — should NOT appear in empty slots.
    assert!(matches!(
        index.best_empty_slot_per_relay(RackUse::Inventory, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));

    // Tote departs.
    let tote = inv.tote(ToteId(1))?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(100))?;

    // Now slot (rack 1, level 0) should appear.
    let candidates = index.best_empty_slot_per_relay(RackUse::Inventory, &HashSet::new())?;
    assert!(!candidates.is_empty());
    let slot = RackSlot { rack_id: RackId(1), level: 0 };
    for c in &candidates {
        assert_eq!(c.item, slot);
        assert_eq!(c.relay.zone_id, ZoneId(1));
        assert_eq!(c.available_at, t(100));
    }
    Ok(())
}

#[test]
fn slot_occupied_again_when_tote_arrives() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;

    // Depart → slot empty.
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(100))?;
    assert!(!index.best_empty_slot_per_relay(RackUse::Inventory, &HashSet::new())?.is_empty());

    // Arrive back → slot occupied.
    tote.move_to(ToteLocation::InRack { rack_id: RackId(1), z_index: 0 }, t(200))?;
    assert!(matches!(
        index.best_empty_slot_per_relay(RackUse::Inventory, &HashSet::new()),
        Err(ReservationError::NoCandidates { .. })
    ));
    Ok(())
}

// -- Two-zone tests -----------------------------------------------------------

#[test]
fn totes_isolated_by_zone() -> TestResult {
    let (index, inv, _observer) = two_zone_index();
    // Tote in zone 1's rack.
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    // Only relay 1 (zone 1) should see it. Relay 2 (zone 2) should not.
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].relay.id, RelayId(1));
    assert_eq!(candidates[0].relay.zone_id, ZoneId(1));
    Ok(())
}

#[test]
fn each_zone_has_own_candidates() -> TestResult {
    let (index, inv, _observer) = two_zone_index();
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

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert_eq!(candidates.len(), 2);
    let z1 = candidates.iter().find(|c| c.relay.zone_id == ZoneId(1)).unwrap();
    let z2 = candidates.iter().find(|c| c.relay.zone_id == ZoneId(2)).unwrap();
    assert_eq!(z1.item, ToteId(1));
    assert_eq!(z2.item, ToteId(2));
    Ok(())
}

// -- Side-bound pairing (dual-sided zone) -------------------------------------

/// One zone with a Left relay+rack and a Right relay+rack. Models the
/// production Sidedness::Dual geometry: relays on each side of an aisle
/// only serve racks on their own side.
fn dual_side_index() -> (RelayInventoryIndex, InventoryService, Arc<dyn InventoryObserver>) {
    // relay 1 (Left, x=10), relay 2 (Right, x=20). rack 1 (Left, x=10), rack 2 (Right, x=20).
    // Test fixture allows rack and relay to share grid_x (same column, different cells
    // would be the production geometry; here they coincide for simplicity).
    let zone_spec: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10), (2, 20)])];
    let rack_spec: &[(u32, u32, u32)] = &[(1, 11, 1), (2, 21, 1)];
    let (mut data, inventory, _) = build_test_env(zone_spec, rack_spec);

    // Override sides: relay 2 and rack 2 are Right; the rest stay Left.
    for r in data.relay_data.iter_mut() {
        if r.relay_id == RelayId(2) {
            r.side = BlockSide::Right;
        }
    }
    data.rack_side.insert(RackId(2), BlockSide::Right);

    // Rebuild RelayZones with the new sides.
    data.relay_zones =
        RelayZones::from_scenario_legacy(&data, CycleTimes::default()).expect("dual-side scenario valid");

    let index = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let observer = inventory.add_observer(Arc::new(index.clone()));
    (index, inventory, observer)
}

#[test]
fn left_tote_only_appears_in_left_relay_index() -> TestResult {
    let (index, inv, _observer) = dual_side_index();
    // Tote on the Left rack — visible only to the Left relay.
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert_eq!(candidates.len(), 1, "only the Left relay should index the Left tote");
    assert_eq!(candidates[0].relay.id, RelayId(1));
    assert_eq!(candidates[0].relay.side, BlockSide::Left);
    Ok(())
}

#[test]
fn right_tote_only_appears_in_right_relay_index() -> TestResult {
    let (index, inv, _observer) = dual_side_index();
    inv.register_tote(
        ToteId(2),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(2), z_index: 0 },
    )?;

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert_eq!(candidates.len(), 1, "only the Right relay should index the Right tote");
    assert_eq!(candidates[0].relay.id, RelayId(2));
    assert_eq!(candidates[0].relay.side, BlockSide::Right);
    Ok(())
}

#[test]
fn each_side_indexes_its_own_totes_independently() -> TestResult {
    let (index, inv, _observer) = dual_side_index();
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

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert_eq!(candidates.len(), 2);
    let left = candidates.iter().find(|c| c.relay.side == BlockSide::Left).expect("left candidate");
    let right = candidates.iter().find(|c| c.relay.side == BlockSide::Right).expect("right candidate");
    assert_eq!(left.item, ToteId(1));
    assert_eq!(right.item, ToteId(2));
    Ok(())
}

#[test]
fn empty_slots_partition_by_side() -> TestResult {
    let (index, inv, _observer) = dual_side_index();
    // Put totes in both slots so the slots become tracked.
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
    // Depart both → both slots are empty.
    inv.tote(ToteId(1))?.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(100))?;
    inv.tote(ToteId(2))?.move_to(ToteLocation::WithRobot { robot_id: RobotId(2) }, t(100))?;

    let candidates = index.best_empty_slot_per_relay(RackUse::Inventory, &HashSet::new())?;
    assert_eq!(candidates.len(), 2);
    let left = candidates.iter().find(|c| c.relay.side == BlockSide::Left).expect("left slot");
    let right = candidates.iter().find(|c| c.relay.side == BlockSide::Right).expect("right slot");
    assert_eq!(left.item.rack_id, RackId(1), "Left relay sees only its Left slot");
    assert_eq!(right.item.rack_id, RackId(2), "Right relay sees only its Right slot");
    Ok(())
}

#[test]
fn tote_moved_across_aisle_switches_indexed_side() -> TestResult {
    let (index, inv, _observer) = dual_side_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    // Initially indexed under the Left relay.
    {
        let candidates = index.best_tote_per_relay(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            &HashSet::new(),
        )?;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].relay.side, BlockSide::Left);
    }

    // Relocate to the Right rack — index re-keys to the Right relay only.
    let tote = inv.tote(ToteId(1))?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(100))?;
    tote.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 0 }, t(200))?;

    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert_eq!(candidates.len(), 1, "tote should now appear on exactly one (Right) relay");
    assert_eq!(candidates[0].relay.side, BlockSide::Right);
    assert_eq!(candidates[0].relay.id, RelayId(2));
    Ok(())
}

// -- Tote moved to different rack then emptied --------------------------------

#[test]
fn tote_moved_to_different_rack_then_emptied_re_enters_empty_index() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;

    // Add content, move out, move to different rack.
    tote.add_item(SkuId(42), 5, 100)?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(200))?;
    tote.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 3 }, t(300))?;

    // Tote has content at rack 2 — not in empty index.
    assert!(matches!(
        index.best_tote_per_relay(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            &HashSet::new()
        ),
        Err(ReservationError::NoCandidates { .. })
    ));

    // Remove all content — tote should re-enter empty index.
    tote.remove_item(SkuId(42), 5, 5000)?;
    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert!(!candidates.is_empty(), "tote should be in empty index after content removed at new rack");
    assert_eq!(candidates[0].item, ToteId(1));
    Ok(())
}

// -- Empty tote transition (content emptied while in rack) --------------------

#[test]
fn tote_becomes_empty_in_rack_enters_empty_index() -> TestResult {
    let (index, inv, _observer) = test_index();
    inv.register_tote(
        ToteId(1),
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    let tote = inv.tote(ToteId(1))?;
    tote.add_item(SkuId(42), 1, 100)?;

    // Tote has content — not in empty index.
    assert!(matches!(
        index.best_tote_per_relay(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            &HashSet::new()
        ),
        Err(ReservationError::NoCandidates { .. })
    ));

    // Remove last item — tote becomes empty while in rack.
    tote.remove_item(SkuId(42), 1, 200)?;

    // Should now appear in empty index.
    let candidates = index
        .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory }, &HashSet::new())?;
    assert_eq!(candidates.len(), 2);
    for c in &candidates {
        assert_eq!(c.item, ToteId(1));
    }
    Ok(())
}

#[test]
fn late_index_reconstructs_same_time_moves_and_out_of_order_frontiers() -> TestResult {
    let (early, inventory, _early_observer) = test_index();
    let register = |id, rack| {
        inventory.register_tote(
            ToteId(id),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(rack), z_index: 0 },
        )
    };
    register(1, 1)?;
    let first = inventory.tote(ToteId(1))?;
    first.add_item(SkuId(7), 5, 0)?;
    first.move_to(ToteLocation::WithRobot { robot_id: RobotId(101) }, t(0))?;
    register(2, 2)?;
    let second = inventory.tote(ToteId(2))?;
    second.add_item(SkuId(7), 5, 0)?;
    second.move_to(ToteLocation::WithRobot { robot_id: RobotId(102) }, t(20))?;
    second.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 0 }, t(40))?;
    second.move_to(ToteLocation::Reserved { rack_id: RackId(2), z_index: 0 }, t(35))?;
    register(3, 3)?;
    let third = inventory.tote(ToteId(3))?;
    third.add_item(SkuId(7), 1, 10)?;
    third.remove_item(SkuId(7), 1, 10)?;
    third.add_item(SkuId(8), 2, 50)?;
    third.add_item(SkuId(7), 1, 5)?;

    let zone_spec: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10), (2, 20)])];
    let (data, _, _) = build_test_env(zone_spec, &[(1, 12, 1), (2, 22, 1), (3, 15, 1)]);
    let late = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let _late_observer =
        inventory.add_observer_with_snapshot(Arc::new(late.clone()), |snapshot| late.initialize(snapshot))?;

    let frontier = |index: &RelayInventoryIndex| {
        let state = index.0.state.lock().unwrap();
        let mut snapshot = Vec::new();
        for (&relay, row) in &state.relays {
            for tote in [ToteId(1), ToteId(2), ToteId(3)] {
                snapshot.push((
                    relay,
                    format!("empty-{tote:?}"),
                    row.empty_pool(ToteDesignation::Inventory).get(&tote),
                ));
                for sku in [SkuId(7), SkuId(8)] {
                    snapshot.push((
                        relay,
                        format!("{sku:?}-{tote:?}"),
                        state
                            .sku_totes
                            .get(&(row.relay.zone_id, row.relay.side, sku))
                            .and_then(|set| set.get(&tote))
                            .map(|(slot, at)| {
                                (index.0.zone_configs[&row.relay.zone_id].min_forward_cycle_cost(relay, slot), at)
                            }),
                    ));
                }
            }
            for rack_id in [RackId(1), RackId(2), RackId(3)] {
                snapshot.push((
                    relay,
                    format!("slot-{rack_id:?}"),
                    row.slots(RackUse::Inventory).get(&RackSlot { rack_id, level: 0 }),
                ));
            }
        }
        snapshot.sort_by_key(|(relay, name, _)| (*relay, name.clone()));
        snapshot
    };
    assert_eq!(frontier(&early), frontier(&late));
    assert!(frontier(&late).iter().any(|(_, name, value)| name == "slot-RackId(1)" && value.is_some()));
    assert_eq!(first.location_at(t(0)), ToteLocation::WithRobot { robot_id: RobotId(101) });
    assert_eq!(second.location_at(t(40)), ToteLocation::InRack { rack_id: RackId(2), z_index: 0 });

    // Both indices must retain removal tombstones, so a subsequent stale
    // addition cannot resurrect an already departed tote.
    second.move_to(ToteLocation::WithRobot { robot_id: RobotId(102) }, t(80))?;
    second.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 0 }, t(60))?;
    assert_eq!(frontier(&early), frontier(&late));
    assert!(
        frontier(&late)
            .iter()
            .any(|(_, name, value)| name == "slot-RackId(2)" && value.is_some_and(|(_, at)| at == t(80)))
    );
    Ok(())
}

#[test]
fn order_tote_contents_never_enter_pick_stock_frontiers() -> TestResult {
    let (index, inventory, _observer) = test_index();
    let sku = SkuId(42);
    let order = ToteId(800);
    inventory.register_tote(
        order,
        ToteType::Tote,
        ToteDesignation::Order,
        ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
    )?;
    inventory.tote(order)?.add_item(sku, 3, 0)?;
    assert!(index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku }, &HashSet::new()).is_err());
    inventory.tote(order)?.move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, t(10))?;
    inventory.tote(order)?.move_to(ToteLocation::InRack { rack_id: RackId(1), z_index: 0 }, t(20))?;
    assert!(index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku }, &HashSet::new()).is_err());
    assert!(inventory.find_sku_availability(sku, &HashSet::new(), t(0)).is_empty());
    inventory.seed_inventory_from_totes();
    assert_eq!(inventory.sku_inventory_at(sku, t(20)), 0);

    let stock = ToteId(801);
    inventory.register_tote(
        stock,
        ToteType::Tote,
        ToteDesignation::Inventory,
        ToteLocation::InRack { rack_id: RackId(2), z_index: 0 },
    )?;
    inventory.tote(stock)?.add_item(sku, 2, 30)?;
    let candidates = index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku }, &HashSet::new())?;
    assert!(!candidates.is_empty());
    assert!(candidates.iter().all(|candidate| candidate.item == stock));
    Ok(())
}

#[test]
fn typed_empty_search_skips_cheaper_wrong_types_and_tracks_content_and_location_changes() -> TestResult {
    let (early, inventory, _observer) = test_index();
    for (id, kind, pool, rack) in [
        (1, ToteType::Tote, ToteDesignation::Order, 1),
        (2, ToteType::Tray, ToteDesignation::Order, 2),
        (3, ToteType::Tray, ToteDesignation::Inventory, 3),
    ] {
        inventory.register_tote(ToteId(id), kind, pool, ToteLocation::InRack { rack_id: RackId(rack), z_index: 0 })?;
    }
    let (data, _, _) = build_test_env(&[(1, &[(1, 10), (2, 20)])], &[(1, 12, 1), (2, 22, 1), (3, 15, 1)]);
    let late = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let _late_observer =
        inventory.add_observer_with_snapshot(Arc::new(late.clone()), |snapshot| late.initialize(snapshot))?;
    let typed = ToteSearchCriteria::EmptyOfType { designation: ToteDesignation::Order, tote_type: ToteType::Tray };
    for index in [&early, &late] {
        let any = index
            .best_tote_per_relay(ToteSearchCriteria::Empty { designation: ToteDesignation::Order }, &HashSet::new())?;
        assert_eq!(any.iter().find(|candidate| candidate.relay.id == RelayId(1)).unwrap().item, ToteId(1));
        let candidates = index.best_tote_per_relay(typed, &HashSet::new())?;
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|candidate| candidate.item == ToteId(2)));
        assert!(
            index
                .best_tote_per_relay(
                    ToteSearchCriteria::EmptyOfType {
                        designation: ToteDesignation::Order,
                        tote_type: ToteType::Carton,
                    },
                    &HashSet::new()
                )
                .is_err()
        );
    }
    let tray = inventory.tote(ToteId(2))?;
    tray.add_item(SkuId(1), 1, 10)?;
    for index in [&early, &late] {
        assert!(index.best_tote_per_relay(typed, &HashSet::new()).is_err());
    }
    tray.remove_item(SkuId(1), 1, 20)?;
    for index in [&early, &late] {
        let candidates = index.best_tote_per_relay(typed, &HashSet::new())?;
        assert!(candidates.iter().all(|candidate| candidate.item == ToteId(2) && candidate.available_at == t(20)));
    }
    tray.move_to(ToteLocation::Reserved { rack_id: RackId(2), z_index: 0 }, t(30))?;
    for index in [&early, &late] {
        assert!(index.best_tote_per_relay(typed, &HashSet::new()).is_err());
    }
    tray.move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, t(40))?;
    tray.move_to(ToteLocation::InRack { rack_id: RackId(2), z_index: 0 }, t(50))?;
    for index in [&early, &late] {
        let candidates = index.best_tote_per_relay(typed, &HashSet::new())?;
        assert!(candidates.iter().all(|candidate| candidate.item == ToteId(2) && candidate.available_at == t(50)));
    }
    Ok(())
}

/// Keep the previous per-relay representation as an independent oracle for
/// the shared membership layout. Expected writes below follow the physical
/// workflow, including writes which must lose to a later timestamp.
#[test]
fn shared_sku_membership_matches_per_relay_cost_frontiers_through_late_writes() -> TestResult {
    use crate::model::SimDuration;
    use crate::utils::frontier_sorted_set::FrontierSortedSet;

    type Reference = FxHashMap<(RelayId, SkuId), FrontierSortedSet<ToteId, SimDuration>>;
    fn write(
        reference: &mut Reference,
        index: &RelayInventoryIndex,
        tote: ToteId,
        slot: RackSlot,
        sku: SkuId,
        at: SimTime,
        present: bool,
    ) -> TestResult {
        for relay in index.relays_for_rack(slot.rack_id) {
            let set = reference.entry((relay, sku)).or_insert_with(FrontierSortedSet::new);
            if present {
                set.add(
                    tote,
                    index.zone_config_for_rack(slot.rack_id).unwrap().min_forward_cycle_cost(relay, slot),
                    at,
                )?;
            } else {
                set.remove(&tote, at)?;
            }
        }
        Ok(())
    }
    fn compare(index: &RelayInventoryIndex, reference: &Reference) -> TestResult {
        for sku in [SkuId(7), SkuId(8), SkuId(9)] {
            for skipped in [HashSet::new(), HashSet::from([ZoneId(1)])] {
                let mut expected: Vec<_> = reference
                    .iter()
                    .filter_map(|(&(relay, item), set)| {
                        if item != sku || skipped.contains(&index.0.relays[&relay].zone_id) {
                            return None;
                        }
                        set.first_with_time().map(|(&tote, cost, at)| (relay, tote, cost, at))
                    })
                    .collect();
                let mut actual: Vec<_> =
                    match index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku }, &skipped) {
                        Ok(candidates) => {
                            candidates.into_iter().map(|c| (c.relay.id, c.item, c.vtm_cost, c.available_at)).collect()
                        }
                        Err(ReservationError::NoCandidates { .. }) => Vec::new(),
                        Err(error) => return Err(error.into()),
                    };
                expected.sort_unstable();
                actual.sort_unstable();
                assert_eq!(actual, expected, "SKU {sku:?}, skipped zones {skipped:?}");
            }
        }
        Ok(())
    }

    let (data, inventory, _) =
        build_test_env(&[(1, &[(1, 10), (2, 20)])], &[(1, 12, 1), (2, 22, 1), (3, 15, 1), (4, 15, 1)]);
    let index = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let _observer = inventory.add_observer(Arc::new(index.clone()));
    let slot = |rack| RackSlot { rack_id: RackId(rack), level: 0 };
    let location = |rack| ToteLocation::InRack { rack_id: RackId(rack), z_index: 0 };
    let mut reference = Reference::default();
    for (id, rack) in [(9, 1), (3, 2)] {
        inventory.register_tote(ToteId(id), ToteType::Tote, ToteDesignation::Inventory, location(rack))?;
        for sku in [SkuId(7), SkuId(8)] {
            inventory.tote(ToteId(id))?.add_item(sku, 4, 0)?;
            write(&mut reference, &index, ToteId(id), slot(rack), sku, t(0), true)?;
        }
    }
    compare(&index, &reference)?;
    let mut choices = index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(7) }, &HashSet::new())?;
    choices.sort_unstable_by_key(|c| c.relay.id);
    assert_eq!(
        choices.iter().map(|c| c.item).collect::<Vec<_>>(),
        [ToteId(9), ToteId(3)],
        "same members must produce different winners for different relay geometry"
    );

    let tote = inventory.tote(ToteId(9))?;
    tote.remove_item(SkuId(7), 1, 10)?;
    compare(&index, &reference)?; // Still present: membership timestamp stays at zero.
    tote.move_to(ToteLocation::Reserved { rack_id: RackId(1), z_index: 0 }, t(20))?;
    for sku in [SkuId(7), SkuId(8)] {
        write(&mut reference, &index, ToteId(9), slot(1), sku, t(20), false)?;
    }
    compare(&index, &reference)?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(101) }, t(25))?;
    compare(&index, &reference)?;
    tote.move_to(location(3), t(40))?;
    for sku in [SkuId(7), SkuId(8)] {
        write(&mut reference, &index, ToteId(9), slot(3), sku, t(40), true)?;
    }
    compare(&index, &reference)?;
    tote.move_to(ToteLocation::Reserved { rack_id: RackId(3), z_index: 0 }, t(35))?;
    for sku in [SkuId(7), SkuId(8)] {
        write(&mut reference, &index, ToteId(9), slot(3), sku, t(35), false)?;
    }
    compare(&index, &reference)?; // Stale removal cannot erase a newer return.
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(101) }, t(80))?;
    for sku in [SkuId(7), SkuId(8)] {
        write(&mut reference, &index, ToteId(9), slot(3), sku, t(80), false)?;
    }
    tote.move_to(location(3), t(60))?;
    for sku in [SkuId(7), SkuId(8)] {
        write(&mut reference, &index, ToteId(9), slot(3), sku, t(60), true)?;
    }
    compare(&index, &reference)?; // Removal tombstone defeats an older return.
    inventory.tote(ToteId(3))?.remove_item(SkuId(7), 4, 100)?;
    write(&mut reference, &index, ToteId(3), slot(2), SkuId(7), t(100), false)?;
    compare(&index, &reference)?; // Removing one SKU leaves the other intact.

    for (id, rack) in [(5, 3), (4, 4)] {
        inventory.register_tote(ToteId(id), ToteType::Tote, ToteDesignation::Inventory, location(rack))?;
        inventory.tote(ToteId(id))?.add_item(SkuId(9), 1, 110)?;
        write(&mut reference, &index, ToteId(id), slot(rack), SkuId(9), t(110), true)?;
    }
    compare(&index, &reference)?;
    assert!(
        index
            .best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku: SkuId(9) }, &HashSet::new())?
            .iter()
            .all(|c| c.item == ToteId(4)),
        "equal costs retain the ToteId tie-break"
    );
    Ok(())
}

#[test]
fn shared_sku_membership_stays_with_its_side_and_ignores_zones_without_relays() -> TestResult {
    let (mut data, inventory, _) =
        build_test_env(&[(1, &[(1, 10), (2, 20)]), (2, &[(3, 30)])], &[(1, 11, 1), (2, 21, 1), (3, 31, 2)]);
    data.relay_data.iter_mut().find(|relay| relay.relay_id == RelayId(2)).unwrap().side = BlockSide::Right;
    data.rack_side.insert(RackId(2), BlockSide::Right);
    data.zone_data.iter_mut().find(|zone| zone.zone_id == ZoneId(2)).unwrap().relay_ids.clear();
    data.relay_data.retain(|relay| relay.relay_id != RelayId(3));
    data.relay_zones = RelayZones::from_scenario_legacy(&data, CycleTimes::default())?;
    let index = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
    let _observer = inventory.add_observer(Arc::new(index.clone()));
    let location = |rack| ToteLocation::InRack { rack_id: RackId(rack), z_index: 0 };
    inventory.register_tote(ToteId(1), ToteType::Tote, ToteDesignation::Inventory, location(1))?;
    let tote = inventory.tote(ToteId(1))?;
    tote.add_item(SkuId(7), 3, 0)?;
    let criteria = ToteSearchCriteria::ContainsSku { sku: SkuId(7) };
    for (rack, relay, at) in [(1, 1, 0), (2, 2, 20)] {
        if at > 0 {
            tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(at - 10))?;
            tote.move_to(location(rack), t(at))?;
        }
        let candidates = index.best_tote_per_relay(criteria, &HashSet::new())?;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].relay.id, RelayId(relay));
        assert_eq!(candidates[0].available_at, t(at));
        assert!(index.best_tote_per_relay(criteria, &HashSet::from([ZoneId(1)])).is_err());
    }
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(1) }, t(30))?;
    tote.move_to(location(3), t(40))?;
    tote.add_item(SkuId(8), 2, 50)?;
    for sku in [SkuId(7), SkuId(8)] {
        assert!(matches!(
            index.best_tote_per_relay(ToteSearchCriteria::ContainsSku { sku }, &HashSet::new()),
            Err(ReservationError::NoCandidates { .. })
        ));
    }
    Ok(())
}

#[test]
fn sku_frontier_storage_is_independent_of_same_side_relay_count() -> TestResult {
    for count in [1, 56] {
        let relays: Vec<_> = (1..=count).map(|id| (id, id * 2)).collect();
        let racks: Vec<_> = (1..=10).map(|id| (id, id * 3, 1)).collect();
        let (data, inventory, _) = build_test_env(&[(1, &relays)], &racks);
        let index = RelayInventoryIndex::new(&data.relay_zones, inventory.clone(), &FxHashMap::default());
        let _observer = inventory.add_observer(Arc::new(index.clone()));
        for id in 1..=10 {
            inventory.register_tote(
                ToteId(id),
                ToteType::Tote,
                ToteDesignation::Inventory,
                ToteLocation::InRack { rack_id: RackId(id), z_index: 0 },
            )?;
            for sku in 0..10 {
                inventory.tote(ToteId(id))?.add_item(SkuId(id * 10 + sku), 3, 0)?;
            }
        }
        let state = index.0.state.lock().unwrap();
        assert_eq!(state.sku_totes.len(), 100, "one frontier per zone/side/SKU with {count} relays");
        let members: usize = state.sku_totes.values().map(|set| set.len()).sum();
        assert_eq!(members, 100, "SKU/tote membership must not multiply by {count} relays");
        eprintln!(
            "[memory] relays={count} shared_sku_frontiers={} shared_sku_members={members}",
            state.sku_totes.len()
        );
    }
    Ok(())
}
