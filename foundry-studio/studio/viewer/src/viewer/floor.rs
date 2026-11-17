//! Within `viewer`, `floor` is A FLOOR SENT WITH NO RUN, AND THE CAMERA A PAGE
//! MOVES ITSELF: a floor document drawn with no movers, the keys under a point
//! and under the centre of the view, and the camera panned and zoomed by keys,
//! framed on entities, read and set (spec 03 section 3.6, what the viewer
//! needs).
//!
//! It exists for the Map panel and a template's evaluation, which show a site
//! rather than a run: the floor arrives compiled (`FloorAndRoll`, the API's
//! floor document), and every question is asked in entity keys, never in cell
//! indexes, since a page names things and the floor's cell ids are this
//! floor's alone.

use desgraph::CellId;
use wasm_bindgen::prelude::*;
use worldview_render::{CameraState, CellIndex, FloorAndRoll, PointerInput, SceneRenderer};

use super::{Drawing, Viewer};
use crate::roll::Roll;
use crate::screen::{FloorMm, ScreenPx};

/// One zoom step is the scroll that brings the camera a fifth closer:
/// `worldview-render`'s camera turns a scroll of `s` into a distance times
/// `exp(-0.002 s)`, and `ln(1 / 0.8) / 0.002` is about 111.6.
const ZOOM_STEP_SCROLL: f32 = 111.57;

/// The least extent, in metres, framing entities fits on the screen, so one
/// cell is framed with the floor around it rather than filling the view.
const FRAMED_AT_LEAST_M: f32 = 12.0;

#[wasm_bindgen]
impl Viewer {
    /// Take a floor document: draw its cells, steel and underlay with no
    /// movers, and forget any run. A second floor keeps the camera where it
    /// stands (a new changeset of the same site is the same place) and
    /// re-marks every outline's keys on it.
    pub fn set_floor(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let (floor, roll) = FloorAndRoll::read(bytes).map_err(|failure| JsValue::from_str(&failure.to_string()))?;
        let had_floor = self.drawing.is_some();
        self.run.take_floor(floor.into_scene());
        let Some(scene) = self.run.scene() else { return Ok(()) };
        if had_floor {
            self.camera.frame = scene.frame().clone();
        } else {
            self.camera = CameraState::new(scene.frame().clone());
        }
        self.floor_alone = true;
        self.floor_arrived(Roll::new(roll));
        Ok(())
    }

    /// The keys of the entities standing on the cell under a point on the
    /// canvas (CSS pixels), fewest cells first and the cell itself last;
    /// empty off the floor.
    pub fn pick_entities(&self, x: f32, y: f32) -> Vec<String> {
        self.entities_at(ScreenPx([f64::from(x), f64::from(y)]))
    }

    /// The same, for the cell under the centre of the view: what Enter on the
    /// page's stage selects.
    pub fn centre_entities(&self) -> Vec<String> {
        self.entities_at(self.centre_px())
    }

    /// Where the centre of the view meets the floor, `[x_mm, y_mm]` in the
    /// floor's millimetres, or empty where it meets no ground: the page's
    /// centre line.
    pub fn centre_mm(&self) -> Vec<f64> {
        self.floor_at(self.centre_px()).map_or_else(Vec::new, |FloorMm(at)| Vec::from(at))
    }

    /// Look down on every cell the keys stand on, fitted to the view with a
    /// margin: answers how many of the keys stand on cells, and leaves the
    /// camera where it was when none does. It jumps, as every camera move
    /// here does.
    pub fn frame_entities(&mut self, keys: Vec<String>) -> u32 {
        let Some(scene) = self.run.scene() else { return 0 };
        let (cells, standing) = self.roll.cells_of_all(keys.iter().map(String::as_str));
        let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for cell in cells.iter().filter_map(|&CellId(id)| scene.floor().cell(id)) {
            let [x, y, width, depth] = cell.bounds_mm();
            low = [low[0].min(x), low[1].min(y)];
            high = [high[0].max(x + width), high[1].max(y + depth)];
        }
        if standing == 0 || low[0] > high[0] {
            return 0;
        }
        let ([x0, y0], [x1, y1]) = (scene.frame().world_mm(low[0], low[1]), scene.frame().world_mm(high[0], high[1]));
        let (width, depth) = (((x1 - x0) * 1.25).max(FRAMED_AT_LEAST_M), ((y1 - y0) * 1.25).max(FRAMED_AT_LEAST_M));
        self.settle_camera();
        self.camera.snap_to_default();
        self.camera.orbit_target = [(x0 + x1) / 2.0, (y0 + y1) / 2.0, 0.0];
        self.camera.distance = CameraState::fit_distance(width, depth, self.viewport.width() / self.viewport.height());
        standing
    }

