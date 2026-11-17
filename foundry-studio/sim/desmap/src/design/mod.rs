//! Compact warehouse design format (v3): zone-based schematic.
//!
//! - [`types`] — Data models (`WarehouseDesign`, zone/station/edge definitions)
//! - [`grid_geometry`] — where each grid row/column physically sits, in mm
//! - [`compile`] — `WarehouseDesign::compile()` → `WarehouseMap`
//! - [`extract`] — `WarehouseMap` → `WarehouseDesign`

mod compile;
mod extract;
mod graph_builder;
mod grid_geometry;
mod highway_detect;
mod queue_edges;
mod reachability;
pub mod types;

pub use extract::extract;
pub use types::*;

#[cfg(test)]
mod tests;
