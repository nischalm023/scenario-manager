//! Within `app`, `grid_data` is the ONCE-PER-MAP derivation: everything the
//! map view draws that does not change while a run plays, worked out from the
//! solver's map at load and then only read.
//!
//! The tile a cell is DRAWN at and the arrows its hops become are
//! `worldview-floor`'s, because a browser reading a recording of the same run
//! draws them from that same compile; what this module adds is everything only
//! desvis says about a cell — its zone, its relay's or station's line of
//! diagnostics, the markers the overlay reads.

use std::collections::{HashMap, HashSet};

use rustc_hash::{FxHashMap, FxHashSet};

use desgraph::{CellId, RobotType};
use desmap::WarehouseMap;
use dessim::metrics::profile;
use dessim::metrics::profile::DynamicGrid;
use dessim::observer::time_slice::BotLocation;

use crate::types::*;
use worldview_render::{BotType, Cell, VisitCounter};

/// Everything desvis says about each cell of an already-compiled floor.
///
/// The tile itself is [`worldview_floor`]'s — the caller has just compiled the
/// scene and hands its cells in — so a panel's facts hang off the same cell a
/// browser is drawn.
pub fn build_grid_data(map: &WarehouseMap, cells: &[Option<Cell>]) -> Vec<Option<CellInfo>> {
    let wait_exclusions = map.navigation_wait_exclusions();
    let mut relay_by_cell: HashMap<u32, String> = HashMap::new();
    for relay in &map.relays {
        let info = format!(
            "Relay {}: block={}, side={:?}, htm_access={}, vtm_anchor={}",
            relay.id.0,
            relay.block.0,
            relay.side,
            relay.htm_access.0,
            relay.vtm_access.anchor().0
        );
        relay_by_cell.insert(relay.position.0, info);
    }

    let mut station_by_cell: FxHashMap<u32, String> = FxHashMap::default();
    for station in &map.stations {
        let edge_str = match station.edge {
            Some(e) => format!("{:?}", e),
            None => "N/A".to_string(),
        };
        let presented: Vec<String> = station.presentation_cells().map(|cell| cell.0.to_string()).collect();
        let queue_lens: Vec<String> =
            station.presentation_points.iter().map(|point| point.queue_path.len().to_string()).collect();
        let info = format!(
            "Station {} ({}): pres={}, edge={}, queue_len={}",
            station.id.0,
            station.outbound_role.map(|role| role.label()).unwrap_or("workstation"),
            presented.join("/"),
            edge_str,
            queue_lens.join("/"),
        );
        station_by_cell.insert(station.position.0, info.clone());
        for point in &station.presentation_points {
            station_by_cell.entry(point.presentation_cell.0).or_insert_with(|| {
                format!(
                    "Station {} (presentation cell): edge={}, queue_len={}",
                    station.id.0,
                    edge_str,
                    point.queue_path.len(),
                )
            });
        }
    }

    // The tiles and the hops are the shared compile's: a browser reading a
    // recording of this run draws them from the same code, so the two viewers
    // cannot put one cell in two places. What desvis adds around each cell is
    // what its panels say about it and nothing the renderer sees.
    cells
        .iter()
        .enumerate()
        .map(|(id, drawn)| {
            let cell = map.graph.cell(CellId(id as u32))?;
            Some(CellInfo {
                cell: drawn.clone()?,
                zone_id: cell.zone.0,
                rack_orientation: cell.rack_orientation,
                outgoing_edges: map.graph.neighbors(cell.id).len() as u32,
                relay_info: relay_by_cell.get(&cell.id.0).cloned(),
                station_info: station_by_cell.get(&cell.id.0).cloned(),
                navigation_wait_exclusion: wait_exclusions.get(&cell.id.0).copied(),
            })
        })
        .collect()
}

