//! Committed-path overlays for the selected resource: the selected bot's
//! one-cycle path, and per-inbound-HTM colored paths when a station is
//! selected. Bound resolution (station entrance / leg boundary / VTM cycle /
//! near horizon), zoom-scaled stroke width, marker-anchored polylines. Split
//! from `selection.rs` (click resolution + highlights) to keep both under the
//! file limit and one-concern.

use dessim::events::PlanLegKind;
use dessim::model::SimTime;
use eframe::egui;

use super::DesvisApp;
use worldview_render::{CameraState, place_pos};

use crate::screen;
use crate::types::*;

/// Hard safety cap on path-overlay points per bot — guards against a
/// pathological run regardless of the resolved bound.
const PATH_OVERLAY_MAX_POINTS: usize = 2048;

/// Cell budget for the `PathBound::NearHorizon` fallback — the path shown
/// when neither a station-entrance nor a leg-boundary bound resolves. Short
/// by design: an unresolved bound must read as "a little way ahead", never
/// the whole committed plan.
const PATH_FALLBACK_MAX_CELLS: usize = 32;

/// How many remaining plan legs the VTM cycle bound inspects to find the
/// current motion's fork-op boundary (a cycle is ≤ ~4 legs; 8 is generous).
const VTM_BOUND_LEG_LOOKAHEAD: usize = 8;

/// Path stroke width scales with on-screen cell size so it reads as a
/// proportional ribbon at any zoom: `clamp(cell_px * FACTOR, MIN, MAX)`.
/// Tuned by eye — FACTOR keeps the stroke ~1/8 of a cell; MIN/MAX stop it
/// from vanishing when zoomed far out or ballooning when zoomed way in.
const PATH_WIDTH_FACTOR: f32 = 0.12;
const PATH_WIDTH_MIN_PX: f32 = 1.5;
const PATH_WIDTH_MAX_PX: f32 = 6.0;

/// Distinct colors for per-inbound-HTM station paths, keyed stably by
/// `robot_id % N` so a bot keeps its color as others come and go.
const INBOUND_PATH_PALETTE: [egui::Color32; 6] = [
    egui::Color32::from_rgb(0x3B, 0x82, 0xF6), // blue
    egui::Color32::from_rgb(0x22, 0xC5, 0x5E), // green
    egui::Color32::from_rgb(0xF5, 0x9E, 0x0B), // amber
    egui::Color32::from_rgb(0xA8, 0x55, 0xF7), // purple
    egui::Color32::from_rgb(0x06, 0xB6, 0xD4), // cyan
    egui::Color32::from_rgb(0xEC, 0x48, 0x99), // pink
];

/// Stable path/chip color for an inbound HTM (station inspector), keyed by
/// `robot_id` so the map path and the inbound-list chip always match.
pub(crate) fn inbound_path_color(robot_id: u32) -> egui::Color32 {
    INBOUND_PATH_PALETTE[(robot_id as usize) % INBOUND_PATH_PALETTE.len()]
}

impl DesvisApp {
    /// Draw committed-path overlays for the current selection: the selected
    /// bot's one-cycle path in red, or — when a station is selected — one
    /// distinctly-colored path per inbound HTM (color keyed by `robot_id` so it
    /// matches the inbound-list chip). Occlusion ignored (diagnostic overlay).
    pub(super) fn paint_resource_paths(&self, cam: &CameraState, painter: &egui::Painter, rect: egui::Rect) {
        let width = self.path_stroke_width(cam, rect);
        match self.selected_resource {
            Some(SelectedResource::Bot { bot_type, bot_id }) => {
                self.paint_one_path(
                    cam,
                    painter,
                    rect,
                    bot_type,
                    bot_id,
                    egui::Color32::from_rgba_unmultiplied(235, 50, 35, 230),
                    width,
                );
            }
            Some(SelectedResource::Station(station_id)) => {
                let cursor = self.resource_cursor();
                let Some(store) = self.station_diag_store.as_ref() else { return };
                for htm in store.inbound_htms_at(station_id, cursor) {
                    self.paint_one_path(
                        cam,
                        painter,
                        rect,
                        BotType::Htm,
                        htm.robot_id,
                        inbound_path_color(htm.robot_id),
                        width,
                    );
                }
            }
            _ => {}
        }
    }

    /// Stroke width for path overlays, scaled to the current on-screen cell
    /// size and clamped. Cell px is measured by projecting one nominal cell
    /// pitch at the map's centre (a stable sample under perspective).
    fn path_stroke_width(&self, cam: &CameraState, rect: egui::Rect) -> f32 {
        let (ex, ey) = self.frame.extent();
        let a = screen::pos(cam.world_to_screen(ex * 0.5, ey * 0.5, screen::viewport(rect)));
        let b = screen::pos(cam.world_to_screen(
            ex * 0.5 + self.cell_size_mm as f32 / 1000.0,
            ey * 0.5,
            screen::viewport(rect),
        ));
        let cell_px = (b - a).length();
        (cell_px * PATH_WIDTH_FACTOR).clamp(PATH_WIDTH_MIN_PX, PATH_WIDTH_MAX_PX)
    }

