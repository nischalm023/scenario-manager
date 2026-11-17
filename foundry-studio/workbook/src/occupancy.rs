//! Within `workbook`, `occupancy` renders the Aisle Occupancy sheet:
//! capacity-time ratio tables (storage slots by payload state, physical
//! diving-board positions) per scope and per hourly bucket, with the hourly
//! chart. Values come entirely from
//! `dessim::metrics::reporting::aisle_occupancy_report`; this module owns
//! only layout.

use std::collections::BTreeSet;

use dessim::metrics::reporting::{AisleOccupancyData, AisleOccupancyRow, MetricsReport, Ratio};
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};

use super::{bold, hour_fmt, line_chart_for_columns, metric_label, support_status_label};

#[derive(Clone, Copy)]
enum AisleMetricKind {
    StorageOccupied,
    StorageEmpty,
    StorageNonEmpty,
    StorageUnknown,
    DivingBoardOccupied,
}

pub(super) fn write_aisle_occupancy_sheet(
    wb: &mut Workbook,
    full_aisle: &MetricsReport<AisleOccupancyData>,
    post_embargo_aisle: &MetricsReport<AisleOccupancyData>,
    hourly_aisle: &MetricsReport<AisleOccupancyData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Aisle Occupancy")?;
    let b = bold();
    let percent = Format::new().set_num_format("0.00%");

    let mut row = 0;
    for (metric_id, kind) in [
        ("storage.slots.occupied", AisleMetricKind::StorageOccupied),
        ("storage.slots.empty_tote", AisleMetricKind::StorageEmpty),
        ("storage.slots.non_empty_tote", AisleMetricKind::StorageNonEmpty),
        ("storage.slots.unknown_payload", AisleMetricKind::StorageUnknown),
        ("relay.positions.occupied", AisleMetricKind::DivingBoardOccupied),
    ] {
        let title = format!("{} by aisle", metric_label(&hourly_aisle.descriptors, metric_id));
        row = write_aisle_metric_matrix(ws, row, &title, hourly_aisle, kind, &b, &percent)?;
    }

    ws.write_string_with_format(row, 0, "Normalized occupancy data", &b)?;
    row += 1;
    let headers = [
        "Scope",
        "Period",
        "Start hour",
        "End hour",
        "Partial",
        "Aisle",
        "Status",
        "Storage occupied avg slots",
        "Storage occupied slot-ms",
        "Storage capacity slot-ms",
        "Storage occupied %",
        "Empty tote avg slots",
        "Empty tote slot-ms",
        "Storage capacity slot-ms",
        "Empty tote %",
        "Non-empty tote avg slots",
        "Non-empty tote slot-ms",
        "Storage capacity slot-ms",
        "Non-empty tote %",
        "Unknown tote avg slots",
        "Unknown tote slot-ms",
        "Storage capacity slot-ms",
        "Unknown tote %",
        "Diving board avg occupied",
        "Diving board occupied position-ms",
        "Diving board capacity position-ms",
        "Diving board occupied %",
    ];
    for (column, header) in headers.iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    let hour = hour_fmt();
    for (scope, period, report) in [
        ("Full run", "Overall", full_aisle),
        ("Post-embargo", "Overall", post_embargo_aisle),
        ("Full run", "Hourly", hourly_aisle),
    ] {
        for occupancy in &report.data.rows {
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, period)?;
            ws.write_number_with_format(row, 2, occupancy.bucket.start_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_number_with_format(row, 3, occupancy.bucket.end_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_boolean(row, 4, occupancy.bucket.partial)?;
            ws.write_number(row, 5, occupancy.zone_id as f64)?;
            ws.write_string(row, 6, support_status_label(occupancy.status))?;
            write_aisle_ratio_group(ws, row, 7, occupancy.storage_occupied, occupancy.bucket.coverage_ms, &percent)?;
            write_aisle_ratio_group(ws, row, 11, occupancy.storage_empty, occupancy.bucket.coverage_ms, &percent)?;
            write_aisle_ratio_group(ws, row, 15, occupancy.storage_non_empty, occupancy.bucket.coverage_ms, &percent)?;
            write_aisle_ratio_group(ws, row, 19, occupancy.storage_unknown, occupancy.bucket.coverage_ms, &percent)?;
            write_aisle_ratio_group(
                ws,
                row,
                23,
                occupancy.diving_board_occupied,
                occupancy.bucket.coverage_ms,
                &percent,
            )?;
            row += 1;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_aisle_metric_matrix(
    ws: &mut Worksheet,
    start_row: u32,
    title: &str,
    report: &MetricsReport<AisleOccupancyData>,
    kind: AisleMetricKind,
    b: &Format,
    percent: &Format,
) -> Result<u32, XlsxError> {
    let zones: Vec<_> = report.data.rows.iter().map(|row| row.zone_id).collect::<BTreeSet<_>>().into_iter().collect();
    let buckets: Vec<_> = report.data.rows.iter().map(|row| row.bucket).collect::<BTreeSet<_>>().into_iter().collect();
    ws.write_string_with_format(start_row, 0, title, b)?;
    let header_row = start_row + 1;
    ws.write_string_with_format(header_row, 0, "End hour", b)?;
    for (index, zone_id) in zones.iter().enumerate() {
        ws.write_string_with_format(header_row, index as u16 + 1, format!("Aisle {zone_id}"), b)?;
    }
    for (bucket_index, bucket) in buckets.iter().enumerate() {
        let row = header_row + 1 + bucket_index as u32;
        ws.write_number_with_format(row, 0, bucket.end_ms as f64 / 3_600_000.0, &hour_fmt())?;
        for (zone_index, zone_id) in zones.iter().enumerate() {
            let ratio = report
                .data
                .rows
                .iter()
                .find(|candidate| candidate.bucket == *bucket && candidate.zone_id == *zone_id)
                .and_then(|row| aisle_metric_ratio(row, kind));
            if let Some(fraction) = ratio.and_then(Ratio::fraction) {
                ws.write_number_with_format(row, zone_index as u16 + 1, fraction, percent)?;
            }
        }
    }
    if !zones.is_empty() && !buckets.is_empty() {
        let first_data_row = header_row + 1;
        let last_data_row = first_data_row + buckets.len() as u32 - 1;
        let value_columns: Vec<_> = (1..=zones.len() as u16).collect();
        if let Some(mut chart) = line_chart_for_columns(
            "Aisle Occupancy",
            title,
            "Capacity-time occupancy",
            first_data_row,
            last_data_row,
            0,
            &value_columns,
            "Simulation time (h)",
        ) {
            chart.y_axis().set_num_format("0%");
            ws.insert_chart(start_row, zones.len() as u16 + 2, &chart)?;
        }
    }
    Ok(start_row + (buckets.len() as u32 + 3).max(17))
}

fn aisle_metric_ratio(row: &AisleOccupancyRow, kind: AisleMetricKind) -> Option<Ratio> {
    match kind {
        AisleMetricKind::StorageOccupied => row.storage_occupied,
        AisleMetricKind::StorageEmpty => row.storage_empty,
        AisleMetricKind::StorageNonEmpty => row.storage_non_empty,
        AisleMetricKind::StorageUnknown => row.storage_unknown,
        AisleMetricKind::DivingBoardOccupied => row.diving_board_occupied,
    }
}

fn write_aisle_ratio_group(
    ws: &mut Worksheet,
    row: u32,
    start_column: u16,
    ratio: Option<Ratio>,
    coverage_ms: u64,
    percent: &Format,
) -> Result<(), XlsxError> {
    let Some(ratio) = ratio else {
        return Ok(());
    };
    if coverage_ms > 0 {
        ws.write_number(row, start_column, ratio.numerator as f64 / coverage_ms as f64)?;
    }
    ws.write_number(row, start_column + 1, ratio.numerator as f64)?;
    ws.write_number(row, start_column + 2, ratio.denominator as f64)?;
    if let Some(fraction) = ratio.fraction() {
        ws.write_number_with_format(row, start_column + 3, fraction, percent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use dessim::metrics::reporting::{FixedPeriod, MetricScope, ReportPeriod, ReportRequest, aisle_occupancy_report};
    use dessim::observer::output::AisleStateRecord;
    use rust_xlsxwriter::Workbook;
    use zip::ZipArchive;

    use super::write_aisle_occupancy_sheet;

    #[test]
    fn aisle_occupancy_sheet_charts_canonical_capacity_time_tables() {
        let records = [
            AisleStateRecord {
                sim_time_ms: 0,
                zone_id: 1,
                storage_capacity: 4,
                storage_occupied: 2,
                storage_empty: 1,
                storage_non_empty: 1,
                storage_unknown: 0,
                diving_board_capacity: 2,
                diving_board_occupied: 0,
            },
            AisleStateRecord {
                sim_time_ms: 3_600_000,
                zone_id: 1,
                storage_capacity: 4,
                storage_occupied: 3,
                storage_empty: 1,
                storage_non_empty: 2,
                storage_unknown: 0,
                diving_board_capacity: 2,
                diving_board_occupied: 1,
            },
        ];
        let full = aisle_occupancy_report(
            &records,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 7_200_000, 0),
        )
        .unwrap();
        let post_embargo = aisle_occupancy_report(
            &records,
            ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 7_200_000, 3_600_000),
        )
        .unwrap();
        let hourly = aisle_occupancy_report(
            &records,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), 7_200_000, 0),
        )
        .unwrap();

        let mut workbook = Workbook::new();
        write_aisle_occupancy_sheet(&mut workbook, &full, &post_embargo, &hourly).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing
        // left behind on disk when an assertion below fails.
        let buf = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"Aisle Occupancy\""));

        let mut chart_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut chart_xml).unwrap();
        assert!(chart_xml.contains("<c:lineChart>"));
        assert!(chart_xml.contains("'Aisle Occupancy'!$A$3:$A$4"));
        assert!(chart_xml.contains("'Aisle Occupancy'!$B$3:$B$4"));

        let mut relay_chart_xml = String::new();
        archive.by_name("xl/charts/chart5.xml").unwrap().read_to_string(&mut relay_chart_xml).unwrap();
        // The title comes from the producer's descriptor label, not a
        // hand-written string — the wiring under test.
        let descriptors = dessim::metrics::reporting::aisle_occupancy_descriptors();
        let relay_label =
            &descriptors.iter().find(|descriptor| descriptor.id == "relay.positions.occupied").unwrap().label;
        assert!(relay_chart_xml.contains(relay_label.as_str()));

        drop(archive);
    }
}
