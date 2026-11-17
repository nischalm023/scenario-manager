//! Left sidebar: controls, legend, status, profile metadata.

mod metrics;
mod sim_buttons;

use eframe::egui;
use eframe::egui::Color32;

use dessim::{FidelityMode, PickSequencing, ScenarioConfig, ScenarioSourceKind};

use crate::app::DesvisApp;
use crate::theme;
use crate::types::*;

impl DesvisApp {
    pub fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.set_width(SIDEBAR_WIDTH);
        ui.add_space(4.0);

        // ---- Display: view + heatmap + camera ----
        let in_sim_mode = self.view_mode == ViewMode::Simulation;
        theme::section_frame().show(ui, |ui| {
            ui.label(theme::heading("Display"));
            ui.add_space(4.0);

            labeled_dropdown(ui, "View", |ui| {
                theme::combobox(ui, "view_mode", &self.view_mode.to_string(), |ui| {
                    for &mode in &[
                        ViewMode::State,
                        ViewMode::UnladenHtm,
                        ViewMode::LadenHtm,
                        ViewMode::Vtm,
                        ViewMode::Simulation,
                    ] {
                        ui.selectable_value(&mut self.view_mode, mode, mode.to_string());
                    }
                });
            });

            ui.add_space(3.0);
            labeled_dropdown(ui, "Heatmap", |ui| {
                ui.add_enabled_ui(in_sim_mode, |ui| {
                    theme::combobox(ui, "heatmap_mode", &self.heatmap_mode.to_string(), |ui| {
                        for &mode in &[
                            HeatmapMode::Off,
                            HeatmapMode::HtmWait,
                            HeatmapMode::HtmSpeed,
                            HeatmapMode::VtmOnly,
                            HeatmapMode::HtmOnly,
                            HeatmapMode::Combined,
                        ] {
                            ui.selectable_value(&mut self.heatmap_mode, mode, mode.to_string());
                        }
                    });
                });
            });

            if matches!(self.heatmap_mode, HeatmapMode::HtmWait | HeatmapMode::HtmSpeed) {
                ui.weak(format!("Cumulative: 0–{:.1} min", self.traffic.until_ms as f64 / 60_000.0));
                if self.heatmap_mode == HeatmapMode::HtmWait {
                    ui.weak(format!("White → red: 0–{:.1} HTM-s", self.traffic.max_wait_ms / 1000.0));
                    ui.weak("Queues and presentation/access cells excluded.");
                } else {
                    ui.weak("Orange → blue: 0–5 m/s (≥5 capped)");
                    ui.weak("Moving time only. Hover for the distribution.");
                    if self.traffic.legacy_motion {
                        ui.weak("Older recording: speed data unavailable for some motion.");
                    }
                }
                ui.weak("Grey: excluded or no observation; hover for details.");
            }

            if let Some(ref mut cam) = self.gpu_camera {
                ui.add_space(6.0);
                if theme::button(ui, "Reset camera").clicked() {
                    cam.reset();
                }
            }

            ui.add_space(6.0);
            let entities_label = if self.entity_panel.open { "Entities ✓" } else { "Entities" };
            if theme::button(ui, entities_label)
                .on_hover_text("Every entity of the loaded world document; hover a row to outline it on the map")
                .clicked()
            {
                self.entity_panel.open = !self.entity_panel.open;
            }
        });

        ui.add_space(3.0);

        // ---- Legends ----
        theme::section_frame().show(ui, |ui| {
            ui.label(theme::heading("Cells"));
            ui.add_space(4.0);
            inline_legend(
                ui,
                &[
                    ("Highway", theme::CELL_HIGHWAY),
                    ("Aisle", theme::CELL_AISLE),
                    ("Storage", theme::CELL_STORAGE),
                    ("Relay", theme::CELL_RELAY),
                    ("Station", theme::CELL_STATION),
                ],
            );

            if self.view_mode == ViewMode::State {
                ui.add_space(6.0);
                ui.label(theme::heading("Bots"));
                ui.add_space(4.0);

                let vtm_count = self.bot_positions.iter().filter(|b| b.bot_type == BotType::Vtm).count();
                let htm_count = self.bot_positions.iter().filter(|b| b.bot_type == BotType::Htm).count();
                let rtp_count = self.bot_positions.iter().filter(|b| b.bot_type == BotType::Rtp).count();
                if rtp_count > 0 {
                    inline_legend(ui, &[("RTP", theme::BOT_RTP)]);
                    ui.label(theme::caption(&format!("{} RTPs", rtp_count)));
                } else {
                    inline_legend(ui, &[("DA50", theme::BOT_VTM), ("D30", theme::BOT_HTM)]);
                    ui.label(theme::caption(&format!("{} DA50 · {} D30", vtm_count, htm_count)));
                }
            }
        });

