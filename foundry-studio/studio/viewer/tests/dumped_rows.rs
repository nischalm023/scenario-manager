//! The claim this round is FOR: for one finished recording, the rows the
//! browser viewer uploads are the rows desvis uploads for the same instant,
//! byte for byte.
//!
//! The two reach the run by entirely different paths — desvis rebuilds the
//! simulator's slice records out of the file and interpolates them, the
//! browser decodes the same frames into a `RunState` and asks
//! `worldview_render::from_recording` — and they meet only at the renderer's
//! instance builders. So a difference here is a difference in the conversion,
//! never in the drawing, and nothing else in the program would find it.
//!
//! **`scripts/worldview/viewer_rows.sh` runs it**, not a plain `cargo test`:
//! desvis's side of the comparison is its own `--dump-render` of the recording,
//! which is generated output and is not checked in. The script has desvis dump
//! the ten-minute recording under `tests/fixtures/minimal_easyloop_flow` and
//! then runs this test against that dump. `CRUCIBLE_VIEWER_RECORDING` and
//! `CRUCIBLE_VIEWER_DUMP` name a larger pair to compare as well — cencora, far
//! too big to keep — and naming one that is not there fails rather than quietly
//! comparing less. A plain `cargo test` reports this test as ignored, by name.
//!
//! The scene is `worldview_floor::compile` of the document the recording
//! carries — what the server sends a browser — and it is held byte for byte
//! against the floor desvis wrote beside its own rows, which is the same
//! compile seen from the other side.

use std::fs::{read, read_to_string};
use std::path::{Path, PathBuf};

use bytemuck::Zeroable;
use crucible_viewer::Run;
use dessim::worldmap;
use desunits::SimTime;
use serde::Serialize;
use worldview::{Frame, Ms, Opaque, Payload, Reader, Scenario, Tag};
use worldview_render::{
    BotInstance, BotType, CellInstance, Census, Drawn, FloorDoc, HeatmapData, HeatmapMode, Overlays, ToteInstance,
    TrafficTints,
};

/// The recording that ships with the test beside the dump the script made of
/// it, and any larger pair asked for. The flag is whether that fixture has
/// conveyor belts, which the shipped one is chosen for and no cencora document
/// has.
fn fixtures() -> Vec<(String, PathBuf, PathBuf, bool)> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop_flow");
    // desvis's dump of the shipped recording is generated, so where it is has
    // to be said; running without it would compare nothing and pass.
    let dump = std::env::var("CRUCIBLE_VIEWER_FIXTURE_DUMP").unwrap_or_else(|_| {
        panic!("CRUCIBLE_VIEWER_FIXTURE_DUMP names no dump of the shipped recording; scripts/worldview/viewer_rows.sh makes one and runs this test")
    });
    let mut pairs = vec![("minimal_easyloop_flow".to_string(), here.join("recording.worldview"), dump.into(), true)];
    match (std::env::var("CRUCIBLE_VIEWER_RECORDING"), std::env::var("CRUCIBLE_VIEWER_DUMP")) {
        (Ok(recording), Ok(dump)) => pairs.push(("the named recording".into(), recording.into(), dump.into(), false)),
        (Err(_), Err(_)) => {}
        // One without the other is a mistake worth saying out loud: quietly
        // comparing only the small fixture is how a second one stops being
        // compared without anybody noticing.
        _ => panic!("CRUCIBLE_VIEWER_RECORDING and CRUCIBLE_VIEWER_DUMP are set together or not at all"),
    }
    pairs
}

/// The frames an attachment at 0 carries, in order: the scenario block with
/// the compiled floor put in, then the file's FIRST keyframe and every delta
/// after it. Later keyframes never travel — the browser is holding the state
/// the deltas advance, and a second one would only restate it — which is what
/// makes a file and an attachment different lists of frames.
///
/// This is `studio/runs`'s `floor::compile`, `attach`'s re-encode and its
/// `to_send`, done here so the test needs no server: the bytes a browser is
/// handed are the same either way.
fn attachment(recording: &Path, floor: &Opaque) -> Vec<Vec<u8>> {
    let mut reader = Reader::open(recording).expect("the recording opens");
    let mut frames = Vec::new();
    let mut opened = false;
    for walked in reader.frames_from(0) {
        let (_, frame) = walked.expect("a frame reads");
        if frame.header.tag == Tag::Keyframe {
            if opened {
                continue;
            }
            opened = true;
        }
        let bytes = match frame.header.tag {
            Tag::Scenario => {
                let mut sent = Scenario::from_bytes(&frame.contents(0).expect("its body"), 0).expect("it decodes");
                sent.floor = Some(floor.clone());
                sent.census = Some(Opaque::of(&Census::new(), "the entity census").expect("encodes"));
                Frame::encode(Tag::Scenario, Ms::ZERO, &sent.to_bytes().expect("encodes"), true).expect("a frame")
            }
            _ => frame.to_bytes(),
        };
        frames.push(bytes);
    }
    frames
}

