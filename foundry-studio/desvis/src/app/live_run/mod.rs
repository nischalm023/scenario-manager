//! Within `app`, `live_run` is THE RUN BEING WATCHED: the bytes of the
//! recording a headed run writes become, as they reach the window, the app's
//! state — the slices, the heat, the robots at the live edge, the status and
//! the Utilization panel — and, once the run has returned, the profile file
//! playback holds.
//!
//! It exists so that a run watched as it goes and the same run played back
//! from its file are one reading of one recording: the window receives the
//! bytes the file takes, decodes them as a file is read (`IncomingRecording`),
//! and hears nothing else from the run but how it ended.

mod tap;
#[cfg(test)]
mod tests;
mod utilization;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use desgraph::RobotType;
use dessim::metrics::profile::SliceRecord;
use dessim::metrics::worldview::IncomingRecording;
use dessim::model::SimTime;
use dessim::{StopReason, StopSignal};

use super::relay_lanes::{extend_relay_lanes, relay_lanes};
use super::{DesvisApp, grid_data, heading};
use crate::output::{check_completion, print_results};
use crate::types::*;
pub(crate) use tap::{FromRun, RunTap};
pub(crate) use utilization::UtilizationSeen;

/// Max of a per-slice scalar across the series, 0 if absent everywhere.
/// Used to read a run's constant structural counts (`rack_capacity`,
/// `relay_cells_total`) off the slices.
pub(super) fn max_scalar(time_series: &[SliceRecord], key: &str) -> u32 {
    time_series.iter().filter_map(|s| s.metrics.get(key).copied()).fold(0.0_f64, f64::max) as u32
}

/// A run the window is watching: its recording, read as far as its bytes have
/// come, and what the window has folded out of the slices so far.
pub(crate) struct LiveRun {
    from_run: Receiver<FromRun>,
    recording: IncomingRecording,
    stop: Arc<StopSignal>,
    /// The picks and the fully picked orders the slices received state.
    picks: u64,
    orders_completed: u64,
    pub(crate) utilization: UtilizationSeen,
}

impl LiveRun {
    pub(crate) fn new(from_run: Receiver<FromRun>, stop: Arc<StopSignal>, utilization: UtilizationSeen) -> Self {
        Self { from_run, recording: IncomingRecording::default(), stop, picks: 0, orders_completed: 0, utilization }
    }

    /// Ask the run to stop, which it does at its next chance; its recording
    /// is closed as any run's is.
    pub(crate) fn stop(&self, reason: StopReason) {
        self.stop.stop(reason);
    }

    pub(crate) fn picks(&self) -> u64 {
        self.picks
    }

    pub(crate) fn orders_completed(&self) -> u64 {
        self.orders_completed
    }

    /// How many orders the run's scenario resolved to, once its recording's
    /// scenario block is in.
    pub(crate) fn orders(&self) -> Option<usize> {
        self.recording.orders()
    }

    /// Take everything the run has sent since the last call. `Some` once the
    /// window has heard the last of the run: how it ended, or the damage that
    /// stopped the reading, which stops the run too.
    fn drain(&mut self) -> Option<Result<(), String>> {
        loop {
            match self.from_run.try_recv() {
                Ok(FromRun::Bytes(bytes)) => {
                    if let Err(damage) = self.recording.push(&bytes) {
                        let damage = format!("the run's recording cannot be read: {damage}");
                        self.stop(StopReason::Error(damage.clone()));
                        return Some(Err(damage));
                    }
                }
                Ok(FromRun::Ended(outcome)) => return Some(outcome),
                Err(TryRecvError::Empty) => return None,
                // The thread sends its outcome before it lets go of the
                // channel, even when the run panics.
                Err(TryRecvError::Disconnected) => {
                    return Some(Err("the run's thread ended without saying how the run ended".into()));
                }
            }
        }
    }

    /// Fold the slices received from index `from` on into the status sums and
    /// the Utilization panel, and lend them.
    fn fold_from(&mut self, from: usize) -> &[SliceRecord] {
        let Some(width) = self.recording.slice_width() else { return &[] };
        if from == 0
            && let Some(roster) = self.recording.robots()
        {
            self.utilization.seat(roster);
        }
        let slices = &self.recording.slices()[from..];
        for slice in slices {
            self.picks += slice.get("picks_completed") as u64;
            self.orders_completed += slice.get("orders_completed") as u64;
            self.utilization.add_slice(slice, SimTime::ms(slice.start_ms) + width);
        }
        slices
    }
}

impl DesvisApp {
    /// Read what the run has sent since the last frame and fold it into what
    /// the window draws; once the run has ended, take its outcome.
    pub(crate) fn poll_live_run(&mut self) {
        let Some(live) = self.live_run.as_mut() else { return };
        let received = live.recording.slices().len();
        let ended = live.drain();
        self.take_slices_from(received);
        match ended {
            None => {}
            Some(Ok(())) => self.finish_live_run(),
            Some(Err(failure)) => self.sim_status = SimStatus::Error(failure),
        }
    }