        ui.add_space(3.0);

        // ---- Charts: sidebar list (curated) + "+ Add chart" popup ----
        theme::section_frame().show(ui, |ui| {
            theme::collapsing_section(ui, "sidebar_charts", "Charts", true, |ui| {
                // The sidebar list is whatever the user has chosen to keep
                // visible. Featured metrics seed it at startup. Checkbox
                // toggles plot visibility (does NOT remove from list);
                // on-hover `−` removes from the list entirely.
                //
                // Alphabetize by label so the rendered order matches what
                // the user reads, not the source order of the enum.
                // Case-insensitive so any future lowercase labels don't
                // sort after the uppercase ones.
                let mut visible: Vec<MetricKind> = self.sidebar_charts.clone();
                visible.sort_by(|a, b| a.label().to_lowercase().cmp(&b.label().to_lowercase()));
                for kind in visible {
                    let unavailable =
                        kind.unavailable_label(self.num_stations, self.total_rack_slots, self.total_relay_cells);
                    draw_sidebar_chart_row(ui, kind, &mut self.selected_metrics, &mut self.sidebar_charts, unavailable);
                }
                if theme::button(ui, "+ Add chart").clicked() {
                    self.more_charts_open = true;
                }
            });
        });

        ui.add_space(3.0);

        // ---- Scenario + Sim config + Run ----
        theme::section_frame().show(ui, |ui| {
            ui.label(theme::heading("Scenario"));
            ui.add_space(2.0);
            self.draw_scenario_text(ui);

            if matches!(self.sim_status, SimStatus::Idle) && !matches!(self.data_source, DataSource::Profile(_)) {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(theme::heading("Config"));
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Threads").small());
                    ui.add(egui::DragValue::new(&mut self.num_threads).range(0..=256).speed(0.1));
                    if self.num_threads == 0 {
                        ui.label(theme::caption("(all cores)"));
                    }
                });

                ui.add_space(2.0);
                ui.label(egui::RichText::new("Time slice").small());
                ui.horizontal_wrapped(|ui| {
                    for &(label, ms) in
                        &[("10s", 10_000u64), ("1m", 60_000), ("5m", 300_000), ("10m", 600_000), ("1h", 3_600_000)]
                    {
                        if ui.selectable_label(self.time_slice_ms == ms, label).clicked() {
                            self.time_slice_ms = ms;
                        }
                    }
                });

                ui.add_space(2.0);
                labeled_dropdown(ui, "Fidelity", |ui| {
                    theme::combobox(ui, "fidelity_mode", self.fidelity.label(), |ui| {
                        for &mode in FidelityMode::ALL {
                            ui.selectable_value(&mut self.fidelity, mode, mode.label());
                        }
                    });
                });

                ui.add_space(2.0);
                ui.checkbox(&mut self.htm_only, "D30-only mode")
                    .on_hover_text("Skip DA50 transport; totes are instantly available at relays");

                // Gate on the loaded scenario, not the last run's -- this must
                // render before the first run and after a Reset.
                if matches!(self.base_scenario, Some(ScenarioConfig::BulkSource(_))) {
                    ui.add_space(2.0);
                    ui.checkbox(&mut self.ignore_hour_gate, "Ignore production hour gate").on_hover_text(
                        "Induct putaway with no hourly floor — ignore each lot's PRD_HR. Off (default) = gated. Applies on next run.",
                    );
                }

                ui.add_space(2.0);
                labeled_dropdown(ui, "Pick sequencing", |ui| {
                    theme::combobox(ui, "pick_sequencing", self.pick_sequencing.label(), |ui| {
                        for &mode in &PickSequencing::ALL {
                            ui.selectable_value(&mut self.pick_sequencing, mode, mode.label());
                        }
                    });
                });
            }

