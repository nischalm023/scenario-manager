//! Within `renderer`, an INSTANCE is one row of per-object attributes that a
//! single instanced draw call fans a shared mesh out over: one cell, one
//! container, one arrow. This module owns their `#[repr(C)]` layouts (which
//! must match the `@location` bindings in `shaders/`) and turns a scene and an
//! instant into those rows — the one place either viewer does it.
//!
//! It resolves position, never state. A container's `world_z` and `tote_type`
//! come from the [`ToteLocation`] its run reported -- so a robot looks laden
//! iff some container's location names it, and a flow that moves a container
//! without reporting one is invisible here rather than wrong here.

use bytemuck::{Pod, Zeroable};
use desgraph::types::{CellKind, HtmTraversal};
use rustc_hash::FxHashMap;
use worldview::frames::playback::ToteLocation;

use super::htm_mesh;
use crate::frame::{FLOOR_CLEARANCE_M, RenderFrame};
use crate::rack::LEVEL_HEIGHT;
use crate::scene::{BotType, ContainerKind, Designation, Edge, Place, Robot, Scene, Tote, ToteAppearance};

// ---------------------------------------------------------------------------
// Bot instances
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct BotInstance {
    pub world_x: f32,
    pub world_y: f32,
    pub world_z: f32,
    pub bot_type: u32,
    /// Selection highlight: 0 = none, 1 = primary (the selected bot),
    /// 2 = secondary (an inbound HTM of the selected station). The shader
    /// tints and, for primary, brightens.
    pub highlight: u32,
    /// Facing in radians: 0 points along +x, turning toward +y. The shader
    /// spins the glyph about +z by this, so an HTM sweeping a `Rotate` step
    /// is seen to turn.
    pub heading: f32,
}

fn bot_type_to_u32(bt: BotType) -> u32 {
    match bt {
        BotType::Vtm => 0,
        BotType::Htm => 1,
        BotType::Rtp => 2,
    }
}

/// Where a mover stands, in render metres: the position its viewer stated,
/// converted from world millimetres.
pub fn place_pos(scene: &Scene, at: Place) -> [f32; 2] {
    let [x_mm, y_mm] = at.pos_mm;
    scene.frame().world_mm(x_mm, y_mm)
}

/// The existing rack-top proxy height, located by physical position on world
/// maps. A reporting anchor alone cannot put a VTM on top of a rack.
pub fn place_z(class: BotType, at: Place, scene: &Scene) -> f32 {
    if class != BotType::Vtm {
        return FLOOR_CLEARANCE_M;
    }
    let rack = scene.rack();
    let [x, y] = at.pos_mm;
    let on_rack = if rack.steel_mm.is_empty() {
        scene.floor().cell(at.cell).is_some_and(|cell| matches!(cell.kind, CellKind::Storage | CellKind::Relay))
    } else {
        rack.steel_mm.iter().any(|[rx, ry, w, h]| x >= *rx && x <= rx + w && y >= *ry && y <= ry + h)
    };
    if on_rack { rack.top_z + FLOOR_CLEARANCE_M } else { FLOOR_CLEARANCE_M }
}

/// Build the bot instance buffer, HTMs first, and report how many those are.
///
/// The two bot glyphs are two draws — HTMs get the proxy mesh, VTMs and RTPs
/// the cylinder — so the instances they read have to be contiguous, and the
/// returned count is where one draw's range ends and the other's begins.
pub fn build_bot_instances(robots: &[Robot], scene: &Scene) -> (Vec<BotInstance>, u32) {
    let to_instance = |robot: &Robot| {
        let [wx, wy] = place_pos(scene, robot.at);
        BotInstance {
            world_x: wx,
            world_y: wy,
            world_z: place_z(robot.class, robot.at, scene),
            bot_type: bot_type_to_u32(robot.class),
            highlight: robot.highlight.to_u32(),
            heading: robot.heading_rad,
        }
    };

    let (htms, others): (Vec<&Robot>, Vec<&Robot>) = robots.iter().partition(|r| r.class == BotType::Htm);
    let htm_count = htms.len() as u32;
    (htms.into_iter().chain(others).map(to_instance).collect(), htm_count)
}

