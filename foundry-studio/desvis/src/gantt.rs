//! Per-bot Gantt strip rendered into the bottom panel.
//!
//! One horizontal lane per robot, colored intervals from `BusyInterval`.
//! Gaps between intervals = Idle (lane background). VTMs and HTMs render in
//! two stacked sections so the user can scan one fleet at a time.
//!
//! Interactive zoom: mouse wheel (or pinch / ctrl+scroll) over the Gantt
//! shrinks or grows the visible time window. At the full window the run fits
//! the panel width and auto-scales as a live sim grows; once zoomed in the
//! window is pinned (stays put as the run grows) and horizontal scroll reveals
//! the rest. Click or drag a lane to scrub the time cursor.

use std::collections::HashMap;

use eframe::egui;
use eframe::egui::Color32;

use desgraph::{StationId, StationType};
use dessim::events::VtmIdleReasonKind;
use dessim::metrics::profile::StationGapStats;
use dessim::model::relay::{RelayUtilizationSummary, RelayUtilizationTimeline};
use dessim::model::{RobotId, SimDuration};
use dessim::observer::activity::ActivityKind;
use dessim::observer::duty::{BusyInterval, DutyBreakdown, StationBusyInterval};

use crate::theme;

/// Smallest visible time window the zoom can reach, in ms — the most
/// zoomed-in view spans exactly this regardless of run length. 30s, fine
/// enough to read individual cycles. The largest window is the full run
/// (fit-to-window); past that there's nothing more to show.
const MIN_VISIBLE_WINDOW_MS: f64 = 30_000.0;

/// Render the Utilization panel into `ui`. Top: zoom toolbar. Below:
/// collapsible per-section Gantt strips (Stations / DA50 / D30), each
/// carrying its own legend, with horizontal zoom/scroll. At the full window
/// the lanes fit the panel; zooming in widens them past it. The three
/// sections share `label_width` (so their time axes line up) and a single
/// `gantt_scroll_x` (so they pan in lockstep).
///
/// `gantt_window_ms` is read/written so the toolbar buttons and wheel/pinch
/// inputs adjust the visible window in-place (`None` = fit-to-window).
/// `gantt_scroll_x` is the shared horizontal offset: each section forces it
/// before drawing and writes back whichever value the user just scrolled to.
///
/// Returns `Some(sim_ms)` when the user clicks or drags on a lane — the sim
/// time under the pointer — so the caller can scrub the time cursor there,
/// mirroring the time-series chart. `None` when no lane was clicked.
pub fn draw_gantt_panel(
    ui: &mut egui::Ui,
    vtm_timelines: &HashMap<RobotId, Vec<BusyInterval>>,
    htm_timelines: &HashMap<RobotId, Vec<BusyInterval>>,
    vtm_duty_totals: &HashMap<RobotId, DutyBreakdown>,
    htm_duty_totals: &HashMap<RobotId, DutyBreakdown>,
    vtm_idle_reason_counts: &HashMap<VtmIdleReasonKind, u64>,
    relay_utilization: Option<&RelayUtilizationSummary>,
    relay_busy_lanes: Option<&RelayUtilizationTimeline>,
    relay_slot_occupancy_timeline: Option<&RelayUtilizationTimeline>,
    station_timelines: &HashMap<StationId, Vec<StationBusyInterval>>,
    station_types: &HashMap<StationId, StationType>,
    station_busy_pct: &HashMap<StationId, f64>,
    station_gap_stats: Option<&[StationGapStats]>,
    sim_elapsed: SimDuration,
    time_cursor_ms: f64,
    gantt_window_ms: &mut Option<f64>,
    gantt_scroll_x: &mut f32,
) -> Option<f64> {
    if vtm_timelines.is_empty() && htm_timelines.is_empty() && station_timelines.is_empty() {
        ui.label("No timeline data yet (set record_robot_timelines and run a sim).");
        return None;
    }

    let total_ms = sim_elapsed.as_ms().max(1) as f64;
    // Visible window the user is looking at. `None` = fit-to-window: the whole
    // run, which auto-scales as a live sim grows (the window tracks `total_ms`).
    // `Some(w)` = pinned at `w` ms — stays put as the run grows so zooming in to
    // inspect detail doesn't drift. `effective_window` clamps both to
    // [30s, full run].
    let visible_window_ms = effective_window(*gantt_window_ms, total_ms);
    // Lane geometry still works in multiplier terms: at the full window zoom is
    // 1.0 (fits the panel); a narrower window scales the lanes wider so the same
    // pixels show less time.
    let zoom = (total_ms / visible_window_ms) as f32;

    // One label width for every section so the lane columns start at the
    // same x and span the same width — a prerequisite for the time axes to
    // line up across fleets and for the shared scroll offset (below) to be
    // valid for all three. Widen when gap-stats are present (the station
    // labels carry an extra ` p95=XXs`); robot labels just gain trailing
    // space, which is cheaper than misaligned axes.
    let label_width = if station_gap_stats.is_some() { 150.0 } else { 90.0 };

    // Toolbar: the visible time window plus zoom controls. `+` zooms in (smaller
    // pinned window), `−` zooms out; growing the window to the full run snaps
    // back to fit-to-window (`None`) so it auto-scales again. `Reset` jumps
    // straight to the full window.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format_window(visible_window_ms)).small().monospace().color(theme::TEXT_0));
        if ui.small_button("−").clicked() {
            *gantt_window_ms = pin_window(visible_window_ms * 1.5, total_ms);
        }
        if ui.small_button("+").clicked() {
            *gantt_window_ms = pin_window(visible_window_ms / 1.5, total_ms);
        }
        if ui.small_button("Reset").clicked() {
            *gantt_window_ms = None;
        }
        ui.label(egui::RichText::new("(scroll to zoom)").small().color(theme::TEXT_2));
    });

    // Set by any lane the user clicks/drags on — the sim time under the
    // pointer, so the time cursor (and the grid view) can scrub to it, the
    // same gesture the time-series chart supports.
    let mut new_cursor: Option<f64> = None;

    // auto_shrink(false) so collapsing every Gantt section doesn't let the
    // ScrollArea (and therefore the bottom panel) shrink to fit short content.
    // The panel's height should be governed entirely by the user-dragged size.
    theme::apply_scrollbar_style_with_right_offset(ui, 8.0);
    egui::ScrollArea::vertical().id_salt("gantt_outer").auto_shrink([false, false]).show(ui, |ui| {
        // Per-station Gantt with busy% in the lane label. Default open —
        // station saturation is the headline diagnostic.
        if !station_timelines.is_empty() {
            let title = format!("Stations ({})", station_timelines.len());
            theme::collapsing_section(ui, "gantt_stations", &title, true, |ui| {
                draw_station_section(
                    ui,
                    "gantt_stn_scroll",
                    station_timelines,
                    station_types,
                    station_busy_pct,
                    station_gap_stats,
                    total_ms,
                    time_cursor_ms,
                    zoom,
                    label_width,
                    gantt_scroll_x,
                    &mut new_cursor,
                );
            });
        }
        if !vtm_timelines.is_empty() {
            let title = format!("DA50 ({})", vtm_timelines.len());
            theme::collapsing_section(ui, "gantt_vtm", &title, true, |ui| {
                // Cycle-start reasons nested under DA50 — it's VTM-specific
                // categorical data and belongs with the fleet it describes
                // rather than as a sibling section.
                draw_vtm_idle_reasons(ui, vtm_idle_reason_counts);
                draw_robot_legend(ui, &[ActivityKind::Travel, ActivityKind::ForkOp]);
                draw_section(
                    ui,
                    "gantt_vtm_scroll",
                    "A",
                    vtm_timelines,
                    vtm_duty_totals,
                    sim_elapsed,
                    total_ms,
                    time_cursor_ms,
                    zoom,
                    label_width,
                    gantt_scroll_x,
                    &mut new_cursor,
                );
            });
        }
        // Relay utilization is its own top-level section beneath DA50 — it's
        // shared relay-access infrastructure, not a per-VTM breakdown, so it
        // reads as a sibling of the fleet sections rather than nested in one.
        // The diving-board slot occupancy (a distinct resource) sits right
        // below it: access-cell "how busy is the approach" then slot "how full
        // is the buffer".
        draw_relay_utilization(
            ui,
            relay_utilization,
            relay_busy_lanes,
            total_ms,
            time_cursor_ms,
            zoom,
            label_width,
            gantt_scroll_x,
            &mut new_cursor,
        );
        draw_diving_board_occupancy(
            ui,
            relay_slot_occupancy_timeline,
            total_ms,
            time_cursor_ms,
            zoom,
            label_width,
            gantt_scroll_x,
            &mut new_cursor,
        );
        if !htm_timelines.is_empty() {
            let title = format!("D30 ({})", htm_timelines.len());
            theme::collapsing_section(ui, "gantt_htm", &title, true, |ui| {
                draw_robot_legend(ui, &[ActivityKind::Travel, ActivityKind::ForkOp, ActivityKind::Presentation]);
                draw_section(
                    ui,
                    "gantt_htm_scroll",
                    "D",
                    htm_timelines,
                    htm_duty_totals,
                    sim_elapsed,
                    total_ms,
                    time_cursor_ms,
                    zoom,
                    label_width,
                    gantt_scroll_x,
                    &mut new_cursor,
                );
            });
        }
    });

    // Interactive zoom: wheel / pinch / ctrl+scroll over the panel. egui's
    // `zoom_delta` is 1.0 = no change; >1 = zoom in. We apply
    // multiplicatively and clamp. Also honors raw vertical scroll if the
    // pointer is hovering and the user holds Ctrl — covers trackpads that
    // emit scroll_delta rather than zoom_delta.
    let pointer_over_panel = ui.ui_contains_pointer();
    if pointer_over_panel {
        let (zoom_delta, scroll_y, ctrl) =
            ui.ctx().input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y, i.modifiers.ctrl || i.modifiers.command));
        let mut delta = zoom_delta;
        if ctrl && scroll_y != 0.0 {
            // Map vertical scroll to zoom when Ctrl/Cmd is held. 100 units
            // ≈ one notch ≈ 1.1x change.
            delta *= 1.0 + scroll_y / 1000.0;
        }
        if (delta - 1.0).abs() > 1e-4 {
            // delta > 1 = zoom in = narrower window. Recompute the current
            // window from the (possibly button-mutated) state rather than the
            // local captured at the top of the frame.
            let current = effective_window(*gantt_window_ms, total_ms);
            *gantt_window_ms = pin_window(current / delta as f64, total_ms);
        }
    }

    new_cursor
}

