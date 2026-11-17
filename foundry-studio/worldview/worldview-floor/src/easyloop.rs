//! Within `worldview-floor`, `easyloop` is THE CONVEYOR'S FIXED GEOMETRY,
//! measured in the DWG block's XY frame.
//!
//! Order totes remain on HTMs. Inventory totes belong to the simulation, so
//! neither containers, presentation tables nor an operator are static props.
//! The two central chutes slope toward the operator. Heights are provisional
//! visual dimensions; conveyor travel uses recorded task intervals.

use crate::props::{Builder, Legs, Z_DECK_TOP, Z_LEG_TOP};
use desgraph::CellId;
use desmap::WarehouseMap;
use dessim::worldmap::WorldMap;
use rustc_hash::FxHashSet;
use site_model::components::spatial::Pose;
use worldview_render::{BeltRoute, ContainerPose, EasyLoopBelt, RackBox, RackBoxKind as K, RackVisual, RenderFrame};

/// The drawing stays in physical millimetres; its handedness comes from the
/// dropoff cell's declared frame, and its position from that cell's loaded pose.
pub struct Placement {
    pub dropoff: CellId,
    at_mm: [f64; 2],
    reflected: [bool; 2],
}

impl Placement {
    pub fn load(loaded: &WorldMap) -> Result<Vec<Self>, String> {
        let mut seen = FxHashSet::default();
        let mut placements = Vec::new();
        for station in &loaded.map.stations {
            let Some(dropoff) = station.dropoff_cell else { continue };
            if !seen.insert(dropoff) {
                continue;
            }
            let entity_id = loaded
                .index
                .cell_entities
                .get(&dropoff)
                .ok_or_else(|| format!("EasyLoop dropoff {dropoff:?} has no document entity"))?;
            let world = &loaded.world;
            let entity = world
                .entities
                .get(entity_id)
                .ok_or_else(|| format!("EasyLoop dropoff {entity_id} is absent from the document"))?;
            let pose = entity
                .component::<Pose>("pose")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("EasyLoop dropoff {entity_id} has no pose"))?;
            let frame = pose
                .frame
                .as_ref()
                .or_else(|| world.frames.primary())
                .ok_or_else(|| format!("EasyLoop dropoff {entity_id} has no frame"))?;
            let direction = world.frames.direction_to_primary(frame, [1.0, 1.0]).map_err(|e| e.to_string())?;
            if direction.iter().any(|axis| !axis.is_finite() || *axis == 0.0) {
                return Err(format!("EasyLoop dropoff {entity_id} has a degenerate frame"));
            }
            let at = loaded
                .map
                .graph
                .cell(dropoff)
                .and_then(|c| c.world_pos)
                .ok_or_else(|| format!("EasyLoop dropoff {entity_id} has no loaded position"))?;
            placements.push(Self { dropoff, at_mm: [at.x_mm, at.y_mm], reflected: direction.map(|axis| axis < 0.0) });
        }
        Ok(placements)
    }

    fn reflect(&self, point: [f32; 2], frame: &RenderFrame) -> [f32; 2] {
        let anchor = frame.world_mm(self.at_mm[0], self.at_mm[1]);
        std::array::from_fn(|axis| if self.reflected[axis] { 2.0 * anchor[axis] - point[axis] } else { point[axis] })
    }

    pub fn boxes(&self, frame: &RenderFrame) -> Vec<RackBox> {
        easyloop_prop(self.at_mm, frame)
            .into_iter()
            .map(|mut b| {
                let centre = self.reflect([b.min[0] + b.size[0] / 2.0, b.min[1] + b.size[1] / 2.0], frame);
                b.min[0] = centre[0] - b.size[0] / 2.0;
                b.min[1] = centre[1] - b.size[1] / 2.0;
                let [sx, sy] = self.reflected.map(|reflected| if reflected { -1.0 } else { 1.0 });
                b.yaw = (sy * b.yaw.sin()).atan2(sx * b.yaw.cos());
                b
            })
            .collect()
    }

    pub fn add_to_scene(&self, map: &WarehouseMap, frame: &RenderFrame, rack: &mut RackVisual) {
        rack.frames.extend(self.boxes(frame));
        let stations: Vec<_> =
            map.stations.iter().filter(|station| station.dropoff_cell == Some(self.dropoff)).collect();
        if let Some(collection) = stations.iter().find_map(|station| station.collection_cell) {
            let seats: Vec<_> = stations
                .iter()
                .filter_map(|station| {
                    let at = map.graph.cell(station.position)?.world_pos?;
                    Some((station.position, [at.x_mm, at.y_mm]))
                })
                .collect();
            rack.easyloop_belts.push(self.belt(frame, collection, &seats));
        }
    }

    pub fn belt(&self, frame: &RenderFrame, collection: CellId, seats: &[(CellId, [f64; 2])]) -> EasyLoopBelt {
        let origin = [self.at_mm[0] - V_WEST_C, self.at_mm[1] - V_Y_N];
        let world = |p: [f64; 3]| {
            let xy = self.reflect(frame.world_mm(origin[0] + p[0], origin[1] + p[1]), frame);
            [xy[0], xy[1], frame.z_of_mm(p[2])]
        };
        let mut seats = seats.to_vec();
        seats.sort_by(|a, b| {
            let order = a.1[0].total_cmp(&b.1[0]);
            if self.reflected[0] { order.reverse() } else { order }
        });
        let mut belt = EasyLoopBelt::default();
        let heading = if self.reflected[1] { std::f32::consts::PI } else { 0.0 };
        belt.landmarks.insert(
            self.dropoff.0,
            ContainerPose { position: world([V_WEST_C, V_Y_N, Z_MAIN_TOP]), heading, pitch: 0.0 },
        );
        belt.landmarks.insert(
            collection.0,
            ContainerPose {
                position: world([V_EAST_C, V_Y_N, Z_MAIN_TOP]),
                heading: heading + std::f32::consts::PI,
                pitch: 0.0,
            },
        );
        for (side, (cell, _)) in seats.iter().enumerate() {
            let cx = PRESENT_CENTER_X + (side as f64 - 0.5) * DECK_W;
            let takeoff = H_SOUTH_C - DECK_W / 2.0;
            // The tote stops with its leading edge at the chute's end stop.
            let seat =
                [cx, takeoff - PRESENT_LEN + 225.0, Z_PRESENT_TOP + (Z_MAIN_TOP - Z_PRESENT_TOP) * 225.0 / PRESENT_LEN];
            let mut inbound = vec![[V_WEST_C, V_Y_N, Z_MAIN_TOP], [V_WEST_C, V_Y_S_WEST, Z_MAIN_TOP]];
            append_arc(&mut inbound, CURVE_A_C, 180.0, 270.0);
            inbound.extend([[cx, H_SOUTH_C, Z_MAIN_TOP], [cx, takeoff, Z_MAIN_TOP], seat]);
            let mut outbound = vec![
                seat,
                [cx, takeoff, Z_MAIN_TOP],
                [cx, H_NORTH_C, Z_MAIN_TOP],
                [CURVE_B_C[0], H_NORTH_C, Z_MAIN_TOP],
            ];
            append_arc(&mut outbound, CURVE_B_C, 270.0, 180.0);
            outbound.push([V_EAST_C, V_Y_N, Z_MAIN_TOP]);
            belt.landmarks.insert(
                cell.0,
                ContainerPose {
                    position: world(seat),
                    heading,
                    pitch: ((Z_MAIN_TOP - Z_PRESENT_TOP) / PRESENT_LEN).atan() as f32,
                },
            );
            belt.routes.push(BeltRoute {
                from_cell: self.dropoff.0,
                to_cell: cell.0,
                points: inbound.into_iter().map(world).collect(),
            });
            belt.routes.push(BeltRoute {
                from_cell: cell.0,
                to_cell: collection.0,
                points: outbound.into_iter().map(world).collect(),
            });
        }
        belt
    }
}

