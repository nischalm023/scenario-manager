//! Bot inspector body (HTM & VTM): current cell, what the bot is doing, current
//! task/leg + next waypoints (from the committed path in `bot_locations`),
//! blocked/idle diagnosis, HTM active assignment, and the recent-events feed.
//!
//! "Current task/leg" is derived from what already flows — the committed
//! waypoint steps in the cursor slice — plus the HTM assignment index. There
//! is no separate plan-snapshot payload: the Nav-derived steps in the slice
//! already carry the future path (see design.md §data-flow / deviations).

use dessim::model::SimTime;
use dessim::observer::activity::ActivityKind;
use dessim::observer::time_slice::{BotStep, BotStepKind};
use eframe::egui;

use super::super::DesvisApp;
use super::steps::{StepInfo, WaypointRun};
use super::{InspectorAction, ResourceTimeline, eta_label, fmt_hms, recent_events_section, steps, strip};
use crate::types::*;

/// Cap on the "next waypoints" list shown in the bot inspector.
const NEXT_WAYPOINTS_MAX: usize = 6;

impl DesvisApp {
    /// Returns the frame's `InspectorAction` (strip scrub and/or a cross-link
    /// selection from the assignment's station / relay links).
    pub(super) fn bot_inspector_ui(
        &self,
        ui: &mut egui::Ui,
        bot_type: BotType,
        bot_id: u32,
        cursor: SimTime,
        max_ms: f64,
    ) -> InspectorAction {
        let store = self.station_diag_store.as_ref().expect("store built in draw_inspector_window");
        let mut action = InspectorAction::default();
        let current_cell = match self.bot_positions.iter().find(|b| b.bot_id == bot_id && b.bot_type == bot_type) {
            Some(bot) => {
                ui.label(format!("current cell: {}", bot.cell_id));
                Some(bot.cell_id)
            }
            None => {
                ui.label("current cell: unknown");
                None
            }
        };

        // Co-location cross-link: if the bot is standing on a relay-owned cell,
        // link to that relay (the reverse of the relay inspector's "bots here"
        // — reachable without click-cycling the shared cell).
        if let Some(cell) = current_cell {
            if let Some(&meta_index) = self.relay_by_cell.get(&cell) {
                let relay = &self.relay_metas[meta_index];
                ui.horizontal(|ui| {
                    ui.label("at relay:");
                    if ui.link(format!("{} (cell {})", relay.relay_id, relay.htm_access_cell)).clicked() {
                        action.select = Some(SelectedResource::Relay { htm_access_cell: relay.htm_access_cell });
                    }
                });
            }
        }

        // What the bot is doing as of the cursor.
        match store.activity_at(bot_id, cursor) {
            Some((kind, since)) => ui.label(format!("doing: {kind} (since {})", fmt_hms(since))),
            None => ui.weak("doing: no activity recorded"),
        };

        // Full-run activity strip (gaps visible + scrubbable), from the cached
        // extraction (audit M3).
        action.scrub = match &self.resource_timeline_cache {
            Some(ResourceTimeline::Bot { spans, .. }) => strip::bot_strip(ui, spans, cursor.as_ms() as f64, max_ms),
            _ => None,
        };

        // Current task / next legs, then blocked/idle descriptor.
        self.bot_path_section(ui, bot_id, cursor);
        self.bot_idle_section(ui, bot_type, bot_id, cursor);

        // HTM active assignment (station + ETA), with cross-link rows.
        if bot_type == BotType::Htm {
            ui.separator();
            match store.active_assignment_for_bot(bot_id, cursor) {
                Some(assignment) => {
                    let (eta, overdue) = eta_label(assignment.quoted_arrival, cursor);
                    ui.horizontal(|ui| {
                        ui.strong("assigned → station");
                        if ui.link(format!("{}", assignment.station_id.0)).clicked() {
                            action.select = Some(SelectedResource::Station(assignment.station_id));
                        }
                    });
                    ui.label(format!("source: {}", assignment.source));
                    match assignment.tote_id {
                        Some(tote) => ui.label(format!("tote: {tote}")),
                        None => ui.label("tote: none"),
                    };
                    match assignment.relay_cell {
                        Some(cell) => {
                            ui.horizontal(|ui| {
                                ui.label("relay cell:");
                                if ui.link(format!("{cell}")).clicked() {
                                    action.select = Some(SelectedResource::Relay { htm_access_cell: cell });
                                }
                            });
                        }
                        None => {
                            ui.label("relay cell: none (direct)");
                        }
                    };
                    ui.label(format!("dispatched: {}", fmt_hms(assignment.dispatch_time)));
                    if overdue {
                        ui.colored_label(egui::Color32::from_rgb(240, 100, 90), format!("ETA: {eta}"));
                    } else {
                        ui.label(format!("ETA: {eta}"));
                    }
                }
                None => {
                    ui.weak("no active station assignment");
                }
            }
        }

        recent_events_section(ui, cursor, &store.recent_bot_events(bot_id, cursor));
        action
    }

