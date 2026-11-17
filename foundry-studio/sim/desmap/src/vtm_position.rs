//! VTM coordinates with an existing map cell for event attribution. A physical
//! position does not require a separate cell in the floor traversal graph.

use desgraph::{CellId, WarehouseGraph, WorldPos};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VtmPosition {
    /// Existing grid maps place VTMs at the centre of an aisle cell.
    Cell(CellId),
    /// `anchor` identifies existing rack geometry in reports; it is not a
    /// travel destination or a reservation of the floor at `position`.
    At { position: WorldPos, anchor: CellId },
}

impl VtmPosition {
    pub fn anchor(self) -> CellId {
        match self {
            Self::Cell(cell) | Self::At { anchor: cell, .. } => cell,
        }
    }

    pub fn world_pos(self, graph: &WarehouseGraph) -> Option<WorldPos> {
        match self {
            Self::Cell(cell) => graph.cell(cell).and_then(|cell| cell.world_pos),
            Self::At { position, .. } => Some(position),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn existing_map_cell_ids_keep_their_numeric_json_shape() {
        let start: VtmPosition = serde_json::from_str("49").unwrap();
        assert_eq!(start, VtmPosition::Cell(CellId(49)));
        assert_eq!(serde_json::to_value(start).unwrap(), json!(49));
    }

    #[test]
    fn rail_positions_round_trip_without_becoming_their_anchor() {
        let json = json!({"position": {"x_mm": 7500.0, "y_mm": 850.0}, "anchor": 49});
        let start: VtmPosition = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(start.anchor(), CellId(49));
        assert_eq!(serde_json::to_value(start).unwrap(), json);
    }
}
