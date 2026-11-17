//! Canonical Metrics report rendering for Desvis.
//!
//! These views consume the same typed reporting API used by XLSX export. The
//! UI does not maintain a separate KPI calculation path: "KPI" is stakeholder
//! language for a metric, not a different product concept.

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::TAU;

use desgraph::StationId;
use dessim::events::VtmOperationKind;
use dessim::metrics::profile::SummaryStats;
use dessim::metrics::reporting::{
    AisleOccupancyData, AisleOccupancyRow, CompletedWorkData, D30_STATES, D30_WAIT_CATEGORIES, D30State, D30StateData,
    D30TravelKind, D30WaitCategory, DA50_OPERATION_KINDS, DA50_STATES, Da50OperationCountRow, Da50PayloadGroup,
    Da50State, Da50StateData, Da50ThroughputData, Da50TravelLoad, Da50WaitReason, DcStationWorkRow,
    DestinationStripingData, ExtractionData, HOUR_MS, MetricScope, MetricsReport, Ratio, ReportBuildError,
    ReportPeriod, ReportRequest, StripePlanSnapshot, SupportStatus, TimeBucket, TrailingSpec,
    aisle_occupancy_descriptors, aisle_occupancy_report, bucket_at, completed_work_descriptors, completed_work_report,
    d30_state_descriptors, d30_state_report, da50_operations_descriptors, da50_state_descriptors, da50_state_report,
    da50_throughput_report, destination_striping_descriptors, destination_striping_report, extraction_descriptors,
    extraction_report, report_buckets,
};
use eframe::egui::{self, Color32, RichText, Stroke};
use egui_plot::{Bar, BarChart, Legend, Line, Plot, PlotBounds, PlotPoints, Polygon, uniform_grid_spacer};

mod extraction;

use extraction::draw_extraction;

use crate::theme;
use crate::types::{
    DcMeasureSelection, DestinationsSourceSelection, MatrixDisplaySelection, MetricsPanelState,
    MetricsPayloadSelection, MetricsScopeSelection, MetricsView,
};

const PERIOD_OPTIONS: &[(u64, &str)] = &[
    (60_000, "1 min"),
    (5 * 60_000, "5 min"),
    (15 * 60_000, "15 min"),
    (30 * 60_000, "30 min"),
    (HOUR_MS, "1 hour"),
    (2 * HOUR_MS, "2 hours"),
    (4 * HOUR_MS, "4 hours"),
];

/// One destination = one color everywhere (stripe ribbon, throughput chart);
/// the index is the destination's plan/first-appearance position. Note the
/// chart's legend BOX lists names alphabetically — egui_plot stores legend
/// entries in a `BTreeMap<String, _>` — so display order there can never
/// follow plan order; color identity is the cross-surface invariant.
fn destination_color(plan_index: usize) -> Color32 {
    SERIES_COLORS[plan_index % SERIES_COLORS.len()]
}

const SERIES_COLORS: [Color32; 15] = [
    Color32::from_rgb(0x25, 0x63, 0xEB),
    Color32::from_rgb(0x16, 0xA3, 0x4A),
    Color32::from_rgb(0xEA, 0x58, 0x0C),
    Color32::from_rgb(0x93, 0x33, 0xEA),
    Color32::from_rgb(0xDB, 0x27, 0x77),
    Color32::from_rgb(0x08, 0x91, 0xB2),
    Color32::from_rgb(0xCA, 0x8A, 0x04),
    Color32::from_rgb(0x4F, 0x46, 0xE5),
    Color32::from_rgb(0x65, 0xA3, 0x0D),
    Color32::from_rgb(0x47, 0x55, 0x69),
    // Slots 10-14 exist for the flat state taxonomies: states plus promoted
    // waiting reasons/categories must all chart distinctly in one pie.
    Color32::from_rgb(0x0D, 0x94, 0x88),
    Color32::from_rgb(0xDC, 0x26, 0x26),
    Color32::from_rgb(0x92, 0x40, 0x0E),
    Color32::from_rgb(0xC0, 0x26, 0xD3),
    Color32::from_rgb(0x60, 0xA5, 0xFA),
];

// D30's ten-category Waiting split needs three more slots than the shared
// 15-color series palette. Keep them local: changing `SERIES_COLORS.len()`
// would remap destination colors that intentionally cycle over that palette.
//
// The fifteen series these three chart beside already cover the hue wheel at
// mid tone, so each takes its separation from lightness plus a chromatic
// family the co-occurring set leaves free: rose for the first two, and for
// the third the widest gap left on the wheel, the 57 degrees of spring green
// between Travel to park (lime) and Drop (green). Lavender sits 94 RGB from
// its nearest neighbor (Laden travel), salmon 82 (Station processing), spring
// green 156 (Travel to park), and no two of the three sit closer than 134;
// `d30_flat_timeline_order_is_complete_and_colors_stay_apart` holds that floor.
const D30_EXTRA_COLORS: [Color32; 5] = [
    Color32::from_rgb(0xC0, 0x84, 0xFC), // lavender: In Queue (station)
    Color32::from_rgb(0xFB, 0x71, 0x85), // salmon: Congestion
    Color32::from_rgb(0x97, 0xFC, 0x83), // spring green: No facts
    Color32::from_rgb(0x7A, 0xC7, 0xE0), // pale blue: Awaiting Relay Deposit (drove)
    Color32::from_rgb(0xE8, 0xB4, 0x6A), // amber: Awaiting Departure (station)
];

/// Run-local cache for the typed reports rendered by this panel. Report
/// generation can scan long journals, so it must not happen once per frame.
#[derive(Default)]
pub(crate) struct MetricsPanelCache {
    bundle: Option<Box<CachedReportBundle>>,
}

impl MetricsPanelCache {
    pub(crate) fn clear(&mut self) {
        self.bundle = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReportCacheKey {
    request: ReportRequest,
    overall_request: ReportRequest,
    vtm_count: u32,
    htm_count: u32,
    vtm_leg_count: usize,
    htm_leg_count: usize,
    vtm_operation_count: usize,
    aisle_state_count: usize,
    pick_count: usize,
    completed_order_count: usize,
    extraction_count: Option<u64>,
    /// Stripe-plan snapshots observed so far; the count changes when a
    /// plan-bearing profile loads (and, later, on dynamic restripes), so
    /// the bundle rebuilds.
    stripe_plan_count: usize,
}

struct CachedReportBundle {
    key: ReportCacheKey,
    da50_state_overall: Result<MetricsReport<Da50StateData>, ReportBuildError>,
    da50_state_temporal: Result<MetricsReport<Da50StateData>, ReportBuildError>,
    d30_state_overall: Result<MetricsReport<D30StateData>, ReportBuildError>,
    d30_state_temporal: Result<MetricsReport<D30StateData>, ReportBuildError>,
    da50_throughput: Result<MetricsReport<Da50ThroughputData>, ReportBuildError>,
    da50_throughput_overall: Result<MetricsReport<Da50ThroughputData>, ReportBuildError>,
    aisle_occupancy: Result<MetricsReport<AisleOccupancyData>, ReportBuildError>,
    completed_work: Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    completed_work_temporal: Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    extraction_overall: Result<MetricsReport<ExtractionData>, ReportBuildError>,
    extraction_temporal: Result<MetricsReport<ExtractionData>, ReportBuildError>,
    destination_striping_overall: Result<MetricsReport<DestinationStripingData>, ReportBuildError>,
    destination_striping_temporal: Result<MetricsReport<DestinationStripingData>, ReportBuildError>,
}

pub(crate) fn draw_metrics_panel(
    ui: &mut egui::Ui,
    view: MetricsView,
    summary: &SummaryStats,
    vtm_count: usize,
    htm_count: usize,
    embargo_ms: u64,
    state: &mut MetricsPanelState,
    cache: &mut MetricsPanelCache,
) {
    let run_end_ms = (summary.sim_time_secs * 1_000.0).round().max(0.0) as u64;
    let temporal_request = request(state, run_end_ms, embargo_ms, false);
    let overall_request = request(state, run_end_ms, embargo_ms, true);
    let key = ReportCacheKey {
        request: temporal_request,
        overall_request,
        vtm_count: vtm_count as u32,
        htm_count: htm_count as u32,
        vtm_leg_count: summary.vtm_leg_log.len(),
        htm_leg_count: summary.htm_leg_log.len(),
        vtm_operation_count: summary.vtm_operation_log.len(),
        aisle_state_count: summary.aisle_state_log.len(),
        pick_count: summary.pick_log.len(),
        completed_order_count: summary.completed_order_log.len(),
        extraction_count: summary
            .log_streams
            .iter()
            .find(|stream| stream.descriptor.id == "fulfillment.extractions")
            .map(|stream| stream.entries.entry_count),
        stripe_plan_count: summary.stripe_plan_log.len(),
    };
    if cache.bundle.as_ref().is_none_or(|bundle| bundle.key != key) {
        cache.bundle = Some(Box::new(CachedReportBundle {
            key,
            da50_state_overall: da50_state_report(&summary.vtm_leg_log, vtm_count as u32, overall_request),
            da50_state_temporal: da50_state_report(&summary.vtm_leg_log, vtm_count as u32, temporal_request),
            d30_state_overall: d30_state_report(&summary.htm_leg_log, htm_count as u32, overall_request),
            d30_state_temporal: d30_state_report(&summary.htm_leg_log, htm_count as u32, temporal_request),
            da50_throughput: da50_throughput_report(&summary.vtm_operation_log, temporal_request),
            da50_throughput_overall: da50_throughput_report(&summary.vtm_operation_log, overall_request),
            aisle_occupancy: aisle_occupancy_report(&summary.aisle_state_log, temporal_request),
            extraction_overall: extraction_report(&summary.log_streams, overall_request),
            extraction_temporal: extraction_report(&summary.log_streams, temporal_request),
            completed_work: completed_work_report(&summary.pick_log, &summary.completed_order_log, overall_request),
            completed_work_temporal: completed_work_report(
                &summary.pick_log,
                &summary.completed_order_log,
                temporal_request,
            ),
            destination_striping_overall: destination_striping_report(
                &summary.stripe_plan_log,
                &summary.pick_log,
                overall_request,
            ),
            destination_striping_temporal: destination_striping_report(
                &summary.stripe_plan_log,
                &summary.pick_log,
                temporal_request,
            ),
        }));
    }
    let reports = cache.bundle.as_ref().expect("report bundle was initialized");

    // Destinations owns its layout: the throughput chart docks at the
    // bottom as the timeline cursor while the matrices scroll above it —
    // it must not scroll away with the page.
    if view == MetricsView::Destinations {
        ui.horizontal(|ui| {
            ui.heading(view.label());
            ui.weak(view.description());
        });
        ui.add_space(4.0);
        draw_destinations(
            ui,
            &reports.completed_work,
            &reports.completed_work_temporal,
            &reports.destination_striping_overall,
            &reports.destination_striping_temporal,
            state,
            run_end_ms,
        );
        return;
    }

    // The state tabs dock their chrome (Daniel's rulings, 2026-07-30):
    // heading + Scope/Window/Resolution pinned at the top, the Timeline
    // pinned at the bottom, and only the middle (pie + table) scrolls. The
    // per-robot selector lives beside the pie as a scrollable list.
    if matches!(view, MetricsView::Da50State | MetricsView::D30State) {
        // Robot ids and stale-selection clearing run BEFORE the docks so the
        // Timeline and the tab body render the same validated selection in
        // the same frame.
        let vehicle_ids: Vec<u32> = match view {
            MetricsView::Da50State => reports.da50_state_overall.as_ref().map_or_else(
                |_| Vec::new(),
                |overall| {
                    overall
                        .data
                        .vehicle_state_rows
                        .iter()
                        .map(|row| row.vtm_id)
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect()
                },
            ),
            _ => reports.d30_state_overall.as_ref().map_or_else(
                |_| Vec::new(),
                |overall| {
                    overall
                        .data
                        .vehicle_state_rows
                        .iter()
                        .map(|row| row.htm_id)
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect()
                },
            ),
        };
        let selected = match view {
            MetricsView::Da50State => &mut state.selected_da50,
            _ => &mut state.selected_d30,
        };
        if selected.is_some_and(|id| !vehicle_ids.contains(&id)) {
            *selected = None;
        }
        // The Timeline's y unit follows the selection: fleet shares for the
        // fleet, that one robot's shares when a robot is picked.
        let robot_noun = if matches!(view, MetricsView::Da50State) { "DA50" } else { "D30" };
        let y_label = match *selected {
            Some(id) => format!("% of {robot_noun} {id} time"),
            None => "% fleet time".to_string(),
        };

        egui::TopBottomPanel::top(format!("controls_dock_{view:?}")).show_separator_line(false).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(view.label());
                ui.weak(view.description());
            });
            ui.add_space(4.0);
            draw_common_controls(ui, state, true);
        });
        let timeline_dock = |ui: &mut egui::Ui, draw: &mut dyn FnMut(&mut egui::Ui, f32)| {
            egui::TopBottomPanel::bottom(format!("{view:?}_timeline_dock"))
                .resizable(true)
                .default_height(265.0)
                .min_height(130.0)
                .show_separator_line(true)
                .show_inside(ui, |ui| {
                    ui.label(RichText::new("Timeline").strong());
                    // egui persists a panel's content rect as its height, so
                    // underfilling ratchets the dock shorter every repaint.
                    let dock_rect = ui.available_rect_before_wrap();
                    let plot_height = ui.available_height();
                    draw(ui, plot_height);
                    ui.expand_to_include_rect(dock_rect);
                });
        };
        match view {
            MetricsView::Da50State => match &reports.da50_state_temporal {
                Ok(temporal) => {
                    let rows = da50_timeline_rows(temporal, state);
                    timeline_dock(ui, &mut |ui, plot_height| {
                        draw_state_bars(
                            ui,
                            "da50_state_timeline",
                            &DA50_STATES,
                            &rows,
                            Da50State::label,
                            state_color,
                            plot_height,
                            &y_label,
                        );
                    });
                }
                Err(error) => {
                    ui.colored_label(theme::ERR, format!("Unable to build the DA50 timeline report: {error}"));
                }
            },
            _ => match &reports.d30_state_temporal {
                Ok(temporal) => {
                    let rows = d30_timeline_rows(temporal, state);
                    let segments = d30_timeline_segments();
                    timeline_dock(ui, &mut |ui, plot_height| {
                        draw_state_bars(
                            ui,
                            "d30_state_timeline",
                            &segments,
                            &rows,
                            d30_timeline_label,
                            d30_timeline_color,
                            plot_height,
                            &y_label,
                        );
                    });
                }
                Err(error) => {
                    ui.colored_label(theme::ERR, format!("Unable to build the D30 timeline report: {error}"));
                }
            },
        }
        egui::CentralPanel::default().frame(egui::Frame::none()).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().id_salt(format!("metrics_{view:?}")).auto_shrink([false, false]).show(
                ui,
                |ui| match view {
                    MetricsView::Da50State => draw_da50_state(ui, &reports.da50_state_overall, &vehicle_ids, state),
                    _ => draw_d30_state(ui, &reports.d30_state_overall, &vehicle_ids, state),
                },
            );
        });
        return;
    }

    egui::ScrollArea::vertical().id_salt(format!("metrics_{view:?}")).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(view.label());
            ui.weak(view.description());
        });
        ui.add_space(4.0);

        match view {
            MetricsView::TimeSeries => unreachable!("rendered by graph module"),
            MetricsView::Da50State | MetricsView::D30State => {
                unreachable!("state tabs render through the docked layout above")
            }
            MetricsView::Da50Throughput => {
                draw_da50_throughput(ui, &reports.da50_throughput, &reports.da50_throughput_overall, state)
            }
            MetricsView::OrderExtraction => {
                draw_extraction(ui, &reports.extraction_temporal, &reports.extraction_overall, state)
            }
            MetricsView::BufferStorage => draw_buffer_storage(ui, &reports.aisle_occupancy, state),
            MetricsView::AisleDistribution => draw_aisle_distribution(ui, &reports.completed_work, state),
            MetricsView::Destinations => {
                unreachable!("Destinations draws its own docked layout above")
            }
        }
    });
}

