//! Relay presentation is a terminal maneuver beneath a shelf: drive in from
//! the adjacent travel lane, exchange the tote, then back out without turning.
//! This projection is shared by legacy designs and world-protocol maps.

use desgraph::{
    CellId, DestGate, HtmDriveDirection, HtmTraversal, RackOrientation, WarehouseGraph, WarehouseGraphBuilder,
};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::RelayPosition;

fn point(graph: &WarehouseGraph, cell: CellId) -> Option<[f64; 2]> {
    let cell = graph.cell(cell)?;
    cell.world_pos.map(|p| [p.x_mm, p.y_mm]).or_else(|| cell.grid_pos.map(|p| [p.x as f64, p.y as f64]))
}

/// Keep each relay's closest perpendicular approach on the side away from its
/// VTM aisle. The existing directed links must support entry and return; this
/// function never invents a connection across intervening floor cells.
pub fn restrict_relay_access(graph: &WarehouseGraph, relays: &[RelayPosition]) -> Result<WarehouseGraph, String> {
    let stands: FxHashSet<_> = relays.iter().map(|relay| relay.htm_access).collect();
    let mut approaches = FxHashMap::default();
    for relay in relays {
        let stand = relay.htm_access;
        let at = point(graph, stand).ok_or_else(|| format!("relay {:?} has no HTM stand coordinates", relay.id))?;
        let aisle = relay
            .vtm_access
            .world_pos(graph)
            .map(|p| [p.x_mm, p.y_mm])
            .or_else(|| point(graph, relay.vtm_access.anchor()))
            .ok_or_else(|| format!("relay {:?} has no VTM aisle coordinates", relay.id))?;
        let axis = match graph.cell(relay.position).and_then(|cell| cell.rack_orientation) {
            Some(RackOrientation::NorthSouth) => 0,
            Some(RackOrientation::EastWest) => 1,
            None => usize::from((aisle[1] - at[1]).abs() > (aisle[0] - at[0]).abs()),
        };
        let inward = aisle[axis] - at[axis];
        if inward.abs() < 0.1 {
            return Err(format!("relay {:?} does not identify a distinct aisle-facing direction", relay.id));
        }
        let mut candidates: Vec<_> = graph
            .neighbors(stand)
            .into_iter()
            .filter_map(|(cell, edge)| {
                if stands.contains(&cell)
                    || edge.traversability.htm_traversal == HtmTraversal::None
                    || edge.curve.is_some()
                {
                    return None;
                }
                let p = point(graph, cell)?;
                if (p[1 - axis] - at[1 - axis]).abs() > 0.11 || (p[axis] - at[axis]) * inward >= 0.0 {
                    return None;
                }
                let entry = graph.neighbors(cell).into_iter().any(|(to, edge)| {
                    to == stand
                        && edge.traversability.htm_traversal != HtmTraversal::None
                        && edge.curve.is_none()
                        && edge.dest_only.is_none_or(|gate| gate.admits(stand))
                });
                entry.then_some(((p[axis] - at[axis]).abs(), cell))
            })
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let Some(&(distance, approach)) = candidates.first() else {
            return Err(format!(
                "relay {:?}, stand {stand:?}, has no perpendicular HTM entry/back-out connection away from its aisle",
                relay.id
            ));
        };
        if candidates.get(1).is_some_and(|next| (next.0 - distance).abs() < 0.1) {
            return Err(format!("relay {:?}, stand {stand:?}, has ambiguous HTM approaches", relay.id));
        }
        if approaches.insert(stand, approach).is_some_and(|previous| previous != approach) {
            return Err(format!("relay stand {stand:?} is shared by incompatible approaches"));
        }
    }
    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        let mut cell = cell.clone();
        if stands.contains(&cell.id) {
            cell.htm_rotation_forbidden = true;
        }
        builder.add_cell(cell);
    }
    for (from, to, edge) in graph.edges() {
        let mut edge = *edge;
        if stands.contains(&from) || stands.contains(&to) {
            if approaches.get(&to) == Some(&from) {
                edge.dest_only = Some(DestGate::Cell(to));
                edge.htm_drive = HtmDriveDirection::Forward;
            } else if approaches.get(&from) == Some(&to) {
                edge.htm_drive = HtmDriveDirection::Reverse;
            } else {
                edge.traversability.htm_traversal = HtmTraversal::None;
            }
        }
        if edge.traversability.htm_traversal != HtmTraversal::None
            || edge.traversability.vtm_traversal
            || edge.traversability.rtp_traversal
        {
            builder.add_edge(from, to, edge);
        }
    }
    Ok(builder.build())
}
