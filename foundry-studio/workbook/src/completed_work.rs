//! Within `workbook`, `completed_work` renders the completed-work sheets:
//! the raw Completed Tote Work log sheet (straight from the pick and
//! completed-order fact logs) and the Aisle x Station / DC x Station
//! distribution matrices with their charts, whose values come entirely from
//! `dessim::metrics::reporting::completed_work_report`. This module owns
//! only layout.

use std::collections::BTreeSet;

use dessim::metrics::reporting::{CompletedWorkData, MetricsReport};
use dessim::observer::output::{CompletedOrderRecord, PickRecord};
use rust_xlsxwriter::{Chart, ChartType, ConditionalFormat2ColorScale, Format, Workbook, Worksheet, XlsxError};

use super::{bold, hour_fmt, hour_index};

pub(super) fn write_completed_tote_work_sheet(
    wb: &mut Workbook,
    picks: &[PickRecord],
    completed_orders: &[CompletedOrderRecord],
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Completed Tote Work")?;
    let b = bold();
    let pick_headers = [
        "Completion Time (ms)",
        "Hour",
        "Trip ID",
        "Order ID",
        "Tote ID",
        "Pick Station",
        "Source Aisle",
        "Source Relay",
        "DC",
        "SKU",
        "Picked Quantity",
    ];
    for (column, header) in pick_headers.iter().enumerate() {
        ws.write_string_with_format(0, column as u16, *header, &b)?;
    }
    // Chronological, like every other journal sheet: a recording states a row
    // in the frame written once B has passed it, so its rows reach the book in
    // another order than the run appended them. A stable sort by instant
    // prints the same sheet from either, rows sharing an instant in the order
    // the run recorded them.
    let mut picks: Vec<&PickRecord> = picks.iter().collect();
    picks.sort_by_key(|pick| pick.sim_time_ms);
    let mut completed_orders: Vec<&CompletedOrderRecord> = completed_orders.iter().collect();
    completed_orders.sort_by_key(|order| order.sim_time_ms);
    for (index, pick) in picks.iter().enumerate() {
        let row = index as u32 + 1;
        ws.write_number(row, 0, pick.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(pick.sim_time_ms))?;
        if let Some(value) = pick.presentation_id {
            ws.write_number(row, 2, value as f64)?;
        }
        if let Some(value) = pick.order_id {
            ws.write_number(row, 3, value.0 as f64)?;
        }
        if let Some(value) = pick.tote_id {
            ws.write_number(row, 4, value as f64)?;
        }
        ws.write_number(row, 5, pick.station_id.0 as f64)?;
        if let Some(value) = pick.source_zone_id {
            ws.write_number(row, 6, value as f64)?;
        }
        if let Some(value) = pick.source_relay_id {
            ws.write_number(row, 7, value.0 as f64)?;
        }
        if let Some(value) = &pick.dc_id {
            ws.write_string(row, 8, value)?;
        }
        ws.write_number(row, 9, pick.sku.0 as f64)?;
        ws.write_number(row, 10, pick.quantity as f64)?;
    }

    let mut row = picks.len() as u32 + 3;
    ws.write_string_with_format(row, 0, "Fully picked orders", &b)?;
    row += 1;
    for (column, header) in ["Completion Time (ms)", "Hour", "Order ID", "Pick Station", "DC"].iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for order in completed_orders {
        ws.write_number(row, 0, order.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(order.sim_time_ms))?;
        ws.write_number(row, 2, order.order_id.0 as f64)?;
        if let Some(value) = order.station_id {
            ws.write_number(row, 3, value.0 as f64)?;
        }
        if let Some(value) = &order.dc_id {
            ws.write_string(row, 4, value)?;
        }
        row += 1;
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

pub(super) fn write_aisle_station_sheet(
    wb: &mut Workbook,
    full: &MetricsReport<CompletedWorkData>,
    post_embargo: &MetricsReport<CompletedWorkData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Aisle x Station")?;
    let b = bold();
    let percent = Format::new().set_num_format("0.00%");
    let mut row = 0;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        row = write_aisle_station_matrix(ws, row, &format!("{scope} — completed totes"), report, false, &b, &percent)?;
        row = write_aisle_station_matrix(
            ws,
            row,
            &format!("{scope} — aisle share per pick station"),
            report,
            true,
            &b,
            &percent,
        )?;
    }

    ws.write_string_with_format(row, 0, "Normalized data", &b)?;
    row += 1;
    for (column, header) in [
        "Scope",
        "Start hour",
        "End hour",
        "Aisle",
        "Pick Station",
        "Tote Count",
        "Share Numerator",
        "Share Denominator",
        "Aisle Share of Station",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for value in &report.data.aisle_station_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_number(row, 1, value.bucket.start_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 2, value.bucket.end_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 3, value.zone_id as f64)?;
            ws.write_number(row, 4, value.station_id.0 as f64)?;
            ws.write_number(row, 5, value.tote_count as f64)?;
            ws.write_number(row, 6, value.aisle_share_of_station.numerator as f64)?;
            ws.write_number(row, 7, value.aisle_share_of_station.denominator as f64)?;
            if let Some(fraction) = value.aisle_share_of_station.fraction() {
                ws.write_number_with_format(row, 8, fraction, &percent)?;
            }
            row += 1;
        }
    }

    // Reconciliation remainder: matrix totals + these = all identifiable
    // presentations (see `completed_work_report`).
    row += 1;
    ws.write_string_with_format(row, 0, "Unattributed", &b)?;
    row += 1;
    for (column, header) in
        ["Scope", "Start hour", "End hour", "Aisle-unattributed Presentations", "Unidentified Pick Lines"]
            .iter()
            .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for value in &report.data.unattributed_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_number(row, 1, value.bucket.start_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 2, value.bucket.end_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 3, value.aisle_unattributed_presentations as f64)?;
            ws.write_number(row, 4, value.unidentified_pick_lines as f64)?;
            row += 1;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_aisle_station_matrix(
    ws: &mut Worksheet,
    start_row: u32,
    title: &str,
    report: &MetricsReport<CompletedWorkData>,
    share: bool,
    b: &Format,
    percent: &Format,
) -> Result<u32, XlsxError> {
    let zones: Vec<_> =
        report.data.aisle_station_rows.iter().map(|row| row.zone_id).collect::<BTreeSet<_>>().into_iter().collect();
    let stations: Vec<_> =
        report.data.aisle_station_rows.iter().map(|row| row.station_id).collect::<BTreeSet<_>>().into_iter().collect();
    ws.write_string_with_format(start_row, 0, title, b)?;
    let header_row = start_row + 1;
    ws.write_string_with_format(header_row, 0, "Aisle", b)?;
    for (index, station_id) in stations.iter().enumerate() {
        ws.write_string_with_format(header_row, index as u16 + 1, format!("Station {}", station_id.0), b)?;
    }
    for (zone_index, zone_id) in zones.iter().enumerate() {
        let row = header_row + zone_index as u32 + 1;
        ws.write_number(row, 0, *zone_id as f64)?;
        for (station_index, station_id) in stations.iter().enumerate() {
            if let Some(value) = report
                .data
                .aisle_station_rows
                .iter()
                .find(|candidate| candidate.zone_id == *zone_id && candidate.station_id == *station_id)
            {
                if share {
                    if let Some(fraction) = value.aisle_share_of_station.fraction() {
                        ws.write_number_with_format(row, station_index as u16 + 1, fraction, percent)?;
                    }
                } else {
                    ws.write_number(row, station_index as u16 + 1, value.tote_count as f64)?;
                }
            }
        }
    }
    add_matrix_heatmap(ws, header_row, zones.len(), stations.len())?;
    insert_work_matrix_chart(ws, "Aisle x Station", title, header_row, zones.len(), stations.len(), share)?;
    Ok(start_row + (zones.len() as u32 + 3).max(17))
}

#[derive(Clone, Copy)]
enum DcMatrixKind {
    ToteCount,
    ToteShare,
    OrderCount,
    OrderShare,
}

pub(super) fn write_dc_station_sheet(
    wb: &mut Workbook,
    full: &MetricsReport<CompletedWorkData>,
    post_embargo: &MetricsReport<CompletedWorkData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("DC x Station")?;
    let b = bold();
    let percent = Format::new().set_num_format("0.00%");
    if full.data.dc_station_rows.is_empty() && post_embargo.data.dc_station_rows.is_empty() {
        ws.write_string_with_format(0, 0, "DC x Station", &b)?;
        ws.write_string(1, 0, "No completed tote or order work in this run carries DC attribution.")?;
        // The unattributed counters still exist per bucket; state them here
        // rather than pointing at a table that does not carry them.
        let header_row = 3;
        for (column, header) in [
            "Scope",
            "Bucket start hour",
            "Bucket end hour",
            "DC-unattributed presentations",
            "Unattributed orders",
            "Unidentified pick lines",
        ]
        .iter()
        .enumerate()
        {
            ws.write_string_with_format(header_row, column as u16, *header, &b)?;
        }
        let hour = hour_fmt();
        let mut row = header_row + 1;
        for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
            for value in &report.data.unattributed_rows {
                ws.write_string(row, 0, scope)?;
                ws.write_number_with_format(row, 1, value.bucket.start_ms as f64 / 3_600_000.0, &hour)?;
                ws.write_number_with_format(row, 2, value.bucket.end_ms as f64 / 3_600_000.0, &hour)?;
                ws.write_number(row, 3, value.dc_unattributed_presentations as f64)?;
                ws.write_number(row, 4, value.unattributed_orders as f64)?;
                ws.write_number(row, 5, value.unidentified_pick_lines as f64)?;
                row += 1;
            }
        }
        ws.set_column_range_width(0, 5, 28)?;
        return Ok(());
    }
    let mut row = 0;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for (label, kind) in [
            ("completed totes", DcMatrixKind::ToteCount),
            ("tote station share per DC", DcMatrixKind::ToteShare),
            ("completed orders", DcMatrixKind::OrderCount),
            ("order station share per DC", DcMatrixKind::OrderShare),
        ] {
            row = write_dc_station_matrix(ws, row, &format!("{scope} — {label}"), report, kind, &b, &percent)?;
        }
    }

    ws.write_string_with_format(row, 0, "Normalized data", &b)?;
    row += 1;
    for (column, header) in [
        "Scope",
        "Start hour",
        "End hour",
        "DC",
        "Pick Station",
        "Tote Count",
        "Tote Share Numerator",
        "Tote Share Denominator",
        "Tote Station Share",
        "Order Count",
        "Order Share Numerator",
        "Order Share Denominator",
        "Order Station Share",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for value in &report.data.dc_station_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_number(row, 1, value.bucket.start_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 2, value.bucket.end_ms as f64 / 3_600_000.0)?;
            ws.write_string(row, 3, &value.dc_id)?;
            ws.write_number(row, 4, value.station_id.0 as f64)?;
            ws.write_number(row, 5, value.tote_count as f64)?;
            ws.write_number(row, 6, value.tote_station_share_of_dc.numerator as f64)?;
            ws.write_number(row, 7, value.tote_station_share_of_dc.denominator as f64)?;
            if let Some(fraction) = value.tote_station_share_of_dc.fraction() {
                ws.write_number_with_format(row, 8, fraction, &percent)?;
            }
            ws.write_number(row, 9, value.order_count as f64)?;
            ws.write_number(row, 10, value.order_station_share_of_dc.numerator as f64)?;
            ws.write_number(row, 11, value.order_station_share_of_dc.denominator as f64)?;
            if let Some(fraction) = value.order_station_share_of_dc.fraction() {
                ws.write_number_with_format(row, 12, fraction, &percent)?;
            }
            row += 1;
        }
    }

    // Reconciliation remainder: matrix totals + these = all identifiable
    // presentations / distinct orders (see `completed_work_report`).
    row += 1;
    ws.write_string_with_format(row, 0, "Unattributed", &b)?;
    row += 1;
    for (column, header) in [
        "Scope",
        "Start hour",
        "End hour",
        "DC-unattributed Presentations",
        "Unidentified Pick Lines",
        "Unattributed Orders",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for value in &report.data.unattributed_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_number(row, 1, value.bucket.start_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 2, value.bucket.end_ms as f64 / 3_600_000.0)?;
            ws.write_number(row, 3, value.dc_unattributed_presentations as f64)?;
            ws.write_number(row, 4, value.unidentified_pick_lines as f64)?;
            ws.write_number(row, 5, value.unattributed_orders as f64)?;
            row += 1;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_dc_station_matrix(
    ws: &mut Worksheet,
    start_row: u32,
    title: &str,
    report: &MetricsReport<CompletedWorkData>,
    kind: DcMatrixKind,
    b: &Format,
    percent: &Format,
) -> Result<u32, XlsxError> {
    let dcs: Vec<_> =
        report.data.dc_station_rows.iter().map(|row| row.dc_id.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let stations: Vec<_> =
        report.data.dc_station_rows.iter().map(|row| row.station_id).collect::<BTreeSet<_>>().into_iter().collect();
    ws.write_string_with_format(start_row, 0, title, b)?;
    let header_row = start_row + 1;
    ws.write_string_with_format(header_row, 0, "DC", b)?;
    for (index, station_id) in stations.iter().enumerate() {
        ws.write_string_with_format(header_row, index as u16 + 1, format!("Station {}", station_id.0), b)?;
    }
    for (dc_index, dc_id) in dcs.iter().enumerate() {
        let row = header_row + dc_index as u32 + 1;
        ws.write_string(row, 0, dc_id)?;
        for (station_index, station_id) in stations.iter().enumerate() {
            if let Some(value) = report
                .data
                .dc_station_rows
                .iter()
                .find(|candidate| candidate.dc_id == *dc_id && candidate.station_id == *station_id)
            {
                let column = station_index as u16 + 1;
                match kind {
                    DcMatrixKind::ToteCount => {
                        ws.write_number(row, column, value.tote_count as f64)?;
                    }
                    DcMatrixKind::OrderCount => {
                        ws.write_number(row, column, value.order_count as f64)?;
                    }
                    DcMatrixKind::ToteShare => {
                        if let Some(fraction) = value.tote_station_share_of_dc.fraction() {
                            ws.write_number_with_format(row, column, fraction, percent)?;
                        }
                    }
                    DcMatrixKind::OrderShare => {
                        if let Some(fraction) = value.order_station_share_of_dc.fraction() {
                            ws.write_number_with_format(row, column, fraction, percent)?;
                        }
                    }
                }
            }
        }
    }
    add_matrix_heatmap(ws, header_row, dcs.len(), stations.len())?;
    let share = matches!(kind, DcMatrixKind::ToteShare | DcMatrixKind::OrderShare);
    insert_work_matrix_chart(ws, "DC x Station", title, header_row, dcs.len(), stations.len(), share)?;
    Ok(start_row + (dcs.len() as u32 + 3).max(17))
}

/// Two-color heatmap over a matrix's value cells so hot cells read at a
/// glance; display only, values unchanged.
fn add_matrix_heatmap(
    ws: &mut Worksheet,
    header_row: u32,
    row_count: usize,
    column_count: usize,
) -> Result<(), XlsxError> {
    if row_count == 0 || column_count == 0 {
        return Ok(());
    }
    let heatmap = ConditionalFormat2ColorScale::new().set_minimum_color("F8FAFC").set_maximum_color("2563EB");
    ws.add_conditional_format(header_row + 1, 1, header_row + row_count as u32, column_count as u16, &heatmap)?;
    Ok(())
}

fn insert_work_matrix_chart(
    ws: &mut Worksheet,
    sheet_name: &str,
    title: &str,
    header_row: u32,
    category_count: usize,
    series_count: usize,
    percentage: bool,
) -> Result<(), XlsxError> {
    if category_count == 0 || series_count == 0 {
        return Ok(());
    }
    let first_row = header_row + 1;
    let last_row = first_row + category_count as u32 - 1;
    let mut chart = Chart::new(if percentage { ChartType::Column } else { ChartType::ColumnStacked });
    for column in 1..=series_count as u16 {
        chart
            .add_series()
            .set_categories((sheet_name, first_row, 0, last_row, 0))
            .set_values((sheet_name, first_row, column, last_row, column))
            .set_name((sheet_name, header_row, column));
    }
    chart.title().set_name(title);
    chart.x_axis().set_name(if sheet_name == "Aisle x Station" { "Aisle" } else { "DC" });
    chart.y_axis().set_name(if percentage { "Share (%)" } else { "Count" });
    if percentage {
        chart.y_axis().set_num_format("0%");
    }
    chart.set_style(10);
    ws.insert_chart(header_row - 1, series_count as u16 + 2, &chart)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use desgraph::{RelayId, StationId};
    use dessim::metrics::reporting::{MetricScope, ReportPeriod, ReportRequest, completed_work_report};
    use dessim::model::OrderId;
    use dessim::model::ids::SkuId;
    use dessim::observer::output::{CompletedOrderRecord, PickRecord};
    use rust_xlsxwriter::Workbook;
    use zip::ZipArchive;

    use super::{write_aisle_station_sheet, write_completed_tote_work_sheet, write_dc_station_sheet};
    use crate::tests::{column_pairs, sheet_xml};

    #[test]
    fn dc_station_sheet_states_absent_dc_attribution_plainly() {
        let picks = [PickRecord {
            sim_time_ms: 1_000,
            station_id: StationId(7),
            sku: SkuId(1),
            quantity: 25,
            dc_id: None,
            order_id: Some(OrderId(1)),
            tote_id: Some(10),
            source_zone_id: Some(3),
            source_relay_id: Some(RelayId(4)),
            presentation_id: Some(100),
        }];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 4_000, 0);
        let full = completed_work_report(&picks, &[], request).unwrap();
        assert!(full.data.dc_station_rows.is_empty(), "no DC-attributed work");

        let mut workbook = Workbook::new();
        write_dc_station_sheet(&mut workbook, &full, &full).unwrap();
        let buffer = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buffer)).unwrap();
        let mut shared_strings = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared_strings).unwrap();
        assert!(shared_strings.contains("carries DC attribution"));
    }

    #[test]
    fn completed_work_sheets_use_canonical_tote_and_order_matrices() {
        let picks = [
            PickRecord {
                sim_time_ms: 1_000,
                station_id: StationId(7),
                sku: SkuId(1),
                quantity: 25,
                dc_id: Some("DC-A".into()),
                order_id: Some(OrderId(1)),
                tote_id: Some(10),
                source_zone_id: Some(3),
                source_relay_id: Some(RelayId(4)),
                presentation_id: Some(100),
            },
            PickRecord {
                sim_time_ms: 2_000,
                station_id: StationId(8),
                sku: SkuId(2),
                quantity: 1,
                dc_id: Some("DC-A".into()),
                order_id: Some(OrderId(1)),
                tote_id: Some(11),
                source_zone_id: Some(4),
                source_relay_id: Some(RelayId(5)),
                presentation_id: Some(101),
            },
        ];
        let orders = [CompletedOrderRecord {
            sim_time_ms: 3_000,
            order_id: OrderId(1),
            station_id: Some(StationId(8)),
            dc_id: Some("DC-A".into()),
        }];
        let full = completed_work_report(
            &picks,
            &orders,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 4_000, 0),
        )
        .unwrap();
        let post = completed_work_report(
            &picks,
            &orders,
            ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 4_000, 500),
        )
        .unwrap();

        let mut workbook = Workbook::new();
        write_completed_tote_work_sheet(&mut workbook, &picks, &orders).unwrap();
        write_aisle_station_sheet(&mut workbook, &full, &post).unwrap();
        write_dc_station_sheet(&mut workbook, &full, &post).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing
        // left behind on disk when an assertion below fails.
        let buf = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"Completed Tote Work\""));
        assert!(workbook_xml.contains("name=\"Aisle x Station\""));
        assert!(workbook_xml.contains("name=\"DC x Station\""));

        let mut chart_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut chart_xml).unwrap();
        assert!(chart_xml.contains("<c:barChart>"));
        assert!(chart_xml.contains("'Aisle x Station'!$A$3:$A$4"));
        assert!(chart_xml.contains("'Aisle x Station'!$B$3:$B$4"));

        // The reconciliation remainder is surfaced on both matrix sheets.
        let mut shared_strings = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared_strings).unwrap();
        assert!(shared_strings.contains("Aisle-unattributed Presentations"));
        assert!(shared_strings.contains("DC-unattributed Presentations"));
        assert!(shared_strings.contains("Unattributed Orders"));
        assert!(shared_strings.contains("Unidentified Pick Lines"));

        drop(archive);
    }

    /// Both of the sheet's lists read in instant order, rows sharing an instant
    /// in the order the run recorded them: a recording states a row in the
    /// frame written once B has passed it, so its rows reach the book in
    /// another order than the run appended them.
    #[test]
    fn the_completed_tote_work_sheet_reads_in_instant_order() {
        let pick = |sim_time_ms, quantity| PickRecord {
            sim_time_ms,
            station_id: StationId(7),
            sku: SkuId(1),
            quantity,
            dc_id: None,
            order_id: None,
            tote_id: None,
            source_zone_id: None,
            source_relay_id: None,
            presentation_id: None,
        };
        let order = |sim_time_ms, id| CompletedOrderRecord {
            sim_time_ms,
            order_id: OrderId(id),
            station_id: None,
            dc_id: None,
        };
        let picks = [pick(3_000, 1), pick(1_000, 2), pick(3_000, 3)];
        let orders = [order(3_000, 1), order(1_000, 2), order(3_000, 3)];
        let mut workbook = Workbook::new();
        write_completed_tote_work_sheet(&mut workbook, &picks, &orders).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        let expected = [["1000", "2"], ["3000", "1"], ["3000", "3"]];
        // The picks under the header, told apart by their quantity; then,
        // under a title and a header of their own, the orders.
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'K'), expected, "the picks");
        assert_eq!(column_pairs(&sheet, 9..=11, 'A', 'C'), expected, "the fully picked orders");
    }
}