/// The per-robot selector on the state tabs: a scrollable Fleet-plus-robots
/// list at pie height beside the pie (Daniel's ruling, 2026-07-30: a list,
/// not a dropdown). Callers clear a stale selection before computing totals.
fn vehicle_list(
    ui: &mut egui::Ui,
    list_id: &'static str,
    label_prefix: &str,
    vehicle_ids: &[u32],
    selected: &mut Option<u32>,
    height: f32,
) {
    // The list renders inside a horizontal row next to the pie; without an
    // explicit top-down child layout the ScrollArea's contents inherit the
    // row's left-to-right flow and spill across the panel.
    ui.allocate_ui_with_layout(egui::vec2(96.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_size(egui::vec2(96.0, height));
        egui::ScrollArea::vertical().id_salt(list_id).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            ui.selectable_value(selected, None, "Fleet");
            for id in vehicle_ids {
                ui.selectable_value(selected, Some(*id), format!("{label_prefix} {id}"));
            }
        });
    });
}

/// Temporal state rows for the DA50 Timeline dock, honoring the per-robot
/// selection the tab's own tables use.
fn da50_timeline_rows(
    temporal: &MetricsReport<Da50StateData>,
    state: &MetricsPanelState,
) -> Vec<(TimeBucket, Da50State, Option<Ratio>)> {
    match state.selected_da50 {
        Some(vtm_id) => temporal
            .data
            .vehicle_state_rows
            .iter()
            .filter(|row| row.vtm_id == vtm_id)
            .map(|row| (row.bucket, row.state, row.vehicle_share))
            .collect(),
        None => temporal.data.state_rows.iter().map(|row| (row.bucket, row.state, row.fleet_share)).collect(),
    }
}

/// A D30 Timeline series. Waiting is not a merged state on this surface:
/// its place in the state stack is occupied by the ratified #272 categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum D30TimelineSegment {
    State(D30State),
    Wait(D30WaitCategory),
}

fn d30_timeline_segments() -> Vec<D30TimelineSegment> {
    let mut segments = Vec::with_capacity(D30_STATES.len() - 1 + D30_WAIT_CATEGORIES.len());
    for state in D30_STATES {
        if state == D30State::Waiting {
            segments.extend(D30_WAIT_CATEGORIES.into_iter().map(D30TimelineSegment::Wait));
        } else {
            segments.push(D30TimelineSegment::State(state));
        }
    }
    segments
}

fn d30_timeline_label(segment: D30TimelineSegment) -> &'static str {
    match segment {
        D30TimelineSegment::State(state) => state.label(),
        D30TimelineSegment::Wait(category) => category.label(),
    }
}

fn d30_timeline_color(segment: D30TimelineSegment) -> Color32 {
    match segment {
        D30TimelineSegment::State(state) => d30_state_color(state),
        D30TimelineSegment::Wait(category) => d30_wait_category_color(category),
    }
}

/// Temporal D30 rows with Waiting flattened into its canonical categories.
///
/// Category rows carry share-of-Waiting because that is their canonical
/// report ratio. The stacked state chart instead needs share-of-D30 time, so
/// this adapter combines each category duration with the denominator already
/// emitted on the corresponding Waiting state row. It does not recompute a
/// metric from raw facts.
fn d30_timeline_rows(
    temporal: &MetricsReport<D30StateData>,
    state: &MetricsPanelState,
) -> Vec<(TimeBucket, D30TimelineSegment, Option<Ratio>)> {
    let (states, categories): (Vec<_>, Vec<_>) = match state.selected_d30 {
        Some(htm_id) => (
            temporal
                .data
                .vehicle_state_rows
                .iter()
                .filter(|row| row.htm_id == htm_id)
                .map(|row| (row.bucket, row.state, row.vehicle_share))
                .collect(),
            temporal
                .data
                .vehicle_wait_category_rows
                .iter()
                .filter(|row| row.robot_id.0 == htm_id)
                .map(|row| (row.bucket, row.category, row.duration_ms))
                .collect(),
        ),
        None => (
            temporal.data.state_rows.iter().map(|row| (row.bucket, row.state, row.fleet_share)).collect(),
            temporal.data.wait_category_rows.iter().map(|row| (row.bucket, row.category, row.duration_ms)).collect(),
        ),
    };
    flatten_d30_timeline_rows(states, categories)
}

fn flatten_d30_timeline_rows(
    states: Vec<(TimeBucket, D30State, Option<Ratio>)>,
    categories: Vec<(TimeBucket, D30WaitCategory, Option<u64>)>,
) -> Vec<(TimeBucket, D30TimelineSegment, Option<Ratio>)> {
    let denominators: BTreeMap<_, _> = states
        .iter()
        .filter(|(_, metric_state, _)| *metric_state == D30State::Waiting)
        .filter_map(|(bucket, _, share)| share.map(|ratio| (*bucket, ratio.denominator)))
        .collect();
    let mut rows: Vec<_> = states
        .into_iter()
        .filter(|(_, metric_state, _)| *metric_state != D30State::Waiting)
        .map(|(bucket, metric_state, share)| (bucket, D30TimelineSegment::State(metric_state), share))
        .collect();
    rows.extend(categories.into_iter().map(|(bucket, category, duration_ms)| {
        let share = duration_ms
            .zip(denominators.get(&bucket).copied())
            .map(|(duration_ms, denominator)| Ratio::new(duration_ms, denominator));
        (bucket, D30TimelineSegment::Wait(category), share)
    }));
    rows
}

// The Period menu is retired (Daniel, 2026-07-30): the trailing lens is the
// only temporal lens (Hourly = a 60m trailing window; whole-run totals come
// from dedicated overall-request reports, never from a user-selected
// period). `include_window` gates the trailing Window/Resolution selectors
// for tabs with temporal charts.
fn draw_common_controls(ui: &mut egui::Ui, state: &mut MetricsPanelState, include_window: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Scope");
        egui::ComboBox::from_id_salt("metrics_scope").selected_text(state.scope.label()).show_ui(ui, |ui| {
            ui.selectable_value(
                &mut state.scope,
                MetricsScopeSelection::FullRun,
                MetricsScopeSelection::FullRun.label(),
            );
            ui.selectable_value(
                &mut state.scope,
                MetricsScopeSelection::PostEmbargo,
                MetricsScopeSelection::PostEmbargo.label(),
            );
        });

        if include_window {
            ui.separator();
            period_selector(ui, "metrics_trailing_window", "Window", &mut state.trailing_window_ms);
            period_selector(ui, "metrics_trailing_resolution", "Resolution", &mut state.trailing_resolution_ms);
        }
    });
    ui.separator();
}

fn period_selector(ui: &mut egui::Ui, id: &'static str, label: &'static str, value: &mut u64) {
    ui.label(label);
    egui::ComboBox::from_id_salt(id).selected_text(period_label(*value)).show_ui(ui, |ui| {
        for &(candidate, candidate_label) in PERIOD_OPTIONS {
            ui.selectable_value(value, candidate, candidate_label);
        }
    });
}

fn period_label(value: u64) -> String {
    PERIOD_OPTIONS
        .iter()
        .find_map(|&(candidate, label)| (candidate == value).then_some(label.to_string()))
        .unwrap_or_else(|| format_duration(value))
}

fn request(state: &MetricsPanelState, run_end_ms: u64, embargo_ms: u64, force_overall: bool) -> ReportRequest {
    let scope = match state.scope {
        MetricsScopeSelection::FullRun => MetricScope::FullRun,
        MetricsScopeSelection::PostEmbargo => MetricScope::PostEmbargo,
    };
    let period = if force_overall {
        ReportPeriod::Overall
    } else {
        ReportPeriod::Trailing(
            TrailingSpec::new(state.trailing_window_ms, state.trailing_resolution_ms, 0)
                .expect("selector values are non-zero"),
        )
    };
    ReportRequest::for_run(scope, period, run_end_ms, embargo_ms)
}

/// Family-neutral presentation data for the shared DA50/D30 state-tab body.
/// Each adapter below supplies only canonical report rows and vocabulary;
/// this function owns the selector/pie/table/distance layout for both tabs.
struct StateDistanceRow {
    label: &'static str,
    status: SupportStatus,
    distance_mm: Option<u64>,
}

struct StateTabBody<'a> {
    robot_noun: &'static str,
    vehicle_list_id: &'static str,
    pie_id: &'static str,
    distance_grid_id: &'static str,
    state_grid_id: &'static str,
    distance_header: &'static str,
    vehicle_ids: &'a [u32],
    fleet_size: u32,
    flat_rows: &'a [SummaryRow],
    distance_rows: &'a [StateDistanceRow],
    unobserved_state: &'static str,
    disclosures: &'a [&'static str],
}

