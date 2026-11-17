//! Playback state management: cursor update, bot interpolation, heatmap derivation.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

use desgraph::RobotType;
use dessim::metrics::profile::PerfProfile;
use dessim::observer::time_slice::ToteStateRecord;
use dessim::worldmap;
use dessim::{FidelityMode, PickSequencing};
use desunits::ToteId;

use super::grid_data::derive_heatmap;
use super::live_run::max_scalar;
use super::{DesvisApp, build_grid_data, grid_data, heading, relay_lanes, tote_state};
use crate::entity_panel::EntityPanel;
use crate::metrics_panel::MetricsPanelCache;
use crate::traffic::TrafficCache;
use crate::types::*;
use worldview_floor::easyloop::Placement;
use worldview_floor::rack as rack_geometry;
use worldview_render::{CellIndex, Floor, RenderFrame, Tote, ToteTransit, Underlay};
#[cfg(test)]
use {
    bytemuck::Zeroable,
    worldview_render::{Overlays, Playback, ToteInstance},
};

impl DesvisApp {
    /// Build the playback app from a run model read out of `recording`.
    ///
    /// Every way this fails is something wrong inside the file — a world
    /// document that will not parse, conveyor geometry that does not fit its
    /// machine, a map with no bounds — so each message names the recording it
    /// came out of rather than stopping the program where nobody can see which
    /// file was at fault.
    pub fn from_profile(profile: PerfProfile, recording: &Path) -> Result<Self, String> {
        let named = |what: &str, why: String| format!("{}: {what}: {why}", recording.display());
        // The recording embeds the world document verbatim, so the loader
        // reads the bytes it carries and no file on this machine.
        let world = worldmap::load_from_bytes(&profile.documents.world)
            .map_err(|e| named("the embedded world document will not load", e.to_string()))?;
        // The ONE compile: the same code, over the same document, that the
        // Crucible server runs on a browser's behalf. Whatever a browser draws
        // from this recording, desvis draws here.
        let compiled = worldview_floor::compile(&world).map_err(|e| named("its floor", e.to_string()))?;
        let map = world.map;
        let bounds = map.grid_bounds.ok_or_else(|| named("its map states no grid bounds", String::new()))?;
        let is_relay = !map.relays.is_empty();
        let num_stations = map.stations.len();

        let cells = compiled.floor.cells.clone();
        // Built here because the compiled map is only in scope during
        // construction.
        let cell_infos = build_grid_data(&map, &cells);
        let cell_index = CellIndex::build(&cells);
        let frame = RenderFrame::from_tiles(&cells);
        let drawn = compiled.floor.into_scene();
        let (station_metas, station_by_cell) = grid_data::build_station_metas(&map);
        let (relay_metas, relay_by_cell) = grid_data::build_relay_metas(&map);

        // The overlay a recording opens on is the run total, from the counters
        // the observer kept for the whole run.
        let heatmap_data = grid_data::dynamic_grid_to_heatmap(&profile.cumulative_grid, cell_infos.len());

        let cell_size_mm = bounds.cell_size_mm;
        let time_series = profile.time_slices.clone();
        // Structural rack/relay counts from the loaded slices (constant across
        // real slices; 0 on rackless/relayless scenarios). Gate metric
        // availability, mirroring the live path's `refresh_structural_counts`.
        let total_rack_slots = max_scalar(&time_series, "rack_capacity");
        let total_relay_cells = max_scalar(&time_series, "relay_cells_total");
        let slice_width_ms = profile.metadata.slice_width_ms;
        let relay_zones = relay_lanes::relay_zones(&map);
        let relay_lanes = relay_lanes::relay_lanes(&time_series, slice_width_ms, &relay_zones);
        let map_label = format!("{} (playback)", profile.name);
        let base_config = profile.sim_config.clone();
        let vtm_count = map.vtm_count();

        Ok(Self {
            drawn,
            cell_infos,
            cell_index,
            cell_size_mm,
            frame,
            frame_times: VecDeque::new(),
            view_mode: ViewMode::Simulation,
            heatmap_mode: HeatmapMode::Combined,
            sim_status: SimStatus::Complete(profile.sim_result.clone()),
            heatmap_data: Some(heatmap_data),
            traffic: TrafficCache::default(),
            bot_positions: Vec::new(),
            vtm_access_cells: Vec::new(),
            htm_access_cells: Vec::new(),
            selected_resource: None,
            last_click_cell: None,
            station_metas,
            station_by_cell,
            relay_metas,
            relay_by_cell,
            // A profile embeds the document it ran on, so playback browses
            // the same roll a live session does.
            entity_panel: EntityPanel::from_census(world.entities, Some(world.world.clone())),
            station_diag_store: None,
            resource_timeline_cache: None,
            data_source: DataSource::Profile(Box::new(profile)),
            bottom_panel_view: BottomPanelView::Metrics,
            metrics_view: MetricsView::TimeSeries,
            metrics_panel_state: MetricsPanelState::default(),
            metrics_panel_cache: MetricsPanelCache::default(),
            bottom_panel_height: 0.0,
            gantt_window_ms: None,
            gantt_scroll_x: 0.0,
            num_stations,
            total_rack_slots,
            total_relay_cells,
            selected_metrics: vec![MetricKind::PicksPerHourRate, MetricKind::PutawaysPerHourRate],
            time_cursor_ms: 0.0,
            time_series,
            slice_width_ms,
            relay_lanes,
            relay_zones,
            chart_cache: None,
            rolling_config: MetricKind::all().iter().map(|&k| (k, k.default_rolling())).collect(),
            sidebar_charts: MetricKind::featured().to_vec(),
            more_charts_open: false,
            more_charts_filter: String::new(),
            _is_relay: is_relay,
            scenario_used: None,
            scenario_summary: None,
            // Profiles carry no scenario; live runs are not startable here.
            base_scenario: None,
            base_config,
            vtm_count,
            map_label,
            title_label: None,
            run_preparation: None,
            live_run: None,
            sim_start_wall: None,
            time_slices: Vec::new(),
            current_heatmap_slice_idx: None,
            gpu_camera: None,
            gpu_initialized: false,
            num_threads: 1,
            auto_run: false,
            test_mode: false,
            _allow_incomplete: true,
            test_complete_at: None,
            time_slice_ms: slice_width_ms,
            xlsx_path: None,
            htm_only: false,
            ignore_hour_gate: false,
            pick_sequencing: PickSequencing::default(),
            fidelity: FidelityMode::Ghost,
            playback_active: false,
            playback_wall_start: None,
            playback_sim_start_ms: 0.0,
            playback_speed: 1.0,
            playback_reverse: false,
            playback_engaged: true,
            #[cfg(debug_assertions)]
            debug_frame_count: 0,
        })
    }

