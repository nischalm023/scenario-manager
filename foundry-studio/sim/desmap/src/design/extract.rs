//! Extract: `WarehouseMap` → `WarehouseDesign` v3.

use std::collections::HashMap;

use rustc_hash::FxHashSet;

use crate::graph::Cell;
use crate::map::WarehouseMap;
use crate::types::*;

use super::highway_detect::detect_highways;
use super::types::*;

/// Extract a compact v3 `WarehouseDesign` from a `WarehouseMap`.
///
/// Analyzes blocks, relays, and cell kinds to produce parametric zone defs.
pub fn extract(map: &WarehouseMap) -> Result<WarehouseDesign, String> {
    let bounds = map.grid_bounds.ok_or("cannot extract design without grid_bounds")?;
    let (w, d, cell_size) = (bounds.width, bounds.depth, bounds.cell_size_mm);

    let hw = detect_highways(map, w, d);
    let zone_defs = extract_zone_defs(map);

    // Mirrors compile(): build lane/station lookups for edge exception detection.
    let (row_lane_dirs, col_lane_dirs) = build_lane_dir_maps(&hw);
    let (presentation_cells, position_cells) = build_station_lookups(map);

    // Row geometry, recovered from cell world positions. `narrow_by_reachability`
    // re-adds every cell, so ids stay dense at `row * w + col` and row 0 of each
    // row is always present to read a center from.
    let row_centers_mm = recover_row_centers(map, w, d, cell_size);

    // When the centers were recovered, per-edge distances are implied by them and
    // must NOT be re-emitted as exceptions. When recovery failed, the geometry has
    // nowhere else to live, so fall back to comparing against the nominal pitch so
    // it survives as exceptions rather than being silently dropped.
    let edge_exceptions = extract_edge_exceptions(
        map,
        cell_size,
        row_centers_mm.is_some(),
        &row_lane_dirs,
        &col_lane_dirs,
        &presentation_cells,
        &position_cells,
    );
    let junction_cells = extract_junctions(map, w * d);
    let stations = extract_stations(map)?;

    Ok(WarehouseDesign {
        version: 3,
        grid: GridDefinition { width: w, depth: d, cell_size_mm: cell_size, row_centers_mm },
        highways: hw,
        zones: zone_defs,
        racks: vec![],
        stations,
        edge_exceptions,
        junction_cells,
        clip_boxes: Vec::new(),
        charging_zones: Vec::new(),
        fleet: map.fleet,
    })
}

/// Recover per-row center Y from cell world positions, or `None` if the map is
/// uniform.
///
/// Returning `None` for uniform maps keeps existing designs round-tripping
/// byte-identically instead of gaining a redundant 96-entry vector. Also `None`
/// if any row is unreadable, in which case the row pitch survives as
/// `EdgeException` distances instead (see `extract_edge_exceptions`), so the
/// geometry is never silently dropped.
fn recover_row_centers(map: &WarehouseMap, w: u32, d: u32, cell_size: f64) -> Option<Vec<f64>> {
    let mut centers = Vec::with_capacity(d as usize);
    for row in 0..d {
        // Grid ids are dense `row * w + col`; col 0 always exists.
        let y = map.graph.cell(CellId(row * w))?.world_pos?.y_mm;
        centers.push(y);
    }
    let uniform = centers.iter().enumerate().all(|(i, &c)| (c - (i as f64 * cell_size + cell_size / 2.0)).abs() < 1e-6);
    if uniform { None } else { Some(centers) }
}

/// Center-to-center distance a cardinal edge should have, from the two cells'
/// own world positions. `dx != 0` selects the x axis, otherwise y.
///
/// Derived from the cells rather than from `cell_size_mm` so that a non-uniform
/// row pitch is not mistaken for a per-edge override and re-emitted as an
/// `EdgeException` on top of the row centers. Falls back to `cell_size` when a
/// cell carries no world position.
fn expected_cardinal_distance_mm(from: &Cell, to: &Cell, dx: i64, cell_size: f64) -> u64 {
    match (from.world_pos, to.world_pos) {
        (Some(fw), Some(tw)) => {
            let delta = if dx != 0 { tw.x_mm - fw.x_mm } else { tw.y_mm - fw.y_mm };
            delta.abs().round() as u64
        }
        _ => cell_size as u64,
    }
}

