//! Spatial separation estimates for prescreening candidate cells.
//!
//! The parent module commits to routes; this module only guesses at them. Per
//! the coding standards, a spatial estimate may rank candidates but never
//! decide one: everything here feeds a shortlist that the pathfinder then
//! evaluates exactly.
//!
//! Its reason to exist is that callers were reading `grid_pos` and subtracting
//! **indices**. That is only proportional to distance when every cell is the
//! same size. On a surveyed map it is not: cells run 684.5 to 1,325mm on the
//! Dallas geometry, so index arithmetic ranks a far candidate ahead of a near
//! one wherever widths differ along the route, and a cell with no grid position
//! at all cannot be ranked. Both problems disappear by measuring in
//! millimetres, which every cell carries.

use desgraph::CellId;
use desgraph::types::WorldPos;
use desmap::map::WarehouseMap;

use crate::model::SimDuration;

/// Where a cell sits, in millimetres, for the purpose of estimating.
///
/// Prefers the cell's own world position. Falls back to its grid position
/// scaled by the map's nominal cell size, which is exact on a uniform map and
/// is the best available reading on any map that predates world positions.
/// `None` when a cell carries neither, which is a real defect in the map rather
/// than a case to paper over.
pub fn cell_point_mm(map: &WarehouseMap, cell: CellId) -> Option<WorldPos> {
    if let Some(world) = map.cell_world_pos(cell) {
        return Some(world);
    }
    let grid = map.cell_grid_pos(cell)?;
    let size = map.cell_size_mm() as f64;
    Some(WorldPos { x_mm: grid.x as f64 * size, y_mm: grid.y as f64 * size })
}

/// Manhattan separation in millimetres.
///
/// Manhattan rather than Euclidean because robots travel the aisles, not the
/// diagonal.
pub fn manhattan_mm(a: WorldPos, b: WorldPos) -> f64 {
    (a.x_mm - b.x_mm).abs() + (a.y_mm - b.y_mm).abs()
}

/// Manhattan separation between two cells, in millimetres. `None` when either
/// cell has no position at all.
pub fn separation_mm(map: &WarehouseMap, a: CellId, b: CellId) -> Option<f64> {
    Some(manhattan_mm(cell_point_mm(map, a)?, cell_point_mm(map, b)?))
}

/// Turn a millimetre separation into a travel-time estimate.
///
/// `traverse` is the time to cross one cell, so the conversion is "how many
/// nominal cells is this, times how long a cell takes". On a non-uniform map
/// the nominal cell size is an approximation, which is acceptable precisely
/// because this only orders a shortlist.
pub fn travel_estimate(distance_mm: f64, cell_size_mm: u64, traverse: SimDuration) -> SimDuration {
    let size = if cell_size_mm == 0 { 1.0 } else { cell_size_mm as f64 };
    let cells = (distance_mm / size).max(0.0);
    SimDuration::ms((traverse.as_ms() as f64 * cells).round() as u64)
}

/// Estimated travel time between two cells. `None` when either is unpositioned.
pub fn travel_estimate_between(map: &WarehouseMap, a: CellId, b: CellId, traverse: SimDuration) -> Option<SimDuration> {
    Some(travel_estimate(separation_mm(map, a, b)?, map.cell_size_mm(), traverse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use desgraph::types::{CellKind, GridPos, SystemType, ZoneId};
    use desgraph::{Cell, WarehouseGraphBuilder};
    use desmap::map::WarehouseMapBuilder;

    fn cell(id: u32, grid: Option<GridPos>, world: Option<WorldPos>) -> Cell {
        Cell {
            id: CellId(id),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: grid,
            world_pos: world,
            barcode: None,
            rack_orientation: None,
            station_id: None,
            htm_rotation_forbidden: false,
        }
    }

    fn map_of(cells: Vec<Cell>) -> WarehouseMap {
        let mut builder = WarehouseGraphBuilder::new();
        for c in cells {
            builder.add_cell(c);
        }
        WarehouseMapBuilder::new(SystemType::Relay).graph(builder.build()).build()
    }

    #[test]
    fn a_cell_with_a_world_position_is_measured_in_millimetres() {
        let map = map_of(vec![
            cell(0, Some(GridPos { x: 0, y: 0 }), Some(WorldPos { x_mm: 0.0, y_mm: 0.0 })),
            cell(1, Some(GridPos { x: 1, y: 0 }), Some(WorldPos { x_mm: 1325.0, y_mm: 0.0 })),
        ]);
        assert_eq!(separation_mm(&map, CellId(0), CellId(1)), Some(1325.0));
    }

    /// The whole point of the module: two candidates one grid step away are not
    /// equidistant when the cells differ in width, and index arithmetic says
    /// they are.
    #[test]
    fn non_uniform_cells_rank_by_distance_not_by_index() {
        let map = map_of(vec![
            cell(0, Some(GridPos { x: 1, y: 0 }), Some(WorldPos { x_mm: 1000.0, y_mm: 0.0 })),
            cell(1, Some(GridPos { x: 0, y: 0 }), Some(WorldPos { x_mm: 315.5, y_mm: 0.0 })),
            cell(2, Some(GridPos { x: 2, y: 0 }), Some(WorldPos { x_mm: 1200.0, y_mm: 0.0 })),
        ]);
        let near = separation_mm(&map, CellId(0), CellId(2)).unwrap();
        let far = separation_mm(&map, CellId(0), CellId(1)).unwrap();
        assert!(near < far, "cell 2 is 200mm away, cell 1 is 684.5mm; one index step each");
    }

    #[test]
    fn a_cell_with_no_grid_position_is_still_measurable() {
        let map = map_of(vec![
            cell(0, Some(GridPos { x: 0, y: 0 }), Some(WorldPos { x_mm: 0.0, y_mm: 0.0 })),
            cell(1, None, Some(WorldPos { x_mm: 900.0, y_mm: 400.0 })),
        ]);
        assert_eq!(separation_mm(&map, CellId(0), CellId(1)), Some(1300.0));
    }

    #[test]
    fn a_cell_with_only_a_grid_position_falls_back_to_the_nominal_size() {
        let map =
            map_of(vec![cell(0, Some(GridPos { x: 0, y: 0 }), None), cell(1, Some(GridPos { x: 3, y: 0 }), None)]);
        // The nominal cell size for a map without grid bounds is 1000mm.
        assert_eq!(separation_mm(&map, CellId(0), CellId(1)), Some(3000.0));
    }

    #[test]
    fn a_cell_with_no_position_at_all_yields_none() {
        let map = map_of(vec![cell(0, Some(GridPos { x: 0, y: 0 }), None), cell(1, None, None)]);
        assert_eq!(separation_mm(&map, CellId(0), CellId(1)), None);
    }

    #[test]
    fn travel_estimate_scales_one_nominal_cell_to_one_traverse() {
        let traverse = SimDuration::ms(1000);
        assert_eq!(travel_estimate(850.0, 850, traverse), SimDuration::ms(1000));
        assert_eq!(travel_estimate(1700.0, 850, traverse), SimDuration::ms(2000));
        assert_eq!(travel_estimate(0.0, 850, traverse), SimDuration::ms(0));
    }

    /// Guards the divide: a map with no declared cell size must not panic or
    /// produce an infinite estimate.
    #[test]
    fn a_zero_nominal_cell_size_does_not_divide_by_zero() {
        let estimate = travel_estimate(500.0, 0, SimDuration::ms(10));
        assert_eq!(estimate, SimDuration::ms(5000));
    }
}
