//! Within `workbook`, `d30` renders the D30 (HTM) report sheets: the
//! canonical D30 State sheet (the flat utilization split, waiting
//! attribution, travel distance, and per-D30 drilldown tables, with native
//! charts) and the D30 Distance sheet. Report values and their rendered
//! names come from `dessim::metrics::reporting::d30_state`; this module owns
//! only layout. The raw journal dump stays on the family-neutral "HTM
//! Journey Legs" sheet in the crate root.

use std::collections::BTreeSet;

use dessim::metrics::reporting::{
    D30_STATES, D30_TRAVEL_KINDS, D30_WAIT_CATEGORIES, D30State, D30StateData, D30StateDurationRow, D30WaitCategory,
    MetricsReport, Ratio, SupportStatus, TimeBucket,
};
use rust_xlsxwriter::{Chart, ChartType, Format, Workbook, Worksheet, XlsxError};

use super::{bold, bucket_completeness, bucket_hour, completeness_fmt, metric_label, support_status_label};

const HOUR_MS: u64 = 3_600_000;

pub(super) fn write_d30_state_sheet(
    wb: &mut Workbook,
    full_state: &MetricsReport<D30StateData>,
    post_embargo_state: &MetricsReport<D30StateData>,
    hourly_state: &MetricsReport<D30StateData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("D30 State")?;
    let b = bold();
    let percent = Format::new().set_num_format("0.00%");

    let flat_rows = d30_flat_rows();
    let first_charted =
        flat_rows.iter().position(|row| d30_flat_row_is_charted(*row)).expect("the D30 flat list has charted rows")
            as u32;
    let last_flat_row = flat_rows.len() as u32 - 1;

    let full_first_row = write_d30_overall_state_section(ws, 0, "Full run", full_state, &b, &percent)?;
    if let Some(bucket) = first_state_bucket(&full_state.data) {
        let mut chart = Chart::new(ChartType::Pie);
        chart
            .add_series()
            .set_categories(("D30 State", full_first_row + first_charted, 0, full_first_row + last_flat_row, 0))
            .set_values(("D30 State", full_first_row + first_charted, 2, full_first_row + last_flat_row, 2));
        chart.title().set_name(&format!("D30 state — full run ({:.2} h)", bucket.coverage_ms as f64 / 3_600_000.0));
        chart.set_style(10);
        ws.insert_chart(0, 7, &chart)?;
    }

    // Match the panel's distinction between operator work and physical cell occupancy.
    ws.write_string(
        23,
        0,
        "Station processing = the recorded operator dwell at the presentation cell only. A \
         pre-dwell station hold is Waiting (Presenting or In Queue (station)). Post-dwell \
         presentation occupancy can include departure clearance or downstream blocking; it is not operator work.",
    )?;

    let post_section_row = 25;
    let post_first_row =
        write_d30_overall_state_section(ws, post_section_row, "Post-embargo", post_embargo_state, &b, &percent)?;
    if first_state_bucket(&post_embargo_state.data).is_some() {
        let mut chart = Chart::new(ChartType::Pie);
        chart
            .add_series()
            .set_categories(("D30 State", post_first_row + first_charted, 0, post_first_row + last_flat_row, 0))
            .set_values(("D30 State", post_first_row + first_charted, 2, post_first_row + last_flat_row, 2));
        chart.title().set_name("D30 state — post-embargo");
        chart.set_style(10);
        ws.insert_chart(post_section_row, 7, &chart)?;
    }

    let hourly_title_row = 49;
    ws.write_string_with_format(hourly_title_row, 0, "Hourly D30 state", &b)?;
    let hourly_header_row = hourly_title_row + 1;
    ws.write_string_with_format(hourly_header_row, 0, "Hour", &b)?;
    ws.write_string_with_format(hourly_header_row, 1, "Hour completeness", &b)?;
    ws.write_string_with_format(hourly_header_row, 2, "Fleet time (ms)", &b)?;
    for (index, flat_row) in flat_rows.iter().enumerate() {
        let duration_column = 3 + index as u16 * 2;
        ws.write_string_with_format(
            hourly_header_row,
            duration_column,
            format!("{} (ms)", d30_flat_row_label(*flat_row)),
            &b,
        )?;
        ws.write_string_with_format(
            hourly_header_row,
            duration_column + 1,
            format!("{} (% fleet)", d30_flat_row_label(*flat_row)),
            &b,
        )?;
    }
    let hourly_buckets = state_report_buckets(&hourly_state.data);
    for (index, bucket) in hourly_buckets.iter().enumerate() {
        let row = hourly_header_row + 1 + index as u32;
        let fleet_time_ms = bucket.coverage_ms * hourly_state.data.fleet_size as u64;
        ws.write_number(row, 0, bucket_hour(bucket, HOUR_MS))?;
        ws.write_number_with_format(row, 1, bucket_completeness(bucket, HOUR_MS), &completeness_fmt())?;
        ws.write_number(row, 2, fleet_time_ms as f64)?;
        for (flat_index, flat_row) in flat_rows.iter().enumerate() {
            let duration_column = 3 + flat_index as u16 * 2;
            let (_, duration_ms, fleet_share) = d30_flat_row_values(&hourly_state.data, *bucket, *flat_row);
            if let Some(duration_ms) = duration_ms {
                ws.write_number(row, duration_column, duration_ms as f64)?;
            }
            if let Some(fraction) = fleet_share.and_then(|ratio| ratio.fraction()) {
                ws.write_number_with_format(row, duration_column + 1, fraction, &percent)?;
            }
        }
    }

    if !hourly_buckets.is_empty() {
        let first_data_row = hourly_header_row + 1;
        let last_data_row = first_data_row + hourly_buckets.len() as u32 - 1;
        let mut chart = Chart::new(ChartType::ColumnPercentStacked);
        for (flat_index, _) in flat_rows.iter().enumerate().filter(|(_, flat_row)| d30_flat_row_is_charted(**flat_row))
        {
            let duration_column = 3 + flat_index as u16 * 2;
            chart
                .add_series()
                .set_categories(("D30 State", first_data_row, 0, last_data_row, 0))
                .set_values(("D30 State", first_data_row, duration_column, last_data_row, duration_column))
                .set_name(("D30 State", hourly_header_row, duration_column));
        }
        chart.title().set_name(&format!("{} by hour", metric_label(&hourly_state.descriptors, "d30.state.duration")));
        chart.x_axis().set_name("Hour");
        chart.x_axis().set_num_format("0");
        chart.y_axis().set_name("Fleet time (%)");
        chart.set_style(10);
        ws.insert_chart(hourly_header_row, 41, &chart)?;
    }

    let detail_row = hourly_header_row + hourly_buckets.len() as u32 + 3;
    let detail_end_row =
        write_d30_wait_and_distance_tables(ws, detail_row, full_state, post_embargo_state, &b, &percent)?;
    write_d30_vehicle_state_tables(ws, detail_end_row + 2, full_state, post_embargo_state, hourly_state, &b, &percent)?;
    ws.set_column_width(0, 24)?;
    ws.set_column_width(1, 30)?;
    // The per-D30 tables repeat the state/category labels in column E.
    ws.set_column_width(4, 30)?;
    ws.set_freeze_panes(hourly_header_row + 1, 0)?;
    Ok(())
}

