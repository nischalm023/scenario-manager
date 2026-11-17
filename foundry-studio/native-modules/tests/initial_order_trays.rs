//! Explicit fixture configuration chooses physical trays without changing the
//! order pool's size, contents, startup positions, or occupied-slot counts.

mod common;

use std::sync::{Arc, mpsc};

use dessim::composition::{self, Composition};
use dessim::events::SimEvent;
use dessim::model::{InventoryService, SkuId, StorageSpaceId, ToteDesignation, ToteId, ToteType};
use dessim::observer::SimObserver;
use dessim::observer::time_slice::{MetricsConfig, ToteVisualState};
use dessim::runner::resolve_scenario_with_racking;
use dessim::{FidelityMode, SimTime};
use native_modules as _;
use rustc_hash::FxHashSet;
use serde_json::json;

#[test]
fn explicit_order_trays_select_empty_ids_stably_and_do_not_double_count_storage() {
    for count in [0, 2] {
        let (world, scenario, mut config) = common::load_world_spec("minimal_easyloop_flow_spec.json");
        config.fidelity = FidelityMode::Ghost;
        config.composition =
            serde_json::from_value(json!({"tasks":[{"type":"FillInventory","config":{"order_trays":count}}]})).unwrap();
        let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
        let mut order_ids: Vec<_> = data
            .tote_placements
            .iter()
            .filter(|placement| placement.designation == ToteDesignation::Order)
            .map(|placement| placement.tote_id)
            .collect();
        order_ids.sort_unstable();
        assert!(order_ids.len() >= 3);
        let filled = order_ids[0];
        data.inventory.insert((SkuId(999), StorageSpaceId(filled.0)), 1);
        data.tote_placements.reverse();
        let expected_trays: Vec<_> = order_ids[1..].iter().copied().take(count).collect();
        let metrics_config = MetricsConfig {
            rack_cell_zone: Arc::new(data.rack_cells.values().map(|cell| (cell.0, 1)).collect()),
            zone_capacity: Arc::new([(1, data.tote_placements.len() as u32)].into()),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let pathfinder = common::fh_sipp_for_map(&data.map, &config);
        let (mut scheduler, reports, _) =
            composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
        scheduler.run().unwrap();
        let inventory = reports.existing::<InventoryService>().unwrap();
        assert_eq!(inventory.tote_count(), data.tote_placements.len());
        for placement in &data.tote_placements {
            let tote = inventory.tote(placement.tote_id).unwrap();
            let expected = if expected_trays.contains(&placement.tote_id) { ToteType::Tray } else { ToteType::Tote };
            assert_eq!(tote.tote_type(), expected);
            assert_eq!(tote.designation(), placement.designation);
            assert_eq!(tote.location_at(SimTime::ZERO), placement.location);
        }
        assert_eq!(inventory.tote(filled).unwrap().contents_at(SimTime::ZERO), vec![(SkuId(999), 1)]);
        let events: Vec<_> = rx.try_iter().collect();
        let initial_index = events.iter().position(|event| matches!(event, SimEvent::ToteInitialStates(_))).unwrap();
        let SimEvent::ToteInitialStates(initial) = &events[initial_index] else { unreachable!() };
        let occupied: FxHashSet<_> = initial
            .totes
            .iter()
            .filter_map(|(_, state)| match state {
                ToteVisualState::InRack { cell_id, level } => Some((*cell_id, *level)),
                _ => None,
            })
            .collect();
        let mut created = Vec::new();
        for (index, event) in events.iter().enumerate() {
            if let SimEvent::ToteCreated(birth) = event {
                assert!(index > initial_index);
                assert_eq!(birth.time, SimTime::ZERO);
                assert_eq!(birth.tote_type, ToteType::Tray);
                assert_eq!(birth.designation, ToteDesignation::Order);
                created.push(birth.tote_id);
            }
        }
        created.sort_unstable();
        assert_eq!(created, expected_trays);
        let (tx, rx) = mpsc::channel();
        for event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        let metrics = SimObserver::new(metrics_config, rx).run().expect("the facts break no rule of the event stream");
        let last = metrics.aisle_state_log.last().unwrap();
        assert_eq!(last.storage_occupied, occupied.len() as u32);
        assert_eq!(last.storage_empty + last.storage_non_empty, last.storage_occupied);
        assert_eq!(metrics.time_slices[0].tote_state.creations.len(), count);
    }
}

#[test]
fn order_tray_configuration_refuses_invalid_counts_before_seeding() {
    for value in [json!(-1), json!(1.5), json!(true), json!("2"), json!(null)] {
        let definition = serde_json::from_value::<Composition>(
            json!({"tasks":[{"type":"FillInventory","config":{"order_trays":value}}]}),
        );
        if let Ok(composition) = definition {
            assert!(composition.resolve().is_err());
        }
    }
    let (world, scenario, mut config) = common::load_world_spec("minimal_easyloop_flow_spec.json");
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    let pool = data.tote_placements.iter().filter(|placement| placement.designation == ToteDesignation::Order).count();
    config.composition =
        serde_json::from_value(json!({"tasks":[{"type":"InitializeFleet"},{"type":"FillInventory","after":["InitializeFleet"],"config":{"order_trays":pool+1}}]})).unwrap();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, None).unwrap();
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("seeded empty order containers"), "{error}");
    assert!(reports.existing::<InventoryService>().is_none_or(|inventory| inventory.tote_count() == 0));
}
