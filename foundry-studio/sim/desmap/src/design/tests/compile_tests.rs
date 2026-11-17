//! Within `design::tests`, `compile_tests` covers `WarehouseDesign::compile`:
//! cell stamping, edge rules, station queues, clips/charging, and the
//! materialization of rack placements into per-slot zone geometry.

use crate::WarehouseMap;
use crate::design::*;
use crate::graph::DestGate;
use crate::types::*;

#[test]
fn edge_exceptions() {
    let ast = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(3, 3, 1000.0),
        highways: vec![
            HighwayDefinition { origin: [0, 1], width: 3, height: 1, direction: LaneDirection::Bidirectional },
            HighwayDefinition { origin: [1, 0], width: 1, height: 3, direction: LaneDirection::Bidirectional },
        ],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [1, 0],
            edge: None,
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![EdgeException {
            from: [1, 0],
            to: [1, 1],
            traversability: EdgeTraversability {
                vtm_traversal: true,
                htm_traversal: HtmTraversal::UnladenOnly,
                rtp_traversal: true,
            },
            distance_mm: None,
        }],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    };
    let map = ast.compile().unwrap();
    assert_eq!(map.graph.cell_count(), 9);
    let edge = map.graph.edge_between(CellId(1), CellId(4)).unwrap();
    assert_eq!(edge.traversability.htm_traversal, HtmTraversal::UnladenOnly);
    match map.graph.edge_between(CellId(4), CellId(1)) {
        Some(e) => assert_eq!(e.traversability.htm_traversal, HtmTraversal::None),
        None => {}
    }
}

#[test]
fn junction_cell() {
    let ast = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(2, 2, 1000.0),
        highways: vec![],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [1, 0],
            edge: None,
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![JunctionCell {
            id: 100,
            world_pos: [5000.0, 5000.0],
            kind: CellKind::Junction,
            zone: 0,
            edges: vec![JunctionEdge {
                target: 0,
                traversability: EdgeTraversability::fully_traversable(),
                distance_mm: 1500,
                bidirectional: true,
            }],
        }],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    };
    let map = ast.compile().unwrap();
    assert_eq!(map.graph.cell_count(), 5);
    let jc = map.graph.cell(CellId(100)).unwrap();
    assert_eq!(jc.kind, CellKind::Junction);
    assert!(jc.grid_pos.is_none());
    assert_eq!(map.graph.edge_between(CellId(100), CellId(0)).unwrap().distance_mm, 1500);
    assert_eq!(map.graph.edge_between(CellId(0), CellId(100)).unwrap().distance_mm, 1500);
}

#[test]
fn eastbound_highway() {
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(10, 5, 1000.0),
        highways: vec![HighwayDefinition { origin: [0, 1], width: 10, height: 1, direction: LaneDirection::Eastbound }],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [0, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    }
    .compile()
    .unwrap();
    let w = 10u32;

    for col in 0..9u32 {
        let l = CellId(w + col);
        let r = CellId(w + col + 1);
        if let Some(e) = map.graph.edge_between(l, r) {
            assert_ne!(e.traversability.htm_traversal, HtmTraversal::None, "eastward col {}", col);
        }
        if let Some(e) = map.graph.edge_between(r, l) {
            assert_eq!(e.traversability.htm_traversal, HtmTraversal::None, "westward col {}", col);
        }
    }
    // Vertical edges on eastbound lane remain open.
    if let Some(e) = map.graph.edge_between(CellId(w + 5), CellId(5)) {
        assert_ne!(e.traversability.htm_traversal, HtmTraversal::None);
    }
}

#[test]
fn westbound_highway() {
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(10, 5, 1000.0),
        highways: vec![HighwayDefinition { origin: [0, 1], width: 10, height: 1, direction: LaneDirection::Westbound }],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [0, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    }
    .compile()
    .unwrap();
    let w = 10u32;

    for col in 0..9u32 {
        let l = CellId(w + col);
        let r = CellId(w + col + 1);
        if let Some(e) = map.graph.edge_between(l, r) {
            assert_eq!(e.traversability.htm_traversal, HtmTraversal::None, "eastward blocked");
        }
        if let Some(e) = map.graph.edge_between(r, l) {
            assert_ne!(e.traversability.htm_traversal, HtmTraversal::None, "westward open");
        }
    }
}