/// Resolve the stored window state into the window to actually render, in ms.
/// `None` = fit-to-window → the full run, so it auto-scales as `total_ms`
/// grows. `Some(w)` is clamped to [30s, full run]; a pinned window wider than
/// the current run just shows the whole run.
fn effective_window(window: Option<f64>, total_ms: f64) -> f64 {
    match window {
        None => total_ms,
        // Floor at `min(30s, full run)`, not a bare 30s: a run shorter than the
        // 30s minimum would make `min > max` and `f64::clamp` would panic. For
        // such runs the smallest window is just the whole run.
        Some(w) => w.clamp(MIN_VISIBLE_WINDOW_MS.min(total_ms), total_ms),
    }
}

/// Turn a candidate window (ms) into stored state: once it reaches the full
/// run, snap to fit-to-window (`None`) so it auto-scales again; otherwise pin
/// it, floored at the 30s minimum (or the full run, if it's shorter than 30s —
/// you can't zoom past the whole run). Used by the zoom buttons and scroll/pinch.
fn pin_window(candidate_ms: f64, total_ms: f64) -> Option<f64> {
    let w = candidate_ms.max(MIN_VISIBLE_WINDOW_MS.min(total_ms));
    if w >= total_ms { None } else { Some(w) }
}

/// Map a pointer's screen x within a lane to the sim time under it.
/// `rect_min_x` is the lane's (scrolled) left edge in screen space and
/// `lane_width` spans the whole run, so the mapping holds at any zoom/scroll.
/// Clamped to `[0, total_ms]` for drags that wander past the lane edges.
fn lane_x_to_time(pointer_x: f32, rect_min_x: f32, lane_width: f32, total_ms: f64) -> f64 {
    let frac = ((pointer_x - rect_min_x) / lane_width).clamp(0.0, 1.0) as f64;
    frac * total_ms
}

