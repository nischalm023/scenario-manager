use petgraph::graph::{DiGraph, NodeIndex};
use rustc_hash::FxHashMap;

use super::{Cell, Edge, WarehouseGraph};
use crate::types::*;

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for constructing an immutable [`WarehouseGraph`].
pub struct WarehouseGraphBuilder {
    pub(super) graph: DiGraph<Cell, Edge>,
    pub(super) cell_index: FxHashMap<CellId, NodeIndex>,
    pub(super) grid_lookup: FxHashMap<GridPos, CellId>,
}

impl WarehouseGraphBuilder {
    pub fn new() -> Self {
        Self { graph: DiGraph::new(), cell_index: FxHashMap::default(), grid_lookup: FxHashMap::default() }
    }

    /// Pre-allocate capacity for a known number of cells and edges.
    pub fn with_capacity(nodes: usize, edges: usize) -> Self {
        Self {
            graph: DiGraph::with_capacity(nodes, edges),
            cell_index: FxHashMap::default(),
            grid_lookup: FxHashMap::default(),
        }
    }

    /// Add a cell. Panics if the CellId is already present.
    pub fn add_cell(&mut self, cell: Cell) -> &mut Self {
        assert!(!self.cell_index.contains_key(&cell.id), "duplicate CellId: {:?}", cell.id);
        let id = cell.id;
        if let Some(gp) = cell.grid_pos {
            self.grid_lookup.insert(gp, id);
        }
        let idx = self.graph.add_node(cell);
        self.cell_index.insert(id, idx);
        self
    }

    /// Add a directed edge from `from` to `to`. Both cells must already exist.
    pub fn add_edge(&mut self, from: CellId, to: CellId, edge: Edge) -> &mut Self {
        let &from_idx = self.cell_index.get(&from).unwrap_or_else(|| panic!("unknown source CellId: {:?}", from));
        let &to_idx = self.cell_index.get(&to).unwrap_or_else(|| panic!("unknown target CellId: {:?}", to));
        self.graph.add_edge(from_idx, to_idx, edge);
        self
    }

    /// Add bidirectional edges between two cells with the same edge weight.
    pub fn add_edge_bidi(&mut self, a: CellId, b: CellId, edge: Edge) -> &mut Self {
        self.add_edge(a, b, edge);
        self.add_edge(b, a, Edge { curve: edge.curve.map(|arc| arc.reversed()), ..edge });
        self
    }

    /// Consume the builder and produce an immutable `WarehouseGraph`.
    pub fn build(self) -> WarehouseGraph {
        WarehouseGraph { graph: self.graph, cell_index: self.cell_index, grid_lookup: self.grid_lookup }
    }
}

impl Default for WarehouseGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}
