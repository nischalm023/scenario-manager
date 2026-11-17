//! Within `crucible-viewer`, `viewer` is WHAT A TAB HOLDS: one canvas, the GPU
//! behind it, the camera looking at the run, and the `#[wasm_bindgen]` surface
//! a TypeScript shell drives all three through.
//!
//! It exists to be the only place the browser's world meets the run's. Frames
//! arrive as bytes and go straight to [`Run`]; gestures arrive as numbers and
//! go straight to [`CameraState`]; and once a frame the two answers are turned
//! into a [`Moment`] and drawn. Nothing here decides anything about a run — a
//! difference between this picture and desvis's is in `worldview-render`, not
//! in this file.
//!
//! A viewer is built before anything about the run is known, because the floor
//! arrives as a frame like everything else. So construction makes the GPU
//! surface alone, and the scenario block builds the camera around the floor it
//! carries. A floor can also come alone (`set_floor`, a site's or an
//! evaluation's; spec 03 section 3.6), and the calls that name entities, mark
//! them and move the camera to them are in [`floor`] and [`marks`]; they work
//! on either floor, so a run's playback takes highlights with no second
//! mechanism.
//!
//! wasm32 only (see `lib.rs`'s gate): everything below hangs off a browser
//! canvas.
//!
//! TODO(overstuffed): about thirty comments against a budget of fifteen, most
//! of them the recording calls' own; those and the drawing could part, as the
//! floor-source calls have into `floor`, `marks` and `paint`.

use desgraph::CellId;
use desunits::SimTime;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use worldview_render::{
    CameraState, CellIndex, HeatmapMode, Measured, Overlays, Playback, SceneRenderer, TrafficTints, ViewMode, Viewport,
};

use crate::canvas::Canvas;
use crate::gesture::Gesture;
use crate::outline::Outlines;
use crate::palette::Palette;
use crate::roll::Roll;
use crate::run::{Run, Took};
use crate::screen::{Axes, ScreenPx};

mod floor;
mod marks;
mod paint;
mod projection;

use paint::Lines;
pub(crate) use projection::Projection;

/// What is painted over the cells, if anything.
///
/// The three measured overlays are the run's own answers about the floor, so
/// switching one on is switching to the measured view — the robots and
/// containers come off it, because the tint is the answer and movers standing
/// over it would be read as part of it. That is desvis's own rule.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Off,
    /// How often robots stood on each cell during the cursor's own slice.
    Visits,
    /// How much of a cell's exposure HTMs spent waiting on it.
    Wait,
    /// The mean speed HTMs crossed it at.
    Speed,
}

impl Overlay {
    fn view(self) -> Option<ViewMode> {
        match self {
            Self::Off => None,
            Self::Visits => Some(ViewMode::Simulation(Measured::Visits)),
            Self::Wait => Some(ViewMode::Simulation(Measured::Wait)),
            Self::Speed => Some(ViewMode::Simulation(Measured::Speed)),
        }
    }

    fn mode(self) -> HeatmapMode {
        match self {
            Self::Off => HeatmapMode::Off,
            Self::Visits => HeatmapMode::Combined,
            Self::Wait => HeatmapMode::HtmWait,
            Self::Speed => HeatmapMode::HtmSpeed,
        }
    }
}

/// One canvas's viewer: the run it holds, the GPU it draws on, and where the
/// person looking is standing.
#[wasm_bindgen]
pub struct Viewer {
    gpu: Canvas,
    run: Run,
    /// The pipelines this floor is drawn with and the index a pointer is
    /// turned into a cell through, built when the floor arrives: they are one
    /// map's, not one frame's.
    drawing: Option<Drawing>,
    camera: CameraState,
    viewport: Viewport,
    gesture: Gesture,
    cursor_ms: f64,
    speed: Option<f64>,
    view: ViewMode,
    overlay: Overlay,
    drawn_counts: [u32; 3],
    /// Whether the floor came alone, so it is drawn with no run on it rather
    /// than waiting for a keyframe that will never arrive.
    floor_alone: bool,
    roll: Roll,
    outlines: Outlines,
    palette: Palette,
    axes: Option<Axes>,
    centre_cross: bool,
    /// The 2D canvas a page laid over this one, and the device pixels per CSS
    /// pixel both are sized by.
    lines: Option<Lines>,
    ratio: f64,
    outlined_counts: [u32; 3],
    /// Whether the page sized the canvas to nothing, as it does a hidden panel.
    hidden: bool,
    projection: Projection,
}

struct Drawing {
    renderer: SceneRenderer,
    cells: CellIndex,
}