    /// Move the view `dx` CSS pixels right and `dy` down, so what stood that
    /// far from the centre stands at it: an arrow key pans a tenth of the view.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        let drag =
            PointerInput { viewport: self.viewport, dragged: true, drag_delta: [-dx, -dy], ..PointerInput::default() };
        self.camera.handle_input(&drag, true);
    }

    /// Zoom by whole steps about the centre of the view: positive is closer.
    pub fn zoom_by(&mut self, steps: f32) {
        let scroll = PointerInput {
            viewport: self.viewport,
            hovered: true,
            hover_pos: Some(self.viewport.centre()),
            scroll_delta: [0.0, steps * ZOOM_STEP_SCROLL],
            ..PointerInput::default()
        };
        self.camera.handle_input(&scroll, true);
    }

    /// Where the camera stands, `[target_x_m, target_y_m, target_z_m,
    /// distance_m, azimuth_rad, elevation_rad]`: what a page keeps to open the
    /// same view again.
    pub fn camera(&self) -> Vec<f32> {
        let [x, y, z] = self.camera.orbit_target;
        vec![x, y, z, self.camera.distance, self.camera.azimuth, self.camera.elevation]
    }

    /// Put the camera back where [`Self::camera`] said it stood.
    pub fn set_camera(&mut self, view: &[f32]) -> Result<(), JsValue> {
        let &[x, y, z, distance, azimuth, elevation] = view else {
            return Err(JsValue::from_str(&format!("a camera is six numbers, and {} were given", view.len())));
        };
        if view.iter().any(|value| !value.is_finite()) || distance <= 0.0 {
            return Err(JsValue::from_str("a camera's numbers are finite and its distance is above zero"));
        }
        self.settle_camera();
        self.camera.snap_to_default();
        self.camera.orbit_target = [x, y, z];
        (self.camera.distance, self.camera.azimuth, self.camera.elevation) = (distance, azimuth, elevation);
        Ok(())
    }
}

impl Viewer {
    /// A floor has arrived, from a scenario block or alone: its pipelines and
    /// its index are built, its roll kept, and every outline marks its keys
    /// where they stand on it. The camera is the caller's.
    pub(super) fn floor_arrived(&mut self, roll: Roll) {
        let Some(scene) = self.run.scene() else { return };
        self.drawing = Some(Drawing {
            renderer: SceneRenderer::new(&self.gpu.device, self.gpu.format(), scene),
            cells: CellIndex::build(scene.cells()),
        });
        self.roll = roll;
        self.outlines.refloor(&self.roll, scene.cells());
        self.drawn_counts = [0; 3];
    }

    /// Let the camera take in the viewport before anything sets it by hand:
    /// it fits itself to the floor the first time it is handed one, which
    /// would otherwise undo a camera set before the first frame.
    pub(super) fn settle_camera(&mut self) {
        let still = PointerInput { viewport: self.viewport, ..PointerInput::default() };
        self.camera.handle_input(&still, true);
    }

    /// The centre of the view, where the page's centre cross is drawn.
    pub(super) fn centre_px(&self) -> ScreenPx {
        let [x, y] = self.viewport.centre();
        ScreenPx([f64::from(x), f64::from(y)])
    }

    /// Where a point on the canvas meets the floor, nil where it meets no ground.
    pub(super) fn floor_at(&self, ScreenPx([x, y]): ScreenPx) -> Option<FloorMm> {
        self.camera.screen_to_mm([x as f32, y as f32], self.viewport).map(FloorMm)
    }

    /// The cell drawn at a point on the canvas: where tiles overlap, the one
    /// the renderer draws on top.
    pub(super) fn cell_at(&self, at: ScreenPx) -> Option<CellId> {
        let (drawing, scene) = (self.drawing.as_ref()?, self.run.scene()?);
        let FloorMm([x, y]) = self.floor_at(at)?;
        drawing.cells.drawn_at(x, y, scene.cells()).map(CellId)
    }

    fn entities_at(&self, at: ScreenPx) -> Vec<String> {
        self.cell_at(at).map(|cell| self.roll.keys_at(cell)).unwrap_or_default()
    }
}
