//! `DesvisApp` struct, constructors, and module organization.

mod central_panel;
mod frame;
pub(crate) mod grid_data;
mod heading;
mod inspector;
mod live_run;
mod path_overlay;
mod playback;
mod relay_lanes;
mod render_callback;
pub(crate) mod render_dump;
mod selection;
mod sim_lifecycle;
mod tote_state;
mod traffic;

use desmap::OutboundRole;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use desgraph::{StationType, ZoneId};
use desmap::WarehouseMap;
use dessim::metrics::profile::SliceRecord;
use dessim::model::SimTime;
use dessim::model::relay::RelayUtilizationTimeline;
use dessim::observer::time_slice::BotLocation;
use dessim::{FidelityMode, PickSequencing, ScenarioConfig, ScenarioSummary, SimConfig};
use rustc_hash::FxHashMap;

use self::inspector::ResourceTimeline;
use self::live_run::LiveRun;
use crate::carton_line_prop;
use crate::entity_panel::EntityPanel;
use crate::graph::ChartCache;
use crate::metrics_panel::MetricsPanelCache;
use crate::station_diag::StationDiagStore;
use crate::traffic::TrafficCache;
use crate::types::*;
use worldview_render::{CameraState, CellIndex, RenderFrame};
#[cfg(test)]
use worldview_render::{Floor, RackVisual};

pub use grid_data::build_grid_data;
pub(crate) use sim_lifecycle::{LaunchFile, RunPreparation};

pub struct DesvisApp {
    // Grid metadata.
    pub(crate) cell_infos: Vec<Option<CellInfo>>,
    /// Spatial picking index over cell tiles: which cell covers a millimetre
    /// point. Rebuilt whenever `cell_infos` is.
    pub(crate) cell_index: CellIndex,
    /// Nominal cell pitch in millimetres. Display and the reference-image
    /// underlay only — anything positional goes through `frame`, which carries
    /// the map's real per-axis geometry.
    pub(crate) cell_size_mm: f64,
    /// The renderer's coordinate system for this map: the millimetres-to-
    /// metres rescale everything drawn or picked goes through. The same one
    /// `drawn` is framed in, kept beside it because every panel asks.
    pub(crate) frame: RenderFrame,

    // View state.
    pub(crate) view_mode: ViewMode,
    pub(crate) heatmap_mode: HeatmapMode,
    pub(crate) sim_status: SimStatus,
    pub(crate) heatmap_data: Option<HeatmapData>,
    pub(crate) traffic: TrafficCache,

    // State view markers.
    pub(crate) bot_positions: Vec<BotMarker>,
    pub(crate) vtm_access_cells: Vec<VtmAccessMarker>,
    pub(crate) htm_access_cells: Vec<HtmAccessMarker>,

    // Resource selection + station inspector.
    /// Station/bot selected by a single click in the State view. Drives the
    /// inspector window and the selection highlight.
    pub(crate) selected_resource: Option<SelectedResource>,
    /// Grid cell of the previous click, so a repeat click on the same cell
    /// cycles through its co-located candidates (bot ↔ station ↔ relay).
    pub(crate) last_click_cell: Option<usize>,
    /// Static per-station facts (id, type, cells) for click resolution and
    /// the inspector header. Built from the map at construction.
    pub(crate) station_metas: Vec<StationMeta>,
    /// Cell id → index into `station_metas` for every station-owned cell.
    pub(crate) station_by_cell: FxHashMap<u32, usize>,
    /// Static per-relay facts for click resolution and the relay inspector.
    pub(crate) relay_metas: Vec<RelayMeta>,
    /// Cell id → index into `relay_metas` for every relay-owned cell
    /// (position, htm-access, vtm-access).
    pub(crate) relay_by_cell: HashMap<u32, usize>,
    /// The loaded document's entities, browsable, and the outline the map
    /// draws for the row the pointer is on. Empty when the session has no
    /// world document behind it.
    pub(crate) entity_panel: EntityPanel,
    /// Lazily built store over the time series' station_diag payloads.
    /// Invalidated (set to `None`) whenever `time_series` changes, alongside
    /// `chart_cache`.
    pub(crate) station_diag_store: Option<StationDiagStore>,
    /// Cached full-run timeline extraction for the selected resource's
    /// inspector strip. Rebuilt when the selection changes or the store is
    /// invalidated (same seam as `station_diag_store`), so per-frame strip
    /// draws don't re-extract over the whole run. `None` = not yet built.
    pub(crate) resource_timeline_cache: Option<ResourceTimeline>,

