//! Within desvis, `export_png` is the map drawn WITHOUT a GPU: a CPU
//! rasterizer that paints each cell's own tile in millimetres and writes a
//! PNG, for a caller that has no window — a script, a report, a machine over
//! ssh.
//!
//! It draws cells and nothing else: no underlay, no props, no bots. Its two
//! obligations to the GPU path are that a cell's COLOUR agrees with
//! `shaders/grid.wgsl`, so the two pictures name the same kinds the same way,
//! and that north is up, as the window draws a world document.

use std::path::Path;

use desgraph::types::CellKind;
use image::{Rgb, RgbImage};

use crate::types::CellInfo;

/// Output pixels per nominal cell pitch (the mm→px scale anchor).
const CELL_PX: f64 = 4.0;

/// Map CellKind to an RGB color (matches shaders/grid.wgsl palette).
fn cell_color(kind: CellKind) -> [u8; 3] {
    match kind {
        CellKind::Highway => [135, 206, 250], // light blue
        CellKind::Aisle => [180, 220, 255],   // pale blue
        // Open is a cell and paints like one, since the ruling of 2026-09-04
        // ("All cells should be drawn"). The grey is `grid.wgsl`'s 0.851.
        CellKind::Open => [217, 217, 217],        // open floor
        CellKind::Storage => [237, 201, 71],      // yellow-gold
        CellKind::Relay => [102, 199, 120],       // green
        CellKind::StationArea => [176, 125, 212], // purple
        CellKind::Junction => [230, 77, 77],      // red
        CellKind::Blank => [255, 255, 255],       // white
        CellKind::Presentation => [255, 140, 0],  // orange
        CellKind::QueueCell => [255, 200, 100],   // amber
        CellKind::Charging => [135, 206, 250],    // soft blue
        CellKind::Exchange => [46, 139, 87],      // sea green: belt handoff (dropoff/collection) cells
    }
}

/// Render the cells as a 2D top-down PNG image and save it to `path`.
///
/// Each cell paints its own tile at `CELL_PX / nominal_mm` pixels per
/// millimetre; uncovered floor stays white. Tiles carry their true extents,
/// so non-uniform maps come out dimensionally faithful rather than one block
/// per index.
pub fn export_png(cell_infos: &[Option<CellInfo>], nominal_mm: f64, path: &Path) -> Result<(), String> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for info in cell_infos.iter().flatten() {
        let t = info.bounds_mm();
        min[0] = min[0].min(t[0]);
        min[1] = min[1].min(t[1]);
        max[0] = max[0].max(t[0] + t[2]);
        max[1] = max[1].max(t[1] + t[3]);
    }
    if !min[0].is_finite() {
        return Err("map has no positioned cells".into());
    }

    let scale = CELL_PX / nominal_mm.max(1.0);
    let img_w = (((max[0] - min[0]) * scale).ceil() as u32).max(1);
    let img_h = (((max[1] - min[1]) * scale).ceil() as u32).max(1);

    let mut img = RgbImage::from_pixel(img_w, img_h, Rgb([255, 255, 255]));

    // North-up, as the renderer draws a world document: a document's y is
    // northing, so the top row of the image is the largest y.
    for info in cell_infos.iter().flatten() {
        let t = info.bounds_mm();
        let color = Rgb(cell_color(info.kind));
        let x0 = ((t[0] - min[0]) * scale) as u32;
        let y0 = ((max[1] - (t[1] + t[3])) * scale) as u32;
        let x1 = (((t[0] + t[2] - min[0]) * scale) as u32).min(img_w);
        let y1 = (((max[1] - t[1]) * scale) as u32).min(img_h);
        for y in y0..y1 {
            for x in x0..x1 {
                if info.contains_mm(min[0] + (x as f64 + 0.5) / scale, max[1] - (y as f64 + 0.5) / scale) {
                    img.put_pixel(x, y, color);
                }
            }
        }
    }

    img.save(path).map_err(|e| format!("failed to save PNG: {}", e))
}