/// Build per-station selection metadata: one [`StationMeta`] per station plus
/// a cell-id → station index lookup covering each station's position,
/// presentation, and queue-path cells. Drives single-click station selection
/// and the inspector header in both live and playback modes.
pub(crate) fn build_station_metas(map: &WarehouseMap) -> (Vec<StationMeta>, FxHashMap<u32, usize>) {
    let mut metas = Vec::with_capacity(map.stations.len());
    let mut by_cell = FxHashMap::default();
    for station in &map.stations {
        let meta_index = metas.len();
        let mut cell_ids = Vec::new();
        let cells = std::iter::once(station.position).chain(station.presentation_and_queue_cells());
        for cell in cells {
            // First-writer wins on shared cells; station cells don't overlap
            // between stations in practice.
            by_cell.entry(cell.0).or_insert(meta_index);
            if !cell_ids.contains(&cell.0) {
                cell_ids.push(cell.0);
            }
        }
        metas.push(StationMeta {
            process_label: station.outbound_role.map(|role| role.label()),
            id: station.id,
            station_type: station.station_type,
            presentation_points: station.presentation_points.clone(),
            delivery_cell: station.delivery_cell().0,
            bins: station.bins,
            cell_ids,
        });
    }
    (metas, by_cell)
}

/// Build per-relay selection metadata: one [`RelayMeta`] per relay plus a
/// cell-id → relay index lookup covering each relay's position, HTM-access,
/// and VTM-access cells. A click on any of those three selects the relay
/// (keyed by its HTM-access cell — the diagnostics flow under that cell id).
pub(crate) fn build_relay_metas(map: &WarehouseMap) -> (Vec<RelayMeta>, HashMap<u32, usize>) {
    let mut metas = Vec::with_capacity(map.relays.len());
    let mut by_cell = HashMap::new();
    for relay in &map.relays {
        let meta_index = metas.len();
        let mut cell_ids = Vec::new();
        for cell in [relay.position, relay.htm_access, relay.vtm_access.anchor()] {
            by_cell.entry(cell.0).or_insert(meta_index);
            if !cell_ids.contains(&cell.0) {
                cell_ids.push(cell.0);
            }
        }
        metas.push(RelayMeta {
            relay_id: relay.id.0,
            htm_access_cell: relay.htm_access.0,
            vtm_access_cell: relay.vtm_access.anchor().0,
            block_id: relay.block.0,
            cell_ids,
        });
    }
    (metas, by_cell)
}

/// Compute HTM access markers from station presentation cells in the warehouse map.
pub(crate) fn compute_htm_access_markers(map: &WarehouseMap) -> Vec<HtmAccessMarker> {
    let mut htm_access_cells = Vec::new();
    let mut seen = FxHashSet::default();
    for station in &map.stations {
        for access_cell in station.presentation_cells().map(|cell| cell.0) {
            if seen.insert(access_cell) {
                htm_access_cells.push(HtmAccessMarker { cell_id: access_cell, station_id: station.id.0 });
            }
        }
    }
    htm_access_cells
}

/// Compute VTM access markers from relay configs in the warehouse map.
pub(crate) fn compute_vtm_access_markers(map: &WarehouseMap) -> Vec<VtmAccessMarker> {
    let mut vtm_access_cells = Vec::new();
    let mut seen_vtm_access = HashSet::new();
    for relay in &map.relays {
        let vtm_cell = relay.vtm_access.anchor().0;
        if seen_vtm_access.insert(vtm_cell) {
            vtm_access_cells.push(VtmAccessMarker {
                cell_id: vtm_cell,
                relay_position: relay.position.0,
                block_id: relay.block.0,
                side: relay.side,
            });
        }
    }
    vtm_access_cells
}

