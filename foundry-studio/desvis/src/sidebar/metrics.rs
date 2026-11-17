//! Sidebar metrics display: live, completed, and profile stats.

use std::time::Duration;

use eframe::egui;

use crate::app::DesvisApp;
use crate::theme;
use crate::types::*;

impl DesvisApp {
    /// Unified metrics section that works for Running, Complete, and Profile states.
    pub(crate) fn draw_metrics(&self, ui: &mut egui::Ui) {
        ui.label(theme::heading("Status"));
        ui.add_space(2.0);

        match &self.sim_status {
            SimStatus::Idle => {
                ui.label(egui::RichText::new("Idle").color(theme::TEXT_1));
            }
            SimStatus::Running => {
                // What the slices received state: their picks and fully picked
                // orders, summed, and the simtime the newest of them reaches;
                // and, of the orders, how many the scenario resolved to.
                let (picks, orders, of) = self
                    .live_run
                    .as_ref()
                    .map_or((0, 0, None), |live| (live.picks(), live.orders_completed(), live.orders()));
                let sim_ms = self.time_series.last().map_or(0, |slice| slice.start_ms + self.slice_width_ms);

                // Live aggregate rate mirrors the final `throughput_picks_per_hour`:
                // divide by the post-embargo window (0 while still in the embargo
                // lead-in) so the running number matches the completed one
                // (greymatter-foundry#208).
                let picks_per_hr = dessim::sim_types::picks_per_hour(
                    picks,
                    Duration::from_millis(sim_ms),
                    self.base_config.pick.embargo_secs,
                );
                let wall_secs = self.sim_start_wall.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
                let sim_h = sim_ms / 3_600_000;
                let sim_m = (sim_ms % 3_600_000) / 60_000;

                ui.label(egui::RichText::new("Running").color(theme::OK).strong());
                ui.add_space(3.0);
                stat_row(ui, "Picks", &format!("{} ({:.0}/hr)", picks, picks_per_hr));
                let orders = match of {
                    Some(of) => format!("{orders} / {of}"),
                    None => format!("{orders}"),
                };
                stat_row(ui, "Fully picked orders", &orders);
                stat_row(ui, "Wall", &format!("{:.0}s", wall_secs));
                stat_row(ui, "Sim time", &format!("{}h {:02}m", sim_h, sim_m));
            }
            SimStatus::Complete(_) => {
                ui.label(egui::RichText::new("Complete").color(theme::OK).strong());
                ui.add_space(3.0);

                // A run that completes, live or played back, is its profile.
                if let DataSource::Profile(ref profile) = self.data_source {
                    let s = &profile.summary;
                    let sim_h = s.sim_time_secs / 3600.0;
                    stat_row(ui, "Picks", &format!("{} ({:.0}/hr)", s.total_picks, s.throughput_picks_per_hour));
                    stat_row(ui, "Fully picked orders", &format!("{}", s.total_orders_completed));
                    stat_row(ui, "Throughput", &format!("{:.0} picks/hr", s.throughput_picks_per_hour));
                    stat_row(ui, "Wall", &format!("{:.1}s", s.wall_clock_secs));
                    stat_row(ui, "Sim time", &format!("{:.1}h", sim_h));
                    stat_row(ui, "DA50 idle", &format!("{:.1}%", s.vtm_idle_pct * 100.0));
                    stat_row(ui, "D30 idle", &format!("{:.1}%", s.htm_idle_pct * 100.0));
                    stat_row(ui, "Flow score", &format!("{:.2}", s.flow_score));
                    stat_row(ui, "Avg stn util", &format!("{:.1}%", s.avg_station_utilization_pct * 100.0));
                    stat_row(ui, "Pathfinder fail", &format!("{:.2}%", s.pathfinder_failure_rate * 100.0));
                    stat_row(ui, "Events", &format!("{}", s.event_count));
                }
            }
            SimStatus::Error(e) => {
                ui.label(egui::RichText::new("Error").color(theme::ERR).strong());
                ui.label(egui::RichText::new(e).small().color(theme::ERR));
            }
        }
    }
}

/// Label : value row. Label muted (TEXT_2), value primary (TEXT_0).
/// Monospace value column keeps numbers aligned when scanned vertically.
fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small().color(theme::TEXT_2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).small().monospace().color(theme::TEXT_0));
        });
    });
}