// ── Measured, from the block's local millimetre frame ──────────────────────

/// Belt deck width, rail to rail. The same on all four runs and on both
/// curves (the arcs' inner and outer radii differ by exactly this).
const DECK_W: f64 = 610.0;
/// Centreline x of the two vertical-arm belts.
const V_WEST_C: f64 = -840.0;
const V_EAST_C: f64 = 0.0;
/// North end of the vertical arm — where HTMs hand carriers on and off.
const V_Y_N: f64 = -1424.0;
/// Centreline y of the two horizontal-arm belts.
const H_SOUTH_C: f64 = -6659.4;
const H_NORTH_C: f64 = -5891.65;
/// East end of the horizontal arm.
const H_X_E: f64 = 5729.4;
/// The two corner arcs' centres, and the centreline radius they share.
/// Curve A carries the west vertical belt into the south horizontal belt,
/// curve B the east vertical belt into the north horizontal belt. Each arc
/// sweeps 180 to 270 degrees, so its 180-degree point is the vertical belt's
/// centreline and its 270-degree point the horizontal belt's — which is what
/// fixes the vertical arms' south ends below.
const CURVE_A_C: [f64; 2] = [255.0, -5564.4];
const CURVE_B_C: [f64; 2] = [1095.0, -4795.4];
const CURVE_R: f64 = 1095.0;
/// Each vertical belt runs to ITS OWN curve tangent. The long straight lines
/// in the block stop at y = -3985, but that is drawing segmentation: the arc
/// centres put the real tangents here.
const V_Y_S_WEST: f64 = -5564.4;
const V_Y_S_EAST: f64 = -4795.4;
/// The east-end transfer, spanning both belts to close the loop.
const EAST_XFER: [f64; 4] = [5470.0, 5729.4, -7181.9, -5620.4];
/// Provisional visual heights, reduced to match the reviewed robot scale.
const Z_MAIN_TOP: f64 = 647.5;
const Z_PRESENT_TOP: f64 = 575.0;
const PRESENT_CENTER_X: f64 = 3043.5;
const PRESENT_LEN: f64 = 600.0;
/// Chords per 90-degree curve. Ten keeps a 1095mm-radius corner visually
/// round while staying axis-box cheap.
const CURVE_SEGS: usize = 10;