#[wasm_bindgen]
impl Viewer {
    /// Open a viewer on a canvas. Asynchronous because asking a browser for a
    /// GPU adapter is, which is why it is a static method and not a JS `new`.
    pub async fn new(canvas: HtmlCanvasElement) -> Result<Viewer, JsValue> {
        // Without this a panic anywhere in this crate reaches the tab as
        // "unreachable executed" and nothing else.
        console_error_panic_hook::set_once();
        let (width, height) = (canvas.width().max(1) as f32, canvas.height().max(1) as f32);
        let gpu = Canvas::new(canvas).await?;
        Ok(Self {
            gpu,
            run: Run::default(),
            drawing: None,
            camera: CameraState::new(Default::default()),
            viewport: Viewport::new([0.0, 0.0], [width, height]),
            gesture: Gesture::default(),
            cursor_ms: 0.0,
            speed: None,
            view: ViewMode::State,
            overlay: Overlay::Off,
            drawn_counts: [0; 3],
            floor_alone: false,
            roll: Roll::default(),
            outlines: Outlines::default(),
            palette: Palette::default(),
            axes: None,
            centre_cross: false,
            lines: None,
            ratio: 1.0,
            outlined_counts: [0; 3],
            hidden: false,
            projection: Projection::default(),
        })
    }

    /// Take one worldview frame, header included, exactly as the server sent
    /// it: the scenario block builds the floor, the keyframe opens the run,
    /// each delta extends what can be scrubbed.
    pub fn push_frame(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let took = self.run.push_frame(bytes).map_err(|failure| JsValue::from_str(&failure.to_string()))?;
        let Some(scene) = self.run.scene() else { return Ok(()) };
        // A scenario block is a floor, and a second one is a different floor:
        // the pipelines hold its cells and the index its squares, so both are
        // built again rather than left describing the last attachment's map.
        if self.drawing.is_none() || took == Took::Scenario {
            self.camera = CameraState::new(scene.frame().clone());
            self.floor_alone = false;
            self.floor_arrived(Roll::of_census(self.run.census().clone()));
        }
        Ok(())
    }

    /// Why the run stopped being drawable, or an empty string while it is.
    ///
    /// A stream that broke its own shape cannot be drawn on from, and the
    /// canvas alone cannot say so — it would simply stop changing. The shell
    /// asks after each frame and shows what this says.
    pub fn stopped(&self) -> String {
        self.run.stopped().map(|why| why.to_string()).unwrap_or_default()
    }

    /// Forget every frame this attachment sent, keeping the floor. The shell
    /// calls this before attaching somewhere else, so the frames that follow
    /// start a window of their own instead of joining one they have no
    /// continuity with.
    pub fn clear(&mut self) {
        self.run.clear();
    }

    /// The stretch the cursor can be moved within without attaching again, as
    /// `[from_ms, to_ms]`. Empty until a keyframe has arrived.
    pub fn held_window(&self) -> Vec<f64> {
        match self.run.held_window() {
            Some((from, to)) => vec![from.as_ms() as f64, to.as_ms() as f64],
            None => Vec::new(),
        }
    }

    pub fn set_cursor(&mut self, ms: f64) {
        self.cursor_ms = ms.max(0.0);
    }

    pub fn cursor_ms(&self) -> f64 {
        self.cursor_ms
    }

    /// Play forward at `speed` simulated seconds per real second. Playing past
    /// the held window stops at its edge, which is the shell's cue to attach
    /// again rather than this viewer's to do it.
    pub fn play(&mut self, speed: f64) {
        self.speed = Some(speed);
    }

    pub fn pause(&mut self) {
        self.speed = None;
    }

    pub fn playing(&self) -> bool {
        self.speed.is_some()
    }

    pub fn ended(&self) -> bool {
        self.run.ended()
    }

    pub fn has_floor(&self) -> bool {
        self.drawing.is_some()
    }

    pub fn backend(&self) -> String {
        self.gpu.backend.to_string()
    }

    /// What the last frame SUBMITTED: robots, containers, cells. A view that
    /// is about the cells draws no movers however many rows were built, and a
    /// frame that drew nothing at all reports zeroes — which is what makes
    /// this worth checking a picture against.
    pub fn drawn_counts(&self) -> Vec<u32> {
        self.drawn_counts.to_vec()
    }

    /// One metric's running value where the cursor stands, or NaN for a name
    /// the run never declared.
    pub fn metric(&self, name: &str) -> f64 {
        self.run.metric(name).unwrap_or(f64::NAN)
    }