fn extract_zone_defs(map: &WarehouseMap) -> Vec<ZoneDef> {
    map.blocks
        .iter()
        .map(|block| {
            let tl = block.top_left;
            let br = block.bottom_right;
            let length = br.y - tl.y + 1;
            if map.system_type == SystemType::Relay {
                // Sidedness is block geometry, independent of whether an
                // outer travel lane happens to make any rack cell usable as
                // an HTM relay. Inferring it from RelayPosition metadata loses
                // a rack column when a side has only invalid end-cap access.
                let sides = match br.x - tl.x + 1 {
                    3 => Sidedness::Dual,
                    2 if map.graph.cells().any(|cell| {
                        cell.grid_pos == Some(tl) && matches!(cell.kind, CellKind::Relay | CellKind::Storage)
                    }) =>
                    {
                        Sidedness::LeftOnly
                    }
                    2 => Sidedness::RightOnly,
                    _ => Sidedness::Dual,
                };
                ZoneDef::Relay(RelayZone {
                    id: block.id.0,
                    origin: [tl.x, tl.y],
                    length,
                    sides,
                    vertical_tote_capacity: default_vtc(),
                    zone_height_mm: 9000,
                    zone_width_mm: None,
                    real_shelf_count: 10,
                    rack_placements: vec![],
                    vtm_model: Default::default(),
                    vtm_count: None,
                })
            } else {
                ZoneDef::Rtp(RtpZone { id: block.id.0, origin: [tl.x, tl.y], rows: length, cols: br.x - tl.x + 1 })
            }
        })
        .collect()
}

fn build_lane_dir_maps(
    highways: &[HighwayDefinition],
) -> (HashMap<(u32, u32), LaneDirection>, HashMap<(u32, u32), LaneDirection>) {
    let mut row_dirs = HashMap::new();
    let mut col_dirs = HashMap::new();
    for hd in highways {
        if hd.direction != LaneDirection::Bidirectional {
            if hd.width > hd.height {
                for r in hd.origin[1]..hd.origin[1] + hd.height {
                    for c in hd.origin[0]..hd.origin[0] + hd.width {
                        row_dirs.insert((c, r), hd.direction);
                    }
                }
            } else if hd.height > hd.width {
                for r in hd.origin[1]..hd.origin[1] + hd.height {
                    for c in hd.origin[0]..hd.origin[0] + hd.width {
                        col_dirs.insert((c, r), hd.direction);
                    }
                }
            }
        }
    }
    (row_dirs, col_dirs)
}

fn build_station_lookups(map: &WarehouseMap) -> (FxHashSet<u32>, FxHashSet<u32>) {
    let mut presentation = FxHashSet::default();
    let mut position = FxHashSet::default();
    for s in &map.stations {
        position.insert(s.position.0);
        presentation.extend(s.presentation_cells().map(|cell| cell.0));
    }
    (presentation, position)
}

