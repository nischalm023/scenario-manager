//! `workbook` is THE RUN AS AN XLSX: one finished run rendered as the
//! spreadsheet people read it in — raw fact-log sheets plus canonical report
//! sheets with native charts.
//!
//! **What it is for.** Everyone who does not run the simulator meets a run
//! through this file, so the sheets are the run's public face and have to say
//! the same thing every time. The crate computes nothing canonical: builders
//! in `dessim::metrics::reporting` own every value, and these modules own only
//! which sheet a value lands on, where, and in what number format. It reads
//! the run model (`dessim::metrics::profile`) and knows nothing about desvis,
//! so both writers of a workbook — the `workbook` command over a `.worldview`
//! recording, and desvis on a run's completion — go through [`write_xlsx`] and
//! produce the same book.
//!
//! This file owns the workbook skeleton: [`XlsxData`], the [`build_workbook`]
//! assembler and its [`write_xlsx`] persist wrapper, shared formatting/label
//! helpers, the workbook-scope sheets
//! (the Metric Descriptors dictionary and Throughput), and the
//! family-neutral raw fact-log dump sheets. The report families render in
//! the child modules `da50`, `d30`, `occupancy`, `completed_work`, and `extraction`; raw
//! dumps with family vocabulary (the DA50 operations journal, Completed
//! Tote Work) live with their families. Header arrays for raw-log sheets
//! migrate to descriptor-driven rendering under issue #300.
//!
//! TODO(overstuffed): this file does two jobs — it is the crate's skeleton
//! (the inputs, the assembler, the shared format and label helpers) and it is
//! also the sheets that belong to no family (Cover, Configuration, Summary,
//! Metric Descriptors, Time Series, Throughput and every raw fact-log dump).
//! Splitting it during the move out of desvis would have destroyed the rename
//! the round's parity rests on; the second job wants a module of its own.

use std::collections::BTreeSet;
use std::path::Path;

use dessim::events::{ToteSource, VtmCycleDirection, VtmIdleReasonKind};
use dessim::metrics::profile::{PerfProfile, ScenarioSource, SliceRecord, SummaryStats};
use dessim::metrics::reporting::{
    AggregationClass, ExtractionData, FixedPeriod, MetricDescriptor, MetricScope, MetricUnit, MetricsReport,
    ReportPeriod, ReportRequest, SupportStatus, TimeBucket, aisle_occupancy_report, completed_work_report,
    d30_state_report, da50_state_report, da50_throughput_report, extraction_report, trailing_slice_sum,
};
use dessim::model::OrderSpec as Order;
use dessim::observer::output::{
    Activity, AisleFluxRecord, HtmJournalEndpoint, HtmLegRecord, HtmWaitCause, HtmWaitPosition, HtmWaitReason,
    PickRecord, PutawayRecord, TripType, VtmCycleRecord, VtmJournalEndpoint, VtmLegRecord,
};
use dessim::scenario_gen::SkuInfo;
use dessim::{ScenarioConfig, SimConfig};
use native_modules as _;
use rust_xlsxwriter::{Chart, ChartType, Format, Workbook, Worksheet, XlsxError};

mod completed_work;
// desvis renders the same D30 flat-state list in its metrics panel and pins
// the two against each other, so the module the sheet builds it in is public.
pub mod d30;
mod da50;
mod extraction;
mod occupancy;

use completed_work::{write_aisle_station_sheet, write_completed_tote_work_sheet, write_dc_station_sheet};
use d30::{write_d30_distance_sheet, write_d30_state_sheet};
use da50::{
    write_da50_distance_sheet, write_da50_operations_sheet, write_da50_state_sheet, write_da50_throughput_sheet,
};
use extraction::write_extraction_sheets;
use occupancy::write_aisle_occupancy_sheet;

/// All the data needed to produce an xlsx export.
pub struct XlsxData<'a> {
    /// Map file name or run label.
    pub name: &'a str,
    /// Git branch.
    pub branch: &'a str,
    /// Git commit (full hash).
    pub commit: &'a str,
    /// Tracked working-tree changes vs HEAD at run time.
    pub dirty: bool,
    /// Hostname of the machine that ran the sim.
    pub hostname: &'a str,
    /// Run timestamp.
    pub timestamp: &'a str,
    /// Simulation config.
    pub config: &'a SimConfig,
    /// Resolved VTM fleet size, derived from the map (sum of per-zone
    /// `vtm_count`) — VTM count is not a SimConfig parameter.
    pub num_vtms: usize,
    /// Scenario source (for cover sheet metadata).
    pub scenario: Option<&'a ScenarioSource>,
    /// Resolved SKU data (from any scenario source).
    pub skus: Option<&'a [SkuInfo]>,
    /// Resolved order data (from any scenario source).
    pub orders: Option<&'a [Order]>,
    /// Summary statistics.
    pub summary: &'a SummaryStats,
    /// Per-slice time series.
    pub time_series: &'a [SliceRecord],
    /// Slice width in ms.
    pub slice_width_ms: u64,
}

impl<'a> XlsxData<'a> {
    /// The workbook's inputs taken straight off a finished run.
    ///
    /// Everything the sheets read is already in the run model, so a caller
    /// holding a run — the `workbook` command over a recording, desvis at the
    /// end of a headless run — never assembles the fields by hand and the two
    /// cannot drift. A headed run has no run model until it finishes, so it
    /// fills the fields itself.
    pub fn from_profile(profile: &'a PerfProfile) -> Self {
        XlsxData {
            name: &profile.name,
            branch: &profile.metadata.branch,
            commit: &profile.metadata.commit,
            dirty: profile.metadata.dirty,
            hostname: &profile.metadata.hostname,
            timestamp: &profile.metadata.timestamp,
            config: &profile.sim_config,
            num_vtms: profile.vtm_fleet.into(),
            scenario: Some(&profile.scenario),
            // A loaded scenario IS its lists, so the run carries no second
            // copy and the sheets read them where they are.
            skus: Some(match &profile.scenario {
                ScenarioSource::Loaded { skus, .. } => skus,
                ScenarioSource::Parametric(_) => &profile.workload.skus,
            }),
            orders: Some(match &profile.scenario {
                ScenarioSource::Loaded { orders, .. } => orders,
                ScenarioSource::Parametric(_) => &profile.workload.orders,
            }),
            summary: &profile.summary,
            time_series: &profile.time_slices,
            slice_width_ms: profile.sim_config.slice_width_ms,
        }
    }
}

/// Write simulation metrics to an xlsx file.
pub fn write_xlsx(path: &Path, data: &XlsxData) -> Result<(), XlsxError> {
    let mut wb = build_workbook(data)?;
    wb.save(path)?;
    eprintln!("Wrote xlsx: {}", path.display());
    Ok(())
}

/// Where the run's facts stop, which every report sheet's window ends at.
///
/// The summary says how long the run was — except for a recording of a run
/// that died, which has no summary at all. The slices then say it instead, so
/// a crashed run yields report sheets covering the slices it got through
/// rather than empty ones over the window `[0, 0)`.
fn run_end_ms(data: &XlsxData) -> u64 {
    let stated = (data.summary.sim_time_secs * 1000.0).round() as u64;
    if stated > 0 {
        return stated;
    }
    match data.time_series {
        [.., before, last] => last.start_ms + (last.start_ms - before.start_ms),
        [only] => only.start_ms + data.slice_width_ms,
        [] => 0,
    }
}