    /// Paint one bot's committed path from the cursor in `color` at `width`.
    /// The polyline is anchored at the bot's current interpolated marker
    /// position (the same frac position the marker renders at), so the first
    /// segment tracks the bot mid-cell instead of swinging from the center of
    /// the cell it is leaving.
    fn paint_one_path(
        &self,
        cam: &CameraState,
        painter: &egui::Painter,
        rect: egui::Rect,
        bot_type: BotType,
        bot_id: u32,
        color: egui::Color32,
        width: f32,
    ) {
        let bound = self.path_bound_for(bot_type, bot_id);
        let points = self.gather_bot_path_points(bot_id, bound);
        // Anchor: the bot's marker position (mm-interpolated when moving).
        let anchor = self.bot_positions.iter().find(|b| b.bot_id == bot_id && b.bot_type == bot_type).map(|bot| {
            let [wx, wy] = place_pos(&self.drawn, bot.place());
            screen::pos(cam.world_to_screen(wx, wy, screen::viewport(rect)))
        });
        // `points_px`, not `screen`: `screen` is the module that crosses to
        // the renderer, and a local of that name hid it here.
        let mut points_px: Vec<egui::Pos2> = Vec::with_capacity(points.len() + 1);
        if let Some(anchor) = anchor {
            points_px.push(anchor);
        }
        points_px.extend(points.into_iter().map(|mm| {
            let [wx, wy] = self.frame.world_mm(mm[0], mm[1]);
            screen::pos(cam.world_to_screen(wx, wy, screen::viewport(rect)))
        }));
        if points_px.len() < 2 {
            return;
        }
        painter.add(egui::Shape::line(points_px, egui::Stroke::new(width, color)));
    }

    /// Where to stop a bot's path overlay — bounds it to ONE cycle instead of
    /// the whole committed plan.
    /// - HTM: the active assignment's target station presentation cell (the
    ///   entrance), else the next committed leg boundary.
    /// - VTM: through the end of the current fetch-or-deposit motion — the
    ///   first remaining `VtmForkOp` leg's end; if the first remaining leg IS
    ///   a fork op (currently completing one), through the end of the
    ///   following `VtmForkOp` (the next travel+fork pair).
    /// - Anything unresolved: the short near horizon (never whole-plan).
    fn path_bound_for(&self, bot_type: BotType, bot_id: u32) -> PathBound {
        let cursor = self.resource_cursor();
        let Some(store) = self.station_diag_store.as_ref() else { return PathBound::NearHorizon };
        if bot_type == BotType::Vtm {
            return Self::vtm_cycle_bound(store.plan_legs_from(bot_id, cursor, VTM_BOUND_LEG_LOOKAHEAD));
        }
        if let Some(assignment) = store.active_assignment_for_bot(bot_id, cursor) {
            if let Some(meta) = self.station_metas.iter().find(|m| m.id == assignment.station_id) {
                return PathBound::ToStationCell(meta.delivery_cell);
            }
        }
        // No station target: stop at the next committed leg boundary. Legs
        // stream only at planning completion, so a live run falls through to
        // the near horizon — bounded either way.
        match store.plan_legs_from(bot_id, cursor, 1).first() {
            Some((_, _, enter)) => PathBound::ToTime(*enter),
            None => PathBound::NearHorizon,
        }
    }

    /// The VTM "current cycle" bound over its remaining legs: the end of the
    /// first `VtmForkOp`; if that fork op is the very first remaining leg
    /// (the VTM is completing it now), the end of the following `VtmForkOp`
    /// so the drawn path covers the next full travel+fork motion. No fork op
    /// in the lookahead → near horizon.
    fn vtm_cycle_bound(legs: Vec<(PlanLegKind, u32, SimTime)>) -> PathBound {
        use dessim::events::PlanLegKind;
        let fork_ends: Vec<(usize, SimTime)> = legs
            .iter()
            .enumerate()
            .filter(|(_, (kind, _, _))| *kind == PlanLegKind::VtmForkOp)
            .map(|(index, (_, _, enter))| (index, *enter))
            .collect();
        match fork_ends.as_slice() {
            [] => PathBound::NearHorizon,
            // First remaining leg is the fork op ending now — bound at the
            // NEXT fork op if one is visible, else at this one.
            [(0, first), rest @ ..] => match rest.first() {
                Some((_, next)) => PathBound::ToTime(*next),
                None => PathBound::ToTime(*first),
            },
            [(_, first), ..] => PathBound::ToTime(*first),
        }
    }

