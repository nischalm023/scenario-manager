//! Per-resource timeline strip for the inspector: a compact full-run
//! `[0, max_ms]` gantt/histogram showing where the selected resource is idle
//! vs busy, with the global playback cursor drawn in and click/drag-to-scrub
//! (mirrors `gantt.rs`). One strip per resource kind; all data comes from the
//! per-slice diag store + time series, so it works live and in playback.
//!
//! No zoom: the bottom Gantt/chart panel already covers detailed multi-lane
//! inspection. This strip is a glance-level "see the idle spots" view for the
//! one selected resource.

use dessim::events::VtmIdleReasonKind;
use dessim::model::SimTime;
use eframe::egui;

use crate::gantt::activity_color;
use crate::station_diag::{ActivitySpan, ReservedSpan, ServingSpan};
use crate::theme;

/// Strip height in points — compact, per the brief (~40–60px).
const STRIP_HEIGHT: f32 = 46.0;

/// Hover target radius for tick marks, in screen pixels. Converted to a ms
/// tolerance per-strip so it stays a few px regardless of run length.
const TICK_HOVER_TOLERANCE_PX: f32 = 4.0;

/// Allocate the strip rect, paint its background + cursor, resolve click/drag
/// to a scrubbed sim time, and attach a hover tooltip. Returns `Some(ms)` when
/// the user scrubbed. `fill` paints the interval content; `describe` maps the
/// sim time under the pointer (and a screen-space tick tolerance in ms) to
/// tooltip text. Shared by every resource strip.
fn timeline_strip(
    ui: &mut egui::Ui,
    cursor_ms: f64,
    max_ms: f64,
    fill: impl FnOnce(&egui::Painter, egui::Rect, &dyn Fn(f64) -> f32),
    describe: impl Fn(SimTime, u64) -> Option<String>,
) -> Option<f64> {
    let width = ui.available_width();
    let (response, painter) = ui.allocate_painter(egui::vec2(width, STRIP_HEIGHT), egui::Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, egui::Rounding::same(2.0), theme::BG_1);

    let span = max_ms.max(1.0);
    let ms_to_x = move |ms: f64| rect.min.x + (ms / span).clamp(0.0, 1.0) as f32 * rect.width();
    fill(&painter, rect, &ms_to_x);

    // Playback cursor line.
    if cursor_ms >= 0.0 && cursor_ms <= max_ms {
        let x = ms_to_x(cursor_ms);
        painter
            .line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], egui::Stroke::new(1.5, theme::ERR));
    }

    // Hover tooltip for the interval/mark under the pointer. Tick tolerance is
    // fixed in screen space (`TICK_HOVER_TOLERANCE_PX`) converted to ms via the
    // strip's px-per-ms, so a 10h run doesn't get a ±6-minute hover target.
    if let Some(pointer) = response.hover_pos() {
        let frac = ((pointer.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64;
        let tol_ms = (TICK_HOVER_TOLERANCE_PX as f64 * max_ms / rect.width().max(1.0) as f64) as u64;
        if let Some(text) = describe(SimTime::ms((frac * max_ms) as u64), tol_ms) {
            egui::show_tooltip_at_pointer(ui.ctx(), egui::LayerId::background(), egui::Id::new("strip_tip"), |ui| {
                ui.label(text);
            });
        }
    }

    // Click/drag scrubs the global cursor to the sim time under the pointer.
    if response.clicked() || response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let frac = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64;
            return Some(frac * max_ms);
        }
    }
    None
}

/// `mm:ss` (or `h:mm:ss`) label for tooltips.
fn hms(time: SimTime) -> String {
    let s = time.as_ms() / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Bot strip: what the bot was doing, colored by kind; a VTM's annotated gap
/// gets a hatch-dark shade and its idle-reason in the hover tooltip.
pub(super) fn bot_strip(ui: &mut egui::Ui, spans: &[ActivitySpan], cursor_ms: f64, max_ms: f64) -> Option<f64> {
    timeline_strip(
        ui,
        cursor_ms,
        max_ms,
        |painter, rect, ms_to_x| {
            for span in spans {
                let x0 = ms_to_x(span.start.as_ms() as f64);
                let x1 = ms_to_x(span.end.as_ms() as f64).max(x0 + 1.0);
                let mut color = activity_color(span.kind);
                // An annotated gap (a VTM waiting on a recorded reason) reads
                // darker than a plain one so "idle because X" stands out.
                if span.idle_reason.is_some() {
                    color = darken(color, 40);
                }
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(x0, rect.min.y), egui::pos2(x1, rect.max.y)),
                    egui::Rounding::ZERO,
                    color,
                );
            }
        },
        |at, _tol_ms| {
            spans.iter().find(|span| span.covers(at)).map(|span| {
                let reason = span.idle_reason.map(idle_reason_name).map(|r| format!(" — {r}")).unwrap_or_default();
                format!("{}{reason}\n{} – {}", span.kind, hms(span.start), hms(span.end))
            })
        },
    )
}

