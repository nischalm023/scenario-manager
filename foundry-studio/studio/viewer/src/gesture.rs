//! Within `crucible-viewer`, `gesture` is WHAT THE POINTER HAS DONE SINCE THE
//! LAST FRAME: a browser's stream of pointer and wheel events, summed into the
//! one frame of input the camera reads.
//!
//! The two sides disagree about time, which is the whole reason this exists. A
//! tab delivers an event whenever the mouse moves — several between two
//! animation frames, or none — while [`CameraState::handle_input`] wants a
//! single "this is what happened" and applies it once. So each event adds its
//! movement here, and the frame takes the total and leaves the pointer where
//! it is; a drag that arrived as twenty events and one that arrived as two
//! move the camera by the same amount.
//!
//! wasm32 only: it exists to serve the browser surface.

use worldview_render::{CameraState, PointerInput, Viewport};

use crate::viewer::Projection;

/// The events since the last frame, and enough about the pointer to know what
/// the next one means.
#[derive(Default)]
pub(crate) struct Gesture {
    input: PointerInput,
    /// Whether the primary button is down, which is what makes a move a drag.
    dragging: bool,
    /// And whether the middle one is, which makes a move a drag that moves the
    /// camera by nothing. Both are the state of a button rather than something
    /// that happened, so both outlive the frame that clears the movements.
    middle_down: bool,
    at: [f32; 2],
}

impl Gesture {
    pub(crate) fn down(&mut self, x: f32, y: f32, middle: bool) {
        self.dragging = !middle;
        self.middle_down = middle;
        self.at = [x, y];
        self.hovering(x, y);
    }

    pub(crate) fn moved(&mut self, x: f32, y: f32, shift: bool) {
        let by = [x - self.at[0], y - self.at[1]];
        self.at = [x, y];
        self.hovering(x, y);
        self.input.shift = shift;
        // A move with no button down is a hover and nothing more; the camera
        // reads where the pointer is, and moves only for a drag. A middle-button
        // drag's movement is dropped, never added to the primary drag's: the
        // camera has no middle-button gesture.
        if !self.middle_down {
            if !self.dragging {
                return;
            }
            self.input.drag_delta[0] += by[0];
            self.input.drag_delta[1] += by[1];
        }
        self.input.dragged = true;
    }

    pub(crate) fn up(&mut self) {
        self.dragging = false;
        self.middle_down = false;
    }

    pub(crate) fn left(&mut self) {
        self.input.hovered = false;
        self.input.hover_pos = None;
    }

    pub(crate) fn wheel(&mut self, delta_x: f32, delta_y: f32, shift: bool) {
        self.input.scroll_delta[0] += delta_x;
        self.input.scroll_delta[1] += delta_y;
        self.input.shift = shift;
        self.input.hovered = true;
    }

    /// Move the camera by everything that has happened, and start summing
    /// again. Where the pointer IS survives — it is the state of the world,
    /// not an event — so the next frame still knows what is hovered. In the
    /// orbit `projection` a drag turns the camera and Shift with a drag pans,
    /// the other way round from the camera's own; orthographic, nothing turns.
    pub(crate) fn apply(&mut self, camera: &mut CameraState, viewport: Viewport, projection: Projection) {
        self.input.viewport = viewport;
        if projection.orbits() && self.input.dragged && self.input.scroll_delta == [0.0, 0.0] {
            self.input.shift = !self.input.shift;
        }
        camera.handle_input(&self.input, projection.may_turn());
        self.input =
            PointerInput { hovered: self.input.hovered, hover_pos: self.input.hover_pos, ..PointerInput::default() };
    }

    fn hovering(&mut self, x: f32, y: f32) {
        self.input.hover_pos = Some([x, y]);
        self.input.hovered = true;
    }
}