fn draw_state_tab_body(ui: &mut egui::Ui, selected: &mut Option<u32>, body: StateTabBody<'_>) {
    let measured: Vec<_> = body
        .flat_rows
        .iter()
        .filter_map(|row| match (row.color, row.duration_ms) {
            (Some(color), Some(duration_ms)) => Some((row.label, duration_ms, color)),
            _ => None,
        })
        .collect();

    ui.columns(2, |columns| {
        columns[0].label(
            RichText::new(match *selected {
                Some(id) => format!("{} {id} time by state", body.robot_noun),
                None => "Fleet time by state".to_string(),
            })
            .strong(),
        );
        columns[0].horizontal(|ui| {
            vehicle_list(ui, body.vehicle_list_id, body.robot_noun, body.vehicle_ids, selected, 190.0);
            draw_pie_bare(ui, body.pie_id, &measured, 190.0);
        });
        // Gated on non-empty totals: an empty scope also has zero journaled
        // robots, and this note would then falsely describe the whole fleet.
        if selected.is_none() && !body.flat_rows.is_empty() && body.fleet_size as usize > body.vehicle_ids.len() {
            columns[0].weak(format!(
                "{} configured {}s have no journey facts; their whole window counts as {}",
                body.fleet_size as usize - body.vehicle_ids.len(),
                body.robot_noun,
                body.unobserved_state,
            ));
        }
        columns[0].add_space(6.0);
        columns[0].label(RichText::new("Travel distance").strong());
        egui::Grid::new(body.distance_grid_id).striped(true).show(&mut columns[0], |ui| {
            ui.strong(body.distance_header);
            ui.strong("Distance");
            ui.end_row();
            for row in body.distance_rows {
                ui.label(row.label);
                ui.label(match (row.status, row.distance_mm) {
                    (SupportStatus::Measured, Some(mm)) => format_distance(mm),
                    (SupportStatus::Unsupported, _) => "Not modeled".to_string(),
                    _ => "Unknown".to_string(),
                });
                ui.end_row();
            }
        });

        columns[1].label(RichText::new("States").strong());
        draw_state_summary(&mut columns[1], body.state_grid_id, body.flat_rows);
        for disclosure in body.disclosures {
            columns[1].weak(*disclosure);
        }
    });
}

fn draw_da50_state(
    ui: &mut egui::Ui,
    overall: &Result<MetricsReport<Da50StateData>, ReportBuildError>,
    vehicle_ids: &[u32],
    state: &mut MetricsPanelState,
) {
    let Ok(overall) = overall else {
        ui.colored_label(theme::ERR, "Unable to build the DA50 state report.");
        return;
    };

    let state_totals: Vec<_> = match state.selected_da50 {
        Some(vtm_id) => overall
            .data
            .vehicle_state_rows
            .iter()
            .filter(|row| row.vtm_id == vtm_id)
            .map(|row| (row.state, row.status, row.duration_ms, row.vehicle_share))
            .collect(),
        None => overall
            .data
            .state_rows
            .iter()
            .map(|row| (row.state, row.status, row.duration_ms, row.fleet_share))
            .collect(),
    };
    let wait_totals: Vec<_> = match state.selected_da50 {
        Some(vtm_id) => overall
            .data
            .vehicle_wait_rows
            .iter()
            .filter(|row| row.vtm_id == vtm_id)
            .map(|row| (row.reason, row.status, row.duration_ms, row.waiting_share))
            .collect(),
        None => overall
            .data
            .wait_rows
            .iter()
            .map(|row| (row.reason, row.status, row.duration_ms, row.waiting_share))
            .collect(),
    };
    let distance_totals: Vec<_> = match state.selected_da50 {
        Some(vtm_id) => overall
            .data
            .vehicle_distance_rows
            .iter()
            .filter(|row| row.vtm_id == vtm_id)
            .map(|row| StateDistanceRow {
                label: match row.load {
                    Da50TravelLoad::Empty => "Empty",
                    Da50TravelLoad::Laden => "Laden",
                },
                status: row.status,
                distance_mm: row.distance_mm,
            })
            .collect(),
        None => overall
            .data
            .distance_rows
            .iter()
            .map(|row| StateDistanceRow {
                label: match row.load {
                    Da50TravelLoad::Empty => "Empty",
                    Da50TravelLoad::Laden => "Laden",
                },
                status: row.status,
                distance_mm: row.distance_mm,
            })
            .collect(),
    };
    let flat_rows = flat_summary_rows(
        &state_totals,
        &wait_totals,
        Da50State::label,
        state_color,
        Da50State::Waiting,
        Da50WaitReason::label,
        wait_color,
    );
    draw_state_tab_body(
        ui,
        &mut state.selected_da50,
        StateTabBody {
            robot_noun: "DA50",
            vehicle_list_id: "da50_vehicle_list",
            pie_id: "da50_state_pie",
            distance_grid_id: "da50_distance_totals",
            state_grid_id: "da50_state_totals",
            distance_header: "Load",
            vehicle_ids,
            fleet_size: overall.data.fleet_size,
            flat_rows: &flat_rows,
            distance_rows: &distance_totals,
            unobserved_state: "No selected task",
            disclosures: &[
                "Waiting-derived states are post-hoc classification of the selected schedule; not proof of congestion or optimality.",
            ],
        },
    );
}

/// Percent-stacked state-share bars shared by the DA50 and D30 state tabs:
/// one column per emitted bucket, one segment per state in canonical order.
#[allow(clippy::too_many_arguments)]
fn draw_state_bars<S: Copy + PartialEq>(
    ui: &mut egui::Ui,
    plot_id: &'static str,
    states: &[S],
    rows: &[(TimeBucket, S, Option<Ratio>)],
    label: fn(S) -> &'static str,
    color: fn(S) -> Color32,
    plot_height: f32,
    y_label: &str,
) {
    if rows.is_empty() {
        // A trailing report emits nothing until one full window fits the
        // scope — say so instead of drawing bare axes.
        ui.weak("No complete trailing window fits this scope yet — shrink the window or let the run progress.");
        return;
    }
    let mut bases: BTreeMap<TimeBucket, f64> = BTreeMap::new();
    // Bars are laid out on the EMISSION grid, not the bucket span: trailing
    // buckets overlap (a 15-min window emitted every 5 min), so span-wide
    // bars would occlude each other. Each bar's width is the LOCAL step back
    // to the previous distinct bucket end (capped by its own span) — a global
    // minimum would let one small partial bucket at a stopped run's edge
    // collapse every full-window column to a sliver. The earliest bucket has
    // no predecessor, so it takes the step FORWARD to the next end: falling
    // back to its own span would draw the first column window-wide (3x its
    // neighbors at the default 15m/5m lens).
    let mut bucket_ends: Vec<u64> = rows.iter().map(|(bucket, _, _)| bucket.end_ms).collect();
    bucket_ends.sort_unstable();
    bucket_ends.dedup();
    let local_step = |bucket: &TimeBucket| -> u64 {
        let span = bucket.end_ms.saturating_sub(bucket.start_ms).max(1);
        match bucket_ends.binary_search(&bucket.end_ms) {
            Ok(0) => bucket_ends.get(1).map_or(span, |next| (next - bucket.end_ms).min(span)),
            Err(_) => span,
            Ok(index) => (bucket.end_ms - bucket_ends[index - 1]).min(span),
        }
    };
    // Bounds are forced every frame: all interactions are disabled, so the
    // only other bounds source is plot memory, which pins the x-range seen
    // at first render -- switching scope (post-embargo data starts at the
    // embargo boundary, not 0) left dead space where the old range began.
    let x_bounds = bucket_ends.last().map(|last_end| {
        let first_bar_left = rows.iter().map(|(bucket, _, _)| bucket.end_ms - local_step(bucket)).min().unwrap_or(0);
        (first_bar_left as f64 / 60_000.0, *last_end as f64 / 60_000.0)
    });
    Plot::new(plot_id)
        .height(plot_height)
        .legend(Legend::default())
        .include_y(0.0)
        .include_y(100.0)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        // Fixed 25% y-steps: the axis only labels marks whose screen spacing
        // clears ~20-30 px, and the auto spacer's 10%-steps land under that
        // at this plot height, leaving every label but 0 invisible.
        .y_grid_spacer(uniform_grid_spacer(|_input| [100.0, 50.0, 25.0]))
        .x_axis_label("Simulation time (min)")
        .y_axis_label(y_label.to_string())
        .label_formatter({
            let y_label = y_label.to_string();
            move |_name, value| format!("t = {:.0} min\n{:.1}{y_label}", value.x, value.y)
        })
        .show(ui, |plot_ui| {
            if let Some((x_min, x_max)) = x_bounds {
                if x_max > x_min {
                    plot_ui.set_plot_bounds(PlotBounds::from_min_max([x_min, 0.0], [x_max, 100.0]));
                }
            }
            for &state in states {
                let mut bars = Vec::new();
                for (bucket, _, ratio) in rows.iter().filter(|(_, row_state, _)| *row_state == state) {
                    let Some(share) = ratio.and_then(Ratio::percent) else {
                        continue;
                    };
                    let base = bases.entry(*bucket).or_default();
                    let bar_ms = local_step(bucket);
                    let center_min = (bucket.end_ms as f64 - bar_ms as f64 / 2.0) / 60_000.0;
                    let width_min = bar_ms as f64 / 60_000.0 * 0.86;
                    bars.push(
                        Bar::new(center_min, share).base_offset(*base).width(width_min.max(0.05)).fill(color(state)),
                    );
                    *base += share;
                }
                if !bars.is_empty() {
                    plot_ui.bar_chart(BarChart::new(bars).name(label(state)).color(color(state)));
                }
            }
        });
}

/// The D30 state tab: the DA50 tab's composition (pie + exact totals +
/// share-over-time bars + waiting attribution + travel distance, with a
/// per-vehicle selector) over the D30 report family.
fn draw_d30_state(
    ui: &mut egui::Ui,
    overall: &Result<MetricsReport<D30StateData>, ReportBuildError>,
    vehicle_ids: &[u32],
    state: &mut MetricsPanelState,
) {
    // Surface the tripwire payload (counts, timestamps, mm) -- the errors
    // exist to name the integrity breach, not to be collapsed.
    let overall = match overall {
        Ok(overall) => overall,
        Err(error) => {
            ui.colored_label(theme::ERR, format!("Unable to build the D30 state report: {error}"));
            return;
        }
    };

    let state_totals: Vec<_> = match state.selected_d30 {
        Some(htm_id) => overall
            .data
            .vehicle_state_rows
            .iter()
            .filter(|row| row.htm_id == htm_id)
            .map(|row| (row.state, row.status, row.duration_ms, row.vehicle_share))
            .collect(),
        None => overall
            .data
            .state_rows
            .iter()
            .map(|row| (row.state, row.status, row.duration_ms, row.fleet_share))
            .collect(),
    };
    let wait_totals: Vec<_> = match state.selected_d30 {
        Some(htm_id) => overall
            .data
            .vehicle_wait_category_rows
            .iter()
            .filter(|row| row.robot_id.0 == htm_id)
            .map(|row| (row.category, row.status, row.duration_ms, row.waiting_share))
            .collect(),
        None => overall
            .data
            .wait_category_rows
            .iter()
            .map(|row| (row.category, row.status, row.duration_ms, row.waiting_share))
            .collect(),
    };
    let distance_totals: Vec<_> = match state.selected_d30 {
        Some(htm_id) => overall
            .data
            .vehicle_distance_rows
            .iter()
            .filter(|row| row.htm_id == htm_id)
            .map(|row| StateDistanceRow {
                label: match row.kind {
                    D30TravelKind::Empty => "Empty",
                    D30TravelKind::Laden => "Laden",
                    D30TravelKind::Parking => "Parking",
                },
                status: row.status,
                distance_mm: row.distance_mm,
            })
            .collect(),
        None => overall
            .data
            .distance_rows
            .iter()
            .map(|row| StateDistanceRow {
                label: match row.kind {
                    D30TravelKind::Empty => "Empty",
                    D30TravelKind::Laden => "Laden",
                    D30TravelKind::Parking => "Parking",
                },
                status: row.status,
                distance_mm: row.distance_mm,
            })
            .collect(),
    };
    let flat_rows = flat_summary_rows(
        &state_totals,
        &wait_totals,
        D30State::label,
        d30_state_color,
        D30State::Waiting,
        D30WaitCategory::label,
        |category| Some(d30_wait_category_color(category)),
    );
    draw_state_tab_body(
        ui,
        &mut state.selected_d30,
        StateTabBody {
            robot_noun: "D30",
            vehicle_list_id: "d30_vehicle_list",
            pie_id: "d30_state_pie",
            distance_grid_id: "d30_distance_totals",
            state_grid_id: "d30_state_totals",
            distance_header: "Kind",
            vehicle_ids,
            fleet_size: overall.data.fleet_size,
            flat_rows: &flat_rows,
            distance_rows: &distance_totals,
            // Where the producer actually books this mass: a configured D30
            // that journals nothing has no facts to attribute, so its whole
            // window lands in the `NoFacts` category, not the bounded-gap
            // `Unknown` one.
            unobserved_state: "No facts",
            disclosures: &[
                // Same three facts, same order, as the XLSX D30 State sheet's
                // once-per-sheet disclosure: what the rows measure, where a
                // pre-dwell hold lands instead, and what post-dwell time means.
                "Station processing is recorded operator dwell only. A pre-dwell station hold is Waiting (Presenting or In Queue (station)). Post-dwell presentation occupancy can include departure clearance or downstream blocking; it is not operator work.",
                "Waiting-derived states are post-hoc classification of the selected schedule; not proof of congestion or optimality.",
            ],
        },
    );
}

