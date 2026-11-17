//! Within `workbook`, `extraction` renders the order-extraction sheets: the
//! extraction reports and their descriptor-backed completion journal. Counts,
//! station grouping, temporal boundaries and rates come from the shared
//! producer; this module owns spreadsheet layout and missing-data presentation.

use dessim::metrics::reporting::{ExtractionData, MetricsReport};
use dessim::metrics::streams::{EntryValue, EntryValueType, LogStream};
use rust_xlsxwriter::{Workbook, XlsxError};

use super::{bold, hour_fmt, line_chart_for_columns, support_status_label};

/// The two sheets together: the report table above, the journal it is built
/// from beside it. Two sheets and not one because the table is one row per
/// scope, period and station while the journal is one row per completion.
pub(super) fn write_extraction_sheets(
    wb: &mut Workbook,
    full: &MetricsReport<ExtractionData>,
    post_embargo: &MetricsReport<ExtractionData>,
    hourly: &MetricsReport<ExtractionData>,
    streams: &[LogStream],
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Order Extraction")?;
    let b = bold();
    ws.write_string_with_format(0, 0, "Completed physical extraction work", &b)?;
    ws.write_string(1, 0, "Completion is the end of extraction dwell; fully picked orders are a separate milestone.")?;
    for (column, header) in [
        "Scope",
        "Period",
        "Start hour",
        "End hour",
        "Extraction station",
        "Status",
        "Completed extractions",
        "Extracted units",
        "Extractions/hr",
        "Extracted units/hr",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(3, column as u16, *header, &b)?;
    }
    // Three scopes down one table rather than three tables: a reader compares
    // the same station's full-run, post-embargo and hourly rows by filtering
    // one column, and the Status column says which of them the run supports.
    let mut row = 4;
    for (scope, period, report) in
        [("Full run", "Overall", full), ("Post-embargo", "Overall", post_embargo), ("Full run", "Hourly", hourly)]
    {
        for value in &report.data.rows {
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, period)?;
            ws.write_number_with_format(row, 2, value.bucket.start_ms as f64 / 3_600_000.0, &hour_fmt())?;
            ws.write_number_with_format(row, 3, value.bucket.end_ms as f64 / 3_600_000.0, &hour_fmt())?;
            if let Some(station) = value.station_id {
                ws.write_number(row, 4, station.0 as f64)?;
            } else {
                ws.write_string(row, 4, "All extraction stations")?;
            }
            ws.write_string(row, 5, support_status_label(value.status))?;
            // A count the run cannot state leaves its cell EMPTY rather than
            // writing a zero: zero extractions and no way to know are
            // different answers, and a chart over the column must not plot
            // the second as the first.
            for (column, count) in [(6, value.extractions), (7, value.extracted_units)] {
                if let Some(count) = count {
                    ws.write_number(row, column, count as f64)?;
                }
            }
            for (column, rate) in [(8, value.extractions_per_hour()), (9, value.extracted_units_per_hour())] {
                if let Some(rate) = rate {
                    ws.write_number(row, column, rate)?;
                }
            }
            row += 1;
        }
    }

    // The chart reads its own small table below the report rather than the
    // report itself: it plots the hourly fleet-wide rates, and the report's
    // rows interleave scopes and stations that would each become a series.
    row += 2;
    let header = row;
    for (column, label) in ["End hour", "Extractions/hr", "Extracted units/hr"].iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *label, &b)?;
    }
    row += 1;
    for value in hourly.data.rows.iter().filter(|value| value.station_id.is_none() && value.extractions.is_some()) {
        ws.write_number_with_format(row, 0, value.bucket.end_ms as f64 / 3_600_000.0, &hour_fmt())?;
        for (column, rate) in [(1, value.extractions_per_hour()), (2, value.extracted_units_per_hour())] {
            if let Some(rate) = rate {
                ws.write_number(row, column, rate)?;
            }
        }
        row += 1;
    }
    // No rows, no chart: an empty chart claims a measured zero.
    if row > header + 1 {
        for (column, title, offset) in [(1, "Completed extractions/hr", 0), (2, "Extracted units/hr", 16)] {
            if let Some(chart) = line_chart_for_columns(
                "Order Extraction",
                title,
                title,
                header + 1,
                row - 1,
                0,
                &[column],
                "Simulation time (h)",
            ) {
                ws.insert_chart(header + offset, 4, &chart)?;
            }
        }
    }
    ws.set_column_range_width(0, 9, 22)?;
    ws.set_freeze_panes(4, 0)?;

    write_completion_journal(wb, streams.iter().find(|stream| stream.descriptor.id == "fulfillment.extractions"))
}

