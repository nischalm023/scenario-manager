//! What a world-protocol document becomes when dessim reads it, pinned by
//! arithmetic rather than by whatever the loader happened to produce.
//!
//! The document under test is `data/seed/worlds/minimal-putaway2/`, the
//! hand conversion of `sim/desmap/tests/fixtures/minimal_putaway2.json`. Every count below is
//! derived from the source geometry in the comment beside it, so a change in
//! the loader that moves a number has to argue with the arithmetic.
//!
//! The seam these tests guard, and the behavioural changes they encode, are
//! written down in `sim/dessim/PORT-NOTES.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use desgraph::{BlockSide, CellId, CellKind, RelayId, StationType, SystemType};
use desmap::{PresentationPoint, ZoneHolds};
use dessim::events::{EventSink, SimEvent};
use dessim::pathfinding::holding_cells;
use dessim::runner::{RunnerHooks, resolve_scenario_with_racking};
use dessim::scenario::{GeneratedScenario, ScenarioConfig};
use dessim::worldmap::{self, WorldMap};
use dessim::{FHSippPathfinder, FidelityMode, SimConfig, SimDuration};
use native_modules as _;
use serde_json::Value;

fn minimal_putaway2() -> WorldMap {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../data/seed/worlds/minimal-putaway2/minimal_putaway2.json");
    worldmap::load(&path).expect("load the converted minimal_putaway2 world")
}

fn kinds(world: &WorldMap) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    for cell in world.map.graph.cells() {
        *out.entry(format!("{:?}", cell.kind)).or_default() += 1;
    }
    out
}

#[test]
fn the_document_loads_without_warnings() {
    let world = minimal_putaway2();
    assert_eq!(world.warnings, Vec::<String>::new());
    assert_eq!(world.map.system_type, SystemType::Relay);
}

#[test]
fn the_header_extent_comes_out_of_the_geometry() {
    // The floor region is 8000 x 18000 mm at a 1000 mm pitch, so its cells sit
    // on 8 distinct x centres (500..7500) and 18 distinct y centres
    // (500..17500). The header measures this floor, independently of rack anchors.
    let bounds = minimal_putaway2().map.grid_bounds.expect("the header extent");
    assert_eq!((bounds.width, bounds.depth), (8, 18));
    assert_eq!(bounds.cell_size_mm, 1000.0);
}

#[test]
fn every_place_in_the_document_is_a_cell_exactly_once() {
    let world = minimal_putaway2();
    // 8 x 18 = 144 floor cells, less the 12 the zone exclusion removes (two
    // grid columns over six rows), leaves 132 cells of the floor. Six bays add
    // one rack anchor each. Each bay's handoff shelf binds an existing floor
    // cell, and VTM positions create no cells.
    assert_eq!(world.map.graph.cell_count(), 132 + 6);
    assert_eq!(
        kinds(&world),
        BTreeMap::from([
            // 132 cells of the floor, less the six the two stations claim and
            // the six the relays bind as their HTM access cells.
            ("Exchange".into(), 6),
            ("Open".into(), 120),
            ("Presentation".into(), 2),
            ("QueueCell".into(), 2),
            ("StationArea".into(), 2),
            // One bay each, all six carrying a handoff shelf.
            ("Relay".into(), 6),
        ])
    );
}

#[test]
fn the_arcs_are_the_documents_own() {
    // A fully two-way 8 x 18 floor has 7 x 18 = 126 horizontal and 8 x 17 = 136
    // vertical neighbour pairs: 262 pairs, 524 directed arcs. The zone
    // exclusion takes out two columns over six rows, removing 12 horizontal
    // pairs (each excluded row loses the 5|6 and 6|7 pairs) and 14 vertical
    // pairs (7 in each excluded column) — 26 pairs, 52 arcs.
    //
    // A relay's binding adds none: it names the floor cell an HTM stands on
    // rather than linking a cell of its own into the graph.
    //
    // Minus 18 more: the station's queue discipline (the same one-way
    // dest_only corridor v3 maps get from queue_edges::restrict_queue_htm)
    // drops every edge touching its queue and presentation cells except the
    // entry, the forward chain, and the exit.
    let world = minimal_putaway2();
    assert_eq!(world.map.graph.edge_count(), 524 - 52 - 18);
}