    /// Every metric this run declared, in slot order: what a chart can offer.
    pub fn metric_names(&self) -> Vec<String> {
        self.run.metric_names()
    }

    /// One metric's running value at the end of each slice this attachment
    /// holds, flattened to `[t_ms, value, t_ms, value, …]` because that is what
    /// crosses to a tab as one array of numbers. A chart draws the server's
    /// `series` up to the attach instant and this after it.
    pub fn metric_series(&self, name: &str) -> Vec<f64> {
        self.run.metric_series(name).into_iter().flat_map(|(at, value)| [at as f64, value]).collect()
    }

    /// What the viewer is looking at: `state`, `unladen-htm`, `laden-htm`,
    /// `vtm` or `simulation`. Robots and containers stand on the floor in
    /// `state` alone; the others are about the cells, and movers over them
    /// would be read as part of the answer.
    pub fn set_view_mode(&mut self, mode: &str) -> Result<(), JsValue> {
        self.view = match mode {
            "state" => ViewMode::State,
            "unladen-htm" => ViewMode::UnladenHtm,
            "laden-htm" => ViewMode::LadenHtm,
            "vtm" => ViewMode::Vtm,
            "simulation" => ViewMode::Simulation(Measured::Nothing),
            other => {
                return Err(JsValue::from_str(&format!(
                    "`{other}` is not a view this viewer has; they are state, unladen-htm, laden-htm, vtm, simulation"
                )));
            }
        };
        Ok(())
    }

    /// What to paint over the cells: `off`, `visits` (the cursor's own slice),
    /// `wait` or `speed` (the HTM traffic measured from the attachment's
    /// keyframe to the cursor). Anything on but `off` puts the viewer in the
    /// measured view, which draws no movers.
    pub fn set_overlay(&mut self, overlay: &str) -> Result<(), JsValue> {
        self.overlay = match overlay {
            "off" => Overlay::Off,
            "visits" => Overlay::Visits,
            "wait" => Overlay::Wait,
            "speed" => Overlay::Speed,
            other => {
                return Err(JsValue::from_str(&format!(
                    "`{other}` is not an overlay this viewer paints; they are off, visits, wait, speed"
                )));
            }
        };
        Ok(())
    }

    /// The canvas's size in device pixels, and how many of those a CSS pixel
    /// is. A size of nothing hides the viewer: nothing is drawn, and the view
    /// keeps its last real size, so a camera move asked meanwhile is made
    /// against the view the page will show again.
    pub fn resize(&mut self, width: u32, height: u32, device_pixel_ratio: f32) {
        self.hidden = width == 0 || height == 0;
        if self.hidden {
            return;
        }
        self.gpu.resize(width, height);
        let ratio = if device_pixel_ratio > 0.0 { device_pixel_ratio } else { 1.0 };
        self.viewport = Viewport::new([0.0, 0.0], [width as f32 / ratio, height as f32 / ratio]);
        self.ratio = f64::from(ratio);
        if let Some(lines) = &self.lines {
            lines.resize(width, height);
        }
    }

    /// Which cell's square holds a point on the canvas, or −1 for the floor's
    /// outside. The squares are the ones drawn, turned cells included.
    pub fn pick(&self, x: f32, y: f32) -> i32 {
        self.cell_at(ScreenPx([f64::from(x), f64::from(y)])).map_or(-1, |CellId(cell)| cell as i32)
    }

    pub fn pointer_down(&mut self, x: f32, y: f32, middle: bool) {
        self.gesture.down(x, y, middle);
    }

    pub fn pointer_move(&mut self, x: f32, y: f32, shift: bool) {
        self.gesture.moved(x, y, shift);
    }

    pub fn pointer_up(&mut self) {
        self.gesture.up();
    }

    pub fn pointer_left(&mut self) {
        self.gesture.left();
    }

    pub fn wheel(&mut self, delta_x: f32, delta_y: f32, shift: bool) {
        self.gesture.wheel(delta_x, delta_y, shift);
    }

    /// Look closer at the place under a point on the canvas, as desvis's
    /// double-click does, but in one jump: every camera move here jumps (the
    /// proof of concept's cut), so no camera ever flies, whoever is looking
    /// and whatever their `prefers-reduced-motion` (spec 03 section 5).
    pub fn fly_to(&mut self, x: f32, y: f32, keep_elevation: bool) {
        if let Some([x_mm, y_mm]) = self.camera.screen_to_mm([x, y], self.viewport) {
            let [wx, wy] = self.camera.frame.world_mm(x_mm, y_mm);
            self.camera.fly_to_world(wx, wy, keep_elevation);
            self.camera.land();
        }
    }