    /// The container rows the renderer would draw at the cursor — what a test
    /// about the picture asks for, through the same code the app draws with.
    #[cfg(test)]
    pub(super) fn tote_instances_at_cursor(&self) -> Vec<ToteInstance> {
        let (totes, transits) = self.totes_at_cursor();
        let robots = self.highlighted_robots(&Default::default());
        self.drawn
            .moment(
                Zeroable::zeroed(),
                None,
                &Playback {
                    at_ms: self.resource_cursor().as_ms(),
                    robots: &robots,
                    totes: &totes,
                    transits: &transits,
                    overlays: Overlays::default(),
                },
            )
            .tote_instances
    }

    /// The containers at the cursor, as the renderer takes them: where each
    /// one is, what it looks like, and the conveyor travels under way.
    ///
    /// Reading a container's place and its contents out of the slice records
    /// is desvis's; placing it, and moving it along a belt it is part-way
    /// down, is the renderer's — which is how the browser viewer gets the same
    /// picture without a line of this.
    pub(crate) fn totes_at_cursor(&self) -> (Vec<Tote>, Vec<ToteTransit>) {
        if self.time_series.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let cursor_ms = self.resource_cursor().as_ms();
        let slice_idx = (cursor_ms / self.slice_width_ms) as usize;
        let slice_idx = slice_idx.min(self.time_series.len().saturating_sub(1));
        let record = &self.time_series[slice_idx];
        let tote = &record.tote_state;
        let appearances = tote_state::appearances_at(
            self.time_series.iter().map(|record| (record.start_ms, &record.tote_state)),
            cursor_ms,
        );
        let totes = ToteStateRecord::positions_at(
            self.time_series.iter().map(|record| (record.start_ms, &record.tote_state)),
            cursor_ms,
        )
        .iter()
        .map(|(id, state)| Tote {
            id: ToteId(*id),
            at: tote_state::located(state),
            appearance: appearances.get(id).copied().unwrap_or_default(),
        })
        .collect();
        (totes, tote.transits.iter().map(tote_state::travelling).collect())
    }

