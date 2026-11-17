//! Cumulative elapsed HTM traffic for the map. Retains one sparse prefix and
//! one current snapshot, not a cell × time × histogram history. Revised slices
//! invalidate the prefix; reverse seeking rebuilds from the recorded facts.

use desgraph::RobotType;
use dessim::metrics::profile::SliceRecord;
use dessim::observer::time_slice::BotStepKind;
use dessim::observer::traffic::{CellTraffic, SPEED_BINS};
use eframe::egui::{self, Color32, Rect, Sense, Vec2};
use rustc_hash::FxHashMap;

use worldview_render::TrafficTints;

use crate::types::{CellInfo, HeatmapMode};

/// The tints the two measured overlays paint with, as the renderer asks for
/// them: the share of a cell's exposure spent waiting, or the mean speed over
/// it, and −1 wherever this run recorded no HTM exposure at all.
impl TrafficTints for TrafficCache {
    fn intensity(&self, cell: u32, mode: HeatmapMode) -> f32 {
        let Some(row) = self.snapshot.get(&cell) else {
            return -1.0;
        };
        match mode {
            HeatmapMode::HtmWait if !self.exclusions.contains_key(&cell) && row.observed_ms > 0.0 => {
                (row.wait_ms / self.max_wait_ms.max(1.0)) as f32
            }
            HeatmapMode::HtmSpeed => row.moving_speed_m_s().map(|s| (s / 5.0).min(1.0) as f32).unwrap_or(-1.0),
            _ => -1.0,
        }
    }
}

#[derive(Default)]
pub(crate) struct TrafficCache {
    prefix: FxHashMap<u32, CellTraffic>,
    prefix_slices: usize,
    snapshot: FxHashMap<u32, CellTraffic>,
    cached_until: Option<u64>,
    pub until_ms: u64,
    pub max_wait_ms: f64,
    exclusions: FxHashMap<u32, &'static str>,
    configured: bool,
    pub legacy_motion: bool,
    prefix_legacy: bool,
}

impl TrafficCache {
    pub fn configure(&mut self, cells: &[Option<CellInfo>]) {
        if self.configured {
            return;
        }
        for cell in cells.iter().flatten() {
            if let Some(reason) = cell.navigation_wait_exclusion {
                self.exclusions.insert(cell.id, reason);
            }
        }
        self.configured = true;
    }

    pub fn invalidate(&mut self, from_slice: usize) {
        if from_slice < self.prefix_slices {
            self.prefix.clear();
            self.prefix_slices = 0;
            self.prefix_legacy = false;
        }
        self.cached_until = None;
    }

    pub fn update(&mut self, slices: &[SliceRecord], width_ms: u64, until_ms: u64) {
        if self.cached_until == Some(until_ms) {
            return;
        }
        let full = ((until_ms / width_ms) as usize).min(slices.len());
        if full < self.prefix_slices {
            self.invalidate(0);
        }
        // Keep the newest two slices outside the prefix so normal live
        // deliveries do not repeatedly rebuild the entire elapsed history.
        let prefix_end = full.saturating_sub(2);
        if prefix_end < self.prefix_slices {
            self.invalidate(0);
        }
        for record in &slices[self.prefix_slices..prefix_end] {
            self.prefix_legacy |= add_slice(record, record.start_ms + width_ms, &mut self.prefix);
        }
        self.prefix_slices = prefix_end;
        self.snapshot.clone_from(&self.prefix);
        self.legacy_motion = self.prefix_legacy;
        for record in &slices[prefix_end..full] {
            self.legacy_motion |= add_slice(record, record.start_ms + width_ms, &mut self.snapshot);
        }
        if let Some(record) = slices.get(full) {
            self.legacy_motion |= add_slice(record, until_ms, &mut self.snapshot);
        }
        self.max_wait_ms = self
            .snapshot
            .iter()
            .filter(|(id, _)| !self.exclusions.contains_key(id))
            .map(|(_, row)| row.wait_ms)
            .fold(0.0, f64::max);
        self.until_ms = until_ms;
        self.cached_until = Some(until_ms);
    }

