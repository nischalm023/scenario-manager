//! Station inspector body: the "why idle?" status line, facts, pipeline
//! depth, inbound HTMs, recent blockers, and the shared recent-events feed.

use desgraph::StationId;
use dessim::model::SimTime;
use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};

use super::super::DesvisApp;
use super::super::path_overlay::inbound_path_color;
use super::{InspectorAction, ResourceTimeline, eta_label, fmt_hms, recent_events_section, station_type_label, strip};
use crate::station_diag::{REJECTION_WINDOW, StationDiagnosis};
use crate::types::{BotType, SelectedResource};

impl DesvisApp {
    /// Returns the frame's `InspectorAction` (strip scrub and/or a cross-link
    /// selection from a clicked inbound-HTM row).
    pub(super) fn station_inspector_ui(
        &self,
        ui: &mut egui::Ui,
        station_id: StationId,
        cursor: SimTime,
        max_ms: f64,
    ) -> InspectorAction {
        let store = self.station_diag_store.as_ref().expect("store built in draw_inspector_window");
        let mut action = InspectorAction::default();

        // Prominent "Status:" line answering "why is this station idle?".
        let diagnosis = store.station_idle_diagnosis(station_id, cursor, REJECTION_WINDOW);
        Self::render_status_line(ui, &diagnosis);
        ui.separator();

        // Full-run timeline strip: serving spans vs idle, rejection ticks,
        // from the cached extraction (audit M3).
        action.scrub = match &self.resource_timeline_cache {
            Some(ResourceTimeline::Station { serving, rejection_marks, .. }) => {
                strip::station_strip(ui, serving, rejection_marks, cursor.as_ms() as f64, max_ms)
            }
            _ => None,
        };
        ui.separator();

        if let Some(meta) = self.station_metas.iter().find(|meta| meta.id == station_id) {
            ui.label(format!("type: {}", meta.process_label.unwrap_or_else(|| station_type_label(meta.station_type))));
            for point in &meta.presentation_points {
                ui.label(format!("presentation cell: {}", point.presentation_cell.0));
                ui.label(format!("queue path length: {}", point.queue_path.len()));
            }
            match meta.bins {
                Some(bins) => ui.label(format!("bins: {bins}")),
                None => ui.label("bins: default"),
            };
        }
        match store.latest_pipeline_depth(station_id, cursor) {
            Some(depth) => ui.label(format!("pipeline depth: {depth}")),
            None => ui.label("pipeline depth: n/a"),
        };

        let inbound = store.inbound_htms_at(station_id, cursor);
        let blockers = store.rejections_in_window(station_id, cursor, REJECTION_WINDOW);

        ui.separator();
        ui.strong(format!("Inbound HTMs ({})", inbound.len()));
        for htm in &inbound {
            let (eta, overdue) = eta_label(htm.quoted_arrival, cursor);
            let tote = htm.tote_id.map(|tote| format!(" tote {tote}")).unwrap_or_default();
            let relay = htm.relay_cell.map(|c| format!(" via relay {c}")).unwrap_or_default();
            let line = format!(
                "D{} · {}{tote}{relay} · dispatched {} · ETA {eta}",
                htm.robot_id,
                htm.source,
                fmt_hms(htm.dispatch_time)
            );
            // A color chip matching this HTM's on-map path color (keyed by
            // robot_id), so the inbound list and the drawn paths correspond.
            // The row is a clickable link that cross-selects the HTM.
            ui.horizontal(|ui| {
                let (chip, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().rect_filled(chip, egui::Rounding::same(2.0), inbound_path_color(htm.robot_id));
                let text = if overdue {
                    egui::RichText::new(line).color(egui::Color32::from_rgb(240, 100, 90))
                } else {
                    egui::RichText::new(line)
                };
                if ui.link(text).clicked() {
                    action.select = Some(SelectedResource::Bot { bot_type: BotType::Htm, bot_id: htm.robot_id });
                }
            });
        }
        if store.has_order_phases(station_id) {
            let counts = store.order_phase_counts(station_id, cursor);
            ui.label(format!("assigned orders: {}", counts.assigned));
            ui.label(format!(
                "preparing {} · dispatched {} · admitted {} · waiting {} · presenting {}",
                counts.preparing, counts.dispatched, counts.admitted, counts.waiting, counts.presenting
            ));
            ui.label(format!("physical residents: {}", counts.physical_residents));
            match counts.conveyor_id {
                Some(conveyor) => match store.shared_operator_busy_since(conveyor, cursor) {
                    Some(since) => ui.label(format!("shared operator: busy since {}", fmt_hms(since))),
                    None => ui.label("shared operator: idle · cause unknown"),
                },
                None => ui.label("shared operator: unknown"),
            };
            match store.latest_readiness_pair(station_id, cursor) {
                Some((order_ready, inventory_ready, order, pick)) => ui.label(format!(
                    "order {order} pick {pick} readiness: order {} · inventory {} · mismatch {} ms",
                    fmt_hms(order_ready),
                    fmt_hms(inventory_ready),
                    order_ready.as_ms().abs_diff(inventory_ready.as_ms())
                )),
                None => ui.label("matched presentation readiness: unknown"),
            };
            let (assigned, residents) = store.order_count_series(station_id, cursor);
            ui.strong("Order state over time");
            Plot::new(format!("station_order_counts_{}", station_id.0))
                .height(140.0)
                .x_axis_label("simulation minutes")
                .y_axis_label("orders")
                .legend(Legend::default())
                .show(ui, |plot_ui| {
                    plot_ui.line(Line::new(PlotPoints::new(assigned)).name("logical assigned"));
                    plot_ui.line(Line::new(PlotPoints::new(residents)).name("physical residents"));
                });

            if let Some(conveyor) =
                store.conveyor_for_station(station_id).filter(|id| store.has_inventory_visit_phases(*id))
            {
                let inventory = store.inventory_visit_counts(conveyor, cursor);
                ui.label(format!(
                    "inventory CONWIP {} · upstream {} · loaded transit {}",
                    inventory.conwip, inventory.upstream, inventory.loaded_transit
                ));
                let (conwip, upstream, loaded) = store.inventory_visit_series(conveyor, cursor);
                ui.strong("Inventory visits over time");
                Plot::new(format!("station_inventory_counts_{}", station_id.0))
                    .height(140.0)
                    .x_axis_label("simulation minutes")
                    .y_axis_label("visits")
                    .legend(Legend::default())
                    .show(ui, |plot_ui| {
                        plot_ui.line(Line::new(PlotPoints::new(conwip)).name("CONWIP"));
                        plot_ui.line(Line::new(PlotPoints::new(upstream)).name("upstream retrieval"));
                        plot_ui.line(Line::new(PlotPoints::new(loaded)).name("loaded transit"));
                    });
            }
        } else {
            ui.weak("EasyLoop order and inventory diagnostics unavailable");
        }

        ui.separator();
        ui.strong(format!("Recent blockers (last {}s)", REJECTION_WINDOW.as_ms() / 1000));
        for blocker in &blockers {
            let detail = blocker.last_detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default();
            ui.label(format!("{} ×{} · last {}{detail}", blocker.reason, blocker.count, fmt_hms(blocker.last_at)));
        }
        if inbound.is_empty() && blockers.is_empty() {
            ui.weak("no dispatch activity in window");
        }

        recent_events_section(ui, cursor, &store.recent_station_events(station_id, cursor));
        action
    }

    /// Render the prominent "Status:" line + one-line evidence from the idle
    /// diagnosis. Green = serving, amber = waiting/blocked, grey = unrecorded,
    /// red = an overdue inbound HTM (the "not in time" case).
    fn render_status_line(ui: &mut egui::Ui, diagnosis: &StationDiagnosis) {
        let amber = egui::Color32::from_rgb(240, 180, 90);
        let (color, headline, evidence) = match diagnosis {
            StationDiagnosis::Serving { since } => (
                egui::Color32::from_rgb(120, 210, 120),
                "Serving".to_string(),
                format!("cycle since {}", fmt_hms(*since)),
            ),
            StationDiagnosis::WaitingInbound { count, next_eta, overdue } => {
                let color = if *overdue { egui::Color32::from_rgb(240, 100, 90) } else { amber };
                let evidence = if *overdue {
                    format!("{count} inbound, next overdue by {}s", next_eta.as_ms() / 1000)
                } else {
                    format!("{count} inbound, next ETA +{}s", next_eta.as_ms() / 1000)
                };
                (color, "Idle — waiting on HTMs".to_string(), evidence)
            }
            StationDiagnosis::Rejected { cause, reason, count, last_at } => {
                (amber, format!("Idle — {cause}"), format!("{reason} ×{count} · last {}", fmt_hms(*last_at)))
            }
            StationDiagnosis::IdleUnrecorded { since } => {
                let evidence = match since {
                    Some(t) => format!("last cycle ended {}", fmt_hms(*t)),
                    None => "no cycle recorded yet".to_string(),
                };
                (egui::Color32::from_gray(160), "Idle — cause unrecorded in window".to_string(), evidence)
            }
        };
        ui.horizontal(|ui| {
            ui.strong("Status:");
            ui.colored_label(color, egui::RichText::new(headline).strong());
        });
        ui.label(egui::RichText::new(evidence).weak().small());
    }
}