fn write_d30_overall_state_section(
    ws: &mut Worksheet,
    section_row: u32,
    scope_label: &str,
    report: &MetricsReport<D30StateData>,
    b: &Format,
    percent: &Format,
) -> Result<u32, XlsxError> {
    ws.write_string_with_format(section_row, 0, scope_label, b)?;
    let header_row = section_row + 1;
    for (column, header) in
        ["State", "Status", "Duration (ms)", "Duration (h)", "Fleet time (ms)", "% fleet time"].iter().enumerate()
    {
        ws.write_string_with_format(header_row, column as u16, *header, b)?;
    }
    let first_data_row = header_row + 1;
    let bucket = first_state_bucket(&report.data);
    for (index, flat_row) in d30_flat_rows().into_iter().enumerate() {
        let row = first_data_row + index as u32;
        ws.write_string(row, 0, d30_flat_row_label(flat_row))?;
        let Some(bucket) = bucket else {
            continue;
        };
        let (status, duration_ms, fleet_share) = d30_flat_row_values(&report.data, bucket, flat_row);
        ws.write_string(row, 1, support_status_label(status))?;
        if let Some(duration_ms) = duration_ms {
            ws.write_number(row, 2, duration_ms as f64)?;
            ws.write_number(row, 3, duration_ms as f64 / 3_600_000.0)?;
        }
        if let Some(ratio) = fleet_share {
            ws.write_number(row, 4, ratio.denominator as f64)?;
            if let Some(fraction) = ratio.fraction() {
                ws.write_number_with_format(row, 5, fraction, percent)?;
            }
        }
    }
    Ok(first_data_row)
}

