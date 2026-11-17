//! Within `worldview-render`, `cell_index` is the SPATIAL LOOKUP over cell
//! tiles: which cell covers a millimetre point. Picking is the one thing a
//! pointer gesture ends in, and it works identically on lattice maps, on world
//! documents, and on a floor that states nothing but its cells' centres --
//! uniform spatial binning over the tiles themselves, never a per-axis search.

use crate::renderer::cell_layer::ground_bias;
use crate::scene::Cell;

/// Uniform-bin spatial index over cell tiles. Bins are an ACCELERATION
/// structure, not semantics: nothing outside `query` knows they exist, and
/// bin size just trades memory for candidates-per-probe.
pub struct CellIndex {
    origin_mm: [f64; 2],
    bin_mm: f64,
    cols: usize,
    rows: usize,
    /// Cell ids whose tile touches each bin.
    bins: Vec<Vec<u32>>,
}

impl CellIndex {
    pub fn build(cells: &[Option<Cell>]) -> Self {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let mut sizes: Vec<f64> = Vec::new();
        for info in cells.iter().flatten() {
            let t = info.bounds_mm();
            min[0] = min[0].min(t[0]);
            min[1] = min[1].min(t[1]);
            max[0] = max[0].max(t[0] + t[2]);
            max[1] = max[1].max(t[1] + t[3]);
            sizes.push(t[2].max(t[3]));
        }
        if sizes.is_empty() {
            return Self { origin_mm: [0.0, 0.0], bin_mm: 1.0, cols: 0, rows: 0, bins: Vec::new() };
        }
        sizes.sort_by(f64::total_cmp);
        // Median tile size: most probes hit 1-4 candidates.
        let bin_mm = sizes[sizes.len() / 2].max(1.0);
        let cols = (((max[0] - min[0]) / bin_mm).ceil() as usize).max(1);
        let rows = (((max[1] - min[1]) / bin_mm).ceil() as usize).max(1);
        let mut bins = vec![Vec::new(); cols * rows];
        for info in cells.iter().flatten() {
            let t = info.bounds_mm();
            let c0 = (((t[0] - min[0]) / bin_mm) as usize).min(cols - 1);
            let c1 = (((t[0] + t[2] - min[0]) / bin_mm) as usize).min(cols - 1);
            let r0 = (((t[1] - min[1]) / bin_mm) as usize).min(rows - 1);
            let r1 = (((t[1] + t[3] - min[1]) / bin_mm) as usize).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    bins[r * cols + c].push(info.id);
                }
            }
        }
        Self { origin_mm: min, bin_mm, cols, rows, bins }
    }

    /// The cell whose tile covers the point, if any. Ties (shared edges) go
    /// to the lowest id, deterministically.
    pub fn query(&self, x_mm: f64, y_mm: f64, cells: &[Option<Cell>]) -> Option<u32> {
        self.covering(x_mm, y_mm, cells).map(|cell| cell.id).min()
    }

    /// The cell DRAWN at the point: of the tiles covering it, the one on the
    /// nearest depth tier the renderer draws ground on (a marker over the
    /// declared tile it stands in), and the lowest id between two on one tier,
    /// where the drawing itself fights.
    pub fn drawn_at(&self, x_mm: f64, y_mm: f64, cells: &[Option<Cell>]) -> Option<u32> {
        let on_top = |a: &&Cell, b: &&Cell| ground_bias(a.tile).total_cmp(&ground_bias(b.tile)).then(a.id.cmp(&b.id));
        self.covering(x_mm, y_mm, cells).min_by(on_top).map(|cell| cell.id)
    }

    fn covering<'a>(&'a self, x_mm: f64, y_mm: f64, cells: &'a [Option<Cell>]) -> impl Iterator<Item = &'a Cell> {
        let c = ((x_mm - self.origin_mm[0]) / self.bin_mm).floor();
        let r = ((y_mm - self.origin_mm[1]) / self.bin_mm).floor();
        let inside =
            !self.bins.is_empty() && c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows;
        let bin = if inside { self.bins[r as usize * self.cols + c as usize].as_slice() } else { &[] };
        bin.iter().filter_map(|&id| cells.get(id as usize)?.as_ref()).filter(move |cell| cell.contains_mm(x_mm, y_mm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Tile;
    use desgraph::CellKind;

    #[test]
    fn picking_uses_the_rotated_footprint_including_its_extended_corners() {
        let cell = Cell {
            id: 0,
            kind: CellKind::Presentation,
            tile_mm: [650.0, 650.0, 700.0, 700.0],
            heading_rad: 45.0_f64.to_radians(),
            tile: Tile::Declared,
            wait_excluded: false,
        };
        let cells = vec![Some(cell)];
        let index = CellIndex::build(&cells);
        assert_eq!(index.query(1450.0, 1000.0, &cells), Some(0), "inside the diamond, outside the old square");
        assert_eq!(index.query(1330.0, 1330.0, &cells), None, "inside the old square, outside the diamond");
        assert_eq!(index.query(1000.0, 1000.0, &cells), Some(0));
    }

    /// A marker inside a declared tile is drawn over it, so the pick names
    /// the marker even where the tile's id is lower; two tiles on one tier go
    /// to the lower id (reviews-impl/astra-t12.md finding 2).
    #[test]
    fn the_cell_drawn_on_top_is_the_one_picked() {
        let tile = |id, tile_mm, tile| Cell {
            id,
            kind: CellKind::Open,
            tile_mm,
            heading_rad: 0.0,
            tile,
            wait_excluded: false,
        };
        let cells = vec![
            Some(tile(0, [0.0, 0.0, 2_000.0, 2_000.0], Tile::Declared)),
            Some(tile(1, [500.0, 500.0, 900.0, 900.0], Tile::Marker)),
            Some(tile(2, [0.0, 0.0, 2_000.0, 2_000.0], Tile::Declared)),
        ];
        let index = CellIndex::build(&cells);
        assert_eq!(index.query(900.0, 900.0, &cells), Some(0), "the lowest id covering the point");
        assert_eq!(index.drawn_at(900.0, 900.0, &cells), Some(1), "the marker, drawn on the nearest tier");
        assert_eq!(index.drawn_at(1_800.0, 1_800.0, &cells), Some(0), "two declared tiles: the lower id");
        assert_eq!(index.drawn_at(5_000.0, 5_000.0, &cells), None, "off the floor");
    }
}