fn draw_da50_throughput(
    ui: &mut egui::Ui,
    report: &Result<MetricsReport<Da50ThroughputData>, ReportBuildError>,
    overall_report: &Result<MetricsReport<Da50ThroughputData>, ReportBuildError>,
    state: &mut MetricsPanelState,
) {
    draw_common_controls(ui, state, true);
    ui.horizontal(|ui| {
        ui.label("Tote contents");
        egui::ComboBox::from_id_salt("da50_payload").selected_text(state.da50_payload.label()).show_ui(ui, |ui| {
            for payload in [
                MetricsPayloadSelection::All,
                MetricsPayloadSelection::Empty,
                MetricsPayloadSelection::NonEmpty,
                MetricsPayloadSelection::Unknown,
            ] {
                ui.selectable_value(&mut state.da50_payload, payload, payload.label());
            }
        });
    });

    let Ok(report) = report else {
        ui.colored_label(theme::ERR, "Unable to build the DA50 throughput report.");
        return;
    };
    let payload = payload_group(state.da50_payload);
    let total_rows: Vec<_> =
        report.data.rows.iter().filter(|row| row.zone_id.is_none() && row.payload == payload).collect();

    // Whole-run totals as cards, temporal series below — no period toggle
    // (the Overall period is retired; totals come from the overall report,
    // whose fixed single bucket sums correctly where overlapping trailing
    // buckets would not).
    match overall_report {
        Ok(overall_report) => {
            let overall_rows: Vec<_> =
                overall_report.data.rows.iter().filter(|row| row.zone_id.is_none() && row.payload == payload).collect();
            draw_operation_totals(ui, &overall_rows);
            ui.add_space(6.0);
        }
        Err(error) => {
            ui.colored_label(theme::ERR, format!("Unable to build the whole-run totals: {error}"));
        }
    }
    let series: Vec<_> = DA50_OPERATION_KINDS
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            let points = total_rows
                .iter()
                .filter(|row| row.operation == operation)
                .map(|row| [row.bucket.end_ms as f64 / 60_000.0, row.count as f64])
                .collect();
            (operation_label(operation).to_string(), SERIES_COLORS[index], points)
        })
        .collect();
    if total_rows.is_empty() {
        ui.weak("No complete trailing window fits this scope yet — shrink the window or let the run progress.");
    } else {
        draw_line_chart(ui, "da50_throughput_chart", "Operations over time", series);
    }

    ui.add_space(8.0);
    ui.label(RichText::new("Exact operation data").strong());
    egui::Grid::new("da50_throughput_table").striped(true).show(ui, |ui| {
        ui.strong("Period end");
        ui.strong("Aisle");
        ui.strong("Operation");
        ui.strong("Count");
        ui.end_row();
        for row in report.data.rows.iter().filter(|row| row.payload == payload) {
            ui.label(format_duration(row.bucket.end_ms));
            ui.label(row.zone_id.map_or_else(|| "All".to_string(), |zone| format!("z{zone}")));
            ui.label(operation_label(row.operation));
            ui.label(row.count.to_string());
            ui.end_row();
        }
    });
}

fn draw_operation_totals(ui: &mut egui::Ui, rows: &[&Da50OperationCountRow]) {
    ui.horizontal_wrapped(|ui| {
        for operation in DA50_OPERATION_KINDS {
            let count = rows.iter().find(|row| row.operation == operation).map_or(0, |row| row.count);
            ui.group(|ui| {
                ui.label(RichText::new(count.to_string()).size(20.0).strong());
                ui.weak(operation_label(operation));
            });
        }
    });
}

fn draw_buffer_storage(
    ui: &mut egui::Ui,
    report: &Result<MetricsReport<AisleOccupancyData>, ReportBuildError>,
    state: &mut MetricsPanelState,
) {
    draw_common_controls(ui, state, true);
    let Ok(report) = report else {
        ui.colored_label(theme::ERR, "Unable to build the buffer and storage report.");
        return;
    };
    if report.data.rows.is_empty() {
        ui.weak("No complete trailing window fits this scope yet — shrink the window or let the run progress.");
        return;
    }

    if let Some(latest_end_ms) = report.data.rows.iter().map(|row| row.bucket.end_ms).max() {
        let latest_rows: Vec<_> = report.data.rows.iter().filter(|row| row.bucket.end_ms == latest_end_ms).collect();
        ui.label(RichText::new("Warehouse capacity-weighted mean").strong());
        ui.horizontal_wrapped(|ui| {
            for (label, value) in [
                ("Storage occupied", sum_ratios(&latest_rows, |row| row.storage_occupied)),
                ("Empty totes", sum_ratios(&latest_rows, |row| row.storage_empty)),
                ("Non-empty totes", sum_ratios(&latest_rows, |row| row.storage_non_empty)),
                ("Unknown payload", sum_ratios(&latest_rows, |row| row.storage_unknown)),
                ("Diving boards", sum_ratios(&latest_rows, |row| row.diving_board_occupied)),
            ] {
                ui.group(|ui| {
                    ui.label(RichText::new(format_ratio(value)).size(18.0).strong());
                    ui.weak(label);
                });
            }
        });
        ui.weak(
            "Aisle numerators and capacities are summed before division; the value is not an average of percentages.",
        );
        ui.separator();
    }

    for (id, title, accessor) in [
        (
            "storage_fill_chart",
            "Storage fill by aisle",
            (|row: &AisleOccupancyRow| row.storage_occupied) as fn(&AisleOccupancyRow) -> Option<Ratio>,
        ),
        (
            "relay_occupancy_chart",
            "Diving-board occupancy by aisle",
            (|row: &AisleOccupancyRow| row.diving_board_occupied) as fn(&AisleOccupancyRow) -> Option<Ratio>,
        ),
    ] {
        let zones: Vec<_> =
            report.data.rows.iter().map(|row| row.zone_id).collect::<BTreeSet<_>>().into_iter().collect();
        let series = zones
            .iter()
            .enumerate()
            .map(|(index, zone)| {
                let points = report
                    .data
                    .rows
                    .iter()
                    .filter(|row| row.zone_id == *zone)
                    .filter_map(|row| {
                        accessor(row).and_then(Ratio::percent).map(|value| [row.bucket.end_ms as f64 / 60_000.0, value])
                    })
                    .collect();
                (format!("z{zone}"), SERIES_COLORS[index % SERIES_COLORS.len()], points)
            })
            .collect();
        draw_line_chart(ui, id, title, series);
    }

    ui.label(RichText::new("Exact aisle values").strong());
    ui.weak("Each cell is the time-weighted mean over the period.");
    egui::Grid::new("aisle_occupancy_table").striped(true).show(ui, |ui| {
        ui.strong("Period end");
        ui.strong("Aisle");
        ui.strong("Status");
        ui.strong("Fill");
        ui.strong("Empty totes");
        ui.strong("Non-empty totes");
        ui.strong("Unknown totes");
        ui.strong("Diving board");
        ui.end_row();
        for row in &report.data.rows {
            ui.label(format_duration(row.bucket.end_ms));
            ui.label(format!("z{}", row.zone_id));
            ui.label(support_status_text(row.status));
            ui.label(format_ratio(row.storage_occupied));
            ui.label(format_ratio(row.storage_empty));
            ui.label(format_ratio(row.storage_non_empty));
            ui.label(format_ratio(row.storage_unknown));
            ui.label(format_ratio(row.diving_board_occupied));
            ui.end_row();
        }
    });
}

fn sum_ratios(rows: &[&AisleOccupancyRow], accessor: fn(&AisleOccupancyRow) -> Option<Ratio>) -> Option<Ratio> {
    rows.iter().try_fold(Ratio::default(), |total, row| {
        let value = accessor(row)?;
        Some(Ratio::new(
            total.numerator.checked_add(value.numerator)?,
            total.denominator.checked_add(value.denominator)?,
        ))
    })
}

fn support_status_text(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::Measured => "Measured",
        SupportStatus::Unknown => "Unknown",
        SupportStatus::Unsupported => "Not modeled",
    }
}

fn draw_aisle_distribution(
    ui: &mut egui::Ui,
    report: &Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    state: &mut MetricsPanelState,
) {
    draw_common_controls(ui, state, false);
    let Ok(report) = report else {
        ui.colored_label(theme::ERR, "Unable to build the aisle distribution report.");
        return;
    };

    let rows = &report.data.aisle_station_rows;
    let zones: Vec<_> = rows.iter().map(|row| row.zone_id).collect::<BTreeSet<_>>().into_iter().collect();
    let stations: Vec<_> = rows.iter().map(|row| row.station_id).collect::<BTreeSet<_>>().into_iter().collect();
    let max_count = rows.iter().map(|row| row.tote_count).max().unwrap_or(1).max(1);
    let mut hovered_zone = None;
    let mut clicked_zone = None;

    ui.label(RichText::new("Completed tote presentations").strong());
    ui.weak("Each cell shows tote count and the aisle's share of completed totes at that pick station.");
    egui::Grid::new("aisle_station_matrix").striped(true).show(ui, |ui| {
        ui.strong("Aisle");
        for station in &stations {
            ui.strong(station.to_string());
        }
        ui.end_row();
        for zone in &zones {
            let response = ui.add(
                egui::Label::new(RichText::new(aisle_dimension_label(*zone)).strong()).sense(egui::Sense::click()),
            );
            if response.hovered() {
                hovered_zone = Some(*zone);
            }
            if response.clicked() {
                clicked_zone = Some(*zone);
            }
            for station in &stations {
                let row = rows.iter().find(|row| row.zone_id == *zone && row.station_id == *station);
                let (count, ratio) =
                    row.map_or((0, Ratio::new(0, 0)), |row| (row.tote_count, row.aisle_share_of_station));
                let percent = ratio.percent();
                let response =
                    heatmap_cell(ui, format!("{count} · {}", format_percent(percent)), count as f64 / max_count as f64)
                        .on_hover_text(format!(
                            "Tote count: {count}\nAisle share of station: {}/{} ({})",
                            ratio.numerator,
                            ratio.denominator,
                            format_percent(percent),
                        ));
                if response.hovered() {
                    hovered_zone = Some(*zone);
                }
                if response.clicked() {
                    clicked_zone = Some(*zone);
                }
            }
            ui.end_row();
        }
    });

    if let Some(zone) = clicked_zone {
        state.selected_aisle = (state.selected_aisle != Some(zone)).then_some(zone);
    }
    let pinned_zone = state.selected_aisle.filter(|zone| zones.contains(zone));
    let detail_zone = hovered_zone.or(pinned_zone);
    if let Some(zone) = detail_zone {
        let values: Vec<_> = stations
            .iter()
            .enumerate()
            .filter_map(|(index, station)| {
                rows.iter()
                    .find(|row| row.zone_id == zone && row.station_id == *station)
                    .map(|row| (station.to_string(), row.tote_count, SERIES_COLORS[index % SERIES_COLORS.len()]))
            })
            .collect();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} station distribution", aisle_dimension_label(zone))).strong());
            if pinned_zone == Some(zone) {
                ui.weak("Pinned");
                if ui.small_button("Clear").clicked() {
                    state.selected_aisle = None;
                }
            }
        });
        draw_pie(ui, "aisle_hover_pie", &values, 170.0);
    } else {
        ui.weak("Hover an aisle row to inspect its pick-station distribution; click to pin it.");
    }

    let unidentified: u64 = report.data.unattributed_rows.iter().map(|row| row.unidentified_pick_lines).sum();
    if unidentified > 0 {
        ui.colored_label(
            theme::WARN,
            format!("{unidentified} completed pick lines lacked a presentation ID and could not be distinct-counted."),
        );
    }
    let aisle_unattributed: u64 =
        report.data.unattributed_rows.iter().map(|row| row.aisle_unattributed_presentations).sum();
    if aisle_unattributed > 0 {
        ui.colored_label(
            theme::WARN,
            format!(
                "{aisle_unattributed} completed presentations had no source aisle and are excluded from this matrix."
            ),
        );
    }
}

