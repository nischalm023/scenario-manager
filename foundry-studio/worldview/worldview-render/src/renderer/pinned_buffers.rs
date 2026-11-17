//! The buffers this crate builds for one scene, held to the bytes DESVIS
//! uploaded for the same scene before the renderer was carved out of it.
//!
//! The fixtures beside this file were written at `db057e21d` — the commit
//! before the move — by `tests/fixtures/capture/golden_capture.rs`, which
//! builds the very same scene against that desvis's own code;
//! `tests/fixtures/capture/MANIFEST.md` states the patch it needs and the
//! command that runs it, so any byte here can be re-derived.
//!
//! The scene is chosen to touch every row a viewer can draw: cells of both
//! tile kinds, hops of all three traversals, steel of three box kinds, the
//! composed underlay, three robots at three instants, containers in all six
//! places, a container part-way along a conveyor, and the four overlays. If a
//! byte moves, the two viewers have stopped drawing the same picture, which is
//! the whole reason for the crate.

use std::path::Path;

use desgraph::CellId;
use desgraph::types::{CellKind, HtmTraversal};
use desunits::{RobotId, SimTime, ToteId};
use worldview::frames::playback::{ToteLocation, ToteTransit};

use super::cell_layer::compose_floor_instances;
use super::instances::arrow_instance_bytes;
use super::{rack_frames, totes};
use crate::camera::{CameraState, CameraUniforms, Viewport};
use crate::rack::{BeltRoute, ContainerPose, EasyLoopBelt, RackBox, RackBoxKind, RackVisual};
use crate::scene::{
    BotHighlight, BotType, Cell, ContainerKind, Designation, Edge, Floor, FloorPatch, HeatmapData, HeatmapMode,
    Overlays, Place, Playback, Robot, Scene, Tile, Tote, ToteAppearance, TrafficTints, Underlay,
};
use crate::view::ViewMode;

fn cell(id: u32, kind: CellKind, tile_mm: [f64; 4], heading_rad: f64, tile: Tile) -> Option<Cell> {
    Some(Cell { id, kind, tile_mm, heading_rad, tile, wait_excluded: false })
}

fn cells() -> Vec<Option<Cell>> {
    vec![
        cell(0, CellKind::Highway, [0.0, 0.0, 850.0, 850.0], 0.0, Tile::Declared),
        cell(1, CellKind::Storage, [850.0, 0.0, 850.0, 850.0], 0.0, Tile::Declared),
        cell(2, CellKind::Relay, [1700.0, 0.0, 850.0, 850.0], 0.3, Tile::Marker),
        None,
        cell(4, CellKind::Presentation, [0.0, 850.0, 700.0, 700.0], std::f64::consts::FRAC_PI_4, Tile::Marker),
        cell(5, CellKind::Aisle, [850.0, 850.0, 1227.0, 645.0], 0.0, Tile::Declared),
    ]
}

fn edges() -> Vec<Edge> {
    vec![
        Edge {
            from_mm: [425.0, 425.0],
            to_mm: [1275.0, 425.0],
            vtm_traversal: false,
            htm_traversal: HtmTraversal::Both,
        },
        Edge {
            from_mm: [1275.0, 425.0],
            to_mm: [2125.0, 425.0],
            vtm_traversal: true,
            htm_traversal: HtmTraversal::UnladenOnly,
        },
        Edge {
            from_mm: [350.0, 1200.0],
            to_mm: [1463.5, 1172.5],
            vtm_traversal: true,
            htm_traversal: HtmTraversal::None,
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
        landmarks: [(4, ContainerPose { position: [0.35, 1.2, 0.15], heading: 0.2, pitch: 0.0 })].into_iter().collect(),
        routes: vec![BeltRoute {
            from_cell: 4,
            to_cell: 5,
            points: vec![[0.35, 1.2, 0.15], [0.9, 1.2, 0.15], [1.4635, 1.1725, 0.12]],
        }],
    }];
    rack
}

fn underlay() -> Underlay {
    Underlay {
        steel_mm: vec![[850.0, 0.0, 850.0, 850.0]],
        equipment: vec![FloorPatch { rect_mm: [0.0, 1600.0, 2550.0, 120.0], kind: CellKind::StationArea }],
        building: vec![FloorPatch { rect_mm: [-500.0, -500.0, 4000.0, 3000.0], kind: CellKind::Blank }],
    }
}

fn scene() -> Scene {
    Scene::new(Floor::from_tiles(cells(), 850.0)).with_edges(edges()).with_rack(rack()).with_underlay(underlay())
}

fn robots(instant: usize) -> Vec<Robot> {
    let turn = instant as f32 * 0.37;
    let robot = |id, class, cell, pos_mm, heading_rad, highlight| Robot {
        id: RobotId(id),
        class,
        at: Place { cell, pos_mm },
        heading_rad,
        highlight,
    };
    vec![
        robot(7, BotType::Htm, 0, [2100.0 + instant as f64 * 130.0, 3200.0], turn, BotHighlight::Primary),
        robot(3, BotType::Vtm, 1, [1200.0, 400.0], -turn, BotHighlight::None),
        robot(9, BotType::Rtp, 5, [900.0, 1000.0], 1.1, BotHighlight::None),
    ]
}

