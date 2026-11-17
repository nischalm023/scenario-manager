//! Within `graph`, `tests` is the STRUCTURE ITSELF UNDER TEST: that a graph
//! built cell by cell answers the questions its consumers ask of it -- how many
//! cells and arcs it holds, what stands at a grid position, which neighbours a
//! cell has and what each arc weighs -- and that it survives a round trip
//! through the desmap JSON format unchanged.
//!
//! Separate from the pins over real documents, which assert what a particular
//! warehouse's map contains: nothing here loads a document, and every graph is
//! two or three cells long.

#[cfg(test)]
mod tests {
    use crate::graph::{Cell, Edge, HtmDriveDirection, WarehouseGraph, WarehouseGraphBuilder};
    use crate::types::*;

    fn make_cell(id: u32, kind: CellKind, x: u32, y: u32) -> Cell {
        Cell {
            id: CellId(id),
            kind,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x, y }),
            world_pos: None,
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        }
    }

    #[test]
    fn build_and_query_small_graph() {
        let mut b = WarehouseGraphBuilder::new();
        b.add_cell(make_cell(0, CellKind::Open, 0, 0));
        b.add_cell(make_cell(1, CellKind::Storage, 1, 0));
        b.add_edge_bidi(CellId(0), CellId(1), Edge::uniform(1000));

        let g = b.build();
        assert_eq!(g.cell_count(), 2);
        assert_eq!(g.edge_count(), 2); // bidirectional
        assert_eq!(g.cell(CellId(0)).unwrap().kind, CellKind::Open);
        assert_eq!(g.cell_at(GridPos { x: 1, y: 0 }), Some(CellId(1)));

        let nbrs = g.neighbors(CellId(0));
        assert_eq!(nbrs.len(), 1);
        assert_eq!(nbrs[0].0, CellId(1));
        assert_eq!(nbrs[0].1.distance_mm, 1000);
    }

    #[test]
    fn serde_round_trip_graph() {
        let mut b = WarehouseGraphBuilder::new();
        b.add_cell(make_cell(0, CellKind::Highway, 0, 0));
        b.add_cell(make_cell(1, CellKind::Highway, 1, 0));
        b.add_cell(make_cell(2, CellKind::Storage, 2, 0));
        b.add_edge_bidi(CellId(0), CellId(1), Edge::uniform(1380));
        b.add_edge(
            CellId(1),
            CellId(2),
            Edge {
                traversability: EdgeTraversability {
                    vtm_traversal: false,
                    htm_traversal: HtmTraversal::UnladenOnly,
                    rtp_traversal: true,
                },
                distance_mm: 1380,
                dest_only: None,
                vtm_zone_gate: None,
                curve: None,
                htm_drive: Default::default(),
            },
        );
        let g = b.build();

        let json = serde_json::to_string_pretty(&g).unwrap();
        let g2: WarehouseGraph = serde_json::from_str(&json).unwrap();

        assert_eq!(g.cell_count(), g2.cell_count());
        assert_eq!(g.edge_count(), g2.edge_count());
        assert_eq!(g2.cell(CellId(0)).unwrap().kind, CellKind::Highway);
        assert_eq!(
            g2.edge_between(CellId(1), CellId(2)).unwrap().traversability.htm_traversal,
            HtmTraversal::UnladenOnly
        );
        assert!(!g2.edge_between(CellId(1), CellId(2)).unwrap().traversability.vtm_traversal);
    }

    #[test]
    fn older_graphs_default_to_forward_motion_and_relay_flags_round_trip() {
        let mut cell = make_cell(0, CellKind::Relay, 0, 0);
        let old_cell = serde_json::to_value(&cell).unwrap();
        assert!(old_cell.get("htm_rotation_forbidden").is_none());
        assert!(!serde_json::from_value::<Cell>(old_cell).unwrap().htm_rotation_forbidden);
        cell.htm_rotation_forbidden = true;
        let restored: Cell = serde_json::from_value(serde_json::to_value(&cell).unwrap()).unwrap();
        assert!(restored.htm_rotation_forbidden);

        let mut edge = Edge::uniform(1000);
        let old_edge = serde_json::to_value(edge).unwrap();
        assert!(old_edge.get("htm_drive").is_none());
        assert_eq!(serde_json::from_value::<Edge>(old_edge).unwrap().htm_drive, HtmDriveDirection::Forward);
        edge.htm_drive = HtmDriveDirection::Reverse;
        let restored: Edge = serde_json::from_value(serde_json::to_value(edge).unwrap()).unwrap();
        assert_eq!(restored.htm_drive, HtmDriveDirection::Reverse);
    }

    #[test]
    #[should_panic(expected = "duplicate CellId")]
    fn duplicate_cell_panics() {
        let mut b = WarehouseGraphBuilder::new();
        b.add_cell(make_cell(0, CellKind::Open, 0, 0));
        b.add_cell(make_cell(0, CellKind::Open, 1, 0));
    }
}