/// Derive heatmap cell visit counts from per-bot location data, keyed by
/// cell id (`len` = the id-dense array length, `cell_infos.len()`).
pub(crate) fn derive_heatmap(bot_locations: &FxHashMap<u32, BotLocation>, len: usize) -> HeatmapData {
    // The counting itself is `worldview-render`'s, so the browser paints the
    // same slice overlay from the same run's frames rather than from a second
    // reading of what a visit is.
    let mut counter = VisitCounter::over(len);
    for location in bot_locations.values() {
        let class = match location.robot_type {
            RobotType::Vtm => BotType::Vtm,
            RobotType::Rtp => BotType::Rtp,
            RobotType::Htm => BotType::Htm,
        };
        counter.visited(class, location.start_cell);
        for step in &location.steps {
            counter.visited(class, step.cell_id);
        }
    }
    counter.finish()
}

/// Convert a profile's cumulative DynamicGrid to id-dense heatmap data.
pub(crate) fn dynamic_grid_to_heatmap(grid: &DynamicGrid, len: usize) -> HeatmapData {
    let mut vtm_counts = vec![0u32; len];
    let mut htm_counts = vec![0u32; len];
    let mut vtm_max = 0u32;
    let mut htm_max = 0u32;

    let (vtm_layer, htm_layer) = profile::grid_visit_layers(grid);

    if let Some(vtm) = vtm_layer {
        for (&cell_id, &count) in vtm {
            if let Some(c) = vtm_counts.get_mut(cell_id as usize) {
                *c += count as u32;
                vtm_max = vtm_max.max(*c);
            }
        }
    }
    if let Some(htm) = htm_layer {
        for (&cell_id, &count) in htm {
            if let Some(c) = htm_counts.get_mut(cell_id as usize) {
                *c += count as u32;
                htm_max = htm_max.max(*c);
            }
        }
    }

    HeatmapData { vtm_counts, htm_counts, vtm_max, htm_max }
}

// TODO(v3-sunset): the free-form-node render test built its map from an
// inline v3 design (junction_cells were a v3 feature). The invariant it
// pinned -- a cell with a world position but no lattice index gets an
// ordinary CellInfo centred on that position -- still holds and should be
// re-pinned from a WarehouseGraphBuilder-built map when tests are restored.

#[cfg(test)]
mod tests {
    #[test]
    fn easyloop_presentation_footprints_follow_the_declared_turn_tangent() {
        use std::path::Path;

        use dessim::worldmap;
        use worldview_render::renderer::cell_layer::build_cell_instances;
        use worldview_render::{CellIndex, RenderFrame};

        use super::build_grid_data;

        for fixture in ["minimal_easyloop.json", "walmart_dfw6.json"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sim/dessim/tests/fixtures").join(fixture);
            let loaded = worldmap::load(&path).unwrap();
            let cells = worldview_floor::cells::tiles(
                &loaded.map,
                &loaded.cell_sizes_mm,
                &loaded.cell_footprint_stated,
                &loaded.cell_headings_rad,
                850.0,
            );
            assert_eq!(build_grid_data(&loaded.map, &cells).len(), cells.len(), "one info per cell of the floor");
            let index = CellIndex::build(&cells);
            let instances = build_cell_instances(&cells, &RenderFrame::from_tiles(&cells), 0);
            for station in loaded.map.stations.iter().filter(|s| s.order_presentation_cell.is_some()) {
                let at = station.order_presentation_cell.unwrap();
                let cell = cells[at.0 as usize].as_ref().unwrap();
                let point = loaded.map.cell_world_pos(at).unwrap();
                let departure = loaded.map.graph.neighbors(at)[0].1;
                let tangent = departure.curve.unwrap().tangent(point);
                assert!((cell.heading_rad.cos() - tangent[0]).abs() < 1e-4);
                assert!((cell.heading_rad.sin() - tangent[1]).abs() < 1e-4);
                assert!((instances[at.0 as usize].heading_rad as f64 - cell.heading_rad).abs() < 1e-6);
                assert_eq!(index.query(point.x_mm, point.y_mm, &cells), Some(at.0));
            }
        }
    }
}