    pub fn hover(&self, ui: &mut egui::Ui, cell: u32, mode: HeatmapMode) {
        ui.separator();
        ui.label(format!("HTM traffic · 0–{:.1} min", self.until_ms as f64 / 60_000.0));
        if mode == HeatmapMode::HtmWait {
            if let Some(reason) = self.exclusions.get(&cell) {
                ui.label(format!("Excluded from waiting heatmap: {reason}"));
                return;
            }
        }
        let Some(row) = self.snapshot.get(&cell) else {
            ui.label("No recorded HTM exposure");
            return;
        };
        match mode {
            HeatmapMode::HtmWait => {
                ui.label(format!("Stationary waiting: {:.2} HTM-s", row.wait_ms / 1000.0));
                ui.weak("Includes waits for work/resources; not a pure congestion measure.");
                ui.label(format!("Rotation excluded: {:.2} HTM-s", row.rotation_ms / 1000.0));
            }
            HeatmapMode::HtmSpeed => {
                if let Some(speed) = row.moving_speed_m_s() {
                    ui.label(format!("Moving mean: {speed:.2} m/s"));
                    ui.label(format!(
                        "Moving exposure: {:.2} HTM-s · distance: {:.2} m",
                        row.moving_ms / 1000.0,
                        row.distance_mm / 1000.0
                    ));
                    if let Some(bin) = row.median_bin() {
                        ui.label(format!("Median speed lies in {}", bin_label(bin)));
                    }
                    histogram(ui, row);
                    ui.weak("Movement only; stops and rotation excluded.");
                    ui.weak("Link distance split equally between its endpoint cells.");
                    ui.weak("A cumulative distribution does not show when traffic waves overlap.");
                } else {
                    ui.label("No measured moving speed for this cell");
                }
                // Reachable only from a live run now: a `.worldview` whose
                // delta states no motion section is refused by the reader, so
                // a recording always answers what its robots were doing.
                if self.legacy_motion {
                    ui.weak(
                        "This recording lacks canonical motion for some intervals; rerun for complete speed coverage.",
                    );
                }
            }
            _ => {}
        }
    }
}

fn add_slice(record: &SliceRecord, until_ms: u64, cells: &mut FxHashMap<u32, CellTraffic>) -> bool {
    if until_ms <= record.start_ms {
        return false;
    }
    let mut legacy = false;
    let mut bots: Vec<_> = record.bot_locations.iter().collect();
    bots.sort_unstable_by_key(|(id, _)| **id);
    for (_, loc) in bots {
        if loc.robot_type != RobotType::Htm {
            continue;
        }
        if let Some(motion) = &loc.motion {
            for span in motion {
                span.accumulate(record.start_ms, until_ms, cells);
            }
        } else {
            legacy = true;
        }
        let mut time = record.start_ms;
        let mut cell = loc.start_cell;
        for step in &loc.steps {
            let end = time.saturating_add(step.delta_ms);
            let ms = end.min(until_ms).saturating_sub(time) as f64;
            if ms > 0.0 {
                match step.kind {
                    BotStepKind::Wait | BotStepKind::Rotate => {
                        let row = cells.entry(step.cell_id).or_default();
                        row.observed_ms += ms;
                        if step.kind == BotStepKind::Wait {
                            row.wait_ms += ms;
                        } else {
                            row.rotation_ms += ms;
                        }
                    }
                    BotStepKind::Move | BotStepKind::Reverse => {}
                }
            }
            time = end;
            cell = step.cell_id;
            if time >= until_ms {
                break;
            }
        }
        if time < until_ms {
            let row = cells.entry(cell).or_default();
            let ms = (until_ms - time) as f64;
            row.observed_ms += ms;
            row.wait_ms += ms;
        }
    }
    legacy
}