#[test]
fn station_access_dest_only() {
    let w = 12u32;
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(w, 5, 1000.0),
        highways: vec![
            HighwayDefinition { origin: [0, 1], width: 12, height: 1, direction: LaneDirection::Eastbound },
            HighwayDefinition { origin: [0, 2], width: 12, height: 1, direction: LaneDirection::Bidirectional },
        ],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [5, 0],
            presentation_cell: [5, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    }
    .compile()
    .unwrap();
    let acc = CellId(w + 5);

    // Eastward entry (matches lane): dest_only set.
    if let Some(e) = map.graph.edge_between(CellId(w + 4), acc) {
        assert_eq!(e.dest_only, Some(DestGate::Cell(acc)));
    }
    // Westward entry (against lane): blocked.
    if let Some(e) = map.graph.edge_between(CellId(w + 6), acc) {
        assert_eq!(e.traversability.htm_traversal, HtmTraversal::None);
    }
    // Vertical entry from non-station: blocked.
    match map.graph.edge_between(CellId(2 * w + 5), acc) {
        Some(e) => assert_eq!(e.traversability.htm_traversal, HtmTraversal::None),
        None => {}
    }
    // Entry from station position: no dest_only.
    if let Some(e) = map.graph.edge_between(CellId(5), acc) {
        assert_eq!(e.dest_only, None);
    }
    // Exits: no dest_only.
    if let Some(e) = map.graph.edge_between(acc, CellId(2 * w + 5)) {
        assert_eq!(e.dest_only, None);
    }
    if let Some(e) = map.graph.edge_between(acc, CellId(w + 6)) {
        assert_eq!(e.dest_only, None);
    }
}

#[test]
fn north_south_only_direction() {
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(10, 5, 1000.0),
        highways: vec![HighwayDefinition {
            origin: [0, 1],
            width: 10,
            height: 1,
            direction: LaneDirection::NorthSouthOnly,
        }],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [0, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    }
    .compile()
    .unwrap();
    let w = 10u32;

    // Horizontal edges on row 1: both blocked.
    for col in 0..w - 1 {
        let l = CellId(w + col);
        let r = CellId(w + col + 1);
        assert_eq!(map.graph.edge_between(l, r).unwrap().traversability.htm_traversal, HtmTraversal::None);
        assert_eq!(map.graph.edge_between(r, l).unwrap().traversability.htm_traversal, HtmTraversal::None);
    }
    // Vertical edges (skip col 0 = station access): open.
    for col in 1..w {
        if let Some(e) = map.graph.edge_between(CellId(col), CellId(w + col)) {
            assert_ne!(e.traversability.htm_traversal, HtmTraversal::None, "south col {}", col);
        }
        if let Some(e) = map.graph.edge_between(CellId(2 * w + col), CellId(w + col)) {
            assert_ne!(e.traversability.htm_traversal, HtmTraversal::None, "north col {}", col);
        }
    }
}

#[test]
fn vertical_one_way_columns_enforced_without_horizontal_blocking() {
    let (w, d) = (6u32, 6u32);
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(w, d, 1000.0),
        highways: vec![
            HighwayDefinition { origin: [2, 0], width: 1, height: d, direction: LaneDirection::Northbound },
            HighwayDefinition { origin: [3, 0], width: 1, height: d, direction: LaneDirection::Southbound },
        ],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [0, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    }
    .compile()
    .unwrap();

    for row in 0..d - 1 {
        let n_top = CellId(row * w + 2);
        let n_bot = CellId((row + 1) * w + 2);
        assert_eq!(
            map.graph.edge_between(n_top, n_bot).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "northbound southward blocked row {}",
            row
        );
        assert_ne!(
            map.graph.edge_between(n_bot, n_top).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "northbound northward open row {}",
            row
        );

        let s_top = CellId(row * w + 3);
        let s_bot = CellId((row + 1) * w + 3);
        assert_ne!(
            map.graph.edge_between(s_top, s_bot).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "southbound southward open row {}",
            row
        );
        assert_eq!(
            map.graph.edge_between(s_bot, s_top).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "southbound northward blocked row {}",
            row
        );
    }

    for row in 0..d {
        for (left_col, right_col) in [(1u32, 2u32), (2u32, 3u32), (3u32, 4u32)] {
            let left = CellId(row * w + left_col);
            let right = CellId(row * w + right_col);
            assert_ne!(
                map.graph.edge_between(left, right).unwrap().traversability.htm_traversal,
                HtmTraversal::None,
                "eastward open row {} cols {}->{}",
                row,
                left_col,
                right_col
            );
            assert_ne!(
                map.graph.edge_between(right, left).unwrap().traversability.htm_traversal,
                HtmTraversal::None,
                "westward open row {} cols {}->{}",
                row,
                right_col,
                left_col
            );
        }
    }
}

