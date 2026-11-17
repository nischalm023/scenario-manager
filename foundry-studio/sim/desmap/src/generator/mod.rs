//! Parametric warehouse generator.
//!
//! Takes a [`GeneratorConfig`] and produces a [`WarehouseDesign`] (compact warehouse
//! description) which is then compiled into a [`WarehouseMap`].
//!
//! # Generation phases (sequential, each depends on the previous)
//!
//! 1. **Stations** -- placed along grid edges with configurable spacing
//! 2. **Highways** -- parallel transit corridors near station edges
//! 3. **Rack blocks** -- storage blocks tiled into remaining space
//! 4. **Design assembly** -- produce v3 `WarehouseDesign` with zone definitions
//! 5. **Compilation** -- `WarehouseDesign::compile()` derives cell kinds, zones,
//!    relay positions, and produces the full `WarehouseMap`

mod layout;
mod stations;
pub mod types;

pub use types::{GeneratorConfig, StationEdgeSpec, StationLayout};

use std::collections::BTreeSet;

use crate::design::{
    GridDefinition, HighwayDefinition, LaneDirection, RelayZone, RtpZone, Sidedness, StationDefinition,
    WarehouseDesign, ZoneDef,
};
use crate::map::WarehouseMap;
use crate::types::*;

use layout::{calculate_occupied, place_highways, place_rack_blocks};
use stations::{place_stations, resolve_edge_specs};
use types::*;

/// Generate a warehouse map from the given configuration.
///
/// This is the primary entry point. Internally it produces a `WarehouseDesign`
/// and then compiles it to a `WarehouseMap`.
pub fn generate(config: &GeneratorConfig) -> Result<WarehouseMap, String> {
    let design = generate_design(config)?;
    design.compile()
}

/// Generate a compact `WarehouseDesign` from the given configuration.
///
/// The design is the source representation of the warehouse. Call
/// `design.compile()` to get the full `WarehouseMap` with petgraph.
pub fn generate_design(config: &GeneratorConfig) -> Result<WarehouseDesign, String> {
    config.validate()?;

    let w = config.grid_width;
    let d = config.grid_depth;

    let edge_specs = resolve_edge_specs(config);
    let stations = place_stations(&edge_specs, w, d);
    let station_edges_used: BTreeSet<StationEdge> = stations.iter().map(|s| s.edge).collect();

    let highways = place_highways(&station_edges_used, config.highways_per_edge, w, d);
    let occupied = calculate_occupied(&stations, &highways, &station_edges_used, w, d);

    // A relay block is rack / VTM aisle / rack. Keep one open column between
    // adjacent blocks so both outer rack faces have a real perpendicular HTM
    // approach; packing the three-column blocks contiguously stranded both
    // facing rack sides and left their zones without usable relay positions.
    let (block_cols, inter_block_gap) =
        if config.system_type == SystemType::Relay { (3u32, 1u32) } else { (config.rack_columns, 1u32) };
    let blocks = place_rack_blocks(
        &occupied,
        config.rows_per_block,
        block_cols,
        config.cross_aisle_width,
        inter_block_gap,
        w,
        d,
    );

    let hw = build_highway_defs(&station_edges_used, config);
    let zone_defs = build_zone_defs(&blocks, config);
    let station_defs = build_station_defs(&stations);

    Ok(WarehouseDesign {
        version: 3,
        grid: GridDefinition::uniform(w, d, config.cell_size_mm),
        highways: hw,
        zones: zone_defs,
        racks: vec![],
        stations: station_defs,
        edge_exceptions: Vec::new(),
        junction_cells: Vec::new(),
        clip_boxes: Vec::new(),
        charging_zones: Vec::new(),
        fleet: None,
    })
}

fn build_highway_defs(station_edges_used: &BTreeSet<StationEdge>, config: &GeneratorConfig) -> Vec<HighwayDefinition> {
    let n = config.highways_per_edge;
    let mut hw = Vec::new();
    for &edge in station_edges_used {
        for i in 0..n {
            hw.push(match edge {
                StationEdge::Bottom => HighwayDefinition {
                    origin: [0, 1 + i],
                    width: config.grid_width,
                    height: 1,
                    direction: LaneDirection::Bidirectional,
                },
                StationEdge::Top => HighwayDefinition {
                    origin: [0, config.grid_depth.saturating_sub(2 + i)],
                    width: config.grid_width,
                    height: 1,
                    direction: LaneDirection::Bidirectional,
                },
                StationEdge::Left => HighwayDefinition {
                    origin: [1 + i, 0],
                    width: 1,
                    height: config.grid_depth,
                    direction: LaneDirection::Bidirectional,
                },
                StationEdge::Right => HighwayDefinition {
                    origin: [config.grid_width.saturating_sub(2 + i), 0],
                    width: 1,
                    height: config.grid_depth,
                    direction: LaneDirection::Bidirectional,
                },
            });
        }
    }
    hw
}

fn build_zone_defs(blocks: &[BlockPlacement], config: &GeneratorConfig) -> Vec<ZoneDef> {
    blocks
        .iter()
        .map(|bp| {
            let block_height = bp.bottom_right.row - bp.top_left.row + 1;
            if config.system_type == SystemType::Relay {
                ZoneDef::Relay(RelayZone {
                    id: bp.block_id,
                    origin: [bp.top_left.col, bp.top_left.row],
                    length: block_height,
                    sides: Sidedness::default(),
                    vertical_tote_capacity: 8,
                    zone_height_mm: 9000,
                    zone_width_mm: None,
                    real_shelf_count: 10,
                    rack_placements: vec![],
                    vtm_model: Default::default(),
                    vtm_count: None,
                })
            } else {
                let block_width = bp.bottom_right.col - bp.top_left.col + 1;
                ZoneDef::Rtp(RtpZone {
                    id: bp.block_id,
                    origin: [bp.top_left.col, bp.top_left.row],
                    rows: block_height,
                    cols: block_width,
                })
            }
        })
        .collect()
}

fn build_station_defs(stations: &[StationPlacement]) -> Vec<StationDefinition> {
    stations
        .iter()
        .map(|sp| {
            let pres_rc = RC {
                row: (sp.position.row as i32 + sp.access_direction.0) as u32,
                col: (sp.position.col as i32 + sp.access_direction.1) as u32,
            };
            StationDefinition {
                id: sp.station_id,
                position: [sp.position.col, sp.position.row],
                presentation_cell: [pres_rc.col, pres_rc.row],
                edge: Some(sp.edge),
                queue_path: vec![],
                exit_direction: CardinalDirection::East,
                bins: None,
                station_type: StationType::Pick,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
