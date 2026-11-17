//! Within `workbook`, `da50` renders the DA50 (VTM) sheets: the raw
//! operations-journal dump plus the canonical DA50 State (with per-DA50
//! drilldown tables), DA50 Throughput, and DA50 Distance report sheets with
//! their charts, and owns the operation and payload labels for the journal
//! dump. Report values and their rendered names come from
//! `dessim::metrics::reporting`; this module owns only layout.

use std::collections::BTreeSet;

use dessim::events::{TotePayloadState, VtmOperationKind};
use dessim::metrics::reporting::{
    DA50_LOAD_STATES, DA50_OPERATION_KINDS, DA50_PAYLOAD_GROUPS, DA50_STATES, DA50_WAIT_REASONS, Da50PayloadGroup,
    Da50State, Da50StateData, Da50StateDurationRow, Da50ThroughputData, MetricsReport, TimeBucket,
};
use dessim::observer::output::VtmOperationRecord;
use rust_xlsxwriter::{Chart, ChartType, Format, Workbook, Worksheet, XlsxError};

use super::{bold, hour_fmt, hour_index, line_chart_for_columns, metric_label, support_status_label};

pub(super) fn write_da50_operations_sheet(
    wb: &mut Workbook,
    operations: &[VtmOperationRecord],
) -> Result<(), XlsxError> {
    if operations.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("DA50 Operations")?;
    let b = bold();
    let headers = [
        "Time (ms)",
        "Hour",
        "DA50 ID",
        "Aisle",
        "Operation",
        "Endpoint",
        "Endpoint ID",
        "Rack ID",
        "Diving Board ID",
        "Tote ID",
        "Payload",
        "Cycle ID",
        "Trip ID",
    ];
    for (column, header) in headers.iter().enumerate() {
        ws.write_string_with_format(0, column as u16, *header, &b)?;
    }
    let mut ordered: Vec<_> = operations.iter().collect();
    ordered.sort_by_key(|operation| operation.sim_time_ms);
    for (index, operation) in ordered.iter().enumerate() {
        let row = (index + 1) as u32;
        let (endpoint, endpoint_id) = da50_operation_endpoint(operation);
        ws.write_number(row, 0, operation.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(operation.sim_time_ms))?;
        ws.write_number(row, 2, operation.vtm_id as f64)?;
        ws.write_number(row, 3, operation.zone_id as f64)?;
        ws.write_string(row, 4, da50_operation_label(operation.kind))?;
        ws.write_string(row, 5, endpoint)?;
        ws.write_number(row, 6, endpoint_id as f64)?;
        ws.write_number(row, 7, operation.rack_id as f64)?;
        ws.write_number(row, 8, operation.relay_id as f64)?;
        ws.write_number(row, 9, operation.tote_id as f64)?;
        ws.write_string(row, 10, tote_payload_label(operation.payload))?;
        ws.write_number(row, 11, operation.vtm_cycle_id as f64)?;
        if let Some(presentation_id) = operation.presentation_id {
            ws.write_number(row, 12, presentation_id as f64)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

pub(super) fn write_da50_state_sheet(
    wb: &mut Workbook,
    full_state: &MetricsReport<Da50StateData>,
    post_embargo_state: &MetricsReport<Da50StateData>,
    hourly_state: &MetricsReport<Da50StateData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("DA50 State")?;
    let b = bold();
    let percent = Format::new().set_num_format("0.00%");
    let state_columns = DA50_STATES;

    let full_first_row = write_da50_overall_state_section(ws, 0, "Full run", full_state, &b, &percent)?;
    if let Some(bucket) = first_state_bucket(&full_state.data) {
        let mut chart = Chart::new(ChartType::Pie);
        chart
            .add_series()
            .set_categories(("DA50 State", full_first_row, 0, full_first_row + state_columns.len() as u32 - 1, 0))
            .set_values(("DA50 State", full_first_row, 2, full_first_row + state_columns.len() as u32 - 1, 2));
        chart.title().set_name(&format!("DA50 state — full run ({:.2} h)", bucket.coverage_ms as f64 / 3_600_000.0));
        chart.set_style(10);
        ws.insert_chart(0, 7, &chart)?;
    }

    let post_section_row = 18;
    let post_first_row =
        write_da50_overall_state_section(ws, post_section_row, "Post-embargo", post_embargo_state, &b, &percent)?;
    if first_state_bucket(&post_embargo_state.data).is_some() {
        let mut chart = Chart::new(ChartType::Pie);
        chart
            .add_series()
            .set_categories(("DA50 State", post_first_row, 0, post_first_row + state_columns.len() as u32 - 1, 0))
            .set_values(("DA50 State", post_first_row, 2, post_first_row + state_columns.len() as u32 - 1, 2));
        chart.title().set_name("DA50 state — post-embargo");
        chart.set_style(10);
        ws.insert_chart(post_section_row, 7, &chart)?;
    }

    let hourly_title_row = 36;
    ws.write_string_with_format(hourly_title_row, 0, "Hourly DA50 state", &b)?;
    let hourly_header_row = hourly_title_row + 1;
    ws.write_string_with_format(hourly_header_row, 0, "Start hour", &b)?;
    ws.write_string_with_format(hourly_header_row, 1, "End hour", &b)?;
    ws.write_string_with_format(hourly_header_row, 2, "Partial", &b)?;
    ws.write_string_with_format(hourly_header_row, 3, "Fleet time (ms)", &b)?;
    for (index, state) in state_columns.iter().enumerate() {
        let duration_column = 4 + index as u16 * 2;
        ws.write_string_with_format(hourly_header_row, duration_column, format!("{} (ms)", state.label()), &b)?;
        ws.write_string_with_format(
            hourly_header_row,
            duration_column + 1,
            format!("{} (% fleet)", state.label()),
            &b,
        )?;
    }
    let hourly_buckets = state_report_buckets(&hourly_state.data);
    for (index, bucket) in hourly_buckets.iter().enumerate() {
        let row = hourly_header_row + 1 + index as u32;
        let fleet_time_ms = bucket.coverage_ms * hourly_state.data.fleet_size as u64;
        let hour = hour_fmt();
        ws.write_number_with_format(row, 0, bucket.start_ms as f64 / 3_600_000.0, &hour)?;
        ws.write_number_with_format(row, 1, bucket.end_ms as f64 / 3_600_000.0, &hour)?;
        ws.write_boolean(row, 2, bucket.partial)?;
        ws.write_number(row, 3, fleet_time_ms as f64)?;
        for (state_index, state) in state_columns.iter().enumerate() {
            let duration_column = 4 + state_index as u16 * 2;
            if let Some(state_row) = find_state_row(&hourly_state.data, *bucket, *state) {
                if let Some(duration_ms) = state_row.duration_ms {
                    ws.write_number(row, duration_column, duration_ms as f64)?;
                }
                if let Some(fraction) = state_row.fleet_share.and_then(|ratio| ratio.fraction()) {
                    ws.write_number_with_format(row, duration_column + 1, fraction, &percent)?;
                }
            }
        }
    }

    if !hourly_buckets.is_empty() {
        let first_data_row = hourly_header_row + 1;
        let last_data_row = first_data_row + hourly_buckets.len() as u32 - 1;
        let mut chart = Chart::new(ChartType::ColumnPercentStacked);
        for state in state_columns.iter().filter(|state| da50_state_is_charted(**state)) {
            let source_index = state_columns.iter().position(|candidate| candidate == state).unwrap();
            let duration_column = 4 + source_index as u16 * 2;
            chart
                .add_series()
                .set_categories(("DA50 State", first_data_row, 1, last_data_row, 1))
                .set_values(("DA50 State", first_data_row, duration_column, last_data_row, duration_column))
                .set_name(("DA50 State", hourly_header_row, duration_column));
        }
        chart.title().set_name(&format!("{} by hour", metric_label(&hourly_state.descriptors, "da50.state.duration")));
        chart.x_axis().set_name("Simulation time (h)");
        chart.x_axis().set_num_format("0.00");
        chart.y_axis().set_name("Fleet time (%)");
        chart.set_style(10);
        ws.insert_chart(hourly_header_row, 23, &chart)?;
    }

    let detail_row = hourly_header_row + hourly_buckets.len() as u32 + 3;
    let detail_end_row =
        write_da50_wait_and_distance_tables(ws, detail_row, full_state, post_embargo_state, &b, &percent)?;
    write_da50_vehicle_state_tables(
        ws,
        detail_end_row + 2,
        full_state,
        post_embargo_state,
        hourly_state,
        &b,
        &percent,
    )?;
    ws.set_freeze_panes(hourly_header_row + 1, 0)?;
    Ok(())
}

fn write_da50_overall_state_section(
    ws: &mut Worksheet,
    section_row: u32,
    scope_label: &str,
    report: &MetricsReport<Da50StateData>,
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
    for (index, state) in DA50_STATES.iter().enumerate() {
        let row = first_data_row + index as u32;
        ws.write_string(row, 0, state.label())?;
        let state_row = bucket.map(|bucket| {
            find_state_row(&report.data, bucket, *state).expect("state builder emits every state row per bucket")
        });
        if let Some(state_row) = state_row {
            ws.write_string(row, 1, support_status_label(state_row.status))?;
        }
        if let Some(duration_ms) = state_row.and_then(|row| row.duration_ms) {
            ws.write_number(row, 2, duration_ms as f64)?;
            ws.write_number(row, 3, duration_ms as f64 / 3_600_000.0)?;
        }
        if let Some(ratio) = state_row.and_then(|row| row.fleet_share) {
            ws.write_number(row, 4, ratio.denominator as f64)?;
            if let Some(fraction) = ratio.fraction() {
                ws.write_number_with_format(row, 5, fraction, percent)?;
            }
        }
    }
    Ok(first_data_row)
}

fn write_da50_wait_and_distance_tables(
    ws: &mut Worksheet,
    start_row: u32,
    full_state: &MetricsReport<Da50StateData>,
    post_embargo_state: &MetricsReport<Da50StateData>,
    b: &Format,
    percent: &Format,
) -> Result<u32, XlsxError> {
    ws.write_string_with_format(start_row, 0, "Selected-plan waiting attribution", b)?;
    let header_row = start_row + 1;
    for (column, header) in
        ["Scope", "Reason", "Status", "Duration (ms)", "Waiting time (ms)", "% waiting time"].iter().enumerate()
    {
        ws.write_string_with_format(header_row, column as u16, *header, b)?;
    }
    let mut row = header_row + 1;
    for (scope, report) in [("Full run", full_state), ("Post-embargo", post_embargo_state)] {
        let Some(bucket) = first_state_bucket(&report.data) else {
            continue;
        };
        for reason in DA50_WAIT_REASONS {
            let wait_row =
                report.data.wait_rows.iter().find(|candidate| candidate.bucket == bucket && candidate.reason == reason);
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, reason.label())?;
            ws.write_string(
                row,
                2,
                support_status_label(wait_row.expect("state builder emits every wait row per bucket").status),
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
    for (column, header) in ["Scope", "Load", "Status", "Distance (mm)", "Distance (m)"].iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *header, b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full_state), ("Post-embargo", post_embargo_state)] {
        let Some(bucket) = first_state_bucket(&report.data) else {
            continue;
        };
        for load in DA50_LOAD_STATES {
            let distance_row =
                report.data.distance_rows.iter().find(|candidate| candidate.bucket == bucket && candidate.load == load);
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, load.label())?;
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

/// Per-DA50 drilldown tables appended to the DA50 State sheet: each observed
/// vehicle's state partition and waiting attribution per scope and bucket.
/// Values come from the builder's vehicle rows; fleet rows reconcile to these
/// plus the unobserved-vehicle backfill.
fn write_da50_vehicle_state_tables(
    ws: &mut Worksheet,
    start_row: u32,
    full: &MetricsReport<Da50StateData>,
    post_embargo: &MetricsReport<Da50StateData>,
    hourly: &MetricsReport<Da50StateData>,
    b: &Format,
    percent: &Format,
) -> Result<(), XlsxError> {
    let hour = hour_fmt();
    ws.write_string_with_format(start_row, 0, "Per-DA50 state data", b)?;
    let header_row = start_row + 1;
    for (column, header) in [
        "Scope",
        "Start hour",
        "End hour",
        "Partial",
        "DA50",
        "State",
        "Status",
        "Duration (ms)",
        "DA50 included time (ms)",
        "% DA50 time",
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
            ws.write_number_with_format(row, 1, value.bucket.start_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_number_with_format(row, 2, value.bucket.end_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_boolean(row, 3, value.bucket.partial)?;
            ws.write_number(row, 4, value.vtm_id as f64)?;
            ws.write_string(row, 5, value.state.label())?;
            ws.write_string(row, 6, support_status_label(value.status))?;
            if let Some(duration_ms) = value.duration_ms {
                ws.write_number(row, 7, duration_ms as f64)?;
            }
            if let Some(ratio) = value.vehicle_share {
                ws.write_number(row, 8, ratio.denominator as f64)?;
                if let Some(fraction) = ratio.fraction() {
                    ws.write_number_with_format(row, 9, fraction, percent)?;
                }
            }
            row += 1;
        }
    }

    row += 1;
    ws.write_string_with_format(row, 0, "Per-DA50 selected-plan waiting", b)?;
    row += 1;
    for (column, header) in [
        "Scope",
        "Start hour",
        "End hour",
        "Partial",
        "DA50",
        "Reason",
        "Status",
        "Duration (ms)",
        "DA50 waiting time (ms)",
        "% DA50 waiting time",
    ]
    .iter()
    .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo), ("Hourly", hourly)] {
        for value in &report.data.vehicle_wait_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_number_with_format(row, 1, value.bucket.start_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_number_with_format(row, 2, value.bucket.end_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_boolean(row, 3, value.bucket.partial)?;
            ws.write_number(row, 4, value.vtm_id as f64)?;
            ws.write_string(row, 5, value.reason.label())?;
            ws.write_string(row, 6, support_status_label(value.status))?;
            if let Some(duration_ms) = value.duration_ms {
                ws.write_number(row, 7, duration_ms as f64)?;
            }
            if let Some(ratio) = value.waiting_share {
                ws.write_number(row, 8, ratio.denominator as f64)?;
                if let Some(fraction) = ratio.fraction() {
                    ws.write_number_with_format(row, 9, fraction, percent)?;
                }
            }
            row += 1;
        }
    }
    Ok(())
}

/// The DA50 Distance sheet: fleet travel distance by load state per scope,
/// the hourly distance chart, and the per-DA50 drilldown table. Values come
/// entirely from the state builder's distance rows.
pub(super) fn write_da50_distance_sheet(
    wb: &mut Workbook,
    full: &MetricsReport<Da50StateData>,
    post_embargo: &MetricsReport<Da50StateData>,
    hourly: &MetricsReport<Da50StateData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("DA50 Distance")?;
    let b = bold();
    let hour = hour_fmt();
    ws.write_string_with_format(0, 0, "Overall DA50 travel distance", &b)?;
    for (column, header) in ["Scope", "Load", "Status", "Distance (mm)", "Distance (m)"].iter().enumerate() {
        ws.write_string_with_format(1, column as u16, *header, &b)?;
    }
    let mut row = 2;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo)] {
        for value in &report.data.distance_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_string(row, 1, value.load.label())?;
            ws.write_string(row, 2, support_status_label(value.status))?;
            if let Some(distance_mm) = value.distance_mm {
                ws.write_number(row, 3, distance_mm as f64)?;
                ws.write_number(row, 4, distance_mm as f64 / 1_000.0)?;
            }
            row += 1;
        }
    }

    let hourly_title_row = row + 1;
    ws.write_string_with_format(hourly_title_row, 0, "Hourly DA50 travel distance", &b)?;
    let hourly_header_row = hourly_title_row + 1;
    ws.write_string_with_format(hourly_header_row, 0, "End hour", &b)?;
    ws.write_string_with_format(hourly_header_row, 1, "Partial", &b)?;
    for (index, load) in DA50_LOAD_STATES.iter().enumerate() {
        ws.write_string_with_format(hourly_header_row, index as u16 + 2, format!("{} distance (m)", load.label()), &b)?;
    }
    let buckets = state_report_buckets(&hourly.data);
    for (index, bucket) in buckets.iter().enumerate() {
        let data_row = hourly_header_row + index as u32 + 1;
        ws.write_number_with_format(data_row, 0, bucket.end_ms as f64 / 3_600_000.0, &hour)?;
        ws.write_boolean(data_row, 1, bucket.partial)?;
        for (load_index, load) in DA50_LOAD_STATES.iter().enumerate() {
            if let Some(distance_mm) = hourly
                .data
                .distance_rows
                .iter()
                .find(|value| value.bucket == *bucket && value.load == *load)
                .and_then(|value| value.distance_mm)
            {
                ws.write_number(data_row, load_index as u16 + 2, distance_mm as f64 / 1_000.0)?;
            }
        }
    }
    if !buckets.is_empty() {
        let first_row = hourly_header_row + 1;
        let last_row = first_row + buckets.len() as u32 - 1;
        let mut chart = Chart::new(ChartType::Line);
        for (index, _) in DA50_LOAD_STATES.iter().enumerate() {
            let column = index as u16 + 2;
            chart
                .add_series()
                .set_categories(("DA50 Distance", first_row, 0, last_row, 0))
                .set_values(("DA50 Distance", first_row, column, last_row, column))
                .set_name(("DA50 Distance", hourly_header_row, column));
        }
        chart.title().set_name("DA50 travel distance by hour");
        chart.x_axis().set_name("Simulation time (h)");
        chart.x_axis().set_num_format("0.00");
        chart.y_axis().set_name("Distance (m)");
        chart.set_style(10);
        ws.insert_chart(hourly_title_row, 7, &chart)?;
    }

    row = hourly_header_row + buckets.len() as u32 + 3;
    ws.write_string_with_format(row, 0, "Per-DA50 distance data", &b)?;
    row += 1;
    for (column, header) in
        ["Scope", "Start hour", "End hour", "Partial", "DA50", "Load", "Status", "Distance (mm)", "Distance (m)"]
            .iter()
            .enumerate()
    {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full), ("Post-embargo", post_embargo), ("Hourly", hourly)] {
        for value in &report.data.vehicle_distance_rows {
            ws.write_string(row, 0, scope)?;
            ws.write_number_with_format(row, 1, value.bucket.start_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_number_with_format(row, 2, value.bucket.end_ms as f64 / 3_600_000.0, &hour)?;
            ws.write_boolean(row, 3, value.bucket.partial)?;
            ws.write_number(row, 4, value.vtm_id as f64)?;
            ws.write_string(row, 5, value.load.label())?;
            ws.write_string(row, 6, support_status_label(value.status))?;
            if let Some(distance_mm) = value.distance_mm {
                ws.write_number(row, 7, distance_mm as f64)?;
                ws.write_number(row, 8, distance_mm as f64 / 1_000.0)?;
            }
            row += 1;
        }
    }
    ws.set_column_range_width(0, 8, 20)?;
    ws.set_freeze_panes(hourly_header_row + 1, 0)?;
    Ok(())
}

fn da50_state_is_charted(state: Da50State) -> bool {
    !matches!(state, Da50State::TravelToPark | Da50State::TravelToUnblockOther)
}

fn state_report_buckets(data: &Da50StateData) -> Vec<TimeBucket> {
    data.state_rows.iter().map(|row| row.bucket).collect::<BTreeSet<_>>().into_iter().collect()
}

fn first_state_bucket(data: &Da50StateData) -> Option<TimeBucket> {
    state_report_buckets(data).into_iter().next()
}

fn find_state_row(data: &Da50StateData, bucket: TimeBucket, state: Da50State) -> Option<&Da50StateDurationRow> {
    data.state_rows.iter().find(|row| row.bucket == bucket && row.state == state)
}

pub(super) fn write_da50_throughput_sheet(
    wb: &mut Workbook,
    full_throughput: &MetricsReport<Da50ThroughputData>,
    post_embargo_throughput: &MetricsReport<Da50ThroughputData>,
    hourly_throughput: &MetricsReport<Da50ThroughputData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("DA50 Throughput")?;
    let b = bold();
    let operation_columns = DA50_OPERATION_KINDS;
    ws.write_string_with_format(0, 0, "End hour", &b)?;
    for (index, operation) in operation_columns.iter().enumerate() {
        ws.write_string_with_format(0, (index + 1) as u16, da50_operation_label(*operation), &b)?;
    }
    let hourly_buckets = throughput_row_buckets(&hourly_throughput.data);
    for (index, bucket) in hourly_buckets.iter().enumerate() {
        let row = (index + 1) as u32;
        ws.write_number(row, 0, bucket.end_ms as f64 / 3_600_000.0)?;
        for (operation_index, operation) in operation_columns.iter().enumerate() {
            ws.write_number(
                row,
                (operation_index + 1) as u16,
                report_operation_count(&hourly_throughput.data, bucket.end_ms, None, Da50PayloadGroup::All, *operation)
                    as f64,
            )?;
        }
    }
    if let Some(chart) = line_chart_for_columns(
        "DA50 Throughput",
        &format!("{} per hour", metric_label(&hourly_throughput.descriptors, "da50.operations")),
        "Operations",
        1,
        hourly_buckets.len() as u32,
        0,
        &[1, 2, 3, 4],
        "Simulation time (h)",
    ) {
        ws.insert_chart(0, 6, &chart)?;
    }

    let mut row = (hourly_buckets.len() as u32 + 3).max(17);
    ws.write_string_with_format(row, 0, "Overall totals", &b)?;
    row += 1;
    let total_headers = [
        "Scope",
        "Aisle",
        "Payload",
        "Pickup from storage",
        "Drop to diving board",
        "Pickup from diving board",
        "Drop to storage",
    ];
    for (column, header) in total_headers.iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    for (scope, report) in [("Full run", full_throughput), ("Post-embargo", post_embargo_throughput)] {
        let end_ms = report.request.range.end_ms;
        for zone_id in report_zones(&report.data) {
            for payload in DA50_PAYLOAD_GROUPS {
                ws.write_string(row, 0, scope)?;
                write_zone_label(ws, row, 1, zone_id)?;
                ws.write_string(row, 2, da50_payload_group_label(payload))?;
                for (index, operation) in operation_columns.iter().enumerate() {
                    ws.write_number(
                        row,
                        (index + 3) as u16,
                        report_operation_count(&report.data, end_ms, zone_id, payload, *operation) as f64,
                    )?;
                }
                row += 1;
            }
        }
    }

    row += 1;
    ws.write_string_with_format(row, 0, "Hourly detail", &b)?;
    row += 1;
    let detail_headers = [
        "Start hour",
        "End hour",
        "Partial",
        "Aisle",
        "Payload",
        "Pickup from storage",
        "Drop to diving board",
        "Pickup from diving board",
        "Drop to storage",
    ];
    for (column, header) in detail_headers.iter().enumerate() {
        ws.write_string_with_format(row, column as u16, *header, &b)?;
    }
    row += 1;
    let zones = report_zones(&hourly_throughput.data);
    for bucket in hourly_buckets {
        for &zone_id in &zones {
            for payload in DA50_PAYLOAD_GROUPS {
                ws.write_number(row, 0, bucket.start_ms as f64 / 3_600_000.0)?;
                ws.write_number(row, 1, bucket.end_ms as f64 / 3_600_000.0)?;
                ws.write_boolean(row, 2, bucket.partial)?;
                write_zone_label(ws, row, 3, zone_id)?;
                ws.write_string(row, 4, da50_payload_group_label(payload))?;
                for (index, operation) in operation_columns.iter().enumerate() {
                    ws.write_number(
                        row,
                        (index + 5) as u16,
                        report_operation_count(&hourly_throughput.data, bucket.end_ms, zone_id, payload, *operation)
                            as f64,
                    )?;
                }
                row += 1;
            }
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn throughput_row_buckets(data: &Da50ThroughputData) -> Vec<TimeBucket> {
    data.rows.iter().map(|row| row.bucket).collect::<BTreeSet<_>>().into_iter().collect()
}

fn report_zones(data: &Da50ThroughputData) -> Vec<Option<u32>> {
    data.rows.iter().map(|row| row.zone_id).collect::<BTreeSet<_>>().into_iter().collect()
}

fn report_operation_count(
    data: &Da50ThroughputData,
    bucket_end_ms: u64,
    zone_id: Option<u32>,
    payload: Da50PayloadGroup,
    operation: VtmOperationKind,
) -> u64 {
    data.rows
        .iter()
        .find(|row| {
            row.bucket.end_ms == bucket_end_ms
                && row.zone_id == zone_id
                && row.payload == payload
                && row.operation == operation
        })
        .expect("throughput builder emits every (bucket, zone, kind, payload) row")
        .count
}

fn da50_payload_group_label(payload: Da50PayloadGroup) -> &'static str {
    match payload {
        Da50PayloadGroup::All => "All",
        Da50PayloadGroup::Empty => "Empty",
        Da50PayloadGroup::NonEmpty => "Non-empty",
        Da50PayloadGroup::Unknown => "Unknown",
    }
}

fn write_zone_label(ws: &mut Worksheet, row: u32, column: u16, zone_id: Option<u32>) -> Result<(), XlsxError> {
    match zone_id {
        Some(zone_id) => {
            ws.write_number(row, column, zone_id as f64)?;
        }
        None => {
            ws.write_string(row, column, "All")?;
        }
    }
    Ok(())
}

fn da50_operation_label(kind: VtmOperationKind) -> &'static str {
    match kind {
        VtmOperationKind::PickupStorage => "Pickup from storage",
        VtmOperationKind::DropRelay => "Drop to diving board",
        VtmOperationKind::PickupRelay => "Pickup from diving board",
        VtmOperationKind::DropStorage => "Drop to storage",
    }
}

fn da50_operation_endpoint(operation: &VtmOperationRecord) -> (&'static str, u32) {
    match operation.kind {
        VtmOperationKind::PickupStorage | VtmOperationKind::DropStorage => ("Storage", operation.rack_id),
        VtmOperationKind::DropRelay | VtmOperationKind::PickupRelay => ("Diving board", operation.relay_id),
    }
}

fn tote_payload_label(payload: TotePayloadState) -> &'static str {
    match payload {
        TotePayloadState::Empty => "Empty",
        TotePayloadState::NonEmpty => "Non-empty",
        TotePayloadState::Unknown => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use desgraph::RelayId;
    use dessim::events::{TotePayloadState, VtmCycleDirection, VtmIdleReasonKind, VtmOperationKind};
    use dessim::metrics::reporting::{
        FixedPeriod, MetricScope, ReportPeriod, ReportRequest, da50_state_report, da50_throughput_report,
    };
    use dessim::model::RackId;
    use dessim::observer::output::{Activity, VtmJournalEndpoint, VtmLegRecord, VtmOperationRecord};
    use rust_xlsxwriter::Workbook;
    use zip::ZipArchive;

    use super::{write_da50_state_sheet, write_da50_throughput_sheet};

    #[test]
    fn da50_throughput_sheet_charts_the_canonical_hourly_table() {
        let records = [
            VtmOperationRecord {
                sim_time_ms: 1_000,
                vtm_id: 1,
                vtm_cycle_id: 10,
                zone_id: 3,
                kind: VtmOperationKind::PickupStorage,
                rack_id: 4,
                rack_level: Some(0),
                relay_id: 5,
                tote_id: 6,
                payload: TotePayloadState::Empty,
                presentation_id: Some(7),
            },
            VtmOperationRecord {
                sim_time_ms: 3_600_001,
                vtm_id: 1,
                vtm_cycle_id: 11,
                zone_id: 3,
                kind: VtmOperationKind::DropRelay,
                rack_id: 4,
                rack_level: Some(0),
                relay_id: 5,
                tote_id: 6,
                payload: TotePayloadState::NonEmpty,
                presentation_id: Some(8),
            },
        ];
        let overall = da50_throughput_report(
            &records,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 7_200_000, 0),
        )
        .unwrap();
        let post_embargo = da50_throughput_report(
            &records,
            ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 7_200_000, 3_600_000),
        )
        .unwrap();
        let hourly_throughput = da50_throughput_report(
            &records,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), 7_200_000, 0),
        )
        .unwrap();

        let mut workbook = Workbook::new();
        write_da50_throughput_sheet(&mut workbook, &overall, &post_embargo, &hourly_throughput).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing
        // left behind on disk when an assertion below fails.
        let buf = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"DA50 Throughput\""));

        let mut chart_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut chart_xml).unwrap();
        assert!(chart_xml.contains("<c:lineChart>"));
        assert!(chart_xml.contains("'DA50 Throughput'!$A$2:$A$3"));
        assert!(chart_xml.contains("'DA50 Throughput'!$B$2:$B$3"));
        assert!(chart_xml.contains("'DA50 Throughput'!$E$2:$E$3"));

        drop(archive);
    }

    #[test]
    fn da50_state_sheet_contains_pies_and_percent_stacked_hourly_chart() {
        let legs = [
            VtmLegRecord {
                start_ms: 0,
                duration_ms: 1_000,
                activity: Activity::Travel,
                from: VtmJournalEndpoint::Aisle,
                to: VtmJournalEndpoint::Rack(RackId(1)),
                from_zone_id: None,
                to_zone_id: None,
                distance_mm: 500,
                vtm_id: 1,
                vtm_cycle_id: Some(1),
                tote_id: None,
                presentation_id: Some(1),
                direction: VtmCycleDirection::Forward,
                carrying_tote: false,
                wait_reason: None,
                notes: String::new(),
            },
            VtmLegRecord {
                start_ms: 3_600_000,
                duration_ms: 1_000,
                activity: Activity::Wait,
                from: VtmJournalEndpoint::Relay(RelayId(2)),
                to: VtmJournalEndpoint::Relay(RelayId(2)),
                from_zone_id: Some(3),
                to_zone_id: Some(3),
                distance_mm: 0,
                vtm_id: 1,
                vtm_cycle_id: Some(2),
                tote_id: None,
                presentation_id: Some(2),
                direction: VtmCycleDirection::Forward,
                carrying_tote: false,
                wait_reason: Some(VtmIdleReasonKind::AwaitingSlot),
                notes: String::new(),
            },
        ];
        let full = da50_state_report(
            &legs,
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 7_200_000, 0),
        )
        .unwrap();
        let post_embargo = da50_state_report(
            &legs,
            1,
            ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 7_200_000, 3_600_000),
        )
        .unwrap();
        let hourly = da50_state_report(
            &legs,
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), 7_200_000, 0),
        )
        .unwrap();

        let mut workbook = Workbook::new();
        write_da50_state_sheet(&mut workbook, &full, &post_embargo, &hourly).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing
        // left behind on disk when an assertion below fails.
        let buf = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"DA50 State\""));

        let mut full_pie_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut full_pie_xml).unwrap();
        assert!(full_pie_xml.contains("<c:pieChart>"));
        assert!(full_pie_xml.contains("'DA50 State'!$A$3:$A$11"));
        assert!(full_pie_xml.contains("'DA50 State'!$C$3:$C$11"));

        let mut post_pie_xml = String::new();
        archive.by_name("xl/charts/chart2.xml").unwrap().read_to_string(&mut post_pie_xml).unwrap();
        assert!(post_pie_xml.contains("<c:pieChart>"));

        let mut hourly_xml = String::new();
        archive.by_name("xl/charts/chart3.xml").unwrap().read_to_string(&mut hourly_xml).unwrap();
        assert!(hourly_xml.contains("<c:barChart>"));
        assert!(hourly_xml.contains("<c:grouping val=\"percentStacked\"/>"));
        assert!(hourly_xml.contains("'DA50 State'!$B$39:$B$40"));
        assert!(hourly_xml.contains("'DA50 State'!$E$39:$E$40"));

        drop(archive);
    }
}