    /// Start (or redirect) realtime playback from the current cursor.
    /// `reverse = true` plays backward toward t=0. Clicking the opposite
    /// direction mid-play lands here too — the re-anchor makes the
    /// direction flip seamless, with no cursor jump.
    pub(crate) fn start_playback(&mut self, reverse: bool) {
        self.playback_active = true;
        self.playback_reverse = reverse;
        self.playback_engaged = true;
        self.playback_wall_start = Some(Instant::now());
        self.playback_sim_start_ms = self.time_cursor_ms;
        self.view_mode = ViewMode::State;
        self.heatmap_mode = HeatmapMode::Off;
    }

    /// Move the time cursor in response to a user interaction — a click/drag on
    /// a time-series chart or a Gantt lane. Clamps to `[0, max_ms]`, engages
    /// cursor scrubbing, re-anchors any active playback so it keeps advancing
    /// from the new cursor (rather than warping back to the old anchor next
    /// frame), and refreshes the grid view to the new time.
    pub(crate) fn engage_cursor(&mut self, new_cursor_ms: f64, max_ms: f64) {
        self.time_cursor_ms = new_cursor_ms.clamp(0.0, max_ms);
        self.playback_engaged = true;
        if self.playback_active {
            self.playback_wall_start = Some(Instant::now());
            self.playback_sim_start_ms = self.time_cursor_ms;
        }
        self.update_grid_for_cursor();
    }

