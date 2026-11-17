//! Sim control buttons: run/stop/reset and playback controls.

use std::time::Instant;

use eframe::egui;

use dessim::StopReason;

use crate::app::DesvisApp;
use crate::theme;
use crate::types::*;

impl DesvisApp {
    pub(crate) fn draw_sim_buttons(&mut self, ui: &mut egui::Ui) {
        // A window that plays a recording back starts no run; one that ran a
        // run holds its profile once the run is over, and runs again.
        let plays_back = self.run_preparation.is_none();

        if !plays_back {
            match &self.sim_status {
                SimStatus::Running => {
                    ui.horizontal(|ui| {
                        ui.label("Running...");
                        if theme::button(ui, "Stop").clicked() {
                            if let Some(live) = &self.live_run {
                                live.stop(StopReason::UserStop);
                            }
                        }
                    });
                }
                SimStatus::Idle => {
                    let w = ui.available_width();
                    if theme::button_with_min_size(ui, "Run Simulation", egui::Vec2::new(w, 0.0)).clicked() {
                        self.start_simulation();
                    }
                }
                SimStatus::Complete(_) | SimStatus::Error(_) => {
                    ui.horizontal(|ui| {
                        if theme::button(ui, "Re-run").clicked() {
                            self.start_simulation();
                        }
                        if theme::button(ui, "Reset").clicked() {
                            self.reset_simulation();
                        }
                    });
                }
            }
        }

        // Play/Pause button: available whenever there's data to scrub through.
        // During a Running sim the cursor auto-follows the live edge (see
        // playback-advance in app/frame.rs).
        let can_play =
            matches!(self.sim_status, SimStatus::Running | SimStatus::Complete(_) | SimStatus::Error(_)) || plays_back;
        if can_play && !self.time_series.is_empty() {
            ui.horizontal(|ui| {
                // Two direction buttons; the active direction shows Pause.
                // Clicking the inactive direction mid-play flips direction
                // in place (start_playback re-anchors at the cursor).
                let reverse_playing = self.playback_active && self.playback_reverse;
                let forward_playing = self.playback_active && !self.playback_reverse;
                let rev_label = if reverse_playing { "Pause" } else { "◀ Rev" };
                // There's nothing before t=0, so reverse can't move: disable
                // the button at the start rather than let it flicker
                // engage-then-instantly-stop. Stays enabled while reversing so
                // the Pause it turns into is always clickable.
                let rev_enabled = reverse_playing || self.time_cursor_ms > 0.0;
                let rev_clicked = ui.add_enabled_ui(rev_enabled, |ui| theme::button(ui, rev_label).clicked()).inner;
                if rev_clicked {
                    if reverse_playing {
                        self.playback_active = false;
                    } else {
                        self.start_playback(true);
                    }
                }
                let play_label = if forward_playing { "Pause" } else { "Play ▶" };
                if theme::button(ui, play_label).clicked() {
                    if forward_playing {
                        self.playback_active = false;
                    } else {
                        self.start_playback(false);
                    }
                }
                // Speed label next to buttons.
                ui.label(egui::RichText::new(format!("{:.0}x", self.playback_speed)).small());
            });

            // Speed slider: interactive whenever playback controls are
            // visible. Changing it while paused just stages the new speed
            // for the next Play; while active, the re-anchor below keeps
            // the cursor smooth across the change.
            ui.add(
                egui::Slider::new(&mut self.playback_speed, 1.0..=50.0)
                    .logarithmic(true)
                    .text("Speed")
                    .custom_formatter(|v, _| format!("{:.0}x", v))
                    .custom_parser(|s| s.trim_end_matches('x').parse::<f64>().ok()),
            )
            .on_hover_text("Playback speed multiplier (1x = realtime)");

            // When speed changes during active playback, re-anchor the
            // wall-clock start so the cursor doesn't jump.
            if self.playback_active {
                // Re-anchor every frame is harmless -- the elapsed calc uses
                // (now - wall_start) * speed, and we reset wall_start = now,
                // playback_sim_start_ms = current cursor.  This keeps speed
                // changes instantaneous without a position discontinuity.
                self.playback_wall_start = Some(Instant::now());
                self.playback_sim_start_ms = self.time_cursor_ms;
            }

            // Sim time display in decimal seconds.
            let t_sec = self.time_cursor_ms / 1000.0;
            ui.label(egui::RichText::new(format!("T = {:.1}s", t_sec)).monospace());
        }
    }
}
