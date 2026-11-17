//! A physical VTM return changes storage location. Only newly received
//! inventory adds pickable stock; residual and already picked units do not.

use std::sync::Arc;

use desgraph::{CellId, ZoneId};
use desmap::{Zone, ZoneHolds};

use crate::config::VtmConfig;
use crate::events::{EventJournal, ToteSource};
use crate::flows::ttp::handoff::ToteAtRelay;
use crate::flows::ttp::task::VtmReturn;
use crate::model::relay::RelayService;
use crate::model::relay::tests::helpers::{build_test_env, empty_htm_registry, test_navigation};
use crate::model::relay::vtm::VtmService;
use crate::model::reservation::oracle::GhostOracle;
use crate::model::test_model::block;
use crate::model::{RackId, RobotId, SimTime, SkuId, ToteId};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::scenario_gen::RackUse;
use crate::scheduler::Scheduler;

#[test]
fn rack_return_counts_only_new_inventory_receipts() {
    for (designation, source, initial_stock, expected_stock) in [
        (ToteDesignation::Inventory, ToteSource::Pick, 3, 3),
        (ToteDesignation::Order, ToteSource::Pick, 0, 0),
        (ToteDesignation::Order, ToteSource::Putaway, 0, 0),
        (ToteDesignation::Inventory, ToteSource::Putaway, 0, 3),
    ] {
        let (mut data, inventory, pathfinder) = build_test_env(&[(1, &[(1, 10), (2, 20), (3, 30)])], &[(1, 12, 1)]);
        if designation == ToteDesignation::Order {
            data.rack_use.insert(RackId(1), RackUse::OrderBuffer);
            data.map.zones.push(Zone {
                id: ZoneId(1),
                name: None,
                cells: vec![CellId(12)],
                relays: data.relay_data.iter().map(|relay| relay.relay_id).collect(),
                vtm_count: 1,
                holds: ZoneHolds::Order,
                vtm_configuration: None,
                zone_height_mm: 10_000,
                zone_width_mm: None,
                real_shelf_count: 10,
                slots: None,
            });
        }
        let available = empty_htm_registry(&data, &inventory);
        let relays = RelayService::new(
            &data,
            inventory.clone(),
            test_navigation(&data, pathfinder, Arc::new(GhostOracle)),
            &available,
            &VtmConfig::default(),
        )
        .unwrap();
        // Vacating this slot supplies an actual deposit reservation for the return.
        let tote_id = ToteId(1);
        inventory
            .register_tote(
                tote_id,
                ToteType::Tote,
                designation,
                ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
            )
            .unwrap();
        inventory.tote(tote_id).unwrap().add_item(SkuId(1), 3, 0).unwrap();
        for _ in 0..initial_stock {
            inventory.record_inventory_added(SkuId(1), SimTime::ZERO);
        }
        inventory
            .tote(tote_id)
            .unwrap()
            .move_to(ToteLocation::WithRobot { robot_id: RobotId(99) }, SimTime::ms(1))
            .unwrap();
        let reservation = if designation == ToteDesignation::Order {
            block(relays.reserve_buffer_relay_for_deposit(
                CellId(10),
                Default::default(),
                SimTime::ms(2),
                RobotId(99),
                None,
            ))
        } else {
            block(relays.reserve_relay_for_deposit(CellId(10), Default::default(), SimTime::ms(2), RobotId(99), None))
        }
        .unwrap();
        let relay_id = reservation.relay_id;
        let rack_slot = reservation.recommendation;
        let deposit_time = reservation.start;
        inventory.tote(tote_id).unwrap().move_to(ToteLocation::AtRelay { relay_id }, deposit_time).unwrap();
        let handoff = ToteAtRelay {
            tote_id,
            relay_id,
            rack_slot,
            reservation,
            deposit_time,
            presentation_id: None,
            tote_source: source,
        };
        let task = VtmReturn::new(
            handoff,
            VtmService::new(&data, &VtmConfig::default()),
            inventory.clone(),
            EventJournal::new(None),
        );
        let mut scheduler = Scheduler::new();
        scheduler.add_task(task);
        scheduler.run().unwrap();
        assert_eq!(
            inventory.tote(tote_id).unwrap().location(),
            ToteLocation::InRack { rack_id: rack_slot.rack_id, z_index: rack_slot.level }
        );
        assert_eq!(
            inventory.sku_inventory_at(SkuId(1), SimTime::EOT),
            expected_stock,
            "return of {designation:?}/{source:?} changed stock incorrectly"
        );
        assert_eq!(inventory.tote(tote_id).unwrap().sku_qty_at(SkuId(1), SimTime::EOT), 3);
    }
}
