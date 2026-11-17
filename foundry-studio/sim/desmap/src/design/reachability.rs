//! Reachability analysis: narrow edge traversability flags by robot-type BFS.

use std::collections::{HashSet, VecDeque};

use crate::graph::{Edge, WarehouseGraph, WarehouseGraphBuilder};
use crate::station::{RelayPosition, Station};
use crate::types::*;

/// Rebuild the graph with reachability-narrowed traversability flags.
///
/// For each robot type (VTM, HTM unladen, HTM laden, RTP), BFS from known
/// origins determines which cells are reachable. Edges between unreachable
/// cells are downgraded or pruned.
pub(super) fn narrow_by_reachability(
    graph: &WarehouseGraph,
    relays: &[RelayPosition],
    stations: &[Station],
) -> WarehouseGraph {
    let bfs = |origins: &[CellId], can_traverse: &dyn Fn(&EdgeTraversability) -> bool| -> HashSet<CellId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        for &o in origins {
            if graph.cell(o).is_some() && visited.insert(o) {
                queue.push_back(o);
            }
        }
        while let Some(cur) = queue.pop_front() {
            for (nid, edge) in graph.neighbors(cur) {
                if can_traverse(&edge.traversability) && visited.insert(nid) {
                    queue.push_back(nid);
                }
            }
        }
        visited
    };

    // VTM: BFS restricted to Aisle cells.
    let vtm_reachable = {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        for r in relays {
            if let Some(cell) = graph.cell(r.vtm_access.anchor()) {
                if matches!(cell.kind, CellKind::Aisle) && visited.insert(r.vtm_access.anchor()) {
                    queue.push_back(r.vtm_access.anchor());
                }
            }
        }
        while let Some(cur) = queue.pop_front() {
            for (nid, _edge) in graph.neighbors(cur) {
                if let Some(tc) = graph.cell(nid) {
                    if matches!(tc.kind, CellKind::Aisle) && visited.insert(nid) {
                        queue.push_back(nid);
                    }
                }
            }
        }
        visited
    };

    let presentation_cells = || stations.iter().flat_map(|s| s.presentation_cells());
    let htm_origins: Vec<CellId> = relays.iter().map(|r| r.htm_access).chain(presentation_cells()).collect();
    let htm_unladen = bfs(&htm_origins, &|_| true);
    let htm_laden = bfs(&htm_origins, &|t| t.htm_traversal == HtmTraversal::Both);

    let rtp_origins: Vec<CellId> = presentation_cells().collect();
    let rtp_reachable = bfs(&rtp_origins, &|_| true);

    // Rebuild with narrowed flags, pruning fully-dead edges.
    let mut builder = WarehouseGraphBuilder::with_capacity(graph.cell_count(), graph.edge_count());
    for cell in graph.cells() {
        builder.add_cell(cell.clone());
    }

    for (src, tgt, edge) in graph.edges() {
        let mut trav = edge.traversability;

        if !vtm_reachable.contains(&src) || !vtm_reachable.contains(&tgt) {
            trav.vtm_traversal = false;
        }
        if !htm_unladen.contains(&src) || !htm_unladen.contains(&tgt) {
            trav.htm_traversal = HtmTraversal::None;
        } else if trav.htm_traversal == HtmTraversal::Both && (!htm_laden.contains(&src) || !htm_laden.contains(&tgt)) {
            trav.htm_traversal = HtmTraversal::UnladenOnly;
        }
        if !rtp_reachable.contains(&src) || !rtp_reachable.contains(&tgt) {
            trav.rtp_traversal = false;
        }

        let dead = !trav.vtm_traversal && trav.htm_traversal == HtmTraversal::None && !trav.rtp_traversal;
        if dead {
            continue;
        }

        builder.add_edge(
            src,
            tgt,
            Edge {
                traversability: trav,
                distance_mm: edge.distance_mm,
                dest_only: edge.dest_only,
                vtm_zone_gate: edge.vtm_zone_gate,
                curve: edge.curve,
                htm_drive: edge.htm_drive,
            },
        );
    }
    builder.build()
}