/// Station strip: green serving spans over a pale idle track; rejection marks
/// as amber ticks in the gaps so blocked gaps read differently from quiet ones.
pub(super) fn station_strip(
    ui: &mut egui::Ui,
    serving: &[ServingSpan],
    rejection_marks: &[(SimTime, u64)],
    cursor_ms: f64,
    max_ms: f64,
) -> Option<f64> {
    timeline_strip(
        ui,
        cursor_ms,
        max_ms,
        |painter, rect, ms_to_x| {
            // Idle track background.
            painter.rect_filled(rect, egui::Rounding::ZERO, theme::STATE_IDLE);
            for span in serving {
                let x0 = ms_to_x(span.start.as_ms() as f64);
                let x1 = ms_to_x(span.end.as_ms() as f64).max(x0 + 1.0);
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(x0, rect.min.y), egui::pos2(x1, rect.max.y)),
                    egui::Rounding::ZERO,
                    theme::OK,
                );
            }
            // Rejection ticks — thin amber marks along the bottom third.
            for (at, _count) in rejection_marks {
                let x = ms_to_x(at.as_ms() as f64);
                painter.line_segment(
                    [egui::pos2(x, rect.max.y - rect.height() / 3.0), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(1.0, theme::STATE_LOADING),
                );
            }
        },
        |at, tol_ms| {
            if let Some(span) = serving.iter().find(|span| span.covers(at)) {
                return Some(format!("Serving\n{} – {}", hms(span.start), hms(span.end)));
            }
            // Nearest rejection mark within the screen-space hover tolerance.
            rejection_marks
                .iter()
                .find(|(mark_at, _)| mark_at.as_ms().abs_diff(at.as_ms()) <= tol_ms)
                .map(|(mark_at, count)| format!("blocked ×{count}\nat {}", hms(*mark_at)))
        },
    )
}

/// Relay strip: reserved spans over a free track; pickup-wait ticks.
pub(super) fn relay_strip(
    ui: &mut egui::Ui,
    reserved: &[ReservedSpan],
    pickup_marks: &[(SimTime, u64)],
    cursor_ms: f64,
    max_ms: f64,
) -> Option<f64> {
    timeline_strip(
        ui,
        cursor_ms,
        max_ms,
        |painter, rect, ms_to_x| {
            for span in reserved {
                let x0 = ms_to_x(span.start.as_ms() as f64);
                let x1 = ms_to_x(span.end.as_ms() as f64).max(x0 + 1.0);
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(x0, rect.min.y), egui::pos2(x1, rect.max.y)),
                    egui::Rounding::ZERO,
                    theme::STATE_TRAVELING,
                );
            }
            for (at, _wait) in pickup_marks {
                let x = ms_to_x(at.as_ms() as f64);
                painter.line_segment(
                    [egui::pos2(x, rect.max.y - rect.height() / 3.0), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(1.0, theme::STATE_LOADING),
                );
            }
        },
        |at, tol_ms| {
            if let Some(span) = reserved.iter().find(|span| span.covers(at)) {
                return Some(format!("Reserved\n{} – {}", hms(span.start), hms(span.end)));
            }
            pickup_marks
                .iter()
                .find(|(mark_at, _)| mark_at.as_ms().abs_diff(at.as_ms()) <= tol_ms)
                .map(|(mark_at, wait)| format!("pickup wait {wait}ms\nat {}", hms(*mark_at)))
        },
    )
}

fn idle_reason_name(reason: VtmIdleReasonKind) -> &'static str {
    match reason {
        VtmIdleReasonKind::Continuous => "continuous",
        VtmIdleReasonKind::AwaitingDemand => "awaiting demand",
        VtmIdleReasonKind::AwaitingTote => "awaiting tote",
        VtmIdleReasonKind::AwaitingSlot => "awaiting slot",
        VtmIdleReasonKind::AwaitingHtmDeposit => "awaiting HTM deposit",
    }
}

fn darken(c: egui::Color32, by: u8) -> egui::Color32 {
    egui::Color32::from_rgb(c.r().saturating_sub(by), c.g().saturating_sub(by), c.b().saturating_sub(by))
}
