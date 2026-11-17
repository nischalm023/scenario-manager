//! Destinations permitted for explicitly indefinite floor holds. Relay access
//! and charging cells qualify; choosing when to use either belongs to tasks.

use std::collections::HashSet;

use desgraph::{CellId, CellKind};
use desmap::WarehouseMap;

use crate::model::parking::usable_cells;

pub fn holding_cells(map: &WarehouseMap) -> HashSet<CellId> {
    let usable = usable_cells(map);
    map.relays
        .iter()
        .map(|relay| relay.htm_access)
        .chain(map.graph.cells().filter(|cell| cell.kind == CellKind::Charging).map(|cell| cell.id))
        .filter(|cell| usable.contains(cell))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use desgraph::{BlockId, Cell, Edge, RelayId, SystemType, WarehouseGraphBuilder, WorldPos, ZoneId};
    use desmap::{BlockSide, RelayPosition, VtmPosition, WarehouseMapBuilder};

    #[test]
    fn only_usable_relay_access_and_actual_chargers_allow_indefinite_holds() {
        let mut graph = WarehouseGraphBuilder::new();
        for (id, kind) in [CellKind::Open, CellKind::Open, CellKind::Charging, CellKind::Relay, CellKind::Charging]
            .into_iter()
            .enumerate()
        {
            let id = id as u32;
            graph.add_cell(Cell {
                id: CellId(id),
                kind,
                zone: ZoneId(0),
                sector: None,
                grid_pos: None,
                world_pos: Some(WorldPos { x_mm: f64::from(id) * 1000.0, y_mm: 0.0 }),
                barcode: None,
                rack_orientation: None,
                station_id: None,
                htm_rotation_forbidden: false,
            });
            if id > 0 && id < 4 {
                graph.add_edge_bidi(CellId(0), CellId(id), Edge::uniform(1000));
            }
        }
        let map = WarehouseMapBuilder::new(SystemType::Relay)
            .graph(graph.build())
            .add_relay(RelayPosition {
                id: RelayId(0),
                position: CellId(3),
                htm_access: CellId(1),
                vtm_access: VtmPosition::Cell(CellId(0)),
                block: BlockId(0),
                side: BlockSide::Left,
            })
            .build();
        assert_eq!(holding_cells(&map), [CellId(1), CellId(2)].into_iter().collect());
    }
}
