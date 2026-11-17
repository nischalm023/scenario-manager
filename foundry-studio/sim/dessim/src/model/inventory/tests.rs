//! InventoryService tests — temporal state queries.

use std::collections::{BTreeMap, HashMap, HashSet};

use desgraph::RelayId;

use super::error::InventoryError;
use crate::events::EventSink;
use crate::model::ToteDesignation;
use crate::model::location::{ToteLocation, ToteType};
use crate::model::{RackId, RobotId, SimTime, SkuId, ToteId};

fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

use super::*;

fn no_exclude() -> HashSet<ToteId> {
    HashSet::new()
}

fn rack_loc(rack: u32, z: u16) -> ToteLocation {
    ToteLocation::InRack { rack_id: RackId(rack), z_index: z }
}

fn service_with_totes(totes: &[(u32, u32, u32)]) -> Result<InventoryService, InventoryError> {
    // Each tote gets its own rack (keyed by tote id) to ensure unique positions.
    let mut rack_positions = HashMap::new();
    for &(id, x, y) in totes {
        rack_positions.insert(RackId(id), (x, y));
    }
    let svc = InventoryService::new();
    for &(id, _, _) in totes {
        svc.register_tote(ToteId(id), ToteType::Tote, ToteDesignation::Inventory, rack_loc(id, 0))?;
    }
    Ok(svc)
}

// -- Tote lifecycle --

#[test]
fn register_and_get_tote() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    assert_eq!(tote.tote_type(), ToteType::Tote);
    assert!(tote.is_empty());
    assert!(svc.tote(ToteId(999)).is_err());
    Ok(())
}

#[test]
fn tote_count() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0), (2, 5, 0), (3, 10, 0)])?;
    assert_eq!(svc.tote_count(), 3);
    Ok(())
}

// -- Item tracking --

#[test]
fn add_item_makes_tote_non_empty() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    assert!(tote.is_empty());

    tote.add_item(SkuId(10), 3, 0)?;
    assert!(!tote.is_empty());

    let contents = tote.contents();
    assert_eq!(contents[&SkuId(10)], 3);
    Ok(())
}

#[test]
fn remove_item_makes_tote_empty_again() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    tote.add_item(SkuId(10), 2, 0)?;
    tote.remove_item(SkuId(10), 2, 100)?;
    assert!(tote.is_empty());
    Ok(())
}

#[test]
fn add_item_updates_stock_index() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0), (2, 10, 0)])?;
    svc.tote(ToteId(1))?.add_item(SkuId(10), 5, 0)?;

    let results = svc.find_sku_availability(SkuId(10), &no_exclude(), t(0));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tote_id(), ToteId(1));
    assert_eq!(results[0].quantity(), 5);
    Ok(())
}

#[test]
fn remove_item_updates_stock_index() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    tote.add_item(SkuId(10), 3, 0)?;
    tote.remove_item(SkuId(10), 3, 100)?;

    // At T=100 the tote is empty
    let results = svc.find_sku_availability(SkuId(10), &no_exclude(), t(100));
    assert!(results.is_empty());

    // At T=50 the tote still has items
    let results = svc.find_sku_availability(SkuId(10), &no_exclude(), t(50));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].quantity(), 3);
    Ok(())
}

// -- Location tracking --

#[test]
fn move_tote_updates_location() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    assert!(matches!(tote.location(), ToteLocation::InRack { .. }));

    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, t(100))?;
    assert!(matches!(tote.location(), ToteLocation::WithRobot { .. }));

    // Temporal: at T=50 it's still in rack
    assert!(matches!(tote.location_at(t(50)), ToteLocation::InRack { .. }));
    // At T=100 it's at the station
    assert!(matches!(tote.location_at(t(100)), ToteLocation::WithRobot { .. }));
    Ok(())
}

// -- Temporal content queries --

#[test]
fn temporal_content_lifecycle() -> Result<(), InventoryError> {
    // Simulates putaway → storage → pick cycle
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;

    // T=0: empty in rack
    assert!(tote.is_empty_at(t(0)));

    // T=100: putaway fills it with SKU 10
    tote.add_item(SkuId(10), 3, 100)?;
    assert!(!tote.is_empty_at(t(100)));
    assert!(tote.has_sku_at(SkuId(10), t(100)));

    // T=50: still empty (before putaway)
    assert!(tote.is_empty_at(t(50)));
    assert!(!tote.has_sku_at(SkuId(10), t(50)));

    // T=5000: pick removes all 3 units
    tote.remove_item(SkuId(10), 3, 5000)?;
    assert!(tote.is_empty_at(t(5000)));
    assert!(!tote.has_sku_at(SkuId(10), t(5000)));

    // T=3000: still has SKU (between putaway and pick)
    assert!(tote.has_sku_at(SkuId(10), t(3000)));
    Ok(())
}

