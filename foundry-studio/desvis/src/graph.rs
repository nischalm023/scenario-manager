//! Stacked per-metric mini-charts via egui_plot with time cursor.
//!
//! Each selected metric gets its own chart with an independent Y axis so metrics
//! at wildly different scales (e.g. cumulative picks 0–3920 vs picks/slice 0–10)
//! are all readable.  All charts share the same X axis (sim time in minutes).
//!
//! Each chart can apply a per-metric rolling-window aggregation (mean / sum /
//! max) on top of the raw per-slice series.  The raw line is drawn faded
//! underneath the smoothed line so the user can see what was rolled away.

use rustc_hash::FxHashMap;
use std::cmp::Ordering;
use std::collections::HashMap;

use eframe::egui;
use egui_plot::{
    CoordinatesFormatter, Corner, GridInput, Line, MarkerShape, Plot, PlotPoints, PlotUi, Points, VLine,
    uniform_grid_spacer,
};

use dessim::metrics::profile::SliceRecord;
use dessim::metrics::reporting::trailing_slice_sum;

use crate::theme;
use crate::types::{MetricKind, RollingConfig, RollingKind};

/// Compact number formatter for chart Y-axis: 12345 → "12.3k", 2_500_000 → "2.5M".
fn compact_number(v: f64) -> String {
    let a = v.abs();
    if a >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if a >= 10_000.0 {
        format!("{:.0}k", v / 1_000.0)
    } else if a >= 1_000.0 {
        format!("{:.1}k", v / 1_000.0)
    } else if a >= 10.0 || a == 0.0 {
        format!("{:.0}", v)
    } else {
        format!("{:.2}", v)
    }
}

/// Step-size ladder (minutes) for X-axis grid lines.  Each entry divides the
/// next so the three nested levels egui_plot renders line up on shared marks:
/// 1m, 5m, 15m, 30m, 1h, 2h, 6h, 12h, 24h.
const TIME_STEPS_MIN: [f64; 9] = [1.0, 5.0, 15.0, 30.0, 60.0, 120.0, 360.0, 720.0, 1440.0];

/// Pick three nested time-aligned step sizes for the visible X range.
///
/// Mirrors `egui_plot::log_grid_spacer`, but snaps to a base-60 time ladder
/// instead of powers of ten, so labels land on whole hours / round minutes
/// rather than base-10-round values (50, 100, 150 min...) that render as
/// awkward "1h 40m" labels.
///
/// Returning *three* nested levels (not one) is what lets egui_plot fade finer
/// grid lines in/out smoothly as the range grows: each line's opacity is a
/// function of its on-screen spacing (`pixels_per_unit * step_size`), so the
/// finest level dims and drops out while coarser levels persist — instead of
/// the whole grid swapping at once when a single step crosses a threshold.
fn time_step_sizes(input: GridInput) -> [f64; 3] {
    // `base_step_size` is the smallest data distance still wide enough to draw
    // (≈ the fade threshold in pixels).  Snap up to the finest ladder step that
    // clears it; if the range is coarser than the whole ladder, use the top.
    let base = input.base_step_size.abs();
    let last = TIME_STEPS_MIN.len() - 1;
    let i = match TIME_STEPS_MIN.iter().position(|&s| s >= base) {
        Some(i) => i,
        None => last,
    };
    [TIME_STEPS_MIN[i], TIME_STEPS_MIN[(i + 1).min(last)], TIME_STEPS_MIN[(i + 2).min(last)]]
}

