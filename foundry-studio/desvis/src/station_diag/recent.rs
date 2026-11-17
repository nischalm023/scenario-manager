//! Per-resource "recent events" feed builders over [`StationDiagStore`].
//! Split from `mod.rs` to keep the store's core cursor queries and the
//! feed rendering as separate concerns. A descendant module, so it reads the
//! store's private record vecs directly.

use std::cmp::Reverse;

use desgraph::StationId;
use dessim::model::SimTime;

use super::{RECENT_EVENTS_MAX, RECENT_EVENTS_WINDOW, RecentEvent, StationDiagStore};

impl StationDiagStore {
    /// Recent events for a station: assignments / arrivals / rejections at or
    /// before `cursor` within `RECENT_EVENTS_WINDOW`, newest first, capped at
    /// `RECENT_EVENTS_MAX`.
    pub fn recent_station_events(&self, station: StationId, cursor: SimTime) -> Vec<RecentEvent> {
        let window_start = cursor.saturating_sub(RECENT_EVENTS_WINDOW);
        let mut events: Vec<RecentEvent> = Vec::new();
        for assignment in &self.assignments {
            if assignment.dispatch_time > cursor {
                break;
            }
            if assignment.dispatch_time > window_start && assignment.station_id == station {
                events.push(RecentEvent {
                    at: assignment.dispatch_time,
                    text: format!("assigned D{} ({})", assignment.robot_id, assignment.source),
                });
            }
        }
        for arrival in &self.arrivals {
            if arrival.arrival > cursor {
                break;
            }
            if arrival.arrival > window_start && arrival.station_id == station {
                events.push(RecentEvent { at: arrival.arrival, text: format!("arrived D{}", arrival.robot_id) });
            }
        }
        for aggregate in &self.rejections {
            if aggregate.last_at > cursor {
                break;
            }
            if aggregate.last_at > window_start && aggregate.station_id == station {
                events.push(RecentEvent {
                    at: aggregate.last_at,
                    text: format!("blocked: {} ×{}", aggregate.reason, aggregate.count),
                });
            }
        }
        Self::newest_capped(events)
    }

    /// Recent events for a bot: the intervals of work it began, assignment
    /// starts, and (VTM) idle reasons at or before `cursor` within the window,
    /// newest first, capped.
    pub fn recent_bot_events(&self, robot_id: u32, cursor: SimTime) -> Vec<RecentEvent> {
        let window_start = cursor.saturating_sub(RECENT_EVENTS_WINDOW);
        let mut events: Vec<RecentEvent> = Vec::new();
        for record in &self.activity {
            if record.start > cursor {
                break;
            }
            if record.start > window_start && record.robot_id == robot_id {
                events.push(RecentEvent { at: record.start, text: format!("→ {}", record.kind) });
            }
        }
        for assignment in &self.assignments {
            if assignment.dispatch_time > cursor {
                break;
            }
            if assignment.dispatch_time > window_start && assignment.robot_id == robot_id {
                events.push(RecentEvent {
                    at: assignment.dispatch_time,
                    text: format!("assigned → station {}", assignment.station_id.0),
                });
            }
        }
        for record in &self.vtm_idle_reasons {
            if record.at > cursor {
                break;
            }
            if record.at > window_start && record.robot_id == robot_id {
                events.push(RecentEvent {
                    at: record.at,
                    text: format!("idle {:?} ({}s)", record.reason, record.gap_ms / 1000),
                });
            }
        }
        Self::newest_capped(events)
    }

    /// Recent events for a relay cell: reserve/free are carried by the
    /// per-slice relay_state (drawn by the inspector directly); here we
    /// surface pickup waits at or before `cursor` within the window, newest
    /// first, capped.
    pub fn recent_relay_events(&self, cell: u32, cursor: SimTime) -> Vec<RecentEvent> {
        let window_start = cursor.saturating_sub(RECENT_EVENTS_WINDOW);
        let mut events: Vec<RecentEvent> = Vec::new();
        for record in &self.htm_relay_pickups {
            if record.at > cursor {
                break;
            }
            if record.at > window_start && record.relay_cell == cell {
                events.push(RecentEvent {
                    at: record.at,
                    text: format!("D{} pickup wait {}ms", record.robot_id, record.wait_ms),
                });
            }
        }
        Self::newest_capped(events)
    }

    fn newest_capped(mut events: Vec<RecentEvent>) -> Vec<RecentEvent> {
        events.sort_by_key(|event| Reverse(event.at));
        events.truncate(RECENT_EVENTS_MAX);
        events
    }
}