    // Data source.
    pub(crate) data_source: DataSource,

    /// Which view the bottom panel renders (Metrics vs Utilization). Persisted on
    /// the app so the choice survives across sim runs.
    pub(crate) bottom_panel_view: BottomPanelView,

    /// Metric family selected within the Metrics view.
    pub(crate) metrics_view: MetricsView,

    /// Scope, time aggregation, and family-specific selectors for the
    /// canonical Metrics reports. These are presentation choices only; all
    /// views are derived from the same report definitions as XLSX export.
    pub(crate) metrics_panel_state: MetricsPanelState,

    /// Run-local typed report cache. Canonical reports can scan long event
    /// journals, so selectors invalidate this cache instead of recomputing
    /// the active view on every rendered frame.
    pub(crate) metrics_panel_cache: MetricsPanelCache,

    /// User-dragged height of the bottom Metrics/Utilization panel. Stored
    /// on the app so it survives hide/show cycles — when the time-series
    /// momentarily becomes empty (e.g. start of a sim) the panel hides;
    /// without our own copy egui forgets the drag and snaps back to the
    /// default on next show.
    pub(crate) bottom_panel_height: f32,

    /// Visible time window for the Gantt view, in ms. `None` = fit-to-window
    /// (the whole run; auto-scales as a live sim grows). `Some(ms)` = a window
    /// pinned at that span, which stays put as the run grows so zooming in to
    /// inspect detail doesn't drift. Clamped to [30s, full run] at use.
    /// Persisted across frames.
    pub(crate) gantt_window_ms: Option<f64>,

    /// Shared horizontal scroll offset (px) linking the three Gantt sections
    /// (Stations / DA50 / D30) so they pan together — scrolling one keeps the
    /// same time window in view across all fleets. `0.0` in the fit-to-window
    /// view (nothing to scroll).
    pub(crate) gantt_scroll_x: f32,

    // Station count (for pres/stn/hr metric).
    pub(crate) num_stations: usize,

    // Structural rack capacity for the loaded scenario, read from the
    // per-slice `rack_capacity` scalar (the sim's single definition of rack
    // capacity; constant across slices, 0 on rackless scenarios). Used only
    // to decide whether rack metrics are structurally available — the fill %
    // denominator still comes straight from the slice. 0 until the first
    // slice arrives, so rack metrics read as unavailable until then, which is
    // correct: no rack data has been observed yet. Refreshed on ingestion by
    // `refresh_structural_counts`.
    pub(crate) total_rack_slots: u32,

    // Structural relay-cell count, read from the per-slice `relay_cells_total`
    // scalar (constant across slices, 0 on relayless scenarios). Gates relay-
    // occupancy metric availability, mirroring `total_rack_slots`.
    pub(crate) total_relay_cells: u32,

    // Multi-line chart state.
    pub(crate) selected_metrics: Vec<MetricKind>,
    pub(crate) time_cursor_ms: f64,
    pub(crate) time_series: Vec<SliceRecord>,
    pub(crate) slice_width_ms: u64,
    /// Each relay zone's busy lane over `time_series`, as the slices state it;
    /// `None` when they do not.
    pub(crate) relay_lanes: Option<RelayUtilizationTimeline>,
    /// Each relay zone with its relay count, as the map states them, which a
    /// live run's lanes are read for.
    pub(crate) relay_zones: Vec<(ZoneId, u32)>,
    /// Cached chart plot points. Invalidated when slices or metrics change.
    pub(crate) chart_cache: Option<ChartCache>,
    /// Per-metric rolling-window configuration (smoothing applied at render).
    /// Pre-populated for every `MetricKind` so reads can index unconditionally.
    pub(crate) rolling_config: HashMap<MetricKind, RollingConfig>,
    /// Metrics shown in the sidebar's Charts list. A row's checkbox here
    /// toggles plot visibility (via `selected_metrics`), and the on-hover
    /// `−` removes the row from the sidebar entirely. Initialized to
    /// `MetricKind::featured()`; users add more via the "+ Add chart"
    /// popup, which pushes onto this Vec.
    ///
    /// Invariant: every plotted metric (`selected_metrics`) is in
    /// `sidebar_charts`. Removing a row from the sidebar always also
    /// removes it from `selected_metrics`.
    pub(crate) sidebar_charts: Vec<MetricKind>,
    /// True while the "+ Add chart" popup is open.
    pub(crate) more_charts_open: bool,
    /// Search filter applied to the "+ Add chart" popup. Substring match
    /// against `MetricKind::label()` (case-insensitive).
    pub(crate) more_charts_filter: String,

