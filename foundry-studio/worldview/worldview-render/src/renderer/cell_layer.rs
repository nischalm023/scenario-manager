//! Within `renderer`, `cell_layer` is the CELL LAYER: the derived reservation
//! cells drawn as floor tiles, plus the free-form nodes that have a position
//! but no tile. This is the one module here that knows what a cell is.
//!
//! Cells are a solver artifact, not the renderer's coordinate system —
//! everything else here (bots, totes, racks, the camera, picking) works in
//! metres and never sees an index. But the cell layer is deliberately
//! first-class rather than a compatibility shim: reading which cells are
//! reserved, and where congestion is, is a large part of what a viewer is FOR,
//! and that data is intrinsically cell-shaped because the reservation system
//! is.
//!
//! The dense `width * depth` prefix ordering is load-bearing beyond drawing:
//! the heatmap and selection arrays index into it positionally, so
//! [`rewrite_with_heatmap`] must reproduce byte-identical geometry and
//! free-form nodes must be appended as a TAIL, never interleaved.

use bytemuck::{Pod, Zeroable};
use desgraph::types::CellKind;

use crate::frame::RenderFrame;
use crate::scene::{Cell, FloorPatch, HeatmapMode, Overlays, Scene, Tile};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct CellInstance {
    /// Low corner of the cell's floor tile, in render metres, Y already
    /// flipped. Was a `(grid_x, grid_y)` index pair, which only located a
    /// cell because every cell was exactly one unit wide.
    pub min_x: f32,
    pub min_y: f32,
    /// Tile size in metres. Per-instance because a rack row and a highway
    /// lane are genuinely different widths.
    pub size_x: f32,
    pub size_y: f32,
    pub cell_kind: u32,
    pub rack_levels: u32,
    pub heatmap_vtm: f32,
    pub heatmap_htm: f32,
    pub reserved: u32,
    /// Which of the three coplanar tiers this instance's GROUND SURFACE draws
    /// on, as a multiple of [`DEPTH_TIER`] — see that constant for why the
    /// unit is a slice of the DEPTH BUFFER and not a distance.
    ///
    /// `grid.wgsl` applies it to vertices at world z = 0 and to no others, so
    /// an extruded rack's floor face sits on its tier while the walls and top
    /// standing on it keep their true depth. It is per-instance rather than
    /// per-vertex because every instance has exactly one ground surface.
    pub depth_bias: f32,
    /// Rotation around the tile centre, in world radians.
    pub heading_rad: f32,
    pub _pad: u32, // 48-byte instance stride
}

/// One step of separation between coplanar tiers, as a fraction of the depth
/// buffer's own [0, 1] range — NOT a distance. `grid.wgsl` adds it after the
/// projection, so it means the same thing under both of the cameras here, and
/// only to the ground surface at z = 0, which is the only place surfaces are
/// coplanar.
///
/// **A world-space offset cannot do this job, and a millimetre of one was the
/// z-fight.** Depth32Float spends its mantissa near the near plane, and this
/// camera pairs a 0.1m `NEAR_PLANE` with a 10,000m `FAR_PLANE`. Measured
/// through these same matrices at a 0.6rad tilt, the 1mm underlay bias that
/// stood here until 2026-09-04 was worth 38 steps of the depth buffer at a 5m
/// camera, 2 at 20m, ONE from 30m to 50m, and NOTHING at 80m and beyond — past
/// which the underlay's depth and the cell's above it are the same float, and
/// which one a pixel gets is decided by the step of interpolation error that a
/// building-wide ground quad and an 0.85m tile happen to carry there. That is
/// the shredding Daniel reported on DFW6 (2026-09-04): "two surfaces at the
/// exact same position fighting between which should render, and the answer is
/// both sometimes". The orthographic top-down view keeps 13 steps even at a
/// 1000m camera, which is why looking straight down looks clean.
///
/// 2e-6 of the range measures 32 to 35 steps under the perspective camera at
/// every distance and every tilt, and hundreds to thousands under the
/// orthographic one — an order of magnitude clear of interpolation error. The
/// world distance it stands for grows with the zoom and stays under a pixel:
/// about 0.5mm at a 5m camera, 50mm at 50m, 0.8m at 200m. Nothing moves on
/// screen, because the bias is added to depth alone.
///
/// Three tiers use it, in the order a person looking down would name them:
/// markers over cell tiles over the underlay, the nearest of them one step
/// behind true z=0. `projection.rs`'s
/// `a_depth_tier_survives_the_depth_buffer_at_every_zoom` is the gate.
pub(super) const DEPTH_TIER: f32 = 2e-6;