fn write_d30_wait_and_distance_tables(
    ws: &mut Worksheet,
    start_row: u32,
    full_state: &MetricsReport<D30StateData>,
    post_embargo_state: &MetricsReport<D30StateData>,
    b: &Format,
    percent: &Format,
) -> Result<u32, XlsxError> {
    ws.write_string_with_format(start_row, 0, "Selected-plan waiting attribution", b)?;
    let header_row = start_row + 1;
    for (column, header) in
        ["Scope", "Category", "Status", "Duration (ms)", "Waiting time (ms)", "% waiting time"].iter().enumerate()
    {
        ws.write_string_with_format(header_row, column as u16, *header, b)?;
    }
    let mut row = header_row + 1;
    for (scope, report) in [("Full run", full_state), ("Post-embargo", post_embargo_state)] {
        let Some(bucket) = first_state_bucket(&report.data) else {
            continue;
        };
        for category in D30_WAIT_CATEGORIES {
            let wait_row = report
                .data
                .wait_category_rows
                .iter()
                .find(|candidate| candidate.bucket == bucket && candidate.category == category);
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, category.label())?;
            ws.write_string(
                row,
                2,
                support_status_label(wait_row.expect("state builder emits every wait-category row per bucket").status),
            )?;
            if let Some(duration_ms) = wait_row.and_then(|row| row.duration_ms) {
                ws.write_number(row, 3, duration_ms as f64)?;
            }
            if let Some(ratio) = wait_row.and_then(|row| row.waiting_share) {
                ws.write_number(row, 4, ratio.denominator as f64)?;
                if let Some(fraction) = ratio.fraction() {
                    ws.write_number_with_format(row, 5, fraction, percent)?;
                }
            }
            row += 1;
        }
    }

    row += 1;
    ws.write_string_with_format(row, 0, "Travel distance", b)?;
    row += 1;
    for (column, header) in ["Scope", "Kind", "Status", "Distance (mm)", "Distance (m)"].iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *header, b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full_state), ("Post-embargo", post_embargo_state)] {
        let Some(bucket) = first_state_bucket(&report.data) else {
            continue;
        };
        for kind in D30_TRAVEL_KINDS {
            let distance_row =
                report.data.distance_rows.iter().find(|candidate| candidate.bucket == bucket && candidate.kind == kind);
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, kind.label())?;
            ws.write_string(
                row,
                2,
                support_status_label(distance_row.expect("state builder emits every distance row per bucket").status),
            )?;
            if let Some(distance_mm) = distance_row.and_then(|row| row.distance_mm) {
                ws.write_number(row, 3, distance_mm as f64)?;
                ws.write_number(row, 4, distance_mm as f64 / 1_000.0)?;
            }
            row += 1;
        }
    }
    Ok(row)
}

/// Per-D30 drilldown tables appended to the D30 State sheet: each observed
/// vehicle's state partition and waiting attribution per scope and bucket.
/// Values come from the builder's vehicle rows; fleet rows reconcile to these
/// plus the unobserved-vehicle backfill.
fn write_d30_vehicle_state_tables(
    ws: &mut Worksheet,
    start_row: u32,
    full: &MetricsReport<D30StateData>,
    post_embargo: &MetricsReport<D30StateData>,
    hourly: &MetricsReport<D30StateData>,
    b: &Format,
    percent: &Format,
) -> Result<(), XlsxError> {
    ws.write_string_with_format(start_row, 0, "Per-D30 state data", b)?;
    let header_row = start_row + 1;
    for (column, header) in [
        "Scope",
        "Hour",
        "Hour completeness",
        "D30",
        "State",
        "Status",
        "Duration (ms)",
        "D30 included time (ms)",
        "% D30 time",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(header_row, column as u16, *header, b)?;
    }
    let mut row = header_row + 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo), ("Hourly", hourly)] {
        for value in &report.data.vehicle_state_rows {
            ws.write_string(row, 0, scope)?;
            if scope == "Hourly" {
                ws.write_number(row, 1, bucket_hour(&value.bucket, HOUR_MS))?;
                ws.write_number_with_format(row, 2, bucket_completeness(&value.bucket, HOUR_MS), &completeness_fmt())?;
            }
            ws.write_number(row, 3, value.htm_id as f64)?;
            ws.write_string(row, 4, value.state.label())?;
            ws.write_string(row, 5, support_status_label(value.status))?;
            if let Some(duration_ms) = value.duration_ms {
                ws.write_number(row, 6, duration_ms as f64)?;
            }
            if let Some(ratio) = value.vehicle_share {
                ws.write_number(row, 7, ratio.denominator as f64)?;
                if let Some(fraction) = ratio.fraction() {
                    ws.write_number_with_format(row, 8, fraction, percent)?;
                }
            }
            row += 1;
        }
    }

    row += 1;
    ws.write_string_with_format(row, 0, "Per-D30 selected-plan waiting", b)?;
    row += 1;
    for (column, header) in [
        "Scope",
        "Hour",
        "Hour completeness",
        "D30",
        "Category",
        "Status",
        "Duration (ms)",
        "D30 waiting time (ms)",
        "% D30 waiting time",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo), ("Hourly", hourly)] {
        for value in &report.data.vehicle_wait_category_rows {
            ws.write_string(row, 0, scope)?;
            if scope == "Hourly" {
                ws.write_number(row, 1, bucket_hour(&value.bucket, HOUR_MS))?;
                ws.write_number_with_format(row, 2, bucket_completeness(&value.bucket, HOUR_MS), &completeness_fmt())?;
            }
            ws.write_number(row, 3, value.robot_id.0 as f64)?;
            ws.write_string(row, 4, value.category.label())?;
            ws.write_string(row, 5, support_status_label(value.status))?;
            if let Some(duration_ms) = value.duration_ms {
                ws.write_number(row, 6, duration_ms as f64)?;
            }
            if let Some(ratio) = value.waiting_share {
                ws.write_number(row, 7, ratio.denominator as f64)?;
                if let Some(fraction) = ratio.fraction() {
                    ws.write_number_with_format(row, 8, fraction, percent)?;
                }
            }
            row += 1;
        }
    }
    Ok(())
}