#[test]
fn a_relay_stands_on_its_bay_and_is_reached_from_the_lane() {
    let world = minimal_putaway2();
    assert_eq!(world.map.relays.len(), 6);

    // Positions stay physical coordinates when internal CellIds change.
    for (index, relay) in world.map.relays.iter().enumerate() {
        let y = 2500.0 + 1000.0 * index as f64;
        let at = |cell| {
            let point = world.map.cell_world_pos(cell).unwrap();
            [point.x_mm, point.y_mm]
        };
        assert_eq!(relay.id, RelayId(index as u32));
        assert_eq!(at(relay.position), [6500.0, y], "the bay's steel");
        assert_eq!(at(relay.htm_access), [5500.0, y], "HTM floor access");
        let vtm = relay.vtm_access.world_pos(&world.map.graph).unwrap();
        assert_eq!([vtm.x_mm, vtm.y_mm], [7500.0, y]);
        assert_eq!(relay.vtm_access.anchor(), relay.position, "events identify the existing bay");
        assert_eq!(relay.side, BlockSide::Left);
        assert_eq!(world.map.graph.cell(relay.position).unwrap().kind, CellKind::Relay);
        assert_eq!(world.map.graph.cell(relay.htm_access).unwrap().kind, CellKind::Exchange);
    }

    // Each bay carries one relay level with one position on it, so one shelf
    // stands at each exchange point and the six bind six different cells.
    let shelves: Vec<usize> = world.index.relay_shelves.values().map(Vec::len).collect();
    assert_eq!(shelves, vec![1; 6]);
}

#[test]
fn the_zone_is_the_aisle_and_what_it_serves() {
    let world = minimal_putaway2();
    assert_eq!(world.map.zones.len(), 1);
    let zone = &world.map.zones[0];
    assert_eq!(zone.cells.len(), 6, "six rack anchors; VTM positions add no cells");
    assert_eq!(zone.relays.len(), 6);
    assert_eq!(zone.vtm_count, 2, "the cohort hard-assigned to the run's sector");
    // The bay enumerates eleven levels — ten storage shelves 350 mm apart from
    // 350 to 3500, and the relay level at 400 wedged between the first two — so
    // its topmost shelf is the list's last entry.
    assert_eq!(zone.zone_height_mm, 3500);
    // Bay centres run from y = 2500 to y = 7500: a 5000 mm span, which is the
    // travel the VTM cycle model normalises its distances over.
    assert_eq!(zone.zone_width_mm, Some(5000));
    assert_eq!(zone.real_shelf_count, 10, "the storage levels, not the relay level with them");
    // Ten storage levels, one column, one deep: ten slot entities per bay, and
    // one dessim rack per slot holding one tote each.
    assert_eq!(world.racking.columns.len(), 60);
    assert_eq!(world.racking.vertical_tote_capacity, 1);
    assert!(zone.slots.is_none(), "shelf heights travel in the racking plan, not in a grid slot table");
}