    // Map + scenario.
    pub(crate) _is_relay: bool,
    /// Scenario the *last run* actually consumed. Set by `start_simulation`,
    /// cleared by `reset_simulation`. A per-run record for display only --
    /// never the source of truth for the next run (see `base_scenario`).
    pub(crate) scenario_used: Option<ScenarioConfig>,
    pub(crate) scenario_summary: Option<ScenarioSummary>,

    /// Scenario parsed from the SimSpec, the durable source of truth for every
    /// run. Survives Reset, mirroring `base_config`. `None` only in profile
    /// playback, which carries no scenario; `start_simulation` refuses to run
    /// rather than substituting a default.
    pub(crate) base_scenario: Option<ScenarioConfig>,

    /// Base SimConfig from SimSpec (map JSON settings + CLI overrides).
    /// UI controls (htm_only, num_threads, fidelity, time_slice_ms) are
    /// overlaid at launch time by [`Self::effective_config`].
    pub(crate) base_config: SimConfig,
    /// Resolved VTM fleet size, derived from the map (sum of per-zone
    /// `vtm_count`). VTM count is not a SimConfig parameter — see
    /// `WarehouseMap::vtm_count`.
    pub(crate) vtm_count: usize,
    /// Human-readable map label shown in the window title (file stem of the
    /// loaded map JSON, or "playback" / "generated" for those modes).
    pub(crate) map_label: String,
    /// Optional free-form suffix shown in the window title — e.g. a git
    /// commit short hash for matrix runs. `None` means no suffix.
    pub(crate) title_label: Option<String>,

    /// How this window's runs are prepared. `None` when it plays a recording
    /// back, which starts no run.
    pub(crate) run_preparation: Option<RunPreparation>,
    /// The run being watched, from its start until it has returned; a run
    /// that failed stays, for what it showed before it failed.
    pub(crate) live_run: Option<LiveRun>,

    // Wall-clock tracking.
    pub(crate) sim_start_wall: Option<Instant>,

    // Legacy time slices (for heatmap slider).
    pub(crate) time_slices: Vec<TimeSlice>,
    /// The slice index used for the last heatmap rebuild; used to skip
    /// redundant rebuilds when the cursor moves within the same time slice.
    pub(crate) current_heatmap_slice_idx: Option<usize>,

    // GPU grid renderer state.
    pub(crate) gpu_camera: Option<CameraState>,
    pub(crate) gpu_initialized: bool,
    /// Rack elevation model (beam ladders, envelope top, per-cell shelf
    /// floors) threaded to the grid pipeline and bot/tote instance builders.
    /// The scene the renderer draws: the floor, the hops, the steel and the
    /// ground under them, derived from the map once.
    pub(crate) drawn: DrawnScene,

    // Sim thread count (UI-configurable).
    pub(crate) num_threads: usize,

    // Auto-start simulation on launch (--run or --test).
    pub(crate) auto_run: bool,
    // Test mode: auto-start sim + exit on completion (--test).
    pub(crate) test_mode: bool,
    // Allow incomplete runs without exit(1) (--allow-incomplete).
    pub(crate) _allow_incomplete: bool,
    /// Wall-clock instant when sim completed (for test-mode delayed exit).
    pub(crate) test_complete_at: Option<Instant>,

