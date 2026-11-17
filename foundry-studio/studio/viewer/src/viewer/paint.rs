//! Within `viewer`, `paint` is THE 2D CANVAS THE LINES ARE STROKED ON: laid
//! over the GPU's canvas by the page, sized with it, cleared and stroked once
//! per frame with what [`outline`](crate::outline) and
//! [`screen`](crate::screen) worked out.
//!
//! It exists because a dash, a pixel width and a name in a corner are what a
//! 2D canvas draws natively and a GPU pipeline would have to be taught, and
//! because what is stroked here can never change what the GPU drew: the
//! floor's own drawing, and its drawn cell count, are the same with lines or
//! without. Every line has a halo of the floor colour a pixel wide on each
//! side, so it keeps its contrast over whatever it crosses (spec 03 section 5).

use js_sys::Array;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use worldview_render::CameraUniforms;

use super::Viewer;
use crate::outline::{Ink, Stroke};
use crate::palette::Palette;
use crate::screen::{AxesDrawn, Label, Screen, ScreenPx, Segment, centre_cross};

/// The letters and names beside the axes.
const FONT: &str = "600 12px system-ui, sans-serif";

impl Viewer {
    /// Stroke the outlines, the axes and the centre cross over what the GPU
    /// just drew, with the camera it drew with.
    pub(super) fn paint_lines(&mut self, uniforms: &CameraUniforms) {
        let (Some(lines), Some(scene)) = (&self.lines, self.run.scene()) else { return };
        let screen = Screen::of(uniforms);
        let strokes = self.outlines.strokes(scene.frame(), &screen);
        let ground = self.floor_at(self.centre_px());
        let axes = self.axes.as_ref().zip(ground).and_then(|(axes, ground)| axes.drawn(&screen, scene.frame(), ground));
        let cross = self.centre_cross.then(|| centre_cross(&screen));
        self.outlined_counts = lines.paint(&strokes, axes, cross, &self.palette, self.ratio);
    }
}

pub(crate) struct Lines {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
}

impl Lines {
    pub(crate) fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("the lines canvas gives no 2D context; is it already drawn on by a GPU?"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        Ok(Self { canvas, context })
    }

    /// The same device pixels as the GPU's canvas, so the two stay aligned.
    pub(crate) fn resize(&self, width: u32, height: u32) {
        if (self.canvas.width(), self.canvas.height()) != (width, height) {
            self.canvas.set_width(width);
            self.canvas.set_height(height);
        }
    }

    /// Clear, then stroke each set and the axes and the cross in CSS pixels
    /// (`ratio` device pixels each). Answers the outline segments stroked:
    /// the agent's, the selection's, and the update's.
    pub(crate) fn paint(
        &self,
        strokes: &[Stroke],
        axes: Option<AxesDrawn>,
        cross: Option<Vec<Segment<ScreenPx>>>,
        palette: &Palette,
        ratio: f64,
    ) -> [u32; 3] {
        let context = &self.context;
        let _ = context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        context.clear_rect(0.0, 0.0, f64::from(self.canvas.width()), f64::from(self.canvas.height()));
        let _ = context.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0);
        let mut counted = [0u32; 3];
        for stroke in strokes {
            self.stroke(stroke.ink, &stroke.segments, palette);
            let slot = match stroke.ink {
                Ink::Agent => 0,
                Ink::Selected => 1,
                Ink::Added | Ink::Changed | Ink::Lines => 2,
            };
            counted[slot] += stroke.segments.len() as u32;
        }
        if let Some(AxesDrawn { segments, labels }) = axes {
            self.stroke(Ink::Lines, &segments, palette);
            context.set_font(FONT);
            context.set_text_baseline("middle");
            for Label { text, at: ScreenPx(at) } in labels {
                context.set_line_width(3.0);
                context.set_stroke_style_str(&palette.floor.css);
                let _ = context.stroke_text(&text, at[0] - 3.0, at[1]);
                context.set_fill_style_str(&palette.lines.css);
                let _ = context.fill_text(&text, at[0] - 3.0, at[1]);
            }
        }
        if let Some(segments) = cross {
            self.stroke(Ink::Lines, &segments, palette);
        }
        counted
    }

    /// The halo first, wider by a pixel each side and in the floor colour,
    /// then the line in its own colour, both with the ink's dash.
    fn stroke(&self, ink: Ink, segments: &[Segment<ScreenPx>], palette: &Palette) {
        if segments.is_empty() {
            return;
        }
        let context = &self.context;
        let colour = match ink {
            Ink::Agent => &palette.agent,
            Ink::Selected => &palette.selection,
            Ink::Added | Ink::Changed | Ink::Lines => &palette.lines,
        };
        let dash: Array = ink.dash_px().iter().map(|&length| JsValue::from_f64(length)).collect();
        let _ = context.set_line_dash(&dash);
        context.set_line_cap(if ink.dash_px().is_empty() { "square" } else { "butt" });
        for (width, css) in [(ink.width_px() + 2.0, &palette.floor.css), (ink.width_px(), &colour.css)] {
            context.set_line_width(width);
            context.set_stroke_style_str(css);
            context.begin_path();
            for [ScreenPx(from), ScreenPx(to)] in segments {
                context.move_to(from[0], from[1]);
                context.line_to(to[0], to[1]);
            }
            context.stroke();
        }
    }
}