#[test]
fn a_station_keeps_its_pose_its_presentation_point_and_its_queue() {
    let world = minimal_putaway2();
    assert_eq!(world.map.stations.len(), 2);

    // Station 1: posed at (5500, 500) — grid (5, 0) — presenting at the floor
    // cell the document decorates, grid (4, 0), queued over grid (3, 0).
    let pick = &world.map.stations[0];
    assert_eq!(pick.station_type, StationType::Pick);
    assert_eq!(pick.position, CellId(5));
    assert_eq!(
        pick.presentation_points,
        vec![PresentationPoint { presentation_cell: CellId(4), queue_path: vec![CellId(3)] }]
    );
    assert_eq!(pick.bins, Some(1));

    // Station 2: posed at (5500, 8500) — grid (5, 8) — so its cells are the
    // same three, eight rows up.
    let put = &world.map.stations[1];
    assert_eq!(put.station_type, StationType::Putaway);
    assert_eq!(put.position, CellId(8 * 8 + 5 - 6));
    assert_eq!(
        put.presentation_points,
        vec![PresentationPoint { presentation_cell: CellId(8 * 8 + 4 - 6), queue_path: vec![CellId(8 * 8 + 3 - 6)] }]
    );
}

#[test]
fn the_fleet_is_the_documents_cohorts() {
    let world = minimal_putaway2();
    assert_eq!(world.map.fleet_config().htms, 10, "the free HTM cohort");
    assert_eq!(world.map.fleet_config().rtps, 0);
    assert_eq!(world.map.vtm_count(), 2, "summed from the zones the VTM cohorts are assigned to");
}

#[test]
fn no_cell_of_the_document_lost_its_identity() {
    let world = minimal_putaway2();
    // Every graph cell stands for an entity: a floor cell or a rack bay.
    let named = world.index.cell_entities.len();
    assert_eq!(named, world.map.graph.cell_count());
    // The mapping runs both ways with nothing left over: no entity resolves to
    // a cell that does not stand for it.
    assert_eq!(world.index.cells_by_entity.len(), named);
}

/// The shelf heights are the levels the document enumerates, read straight off
/// the list rather than spread over a pitch.
#[test]
fn every_shelf_sits_where_the_level_list_puts_it() {
    let world = minimal_putaway2();
    let mut heights: Vec<f64> = world.racking.columns.iter().filter_map(|r| r.floor_m).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup();
    assert_eq!(heights, vec![0.35, 0.7, 1.05, 1.4, 1.75, 2.1, 2.45, 2.8, 3.15, 3.5]);
    // The one relay level, 400 mm up, is where the exchange happens.
    assert_eq!(world.racking.board_floors_m.values().collect::<Vec<_>>(), vec![&vec![0.400]]);
}

/// A short run over the loaded map: robots move, handoffs happen at the access
/// cells, and totes change hands. The horizon is deliberately under the point
/// where a pre-existing debug assertion in the SKU ledger
/// (`model/inventory/mod.rs`, `sku_reserved went negative`) fires — it fires on
/// the desmap path too, so chasing it is not this test's business.
#[test]
fn a_run_over_the_loaded_map_moves_robots_and_totes() {
    let world = minimal_putaway2();
    let scenario = ScenarioConfig::Generated(GeneratedScenario {
        num_skus: 10,
        total_inventory_units: 2000,
        num_orders: 100,
        total_order_picks: 200,
        units_per_tote: 1,
        popular_skus_pct: 0.0,
        inventory_popular_share: 0.0,
        order_popular_share: 0.0,
        order_totes: 0,
    });
    let mut config = SimConfig { num_htms: world.map.fleet_config().htms, ..SimConfig::default() };
    config.fidelity = FidelityMode::CollisionDes;
    config.sim_duration = Duration::from_secs(30);
    config.putaway.enabled = true;
    config.queue_factor = 3.0;

    let mut data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config)
        .expect("scenario generation over the loaded map");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding_cells(&data.map),
    ));

    let (capture_tx, capture_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || capture_rx.iter().collect::<Vec<_>>());
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, _metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![capture_tx], recording: None },
    )
    .expect("the run completes");
    let events = capture.join().expect("event capture");

    assert_eq!(result.sim_duration, Duration::from_secs(30));
    let navigations = events.iter().filter(|e| matches!(e, SimEvent::Nav(_))).count();
    let cycles = events.iter().filter(|e| matches!(e, SimEvent::VtmCycle(_))).count();
    let handoffs = events.iter().filter(|e| matches!(e, SimEvent::RelayStateChange(_))).count();
    let totes = events.iter().filter(|e| matches!(e, SimEvent::ToteMoved(_))).count();
    assert!(navigations > 0, "no HTM ever drove anywhere");
    assert!(cycles > 0, "no VTM ever ran a cycle");
    assert!(handoffs > 0, "no relay was ever claimed, so nothing changed hands");
    assert!(totes > 0, "no tote ever moved");
}