    // Configurable time-slice width (passed to SimObserver).
    pub(crate) time_slice_ms: u64,

    /// Path for xlsx export on completion (from --xlsx flag).
    pub(crate) xlsx_path: Option<PathBuf>,

    // HTM-only mode toggle (passed to SimConfig).
    pub(crate) htm_only: bool,

    /// PRD_HR hour-gate bypass (bulk-source only). When `true`, putaway
    /// ignores each lot's PRD_HR net-time floor. Mirrors
    /// `config.putaway.ignore_hour_gate`; overlaid back onto it at run start.
    pub(crate) ignore_hour_gate: bool,

    /// Pick line-sequencing mode (binning candidate model). Mirrors
    /// `config.pick.sequencing`; overlaid back onto it at run start.
    pub(crate) pick_sequencing: PickSequencing,

    // Movement fidelity mode (passed to SimConfig).
    pub(crate) fidelity: FidelityMode,

    // 1:1 realtime playback state.
    pub(crate) playback_active: bool,
    pub(crate) playback_wall_start: Option<Instant>,
    pub(crate) playback_sim_start_ms: f64,
    /// Playback speed multiplier (1.0 = realtime, 50.0 = 50x).
    pub(crate) playback_speed: f32,
    /// Direction of active playback: true = cursor moves backward in sim
    /// time, stopping at t=0.
    pub(crate) playback_reverse: bool,
    /// Sticky flag: user has interacted with playback this run (clicked Play
    /// or scrubbed the chart cursor). Polling stops overwriting bot_positions
    /// once engaged; reset on Run/Reset.
    pub(crate) playback_engaged: bool,

    pub(crate) frame_times: VecDeque<f32>,

    // Debug: frame counter for periodic logging.
    #[cfg(debug_assertions)]
    pub(crate) debug_frame_count: u64,
}

