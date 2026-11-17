//! Resource inspector: a closable right-docked window showing, as-of the
//! playback cursor, per-resource dispatch state — station (inbound HTMs,
//! blockers, pipeline depth), bot (state / task / assignment / idle reason),
//! or relay (reserved state, pickup waits, HTMs routed through it). Plus a
//! shared "recent events" feed. Split into one submodule per resource kind
//! to keep each body focused and under the file/function limits.

mod bot;
mod jump;
mod relay;
mod station;
mod steps;
mod strip;

use desgraph::{StationId, StationType};
use dessim::model::SimTime;
use eframe::egui;

use super::DesvisApp;
use crate::station_diag::{ActivitySpan, RecentEvent, ReservedSpan, ServingSpan};
use crate::types::*;

/// What an inspector body wants applied after its window borrow ends: a strip
/// scrub (`engage_cursor`) and/or a cross-link selection change. Both are
/// `Option` because most frames produce neither.
#[derive(Default)]
pub(super) struct InspectorAction {
    pub scrub: Option<f64>,
    pub select: Option<SelectedResource>,
}

/// Full-run timeline data extracted once per (resource, store-generation) and
/// cached on the app so the per-frame strip draw doesn't re-extract over the
/// whole run (audit M3). Rebuilt when the selection changes or the store is
/// invalidated. One variant per resource kind.
pub(crate) enum ResourceTimeline {
    Bot { key: (BotType, u32), spans: Vec<ActivitySpan> },
    Station { station_id: StationId, serving: Vec<ServingSpan>, rejection_marks: Vec<(SimTime, u64)> },
    Relay { htm_access_cell: u32, reserved: Vec<ReservedSpan>, pickup_marks: Vec<(SimTime, u64)> },
}

/// `h:mm:ss` sim-time label.
pub(super) fn fmt_hms(time: SimTime) -> String {
    let total_secs = time.as_ms() / 1000;
    format!("{}:{:02}:{:02}", total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60)
}

pub(super) fn bot_label(bot_type: BotType, bot_id: u32) -> String {
    match bot_type {
        BotType::Vtm => format!("A{bot_id} (VTM)"),
        BotType::Htm => format!("D{bot_id} (HTM)"),
        BotType::Rtp => format!("R{bot_id} (RTP)"),
    }
}

/// Short lowercase station-type word (`pick` / `putaway`).
pub(super) fn station_type_label(station_type: StationType) -> &'static str {
    match station_type {
        StationType::Pick => "pick",
        StationType::Putaway => "putaway",
        StationType::BoxInduction => "box induction",
        StationType::Extraction => "extraction",
        StationType::ContainerPickup => "container pickup",
    }
}

/// Type-qualified station name used in the inspector title and jump list, so
/// pick and putaway/induction stations read apart at a glance:
/// `Pick station 5` / `Putaway station 5`.
pub(super) fn station_label(station_type: StationType, id: StationId, process: Option<&str>) -> String {
    if let Some(process) = process {
        return format!("{process} {}", id.0);
    }
    match station_type {
        StationType::Pick => format!("Pick station {}", id.0),
        StationType::Putaway => format!("Putaway station {}", id.0),
        StationType::BoxInduction => format!("Box induction {}", id.0),
        StationType::Extraction => format!("Extraction station {}", id.0),
        StationType::ContainerPickup => format!("Container pickup {}", id.0),
    }
}

/// Signed countdown label against the cursor: "in 12s" / "overdue 5s".
pub(super) fn eta_label(quoted: SimTime, cursor: SimTime) -> (String, bool) {
    if quoted >= cursor {
        (format!("in {}s", (quoted - cursor).as_ms() / 1000), false)
    } else {
        (format!("overdue {}s", (cursor - quoted).as_ms() / 1000), true)
    }
}

/// Render a "Recent events" section — newest first, relative timestamps. Says
/// so explicitly when empty rather than leaving a bare heading.
pub(super) fn recent_events_section(ui: &mut egui::Ui, cursor: SimTime, events: &[RecentEvent]) {
    ui.separator();
    ui.strong("Recent events");
    if events.is_empty() {
        ui.weak("none in window");
        return;
    }
    for event in events {
        let ago = cursor.as_ms().saturating_sub(event.at.as_ms()) / 1000;
        ui.label(format!("−{ago}s · {}", event.text));
    }
}