/// Render one section (VTM or HTM) — sorted lanes plus a vertical cursor
/// line spanning the whole section at `time_cursor_ms`. The label column
/// shows `prefix + robot_id + busy%` and stays fixed; only the lane area
/// scrolls horizontally when zoomed (`zoom` multiplies the lane width).
fn draw_section(
    ui: &mut egui::Ui,
    scroll_salt: &str,
    prefix: &str,
    timelines: &HashMap<RobotId, Vec<BusyInterval>>,
    duty_totals: &HashMap<RobotId, DutyBreakdown>,
    sim_elapsed: SimDuration,
    total_ms: f64,
    time_cursor_ms: f64,
    zoom: f32,
    label_width: f32,
    scroll_x: &mut f32,
    new_cursor: &mut Option<f64>,
) {
    let mut robot_ids: Vec<RobotId> = timelines.keys().copied().collect();
    robot_ids.sort();

    // `label_width` is passed in (shared across all sections) so the lane
    // columns align. The label carries `prefix+id` (4 chars) + space +
    // `100.0%` (6 chars); any extra width is trailing space.
    let row_height = 14.0;
    let row_spacing = 1.0;
    // Subtract egui's inter-column spacing here too — the outer
    // `ui.horizontal` below adds `item_spacing.x` between the label column
    // and the lane column. If we don't account for it the row exceeds
    // `available_width`, which then bubbles up and pushes the bottom
    // card's frame_rect past the panel's right outer margin.
    let base_lane_width = (ui.available_width() - label_width - ui.spacing().item_spacing.x).max(50.0);
    let lane_width = base_lane_width * zoom;

    ui.horizontal(|ui| {
        // Label column: paint manually via `allocate_exact_size + painter.galley`
        // so each row is exactly `row_height` tall — `add_sized + Label` was
        // letting font metrics push the row taller than the lane column,
        // which drifted the labels out of alignment with the bars.
        let label_font = egui::TextStyle::Monospace.resolve(ui.style());
        ui.vertical(|ui| {
            for &robot_id in &robot_ids {
                let busy_pct =
                    duty_totals.get(&robot_id).map(|d| d.duty_pct_global(sim_elapsed) * 100.0).unwrap_or(0.0);
                let text = format!("{}{:<3} {:>5.1}%", prefix, robot_id.0, busy_pct);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(label_width, row_height), egui::Sense::hover());
                let galley = ui.painter().layout_no_wrap(text, label_font.clone(), theme::TEXT_0);
                let text_pos = egui::pos2(rect.min.x, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(text_pos, galley, theme::TEXT_0);
                ui.add_space(row_spacing);
            }
        });

        // Scrollable lane column: one painter row per bot. The outer
        // wrapper is `ui.horizontal(...)` which sets horizontal layout;
        // a horizontal ScrollArea inherits that, so we explicitly switch
        // back to vertical to stack the rows top-to-bottom. At `zoom == 1.0`
        // `lane_width == base_lane_width` so the content fits and no bar
        // shows; past that the bar reveals the overflow. `scroll_x` is
        // forced before drawing to keep all sections in lockstep.
        let out =
            egui::ScrollArea::horizontal().id_salt(scroll_salt).horizontal_scroll_offset(*scroll_x).show(ui, |ui| {
                ui.vertical(|ui| {
                    for (row_idx, robot_id) in robot_ids.iter().enumerate() {
                        let intervals = &timelines[robot_id];
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(lane_width, row_height), egui::Sense::click_and_drag());
                        let painter = ui.painter();
                        // Alternating row tint helps the eye track horizontally
                        // across long sims; tint only the gap (Idle) color.
                        let bg = if row_idx % 2 == 0 { theme::STATE_IDLE } else { lighten(theme::STATE_IDLE, 8) };
                        painter.rect_filled(rect, egui::Rounding::same(2.0), bg);
                        for interval in intervals {
                            let start_frac = (interval.start.as_ms() as f64 / total_ms).clamp(0.0, 1.0);
                            let end_frac = (interval.end.as_ms() as f64 / total_ms).clamp(0.0, 1.0);
                            if end_frac <= start_frac {
                                continue;
                            }
                            let x_start = rect.min.x + (start_frac as f32) * lane_width;
                            let x_end = rect.min.x + (end_frac as f32) * lane_width;
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(x_start, rect.min.y),
                                egui::pos2(x_end.max(x_start + 1.0), rect.max.y),
                            );
                            painter.rect_filled(bar, egui::Rounding::same(1.5), activity_color(interval.kind));
                        }
                        if time_cursor_ms >= 0.0 && time_cursor_ms <= total_ms {
                            let cursor_x = rect.min.x + ((time_cursor_ms / total_ms) as f32) * lane_width;
                            painter.line_segment(
                                [egui::pos2(cursor_x, rect.min.y), egui::pos2(cursor_x, rect.max.y)],
                                egui::Stroke::new(1.0, theme::CURSOR),
                            );
                        }
                        // Click or drag anywhere on the lane scrubs the time
                        // cursor to the sim time under the pointer (the gesture
                        // the time-series chart supports). `interact_pointer_pos`
                        // is screen-space and `rect.min.x` already folds in the
                        // scroll offset, so the mapping holds at any zoom/scroll.
                        if response.clicked() || response.dragged() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                *new_cursor = Some(lane_x_to_time(pos.x, rect.min.x, lane_width, total_ms));
                            }
                        }
                        ui.add_space(row_spacing);
                    }
                });
            });
        // If the user scrolled THIS section, its post-show offset differs
        // from the value we forced above — adopt it as the shared offset so
        // the other sections follow. Sections drawn after this one pick it
        // up the same frame; earlier ones lag a single frame (imperceptible).
        if (out.state.offset.x - *scroll_x).abs() > 0.5 {
            *scroll_x = out.state.offset.x;
        }
    });
}

/// Lighten a color by adding a fixed amount to each RGB channel, clamping
/// to 255. Used for alternating-row tinting on Gantt lanes.
fn lighten(c: Color32, by: u8) -> Color32 {
    let r = c.r().saturating_add(by);
    let g = c.g().saturating_add(by);
    let b = c.b().saturating_add(by);
    Color32::from_rgb(r, g, b)
}

