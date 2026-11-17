//! Explicit chargers preserve bay access; retired parking annotations do not create infrastructure.

use desmap::design::types::{
    ChargingZone, GridDefinition, HighwayDefinition, LaneDirection, StationDefinition, WarehouseDesign,
};
use desmap::types::{CardinalDirection, CellId, CellKind, HtmTraversal, StationEdge, StationType};

#[test]
fn edge_charging_access_ignores_partial_row_lane_rules() {
    let map = WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(10, 10, 1000.0),
        highways: vec![HighwayDefinition {
            origin: [5, 6],
            width: 5,
            height: 1,
            direction: LaneDirection::NorthSouthOnly,
        }],
        zones: vec![],
        racks: vec![],
        stations: vec![StationDefinition {
            id: 1,
            position: [1, 4],
            presentation_cell: [1, 5],
            edge: Some(StationEdge::Top),
            queue_path: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            station_type: StationType::Pick,
        }],
        edge_exceptions: vec![],
        junction_cells: vec![],
        clip_boxes: vec![],
        charging_zones: vec![ChargingZone { origin: [0, 6], width: 1, height: 1 }],
        fleet: None,
    }
    .compile()
    .unwrap();
    let w = 10u32;

    // The lane rule starts at x=5, so edge charging outside the highway band
    // keeps its straight-in/out access to the adjacent open cell.
    let charging = CellId(6 * w);
    let access = CellId(6 * w + 1);
    assert_eq!(map.graph.cell(charging).unwrap().kind, CellKind::Charging);
    assert_eq!(map.graph.edge_between(charging, access).unwrap().traversability.htm_traversal, HtmTraversal::Both);
    assert_eq!(map.graph.edge_between(access, charging).unwrap().traversability.htm_traversal, HtmTraversal::Both);

    // Inside the highway rectangle, NorthSouthOnly still blocks horizontal
    // HTM traversal.
    let h_left = CellId(6 * w + 5);
    let h_right = CellId(6 * w + 6);
    assert_eq!(map.graph.edge_between(h_left, h_right).unwrap().traversability.htm_traversal, HtmTraversal::None);
    assert_eq!(map.graph.edge_between(h_right, h_left).unwrap().traversability.htm_traversal, HtmTraversal::None);
}

#[test]
fn explicit_charging_bays_cannot_be_used_as_a_through_lane() {
    let design: WarehouseDesign = serde_json::from_str(
        r#"{
        "version": 3,
        "grid": {"width": 5, "depth": 5, "cell_size_mm": 1000},
        "zones": [], "stations": [{"id": 1, "position": [4, 4], "presentation_cell": [3, 4]}],
        "charging_zones": [{"origin": [0, 1], "width": 1, "height": 2}]
    }"#,
    )
    .unwrap();
    let map = design.compile().unwrap();
    assert_eq!(map.graph.cell(CellId(5)).unwrap().kind, CellKind::Charging);
    assert_eq!(map.graph.cell(CellId(10)).unwrap().kind, CellKind::Charging);
    for (from, to) in [(CellId(5), CellId(10)), (CellId(10), CellId(5))] {
        assert!(
            map.graph.edge_between(from, to).is_none_or(|edge| edge.traversability.htm_traversal == HtmTraversal::None)
        );
    }
    assert_eq!(map.graph.edge_between(CellId(5), CellId(6)).unwrap().traversability.htm_traversal, HtmTraversal::Both);
    assert_eq!(
        map.graph.edge_between(CellId(10), CellId(11)).unwrap().traversability.htm_traversal,
        HtmTraversal::Both
    );
}

#[test]
fn fleet_size_does_not_synthesize_chargers_and_old_parking_regions_stay_floor() {
    let design: WarehouseDesign = serde_json::from_str(
        r#"{
        "version": 3,
        "grid": {"width": 4, "depth": 4, "cell_size_mm": 1000},
        "zones": [], "stations": [], "fleet": {"htms": 50},
        "parking_zones": [{"origin": [0, 0], "width": 1, "height": 3}]
    }"#,
    )
    .unwrap();
    let map = design.compile().unwrap();
    assert!(map.graph.cells().all(|cell| cell.kind == CellKind::Open));
    let serialized = serde_json::to_value(design).unwrap();
    assert!(serialized.get("parking_zones").is_none());
    assert!(serialized.get("charging_zones").is_none());
}

#[test]
fn old_recorded_parking_kind_reads_as_ordinary_floor() {
    let kind: CellKind = serde_json::from_str("\"Parking\"").unwrap();
    assert_eq!(kind, CellKind::Open);
    assert_eq!(serde_json::to_string(&kind).unwrap(), "\"Open\"");
}