    /// The bot's current task — real planner leg semantics (kind + destination)
    /// from the plan snapshot, augmented with the committed step-level detail.
    fn bot_path_section(&self, ui: &mut egui::Ui, bot_id: u32, cursor: SimTime) {
        ui.separator();

        // Real leg semantics from the committed plan snapshot: the current leg
        // (the one whose enter is the latest <= cursor... expressed as "the
        // next leg at/after cursor is the current target") plus the next few.
        let store = self.station_diag_store.as_ref().expect("store built in draw_inspector_window");
        let legs = store.plan_legs_from(bot_id, cursor, NEXT_WAYPOINTS_MAX + 1);
        if let Some((kind, dest, enter)) = legs.first() {
            ui.label(format!("current leg: {} → cell {} (at {})", kind.label(), dest, fmt_hms(*enter)));
            if legs.len() > 1 {
                ui.label("next legs:");
                for (kind, dest, enter) in &legs[1..] {
                    ui.label(format!("  {} → cell {} (at {})", kind.label(), dest, fmt_hms(*enter)));
                }
            }
        } else {
            ui.weak("plan legs: none remaining (committed plan complete)");
        }

        // Step-level committed detail (Move/Wait/Rotate + cells) from the
        // cursor slice — finer than legs, the truthful geometry.
        let Some((location, slice_start_ms)) = self.bot_location_at(cursor, bot_id) else {
            ui.weak("current step: no committed movement in this slice");
            return;
        };
        // Find the step active at the cursor and collect the following ones.
        let mut step_end_ms = slice_start_ms;
        let mut active: Option<&BotStep> = None;
        let mut upcoming: Vec<&BotStep> = Vec::new();
        for step in &location.steps {
            step_end_ms += step.delta_ms;
            if step_end_ms >= cursor.as_ms() {
                if active.is_none() {
                    active = Some(step);
                } else {
                    upcoming.push(step);
                }
            }
        }
        match active {
            Some(step) => ui.label(format!("current step: {} at cell {}", step_kind_label(step.kind), step.cell_id)),
            None => ui.weak("current step: path complete for this slice"),
        };
        // Run-length aggregated waypoint rows: straight Move runs collapse to
        // one row, consecutive Waits/Rotates coalesce. Dwell is not
        // distinguishable from Wait in BotStepKind, so dwells read as Wait.
        let step_infos: Vec<StepInfo> = upcoming
            .iter()
            .map(|step| StepInfo {
                kind: step.kind,
                cell: step.cell_id,
                duration_ms: step.delta_ms,
                pos_mm: self
                    .cell_infos
                    .get(step.cell_id as usize)
                    .and_then(|i| i.as_ref())
                    .map(|info| info.center_mm()),
            })
            .collect();
        let runs = steps::aggregate_steps(&step_infos);
        if !runs.is_empty() {
            ui.label("next waypoints:");
            for run in runs.iter().take(NEXT_WAYPOINTS_MAX) {
                let row = match run {
                    WaypointRun::Move { from_cell, to_cell, cells, duration_ms } if cells > &1 => {
                        format!("  Move {from_cell}→{to_cell} ({cells} cells, {:.1}s)", *duration_ms as f64 / 1000.0)
                    }
                    WaypointRun::Move { from_cell, duration_ms, .. } => {
                        format!("  Move → cell {from_cell} ({:.1}s)", *duration_ms as f64 / 1000.0)
                    }
                    WaypointRun::Wait { duration_ms } => {
                        format!("  Wait ({:.1}s)", *duration_ms as f64 / 1000.0)
                    }
                    WaypointRun::Rotate { duration_ms } => {
                        format!("  Rotate ({:.1}s)", *duration_ms as f64 / 1000.0)
                    }
                };
                ui.label(row);
            }
            if runs.len() > NEXT_WAYPOINTS_MAX {
                ui.weak(format!("  … {} more", runs.len() - NEXT_WAYPOINTS_MAX));
            }
        }
    }

    /// Blocked/idle diagnosis. Truthful signals only: a VTM's classified idle
    /// reason (the sim records one for VTMs), or — for HTMs, which record NO
    /// idle reason — a lightweight descriptor derived from state + assignment
    /// data already in the store. Plus a Wait step at the cursor. Never
    /// invents a reason the sim doesn't record.
    fn bot_idle_section(&self, ui: &mut egui::Ui, bot_type: BotType, bot_id: u32, cursor: SimTime) {
        let store = self.station_diag_store.as_ref().expect("store built in draw_inspector_window");
        if bot_type == BotType::Vtm {
            if let Some((reason, gap, at)) = store.latest_vtm_idle_reason(bot_id, cursor) {
                ui.separator();
                ui.label(format!("idle reason: {reason:?} (gap {}s, at {})", gap.as_ms() / 1000, fmt_hms(at)));
            }
        }
        // HTM idle descriptor. Nothing classifies WHY a D30 sits idle, so
        // this describes rather than diagnoses: a gap at the cursor plus the
        // bot's assignment separates "no assignment" from "assigned, not yet
        // moving", and carries the instant the gap began.
        if bot_type == BotType::Htm
            && let Some((ActivityKind::Gap, since)) = store.activity_at(bot_id, cursor)
        {
            ui.separator();
            let assigned = store.active_assignment_for_bot(bot_id, cursor).is_some();
            let descriptor = if assigned { "assigned, not yet moving" } else { "idle, no assignment" };
            ui.colored_label(
                egui::Color32::from_rgb(240, 180, 90),
                format!("{descriptor} · idle since {}", fmt_hms(since)),
            );
        }
        // A Wait step spanning the cursor = "waiting at <cell> since <t>".
        if let Some((location, slice_start_ms)) = self.bot_location_at(cursor, bot_id) {
            let mut step_start_ms = slice_start_ms;
            for step in &location.steps {
                let step_end_ms = step_start_ms + step.delta_ms;
                if step.kind == BotStepKind::Wait && step_start_ms <= cursor.as_ms() && cursor.as_ms() < step_end_ms {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 180, 90),
                        format!("waiting at cell {} since {}", step.cell_id, fmt_hms(SimTime::ms(step_start_ms))),
                    );
                    break;
                }
                step_start_ms = step_end_ms;
            }
        }
    }
}

fn step_kind_label(kind: BotStepKind) -> &'static str {
    match kind {
        BotStepKind::Move => "Move",
        BotStepKind::Reverse => "Reverse",
        BotStepKind::Wait => "Wait",
        BotStepKind::Rotate => "Rotate",
    }
}