/// Render the per-station Gantt section. One lane per station, sorted by
/// `StationId`. Busy intervals are solid bars colored by station type;
/// gaps between intervals are the lane background (operator-cell idle).
///
/// `gap_stats` (when present) is folded into the lane label as ` p95={s}s`
/// — the inter-presentation gap p95 is a direct readout of flow smoothness
/// at that station. Live-snapshot runs pass `None` since gap stats are only
/// computed once the run finishes.
fn draw_station_section(
    ui: &mut egui::Ui,
    scroll_salt: &str,
    timelines: &HashMap<StationId, Vec<StationBusyInterval>>,
    station_types: &HashMap<StationId, StationType>,
    station_busy_pct: &HashMap<StationId, f64>,
    gap_stats: Option<&[StationGapStats]>,
    total_ms: f64,
    time_cursor_ms: f64,
    zoom: f32,
    label_width: f32,
    scroll_x: &mut f32,
    new_cursor: &mut Option<f64>,
) {
    let mut station_ids: Vec<StationId> = timelines.keys().copied().collect();
    station_ids.sort();

    let gap_p95_by_station: HashMap<StationId, u64> =
        gap_stats.map(|stats| stats.iter().map(|g| (g.station_id, g.p95_ms)).collect()).unwrap_or_default();

    // `label_width` is passed in (shared across all sections) so the lane
    // columns align. The station label is `stnNN` + ` 100.0%` + optional
    // ` p95=XXs`; the caller already widened the column when gap stats exist.
    let row_height = 14.0;
    let row_spacing = 1.0;
    // Subtract egui's inter-column spacing here too — the outer
    // `ui.horizontal` below adds `item_spacing.x` between the label column
    // and the lane column. If we don't account for it the row exceeds
    // `available_width`, which then bubbles up and pushes the bottom
    // card's frame_rect past the panel's right outer margin.
    let base_lane_width = (ui.available_width() - label_width - ui.spacing().item_spacing.x).max(50.0);
    let lane_width = base_lane_width * zoom;

    // Legend strip showing the two station-type colors, so a viewer can
    // tell pick lanes from putaway at a glance.
    ui.horizontal(|ui| {
        legend_swatch(ui, station_color(Some(StationType::Pick)), "Pick");
        legend_swatch(ui, station_color(Some(StationType::Putaway)), "Putaway");
        legend_swatch(ui, theme::STATE_IDLE, "Idle");
    });

    ui.horizontal(|ui| {
        // Same trick as `draw_section`: manual paint per row so the label
        // column's row height matches the lane column exactly.
        let label_font = egui::TextStyle::Monospace.resolve(ui.style());
        ui.vertical(|ui| {
            for &station_id in &station_ids {
                let busy_pct = station_busy_pct.get(&station_id).copied().unwrap_or(0.0) * 100.0;
                let text = match gap_p95_by_station.get(&station_id) {
                    Some(&p95_ms) => format!("stn{:<2} {:>5.1}% p95={:>3}s", station_id.0, busy_pct, p95_ms / 1000,),
                    None => format!("stn{:<2} {:>5.1}%", station_id.0, busy_pct),
                };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(label_width, row_height), egui::Sense::hover());
                let galley = ui.painter().layout_no_wrap(text, label_font.clone(), theme::TEXT_0);
                let text_pos = egui::pos2(rect.min.x, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(text_pos, galley, theme::TEXT_0);
                ui.add_space(row_spacing);
            }
        });

        // Scrollable lane column: see `draw_section` for why we re-enter a
        // vertical layout inside the horizontal ScrollArea. At `zoom == 1.0`
        // the content fits and no bar shows. `scroll_x` is forced before
        // drawing so this section pans in lockstep with the others.
        let out =
            egui::ScrollArea::horizontal().id_salt(scroll_salt).horizontal_scroll_offset(*scroll_x).show(ui, |ui| {
                ui.vertical(|ui| {
                    for (row_idx, station_id) in station_ids.iter().enumerate() {
                        let intervals = &timelines[station_id];
                        let color = station_color(station_types.get(station_id).copied());
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(lane_width, row_height), egui::Sense::click_and_drag());
                        let painter = ui.painter();
                        let bg = if row_idx % 2 == 0 { theme::STATE_IDLE } else { lighten(theme::STATE_IDLE, 8) };
                        painter.rect_filled(rect, egui::Rounding::same(2.0), bg);
                        for interval in intervals {
                            let start_frac = (interval.start.as_ms() as f64 / total_ms).clamp(0.0, 1.0);
                            let end_frac = (interval.end.as_ms() as f64 / total_ms).clamp(0.0, 1.0);
                            if end_frac <= start_frac {
                                continue;
                            }
                            let x_start = rect.min.x + (start_frac as f32) * lane_width;
                            let x_end = rect.min.x + (end_frac as f32) * lane_width;
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(x_start, rect.min.y),
                                egui::pos2(x_end.max(x_start + 1.0), rect.max.y),
                            );
                            painter.rect_filled(bar, egui::Rounding::same(1.5), color);
                        }
                        if time_cursor_ms >= 0.0 && time_cursor_ms <= total_ms {
                            let cursor_x = rect.min.x + ((time_cursor_ms / total_ms) as f32) * lane_width;
                            painter.line_segment(
                                [egui::pos2(cursor_x, rect.min.y), egui::pos2(cursor_x, rect.max.y)],
                                egui::Stroke::new(1.0, theme::CURSOR),
                            );
                        }
                        // Click/drag scrubs the time cursor — see `draw_section`.
                        if response.clicked() || response.dragged() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                *new_cursor = Some(lane_x_to_time(pos.x, rect.min.x, lane_width, total_ms));
                            }
                        }
                        ui.add_space(row_spacing);
                    }
                });
            });
        // Adopt this section's offset as the shared one if the user scrolled
        // it (see `draw_section` for the rationale).
        if (out.state.offset.x - *scroll_x).abs() > 0.5 {
            *scroll_x = out.state.offset.x;
        }
    });
}

/// Color per station type. Pick is blue (the primary throughput-binding
/// flow); putaway is amber (the supply flow that feeds picks). Unknown
/// type falls back to neutral gray — should not happen in practice since
/// `handle_cycle_complete` seeds the type from `MetricsConfig`.
fn station_color(station_type: Option<StationType>) -> Color32 {
    match station_type {
        Some(StationType::Pick) => theme::STATION_PICK,
        Some(StationType::Putaway) => theme::STATION_PUTAWAY,
        Some(StationType::BoxInduction) => theme::STATION_BOX,
        Some(StationType::Extraction) => theme::STATION_EXTRACTION,
        Some(StationType::ContainerPickup) => theme::STATION_BOX,
        None => theme::TEXT_2,
    }
}