fn bin_label(bin: usize) -> String {
    if bin + 1 == SPEED_BINS {
        "≥5.0 m/s".into()
    } else {
        format!("{:.1}–{:.1} m/s", bin as f64 * 0.5, (bin + 1) as f64 * 0.5)
    }
}

fn histogram(ui: &mut egui::Ui, row: &CellTraffic) {
    ui.label("Share of moving time by speed");
    let max = row.speed_ms.iter().copied().fold(0.0, f64::max).max(1.0);
    ui.horizontal(|ui| {
        for (bin, &ms) in row.speed_ms.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(20.0, 48.0), Sense::hover());
            let height = (ms / max) as f32 * rect.height();
            let bar = Rect::from_min_max(rect.left_bottom() - Vec2::new(0.0, height), rect.right_bottom());
            ui.painter().rect_filled(bar, 0.0, Color32::from_rgb(51, 122, 174));
            response.on_hover_text(format!(
                "{}: {:.1}% ({:.2} HTM-s)",
                bin_label(bin),
                100.0 * ms / row.moving_ms,
                ms / 1000.0
            ));
        }
    });
    ui.weak("0        1        2        3        4        ≥5 m/s");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use desgraph::WorldPos;
    use dessim::observer::time_slice::{BotLocation, BotStep};
    use dessim::observer::traffic::MotionSpan;

    fn record() -> SliceRecord {
        let mut bots = FxHashMap::default();
        bots.insert(
            7,
            BotLocation {
                motion: Some(vec![MotionSpan {
                    source: 0,
                    destination: 1,
                    start_ms: 1000,
                    end_ms: 3000,
                    distance_mm: 2000.0,
                    profile: None,
                }]),
                start_cell: 0,
                start_pos: WorldPos::default(),
                start_z_mm: 0.0,
                start_toward: None,
                start_heading_rad: None,
                robot_type: RobotType::Htm,
                steps: vec![
                    BotStep::whole(0, WorldPos::default(), 1000, BotStepKind::Wait),
                    BotStep::whole(1, WorldPos { x_mm: 2000.0, y_mm: 0.0 }, 2000, BotStepKind::Move),
                    BotStep::whole(1, WorldPos { x_mm: 2000.0, y_mm: 0.0 }, 1000, BotStepKind::Rotate),
                ]
                .into(),
            },
        );
        SliceRecord {
            start_ms: 0,
            metrics: Default::default(),
            grid: Default::default(),
            bot_locations: bots,
            relay_state: Default::default(),
            tote_state: Default::default(),
            station_diag: Default::default(),
        }
    }

    #[test]
    fn cursor_clipping_repeated_updates_and_reverse_seek_agree() {
        let slices = [record()];
        let mut cache = TrafficCache::default();
        cache.update(&slices, 4000, 2500);
        assert_eq!(cache.snapshot[&0].wait_ms, 1000.0);
        assert_eq!(cache.snapshot[&1].moving_ms, 500.0);
        assert_eq!(cache.snapshot[&1].rotation_ms, 0.0);
        cache.update(&slices, 4000, 2500);
        assert_eq!(cache.snapshot[&0].wait_ms, 1000.0);
        cache.update(&slices, 4000, 4000);
        assert_eq!(cache.snapshot[&1].rotation_ms, 1000.0);
        assert_eq!(cache.snapshot[&1].wait_ms, 0.0);
        cache.update(&slices, 4000, 2500);
        assert_eq!(cache.snapshot[&1].moving_ms, 500.0);
        assert_eq!(cache.snapshot[&1].rotation_ms, 0.0);
    }

    #[test]
    fn later_delivered_navigation_refreshes_cached_idle_snapshot_and_excludes_relay_wait() {
        let mut slices = [record()];
        let mut cache = TrafficCache::default();
        cache.exclusions.insert(0, "relay presentation / access cell");
        let committed = slices[0].bot_locations[&7].clone();
        let loc = slices[0].bot_locations.get_mut(&7).unwrap();
        loc.steps.clear();
        loc.motion.as_mut().unwrap().clear();
        cache.update(&slices, 4000, 4000);
        assert_eq!(cache.snapshot[&0].wait_ms, 4000.0);
        slices[0].bot_locations.insert(7, committed);
        cache.invalidate(0);
        cache.update(&slices, 4000, 4000);
        assert_eq!(cache.intensity(0, HeatmapMode::HtmWait), -1.0);
        assert!(cache.intensity(1, HeatmapMode::HtmSpeed) >= 0.0);
        assert_eq!(cache.snapshot[&0].wait_ms, 1000.0);
        assert_eq!(cache.snapshot[&1].moving_ms, 1000.0);
        assert_eq!(cache.snapshot[&0].moving_ms, 1000.0);
    }

    /// The overlay's "rerun for complete speed coverage" line is about one
    /// thing: a slice that states no motion at all. A `.worldview` always
    /// carries a motion section, so a robot that stood still states an empty
    /// span list and the line stays away.
    #[test]
    fn only_a_slice_with_no_motion_at_all_asks_for_a_rerun() {
        let mut slices = [record()];
        let mut cache = TrafficCache::default();
        cache.update(&slices, 4000, 4000);
        assert!(!cache.legacy_motion);

        slices[0].bot_locations.get_mut(&7).unwrap().motion = Some(Vec::new());
        cache.invalidate(0);
        cache.update(&slices, 4000, 4000);
        assert!(!cache.legacy_motion, "a robot that moved nowhere is not a recording without motion");

        slices[0].bot_locations.get_mut(&7).unwrap().motion = None;
        cache.invalidate(0);
        cache.update(&slices, 4000, 4000);
        assert!(cache.legacy_motion);
    }

    #[test]
    fn all_easyloop_cells_use_the_shared_filter_while_ordinary_navigation_stays_visible() {
        let spec = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sim/dessim/tests/fixtures/minimal_easyloop_flow_spec.json");
        let world = dessim::world_spec::load(&spec).unwrap().world;
        let tiles = worldview_floor::cells::tiles(
            &world.map,
            &world.cell_sizes_mm,
            &world.cell_footprint_stated,
            &world.cell_headings_rad,
            1000.0,
        );
        let cells = crate::app::grid_data::build_grid_data(&world.map, &tiles);
        let mut cache = TrafficCache::default();
        cache.configure(&cells);
        let mut count = 0;
        for machine in &world.map.conveyor_machines {
            for cell in &machine.cells {
                count += 1;
                cache
                    .snapshot
                    .insert(cell.0, CellTraffic { wait_ms: 1000.0, observed_ms: 1000.0, ..Default::default() });
                assert_eq!(cache.intensity(cell.0, HeatmapMode::HtmWait), -1.0);
            }
            for cell in &machine.inventory_queue.as_ref().unwrap().path {
                assert!(cache.exclusions.contains_key(&cell.0));
            }
        }
        assert!(count > 0, "fixture must contain declared equipment waiting cells");
        let ordinary = cells.iter().flatten().find(|cell| cell.navigation_wait_exclusion.is_none()).unwrap();
        assert!(!cache.exclusions.contains_key(&ordinary.id));
        for (id, reason) in world.map.navigation_wait_exclusions() {
            if cells.get(id as usize).is_some_and(Option::is_some) {
                assert_eq!(cache.exclusions.get(&id), Some(&reason));
            }
        }
    }

    #[test]
    fn open_handoff_queue_is_excluded_but_its_upstream_navigation_cell_is_not() {
        use desgraph::CellKind;
        use desmap::HandoffQueue;

        let spec = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sim/dessim/tests/fixtures/minimal_easyloop_flow_spec.json");
        let mut map = dessim::world_spec::load(&spec).unwrap().world.map;
        let before = map.navigation_wait_exclusions();
        let ordinary: Vec<_> = map
            .graph
            .cells()
            .filter(|cell| cell.kind == CellKind::Open && !before.contains_key(&cell.id.0))
            .take(2)
            .map(|cell| cell.id)
            .collect();
        // A handoff queue serves a belt's handover; a lane into a cell a station
        // presents at is that cell's queue instead, never a handoff queue.
        let station = map.stations.iter_mut().find(|s| s.dropoff_cell.is_some()).expect("a station with a belt");
        let dropoff = station.dropoff_cell.expect("found by its dropoff");
        station.handoff_queues.push(HandoffQueue {
            into: dropoff,
            path: vec![ordinary[0]],
            entry_from: Some(ordinary[1]),
            approach: false,
        });
        let after = map.navigation_wait_exclusions();
        assert!(after.contains_key(&ordinary[0].0));
        assert!(!after.contains_key(&ordinary[1].0));
    }

    #[test]
    fn late_delivery_before_the_prefix_rebuilds_without_double_counting() {
        let mut slices: Vec<_> = (0..8)
            .map(|index| {
                let mut slice = record();
                slice.start_ms = index * 4000;
                for span in slice.bot_locations.get_mut(&7).unwrap().motion.as_mut().unwrap() {
                    span.start_ms += slice.start_ms;
                    span.end_ms += slice.start_ms;
                }
                slice
            })
            .collect();
        let committed = slices[0].clone();
        slices[0].bot_locations.clear();
        let mut cache = TrafficCache::default();
        cache.update(&slices, 4000, 32_000);
        assert!(cache.prefix_slices > 0);
        slices[0] = committed;
        cache.invalidate(0);
        cache.update(&slices, 4000, 32_000);
        assert_eq!(cache.snapshot[&0].wait_ms, 8000.0);
        assert_eq!(cache.snapshot[&1].moving_ms, 8000.0);
        cache.update(&slices, 4000, 6500);
        assert_eq!(cache.snapshot[&0].wait_ms, 2000.0);
        assert_eq!(cache.snapshot[&1].moving_ms, 1500.0);
    }

    #[test]
    fn legacy_steps_do_not_fabricate_precise_speed_measurements() {
        let mut slices = [record()];
        slices[0].bot_locations.get_mut(&7).unwrap().motion = None;
        let mut cache = TrafficCache::default();
        cache.update(&slices, 4000, 4000);
        assert!(cache.legacy_motion);
        assert_eq!(cache.snapshot[&0].wait_ms, 1000.0);
        assert_eq!(cache.intensity(1, HeatmapMode::HtmSpeed), -1.0);
    }

    #[test]
    fn profile_round_trip_preserves_heatmaps_and_hover_units() {
        let live = [record()];
        let encoded = rmp_serde::to_vec_named(&live).unwrap();
        let loaded: Vec<SliceRecord> = rmp_serde::from_slice(&encoded).unwrap();
        let mut online = TrafficCache::default();
        let mut replay = TrafficCache::default();
        online.update(&live, 4000, 4000);
        replay.update(&loaded, 4000, 4000);
        for cell in [0, 1] {
            for mode in [HeatmapMode::HtmWait, HeatmapMode::HtmSpeed] {
                assert_eq!(online.intensity(cell, mode), replay.intensity(cell, mode));
            }
            assert_eq!(online.snapshot[&cell].speed_ms, replay.snapshot[&cell].speed_ms);
        }
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                replay.hover(ui, 0, HeatmapMode::HtmWait);
                replay.hover(ui, 1, HeatmapMode::HtmSpeed);
            });
        });
        let text: String = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape { Some(text.galley.job.text.as_str()) } else { None }
            })
            .collect();
        assert!(text.contains("Stationary waiting: 1.00 HTM-s"), "{text}");
        assert!(text.contains("Moving mean: 1.00 m/s"), "{text}");
        assert!(text.contains("Median speed lies in"), "{text}");
    }
}