// ---------------------------------------------------------------------------
// Tote instances
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct ToteInstance {
    pub tote_id: u32,
    /// Actual physical units at the playback cursor; geometry never invents a fill.
    pub contents_units: u64,
    pub world_x: f32,
    pub world_y: f32,
    pub world_z: f32,
    pub tote_type: u32, // 0=InRack, 1=AtRelay, 2=OnVtm, 3=OnHtm, 4=OnConveyor
    /// 1 for a tote designated ORDER, 0 for inventory.
    ///
    /// A separate field rather than more `tote_type` variants: the two are
    /// independent axes. `tote_type` says where a carrier is riding, and the
    /// designation says which flow it belongs to, and every combination of the
    /// two occurs.
    pub designation: u32,
    /// Carried totes share their HTM's current yaw, in radians about +z.
    pub heading: f32,
    /// Inclination about local X, applied before yaw.
    pub pitch: f32,
    /// Physical container: 0 = reusable tote, 1 = tray, 2 = cardboard carton.
    pub physical_type: u32,
}

/// The shader's code for the physical container a load rides in.
fn physical_type(appearance: ToteAppearance) -> u32 {
    match appearance.container {
        ContainerKind::Tote => 0,
        ContainerKind::Tray => 1,
        ContainerKind::Carton => 2,
    }
}

/// VTM slab height in world units. MUST match the slab half-extent math in
/// `shaders/bots.wgsl` (`vs_main`, the Vtm branch) — a carried tote rides
/// on top of the slab.
const VTM_SLAB_HEIGHT: f32 = 0.02;

/// Height of an HTM's load deck, in metres — where a carried tote sits.
const HTM_DECK_Z_M: f32 = 0.15;

/// Height of an EasyLoop's belt deck, in metres. NOT measured: the EasyLoop
/// DWG is pure 2D and states no heights anywhere. Set equal to the HTM deck so
/// a tote does not jump vertically across the handoff it is meant to make
/// look continuous. Replace once a real elevation exists.
const EASYLOOP_BELT_Z_M: f32 = HTM_DECK_Z_M;