fn totes(instant: usize) -> Vec<Tote> {
    let carried = match instant {
        0 => ToteLocation::InRack { cell: CellId(1), level: 0 },
        1 => ToteLocation::OnBot(RobotId(7)),
        _ => ToteLocation::AtRelay(CellId(2)),
    };
    let look = |container, designation, contents_units| ToteAppearance { container, designation, contents_units };
    vec![
        Tote { id: ToteId(11), at: carried, appearance: look(ContainerKind::Tote, Designation::Order, 3) },
        Tote {
            id: ToteId(12),
            at: ToteLocation::InRack { cell: CellId(1), level: 2 },
            appearance: look(ContainerKind::Tray, Designation::Inventory, 0),
        },
        Tote {
            id: ToteId(13),
            at: ToteLocation::AtCell(CellId(0)),
            appearance: look(ContainerKind::Carton, Designation::Order, 7),
        },
        Tote {
            id: ToteId(14),
            at: ToteLocation::OnConveyor(CellId(4)),
            appearance: look(ContainerKind::Tote, Designation::Inventory, 1),
        },
        Tote { id: ToteId(15), at: ToteLocation::OnBot(RobotId(3)), appearance: ToteAppearance::default() },
        Tote { id: ToteId(16), at: ToteLocation::Extracted, appearance: ToteAppearance::default() },
    ]
}

/// The one conveyor travel: under way at the second and third instants, not
/// yet at the first.
fn transit() -> ToteTransit {
    ToteTransit {
        tote: ToteId(14),
        from_cell: CellId(4),
        to_cell: CellId(5),
        start: SimTime::ms(1_500),
        end: SimTime::ms(3_500),
    }
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

/// What a viewer that measured this run's motion reports: waiting as a share
/// of the run's own maximum, speed as a share of 5 m/s. Deliberately not a
/// constant — a renderer that stopped asking would be found out.
struct Measured;

impl TrafficTints for Measured {
    fn intensity(&self, cell_id: u32, mode: HeatmapMode) -> f32 {
        match mode {
            HeatmapMode::HtmWait => (f64::from(cell_id) / 8.0) as f32,
            HeatmapMode::HtmSpeed => ((f64::from(cell_id) * 0.5 / 5.0) as f32).min(1.0),
            _ => -1.0,
        }
    }
}

fn overlays<'a>(mode: HeatmapMode, heat: &'a HeatmapData, reserved: &'a [bool]) -> Overlays<'a> {
    Overlays { heatmap: Some(heat), mode, traffic: &Measured, reserved_cells: reserved }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn camera(scene: &Scene) -> CameraUniforms {
    CameraState::new(scene.frame().clone()).build_uniforms(
        Viewport::new([0.0, 0.0], [1600.0, 1000.0]),
        1.0,
        ViewMode::State,
    )
}

/// The robots, the containers — a travelling one among them — and the cell
/// rows, at three instants.
#[test]
fn a_moment_uploads_the_bytes_desvis_uploaded_before_the_move() {
    let scene = scene();
    let camera = camera(&scene);
    let (heat, reserved) = (heat(), reserved());
    for instant in 0..3 {
        let robots = robots(instant);
        let totes = totes(instant);
        let moment = scene.moment(
            camera,
            None,
            &Playback {
                at_ms: at_ms(instant),
                robots: &robots,
                totes: &totes,
                transits: std::slice::from_ref(&transit()),
                overlays: overlays(HeatmapMode::Off, &heat, &reserved),
            },
        );
        assert_eq!(moment.htm_instance_count, 1);
        assert_eq!(bytemuck::cast_slice::<_, u8>(&moment.bot_instances), fixture(&format!("bots_{instant}.bin")));
        assert_eq!(totes::draw_instance_bytes(&moment.tote_instances), fixture(&format!("totes_{instant}.bin")));
        assert_eq!(bytemuck::cast_slice::<_, u8>(&moment.cell_instances), fixture("cells_off.bin"));
    }
}

/// The cell rows under each overlay, including the two measured tints the
/// renderer asks a viewer for.
#[test]
fn every_overlay_tints_the_cells_as_it_did_before_the_move() {
    let scene = scene();
    let (heat, reserved) = (heat(), reserved());
    for (name, mode) in [
        ("off", HeatmapMode::Off),
        ("combined", HeatmapMode::Combined),
        ("wait", HeatmapMode::HtmWait),
        ("speed", HeatmapMode::HtmSpeed),
    ] {
        let rows = scene.cell_rows(&overlays(mode, &heat, &reserved));
        assert_eq!(bytemuck::cast_slice::<_, u8>(&rows), fixture(&format!("cells_{name}.bin")), "overlay {name}");
    }
}

/// The three layers a viewer adds beside the floor: the hops' arrows, the
/// steel, and the ground composed under the cells.
#[test]
fn the_optional_layers_build_the_bytes_desvis_built_before_the_move() {
    let scene = scene();
    assert_eq!(arrow_instance_bytes(scene.edges(), scene.frame()), fixture("arrows.bin"));
    assert_eq!(rack_frames::frame_instance_bytes(&scene.rack().frames), fixture("rack_frames.bin"));
    let underlay = scene.underlay();
    let ground = compose_floor_instances(&underlay.steel_mm, &underlay.equipment, &underlay.building, scene.frame());
    assert_eq!(bytemuck::cast_slice::<_, u8>(&ground), fixture("underlay.bin"));
}