/// The floor the server sends a browser: `worldview_floor::compile` of the
/// document the recording carries, which is the one compile desvis draws from
/// too — so the floor in the dump beside its rows must be that same floor,
/// belts and all, and this says so before a single row is compared.
fn floor_of(recording: &Path, dump: &Path, belts: bool) -> Opaque {
    let mut reader = Reader::open(recording).expect("the recording opens");
    let scenario = Scenario::read_from(&mut reader).expect("its scenario block reads");
    let world = worldmap::load_from_bytes(&scenario.world.0).expect("its world document loads");
    let served = worldview_floor::compile(&world).expect("the document states a floor").floor;

    let bytes = read(dump.join("floor.msgpack")).expect("the dump carries the floor desvis drew on");
    let drawn: FloorDoc = rmp_serde::from_slice(&bytes).expect("it decodes");
    // One document, one encoding: the whole floor, byte for byte. Its two
    // per-cell maps and its belts' landmarks are ordered for exactly this —
    // the same floor compiled twice, or compiled and read back, is the same
    // bytes, so a server, a cache and desvis can be held to one another.
    //
    // A difference here prints a layer's name rather than a megabyte of
    // MessagePack, which is what a whole-document `assert_eq!` would do.
    let (mine, theirs) = (pack(&served), pack(&drawn));
    if mine != theirs {
        let layers: [(&str, Vec<u8>, Vec<u8>); 6] = [
            ("version", pack(&served.version), pack(&drawn.version)),
            ("cells", pack(&served.cells), pack(&drawn.cells)),
            ("cell size", pack(&served.cell_size_mm), pack(&drawn.cell_size_mm)),
            ("hops", pack(&served.edges), pack(&drawn.edges)),
            ("rack", pack(&served.rack), pack(&drawn.rack)),
            ("underlay", pack(&served.underlay), pack(&drawn.underlay)),
        ];
        let differ: Vec<&str> = layers.iter().filter(|(_, a, b)| a != b).map(|(what, _, _)| *what).collect();
        panic!("the floor the server compiles is not the floor desvis drew; its {differ:?} differ");
    }
    assert_eq!(
        !served.rack.easyloop_belts.is_empty(),
        belts,
        "whether this fixture's rows exercise the conveyor routing"
    );

    Opaque::of(&served, "the compiled floor").expect("encodes")
}

fn pack<T: Serialize>(value: &T) -> Vec<u8> {
    rmp_serde::to_vec(value).expect("encodes")
}

/// What a row is made of, so a difference is reported as a field and not as a
/// byte offset. The widths are the dump manifest's own.
type Layout = &'static [(&'static str, usize)];

const BOTS: Layout = &[
    ("bot_id", 4),
    ("world_x", 4),
    ("world_y", 4),
    ("world_z", 4),
    ("bot_type", 4),
    ("highlight", 4),
    ("heading_mrad", 4),
];
const TOTES: Layout = &[
    ("tote_id", 4),
    ("contents_units", 8),
    ("world_x", 4),
    ("world_y", 4),
    ("world_z", 4),
    ("tote_type", 4),
    ("designation", 4),
    ("heading_mrad", 4),
    ("pitch", 4),
    ("physical_type", 4),
];
const CELLS: Layout = &[
    ("min_x", 4),
    ("min_y", 4),
    ("size_x", 4),
    ("size_y", 4),
    ("cell_kind", 4),
    ("rack_levels", 4),
    ("heatmap_vtm", 4),
    ("heatmap_htm", 4),
    ("reserved", 4),
    ("depth_bias", 4),
    ("heading_rad", 4),
    ("pad", 4),
];
const RESERVED: Layout = &[("reserved", 1)];
const HEAT: Layout = &[("heatmap_vtm", 4), ("heatmap_htm", 4)];

/// The rows, laid out exactly as `desvis`'s `app/render_dump.rs` lays them
/// out. Little-endian, no padding, fields in the order the manifest states.
#[derive(Default)]
struct Row(Vec<u8>);