#[test]
fn temporal_sku_query_respects_time() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;

    // SKU deposited at T=100, picked at T=5000
    tote.add_item(SkuId(10), 1, 100)?;
    tote.remove_item(SkuId(10), 1, 5000)?;

    // Before deposit: found with forward-looking, available_at = 100
    let r = svc.find_sku_availability(SkuId(10), &no_exclude(), t(50));
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].tote_id(), ToteId(1));
    assert_eq!(r[0].available_at(), t(100), "tote available when SKU deposited");

    // During window: found, available immediately
    let r = svc.find_sku_availability(SkuId(10), &no_exclude(), t(100));
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].available_at(), t(100));

    // After pick: not found (SKU cleared at T=5000)
    assert!(svc.find_sku_availability(SkuId(10), &no_exclude(), t(5000)).is_empty());
    Ok(())
}

#[test]
fn tote_with_multiple_skus() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;

    tote.add_item(SkuId(10), 2, 0)?;
    tote.add_item(SkuId(20), 3, 0)?;
    assert!(tote.has_sku_at(SkuId(10), t(0)));
    assert!(tote.has_sku_at(SkuId(20), t(0)));

    let contents = tote.contents_at(t(0));
    assert_eq!(contents.len(), 2);
    Ok(())
}

// -- find_nearest_with_sku --

// -- find_nearest_empty_tote --

// -- SKU ledger: temporal availability --

#[test]
fn earliest_available_empty_ledger() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    assert!(!svc.has_sku_at_or_after(SkuId(10), t(0)));
    assert_eq!(svc.earliest_sku_available(SkuId(10), t(0)), None);
    Ok(())
}

#[test]
fn earliest_available_after_putaway() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    // Simulate putaway: inventory added at T=5000
    svc.record_inventory_added(SkuId(10), t(5000));
    assert!(svc.has_sku_at_or_after(SkuId(10), t(0)));
    assert_eq!(svc.earliest_sku_available(SkuId(10), t(0)), Some(t(5000)));
    // At T=5000 itself
    assert_eq!(svc.earliest_sku_available(SkuId(10), t(5000)), Some(t(5000)));
    // After the event
    assert_eq!(svc.earliest_sku_available(SkuId(10), t(6000)), Some(t(6000)));
    // Not available if all reserved
    svc.reserve_for_order(&[(SkuId(10), 1)], t(5000));
    assert!(!svc.has_sku_at_or_after(SkuId(10), t(5000)));
    Ok(())
}

#[test]
fn reserve_for_order_refuses_a_unit_consumed_later() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    // One unit arrives at T=1000; an order reserves it at T=2000 and picks it at T=3000.
    svc.record_inventory_added(SkuId(10), t(1000));
    assert!(svc.reserve_for_order(&[(SkuId(10), 1)], t(2000)));
    svc.record_pick_consumed(SkuId(10), t(3000));
    // At T=1500 the ledger shows the unit free, yet it is gone by T=3000: a
    // reservation placed there would book a unit nobody can pick.
    assert!(!svc.reserve_for_order(&[(SkuId(10), 1)], t(1500)));
    // A second unit arriving at T=4000 is reservable from then on.
    svc.record_inventory_added(SkuId(10), t(4000));
    assert!(svc.reserve_for_order(&[(SkuId(10), 1)], t(4000)));
    Ok(())
}

#[test]
fn earliest_sku_qty_lasting_starts_after_the_last_dip() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    // One unit arrives at T=1000; an order holds it from T=2000 and picks it
    // at T=3000; a second unit arrives at T=4000.
    svc.record_inventory_added(SkuId(10), t(1000));
    assert!(svc.reserve_for_order(&[(SkuId(10), 1)], t(2000)));
    svc.record_pick_consumed(SkuId(10), t(3000));
    svc.record_inventory_added(SkuId(10), t(4000));
    // A unit is free at T=1000, but one stays free only from T=4000 on.
    assert_eq!(svc.earliest_sku_qty_available(SkuId(10), 1, t(0)), Some(t(1000)));
    assert_eq!(svc.earliest_sku_qty_lasting(SkuId(10), 1, t(0)), Some(t(4000)));
    assert_eq!(svc.earliest_sku_qty_lasting(SkuId(10), 1, t(5000)), Some(t(5000)));
    assert_eq!(svc.earliest_sku_qty_lasting(SkuId(10), 2, t(0)), None);
    // The answer is the earliest simtime the reservation accepts.
    assert!(!svc.reserve_for_order(&[(SkuId(10), 1)], t(3999)));
    assert!(svc.reserve_for_order(&[(SkuId(10), 1)], t(4000)));
    Ok(())
}