/// The D30 Distance sheet: fleet travel distance by kind per scope, the
/// hourly distance chart, and the per-D30 drilldown table. Values come
/// entirely from the state builder's distance rows.
pub(super) fn write_d30_distance_sheet(
    wb: &mut Workbook,
    full: &MetricsReport<D30StateData>,
    post_embargo: &MetricsReport<D30StateData>,
    hourly: &MetricsReport<D30StateData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("D30 Distance")?;
    let b = bold();
    ws.write_string_with_format(0, 0, "Overall D30 travel distance", &b)?;
    for (column, header) in ["Scope", "Kind", "Status", "Distance (mm)", "Distance (m)"].iter().enumerate() {
        ws.write_string_with_format(1, column as u16, *header, &b)?;
    }
    let mut row = 2;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for value in &report.data.distance_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, value.kind.label())?;
            ws.write_string(row, 2, support_status_label(value.status))?;
            if let Some(distance_mm) = value.distance_mm {
                ws.write_number(row, 3, distance_mm as f64)?;
                ws.write_number(row, 4, distance_mm as f64 / 1_000.0)?;
            }
            row += 1;
        }
    }

    let hourly_title_row = row + 1;
    ws.write_string_with_format(hourly_title_row, 0, "Hourly D30 travel distance", &b)?;
    let hourly_header_row = hourly_title_row + 1;
    ws.write_string_with_format(hourly_header_row, 0, "Hour", &b)?;
    ws.write_string_with_format(hourly_header_row, 1, "Hour completeness", &b)?;
    for (index, kind) in D30_TRAVEL_KINDS.iter().enumerate() {
        ws.write_string_with_format(hourly_header_row, index as u16 + 2, format!("{} distance (m)", kind.label()), &b)?;
    }
    let buckets = state_report_buckets(&hourly.data);
    for (index, bucket) in buckets.iter().enumerate() {
        let data_row = hourly_header_row + index as u32 + 1;
        ws.write_number(data_row, 0, bucket_hour(bucket, HOUR_MS))?;
        ws.write_number_with_format(data_row, 1, bucket_completeness(bucket, HOUR_MS), &completeness_fmt())?;
        for (kind_index, kind) in D30_TRAVEL_KINDS.iter().enumerate() {
            if let Some(distance_mm) = hourly
                .data
                .distance_rows
                .iter()
                .find(|value| value.bucket == *bucket && value.kind == *kind)
                .and_then(|value| value.distance_mm)
            {
                ws.write_number(data_row, kind_index as u16 + 2, distance_mm as f64 / 1_000.0)?;
            }
        }
    }
    if !buckets.is_empty() {
        let first_row = hourly_header_row + 1;
        let last_row = first_row + buckets.len() as u32 - 1;
        let mut chart = Chart::new(ChartType::Line);
        for (index, _) in D30_TRAVEL_KINDS.iter().enumerate() {
            let column = index as u16 + 2;
            chart
                .add_series()
                .set_categories(("D30 Distance", first_row, 0, last_row, 0))
                .set_values(("D30 Distance", first_row, column, last_row, column))
                .set_name(("D30 Distance", hourly_header_row, column));
        }
        chart.title().set_name("D30 travel distance by hour");
        chart.x_axis().set_name("Hour");
        chart.x_axis().set_num_format("0");
        chart.y_axis().set_name("Distance (m)");
        chart.set_style(10);
        ws.insert_chart(hourly_title_row, 7, &chart)?;
    }

    row = hourly_header_row + buckets.len() as u32 + 3;
    ws.write_string_with_format(row, 0, "Per-D30 distance data", &b)?;
    row += 1;
    for (column, header) in
        ["Scope", "Hour", "Hour completeness", "D30", "Kind", "Status", "Distance (mm)", "Distance (m)"]
            .iter()
            .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo), ("Hourly", hourly)] {
        for value in &report.data.vehicle_distance_rows {
            ws.write_string(row, 0, scope)?;
            if scope == "Hourly" {
                ws.write_number(row, 1, bucket_hour(&value.bucket, HOUR_MS))?;
                ws.write_number_with_format(row, 2, bucket_completeness(&value.bucket, HOUR_MS), &completeness_fmt())?;
            }
            ws.write_number(row, 3, value.htm_id as f64)?;
            ws.write_string(row, 4, value.kind.label())?;
            ws.write_string(row, 5, support_status_label(value.status))?;
            if let Some(distance_mm) = value.distance_mm {
                ws.write_number(row, 6, distance_mm as f64)?;
                ws.write_number(row, 7, distance_mm as f64 / 1_000.0)?;
            }
            row += 1;
        }
    }
    ws.set_column_range_width(0, 7, 20)?;
    ws.set_freeze_panes(hourly_header_row + 1, 0)?;
    Ok(())
}