/// The Destinations tab: one axes pair (destination × pick station), two
/// strictly-separated matrices — completed facts vs the planned stripe map —
/// plus the striping-state vs pick-destination-throughput time series.
/// The stripe snapshot governing sim time `at_ms` — the last one that took
/// effect at or before it (index 0 when `at_ms` precedes them all).
fn snapshot_index(snapshots: &[StripePlanSnapshot], at_ms: u64) -> usize {
    snapshots.iter().rposition(|snapshot| snapshot.from_ms <= at_ms).unwrap_or(0)
}

fn draw_destinations(
    ui: &mut egui::Ui,
    picks_overall: &Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    picks_temporal: &Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    striping_overall: &Result<MetricsReport<DestinationStripingData>, ReportBuildError>,
    striping_temporal: &Result<MetricsReport<DestinationStripingData>, ReportBuildError>,
    state: &mut MetricsPanelState,
    run_end_ms: u64,
) {
    // A cursor inherited from a longer previous run (panel state outlives
    // runs) would sit past this run's end.
    if state.chart_cursor_ms.is_some_and(|cursor| cursor > run_end_ms) {
        state.chart_cursor_ms = None;
    }
    // The chart docks as a real inner bottom panel and the matrices fill
    // the exact remainder — measured by egui, never estimated. (An earlier
    // estimate-based split under-filled the panel by the estimation error,
    // and egui persists the content height, so the panel ratcheted shorter
    // every frame the user let go of the resize handle.)
    let full_rect = ui.available_rect_before_wrap();
    egui::TopBottomPanel::bottom("destinations_chart_dock")
        .resizable(true)
        .default_height(265.0)
        .min_height(130.0)
        .show_inside(ui, |ui| {
            // egui persists a panel's CONTENT rect as its height
            // (TopBottomPanel::show_inside stores the frame's rect, which
            // derives from content), so content that underfills makes this
            // dock — and every panel above it — shrink by the shortfall on
            // every repaint. Claim the whole dock rect regardless of what
            // the content measured (Opus review repro, 2026-07-30).
            let dock_rect = ui.available_rect_before_wrap();
            ui.add_space(4.0);
            draw_destination_throughput_chart(ui, striping_temporal, state, run_end_ms);
            ui.expand_to_include_rect(dock_rect);
        });
    egui::CentralPanel::default().show_inside(ui, |ui| {
    egui::ScrollArea::vertical()
        .id_salt("metrics_destinations_matrices")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.weak("Destination = distribution center (DC) for Cencora.");
            ui.add_space(4.0);
            // Period/scope/trailing controls govern the chart's bucketing
            // and the cursor's slice width.
            draw_common_controls(ui, state, true);
            ui.horizontal(|ui| {
                ui.label("Matrix");
                for source in [
                    DestinationsSourceSelection::Picks,
                    DestinationsSourceSelection::Planned,
                ] {
                    if ui
                        .selectable_label(state.destinations_source == source, source.label())
                        .clicked()
                    {
                        state.destinations_source = source;
                    }
                }
            });
            ui.add_space(4.0);
            // Both matrices keep ONE axes ordering — the cursor-selected
            // stripe snapshot's, when the run has one — so toggling
            // Picks↔Planned never silently reorders rows or columns (they
            // are built to be compared).
            let plan_axes: Option<(Vec<String>, Vec<u32>)> =
                striping_overall.as_ref().ok().and_then(|overall| {
                    let snapshots = &overall.data.plan_snapshots;
                    if snapshots.is_empty() {
                        return None;
                    }
                    let at_ms = state.chart_cursor_ms.unwrap_or(run_end_ms);
                    let snapshot = &snapshots[snapshot_index(snapshots, at_ms)];
                    let destinations: Vec<String> =
                        snapshot.destinations.iter().map(|d| d.destination.clone()).collect();
                    Some((destinations, snapshot.stations.iter().map(|station| station.0).collect()))
                });
            match state.destinations_source {
                DestinationsSourceSelection::Picks => draw_destinations_picks(
                    ui,
                    picks_overall,
                    picks_temporal,
                    plan_axes.as_ref(),
                    state,
                ),
                DestinationsSourceSelection::Planned => {
                    draw_destinations_planned(ui, striping_overall, state, run_end_ms)
                }
            }
            // Chart-side exclusions, surfaced the way the Picks matrix
            // surfaces its own — counted facts are never silently dropped.
            // Sums come from the overall report (single bucket), where
            // every presentation counts exactly once; trailing buckets
            // overlap and would double-count.
            if let Ok(overall) = striping_overall {
                let unattributed: u64 = overall
                    .data
                    .unattributed_rows
                    .iter()
                    .map(|row| row.destination_unattributed_presentations)
                    .sum();
                if unattributed > 0 {
                    ui.colored_label(
                        theme::WARN,
                        format!("{unattributed} completed presentations had no destination and are excluded from the chart."),
                    );
                }
                let unplanned: u64 = overall
                    .data
                    .unattributed_rows
                    .iter()
                    .map(|row| row.unplanned_destination_presentations)
                    .sum();
                if unplanned > 0 {
                    ui.colored_label(
                        theme::WARN,
                        format!("{unplanned} presentations were for destinations absent from the stripe plan (plan-vs-fact drift)."),
                    );
                }
            }
        });
    });
    // Same persistence contract for the outer metrics panel: the pair of
    // inner panels must account for the full rect we were given.
    ui.expand_to_include_rect(full_rect);
}

