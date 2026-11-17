//! Render completed extraction counts and rates from the shared report, with
//! separate warehouse totals and station rows. Picking completion is a different
//! milestone; missing historical extraction facts stay unavailable.

use dessim::metrics::reporting::{ExtractionData, MetricsReport, ReportBuildError};
use eframe::egui::{self, RichText};

use super::{SERIES_COLORS, draw_common_controls, draw_line_chart, format_duration, unavailable};
use crate::theme;
use crate::types::MetricsPanelState;

pub(super) fn draw_extraction(
    ui: &mut egui::Ui,
    temporal: &Result<MetricsReport<ExtractionData>, ReportBuildError>,
    overall: &Result<MetricsReport<ExtractionData>, ReportBuildError>,
    state: &mut MetricsPanelState,
) {
    draw_common_controls(ui, state, true);
    ui.weak("An extraction completes when its physical dwell ends. Fully picked orders may still await extraction.");
    let (Ok(temporal), Ok(overall)) = (temporal, overall) else {
        ui.colored_label(theme::ERR, "Unable to build the extraction report.");
        return;
    };
    let total = overall.data.rows.iter().find(|row| row.station_id.is_none());
    let Some(total) = total else {
        ui.weak("The selected scope does not yet contain any simulation time.");
        return;
    };
    if total.extractions.is_none() {
        unavailable(ui, "This profile did not record completed extraction work.");
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (label, value) in [("Completed extractions", total.extractions), ("Extracted units", total.extracted_units)]
        {
            ui.group(|ui| {
                ui.label(RichText::new(count(value)).size(20.0).strong());
                ui.weak(label);
            });
        }
    });

    ui.add_space(8.0);
    ui.label(RichText::new("Extraction stations — selected scope").strong());
    egui::Grid::new("extraction_station_totals").striped(true).show(ui, |ui| {
        for label in ["Station", "Completed extractions", "Extracted units", "Extractions/hr", "Units/hr"] {
            ui.strong(label);
        }
        ui.end_row();
        for row in &overall.data.rows {
            ui.label(
                row.station_id.map_or_else(|| "All extraction stations".into(), |id| format!("Extraction {}", id.0)),
            );
            ui.label(count(row.extractions));
            ui.label(count(row.extracted_units));
            ui.label(rate(row.extractions_per_hour()));
            ui.label(rate(row.extracted_units_per_hour()));
            ui.end_row();
        }
    });

    let totals: Vec<_> = temporal.data.rows.iter().filter(|row| row.station_id.is_none()).collect();
    if totals.is_empty() {
        ui.weak("No complete trailing window fits this scope yet — shorten the window to view extraction rates.");
    } else {
        for (index, title) in ["Completed extractions/hr", "Extracted units/hr"].into_iter().enumerate() {
            let points = totals
                .iter()
                .filter_map(|row| {
                    let value = if index == 0 { row.extractions_per_hour() } else { row.extracted_units_per_hour() }?;
                    Some([row.bucket.end_ms as f64 / 60_000.0, value])
                })
                .collect();
            draw_line_chart(
                ui,
                if index == 0 { "extraction_rate" } else { "extracted_units_rate" },
                title,
                vec![("All extraction stations".into(), SERIES_COLORS[index + 2], points)],
            );
        }
    }
    ui.weak(format!(
        "Scope ends at {}. Rates use the included duration of each report window.",
        format_duration(total.bucket.end_ms)
    ));
}

fn count(value: Option<u64>) -> String {
    value.map_or_else(|| "Unavailable".into(), |value| value.to_string())
}

fn rate(value: Option<f64>) -> String {
    value.map_or_else(|| "Unavailable".into(), |value| format!("{value:.1}"))
}
