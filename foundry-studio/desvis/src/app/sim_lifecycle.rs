//! Within `app`, `sim_lifecycle` owns the RUN BOUNDARY: preparing a headed
//! run as desvis headless prepares one, starting it on a thread of its own
//! with its recording tapped to the window (`live_run` reads the bytes), and
//! resetting back to idle.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use dessim::harness::{HarnessError, PreparedRun, RecordingTarget, RunOverrides, prepare, prepare_world};
use dessim::runner::{RunnerHooks, RunnerLiveHandles};
use dessim::{SimConfig, StopSignal};
use workbook::{XlsxData, write_xlsx};

use super::DesvisApp;
use super::live_run::{FromRun, LiveRun, RunTap, UtilizationSeen};
use crate::output::recording_path;
use crate::traffic::TrafficCache;
use crate::types::*;

/// The file desvis was launched with, which every run its window starts is
/// prepared from.
#[derive(Clone)]
pub(crate) enum LaunchFile {
    /// A world spec: the world document it names, its scenario and settings.
    Spec(PathBuf),
    /// A world document with no spec beside it.
    World(PathBuf),
}

/// How every run a window starts is prepared: from the file desvis was
/// launched with, under the flags it was launched with, through the harness
/// desvis headless runs through.
#[derive(Clone)]
pub(crate) struct RunPreparation {
    pub(crate) file: LaunchFile,
    /// The flags whose effect lies outside the configuration, which the window
    /// settles itself (`base_config` under the sidebar's knobs): the order cap
    /// and `--settings` shape the scenario, `--events` the event log.
    pub(crate) overrides: RunOverrides,
    /// Where each run's recording is written, under the name headless gives
    /// it.
    pub(crate) output_dir: PathBuf,
}

impl RunPreparation {
    fn prepare(&self) -> Result<PreparedRun, HarnessError> {
        match &self.file {
            LaunchFile::Spec(path) => prepare(path, &self.overrides),
            LaunchFile::World(path) => prepare_world(path, &self.overrides),
        }
    }
}

impl DesvisApp {
    pub(crate) fn start_simulation(&mut self) {
        if matches!(self.sim_status, SimStatus::Running) {
            return;
        }
        let Some(preparation) = self.run_preparation.clone() else {
            self.sim_status = SimStatus::Error("This window plays a recording back and starts no run".into());
            return;
        };
        // Durable source of truth, not `scenario_used` -- that one is a per-run
        // record and Reset clears it. No `unwrap_or_default` here: silently
        // swapping in the generated default runs a different workload under the
        // same banner (see the Reset/Run divergence bug).
        let Some(scenario) = self.base_scenario.clone() else {
            self.sim_status = SimStatus::Error("No scenario available".into());
            return;
        };

        self.sim_status = SimStatus::Running;
        self.data_source = DataSource::Live;
        self.time_series.clear();
        // Re-arm rack/relay availability detection for the new run's slices.
        self.total_rack_slots = 0;
        self.total_relay_cells = 0;
        self.relay_lanes = None;
        self.time_slices.clear();
        self.bot_positions.clear();
        self.heatmap_data = None;
        self.traffic = TrafficCache::default();
        self.chart_cache = None;
        self.station_diag_store = None;
        self.resource_timeline_cache = None;
        self.selected_resource = None;
        self.metrics_panel_cache.clear();
        self.current_heatmap_slice_idx = None;
        self.time_cursor_ms = 0.0;
        self.playback_active = false;
        self.playback_reverse = false;
        self.playback_engaged = false;
        self.playback_wall_start = None;
        self.playback_sim_start_ms = 0.0;
        self.sim_start_wall = Some(Instant::now());

        // Per-run record of what this run consumed.
        self.scenario_used = Some(scenario);

        let mut config = self.effective_config();
        // The PRD_HR hour gate is a load-time putaway knob; overlay the
        // sidebar toggle onto the config before the solver builds the pool.
        config.putaway.ignore_hour_gate = self.ignore_hour_gate;
        // Pick sequencing is a pick-demand knob; overlay the sidebar selection
        // before the solver builds the demand modules.
        config.pick.sequencing = self.pick_sequencing;
        // desvis renders per-bot Gantt timelines; the observer must record
        // them. Set after `effective_config` because the title bar should
        // reflect the user-visible knobs, not the desvis-internal overlay.
        config.record_robot_timelines = true;

        // Positive confirmation that map-file + CLI + UI overlays
        // propagated into the actual SimConfig that the solver will see.
        crate::sim_config_display::print_summary(&config, self.vtm_count, &self.map_label, self.title_label.as_deref());

        let stop = StopSignal::new();
        let (to_window, from_run) = mpsc::channel();
        let station_types = self.station_metas.iter().map(|station| (station.id, station.station_type)).collect();
        self.live_run = Some(LiveRun::new(from_run, stop.clone(), UtilizationSeen::new(station_types)));
        let xlsx = self.xlsx_path.clone();
        thread::spawn(move || {
            let tap = RunTap(to_window.clone());
            let ran =
                panic::catch_unwind(AssertUnwindSafe(|| run_headed(&preparation, config, stop, tap, xlsx.as_deref())));
            let outcome = ran.unwrap_or_else(|panic| Err(panic_words(panic.as_ref())));
            // A window that has stopped watching has no one to tell.
            let _ = to_window.send(FromRun::Ended(outcome));
        });

        self.view_mode = ViewMode::Simulation;
        if self.heatmap_mode == HeatmapMode::Off {
            self.heatmap_mode = HeatmapMode::Combined;
        }
    }