/// One legend entry: a small colored swatch and a short label, with a
/// trailing space for the next entry.
fn legend_swatch(ui: &mut egui::Ui, color: Color32, label: &str) {
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::Rounding::same(2.0), color);
    ui.add_space(2.0);
    ui.label(egui::RichText::new(label).small());
    ui.add_space(8.0);
}

/// Color per `ActivityKind`. Used by the Gantt lanes, their legend swatches
/// and the bot inspector's strip, so all three agree.
pub(crate) fn activity_color(kind: ActivityKind) -> Color32 {
    match kind {
        ActivityKind::Gap => theme::STATE_IDLE,
        ActivityKind::Travel => theme::STATE_TRAVELING,
        ActivityKind::ForkOp => theme::STATE_LOADING,
        ActivityKind::Presentation => theme::STATE_PROCESSING,
    }
}

/// Legend strip for a robot fleet section. `kinds` lists the kinds of work
/// the fleet does (DA50s 2, D30s 3); `Gap` is appended automatically as the
/// trailing idle-color entry.
fn draw_robot_legend(ui: &mut egui::Ui, kinds: &[ActivityKind]) {
    ui.horizontal(|ui| {
        for &kind in kinds {
            legend_swatch(ui, activity_color(kind), &kind.to_string());
        }
        legend_swatch(ui, activity_color(ActivityKind::Gap), &ActivityKind::Gap.to_string());
    });
}

/// Collapsible breakdown of VTM cycle-start classifications.
///
/// Continuous = `vtm_start == available_at` (back-to-back). The awaiting
/// variants name the floor that pinned the next cycle later than the VTM was
/// free: demand (work not due yet), tote (not back in rack), slot (relay
/// still held), or the HTM's return deposit.
/// Relay-access utilization: an overall % headline, the per-zone min/avg/max
/// spread, and one heat lane per zone — each slice shaded white (0%) → red
/// (100%) by that zone's busy share, on the shared Gantt time axis. The lanes
/// are the slices' own figures, there while a run goes; the headline is the
/// run's oracle-based summary (same numbers as the headless
/// `relay_utilization_pct` line), there once it has ended. A recording whose
/// slices predate the zones' figures falls back to the summary's static
/// per-zone bar list. Low utilization with high demand is the signal that
/// relays aren't the capacity bottleneck.
#[allow(clippy::too_many_arguments)]
fn draw_relay_utilization(
    ui: &mut egui::Ui,
    summary: Option<&RelayUtilizationSummary>,
    timeline: Option<&RelayUtilizationTimeline>,
    total_ms: f64,
    time_cursor_ms: f64,
    zoom: f32,
    label_width: f32,
    scroll_x: &mut f32,
    new_cursor: &mut Option<f64>,
) {
    let summary = summary.filter(|s| !s.per_zone.is_empty());
    let timeline = timeline.filter(|t| !t.zones.is_empty());
    if summary.is_none() && timeline.is_none() {
        return;
    }
    let title = match summary {
        Some(s) => format!("Relay cell occupancy ({:.1}%)", s.overall_pct),
        None => "Relay cell occupancy".to_string(),
    };
    theme::collapsing_section(ui, "relay_utilization", &title, false, |ui| {
        ui.horizontal(|ui| {
            if let Some(s) = summary {
                let (min, avg, max) = s.zone_min_avg_max();
                ui.label(
                    egui::RichText::new(format!(
                        "overall {:.1}%  ·  per-zone min {:.1}% / avg {:.1}% / max {:.1}%  ·  {} relays",
                        s.overall_pct, min, avg, max, s.total_relays
                    ))
                    .small(),
                );
            }
            if timeline.is_some() {
                draw_heat_legend(ui, HeatRamp::Red);
            }
        });
        if let Some(t) = timeline {
            let zone_pct: Option<HashMap<u32, f64>> = summary.map(|s| s.per_zone.iter().copied().collect());
            draw_relay_heat_lanes(
                ui,
                zone_pct.as_ref(),
                t,
                HeatRamp::Red,
                "gantt_relay_scroll",
                "relay_heat",
                total_ms,
                time_cursor_ms,
                zoom,
                label_width,
                scroll_x,
                new_cursor,
            );
        } else if let Some(s) = summary {
            // Slices that predate the zones' figures — static bars, scaled to
            // the max so the busiest zone fills the row.
            let (_, _, max) = s.zone_min_avg_max();
            let scale = if max > 0.0 { max } else { 1.0 };
            for &(zone, pct) in &s.per_zone {
                let frac = (pct / scale).clamp(0.0, 1.0) as f32;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("z{zone:<3}")).small().monospace());
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 8.0), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 1.0, ui.visuals().faint_bg_color);
                    let mut fill = rect;
                    fill.set_width(rect.width() * frac);
                    painter.rect_filled(fill, 1.0, ui.visuals().selection.bg_fill);
                    ui.label(egui::RichText::new(format!("{pct:.1}%")).small().monospace());
                });
            }
        }
    });
}

/// Diving-board **slot** occupancy: heat lanes like [`draw_relay_utilization`]
/// but on the green ramp, shaded by how much of each slice a relay's buffer
/// slot was held/reserved — an upper bound on physical tote dwell, and a
/// resource distinct from the HTM-access cell (this is "how long is the buffer
/// committed", not "how busy is the approach"). Green vs the access lanes' red
/// so the two read apart. Its own section beneath the relay-access lanes; only
/// renders when the profile carries the slot-occupancy timeline (older
/// profiles omit it). Per-zone / overall percentages come from the timeline
/// itself (no oracle summary).
#[allow(clippy::too_many_arguments)]
fn draw_diving_board_occupancy(
    ui: &mut egui::Ui,
    timeline: Option<&RelayUtilizationTimeline>,
    total_ms: f64,
    time_cursor_ms: f64,
    zoom: f32,
    label_width: f32,
    scroll_x: &mut f32,
    new_cursor: &mut Option<f64>,
) {
    let Some(t) = timeline else {
        return;
    };
    if t.zones.is_empty() {
        return;
    }
    let per_zone = t.per_zone_pct();
    let overall = t.overall_pct();
    let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
    for &(_, pct) in &per_zone {
        min = min.min(pct);
        max = max.max(pct);
        sum += pct;
    }
    let avg = if per_zone.is_empty() { 0.0 } else { sum / per_zone.len() as f64 };
    let total_relays: u32 = t.zones.iter().map(|z| z.relays).sum();
    let title = format!("Diving-board occupancy ({:.1}%)", overall);
    theme::collapsing_section(ui, "diving_board_occupancy", &title, false, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "overall {:.1}%  ·  per-zone min {:.1}% / avg {:.1}% / max {:.1}%  ·  {} slots",
                    overall, min, avg, max, total_relays
                ))
                .small(),
            );
            draw_heat_legend(ui, HeatRamp::Green);
        });
        let zone_pct: HashMap<u32, f64> = per_zone.into_iter().collect();
        draw_relay_heat_lanes(
            ui,
            Some(&zone_pct),
            t,
            HeatRamp::Green,
            "gantt_slot_scroll",
            "slot_heat",
            total_ms,
            time_cursor_ms,
            zoom,
            label_width,
            scroll_x,
            new_cursor,
        );
    });
}