fn append_arc(path: &mut Vec<[f64; 3]>, centre: [f64; 2], from: f64, to: f64) {
    for i in 1..=40 {
        let angle = (from + (to - from) * i as f64 / 40.0).to_radians();
        path.push([centre[0] + CURVE_R * angle.cos(), centre[1] + CURVE_R * angle.sin(), Z_MAIN_TOP]);
    }
}

pub fn easyloop_prop(dropoff_mm: [f64; 2], frame: &RenderFrame) -> Vec<RackBox> {
    let origin = [dropoff_mm[0] - V_WEST_C, dropoff_mm[1] - V_Y_N];
    let mut b = Builder::new(origin, frame).with_legs(Legs::None);

    // Both handoff ends are flat extensions of the main belt.
    for (cx, y_s) in [(V_WEST_C, V_Y_S_WEST), (V_EAST_C, V_Y_S_EAST)] {
        b.straight_belt([cx, V_Y_N], [cx, y_s], DECK_W);
    }
    // The two corners, each sweeping its vertical belt into its horizontal one.
    b.curved_belt(CURVE_A_C, CURVE_R, DECK_W, CURVE_SEGS);
    b.curved_belt(CURVE_B_C, CURVE_R, DECK_W, CURVE_SEGS);
    // The horizontal arm, each belt starting at its own curve's exit tangent.
    b.straight_belt([CURVE_A_C[0], H_SOUTH_C], [H_X_E, H_SOUTH_C], DECK_W);
    b.straight_belt([CURVE_B_C[0], H_NORTH_C], [H_X_E, H_NORTH_C], DECK_W);

    // The east-end transfer that closes the loop between the two belts.
    let [tx0, tx1, ty0, ty1] = EAST_XFER;
    b.push_rect([tx0, tx1, ty0, ty1], Z_LEG_TOP, Z_DECK_TOP, K::ConveyorTransfer);

    let mut boxes = b.into_boxes();
    for part in &mut boxes {
        part.min[2] += frame.z_of_mm(Z_MAIN_TOP - Z_DECK_TOP);
    }
    let mut ends = Builder::new(origin, frame).with_legs(Legs::None);
    let takeoff_y = H_SOUTH_C - DECK_W / 2.0;
    for side in [-1.0, 1.0] {
        let cx = PRESENT_CENTER_X + side * DECK_W / 2.0;
        ends.ramp(cx, takeoff_y, PRESENT_LEN, 8, Z_MAIN_TOP, Z_PRESENT_TOP, DECK_W);
        ends.push(
            cx,
            takeoff_y - PRESENT_LEN,
            Z_PRESENT_TOP,
            Z_PRESENT_TOP + 50.0,
            DECK_W,
            40.0,
            0.0,
            K::ConveyorGuard,
        );
    }
    boxes.extend(ends.into_boxes());
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::prop_bounds;

    #[test]
    fn order_presentations_stand_beside_the_inventory_chutes() {
        use crate::cells::tiles;
        use dessim::worldmap;
        use std::path::Path;

        for fixture in ["minimal_easyloop.json", "walmart_dfw6.json"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures").join(fixture);
            let w = worldmap::load(&path).unwrap();
            let frame = RenderFrame::from_extent_mm(1_000_000.0, 1_000_000.0);
            let cells = tiles(&w.map, &w.cell_sizes_mm, &w.cell_footprint_stated, &w.cell_headings_rad, 850.0);
            for placement in Placement::load(&w).unwrap() {
                let mut stops: Vec<_> = placement
                    .boxes(&frame)
                    .into_iter()
                    .filter(|part| {
                        part.kind == K::ConveyorGuard && (part.min[2] - frame.z_of_mm(Z_PRESENT_TOP)).abs() < 1e-5
                    })
                    .collect();
                stops.sort_by(|a, b| a.min[0].total_cmp(&b.min[0]));
                assert_eq!(stops.len(), 2, "the two inventory chute stops");
                let mut orders: Vec<_> = w
                    .map
                    .stations
                    .iter()
                    .filter(|s| s.dropoff_cell == Some(placement.dropoff))
                    .map(|s| cells[s.order_presentation_cell.unwrap().0 as usize].as_ref().unwrap())
                    .collect();
                orders.sort_by(|a, b| a.center_mm()[0].total_cmp(&b.center_mm()[0]));
                assert_eq!(orders.len(), 2);
                for (side, (cell, stop)) in orders.iter().zip(&stops).enumerate() {
                    let corners = cell.corners_mm();
                    let min_x = corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
                    let max_x = corners.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
                    let gap = if side == 0 {
                        stop.min[0] as f64 * 1000.0 - max_x
                    } else {
                        min_x - (stop.min[0] + stop.size[0]) as f64 * 1000.0
                    };
                    assert!((25.0..65.0).contains(&gap), "{fixture}: side {side} is beside the chute, gap {gap} mm");
                    let north_of_stop = cell.center_mm()[1] - (stop.min[1] + stop.size[1] / 2.0) as f64 * 1000.0;
                    assert!(
                        (80.0..140.0).contains(&north_of_stop),
                        "order cell is abreast the chute bottom, {north_of_stop} mm north"
                    );
                    let inward = if side == 0 { 1.0 } else { -1.0 };
                    assert!(
                        (0..4).any(|edge| {
                            let a = corners[edge];
                            let b = corners[(edge + 1) % 4];
                            let dx = b[0] - a[0];
                            let dy = b[1] - a[1];
                            let len = dx.hypot(dy);
                            ((inward * dy + dx) / len / 2.0_f64.sqrt()) > 0.999
                        }),
                        "an angled edge faces inward toward the operator"
                    );
                }
            }
        }
    }

    #[test]
    fn all_four_drawings_fit_their_declared_machine_envelopes() {
        use dessim::worldmap;
        use std::path::Path;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/walmart_dfw6.json");
        let loaded = worldmap::load(&path).expect("DFW6 loads");
        let frame = RenderFrame::from_extent_mm(1_000_000.0, 1_000_000.0);
        let placements = Placement::load(&loaded).expect("drawings have placements");
        assert_eq!(placements.len(), 4);
        assert_eq!(placements.iter().filter(|p| p.reflected[0]).count(), 2);
        for placement in placements {
            let envelope = loaded
                .equipment
                .iter()
                .find(|e| {
                    let [x, y, w, h] = e.rect_mm;
                    e.part == "envelope"
                        && placement.at_mm[0] >= x
                        && placement.at_mm[0] <= x + w
                        && placement.at_mm[1] >= y
                        && placement.at_mm[1] <= y + h
                })
                .expect("dropoff belongs to one envelope");
            let boxes = placement.boxes(&frame);
            assert!(boxes.iter().all(|b| !matches!(
                b.kind,
                K::InventoryTote | K::OrderTote | K::Operator | K::OperatorPlatform | K::PresentStand
            )));
            let [x0, y0, x1, y1] = prop_bounds(&boxes);
            let [left, bottom] = frame.mm_at_render(x0, y0);
            let [right, top] = frame.mm_at_render(x1, y1);
            let [x, y, w, h] = envelope.rect_mm;
            assert!(
                left >= x - 1.0 && right <= x + w + 1.0 && bottom >= y - 1.0 && top <= y + h + 1.0,
                "drawing {left},{bottom}..{right},{top} exceeds its envelope {:?}",
                envelope.rect_mm
            );
        }
    }

    /// The two corners are the load-bearing measurement: they are what makes
    /// the prop read as one continuous loop rather than four disjoint runs.
    /// Each arc's 180-degree point must land on its vertical belt's
    /// centreline and its 270-degree point on its horizontal belt's.
    #[test]
    fn curve_tangents_meet_the_straight_belts() {
        assert_eq!(CURVE_A_C[0] - CURVE_R, V_WEST_C, "curve A enters on the west belt");
        assert_eq!(CURVE_A_C[1] - CURVE_R, H_SOUTH_C, "curve A exits on the south belt");
        assert_eq!(CURVE_B_C[0] - CURVE_R, V_EAST_C, "curve B enters on the east belt");
        assert!(
            (CURVE_B_C[1] - CURVE_R - H_NORTH_C).abs() < 1.3,
            "curve B exits on the north belt (within the block's own 1.25mm rail spread)"
        );
        // And the vertical arms must reach those entry tangents.
        assert_eq!(V_Y_S_WEST, CURVE_A_C[1], "west arm runs to curve A's centre line");
        assert_eq!(V_Y_S_EAST, CURVE_B_C[1], "east arm runs to curve B's centre line");
    }

    /// The conveyor's ingress lands ON the dropoff cell it is anchored to.
    ///
    /// This replaced an envelope-relative check. The equipment envelope is the
    /// robot lattice padded symmetrically, and the machine is asymmetric, so
    /// the envelope was never its footprint -- anchored on it the prop sat
    /// 1,280mm west of its own envelope and 2.16m south of the handoff cells
    /// its belt is supposed to meet. A dropoff cell is a thing the map has.
    #[test]
    fn the_conveyor_ingress_lands_on_the_dropoff_cell() {
        let dropoff = [26_447.2_f64, 11_427.5_f64];
        let frame = RenderFrame::from_extent_mm(1_000_000.0, 1_000_000.0);
        let boxes = easyloop_prop(dropoff, &frame);
        assert!(!boxes.is_empty(), "the prop has geometry");

        let [x0, y0, x1, y1] = prop_bounds(&boxes);
        let (dx, dy) = ((dropoff[0] / 1000.0) as f32, (dropoff[1] / 1000.0) as f32);
        // The unload point is the block's north-west corner of the vertical
        // arm, so the dropoff sits at the prop's northern edge, within a belt
        // half-width of it.
        assert!(
            (y1 - dy).abs() < 0.6,
            "the belt's north end meets the dropoff: prop tops out at {y1:.2}m, cell at {dy:.2}m"
        );
        assert!(x0 <= dx && dx <= x1, "the dropoff lies within the machine's x span");
        assert!(y0 < dy, "and the machine extends south of it");
    }

    #[test]
    fn every_box_is_a_plausible_height() {
        let frame = RenderFrame::from_extent_mm(1_000_000.0, 1_000_000.0);
        for b in easyloop_prop([26_447.2, 11_427.5], &frame) {
            assert!(b.min[2] >= 0.0, "{:?} starts at or above the floor", b.kind);
            assert!(b.size[2] > 0.0, "{:?} has positive height", b.kind);
            assert!(
                b.min[2] + b.size[2] <= (Z_MAIN_TOP + 50.0) as f32 / 1000.0 + 1e-4,
                "{:?} fits below the conveyor guard top",
                b.kind
            );
        }
    }

    #[test]
    fn handoff_decks_are_flat_at_the_main_belt_height() {
        let frame = RenderFrame::from_extent_mm(1_000_000.0, 1_000_000.0);
        let dropoff = [26_447.2, 11_427.5];
        for part in easyloop_prop(dropoff, &frame).into_iter().filter(|part| part.kind == K::ConveyorDeck) {
            if part.min[1] > frame.world_mm(dropoff[0], dropoff[1] - 900.0)[1] {
                assert!((part.min[2] + part.size[2] - frame.z_of_mm(Z_MAIN_TOP)).abs() < 1e-5);
            }
        }
    }
}