    pub(crate) fn reset_simulation(&mut self) {
        self.sim_status = SimStatus::Idle;
        self.data_source = DataSource::None;
        self.heatmap_data = None;
        self.traffic = TrafficCache::default();
        self.live_run = None;
        self.sim_start_wall = None;
        self.scenario_used = None;
        self.time_slices.clear();
        self.time_series.clear();
        self.total_rack_slots = 0;
        self.total_relay_cells = 0;
        self.relay_lanes = None;
        self.metrics_panel_cache.clear();
        self.bot_positions.clear();
        self.time_cursor_ms = 0.0;
        self.playback_active = false;
        self.playback_reverse = false;
        self.playback_engaged = false;
        self.playback_wall_start = None;
        self.playback_sim_start_ms = 0.0;
        self.chart_cache = None;
        self.station_diag_store = None;
        self.resource_timeline_cache = None;
        self.selected_resource = None;
        self.current_heatmap_slice_idx = None;
        self.view_mode = ViewMode::State;
        self.heatmap_mode = HeatmapMode::Off;
    }
}

/// Prepare the run, execute it with its recording tapped to the window, and
/// write the workbook from the profile it returns, as headless writes it.
fn run_headed(
    preparation: &RunPreparation,
    config: SimConfig,
    stop: Arc<StopSignal>,
    tap: RunTap,
    xlsx: Option<&Path>,
) -> Result<(), String> {
    let mut prepared = preparation.prepare().map_err(|e| e.to_string())?;
    // The run takes the window's configuration; `execute` resolves the
    // scenario against it.
    prepared.config = config;
    let path = recording_path(prepared.identity(), &preparation.output_dir);
    let target = RecordingTarget { path, run: String::new(), attempt: 0, tap: Some(Box::new(tap)) };
    // The window reads a run's progress off its frames, so it hands the
    // runner its stop signal alone.
    let live = RunnerLiveHandles { counters: None, stop_signal: Some(stop) };
    let hooks = RunnerHooks { live: Some(live), ..RunnerHooks::default() };
    let (profile, _) = prepared.execute(hooks, Some(target)).map_err(|e| e.to_string())?;
    if let Some(xlsx) = xlsx
        && let Err(e) = write_xlsx(xlsx, &XlsxData::from_profile(&profile))
    {
        eprintln!("xlsx export failed: {e}");
    }
    Ok(())
}

/// What a panic said, for the window's error line.
fn panic_words(panic: &(dyn Any + Send)) -> String {
    match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
        (Some(words), _) => format!("the run panicked: {words}"),
        (_, Some(words)) => format!("the run panicked: {words}"),
        _ => "the run panicked".into(),
    }
}