#[test]
fn partial_height_vertical_columns_keep_lane_cells_traversable() {
    let (w, d) = (40u32, 30u32);
    let (origin_row, lane_height) = (8u32, 12u32);
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(w, d, 1000.0),
        highways: vec![
            HighwayDefinition { origin: [0, 4], width: w, height: 2, direction: LaneDirection::Eastbound },
            HighwayDefinition { origin: [0, 6], width: w, height: 1, direction: LaneDirection::NorthSouthOnly },
            HighwayDefinition { origin: [0, 7], width: w, height: 2, direction: LaneDirection::Westbound },
            HighwayDefinition {
                origin: [10, origin_row],
                width: 1,
                height: lane_height,
                direction: LaneDirection::Northbound,
            },
            HighwayDefinition {
                origin: [11, origin_row],
                width: 1,
                height: lane_height,
                direction: LaneDirection::Southbound,
            },
        ],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [0, 1],
            edge: Some(StationEdge::Bottom),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    }
    .compile()
    .unwrap();

    for row in origin_row..origin_row + lane_height - 1 {
        let n_top = CellId(row * w + 10);
        let n_bot = CellId((row + 1) * w + 10);
        assert_eq!(
            map.graph.edge_between(n_top, n_bot).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "northbound southward blocked row {}",
            row
        );
        assert_ne!(
            map.graph.edge_between(n_bot, n_top).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "northbound northward open row {}",
            row
        );

        let s_top = CellId(row * w + 11);
        let s_bot = CellId((row + 1) * w + 11);
        assert_ne!(
            map.graph.edge_between(s_top, s_bot).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "southbound southward open row {}",
            row
        );
        assert_eq!(
            map.graph.edge_between(s_bot, s_top).unwrap().traversability.htm_traversal,
            HtmTraversal::None,
            "southbound northward blocked row {}",
            row
        );
    }

    for row in origin_row..origin_row + lane_height {
        for col in [10u32, 11u32] {
            let id = CellId(row * w + col);
            let traversable_out = map
                .graph
                .neighbors(id)
                .iter()
                .filter(|(_, e)| e.traversability.htm_traversal != HtmTraversal::None)
                .count();
            assert!(traversable_out > 0, "cell ({},{}) has zero traversable outbound edges", col, row);
        }
    }
}

/// Non-uniform row pitch: the cencora rack geometry, 6 units of 2140mm holding
/// 3 relays at 645mm centers with 425mm to each unit edge, so pitch alternates
/// 645, 645, 850 along a rack column. A uniform grid cannot land on that at any
/// pitch, which is why `row_centers_mm` exists.
#[test]
fn row_centers_drive_world_pos_and_edge_distances() {
    let w = 3u32;
    // 6 rack units x 3 relays = 18 rows, exactly the relay count the uniform
    // model already generated. Only the spacing was ever wrong.
    let centers: Vec<f64> = (0..6)
        .flat_map(|u| {
            let b = u as f64 * 2140.0;
            [b + 425.0, b + 1070.0, b + 1715.0]
        })
        .collect();
    assert_eq!(centers.len(), 18);

    let ast = WarehouseDesign {
        version: 3,
        grid: GridDefinition { width: w, depth: 18, cell_size_mm: 850.0, row_centers_mm: Some(centers.clone()) },
        highways: vec![],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [1, 0],
            edge: None,
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    };
    let map = ast.compile().expect("compile");

    // world_pos y comes from the center list; x stays on the uniform column pitch.
    for (row, &want_y) in centers.iter().enumerate() {
        let cell = map.graph.cell(CellId(row as u32 * w)).expect("cell");
        let wp = cell.world_pos.expect("world_pos");
        assert_eq!(wp.y_mm, want_y, "row {row} center");
        assert_eq!(wp.x_mm, 425.0, "row {row} x should stay uniform (850/2)");
    }

    // Vertical edge distances are the real alternating pitch.
    for row in 0..17u32 {
        let a = CellId(row * w);
        let b = CellId((row + 1) * w);
        let edge = map.graph.edge_between(a, b).unwrap_or_else(|| panic!("edge {row}->{}", row + 1));
        let want = if row % 3 == 2 { 850 } else { 645 };
        assert_eq!(edge.distance_mm, want, "vertical edge {row}->{}", row + 1);
    }

    // Horizontal edges are untouched by row geometry.
    let h = map.graph.edge_between(CellId(0), CellId(1)).expect("horizontal edge");
    assert_eq!(h.distance_mm, 850);

    // Round-trip: extract recovers the centers from world positions and does NOT
    // re-encode the row pitch as redundant per-edge exceptions.
    let design2 = crate::design::extract(&map).expect("extract");
    assert_eq!(design2.grid.row_centers_mm.as_deref(), Some(centers.as_slice()));
    let vertical_exceptions =
        design2.edge_exceptions.iter().filter(|e| e.from[0] == e.to[0] && e.distance_mm.is_some()).count();
    assert_eq!(vertical_exceptions, 0, "row pitch must live in row_centers_mm, not edge_exceptions");
}

