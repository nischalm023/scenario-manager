//! # desgraph
//!
//! Shared warehouse graph types used by both `desmap` and `dessim`.
//!
//! - [`types`] -- Newtype IDs (`CellId`, `ZoneId`, etc.), coordinates, enums
//! - [`graph`] -- `WarehouseGraph` wrapping petgraph `DiGraph`, `Cell`, `Edge`

pub mod graph;
pub mod types;

pub use graph::{Cell, CircularArc, DestGate, Edge, HtmDriveDirection, WarehouseGraph, WarehouseGraphBuilder};
pub use types::*;
// Explicit re-exports for new types (also available via `types::*`)
pub use types::{CardinalDirection, HtmTraversal, RackOrientation, RobotType, StationType};