    /// Update heatmap and bot positions for the current cursor position.
    ///
    /// Derives heatmap from `bot_locations` on the corresponding `SliceRecord`.
    /// Works for both live sim and profile playback modes.
    ///
    /// When `playback_active`, bot positions are interpolated within the slice
    /// using step timing data, so the "skip if same slice" optimisation is
    /// bypassed for the bot-position portion.
    pub(crate) fn update_grid_for_cursor(&mut self) {
        if self.time_series.is_empty() {
            return;
        }
        let slice_idx = (self.time_cursor_ms / self.slice_width_ms as f64) as usize;
        let slice_idx = slice_idx.min(self.time_series.len().saturating_sub(1));

        let slice_changed = self.current_heatmap_slice_idx != Some(slice_idx);

        // Heatmap: rebuild only on slice crossing. The slice's overlay is
        // counted off the step cells, which is what desvis has always drawn. A
        // recording also carries the observer's own per-slice visit
        // increments; nothing here reads them — they are for the browser in
        // phase 2, and drawing them would change this picture.
        if slice_changed {
            self.current_heatmap_slice_idx = Some(slice_idx);

            let record = &self.time_series[slice_idx];
            if record.bot_locations.is_empty() {
                // A live slice that carried no robot: fall back to the counts
                // the live path collected. A recording leaves this list empty.
                if !self.time_slices.is_empty() {
                    let legacy_idx = slice_idx.min(self.time_slices.len().saturating_sub(1));
                    let slice = &self.time_slices[legacy_idx];
                    self.heatmap_data = Some(HeatmapData {
                        vtm_counts: slice.vtm_counts.clone(),
                        htm_counts: slice.htm_counts.clone(),
                        vtm_max: slice.vtm_max,
                        htm_max: slice.htm_max,
                    });
                }
            } else {
                self.heatmap_data = Some(derive_heatmap(&record.bot_locations, self.cell_infos.len()));
            }
        }

        // Bot positions: during playback, interpolate within slice every frame.
        // Without playback, only update on slice crossing.
        if !slice_changed && !self.playback_active {
            return;
        }

        let record = &self.time_series[slice_idx];
        if record.bot_locations.is_empty() {
            return;
        }

        // Offset within this slice (milliseconds).
        let slice_start_ms = record.start_ms as f64;
        let offset_ms = self.time_cursor_ms - slice_start_ms;

        for (&bot_id, loc) in &record.bot_locations {
            let bot_type = match loc.robot_type {
                RobotType::Vtm => BotType::Vtm,
                RobotType::Htm => BotType::Htm,
                RobotType::Rtp => BotType::Rtp,
            };
            // UI history is only a fallback for older slices without a recorded
            // start heading; seeking must use the destination slice's facts.
            let prior_heading =
                self.bot_positions.iter().find(|b| b.bot_id == bot_id && b.bot_type == bot_type).map(|b| b.heading_rad);
            let (cell, pos_mm, heading) = if self.playback_active && !loc.steps.is_empty() {
                // Walk through steps to find the active step at offset_ms.
                let mut prev_pos = loc.start_pos;
                let mut elapsed: f64 = 0.0;
                let mut interpolated_pos: Option<[f64; 2]> = None;
                let mut found_cell = loc.start_cell;
                let mut prev_cell = loc.start_cell;
                let mut active_step: Option<(usize, f32)> = None;

                for (step_idx, s) in loc.steps.iter().enumerate() {
                    let step_start = elapsed;
                    elapsed += s.delta_ms as f64;
                    if elapsed > offset_ms && s.delta_ms > 0 {
                        let progress = ((offset_ms - step_start) / s.delta_ms as f64).clamp(0.0, 1.0) as f32;
                        active_step = Some((step_idx, progress));
                        // Interpolate the millimetres the sim actually
                        // reported. The renderer draws mm directly, so there is
                        // no index to convert through — and a mm lerp is
                        // linear in real space, which an index lerp is not
                        // wherever two adjacent rows have different pitch.
                        let lerp = |a: f64, b: f64| a + (b - a) * progress as f64;
                        interpolated_pos = Some([lerp(prev_pos.x_mm, s.pos.x_mm), lerp(prev_pos.y_mm, s.pos.y_mm)]);
                        found_cell = prev_cell;
                        break;
                    }
                    prev_pos = s.pos;
                    prev_cell = s.cell_id;
                    found_cell = s.cell_id;
                }
                // A cursor past every step means the bot has settled; otherwise
                // read the facing out of the step it is partway through, which
                // is what turns a `Rotate` into a visible turn.
                let heading = heading::drawn_heading(loc, active_step, prior_heading);
                (found_cell, interpolated_pos.unwrap_or([prev_pos.x_mm, prev_pos.y_mm]), heading)
            } else {
                let cell = loc.steps.last().map(|s| s.cell_id).unwrap_or(loc.start_cell);
                let at = loc.steps.last().map_or(loc.start_pos, |step| step.pos);
                (cell, [at.x_mm, at.y_mm], heading::drawn_heading(loc, None, prior_heading))
            };

            // Reported coordinates remain authoritative even while stopped;
            // a VTM's reporting anchor can be away from its actual position.
            if let Some(existing) = self.bot_positions.iter_mut().find(|b| b.bot_id == bot_id && b.bot_type == bot_type)
            {
                existing.cell_id = cell;
                existing.pos_mm = pos_mm;
                existing.heading_rad = heading;
            } else {
                self.bot_positions.push(BotMarker { cell_id: cell, bot_type, bot_id, pos_mm, heading_rad: heading });
            }
        }
    }
}