// The three tiers, each a step BEHIND the last and ALL of them behind true
// z=0. Biasing backward rather than forward is what keeps this fix from
// breaking something else: everything that stands ON the floor -- a bot lifted
// `FLOOR_CLEARANCE_M`, a tote, a prop's leg, a bay frame -- draws later with
// `LessEqual`, so it beats a tie and stays in front.
//
// The nearest tier is one step back rather than zero because 10mm of clearance
// is not always in front of z=0 either. Looking UPWARD -- which the controls
// allow down to `MIN_ELEVATION`, -0.3 rad, lifting the orbit target to keep
// the eye above ground -- a higher point is FARTHER along the view direction,
// so a bot at `FLOOR_CLEARANCE_M` is behind the ground it stands on. Against a
// marker tier at zero bias it lost at every distance and came out muted
// through the bots' occluded pass, as though a rack were hiding it (Codex
// review, 2026-09-04). A tier of margin buys back everything from 13.5m out;
// closer than that, at the steepest upward tilt, it still loses.
// `projection.rs`'s `a_bot_standing_on_a_cell_is_never_behind_it` measures all
// of it and states the residual.

/// A marker square: the nearest tier, for a cell whose extent the document
/// never stated.
///
/// A cell the document places as a POINT — a port's Exchange cell, a
/// presentation point, a minted rack or aisle cell — stands inside floor
/// another cell declares, by design, and it is drawn as a square anyway so it
/// can be seen at all. On DFW6 that is 3,551 pairs of coplanar quads, most of
/// them minted rack cells inside aisle tiles.
///
/// Which cells these are is decided by PROVENANCE and never by footprint size:
/// see `types::Tile`. A further 364 DFW6 pairs are marker over MARKER — two
/// minted docking squares on one centre — which share this tier and are not
/// separated by it. Every one of those 364 is Aisle over Aisle, so the two
/// squares are the same colour and nothing in the picture is wrong; they are
/// redundant drawing.
pub(crate) const MARKER_BIAS: f32 = DEPTH_TIER;

/// A declared floor tile: one tier behind the markers standing in it.
///
/// This does NOT separate the 116 pairs where two STATED tiles claim one patch
/// of ground — they share this tier, and under the orbited camera they still
/// shred. That is a defect in the DOCUMENT (`dfw6_easyloop_floor.rs` lists
/// every pair: the station floor's own tiling against the four EasyLoops'
/// `order_queue` and `waiting_block` lattices standing in it); separating them
/// here would draw a floor the document does not have.
pub(crate) const TILE_BIAS: f32 = 2.0 * DEPTH_TIER;

/// The composed underlay: one tier behind the tiles. Only one underlay tier
/// exists — the base surface is composed CPU-side into disjoint rectangles, so
/// nothing within it ever overlaps. Cells draw their own color and their own
/// boundary lines (the shader's edge darkening); the underlay is only the
/// THINGS beneath them — steel, equipment strips, the building's ground.
pub(crate) const UNDERLAY_BIAS: f32 = 3.0 * DEPTH_TIER;

/// The depth tier a cell's ground surface draws on: the smaller, the nearer,
/// so where tiles overlap the one on the smallest is the one seen. Picking
/// reads it too (`CellIndex::drawn_at`), so a pick names the cell drawn.
pub(crate) fn ground_bias(tile: Tile) -> f32 {
    match tile {
        Tile::Declared => TILE_BIAS,
        Tile::Marker => MARKER_BIAS,
    }
}

/// Per-vertex data for the box mesh (shared across all cell instances).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(super) struct BoxVertex {
    pub pos: [f32; 3], // x, y, z_factor (0=bottom, 1=top)
    pub face_id: u32,  // 0=top, 1=front(+Y), 2=back(-Y), 3=right(+X), 4=left(-X)
}

pub(super) fn cell_kind_to_u32(kind: CellKind) -> u32 {
    match kind {
        CellKind::Highway => 0,
        CellKind::Aisle => 1,
        CellKind::Open => 2,
        CellKind::Storage => 3,
        CellKind::Relay => 4,
        CellKind::StationArea => 5,
        CellKind::Junction => 6,
        CellKind::Blank => 7,
        CellKind::Presentation => 8,
        CellKind::QueueCell => 9,
        CellKind::Charging => 11,
        CellKind::Exchange => 12,
    }
}