#[test]
fn earliest_available_multiple_putaways() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    svc.record_inventory_added(SkuId(10), t(3000));
    svc.record_inventory_added(SkuId(10), t(7000));
    // Reserve 1 at T=3000 — first unit spoken for
    svc.reserve_for_order(&[(SkuId(10), 1)], t(3000));
    // Still available at T=7000 (second putaway unit)
    assert_eq!(svc.earliest_sku_available(SkuId(10), t(0)), Some(t(7000)));
    Ok(())
}

// -- find_sku_availability: forward-looking tote search --

#[test]
fn find_sku_availability_putaway_then_rack() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    // Tote starts empty InRack. Putaway fills it at T=5000.
    tote.add_item(SkuId(10), 1, 5000)?;
    // Tote travels: WithRobot at T=3000, AtRelay at T=4000, back InRack at T=8000.
    // (putaway chain: HTM carries to station, presents, HTM stores, VTM returns)
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(0) }, t(3000))?;
    tote.move_to(ToteLocation::AtRelay { relay_id: RelayId(0) }, t(4000))?;
    tote.move_to(rack_loc(0, 0), t(8000))?;

    let avail = svc.find_sku_availability(SkuId(10), &no_exclude(), SimTime::ms(0));
    // Should find InRack phase starting at T=8000 (has SKU + InRack).
    // Content set at T=5000, but tote is WithRobot/AtRelay until T=8000.
    let in_rack: Vec<_> = avail.iter().filter(|a| matches!(a, SkuAvailability::InRack { .. })).collect();
    assert_eq!(in_rack.len(), 1);
    assert_eq!(in_rack[0].available_at().as_ms(), 8000);
    assert_eq!(in_rack[0].tote_id(), ToteId(1));
    Ok(())
}

#[test]
fn find_sku_availability_pre_stocked() -> Result<(), InventoryError> {
    let svc = service_with_totes(&[(1, 0, 0)])?;
    let tote = svc.tote(ToteId(1))?;
    tote.add_item(SkuId(10), 1, 0)?;
    svc.seed_inventory_from_totes();

    let avail = svc.find_sku_availability(SkuId(10), &no_exclude(), SimTime::ms(0));
    let in_rack: Vec<_> = avail.iter().filter(|a| matches!(a, SkuAvailability::InRack { .. })).collect();
    assert_eq!(in_rack.len(), 1);
    assert_eq!(in_rack[0].available_at().as_ms(), 0);
    Ok(())
}

// -- Benchmarks --

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "scale benchmark; slow gate: scripts/test-slow-gate.sh")]
fn bench_data_structure_variants() {
    super::bench_variants::run_all_benchmarks();
}

#[test]
fn tote_mutations_emit_inventory_delta_events() -> Result<(), InventoryError> {
    use crate::events::SimEvent;
    use std::sync::Arc;
    use std::sync::mpsc;

    let mut rack_positions = HashMap::new();
    rack_positions.insert(RackId(1), (0, 0));

    let (tx, rx) = mpsc::channel::<SimEvent>();
    let sink: Arc<dyn EventSink> = Arc::new(tx);
    let svc = InventoryService::with_event_sink(Some(sink));

    svc.register_tote(ToteId(1), ToteType::Tote, ToteDesignation::Inventory, rack_loc(1, 0))?;
    let tote = svc.tote(ToteId(1))?;
    tote.add_item(SkuId(10), 50, 0)?;
    tote.remove_item(SkuId(10), 3, 1_000)?;

    // Drain channel; collect the InventoryDelta-shaped events.
    let deltas: Vec<(i32, u64)> = rx
        .try_iter()
        .filter_map(|e| match e {
            SimEvent::InventoryDelta(d) => Some((d.delta, d.at.as_ms())),
            _ => None,
        })
        .collect();

    // Regression: before the dispatch_event refactor, Tote::emit only hit
    // registered observers and never reached the event_sink, so neither of
    // these `ContentsChanged` mutations produced a SimEvent.
    assert_eq!(deltas, vec![(50, 0), (-3, 1_000)]);
    Ok(())
}

