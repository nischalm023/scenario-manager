//! Central panel rendering: GPU grid, bot labels, camera interaction, tooltips.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use eframe::egui;
use rustc_hash::FxHashSet;

use super::DesvisApp;
use super::selection::BotHighlightSet;
use worldview_render::{
    ArrowSet, CameraState, Measured, Overlays, Playback, PointerInput, Robot, ViewMode as Drawn, place_pos, place_z,
};

use super::render_callback::SceneCallback;
use crate::screen;
use crate::types::*;

/// What the camera reads, translated from what egui saw. The camera takes no
/// toolkit types at all, so this is where a drag, a wheel tick and a held key
/// stop being egui's and become a gesture.
fn pointer_input(response: &egui::Response, ui: &egui::Ui) -> PointerInput {
    let rect = response.rect;
    let scroll = ui.input(|i| i.smooth_scroll_delta);
    let drag = response.drag_delta();
    PointerInput {
        viewport: screen::viewport(rect),
        hovered: response.hovered(),
        hover_pos: response.hover_pos().map(|p| [p.x, p.y]),
        dragged: response.dragged(),
        drag_delta: [drag.x, drag.y],
        scroll_delta: [scroll.x, scroll.y],
        shift: ui.input(|i| i.modifiers.shift),
        escape_pressed: ui.input(|i| i.key_pressed(egui::Key::Escape)),
    }
}

impl DesvisApp {
    /// What the renderer is being asked to show, from the two selectors desvis
    /// offers: the view tab, and the overlay the Simulation tab is showing.
    pub(crate) fn drawn_view(&self) -> Drawn {
        match self.view_mode {
            ViewMode::State => Drawn::State,
            ViewMode::UnladenHtm => Drawn::UnladenHtm,
            ViewMode::LadenHtm => Drawn::LadenHtm,
            ViewMode::Vtm => Drawn::Vtm,
            ViewMode::Simulation => Drawn::Simulation(match self.heatmap_mode {
                HeatmapMode::Off => Measured::Nothing,
                HeatmapMode::HtmWait => Measured::Wait,
                HeatmapMode::HtmSpeed => Measured::Speed,
                _ => Measured::Visits,
            }),
        }
    }

    /// This instant's robots as the renderer takes them, with the selection's
    /// highlight on the ones the inspector is pointing at.
    pub(super) fn highlighted_robots(&self, highlights: &BotHighlightSet) -> Vec<Robot> {
        self.bot_positions.iter().map(|marker| marker.robot(highlights.classify(marker))).collect()
    }

    /// Per-CELL-ID reserved relay flags for State view rendering.
    ///
    /// Sources: live shared set during sim, or per-slice relay_state during
    /// playback. The flags are id-keyed like the instance buffer, so a relay
    /// cell reads reserved on any map, gridded or not.
    pub(crate) fn build_reserved_cells(&self) -> Vec<bool> {
        let len = self.cell_infos.len();
        if self.view_mode != ViewMode::State {
            return vec![false; len];
        }

        let mut flags = vec![false; len];
        // Live sim: read from shared dispatcher state.
        // TODO: wire relay_visual_state through Simulation
        if let Some(ref rvs) = None::<Arc<Mutex<HashSet<u32>>>> {
            if let Ok(set) = rvs.lock() {
                for &cell_id in set.iter() {
                    if let Some(f) = flags.get_mut(cell_id as usize) {
                        *f = true;
                    }
                }
            }
        } else if let Some(slice_idx) = self.current_heatmap_slice_idx {
            // Playback: reconstruct from per-slice relay state.
            if let Some(record) = self.time_series.get(slice_idx) {
                let relay = &record.relay_state;
                if !relay.is_empty() {
                    let mut reserved: FxHashSet<u32> = relay.start_reserved.iter().copied().collect();
                    let slice_start_ms = record.start_ms as f64;
                    let offset_ms = (self.time_cursor_ms - slice_start_ms).max(0.0) as u64;
                    for &(cell_id, t_offset, is_reserved) in &relay.transitions {
                        if t_offset <= offset_ms {
                            if is_reserved {
                                reserved.insert(cell_id);
                            } else {
                                reserved.remove(&cell_id);
                            }
                        }
                    }
                    for &cell_id in &reserved {
                        if let Some(f) = flags.get_mut(cell_id as usize) {
                            *f = true;
                        }
                    }
                }
            }
        }
        flags
    }