#[cfg(test)]
mod tote_tests {
    use desgraph::{RobotType, WorldPos};
    use dessim::metrics::profile::{SliceRecord, grid_to_dynamic, scalars_to_map};
    use dessim::observer::time_slice::{BotLocation, BotStep, BotStepKind, TimeSliceData, ToteVisualState};

    use crate::app::DesvisApp;

    /// Robot 7, an HTM, driving from cell 0 to `destination` in the slice from
    /// `start_ms`, as the run model holds the slice.
    fn moving_slice(start_ms: u64, destination: u32, edit: impl FnOnce(&mut TimeSliceData)) -> SliceRecord {
        let mut slice = TimeSliceData { start_ms, ..Default::default() };
        slice.bot_locations.insert(
            7,
            BotLocation {
                motion: Some(Vec::new()),
                start_cell: 0,
                start_pos: WorldPos::default(),
                start_z_mm: 0.0,
                start_toward: None,
                start_heading_rad: None,
                robot_type: RobotType::Htm,
                steps: vec![BotStep::whole(
                    destination,
                    WorldPos { x_mm: destination as f64 * 1000.0, y_mm: 0.0 },
                    1000,
                    BotStepKind::Move,
                )]
                .into(),
            },
        );
        edit(&mut slice);
        SliceRecord {
            start_ms: slice.start_ms,
            metrics: scalars_to_map(&slice.scalars),
            grid: grid_to_dynamic(&slice.grid),
            bot_locations: slice.bot_locations,
            relay_state: slice.relay_state,
            tote_state: slice.tote_state,
            station_diag: slice.station_diag,
        }
    }

    /// With no cursor engaged a container is drawn as the newest slice leaves
    /// it — on the robot that picked it up from its relay, with the unit
    /// inducted into it — and scrubbing back to before the pickup draws it
    /// empty at the relay again.
    #[test]
    fn containers_are_drawn_at_the_live_edge_and_where_the_cursor_scrubs_to() {
        let mut app = DesvisApp::for_tests(4);
        let mut rack = app.drawn.rack().clone();
        rack.relay_tote_pos.insert(1, [1.0, 0.0, 0.5]);
        app.drawn = std::mem::take(&mut app.drawn).with_rack(rack);
        app.time_series = vec![
            moving_slice(0, 1, |slice| {
                slice.tote_state.start_states.push((42, ToteVisualState::AtRelay { cell_id: 1 }));
                slice.tote_state.contents.push((0, 42, 0));
            }),
            moving_slice(1000, 3, |slice| {
                slice.tote_state.carries_previous = true;
                slice.tote_state.transitions.push((100, 42, ToteVisualState::OnBot { bot_id: 7 }));
                slice.tote_state.contents.push((500, 42, 1));
            }),
        ];
        app.stand_robots_at_live_edge();

        let totes = app.tote_instances_at_cursor();
        assert_eq!(totes.len(), 1);
        assert_eq!(totes[0].tote_type, 3, "the tote rides its HTM after relay pickup");
        assert_eq!(totes[0].world_x, 3.0, "the tote and its HTM share a position");
        assert_eq!(totes[0].contents_units, 1, "the inducted unit is visible");

        app.engage_cursor(500.0, 2000.0);
        let totes = app.tote_instances_at_cursor();
        assert_eq!(totes[0].tote_type, 1, "scrubbing before pickup restores the relay tote");
        assert_eq!(totes[0].world_x, 1.0);
        assert_eq!(totes[0].contents_units, 0, "scrubbing before induction restores the empty tote");
    }
}