impl Row {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A heading is stated in whole milliradians on both sides, because that
    /// is all a recording carries.
    fn heading(&mut self, radians: f32) {
        self.u32(((f64::from(radians) * 1_000.0).round() as i32) as u32);
    }
    /// A position is stated in whole millimetres on both sides, for the same
    /// reason: the recording's coordinates are whole millimetres, and what
    /// each viewer interpolates between two of them is drawn at the format's
    /// own precision. `RenderFrame::world_mm` is a plain divide by a
    /// thousand, so rounding a render metre to the nearest thousandth here
    /// rounds the millimetre. This is `render_dump.rs`'s own arithmetic,
    /// spelled out so both encoders say the same thing.
    fn world_mm(&mut self, metres: f32) {
        self.f32(((f64::from(metres) * 1_000.0).round() / 1_000.0) as f32);
    }
}

/// The robot rows the GPU is fed, in ascending id: the buffer the renderer
/// built, re-ordered rather than rebuilt, since the draw order is the
/// renderer's own and never enters the file.
///
/// `build_bot_instances` puts the HTMs first and keeps each group in the order
/// it was handed, which is ascending id — so pairing the two lists that way
/// names every row of that buffer.
fn bot_rows(drawn: &Drawn, instances: &[BotInstance]) -> Vec<u8> {
    let htms = drawn.robots.iter().filter(|robot| robot.class == BotType::Htm);
    let others = drawn.robots.iter().filter(|robot| robot.class != BotType::Htm);
    let mut rows: Vec<(u32, &BotInstance)> = htms.chain(others).map(|robot| robot.id.0).zip(instances).collect();
    assert_eq!(rows.len(), instances.len(), "one id per row of the buffer");
    rows.sort_by_key(|(id, _)| *id);

    let mut row = Row::default();
    for (id, bot) in rows {
        row.u32(id);
        row.world_mm(bot.world_x);
        row.world_mm(bot.world_y);
        row.f32(bot.world_z);
        row.u32(bot.bot_type);
        row.u32(bot.highlight);
        row.heading(bot.heading);
    }
    row.0
}

fn tote_rows(instances: &[ToteInstance]) -> Vec<u8> {
    let mut sorted: Vec<&ToteInstance> = instances.iter().collect();
    sorted.sort_by_key(|tote| tote.tote_id);
    let mut row = Row::default();
    for tote in sorted {
        row.u32(tote.tote_id);
        row.u64(tote.contents_units);
        row.world_mm(tote.world_x);
        row.world_mm(tote.world_y);
        row.f32(tote.world_z);
        row.u32(tote.tote_type);
        row.u32(tote.designation);
        row.heading(tote.heading);
        row.f32(tote.pitch);
        row.u32(tote.physical_type);
    }
    row.0
}

fn cell_rows(instances: &[CellInstance]) -> Vec<u8> {
    let mut row = Row::default();
    for cell in instances {
        row.f32(cell.min_x);
        row.f32(cell.min_y);
        row.f32(cell.size_x);
        row.f32(cell.size_y);
        row.u32(cell.cell_kind);
        row.u32(cell.rack_levels);
        row.f32(cell.heatmap_vtm);
        row.f32(cell.heatmap_htm);
        row.u32(cell.reserved);
        row.f32(cell.depth_bias);
        row.f32(cell.heading_rad);
        row.u32(cell._pad);
    }
    row.0
}

fn reserved_rows(flags: &[bool]) -> Vec<u8> {
    flags.iter().map(|flag| u8::from(*flag)).collect()
}

/// The two tint columns a visit overlay paints, normalised the way the cell
/// shader reads them.
fn heat_rows(heat: &HeatmapData, cells: usize) -> Vec<u8> {
    let global_max = heat.vtm_max.max(heat.htm_max).max(1) as f32;
    let mut row = Row::default();
    for cell in 0..cells {
        row.f32(heat.vtm_counts.get(cell).copied().unwrap_or(0) as f32 / global_max);
        row.f32(heat.htm_counts.get(cell).copied().unwrap_or(0) as f32 / global_max);
    }
    row.0
}

/// The two tint columns the measured overlay paints: the share of a cell's
/// exposure spent waiting, and the mean speed over it.
fn traffic_rows(run: &Run, cells: usize) -> Vec<u8> {
    let traffic = run.traffic().expect("a floor and a keyframe");
    let mut row = Row::default();
    for cell in 0..cells as u32 {
        row.f32(traffic.intensity(cell, HeatmapMode::HtmWait));
        row.f32(traffic.intensity(cell, HeatmapMode::HtmSpeed));
    }
    row.0
}