#[test]
fn tote_moves_emit_inventory_move_events() -> Result<(), InventoryError> {
    use std::sync::Arc;
    use std::sync::mpsc;

    use crate::events::SimEvent;

    let mut rack_positions = HashMap::new();
    rack_positions.insert(RackId(1), (0, 0));

    let (tx, rx) = mpsc::channel::<SimEvent>();
    let sink: Arc<dyn EventSink> = Arc::new(tx);
    let svc = InventoryService::with_event_sink(Some(sink));

    // Register in rack (emits a move Uninitialized→InRack), fill it, then carry
    // the laden tote out on a robot (the departure that a pick begins with).
    svc.register_tote(ToteId(1), ToteType::Tote, ToteDesignation::Inventory, rack_loc(1, 0))?;
    let tote = svc.tote(ToteId(1))?;
    tote.add_item(SkuId(10), 40, 0)?;
    tote.move_to(ToteLocation::WithRobot { robot_id: RobotId(7) }, t(1_000))?;

    let moves: Vec<(ToteLocation, ToteLocation, Vec<(SkuId, u32)>, u64)> = rx
        .try_iter()
        .filter_map(|e| match e {
            SimEvent::InventoryMove(m) => Some((m.from, m.to, m.contents, m.at.as_ms())),
            _ => None,
        })
        .collect();

    // Two moves bridged: the T=0 registration (empty), then the pickup carrying
    // the tote's contents as-of the move instant — the value the observer
    // attributes to the aisle crossing.
    assert_eq!(moves.len(), 2);
    assert!(matches!(moves[0].0, ToteLocation::Uninitialized));
    assert!(matches!(moves[0].1, ToteLocation::InRack { .. }));
    assert!(moves[0].2.is_empty());
    assert!(matches!(moves[1].0, ToteLocation::InRack { .. }));
    assert!(matches!(moves[1].1, ToteLocation::WithRobot { .. }));
    assert_eq!(moves[1].2, vec![(SkuId(10), 40)]);
    assert_eq!(moves[1].3, 1_000);
    Ok(())
}

#[test]
fn event_channel_closes_when_inventory_drops_with_back_referencing_observer() {
    // Regression for the "wallclock continues forever" hang. The cycle:
    //   InventoryService.observers (strong Arc) -> ObserverHoldingInventory
    //   ObserverHoldingInventory.inventory     (strong Arc) -> InventoryService
    // leaks both, and once `event_sink` lives inside InventoryService.Inner
    // that leak holds an mpsc::Sender clone open, which prevents the
    // observer thread's `event_rx.iter()` from terminating at sim shutdown.
    //
    // After the Weak-in-observers fix, dropping the caller-held strong
    // Arc (here: `_observer_handle`) plus the InventoryService should close
    // the channel even though the observer back-references inventory.
    use crate::events::SimEvent;
    use crate::model::inventory::events::{InventoryEvent, InventoryObserver, InventoryObserverError};
    use std::sync::Arc;
    use std::sync::mpsc;

    /// Observer that holds a strong reference back to InventoryService —
    /// the exact shape that creates the cycle in production
    /// (RelayInventoryIndex).
    struct ObserverHoldingInventory {
        _inventory: InventoryService,
    }
    impl InventoryObserver for ObserverHoldingInventory {
        fn on_inventory_event(&self, _: &InventoryEvent) -> Result<(), InventoryObserverError> {
            Ok(())
        }
    }

    let (tx, rx) = mpsc::channel::<SimEvent>();
    let sink: Arc<dyn EventSink> = Arc::new(tx);
    let inventory = InventoryService::with_event_sink(Some(sink));

    let observer = Arc::new(ObserverHoldingInventory { _inventory: inventory.clone() });
    let observer_handle = inventory.add_observer(observer);

    // Drop both the strong observer handle and the inventory. With strong
    // observers + back-reference these would leak the sink Arc; with weak
    // observers the sink Arc reaches refcount 0 and closes the channel.
    drop(observer_handle);
    drop(inventory);

    // Channel must report Disconnected — that's the signal the real
    // observer thread waits on (event_rx.iter() terminates).
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)));
}