/// One instance per CELL ID, dense: instance index == cell id, so the
/// heatmap path rewrites by id and the buffer never learns about rows or
/// columns. A missing id draws nothing (degenerate size) -- kind 8 as a
/// "None" placeholder was Presentation orange and once painted 14,688 empty
/// slots as a solid sheet. Every cell carries its own tile (computed once at
/// load), so this list is the whole floor: lattice, derived and point cells
/// alike, point cells at their nominal marker square.
fn instance_of(info: &Cell, frame: &RenderFrame, rack_levels: u32) -> CellInstance {
    let kind = cell_kind_to_u32(info.kind);
    let depth_bias = ground_bias(info.tile);
    let levels = if kind == 3 || kind == 4 { rack_levels } else { 0 };
    let [x0, y0] = frame.world_mm(info.tile_mm[0], info.tile_mm[1]);
    let [x1, y1] = frame.world_mm(info.tile_mm[0] + info.tile_mm[2], info.tile_mm[1] + info.tile_mm[3]);
    CellInstance {
        min_x: x0.min(x1),
        min_y: y0.min(y1),
        size_x: (x1 - x0).abs(),
        size_y: (y1 - y0).abs(),
        cell_kind: kind,
        rack_levels: levels,
        heatmap_vtm: 0.0,
        heatmap_htm: 0.0,
        reserved: 0,
        depth_bias,
        heading_rad: info.heading_rad as f32,
        _pad: 0,
    }
}

const EMPTY_SLOT: CellInstance = CellInstance {
    min_x: 0.0,
    min_y: 0.0,
    size_x: 0.0,
    size_y: 0.0,
    cell_kind: 2,
    rack_levels: 0,
    heatmap_vtm: 0.0,
    heatmap_htm: 0.0,
    reserved: 0,
    depth_bias: 0.0,
    heading_rad: 0.0,
    _pad: 0,
};

/// Axis-aligned rectangle difference: every piece of `pieces` minus `cut`,
/// as up-to-4 disjoint bands per intersected piece. `[x, y, w, h]` in mm.
fn subtract_rect(pieces: Vec<[f64; 4]>, cut: [f64; 4]) -> Vec<[f64; 4]> {
    let (cx0, cy0, cx1, cy1) = (cut[0], cut[1], cut[0] + cut[2], cut[1] + cut[3]);
    let mut out = Vec::with_capacity(pieces.len() + 3);
    for p in pieces {
        let (px0, py0, px1, py1) = (p[0], p[1], p[0] + p[2], p[1] + p[3]);
        if cx0 >= px1 || cx1 <= px0 || cy0 >= py1 || cy1 <= py0 {
            out.push(p);
            continue;
        }
        // Bands below, above, left, right of the cut -- sharing the cut's
        // own coordinates, so the decomposition rasterizes watertight.
        if cy0 > py0 {
            out.push([px0, py0, px1 - px0, cy0 - py0]);
        }
        if cy1 < py1 {
            out.push([px0, cy1, px1 - px0, py1 - cy1]);
        }
        let (by0, by1) = (cy0.max(py0), cy1.min(py1));
        if cx0 > px0 {
            out.push([px0, by0, cx0 - px0, by1 - by0]);
        }
        if cx1 < px1 {
            out.push([cx1, by0, px1 - cx1, by1 - by0]);
        }
    }
    out
}

/// The composed underlay: ONE disjoint layer of the THINGS the cells stand
/// among — steel footprints, equipment strips, and the building's ground.
/// Priority is steel > building; each lower layer is drawn MINUS everything
/// above it, so no fragment is ever drawn twice and depth never has to
/// arbitrate within the tier.
///
/// A tiled region deliberately paints NOTHING here. A region is a rule, not
/// a thing: its cells carry their own color and boundary, and where its
/// tiling carves a hole — or wherever else no cell stands — what shows is
/// what is really there: the building's ground inside the walls, the void
/// outside them. Painting region rectangles used to shade carve-outs and
/// end caps with floor that the document says does not exist.
pub(super) fn compose_floor_instances(
    steel_mm: &[[f64; 4]],
    equipment: &[FloorPatch],
    building: &[FloorPatch],
    frame: &RenderFrame,
) -> Vec<CellInstance> {
    let mut rects: Vec<([f64; 4], CellKind)> = Vec::new();

    // Steel footprints, as-is: the top of the base stack. The rectangle is
    // the dimensional truth a map check needs; HEIGHT belongs to the bay
    // frames (RackVisual), not the floor stack.
    for rect_mm in steel_mm {
        rects.push((*rect_mm, CellKind::Storage));
    }
    // Equipment border strips (EasyLoop envelopes), same tier as steel: thin
    // and pre-cut by the producer, so nothing here overlaps; the building
    // below subtracts them so the outline survives whatever it crosses.
    for f in equipment {
        rects.push((f.rect_mm, f.kind));
    }
    // The building's ground minus the equipment and steel standing on it:
    // everything inside the walls, under the cells and showing through
    // wherever no cell stands.
    for f in building {
        let mut pieces = vec![f.rect_mm];
        for cut in equipment.iter().map(|e| e.rect_mm).chain(steel_mm.iter().copied()) {
            pieces = subtract_rect(pieces, cut);
        }
        for p in pieces {
            rects.push((p, f.kind));
        }
    }

    rects
        .into_iter()
        .map(|(r, kind)| {
            let [x0, y0] = frame.world_mm(r[0], r[1]);
            let [x1, y1] = frame.world_mm(r[0] + r[2], r[1] + r[3]);
            CellInstance {
                min_x: x0.min(x1),
                min_y: y0.min(y1),
                size_x: (x1 - x0).abs(),
                size_y: (y1 - y0).abs(),
                cell_kind: cell_kind_to_u32(kind),
                rack_levels: 0,
                heatmap_vtm: 0.0,
                heatmap_htm: -1.0,
                reserved: 0,
                depth_bias: UNDERLAY_BIAS,
                heading_rad: 0.0,
                _pad: 0,
            }
        })
        .collect()
}

