//! Configuration and internal types for the parametric warehouse generator.

use crate::types::*;

/// Which edges of the grid have stations.
#[derive(Clone, Debug, PartialEq)]
pub enum StationLayout {
    SingleBottom,
    SingleTop,
    SingleLeft,
    SingleRight,
    OppositeVertical,
    OppositeHorizontal,
    FourEdge,
    Custom(Vec<StationEdgeSpec>),
}

/// Station placement on a specific edge.
#[derive(Clone, Debug, PartialEq)]
pub struct StationEdgeSpec {
    pub edge: StationEdge,
    pub count: u32,
    pub spacing: u32,
}

/// Complete configuration for the parametric warehouse generator.
///
/// All parameters have defaults matching the Java `MapDataRequestDTO` defaults.
#[derive(Clone, Debug)]
pub struct GeneratorConfig {
    /// Grid columns (X axis). Range: 5..=200.
    pub grid_width: u32,
    /// Grid rows (Y axis). Range: 5..=200.
    pub grid_depth: u32,
    /// Physical cell dimension in millimeters. Range: 500.0..=2000.0.
    pub cell_size_mm: f64,
    /// Total station count. Range: 1..=50.
    pub station_count: u32,
    /// Station layout type.
    pub station_layout: StationLayout,
    /// Highway rows/cols per station edge. Range: 1..=6.
    pub highways_per_edge: u32,
    /// Rack rows per block. Range: 3..=20.
    pub rows_per_block: u32,
    /// Rack columns per block. Range: 1..=5.
    pub rack_columns: u32,
    /// Cross-aisle width in grid units between row-groups. Range: 1..=2.
    pub cross_aisle_width: u32,
    /// System type (RTP or RELAY).
    pub system_type: SystemType,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            grid_width: 20,
            grid_depth: 20,
            cell_size_mm: 1000.0,
            station_count: 4,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        }
    }
}

impl GeneratorConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.grid_width < 5 || self.grid_width > 200 {
            return Err(format!("grid_width must be 5..=200, got {}", self.grid_width));
        }
        if self.grid_depth < 5 || self.grid_depth > 200 {
            return Err(format!("grid_depth must be 5..=200, got {}", self.grid_depth));
        }
        if self.cell_size_mm < 500.0 || self.cell_size_mm > 2000.0 {
            return Err(format!("cell_size_mm must be 500.0..=2000.0, got {}", self.cell_size_mm));
        }
        if self.station_count < 1 || self.station_count > 50 {
            return Err(format!("station_count must be 1..=50, got {}", self.station_count));
        }
        if self.highways_per_edge < 1 || self.highways_per_edge > 6 {
            return Err(format!("highways_per_edge must be 1..=6, got {}", self.highways_per_edge));
        }
        if self.rows_per_block < 3 || self.rows_per_block > 20 {
            return Err(format!("rows_per_block must be 3..=20, got {}", self.rows_per_block));
        }
        if self.rack_columns < 1 || self.rack_columns > 5 {
            return Err(format!("rack_columns must be 1..=5, got {}", self.rack_columns));
        }
        if self.cross_aisle_width < 1 || self.cross_aisle_width > 2 {
            return Err(format!("cross_aisle_width must be 1..=2, got {}", self.cross_aisle_width));
        }
        Ok(())
    }
}

/// Row, col pair used internally during generation (matches Java's GridCoordinate).
/// Converted to GridPos(x=col, y=row) when building cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct RC {
    pub row: u32,
    pub col: u32,
}

pub(super) struct StationPlacement {
    pub station_id: u32,
    pub position: RC,
    pub edge: StationEdge,
    pub access_direction: (i32, i32),
}

pub(super) struct HighwaySegment {
    pub start: RC,
    pub end: RC,
}

pub(super) struct BlockPlacement {
    pub block_id: u32,
    pub top_left: RC,
    pub bottom_right: RC,
}
