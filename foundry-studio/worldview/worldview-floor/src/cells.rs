//! Within `worldview-floor`, `cells` is WHERE EACH CELL IS DRAWN AND WHICH
//! HOPS JOIN THEM: a compiled map's cells as the renderer's tiles, and its
//! graph edges as the segments arrows are drawn along.
//!
//! This is the once-per-map half of the compile — nothing here changes while a
//! run plays. It was desvis's `build_grid_data`, and moved here whole when a
//! browser needed the same tiles: a floor drawn from centres alone puts a
//! port's exchange point and a turned presentation cell somewhere desvis does
//! not, and two viewers of one run disagreeing about where a cell is drawn is
//! exactly what the shared renderer exists to prevent.

use desmap::WarehouseMap;
use worldview_render::{Cell, Edge, Tile};

/// Every cell's drawn tile, dense by cell id: a hole is an id the map does not
/// have, or one it places nowhere.
///
/// `nominal_mm` is the fallback square for a cell the SIMULATOR minted, which
/// carries no size at all. A cell the DOCUMENT places as a point does carry
/// one — the reader's default — and keeps it; which tier the two draw on is
/// the [`Tile`] beside the size, and comes from `footprint_stated` rather than
/// from the number, because a stated size and a defaulted one read alike here.
pub fn tiles(
    map: &WarehouseMap,
    cell_sizes_mm: &[[f64; 2]],
    footprint_stated: &[bool],
    cell_headings_rad: &[f64],
    nominal_mm: f64,
) -> Vec<Option<Cell>> {
    let highest = map.graph.cells().map(|cell| cell.id.0).max().unwrap_or(0) as usize;
    let mut drawn: Vec<Option<Cell>> = vec![None; highest + 1];
    // Which cells the waiting overlay leaves out. It is a fact about the map's
    // operational roles — a station's queue, a relay's board — so it is read
    // here, where the map is, and travels with the cell it belongs to.
    let wait_excluded = map.navigation_wait_exclusions();
    for cell in map.graph.cells() {
        let Some(at) = cell.world_pos else { continue };
        let stated = footprint_stated.get(cell.id.0 as usize).copied().unwrap_or(false);
        let size = cell_sizes_mm
            .get(cell.id.0 as usize)
            .copied()
            .filter(|size| size[0] > 0.0 && size[1] > 0.0)
            .unwrap_or([nominal_mm * 0.6, nominal_mm * 0.6]);
        drawn[cell.id.0 as usize] = Some(Cell {
            id: cell.id.0,
            kind: cell.kind,
            tile_mm: [at.x_mm - size[0] / 2.0, at.y_mm - size[1] / 2.0, size[0], size[1]],
            heading_rad: cell_headings_rad.get(cell.id.0 as usize).copied().unwrap_or(0.0),
            tile: if stated { Tile::Declared } else { Tile::Marker },
            wait_excluded: wait_excluded.contains_key(&cell.id.0),
        });
    }
    drawn
}

/// Every hop between two positioned cells, as the straight segments an arrow
/// is drawn from. A curved hop becomes several segments, fine enough that the
/// chord never strays far from the arc.
pub fn edges(map: &WarehouseMap) -> Vec<Edge> {
    let mut edges: Vec<Edge> = Vec::new();
    for cell in map.graph.cells() {
        let Some(from_world) = cell.world_pos else { continue };
        for (neighbor_id, edge) in map.graph.neighbors(cell.id) {
            let Some(neighbor) = map.graph.cell(neighbor_id) else { continue };
            let Some(to_world) = neighbor.world_pos else { continue };
            let segments = edge.curve.map_or(1, |arc| {
                ((arc.sweep(from_world, to_world).abs() / 0.2).ceil() as usize)
                    .max((arc.length_mm(from_world, to_world) / 150.0).ceil() as usize)
                    .max(1)
            });
            for segment in 0..segments {
                let (from, to) = edge.curve.map_or((from_world, to_world), |arc| {
                    (
                        arc.point(from_world, to_world, segment as f64 / segments as f64),
                        arc.point(from_world, to_world, (segment + 1) as f64 / segments as f64),
                    )
                });
                edges.push(Edge {
                    from_mm: [from.x_mm, from.y_mm],
                    to_mm: [to.x_mm, to.y_mm],
                    vtm_traversal: edge.traversability.vtm_traversal,
                    htm_traversal: edge.traversability.htm_traversal,
                });
            }
        }
    }
    edges
}