            ui.add_space(8.0);
            self.draw_sim_buttons(ui);
        });

        ui.add_space(3.0);

        // ---- Metrics readout ----
        theme::section_frame().show(ui, |ui| {
            self.draw_metrics(ui);
        });

        // ---- Profile metadata (playback mode) ----
        if let DataSource::Profile(ref profile) = self.data_source {
            ui.add_space(3.0);
            theme::section_frame().show(ui, |ui| {
                ui.label(theme::heading("Profile"));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&profile.name).strong());
                ui.label(theme::caption(&format!("Branch: {}", profile.metadata.branch)));
                ui.label(theme::caption(&format!("Commit: {}", profile.metadata.commit_short)));
                ui.label(theme::caption(&format!("Time: {}", profile.metadata.timestamp)));
            });
        }

        ui.add_space(8.0);
    }

    fn draw_scenario_text(&self, ui: &mut egui::Ui) {
        if let Some(ref summary) = self.scenario_summary {
            let source = match &summary.source {
                ScenarioSourceKind::Generated => "Generated".to_string(),
                ScenarioSourceKind::Loaded { dir_name } => format!("Loaded ({})", dir_name),
            };
            ui.label(egui::RichText::new(source).small().color(theme::TEXT_0));
            ui.label(theme::caption(&format!(
                "{} SKUs ({} popular) · {} units",
                summary.num_skus, summary.num_popular_skus, summary.total_inventory_units,
            )));
            ui.label(theme::caption(&format!("{} orders · {} picks", summary.num_orders, summary.total_picks)));
        } else {
            ui.label(theme::caption("(no scenario loaded)"));
        }
    }
}

/// Single-row labeled control: small caption on the left, widget on the
/// right. Used for tight `Label: [combobox ▾]` rows.
pub(super) fn labeled_dropdown(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small().color(theme::TEXT_1));
        content(ui);
    });
}

/// Compact inline legend: small swatch + label, wrapping across the
/// available width. Each entry is allocated as a single rect so the
/// swatch and label always wrap together — never on separate lines.
fn inline_legend(ui: &mut egui::Ui, entries: &[(&str, Color32)]) {
    let prev_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing = egui::Vec2::new(8.0, 4.0);
    ui.horizontal_wrapped(|ui| {
        for &(label, color) in entries {
            legend_chip(ui, label, color);
        }
    });
    ui.spacing_mut().item_spacing = prev_spacing;
}

/// One legend entry painted as a single widget so wrap behaviour treats
/// (swatch + label) as an atomic unit.
fn legend_chip(ui: &mut egui::Ui, label: &str, color: Color32) {
    const SWATCH: f32 = 9.0;
    const GAP: f32 = 4.0;

    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, theme::TEXT_0);
    let width = SWATCH + GAP + galley.size().x;
    let height = galley.size().y.max(SWATCH);

    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter();

    let swatch_rect =
        egui::Rect::from_min_size(egui::pos2(rect.min.x, rect.center().y - SWATCH / 2.0), egui::Vec2::splat(SWATCH));
    painter.rect_filled(swatch_rect, egui::Rounding::same(2.0), color);

    let text_pos = egui::pos2(rect.min.x + SWATCH + GAP, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, theme::TEXT_0);
}

/// Fuzzy subsequence match: `Some(score)` when every char of `needle`
/// appears in `haystack` in order (case-insensitive), `None` otherwise.
/// Higher scores = tighter matches. Empty needle yields score 0 for
/// every input so callers can treat "no filter" uniformly.
///
/// Scoring favours (in rough order of weight):
/// - matches at the start of `haystack`,
/// - matches at a word boundary (preceded by space / `/` / `-` / `(`),
/// - consecutive matches (no skipped chars between consecutive needle
///   chars).
///
/// Hand-rolled rather than pulling in a fuzzy-match crate — the catalog
/// is ~40 items, so anything more clever would be overkill.
fn fuzzy_match_score(haystack: &str, needle: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let h: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    let n: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();

    let mut score: i64 = 0;
    let mut h_pos: usize = 0;
    let mut last_match: Option<usize> = None;

    for &nc in &n {
        let start = h_pos;
        while h_pos < h.len() && h[h_pos] != nc {
            h_pos += 1;
        }
        if h_pos >= h.len() {
            return None;
        }
        // Base hit reward, minus a small penalty per skipped char so a
        // 5-char match earlier in the label outscores the same chars
        // scattered widely.
        let gap = (h_pos - start) as i64;
        score += 10 - gap;
        if let Some(lm) = last_match {
            if h_pos == lm + 1 {
                score += 5;
            }
        }
        // Word-boundary / leading-char bonus.
        let at_start = h_pos == 0;
        let after_boundary = h_pos > 0 && h.get(h_pos - 1).map(|c| !c.is_alphanumeric()).unwrap_or(false);
        if at_start || after_boundary {
            score += 3;
        }
        last_match = Some(h_pos);
        h_pos += 1;
    }
    Some(score)
}

