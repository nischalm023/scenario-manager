//! Removing fabricated VTM cells must preserve fleet placement and rail timing.
use std::path::Path;

use desgraph::CellKind;
use desmap::VtmPosition;
use dessim::SimConfig;
use dessim::scenario::GeneratedScenario;
use dessim::scenario_gen::generate_scenario_from_map_with_racking;
use dessim::worldmap;
use native_modules as _;

#[test]
fn vtm_positions_add_no_cells_to_world_maps() {
    for fixture in ["minimal_easyloop.json", "cencora_v3.json", "walmart_dfw6.json"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let w = worldmap::load(&path).unwrap();
        assert_eq!(w.map.graph.cell_count(), w.world.cells.len() + w.steel.len(), "{fixture}");
        assert_eq!(w.index.cell_entities.len(), w.map.graph.cell_count(), "every cell has provenance");
        assert!(w.map.graph.cells().all(|cell| cell.kind != CellKind::Aisle));
        for relay in &w.map.relays {
            assert!(matches!(relay.vtm_access, VtmPosition::At { .. }));
            assert_eq!(relay.vtm_access.anchor(), relay.position);
        }
    }
}

#[test]
fn initial_vtm_coordinates_do_not_snap_to_their_reporting_anchors() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop.json");
    let w = worldmap::load(&path).unwrap();
    let scenario = GeneratedScenario {
        num_skus: 1,
        total_inventory_units: 10,
        num_orders: 0,
        total_order_picks: 0,
        ..Default::default()
    };
    let config = SimConfig { num_htms: 0, ..Default::default() };
    let data = generate_scenario_from_map_with_racking(w.map, w.racking, &scenario, &config).unwrap();
    let actual: Vec<_> = data
        .vtm_initial_positions
        .iter()
        .map(|start| {
            let pos = start.world_pos(&data.map.graph).unwrap();
            assert_ne!(Some(pos), data.map.cell_world_pos(start.anchor()));
            [pos.x_mm, pos.y_mm]
        })
        .collect();
    assert_eq!(actual, [[385.0, 17650.0], [3465.0, 17650.0], [8085.0, 17650.0]]);
    assert!((data.relay_zones.mean_forward_cycle_secs().unwrap() - 11.693888977764347).abs() < 1e-12);
}