impl DesvisApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scene: Scene,
        width: u32,
        depth: u32,
        cell_size_mm: f64,
        map: Option<WarehouseMap>,
        is_relay: bool,
        scenario_summary: Option<ScenarioSummary>,
        auto_run: bool,
        test_mode: bool,
        run_preparation: Option<RunPreparation>,
        allow_incomplete: bool,
        time_slice_ms: u64,
        cli_metrics: Option<Vec<MetricKind>>,
        base_config: SimConfig,
        embedded_scenario: ScenarioConfig,
        xlsx_path: Option<PathBuf>,
        map_label: String,
        title_label: Option<String>,
    ) -> Self {
        let Scene { drawn, cell_infos, easyloops } = scene;
        let cell_index = CellIndex::build(drawn.cells());
        // The frame is the tiles' bounding box; a mapless session frames the
        // stated nominal extent instead.
        let drawn = if map.is_some() {
            drawn
        } else {
            drawn.with_frame(RenderFrame::from_extent_mm(width as f64 * cell_size_mm, depth as f64 * cell_size_mm))
        };
        let frame = drawn.frame().clone();
        let mut rack_visual = drawn.rack().clone();

        // One drawing per machine, using the same declared frame as its cells.
        if let Some(ref m) = map {
            for placement in easyloops {
                placement.add_to_scene(m, &frame, &mut rack_visual);
            }
        }
        if let Some(ref m) = map {
            for station in &m.stations {
                // New handoffs have declared cells; equipment dimensions are
                // not known. Keep the existing eastern dispense visuals only.
                if station.outbound_role.is_some_and(|role| role != OutboundRole::RouteDispense) {
                    continue;
                }
                // One machine, two station types -- see `carton_line_prop`.
                let build = match station.station_type {
                    StationType::Extraction => carton_line_prop::extraction_prop,
                    _ => continue,
                };
                // Induction centres on its station point; extraction aligns
                // its south end to the north edge of the cell the robot
                // presents at, so pass that cell and that edge.
                let at = match station.station_type {
                    // The cell's own tile gives the north edge directly:
                    // `tile_mm` is [x, y, w, h].
                    StationType::Extraction => {
                        let cell = station.delivery_cell();
                        let Some(Some(info)) = cell_infos.get(cell.0 as usize) else { continue };
                        let t = info.tile_mm;
                        [t[0] + t[2] / 2.0, t[1] + t[3]]
                    }
                    _ => {
                        let Some(p) = m.graph.cell(station.position).and_then(|c| c.world_pos) else { continue };
                        [p.x_mm, p.y_mm]
                    }
                };
                rack_visual.frames.extend(build(at, &frame));
            }
        }

        let vtm_access_cells =
            if let Some(ref m) = map { grid_data::compute_vtm_access_markers(m) } else { Vec::new() };

        let htm_access_cells =
            if let Some(ref m) = map { grid_data::compute_htm_access_markers(m) } else { Vec::new() };

        let (station_metas, station_by_cell) = match map {
            Some(ref m) => grid_data::build_station_metas(m),
            None => (Vec::new(), FxHashMap::default()),
        };
        let (relay_metas, relay_by_cell) = match map {
            Some(ref m) => grid_data::build_relay_metas(m),
            None => (Vec::new(), HashMap::new()),
        };

        let num_stations = map.as_ref().map(|m| m.stations.len()).unwrap_or(0);
        let relay_zones = map.as_ref().map(relay_lanes::relay_zones).unwrap_or_default();
        let vtm_count = map.as_ref().map(|m| m.vtm_count()).unwrap_or(0);
        let init_htm_only = base_config.htm_only;
        let init_num_threads = base_config.num_threads;
        let init_fidelity = base_config.fidelity;
        let init_ignore_hour_gate = base_config.putaway.ignore_hour_gate;
        let init_pick_sequencing = base_config.pick.sequencing;

        Self {
            drawn: drawn.with_rack(rack_visual),
            frame,
            cell_infos,
            cell_index,
            cell_size_mm,
            view_mode: ViewMode::State,
            heatmap_mode: HeatmapMode::Off,
            sim_status: SimStatus::Idle,
            heatmap_data: None,
            traffic: TrafficCache::default(),
            bot_positions: Vec::new(),
            vtm_access_cells,
            htm_access_cells,
            selected_resource: None,
            last_click_cell: None,
            station_metas,
            station_by_cell,
            relay_metas,
            relay_by_cell,
            // Filled in by the launcher, which is where the loaded document
            // is still in scope; a session without one keeps the empty roll.
            entity_panel: EntityPanel::default(),
            station_diag_store: None,
            resource_timeline_cache: None,
            data_source: DataSource::None,
            bottom_panel_view: BottomPanelView::Metrics,
            metrics_view: MetricsView::TimeSeries,
            metrics_panel_state: MetricsPanelState::default(),
            metrics_panel_cache: MetricsPanelCache::default(),
            // 0.0 = "uninitialized" sentinel; frame.rs replaces it with
            // 1/3 of the OS-window height on the first frame so two charts
            // fit comfortably without manual resize on a fresh launch.
            bottom_panel_height: 0.0,
            gantt_window_ms: None,
            gantt_scroll_x: 0.0,
            num_stations,
            // 0 until the first slice reports the scalar; refreshed on ingestion.
            total_rack_slots: 0,
            total_relay_cells: 0,
            selected_metrics: cli_metrics
                .unwrap_or_else(|| vec![MetricKind::PicksPerHourRate, MetricKind::PutawaysPerHourRate]),
            time_cursor_ms: 0.0,
            time_series: Vec::new(),
            slice_width_ms: time_slice_ms,
            relay_lanes: None,
            relay_zones,
            chart_cache: None,
            rolling_config: MetricKind::all().iter().map(|&k| (k, k.default_rolling())).collect(),
            sidebar_charts: MetricKind::featured().to_vec(),
            more_charts_open: false,
            more_charts_filter: String::new(),
            _is_relay: is_relay,
            scenario_used: None,
            scenario_summary,
            base_scenario: Some(embedded_scenario),
            base_config,
            vtm_count,
            map_label,
            title_label,
            run_preparation,
            live_run: None,
            sim_start_wall: None,
            time_slices: Vec::new(),
            current_heatmap_slice_idx: None,
            gpu_camera: None,
            gpu_initialized: false,
            num_threads: init_num_threads,
            auto_run,
            test_mode,
            _allow_incomplete: allow_incomplete,
            test_complete_at: None,
            time_slice_ms,
            xlsx_path,
            htm_only: init_htm_only,
            ignore_hour_gate: init_ignore_hour_gate,
            pick_sequencing: init_pick_sequencing,
            fidelity: init_fidelity,
            playback_active: false,
            playback_wall_start: None,
            playback_sim_start_ms: 0.0,
            playback_speed: 1.0,
            playback_reverse: false,
            playback_engaged: false,
            frame_times: VecDeque::new(),
            #[cfg(debug_assertions)]
            debug_frame_count: 0,
        }
    }

    /// A window over `cells` empty cells, with no map, run or recording behind
    /// it, for a test to hand what the window reads.
    #[cfg(test)]
    pub(crate) fn for_tests(cells: usize) -> Self {
        let scene = Scene {
            drawn: DrawnScene::new(Floor::from_tiles(vec![None; cells], 1000.0)).with_rack(RackVisual::uniform(0)),
            cell_infos: vec![None; cells],
            easyloops: Vec::new(),
        };
        Self::new(
            scene,
            cells as u32,
            1,
            1000.0,
            None,
            false,
            None,
            false,
            false,
            None,
            false,
            1000,
            None,
            SimConfig::default(),
            ScenarioConfig::default(),
            None,
            "test".into(),
            None,
        )
    }

    /// The sim time containers, the resource inspector and highlights render
    /// as-of. Follows the scrub cursor once playback is engaged; otherwise
    /// the live edge of the time series (where a live run stands its robots
    /// on the map). Shared by the scene and the inspector so they
    /// never disagree about "now".
    pub(crate) fn resource_cursor(&self) -> SimTime {
        if self.playback_engaged {
            SimTime::ms(self.time_cursor_ms.max(0.0) as u64)
        } else {
            SimTime::ms(self.time_series.last().map(|s| s.start_ms + self.slice_width_ms).unwrap_or(0))
        }
    }

    /// The `BotLocation` for `bot_id` in the slice containing `cursor`, plus
    /// that slice's start time (ms). `None` when the bot has no recorded
    /// movement in that slice (idle/parked, or before it first appears).
    pub(crate) fn bot_location_at(&self, cursor: SimTime, bot_id: u32) -> Option<(&BotLocation, u64)> {
        if self.time_series.is_empty() {
            return None;
        }
        let slice_index = ((cursor.as_ms() / self.slice_width_ms) as usize).min(self.time_series.len() - 1);
        let record = &self.time_series[slice_index];
        record.bot_locations.get(&bot_id).map(|location| (location, record.start_ms))
    }

    /// Build the resource-diagnostics store from the current time series if it
    /// isn't cached, and return it. Invalidated to `None` whenever the time
    /// series changes (see live_run / sim_lifecycle).
    pub(crate) fn ensure_station_diag_store(&mut self) -> &StationDiagStore {
        if self.station_diag_store.is_none() {
            self.station_diag_store =
                Some(StationDiagStore::build(self.time_series.iter().map(|slice| &slice.station_diag)));
        }
        self.station_diag_store.as_ref().expect("just built")
    }

    /// Compute the SimConfig the next run will actually use. In **live**
    /// mode this is `base_config` (which already includes map-file
    /// settings + CLI flags + `--settings` overrides) with the
    /// UI-mutable fields (`num_threads`, `htm_only`, `fidelity`,
    /// `slice_width_ms`) overlaid on top — matching the overlay logic in
    /// [`sim_lifecycle::start_simulation`](super::app::sim_lifecycle).
    /// In **playback** mode (`DataSource::Profile`) it returns the
    /// profile's saved `sim_config` unchanged — the UI overlay fields
    /// don't apply to a prerecorded run.
    pub(crate) fn effective_config(&self) -> SimConfig {
        if let DataSource::Profile(profile) = &self.data_source {
            return profile.sim_config.clone();
        }
        let mut cfg = self.base_config.clone();
        cfg.num_threads = self.num_threads;
        cfg.htm_only = self.htm_only;
        cfg.fidelity = self.fidelity;
        cfg.slice_width_ms = self.time_slice_ms;
        cfg
    }
}
