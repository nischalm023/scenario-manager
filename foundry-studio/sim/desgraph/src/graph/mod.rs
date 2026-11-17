//! Within `desgraph`, `graph` is the DRIVABLE FLOOR: the cells a robot may
//! stand on and the directed arcs between them, plus what each arc says about
//! who may take it.
//!
//! It is the one shape both the map compiler and the simulator compute over,
//! which is why it lives in neither: `desmap` builds one out of a design or a
//! world document, `dessim` searches it, and the serialized form here is what a
//! map on disk is. Geometry is carried, not derived -- a cell knows where it
//! stands so a consumer need not re-derive it from an index.

mod builder;
#[allow(clippy::module_inception)]
mod tests;
mod types;

pub use builder::WarehouseGraphBuilder;
use types::EdgeRecord;
pub use types::{Cell, CircularArc, DestGate, Edge, HtmDriveDirection};

use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

use crate::types::*;

// ---------------------------------------------------------------------------
// WarehouseGraph
// ---------------------------------------------------------------------------

/// The core warehouse navigation graph.
///
/// Wraps a petgraph directed graph. Immutable after construction -- use
/// [`WarehouseGraphBuilder`] to build.
///
/// petgraph `Graph` (not `StableGraph`) is used because the graph is immutable
/// after construction and `Graph` has lower overhead for parallel read access
/// (contiguous node/edge storage, no tombstones).
///
/// Serializes to a flat `{ cells: [...], edges: [...] }` format for human-readable
/// desmap JSON. The petgraph and lookup indices are reconstructed on deserialization.
pub struct WarehouseGraph {
    pub(super) graph: DiGraph<Cell, Edge>,
    pub(super) cell_index: FxHashMap<CellId, NodeIndex>,
    pub(super) grid_lookup: FxHashMap<GridPos, CellId>,
}

impl Clone for WarehouseGraph {
    fn clone(&self) -> Self {
        Self { graph: self.graph.clone(), cell_index: self.cell_index.clone(), grid_lookup: self.grid_lookup.clone() }
    }
}

impl std::fmt::Debug for WarehouseGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarehouseGraph")
            .field("cell_count", &self.graph.node_count())
            .field("edge_count", &self.graph.edge_count())
            .finish()
    }
}

// Custom serde: serialize as { cells: [...], edges: [{ from, to, ... }, ...] }
impl Serialize for WarehouseGraph {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let cells: Vec<&Cell> = self.graph.node_weights().collect();
        let edges: Vec<EdgeRecord> = self
            .graph
            .edge_indices()
            .map(|ei| {
                let (src, tgt) = self.graph.edge_endpoints(ei).unwrap();
                EdgeRecord { from: self.graph[src].id, to: self.graph[tgt].id, edge: self.graph[ei] }
            })
            .collect();
        let mut s = serializer.serialize_struct("WarehouseGraph", 2)?;
        s.serialize_field("cells", &cells)?;
        s.serialize_field("edges", &edges)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for WarehouseGraph {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Flat {
            cells: Vec<Cell>,
            edges: Vec<EdgeRecord>,
        }
        let flat = Flat::deserialize(deserializer)?;
        let mut builder = WarehouseGraphBuilder::with_capacity(flat.cells.len(), flat.edges.len());
        for cell in flat.cells {
            builder.add_cell(cell);
        }
        for er in flat.edges {
            builder.add_edge(er.from, er.to, er.edge);
        }
        Ok(builder.build())
    }
}

impl WarehouseGraph {
    /// Look up a cell by its CellId.
    pub fn cell(&self, id: CellId) -> Option<&Cell> {
        self.cell_index.get(&id).map(|&idx| &self.graph[idx])
    }

    /// Look up a CellId by grid position.
    pub fn cell_at(&self, pos: GridPos) -> Option<CellId> {
        self.grid_lookup.get(&pos).copied()
    }

    /// Iterate over all cells.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.graph.node_weights()
    }

    /// Number of cells.
    pub fn cell_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of directed edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Get the outgoing edges for a cell, yielding (target CellId, Edge).
    ///
    /// Returns a `SmallVec<[_; 4]>` since grid cells have at most 4 neighbors,
    /// avoiding heap allocation in the common case (critical for pathfinding).
    pub fn neighbors(&self, id: CellId) -> SmallVec<[(CellId, &Edge); 4]> {
        let Some(&idx) = self.cell_index.get(&id) else {
            return SmallVec::new();
        };
        self.graph
            .edges(idx)
            .map(|e| {
                let target = &self.graph[e.target()];
                (target.id, e.weight())
            })
            .collect()
    }

    /// Get the edge between two cells, if it exists.
    pub fn edge_between(&self, from: CellId, to: CellId) -> Option<&Edge> {
        let &from_idx = self.cell_index.get(&from)?;
        let &to_idx = self.cell_index.get(&to)?;
        self.graph.find_edge(from_idx, to_idx).map(|e| &self.graph[e])
    }

    /// Get the petgraph NodeIndex for a CellId (for advanced petgraph operations).
    pub fn node_index(&self, id: CellId) -> Option<NodeIndex> {
        self.cell_index.get(&id).copied()
    }

    /// Get the petgraph EdgeIndex between two cells (for advanced petgraph operations).
    pub fn edge_index(&self, from: CellId, to: CellId) -> Option<EdgeIndex> {
        let &from_idx = self.cell_index.get(&from)?;
        let &to_idx = self.cell_index.get(&to)?;
        self.graph.find_edge(from_idx, to_idx)
    }

    /// Access the underlying petgraph for advanced read-only operations.
    pub fn inner(&self) -> &DiGraph<Cell, Edge> {
        &self.graph
    }

    /// Iterate over all edges yielding (source CellId, target CellId, &Edge).
    pub fn edges(&self) -> impl Iterator<Item = (CellId, CellId, &Edge)> {
        self.graph.edge_indices().map(move |ei| {
            let (src, tgt) = self.graph.edge_endpoints(ei).unwrap();
            (self.graph[src].id, self.graph[tgt].id, &self.graph[ei])
        })
    }
}