/// A uniform design must still round-trip without gaining a redundant center
/// vector, so existing fixtures are untouched by this feature.
#[test]
fn uniform_grid_round_trips_without_row_centers() {
    let ast = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(4, 4, 1000.0),
        highways: vec![],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [1, 0],
            edge: None,
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    };
    let map = ast.compile().expect("compile");
    let design2 = crate::design::extract(&map).expect("extract");
    assert_eq!(design2.grid.row_centers_mm, None);
    assert_eq!(map.graph.edge_between(CellId(0), CellId(4)).expect("vertical").distance_mm, 1000);
}

#[test]
fn bad_row_centers_are_rejected_at_compile() {
    let base = |centers: Option<Vec<f64>>| WarehouseDesign {
        version: 3,
        grid: GridDefinition { width: 2, depth: 3, cell_size_mm: 1000.0, row_centers_mm: centers },
        highways: vec![],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [0, 0],
            presentation_cell: [1, 0],
            edge: None,
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![],
        fleet: None,
    };
    let err = base(Some(vec![0.0, 500.0])).compile().unwrap_err();
    assert!(err.contains("2 entries") && err.contains("is 3"), "{err}");
    let err = base(Some(vec![0.0, 500.0, 500.0])).compile().unwrap_err();
    assert!(err.contains("strictly increasing"), "{err}");
}

// -- rack prototypes + placements --

/// Compile a design with a top-level `racks` table and one Dual relay zone
/// at origin [1,1], length 3, on an 8×6 grid.
fn prototype_design(racks_json: &str, zone_extras: &str) -> Result<WarehouseMap, String> {
    let json = format!(
        r#"{{"version":3,"grid":{{"width":8,"depth":6,"cell_size_mm":1000.0}},
            "racks":{racks_json},
            "zones":[{{"type":"relay","id":1,"origin":[1,1],"length":3{zone_extras}}}],
            "stations":[]}}"#
    );
    let design: WarehouseDesign = serde_json::from_str(&json).expect("parse design");
    design.compile()
}

/// One three-storage-slot rack spanning xs 0/1000/2000 (plus a board so the
/// relay model has an exchange height).
const SIMPLE_RACK: &str = r#"[{"name":"bay","length_mm":2000.0,"slots":[
    {"x":0.0,"y":394.0,"diving_board":true,"diving_board_slot":0},
    {"x":0.0,"y":1580.0},{"x":1000.0,"y":1580.0},{"x":2000.0,"y":1580.0}]}]"#;

