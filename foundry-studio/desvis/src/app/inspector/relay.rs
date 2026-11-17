//! Relay inspector body: reserved/free at cursor (from the per-slice
//! `relay_state` transitions that already flow), recent reserve/free
//! transitions, pickup waits, and HTMs routed through this relay.

use dessim::model::{SimDuration, SimTime};
use eframe::egui;

use super::super::DesvisApp;
use super::{InspectorAction, ResourceTimeline, bot_label, eta_label, recent_events_section, strip};
use crate::station_diag::ReservedSpan;
use crate::types::{BotType, SelectedResource};

/// Trailing window for the relay's reserve/free and pickup-wait summaries.
const RELAY_WINDOW: SimDuration = SimDuration::ms(60_000);

impl DesvisApp {
    /// Returns the frame's `InspectorAction` (strip scrub and/or a cross-link
    /// selection from a clicked "HTMs via relay" row).
    pub(super) fn relay_inspector_ui(
        &self,
        ui: &mut egui::Ui,
        htm_access_cell: u32,
        cursor: SimTime,
        max_ms: f64,
    ) -> InspectorAction {
        let store = self.station_diag_store.as_ref().expect("store built in draw_inspector_window");
        let mut action = InspectorAction::default();
        let meta_index = self.relay_metas.iter().position(|meta| meta.htm_access_cell == htm_access_cell);
        if let Some(meta) = meta_index.map(|index| &self.relay_metas[index]) {
            ui.label(format!("relay id: {}", meta.relay_id));
            ui.label(format!("block: {}", meta.block_id));
            ui.label(format!("HTM-access cell: {}", meta.htm_access_cell));
            ui.label(format!("VTM reporting anchor: {}", meta.vtm_access_cell));
        }

        // Full-run reserved/free timeline strip + pickup-wait ticks, from the
        // cached extraction (audit M3).
        action.scrub = match &self.resource_timeline_cache {
            Some(ResourceTimeline::Relay { reserved, pickup_marks, .. }) => {
                strip::relay_strip(ui, reserved, pickup_marks, cursor.as_ms() as f64, max_ms)
            }
            _ => None,
        };
        ui.separator();

        // Reserved/free at cursor, reconstructed from the per-slice relay_state
        // (start set + intra-slice transitions), same source the map overlay
        // uses. Keyed by the relay's HTM-access cell id.
        let reserved = self.relay_reserved_at(htm_access_cell, cursor);
        if reserved {
            ui.colored_label(egui::Color32::from_rgb(240, 180, 90), "state: reserved / occupied");
        } else {
            ui.label("state: free");
        }

        // Pickup-wait summary in the trailing window.
        let (wait_count, wait_total) = store.htm_relay_pickups_in_window(htm_access_cell, cursor, RELAY_WINDOW);
        ui.label(format!(
            "pickup waits (last {}s): {} · total {}ms",
            RELAY_WINDOW.as_ms() / 1000,
            wait_count,
            wait_total.as_ms()
        ));

        // Bots physically standing on this relay's cells right now — the
        // co-location cross-link (the reverse of the bot inspector's "at
        // relay"). Distinct from the assignment-based "routed through" list
        // below: a bot can sit here without a live relay assignment.
        if let Some(index) = meta_index {
            let here: Vec<(BotType, u32)> = self
                .bot_positions
                .iter()
                .filter(|bot| self.relay_by_cell.get(&bot.cell_id) == Some(&index))
                .map(|bot| (bot.bot_type, bot.bot_id))
                .collect();
            if !here.is_empty() {
                ui.separator();
                ui.strong("bots here now");
                for (bot_type, bot_id) in here {
                    if ui.link(bot_label(bot_type, bot_id)).clicked() {
                        action.select = Some(SelectedResource::Bot { bot_type, bot_id });
                    }
                }
            }
        }

        // HTMs currently routed through this relay (open assignments).
        let inbound = store.inbound_htms_via_relay(htm_access_cell, cursor);
        ui.separator();
        ui.strong(format!("HTMs via this relay ({})", inbound.len()));
        for htm in &inbound {
            let (eta, overdue) = eta_label(htm.quoted_arrival, cursor);
            let line = format!("D{} · {} · ETA {eta}", htm.robot_id, htm.source);
            let text = if overdue {
                egui::RichText::new(line).color(egui::Color32::from_rgb(240, 100, 90))
            } else {
                egui::RichText::new(line)
            };
            if ui.link(text).clicked() {
                action.select = Some(SelectedResource::Bot { bot_type: BotType::Htm, bot_id: htm.robot_id });
            }
        }
        if inbound.is_empty() {
            ui.weak("no HTMs routed through this relay in window");
        }

        recent_events_section(ui, cursor, &store.recent_relay_events(htm_access_cell, cursor));
        action
    }

    /// Full-run reserved spans for relay HTM-access `cell`, reconstructed from
    /// the per-slice `relay_state` (start-reserved set + intra-slice
    /// transitions) walked in order. An open reservation runs to the next
    /// free transition, or to `max_ms` if never freed. Uses absolute sim time
    /// (slice start + intra-slice offset).
    pub(super) fn relay_reserved_spans(&self, cell: u32, max_ms: f64) -> Vec<ReservedSpan> {
        let mut spans = Vec::new();
        let mut open_since: Option<u64> = None;
        for record in &self.time_series {
            let relay = &record.relay_state;
            // A slice's start-reserved set opens a span at slice start if not
            // already open (covers reservations spanning slice boundaries).
            if relay.start_reserved.contains(&cell) && open_since.is_none() {
                open_since = Some(record.start_ms);
            }
            let mut transitions: Vec<_> = relay.transitions.iter().filter(|(tcell, _, _)| *tcell == cell).collect();
            transitions.sort_by_key(|(_, offset, _)| *offset);
            for &(_, offset, is_reserved) in transitions {
                let at = record.start_ms + offset;
                match (is_reserved, open_since) {
                    (true, None) => open_since = Some(at),
                    (false, Some(start)) => {
                        if at > start {
                            spans.push(ReservedSpan { start: SimTime::ms(start), end: SimTime::ms(at) });
                        }
                        open_since = None;
                    }
                    _ => {}
                }
            }
        }
        if let Some(start) = open_since {
            let end = (max_ms as u64).max(start);
            spans.push(ReservedSpan { start: SimTime::ms(start), end: SimTime::ms(end) });
        }
        spans
    }

    /// Whether the relay HTM-access `cell` is reserved as-of `cursor`, from the
    /// per-slice `relay_state` record: start-reserved set for the cursor slice
    /// plus its intra-slice transitions up to the cursor offset. Mirrors the
    /// central panel's `build_reserved_grid_indices` reconstruction.
    fn relay_reserved_at(&self, cell: u32, cursor: SimTime) -> bool {
        if self.time_series.is_empty() {
            return false;
        }
        let slice_index = ((cursor.as_ms() / self.slice_width_ms) as usize).min(self.time_series.len() - 1);
        let record = &self.time_series[slice_index];
        let relay = &record.relay_state;
        let mut reserved = relay.start_reserved.contains(&cell);
        let offset_ms = cursor.as_ms().saturating_sub(record.start_ms);
        for &(transition_cell, transition_offset, is_reserved) in &relay.transitions {
            if transition_cell == cell && transition_offset <= offset_ms {
                reserved = is_reserved;
            }
        }
        reserved
    }
}