    /// Back to the whole floor from above, in one jump.
    pub fn reset_camera(&mut self) {
        self.settle_camera();
        self.camera.snap_to_default();
    }

    /// Draw one frame, `dt_seconds` of wall clock after the last: the shell
    /// calls this from `requestAnimationFrame` and nowhere else. Answers
    /// whether a frame of the floor was presented: not while the canvas has
    /// no size, the surface is not handed over, or no floor is held, so a
    /// copy of the canvas is made only of a frame this call drew.
    ///
    /// The order is the order the answers depend on each other in — the
    /// gesture moves the camera, the wall clock moves the cursor through
    /// simulated time, the run says what is true there, and only then is
    /// anything uploaded.
    pub fn frame(&mut self, dt_seconds: f32) -> Result<bool, JsValue> {
        self.gesture.apply(&mut self.camera, self.viewport, self.projection);
        self.projection.hold(&mut self.camera);

        if let (Some(speed), Some((_, to))) = (self.speed, self.run.held_window()) {
            self.cursor_ms = (self.cursor_ms + f64::from(dt_seconds) * 1_000.0 * speed).min(to.as_ms() as f64);
        }
        if self.hidden {
            (self.drawn_counts, self.outlined_counts) = ([0; 3], [0; 3]);
            return Ok(false);
        }
        self.draw().map_err(|why| JsValue::from_str(&why))
    }
}

impl Viewer {
    /// Turn the cursor's instant into rows and put them on screen. A viewer
    /// with no floor yet clears the canvas and nothing more, which is what a
    /// tab looks like between asking to attach and being answered.
    fn draw(&mut self) -> Result<bool, String> {
        // The run advances first and everything else is read from where it
        // then stands: an overlay taken before the cursor's slice was applied
        // would paint the slice before this one over this one's robots.
        let cursor = SimTime::ms(self.cursor_ms.max(0.0) as u64);
        let drawn = match self.run.drawn_at(cursor) {
            Some(Ok(drawn)) => Some(drawn),
            Some(Err(stopped)) => return Err(stopped.to_string()),
            None => None,
        };
        let view = self.overlay.view().unwrap_or(self.view);
        let cell_size = self.run.scene().map_or(1.0, |scene| scene.floor().cell_size_mm() as f32 / 1_000.0);
        let uniforms = self.camera.build_uniforms(self.viewport, cell_size, view);
        let visits = matches!(self.overlay, Overlay::Visits).then(|| self.run.slice_overlay()).flatten();
        let nothing_measured = ();
        let traffic: &dyn TrafficTints = match self.run.traffic() {
            Some(traffic) => traffic,
            None => &nothing_measured,
        };

        let moment = match (drawn.as_ref(), self.run.scene()) {
            (Some(drawn), Some(scene)) => Some(scene.moment(
                uniforms,
                view.arrows(),
                &drawn.playback(Overlays {
                    heatmap: visits.as_ref(),
                    mode: self.overlay.mode(),
                    traffic,
                    reserved_cells: &drawn.reserved_cells,
                }),
            )),
            (None, Some(scene)) if self.floor_alone => Some(scene.moment(uniforms, view.arrows(), &still())),
            _ => None,
        };
        if let (Some(drawing), Some(moment)) = (&mut self.drawing, &moment) {
            drawing.renderer.upload(&self.gpu.device, &self.gpu.queue, moment);
        }
        let presented = self.gpu.painted(self.palette.floor.rgba, |pass| {
            if let (Some(drawing), Some(moment)) = (&self.drawing, &moment) {
                drawing.renderer.draw(pass, moment);
            }
        });
        // What was SUBMITTED, which is not what was built: a view about the
        // cells draws no movers, and a frame the surface would not give us is
        // no picture at all.
        self.drawn_counts = match (presented, &moment) {
            (Some(()), Some(moment)) if moment.camera_uniforms.draws_movers() => [
                moment.bot_instances.len() as u32,
                moment.tote_instances.len() as u32,
                moment.cell_instances.len() as u32,
            ],
            (Some(()), Some(moment)) => [0, 0, moment.cell_instances.len() as u32],
            _ => [0; 3],
        };
        self.paint_lines(&uniforms);
        Ok(presented.is_some() && moment.is_some())
    }
}

/// A floor with nothing on it, which is how a floor sent with no run is drawn.
fn still() -> Playback<'static> {
    Playback { at_ms: 0, robots: &[], totes: &[], transits: &[], overlays: Overlays::default() }
}