#[test]
fn materialized_rows_follow_nearest_cell_rule() {
    let map = prototype_design(SIMPLE_RACK, r#","rack_placements":[{"rack":"bay","side":"Left"}]"#).unwrap();
    let zone = map.zones.iter().find(|z| z.id.0 == 1).unwrap();
    let slots = zone.slots.as_ref().unwrap();
    // Storage xs 0/1000/2000 over span [0,2000] → t 0/0.5/1 → rows 0/1/2,
    // absolute cells from origin [1,1] on the Left rack column (col 1).
    let storage: Vec<_> = slots.iter().filter(|s| !s.diving_board).collect();
    assert_eq!(storage.iter().map(|s| s.cell).collect::<Vec<_>>(), vec![[1, 1], [1, 2], [1, 3]]);
    // The board at x=0 anchors to row 0.
    let board = slots.iter().find(|s| s.diving_board).unwrap();
    assert_eq!(board.cell, [1, 1]);
    assert_eq!(board.diving_board_slot, Some(0));
}

/// Anchoring reads row *positions*, not row *indices*. With `row_centers_mm`
/// the two disagree, and the position is the one that matches the steel.
///
/// Rows 1..4 sit at 1000/1200/1400/5000 — three rows bunched at one end and a
/// long reach to the last, the pathological version of cencora's 645/645/850.
/// Slots at x 0/100/2000 anchor onto that extent at 1000/1200/5000mm, so the
/// middle slot belongs in row 2, whose center it lands on exactly. Anchoring by
/// index would compute `round(0.05 * 3) = 0` and put it back in row 1, 200mm
/// from where the slot physically is.
#[test]
fn slot_rows_follow_row_centers_not_row_indices() {
    let json = r#"{"version":3,
        "grid":{"width":8,"depth":6,"cell_size_mm":1000.0,
                "row_centers_mm":[200.0,1000.0,1200.0,1400.0,5000.0,6000.0]},
        "racks":[{"name":"bay","length_mm":2000.0,"slots":[
            {"x":0.0,"y":394.0,"diving_board":true,"diving_board_slot":0},
            {"x":0.0,"y":1580.0},{"x":100.0,"y":1580.0},{"x":2000.0,"y":1580.0}]}],
        "zones":[{"type":"relay","id":1,"origin":[1,1],"length":4,
                  "rack_placements":[{"rack":"bay","side":"Left"}]}],
        "stations":[]}"#;
    let design: WarehouseDesign = serde_json::from_str(json).expect("parse design");
    let map = design.compile().unwrap();
    let zone = map.zones.iter().find(|z| z.id.0 == 1).unwrap();
    let storage: Vec<_> = zone.slots.as_ref().unwrap().iter().filter(|s| !s.diving_board).collect();
    assert_eq!(
        storage.iter().map(|s| (s.cell, s.x)).collect::<Vec<_>>(),
        vec![([1, 1], 0.0), ([1, 2], 100.0), ([1, 4], 2000.0)],
        "the x=100 slot belongs in row 2 (center 1200), not row 1 (center 1000)"
    );
}

#[test]
fn degenerate_x_span_materializes_to_middle_row() {
    let racks = r#"[{"name":"bay","length_mm":1400.0,"slots":[{"x":700.0,"y":1580.0}]}]"#;
    let map = prototype_design(racks, r#","rack_placements":[{"rack":"bay","side":"Left"}]"#).unwrap();
    let zone = map.zones.iter().find(|z| z.id.0 == 1).unwrap();
    // Zero span → t = 0.5 → round(0.5 * 2) = 1 (ties away from zero).
    assert_eq!(zone.slots.as_ref().unwrap()[0].cell, [1, 2]);
}

#[test]
fn x_offset_shifts_slots_into_the_zone_frame() {
    // Two stampings of a one-slot bay: offsets 0 and 2000 span the run, so
    // the copies anchor to the first and last rows.
    let racks = r#"[{"name":"bay","length_mm":1000.0,"slots":[{"x":0.0,"y":1580.0}]}]"#;
    let map = prototype_design(
        racks,
        r#","rack_placements":[{"rack":"bay","side":"Left"},
                               {"rack":"bay","side":"Left","x_offset_mm":2000.0}]"#,
    )
    .unwrap();
    let zone = map.zones.iter().find(|z| z.id.0 == 1).unwrap();
    let slots = zone.slots.as_ref().unwrap();
    // Placements materialize in authored order; x carries the offset.
    assert_eq!(slots.iter().map(|s| (s.cell, s.x)).collect::<Vec<_>>(), vec![([1, 1], 0.0), ([1, 3], 2000.0)]);
}

#[test]
fn dual_sides_map_to_their_rack_columns() {
    let map = prototype_design(
        SIMPLE_RACK,
        r#","rack_placements":[{"rack":"bay","side":"Left"},{"rack":"bay","side":"Right"}]"#,
    )
    .unwrap();
    let zone = map.zones.iter().find(|z| z.id.0 == 1).unwrap();
    let slots = zone.slots.as_ref().unwrap();
    assert_eq!(slots.len(), 8);
    // Left placement first (col 1 = origin+0), then Right (col 3 = origin+2).
    assert!(slots[..4].iter().all(|s| s.cell[0] == 1));
    assert!(slots[4..].iter().all(|s| s.cell[0] == 3));
}

