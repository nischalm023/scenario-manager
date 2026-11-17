//! Chassis heading at a no-turn cell before an HTM has an owned frontier.
//! Fleet seeds and future relay pickups use the same unique forward ingress.

use desgraph::{CellId, HtmDriveDirection, HtmTraversal};
use desmap::WarehouseMap;
use petgraph::Direction::Incoming;
use petgraph::visit::EdgeRef;

use crate::error::InvalidScenario;
use crate::pathfinding::{DriveModel, Orientation, arrival_orientation};

/// The heading an HTM without a frontier stands with on `cell`: East where it may
/// turn in place, otherwise the heading it arrives with over the cell's one
/// forward ingress, as `drive` (an HTM's) drives that edge.
pub fn unassigned_htm_orientation(
    map: &WarehouseMap,
    cell: CellId,
    drive: &dyn DriveModel,
) -> Result<Orientation, InvalidScenario> {
    let target = map.graph.cell(cell).ok_or_else(|| InvalidScenario::HtmHeading {
        cell_id: cell.0,
        reason: format!("robot starts at unknown cell {cell:?}"),
    })?;
    if !target.htm_rotation_forbidden {
        return Ok(Orientation::East);
    }
    let index = map.graph.node_index(cell).expect("known seed cell has a graph index");
    let mut approaches: Vec<_> = map
        .graph
        .inner()
        .edges_directed(index, Incoming)
        .filter_map(|incoming| {
            let edge = incoming.weight();
            (edge.htm_drive == HtmDriveDirection::Forward
                && edge.traversability.htm_traversal != HtmTraversal::None
                && edge.dest_only.is_none_or(|gate| gate.admits(cell))
                && edge.vtm_zone_gate.is_none_or(|zone| zone == target.zone))
            .then_some(map.graph.inner()[incoming.source()].id)
        })
        .collect();
    approaches.sort_unstable();
    let [from] = approaches.as_slice() else {
        return Err(InvalidScenario::HtmHeading {
            cell_id: cell.0,
            reason: format!("no-turn HTM seed at {cell:?} needs one forward ingress, found {approaches:?}"),
        });
    };
    arrival_orientation(&map.graph, *from, cell, drive).ok_or_else(|| InvalidScenario::HtmHeading {
        cell_id: cell.0,
        reason: format!("cannot derive HTM ingress heading from {from:?} to {cell:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SimDuration;
    use crate::pathfinding::{Constant, HtmKinematics};
    use desgraph::{Cell, CellKind, Edge, WarehouseGraphBuilder, WorldPos, ZoneId};
    use desmap::{SystemType, WarehouseMapBuilder};

    fn htm() -> Constant {
        Constant::htm(&HtmKinematics::uniform(1000, SimDuration::seconds(1)), false)
    }

    fn map(no_turn: bool, second_ingress: Option<HtmTraversal>) -> WarehouseMap {
        let mut graph = WarehouseGraphBuilder::new();
        for (id, x_mm, y_mm) in [(0, 0.0, 0.0), (1, 0.0, 1000.0), (2, 1000.0, 0.0)] {
            graph.add_cell(Cell {
                id: CellId(id),
                kind: CellKind::Open,
                zone: ZoneId(1),
                sector: None,
                grid_pos: None,
                world_pos: Some(WorldPos { x_mm, y_mm }),
                barcode: None,
                rack_orientation: None,
                station_id: None,
                htm_rotation_forbidden: id == 0 && no_turn,
            });
        }
        graph.add_edge(CellId(1), CellId(0), Edge::uniform(1000));
        let mut reverse = Edge::uniform(1000);
        reverse.htm_drive = HtmDriveDirection::Reverse;
        graph.add_edge(CellId(0), CellId(1), reverse);
        if let Some(traversal) = second_ingress {
            let mut edge = Edge::uniform(1000);
            edge.traversability.htm_traversal = traversal;
            graph.add_edge(CellId(2), CellId(0), edge);
        }
        WarehouseMapBuilder::new(SystemType::Relay).graph(graph.build()).build()
    }

    #[test]
    fn no_turn_frontier_uses_forward_ingress_heading_and_ignores_vtm_only_edges() {
        let constrained = map(true, Some(HtmTraversal::None));
        assert_eq!(unassigned_htm_orientation(&constrained, CellId(0), &htm()).unwrap(), Orientation::North);
        assert_eq!(unassigned_htm_orientation(&constrained, CellId(1), &htm()).unwrap(), Orientation::East);
        assert_eq!(unassigned_htm_orientation(&map(false, None), CellId(0), &htm()).unwrap(), Orientation::East);
    }

    #[test]
    fn ambiguous_no_turn_frontier_is_rejected() {
        let error = unassigned_htm_orientation(&map(true, Some(HtmTraversal::Both)), CellId(0), &htm()).unwrap_err();
        assert!(matches!(error, InvalidScenario::HtmHeading { cell_id: 0, .. }));
    }
}