/// Sidebar's Charts-list row for one `MetricKind`. Checkbox toggles
/// "active" state (plot visibility) via `selected`; on-hover `−` button
/// removes the row from `sidebar_charts` entirely (and unselects). Any
/// row can be removed — featured metrics seed the list at startup but
/// aren't pinned there forever.
///
/// Hover detection runs over an explicit rect that spans the full row
/// width (the parent ui's max_rect right edge), not just the painted
/// widgets — without that override, hovering empty space to the right
/// of the label wouldn't trigger. One-frame-lagged via egui memory so
/// the layout (right-aligned button slot) stays stable across frames.
fn draw_sidebar_chart_row(
    ui: &mut egui::Ui,
    kind: MetricKind,
    selected: &mut Vec<MetricKind>,
    sidebar_charts: &mut Vec<MetricKind>,
    unavailable: Option<String>,
) {
    let row_id = ui.id().with(("sidebar_chart_row", kind));
    let prev_hovered: bool = ui.ctx().data(|d| d.get_temp(row_id).unwrap_or(false));

    let mut checked = selected.contains(&kind);
    let color = if unavailable.is_some() { egui::Color32::DARK_GRAY } else { kind.color() };

    let inner = ui.horizontal(|ui| {
        // An unavailable metric can't be enabled, but a stale selection
        // (e.g. picked before loading a scenario that lacks the data) can
        // still be unchecked.
        let checkbox_enabled = unavailable.is_none() || checked;
        if ui.add_enabled(checkbox_enabled, egui::Checkbox::new(&mut checked, "")).changed() {
            if checked {
                if !selected.contains(&kind) {
                    selected.push(kind);
                }
            } else {
                selected.retain(|&k| k != kind);
            }
        }
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(10.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, egui::Rounding::same(1.5), color);
        let label_text = egui::RichText::new(kind.label()).small();
        let label_text = if unavailable.is_some() { label_text.weak() } else { label_text };
        let label = ui.label(label_text);
        if let Some(text) = unavailable {
            label.on_hover_text(text);
        }

        // Reserve a right-aligned slot for the `−` button. Paint it only
        // when last frame was hovered; the slot is always allocated so
        // the surrounding layout stays stable.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if prev_hovered {
                if ui.small_button("−").on_hover_text("Remove from charts").clicked() {
                    sidebar_charts.retain(|&k| k != kind);
                    selected.retain(|&k| k != kind);
                }
            }
        });
    });

    // Extend hover detection to the full row width: a separate `interact`
    // call against the parent ui's right edge. The horizontal's own
    // response only covers painted widgets, so hovering empty space past
    // the label wouldn't register without this.
    //
    // Use `contains_pointer()` rather than `hovered()` so the row stays
    // "hovered" even when the pointer is directly over the `−` button —
    // egui's `hovered()` returns false for outer rects when an inner
    // interactive widget claims primary hover, which would make the
    // button vanish the instant the cursor touched it.
    let row_top = inner.response.rect.top();
    let row_bottom = inner.response.rect.bottom();
    let row_left = inner.response.rect.left();
    let row_right = ui.max_rect().right();
    let hovered_now = if row_right > row_left {
        let extended = egui::Rect::from_min_max(egui::pos2(row_left, row_top), egui::pos2(row_right, row_bottom));
        let hover_resp = ui.interact(extended, row_id.with("hover_rect"), egui::Sense::hover());
        hover_resp.contains_pointer()
    } else {
        inner.response.contains_pointer()
    };
    ui.ctx().data_mut(|d| d.insert_temp(row_id, hovered_now));
}

/// "+ Add chart" popup row. Checkbox represents *active/plotted state*
/// — same meaning as the sidebar row's checkbox so the two surfaces
/// reflect one truth. Checking promotes a metric into both
/// `sidebar_charts` (so it appears in the sidebar list) and `selected`
/// (so it plots). Unchecking removes only from `selected` — the row
/// stays in the sidebar as an inactive entry, removable only via the
/// sidebar's `−` button.
fn draw_popup_chart_row(
    ui: &mut egui::Ui,
    kind: MetricKind,
    selected: &mut Vec<MetricKind>,
    sidebar_charts: &mut Vec<MetricKind>,
    unavailable: Option<String>,
) {
    let mut active = selected.contains(&kind);
    let color = if unavailable.is_some() { egui::Color32::DARK_GRAY } else { kind.color() };

    ui.horizontal(|ui| {
        // Same enable rule as the sidebar row: no enabling an unavailable
        // metric, but stale selections stay uncheckable.
        let checkbox_enabled = unavailable.is_none() || active;
        if ui.add_enabled(checkbox_enabled, egui::Checkbox::new(&mut active, "")).changed() {
            if active {
                if !selected.contains(&kind) {
                    selected.push(kind);
                }
                if !sidebar_charts.contains(&kind) {
                    sidebar_charts.push(kind);
                }
            } else {
                selected.retain(|&k| k != kind);
                // Intentionally leave `sidebar_charts` alone — unchecking
                // here disables the chart but keeps the row in the
                // sidebar so the user can flip it back on without
                // re-finding it in the catalog.
            }
        }
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(10.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, egui::Rounding::same(1.5), color);
        let label_text = egui::RichText::new(kind.label()).small();
        let label_text = if unavailable.is_some() { label_text.weak() } else { label_text };
        let label = ui.label(label_text);
        if let Some(text) = unavailable {
            label.on_hover_text(text);
        }
    });
}