impl DesvisApp {
    pub(crate) fn draw_inspector_window(&mut self, ctx: &egui::Context) {
        let Some(selected) = self.selected_resource else { return };
        // Build the diag store + cached timeline once for this frame's queries.
        self.ensure_station_diag_store();
        let cursor = self.resource_cursor();

        let title = match selected {
            SelectedResource::Station(station_id) => self
                .station_metas
                .iter()
                .find(|meta| meta.id == station_id)
                .map(|meta| station_label(meta.station_type, station_id, meta.process_label))
                .unwrap_or_else(|| format!("Station {}", station_id.0)),
            SelectedResource::Bot { bot_type, bot_id } => bot_label(bot_type, bot_id),
            SelectedResource::Relay { htm_access_cell } => format!("Relay @ cell {htm_access_cell}"),
        };
        // Full-run span for the timeline strip's x-axis and scrub clamp.
        let max_ms = self.time_series.last().map(|s| s.start_ms + self.slice_width_ms).unwrap_or(0) as f64;
        self.ensure_resource_timeline(selected, max_ms);

        let mut open = true;
        // The per-resource body returns an `InspectorAction` — a scrub and/or
        // a cross-link selection change — applied after the window borrow ends
        // (both mutate `self`).
        let action = egui::Window::new(title)
            .id(egui::Id::new("resource_inspector"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
            .default_width(320.0)
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                let jumped = self.draw_jump_to(ui);
                ui.label(egui::RichText::new(format!("as of {}", fmt_hms(cursor))).weak().small());
                ui.separator();
                let mut action = match selected {
                    SelectedResource::Station(station_id) => self.station_inspector_ui(ui, station_id, cursor, max_ms),
                    SelectedResource::Bot { bot_type, bot_id } => {
                        self.bot_inspector_ui(ui, bot_type, bot_id, cursor, max_ms)
                    }
                    SelectedResource::Relay { htm_access_cell } => {
                        self.relay_inspector_ui(ui, htm_access_cell, cursor, max_ms)
                    }
                };
                // A jump-to pick overrides any row cross-link this frame.
                if jumped.is_some() {
                    action.select = jumped;
                }
                action
            })
            .and_then(|inner| inner.inner);
        if !open {
            self.selected_resource = None;
        }
        // `None` = window collapsed this frame (no body ran) — nothing to do.
        let Some(action) = action else { return };
        if let Some(new_cursor) = action.scrub {
            self.engage_cursor(new_cursor, max_ms);
        }
        // A cross-link click retargets the inspector; drop the cached timeline
        // so the strip rebuilds for the newly-selected resource next frame.
        if let Some(target) = action.select {
            self.selected_resource = Some(target);
            self.resource_timeline_cache = None;
        }
    }

    /// Ensure `resource_timeline_cache` holds the full-run timeline for
    /// `selected`. Rebuilds when the cache is empty or was extracted for a
    /// different resource; the store-invalidation sites already clear it, so a
    /// present cache for the same resource is up to date (audit M3). Extract
    /// into locals first (immutable borrows) before assigning the field.
    fn ensure_resource_timeline(&mut self, selected: SelectedResource, max_ms: f64) {
        let matches = match (&self.resource_timeline_cache, selected) {
            (Some(ResourceTimeline::Bot { key, .. }), SelectedResource::Bot { bot_type, bot_id }) => {
                *key == (bot_type, bot_id)
            }
            (Some(ResourceTimeline::Station { station_id, .. }), SelectedResource::Station(id)) => *station_id == id,
            (Some(ResourceTimeline::Relay { htm_access_cell, .. }), SelectedResource::Relay { htm_access_cell: c }) => {
                *htm_access_cell == c
            }
            _ => false,
        };
        if matches {
            return;
        }
        // Scope the store borrow so it ends before the cache assign below —
        // `built` owns all its data (no references into the store).
        let built = {
            let store = self.station_diag_store.as_ref().expect("store built in draw_inspector_window");
            match selected {
                SelectedResource::Bot { bot_type, bot_id } => ResourceTimeline::Bot {
                    key: (bot_type, bot_id),
                    spans: store.activity_spans(bot_id, SimTime::ms(max_ms as u64)),
                },
                SelectedResource::Station(station_id) => ResourceTimeline::Station {
                    station_id,
                    serving: store.station_serving_spans(station_id),
                    rejection_marks: store.station_rejection_marks(station_id),
                },
                SelectedResource::Relay { htm_access_cell } => {
                    let pickup_marks = store.relay_pickup_marks(htm_access_cell);
                    // Reserved spans need the per-slice relay_state (on
                    // time_series, not the store) — reconstruct inspector-side.
                    let reserved = self.relay_reserved_spans(htm_access_cell, max_ms);
                    ResourceTimeline::Relay { htm_access_cell, reserved, pickup_marks }
                }
            }
        };
        self.resource_timeline_cache = Some(built);
    }
}