/// How the browser's array differs from desvis's: how many rows, which
/// fields, and the first three disagreements in full.
fn difference(layout: Layout, mine: &[u8], theirs: &[u8]) -> String {
    let stride: usize = layout.iter().map(|(_, width)| width).sum();
    if mine.len() != theirs.len() {
        return format!("{} rows against {}", mine.len() / stride, theirs.len() / stride);
    }
    let (mut rows, mut fields, mut shown) = (0usize, Vec::new(), Vec::new());
    for row in 0..mine.len() / stride {
        let at = row * stride;
        if mine[at..at + stride] == theirs[at..at + stride] {
            continue;
        }
        rows += 1;
        let mut within = at;
        for (field, width) in layout {
            if mine[within..within + width] != theirs[within..within + width] {
                if !fields.contains(field) {
                    fields.push(field);
                }
                if shown.len() < 3 {
                    shown.push(format!(
                        "row {row} {field}: {:?} against {:?}",
                        &mine[within..within + width],
                        &theirs[within..within + width]
                    ));
                }
            }
            within += width;
        }
    }
    match rows {
        0 => "identical".to_string(),
        _ => format!("{rows} of {} rows differ, in {fields:?} — {}", mine.len() / stride, shown.join("; ")),
    }
}

/// The instants the dump holds, read off its own manifest.
fn instants(manifest: &str) -> Vec<u64> {
    let opened = manifest.split("\"ticks_ms\": [").nth(1).expect("the manifest states its instants");
    let ticks: Vec<u64> = opened
        .split(']')
        .next()
        .expect("a closed list")
        .split(',')
        .map(|word| word.trim().parse::<u64>().expect("an instant"))
        .collect();
    assert!(ticks.len() >= 3, "a dump of fewer than three instants proves little about a run");
    ticks
}

#[test]
#[ignore = "needs desvis's render dump of the shipped recording: run scripts/worldview/viewer_rows.sh"]
fn the_browsers_rows_are_the_rows_desvis_uploads() {
    let (mut compared, mut faults) = (0usize, Vec::<String>::new());
    for (name, recording, dump, belts) in fixtures() {
        let manifest =
            read_to_string(dump.join("manifest.json")).unwrap_or_else(|e| panic!("{name}: no manifest: {e}"));
        let ticks = instants(&manifest);
        let floor = floor_of(&recording, &dump, belts);

        let mut run = Run::default();
        let frames = attachment(&recording, &floor);
        for frame in &frames {
            run.push_frame(frame).expect("every frame the server would send decodes");
        }
        println!("{name}: {} frames pushed, {} instants to compare", frames.len(), ticks.len());

        for tick_ms in ticks {
            let drawn = run.drawn_at(SimTime::ms(tick_ms)).expect("the window holds it").expect("the slice reads");
            assert_eq!(drawn.at_ms, tick_ms, "the cursor stands where it was put");
            let scene = run.scene().expect("a floor");
            let cells = scene.cells().len();
            let moment = scene.moment(
                Zeroable::zeroed(),
                None,
                &drawn.playback(Overlays {
                    heatmap: None,
                    mode: HeatmapMode::Off,
                    traffic: &(),
                    reserved_cells: &drawn.reserved_cells,
                }),
            );

            let mut arrays = vec![
                ("bots", BOTS, bot_rows(&drawn, &moment.bot_instances)),
                ("totes", TOTES, tote_rows(&moment.tote_instances)),
                ("cells", CELLS, cell_rows(&moment.cell_instances)),
                ("reserved", RESERVED, reserved_rows(&drawn.reserved_cells)),
                ("slice_heat", HEAT, heat_rows(&run.slice_overlay().expect("a slice"), cells)),
                ("traffic", HEAT, traffic_rows(&run, cells)),
            ];
            // The run's total visit counters belong to its end instant and to
            // no other, so the dump holds that array once.
            if dump.join(format!("t{tick_ms}.heat.bin")).exists() {
                arrays.push(("heat", HEAT, heat_rows(&run.visits_so_far().expect("counters"), cells)));
            }

            for (array, layout, mine) in arrays {
                let theirs = read(dump.join(format!("t{tick_ms}.{array}.bin")))
                    .unwrap_or_else(|e| panic!("{name} t{tick_ms}.{array}.bin: {e}"));
                let said = difference(layout, &mine, &theirs);
                println!("  t{tick_ms} {array}: {said}");
                faults.extend((said != "identical").then(|| format!("{name} t{tick_ms} {array}: {said}")));
                compared += 1;
            }
        }
    }
    assert!(faults.is_empty(), "the two viewers disagree:\n{}", faults.join("\n"));
    println!("{compared} arrays compared, all identical");
}