/// One heat lane per relay zone on the shared Gantt time axis: each slice is
/// shaded by `busy_ms / (relays × slice_width)` on the section's `ramp`. Lane
/// labels carry each zone's overall `zone_pct` when there is one (a run still
/// going has none: the share is over the run's whole horizon); the lanes share the
/// cross-section scroll offset, zoom, cursor line, and click-to-scrub of the
/// robot/station sections. `scroll_salt` / `tooltip_key` keep the two heat-lane
/// sections (relay access, diving-board slot) from colliding on egui ids.
#[allow(clippy::too_many_arguments)]
fn draw_relay_heat_lanes(
    ui: &mut egui::Ui,
    zone_pct: Option<&HashMap<u32, f64>>,
    timeline: &RelayUtilizationTimeline,
    ramp: HeatRamp,
    scroll_salt: &str,
    tooltip_key: &'static str,
    total_ms: f64,
    time_cursor_ms: f64,
    zoom: f32,
    label_width: f32,
    scroll_x: &mut f32,
    new_cursor: &mut Option<f64>,
) {
    let row_height = 14.0;
    let row_spacing = 1.0;
    // Same geometry derivation as `draw_section` so the lane column aligns
    // with the robot/station sections above and below.
    let base_lane_width = (ui.available_width() - label_width - ui.spacing().item_spacing.x).max(50.0);
    let lane_width = base_lane_width * zoom;

    ui.horizontal(|ui| {
        let label_font = egui::TextStyle::Monospace.resolve(ui.style());
        ui.vertical(|ui| {
            for zone in &timeline.zones {
                let text = match zone_pct {
                    Some(zone_pct) => {
                        format!("z{:<3} {:>5.1}%", zone.zone_id, zone_pct.get(&zone.zone_id).copied().unwrap_or(0.0))
                    }
                    None => format!("z{:<3}", zone.zone_id),
                };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(label_width, row_height), egui::Sense::hover());
                let galley = ui.painter().layout_no_wrap(text, label_font.clone(), theme::TEXT_0);
                let text_pos = egui::pos2(rect.min.x, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(text_pos, galley, theme::TEXT_0);
                ui.add_space(row_spacing);
            }
        });

        let out =
            egui::ScrollArea::horizontal().id_salt(scroll_salt).horizontal_scroll_offset(*scroll_x).show(ui, |ui| {
                ui.vertical(|ui| {
                    let slice_width_ms = timeline.slice_width_ms as f64;
                    for zone in &timeline.zones {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(lane_width, row_height), egui::Sense::click_and_drag());
                        let painter = ui.painter();
                        // Base = 0% (white); only non-zero runs paint over it.
                        painter.rect_filled(rect, egui::Rounding::same(2.0), ramp.color(0));
                        let denominator = (zone.relays as u64 * timeline.slice_width_ms).max(1) as f64;
                        // Quantize each slice onto the ramp and paint runs of
                        // equal level as one rect — at 10s slices a 10h run is
                        // 3600 slices/lane, and coalescing keeps the mesh small.
                        // TODO(perf): `busy_ms` is immutable, so the quantized
                        // run list could be memoized per (zone, denominator)
                        // instead of recomputed every frame — matters at long
                        // horizons (~1.4M `level_of` calls/frame at 100h).
                        let level_of = |busy_ms: u64| -> u8 {
                            ((busy_ms as f64 / denominator).clamp(0.0, 1.0) * HEAT_LEVELS as f64).round() as u8
                        };
                        let paint_run = |first_slice: usize, end_slice: usize, level: u8| {
                            if level == 0 {
                                return;
                            }
                            let start_frac = (first_slice as f64 * slice_width_ms / total_ms).clamp(0.0, 1.0);
                            let end_frac = (end_slice as f64 * slice_width_ms / total_ms).clamp(0.0, 1.0);
                            if end_frac <= start_frac {
                                return;
                            }
                            let x_start = rect.min.x + (start_frac as f32) * lane_width;
                            let x_end = rect.min.x + (end_frac as f32) * lane_width;
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(x_start, rect.min.y),
                                egui::pos2(x_end.max(x_start + 1.0), rect.max.y),
                            );
                            painter.rect_filled(bar, egui::Rounding::ZERO, ramp.color(level));
                        };
                        let mut run_start = 0usize;
                        let mut run_level = zone.busy_ms.first().map(|&b| level_of(b)).unwrap_or(0);
                        for (index, &busy_ms) in zone.busy_ms.iter().enumerate().skip(1) {
                            let level = level_of(busy_ms);
                            if level != run_level {
                                paint_run(run_start, index, run_level);
                                run_start = index;
                                run_level = level;
                            }
                        }
                        paint_run(run_start, zone.busy_ms.len(), run_level);
                        if time_cursor_ms >= 0.0 && time_cursor_ms <= total_ms {
                            let cursor_x = rect.min.x + ((time_cursor_ms / total_ms) as f32) * lane_width;
                            painter.line_segment(
                                [egui::pos2(cursor_x, rect.min.y), egui::pos2(cursor_x, rect.max.y)],
                                egui::Stroke::new(1.0, theme::CURSOR),
                            );
                        }
                        if response.clicked() || response.dragged() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                *new_cursor = Some(lane_x_to_time(pos.x, rect.min.x, lane_width, total_ms));
                            }
                        }
                        // Hover: the exact utilization of the slice under
                        // the pointer (the raw fraction, not the quantized
                        // paint level), plus its zone and time window.
                        if let Some(pos) = response.hover_pos() {
                            let hover_ms = lane_x_to_time(pos.x, rect.min.x, lane_width, total_ms);
                            let slice_index = (hover_ms / slice_width_ms) as usize;
                            if let Some(&busy_ms) = zone.busy_ms.get(slice_index) {
                                let pct = 100.0 * busy_ms as f64 / denominator;
                                let start = slice_index as f64 * slice_width_ms;
                                let end = start + slice_width_ms;
                                let tooltip_id = ui.id().with((tooltip_key, zone.zone_id, slice_index));
                                let layer_id = egui::LayerId::new(egui::Order::Tooltip, tooltip_id);
                                egui::show_tooltip_at_pointer(ui.ctx(), layer_id, tooltip_id, |ui| {
                                    ui.monospace(format!("zone {}", zone.zone_id));
                                    ui.monospace(format!("{pct:.2}% occupied"));
                                    ui.monospace(format!(
                                        "{}–{}  ({} relays)",
                                        format_window(start),
                                        format_window(end),
                                        zone.relays,
                                    ));
                                });
                            }
                        }
                        ui.add_space(row_spacing);
                    }
                });
            });
        if (out.state.offset.x - *scroll_x).abs() > 0.5 {
            *scroll_x = out.state.offset.x;
        }
    });
}

