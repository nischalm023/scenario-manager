//! Within `app`, the RENDER DUMP is the set of arrays desvis's renderer would
//! upload for one simulated instant, written to files instead of to the GPU.
//!
//! It exists so that two desvis binaries — this branch's, reading a
//! `.worldview`, and master's, reading its own recording — can be shown to
//! draw the same picture from the same run without anybody comparing
//! screenshots. Harness B (`scripts/worldview/parity_b.sh`) dumps both over
//! one tick list and diffs the bytes. The layout every array uses is written
//! field by field in `docs/designs/worldview-render-dump.md`.

use std::fmt::Write as _;
use std::fs::{create_dir_all, write};
use std::path::Path;

use dessim::metrics::worldview::read_worldview;

use super::DesvisApp;
use crate::traffic::TrafficCache;
use crate::types::{DataSource, HeatmapData, HeatmapMode, ViewMode};
use bytemuck::Zeroable;
use worldview_render::{
    BotInstance, CellInstance, FloorDoc, Overlays, Playback, ToteInstance, TrafficTints, build_bot_instances,
};

/// The field types a dumped row is made of. Every value is little-endian and
/// sits immediately after the one before it, so a row has no padding at all.
#[derive(Clone, Copy)]
enum Ty {
    U8,
    U32,
    I32,
    U64,
    F32,
}

impl Ty {
    fn width(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 => 8,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U32 => "u32",
            Self::I32 => "i32",
            Self::U64 => "u64",
            Self::F32 => "f32",
        }
    }
}

type Layout = &'static [(&'static str, Ty)];

const BOTS: Layout = &[
    ("bot_id", Ty::U32),
    ("world_x", Ty::F32),
    ("world_y", Ty::F32),
    ("world_z", Ty::F32),
    ("bot_type", Ty::U32),
    ("highlight", Ty::U32),
    ("heading_mrad", Ty::I32),
];

const TOTES: Layout = &[
    ("tote_id", Ty::U32),
    ("contents_units", Ty::U64),
    ("world_x", Ty::F32),
    ("world_y", Ty::F32),
    ("world_z", Ty::F32),
    ("tote_type", Ty::U32),
    ("designation", Ty::U32),
    ("heading_mrad", Ty::I32),
    ("pitch", Ty::F32),
    ("physical_type", Ty::U32),
];

const CELLS: Layout = &[
    ("min_x", Ty::F32),
    ("min_y", Ty::F32),
    ("size_x", Ty::F32),
    ("size_y", Ty::F32),
    ("cell_kind", Ty::U32),
    ("rack_levels", Ty::U32),
    ("heatmap_vtm", Ty::F32),
    ("heatmap_htm", Ty::F32),
    ("reserved", Ty::U32),
    ("depth_bias", Ty::F32),
    ("heading_rad", Ty::F32),
    ("pad", Ty::U32),
];

const RESERVED: Layout = &[("reserved", Ty::U8)];

const HEAT: Layout = &[("heatmap_vtm", Ty::F32), ("heatmap_htm", Ty::F32)];

const TRAFFIC: Layout = &[("wait", Ty::F32), ("speed", Ty::F32)];

/// One named array of the dump: the bytes, and the layout the manifest states
/// so a reader never has to guess where a field starts.
struct Array {
    name: &'static str,
    layout: Layout,
    rows: usize,
    bytes: Vec<u8>,
}

/// Builds rows field by field. Nothing here rounds, scales or reorders — the
/// values are the f32 bit patterns the renderer computed.
#[derive(Default)]
struct Row {
    bytes: Vec<u8>,
    /// Where a NaN was written, as a byte offset into `bytes`. Only the first
    /// is kept; [`Array::new`] turns it into a row and a field name.
    nan_at: Option<usize>,
}

