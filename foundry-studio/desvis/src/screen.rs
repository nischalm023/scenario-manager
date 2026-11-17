//! Within desvis, `screen` is the EGUI BOUNDARY of the shared renderer: egui's
//! rectangles and points on one side, the plain numbers `worldview-render`
//! takes and gives back on the other.
//!
//! Two lines of arithmetic, in one place, so that every panel that projects a
//! robot, a cell or a path onto the canvas crosses that boundary the same way.

use eframe::egui::{Pos2, Rect};
use worldview_render::Viewport;

/// The renderer's view of a panel's canvas.
pub(crate) fn viewport(rect: Rect) -> Viewport {
    Viewport::new([rect.min.x, rect.min.y], [rect.width(), rect.height()])
}

/// Where a point the renderer projected lands, in egui's own terms.
pub(crate) fn pos(point: [f32; 2]) -> Pos2 {
    Pos2::new(point[0], point[1])
}