/// Quantization steps on a heat ramp (level 0 = 0% = white, level
/// `HEAT_LEVELS` = 100% = the ramp's full color).
const HEAT_LEVELS: usize = 64;

/// Which white→color heat ramp a lane section uses. Two hues so the two
/// relay resources read apart at a glance: red for HTM-access-cell
/// utilization ("how busy is the approach"), green for diving-board slot
/// occupancy ("how full is the buffer").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HeatRamp {
    Red,
    Green,
}

impl HeatRamp {
    /// Full-color sRGB endpoint at level `HEAT_LEVELS` (100%).
    fn endpoint_srgb(self) -> [u8; 3] {
        match self {
            HeatRamp::Red => [220, 30, 30],
            HeatRamp::Green => [30, 150, 40],
        }
    }

    /// Quantized heat color, interpolated white→endpoint in Oklab so equal
    /// steps read as equal perceptual steps (a plain sRGB lerp bunches the
    /// visible change into the light end). LUT-backed per ramp — endpoints
    /// and per-level conversion are computed once.
    fn color(self, level: u8) -> Color32 {
        use std::sync::OnceLock;
        static RED: OnceLock<[Color32; HEAT_LEVELS + 1]> = OnceLock::new();
        static GREEN: OnceLock<[Color32; HEAT_LEVELS + 1]> = OnceLock::new();
        let lut = match self {
            HeatRamp::Red => RED.get_or_init(|| build_heat_ramp(HeatRamp::Red.endpoint_srgb())),
            HeatRamp::Green => GREEN.get_or_init(|| build_heat_ramp(HeatRamp::Green.endpoint_srgb())),
        };
        lut[(level as usize).min(HEAT_LEVELS)]
    }
}

/// Build the `HEAT_LEVELS + 1` white→`endpoint_srgb` ramp LUT, interpolating
/// in Oklab (perceptually even) and re-encoding to sRGB.
fn build_heat_ramp(endpoint_srgb: [u8; 3]) -> [Color32; HEAT_LEVELS + 1] {
    let white = oklab_from_linear([1.0, 1.0, 1.0]);
    let end = oklab_from_linear(linear_from_srgb(endpoint_srgb));
    std::array::from_fn(|level| {
        let t = level as f32 / HEAT_LEVELS as f32;
        let lab = [
            white[0] + (end[0] - white[0]) * t,
            white[1] + (end[1] - white[1]) * t,
            white[2] + (end[2] - white[2]) * t,
        ];
        let [r, g, b] = linear_from_oklab(lab);
        Color32::from_rgb(srgb_u8(r), srgb_u8(g), srgb_u8(b))
    })
}

/// sRGB-encoded u8 channels → linear RGB (inverse of [`srgb_u8`]).
fn linear_from_srgb([r, g, b]: [u8; 3]) -> [f32; 3] {
    let f = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    [f(r), f(g), f(b)]
}

