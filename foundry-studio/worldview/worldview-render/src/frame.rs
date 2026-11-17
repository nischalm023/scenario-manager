//! Within `worldview-render`, `frame` is the RENDER COORDINATE SYSTEM: the single
//! place that converts the map's millimetres into the units the shaders draw
//! in, plus the map's overall extent for camera framing.
//!
//! **The unit is METRES, not millimetres.** Millimetres would put a
//! warehouse at ~340,000 units across, forcing a near/far ratio that
//! destroys `Depth32Float` resolution and z-fights rack floors against
//! beams. Metres keep a cell at ~0.86 units — within 15% of the 1.0 that
//! index space used — so camera distances, level heights, and the
//! orthographic zoom all keep their existing magnitudes while the geometry
//! stays physically true.
//!
//! **There is no Y flip and no axis model.** Both died with the v3 grid: a
//! world document's y is real northing and renders north-up unmirrored, and
//! every cell carries its own tile (`CellInfo::tile_mm`), so the frame no
//! longer holds per-axis centres — the extent is the tiles' bounding box.

use crate::scene::Cell;

/// The renderer's coordinate system for one map: a mm→m scale and the
/// extent the camera frames.
#[derive(Clone, Debug, Default)]
pub struct RenderFrame {
    extent_m: (f32, f32),
}

impl RenderFrame {
    /// Build from the cells' own tiles: the extent is their bounding box's
    /// far corner (maps anchor at the origin; a margin buys nothing the
    /// camera's fit factor doesn't already).
    pub fn from_tiles(cells: &[Option<Cell>]) -> Self {
        let mut max = [0.0f64; 2];
        for info in cells.iter().flatten() {
            let t = info.bounds_mm();
            max[0] = max[0].max(t[0] + t[2]);
            max[1] = max[1].max(t[1] + t[3]);
        }
        Self { extent_m: ((max[0] / 1000.0) as f32, (max[1] / 1000.0) as f32) }
    }

    /// Build from a stated extent in millimetres (mapless sessions).
    pub fn from_extent_mm(width_mm: f64, depth_mm: f64) -> Self {
        Self { extent_m: ((width_mm / 1000.0) as f32, (depth_mm / 1000.0) as f32) }
    }

    /// Render position of a world millimetre point. This is the path for
    /// everything — cells, bots, totes, labels — and it is a pure rescale:
    /// world documents render north-up, so y passes through unmirrored.
    pub fn world_mm(&self, x_mm: f64, y_mm: f64) -> [f32; 2] {
        [(x_mm / 1000.0) as f32, (y_mm / 1000.0) as f32]
    }

    /// Vertical metres from a height in millimetres.
    pub fn z_of_mm(&self, z_mm: f64) -> f32 {
        (z_mm / 1000.0) as f32
    }

    /// Millimetre position of a render-space point — the picking path's
    /// inverse of [`Self::world_mm`].
    pub fn mm_at_render(&self, x_m: f32, y_m: f32) -> [f64; 2] {
        [x_m as f64 * 1000.0, y_m as f64 * 1000.0]
    }

    /// Full extent in metres, as `(width, depth)`, for camera framing.
    pub fn extent(&self) -> (f32, f32) {
        self.extent_m
    }
}

/// Metres a floor-level object is lifted so it does not z-fight the floor.
pub const FLOOR_CLEARANCE_M: f32 = 0.01;

#[cfg(test)]
mod tests {
    use desgraph::CellKind;

    use super::*;
    use crate::scene::Tile;

    fn info(id: u32, tile: [f64; 4]) -> Option<Cell> {
        Some(Cell {
            id,
            kind: CellKind::Open,
            tile_mm: tile,
            heading_rad: 0.0,
            tile: Tile::Declared,
            wait_excluded: false,
        })
    }

    /// The extent is the tiles' far corner, and a world point is a pure
    /// rescale in both directions.
    #[test]
    fn extent_is_the_tile_bounding_box() {
        let cells = vec![info(0, [0.0, 0.0, 850.0, 850.0]), None, info(2, [850.0, 1700.0, 1227.0, 645.0])];
        let f = RenderFrame::from_tiles(&cells);
        let (w, d) = f.extent();
        assert!((w - 2.077).abs() < 1e-6);
        assert!((d - 2.345).abs() < 1e-6);
        assert_eq!(f.world_mm(725.0, 525.0), [0.725, 0.525]);
        let back = f.mm_at_render(0.725, 0.525);
        assert!((back[0] - 725.0).abs() < 1e-3 && (back[1] - 525.0).abs() < 1e-3);
    }
}
