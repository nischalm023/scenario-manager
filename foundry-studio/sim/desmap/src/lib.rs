//! # desmap
//!
//! Warehouse graph representation (`desgraph`) and JSON serialization format (`desmap`).
//!
//! The `WarehouseMap` is the root type. Serialize it with `serde_json` to produce
//! the desmap JSON format. The graph is immutable after construction -- use the
//! builder types to construct.
//!
//! ## Modules
//!
//! - [`graph`] / [`types`] -- Re-exported from `desgraph` (shared with `dessim`)
//! - [`station`] -- `Station`, `RelayPosition`
//! - [`zone`] -- `Zone`, `Sector`, `RackBlock`
//! - [`map`] -- Top-level `WarehouseMap` and builder

pub mod design;
pub mod generator;
pub mod machine;
pub mod map;
pub mod station;
pub mod vtm_model;
pub mod vtm_position;
pub mod zone;

// Re-export desgraph modules so existing `desmap::graph::*` and `desmap::types::*` paths keep working.
pub use desgraph::graph;
pub use desgraph::types;

// Re-export the most commonly used types at crate root.
pub use desgraph::{
    BlockId, BlockSide, FloorId, GridPos, RackOrientation, RelayId, RobotType, SectorId, StationEdge, StationId,
    StationType, SystemType, WorldPos, ZoneId,
};
pub use desgraph::{
    Cell, CellId, CellKind, DestGate, Edge, EdgeTraversability, HtmTraversal, WarehouseGraph, WarehouseGraphBuilder,
};
pub use design::WarehouseDesign;
pub use generator::{GeneratorConfig, StationEdgeSpec, StationLayout};
pub use machine::{ConveyorMachine, MachinePresentation, OrderHtmArea};
pub use map::{FleetConfig, GridBounds, WarehouseMap, WarehouseMapBuilder};
pub use station::{HandoffQueue, OutboundRole, PresentationPoint, RelayPosition, Station};
pub use vtm_model::VtmModel;
pub use vtm_position::VtmPosition;
pub use zone::{RackBlock, Sector, Zone, ZoneHolds};