impl Row {
    fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    /// A heading is stated in whole milliradians on both sides of harness B,
    /// because that is all a recording carries (the round-2a ruling): the
    /// branch's radians are a milliradian count divided by a thousand, and
    /// master's are the raw f32 the observer derived.
    ///
    /// The arithmetic is `wire::milliradians`' own, spelled out rather than
    /// imported so that master's copy of this file says the same thing.
    ///
    /// The NaN check happens BEFORE the cast, because `f32 as i32` turns a NaN
    /// into 0 — a heading pointing along +x, which no later scan can tell from
    /// a real one.
    fn heading_mrad(&mut self, radians: f32) {
        if radians.is_nan() {
            self.nan_at.get_or_insert(self.bytes.len());
        }
        self.bytes.extend_from_slice(&((f64::from(radians) * 1_000.0).round() as i32).to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    /// A position is stated in whole millimetres on both sides of harness B,
    /// because that is all a recording carries (the 2026-09-18 ruling): the
    /// branch's coordinates are whole millimetres the renderer interpolated in
    /// f64, and master's are the f64 its own observer computed, so the two
    /// land on the same millimetre rather than a fraction apart.
    ///
    /// `RenderFrame::world_mm` is a plain divide by a thousand, so rounding a
    /// render metre back to the nearest thousandth here rounds the millimetre
    /// itself. Spelled out rather than imported so that master's copy of this
    /// file says the same thing.
    fn world_mm(&mut self, metres: f32) {
        if metres.is_nan() {
            self.nan_at.get_or_insert(self.bytes.len());
        }
        self.bytes.extend_from_slice(&(((f64::from(metres) * 1_000.0).round() / 1_000.0) as f32).to_le_bytes());
    }
}

impl Array {
    fn new(name: &'static str, layout: Layout, rows: usize, row: Row) -> Result<Self, String> {
        let stride: usize = layout.iter().map(|(_, ty)| ty.width()).sum();
        if row.bytes.len() != rows * stride {
            return Err(format!("{name}: {} bytes for {rows} rows of {stride}", row.bytes.len()));
        }
        if let Some(at) = row.nan_at {
            return Err(format!("{name}: {} is NaN", Self::field_at(layout, stride, at)));
        }
        let array = Self { name, layout, rows, bytes: row.bytes };
        array.refuse_nan()?;
        Ok(array)
    }

    /// Which row and field a byte offset falls in, for a message about it.
    fn field_at(layout: Layout, stride: usize, at: usize) -> String {
        let (row, mut within) = (at / stride, at % stride);
        for (field, ty) in layout {
            if within < ty.width() {
                return format!("row {row} field {field}");
            }
            within -= ty.width();
        }
        format!("row {row} byte {}", at % stride)
    }

    /// A NaN coordinate draws nothing and compares unequal to itself, so a
    /// dump carrying one is a defect report, not a baseline.
    fn refuse_nan(&self) -> Result<(), String> {
        let stride: usize = self.layout.iter().map(|(_, ty)| ty.width()).sum();
        for index in 0..self.rows {
            let mut at = index * stride;
            for (field, ty) in self.layout {
                if matches!(ty, Ty::F32) {
                    let word = u32::from_le_bytes([
                        self.bytes[at],
                        self.bytes[at + 1],
                        self.bytes[at + 2],
                        self.bytes[at + 3],
                    ]);
                    if f32::from_bits(word).is_nan() {
                        return Err(format!("{}: row {index} field {field} is NaN", self.name));
                    }
                }
                at += ty.width();
            }
        }
        Ok(())
    }

    fn manifest_entry(&self, tick_ms: u64, file: &str) -> String {
        let mut fields = String::new();
        let mut offset = 0usize;
        for (field, ty) in self.layout {
            if !fields.is_empty() {
                fields.push_str(", ");
            }
            let _ = write!(fields, r#"{{"name": "{field}", "type": "{}", "offset": {offset}}}"#, ty.name());
            offset += ty.width();
        }
        format!(
            r#"    {{"tick_ms": {tick_ms}, "array": "{}", "file": "{file}", "stride": {offset}, "rows": {}, "fields": [{fields}]}}"#,
            self.name, self.rows
        )
    }
}

fn bot_rows(instances: &[BotInstance], ids: &[u32]) -> Result<Array, String> {
    let mut row = Row::default();
    for (instance, &id) in instances.iter().zip(ids) {
        row.u32(id);
        row.world_mm(instance.world_x);
        row.world_mm(instance.world_y);
        row.f32(instance.world_z);
        row.u32(instance.bot_type);
        row.u32(instance.highlight);
        row.heading_mrad(instance.heading);
    }
    Array::new("bots", BOTS, instances.len(), row)
}

fn tote_rows(instances: &[ToteInstance]) -> Result<Array, String> {
    let mut row = Row::default();
    for instance in instances {
        row.u32(instance.tote_id);
        row.u64(instance.contents_units);
        row.world_mm(instance.world_x);
        row.world_mm(instance.world_y);
        row.f32(instance.world_z);
        row.u32(instance.tote_type);
        row.u32(instance.designation);
        row.heading_mrad(instance.heading);
        row.f32(instance.pitch);
        row.u32(instance.physical_type);
    }
    Array::new("totes", TOTES, instances.len(), row)
}

fn cell_rows(instances: &[CellInstance]) -> Result<Array, String> {
    let mut row = Row::default();
    for instance in instances {
        row.f32(instance.min_x);
        row.f32(instance.min_y);
        row.f32(instance.size_x);
        row.f32(instance.size_y);
        row.u32(instance.cell_kind);
        row.u32(instance.rack_levels);
        row.f32(instance.heatmap_vtm);
        row.f32(instance.heatmap_htm);
        row.u32(instance.reserved);
        row.f32(instance.depth_bias);
        row.f32(instance.heading_rad);
        row.u32(instance._pad);
    }
    Array::new("cells", CELLS, instances.len(), row)
}

fn reserved_rows(flags: &[bool]) -> Result<Array, String> {
    let mut row = Row::default();
    for &flag in flags {
        row.u8(u8::from(flag));
    }
    Array::new("reserved", RESERVED, flags.len(), row)
}

/// The two tint columns the traffic overlay paints: how much of a cell's
/// exposure was spent waiting, and the mean moving speed over it. −1 is the
/// overlay's own "nothing measured here", not a missing value.
fn traffic_rows(traffic: &TrafficCache, cells: usize) -> Result<Array, String> {
    let mut row = Row::default();
    for id in 0..cells {
        row.f32(traffic.intensity(id as u32, HeatmapMode::HtmWait));
        row.f32(traffic.intensity(id as u32, HeatmapMode::HtmSpeed));
    }
    Array::new("traffic", TRAFFIC, cells, row)
}

/// The two tint columns an overlay paints, normalised the way
/// `rewrite_with_heatmap` normalises them — the numbers the shader reads.
fn heat_rows(name: &'static str, heat: &HeatmapData, cells: usize) -> Result<Array, String> {
    let global_max = heat.vtm_max.max(heat.htm_max).max(1) as f32;
    let mut row = Row::default();
    for id in 0..cells {
        row.f32(heat.vtm_counts.get(id).copied().unwrap_or(0) as f32 / global_max);
        row.f32(heat.htm_counts.get(id).copied().unwrap_or(0) as f32 / global_max);
    }
    Array::new(name, HEAT, cells, row)
}

impl DesvisApp {
    /// What a playback frame uploads at the cursor's instant: the bots, the
    /// totes, the per-cell rows, the reservation flags and the slice's heat.
    ///
    /// The view is the one realtime playback puts the app in — State, with the
    /// heat overlay off — so the cell rows carry geometry and the live relay
    /// reservations, and the heat overlay is dumped on its own beside them.
    fn upload_at(&mut self, tick_ms: u64) -> Result<Vec<Array>, String> {
        self.view_mode = ViewMode::State;
        self.heatmap_mode = HeatmapMode::Off;
        self.playback_active = true;
        self.playback_engaged = true;
        self.time_cursor_ms = tick_ms as f64;
        self.update_grid_for_cursor();

        // Both ends walk their own hash map to place robots and the renderer
        // then puts the HTMs first so the two glyph draws stay contiguous, so
        // the dump asks for one robot at a time and states them by ascending
        // id; the draw order never enters the file.
        let highlights = self.bot_highlight_set();
        let (totes, transits) = self.totes_at_cursor();
        let reserved = self.build_reserved_cells();
        let at_ms = self.resource_cursor().as_ms();

        let mut rows: Vec<(u32, BotInstance)> = Vec::new();
        for marker in &self.bot_positions {
            let one = [marker.robot(highlights.classify(marker))];
            if let Some(instance) = build_bot_instances(&one, &self.drawn).0.into_iter().next() {
                rows.push((marker.bot_id, instance));
            }
        }
        rows.sort_by_key(|(id, _)| *id);
        let (ids, bots): (Vec<u32>, Vec<BotInstance>) = rows.into_iter().unzip();

        // One moment for everything else: the containers, and the cell rows
        // with this instant's overlays folded in.
        let whole = self.drawn.moment(
            Zeroable::zeroed(),
            None,
            &Playback {
                at_ms,
                robots: &self.highlighted_robots(&highlights),
                totes: &totes,
                transits: &transits,
                overlays: Overlays {
                    heatmap: self.heatmap_data.as_ref(),
                    mode: self.heatmap_mode,
                    traffic: &self.traffic,
                    reserved_cells: &reserved,
                },
            },
        );
        let mut totes = whole.tote_instances;
        totes.sort_by_key(|tote| tote.tote_id);
        let cells = whole.cell_instances;

        let mut arrays =
            vec![bot_rows(&bots, &ids)?, tote_rows(&totes)?, cell_rows(&cells)?, reserved_rows(&reserved)?];
        // The slice's overlay as the app now holds it — the count off the step
        // cells that `update_grid_for_cursor` just put there, not a second
        // derivation of it. The recording also carries per-slice heat
        // increments, and desvis draws none of them yet: they are for the
        // browser in phase 2, and drawing them would change this picture.
        if let Some(heat) = &self.heatmap_data {
            arrays.push(heat_rows("slice_heat", heat, self.cell_infos.len())?);
        }

        // The traffic overlay reads the motion spans, which are the newest
        // thing in a recording, so the dump asks for them in the view that
        // draws them and then puts the app back.
        self.view_mode = ViewMode::Simulation;
        self.heatmap_mode = HeatmapMode::HtmWait;
        self.refresh_traffic();
        arrays.push(traffic_rows(&self.traffic, self.cell_infos.len())?);
        self.view_mode = ViewMode::State;
        self.heatmap_mode = HeatmapMode::Off;
        Ok(arrays)
    }

    /// The instant the run ended at, which is where the run-total overlay is
    /// dumped. It is the summary's own sim time, never a slice boundary, and
    /// never earlier than the last slice a recording holds.
    fn run_end_ms(&self) -> u64 {
        let DataSource::Profile(profile) = &self.data_source else { return 0 };
        let ended = (profile.summary.sim_time_secs * 1000.0).round().max(0.0) as u64;
        ended.max(self.time_series.last().map(|slice| slice.start_ms).unwrap_or(0))
    }
}

/// Dump every tick of `ticks` into `out`, in the order given, from one app
/// built once: a marker desvis placed in an earlier slice stays where it
/// stands until a later slice moves it, exactly as a playback session does.
///
/// The run-total heat overlay belongs to the run's END instant and to no
/// other, so that instant is appended when the list leaves it out.
pub(crate) fn run(recording: &Path, ticks: &[u64], out: &Path) -> Result<(), String> {
    let named = |e: String| format!("{}: {e}", recording.display());
    let read = read_worldview(recording).map_err(|e| named(e))?;
    if read.truncated {
        eprintln!("note: {} has no end block — the run crashed or is still going", recording.display());
    }
    let mut app = DesvisApp::from_profile(read.profile, recording)?;
    let app = &mut app;
    // The overlay the app opens on, before any cursor move replaces it with a
    // slice's own count: the run total, read off the recording's counters.
    let opened_on = app.heatmap_data.as_ref().ok_or_else(|| named("it states no cumulative heat".to_string()))?;
    let mut run_total = Some(heat_rows("heat", opened_on, app.cell_infos.len()).map_err(|e| named(e))?);
    let end_ms = app.run_end_ms();

    let mut wanted: Vec<u64> = ticks.to_vec();
    for (index, tick) in wanted.iter().enumerate() {
        if wanted[..index].contains(tick) {
            return Err(named(format!("the instant {tick} is listed twice")));
        }
    }
    if !wanted.contains(&end_ms) {
        wanted.push(end_ms);
    }

    create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    // The floor desvis drew these rows on, beside them. A reader comparing
    // another viewer's rows needs the same scene to build them from, and the
    // conveyor belts in it are desvis's own placement — which the server's
    // floor compile does not carry yet.
    let floor = rmp_serde::to_vec(&FloorDoc::of_scene(&app.drawn)).map_err(|e| format!("floor.msgpack: {e}"))?;
    write(out.join("floor.msgpack"), floor).map_err(|e| format!("floor.msgpack: {e}"))?;
    let mut entries: Vec<String> = Vec::new();
    for &tick_ms in &wanted {
        let mut arrays = app.upload_at(tick_ms).map_err(|e| named(e))?;
        if tick_ms == end_ms
            && let Some(heat) = run_total.take()
        {
            arrays.push(heat);
        }
        for array in &arrays {
            let file = format!("t{tick_ms}.{}.bin", array.name);
            write(out.join(&file), &array.bytes).map_err(|e| format!("{file}: {e}"))?;
            entries.push(array.manifest_entry(tick_ms, &file));
        }
    }
    let manifest = format!(
        "{{\n  \"schema\": \"worldview-render-dump/1\",\n  \"recording\": {:?},\n  \"run_end_ms\": {end_ms},\n  \"ticks_ms\": [{}],\n  \"arrays\": [\n{}\n  ]\n}}\n",
        recording.display().to_string(),
        wanted.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", "),
        entries.join(",\n")
    );
    write(out.join("manifest.json"), manifest).map_err(|e| format!("manifest.json: {e}"))?;
    println!("dumped {} instants to {}, the run total at {end_ms} ms", wanted.len(), out.display());
    Ok(())
}