/// Format chart X-axis values (sim time in minutes) as "2h 45m" / "23m" / "45s".
fn format_time_minutes(min: f64) -> String {
    if !min.is_finite() {
        return String::new();
    }
    let total_secs = (min * 60.0).round() as i64;
    if total_secs <= 0 {
        return "0".to_string();
    }
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        if m > 0 { format!("{h}h {m}m") } else { format!("{h}h") }
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

/// Per-metric series cache: raw points, plus a smoothed series when the metric
/// has a non-`Off` rolling configuration.
pub struct MetricSeries {
    pub raw: Vec<[f64; 2]>,
    pub smoothed: Option<Vec<[f64; 2]>>,
}

/// Pre-computed plot points per metric, keyed by MetricKind.
///
/// Invalidated when the number of slices, the selected metrics, the rolling
/// configuration of any selected metric, or the structural scenario inputs
/// the series depends on (`num_stations`, `total_rack_slots`,
/// `total_relay_cells`) change.
pub struct ChartCache {
    pub slice_count: usize,
    /// Structural scenario inputs the per-metric availability verdict depends
    /// on. Tracked so the cache rebuilds when one transitions from its initial
    /// 0 to the value observed once the first slice reports the corresponding
    /// scalar (`rack_capacity` / `relay_cells_total`), rather than serving a
    /// stale available/unavailable verdict (e.g. rack or relay metrics wrongly
    /// left out as unavailable).
    pub num_stations: usize,
    pub total_rack_slots: u32,
    pub total_relay_cells: u32,
    pub cached_metrics: Vec<MetricKind>,
    /// Snapshot of rolling configs that were used to build `series`.  Only
    /// configs for metrics in `cached_metrics` are tracked here.
    pub cached_rolling: HashMap<MetricKind, RollingConfig>,
    pub series: HashMap<MetricKind, MetricSeries>,
    unavailable: FxHashMap<MetricKind, &'static str>,
}

impl ChartCache {
    fn is_valid(
        &self,
        slice_count: usize,
        num_stations: usize,
        total_rack_slots: u32,
        total_relay_cells: u32,
        selected_metrics: &[MetricKind],
        rolling: &HashMap<MetricKind, RollingConfig>,
    ) -> bool {
        if self.slice_count != slice_count {
            return false;
        }
        // All feed the available/unavailable verdict (and num_stations also
        // the pres/stn/hr value), so a change must rebuild — e.g.
        // total_rack_slots going 0 → N once the first slice reports rack_capacity.
        if self.num_stations != num_stations
            || self.total_rack_slots != total_rack_slots
            || self.total_relay_cells != total_relay_cells
        {
            return false;
        }
        if self.cached_metrics != selected_metrics {
            return false;
        }
        for kind in selected_metrics {
            let cached = self.cached_rolling.get(kind);
            let current = rolling.get(kind);
            if cached != current {
                return false;
            }
        }
        true
    }
}

/// Format slice width for display (e.g. "10s", "5min", "1h").
fn format_slice_width(ms: u64) -> String {
    if ms >= 3_600_000 {
        format!("{}h", ms / 3_600_000)
    } else if ms >= 60_000 {
        format!("{}m", ms / 60_000)
    } else {
        format!("{}s", ms / 1_000)
    }
}

/// Apply a centered rolling-window aggregation over a 1D series.
/// Returns `None` if smoothing is disabled or belongs to the causal mode.
fn rolling_apply(values: &[f64], cfg: RollingConfig) -> Option<Vec<f64>> {
    if matches!(cfg.kind, RollingKind::Off | RollingKind::TrailingSum) || cfg.window <= 1 || values.is_empty() {
        return None;
    }
    let w = cfg.window;
    let half = w / 2;
    let mut out = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        let lo = i.saturating_sub(half);
        let hi = (i + w - half).min(values.len()); // exclusive
        let window = &values[lo..hi];
        let v = match cfg.kind {
            RollingKind::Mean => window.iter().sum::<f64>() / window.len() as f64,
            RollingKind::Sum => window.iter().sum::<f64>(),
            RollingKind::TrailingSum => unreachable!("causal mode is handled separately"),
            RollingKind::Min => window.iter().copied().fold(f64::INFINITY, f64::min),
            RollingKind::Max => window.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            RollingKind::Median => {
                let mut sorted: Vec<f64> = window.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                let n = sorted.len();
                if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 }
            }
            RollingKind::Off => unreachable!("guarded above"),
        };
        out.push(v);
    }
    Some(out)
}

/// Causal trailing sum over the preceding `window` source slices, emitted at
/// an independent `resolution` in slices. Leading points are suppressed until
/// a complete window exists.
fn trailing_sum_apply(values: &[f64], slice_end_minutes: &[f64], cfg: RollingConfig) -> Option<Vec<[f64; 2]>> {
    if cfg.kind != RollingKind::TrailingSum || values.len() != slice_end_minutes.len() {
        return None;
    }
    trailing_slice_sum(values, cfg.window, cfg.resolution)
        .map(|points| points.into_iter().map(|point| [slice_end_minutes[point.end_index], point.sum]).collect())
}