fn write_completion_journal(wb: &mut Workbook, stream: Option<&LogStream>) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Extraction Operations")?;
    // The sheet exists either way, saying which case it is: a missing sheet
    // reads as a workbook built by an older binary, an empty one as a run that
    // extracted nothing.
    let Some(stream) = stream else {
        ws.write_string(0, 0, "Unavailable: this profile did not record completed extraction work.")?;
        return Ok(());
    };
    ws.write_string_with_format(0, 0, &stream.descriptor.label, &bold())?;
    ws.write_string(1, 0, &stream.descriptor.description)?;
    // Header from the stream's own descriptor, not an array here: this journal
    // is self-describing, so a column the producer adds shows up with its name
    // and its description without this file being touched (issue #300 moves
    // the other raw dumps to the same rule).
    for (column, field) in stream.descriptor.columns.iter().enumerate() {
        let mut label = field.name.replace('_', " ");
        if matches!(field.value_type, EntryValueType::SimTime | EntryValueType::SimDuration) {
            label.push_str(" (ms)");
        }
        ws.write_string_with_format(3, column as u16, label, &bold())?;
        ws.write_string(4, column as u16, &field.description)?;
    }
    let rows = stream.entries.generic_rows().map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    for (index, values) in rows.iter().enumerate() {
        let row = index as u32 + 5;
        for (column, value) in values.iter().enumerate() {
            let column = column as u16;
            match value {
                EntryValue::UInt(value) => {
                    ws.write_number(row, column, *value as f64)?;
                }
                EntryValue::Int(value) => {
                    ws.write_number(row, column, *value as f64)?;
                }
                EntryValue::Float(value) => {
                    ws.write_number(row, column, *value)?;
                }
                EntryValue::Bool(value) => {
                    ws.write_boolean(row, column, *value)?;
                }
                // Empty text is the journals' way of saying the producer
                // recorded none, so the cell stays empty rather than holding
                // an empty string a filter would count as a value.
                EntryValue::Text(value) if !value.is_empty() => {
                    ws.write_string(row, column, value)?;
                }
                EntryValue::Text(_) => {}
            }
        }
    }
    ws.set_column_range_width(0, stream.descriptor.columns.len().saturating_sub(1) as u16, 24)?;
    ws.set_freeze_panes(5, 0)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use desgraph::StationId;
    use dessim::metrics::reporting::{MetricScope, ReportPeriod, ReportRequest, extraction_report, extraction_stream};
    use dessim::model::RobotId;
    use dessim::observer::output::ExtractionRecord;
    use zip::ZipArchive;

    use super::*;

    fn read_xml(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> String {
        let mut xml = String::new();
        archive.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
        xml
    }

    fn number(xml: &str, cell: &str) -> Option<f64> {
        xml.split_once(&format!("r=\"{cell}\""))?.1.split_once("<v>")?.1.split_once("</v>")?.0.parse().ok()
    }

    #[test]
    fn extraction_workbook_uses_shared_totals_rates_and_physical_completion_journal() {
        let records = [
            ExtractionRecord {
                sim_time_ms: 4_000,
                station_id: StationId(17),
                robot_id: RobotId(3),
                quantity: 2,
                presentation_id: Some(11),
            },
            ExtractionRecord {
                sim_time_ms: 9_000,
                station_id: StationId(18),
                robot_id: RobotId(4),
                quantity: 5,
                presentation_id: None,
            },
        ];
        let streams = [extraction_stream(&records).unwrap()];
        let report =
            extraction_report(&streams, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10_000, 0))
                .unwrap();
        let total = report.data.rows.iter().find(|row| row.station_id.is_none()).unwrap();
        let mut wb = Workbook::new();
        write_extraction_sheets(&mut wb, &report, &report, &report, &streams).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(wb.save_to_buffer().unwrap())).unwrap();
        let sheet = read_xml(&mut archive, "xl/worksheets/sheet1.xml");
        assert_eq!(number(&sheet, "G5"), Some(2.0));
        assert_eq!(number(&sheet, "H5"), Some(7.0));
        assert_eq!(number(&sheet, "I5"), total.extractions_per_hour());
        assert_eq!(number(&sheet, "J5"), total.extracted_units_per_hour());
        assert_eq!(number(&sheet, "E6"), Some(17.0));
        assert_eq!(number(&sheet, "H6"), Some(2.0));
        assert_eq!(number(&sheet, "E7"), Some(18.0));
        assert_eq!(number(&sheet, "H7"), Some(5.0));
        let journal = read_xml(&mut archive, "xl/worksheets/sheet2.xml");
        assert_eq!(number(&journal, "A6"), Some(4_000.0));
        assert_eq!(number(&journal, "A7"), Some(9_000.0));
        assert_eq!(number(&journal, "B6"), Some(17.0));
        assert_eq!(number(&journal, "D7"), Some(5.0));
        assert!(!journal.contains("r=\"E7\""), "unknown trip identity is blank, not zero");
        assert!(archive.by_name("xl/charts/chart1.xml").is_ok());
    }

    #[test]
    fn legacy_extraction_report_is_blank_but_an_empty_measured_stream_is_zero() {
        for (streams, measured) in [(vec![], false), (vec![extraction_stream(&[]).unwrap()], true)] {
            let report = extraction_report(
                &streams,
                ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10_000, 0),
            )
            .unwrap();
            let mut wb = Workbook::new();
            write_extraction_sheets(&mut wb, &report, &report, &report, &streams).unwrap();
            let mut archive = ZipArchive::new(Cursor::new(wb.save_to_buffer().unwrap())).unwrap();
            let sheet = read_xml(&mut archive, "xl/worksheets/sheet1.xml");
            if measured {
                assert_eq!(number(&sheet, "G5"), Some(0.0));
                assert_eq!(number(&sheet, "H5"), Some(0.0));
            } else {
                assert!(!sheet.contains("r=\"G5\""));
                assert!(!sheet.contains("r=\"H5\""));
                let shared = read_xml(&mut archive, "xl/sharedStrings.xml");
                assert!(shared.contains("Unavailable: this profile did not record completed extraction work."));
            }
        }
    }
}