/// The planned stripe-share matrix — the plan the run was composed with,
/// never mixed with completed facts (planned-vs-completed separation rule,
/// specs/dematic_kpi_reporting/plan.md).
fn draw_destinations_planned(
    ui: &mut egui::Ui,
    report: &Result<MetricsReport<DestinationStripingData>, ReportBuildError>,
    state: &MetricsPanelState,
    run_end_ms: u64,
) {
    let Ok(report) = report else {
        ui.colored_label(theme::ERR, "Unable to build the destination striping report.");
        return;
    };
    let snapshots = &report.data.plan_snapshots;
    if snapshots.is_empty() {
        unavailable(ui, "This run carried no stripe plan (no bulk demand pool).");
        return;
    }

    // The chart cursor is the temporal selector (a step-function series:
    // one t=0 snapshot under the static v1 map; dynamic restriping later
    // populates the axis). No cursor = the latest snapshot.
    let at_ms = state.chart_cursor_ms.unwrap_or(run_end_ms);
    let index = snapshot_index(snapshots, at_ms);
    let snapshot = &snapshots[index];

    ui.weak(format!(
        "Assigned stripes — configuration, not measurement. A cell is the station's share of \
         the destination's demand interval. Snapshot {}/{} (took effect {:.0} min{}){}.",
        index + 1,
        snapshots.len(),
        snapshot.from_ms as f64 / 60_000.0,
        if snapshots.len() == 1 { " — constant this run" } else { "" },
        if state.chart_cursor_ms.is_some() { "; following the chart cursor" } else { "" },
    ));
    if snapshot.rows.is_empty() {
        unavailable(ui, "The stripe plan is empty.");
        return;
    }

    // The stripe ribbon: the cumulative-demand line [0, total_units) as one
    // horizontal band, segmented by destination in plan order, with ticks
    // where station boundaries fall — demand widths, straddles, and the
    // boundary geometry shown exactly, which the share matrix below cannot.
    {
        let total = snapshot.total_units.max(1) as f64;
        let n_stations = snapshot.stations.len().max(1);
        let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
        let rect = response.rect;
        let unit_x = |units: f64| rect.left() + (units / total) as f32 * rect.width();
        // Per-(destination, station) shares for the sub-segment labels and
        // the pointer tip.
        let share_cells: BTreeMap<(&str, u32), Ratio> = snapshot
            .rows
            .iter()
            .map(|row| ((row.destination.as_str(), row.station_id.0), row.share_of_destination))
            .collect();
        let mut cum: u64 = 0;
        let mut segments: Vec<(f32, f32, &str, u64)> = Vec::new();
        for (index, dest) in snapshot.destinations.iter().enumerate() {
            let (lo, hi) = (cum as f64, (cum + dest.units) as f64);
            cum += dest.units;
            let (x0, x1) = (unit_x(lo), unit_x(hi));
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
                0.0,
                destination_color(index).gamma_multiply(0.85),
            );
            segments.push((x0, x1, dest.destination.as_str(), dest.units));
        }
        for k in 1..n_stations {
            let x = rect.left() + (k as f64 / n_stations as f64) as f32 * rect.width();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                Stroke::new(2.0, ui.visuals().strong_text_color()),
            );
        }
        // Instant tooltip (Daniel: no hover delay) — drawn directly every
        // frame the ribbon is hovered, naming the destination AND the
        // station whose interval is under the pointer: a straddling
        // destination shows the side of the boundary you are on, not its
        // whole station list.
        if response.hovered() {
            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new("destinations_ribbon_tip"), |ui| {
                let Some(pos) = ui.ctx().pointer_latest_pos() else { return };
                let Some((_, _, dest, units)) = segments.iter().find(|(x0, x1, _, _)| (*x0..=*x1).contains(&pos.x))
                else {
                    return;
                };
                let share = 100.0 * *units as f64 / total;
                ui.label(format!("{dest} — {units} units ({share:.1}% of demand)"));
                let frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 0.9999);
                let k = (frac as f64 * n_stations as f64) as usize;
                if let Some(station) = snapshot.stations.get(k) {
                    let here = share_cells
                        .get(&(*dest, station.0))
                        .map(|cell| format_percent(cell.percent()))
                        .unwrap_or_else(|| "—".to_string());
                    ui.label(format!("stn {} — {dest}'s share here: {here}", station.0));
                }
            });
        }
    }
    ui.add_space(4.0);

    // The ribbon is the primary Assigned-stripes view; the share matrix is
    // a collapsed detail behind it (Daniel, 2026-07-30). Destination rows
    // in plan (first-appearance) order — from the snapshot's own row axis,
    // so a zero-volume destination renders as an empty row instead of
    // vanishing; station columns in planned physical order.
    egui::CollapsingHeader::new("Share matrix (detail)").default_open(false).show(ui, |ui| {
    let plan_rows = &snapshot.rows;
    let destinations: Vec<&str> =
        snapshot.destinations.iter().map(|d| d.destination.as_str()).collect();
    let units_by_destination: BTreeMap<&str, u64> =
        snapshot.destinations.iter().map(|d| (d.destination.as_str(), d.units)).collect();
    let total_units = snapshot.total_units.max(1);
    let cells: BTreeMap<(&str, u32), Ratio> = plan_rows
        .iter()
        .map(|row| ((row.destination.as_str(), row.station_id.0), row.share_of_destination))
        .collect();

    egui::ScrollArea::horizontal()
        .id_salt("destinations_planned_scroll")
        .show(ui, |ui| {
            egui::Grid::new("destinations_planned_matrix")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Destination");
                    for station in &snapshot.stations {
                        ui.strong(station.to_string());
                    }
                    ui.end_row();
                    for dest in &destinations {
                        let units = units_by_destination.get(dest).copied().unwrap_or(0);
                        let demand_pct = 100.0 * units as f64 / total_units as f64;
                        ui.strong(*dest).on_hover_text(format!(
                            "Demand: {units} units ({demand_pct:.1}% of total)"
                        ));
                        for station in &snapshot.stations {
                            match cells.get(&(*dest, station.0)) {
                                None => {
                                    ui.label("");
                                }
                                Some(share) => {
                                    let percent = share.percent();
                                    heatmap_cell(ui, format_percent(percent), share.fraction().unwrap_or(0.0))
                                        .on_hover_text(format!(
                                            "Station share of destination: {}/{} ({})\nDestination demand: {units} units ({demand_pct:.1}% of total)",
                                            share.numerator,
                                            share.denominator,
                                            format_percent(percent),
                                        ));
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    });
}

/// The docked throughput chart doubles as the timeline cursor: one line of
/// distinct presentations per bucket per destination, and clicking selects
/// the sim time both matrices render (Picks = the bucket under the cursor,
/// Assigned stripes = the governing snapshot). Plot interactions are
/// clamped — no pan, no scroll-zoom — so the view never drifts.
fn draw_destination_throughput_chart(
    ui: &mut egui::Ui,
    striping_temporal: &Result<MetricsReport<DestinationStripingData>, ReportBuildError>,
    state: &mut MetricsPanelState,
    run_end_ms: u64,
) {
    let Ok(report) = striping_temporal else {
        ui.colored_label(theme::ERR, "Unable to build the destination striping report.");
        return;
    };
    // The cursor's bucket, resolved by the reporting module's own
    // membership rule, clamped at the range ends — a set cursor always
    // names a real bucket, so header, cursor line, and matrices can never
    // disagree. An empty schedule (e.g. a trailing window longer than the
    // run) is its own condition, distinct from having no picks (Codex
    // review).
    let buckets = report_buckets(report.request).unwrap_or_default();
    if buckets.is_empty() {
        state.chart_cursor_ms = None;
        ui.weak(
            "No complete buckets for this period — the trailing window may be longer than the \
             run. Shorten the window or choose a fixed period.",
        );
        return;
    }
    if report.data.presentation_rows.is_empty() {
        unavailable(ui, "No destination-attributed pick presentations in this run.");
        return;
    }
    let cursor_bucket = state.chart_cursor_ms.and_then(|cursor| bucket_at(&buckets, cursor, report.request));
    // Legend entries are the bare destination ids — deliberately no
    // station annotation: under dynamic restriping a destination's
    // stations change over the run, so a single per-line station label
    // would lie about part of the series (Daniel, 2026-07-30). The
    // cursor-following Assigned-stripes matrix is the station view.
    // Destination list preserving the producer's row order (plan
    // first-appearance, then observed-only appended), so legend order
    // matches the matrices' row order.
    let mut destinations: Vec<&str> = Vec::new();
    for row in &report.data.presentation_rows {
        if !destinations.contains(&row.destination.as_str()) {
            destinations.push(row.destination.as_str());
        }
    }
    // More destinations than series colors is the normal regime (26 over
    // 10 in cencora), so the picker isolates one line when the legend
    // alone can't disambiguate.
    let picked = state.chart_destination_index.filter(|index| *index < destinations.len());
    ui.horizontal(|ui| {
        ui.label(RichText::new("Pick destination throughput (distinct presentations per bucket)").strong());
        egui::ComboBox::from_id_salt("destinations_chart_pick")
            .selected_text(picked.map_or("All destinations", |index| destinations[index]))
            .show_ui(ui, |ui| {
                if ui.selectable_label(picked.is_none(), "All destinations").clicked() {
                    state.chart_destination_index = None;
                }
                for (index, dest) in destinations.iter().enumerate() {
                    if ui.selectable_label(picked == Some(index), *dest).clicked() {
                        state.chart_destination_index = Some(index);
                    }
                }
            });
        match cursor_bucket {
            Some(bucket) => {
                ui.weak(format!(
                    "slice {:.0}–{:.0} min — matrices show this bucket",
                    bucket.start_ms as f64 / 60_000.0,
                    bucket.end_ms as f64 / 60_000.0,
                ));
                if ui.small_button("clear (run totals)").clicked() {
                    state.chart_cursor_ms = None;
                }
            }
            None => {
                ui.weak("click the chart to slice the matrices at a time");
            }
        }
    });
    let shown: Vec<(usize, &str)> = match picked {
        Some(index) => vec![(index, destinations[index])],
        None => destinations.iter().copied().enumerate().collect(),
    };
    // Step traces (Daniel, 2026-07-30): each bucket's value holds flat
    // across its tread — [max(bucket start, previous end), bucket end] —
    // so the quantization is visible instead of implied by a
    // point-to-point ramp. For fixed periods the tread IS the bucket; for
    // trailing, it is the emission step, ending on the window's end.
    let mut y_max: f64 = 1.0;
    let series: Vec<(String, Color32, Vec<[f64; 2]>)> = shown
        .into_iter()
        .map(|(index, dest)| {
            let mut points: Vec<[f64; 2]> = Vec::new();
            let mut previous_end_ms: Option<u64> = None;
            for row in report.data.presentation_rows.iter().filter(|row| row.destination == dest) {
                let tread_start_ms = row.bucket.start_ms.max(previous_end_ms.unwrap_or(0));
                let value = row.presentations as f64;
                y_max = y_max.max(value);
                points.push([tread_start_ms as f64 / 60_000.0, value]);
                points.push([row.bucket.end_ms as f64 / 60_000.0, value]);
                previous_end_ms = Some(row.bucket.end_ms);
            }
            (dest.to_string(), destination_color(index), points)
        })
        .collect();

    let mut clicked_minutes: Option<f64> = None;
    // Fill the (resizable) chart dock exactly — any fixed subtraction or
    // floor here re-opens the panel-height ratchet (the dock's min_height
    // is the single floor).
    let plot_height = ui.available_height();
    Plot::new("destinations_throughput_cursor")
        .height(plot_height)
        .legend(Legend::default())
        .include_y(0.0)
        .include_x(0.0)
        .include_x(run_end_ms as f64 / 60_000.0)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show(ui, |plot_ui| {
            for (label, color, points) in series {
                plot_ui.line(Line::new(PlotPoints::new(points)).name(label).color(color).width(2.0));
            }
            if let Some(bucket) = cursor_bucket {
                // The selected bucket is shaded across its whole span —
                // the highlighted region IS the region the matrices show.
                let (x0, x1) = (bucket.start_ms as f64 / 60_000.0, bucket.end_ms as f64 / 60_000.0);
                let top = y_max * 1.05;
                plot_ui.polygon(
                    Polygon::new(PlotPoints::new(vec![[x0, 0.0], [x1, 0.0], [x1, top], [x0, top]]))
                        .fill_color(theme::WARN.gamma_multiply(0.18))
                        .stroke(Stroke::new(1.0, theme::WARN)),
                );
            }
            if plot_ui.response().clicked() {
                if let Some(coord) = plot_ui.pointer_coordinate() {
                    clicked_minutes = Some(coord.x);
                }
            }
        });
    if let Some(minutes) = clicked_minutes {
        let clicked_ms = (minutes * 60_000.0).clamp(0.0, run_end_ms as f64) as u64;
        if let Some(bucket) = bucket_at(&buckets, clicked_ms, report.request) {
            // Store the bucket's midpoint: strictly interior under either
            // membership convention (fixed half-open, trailing half-open
            // from the other side), so every consumer resolves exactly the
            // bucket the click selected.
            state.chart_cursor_ms = Some((bucket.start_ms + bucket.end_ms) / 2);
        }
    }
}

fn draw_destinations_picks(
    ui: &mut egui::Ui,
    report: &Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    temporal: &Result<MetricsReport<CompletedWorkData>, ReportBuildError>,
    plan_axes: Option<&(Vec<String>, Vec<u32>)>,
    state: &mut MetricsPanelState,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Measure");
        egui::ComboBox::from_id_salt("dc_measure").selected_text(state.dc_measure.label()).show_ui(ui, |ui| {
            for measure in [DcMeasureSelection::Totes, DcMeasureSelection::Orders] {
                ui.selectable_value(&mut state.dc_measure, measure, measure.label());
            }
        });
        ui.separator();
        ui.label("Display");
        egui::ComboBox::from_id_salt("dc_display").selected_text(state.dc_display.label()).show_ui(ui, |ui| {
            for display in [MatrixDisplaySelection::Count, MatrixDisplaySelection::Percentage] {
                ui.selectable_value(&mut state.dc_display, display, display.label());
            }
        });
    });

    let Ok(report) = report else {
        ui.colored_label(theme::ERR, "Unable to build the Destinations completed-work report.");
        return;
    };

    // Cursor slice: with a chart cursor set, render the bucket under it
    // from the temporal report instead of run totals. The bucket schedule
    // comes from the temporal request itself, resolved by the reporting
    // module's own membership rule via `bucket_at` (clamped at the ends) —
    // a set cursor always names a real bucket, never a silent fallback.
    let slice: Option<(TimeBucket, Vec<DcStationWorkRow>)> = state.chart_cursor_ms.and_then(|cursor| {
        let temporal = temporal.as_ref().ok()?;
        let buckets = report_buckets(temporal.request).ok()?;
        let bucket = bucket_at(&buckets, cursor, temporal.request)?;
        let rows = temporal.data.dc_station_rows.iter().filter(|row| row.bucket == bucket).cloned().collect();
        Some((bucket, rows))
    });

    let dc_unattributed: u64 = report.data.unattributed_rows.iter().map(|row| row.dc_unattributed_presentations).sum();
    if dc_unattributed > 0 {
        ui.colored_label(
            theme::WARN,
            format!("{dc_unattributed} completed presentations had no DC and are excluded from this matrix."),
        );
    }
    let unattributed_orders: u64 = report.data.unattributed_rows.iter().map(|row| row.unattributed_orders).sum();
    if unattributed_orders > 0 {
        ui.colored_label(
            theme::WARN,
            format!("{unattributed_orders} completed orders lacked a station or DC and are excluded from this matrix."),
        );
    }

    let rows: &[DcStationWorkRow] = match &slice {
        Some((_, slice_rows)) => slice_rows,
        None => &report.data.dc_station_rows,
    };
    if rows.is_empty() && slice.is_none() {
        unavailable(ui, "No completed tote or order work in this run contains DC attribution.");
        return;
    }
    // Axes follow the stripe plan's ordering when one exists (planned rows
    // never observed still render, as measured zeros); observed-only
    // destinations/stations append after. Without a plan, the pre-striping
    // sorted orders remain.
    let observed_dcs: BTreeSet<&str> = rows.iter().map(|row| row.dc_id.as_str()).collect();
    let observed_stations: BTreeSet<StationId> = rows.iter().map(|row| row.station_id).collect();
    let (dcs, stations): (Vec<&str>, Vec<StationId>) = match plan_axes {
        Some((plan_dcs, plan_station_ids)) => {
            let mut dcs: Vec<&str> = plan_dcs.iter().map(String::as_str).collect();
            dcs.extend(observed_dcs.iter().copied().filter(|dc| !plan_dcs.iter().any(|p| p == dc)));
            let mut stations: Vec<StationId> = plan_station_ids.iter().map(|&id| StationId(id)).collect();
            stations.extend(observed_stations.iter().copied().filter(|s| !plan_station_ids.contains(&s.0)));
            (dcs, stations)
        }
        None => (observed_dcs.into_iter().collect(), observed_stations.into_iter().collect()),
    };
    let max_count = rows.iter().map(|row| dc_count(row, state.dc_measure)).max().unwrap_or(1).max(1);
    let mut hovered_dc_index = None;
    let mut clicked_dc_index = None;

    ui.add_space(6.0);
    let matrix_title = match &slice {
        Some((bucket, _)) => format!(
            "DC × pick station — slice {:.0}–{:.0} min (chart cursor)",
            bucket.start_ms as f64 / 60_000.0,
            bucket.end_ms as f64 / 60_000.0,
        ),
        None => "DC × pick station — run totals".to_string(),
    };
    ui.label(RichText::new(matrix_title).strong());
    ui.weak("Heat is scaled to this view's own maximum, not across slices.");
    if dcs.is_empty() {
        unavailable(ui, "No DC-attributed work in this slice.");
        return;
    }
    ui.weak("Dematic calls a pick station a palletizer; tote and order labels use Foundry terminology.");
    egui::Grid::new("dc_station_matrix").striped(true).show(ui, |ui| {
        ui.strong("DC");
        for station in &stations {
            ui.strong(station.to_string());
        }
        ui.end_row();
        for (dc_index, dc) in dcs.iter().enumerate() {
            let response = ui.add(egui::Label::new(RichText::new(*dc).strong()).sense(egui::Sense::click()));
            if response.hovered() {
                hovered_dc_index = Some(dc_index);
            }
            if response.clicked() {
                clicked_dc_index = Some(dc_index);
            }
            for station in &stations {
                let row = rows.iter().find(|row| row.dc_id == *dc && row.station_id == *station);
                let count = row.map_or(0, |row| dc_count(row, state.dc_measure));
                let ratio = row.map_or(Ratio::new(0, 0), |row| dc_share(row, state.dc_measure));
                let percent = ratio.percent();
                let (text, intensity) = match state.dc_display {
                    MatrixDisplaySelection::Count => (count.to_string(), count as f64 / max_count as f64),
                    MatrixDisplaySelection::Percentage => (format_percent(percent), percent.unwrap_or(0.0) / 100.0),
                };
                let response = heatmap_cell(ui, text, intensity).on_hover_text(format!(
                    "{} count: {count}\nStation share of DC: {}/{} ({})",
                    state.dc_measure.label(),
                    ratio.numerator,
                    ratio.denominator,
                    format_percent(percent),
                ));
                if response.hovered() {
                    hovered_dc_index = Some(dc_index);
                }
                if response.clicked() {
                    clicked_dc_index = Some(dc_index);
                }
            }
            ui.end_row();
        }
    });

    if let Some(index) = clicked_dc_index {
        state.selected_dc_index = (state.selected_dc_index != Some(index)).then_some(index);
    }
    let pinned_dc_index = state.selected_dc_index.filter(|index| *index < dcs.len());
    let detail_dc_index = hovered_dc_index.or(pinned_dc_index);
    if let Some(dc_index) = detail_dc_index {
        let dc = dcs[dc_index];
        let values: Vec<_> = stations
            .iter()
            .enumerate()
            .map(|(index, station)| {
                let value = rows
                    .iter()
                    .find(|row| row.dc_id == dc && row.station_id == *station)
                    .map_or(0, |row| dc_count(row, state.dc_measure));
                (station.to_string(), value, SERIES_COLORS[index % SERIES_COLORS.len()])
            })
            .collect();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} {} distribution", dc, state.dc_measure.label().to_lowercase())).strong(),
            );
            if pinned_dc_index == Some(dc_index) {
                ui.weak("Pinned");
                if ui.small_button("Clear").clicked() {
                    state.selected_dc_index = None;
                }
            }
        });
        draw_pie(ui, "dc_hover_pie", &values, 170.0);
    } else {
        ui.weak("Hover a DC row to inspect its pick-station distribution; click to pin it.");
    }
}