/// States excluded from charts: their rows are permanently `Unsupported`
/// (no duration-bearing facts until #275), so charting them would plot the
/// absence of data as a zero-height honest-looking series.
fn d30_state_is_charted(state: D30State) -> bool {
    !matches!(state, D30State::Pickup | D30State::Drop | D30State::Alignment)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum D30FlatRow {
    State(D30State),
    Wait(D30WaitCategory),
}

fn d30_flat_rows() -> Vec<D30FlatRow> {
    let mut rows = Vec::with_capacity(D30_STATES.len() - 1 + D30_WAIT_CATEGORIES.len());
    for state in D30_STATES {
        match state {
            D30State::Waiting => {
                rows.extend(D30_WAIT_CATEGORIES.into_iter().map(D30FlatRow::Wait));
            }
            other => rows.push(D30FlatRow::State(other)),
        }
    }
    rows
}

fn d30_flat_row_label(row: D30FlatRow) -> &'static str {
    match row {
        D30FlatRow::State(state) => state.label(),
        D30FlatRow::Wait(category) => category.label(),
    }
}

fn d30_flat_row_is_charted(row: D30FlatRow) -> bool {
    match row {
        D30FlatRow::State(state) => d30_state_is_charted(state),
        D30FlatRow::Wait(_) => true,
    }
}

fn d30_flat_row_values(
    data: &D30StateData,
    bucket: TimeBucket,
    row: D30FlatRow,
) -> (SupportStatus, Option<u64>, Option<Ratio>) {
    match row {
        D30FlatRow::State(state) => {
            let state_row =
                find_state_row(data, bucket, state).expect("state builder emits every state row per bucket");
            (state_row.status, state_row.duration_ms, state_row.fleet_share)
        }
        D30FlatRow::Wait(category) => {
            let wait_row = data
                .wait_category_rows
                .iter()
                .find(|candidate| candidate.bucket == bucket && candidate.category == category)
                .expect("state builder emits every wait-category row per bucket");
            let fleet_time_ms = bucket.coverage_ms * data.fleet_size as u64;
            let fleet_share = wait_row.duration_ms.map(|duration_ms| Ratio::new(duration_ms, fleet_time_ms));
            (wait_row.status, wait_row.duration_ms, fleet_share)
        }
    }
}

fn state_report_buckets(data: &D30StateData) -> Vec<TimeBucket> {
    data.state_rows.iter().map(|row| row.bucket).collect::<BTreeSet<_>>().into_iter().collect()
}

fn first_state_bucket(data: &D30StateData) -> Option<TimeBucket> {
    state_report_buckets(data).into_iter().next()
}

