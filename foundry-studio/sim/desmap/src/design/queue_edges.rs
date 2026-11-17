//! Queue path adjacency validation and HTM traversability restriction.

use std::collections::{HashMap, HashSet};

use crate::graph::{DestGate, WarehouseGraph, WarehouseGraphBuilder};
use crate::types::*;

use super::types::StationDefinition;

/// Manhattan distance between two grid positions given as `[col, row]`.
fn manhattan(a: [u32; 2], b: [u32; 2]) -> u32 {
    a[0].abs_diff(b[0]) + a[1].abs_diff(b[1])
}

/// Validate that each station's queue_path forms a contiguous chain of
/// cardinal-adjacent cells ending at the presentation_cell.
///
/// Returns `Err` on the first violation found.
pub(super) fn validate_queue_adjacency(stations: &[StationDefinition]) -> Result<(), String> {
    for sd in stations {
        if sd.queue_path.is_empty() {
            continue;
        }
        // Last queue cell must adjoin presentation_cell.
        let last = sd.queue_path[sd.queue_path.len() - 1];
        if manhattan(last, sd.presentation_cell) != 1 {
            return Err(format!(
                "station {}: last queue cell [{}, {}] is not adjacent to presentation_cell [{}, {}]",
                sd.id, last[0], last[1], sd.presentation_cell[0], sd.presentation_cell[1],
            ));
        }
        // Each consecutive pair must be adjacent.
        for pair in sd.queue_path.windows(2) {
            if manhattan(pair[0], pair[1]) != 1 {
                return Err(format!(
                    "station {}: queue cells [{}, {}] and [{}, {}] are not adjacent",
                    sd.id, pair[0][0], pair[0][1], pair[1][0], pair[1][1],
                ));
            }
        }
    }
    Ok(())
}

/// Enforce strict one-way queue paths and presentation cell access by
/// removing all edges except the allowed forward corridor.
///
/// For each station with a non-empty queue_path, the only edges permitted
/// that touch a queue or presentation cell are:
///
/// 1. **Entry**: one inbound edge into the first queue cell from the
///    tail neighbor (opposite the queue's forward direction).
/// 2. **Forward**: each queue cell to the next (`q[i]` -> `q[i+1]`).
/// 3. **Forward**: last queue cell to the presentation cell.
/// 4. **Exit**: one outbound edge from the presentation cell in the
///    station's `exit_direction`.
///
/// All other edges where either endpoint is a queue or presentation cell
/// are dropped entirely (removed from the graph).
///
/// Entry, forward, and exit edges are tagged with `dest_only = presentation_cell`
/// so only HTMs targeting this station can use them.
///
/// Must be called after `build_initial_graph` but before `narrow_by_reachability`.
pub(super) fn restrict_queue_htm(graph: &WarehouseGraph, stations: &[StationDefinition], w: u32) -> WarehouseGraph {
    // Collected restricted cells: queue cells + presentation cells.
    let mut restricted_cells: HashSet<u32> = HashSet::new();
    // Set of (src, tgt) edges that are allowed (kept with dest_only tag).
    let mut allowed_edges: HashMap<(u32, u32), CellId> = HashMap::new();
    // Set of (src, tgt) edges that are allowed WITHOUT dest_only (exit edges).
    let mut ungated_edges: HashSet<(u32, u32)> = HashSet::new();

    for sd in stations {
        if sd.queue_path.is_empty() {
            continue;
        }
        let pres_id = sd.presentation_cell[1] * w + sd.presentation_cell[0];
        let pres_cell = CellId(pres_id);
        let q_ids: Vec<u32> = sd.queue_path.iter().map(|p| p[1] * w + p[0]).collect();

        for &qid in &q_ids {
            restricted_cells.insert(qid);
        }
        restricted_cells.insert(pres_id);

        // Forward: consecutive queue cells.
        for pair in q_ids.windows(2) {
            allowed_edges.insert((pair[0], pair[1]), pres_cell);
        }
        // Forward: last queue cell -> presentation_cell.
        allowed_edges.insert((q_ids[q_ids.len() - 1], pres_id), pres_cell);

        // Entry: inbound edge into q_ids[0] from the tail neighbor only.
        // The tail is the cell directly opposite q[1] from q[0] — i.e.,
        // the queue extends q[0]->q[1], so approach is from the other side.
        // For single-cell queues, the tail is opposite the presentation cell.
        let q0_pos = sd.queue_path[0];
        let next_pos = if q_ids.len() > 1 { sd.queue_path[1] } else { sd.presentation_cell };
        // Direction from q[0] to next: (dc, dr).
        let dc = next_pos[0] as i32 - q0_pos[0] as i32;
        let dr = next_pos[1] as i32 - q0_pos[1] as i32;
        // Tail is the opposite direction.
        let tail_col = q0_pos[0] as i32 - dc;
        let tail_row = q0_pos[1] as i32 - dr;
        if tail_col >= 0 && tail_row >= 0 {
            let tail_id = tail_row as u32 * w + tail_col as u32;
            if graph.cell(CellId(tail_id)).is_some() {
                allowed_edges.insert((tail_id, q_ids[0]), pres_cell);
            }
        }

        // Exit: outbound edge from presentation cell in exit_direction.
        let pres_pos = sd.presentation_cell;
        let (exit_dc, exit_dr): (i32, i32) = match sd.exit_direction {
            CardinalDirection::North => (0, -1),
            CardinalDirection::South => (0, 1),
            CardinalDirection::East => (1, 0),
            CardinalDirection::West => (-1, 0),
        };
        let exit_col = pres_pos[0] as i32 + exit_dc;
        let exit_row = pres_pos[1] as i32 + exit_dr;
        if exit_col >= 0 && exit_row >= 0 {
            let exit_id = exit_row as u32 * w + exit_col as u32;
            if graph.cell(CellId(exit_id)).is_some() {
                ungated_edges.insert((pres_id, exit_id));
            }
        }
    }

    if restricted_cells.is_empty() {
        return clone_graph(graph);
    }

    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        builder.add_cell(cell.clone());
    }

    for (src, tgt, edge) in graph.edges() {
        let src_restricted = restricted_cells.contains(&src.0);
        let tgt_restricted = restricted_cells.contains(&tgt.0);

        if !src_restricted && !tgt_restricted {
            // Neither endpoint is restricted — pass through unchanged.
            builder.add_edge(src, tgt, *edge);
        } else if ungated_edges.contains(&(src.0, tgt.0)) {
            // Exit edge — keep it without dest_only so departing bots can use it.
            builder.add_edge(src, tgt, *edge);
        } else if let Some(&dest_cell) = allowed_edges.get(&(src.0, tgt.0)) {
            // Allowed corridor edge — keep it with dest_only gate.
            let mut e = *edge;
            e.dest_only = Some(DestGate::Cell(dest_cell));
            builder.add_edge(src, tgt, e);
        }
        // else: edge touches a restricted cell but is not allowed — drop it.
    }

    builder.build()
}

fn clone_graph(graph: &WarehouseGraph) -> WarehouseGraph {
    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        builder.add_cell(cell.clone());
    }
    for (src, tgt, edge) in graph.edges() {
        builder.add_edge(src, tgt, *edge);
    }
    builder.build()
}