/// Extract edge exceptions: edges whose pre-reachability traversability
/// differs from what the compiler would generate structurally.
#[allow(clippy::too_many_arguments)]
fn extract_edge_exceptions(
    map: &WarehouseMap,
    cell_size: f64,
    centers_recovered: bool,
    row_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
    col_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
    presentation_cells: &FxHashSet<u32>,
    position_cells: &FxHashSet<u32>,
) -> Vec<EdgeException> {
    let mut exceptions = Vec::new();

    for cell in map.graph.cells() {
        let Some(from_gp) = cell.grid_pos else {
            continue;
        };
        for (nid, edge) in map.graph.neighbors(cell.id) {
            let Some(to_cell) = map.graph.cell(nid) else {
                continue;
            };
            let Some(to_gp) = to_cell.grid_pos else {
                continue;
            };
            let (dx, dy) = (to_gp.x as i64 - from_gp.x as i64, to_gp.y as i64 - from_gp.y as i64);

            if dx.abs() + dy.abs() != 1 {
                // Non-cardinal: always an exception.
                exceptions.push(EdgeException {
                    from: [from_gp.x, from_gp.y],
                    to: [to_gp.x, to_gp.y],
                    traversability: edge.traversability,
                    distance_mm: Some(edge.distance_mm),
                });
                continue;
            }

            let expected_htm = compute_expected_htm(
                cell,
                to_cell,
                dx,
                dy,
                row_lane_dirs,
                col_lane_dirs,
                presentation_cells,
                position_cells,
            );

            let structural_dest_only =
                edge.dest_only.is_some() && presentation_cells.contains(&nid.0) && !position_cells.contains(&cell.id.0);
            let structural_vtm_zone_gate = edge.vtm_zone_gate.is_some();

            let expected = if centers_recovered {
                expected_cardinal_distance_mm(cell, to_cell, dx, cell_size)
            } else {
                cell_size as u64
            };
            let dist_differs = edge.distance_mm != expected;
            let htm_mismatch = is_htm_structural_mismatch(expected_htm, edge.traversability.htm_traversal);

            if (htm_mismatch && !structural_dest_only && !structural_vtm_zone_gate) || dist_differs {
                exceptions.push(EdgeException {
                    from: [from_gp.x, from_gp.y],
                    to: [to_gp.x, to_gp.y],
                    traversability: edge.traversability,
                    distance_mm: if dist_differs { Some(edge.distance_mm) } else { None },
                });
            }
        }
    }
    exceptions.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
    exceptions
}

/// Compute the expected pre-reachability HTM traversability for a cardinal edge.
fn compute_expected_htm(
    from: &Cell,
    to: &Cell,
    dx: i64,
    dy: i64,
    row_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
    col_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
    presentation_cells: &FxHashSet<u32>,
    position_cells: &FxHashSet<u32>,
) -> HtmTraversal {
    let either_blocked = matches!(from.kind, CellKind::Aisle | CellKind::StationArea | CellKind::Storage)
        || matches!(to.kind, CellKind::Aisle | CellKind::StationArea | CellKind::Storage);

    let mut expected = if either_blocked { HtmTraversal::None } else { HtmTraversal::Both };
    let is_horizontal = dx != 0;
    let from_gp = from.grid_pos.unwrap();
    let to_gp = to.grid_pos.unwrap();

    // Horizontal lane constraints.
    if is_horizontal && !either_blocked {
        let is_east = dx > 0;
        let from_dir = row_lane_dirs.get(&(from_gp.x, from_gp.y)).copied();
        let to_dir = row_lane_dirs.get(&(to_gp.x, to_gp.y)).copied();
        for dir in [from_dir, to_dir].into_iter().flatten() {
            match dir {
                LaneDirection::Westbound if is_east => expected = HtmTraversal::None,
                LaneDirection::Eastbound if !is_east => expected = HtmTraversal::None,
                LaneDirection::NorthSouthOnly => expected = HtmTraversal::None,
                _ => {}
            }
        }
        let ns_from = col_lane_dirs.get(&(from_gp.x, from_gp.y)).copied() == Some(LaneDirection::NorthSouthOnly);
        let ns_to = col_lane_dirs.get(&(to_gp.x, to_gp.y)).copied() == Some(LaneDirection::NorthSouthOnly);
        if ns_from || ns_to {
            expected = HtmTraversal::None;
        }
    }

    // Vertical lane constraints.
    if !is_horizontal && !either_blocked {
        let ew_from = row_lane_dirs.get(&(from_gp.x, from_gp.y)).copied() == Some(LaneDirection::EastWestOnly);
        let ew_to = row_lane_dirs.get(&(to_gp.x, to_gp.y)).copied() == Some(LaneDirection::EastWestOnly);
        if ew_from || ew_to {
            expected = HtmTraversal::None;
        }

        let from_col_dir = col_lane_dirs.get(&(from_gp.x, from_gp.y)).copied();
        let to_col_dir = col_lane_dirs.get(&(to_gp.x, to_gp.y)).copied();

        if dy > 0 {
            let block_south = matches!(from_col_dir, Some(LaneDirection::Northbound | LaneDirection::EastWestOnly))
                || matches!(to_col_dir, Some(LaneDirection::Northbound | LaneDirection::EastWestOnly));
            if block_south {
                expected = HtmTraversal::None;
            }
        } else {
            let block_north = matches!(from_col_dir, Some(LaneDirection::Southbound | LaneDirection::EastWestOnly))
                || matches!(to_col_dir, Some(LaneDirection::Southbound | LaneDirection::EastWestOnly));
            if block_north {
                expected = HtmTraversal::None;
            }
        }
    }

    // Vertical entry into access cell from non-station.
    if !either_blocked
        && !is_horizontal
        && presentation_cells.contains(&to.id.0)
        && !position_cells.contains(&from.id.0)
    {
        expected = HtmTraversal::None;
    }

    expected
}