/// The slices the D30 State sheet's pie charts for the report's first bucket:
/// each charted flat row's label and duration, in the order the sheet lists
/// them. A row whose report states no duration carries none here either, the
/// sheet leaving its cell empty.
///
/// desvis draws the same pie in its metrics panel from its own flat summary
/// rows, and the two must agree slice for slice — the table IS the pie legend.
/// The test that pins that lives in desvis, where both renderings are visible;
/// this is the sheet's side of it.
pub fn d30_state_pie_slices(data: &D30StateData) -> Vec<(&'static str, Option<u64>)> {
    let Some(bucket) = first_state_bucket(data) else {
        return Vec::new();
    };
    d30_flat_rows()
        .into_iter()
        .filter(|row| d30_flat_row_is_charted(*row))
        .map(|row| {
            let (_, duration_ms, _) = d30_flat_row_values(data, bucket, row);
            (d30_flat_row_label(row), duration_ms)
        })
        .collect()
}

fn find_state_row(data: &D30StateData, bucket: TimeBucket, state: D30State) -> Option<&D30StateDurationRow> {
    data.state_rows.iter().find(|row| row.bucket == bucket && row.state == state)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use desgraph::RelayId;
    use dessim::metrics::reporting::{
        D30StateData, FixedPeriod, MetricScope, MetricsReport, ReportPeriod, ReportRequest, d30_state_report,
    };
    use dessim::model::RobotId;
    use dessim::observer::output::{
        Activity, HtmHold, HtmJournalEndpoint, HtmLegRecord, HtmWaitCause, HtmWaitPosition, HtmWaitReason,
    };
    use dessim::{SimDuration, SimTime};
    use rust_xlsxwriter::Workbook;
    use zip::ZipArchive;

    use super::{d30_flat_row_is_charted, d30_flat_rows, write_d30_distance_sheet, write_d30_state_sheet};

    /// Reordering the taxonomy breaks this test rather than the charts.
    #[test]
    fn charted_flat_rows_form_a_contiguous_suffix() {
        let flat_rows = d30_flat_rows();
        let first_charted =
            flat_rows.iter().position(|row| d30_flat_row_is_charted(*row)).expect("charted flat rows exist");
        for (index, row) in flat_rows.iter().enumerate() {
            assert_eq!(
                d30_flat_row_is_charted(*row),
                index >= first_charted,
                "flat row #{index} breaks the contiguous charted suffix"
            );
        }
    }

    fn leg(
        start_ms: u64,
        duration_ms: u64,
        activity: Activity,
        from: HtmJournalEndpoint,
        to: HtmJournalEndpoint,
        carrying_cargo: bool,
        distance_mm: u64,
        wait_reason: Option<HtmWaitReason>,
    ) -> HtmLegRecord {
        HtmLegRecord {
            start: SimTime::ms(start_ms),
            activity,
            from,
            to,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm,
            duration: SimDuration::ms(duration_ms),
            presentation_id: Some(1),
            robot_id: RobotId(1),
            carrying_cargo,
            turn_count: 0,
            wait_reason,
            hold: matches!(wait_reason, Some(HtmWaitReason::NoTask))
                .then_some(HtmHold { cause: HtmWaitCause::NoTask, position: HtmWaitPosition::Relay }),
            notes: String::new(),
        }
    }

    /// The raw XML fragment following cell `reference` (40 chars), or empty
    /// when the cell was never written -- the #318 cell-binding idiom,
    /// shared by the value asserts and the positional label asserts.
    fn cell_owned(sheet: &str, reference: &str) -> String {
        sheet
            .split(&format!("r=\"{reference}\""))
            .nth(1)
            .map(|rest| rest.chars().take(40).collect::<String>())
            .unwrap_or_default()
    }

    /// The 90-minute fixture behind both sheet tests: 1 s of empty travel in
    /// hour one, 1 s of explicit NoTask idle in the ragged half-hour,
    /// everything else a no-leg gap (Waiting → Unknown / unattributed).
    fn fixture_reports() -> (MetricsReport<D30StateData>, MetricsReport<D30StateData>, MetricsReport<D30StateData>) {
        let relay = HtmJournalEndpoint::Relay(RelayId(2));
        let legs = [
            leg(0, 1_000, Activity::Travel, HtmJournalEndpoint::Parking, relay, false, 500, None),
            leg(3_600_000, 1_000, Activity::Wait, relay, relay, false, 0, Some(HtmWaitReason::NoTask)),
        ];
        let full = d30_state_report(
            &legs,
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 5_400_000, 0),
        )
        .unwrap();
        let post_embargo = d30_state_report(
            &legs,
            1,
            ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 5_400_000, 3_600_000),
        )
        .unwrap();
        let hourly = d30_state_report(
            &legs,
            1,
            ReportRequest::for_run(
                MetricScope::FullRun,
                ReportPeriod::Fixed(FixedPeriod::new(3_600_000, 0).unwrap()),
                5_400_000,
                0,
            ),
        )
        .unwrap();
        (full, post_embargo, hourly)
    }

    /// Three states carry no duration-bearing facts until #275, so the pie
    /// leaves them out; desvis's own test pins the remaining slices against
    /// the metrics panel's.
    #[test]
    fn the_pie_charts_every_flat_row_but_the_three_unsupported_states() {
        let (full, _, _) = fixture_reports();
        assert_eq!(super::d30_state_pie_slices(&full.data).len(), d30_flat_rows().len() - 3);
    }

    #[test]
    fn d30_state_sheet_binds_values_charts_and_unsupported_rows() {
        let (full, post_embargo, hourly) = fixture_reports();
        let mut workbook = Workbook::new();
        write_d30_state_sheet(&mut workbook, &full, &post_embargo, &hourly).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing
        // left behind on disk when an assertion below fails.
        let buf = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"D30 State\""), "sheet carries exactly the user-facing name");

        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        let cell = |reference: &str| cell_owned(&sheet, reference);
        // Full-run flat section, rows 3..22 (the twelve ratified categories
        // inline): Empty travel 6, No task (relay/charger) 10, Unattributed gap
        // 19. No merged Waiting row.
        assert!(cell("C6").contains(">1000<"), "empty-travel duration in C6");
        assert!(cell("C10").contains(">1000<"), "relay NoTask duration in C10");
        assert!(cell("C11").contains(">0<"), "elsewhere NoTask duration in C11");
        assert!(cell("C19").contains(">5398000<"), "unattributed-gap duration in C19");
        // Pickup (row 3) is Unsupported: no fabricated duration cell.
        assert_eq!(cell("C3"), "", "unsupported Pickup row carries no duration");
        // Waiting attribution block (shares of Waiting), full-run rows
        // 58..69 in category order. Every fixture robot journals legs, so
        // No facts (68) is a measured zero, not the unobserved backfill it
        // carries on a run with configured-but-silent D30s.
        assert!(cell("D58").contains(">1000<"), "relay NoTask duration in D58");
        assert!(cell("D59").contains(">0<"), "elsewhere NoTask duration in D59");
        assert!(cell("D60").contains(">0<"), "presenting row carries its measured zero");
        assert!(cell("D68").contains(">0<"), "fully journaled fixture leaves No facts a measured zero");
        assert!(cell("D85").contains(">500<"), "empty travel distance (mm) in D85");
        // #291's fixed-hour convention: integer start-edge Hour plus an
        // explicit coverage fraction, not Start/End/Partial.
        assert!(cell("A52").contains(">0<"), "first bucket has Hour 0");
        assert!(cell("B52").contains(">1<"), "first hour is complete");
        assert!(cell("A53").contains(">1<"), "second bucket has Hour 1");
        assert!(cell("B53").contains(">0.5<"), "ragged second hour is half complete");

        let mut shared = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared).unwrap();
        for disclosure in [
            "Pickup",
            "Drop",
            "Alignment",
            "Empty travel",
            "Laden travel",
            "Travel to park",
            "Unsupported",
            "Selected-plan waiting attribution",
            "Hour completeness",
            "No task (relay/charger)",
            "No task (elsewhere)",
            "Presenting",
            "Awaiting Relay Deposit",
            "Awaiting Relay Slot",
            "In Queue (station)",
            "Congestion",
            "Unattributed gap",
            "No facts",
            "Conflicting facts",
            "Per-D30 state data",
            "Per-D30 selected-plan waiting",
            "Station processing = the recorded operator dwell",
            "presentation occupancy can include departure clearance or downstream blocking",
        ] {
            assert!(shared.contains(disclosure), "sheet retains {disclosure}");
        }
        for retired in [
            "Awaiting DA50 deposit",
            "Scheduled route hold",
            "Station queue hold",
            "Post-presentation return hold",
            "Awaiting return slot",
        ] {
            assert!(!shared.contains(retired), "sheet retires {retired}");
        }
        // Positional binds for the substring-shadowed tail of the state
        // column: resolve each cell's sharedStrings index to its exact text.
        let shared_texts: Vec<&str> = shared
            .split("<si>")
            .skip(1)
            .filter_map(|entry| {
                let after_tag = entry.split_once("<t")?.1.split_once('>')?.1;
                after_tag.split_once("</t>").map(|(text, _)| text)
            })
            .collect();
        let cell_text = |reference: &str| -> &str {
            cell_owned(&sheet, reference)
                .split_once("<v>")
                .and_then(|(_, rest)| rest.split_once("</v>"))
                .map(|(index, _)| shared_texts[index.parse::<usize>().unwrap()])
                .unwrap_or("")
        };
        assert_eq!(cell_text("A9"), "Station processing", "flat row 9");
        assert_eq!(cell_text("A10"), "No task (relay/charger)", "first wait category replaces the merged Waiting row");
        assert_eq!(cell_text("A22"), "Unknown", "flat row 22");

        let mut full_pie_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut full_pie_xml).unwrap();
        assert!(full_pie_xml.contains("<c:pieChart>"));
        // Rows 6..22 are the measured suffix; 3..5 are Pickup/Drop/Alignment.
        assert!(full_pie_xml.contains("'D30 State'!$A$6:$A$22"));
        assert!(full_pie_xml.contains("'D30 State'!$C$6:$C$22"));
        assert!(!full_pie_xml.contains("$A$3"), "unsupported head is not charted");

        let mut post_pie_xml = String::new();
        archive.by_name("xl/charts/chart2.xml").unwrap().read_to_string(&mut post_pie_xml).unwrap();
        assert!(post_pie_xml.contains("<c:pieChart>"));
        assert!(post_pie_xml.contains("'D30 State'!$A$31:$A$47"), "post-embargo categories");
        assert!(post_pie_xml.contains("'D30 State'!$C$31:$C$47"), "post-embargo values");

        let mut hourly_xml = String::new();
        archive.by_name("xl/charts/chart3.xml").unwrap().read_to_string(&mut hourly_xml).unwrap();
        assert!(hourly_xml.contains("<c:barChart>"));
        assert!(hourly_xml.contains("<c:grouping val=\"percentStacked\"/>"));
        assert!(hourly_xml.contains("'D30 State'!$A$52:$A$53"), "hour categories");
        assert!(hourly_xml.contains("'D30 State'!$J$52:$J$53"), "first charted series");
        assert!(hourly_xml.contains("'D30 State'!$AJ$52:$AJ$53"), "unattributed-gap series");
        // ALL permanently-Unsupported states are excluded from the chart:
        // Pickup (D), Drop (F), and Alignment (H) duration columns must not
        // appear as series.
        for excluded in ["$D$52:$D$53", "$F$52:$F$53", "$H$52:$H$53"] {
            assert!(
                !hourly_xml.contains(&format!("'D30 State'!{excluded}")),
                "unsupported state column {excluded} is not charted"
            );
        }

        drop(archive);
    }

    #[test]
    fn d30_distance_sheet_binds_values_and_charts_the_hourly_kinds() {
        let (full, post_embargo, hourly) = fixture_reports();
        let mut workbook = Workbook::new();
        write_d30_distance_sheet(&mut workbook, &full, &post_embargo, &hourly).unwrap();
        let buf = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"D30 Distance\""), "sheet carries exactly the user-facing name");

        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        let cell = |reference: &str| {
            sheet
                .split(&format!("r=\"{reference}\""))
                .nth(1)
                .map(|rest| rest.chars().take(40).collect::<String>())
                .unwrap_or_default()
        };
        // Overall table rows 3..8 (two scopes × Empty/Laden/Parking):
        // full-run Empty distance in row 3, mm in D and m in E.
        assert!(cell("D3").contains(">500<"), "full-run empty distance (mm) in D3");
        assert!(cell("E3").contains(">0.5<"), "full-run empty distance (m) in E3");
        assert!(cell("D4").contains(">0<"), "laden distance is a measured zero");
        assert!(cell("A12").contains(">0<"), "first bucket has Hour 0");
        assert!(cell("B12").contains(">1<"), "first hour is complete");
        assert!(cell("A13").contains(">1<"), "second bucket has Hour 1");
        assert!(cell("B13").contains(">0.5<"), "ragged second hour is half complete");

        let mut chart_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut chart_xml).unwrap();
        assert!(chart_xml.contains("<c:lineChart>"));
        // Hourly table data rows 12..13; categories in A, one series per
        // travel kind in C/D/E.
        assert!(chart_xml.contains("'D30 Distance'!$A$12:$A$13"), "hour categories");
        assert!(chart_xml.contains("'D30 Distance'!$C$12:$C$13"), "empty series");
        assert!(chart_xml.contains("'D30 Distance'!$D$12:$D$13"), "laden series");
        assert!(chart_xml.contains("'D30 Distance'!$E$12:$E$13"), "parking series");

        drop(archive);
    }
}
