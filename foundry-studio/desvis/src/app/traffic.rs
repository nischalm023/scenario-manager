//! Select the observed simulation prefix for the map's cumulative traffic view.

use super::DesvisApp;
use crate::types::{HeatmapMode, SimStatus, ViewMode};

impl DesvisApp {
    pub(crate) fn refresh_traffic(&mut self) {
        if self.view_mode != ViewMode::Simulation
            || !matches!(self.heatmap_mode, HeatmapMode::HtmWait | HeatmapMode::HtmSpeed)
        {
            return;
        }
        // A finished run reaches its end; a run still going, or one that
        // failed, as far as the slices received reach.
        let observed = match &self.sim_status {
            SimStatus::Complete(result) => result.sim_duration.as_millis() as u64,
            _ => self.time_series.last().map_or(0, |slice| slice.start_ms + self.slice_width_ms),
        };
        let until = if self.playback_engaged { (self.time_cursor_ms.max(0.0) as u64).min(observed) } else { observed };
        self.traffic.configure(&self.cell_infos);
        self.traffic.update(&self.time_series, self.slice_width_ms, until);
    }
}
