//! Within `crucible-viewer`, `screen` is WHERE THE FLOOR LANDS ON THE CANVAS
//! for one frame's camera, and the lines that belong to the screen rather than
//! to an entity: the named frame's axes in a corner and the centre cross
//! (spec 03 section 3.6, what the viewer needs; section 5, keys on the stage).
//!
//! It exists so that every line over the floor is placed by the matrices the
//! GPU drew the cells with, and cannot drift off the cell it outlines. A point
//! on the floor and a point on the screen are different types here, so a
//! millimetre is never stroked as a pixel; the wasm surface converts at its
//! edge.

use worldview_render::{CameraUniforms, RenderFrame};

/// A point on the floor, in the floor's own millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorMm(pub [f64; 2]);

/// A point on the canvas, in CSS pixels from its top-left corner, y down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPx(pub [f64; 2]);

/// A straight line between two points of one kind.
pub type Segment<P> = [P; 2];

/// One frame's camera as the GPU is handed it: where a point on the floor
/// lands on the screen.
pub struct Screen {
    /// Projection times view, column-major as the shaders read it.
    clip: [[f32; 4]; 4],
    /// The viewport, in CSS pixels.
    size: [f32; 2],
}

impl Screen {
    pub fn of(uniforms: &CameraUniforms) -> Self {
        let (p, v) = (uniforms.projection, uniforms.view);
        let clip = std::array::from_fn(|col| std::array::from_fn(|row| (0..4).map(|k| p[k][row] * v[col][k]).sum()));
        Self { clip, size: uniforms.viewport_size }
    }

    /// Where a point on the floor lands, or nil behind the eye.
    pub fn at(&self, frame: &RenderFrame, FloorMm([x_mm, y_mm]): FloorMm) -> Option<ScreenPx> {
        let ([x, y], m) = (frame.world_mm(x_mm, y_mm), &self.clip);
        let clip: [f32; 4] = std::array::from_fn(|row| m[0][row] * x + m[1][row] * y + m[3][row]);
        if clip[3] <= 1e-6 {
            return None;
        }
        let [ndc_x, ndc_y] = [clip[0] / clip[3], clip[1] / clip[3]];
        let [width, height] = self.size;
        Some(ScreenPx([f64::from((ndc_x + 1.0) * 0.5 * width), f64::from((1.0 - ndc_y) * 0.5 * height)]))
    }

    pub fn centre(&self) -> ScreenPx {
        ScreenPx([f64::from(self.size[0]) * 0.5, f64::from(self.size[1]) * 0.5])
    }

    fn height(&self) -> f64 {
        f64::from(self.size[1])
    }
}

/// A frame's name and its axes as directions on the drawing, which is what a
/// page knows of the frame a template's wall knobs name (spec 03 section 3.6).
#[derive(Clone, Debug, PartialEq)]
pub struct Axes {
    pub name: String,
    pub x: [f64; 2],
    pub y: [f64; 2],
}

/// Text drawn beside the lines.
pub struct Label {
    pub text: String,
    pub at: ScreenPx,
}

/// The axes as drawn: two arrows and their letters, and the frame's name.
pub struct AxesDrawn {
    pub segments: Vec<Segment<ScreenPx>>,
    pub labels: Vec<Label>,
}

const ARROW_PX: f64 = 32.0;

impl Axes {
    /// The axes drawn in the bottom-left corner: an arrow along each axis as
    /// it lies on the screen at `ground` (where the view's centre meets the
    /// floor, since a tilted camera foreshortens each place differently),
    /// its letter past the tip, and the frame's name under them. Nil when an
    /// axis is seen end-on and has no direction on the screen.
    pub fn drawn(&self, screen: &Screen, frame: &RenderFrame, ground: FloorMm) -> Option<AxesDrawn> {
        let origin = [48.0, screen.height() - 56.0];
        let ScreenPx(from) = screen.at(frame, ground)?;
        let mut segments = Vec::new();
        let mut labels = Vec::new();
        for (letter, [dx, dy]) in [("x", self.x), ("y", self.y)] {
            let FloorMm([gx, gy]) = ground;
            let ScreenPx(to) = screen.at(frame, FloorMm([gx + dx * 1_000.0, gy + dy * 1_000.0]))?;
            let (sx, sy) = (to[0] - from[0], to[1] - from[1]);
            let length = sx.hypot(sy);
            if length < 1e-3 {
                return None;
            }
            let (ux, uy) = (sx / length, sy / length);
            let tip = [origin[0] + ux * ARROW_PX, origin[1] + uy * ARROW_PX];
            segments.push([ScreenPx(origin), ScreenPx(tip)]);
            for turn in [0.45_f64, -0.45] {
                let (sin, cos) = turn.sin_cos();
                let back = [-ux * cos + uy * sin, -ux * sin - uy * cos];
                segments.push([ScreenPx(tip), ScreenPx([tip[0] + back[0] * 8.0, tip[1] + back[1] * 8.0])]);
            }
            labels.push(Label { text: letter.to_owned(), at: ScreenPx([tip[0] + ux * 10.0, tip[1] + uy * 10.0]) });
        }
        labels.push(Label { text: self.name.clone(), at: ScreenPx([8.0, screen.height() - 8.0]) });
        Some(AxesDrawn { segments, labels })
    }
}

/// The centre cross, drawn while the page's stage has focus: what Enter
/// selects is under it (spec 03 section 5, keys on the stage).
pub fn centre_cross(screen: &Screen) -> Vec<Segment<ScreenPx>> {
    let ScreenPx([x, y]) = screen.centre();
    vec![[ScreenPx([x - 8.0, y]), ScreenPx([x + 8.0, y])], [ScreenPx([x, y - 8.0]), ScreenPx([x, y + 8.0])]]
}

#[cfg(test)]
mod tests;
