//! THE CAPTURE PROGRAM FOR THE GOLDEN BYTES. Not part of this crate's build.
//!
//! It is dropped into a worktree of `db057e21d` — the commit before the
//! renderer moved out of desvis — as `desvis/src/gpu/renderer/
//! golden_capture.rs`, and run once. Everything it writes is what THAT desvis
//! uploaded for the scene in `renderer::pinned_buffers`, which builds the same
//! scene against the moved code. `MANIFEST.md` beside this file states the
//! four-line patch it needs and the command that runs it.

use std::path::Path;

use rustc_hash::FxHashMap;

use super::cell_layer::{compose_floor_instances, rewrite_with_heatmap};
use super::instances::{BotHighlight, ToteAppearance, build_arrow_instances_by_mode, build_bot_instances,
    build_tote_instances};
use super::rack_frames::frame_instances;
use super::totes::draw_instances;
use crate::app::conveyor_motion::apply_transits;
use crate::rack_geometry::{BeltRoute, ContainerPose, EasyLoopBelt, RackBox, RackBoxKind, RackVisual};
use crate::render_frame::RenderFrame;
use crate::traffic::TrafficCache;
use crate::types::*;
use desgraph::types::{CellKind, HtmTraversal};
use dessim::model::{ToteDesignation, ToteType};
use dessim::observer::time_slice::{ToteTransitRecord, ToteVisualState};
use dessim::worldmap::{Floor as FloorPatch, Steel};

fn cell(id: u32, kind: CellKind, tile_mm: [f64; 4], heading_rad: f64, tile: Tile) -> Option<CellInfo> {
    Some(CellInfo {
        id,
        navigation_wait_exclusion: None,
        kind,
        zone_id: 1,
        rack_orientation: None,
        outgoing_edges: 2,
        relay_info: None,
        station_info: None,
        tile_mm,
        heading_rad,
        tile,
    })
}

fn cells() -> Vec<Option<CellInfo>> {
    vec![
        cell(0, CellKind::Highway, [0.0, 0.0, 850.0, 850.0], 0.0, Tile::Declared),
        cell(1, CellKind::Storage, [850.0, 0.0, 850.0, 850.0], 0.0, Tile::Declared),
        cell(2, CellKind::Relay, [1700.0, 0.0, 850.0, 850.0], 0.3, Tile::Marker),
        None,
        cell(4, CellKind::Presentation, [0.0, 850.0, 700.0, 700.0], std::f64::consts::FRAC_PI_4, Tile::Marker),
        cell(5, CellKind::Aisle, [850.0, 850.0, 1227.0, 645.0], 0.0, Tile::Declared),
    ]
}

fn edges() -> Vec<EdgeInfo> {
    vec![
        EdgeInfo {
            from_mm: [425.0, 425.0],
            to_mm: [1275.0, 425.0],
            vtm_traversal: false,
            htm_traversal: HtmTraversal::Both,
            rtp_traversal: false,
        },
        EdgeInfo {
            from_mm: [1275.0, 425.0],
            to_mm: [2125.0, 425.0],
            vtm_traversal: true,
            htm_traversal: HtmTraversal::UnladenOnly,
            rtp_traversal: false,
        },
        EdgeInfo {
            from_mm: [350.0, 1200.0],
            to_mm: [1463.5, 1172.5],
            vtm_traversal: true,
            htm_traversal: HtmTraversal::None,
            rtp_traversal: false,
        },
    ]
}

fn rack() -> RackVisual {
    let mut rack = RackVisual::uniform(5);
    rack.steel_mm = vec![[850.0, 0.0, 850.0, 850.0]];
    rack.relay_tote_pos.insert(2, [1.7, 0.4, 0.9]);
    rack.cell_slots.insert(1, vec![[0.85, 0.42, 0.25], [1.05, 0.42, 0.75]]);
    rack.board_floors_z = vec![0.9, 1.4];
    rack.frames = vec![
        RackBox { min: [0.85, 0.0, 0.0], size: [0.043, 0.85, 1.5], yaw: 0.0, kind: RackBoxKind::Upright },
        RackBox { min: [0.85, 0.0, 0.9], size: [0.85, 0.051, 0.05], yaw: 0.0, kind: RackBoxKind::Beam },
        RackBox { min: [1.7, 0.4, 0.9], size: [0.65, 0.45, 0.043], yaw: 0.7854, kind: RackBoxKind::BoardTab },
    ];
    rack.easyloop_belts = vec![EasyLoopBelt {
        landmarks: [(4, ContainerPose { position: [0.35, 1.2, 0.15], heading: 0.2, pitch: 0.0 })]
            .into_iter()
            .collect(),
        routes: vec![BeltRoute {
            from_cell: 4,
            to_cell: 5,
            points: vec![[0.35, 1.2, 0.15], [0.9, 1.2, 0.15], [1.4635, 1.1725, 0.12]],
        }],
    }];
    rack
}

fn underlay() -> (Vec<[f64; 4]>, Vec<FloorPatch>, Vec<FloorPatch>) {
    (
        vec![[850.0, 0.0, 850.0, 850.0]],
        vec![FloorPatch { rect_mm: [0.0, 1600.0, 2550.0, 120.0], kind: CellKind::StationArea }],
        vec![FloorPatch { rect_mm: [-500.0, -500.0, 4000.0, 3000.0], kind: CellKind::Blank }],
    )
}