/// Pre-run enumeration for a metrics view: the descriptors its producers
/// will emit, obtained by calling the producers' own descriptor functions —
/// discovery's answer to "what will exist" before any data does. No catalog,
/// no hardcoded metric list.
pub(crate) fn draw_pre_run_metric_list(ui: &mut egui::Ui, view: MetricsView) {
    let descriptors = match view {
        MetricsView::TimeSeries => Vec::new(),
        MetricsView::Da50State => da50_state_descriptors(),
        MetricsView::D30State => d30_state_descriptors(),
        MetricsView::Da50Throughput => da50_operations_descriptors(),
        MetricsView::OrderExtraction => extraction_descriptors(),
        MetricsView::BufferStorage => aisle_occupancy_descriptors(),
        MetricsView::AisleDistribution => completed_work_descriptors(),
        MetricsView::Destinations => {
            let mut descriptors = completed_work_descriptors();
            descriptors.extend(destination_striping_descriptors());
            descriptors
        }
    };
    // Fill the whole panel like the report views do: egui persists the
    // CONTENT rect as the panel's height, so a short placeholder would
    // snap a user-dragged panel back to content size on every frame of a
    // running sim.
    egui::ScrollArea::vertical().id_salt(format!("metrics_prerun_{view:?}")).auto_shrink([false, false]).show(
        ui,
        |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.heading(view.label());
                ui.label("Metrics become available when the run completes.");
            });
            if descriptors.is_empty() {
                return;
            }
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("This view will report:").strong());
                for descriptor in &descriptors {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&descriptor.label).strong());
                        ui.weak(&descriptor.id);
                    });
                    ui.weak(&descriptor.description);
                }
            });
        },
    );
}

fn aisle_dimension_label(zone_id: u32) -> String {
    format!("z{zone_id}")
}

fn draw_line_chart(ui: &mut egui::Ui, id: &'static str, title: &str, series: Vec<(String, Color32, Vec<[f64; 2]>)>) {
    ui.label(RichText::new(title).strong());
    Plot::new(id)
        .height(210.0)
        .legend(Legend::default())
        .include_y(0.0)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show(ui, |plot_ui| {
            for (label, color, points) in series {
                plot_ui.line(Line::new(PlotPoints::new(points)).name(label).color(color).width(2.0));
            }
        });
}

fn draw_pie(ui: &mut egui::Ui, id: &'static str, values: &[(impl AsRef<str>, u64, Color32)], size: f32) {
    draw_pie_impl(ui, id, values, size, true);
}

/// Legend-less pie for the state tabs, where the merged states-and-waiting
/// table carries the labels and swatches instead (Daniel's ruling,
/// 2026-07-30: one table, doubling as the legend).
fn draw_pie_bare(ui: &mut egui::Ui, id: &'static str, values: &[(impl AsRef<str>, u64, Color32)], size: f32) {
    draw_pie_impl(ui, id, values, size, false);
}

fn draw_pie_impl(
    ui: &mut egui::Ui,
    id: &'static str,
    values: &[(impl AsRef<str>, u64, Color32)],
    size: f32,
    show_legend: bool,
) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            let center = rect.center();
            let radius = rect.width().min(rect.height()) * 0.45;
            let total: u64 = values.iter().map(|(_, value, _)| *value).sum();
            if total == 0 {
                ui.painter().circle_stroke(center, radius, Stroke::new(1.0, theme::BORDER_STRONG));
                ui.painter().text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    "No data",
                    egui::TextStyle::Body.resolve(ui.style()),
                    theme::TEXT_1,
                );
            } else {
                let mut start = -TAU / 4.0;
                for (_, value, color) in values {
                    if *value == 0 {
                        continue;
                    }
                    let sweep = TAU * *value as f32 / total as f32;
                    let segments = ((sweep.abs() * 18.0).ceil() as usize).max(2);
                    let mut points = Vec::with_capacity(segments + 2);
                    points.push(center);
                    for index in 0..=segments {
                        let angle = start + sweep * index as f32 / segments as f32;
                        points.push(center + radius * egui::vec2(angle.cos(), angle.sin()));
                    }
                    ui.painter().add(egui::Shape::convex_polygon(points, *color, Stroke::NONE));
                    start += sweep;
                }
            }

            if show_legend {
                ui.vertical(|ui| {
                    for (label, value, color) in values {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, *color);
                            let percent = if total == 0 { 0.0 } else { *value as f64 / total as f64 * 100.0 };
                            ui.label(format!("{}  {:.1}%", label.as_ref(), percent));
                        });
                    }
                });
            }
        });
    });
}

/// One flat display row of the states table on the DA50/D30 state tabs.
pub(crate) struct SummaryRow {
    pub(crate) label: &'static str,
    /// Pie swatch; `None` for rows that cannot chart (Unsupported/Unknown).
    pub(crate) color: Option<Color32>,
    pub(crate) status: SupportStatus,
    pub(crate) duration_ms: Option<u64>,
    /// Pre-rendered share-of-fleet-time text ("" when status text in the
    /// Time column already says Unknown/Not modeled — repeating it is noise).
    share_text: String,
}

/// Build the FLAT state list: states in canonical order, with the Waiting
/// rollup replaced in place by its reasons promoted to top level, all shares
/// re-expressed against fleet time (Daniel's ruling, 2026-07-30: no Waiting
/// group until drill-down exists). The same list feeds the pie and the
/// table — the table IS the pie legend.
pub(crate) fn flat_summary_rows<S: Copy + PartialEq, R: Copy>(
    states: &[(S, SupportStatus, Option<u64>, Option<Ratio>)],
    reasons: &[(R, SupportStatus, Option<u64>, Option<Ratio>)],
    state_label: fn(S) -> &'static str,
    state_color: fn(S) -> Color32,
    waiting_state: S,
    reason_label: fn(R) -> &'static str,
    reason_color: fn(R) -> Option<Color32>,
) -> Vec<SummaryRow> {
    // Fleet (or selected-vehicle) coverage: the measured states partition it.
    let total_ms: u64 = states
        .iter()
        .filter(|(_, status, _, _)| *status == SupportStatus::Measured)
        .filter_map(|(_, _, duration_ms, _)| *duration_ms)
        .sum();
    let fleet_share_text = |duration_ms: u64| -> String {
        if total_ms == 0 { "—".to_string() } else { format!("{:.1}%", duration_ms as f64 / total_ms as f64 * 100.0) }
    };
    let mut rows = Vec::new();
    for (metric_state, status, duration_ms, share) in states {
        if *metric_state == waiting_state {
            for (reason, reason_status, reason_ms, _) in reasons {
                rows.push(SummaryRow {
                    label: reason_label(*reason),
                    color: (*reason_status == SupportStatus::Measured).then(|| reason_color(*reason)).flatten(),
                    status: *reason_status,
                    duration_ms: *reason_ms,
                    share_text: match (*reason_status, *reason_ms) {
                        (SupportStatus::Measured, Some(ms)) => fleet_share_text(ms),
                        _ => String::new(),
                    },
                });
            }
        } else {
            rows.push(SummaryRow {
                label: state_label(*metric_state),
                color: (*status == SupportStatus::Measured).then(|| state_color(*metric_state)),
                status: *status,
                duration_ms: *duration_ms,
                // Same convention as the reason rows: the Time column
                // already carries Unknown/Not modeled, so the Share column
                // stays empty rather than repeating it.
                share_text: match *status {
                    SupportStatus::Measured => supported_ratio(*status, *share),
                    _ => String::new(),
                },
            });
        }
    }
    rows
}

fn draw_state_summary(ui: &mut egui::Ui, grid_id: &'static str, rows: &[SummaryRow]) {
    egui::Grid::new(grid_id).striped(true).show(ui, |ui| {
        ui.label("");
        ui.strong("State");
        ui.strong("Time");
        ui.strong("Share");
        ui.end_row();
        for row in rows {
            if let Some(color) = row.color {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, color);
            } else {
                ui.label("");
            }
            ui.label(row.label);
            ui.label(supported_duration(row.status, row.duration_ms));
            ui.label(row.share_text.clone());
            ui.end_row();
        }
    });
}

fn heatmap_cell(ui: &mut egui::Ui, text: String, intensity: f64) -> egui::Response {
    let intensity = intensity.clamp(0.0, 1.0) as f32;
    let background = Color32::from_rgb(
        (239.0 - 70.0 * intensity) as u8,
        (246.0 - 45.0 * intensity) as u8,
        (255.0 - 5.0 * intensity) as u8,
    );
    ui.add(egui::Label::new(RichText::new(text).monospace().background_color(background)).sense(egui::Sense::click()))
}

fn unavailable(ui: &mut egui::Ui, message: &str) {
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.heading("Metric unavailable");
        ui.label(message);
    });
}

fn supported_duration(status: SupportStatus, value: Option<u64>) -> String {
    match (status, value) {
        (SupportStatus::Measured, Some(value)) => format_duration(value),
        (SupportStatus::Unsupported, _) => "Not modeled".to_string(),
        _ => "Unknown".to_string(),
    }
}

fn supported_ratio(status: SupportStatus, value: Option<Ratio>) -> String {
    match (status, value) {
        (SupportStatus::Measured, value) => format_ratio(value),
        (SupportStatus::Unsupported, _) => "Not modeled".to_string(),
        _ => "Unknown".to_string(),
    }
}

fn format_ratio(value: Option<Ratio>) -> String {
    value.and_then(Ratio::percent).map_or_else(|| "—".to_string(), |percent| format!("{percent:.1}%"))
}

fn format_percent(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |value| format!("{value:.1}%"))
}

fn format_duration(ms: u64) -> String {
    let seconds = ms as f64 / 1_000.0;
    if seconds >= 3_600.0 {
        format!("{:.2} h", seconds / 3_600.0)
    } else if seconds >= 60.0 {
        format!("{:.1} min", seconds / 60.0)
    } else {
        format!("{seconds:.1} s")
    }
}

fn format_distance(mm: u64) -> String {
    if mm >= 1_000_000 { format!("{:.2} km", mm as f64 / 1_000_000.0) } else { format!("{:.1} m", mm as f64 / 1_000.0) }
}