    /// Render the central panel: GPU grid, bot instances, camera, tooltips.
    pub(crate) fn draw_central_panel(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let (response, painter) = ui.allocate_painter(avail, egui::Sense::click_and_drag());
        let rect = response.rect;
        let viewport = screen::viewport(rect);

        // The instant the renderer will be handed, resolved before the camera
        // borrow (which is what keeps these off `&self` methods): the
        // containers and their travels, the robots with the selection's
        // highlight on them, and the reserved relay cells.
        let (totes, transits) =
            if self.view_mode == ViewMode::State { self.totes_at_cursor() } else { (Vec::new(), Vec::new()) };
        let highlight_set =
            if self.view_mode == ViewMode::State { self.bot_highlight_set() } else { BotHighlightSet::default() };
        let robots = self.highlighted_robots(&highlight_set);
        let reserved_cells = self.build_reserved_cells();
        let drawn_view = self.drawn_view();
        let at_ms = self.resource_cursor().as_ms();

        // Handle camera input and build uniforms.
        let mut hovered_id: Option<u32> = None;
        let mut hover_mm: Option<[f64; 2]> = None;
        let mut cursor_ray: Option<([f32; 3], [f32; 3])> = None;
        let mut clicked_cell: Option<u32> = None;
        let mut entity_click = false;
        let alt_held = ui.input(|i| i.modifiers.alt);
        if let Some(ref mut cam) = self.gpu_camera {
            // Advance camera animation.
            let dt = ui.input(|i| i.predicted_dt);
            if cam.tick(dt) {
                ui.ctx().request_repaint();
            }

            let allow_orbit = self.view_mode == ViewMode::State;
            cam.handle_input(&pointer_input(&response, ui), allow_orbit);

            if response.dragged() || response.hovered() {
                ui.ctx().request_repaint();
            }

            let uniforms = cam.build_uniforms(
                viewport, 1.0, // cell_size in meters
                drawn_view,
            );

            // Which arrows: the view decides.
            let arrows: Option<ArrowSet> = drawn_view.arrows();

            // The renderer turns this instant into every row it uploads —
            // the cells with their overlays, the robots, the containers,
            // including one part-way along a conveyor.
            let moment = self.drawn.moment(
                uniforms,
                arrows,
                &Playback {
                    at_ms,
                    robots: &robots,
                    totes: &totes,
                    transits: &transits,
                    overlays: Overlays {
                        heatmap: self.heatmap_data.as_ref(),
                        mode: self.heatmap_mode,
                        traffic: &self.traffic,
                        reserved_cells: &reserved_cells,
                    },
                },
            );

            // Emit wgpu paint callback.
            let callback = eframe::egui_wgpu::Callback::new_paint_callback(rect, SceneCallback { moment });
            painter.add(callback);

            // Bot ID text labels (egui overlay, only when zoomed in enough).
            if self.view_mode == ViewMode::State && cam.distance < 50.0 {
                for bot in &self.bot_positions {
                    // `place_pos` returns render METRES; projecting an index
                    // instead (it converts again) smeared every label away
                    // from its circle in proportion to distance from the
                    // origin. Project the metres directly, at the height the
                    // cylinder is actually drawn.
                    let [wx, wy] = place_pos(&self.drawn, bot.place());
                    let wz = place_z(bot.bot_type, bot.place(), &self.drawn);
                    let screen_pos = screen::pos(cam.world_to_screen_z(wx, wy, wz, viewport));
                    if rect.contains(screen_pos) {
                        let label = match bot.bot_type {
                            BotType::Vtm => format!("A{}", bot.bot_id),
                            BotType::Htm => format!("D{}", bot.bot_id),
                            BotType::Rtp => format!("R{}", bot.bot_id),
                        };
                        painter.text(
                            screen_pos,
                            egui::Align2::CENTER_CENTER,
                            label,
                            egui::FontId::proportional(10.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
            }

            // Camera distance indicator.
            let default_dist = CameraState::default_distance(self.frame.extent().0, self.frame.extent().1);
            if (cam.distance - default_dist).abs() > 1.0 || cam.elevation < 1.5 {
                painter.text(
                    egui::Pos2::new(rect.max.x - 8.0, rect.min.y + 8.0),
                    egui::Align2::RIGHT_TOP,
                    format!("d={:.0}", cam.distance),
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_gray(120),
                );
            }
            let dt = ui.input(|i| i.unstable_dt);
            if dt > 0.5 {
                self.frame_times.clear();
            } else if dt > 0.0 {
                self.frame_times.push_back(dt);
            }
            let mut window: f32 = self.frame_times.iter().sum();
            while window > 1.0 && self.frame_times.len() > 1 {
                window -= self.frame_times.pop_front().unwrap_or(0.0);
            }
            let slowest = self.frame_times.iter().copied().fold(0.0, f32::max);
            let (avg_fps, low_fps) =
                if slowest > 0.0 { (self.frame_times.len() as f32 / window, 1.0 / slowest) } else { (0.0, 0.0) };
            painter.text(
                egui::Pos2::new(rect.max.x - 8.0, rect.min.y + 24.0),
                egui::Align2::RIGHT_TOP,
                format!("{avg_fps:.0} fps · {low_fps:.0} low"),
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(120),
            );

            // Hover detection: inverse-project to millimetres, then ask the
            // spatial index which cell's tile covers the point. No lattice.
            if let Some(hover_pos) = response.hover_pos() {
                cursor_ray = Some(cam.cursor_ray(hover_pos.into(), viewport));
                hover_mm = cam.screen_to_mm(hover_pos.into(), viewport);
                hovered_id = hover_mm.and_then(|mm| self.cell_index.query(mm[0], mm[1], self.drawn.cells()));
            }

            // Double-click: fly to the hovered cell's centre. Shift = keep
            // current elevation.
            if response.double_clicked()
                && let Some(id) = hovered_id
                && let Some(info) = self.cell_infos.get(id as usize).and_then(|i| i.as_ref())
            {
                let shift = ui.input(|i| i.modifiers.shift);
                let c = info.center_mm();
                let [wx, wy] = self.frame.world_mm(c[0], c[1]);
                cam.fly_to_world(wx, wy, shift);
            }

            // Single click (State view): select the resource under
            // the cursor. Resolved after this camera borrow ends; composes with
            // the double-click fly-to (the first click of a double-click selects
            // the same cell the camera then flies to).
            //
            // ⌥ deconflicts the two click consumers: a plain click belongs to
            // the inspector's bot/station/relay selection exactly as before
            // the entity explorer existed; ⌥-click belongs to the explorer,
            // in every view — the inspector never claims it.
            if response.clicked() {
                if alt_held {
                    entity_click = true;
                } else if self.view_mode == ViewMode::State {
                    clicked_cell = hovered_id;
                }
            }
        }

        // Selection highlight + committed-path overlays (State view),
        // painted as egui overlays over the wgpu canvas. Uses an
        // immutable camera re-borrow, after the mutable camera-input borrow
        // above has ended.
        if self.view_mode == ViewMode::State {
            if let Some(cam) = self.gpu_camera.as_ref() {
                self.paint_selection_highlight(cam, &painter, rect);
                self.paint_resource_paths(cam, &painter, rect);
            }
        }

        // The entity explorer's hover outline, in every view: which entity a
        // piece of the floor is stays true whether the map shows state or a
        // heatmap. Nothing is drawn when no row is under the pointer.
        if let Some(cam) = self.gpu_camera.as_ref() {
            self.paint_entity_highlight(cam, &painter, rect);
        }

        if let Some(cell_id) = clicked_cell {
            self.resolve_click(cell_id);
        }
        // ⌥-click: everything the pick ray passes through goes to the entity
        // explorer, nearest first — the list narrows to exactly those.
        if entity_click {
            let hits = self.entity_ray_hits(cursor_ray, hover_mm, hovered_id);
            // TEMP diagnostic for the ⌥-click-does-nothing report.
            eprintln!("[map-click] entity hits: {}", hits.len());
            self.entity_panel.open_ray(hits);
        }

        // The ONE hover card: everything the pick ray passes through, nearest
        // first, with the hovered cell's sim-side facts folded in. This
        // replaced the old cell tooltip — two stacked cards under one pointer
        // said less than one.
        let entity_hits = self.entity_ray_hits(cursor_ray, hover_mm, hovered_id);
        let hovered_info = hovered_id.and_then(|id| self.cell_infos.get(id as usize).and_then(|i| i.as_ref()));
        if !entity_hits.is_empty() || hovered_info.is_some() {
            let tooltip_id = ui.id().with("entity_tooltip");
            let layer_id = egui::LayerId::new(egui::Order::Tooltip, tooltip_id);
            egui::show_tooltip_at_pointer(ui.ctx(), layer_id, tooltip_id, |ui| {
                self.entity_panel.map_hover_card(ui, &entity_hits, hovered_info);
                if self.view_mode == ViewMode::Simulation
                    && matches!(self.heatmap_mode, HeatmapMode::HtmWait | HeatmapMode::HtmSpeed)
                {
                    if let Some(info) = hovered_info {
                        self.traffic.hover(ui, info.id, self.heatmap_mode);
                    }
                }
            });
        }

        // Error overlay when simulation has failed.
        if let SimStatus::Error(ref msg) = self.sim_status {
            let banner_rect = egui::Rect::from_min_size(
                egui::Pos2::new(rect.min.x + 10.0, rect.min.y + 10.0),
                egui::Vec2::new(rect.width() - 20.0, 0.0), // height auto
            );
            egui::Area::new(egui::Id::new("sim_error_overlay")).fixed_pos(banner_rect.min).show(ui.ctx(), |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(180, 0, 0, 220))
                    .inner_margin(egui::Margin::same(12.0))
                    .rounding(6.0)
                    .show(ui, |ui: &mut egui::Ui| {
                        ui.label(
                            egui::RichText::new("SIMULATION HALTED").color(egui::Color32::WHITE).strong().size(16.0),
                        );
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(msg).color(egui::Color32::from_gray(240)).monospace().size(12.0));
                    });
            });
        }
    }
}