    /// Gather a bot's committed path from the cursor as `(col, row)` cell-center
    /// points, truncated by `bound` to one delivery cycle: the cursor slice's
    /// steps ending after the cursor, then subsequent slices' steps until the
    /// bound (target station cell reached, or leg-boundary time passed), and in
    /// all cases the `PATH_OVERLAY_MAX_POINTS` safety cap. The first point is
    /// the cell the bot is moving TOWARD (steps ending at/before the cursor
    /// are skipped; a step's `cell_id` is its destination) — the caller
    /// prepends the bot's live marker position as the polyline anchor, so no
    /// behind-the-bot cell is emitted here. Caller projects.
    fn gather_bot_path_points(&self, bot_id: u32, bound: PathBound) -> Vec<[f64; 2]> {
        if self.time_series.is_empty() {
            return Vec::new();
        }
        let cursor_ms = self.resource_cursor().as_ms();
        let start_slice = (cursor_ms / self.slice_width_ms) as usize;
        let mut points: Vec<[f64; 2]> = Vec::new();
        let push_cell = |points: &mut Vec<[f64; 2]>, cell_id: u32| {
            if let Some(info) = self.cell_infos.get(cell_id as usize).and_then(|i| i.as_ref()) {
                let c = info.center_mm();
                // Skip a repeated cell (a Wait/Rotate sharing the prior cell).
                if points.last() != Some(&c) {
                    points.push(c);
                }
            }
        };
        for slice_index in start_slice..self.time_series.len() {
            let record = &self.time_series[slice_index];
            let Some(location) = record.bot_locations.get(&bot_id) else { continue };
            let mut step_end_ms = record.start_ms;
            for step in &location.steps {
                step_end_ms += step.delta_ms;
                // Skip steps whose span ends at/before the cursor in the
                // cursor slice — the in-progress step's destination is the
                // first vertex ("the cell the bot is moving toward").
                if slice_index == start_slice && step_end_ms <= cursor_ms {
                    continue;
                }
                // Time bound: stop once we've passed the leg-boundary time
                // (the step ending after it is the last one drawn).
                if let PathBound::ToTime(t) = bound {
                    if step_end_ms > t.as_ms() {
                        push_cell(&mut points, step.cell_id);
                        return points;
                    }
                }
                push_cell(&mut points, step.cell_id);
                // Cell bound: stop right after entering the target station cell.
                if let PathBound::ToStationCell(cell) = bound {
                    if step.cell_id == cell {
                        return points;
                    }
                }
                // Unresolved bound: short near horizon, never the whole plan.
                if matches!(bound, PathBound::NearHorizon) && points.len() >= PATH_FALLBACK_MAX_CELLS {
                    return points;
                }
                if points.len() >= PATH_OVERLAY_MAX_POINTS {
                    return points;
                }
            }
        }
        points
    }
}

/// How far a path overlay extends from the cursor — one delivery cycle, not
/// the whole committed plan.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PathBound {
    /// Stop after the step entering this station presentation cell (the
    /// entrance to the bot's target station).
    ToStationCell(u32),
    /// Stop at the next committed leg boundary (no station target).
    ToTime(SimTime),
    /// No station/leg bound resolved — draw only a short near horizon
    /// (`PATH_FALLBACK_MAX_CELLS`). NEVER the whole committed plan: a
    /// transient unresolved bound (assignment just closed, store mid-rebuild,
    /// no legs streamed yet in a live run) must not balloon the path.
    NearHorizon,
}

#[cfg(test)]
mod tests {
    use dessim::events::PlanLegKind;
    use dessim::model::SimTime;

    use super::{DesvisApp, PathBound};

    fn leg(kind: PlanLegKind, enter_ms: u64) -> (PlanLegKind, u32, SimTime) {
        (kind, 0, SimTime::ms(enter_ms))
    }

    /// VTM cycle bound: end of the first fork op; if the first remaining leg
    /// IS the fork op, the end of the following one; no fork op → NearHorizon.
    #[test]
    fn vtm_cycle_bound_resolution() {
        // Travel then fork: bound at the fork's end.
        let bound = DesvisApp::vtm_cycle_bound(vec![
            leg(PlanLegKind::VtmTravel, 1_000),
            leg(PlanLegKind::VtmForkOp, 2_000),
            leg(PlanLegKind::VtmTravel, 3_000),
            leg(PlanLegKind::VtmForkOp, 4_000),
        ]);
        assert_eq!(bound, PathBound::ToTime(SimTime::ms(2_000)));

        // Currently completing a fork op (first remaining leg): bound at the
        // NEXT fork op — the following travel+fork pair.
        let bound = DesvisApp::vtm_cycle_bound(vec![
            leg(PlanLegKind::VtmForkOp, 1_000),
            leg(PlanLegKind::VtmTravel, 2_000),
            leg(PlanLegKind::VtmForkOp, 3_000),
        ]);
        assert_eq!(bound, PathBound::ToTime(SimTime::ms(3_000)));

        // Fork op is first AND last visible: bound at it.
        let bound = DesvisApp::vtm_cycle_bound(vec![leg(PlanLegKind::VtmForkOp, 1_000)]);
        assert_eq!(bound, PathBound::ToTime(SimTime::ms(1_000)));

        // No fork op in the lookahead: near horizon, never whole plan.
        let bound = DesvisApp::vtm_cycle_bound(vec![leg(PlanLegKind::VtmTravel, 1_000)]);
        assert_eq!(bound, PathBound::NearHorizon);
        assert_eq!(DesvisApp::vtm_cycle_bound(vec![]), PathBound::NearHorizon);
    }
}