fn state_color(state: Da50State) -> Color32 {
    match state {
        Da50State::Pickup => SERIES_COLORS[0],
        Da50State::Drop => SERIES_COLORS[1],
        Da50State::Alignment => SERIES_COLORS[2],
        Da50State::EmptyTravel => SERIES_COLORS[5],
        Da50State::LadenTravel => SERIES_COLORS[3],
        Da50State::Waiting => SERIES_COLORS[6],
        Da50State::TravelToPark => SERIES_COLORS[8],
        Da50State::TravelToUnblockOther => SERIES_COLORS[4],
        Da50State::Unknown => SERIES_COLORS[9],
    }
}

pub(crate) fn d30_state_color(state: D30State) -> Color32 {
    match state {
        D30State::Pickup => SERIES_COLORS[0],
        D30State::Drop => SERIES_COLORS[1],
        D30State::Alignment => SERIES_COLORS[2],
        D30State::EmptyTravel => SERIES_COLORS[5],
        D30State::LadenTravel => SERIES_COLORS[3],
        D30State::TravelToPark => SERIES_COLORS[8],
        D30State::StationProcessing => SERIES_COLORS[4],
        D30State::Waiting => SERIES_COLORS[6],
        D30State::Unknown => SERIES_COLORS[9],
    }
}

/// Pie/table colors for promoted DA50 waiting reasons. Every reason takes a
/// palette slot no DA50 state uses, so the flat pie and its legend-table
/// never show two rows in one color.
fn wait_color(reason: Da50WaitReason) -> Option<Color32> {
    match reason {
        Da50WaitReason::NoTask => Some(SERIES_COLORS[6]),
        Da50WaitReason::AwaitingDemand => Some(SERIES_COLORS[7]),
        Da50WaitReason::AwaitingTote => Some(SERIES_COLORS[10]),
        Da50WaitReason::AwaitingSlot => Some(SERIES_COLORS[13]),
        Da50WaitReason::AwaitingD30Deposit => Some(SERIES_COLORS[12]),
        Da50WaitReason::Unknown => Some(SERIES_COLORS[14]),
        Da50WaitReason::ConflictingFacts => Some(SERIES_COLORS[11]),
    }
}

/// Pie/table/Timeline colors for the ratified flat D30 waiting taxonomy.
/// These occupy slots unused by non-Waiting D30 states, so the 18 possible
/// flat rows remain distinguishable when #275 makes the mechanical states
/// measurable.
pub(crate) fn d30_wait_category_color(category: D30WaitCategory) -> Color32 {
    match category {
        D30WaitCategory::NoTaskParked => SERIES_COLORS[6],
        D30WaitCategory::NoTaskElsewhere => SERIES_COLORS[7],
        D30WaitCategory::Presenting => SERIES_COLORS[10],
        D30WaitCategory::AwaitingStationDeparture => D30_EXTRA_COLORS[4],
        // The panel charts the D30 report, which lists no row for it, so it never shares a chart with its kin.
        D30WaitCategory::AwaitingDepartureElsewhere => D30_EXTRA_COLORS[4],
        D30WaitCategory::AwaitingRelayDepositResident => SERIES_COLORS[12],
        D30WaitCategory::AwaitingRelayDepositDrove => D30_EXTRA_COLORS[3],
        D30WaitCategory::AwaitingRelaySlot => SERIES_COLORS[13],
        D30WaitCategory::InQueueStation => D30_EXTRA_COLORS[0],
        D30WaitCategory::Congestion => D30_EXTRA_COLORS[1],
        D30WaitCategory::Unknown => SERIES_COLORS[14],
        D30WaitCategory::NoFacts => D30_EXTRA_COLORS[2],
        D30WaitCategory::ConflictingFacts => SERIES_COLORS[11],
    }
}

fn operation_label(operation: VtmOperationKind) -> &'static str {
    match operation {
        VtmOperationKind::PickupStorage => "Pickup from storage",
        VtmOperationKind::DropRelay => "Drop to relay",
        VtmOperationKind::PickupRelay => "Pickup from relay",
        VtmOperationKind::DropStorage => "Drop to storage",
    }
}

fn payload_group(selection: MetricsPayloadSelection) -> Da50PayloadGroup {
    match selection {
        MetricsPayloadSelection::All => Da50PayloadGroup::All,
        MetricsPayloadSelection::Empty => Da50PayloadGroup::Empty,
        MetricsPayloadSelection::NonEmpty => Da50PayloadGroup::NonEmpty,
        MetricsPayloadSelection::Unknown => Da50PayloadGroup::Unknown,
    }
}

fn dc_count(row: &DcStationWorkRow, measure: DcMeasureSelection) -> u64 {
    match measure {
        DcMeasureSelection::Totes => row.tote_count,
        DcMeasureSelection::Orders => row.order_count,
    }
}

fn dc_share(row: &DcStationWorkRow, measure: DcMeasureSelection) -> Ratio {
    match measure {
        DcMeasureSelection::Totes => row.tote_station_share_of_dc,
        DcMeasureSelection::Orders => row.order_station_share_of_dc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_request_keeps_window_and_resolution_independent() {
        let state = MetricsPanelState {
            trailing_window_ms: HOUR_MS,
            trailing_resolution_ms: 5 * 60_000,
            ..MetricsPanelState::default()
        };

        let request = request(&state, 4 * HOUR_MS, 0, false);
        let ReportPeriod::Trailing(spec) = request.period else {
            panic!("expected trailing period");
        };
        assert_eq!(spec.window_ms(), HOUR_MS);
        assert_eq!(spec.step_ms(), 5 * 60_000);
    }

    #[test]
    fn terminology_uses_metrics_not_kpis() {
        assert_eq!(MetricsView::Da50State.description(), "DA50 time, waiting attribution, and travel distance");
        assert_eq!(MetricsView::D30State.description(), "D30 time, waiting attribution, and travel distance");
    }

    #[test]
    fn d30_timeline_replaces_waiting_with_categories_on_the_state_denominator() {
        let bucket = TimeBucket::complete(0, 60_000);
        let rows = flatten_d30_timeline_rows(
            vec![
                (bucket, D30State::EmptyTravel, Some(Ratio::new(30, 100))),
                (bucket, D30State::Waiting, Some(Ratio::new(70, 100))),
            ],
            vec![(bucket, D30WaitCategory::NoTaskParked, Some(40)), (bucket, D30WaitCategory::Congestion, Some(30))],
        );

        assert!(!rows.iter().any(|(_, segment, _)| { *segment == D30TimelineSegment::State(D30State::Waiting) }));
        let ratios: Vec<_> = rows.iter().filter_map(|(_, _, ratio)| *ratio).collect();
        assert_eq!(ratios.iter().map(|ratio| ratio.numerator).sum::<u64>(), 100);
        assert!(ratios.iter().all(|ratio| ratio.denominator == 100));
    }

    /// How far apart, in RGB space, any two colors of the D30 flat pie /
    /// table / timeline must sit. The floor is the tightest pair the shared
    /// `SERIES_COLORS` already contributes — Empty travel (cyan) against
    /// Presenting (teal), 42 apart — so the guard fails the moment a new or
    /// edited entry crowds a neighbor worse than the palette's own worst
    /// case. Byte inequality is no guard at all: the pair replaced here
    /// differed by 32 and still read as one wedge.
    const MIN_D30_SEGMENT_COLOR_DISTANCE: f32 = 42.0;

    fn rgb_distance(left: Color32, right: Color32) -> f32 {
        let [left_r, left_g, left_b, _] = left.to_array();
        let [right_r, right_g, right_b, _] = right.to_array();
        let squared = |left: u8, right: u8| (f32::from(left) - f32::from(right)).powi(2);
        (squared(left_r, right_r) + squared(left_g, right_g) + squared(left_b, right_b)).sqrt()
    }

    #[test]
    fn d30_flat_timeline_order_is_complete_and_colors_stay_apart() {
        let segments = d30_timeline_segments();
        assert_eq!(segments.len(), D30_STATES.len() - 1 + D30_WAIT_CATEGORIES.len());
        let waiting_index = D30_STATES.iter().position(|state| *state == D30State::Waiting).unwrap();
        assert_eq!(
            &segments[waiting_index..waiting_index + D30_WAIT_CATEGORIES.len()],
            &D30_WAIT_CATEGORIES.map(D30TimelineSegment::Wait)
        );
        for (index, left) in segments.iter().enumerate() {
            for right in &segments[index + 1..] {
                let distance = rgb_distance(d30_timeline_color(*left), d30_timeline_color(*right));
                assert!(
                    distance >= MIN_D30_SEGMENT_COLOR_DISTANCE,
                    "{} and {} chart side by side but are only {distance:.0} apart in RGB",
                    d30_timeline_label(*left),
                    d30_timeline_label(*right)
                );
            }
        }
    }

    /// No flat state table may print one name on two rows. Each vocabulary
    /// names its own members distinctly, but the table splices the promoted
    /// waiting vocabulary into the state list, so only the spliced list shows
    /// what a reader actually sees — which is how both DA50 residuals came to
    /// be spelled "Unknown" side by side.
    #[test]
    fn flat_state_tables_never_print_one_name_twice() {
        use dessim::metrics::reporting::DA50_WAIT_REASONS;

        let assert_distinct = |family: &str, labels: &[&str]| {
            let distinct: BTreeSet<_> = labels.iter().collect();
            assert_eq!(distinct.len(), labels.len(), "the flat {family} table prints one name on two rows: {labels:?}");
        };

        let da50: Vec<&str> = DA50_STATES
            .into_iter()
            .filter(|state| *state != Da50State::Waiting)
            .map(Da50State::label)
            .chain(DA50_WAIT_REASONS.into_iter().map(Da50WaitReason::label))
            .collect();
        assert_distinct("DA50", &da50);

        let d30: Vec<&str> = d30_timeline_segments().into_iter().map(d30_timeline_label).collect();
        assert_distinct("D30", &d30);
    }

    /// The D30 State sheet and this panel draw the same pie, and the table IS
    /// the pie legend (Daniel's ruling, 2026-07-30), so the two lists have to
    /// agree slice for slice. The panel's side applies `draw_state_tab_body`'s
    /// filter: a row charts iff it has both a colour and a duration. The test
    /// lives here because the sheet's side is a library and this side is egui.
    #[test]
    fn the_d30_state_sheets_pie_slices_are_this_panels() {
        use desgraph::RelayId;
        use dessim::model::RobotId;
        use dessim::observer::output::{
            Activity, HtmHold, HtmJournalEndpoint, HtmLegRecord, HtmWaitCause, HtmWaitPosition, HtmWaitReason,
        };
        use dessim::{SimDuration, SimTime};

        let relay = HtmJournalEndpoint::Relay(RelayId(2));
        let leg = |start_ms, activity, from, to, distance_mm, wait_reason: Option<HtmWaitReason>| HtmLegRecord {
            start: SimTime::ms(start_ms),
            activity,
            from,
            to,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm,
            duration: SimDuration::ms(1_000),
            presentation_id: Some(1),
            robot_id: RobotId(1),
            carrying_cargo: false,
            turn_count: 0,
            wait_reason,
            hold: wait_reason
                .is_some()
                .then_some(HtmHold { cause: HtmWaitCause::NoTask, position: HtmWaitPosition::Relay }),
            notes: String::new(),
        };
        // 90 minutes holding one D30: a second of empty travel, a second of
        // declared idle, and no-leg gaps for everything else.
        let legs = [
            leg(0, Activity::Travel, HtmJournalEndpoint::Parking, relay, 500, None),
            leg(3_600_000, Activity::Wait, relay, relay, 0, Some(HtmWaitReason::NoTask)),
        ];
        let report = d30_state_report(
            &legs,
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 5_400_000, 0),
        )
        .expect("the fixture builds a state report");

        let state_totals: Vec<_> = report
            .data
            .state_rows
            .iter()
            .map(|row| (row.state, row.status, row.duration_ms, row.fleet_share))
            .collect();
        let wait_totals: Vec<_> = report
            .data
            .wait_category_rows
            .iter()
            .map(|row| (row.category, row.status, row.duration_ms, row.waiting_share))
            .collect();
        let panel_pie: Vec<(&str, Option<u64>)> = flat_summary_rows(
            &state_totals,
            &wait_totals,
            D30State::label,
            d30_state_color,
            D30State::Waiting,
            D30WaitCategory::label,
            |category| Some(d30_wait_category_color(category)),
        )
        .iter()
        .filter_map(|row| row.color.map(|_| (row.label, row.duration_ms)))
        .collect();

        assert_eq!(workbook::d30::d30_state_pie_slices(&report.data), panel_pie);
    }
}
