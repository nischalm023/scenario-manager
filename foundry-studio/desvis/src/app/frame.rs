//! Within `app`, `frame` is the PER-FRAME DRIVE of `DesvisApp`: the
//! `eframe::App` update loop that lays out the panels, routes each view to
//! its rendering module (graph, gantt, metrics panel), and initializes the
//! GPU renderer.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;

use desgraph::StationId;
use dessim::model::SimDuration;

use super::DesvisApp;
use crate::gantt;
use crate::graph;
use crate::metrics_panel;
use crate::sim_config_display;
use crate::types::*;
use worldview_render::{CameraState, SceneRenderer};

/// Count total shapes in the background paint layer (where all panels draw).
#[cfg(debug_assertions)]
fn count_total_shapes(ctx: &egui::Context) -> usize {
    ctx.graphics(|g| g.get(egui::LayerId::background()).map_or(0, |list| list.all_entries().len()))
}

impl eframe::App for DesvisApp {
    /// Clear color for the wgpu surface. eframe's default is `(12, 12, 12, 180)`
    /// — a dark semi-transparent gray that bleeds through any pixel-alignment
    /// gap between panels and reads as a black seam. We want it to match
    /// `theme::BG_0` painted by egui as the central-panel fill.
    ///
    /// Use `to_normalized_gamma_f32` (raw sRGB normalized values, not a
    /// linear conversion). egui_wgpu's vertex shader does the sRGB → linear
    /// conversion internally before writing to the sRGB surface, so passing
    /// the gamma-encoded values here makes wgpu's clear take the same path
    /// (treated as linear by wgpu, gamma-encoded on store) and produce the
    /// same final display byte as the egui-painted fill.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        crate::theme::BG_0.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Refresh the OS window title every frame from the *effective*
        // SimConfig — base_config + UI overlays (or the saved
        // sim_config in playback). Updating each frame is what catches
        // UI-mutable knobs (htm_only / threads / fidelity) without
        // wiring up explicit invalidation hooks.
        let title = sim_config_display::compact_title(
            &self.effective_config(),
            self.vtm_count,
            &self.map_label,
            self.title_label.as_deref(),
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        // Auto-start simulation on first frame (--run or --test). One-shot:
        // consume the flag so the auto-start fires exactly once at launch.
        // Without this, clicking Reset (which returns to SimStatus::Idle)
        // would re-trigger here on the very next frame, bouncing straight
        // back into a running sim and never showing the idle/config screen.
        if self.auto_run && matches!(self.sim_status, SimStatus::Idle) {
            self.auto_run = false;
            eprintln!("[desvis] auto-starting simulation");
            self.start_simulation();
        }

        // Read what the run has sent since the last frame.
        if matches!(self.sim_status, SimStatus::Running) {
            self.poll_live_run();
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Advance cursor during realtime playback (scaled by playback_speed).
        if self.playback_active {
            if let Some(wall_start) = self.playback_wall_start {
                let elapsed_ms = wall_start.elapsed().as_secs_f64() * 1000.0 * self.playback_speed as f64;
                if self.playback_reverse {
                    // Reverse: walk the cursor back toward t=0, then stop there.
                    let target_ms = self.playback_sim_start_ms - elapsed_ms;
                    if target_ms <= 0.0 {
                        self.time_cursor_ms = 0.0;
                        self.playback_active = false;
                    } else {
                        self.time_cursor_ms = target_ms;
                    }
                    self.update_grid_for_cursor();
                } else {
                    let target_ms = self.playback_sim_start_ms + elapsed_ms;
                    let max_ms =
                        self.time_series.last().map(|s| s.start_ms as f64 + self.slice_width_ms as f64).unwrap_or(0.0);
                    if target_ms >= max_ms {
                        // Caught up to the live edge. If the sim is done,
                        // stop. Otherwise, leave the cursor where it last
                        // landed and wait for the next slice to extend
                        // max_ms — the per-frame re-anchor in the sidebar
                        // means target_ms next frame is `cursor + dt*speed`,
                        // so we'll advance step-by-step as data arrives.
                        if !matches!(self.sim_status, SimStatus::Running) {
                            self.playback_active = false;
                        } else {
                            // The cursor waits at the live edge; re-read its
                            // slice each frame so the robots step into the
                            // next slice the frame it arrives.
                            self.update_grid_for_cursor();
                        }
                    } else {
                        self.time_cursor_ms = target_ms.max(0.0);
                        self.update_grid_for_cursor();
                    }
                }
            }
            ctx.request_repaint();
        }

        // Test mode: exit after completion + 2s delay.
        if self.test_mode {
            match &self.sim_status {
                SimStatus::Complete(_) => {
                    if self.test_complete_at.is_none() {
                        eprintln!("[test] sim complete, will exit in 2s");
                        self.test_complete_at = Some(Instant::now());
                    }
                    if let Some(t) = self.test_complete_at {
                        if t.elapsed() >= Duration::from_secs(2) {
                            eprintln!("[test] exiting with code 0");
                            std::process::exit(0);
                        }
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                }
                SimStatus::Error(e) => {
                    eprintln!("[test] sim error: {}", e);
                    std::process::exit(1);
                }
                _ => {}
            }
        }

        // Debug: count shapes after each major rendering phase.
        #[cfg(debug_assertions)]
        let debug_log_this_frame = {
            self.debug_frame_count += 1;
            let interval = if matches!(self.sim_status, SimStatus::Running) { 10 } else { 60 };
            self.debug_frame_count % interval == 0
        };

        // Left sidebar rendered as a floating card on the BG_0 canvas.
        // The SidePanel uses a transparent frame; the card is painted as
        // an inner `Frame::show` so its rounded right edge isn't clipped
        // at the SidePanel/CentralPanel boundary.
        //
        // Panel width has to account for BOTH outer and inner margins so
        // the card's content area ends up exactly SIDEBAR_WIDTH wide.
        // If the panel is narrower, `set_width(SIDEBAR_WIDTH)` makes the
        // content overflow the card's content rect, which makes egui
        // expand the painted frame_rect past panel_rect — and the painter
        // then clips off the right side, taking the rounded corners with it.
        let prev_view_mode = self.view_mode;
        let card = crate::theme::sidebar_card_frame();
        let card_chrome_w =
            card.outer_margin.left + card.outer_margin.right + card.inner_margin.left + card.inner_margin.right;
        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_WIDTH + card_chrome_w)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                card.show(ui, |ui| {
                    ui.set_width(SIDEBAR_WIDTH);
                    crate::theme::apply_scrollbar_style(ui);
                    // auto_shrink(false) so the ScrollArea claims the panel's
                    // full available height — without this the card only
                    // grows as tall as the content, leaving canvas exposed
                    // below the sections.
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        self.draw_sidebar(ui);
                    });
                });
            });

        // Floating "More charts" popup, rendered at the top level so it
        // can overlay the central panel rather than living inside the
        // sidebar's narrow column.
        self.draw_more_charts_window(ctx);

        // The entity explorer docks on the right. Declared here, ahead of the
        // bottom dock and the central panel, for two reasons: a panel only
        // reserves its strip of the window if it is declared before the
        // panels it should crowd, and the row the pointer lands on decides
        // what the map outlines in THIS frame rather than the next one.
        self.draw_entity_panel(ctx);

        // Auto-reset camera to top-down when leaving State view.
        if prev_view_mode == ViewMode::State && self.view_mode != ViewMode::State {
            if let Some(ref mut cam) = self.gpu_camera {
                cam.snap_to_default();
            }
        }

        #[cfg(debug_assertions)]
        let shapes_after_sidebar = if debug_log_this_frame { count_total_shapes(ctx) } else { 0 };

        // Bottom panel: unified metrics or the per-bot utilization view.
        let skip_charts = std::env::var("DESVIS_SKIP_CHARTS").map_or(false, |v| v == "1");
        let has_time_series = !self.time_series.is_empty();
        let has_bottom_panel_data = has_time_series || matches!(self.data_source, DataSource::Profile(_));
        if has_bottom_panel_data && !skip_charts {
            // First-frame default: half the OS-window height (clamped to
            // a sane band so a tiny / huge OS window still gets a
            // reasonable initial split). After that, the panel's height
            // is whatever the user dragged it to — `frame.rs` below
            // writes `panel_resp.response.rect.height()` back into
            // `bottom_panel_height`, and `default_height` is only
            // honoured by egui on the panel's first appearance anyway.
            if self.bottom_panel_height <= 0.0 {
                let screen_h = ctx.screen_rect().height();
                self.bottom_panel_height = (screen_h / 2.0).clamp(260.0, 900.0);
            }
            // Explicit min/max so egui doesn't quietly clamp to its own
            // defaults when content height differs across tabs. Same
            // floating-card treatment as the sidebar — Frame::show inside
            // the panel rather than on `.frame()` so rounded edges
            // survive the panel/CentralPanel boundary.
            let panel_resp = egui::TopBottomPanel::bottom("chart")
                .resizable(true)
                .default_height(self.bottom_panel_height)
                .height_range(80.0..=1200.0)
                .show_separator_line(false)
                .frame(egui::Frame::none())
                .show(ctx, |ui| {
                    // Bottom panel uses the shared card_frame but with
                    // larger inner_margin so the chart/gantt content
                    // doesn't crowd the card border.
                    let mut bottom_card = crate::theme::card_frame();
                    bottom_card.inner_margin = egui::Margin::symmetric(12.0, 10.0);
                    bottom_card.show(ui, |ui| {
                        // Tab row.
                        ui.horizontal(|ui| {
                            let metrics_selected = self.bottom_panel_view == BottomPanelView::Metrics;
                            let utilization_selected = self.bottom_panel_view == BottomPanelView::Utilization;
                            if ui.selectable_label(metrics_selected, "Metrics").clicked() {
                                self.bottom_panel_view = BottomPanelView::Metrics;
                            }
                            if ui.selectable_label(utilization_selected, "Utilization").clicked() {
                                self.bottom_panel_view = BottomPanelView::Utilization;
                            }
                        });
                        ui.separator();

                        match self.bottom_panel_view {
                            BottomPanelView::Metrics => {
                                ui.horizontal_wrapped(|ui| {
                                    for metrics_view in MetricsView::ALL {
                                        let selected = self.metrics_view == metrics_view;
                                        if ui.selectable_label(selected, metrics_view.label()).clicked() {
                                            self.metrics_view = metrics_view;
                                        }
                                    }
                                });
                                ui.separator();

                                if self.metrics_view == MetricsView::TimeSeries {
                                    if self.time_series.is_empty() {
                                        ui.label("No time-series slices are available for this run.");
                                    } else if let Some(new_cursor) = graph::draw_chart(
                                        ui,
                                        &self.time_series,
                                        &self.selected_metrics,
                                        self.time_cursor_ms,
                                        self.slice_width_ms,
                                        self.num_stations,
                                        self.total_rack_slots,
                                        self.total_relay_cells,
                                        &mut self.chart_cache,
                                        &mut self.rolling_config,
                                    ) {
                                        let max_ms = self
                                            .time_series
                                            .last()
                                            .map(|s| s.start_ms as f64 + self.slice_width_ms as f64)
                                            .unwrap_or(0.0);
                                        self.engage_cursor(new_cursor, max_ms);
                                    }
                                } else if let DataSource::Profile(profile) = &self.data_source {
                                    // D30 fleet size is a SimConfig parameter
                                    // (unlike the map-derived `vtm_count`), so
                                    // it comes from the run's own config, as
                                    // the embargo does.
                                    metrics_panel::draw_metrics_panel(
                                        ui,
                                        self.metrics_view,
                                        &profile.summary,
                                        self.vtm_count,
                                        profile.sim_config.num_htms,
                                        (profile.sim_config.pick.embargo_secs * 1_000.0).round() as u64,
                                        &mut self.metrics_panel_state,
                                        &mut self.metrics_panel_cache,
                                    );
                                } else {
                                    metrics_panel::draw_pre_run_metric_list(ui, self.metrics_view);
                                }
                            }
                            BottomPanelView::Utilization => {
                                // A finished run's summary; a run still going
                                // shows what its slices state so far.
                                if let DataSource::Profile(profile) = &self.data_source {
                                    let s = &profile.summary;
                                    let sim_elapsed = SimDuration::ms((s.sim_time_secs * 1000.0) as u64);
                                    let station_busy_pct: HashMap<StationId, f64> =
                                        s.station_utilization_pct.iter().copied().collect();
                                    let new_cursor = gantt::draw_gantt_panel(
                                        ui,
                                        &s.vtm_robot_timelines,
                                        &s.htm_robot_timelines,
                                        &s.vtm_duty_totals,
                                        &s.htm_duty_totals,
                                        &s.vtm_idle_reason_counts,
                                        s.relay_utilization.as_ref(),
                                        self.relay_lanes.as_ref(),
                                        s.relay_slot_occupancy_timeline.as_ref(),
                                        &s.station_timelines,
                                        &s.station_types,
                                        &station_busy_pct,
                                        Some(&s.station_gap_stats),
                                        sim_elapsed,
                                        self.time_cursor_ms,
                                        &mut self.gantt_window_ms,
                                        &mut self.gantt_scroll_x,
                                    );
                                    if let Some(new_cursor) = new_cursor {
                                        self.engage_cursor(new_cursor, sim_elapsed.as_ms() as f64);
                                    }
                                } else if let Some(live) = self.live_run.as_ref() {
                                    let seen = &live.utilization;
                                    let sim_elapsed = SimDuration::ms(seen.seen_until.as_ms());
                                    let new_cursor = gantt::draw_gantt_panel(
                                        ui,
                                        &seen.vtm_timelines,
                                        &seen.htm_timelines,
                                        &seen.vtm_duty,
                                        &seen.htm_duty,
                                        &seen.vtm_idle_reason_counts,
                                        // The relay headline and the diving-board
                                        // lane are the end block's, as on playback.
                                        None,
                                        self.relay_lanes.as_ref(),
                                        None,
                                        &seen.station_timelines,
                                        &seen.station_types,
                                        &seen.station_busy_share(),
                                        None,
                                        sim_elapsed,
                                        self.time_cursor_ms,
                                        &mut self.gantt_window_ms,
                                        &mut self.gantt_scroll_x,
                                    );
                                    if let Some(new_cursor) = new_cursor {
                                        self.engage_cursor(new_cursor, sim_elapsed.as_ms() as f64);
                                    }
                                } else {
                                    ui.label("Run a sim or load a profile to see timelines.");
                                }
                            }
                        }
                    });
                });
            // Capture whatever height egui ended up with (default, persisted,
            // or just-dragged) so the next time the panel shows — including
            // after a hide cycle when time_series briefly empties — it
            // restores the user's dragged size.
            self.bottom_panel_height = panel_resp.response.rect.height();
        }

        #[cfg(debug_assertions)]
        let shapes_after_charts = if debug_log_this_frame { count_total_shapes(ctx) } else { 0 };

        // Central panel: grid (wgpu renderer).
        let skip_grid = std::env::var("DESVIS_SKIP_GRID").map_or(false, |v| v == "1");

        // Initialize GPU resources on first frame.
        if !self.gpu_initialized {
            if let Some(render_state) = frame.wgpu_render_state() {
                let device = &render_state.device;
                let target_format = render_state.target_format;
                let resources = SceneRenderer::new(device, target_format, &self.drawn);
                render_state.renderer.write().callback_resources.insert(resources);
                self.gpu_camera = Some(CameraState::new(self.frame.clone()));
                self.gpu_initialized = true;
            }
        }

        // The traffic overlay is measured up to the cursor; the central panel
        // then hands the whole instant, overlays included, to the renderer.
        self.refresh_traffic();

        // Force the central panel to paint BG_0 explicitly so the canvas
        // around cards (eframe clear_color) and the wgpu area background
        // (egui-painted panel fill, visible where no cells render) share
        // exactly the same source color.
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(crate::theme::BG_0).inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                if skip_grid {
                    ui.label("Grid rendering disabled (DESVIS_SKIP_GRID=1)");
                    return;
                }
                self.draw_central_panel(ui);
            });

        // Resource inspector (station / bot), drawn over the central panel.
        self.draw_inspector_window(ctx);

        // Debug: log shape counts per component.
        #[cfg(debug_assertions)]
        if debug_log_this_frame {
            let shapes_after_grid = count_total_shapes(ctx);
            let grid_cache_shapes = 0_usize;
            let heatmap_nonzero = self.heatmap_data.as_ref().map_or(0, |hm| {
                hm.vtm_counts.iter().filter(|&&c| c > 0).count() + hm.htm_counts.iter().filter(|&&c| c > 0).count()
            });
            eprintln!(
                "[desvis frame {}] shapes: sidebar={}, charts=+{}, grid=+{} (total={}), \
                 grid_cache={}, heatmap_nz={}, slices={}, metrics={}, view={:?}, hm={:?}{}",
                self.debug_frame_count,
                shapes_after_sidebar,
                shapes_after_charts.saturating_sub(shapes_after_sidebar),
                shapes_after_grid.saturating_sub(shapes_after_charts),
                shapes_after_grid,
                grid_cache_shapes,
                heatmap_nonzero,
                self.time_series.len(),
                self.selected_metrics.len(),
                self.view_mode,
                self.heatmap_mode,
                match (skip_charts, skip_grid) {
                    (true, true) => " [CHARTS+GRID SKIPPED]",
                    (true, false) => " [CHARTS SKIPPED]",
                    (false, true) => " [GRID SKIPPED]",
                    (false, false) => "",
                },
            );
        }
    }
}