#[test]
fn runtime_container_creation_preserves_birth_and_emits_actual_time() {
    use crate::events::SimEvent;
    use desgraph::CellId;
    use std::sync::{Arc, mpsc};

    let (tx, rx) = mpsc::channel();
    let svc = InventoryService::with_event_sink(Some(Arc::new(tx)));
    svc.register_tote(ToteId(41), ToteType::Tote, ToteDesignation::Inventory, rack_loc(1, 0)).unwrap();
    let initial = ToteLocation::AtCell { cell_id: CellId(9) };
    let id = svc.create_tote(ToteType::Carton, ToteDesignation::Order, initial, t(2500)).unwrap();
    assert_eq!(id, ToteId(42));
    let tote = svc.tote(id).unwrap();
    assert_eq!(tote.tote_type(), ToteType::Carton);
    assert_eq!(tote.designation(), ToteDesignation::Order);
    assert_eq!(tote.location_at(t(2499)), ToteLocation::Uninitialized);
    assert_eq!(tote.location_at(t(2500)), initial);
    assert!(tote.contents_at(t(2499)).is_empty());
    tote.add_item(SkuId(1), 2, 2500).unwrap();
    assert!(tote.contents_at(t(2499)).is_empty());
    assert_eq!(tote.contents_at(t(2500)), vec![(SkuId(1), 2)]);
    for error in [
        tote.add_item(SkuId(2), 1, 2499).unwrap_err(),
        tote.remove_item(SkuId(1), 1, 2499).unwrap_err(),
        tote.move_to(ToteLocation::Extracted, t(2499)).unwrap_err(),
    ] {
        assert!(
            matches!(error, InventoryError::BeforeCreation { at, created_at } if at == t(2499) && created_at == t(2500))
        );
    }
    assert_eq!(tote.location_at(t(2499)), ToteLocation::Uninitialized);
    tote.move_to(ToteLocation::Extracted, t(3000)).unwrap();
    assert_eq!(tote.location_at(t(2999)), initial);
    assert_eq!(tote.location_at(t(3000)), ToteLocation::Extracted);
    assert_eq!(tote.contents_at(t(3000)), vec![(SkuId(1), 2)], "whole-container extraction preserves its contents");
    let moves: Vec<_> = rx
        .try_iter()
        .filter_map(|event| match event {
            SimEvent::InventoryMove(moved) if moved.tote_id == id => Some(moved),
            _ => None,
        })
        .collect();
    assert_eq!(moves.len(), 2);
    assert_eq!((moves[0].from, moves[0].to, moves[0].at), (ToteLocation::Uninitialized, initial, t(2500)));
    assert_eq!(moves[1].to, ToteLocation::Extracted);
}

#[test]
fn runtime_container_ids_are_unique_across_concurrent_creators_and_never_wrap() {
    use std::thread;
    let svc = InventoryService::new();
    let mut ids = thread::scope(|scope| {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                scope.spawn(|| {
                    svc.create_tote(ToteType::Tray, ToteDesignation::Order, ToteLocation::Extracted, t(100)).unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    ids.sort_unstable();
    assert_eq!(ids, (0..16).map(ToteId).collect::<Vec<_>>());
    svc.register_tote(ToteId(u32::MAX), ToteType::Tote, ToteDesignation::Inventory, rack_loc(1, 0)).unwrap();
    assert!(matches!(
        svc.create_tote(ToteType::Carton, ToteDesignation::Order, ToteLocation::Extracted, t(100)),
        Err(InventoryError::IdExhausted)
    ));
    assert_eq!(svc.tote_count(), 17);
    assert_eq!(svc.tote(ToteId(u32::MAX)).unwrap().tote_type(), ToteType::Tote);
}

#[test]
fn physical_order_contents_are_reported_without_adding_picked_units_back_to_stock() -> Result<(), InventoryError> {
    use crate::events::SimEvent;
    use std::sync::{Arc, mpsc};

    let (tx, rx) = mpsc::channel();
    let svc = InventoryService::with_event_sink(Some(Arc::new(tx)));
    svc.register_tote(ToteId(1), ToteType::Tote, ToteDesignation::Inventory, rack_loc(1, 0))?;
    svc.register_tote(ToteId(2), ToteType::Tray, ToteDesignation::Order, rack_loc(2, 0))?;
    let stock = svc.tote(ToteId(1))?;
    let order = svc.tote(ToteId(2))?;
    stock.add_item(SkuId(10), 5, 0)?;
    stock.remove_item(SkuId(10), 2, 20)?;
    order.add_item(SkuId(10), 2, 20)?;
    order.add_item(SkuId(11), 3, 25)?;
    order.remove_item(SkuId(10), 2, 30)?;
    order.remove_item(SkuId(11), 3, 30)?;
    let events: Vec<_> = rx.try_iter().collect();
    let stock_deltas: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::InventoryDelta(data) => Some((data.tote_id, data.delta, data.at.as_ms())),
            _ => None,
        })
        .collect();
    assert_eq!(stock_deltas, [(ToteId(1), 5, 0), (ToteId(1), -2, 20)]);
    let physical: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteContentsChanged(data) if data.tote_id == ToteId(2) => Some((data.units, data.at.as_ms())),
            _ => None,
        })
        .collect();
    assert_eq!(physical, [(2, 20), (5, 25), (3, 30), (0, 30)]);
    Ok(())
}