fn markers(instant: usize) -> Vec<BotMarker> {
    let turn = instant as f32 * 0.37;
    vec![
        BotMarker {
            cell_id: 0,
            bot_type: BotType::Htm,
            bot_id: 7,
            pos_mm: [2100.0 + instant as f64 * 130.0, 3200.0],
            heading_rad: turn,
        },
        BotMarker {
            cell_id: 1,
            bot_type: BotType::Vtm,
            bot_id: 3,
            pos_mm: [1200.0, 400.0],
            heading_rad: -turn,
        },
        BotMarker { cell_id: 5, bot_type: BotType::Rtp, bot_id: 9, pos_mm: [900.0, 1000.0], heading_rad: 1.1 },
    ]
}

fn tote_states(instant: usize) -> Vec<(u32, ToteVisualState)> {
    let carried = match instant {
        0 => ToteVisualState::InRack { cell_id: 1, level: 0 },
        1 => ToteVisualState::OnBot { bot_id: 7 },
        _ => ToteVisualState::AtRelay { cell_id: 2 },
    };
    vec![
        (11, carried),
        (12, ToteVisualState::InRack { cell_id: 1, level: 2 }),
        (13, ToteVisualState::AtCell { cell_id: 0 }),
        (14, ToteVisualState::OnConveyor { cell_id: 4 }),
        (15, ToteVisualState::OnBot { bot_id: 3 }),
        (16, ToteVisualState::Extracted),
    ]
}

fn appearances() -> FxHashMap<u32, ToteAppearance> {
    [
        (11, ToteAppearance { tote_type: ToteType::Tote, designation: ToteDesignation::Order, contents_units: 3 }),
        (12, ToteAppearance { tote_type: ToteType::Tray, designation: ToteDesignation::Inventory, contents_units: 0 }),
        (13, ToteAppearance { tote_type: ToteType::Carton, designation: ToteDesignation::Order, contents_units: 7 }),
        (14, ToteAppearance { tote_type: ToteType::Tote, designation: ToteDesignation::Inventory, contents_units: 1 }),
    ]
    .into_iter()
    .collect()
}

/// The one conveyor travel: under way at the second and third instants, not
/// yet at the first.
fn transit() -> ToteTransitRecord {
    ToteTransitRecord { tote_id: 14, from_cell: 4, to_cell: 5, start_ms: 1_500, end_ms: 3_500 }
}

fn at_ms(instant: usize) -> u64 {
    1_000 + instant as u64 * 1_000
}

fn heat() -> HeatmapData {
    HeatmapData { vtm_counts: vec![0, 4, 9, 0, 2, 6], htm_counts: vec![3, 0, 1, 0, 8, 5], vtm_max: 9, htm_max: 8 }
}

fn reserved() -> Vec<bool> {
    vec![false, false, true, false, false, true]
}

/// Wait tint `cell / 8`, speed tint `cell / 10`: `intensity` divides waiting
/// by the run's own maximum and a speed of `distance_mm / moving_ms` by 5 m/s.
fn traffic() -> TrafficCache {
    let mut cache = TrafficCache::default();
    cache.max_wait_ms = 8.0;
    for id in 0..6u32 {
        cache.seed_cell_for_capture(
            id,
            f64::from(id),           // wait_ms
            1_000.0,                 // observed_ms
            f64::from(id) * 0.5,     // distance_mm
            1.0,                     // moving_ms
        );
    }
    cache
}

#[test]
#[ignore = "capture tool: writes the golden bytes the carved-out crate is held to"]
fn capture_golden_bytes() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worldview/worldview-render/tests/fixtures");
    std::fs::create_dir_all(&out).unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(out.join(name), bytes).unwrap();

    let cells = cells();
    let frame = RenderFrame::from_tiles(&cells);
    let rack = rack();
    let appearances = appearances();

    for instant in 0..3 {
        let bots = markers(instant);
        let (bot_instances, htm_count) = build_bot_instances(&bots, &frame, &cells, &rack, |b| {
            if b.bot_id == 7 { BotHighlight::Primary } else { BotHighlight::None }
        });
        assert_eq!(htm_count, 1);
        write(&format!("bots_{instant}.bin"), bytemuck::cast_slice(&bot_instances));

        let mut states = tote_states(instant);
        states.sort_by_key(|(id, _)| *id);
        let mut totes = build_tote_instances(&states, &appearances, &cells, &bots, &frame, &rack);
        apply_transits(&mut totes, std::slice::from_ref(&transit()), at_ms(instant), &rack);
        write(&format!("totes_{instant}.bin"), bytemuck::cast_slice(&draw_instances(&totes)));
    }

    let traffic = traffic();
    for (name, mode) in [
        ("off", HeatmapMode::Off),
        ("combined", HeatmapMode::Combined),
        ("wait", HeatmapMode::HtmWait),
        ("speed", HeatmapMode::HtmSpeed),
    ] {
        let rows =
            rewrite_with_heatmap(&cells, Some(&heat()), mode, &traffic, &frame, rack.uniform_levels, &reserved());
        write(&format!("cells_{name}.bin"), bytemuck::cast_slice(&rows));
    }

    write("arrows.bin", bytemuck::cast_slice(&build_arrow_instances_by_mode(&edges(), &frame).0));
    write("rack_frames.bin", bytemuck::cast_slice(&frame_instances(&rack.frames)));
    let (steel, equipment, building) = underlay();
    let steel: Vec<_> = steel.into_iter().map(|rect_mm| Steel { rect_mm, levels: 5 }).collect();
    write("underlay.bin", bytemuck::cast_slice(&compose_floor_instances(&steel, &equipment, &building, &frame)));
}