/// Linear RGB → Oklab (Björn Ottosson's reference matrices).
fn oklab_from_linear([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// Oklab → linear RGB (inverse of [`oklab_from_linear`]).
fn linear_from_oklab([l_ok, a, b_ok]: [f32; 3]) -> [f32; 3] {
    let l = l_ok + 0.3963377774 * a + 0.2158037573 * b_ok;
    let m = l_ok - 0.1055613458 * a - 0.0638541728 * b_ok;
    let s = l_ok - 0.0894841775 * a - 1.2914855480 * b_ok;
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

/// Linear component → sRGB-encoded u8.
fn srgb_u8(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let srgb = if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (srgb * 255.0).round() as u8
}

/// Small `0% → 100%` gradient strip explaining a heat-lane section's shading,
/// drawn in that section's ramp.
fn draw_heat_legend(ui: &mut egui::Ui, ramp: HeatRamp) {
    ui.label(egui::RichText::new("0%").small().color(theme::TEXT_2));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, 8.0), egui::Sense::hover());
    let painter = ui.painter();
    let step = rect.width() / (HEAT_LEVELS + 1) as f32;
    for level in 0..=HEAT_LEVELS {
        let x_start = rect.min.x + level as f32 * step;
        let swatch =
            egui::Rect::from_min_max(egui::pos2(x_start, rect.min.y), egui::pos2(x_start + step + 0.5, rect.max.y));
        painter.rect_filled(swatch, egui::Rounding::ZERO, ramp.color(level as u8));
    }
    painter.rect_stroke(rect, egui::Rounding::same(1.0), egui::Stroke::new(0.5, theme::TEXT_2));
    ui.label(egui::RichText::new("100%").small().color(theme::TEXT_2));
}

fn draw_vtm_idle_reasons(ui: &mut egui::Ui, counts: &HashMap<VtmIdleReasonKind, u64>) {
    if counts.is_empty() {
        return;
    }
    use VtmIdleReasonKind::*;
    let rows = [
        ("Continuous", Continuous),
        ("Awaiting demand", AwaitingDemand),
        ("Awaiting tote", AwaitingTote),
        ("Awaiting slot", AwaitingSlot),
        ("Awaiting HTM deposit", AwaitingHtmDeposit),
    ];
    let total: u64 = rows.iter().map(|(_, kind)| counts.get(kind).copied().unwrap_or(0)).sum();
    let title = format!("DA50 cycle starts ({})", total);
    theme::collapsing_section(ui, "vtm_idle_reasons", &title, false, |ui| {
        for (label, kind) in rows {
            let count = counts.get(&kind).copied().unwrap_or(0);
            let pct = if total > 0 { count as f64 / total as f64 * 100.0 } else { 0.0 };
            ui.label(egui::RichText::new(format!("{label}: {count} ({pct:.0}%)")).small());
        }
    });
}

/// Format a visible-window duration (ms) as a compact human string for the
/// zoom readout: `1h`, `1h 30m`, `1m`, `1m 30s`, `30s`. At hour scale the
/// seconds are dropped (no `1h 30m 12s` clutter); the smaller unit is omitted
/// when it rounds to zero.
fn format_window(ms: f64) -> String {
    let total_secs = (ms / 1000.0).round().max(0.0) as u64;
    let (h, m, s) = (total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60);
    if h > 0 {
        if m > 0 { format!("{h}h {m}m") } else { format!("{h}h") }
    } else if m > 0 {
        if s > 0 { format!("{m}m {s}s") } else { format!("{m}m") }
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Color32, HEAT_LEVELS, HeatRamp, MIN_VISIBLE_WINDOW_MS, effective_window, format_window, lane_x_to_time,
        pin_window,
    };

    #[test]
    fn heat_ramp_endpoints_are_white_and_full_color() {
        for ramp in [HeatRamp::Red, HeatRamp::Green] {
            assert_eq!(ramp.color(0), Color32::from_rgb(255, 255, 255));
            let [r, g, b] = ramp.endpoint_srgb();
            assert_eq!(ramp.color(HEAT_LEVELS as u8), Color32::from_rgb(r, g, b));
            // Out-of-range levels clamp to the endpoint rather than wrapping.
            assert_eq!(ramp.color(u8::MAX), ramp.color(HEAT_LEVELS as u8));
        }
    }

    #[test]
    fn heat_ramp_lightness_falls_monotonically() {
        // Both ramps start at white and end at a darker full color, so
        // perceived lightness (approximated by the channel sum) never rises
        // step to step — no banding flips from the Oklab interpolation.
        for ramp in [HeatRamp::Red, HeatRamp::Green] {
            for level in 1..=HEAT_LEVELS as u8 {
                let prev = ramp.color(level - 1);
                let cur = ramp.color(level);
                let sum = |c: Color32| c.r() as u16 + c.g() as u16 + c.b() as u16;
                assert!(sum(cur) <= sum(prev), "{ramp:?} lightened at level {level}");
            }
        }
    }

    #[test]
    fn format_window_matches_examples() {
        // The shapes from the feature request, plus the boundary cases.
        assert_eq!(format_window(36_000_000.0), "10h"); // 10h run, fit-to-window
        assert_eq!(format_window(3_600_000.0), "1h");
        assert_eq!(format_window(5_400_000.0), "1h 30m"); // hour scale drops seconds
        assert_eq!(format_window(60_000.0), "1m");
        assert_eq!(format_window(90_000.0), "1m 30s");
        assert_eq!(format_window(30_000.0), "30s"); // the zoom floor
        // Rounds to the nearest second rather than showing fractional noise.
        assert_eq!(format_window(30_400.0), "30s");
        assert_eq!(format_window(29_600.0), "30s");
    }

    #[test]
    fn lane_x_maps_to_time() {
        // 100px lane (left edge at x=0) spanning a 1000s run.
        assert_eq!(lane_x_to_time(0.0, 0.0, 100.0, 1_000_000.0), 0.0); // left edge → t0
        assert_eq!(lane_x_to_time(50.0, 0.0, 100.0, 1_000_000.0), 500_000.0); // mid → halfway
        assert_eq!(lane_x_to_time(100.0, 0.0, 100.0, 1_000_000.0), 1_000_000.0); // right → end
        // A drag past either edge clamps into [0, total].
        assert_eq!(lane_x_to_time(-20.0, 0.0, 100.0, 1_000_000.0), 0.0);
        assert_eq!(lane_x_to_time(150.0, 0.0, 100.0, 1_000_000.0), 1_000_000.0);
        // Scrolled lane: left edge shifted off-screen to x=-200 (zoomed/panned).
        // A pointer at x=-175 is 25px into the lane → 25% of the run.
        assert_eq!(lane_x_to_time(-175.0, -200.0, 100.0, 1_000_000.0), 250_000.0);
    }

    #[test]
    fn fit_to_window_auto_scales_pinned_does_not() {
        // At the full picture (`None`) the window tracks the run as it grows —
        // auto-scaling.
        assert_eq!(effective_window(None, 3_600_000.0), 3_600_000.0);
        assert_eq!(effective_window(None, 36_000_000.0), 36_000_000.0);

        // Beneath the full picture (`Some(w)`) the window stays put as the run
        // grows — no auto-scaling. Same 1-minute window at 1h and at 10h total.
        let pinned = Some(60_000.0);
        assert_eq!(effective_window(pinned, 3_600_000.0), 60_000.0);
        assert_eq!(effective_window(pinned, 36_000_000.0), 60_000.0);

        // A pinned window is clamped to [30s, full run].
        assert_eq!(effective_window(Some(5_000.0), 3_600_000.0), MIN_VISIBLE_WINDOW_MS);
        assert_eq!(effective_window(Some(9_999_999.0), 3_600_000.0), 3_600_000.0);
    }

    #[test]
    fn pin_window_snaps_to_fit_at_full_run() {
        let total = 3_600_000.0;
        // Zooming out until the window reaches the full run snaps back to fit
        // (`None`), so it auto-scales again.
        assert_eq!(pin_window(total, total), None);
        assert_eq!(pin_window(total + 1.0, total), None);
        // Below the full run it pins, floored at the 30s minimum.
        assert_eq!(pin_window(120_000.0, total), Some(120_000.0));
        assert_eq!(pin_window(1_000.0, total), Some(MIN_VISIBLE_WINDOW_MS));
    }

    #[test]
    fn runs_shorter_than_the_30s_floor_dont_panic() {
        // A 10s run: the 30s floor would make `clamp(min, max)` have min > max
        // and panic. Both helpers must treat the whole short run as the window.
        let total = 10_000.0;
        assert_eq!(effective_window(None, total), total);
        assert_eq!(effective_window(Some(60_000.0), total), total); // would panic unguarded
        assert_eq!(pin_window(5_000.0, total), None); // can't zoom into a 10s run
    }
}