impl DesvisApp {
    /// Render the "+ Add chart" popup window. Search-box filters by label
    /// (case-insensitive substring); results are alphabetized. Drawn at the
    /// top level so the window floats above the sidebar/grid panels.
    ///
    /// The default egui title bar (Heading-sized title + hairline divider
    /// below it) reads as a separate slab attached to the card. We disable
    /// it and render our own header row inside the body — a small TEXT_1
    /// label + close button — so the whole popup reads as one card with
    /// the same chrome as the sidebar sections.
    pub(crate) fn draw_more_charts_window(&mut self, ctx: &egui::Context) {
        if !self.more_charts_open {
            return;
        }
        let mut keep_open = true;

        // Pin so the popup's painted left edge lines up with the bottom
        // chart/gantt panel's painted left edge. Both use `card_frame`
        // which has a 6px outer_margin, so anchoring the popup's outer
        // rect flush against the sidebar panel boundary gives matching
        // painted edges (popup_outer_left + 6 == bottom_card_left).
        // `sidebar_card_frame` margins sum to 22px on top of SIDEBAR_WIDTH.
        const SIDEBAR_CARD_CHROME: f32 = 22.0;
        let popup_x = SIDEBAR_WIDTH + SIDEBAR_CARD_CHROME;
        let popup_height = (ctx.screen_rect().height() * 0.5).max(120.0);

        egui::Window::new("add_chart")
            .title_bar(false)
            .frame(theme::card_frame())
            .anchor(egui::Align2::LEFT_CENTER, egui::vec2(popup_x, 0.0))
            .fixed_size(egui::vec2(320.0, popup_height))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                // Header row: title left, close-X right.
                ui.horizontal(|ui| {
                    ui.label(theme::heading("Add chart"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button(ui, "×").clicked() {
                            keep_open = false;
                        }
                    });
                });
                ui.add_space(6.0);

                // Search row.
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.more_charts_filter)
                            .desired_width(ui.available_width() - 60.0)
                            .hint_text("filter metrics…"),
                    );
                    if theme::button(ui, "clear").clicked() {
                        self.more_charts_filter.clear();
                    }
                });
                ui.add_space(4.0);

                // Fuzzy match: each needle char must appear in the label
                // in order (case-insensitive). With a needle present,
                // sort by descending match score so the closest hit is
                // on top; with an empty needle, fall back to alphabetical
                // so the catalog stays predictable when browsing.
                let needle = self.more_charts_filter.trim();
                let mut matches: Vec<(MetricKind, i64)> = MetricKind::all()
                    .iter()
                    .copied()
                    .filter_map(|k| fuzzy_match_score(k.label(), needle).map(|s| (k, s)))
                    .collect();
                if needle.is_empty() {
                    matches.sort_by(|a, b| a.0.label().to_lowercase().cmp(&b.0.label().to_lowercase()));
                } else {
                    matches.sort_by(|a, b| {
                        b.1.cmp(&a.1).then_with(|| a.0.label().to_lowercase().cmp(&b.0.label().to_lowercase()))
                    });
                }
                let matches: Vec<MetricKind> = matches.into_iter().map(|(k, _)| k).collect();

                if matches.is_empty() {
                    ui.label(theme::caption("(no metrics match filter)"));
                    return;
                }

                theme::apply_scrollbar_style(ui);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for kind in matches {
                        let unavailable =
                            kind.unavailable_label(self.num_stations, self.total_rack_slots, self.total_relay_cells);
                        draw_popup_chart_row(
                            ui,
                            kind,
                            &mut self.selected_metrics,
                            &mut self.sidebar_charts,
                            unavailable,
                        );
                    }
                });
            });
        self.more_charts_open = keep_open;
    }
}