#[test]
fn placement_side_must_match_sidedness() {
    let err = prototype_design(SIMPLE_RACK, r#","sides":"LeftOnly","rack_placements":[{"rack":"bay","side":"Right"}]"#)
        .unwrap_err();
    assert!(err.contains("not a rack side"), "{err}");
}

#[test]
fn unknown_rack_reference_errors() {
    let err = prototype_design(SIMPLE_RACK, r#","rack_placements":[{"rack":"nope","side":"Left"}]"#).unwrap_err();
    assert!(err.contains("unknown rack \"nope\""), "{err}");
}

#[test]
fn duplicate_rack_names_error() {
    let racks = r#"[{"name":"bay","length_mm":1000.0,"slots":[{"x":0.0,"y":1580.0}]},
                    {"name":"bay","length_mm":1000.0,"slots":[{"x":0.0,"y":1580.0}]}]"#;
    let err = prototype_design(racks, r#","rack_placements":[{"rack":"bay","side":"Left"}]"#).unwrap_err();
    assert!(err.contains("duplicate rack name"), "{err}");
}

#[test]
fn slots_must_stay_inside_the_rack_envelope() {
    // Bottom-left frame: x measures from the steel's left end; a slot past
    // length_mm hangs off the frame.
    let racks = r#"[{"name":"bay","length_mm":1000.0,"slots":[{"x":1200.0,"y":1580.0}]}]"#;
    let err = prototype_design(racks, r#","rack_placements":[{"rack":"bay","side":"Left"}]"#).unwrap_err();
    assert!(err.contains("outside the rack envelope"), "{err}");
}

#[test]
fn overlapping_placements_error_but_touching_tiles() {
    let racks = r#"[{"name":"bay","length_mm":1000.0,"slots":[{"x":0.0,"y":1580.0}]}]"#;
    // Touching: consecutive offsets exactly length_mm apart (shared legs).
    prototype_design(
        racks,
        r#","rack_placements":[{"rack":"bay","side":"Left"},
                               {"rack":"bay","side":"Left","x_offset_mm":1000.0}]"#,
    )
    .unwrap();
    // Overlap on the same side: steel through steel.
    let err = prototype_design(
        racks,
        r#","rack_placements":[{"rack":"bay","side":"Left"},
                               {"rack":"bay","side":"Left","x_offset_mm":600.0}]"#,
    )
    .unwrap_err();
    assert!(err.contains("overlap"), "{err}");
    // The same offsets on opposite sides are two different runs: fine.
    prototype_design(
        racks,
        r#","rack_placements":[{"rack":"bay","side":"Left"},
                               {"rack":"bay","side":"Right","x_offset_mm":600.0}]"#,
    )
    .unwrap();
}

#[test]
fn placed_zone_needs_a_storage_slot() {
    let racks = r#"[{"name":"boards","length_mm":1000.0,"slots":[{"x":0.0,"y":394.0,"diving_board":true,"diving_board_slot":0}]}]"#;
    let err = prototype_design(racks, r#","rack_placements":[{"rack":"boards","side":"Left"}]"#).unwrap_err();
    assert!(err.contains("no storage slot"), "{err}");
}

#[test]
fn inline_slots_are_rejected() {
    // The pre-prototype inline form. RelayZone is deny_unknown_fields, so a
    // stale design fails at parse time rather than silently dropping slots.
    let json = r#"{"type":"relay","id":1,"origin":[0,0],"length":3,
        "slots":[{"cell":[0,0],"x":100.0,"y":1580.0}]}"#;
    let err = serde_json::from_str::<ZoneDef>(json).unwrap_err().to_string();
    assert!(err.contains("unknown field `slots`"), "{err}");
}

#[test]
fn placements_participate_in_all_or_none() {
    let json = r#"{"version":3,"grid":{"width":12,"depth":6,"cell_size_mm":1000.0},
        "racks":[{"name":"bay","length_mm":1000.0,"slots":[{"x":0.0,"y":1580.0}]}],
        "zones":[
          {"type":"relay","id":1,"origin":[1,1],"length":3,
           "rack_placements":[{"rack":"bay","side":"Left"}]},
          {"type":"relay","id":2,"origin":[6,1],"length":3}],
        "stations":[]}"#;
    let design: WarehouseDesign = serde_json::from_str(json).unwrap();
    let err = design.compile().unwrap_err();
    assert!(err.contains("all relay zones or none"), "{err}");
}

#[test]
fn effective_vtc_is_one_for_placed_zones() {
    let json = r#"{"type":"relay","id":1,"origin":[0,0],"length":3,"vertical_tote_capacity":8,
        "rack_placements":[{"rack":"bay","side":"Left"}]}"#;
    let ZoneDef::Relay(rz) = serde_json::from_str::<ZoneDef>(json).unwrap() else { panic!() };
    assert_eq!(rz.effective_vertical_tote_capacity(), 1);
}