/// Draw the stacked chart panel (one mini-chart per selected metric).
///
/// Returns the new cursor position (in sim ms) if the user clicked on any plot.
pub fn draw_chart(
    ui: &mut egui::Ui,
    time_series: &[SliceRecord],
    selected_metrics: &[MetricKind],
    time_cursor_ms: f64,
    slice_width_ms: u64,
    num_stations: usize,
    total_rack_slots: u32,
    total_relay_cells: u32,
    cache: &mut Option<ChartCache>,
    rolling_config: &mut HashMap<MetricKind, RollingConfig>,
) -> Option<f64> {
    let mut new_cursor = None;

    if time_series.is_empty() {
        ui.label("No time series data yet.");
        return None;
    }

    let slice_count = time_series.len();

    // Rebuild cache if stale (new slices arrived, metrics selection changed,
    // or any selected metric's rolling config changed).
    let needs_rebuild = cache.as_ref().map_or(true, |c| {
        !c.is_valid(slice_count, num_stations, total_rack_slots, total_relay_cells, selected_metrics, rolling_config)
    });
    if needs_rebuild {
        // Slices arrive in chronological order from the sim; sort only as a
        // defensive measure so the cache remains correct if order ever drifts.
        let mut sorted: Vec<&SliceRecord> = time_series.iter().collect();
        sorted.sort_by_key(|s| s.start_ms);

        // Pre-compute cumulative series once.
        //
        // `cumulative_sim_speed_avg[i]` = `sim_time_at_slice_end_i` /
        // `cumulative_wall_clock_at_slice_end_i`. Reading the running ratio
        // (rather than per-slice `slice_width / wall_ms`) gives a stable
        // overall-speedup curve that converges as the run progresses and
        // isn't shifted by GC pauses or scheduler hiccups on any one slice.
        let mut cumulative_picks: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_putaways: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_orders: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_pres: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_pick_pres: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_putaway_pres: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_putaway_pres_direct: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_putaway_pres_from_rack: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_pallets: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut cumulative_sim_speed_avg: Vec<f64> = Vec::with_capacity(sorted.len());
        let mut running_picks = 0.0;
        let mut running_putaways = 0.0;
        let mut running_orders = 0.0;
        let mut running_pres = 0.0;
        let mut running_pick_pres = 0.0;
        let mut running_putaway_pres = 0.0;
        let mut running_putaway_pres_direct = 0.0;
        let mut running_putaway_pres_from_rack = 0.0;
        let mut running_pallets = 0.0;
        let mut running_wall_ms = 0.0;
        for slice in &sorted {
            running_picks += slice.get("picks_completed");
            running_putaways += slice.get("putaways_completed");
            running_orders += slice.get("orders_completed");
            running_pres += slice.get("presentations");
            running_pick_pres += slice.get("pick_presentations");
            running_putaway_pres += slice.get("putaway_presentations");
            running_putaway_pres_direct += slice.get("putaway_station_presentations_direct");
            running_putaway_pres_from_rack += slice.get("putaway_station_presentations_from_rack");
            running_pallets += slice.get("pick_station_orders");
            running_wall_ms += slice.get("wall_clock_duration_ms");
            cumulative_picks.push(running_picks);
            cumulative_putaways.push(running_putaways);
            cumulative_orders.push(running_orders);
            cumulative_pres.push(running_pres);
            cumulative_pick_pres.push(running_pick_pres);
            cumulative_putaway_pres.push(running_putaway_pres);
            cumulative_putaway_pres_direct.push(running_putaway_pres_direct);
            cumulative_putaway_pres_from_rack.push(running_putaway_pres_from_rack);
            cumulative_pallets.push(running_pallets);
            let sim_time_end_ms = slice.start_ms as f64 + slice_width_ms as f64;
            let ratio = if running_wall_ms > 0.0 { sim_time_end_ms / running_wall_ms } else { 0.0 };
            cumulative_sim_speed_avg.push(ratio);
        }

        let mut series: HashMap<MetricKind, MetricSeries> = HashMap::with_capacity(selected_metrics.len());
        let mut unavailable = FxHashMap::default();
        let mut cached_rolling: HashMap<MetricKind, RollingConfig> = HashMap::with_capacity(selected_metrics.len());
        for &kind in selected_metrics {
            // App init pre-populates `rolling_config` for every MetricKind,
            // so absence here would be a wiring bug. Cache the config even
            // for unavailable metrics below, or `is_valid` would see a
            // missing entry and rebuild the cache every frame.
            let cfg = *rolling_config.get(&kind).unwrap_or_else(|| panic!("rolling_config missing entry for {kind:?}"));
            cached_rolling.insert(kind, cfg);

            // y-values per slice (raw). `Err` = structurally unavailable
            // for this scenario (same reason on every slice) — leave the
            // series out of the cache; the render loop surfaces the reason
            // instead of plotting a misleading line.
            let ys: Result<Vec<f64>, &'static str> = sorted
                .iter()
                .enumerate()
                .map(|(i, slice)| {
                    if kind.is_cumulative() {
                        match kind {
                            MetricKind::CumulativePicks => Ok(cumulative_picks[i]),
                            MetricKind::CumulativePutaways => Ok(cumulative_putaways[i]),
                            MetricKind::CumulativeOrders => Ok(cumulative_orders[i]),
                            MetricKind::CumulativePresentations => Ok(cumulative_pres[i]),
                            MetricKind::CumulativePickPresentations => Ok(cumulative_pick_pres[i]),
                            MetricKind::CumulativePutawayPresentations => Ok(cumulative_putaway_pres[i]),
                            MetricKind::CumulativePutawayPresentationsDirect => Ok(cumulative_putaway_pres_direct[i]),
                            MetricKind::CumulativePutawayPresentationsFromRack => {
                                Ok(cumulative_putaway_pres_from_rack[i])
                            }
                            MetricKind::CumulativePallets => Ok(cumulative_pallets[i]),
                            MetricKind::SimSpeedAvg => Ok(cumulative_sim_speed_avg[i]),
                            _ => kind.extract(
                                &slice.metrics,
                                slice_width_ms,
                                num_stations,
                                total_rack_slots,
                                total_relay_cells,
                            ),
                        }
                    } else {
                        kind.extract(&slice.metrics, slice_width_ms, num_stations, total_rack_slots, total_relay_cells)
                    }
                })
                .collect();
            let ys = match ys {
                Ok(values) => values,
                Err(reason) => {
                    unavailable.insert(kind, reason);
                    continue;
                }
            };

            let xs: Vec<f64> =
                sorted.iter().map(|slice| (slice.start_ms as f64 + slice_width_ms as f64 / 2.0) / 60_000.0).collect();
            let slice_end_minutes: Vec<f64> =
                sorted.iter().map(|slice| (slice.start_ms as f64 + slice_width_ms as f64) / 60_000.0).collect();

            let raw: Vec<[f64; 2]> = xs.iter().zip(ys.iter()).map(|(&x, &y)| [x, y]).collect();

            // Apply smoothing if configured for this metric.  Cumulative
            // metrics never smooth (their `allowed_rollings()` is `[Off]`).
            let smoothed = if kind.is_cumulative() {
                None
            } else if cfg.kind == RollingKind::TrailingSum {
                trailing_sum_apply(&ys, &slice_end_minutes, cfg)
            } else {
                rolling_apply(&ys, cfg).map(|sm| xs.iter().zip(sm.iter()).map(|(&x, &y)| [x, y]).collect())
            };

            series.insert(kind, MetricSeries { raw, smoothed });
        }

        *cache = Some(ChartCache {
            slice_count,
            num_stations,
            total_rack_slots,
            total_relay_cells,
            cached_metrics: selected_metrics.to_vec(),
            cached_rolling,
            series,
            unavailable,
        });
    }

    let cache_ref = cache.as_ref().expect("cache was just populated");

    let num_charts = selected_metrics.len().max(1);
    let avail = ui.available_height();
    // Header row eats ~22px; reserve so the plot itself stays at >= 100px.
    const HEADER_H: f32 = 22.0;
    let chart_height = (avail / num_charts as f32 - HEADER_H).max(100.0);

    // auto_shrink(false) so the ScrollArea always claims the panel's full
    // height. With the default auto_shrink, the scroll area would shrink
    // to fit short content (e.g. 1 selected metric), and switching back
    // to a tab with taller content caused the panel resize state to drift.
    theme::apply_scrollbar_style_with_right_offset(ui, 8.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (chart_idx, &kind) in selected_metrics.iter().enumerate() {
            let series = match cache_ref.series.get(&kind) {
                Some(s) => s,
                None => {
                    // The cache builder leaves structurally-unavailable
                    // metrics out of the series map — show the reason
                    // instead of an empty (or misleading flat-zero) plot.
                    if let Some(reason) = cache_ref.unavailable.get(&kind) {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(kind.label()).weak());
                            ui.label(theme::caption(&format!("Unavailable: {reason}")));
                        });
                    }
                    continue;
                }
            };

            let color = kind.color();
            let is_last = chart_idx == num_charts - 1;

            // Header row: metric label left, rolling controls right-aligned.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(kind.label()).strong().color(color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cfg = rolling_config.entry(kind).or_insert_with(|| kind.default_rolling());
                    let allowed = kind.allowed_rollings();
                    let multi_choice = allowed.len() > 1;

                    theme::combobox_with_width(ui, &format!("rolling_kind_{:?}", kind), cfg.kind.label(), 80.0, |ui| {
                        for &rk in allowed {
                            ui.selectable_value(&mut cfg.kind, rk, rk.label());
                        }
                    });
                    ui.label(egui::RichText::new("rolling").small().weak());

                    let window_enabled = multi_choice && cfg.kind != RollingKind::Off;
                    ui.add_enabled(window_enabled, egui::DragValue::new(&mut cfg.window).range(1..=200).speed(0.25));
                    let window_ms = slice_width_ms.saturating_mul(cfg.window as u64);
                    ui.label(egui::RichText::new(format!("window ({})", format_slice_width(window_ms))).small().weak());

                    if cfg.kind == RollingKind::TrailingSum {
                        ui.add(egui::DragValue::new(&mut cfg.resolution).range(1..=200).speed(0.25));
                        let resolution_ms = slice_width_ms.saturating_mul(cfg.resolution as u64);
                        ui.label(
                            egui::RichText::new(format!("resolution ({})", format_slice_width(resolution_ms)))
                                .small()
                                .weak(),
                        );
                    }
                });
            });

            // Per-chart bottom-left readout: snap cursor x to the nearest
            // displayed point and show that y (smoothed value when rolling
            // is on, raw value otherwise).  Borrows directly from the
            // cache; the formatter only runs while `plot.show()` is on the
            // stack, so the borrow is local.
            let display_points: &[[f64; 2]] = series.smoothed.as_deref().unwrap_or(series.raw.as_slice());
            let cfg_label: &'static str = if series.smoothed.is_some() {
                rolling_config
                    .get(&kind)
                    .map(|c| c.kind.label())
                    .unwrap_or_else(|| panic!("rolling_config missing entry for {kind:?}"))
            } else {
                "raw"
            };
            let formatter = CoordinatesFormatter::new(move |point, _bounds| {
                if display_points.is_empty() {
                    return String::new();
                }
                let cursor_x = point.x;
                // Series is sorted ascending by x; pick the nearest sample.
                let upper = display_points.partition_point(|p| p[0] < cursor_x);
                let i = if upper == 0 {
                    0
                } else if upper >= display_points.len() {
                    display_points.len() - 1
                } else {
                    let prev = display_points[upper - 1][0];
                    let next = display_points[upper][0];
                    if (cursor_x - prev).abs() < (next - cursor_x).abs() { upper - 1 } else { upper }
                };
                let [x, y] = display_points[i];
                format!("t = {x:.2} min\ny ({cfg_label}) = {y:.3}")
            });

            // Use the MetricKind debug name as plot ID so it stays stable when
            // metrics are toggled on/off (chart_idx would shift and confuse
            // egui_plot's cached per-plot state).
            let mut plot = Plot::new(format!("metric_{:?}", kind))
                .height(chart_height)
                .y_axis_min_width(56.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .auto_bounds([true, true].into())
                .include_x(0.0)
                .include_y(0.0)
                .link_axis("metrics_x", [true, false])
                .link_cursor("metrics_cursor", [true, false].into())
                .show_axes([is_last, true])
                .show_grid([is_last, true])
                // Cull/fade the finest grid lines at 24px spacing instead of
                // the 8px default, so they thin out a little sooner as the
                // (live-growing) time range widens.
                .grid_spacing(egui::Rangef::new(24.0, 300.0))
                .x_grid_spacer(uniform_grid_spacer(time_step_sizes))
                .x_axis_formatter(|mark, _range| format_time_minutes(mark.value))
                .y_axis_formatter(|mark, _range| compact_number(mark.value))
                // Replace the cursor-following hover label (which clips off
                // the top edge of short stacked charts) with a fixed
                // bottom-left readout that snaps to the displayed series.
                .show_x(false)
                .show_y(false)
                .coordinates_formatter(Corner::LeftBottom, formatter);

            if is_last {
                plot = plot.x_axis_label(format!("Sim time — {} slices", format_slice_width(slice_width_ms)));
            }

            plot.show(ui, |plot_ui: &mut PlotUi| {
                // Raw line: faded when a smoothed line is drawn on top, full
                // strength otherwise.
                let has_smoothed = series.smoothed.is_some();
                let raw_color = if has_smoothed {
                    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 70)
                } else {
                    color
                };
                let raw_width = if has_smoothed { 1.0 } else { 2.0 };
                let raw_line = Line::new(PlotPoints::new(series.raw.clone()))
                    .name(if has_smoothed { format!("{} (raw)", kind.label()) } else { kind.label().to_string() })
                    .color(raw_color)
                    .width(raw_width);
                plot_ui.line(raw_line);

                if let Some(ref sm) = series.smoothed {
                    let smoothed_line =
                        Line::new(PlotPoints::new(sm.clone())).name(kind.label()).color(color).width(2.5);
                    plot_ui.line(smoothed_line);
                }

                // Time cursor.
                let cursor_min = time_cursor_ms / 60_000.0;
                plot_ui.vline(VLine::new(cursor_min).color(theme::CURSOR).width(1.5));

                // Highlight the displayed sample nearest the mouse hover x.
                // Hidden when the mouse isn't over this chart.
                if let Some(pos) = plot_ui.pointer_coordinate() {
                    if !display_points.is_empty() {
                        let hover_x = pos.x;
                        let upper = display_points.partition_point(|p| p[0] < hover_x);
                        let i = if upper == 0 {
                            0
                        } else if upper >= display_points.len() {
                            display_points.len() - 1
                        } else {
                            let prev = display_points[upper - 1][0];
                            let next = display_points[upper][0];
                            if (hover_x - prev).abs() < (next - hover_x).abs() { upper - 1 } else { upper }
                        };
                        plot_ui.points(
                            Points::new(vec![display_points[i]])
                                .shape(MarkerShape::Circle)
                                .radius(3.5)
                                .color(color)
                                .filled(true),
                        );
                    }
                }

                // Update cursor on click or drag only (not mere hover) to
                // avoid triggering heatmap rebuilds on every mouse pixel.
                // Allowed during a running sim too — playback can scrub
                // through already-streamed slices while the sim continues.
                if let Some(pos) = plot_ui.pointer_coordinate() {
                    if plot_ui.response().clicked() || plot_ui.response().dragged() {
                        new_cursor = Some(pos.x * 60_000.0);
                    }
                }
            });
        }
    });

    new_cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_sum_suppresses_partial_windows_and_uses_independent_resolution() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let slice_ends = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let config = RollingConfig { kind: RollingKind::TrailingSum, window: 3, resolution: 2 };

        let points = trailing_sum_apply(&values, &slice_ends, config).unwrap();
        assert_eq!(points, vec![[3.0, 6.0], [5.0, 12.0]]);
    }

    #[test]
    fn trailing_sum_requires_a_complete_window() {
        let config = RollingConfig { kind: RollingKind::TrailingSum, window: 3, resolution: 1 };

        assert!(trailing_sum_apply(&[1.0, 2.0], &[1.0, 2.0], config).is_none());
    }
}
