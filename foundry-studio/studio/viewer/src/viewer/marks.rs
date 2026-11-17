//! Within `viewer`, `marks` is WHAT A PAGE ASKS TO HAVE DRAWN OVER THE FLOOR:
//! the agent's highlight groups, the selection and an update's changes, each
//! named by entity keys, and the colours, the frame's axes and the centre
//! cross the lines are drawn with (spec 03 section 3.6, what the viewer needs;
//! the mount contract's `highlights`, `selection` and `diff`).
//!
//! It exists to turn a page's JavaScript values into the plain Rust
//! [`Outlines`](crate::outline::Outlines) keeps, and to answer what each
//! request found standing on the floor, so a page can say "4 marks, 4 on the
//! floor" and list the rest apart. The types a page writes are declared below,
//! once, and reach TypeScript in the generated declarations.

use js_sys::{Array, Object, Reflect};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use super::Viewer;
use super::paint::Lines;
use crate::palette::{Colour, Palette};
use crate::screen::Axes;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** One of the agent's highlight groups, as the mount's `highlights` prop holds it: the keys it marks. */
export interface HighlightGroup { readonly marks: readonly string[] }
/** An update's changes (spec 05 section 4.5): added and changed are outlined; removed is the page's to list. */
export interface DiffSets { readonly added: readonly string[]; readonly changed: readonly string[]; readonly removed?: readonly string[] }
/** The colours the viewer draws its ground and its lines in, each a CSS colour as a computed style writes it. */
export interface FloorPalette { readonly floor: string; readonly lines: string; readonly agent: string; readonly selection: string }
/** A frame's name and its axes as directions on the drawing: x right and y up the drawing is `{ x: [1, 0], y: [0, 1] }`. */
export interface FrameAxes { readonly name: string; readonly x: readonly [number, number]; readonly y: readonly [number, number] }
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "readonly HighlightGroup[]")]
    pub type HighlightGroups;
    #[wasm_bindgen(typescript_type = "DiffSets")]
    pub type DiffSets;
    #[wasm_bindgen(typescript_type = "FloorPalette")]
    pub type FloorPalette;
    #[wasm_bindgen(typescript_type = "FrameAxes")]
    pub type FrameAxes;
}

#[wasm_bindgen]
impl Viewer {
    /// Outline each group's entities, dashed at 2 px, replacing the groups
    /// before; answers, per group, how many of its keys stand on cells.
    pub fn set_highlights(&mut self, groups: &HighlightGroups) -> Result<Vec<u32>, JsValue> {
        let groups = listed(groups, "highlights")?
            .iter()
            .map(|group| strings(&field(&group, "marks")?, "a group's marks"))
            .collect::<Result<Vec<_>, _>>()?;
        let cells = self.run.scene().map_or(&[][..], |scene| scene.cells());
        Ok(self.outlines.highlight(groups, &self.roll, cells))
    }

    /// Outline what one selection marks, solid at 3 px: the selected
    /// entity's own key, or the members a sector marks, as one union; none
    /// when empty. Answers how many of the keys stand on cells.
    pub fn set_selection(&mut self, keys: Vec<String>) -> u32 {
        let cells = self.run.scene().map_or(&[][..], |scene| scene.cells());
        self.outlines.select(keys, &self.roll, cells)
    }

    /// Outline what an update adds (solid) and changes (dotted), or nothing;
    /// answers how many added and how many changed keys stand on cells.
    /// Removed entities are on no floor the update leaves, so none is drawn.
    pub fn set_diff(&mut self, sets: Option<DiffSets>) -> Result<Vec<u32>, JsValue> {
        let (added, changed) = match &sets {
            Some(sets) => (strings(&field(sets, "added")?, "added")?, strings(&field(sets, "changed")?, "changed")?),
            None => (Vec::new(), Vec::new()),
        };
        let cells = self.run.scene().map_or(&[][..], |scene| scene.cells());
        Ok(self.outlines.show_diff(added, changed, &self.roll, cells).to_vec())
    }

    /// The colours of the ground and the lines: the page's tokens, or under
    /// forced colours the system colours it resolved. Refused whole when one
    /// does not read, so a half-applied palette is never drawn.
    pub fn set_palette(&mut self, palette: &FloorPalette) -> Result<(), JsValue> {
        let colour = |role: &str| -> Result<Colour, JsValue> {
            let css =
                field(palette, role)?.as_string().ok_or_else(|| said(&format!("the palette's {role} is not text")))?;
            Colour::parse(&css).map_err(|why| said(&format!("the palette's {role}: {why}")))
        };
        self.palette = Palette {
            floor: colour("floor")?,
            lines: colour("lines")?,
            agent: colour("agent")?,
            selection: colour("selection")?,
        };
        Ok(())
    }

    /// The palette as it was last given, or the default before any was.
    pub fn palette(&self) -> FloorPalette {
        let palette = Object::new();
        let roles = [
            ("floor", &self.palette.floor),
            ("lines", &self.palette.lines),
            ("agent", &self.palette.agent),
            ("selection", &self.palette.selection),
        ];
        for (role, colour) in roles {
            let _ = Reflect::set(&palette, &role.into(), &colour.css.as_str().into());
        }
        palette.unchecked_into()
    }

    /// Name the frame whose axes are drawn in the corner, or none.
    pub fn set_axes(&mut self, axes: Option<FrameAxes>) -> Result<(), JsValue> {
        self.axes = match &axes {
            Some(axes) => {
                let name = field(axes, "name")?.as_string().ok_or_else(|| said("a frame's name is text"))?;
                Some(Axes { name, x: direction(&field(axes, "x")?)?, y: direction(&field(axes, "y")?)? })
            }
            None => None,
        };
        Ok(())
    }

    /// Draw the centre cross, which the page shows while its stage has focus.
    pub fn show_centre_cross(&mut self, shown: bool) {
        self.centre_cross = shown;
    }

    /// The 2D canvas laid over this viewer's own that the lines are stroked
    /// on, sized with it from now on.
    pub fn attach_lines(&mut self, canvas: HtmlCanvasElement) -> Result<(), JsValue> {
        let lines = Lines::new(canvas)?;
        let (width, height) = self.gpu.size();
        lines.resize(width, height);
        self.lines = Some(lines);
        Ok(())
    }

    /// Outline segments the last frame stroked: the agent's groups, the
    /// selection and the update's changes. With no lines canvas, zeroes.
    pub fn outlined_counts(&self) -> Vec<u32> {
        self.outlined_counts.to_vec()
    }
}

fn said(why: &str) -> JsValue {
    JsValue::from_str(why)
}

fn field(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &name.into())
}

fn listed(value: &JsValue, what: &str) -> Result<Array, JsValue> {
    value.dyn_ref::<Array>().cloned().ok_or_else(|| said(&format!("{what} are an array")))
}

fn strings(value: &JsValue, what: &str) -> Result<Vec<String>, JsValue> {
    listed(value, what)?
        .iter()
        .map(|key| key.as_string().ok_or_else(|| said(&format!("{what} are keys, and one is not text"))))
        .collect()
}

fn direction(value: &JsValue) -> Result<[f64; 2], JsValue> {
    let pair = listed(value, "an axis's directions")?;
    match (pair.length(), pair.get(0).as_f64(), pair.get(1).as_f64()) {
        (2, Some(x), Some(y)) if x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0) => Ok([x, y]),
        _ => Err(said("an axis is two finite numbers, not both zero")),
    }
}
