//! Within `crucible-viewer`, `outline` is THE EDGE OF WHAT A PAGE MARKED: the
//! agent's highlight groups, the selection and an update's changes, each a set
//! of entity keys, outlined around the cells they stand on and turned into
//! screen strokes for one frame's camera (spec 03 section 3.6, what the viewer
//! needs; section 5, how each is told apart).
//!
//! It exists so that a mark is drawn where the cells are and never instead of
//! them: a stroke lies over the floor, whose drawing is untouched, so the drawn
//! cell count is the same with outlines or without (the render rulings:
//! nothing is hidden). Each marked cell is outlined on its own four sides, the
//! proof of concept's cut of spec 03's "outline the cells": a side two marked
//! cells share is drawn once, and cells that lie on one footprint outline it
//! once rather than cancel. The inks differ by dash and width, never by colour
//! alone.

use desgraph::CellId;
use worldview_render::{Cell, RenderFrame};

use crate::roll::Roll;
use crate::screen::{FloorMm, Screen, ScreenPx, Segment};

/// What a stroke draws, which decides its width and dash in CSS pixels (spec
/// 03 section 5): the agent's groups dashed at 2 px, the selection solid at
/// 3 px, what an update adds solid and what it changes dotted (spec 05 section
/// 4.5), and `Lines`, the frame's axes and the centre cross, solid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Added,
    Changed,
    Agent,
    Selected,
    Lines,
}

impl Ink {
    pub fn width_px(self) -> f64 {
        if self == Self::Selected { 3.0 } else { 2.0 }
    }

    /// Empty is a solid line.
    pub fn dash_px(self) -> &'static [f64] {
        match self {
            Self::Agent => &[6.0, 4.0],
            Self::Changed => &[2.0, 3.0],
            Self::Added | Self::Selected | Self::Lines => &[],
        }
    }
}

/// One marked set: the keys it was given, how many of them stand on cells,
/// and the sides of those cells on the floor. The keys are kept so a new
/// floor marks the same entities where they now stand.
pub struct Outline {
    pub ink: Ink,
    keys: Vec<String>,
    pub standing: u32,
    sides: Vec<Segment<FloorMm>>,
}

impl Outline {
    fn of(ink: Ink, keys: Vec<String>, roll: &Roll, cells: &[Option<Cell>]) -> Self {
        let (marked, standing) = roll.cells_of_all(keys.iter().map(String::as_str));
        Self { ink, keys, standing, sides: cell_sides(&marked, cells) }
    }
}

/// Everything marked, in the order it is drawn: an update's changes, then the
/// agent's groups, then the selection over both.
#[derive(Default)]
pub struct Outlines {
    diff: Vec<Outline>,
    groups: Vec<Outline>,
    selection: Option<Outline>,
}

impl Outlines {
    /// Replace the agent's groups; answers, per group, how many keys stand on cells.
    pub fn highlight(&mut self, groups: Vec<Vec<String>>, roll: &Roll, cells: &[Option<Cell>]) -> Vec<u32> {
        self.groups = groups.into_iter().map(|keys| Outline::of(Ink::Agent, keys, roll, cells)).collect();
        self.groups.iter().map(|group| group.standing).collect()
    }

    /// Replace the selection: the keys one selected thing marks, outlined as one union (a sector by its members,
    /// spec 03 section 3.5), none when empty; answers how many of them stand on cells.
    pub fn select(&mut self, keys: Vec<String>, roll: &Roll, cells: &[Option<Cell>]) -> u32 {
        self.selection = (!keys.is_empty()).then(|| Outline::of(Ink::Selected, keys, roll, cells));
        self.selection.as_ref().map_or(0, |selected| selected.standing)
    }

    /// Replace an update's changes; answers how many added and how many
    /// changed keys stand on cells.
    pub fn show_diff(
        &mut self,
        added: Vec<String>,
        changed: Vec<String>,
        roll: &Roll,
        cells: &[Option<Cell>],
    ) -> [u32; 2] {
        self.diff = vec![Outline::of(Ink::Added, added, roll, cells), Outline::of(Ink::Changed, changed, roll, cells)];
        [self.diff[0].standing, self.diff[1].standing]
    }

    /// A new floor: every set marks the same keys where they stand on it.
    pub fn refloor(&mut self, roll: &Roll, cells: &[Option<Cell>]) {
        for outline in self.diff.iter_mut().chain(self.groups.iter_mut()).chain(self.selection.as_mut()) {
            *outline = Outline::of(outline.ink, std::mem::take(&mut outline.keys), roll, cells);
        }
    }

    /// Every set as screen segments for one frame, in drawing order. A side
    /// with an end behind the eye is left out rather than drawn wrapped.
    pub fn strokes(&self, frame: &RenderFrame, screen: &Screen) -> Vec<Stroke> {
        let sets = self.diff.iter().chain(self.groups.iter()).chain(self.selection.iter());
        sets.map(|outline| Stroke {
            ink: outline.ink,
            segments: outline
                .sides
                .iter()
                .filter_map(|&[a, b]| Some([screen.at(frame, a)?, screen.at(frame, b)?]))
                .collect(),
        })
        .collect()
    }
}

/// Segments on the screen, all drawn with one ink.
pub struct Stroke {
    pub ink: Ink,
    pub segments: Vec<Segment<ScreenPx>>,
}

/// Every side of every marked cell, each once, sorted: a dash pattern follows
/// the order and the direction the sides are drawn in, and both are fixed
/// here. Two sides are one where their corners agree to a tenth of a
/// millimetre, so neighbours whose tiles were computed from different centres
/// still share theirs.
pub fn cell_sides(marked: &[CellId], cells: &[Option<Cell>]) -> Vec<Segment<FloorMm>> {
    let at = |[x, y]: [f64; 2]| ((x * 10.0).round() as i64, (y * 10.0).round() as i64);
    let mut sides = Vec::new();
    for cell in marked.iter().filter_map(|&CellId(id)| cells.get(id as usize)?.as_ref()) {
        let corners = cell.corners_mm();
        for side in 0..4 {
            let (a, b) = (corners[side], corners[(side + 1) % 4]);
            let (a, b) = if at(a) <= at(b) { (a, b) } else { (b, a) };
            if at(a) != at(b) {
                sides.push(((at(a), at(b)), [FloorMm(a), FloorMm(b)]));
            }
        }
    }
    sides.sort_unstable_by_key(|(key, _)| *key);
    sides.dedup_by_key(|(key, _)| *key);
    sides.into_iter().map(|(_, side)| side).collect()
}

#[cfg(test)]
mod tests;