/// Assemble the full workbook in memory. Split from [`write_xlsx`] so tests
/// can assert on the produced sheet set without touching disk.
pub fn build_workbook(data: &XlsxData) -> Result<Workbook, XlsxError> {
    let run_end_ms = run_end_ms(data);
    let embargo_ms = (data.config.pick.embargo_secs * 1000.0).round() as u64;
    let fleet_size = u32::try_from(data.num_vtms)
        .map_err(|_| XlsxError::ParameterError("DA50 fleet size exceeds u32".to_string()))?;
    let full_throughput = da50_throughput_report(
        &data.summary.vtm_operation_log,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let post_embargo_throughput = da50_throughput_report(
        &data.summary.vtm_operation_log,
        ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let hourly_throughput = da50_throughput_report(
        &data.summary.vtm_operation_log,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let full_state = da50_state_report(
        &data.summary.vtm_leg_log,
        fleet_size,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let post_embargo_state = da50_state_report(
        &data.summary.vtm_leg_log,
        fleet_size,
        ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let hourly_state = da50_state_report(
        &data.summary.vtm_leg_log,
        fleet_size,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    // D30 fleet size is a SimConfig parameter, unlike the map-derived DA50
    // count carried in `XlsxData::num_vtms`.
    let htm_fleet_size = u32::try_from(data.config.num_htms)
        .map_err(|_| XlsxError::ParameterError("D30 fleet size exceeds u32".to_string()))?;
    let full_d30_state = d30_state_report(
        &data.summary.htm_leg_log,
        htm_fleet_size,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let post_embargo_d30_state = d30_state_report(
        &data.summary.htm_leg_log,
        htm_fleet_size,
        ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let hourly_d30_state = d30_state_report(
        &data.summary.htm_leg_log,
        htm_fleet_size,
        ReportRequest::for_run(
            MetricScope::FullRun,
            ReportPeriod::Fixed(
                FixedPeriod::new(3_600_000, 0).map_err(|error| XlsxError::ParameterError(error.to_string()))?,
            ),
            run_end_ms,
            embargo_ms,
        ),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let full_aisle = aisle_occupancy_report(
        &data.summary.aisle_state_log,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let post_embargo_aisle = aisle_occupancy_report(
        &data.summary.aisle_state_log,
        ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let hourly_aisle = aisle_occupancy_report(
        &data.summary.aisle_state_log,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let full_work = completed_work_report(
        &data.summary.pick_log,
        &data.summary.completed_order_log,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let post_embargo_work = completed_work_report(
        &data.summary.pick_log,
        &data.summary.completed_order_log,
        ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;

    let full_extraction = extraction_report(
        &data.summary.log_streams,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let post_embargo_extraction = extraction_report(
        &data.summary.log_streams,
        ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;
    let hourly_extraction = extraction_report(
        &data.summary.log_streams,
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::HOURLY), run_end_ms, embargo_ms),
    )
    .map_err(|error| XlsxError::ParameterError(error.to_string()))?;

    let mut wb = Workbook::new();
    write_cover_sheet(&mut wb, data)?;
    let mut workbook_descriptors: Vec<&MetricDescriptor> = Vec::new();
    for report in [&full_throughput, &post_embargo_throughput, &hourly_throughput] {
        workbook_descriptors.extend(report.descriptors.iter());
    }
    for report in [&full_state, &post_embargo_state, &hourly_state] {
        workbook_descriptors.extend(report.descriptors.iter());
    }
    for report in [&full_d30_state, &post_embargo_d30_state, &hourly_d30_state] {
        workbook_descriptors.extend(report.descriptors.iter());
    }
    for report in [&full_aisle, &post_embargo_aisle, &hourly_aisle] {
        workbook_descriptors.extend(report.descriptors.iter());
    }
    for report in [&full_work, &post_embargo_work] {
        workbook_descriptors.extend(report.descriptors.iter());
    }
    for report in [&full_extraction, &post_embargo_extraction, &hourly_extraction] {
        workbook_descriptors.extend(report.descriptors.iter());
    }
    write_metric_descriptors_sheet(&mut wb, &workbook_descriptors)?;
    write_summary_sheet(&mut wb, data.summary, &full_extraction)?;
    write_time_series_sheet(&mut wb, data.time_series, data.slice_width_ms)?;
    write_throughput_sheet(&mut wb, data.time_series, data.slice_width_ms, run_end_ms)?;
    write_picks_sheet(&mut wb, &data.summary.pick_log, run_end_ms)?;
    write_putaways_sheet(&mut wb, &data.summary.putaway_log, run_end_ms)?;
    write_putaway_stores_sheet(
        &mut wb,
        &data.summary.vtm_cycle_log,
        &data.summary.putaway_log,
        (data.summary.sim_time_secs * 1000.0) as u64,
    )?;
    write_htm_legs_sheet(&mut wb, &data.summary.htm_leg_log)?;
    write_vtm_cycles_sheet(&mut wb, &data.summary.vtm_cycle_log)?;
    write_da50_operations_sheet(&mut wb, &data.summary.vtm_operation_log)?;
    write_da50_state_sheet(&mut wb, &full_state, &post_embargo_state, &hourly_state)?;
    write_da50_throughput_sheet(&mut wb, &full_throughput, &post_embargo_throughput, &hourly_throughput)?;
    write_da50_distance_sheet(&mut wb, &full_state, &post_embargo_state, &hourly_state)?;
    write_d30_state_sheet(&mut wb, &full_d30_state, &post_embargo_d30_state, &hourly_d30_state)?;
    write_d30_distance_sheet(&mut wb, &full_d30_state, &post_embargo_d30_state, &hourly_d30_state)?;
    write_aisle_occupancy_sheet(&mut wb, &full_aisle, &post_embargo_aisle, &hourly_aisle)?;
    write_completed_tote_work_sheet(&mut wb, &data.summary.pick_log, &data.summary.completed_order_log)?;
    write_aisle_station_sheet(&mut wb, &full_work, &post_embargo_work)?;
    write_dc_station_sheet(&mut wb, &full_work, &post_embargo_work)?;
    write_extraction_sheets(
        &mut wb,
        &full_extraction,
        &post_embargo_extraction,
        &hourly_extraction,
        &data.summary.log_streams,
    )?;
    write_vtm_legs_sheet(&mut wb, &data.summary.vtm_leg_log)?;
    write_rack_sheet(&mut wb, data.summary, data.time_series)?;
    write_inventory_delta_sheet(&mut wb, &data.summary.aisle_flux_log)?;
    if data.orders.is_some() || data.skus.is_some() {
        write_resolved_data_sheets(&mut wb, data.skus, data.orders)?;
    }
    Ok(wb)
}

fn bold() -> Format {
    Format::new().set_bold()
}

fn line_chart_for_columns(
    sheet_name: &str,
    title: &str,
    y_axis_label: &str,
    first_data_row: u32,
    last_data_row: u32,
    category_column: u16,
    value_columns: &[u16],
    x_axis_label: &str,
) -> Option<Chart> {
    if first_data_row > last_data_row || value_columns.is_empty() {
        return None;
    }

    let mut chart = Chart::new(ChartType::Line);
    for &value_column in value_columns {
        chart
            .add_series()
            .set_categories((sheet_name, first_data_row, category_column, last_data_row, category_column))
            .set_values((sheet_name, first_data_row, value_column, last_data_row, value_column))
            .set_name((sheet_name, first_data_row.saturating_sub(1), value_column));
    }
    chart.title().set_name(title);
    chart.x_axis().set_name(x_axis_label);
    chart.x_axis().set_num_format("0.00");
    chart.y_axis().set_name(y_axis_label);
    chart.set_style(10);
    Some(chart)
}

/// Display floats with up to two decimal places; integers render without a
/// decimal point. Underlying values are unchanged — this only affects display.
fn num_fmt() -> Format {
    Format::new().set_num_format("0.##")
}

/// Fractional elapsed hours with two decimals, for hour-axis cells that feed
/// chart categories. Distinct from `hour_index` (whole-hour bucketing).
fn hour_fmt() -> Format {
    Format::new().set_num_format("0.00")
}

/// Render a seconds value as a compact human-friendly duration: `"1h 30m"`,
/// `"10m"`, `"45s"`, `"2.25s"`. Sub-minute values keep up to two decimals so
/// fractional timings (a 2.25s cycle, a 1s rotate) survive; minute-plus values
/// round to whole seconds and drop empty components. Display only — the raw
/// value stays in the adjacent numeric cell.
fn fmt_duration_secs(secs: f64) -> String {
    if secs <= 0.0 {
        return "0s".to_string();
    }
    if secs < 60.0 {
        let s = format!("{secs:.2}");
        return format!("{}s", s.trim_end_matches('0').trim_end_matches('.'));
    }
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    if s > 0 {
        parts.push(format!("{s}s"));
    }
    parts.join(" ")
}

/// Seconds as raw value plus a human-friendly form when they differ, e.g.
/// `"120s (2m)"`; sub-minute values that gain nothing render as just `"45s"`.
/// For string-context rows (e.g. `Option` settings) where the numeric column
/// can't carry the raw value on its own.
fn fmt_secs_labeled(secs: f64) -> String {
    let human = fmt_duration_secs(secs);
    let raw = format!("{secs:.2}");
    let raw = format!("{}s", raw.trim_end_matches('0').trim_end_matches('.'));
    if human == raw { raw } else { format!("{raw} ({human})") }
}

/// 0-based elapsed-hour index for an absolute sim time : the whole
/// hour the event falls in (90 min -> 1). Distinct from fractional "hours
/// elapsed" — use for per-row Hour columns, never for durations.
fn hour_index(sim_time_ms: u64) -> f64 {
    (sim_time_ms / 3_600_000) as f64
}

/// Zero-based index of the fixed period a bucket occupies, labeled by its
/// start edge. For hourly reports, `Hour = k` names `[k, k + 1)`.
fn bucket_hour(bucket: &TimeBucket, nominal_width_ms: u64) -> f64 {
    (bucket.start_ms / nominal_width_ms) as f64
}

/// Fraction of a fixed period covered by a bucket. A complete hour is 1.0;
/// scope-clipped and ragged final hours are explicit fractions instead of a
/// lossy boolean `Partial` flag.
fn bucket_completeness(bucket: &TimeBucket, nominal_width_ms: u64) -> f64 {
    bucket.coverage_ms as f64 / nominal_width_ms as f64
}

fn completeness_fmt() -> Format {
    Format::new().set_num_format("0.00")
}

/// Metric-level display label sourced from the report's own descriptors, so
/// sheet titles cannot drift from what the producer declared. Falls back to
/// the id when the report doesn't carry it (never fabricates a name).
fn metric_label<'a>(descriptors: &'a [MetricDescriptor], id: &'a str) -> &'a str {
    descriptors.iter().find(|descriptor| descriptor.id == id).map(|descriptor| descriptor.label.as_str()).unwrap_or(id)
}

fn support_status_label(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::Measured => "Measured",
        SupportStatus::Unknown => "Unknown",
        SupportStatus::Unsupported => "Unsupported",
    }
}

/// Data dictionary: every metric descriptor attached to a report actually
/// written into this workbook, deduplicated by id. Discovery over
/// enumeration — the sheet reflects what this workbook's producers emitted,
/// never a hand-maintained catalog.
fn write_metric_descriptors_sheet(wb: &mut Workbook, descriptors: &[&MetricDescriptor]) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Metric Descriptors")?;
    let b = bold();
    for (column, header) in ["ID", "Label", "Description", "Unit", "Denominator", "Aggregation"].iter().enumerate() {
        ws.write_string_with_format(0, column as u16, *header, &b)?;
    }
    let mut seen = BTreeSet::new();
    let mut row = 1;
    for descriptor in descriptors {
        if !seen.insert(descriptor.id.as_str()) {
            continue;
        }
        ws.write_string(row, 0, &descriptor.id)?;
        ws.write_string(row, 1, &descriptor.label)?;
        ws.write_string(row, 2, &descriptor.description)?;
        ws.write_string(row, 3, metric_unit_label(descriptor.semantics.unit))?;
        if let Some(denominator) = &descriptor.semantics.denominator_description {
            ws.write_string(row, 4, denominator)?;
        }
        ws.write_string(row, 5, aggregation_label(descriptor.semantics.aggregation))?;
        row += 1;
    }
    ws.set_column_width(0, 34)?;
    ws.set_column_width(1, 30)?;
    ws.set_column_width(2, 72)?;
    ws.set_column_range_width(3, 5, 26)?;
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

/// Typed display of the unit vocabulary; exhaustive so a new unit variant is
/// a compile-time forcing function here, mirroring the taxonomy guards.
fn metric_unit_label(unit: MetricUnit) -> &'static str {
    match unit {
        MetricUnit::Count => "Count",
        MetricUnit::DurationMilliseconds => "Duration (ms)",
        MetricUnit::DistanceMillimeters => "Distance (mm)",
        MetricUnit::Ratio => "Ratio",
        MetricUnit::RatePerHour => "Rate per hour",
    }
}

fn aggregation_label(aggregation: AggregationClass) -> &'static str {
    match aggregation {
        AggregationClass::Additive => "Additive",
        AggregationClass::Interval => "Interval",
        AggregationClass::Ratio => "Ratio of summed components",
        AggregationClass::SnapshotGauge => "Snapshot gauge",
        AggregationClass::DistinctCount => "Distinct count",
    }
}

/// The `Throughput` sheet: trailing 60-minute pick/putaway completion totals
/// at 5-minute resolution, over the coarse general time-series facts.
///
/// Uses the shared `trailing_slice_sum` reporting helper — the same code path
/// Desvis charts use — so the workbook and UI cannot diverge on
/// leading-window suppression or output-step semantics.
fn write_throughput_sheet(
    wb: &mut Workbook,
    series: &[SliceRecord],
    slice_width_ms: u64,
    end_ms: u64,
) -> Result<(), XlsxError> {
    const REQUESTED_WINDOW_MS: u64 = 60 * 60 * 1_000;
    const REQUESTED_STEP_MS: u64 = 5 * 60 * 1_000;

    let ws = wb.add_worksheet();
    ws.set_name("Throughput")?;
    let b = bold();
    ws.write_string_with_format(0, 0, "Trailing-window completion totals", &b)?;
    ws.write_string(1, 0, "Requested window (ms)")?;
    ws.write_number(1, 1, REQUESTED_WINDOW_MS as f64)?;
    ws.write_string(2, 0, "Requested step (ms)")?;
    ws.write_number(2, 1, REQUESTED_STEP_MS as f64)?;
    ws.write_string(3, 0, "Alignment")?;
    ws.write_string(3, 1, "Simulation start")?;
    ws.write_string(4, 0, "Source precision")?;
    ws.write_string(4, 1, "Coarse slice-aligned values; requested durations round up to complete source slices")?;

    let header_row = 6;
    for (column, header) in ["End hour", "Picks completed", "Inducted units", "Putaway totes stored"].iter().enumerate()
    {
        ws.write_string_with_format(header_row, column as u16, *header, &b)?;
    }

    if slice_width_ms == 0 {
        ws.write_string(header_row + 1, 0, "Unavailable: source slice width is zero")?;
        return Ok(());
    }
    let series = &series[..series.partition_point(|row| row.start_ms.saturating_add(slice_width_ms) <= end_ms)];

    let window_slices = usize::try_from(REQUESTED_WINDOW_MS.div_ceil(slice_width_ms))
        .map_err(|_| XlsxError::ParameterError("trailing window exceeds platform size".to_string()))?;
    let step_slices = usize::try_from(REQUESTED_STEP_MS.div_ceil(slice_width_ms))
        .map_err(|_| XlsxError::ParameterError("trailing step exceeds platform size".to_string()))?;
    let applied_window_ms = (window_slices as u64).saturating_mul(slice_width_ms);
    let applied_step_ms = (step_slices as u64).saturating_mul(slice_width_ms);
    ws.write_string(1, 2, "Applied window (ms)")?;
    ws.write_number(1, 3, applied_window_ms as f64)?;
    ws.write_string(2, 2, "Applied step (ms)")?;
    ws.write_number(2, 3, applied_step_ms as f64)?;

    let picks: Vec<_> =
        series.iter().map(|slice| slice.metrics.get("picks_completed").copied().unwrap_or(0.0)).collect();
    let putaways: Vec<_> =
        series.iter().map(|slice| slice.metrics.get("putaways_completed").copied().unwrap_or(0.0)).collect();
    let stores: Vec<_> =
        series.iter().map(|slice| slice.metrics.get("putaway_totes_stored").copied().unwrap_or(0.0)).collect();
    let store_points = trailing_slice_sum(&stores, window_slices, step_slices).unwrap_or_default();
    let pick_points = trailing_slice_sum(&picks, window_slices, step_slices).unwrap_or_default();
    let putaway_points = trailing_slice_sum(&putaways, window_slices, step_slices).unwrap_or_default();

    for (index, (pick_point, putaway_point)) in pick_points.iter().zip(&putaway_points).enumerate() {
        let row = header_row + 1 + index as u32;
        let end_ms = series[pick_point.end_index].start_ms.saturating_add(slice_width_ms);
        ws.write_number_with_format(row, 0, end_ms as f64 / 3_600_000.0, &hour_fmt())?;
        ws.write_number(row, 1, pick_point.sum)?;
        ws.write_number(row, 2, putaway_point.sum)?;
        ws.write_number(row, 3, store_points[index].sum)?;
    }

    if pick_points.is_empty() {
        ws.write_string(header_row + 1, 0, "No point emitted: the run does not contain one complete default window")?;
    } else if let Some(chart) = line_chart_for_columns(
        "Throughput",
        "Trailing 60-minute completion totals at 5-minute resolution",
        "Completed units",
        header_row + 1,
        header_row + pick_points.len() as u32,
        0,
        &[1, 2],
        "Simulation time (h)",
    ) {
        // Column 5 clears the metadata block's widest row (labels+values
        // reach column D); anchored at the table's header row.
        ws.insert_chart(header_row, 5, &chart)?;
    }

    // Wide enough for the metadata labels sharing column A ("Requested
    // window (ms)") — Excel clips, not overflows, when B is occupied.
    ws.set_column_width(0, 22)?;
    ws.set_column_range_width(1, 2, 20)?;
    ws.set_freeze_panes(header_row + 1, 0)?;
    Ok(())
}

fn write_cover_sheet(wb: &mut Workbook, data: &XlsxData) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Cover")?;
    let b = bold();
    let nf = num_fmt();
    let mut r = 0u32;

    let hdr = |ws: &mut Worksheet, r: &mut u32, label: &str, val: &str| -> Result<(), XlsxError> {
        ws.write_string_with_format(*r, 0, label, &b)?;
        ws.write_string(*r, 1, val)?;
        *r += 1;
        Ok(())
    };
    let num = |ws: &mut Worksheet, r: &mut u32, label: &str, val: f64, unit: &str| -> Result<(), XlsxError> {
        ws.write_string_with_format(*r, 0, label, &b)?;
        ws.write_number_with_format(*r, 1, val, &nf)?;
        if !unit.is_empty() {
            ws.write_string(*r, 2, unit)?;
        }
        *r += 1;
        Ok(())
    };
    let boolean = |ws: &mut Worksheet, r: &mut u32, label: &str, val: bool| -> Result<(), XlsxError> {
        ws.write_string_with_format(*r, 0, label, &b)?;
        ws.write_boolean(*r, 1, val)?;
        *r += 1;
        Ok(())
    };
    // A seconds-valued row: the raw value stays in the numeric cell (Excel-
    // usable) with a human-friendly duration alongside it (e.g. 3600 → "1h").
    let dur = |ws: &mut Worksheet, r: &mut u32, label: &str, secs: f64| -> Result<(), XlsxError> {
        ws.write_string_with_format(*r, 0, label, &b)?;
        ws.write_number_with_format(*r, 1, secs, &nf)?;
        ws.write_string(*r, 2, &fmt_duration_secs(secs))?;
        *r += 1;
        Ok(())
    };
    let section = |ws: &mut Worksheet, r: &mut u32, title: &str| -> Result<(), XlsxError> {
        *r += 1;
        ws.write_string_with_format(*r, 0, title, &b)?;
        *r += 1;
        Ok(())
    };

    hdr(ws, &mut r, "Run Name", data.name)?;
    hdr(ws, &mut r, "Branch", data.branch)?;
    // Mirror `git describe --dirty`: a `-dirty` suffix marks a run whose code
    // diverged from the recorded commit, so the hash can't be trusted alone.
    let commit = if data.dirty { format!("{}-dirty", data.commit) } else { data.commit.to_string() };
    hdr(ws, &mut r, "Commit", &commit)?;
    hdr(ws, &mut r, "Hostname", data.hostname)?;
    hdr(ws, &mut r, "Timestamp", data.timestamp)?;
    r += 1;

    ws.write_string_with_format(r, 0, "Simulation Config", &b)?;
    r += 1;
    num(ws, &mut r, "Sim Duration", data.config.sim_duration.as_secs_f64() / 3600.0, "h")?;
    dur(ws, &mut r, "Slice Width", data.slice_width_ms as f64 / 1000.0)?;
    num(ws, &mut r, "VTMs", data.num_vtms as f64, "")?;
    num(ws, &mut r, "HTMs", data.config.num_htms as f64, "")?;
    num(ws, &mut r, "RTPs", data.config.num_rtps as f64, "")?;
    num(ws, &mut r, "Nav Factor", data.config.nav_factor, "")?;
    num(ws, &mut r, "Queue Factor", data.config.queue_factor, "")?;
    hdr(ws, &mut r, "Fidelity", &format!("{:?}", data.config.fidelity))?;
    boolean(ws, &mut r, "HTM-Only", data.config.htm_only)?;
    boolean(ws, &mut r, "Watchdog Disabled", data.config.no_watchdog)?;
    boolean(ws, &mut r, "Tripwires", data.config.tripwires)?;
    num(ws, &mut r, "RNG Seed", data.config.rng_seed as f64, "")?;
    num(ws, &mut r, "Threads", data.config.num_threads as f64, "")?;
    if let Some(wt) = data.config.wall_timeout_secs {
        dur(ws, &mut r, "Wall Timeout", wt)?;
    }
    if let Some(mo) = data.config.max_orders {
        num(ws, &mut r, "Max Orders", mo as f64, "")?;
    }

    // These are preserved settings, not the catalog's resolved robot models.
    // Keep their legacy meaning explicit until per-model run facts are recorded.
    section(ws, &mut r, "Legacy Motion Settings")?;
    hdr(ws, &mut r, "Scope", "Catalog model physics resolve separately; these values are legacy settings.")?;
    hdr(ws, &mut r, "Legacy Motion Model", &format!("{:?}", data.config.motion_model))?;
    num(ws, &mut r, "Legacy Grid Speed", data.config.vtm.speed, "m/s")?;
    num(ws, &mut r, "HTM Loaded v_max", data.config.htm.loaded_speed, "m/s")?;
    num(ws, &mut r, "HTM Empty v_max", data.config.htm.empty_speed, "m/s")?;
    num(ws, &mut r, "HTM Loaded Const Speed", data.config.htm.loaded_constant_speed, "m/s")?;
    num(ws, &mut r, "HTM Empty Const Speed", data.config.htm.empty_constant_speed, "m/s")?;
    num(ws, &mut r, "HTM Loaded Accel", data.config.htm.loaded_accel, "m/s²")?;
    num(ws, &mut r, "HTM Empty Accel", data.config.htm.empty_accel, "m/s²")?;
    dur(ws, &mut r, "HTM Loaded Rotate 90°", data.config.htm.loaded_rotate)?;
    dur(ws, &mut r, "HTM Empty Rotate 90°", data.config.htm.empty_rotate)?;

    // VTM isolation diagnostic lever (`Off` in production). `Ideal Cycle` and
    // `Per-Zone Override` only bite under IdealRate / a forced fleet size, but
    // are recorded unconditionally so the sheet reflects the exact config.
    section(ws, &mut r, "VTM Isolation")?;
    hdr(ws, &mut r, "Isolation Mode", &format!("{:?}", data.config.vtm.isolation))?;
    let ideal_cycle = match data.config.vtm.ideal_cycle_secs {
        Some(secs) => fmt_secs_labeled(secs),
        None => "auto (calibrated)".to_string(),
    };
    hdr(ws, &mut r, "Ideal Cycle", &ideal_cycle)?;
    let per_zone = match data.config.vtm.per_zone_override {
        Some(n) => n.to_string(),
        None => "map default".to_string(),
    };
    hdr(ws, &mut r, "Per-Zone Override", &per_zone)?;

    section(ws, &mut r, "Pick & Putaway")?;
    dur(ws, &mut r, "Pick Time", data.config.operator.time_per_pick_secs)?;
    dur(ws, &mut r, "Pallet Swap", data.config.operator.pallet_swap_secs)?;
    dur(ws, &mut r, "Embargo", data.config.pick.embargo_secs)?;
    hdr(ws, &mut r, "Pick Sequencing", data.config.pick.sequencing.label())?;
    hdr(ws, &mut r, "Pick Pace", if data.config.pick.pace_to_station { "station-JIT" } else { "soonest" })?;
    let delivery_max_wait = match data.config.pick.delivery_max_wait_secs {
        Some(secs) => fmt_secs_labeled(secs),
        None => "none".to_string(),
    };
    hdr(ws, &mut r, "Delivery Max Wait", &delivery_max_wait)?;
    boolean(ws, &mut r, "Putaway Enabled", data.config.putaway.enabled)?;
    // Ignored since admission caps became per-station derivations; the row
    // stays so a workbook from an old spec still shows what that spec asked
    // for, marked as not what the run obeyed.
    let queue_depth = match data.config.putaway.queue_depth {
        Some(depth) => format!("{depth} (ignored — caps derived per station)"),
        None => "unset (caps derived per station)".to_string(),
    };
    hdr(ws, &mut r, "Putaway Queue Depth", &queue_depth)?;
    // Auto derives the bias from the map's Fleet Pressure Index at solver
    // init; report that resolved value (carried on the summary) so the
    // effective bias is recoverable, not just the "auto" provenance.
    let direct_bias = match data.config.putaway.direct_bias_secs {
        Some(secs) => format!("{secs:.1}s (fixed)"),
        None => format!("auto (FPI {})", fmt_duration_secs(data.summary.putaway_direct_bias.as_ms() as f64 / 1000.0)),
    };
    hdr(ws, &mut r, "Putaway Direct Bias", &direct_bias)?;
    boolean(ws, &mut r, "Putaway Ignore Hour Gate", data.config.putaway.ignore_hour_gate)?;
    r += 1;

    if let Some(scenario) = data.scenario {
        ws.write_string_with_format(r, 0, "Scenario", &b)?;
        r += 1;
        match scenario {
            ScenarioSource::Parametric(sc) => {
                if let ScenarioConfig::Generated(g) = sc {
                    num(ws, &mut r, "Num SKUs", g.num_skus as f64, "")?;
                    num(ws, &mut r, "Num Orders", g.num_orders as f64, "")?;
                    num(ws, &mut r, "Total Picks", g.total_order_picks as f64, "")?;
                    num(ws, &mut r, "Inventory Units", g.total_inventory_units as f64, "")?;
                } else {
                    hdr(ws, &mut r, "Type", &format!("{:?}", std::mem::discriminant(sc)))?;
                }
            }
            ScenarioSource::Loaded { dir_name, skus, orders } => {
                hdr(ws, &mut r, "Source", dir_name)?;
                num(ws, &mut r, "SKUs", skus.len() as f64, "")?;
                num(ws, &mut r, "Orders", orders.len() as f64, "")?;
                let total_picks: u32 = orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity).sum();
                num(ws, &mut r, "Total Picks", total_picks as f64, "")?;
            }
        }
    }

    ws.set_column_width(0, 20)?;
    ws.set_column_width(1, 30)?;
    Ok(())
}

fn write_summary_sheet(
    wb: &mut Workbook,
    s: &SummaryStats,
    extraction: &MetricsReport<ExtractionData>,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Summary")?;
    let b = bold();
    let nf = num_fmt();
    let mut r = 0u32;

    let kv = |ws: &mut Worksheet, r: &mut u32, label: &str, val: f64, unit: &str| -> Result<(), XlsxError> {
        ws.write_string_with_format(*r, 0, label, &b)?;
        ws.write_number_with_format(*r, 1, val, &nf)?;
        ws.write_string(*r, 2, unit)?;
        *r += 1;
        Ok(())
    };

    kv(ws, &mut r, "Wall Clock", s.wall_clock_secs, "s")?;
    kv(ws, &mut r, "Sim Time", s.sim_time_secs / 3600.0, "h")?;
    kv(ws, &mut r, "Total Picks", s.total_picks as f64, "")?;
    kv(ws, &mut r, "Fully Picked Orders", s.total_orders_completed as f64, "")?;
    kv(ws, &mut r, "Throughput", s.throughput_picks_per_hour, "picks/hr")?;
    let extraction_total = extraction.data.rows.iter().find(|row| row.station_id.is_none());
    for (label, value, unit) in [
        ("Completed Extractions", extraction_total.and_then(|row| row.extractions).map(|value| value as f64), ""),
        ("Extracted Units", extraction_total.and_then(|row| row.extracted_units).map(|value| value as f64), ""),
        ("Extraction Throughput", extraction_total.and_then(|row| row.extractions_per_hour()), "extractions/hr"),
        ("Extracted Unit Throughput", extraction_total.and_then(|row| row.extracted_units_per_hour()), "units/hr"),
    ] {
        if let Some(value) = value {
            kv(ws, &mut r, label, value, unit)?;
        } else {
            ws.write_string_with_format(r, 0, label, &b)?;
            ws.write_string(r, 1, "Unavailable")?;
            r += 1;
        }
    }
    kv(ws, &mut r, "Inducted Units", s.total_putaways as f64, "units")?;
    kv(ws, &mut r, "Induction Throughput", s.throughput_putaway_items_per_hour, "units/hr")?;
    kv(ws, &mut r, "DA50 Idle", s.vtm_idle_pct * 100.0, "%")?;
    kv(ws, &mut r, "D30 Idle", s.htm_idle_pct * 100.0, "%")?;
    kv(ws, &mut r, "Avg Picked Units/Stn/Hr", s.avg_presentations_per_station_per_hour, "")?;
    kv(ws, &mut r, "PF Fail Rate", s.pathfinder_failure_rate * 100.0, "%")?;
    kv(ws, &mut r, "Flow Score", s.flow_score, "")?;
    kv(ws, &mut r, "Events", s.event_count as f64, "")?;

    if !s.station_gap_stats.is_empty() {
        r += 1;
        ws.write_string_with_format(r, 0, "Station Gap Analysis", &b)?;
        r += 1;
        for (c, hdr) in ["Station", "Count", "Min ms", "Mean ms", "P95 ms", "Max ms"].iter().enumerate() {
            ws.write_string_with_format(r, c as u16, *hdr, &b)?;
        }
        r += 1;
        for gs in &s.station_gap_stats {
            ws.write_string(r, 0, format!("Stn {}", gs.station_id.0))?;
            ws.write_number_with_format(r, 1, gs.count as f64, &nf)?;
            ws.write_number_with_format(r, 2, gs.min_ms as f64, &nf)?;
            ws.write_number_with_format(r, 3, gs.mean_ms, &nf)?;
            ws.write_number_with_format(r, 4, gs.p95_ms as f64, &nf)?;
            ws.write_number_with_format(r, 5, gs.max_ms as f64, &nf)?;
            r += 1;
        }
    }

    if !s.presentations_per_station_per_hour.is_empty() {
        r += 1;
        ws.write_string_with_format(r, 0, "Station Throughput", &b)?;
        r += 1;
        ws.write_string_with_format(r, 0, "Station", &b)?;
        ws.write_string_with_format(r, 1, "Picked Units/Hr", &b)?;
        r += 1;
        for &(sid, rate) in &s.presentations_per_station_per_hour {
            ws.write_string(r, 0, format!("Pick station {}", sid.0))?;
            ws.write_number_with_format(r, 1, rate, &nf)?;
            r += 1;
        }
    }

    // Relay-access utilization (oracle-based; same numbers as the headless
    // relay_utilization_pct line). Overall headline plus one row per zone.
    if let Some(ru) = &s.relay_utilization {
        r += 1;
        ws.write_string_with_format(r, 0, "Relay Utilization", &b)?;
        r += 1;
        kv(ws, &mut r, "Overall", ru.overall_pct, "%")?;
        kv(ws, &mut r, "Total Relays", ru.total_relays as f64, "")?;
        let (z_min, z_avg, z_max) = ru.zone_min_avg_max();
        kv(ws, &mut r, "Zone Min", z_min, "%")?;
        kv(ws, &mut r, "Zone Avg", z_avg, "%")?;
        kv(ws, &mut r, "Zone Max", z_max, "%")?;
        ws.write_string_with_format(r, 0, "Zone", &b)?;
        ws.write_string_with_format(r, 1, "Util %", &b)?;
        r += 1;
        for &(zone, pct) in &ru.per_zone {
            ws.write_string(r, 0, format!("Zone {zone}"))?;
            ws.write_number_with_format(r, 1, pct, &nf)?;
            r += 1;
        }
    }

    ws.set_column_width(0, 20)?;
    ws.set_column_width(1, 15)?;
    Ok(())
}

fn write_time_series_sheet(wb: &mut Workbook, series: &[SliceRecord], slice_width_ms: u64) -> Result<(), XlsxError> {
    if series.is_empty() {
        return Ok(());
    }

    let ws = wb.add_worksheet();
    ws.set_name("Time Series")?;
    let b = bold();
    let nf = num_fmt();

    // Stable column order from sorted keys across all slices.
    let all_keys: Vec<String> =
        BTreeSet::from_iter(series.iter().flat_map(|s| s.metrics.keys().cloned())).into_iter().collect();

    // Header row.
    ws.write_string_with_format(0, 0, "Time (min)", &b)?;
    ws.write_string_with_format(0, 1, "Slice Start (ms)", &b)?;
    for (c, key) in all_keys.iter().enumerate() {
        ws.write_string_with_format(0, (c + 2) as u16, key, &b)?;
    }

    // Data rows.
    for (r, slice) in series.iter().enumerate() {
        let row = (r + 1) as u32;
        let time_min = (slice.start_ms as f64 + slice_width_ms as f64 / 2.0) / 60_000.0;
        ws.write_number_with_format(row, 0, time_min, &nf)?;
        ws.write_number(row, 1, slice.start_ms as f64)?;
        for (c, key) in all_keys.iter().enumerate() {
            let val = slice.metrics.get(key).copied().unwrap_or(0.0);
            ws.write_number_with_format(row, (c + 2) as u16, val, &nf)?;
        }
    }

    // First native workbook chart. Later metric-family sheets use the same
    // helper so their plotted ranges and tabular source stay inseparable.
    let throughput_columns: Vec<u16> = ["picks_completed", "putaways_completed"]
        .into_iter()
        .filter_map(|wanted| all_keys.iter().position(|key| key == wanted).map(|index| (index + 2) as u16))
        .collect();
    if let Some(chart) = line_chart_for_columns(
        "Time Series",
        "Simulation throughput by slice",
        "Completed units",
        1,
        series.len() as u32,
        0,
        &throughput_columns,
        "Simulation time (min)",
    ) {
        ws.insert_chart(1, (all_keys.len() + 3) as u16, &chart)?;
    }

    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

/// Per-aisle rack fill summary. One row per aisle (relay zone): tote-slot
/// capacity plus peak / final / mean filled slots and the corresponding fill
/// %, derived from the `rack_occupancy_zone_{id}` columns already in the time
/// series. "Filled" = slots holding a non-empty tote. Peak fill % is the
/// per-aisle "max fill level" KPI.
fn write_rack_sheet(wb: &mut Workbook, summary: &SummaryStats, series: &[SliceRecord]) -> Result<(), XlsxError> {
    if summary.rack_zone_capacity.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("Rack")?;
    let b = bold();
    let nf = num_fmt();
    let headers =
        ["Aisle", "Capacity (slots)", "Peak Filled", "Final Filled", "Mean Filled", "Peak Fill %", "Final Fill %"];
    for (c, h) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *h, &b)?;
    }
    // A slice missing an aisle's key means the aisle held no filled slots then
    // (0) — the running per-zone map only carries touched aisles.
    let peak_of = |v: &[f64]| v.iter().copied().fold(0.0_f64, f64::max);
    let mean_of = |v: &[f64]| {
        if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
    };
    // Final fill % per aisle, collected for the cross-aisle spread summary.
    let mut final_fills: Vec<f64> = Vec::with_capacity(summary.rack_zone_capacity.len());
    for (i, &(zone_id, capacity)) in summary.rack_zone_capacity.iter().enumerate() {
        let row = (i + 1) as u32;
        let key = format!("rack_occupancy_zone_{zone_id}");
        let occ: Vec<f64> = series.iter().map(|s| s.metrics.get(&key).copied().unwrap_or(0.0)).collect();
        let final_occ = occ.last().copied().unwrap_or(0.0);
        let cap = capacity as f64;
        let fill = |n: f64| if cap > 0.0 { 100.0 * n / cap } else { 0.0 };
        final_fills.push(fill(final_occ));
        ws.write_number(row, 0, zone_id as f64)?;
        ws.write_number_with_format(row, 1, cap, &nf)?;
        ws.write_number_with_format(row, 2, peak_of(&occ), &nf)?;
        ws.write_number_with_format(row, 3, final_occ, &nf)?;
        ws.write_number_with_format(row, 4, mean_of(&occ), &nf)?;
        ws.write_number_with_format(row, 5, fill(peak_of(&occ)), &nf)?;
        ws.write_number_with_format(row, 6, fill(final_occ), &nf)?;
    }

    // Cross-aisle spread of final fill % (the "how uneven are the aisles"
    // summary — the band drawn on the chart, quantified here).
    if !final_fills.is_empty() {
        let mean = mean_of(&final_fills);
        let min = final_fills.iter().copied().fold(f64::INFINITY, f64::min);
        let max = final_fills.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let stddev = (final_fills.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / final_fills.len() as f64).sqrt();
        let r = summary.rack_zone_capacity.len() as u32 + 2;
        ws.write_string_with_format(r, 0, "Final fill % across aisles", &b)?;
        for (c, (label, val)) in
            [("Min", min), ("Max", max), ("Mean", mean), ("Std dev", stddev), ("Spread (max−min)", max - min)]
                .into_iter()
                .enumerate()
        {
            ws.write_string_with_format(r + 1, c as u16, label, &b)?;
            ws.write_number_with_format(r + 2, c as u16, val, &nf)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

/// One row per aisle crossing: a tote carried `delta` units of a SKU into (+)
/// or out of (−) an aisle (relay zone), timestamped — the "as it happens"
/// per-aisle inventory flux (e.g. tote 12 carries −40 SKU 7 out of aisle 3,
/// then +37 SKU 7 into aisle 5). Empty totes and same-aisle moves produce no
/// rows, so this sheet is omitted for scenarios without aisle movement.
fn write_inventory_delta_sheet(wb: &mut Workbook, log: &[AisleFluxRecord]) -> Result<(), XlsxError> {
    if log.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("Inventory Delta")?;
    let b = bold();
    for (c, h) in ["Time (ms)", "Hr", "Aisle", "Tote ID", "SKU", "Delta (units)"].iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *h, &b)?;
    }
    // Chronological. Stable sort keeps emission order for equal timestamps
    // (the − leave and + enter of one relocation share a timestamp).
    let mut ordered: Vec<&AisleFluxRecord> = log.iter().collect();
    ordered.sort_by_key(|r| r.sim_time_ms);
    for (i, r) in ordered.iter().enumerate() {
        let row = (i + 1) as u32;
        ws.write_number(row, 0, r.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(r.sim_time_ms))?;
        ws.write_number(row, 2, r.zone_id as f64)?;
        ws.write_number(row, 3, r.tote_id as f64)?;
        ws.write_number(row, 4, r.sku.0 as f64)?;
        ws.write_number(row, 5, r.delta as f64)?;
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_picks_sheet(wb: &mut Workbook, picks: &[PickRecord], end_ms: u64) -> Result<(), XlsxError> {
    if picks.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("Picks")?;
    let b = bold();
    // DC column: blank for pre-formed-order scenarios. Order ID column: blank
    // when the pick carried no tote-presentation context — every live pick
    // presentation carries one, so only legacy or synthetic events go blank.
    for (c, hdr) in
        ["Completion Time (ms)", "Hour", "Station", "SKU", "Quantity", "Order ID", "DC", "Trip ID"].iter().enumerate()
    {
        ws.write_string_with_format(0, c as u16, *hdr, &b)?;
    }
    // Chronological, like every other journal sheet. A stable sort keeps the
    // order the run recorded them in for rows sharing an instant, and leaves
    // the sheet the same whether the rows reached the file one slice at a time
    // or all at the end.
    let mut ordered: Vec<&PickRecord> = picks.iter().filter(|row| row.sim_time_ms <= end_ms).collect();
    ordered.sort_by_key(|row| row.sim_time_ms);
    for (i, p) in ordered.iter().enumerate() {
        let row = (i + 1) as u32;
        ws.write_number(row, 0, p.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(p.sim_time_ms))?;
        ws.write_number(row, 2, p.station_id.0 as f64)?;
        ws.write_number(row, 3, p.sku.0 as f64)?;
        ws.write_number(row, 4, p.quantity as f64)?;
        if let Some(order) = p.order_id {
            ws.write_number(row, 5, order.0 as f64)?;
        }
        if let Some(dc) = &p.dc_id {
            ws.write_string(row, 6, dc)?;
        }
        // Trip ID (col 7): join key to the journey legs.
        if let Some(id) = p.presentation_id {
            ws.write_number(row, 7, id as f64)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_putaways_sheet(wb: &mut Workbook, putaways: &[PutawayRecord], end_ms: u64) -> Result<(), XlsxError> {
    if putaways.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("Putaways")?;
    let b = bold();
    for (c, hdr) in ["Induction Completed (ms)", "Hour", "Station", "SKU", "Quantity", "Trip ID"].iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *hdr, &b)?;
    }
    let mut ordered: Vec<&PutawayRecord> = putaways.iter().filter(|row| row.sim_time_ms <= end_ms).collect();
    ordered.sort_by_key(|row| row.sim_time_ms);
    for (index, p) in ordered.iter().enumerate() {
        let row = index as u32 + 1;
        ws.write_number(row, 0, p.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(p.sim_time_ms))?;
        ws.write_number(row, 2, p.station_id.0 as f64)?;
        ws.write_number(row, 3, p.sku.0 as f64)?;
        ws.write_number(row, 4, p.quantity as f64)?;
        if let Some(id) = p.presentation_id {
            ws.write_number(row, 5, id as f64)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

/// One raw row per completed inbound rack deposit; station fills are a
/// separate milestone in the Putaways sheet.
fn write_putaway_stores_sheet(
    wb: &mut Workbook,
    cycles: &[VtmCycleRecord],
    fills: &[PutawayRecord],
    horizon_ms: u64,
) -> Result<(), XlsxError> {
    let ws = wb.add_worksheet();
    ws.set_name("Putaway Rack Stores")?;
    let b = bold();
    for (col, label) in ["Stored at (ms)", "Hour", "Tote ID", "Aisle", "Rack", "VTM", "Trip ID"].iter().enumerate() {
        ws.write_string_with_format(0, col as u16, *label, &b)?;
    }
    let mut ordered: Vec<&VtmCycleRecord> = cycles
        .iter()
        .filter(|cycle| {
            cycle.direction == VtmCycleDirection::Return
                && cycle.tote_source == ToteSource::Putaway
                && cycle.sim_time_ms + cycle.total_cycle_ms <= horizon_ms
                && cycle.presentation_id.is_some_and(|id| fills.iter().any(|fill| fill.presentation_id == Some(id)))
        })
        .collect();
    // By the store's end, then by its start, so two stores ending in one
    // millisecond read in the order they began.
    ordered.sort_by_key(|cycle| (cycle.sim_time_ms + cycle.total_cycle_ms, cycle.sim_time_ms));
    for (index, cycle) in ordered.iter().enumerate() {
        let row = index as u32 + 1;
        let end = cycle.sim_time_ms + cycle.total_cycle_ms;
        for (col, value) in [
            end as f64,
            hour_index(end),
            cycle.tote_id as f64,
            cycle.zone_id as f64,
            cycle.rack_id as f64,
            cycle.vtm_id as f64,
        ]
        .into_iter()
        .enumerate()
        {
            ws.write_number(row, col as u16, value)?;
        }
        if let Some(id) = cycle.presentation_id {
            ws.write_number(row, 6, id as f64)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

/// Display label for the From/To columns.
fn endpoint_label(endpoint: HtmJournalEndpoint) -> &'static str {
    match endpoint {
        HtmJournalEndpoint::Parking => "Parking",
        HtmJournalEndpoint::Relay(_) => "Relay",
        HtmJournalEndpoint::PickStation(_) => "Pick Station",
        HtmJournalEndpoint::PutawayStation(_) => "Putaway Station",
        HtmJournalEndpoint::ExtractionStation(_) => "Extraction Station",
    }
}

/// Relay id of an endpoint, if it is a relay (else blank in the sheet).
fn endpoint_relay_id(endpoint: HtmJournalEndpoint) -> Option<u32> {
    match endpoint {
        HtmJournalEndpoint::Relay(id) => Some(id.0),
        _ => None,
    }
}

/// Station id of an endpoint, if it is a station (else blank in the sheet).
fn endpoint_station_id(endpoint: HtmJournalEndpoint) -> Option<u32> {
    match endpoint {
        HtmJournalEndpoint::PickStation(id)
        | HtmJournalEndpoint::PutawayStation(id)
        | HtmJournalEndpoint::ExtractionStation(id) => Some(id.0),
        _ => None,
    }
}

/// The trip type implied by a journal endpoint, if it is a station. The
/// `Trip Type` column derives from whichever endpoint is a station (the pickup
/// leg Parking→Relay has none → blank).
fn trip_type_of(endpoint: HtmJournalEndpoint) -> Option<TripType> {
    match endpoint {
        HtmJournalEndpoint::PickStation(_) => Some(TripType::Pick),
        HtmJournalEndpoint::PutawayStation(_) => Some(TripType::Putaway),
        HtmJournalEndpoint::ExtractionStation(_) => Some(TripType::Extraction),
        HtmJournalEndpoint::Parking | HtmJournalEndpoint::Relay(_) => None,
    }
}

fn write_htm_legs_sheet(wb: &mut Workbook, legs: &[HtmLegRecord]) -> Result<(), XlsxError> {
    if legs.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("HTM Journey Legs")?;
    let b = bold();
    let headers = [
        "Start (ms)",
        "Hour",
        "Activity",
        "From",
        "To",
        "Distance (mm)",
        "Duration (ms)",
        "From Relay ID",
        "From Relay Zone",
        "To Relay ID",
        "To Relay Zone",
        "From Station ID",
        "To Station ID",
        "Trip ID",
        "Robot ID",
        "Carrying Tote",
        "Trip Type",
        "Turns",
        "Wait Reason",
        "Hold Cause",
        "Hold Position",
        "Notes",
    ];
    for (c, hdr) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *hdr, &b)?;
    }
    // Chronological journal. Travel rows are pushed in nav-event order, but
    // Wait rows are pushed when a robot *leaves* a waiting state (after the
    // wait's start), so sort by start to interleave them correctly. The run
    // holds its legs in the order it reported them and its recording in the
    // order they went out, each once B had passed its close, so the two can
    // order legs sharing a start differently: sorting by close next prints
    // one sheet from either, and the stable sort keeps legs sharing both in
    // the order the run reported them.
    let mut ordered: Vec<&HtmLegRecord> = legs.iter().collect();
    ordered.sort_by_key(|t| (t.start, t.start + t.duration));
    for (i, t) in ordered.iter().enumerate() {
        let row = (i + 1) as u32;
        ws.write_number(row, 0, t.start.as_ms() as f64)?;
        ws.write_number(row, 1, hour_index(t.start.as_ms()))?;
        ws.write_string(row, 2, activity_label(t.activity))?;
        ws.write_string(row, 3, endpoint_label(t.from))?;
        ws.write_string(row, 4, endpoint_label(t.to))?;
        ws.write_number(row, 5, t.distance_mm as f64)?;
        ws.write_number(row, 6, t.duration.as_ms() as f64)?;
        // Id columns: written only when the endpoint carries that id; left
        // blank otherwise (e.g. relay ids on a station endpoint). Each relay id
        // is paired with its zone column, blank on the same non-relay rows.
        if let Some(id) = endpoint_relay_id(t.from) {
            ws.write_number(row, 7, id as f64)?;
        }
        if let Some(zone) = t.from_zone_id {
            ws.write_number(row, 8, zone as f64)?;
        }
        if let Some(id) = endpoint_relay_id(t.to) {
            ws.write_number(row, 9, id as f64)?;
        }
        if let Some(zone) = t.to_zone_id {
            ws.write_number(row, 10, zone as f64)?;
        }
        if let Some(id) = endpoint_station_id(t.from) {
            ws.write_number(row, 11, id as f64)?;
        }
        if let Some(id) = endpoint_station_id(t.to) {
            ws.write_number(row, 12, id as f64)?;
        }
        if let Some(id) = t.presentation_id {
            ws.write_number(row, 13, id as f64)?;
        }
        ws.write_number(row, 14, t.robot_id.0 as f64)?;
        ws.write_boolean(row, 15, t.carrying_cargo)?;
        // Trip Type (col 16): derived from whichever endpoint is a station.
        if let Some(trip_type) = trip_type_of(t.from).or_else(|| trip_type_of(t.to)) {
            ws.write_string(row, 16, trip_type_label(trip_type))?;
        }
        ws.write_number(row, 17, t.turn_count as f64)?;
        // Wait Reason (col 18) keeps the legacy two-hold vocabulary; Hold
        // Cause/Position (cols 19-20) carry the full wait taxonomy from the
        // D30 state/waiting metrics work (#272), set on every live Wait row.
        if let Some(reason) = t.wait_reason {
            ws.write_string(row, 18, wait_reason_label(reason))?;
        }
        if let Some(hold) = t.hold {
            ws.write_string(row, 19, hold_cause_label(hold.cause))?;
            ws.write_string(row, 20, hold_position_label(hold.position))?;
        }
        // Notes (col 21): written only when non-empty.
        if !t.notes.is_empty() {
            ws.write_string(row, 21, &t.notes)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn activity_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Travel => "Travel",
        Activity::Wait => "Wait",
        Activity::Alignment => "Alignment",
        Activity::Mechanical => "Mechanical",
    }
}

fn trip_type_label(trip_type: TripType) -> &'static str {
    match trip_type {
        TripType::Pick => "Pick",
        TripType::Putaway => "Putaway",
        TripType::Extraction => "Extraction",
    }
}

fn wait_reason_label(reason: HtmWaitReason) -> &'static str {
    match reason {
        HtmWaitReason::NoTask => "No task available",
        HtmWaitReason::AwaitingDeposit => "Awaiting deposit",
    }
}

fn hold_cause_label(cause: HtmWaitCause) -> &'static str {
    match cause {
        HtmWaitCause::NoTask => "No task assigned",
        HtmWaitCause::AwaitingDepositResident => "Awaiting deposit (resident)",
        HtmWaitCause::AwaitingDepositDrove => "Awaiting deposit (drove)",
        HtmWaitCause::AwaitingSlot => "Awaiting diving-board slot",
        HtmWaitCause::AwaitingStationService => "Awaiting station service",
        HtmWaitCause::AwaitingDeparture => "Awaiting departure",
        HtmWaitCause::PostStationService => "Post-station service",
        HtmWaitCause::RouteHold => "Route hold",
    }
}

fn hold_position_label(position: HtmWaitPosition) -> &'static str {
    match position {
        HtmWaitPosition::Charging => "Charging cell",
        HtmWaitPosition::Presentation => "Presentation cell",
        HtmWaitPosition::StationQueue => "Station queue cell",
        HtmWaitPosition::Relay => "Relay cell",
        HtmWaitPosition::Route => "Route cell",
        HtmWaitPosition::Unknown => "Unknown",
    }
}

fn write_vtm_cycles_sheet(wb: &mut Workbook, cycles: &[VtmCycleRecord]) -> Result<(), XlsxError> {
    if cycles.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("VTM Cycles")?;
    let b = bold();
    let headers = [
        "Start (ms)",
        "Hour",
        "Robot ID",
        "Zone",
        "Direction",
        "Cycle Type",
        "Empty (ms)",
        "Loaded (ms)",
        "Fork (ms)",
        "Buffer (ms)",
        "Rack Alignment (ms)",
        "Relay Alignment (ms)",
        "Relay Transfer (ms)",
        "Total (ms)",
        "Relay",
        "Rack",
        "Tote",
        "Trip ID",
        "Pre-cycle Wait (ms)",
        "Wait Reason",
        "Cycle ID",
    ];
    for (c, hdr) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *hdr, &b)?;
    }
    // By start, then by end. The run holds its cycles in the order it
    // reported them and its recording in the order they went out, each once B
    // had passed its end, so the two can order cycles sharing a start
    // differently: sorting by end next prints one sheet from either, and the
    // stable sort keeps cycles sharing both in the order the run reported
    // them.
    let mut ordered: Vec<&VtmCycleRecord> = cycles.iter().collect();
    ordered.sort_by_key(|cycle| (cycle.sim_time_ms, cycle.sim_time_ms + cycle.total_cycle_ms));
    for (i, r) in ordered.iter().enumerate() {
        let row = (i + 1) as u32;
        ws.write_number(row, 0, r.sim_time_ms as f64)?;
        ws.write_number(row, 1, hour_index(r.sim_time_ms))?;
        ws.write_number(row, 2, r.vtm_id as f64)?;
        ws.write_number(row, 3, r.zone_id as f64)?;
        ws.write_string(row, 4, direction_label(r.direction))?;
        ws.write_string(row, 5, cycle_type_label(r.direction))?;
        ws.write_number(row, 6, r.empty_leg_ms as f64)?;
        ws.write_number(row, 7, r.loaded_leg_ms as f64)?;
        ws.write_number(row, 8, r.fork_ms as f64)?;
        ws.write_number(row, 9, r.buffer_ms as f64)?;
        ws.write_number(row, 10, r.rack_alignment_ms as f64)?;
        ws.write_number(row, 11, r.relay_alignment_ms as f64)?;
        ws.write_number(row, 12, r.relay_transfer_ms as f64)?;
        ws.write_number(row, 13, r.total_cycle_ms as f64)?;
        ws.write_number(row, 14, r.relay_id as f64)?;
        ws.write_number(row, 15, r.rack_id as f64)?;
        ws.write_number(row, 16, r.tote_id as f64)?;
        if let Some(id) = r.presentation_id {
            ws.write_number(row, 17, id as f64)?;
        }
        ws.write_number(row, 18, r.pre_cycle_wait_ms as f64)?;
        ws.write_string(row, 19, vtm_wait_reason_label(r.wait_reason))?;
        if let Some(id) = r.vtm_cycle_id {
            ws.write_number(row, 20, id as f64)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_vtm_legs_sheet(wb: &mut Workbook, legs: &[VtmLegRecord]) -> Result<(), XlsxError> {
    if legs.is_empty() {
        return Ok(());
    }
    let ws = wb.add_worksheet();
    ws.set_name("VTM Journey Legs")?;
    let b = bold();
    let headers = [
        "Start (ms)",
        "Hour",
        "Duration (ms)",
        "Activity",
        "From",
        "To",
        "From Rack ID",
        "To Rack ID",
        "From Relay ID",
        "From Relay Zone",
        "To Relay ID",
        "To Relay Zone",
        "Distance (mm)",
        "Robot ID",
        "Tote",
        "Trip ID",
        "Direction",
        "Cycle Type",
        "Carrying Tote",
        "Wait Reason",
        "Notes",
        "Cycle ID",
    ];
    for (c, hdr) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *hdr, &b)?;
    }
    // Chronological journal. Travel/Mechanical rows are pushed in nav-event
    // order; Wait rows are pushed at cycle start (= the wait's end), so sort by
    // start to interleave correctly. The run holds its legs in the order it
    // reported them and its recording in the order they went out, each once B
    // had passed its close, so the two can order legs sharing a start
    // differently: sorting by close next prints one sheet from either, and
    // the stable sort keeps legs sharing both in the order the run reported
    // them.
    let mut ordered: Vec<&VtmLegRecord> = legs.iter().collect();
    ordered.sort_by_key(|l| (l.start_ms, l.start_ms + l.duration_ms));
    for (i, l) in ordered.iter().enumerate() {
        let row = (i + 1) as u32;
        ws.write_number(row, 0, l.start_ms as f64)?;
        ws.write_number(row, 1, hour_index(l.start_ms))?;
        ws.write_number(row, 2, l.duration_ms as f64)?;
        ws.write_string(row, 3, activity_label(l.activity))?;
        ws.write_string(row, 4, vtm_endpoint_label(l.from))?;
        ws.write_string(row, 5, vtm_endpoint_label(l.to))?;
        // Id columns: written only when the endpoint carries that id. Each relay
        // id is paired with its zone column, blank on the same non-relay rows.
        if let Some(id) = vtm_endpoint_rack_id(l.from) {
            ws.write_number(row, 6, id as f64)?;
        }
        if let Some(id) = vtm_endpoint_rack_id(l.to) {
            ws.write_number(row, 7, id as f64)?;
        }
        if let Some(id) = vtm_endpoint_relay_id(l.from) {
            ws.write_number(row, 8, id as f64)?;
        }
        if let Some(zone) = l.from_zone_id {
            ws.write_number(row, 9, zone as f64)?;
        }
        if let Some(id) = vtm_endpoint_relay_id(l.to) {
            ws.write_number(row, 10, id as f64)?;
        }
        if let Some(zone) = l.to_zone_id {
            ws.write_number(row, 11, zone as f64)?;
        }
        ws.write_number(row, 12, l.distance_mm as f64)?;
        ws.write_number(row, 13, l.vtm_id as f64)?;
        if let Some(tote) = l.tote_id {
            ws.write_number(row, 14, tote as f64)?;
        }
        if let Some(id) = l.presentation_id {
            ws.write_number(row, 15, id as f64)?;
        }
        ws.write_string(row, 16, direction_label(l.direction))?;
        ws.write_string(row, 17, cycle_type_label(l.direction))?;
        ws.write_boolean(row, 18, l.carrying_tote)?;
        if let Some(reason) = l.wait_reason {
            ws.write_string(row, 19, vtm_wait_reason_label(reason))?;
        }
        if !l.notes.is_empty() {
            ws.write_string(row, 20, &l.notes)?;
        }
        if let Some(id) = l.vtm_cycle_id {
            ws.write_number(row, 21, id as f64)?;
        }
    }
    ws.set_freeze_panes(1, 0)?;
    Ok(())
}

fn direction_label(direction: VtmCycleDirection) -> &'static str {
    match direction {
        VtmCycleDirection::Forward => "Forward",
        VtmCycleDirection::Return => "Return",
    }
}

/// Physical storage direction does not identify the business flow: both
/// inventory and order containers can be retrieved from or returned to storage.
fn cycle_type_label(direction: VtmCycleDirection) -> &'static str {
    match direction {
        VtmCycleDirection::Forward => "Storage retrieval",
        VtmCycleDirection::Return => "Storage return",
    }
}

fn vtm_endpoint_label(endpoint: VtmJournalEndpoint) -> &'static str {
    match endpoint {
        VtmJournalEndpoint::Aisle => "Aisle",
        VtmJournalEndpoint::Relay(_) => "Relay",
        VtmJournalEndpoint::Rack(_) => "Rack",
    }
}

fn vtm_endpoint_rack_id(endpoint: VtmJournalEndpoint) -> Option<u32> {
    match endpoint {
        VtmJournalEndpoint::Rack(id) => Some(id.0),
        _ => None,
    }
}

fn vtm_endpoint_relay_id(endpoint: VtmJournalEndpoint) -> Option<u32> {
    match endpoint {
        VtmJournalEndpoint::Relay(id) => Some(id.0),
        _ => None,
    }
}

fn vtm_wait_reason_label(reason: VtmIdleReasonKind) -> &'static str {
    match reason {
        VtmIdleReasonKind::Continuous => "Continuous",
        VtmIdleReasonKind::AwaitingDemand => "Awaiting demand",
        VtmIdleReasonKind::AwaitingTote => "Awaiting tote",
        VtmIdleReasonKind::AwaitingSlot => "Awaiting slot",
        VtmIdleReasonKind::AwaitingHtmDeposit => "Awaiting HTM deposit",
    }
}

fn write_resolved_data_sheets(
    wb: &mut Workbook,
    skus: Option<&[SkuInfo]>,
    orders: Option<&[Order]>,
) -> Result<(), XlsxError> {
    let b = bold();
    if let Some(orders) = orders {
        let ws = wb.add_worksheet();
        ws.set_name("Orders")?;
        ws.write_string_with_format(0, 0, "Order ID", &b)?;
        ws.write_string_with_format(0, 1, "Line #", &b)?;
        ws.write_string_with_format(0, 2, "SKU ID", &b)?;
        ws.write_string_with_format(0, 3, "Quantity", &b)?;
        let mut r = 1u32;
        for order in orders {
            for (li, line) in order.lines.iter().enumerate() {
                ws.write_number(r, 0, order.id.0 as f64)?;
                ws.write_number(r, 1, li as f64)?;
                ws.write_number(r, 2, line.sku.0 as f64)?;
                ws.write_number(r, 3, line.quantity as f64)?;
                r += 1;
            }
        }
        ws.set_freeze_panes(1, 0)?;
    }
    if let Some(skus) = skus {
        let ws = wb.add_worksheet();
        ws.set_name("SKU Catalog")?;
        ws.write_string_with_format(0, 0, "SKU ID", &b)?;
        ws.write_string_with_format(0, 1, "Popular", &b)?;
        ws.write_string_with_format(0, 2, "Inventory Units", &b)?;
        ws.write_string_with_format(0, 3, "Weight (kg)", &b)?;
        for (i, sku) in skus.iter().enumerate() {
            let row = (i + 1) as u32;
            ws.write_number(row, 0, sku.id.0 as f64)?;
            ws.write_boolean(row, 1, sku.is_popular)?;
            ws.write_number(row, 2, sku.inventory_units as f64)?;
            ws.write_number_with_format(row, 3, sku.weight_kg as f64, &num_fmt())?;
        }
        ws.set_freeze_panes(1, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};
    use std::ops::RangeInclusive;

    use desgraph::{RelayId, StationId};
    use dessim::SimConfig;
    use dessim::events::VtmCycleDirection;
    use dessim::metrics::profile::{SliceRecord, SummaryStats};
    use dessim::metrics::reporting::{
        MetricDescriptor, completed_work_descriptors, d30_state_descriptors, da50_state_descriptors,
    };
    use dessim::model::ids::SkuId;
    use dessim::model::{OrderId, RobotId, SimDuration, SimTime};
    use dessim::observer::output::{
        Activity, HtmJournalEndpoint, HtmLegRecord, PickRecord, PutawayRecord, VtmCycleRecord, VtmJournalEndpoint,
        VtmLegRecord,
    };
    use rust_xlsxwriter::Workbook;
    use zip::ZipArchive;

    use super::{
        XlsxData, fmt_duration_secs, fmt_secs_labeled, line_chart_for_columns, write_htm_legs_sheet,
        write_metric_descriptors_sheet, write_picks_sheet, write_putaway_stores_sheet, write_putaways_sheet,
        write_throughput_sheet, write_vtm_cycles_sheet, write_vtm_legs_sheet,
    };

    #[test]
    fn inbound_export_preserves_lot_size_and_excludes_unused_or_future_returns() {
        let mut fills = vec![PutawayRecord {
            sim_time_ms: 1000,
            station_id: StationId(1),
            sku: SkuId(2),
            quantity: 6,
            presentation_id: Some(10),
        }];
        fills.push(PutawayRecord { sim_time_ms: 6000, ..fills[0] });
        let cycles: Vec<VtmCycleRecord> = [(10, 2000), (11, 2000), (10, 8000)]
            .into_iter()
            .map(|(id, at)| {
                serde_json::from_value(serde_json::json!({
                    "sim_time_ms": at, "vtm_id": 1, "zone_id": 2, "direction": "Return",
                    "empty_leg_ms": 100, "loaded_leg_ms": 100, "fork_ms": 100, "total_cycle_ms": 300,
                    "presentation_id": id, "tote_source": "Putaway"
                }))
                .unwrap()
            })
            .collect();
        let mut wb = Workbook::new();
        write_putaways_sheet(&mut wb, &fills, 5000).unwrap();
        write_putaway_stores_sheet(&mut wb, &cycles, &fills, 5000).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(wb.save_to_buffer().unwrap())).unwrap();
        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        assert_eq!(sheet.matches("<row ").count(), 2, "one row per filled tote, plus header");
        assert!(sheet.split_once("r=\"E2\"").unwrap().1.split_once("</c>").unwrap().0.contains("<v>6</v>"));
        sheet.clear();
        archive.by_name("xl/worksheets/sheet2.xml").unwrap().read_to_string(&mut sheet).unwrap();
        assert_eq!(sheet.matches("<row ").count(), 2, "only the filled tote's elapsed store is counted");
        assert!(sheet.split_once("r=\"A2\"").unwrap().1.split_once("</c>").unwrap().0.contains("<v>2300</v>"));
    }

    #[test]
    fn extraction_journey_ids_and_vtm_storage_direction_survive_xlsx_export() {
        let inbound = HtmLegRecord {
            start: SimTime::ms(1_000),
            activity: Activity::Travel,
            from: HtmJournalEndpoint::Relay(RelayId(3)),
            to: HtmJournalEndpoint::ExtractionStation(StationId(17)),
            from_zone_id: Some(2),
            to_zone_id: None,
            distance_mm: 5_000,
            duration: SimDuration::ms(5_000),
            presentation_id: Some(91),
            robot_id: RobotId(8),
            carrying_cargo: true,
            turn_count: 1,
            wait_reason: None,
            hold: None,
            notes: String::new(),
        };
        let outbound = HtmLegRecord {
            start: SimTime::ms(7_000),
            from: inbound.to,
            to: inbound.from,
            from_zone_id: None,
            to_zone_id: Some(2),
            ..inbound.clone()
        };
        let cycles: Vec<VtmCycleRecord> = ["Forward", "Return"]
            .into_iter()
            .map(|direction| {
                serde_json::from_value(serde_json::json!({
                    "sim_time_ms": 0, "vtm_id": 1, "zone_id": 2, "direction": direction,
                    "empty_leg_ms": 100, "loaded_leg_ms": 100, "fork_ms": 100, "total_cycle_ms": 300,
                }))
                .unwrap()
            })
            .collect();
        let mut workbook = Workbook::new();
        write_htm_legs_sheet(&mut workbook, &[inbound, outbound]).unwrap();
        write_vtm_cycles_sheet(&mut workbook, &cycles).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(workbook.save_to_buffer().unwrap())).unwrap();
        let mut legs = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut legs).unwrap();
        for reference in ["M2", "L3"] {
            let cell = legs.split_once(&format!("r=\"{reference}\"")).unwrap().1.split_once("</c>").unwrap().0;
            assert!(cell.contains("<v>17</v>"), "actual extraction station ID in {reference}");
        }
        let mut shared = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared).unwrap();
        let texts: Vec<_> = shared
            .split("<si>")
            .skip(1)
            .map(|entry| entry.split_once("<t").unwrap().1.split_once('>').unwrap().1.split_once("</t>").unwrap().0)
            .collect();
        let mut cycle_sheet = String::new();
        archive.by_name("xl/worksheets/sheet2.xml").unwrap().read_to_string(&mut cycle_sheet).unwrap();
        for (reference, expected) in [("F2", "Storage retrieval"), ("F3", "Storage return")] {
            let cell = cycle_sheet.split_once(&format!("r=\"{reference}\"")).unwrap().1;
            let index: usize = cell.split_once("<v>").unwrap().1.split_once("</v>").unwrap().0.parse().unwrap();
            assert_eq!(texts[index], expected);
        }
        assert!(!shared.contains("PutawaySupport"));
        assert!(!shared.contains("PickSupport"));
    }

    #[test]
    fn throughput_sheet_uses_complete_causal_windows_and_native_chart() {
        // 13 elapsed five-minute slices plus one future slice. A 60-minute
        // window with a 5-minute step has exactly two reportable windows.
        let series: Vec<SliceRecord> = (0..14)
            .map(|index| {
                serde_json::from_value(serde_json::json!({
                    "start_ms": index * 300_000,
                    "metrics": {
                        "picks_completed": index + 1,
                        "putaways_completed": 1
                    }
                }))
                .unwrap()
            })
            .collect();

        let mut workbook = Workbook::new();
        write_throughput_sheet(&mut workbook, &series, 300_000, 3_900_000).unwrap();
        let buffer = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buffer)).unwrap();
        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        // Sums over the two complete windows: 1..=12 = 78 and 2..=13 = 90 —
        // bound to their cells so sums landing in stale columns fail loudly.
        let cell = |reference: &str| {
            sheet
                .split(&format!("r=\"{reference}\""))
                .nth(1)
                .map(|rest| rest.chars().take(40).collect::<String>())
                .unwrap_or_default()
        };
        assert!(cell("B8").contains(">78<"), "first window pick sum in B8");
        assert!(cell("B9").contains(">90<"), "second window pick sum in B9");
        assert!(cell("C8").contains(">12<"), "first window putaway sum in C8");
        assert!(archive.by_name("xl/charts/chart1.xml").is_ok(), "native chart present");
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        assert!(workbook_xml.contains("name=\"Throughput\""), "sheet carries exactly the user-facing name");
        let mut shared = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared).unwrap();
        for header in ["End hour", "Picks completed", "Inducted units", "Putaway totes stored"] {
            assert!(shared.contains(header), "table header {header} present");
        }
        for dropped in ["Aggregation", "Bucket start (ms)", "Coverage (ms)"] {
            assert!(!shared.contains(dropped), "plumbing column {dropped} removed");
        }
        // The once-per-sheet metadata block survives the per-row column trim —
        // all six disclosure strings, not a sample.
        for disclosure in [
            "Requested window (ms)",
            "Requested step (ms)",
            "Applied window (ms)",
            "Applied step (ms)",
            "Alignment",
            "Source precision",
        ] {
            assert!(shared.contains(disclosure), "metadata block retains {disclosure}");
        }
        let mut chart = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut chart).unwrap();
        assert!(chart.contains("Trailing 60-minute completion totals"));
        // Two data rows land at sheet rows 8..9; categories in A, series in B/C.
        assert!(chart.contains("Throughput!$A$8:$A$9"), "category range tracks the trimmed layout");
        assert!(chart.contains("Throughput!$B$8:$B$9"), "picks series range");
        assert!(chart.contains("Throughput!$C$8:$C$9"), "putaways series range");
    }

    /// Workbook inventory through the orchestrator itself: dropping or
    /// reordering the D30 sheet calls in `write_xlsx` must fail here even
    /// though the per-sheet tests call their writers directly.
    #[test]
    fn write_xlsx_emits_d30_sheets_in_workbook_order() {
        let summary: SummaryStats = serde_json::from_value(serde_json::json!({
            "wall_clock_secs": 0.0,
            "sim_time_secs": 7200.0,
            "total_picks": 0,
            "total_orders_completed": 0,
            "throughput_picks_per_hour": 0.0,
            "vtm_idle_pct": 0.0,
            "htm_idle_pct": 0.0,
            "presentations_per_station_per_hour": [],
            "avg_presentations_per_station_per_hour": 0.0,
            "event_count": 0,
            "pathfinder_failure_rate": 0.0
        }))
        .unwrap();
        let config = SimConfig::default();
        let data = XlsxData {
            name: "orchestrator test",
            branch: "test",
            commit: "test",
            dirty: false,
            hostname: "test",
            timestamp: "test",
            config: &config,
            num_vtms: 1,
            scenario: None,
            skus: None,
            orders: None,
            summary: &summary,
            time_series: &[],
            slice_width_ms: 300_000,
        };
        let path = std::env::temp_dir().join(format!("d30-sheet-order-{}.xlsx", std::process::id()));
        super::write_xlsx(&path, &data).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        let position = |name: &str| {
            workbook_xml.find(&format!("name=\"{name}\"")).unwrap_or_else(|| panic!("workbook lacks sheet {name}"))
        };
        assert!(
            position("DA50 Distance") < position("D30 State")
                && position("D30 State") < position("D30 Distance")
                && position("D30 Distance") < position("Aisle Occupancy"),
            "D30 sheets sit between DA50 Distance and Aisle Occupancy"
        );
    }

    /// A run that died leaves a recording with no end block, and therefore no
    /// summary saying how long it ran. The book then covers the slices the run
    /// did reach: the report sheets are built over them, not over `[0, 0)`.
    #[test]
    fn a_recording_with_no_end_block_still_builds_its_report_sheets() {
        use dessim::metrics::profile::{
            IdTables, ProfileMetadata, ResolvedWorkload, RunDocuments, ScenarioSource, VtmFleet,
        };
        use dessim::metrics::worldview::{RunRecording, RunScenario, read_worldview};
        use dessim::observer::finality::recording::{FinalSlice, SliceRecorder, SliceRows};
        use dessim::observer::output::MetricsOutput;
        use dessim::observer::time_slice::TimeSliceData;
        use dessim::world_spec;
        use std::path::Path;

        use crate::build_workbook;

        // A real world document, because the reader compiles the map it
        // carries; everything else about the run is the smallest thing that
        // makes a D30 report row.
        let spec = Path::new("../sim/dessim/tests/fixtures/minimal_putaway_spec.json");
        let loaded = world_spec::load(spec).expect("the fixture spec loads");
        // The spec names the world document and opens nothing; the bytes are
        // read where the run model that carries them is built.
        let world_bytes = std::fs::read(&loaded.world_path).expect("the world document reads");
        let leg = HtmLegRecord {
            start: SimTime::ms(1_000),
            activity: Activity::Travel,
            from: HtmJournalEndpoint::Parking,
            to: HtmJournalEndpoint::PickStation(StationId(1)),
            from_zone_id: None,
            to_zone_id: None,
            distance_mm: 500,
            duration: SimDuration::ms(1_000),
            presentation_id: None,
            robot_id: RobotId(1),
            carrying_cargo: false,
            turn_count: 0,
            wait_reason: None,
            hold: None,
            notes: String::new(),
        };
        let scenario = RunScenario {
            name: "a run that died".to_owned(),
            metadata: ProfileMetadata {
                branch: "test".to_owned(),
                commit: "0123456789abcdef".to_owned(),
                commit_short: "0123456".to_owned(),
                dirty: false,
                timestamp: "2026-09-18T00:00:00Z".to_owned(),
                hostname: "testhost".to_owned(),
                grid_width: 1,
                grid_depth: 1,
                slice_width_ms: 10_000,
            },
            documents: RunDocuments { spec: Vec::new(), world: world_bytes, data: Vec::new() },
            // A recording names the frame its positions are measured in, and
            // the reader holds that name against the document beside it, so
            // the fixture states its own document's primary frame.
            ids: IdTables {
                frame: loaded.world.world.frames.primary().expect("the fixture declares one").to_string(),
                ..IdTables::default()
            },
            // One D30, because the state report divides by the fleet the
            // configuration states and refuses a journal naming more.
            sim_config: SimConfig { num_htms: 1, ..SimConfig::default() },
            scenario: ScenarioSource::from_config(loaded.scenario),
            workload: ResolvedWorkload::default(),
            vtm_fleet: VtmFleet(1),
            run: String::new(),
            attempt: 0,
        };

        // Two slices go out and the run dies: the file has no end block, no
        // index and no trailer, which is what a run that died leaves behind.
        let torn = std::env::temp_dir().join(format!("partial-torn-{}.worldview", std::process::id()));
        let mut recording = RunRecording::create(&torn, None, scenario).expect("the recording opens");
        let output = MetricsOutput::default();
        let mut legs = vec![leg];
        for start_ms in [0, 10_000] {
            let slice = TimeSliceData { start_ms, ..TimeSliceData::default() };
            let rows = SliceRows { htm_legs: std::mem::take(&mut legs), ..SliceRows::default() };
            recording
                .record(FinalSlice {
                    slice: &slice,
                    next: None,
                    ends: SimTime::ms(start_ms + 10_000),
                    last: false,
                    rows,
                    robot_spans: Vec::new(),
                    roster: &output.robot_types,
                })
                .expect("the slice is written");
        }
        drop(recording);
        let recording = read_worldview(&torn).expect("a torn file still opens");
        std::fs::remove_file(&torn).ok();
        assert!(recording.truncated, "the file has no end block");
        assert_eq!(recording.profile.summary.sim_time_secs, 0.0, "and so states no run length");

        let mut workbook = build_workbook(&XlsxData::from_profile(&recording.profile)).expect("the book builds");
        let saved = workbook.save_to_buffer().expect("the book saves");
        let mut archive = ZipArchive::new(Cursor::new(saved)).unwrap();
        let mut workbook_xml = String::new();
        archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut workbook_xml).unwrap();
        let index = workbook_xml
            .match_indices("name=\"")
            .position(|(at, _)| workbook_xml[at..].starts_with("name=\"D30 State\""));
        let mut sheet = String::new();
        let number = index.expect("the book has a D30 State sheet") + 1;
        archive.by_name(&format!("xl/worksheets/sheet{number}.xml")).unwrap().read_to_string(&mut sheet).unwrap();
        assert!(sheet.contains(">1000<"), "the D30 State sheet books the leg's second of travel over the slices");
    }

    /// The Putaways sheet reads in the order the run happened, rows sharing an
    /// instant in the order the run recorded them: a recording states a row in
    /// the frame written once B has passed it, so its rows reach the book in
    /// another order than the run appended them.
    #[test]
    fn the_putaways_sheet_reads_in_instant_order() {
        let putaway = |sim_time_ms, quantity| PutawayRecord {
            sim_time_ms,
            station_id: StationId(1),
            sku: SkuId(2),
            quantity,
            presentation_id: None,
        };
        let mut workbook = Workbook::new();
        let putaways = [putaway(3_000, 1), putaway(1_000, 2), putaway(3_000, 3)];
        write_putaways_sheet(&mut workbook, &putaways, u64::MAX).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'E'), [["1000", "2"], ["3000", "1"], ["3000", "3"]]);
    }

    /// The Picks sheet reads in instant order, rows sharing an instant in the
    /// order the run recorded them, whichever order they reached the book in.
    #[test]
    fn the_picks_sheet_reads_in_instant_order() {
        let pick = |sim_time_ms, quantity| PickRecord {
            sim_time_ms,
            station_id: StationId(1),
            sku: SkuId(2),
            quantity,
            dc_id: None,
            order_id: None,
            tote_id: None,
            source_zone_id: None,
            source_relay_id: None,
            presentation_id: None,
        };
        let mut workbook = Workbook::new();
        write_picks_sheet(&mut workbook, &[pick(3_000, 1), pick(1_000, 2), pick(3_000, 3)], u64::MAX).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'E'), [["1000", "2"], ["3000", "1"], ["3000", "3"]]);
    }

    /// The Putaway Rack Stores sheet reads in the order the stores ended, and
    /// two that end in one millisecond in the order they began, whichever
    /// order the run recorded them in.
    #[test]
    fn the_rack_stores_sheet_reads_by_end_then_start() {
        let fill = PutawayRecord {
            sim_time_ms: 0,
            station_id: StationId(1),
            sku: SkuId(2),
            quantity: 1,
            presentation_id: Some(9),
        };
        let store = |start: u64, total: u64, tote: u32| -> VtmCycleRecord {
            serde_json::from_value(serde_json::json!({
                "sim_time_ms": start, "vtm_id": 1, "zone_id": 2, "direction": "Return",
                "empty_leg_ms": 0, "loaded_leg_ms": 0, "fork_ms": 0, "total_cycle_ms": total,
                "tote_id": tote, "presentation_id": 9, "tote_source": "Putaway"
            }))
            .unwrap()
        };
        // The second and the third end at 3,000 ms; the third began first.
        let stores = [store(500, 1_000, 1), store(2_000, 1_000, 2), store(1_000, 2_000, 3)];
        let mut workbook = Workbook::new();
        write_putaway_stores_sheet(&mut workbook, &stores, &[fill], u64::MAX).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'C'), [["1500", "1"], ["3000", "3"], ["3000", "2"]]);
    }

    /// The VTM Cycles sheet reads in order of each cycle's start, and cycles
    /// sharing a start in the order they end, so the run and its recording,
    /// which can hold those in different orders, print one sheet.
    #[test]
    fn the_vtm_cycles_sheet_reads_by_start_then_end() {
        let cycle = |sim_time_ms: u64, total_cycle_ms: u64, vtm_id: u32| -> VtmCycleRecord {
            serde_json::from_value(serde_json::json!({
                "sim_time_ms": sim_time_ms, "vtm_id": vtm_id, "zone_id": 2, "direction": "Forward",
                "empty_leg_ms": 100, "loaded_leg_ms": 100, "fork_ms": 100, "total_cycle_ms": total_cycle_ms,
            }))
            .unwrap()
        };
        let cycles = [cycle(3_000, 500, 1), cycle(1_000, 300, 2), cycle(3_000, 300, 3)];
        let mut workbook = Workbook::new();
        write_vtm_cycles_sheet(&mut workbook, &cycles).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'C'), [["1000", "2"], ["3000", "3"], ["3000", "1"]]);
    }

    /// The HTM Journey Legs sheet reads in order of each leg's start, and legs
    /// sharing a start in the order they close, so the run and its recording,
    /// which can hold those in different orders, print one sheet.
    #[test]
    fn the_htm_legs_sheet_reads_by_start_then_close() {
        let leg = |start_ms, duration_ms, robot| HtmLegRecord {
            start: SimTime::ms(start_ms),
            activity: Activity::Travel,
            from: HtmJournalEndpoint::Parking,
            to: HtmJournalEndpoint::Parking,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm: 0,
            duration: SimDuration::ms(duration_ms),
            presentation_id: None,
            robot_id: RobotId(robot),
            carrying_cargo: false,
            turn_count: 0,
            wait_reason: None,
            hold: None,
            notes: String::new(),
        };
        let mut workbook = Workbook::new();
        write_htm_legs_sheet(&mut workbook, &[leg(1_000, 5_000, 1), leg(1_000, 2_000, 2), leg(500, 100, 3)]).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'G'), [["500", "100"], ["1000", "2000"], ["1000", "5000"]]);
    }

    /// The VTM Journey Legs sheet reads as the HTM one does: by start, then
    /// by close.
    #[test]
    fn the_vtm_legs_sheet_reads_by_start_then_close() {
        let leg = |start_ms, duration_ms, robot| VtmLegRecord {
            start_ms,
            duration_ms,
            activity: Activity::Travel,
            from: VtmJournalEndpoint::Aisle,
            to: VtmJournalEndpoint::Aisle,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm: 0,
            vtm_id: robot,
            vtm_cycle_id: None,
            tote_id: None,
            presentation_id: None,
            direction: VtmCycleDirection::Forward,
            carrying_tote: false,
            wait_reason: None,
            notes: String::new(),
        };
        let mut workbook = Workbook::new();
        write_vtm_legs_sheet(&mut workbook, &[leg(1_000, 5_000, 1), leg(1_000, 2_000, 2), leg(500, 100, 3)]).unwrap();
        let sheet = sheet_xml(&mut workbook, 1);
        assert_eq!(column_pairs(&sheet, 2..=4, 'A', 'C'), [["500", "100"], ["1000", "2000"], ["1000", "5000"]]);
    }

    /// Sheet `number` of `workbook` as the saved book states it, counting from
    /// one in the order the sheets were added.
    pub(super) fn sheet_xml(workbook: &mut Workbook, number: usize) -> String {
        let mut archive = ZipArchive::new(Cursor::new(workbook.save_to_buffer().unwrap())).unwrap();
        let mut sheet = String::new();
        archive.by_name(&format!("xl/worksheets/sheet{number}.xml")).unwrap().read_to_string(&mut sheet).unwrap();
        sheet
    }

    /// The value cell `reference` holds in `sheet`: a number's digits, or a
    /// text cell's index into the shared strings; empty when nothing was
    /// written there.
    pub(super) fn cell_value(sheet: &str, reference: &str) -> String {
        let Some((_, rest)) = sheet.split_once(&format!("r=\"{reference}\"")) else {
            return String::new();
        };
        let cell = rest.split_once("</c>").map_or(rest, |(cell, _)| cell);
        let value = cell.split_once("<v>").and_then(|(_, value)| value.split_once("</v>"));
        value.map(|(value, _)| value.to_owned()).unwrap_or_default()
    }

    /// The values of columns `first` and `second` in each of `rows`.
    pub(super) fn column_pairs(sheet: &str, rows: RangeInclusive<u32>, first: char, second: char) -> Vec<[String; 2]> {
        rows.map(|row| [cell_value(sheet, &format!("{first}{row}")), cell_value(sheet, &format!("{second}{row}"))])
            .collect()
    }

    /// The Picks sheet is the customer's pick-line export, so both data rows
    /// are pinned cell by cell across all eight columns: Order ID sits between
    /// Quantity and DC, and Trip ID moves right to make room for it.
    #[test]
    fn picks_sheet_carries_the_order_id_between_quantity_and_dc() {
        let picks = [
            PickRecord {
                sim_time_ms: 1_000,
                station_id: StationId(7),
                sku: SkuId(3),
                quantity: 25,
                dc_id: Some("DC-A".into()),
                order_id: Some(OrderId(4242)),
                tote_id: Some(10),
                source_zone_id: Some(3),
                source_relay_id: Some(RelayId(4)),
                presentation_id: Some(100),
            },
            // A pick recorded without tote-presentation context, an hour later.
            // Order ID, DC, and Trip ID all come from facts that context
            // carries, so all three cells stay blank.
            PickRecord {
                sim_time_ms: 3_602_000,
                station_id: StationId(8),
                sku: SkuId(5),
                quantity: 1,
                dc_id: None,
                order_id: None,
                tote_id: None,
                source_zone_id: None,
                source_relay_id: None,
                presentation_id: None,
            },
        ];

        let mut workbook = Workbook::new();
        write_picks_sheet(&mut workbook, &picks, u64::MAX).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing left
        // behind on disk when an assertion below fails.
        let buffer = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buffer)).unwrap();
        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        let mut shared = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared).unwrap();

        // The #318 cell-binding idiom: the raw XML fragment following a cell
        // reference, empty when the cell was never written.
        let cell = |reference: &str| {
            sheet
                .split(&format!("r=\"{reference}\""))
                .nth(1)
                .map(|rest| rest.chars().take(40).collect::<String>())
                .unwrap_or_default()
        };
        // Strings land in sharedStrings, so a text cell holds an index into
        // that table -- resolve it to compare the exact text.
        let shared_texts: Vec<&str> = shared
            .split("<si>")
            .skip(1)
            .filter_map(|entry| {
                let after_tag = entry.split_once("<t")?.1.split_once('>')?.1;
                after_tag.split_once("</t>").map(|(text, _)| text)
            })
            .collect();
        let cell_text = |reference: &str| -> &str {
            cell(reference)
                .split_once("<v>")
                .and_then(|(_, rest)| rest.split_once("</v>"))
                .map(|(index, _)| shared_texts[index.parse::<usize>().unwrap()])
                .unwrap_or("")
        };
        // Numeric cells carry their value inline; `None` when never written.
        let cell_number = |reference: &str| -> Option<f64> {
            cell(reference)
                .split_once("<v>")
                .and_then(|(_, rest)| rest.split_once("</v>"))
                .map(|(value, _)| value.parse::<f64>().unwrap())
        };

        for (reference, header) in [
            ("A1", "Completion Time (ms)"),
            ("B1", "Hour"),
            ("C1", "Station"),
            ("D1", "SKU"),
            ("E1", "Quantity"),
            ("F1", "Order ID"),
            ("G1", "DC"),
            ("H1", "Trip ID"),
        ] {
            assert_eq!(cell_text(reference), header, "header cell {reference}");
        }

        // Row 2, the fully attributed pick: every column bound to its value, so
        // a future column insert cannot slide the customer's data sideways.
        for (reference, value) in
            [("A2", 1_000.0), ("B2", 0.0), ("C2", 7.0), ("D2", 3.0), ("E2", 25.0), ("F2", 4_242.0), ("H2", 100.0)]
        {
            assert_eq!(cell_number(reference), Some(value), "cell {reference}");
        }
        assert_eq!(cell_text("G2"), "DC-A", "DC in G2");

        // Row 3, the pick with no presentation context: the facts that context
        // supplies are absent rather than fabricated, and the ones the cycle
        // itself carries are still written.
        for (reference, value) in [("A3", 3_602_000.0), ("B3", 1.0), ("C3", 8.0), ("D3", 5.0), ("E3", 1.0)] {
            assert_eq!(cell_number(reference), Some(value), "cell {reference}");
        }
        for reference in ["F3", "G3", "H3"] {
            assert_eq!(cell(reference), "", "cell {reference} is blank without context");
        }
    }

    #[test]
    fn metric_descriptors_sheet_dedups_what_the_workbook_carries() {
        // Producers repeated across scopes must still yield one dictionary
        // row per metric id.
        let state = da50_state_descriptors();
        let d30 = d30_state_descriptors();
        let work = completed_work_descriptors();
        let descriptors: Vec<&MetricDescriptor> = state
            .iter()
            .chain(state.iter())
            .chain(d30.iter())
            .chain(d30.iter())
            .chain(work.iter())
            .chain(work.iter())
            .collect();
        let unique = state.len() + d30.len() + work.len();

        let mut workbook = Workbook::new();
        write_metric_descriptors_sheet(&mut workbook, &descriptors).unwrap();
        let buffer = workbook.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(buffer)).unwrap();
        let mut sheet = String::new();
        archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        let rows = sheet.matches("<row ").count();
        assert_eq!(rows, unique + 1, "one header row plus one row per unique id");
        let mut shared = String::new();
        archive.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared).unwrap();
        assert!(shared.contains("da50.state.duration"));
        assert!(shared.contains("d30.state.duration"));
        assert!(shared.contains("d30.travel.distance.parking"));
        assert!(shared.contains("completed_work.tote_presentations"));
    }

    #[test]
    fn duration_secs_is_human_friendly() {
        assert_eq!(fmt_duration_secs(0.0), "0s");
        assert_eq!(fmt_duration_secs(9.0), "9s");
        assert_eq!(fmt_duration_secs(2.25), "2.25s");
        assert_eq!(fmt_duration_secs(1.0), "1s");
        assert_eq!(fmt_duration_secs(90.0), "1m 30s");
        assert_eq!(fmt_duration_secs(600.0), "10m");
        assert_eq!(fmt_duration_secs(3600.0), "1h");
        assert_eq!(fmt_duration_secs(5400.0), "1h 30m");
    }

    #[test]
    fn secs_labeled_shows_raw_plus_human_only_when_it_helps() {
        // Sub-minute: the raw value is already human-friendly, no parenthetical.
        assert_eq!(fmt_secs_labeled(2.25), "2.25s");
        assert_eq!(fmt_secs_labeled(45.0), "45s");
        // Minute-plus: keep the exact seconds and add the regrouped form.
        assert_eq!(fmt_secs_labeled(120.0), "120s (2m)");
        assert_eq!(fmt_secs_labeled(3600.0), "3600s (1h)");
    }

    #[test]
    fn native_line_chart_references_worksheet_data() {
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Data").unwrap();
        worksheet.write_string(0, 0, "Minute").unwrap();
        worksheet.write_string(0, 1, "Completed").unwrap();
        worksheet.write_number(1, 0, 1.0).unwrap();
        worksheet.write_number(1, 1, 4.0).unwrap();
        worksheet.write_number(2, 0, 2.0).unwrap();
        worksheet.write_number(2, 1, 7.0).unwrap();
        let chart = line_chart_for_columns("Data", "Completed", "Count", 1, 2, 0, &[1], "Simulation time").unwrap();
        worksheet.insert_chart(0, 3, &chart).unwrap();
        // Serialize in memory: no temp file to name uniquely, and nothing left
        // behind on disk when an assertion below fails.
        let buf = workbook.save_to_buffer().unwrap();

        let mut archive = ZipArchive::new(Cursor::new(buf)).unwrap();
        let mut chart_xml = String::new();
        archive.by_name("xl/charts/chart1.xml").unwrap().read_to_string(&mut chart_xml).unwrap();
        assert!(chart_xml.contains("<c:lineChart>"));
        assert!(chart_xml.contains("Data!$A$2:$A$3"));
        assert!(chart_xml.contains("Data!$B$2:$B$3"));
    }
}