/// A station whose queue runs along a ONE-WAY lane must still be enterable.
///
/// The queue discipline lists the one edge allowed into the queue head and
/// `restrict_queue_corridors` deletes every other edge touching a queue or
/// presentation cell. Finding that entry edge by walking the head's OUTGOING
/// arcs works only on a two-way lane: under `flow: one_way` the head has no
/// arc back toward its tail, the entry goes unlisted, and the pruning pass
/// then deletes the real inbound arc -- sealing the station off with no error
/// and no warning, so the run simply completes zero cycles. The entry must
/// therefore be sought among the head's PREDECESSORS.
#[test]
fn a_one_way_lane_still_admits_the_queue() {
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../data/seed/worlds/minimal-putaway2/minimal_putaway2.json"),
        )
        .expect("read example"),
    )
    .expect("parse example");
    let mut doc = doc;
    // Make the floor a one-way lane along +x, the direction both queues run.
    let entities = doc.get_mut("entities").and_then(|e| e.as_array_mut()).expect("entities");
    let mut made_one_way = false;
    for e in entities.iter_mut() {
        let Some(components) = e.get_mut("components").and_then(|c| c.as_object_mut()) else { continue };
        if !components.contains_key("tiling") {
            continue;
        }
        components.insert("flow".into(), serde_json::json!({ "rule": "one_way", "direction": "+x" }));
        made_one_way = true;
    }
    assert!(made_one_way, "the example must have a tiled floor to make one-way");

    let dir = std::env::temp_dir().join("dessim-one-way-queue");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("one_way.json");
    std::fs::write(&path, serde_json::to_vec(&doc).expect("re-serialize")).expect("write doc");

    let world = dessim::worldmap::load(&path).expect("the one-way document loads");
    let map = &world.map;
    assert!(!map.stations.is_empty(), "the example has stations");

    for (station, point) in map.stations.iter().flat_map(|s| s.presentation_points.iter().map(move |p| (s, p))) {
        let Some(&head) = point.queue_path.first() else { continue };
        let inbound: Vec<u32> = map
            .graph
            .cells()
            .filter(|c| map.graph.neighbors(c.id).iter().any(|(n, _)| *n == head))
            .map(|c| c.id.0)
            .collect();
        assert!(
            !inbound.is_empty(),
            "station {:?}: queue head {} has no inbound edge on a one-way lane -- the station is sealed off",
            station.id,
            head.0,
        );
        // And the presentation cell stays reachable through the queue.
        let reaches_presentation =
            map.graph.neighbors(head).iter().any(|(n, _)| *n == point.presentation_cell) || point.queue_path.len() > 1;
        assert!(reaches_presentation, "station {:?}: queue head cannot reach presentation", station.id);
    }
}

