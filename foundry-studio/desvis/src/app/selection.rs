//! Resource selection resolution and the in-scene selection overlays:
//! click → `SelectedResource`, the wgpu bot highlight set, the egui selection
//! outline, and the selected-bot path polyline. Split from `central_panel`
//! so grid rendering and selection concerns stay separate.

use std::collections::HashSet;

use eframe::egui;

use super::DesvisApp;
use crate::screen;
use crate::types::*;
use worldview_render::{BotHighlight, CameraState, place_pos};

/// Which bots to tint this frame. `primary` is the selected bot; `inbound`
/// are the HTM ids en-route to a selected station/relay (secondary tint).
/// Resolved once per frame from the selection + diag store.
#[derive(Default)]
pub(super) struct BotHighlightSet {
    primary: Option<(BotType, u32)>,
    inbound_htms: HashSet<u32>,
}

impl BotHighlightSet {
    pub(super) fn classify(&self, bot: &BotMarker) -> BotHighlight {
        if self.primary == Some((bot.bot_type, bot.bot_id)) {
            return BotHighlight::Primary;
        }
        // Inbound set is HTM-only (assignments are HTM chains).
        if bot.bot_type == BotType::Htm && self.inbound_htms.contains(&bot.bot_id) {
            return BotHighlight::Secondary;
        }
        BotHighlight::None
    }
}

impl DesvisApp {
    /// Resolve a single click on `cell_id` to a selection. A fresh cell
    /// selects the most specific candidate there (bot on the cell, else its
    /// station, else its relay); empty floor clears. Clicking the SAME cell
    /// again cycles to the next co-located candidate — the way to reach the
    /// relay/station cell under a bot without moving the cursor.
    pub(super) fn resolve_click(&mut self, cell_id: u32) {
        let cell_id = cell_id as usize;
        let candidates = self.selection_candidates(cell_id);
        let repeat = self.last_click_cell == Some(cell_id);
        self.last_click_cell = Some(cell_id);
        self.selected_resource = if candidates.is_empty() {
            None
        } else if repeat {
            // Advance from the current selection to the next co-located
            // candidate (wrapping); if the current selection isn't one of
            // them, fall back to the most specific.
            let here = self.selected_resource.and_then(|sel| candidates.iter().position(|c| *c == sel));
            match here {
                Some(index) => Some(candidates[(index + 1) % candidates.len()]),
                None => Some(candidates[0]),
            }
        } else {
            Some(candidates[0])
        };
    }

    /// Every resource selectable at `grid_index`, most-specific first: bot
    /// markers standing on the cell, then a station that owns it, then a
    /// relay that owns it. Order defines both the default pick (first) and
    /// the click-cycle order.
    fn selection_candidates(&self, cell_id_at: usize) -> Vec<SelectedResource> {
        let mut candidates = Vec::new();
        for bot in self.bot_positions.iter().filter(|b| b.cell_id as usize == cell_id_at) {
            candidates.push(SelectedResource::Bot { bot_type: bot.bot_type, bot_id: bot.bot_id });
        }
        if let Some(cell_id) = self.cell_infos.get(cell_id_at).and_then(|info| info.as_ref()).map(|info| info.id) {
            if let Some(&meta_index) = self.station_by_cell.get(&cell_id) {
                candidates.push(SelectedResource::Station(self.station_metas[meta_index].id));
            }
            if let Some(&meta_index) = self.relay_by_cell.get(&cell_id) {
                candidates
                    .push(SelectedResource::Relay { htm_access_cell: self.relay_metas[meta_index].htm_access_cell });
            }
        }
        candidates
    }

    /// Resolve the bots to tint this frame from the current selection. A
    /// selected bot is primary; a selected station's/relay's inbound HTMs are
    /// secondary. Builds the diag store if needed (the inbound set reads it).
    pub(super) fn bot_highlight_set(&mut self) -> BotHighlightSet {
        match self.selected_resource {
            Some(SelectedResource::Bot { bot_type, bot_id }) => {
                // Build the store here too (Station/Relay arms already do):
                // the central panel paints BEFORE the inspector rebuilds a
                // polling-invalidated store, and the path bound reads it — a
                // missing store made the path fall back to whole-plan for one
                // frame per poll (the reported flash).
                self.ensure_station_diag_store();
                BotHighlightSet { primary: Some((bot_type, bot_id)), inbound_htms: HashSet::new() }
            }
            Some(SelectedResource::Station(station_id)) => {
                let cursor = self.resource_cursor();
                let store = self.ensure_station_diag_store();
                let inbound_htms =
                    store.inbound_htms_at(station_id, cursor).into_iter().map(|htm| htm.robot_id).collect();
                BotHighlightSet { primary: None, inbound_htms }
            }
            Some(SelectedResource::Relay { htm_access_cell }) => {
                let cursor = self.resource_cursor();
                let store = self.ensure_station_diag_store();
                let inbound_htms =
                    store.inbound_htms_via_relay(htm_access_cell, cursor).into_iter().map(|htm| htm.robot_id).collect();
                BotHighlightSet { primary: None, inbound_htms }
            }
            None => BotHighlightSet::default(),
        }
    }

    /// Paint the selection outline: a selected station's or relay's cells, or
    /// a ring around the selected bot. Occlusion ignored (diagnostic overlay).
    pub(super) fn paint_selection_highlight(&self, cam: &CameraState, painter: &egui::Painter, rect: egui::Rect) {
        let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 196, 0));
        let outline_cells = |cell_ids: &[u32]| {
            for &id in cell_ids {
                let Some(info) = self.cell_infos.get(id as usize).and_then(|i| i.as_ref()) else { continue };
                let corners = info.corners_mm();
                painter.add(egui::Shape::closed_line(
                    corners
                        .iter()
                        .map(|&[cx, cy]| {
                            let [wx, wy] = self.frame.world_mm(cx, cy);
                            screen::pos(cam.world_to_screen(wx, wy, screen::viewport(rect)))
                        })
                        .collect(),
                    stroke,
                ));
            }
        };
        match self.selected_resource {
            Some(SelectedResource::Station(station_id)) => {
                if let Some(meta) = self.station_metas.iter().find(|meta| meta.id == station_id) {
                    outline_cells(&meta.cell_ids);
                }
            }
            Some(SelectedResource::Relay { htm_access_cell }) => {
                if let Some(meta) = self.relay_metas.iter().find(|meta| meta.htm_access_cell == htm_access_cell) {
                    outline_cells(&meta.cell_ids);
                }
            }
            Some(SelectedResource::Bot { bot_type, bot_id }) => {
                if let Some(bot) = self.bot_positions.iter().find(|b| b.bot_id == bot_id && b.bot_type == bot_type) {
                    let [wx, wy] = place_pos(&self.drawn, bot.place());
                    painter.circle_stroke(
                        screen::pos(cam.world_to_screen(wx, wy, screen::viewport(rect))),
                        11.0,
                        stroke,
                    );
                }
            }
            None => {}
        }
    }
}