fn is_htm_structural_mismatch(expected: HtmTraversal, actual: HtmTraversal) -> bool {
    match (expected, actual) {
        (HtmTraversal::Both, _) => false,
        (HtmTraversal::None, HtmTraversal::None) => false,
        (HtmTraversal::None, _) => true,
        (HtmTraversal::UnladenOnly, HtmTraversal::UnladenOnly | HtmTraversal::None) => false,
        (HtmTraversal::UnladenOnly, HtmTraversal::Both) => true,
    }
}

fn extract_junctions(map: &WarehouseMap, grid_id_max: u32) -> Vec<JunctionCell> {
    let mut junctions: Vec<_> = map
        .graph
        .cells()
        .filter(|c| c.grid_pos.is_none() && c.id.0 >= grid_id_max)
        .map(|cell| {
            let wp = cell.world_pos.unwrap_or(WorldPos { x_mm: 0.0, y_mm: 0.0 });
            let edges = map
                .graph
                .neighbors(cell.id)
                .iter()
                .map(|(nid, edge)| {
                    let bidi = map
                        .graph
                        .edge_between(*nid, cell.id)
                        .map(|rev| rev.traversability == edge.traversability && rev.distance_mm == edge.distance_mm)
                        .unwrap_or(false);
                    JunctionEdge {
                        target: nid.0,
                        traversability: edge.traversability,
                        distance_mm: edge.distance_mm,
                        bidirectional: bidi,
                    }
                })
                .collect();
            JunctionCell { id: cell.id.0, world_pos: [wp.x_mm, wp.y_mm], kind: cell.kind, zone: cell.zone.0, edges }
        })
        .collect();
    junctions.sort_by_key(|jc| jc.id);
    junctions
}

/// A v3 design states one presentation cell per station, so a station that
/// presents at several has no design to extract into, and is refused rather
/// than cut down to one of its points.
fn extract_stations(map: &WarehouseMap) -> Result<Vec<StationDefinition>, String> {
    map.stations
        .iter()
        .map(|s| {
            let [point] = s.presentation_points.as_slice() else {
                return Err(format!(
                    "station {} presents at {} cells, and a v3 design states one",
                    s.id.0,
                    s.presentation_points.len()
                ));
            };
            let pos_gp = map.graph.cell(s.position).unwrap().grid_pos.unwrap();
            let pc_gp = map.graph.cell(point.presentation_cell).unwrap().grid_pos.unwrap();
            let qp = point
                .queue_path
                .iter()
                .map(|&cid| {
                    let gp = map.graph.cell(cid).unwrap().grid_pos.unwrap();
                    [gp.x, gp.y]
                })
                .collect();
            Ok(StationDefinition {
                id: s.id.0,
                position: [pos_gp.x, pos_gp.y],
                presentation_cell: [pc_gp.x, pc_gp.y],
                edge: s.edge,
                queue_path: qp,
                exit_direction: s.exit_direction,
                bins: s.bins,
                station_type: s.station_type,
            })
        })
        .collect()
}