/// The order buffer: racking of the same steel, holding order containers
/// instead of inventory, worked by gripper VTMs instead of vacuum heads.
///
/// The protocol already carries both facts and neither had a reader. `storage`
/// says what a rack holds (`inventory` or `order`, SPEC's own vocabulary), and
/// a cohort's `robot_class.configuration` says which machine works the aisle it
/// is assigned to. Without this the two racks are indistinguishable, inventory
/// seeding fills the buffer, and a gripper is asked to do a vacuum head's job.
#[test]
fn an_order_buffer_is_a_zone_that_holds_orders_and_is_worked_by_grippers() {
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../data/seed/worlds/minimal-putaway2/minimal_putaway2.json"),
        )
        .expect("read example"),
    )
    .expect("parse example");
    let mut doc = doc;

    // Turn the example's one rack row into an order buffer worked by a gripper.
    let gripper_class = "11111111-2222-3333-4444-555555555555";
    let entities = doc.get_mut("entities").and_then(|e| e.as_array_mut()).expect("entities");
    let mut declared_buffer = false;
    for e in entities.iter_mut() {
        let Some(components) = e.get_mut("components").and_then(|c| c.as_object_mut()) else { continue };
        // The buffer IS the inventory rack template with what it holds
        // replaced, which is how the protocol intends it: the component sits
        // on the template's rack local and every stamped bay inherits it.
        if let Some(locals) =
            components.get_mut("template").and_then(|t| t.get_mut("entities")).and_then(|e| e.as_array_mut())
        {
            for local in locals.iter_mut() {
                let Some(lc) = local.get_mut("components").and_then(|c| c.as_object_mut()) else { continue };
                if lc.contains_key("rack") {
                    lc.insert("storage".into(), serde_json::json!({ "holds": "order" }));
                    declared_buffer = true;
                }
            }
        }
        // The VTM cohort becomes a gripper cohort.
        if let Some(cohort) = components.get_mut("cohort").and_then(|c| c.as_object_mut()) {
            if cohort.get("assignment").and_then(|a| a.get("kind")).and_then(|k| k.as_str()) == Some("hard") {
                cohort.insert("robot_class".into(), serde_json::json!(gripper_class));
            }
        }
    }
    assert!(declared_buffer, "the example must have racking to convert");
    entities.push(serde_json::json!({
        "id": gripper_class,
        "name": "VTM (gripper)",
        "components": { "robot_class": {
            "role": "vtm", "model": "VTM", "configuration": "gripper", "carries": ["tote"]
        } }
    }));

    let dir = std::env::temp_dir().join("dessim-order-buffer");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("buffer.json");
    std::fs::write(&path, serde_json::to_vec(&doc).expect("re-serialize")).expect("write doc");

    let world = dessim::worldmap::load(&path).expect("the buffer document loads");
    let buffers: Vec<_> = world.map.zones.iter().filter(|z| z.holds == ZoneHolds::Order).collect();
    assert!(!buffers.is_empty(), "the buffer aisle must be distinguishable from inventory racking");
    for zone in &buffers {
        assert_eq!(
            zone.vtm_configuration.as_deref(),
            Some("gripper"),
            "a buffer aisle is worked by the hardware its cohort declares"
        );
        assert!(zone.vtm_count > 0, "the buffer's VTMs are deployed like any other zone's");
    }

    // And the baseline still reads as inventory, so nothing was reclassified
    // by accident.
    let plain = minimal_putaway2();
    assert!(
        plain.map.zones.iter().all(|z| z.holds == ZoneHolds::Inventory),
        "a document that says nothing about what it holds is inventory, as every map was before buffers"
    );
    assert!(
        plain.map.zones.iter().all(|z| z.vtm_configuration.as_deref() == Some("standard")),
        "the unmodified example's VTMs are the standard vacuum configuration"
    );
}

/// A caller that read a world file to tell what it is hands worldmap what it
/// read, and nothing reads the file again: the map builds from the parsed
/// document with no file at the path it names.
#[test]
fn a_world_file_read_once_is_not_read_again() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data/seed/worlds/minimal-putaway2/minimal_putaway2.json");
    let file: Value = serde_json::from_slice(&std::fs::read(&path).expect("the document")).expect("JSON");
    let nowhere = path.with_file_name("read-once-nowhere.json");
    assert!(!nowhere.exists());
    let read = worldmap::load_parsed(&nowhere, &file).expect("the map, from what was read");
    assert_eq!(kinds(&read), kinds(&minimal_putaway2()));
}