    /// Fold the slices received from index `from` on into the series, the
    /// relay lanes and, while the cursor is not engaged, the heat and the
    /// robots at the live edge.
    fn take_slices_from(&mut self, from: usize) {
        let cells = self.cell_infos.len();
        let engaged = self.playback_engaged;
        let Some(live) = self.live_run.as_mut() else { return };
        let Some(width) = live.recording.slice_width() else { return };
        // The window draws from a series of its own, as it does on playback,
        // so each slice is held twice while the run goes: here and in the
        // intake, whose run model the window becomes when the run ends.
        let received = live.fold_from(from).to_vec();
        if received.is_empty() {
            return;
        }
        // The overlay a live run shows is the run total so far, as a recording
        // opens on its run total.
        let heat = match engaged {
            false => live.recording.cumulative_grid().map(|grid| grid_data::dynamic_grid_to_heatmap(grid, cells)),
            true => None,
        };

        self.slice_width_ms = width.as_ms();
        self.relay_lanes = match self.relay_lanes.take() {
            Some(lanes) => extend_relay_lanes(lanes, &received, &self.relay_zones),
            None if from == 0 => relay_lanes(&received, self.slice_width_ms, &self.relay_zones),
            None => None,
        };
        self.traffic.invalidate(from);
        self.time_series.extend(received);
        if !engaged {
            self.heatmap_data = heat;
            self.stand_robots_at_live_edge();
        }
        self.chart_cache = None;
        self.refresh_structural_counts();
        self.station_diag_store = None;
        self.resource_timeline_cache = None;
    }

    /// Stand each robot where the newest slice that moved any leaves it.
    /// Only while the cursor is not engaged: then `update_grid_for_cursor`
    /// owns where the robots stand, and the live edge would teleport them.
    pub(super) fn stand_robots_at_live_edge(&mut self) {
        let Some(slice) = self.time_series.iter().rev().find(|slice| !slice.bot_locations.is_empty()) else {
            return;
        };
        for (&bot_id, loc) in &slice.bot_locations {
            let final_cell = loc.steps.last().map(|s| s.cell_id).unwrap_or(loc.start_cell);
            let at = loc.steps.last().map_or(loc.start_pos, |step| step.pos);
            let pos_mm = [at.x_mm, at.y_mm];
            let bot_type = match loc.robot_type {
                RobotType::Vtm => BotType::Vtm,
                RobotType::Htm => BotType::Htm,
                RobotType::Rtp => BotType::Rtp,
            };
            if let Some(existing) = self.bot_positions.iter_mut().find(|b| b.bot_id == bot_id && b.bot_type == bot_type)
            {
                existing.cell_id = final_cell;
                existing.pos_mm = pos_mm;
                // The live edge uses the final reported position, so there is no
                // turn to animate — only the settled facing, as playback reads it.
                existing.heading_rad = heading::drawn_heading(loc, None, Some(existing.heading_rad));
            } else {
                self.bot_positions.push(BotMarker {
                    cell_id: final_cell,
                    bot_type,
                    bot_id,
                    pos_mm,
                    heading_rad: heading::drawn_heading(loc, None, None),
                });
            }
        }
    }

    /// Structural rack/relay counts for the loaded scenario, taken from the
    /// per-slice `rack_capacity` / `relay_cells_total` scalars (constant across
    /// slices; the sim's definitions of rack capacity and relay count). Each
    /// is rescanned only until a non-zero value is seen — it never changes
    /// within a run; a new run resets both to 0 (see `sim_lifecycle`), which
    /// re-arms this.
    fn refresh_structural_counts(&mut self) {
        if self.total_rack_slots == 0 {
            self.total_rack_slots = max_scalar(&self.time_series, "rack_capacity");
        }
        if self.total_relay_cells == 0 {
            self.total_relay_cells = max_scalar(&self.time_series, "relay_cells_total");
        }
    }

    /// The run has returned, its recording closed by the end block: the
    /// recording becomes the profile file playback holds, and the results
    /// print as a headless run prints them.
    fn finish_live_run(&mut self) {
        let Some(live) = self.live_run.take_if(|live| live.recording.has_ended()) else {
            self.sim_status = SimStatus::Error("the run returned and its recording has no end block".into());
            return;
        };
        let profile = match live.recording.into_recording() {
            Ok(recording) => recording.profile,
            Err(damage) => {
                self.sim_status = SimStatus::Error(format!("the run's recording cannot be read: {damage}"));
                return;
            }
        };
        print_results(&profile.summary);
        let expected = self.scenario_used.as_ref().and_then(|s| s.total_order_picks()).unwrap_or(0) as u64;
        check_completion(profile.summary.total_picks, expected, &profile.sim_result.stop_reason);
        self.sim_status = SimStatus::Complete(profile.sim_result.clone());
        self.data_source = DataSource::Profile(Box::new(profile));
    }
}