/// Build the container instance rows for one instant.
///
/// `totes`: where every container is and what it looks like — pre-sorted by
/// container id so per-cell occupancy indices are stable frame to frame.
/// `robots`: this instant's robots, for placing a carried container.
/// `scene`: the floor a container at a cell sits on, and the rack model that
/// resolves shelf, relay and ride heights.
pub fn build_tote_instances(totes: &[Tote], robots: &[Robot], scene: &Scene) -> Vec<ToteInstance> {
    let frame = scene.frame();
    let rack = scene.rack();
    // Every cell knows its own tile; a container at a cell sits at its centre.
    let cell_xy = |cell_id: u32| -> Option<[f32; 2]> {
        let cell = scene.floor().cell(cell_id)?;
        let c = cell.center_mm();
        Some(frame.world_mm(c[0], c[1]))
    };
    // n-th in-rack tote seen per cell, spread across the cell's slot floors.
    let mut occupancy: FxHashMap<u32, usize> = FxHashMap::default();

    totes
        .iter()
        .filter_map(|tote| {
            let tote_id = tote.id.0;
            let appearance = tote.appearance;
            let designation = u32::from(appearance.designation == Designation::Order);
            let physical_type = physical_type(appearance);
            match tote.at {
                ToteLocation::InRack { cell, level } => {
                    let cell_id = cell.0;
                    let occupancy_index = {
                        let slot_cursor = occupancy.entry(cell_id).or_insert(0);
                        let index = *slot_cursor;
                        *slot_cursor += 1;
                        index
                    };
                    // Explicit-slot maps: the tote sits at a real slot
                    // position (x along the run AND beam height). Uniform
                    // maps keep the legacy cell-center ladder.
                    if let Some([sx, sy, sz]) = rack.in_rack_slot_pos(cell_id, occupancy_index) {
                        return Some(ToteInstance {
                            tote_id,
                            contents_units: appearance.contents_units,
                            world_x: sx,
                            world_y: sy,
                            world_z: sz,
                            tote_type: 0,
                            designation,
                            heading: 0.0,
                            pitch: 0.0,
                            physical_type,
                        });
                    }
                    let [wx, wy] = cell_xy(cell_id)?;
                    let wz = (level as f32 + 1.0) * LEVEL_HEIGHT;
                    Some(ToteInstance {
                        tote_id,
                        contents_units: appearance.contents_units,
                        world_x: wx,
                        world_y: wy,
                        world_z: wz,
                        tote_type: 0,
                        designation,
                        heading: 0.0,
                        pitch: 0.0,
                        physical_type,
                    })
                }
                ToteLocation::AtRelay(cell) => {
                    let cell_id = cell.0;
                    // Placed maps put the tote on the relay's board-0 tab.
                    if let Some([sx, sy, sz]) = rack.relay_tote_pos.get(&cell_id).copied() {
                        return Some(ToteInstance {
                            tote_id,
                            contents_units: appearance.contents_units,
                            world_x: sx,
                            world_y: sy,
                            world_z: sz,
                            tote_type: 1,
                            designation,
                            heading: 0.0,
                            pitch: 0.0,
                            physical_type,
                        });
                    }
                    let [wx, wy] = cell_xy(cell_id)?;
                    Some(ToteInstance {
                        tote_id,
                        contents_units: appearance.contents_units,
                        world_x: wx,
                        world_y: wy,
                        world_z: rack.relay_tote_z(),
                        tote_type: 1,
                        designation,
                        heading: 0.0,
                        pitch: 0.0,
                        physical_type,
                    })
                }
                ToteLocation::AtCell(cell) => {
                    let cell_id = cell.0;
                    let [world_x, world_y] = cell_xy(cell_id)?;
                    Some(ToteInstance {
                        tote_id,
                        contents_units: appearance.contents_units,
                        world_x,
                        world_y,
                        world_z: FLOOR_CLEARANCE_M,
                        tote_type: 1,
                        designation,
                        heading: 0.0,
                        pitch: 0.0,
                        physical_type,
                    })
                }
                ToteLocation::OnBot(carrier) => {
                    // Custody is the container's own statement; the robot it
                    // names says where that is.
                    let bot = robots.iter().find(|robot| robot.id == carrier)?;
                    let [wx, wy] = place_pos(scene, bot.at);
                    let is_vtm = bot.class == BotType::Vtm;
                    let wz = if is_vtm {
                        // VTMs always operate on top of the rack structure;
                        // tote rides on the VTM slab.
                        rack.top_z + FLOOR_CLEARANCE_M + VTM_SLAB_HEIGHT
                    } else {
                        // HTM at floor level; tote rides on its deck.
                        htm_mesh::DECK_Z
                    };
                    let tote_type = if is_vtm { 2 } else { 3 };
                    Some(ToteInstance {
                        tote_id,
                        contents_units: appearance.contents_units,
                        world_x: wx,
                        world_y: wy,
                        world_z: wz,
                        tote_type,
                        designation,
                        heading: if bot.class == BotType::Htm { bot.heading_rad } else { 0.0 },
                        pitch: 0.0,
                        physical_type,
                    })
                }
                ToteLocation::OnConveyor(cell) => {
                    let cell_id = cell.0;
                    let pose = rack.easyloop_belts.iter().find_map(|belt| belt.landmarks.get(&cell_id));
                    let [wx, wy, wz] = match pose {
                        Some(pose) => pose.position,
                        None => {
                            let [x, y] = cell_xy(cell_id)?;
                            [x, y, EASYLOOP_BELT_Z_M]
                        }
                    };
                    Some(ToteInstance {
                        tote_id,
                        contents_units: appearance.contents_units,
                        world_x: wx,
                        world_y: wy,
                        world_z: wz,
                        tote_type: 4,
                        designation,
                        heading: pose.map_or(0.0, |pose| pose.heading),
                        pitch: pose.map_or(0.0, |pose| pose.pitch),
                        physical_type,
                    })
                }
                ToteLocation::Extracted => None,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Arrow instances
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct ArrowInstance {
    /// Source cell centre in render metres, Y already flipped.
    pub from_x: f32,
    pub from_y: f32,
    /// Vector to the target cell centre, in metres. No longer a unit step:
    /// its length is the real centre-to-centre distance, so the shader
    /// normalizes rather than assuming it.
    pub dx: f32,
    pub dy: f32,
    /// Local scale for the glyph, in metres — the smaller of the two pitches
    /// around the source cell. Replaces the hard-coded cell fractions, which
    /// meant nothing once a cell stopped being one unit.
    pub scale: f32,
}

pub(super) fn edge_to_arrow(e: &Edge, frame: &RenderFrame) -> ArrowInstance {
    // Straight from the edge's own mm endpoints, so an edge touching a
    // free-form node draws like any other — it has no grid index to look up.
    let [fx, fy] = frame.world_mm(e.from_mm[0], e.from_mm[1]);
    let [tx, ty] = frame.world_mm(e.to_mm[0], e.to_mm[1]);
    // Glyph scale from the edge's own length: adjacent cells sit one local
    // pitch apart, so the hop distance IS the local pitch, gridless or not.
    let scale = (tx - fx).hypot(ty - fy);
    ArrowInstance { from_x: fx, from_y: fy, dx: tx - fx, dy: ty - fy, scale }
}

/// Precomputed arrow instance ranges for each view mode.
/// Stored contiguously: [unladen_htm edges | laden_htm edges | vtm edges]
pub struct ArrowModeRanges {
    pub unladen_htm_count: u32, // view_mode 1: htm_traversal != None
    pub laden_htm_count: u32,   // view_mode 2: htm_traversal == Both
    pub vtm_count: u32,         // view_mode 3: vtm_traversal == true
    pub unladen_htm_start: u32,
    pub laden_htm_start: u32,
    pub vtm_start: u32,
}

pub(super) fn build_arrow_instances_by_mode(
    edges: &[Edge],
    frame: &RenderFrame,
) -> (Vec<ArrowInstance>, ArrowModeRanges) {
    let unladen: Vec<_> =
        edges.iter().filter(|e| e.htm_traversal != HtmTraversal::None).map(|e| edge_to_arrow(e, frame)).collect();
    let laden: Vec<_> =
        edges.iter().filter(|e| e.htm_traversal == HtmTraversal::Both).map(|e| edge_to_arrow(e, frame)).collect();
    let vtm: Vec<_> = edges.iter().filter(|e| e.vtm_traversal).map(|e| edge_to_arrow(e, frame)).collect();

    let ranges = ArrowModeRanges {
        unladen_htm_start: 0,
        unladen_htm_count: unladen.len() as u32,
        laden_htm_start: unladen.len() as u32,
        laden_htm_count: laden.len() as u32,
        vtm_start: (unladen.len() + laden.len()) as u32,
        vtm_count: vtm.len() as u32,
    };

    let mut all = Vec::with_capacity(unladen.len() + laden.len() + vtm.len());
    all.extend(unladen);
    all.extend(laden);
    all.extend(vtm);
    (all, ranges)
}

/// The arrow rows a scene's hops draw, as bytes, for the golden test.
#[cfg(test)]
pub(super) fn arrow_instance_bytes(edges: &[Edge], frame: &RenderFrame) -> Vec<u8> {
    bytemuck::cast_slice(&build_arrow_instances_by_mode(edges, frame).0).to_vec()
}

#[cfg(test)]
mod tests {
    use desgraph::CellId;
    use desunits::{RobotId, ToteId};

    use super::*;
    use crate::rack::RackVisual;
    use crate::scene::{BotHighlight, Floor};

    fn scene(rack: RackVisual) -> Scene {
        Scene::new(Floor::default()).with_rack(rack)
    }

    fn htm(id: u32, pos_mm: [f64; 2], heading_rad: f32) -> Robot {
        Robot {
            id: RobotId(id),
            class: BotType::Htm,
            at: Place { cell: 0, pos_mm },
            heading_rad,
            highlight: BotHighlight::None,
        }
    }

    fn tote(id: u32, at: ToteLocation, appearance: ToteAppearance) -> Tote {
        Tote { id: ToteId(id), at, appearance }
    }

    #[test]
    fn carried_tote_shares_its_htm_pose_through_presentation_turns() {
        let scene = scene(RackVisual::uniform(5));
        let appearance = ToteAppearance { designation: Designation::Order, ..Default::default() };
        for degrees in [0.0_f32, 45.0, 90.0, 135.0, 180.0] {
            let robots = [htm(7, [2100.0, 3200.0], degrees.to_radians())];
            let (built, _) = build_bot_instances(&robots, &scene);
            let totes = build_tote_instances(&[tote(42, ToteLocation::OnBot(RobotId(7)), appearance)], &robots, &scene);
            assert_eq!(totes.len(), 1);
            assert_eq!([totes[0].world_x, totes[0].world_y], [built[0].world_x, built[0].world_y]);
            assert_eq!(totes[0].heading, built[0].heading);
            assert_eq!(totes[0].world_z, htm_mesh::DECK_Z);
            assert_eq!(totes[0].designation, 1);
        }
    }

    #[test]
    fn tote_heading_follows_custody_without_leaving_a_load_on_the_previous_htm() {
        let mut rack = RackVisual::uniform(5);
        rack.relay_tote_pos.insert(20, [7.0, 8.0, 9.0]);
        let scene = scene(rack);
        let robots = [htm(7, [1000.0, 2000.0], 0.7), htm(8, [3000.0, 4000.0], -1.2)];
        for (at, heading, xy, kind) in [
            (ToteLocation::OnBot(RobotId(7)), 0.7, [1.0, 2.0], 3),
            (ToteLocation::AtRelay(CellId(20)), 0.0, [7.0, 8.0], 1),
            (ToteLocation::OnBot(RobotId(8)), -1.2, [3.0, 4.0], 3),
        ] {
            let totes = build_tote_instances(&[tote(42, at, ToteAppearance::default())], &robots, &scene);
            assert_eq!(totes.len(), 1);
            assert_eq!(totes[0].heading, heading);
            assert_eq!([totes[0].world_x, totes[0].world_y], xy);
            assert_eq!(totes[0].tote_type, kind);
        }
        assert!(build_tote_instances(&[], &robots, &scene).is_empty());
    }

    #[test]
    fn cartons_and_trays_keep_their_type_on_the_same_order_htm() {
        let scene = scene(RackVisual::uniform(5));
        let robots = [htm(7, [2100.0, 3200.0], 0.7)];
        for (container, expected) in [(ContainerKind::Tote, 0), (ContainerKind::Tray, 1), (ContainerKind::Carton, 2)] {
            let appearance = ToteAppearance { container, designation: Designation::Order, contents_units: 7 };
            let instances =
                build_tote_instances(&[tote(42, ToteLocation::OnBot(RobotId(7)), appearance)], &robots, &scene);
            assert_eq!(instances.len(), 1);
            assert_eq!(instances[0].physical_type, expected);
            assert_eq!(instances[0].contents_units, 7);
            assert_eq!(instances[0].tote_id, 42);
            assert_eq!(instances[0].designation, 1);
            assert_eq!(instances[0].heading, robots[0].heading_rad);
            assert_eq!(instances[0].world_z, htm_mesh::DECK_Z);
            assert!(
                build_tote_instances(&[tote(42, ToteLocation::Extracted, appearance)], &robots, &scene).is_empty(),
                "extraction removes the container without leaving a load on its HTM"
            );
        }
    }

    #[test]
    fn a_rack_reporting_anchor_does_not_raise_an_aisle_vtm() {
        let mut rack = RackVisual::uniform(5);
        rack.steel_mm = vec![[1000.0, 0.0, 1000.0, 2000.0]];
        let top_z = rack.top_z;
        let scene = scene(rack);
        let beside = Place { cell: 0, pos_mm: [500.0, 1000.0] };
        let over = Place { cell: 0, pos_mm: [1500.0, 1000.0] };
        assert_eq!(place_z(BotType::Vtm, beside, &scene), FLOOR_CLEARANCE_M);
        assert_eq!(place_z(BotType::Vtm, over, &scene), top_z + FLOOR_CLEARANCE_M);
        assert_eq!(place_z(BotType::Htm, over, &scene), FLOOR_CLEARANCE_M);
    }

    /// The cell a robot names does not move it: two robots on the same cell
    /// draw a metre apart because their stated positions are a metre apart.
    #[test]
    fn the_cell_a_robot_names_does_not_move_it() {
        let floor = Floor::from_centres([(4, CellKind::Highway, [1275.0, 2125.0])], 850.0);
        let scene = Scene::new(floor).with_rack(RackVisual::uniform(2));
        assert_eq!(place_pos(&scene, Place { cell: 4, pos_mm: [1275.0, 2125.0] }), [1.275, 2.125]);
        assert_eq!(place_pos(&scene, Place { cell: 4, pos_mm: [2275.0, 2125.0] }), [2.275, 2.125]);
    }
}