pub fn build_cell_instances(cells: &[Option<Cell>], frame: &RenderFrame, rack_levels: u32) -> Vec<CellInstance> {
    cells.iter().map(|slot| slot.as_ref().map_or(EMPTY_SLOT, |info| instance_of(info, frame, rack_levels))).collect()
}

/// Rebuild the per-cell prefix with the overlay tints and reservation flags
/// folded in, BY CELL ID. MUST reproduce `build_cell_instances`'s geometry
/// exactly -- it overwrites the same buffer -- which is why both call
/// `instance_of`.
pub(crate) fn rewrite_with_overlays(scene: &Scene, overlays: &Overlays) -> Vec<CellInstance> {
    let &Overlays { heatmap, mode, traffic, reserved_cells } = overlays;
    let (cell_infos, frame, rack_levels) = (scene.cells(), scene.frame(), scene.rack().uniform_levels);
    let global_max = heatmap.map(|hm| hm.vtm_max.max(hm.htm_max).max(1) as f32).unwrap_or(1.0);
    let show_vtm = matches!(mode, HeatmapMode::VtmOnly | HeatmapMode::Combined);
    let show_htm = matches!(mode, HeatmapMode::HtmOnly | HeatmapMode::Combined);

    cell_infos
        .iter()
        .enumerate()
        .map(|(id, slot)| {
            let Some(info) = slot.as_ref() else { return EMPTY_SLOT };
            let mut inst = instance_of(info, frame, rack_levels);
            if let Some(hm) = heatmap {
                if show_vtm {
                    inst.heatmap_vtm = hm.vtm_counts.get(id).copied().unwrap_or(0) as f32 / global_max;
                }
                if show_htm {
                    inst.heatmap_htm = hm.htm_counts.get(id).copied().unwrap_or(0) as f32 / global_max;
                }
            }
            if matches!(mode, HeatmapMode::HtmWait | HeatmapMode::HtmSpeed) {
                inst.heatmap_htm = traffic.intensity(id as u32, mode);
            }
            inst.reserved = u32::from(reserved_cells.get(id).copied().unwrap_or(false));
            inst
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::RenderFrame;

    /// An equipment border strip survives the building's ground painted
    /// across it: the strip draws as-is on the top tier, and no ground piece
    /// overlaps it. This is the property that makes the EasyLoop bounding box
    /// visible over the ground it necessarily crosses. (That tiled regions
    /// paint nothing at all is structural now: the composition has no
    /// parameter to hand them to.)
    #[test]
    fn equipment_strips_survive_the_building_ground() {
        let strip = FloorPatch { rect_mm: [1_000.0, 1_000.0, 5_000.0, 300.0], kind: CellKind::StationArea };
        let ground = FloorPatch { rect_mm: [0.0, 0.0, 20_000.0, 20_000.0], kind: CellKind::Blank };
        let frame = RenderFrame::from_extent_mm(20_000.0, 20_000.0);
        let instances = compose_floor_instances(&[], &[strip], &[ground], &frame);

        let strips: Vec<_> =
            instances.iter().filter(|i| i.cell_kind == cell_kind_to_u32(CellKind::StationArea)).collect();
        assert_eq!(strips.len(), 1, "the strip draws exactly once");

        // No ground piece may cover the strip's interior.
        let sx = frame.world_mm(1_000.0 + 2_500.0, 1_000.0 + 150.0); // strip centre
        for i in instances.iter().filter(|i| i.cell_kind == cell_kind_to_u32(CellKind::Blank)) {
            let inside = sx[0] > i.min_x && sx[0] < i.min_x + i.size_x && sx[1] > i.min_y && sx[1] < i.min_y + i.size_y;
            assert!(!inside, "a ground piece covers the strip centre: {:?}", (i.min_x, i.min_y, i.size_x, i.size_y));
        }
    }
}
