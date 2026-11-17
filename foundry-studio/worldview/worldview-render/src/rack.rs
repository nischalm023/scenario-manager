//! Within `worldview-render`, `rack` is the RACK AS IT IS DRAWN: the boxes of
//! steel a bay is made of, the heights a container rests at, and the belt
//! landmarks a conveyor presents at.
//!
//! It is a model, not a derivation: whoever loads a map works out where the
//! slots, boards and steel are, and leaves the answer here for the two
//! pipelines that read it — the bay frames draw [`RackBox`]es, and the
//! container instances ask [`RackVisual`] what height a place sits at.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Height per uniform rack level, in metres. Only meaningful on the uniform
/// fallback path; explicit-slot maps derive heights from the map itself.
pub const LEVEL_HEIGHT: f32 = 0.25;

/// One box of instanced box geometry in render space: a rack bay frame, or a
/// part of a station prop a viewer builds. Both feed the same
/// pipeline, which is why the type is shared rather than duplicated.
///
/// `yaw` rotates the box about its own XY centre — `min + size/2` — and is
/// zero for all rack geometry, which is axis-aligned by construction. Props
/// need it: the EasyLoop presents carriers at 45 degrees and turns its
/// conveyor through two 90-degree corners.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RackBox {
    pub min: [f32; 3],
    pub size: [f32; 3],
    /// Radians about the box's XY centre, counter-clockwise.
    pub yaw: f32,
    pub kind: RackBoxKind,
}

/// What a [`RackBox`] is, physically — drives its color in the shader.
/// Discriminants are the GPU instance `kind` values and must match the
/// `kind_color` switch in `shaders/rack_frames.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum RackBoxKind {
    /// Bay leg: bay-end vertical, floor to top beam.
    Upright,
    /// Storage beam: one shelf level across the bay.
    Beam,
    /// Slot-boundary vertical between adjacent slot positions.
    Divider,
    /// A diving board: the shelf at a relay's exchange point, cantilevered
    /// from the face it is served at INTO the bay. Not aisle-ward — that
    /// reading is what drew every DFW6 board outside its own steel.
    ///
    /// One relay is one exchange point and may carry several boards, one per
    /// board floor in its zone; on DFW6 the two are 1:1, which is why the two
    /// words get used for each other.
    BoardTab,
    // ── Station prop parts. Never rack steel; see `easyloop_prop`. ─────────
    /// Conveyor deck: the belt surface, straight run or curve chord.
    ConveyorDeck,
    /// Guard rail along a deck's long edge.
    ConveyorGuard,
    /// Conveyor or stand understructure.
    ConveyorLeg,
    /// The ramped deck section at an HTM handoff end.
    ConveyorRamp,
    /// A transfer bridging two belts.
    ConveyorTransfer,
    /// Decking on a conveyor's presentation side, or a deck's own frame skirt.
    PresentDeck,
    /// A transverse-roller chute from a take-off point to a seat.
    PresentChute,
    /// The top deck of a presentation stand.
    PresentStand,
    /// An inventory carrier, presented.
    InventoryTote,
    /// An order carrier, presented.
    OrderTote,
    /// The operator's standing platform.
    OperatorPlatform,
    /// The operator.
    Operator,
    /// A pick-to-light post and its head.
    PickToLight,
    /// One cross roller on a conveyor deck. Appended, not slotted in beside the
    /// other conveyor kinds: the discriminants ARE the GPU `kind` values, so a
    /// tidier position would silently recolour every kind after it.
    ConveyorRoller,
}

/// Rack render model, built once per map. Explicit-slot maps carry physical
/// bay geometry (`frames`) plus per-slot tote positions; cells themselves
/// stay flat (`uniform_levels == 0`). Slot-less maps keep the legacy
/// uniform cell extrusion (`uniform_levels == vtc`, everything else empty).
///
/// The default is NO RACK AT ALL — no steel, no shelves, no height — which is
/// what a viewer holding only the floor has: cells and movers, with nothing
/// standing on them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RackVisual {
    /// Uniform cell-extrusion level count — the per-cell 3D gate. 0 on
    /// explicit-slot maps: rack cells render as floor decals and the bay
    /// frames carry the 3D.
    pub uniform_levels: u32,
    /// Envelope top: top storage beam + tote headroom (uniform fallback:
    /// `(vtc + 1) * LEVEL_HEIGHT`). VTM ride height.
    pub top_z: f32,
    /// Distinct diving-board heights, sorted ascending. Empty on uniform maps.
    pub board_floors_z: Vec<f32>,
    /// Physical bay geometry in render space. Empty on uniform maps.
    pub frames: Vec<RackBox>,
    /// Physical steel footprints for locating a bot independently of its event anchor.
    pub steel_mm: Vec<[f64; 4]>,
    /// Per anchor cell id: render-space tote positions of every storage
    /// slot anchored there, sorted bottom-up then along the run. Empty on
    /// uniform maps.
    // ordered: the floor document must have one encoding — a rack travels to a
    // browser and into a cache keyed by its bytes, and a hash map's arbitrary
    // iteration order writes one floor down as two different documents.
    pub cell_slots: BTreeMap<u32, Vec<[f32; 3]>>,
    /// HTM-access cell id → render-space position of a tote on that
    /// relay's board-0 tab. Empty on uniform maps.
    // ordered: as above.
    pub relay_tote_pos: BTreeMap<u32, [f32; 3]>,
    /// Conveyor routes and stationary landmarks, projected from the equipment drawing.
    pub easyloop_belts: Vec<EasyLoopBelt>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EasyLoopBelt {
    // ordered: as `RackVisual`'s two maps — a belt is part of the same
    // document, and its landmarks are written down with it.
    pub landmarks: BTreeMap<u32, ContainerPose>,
    pub routes: Vec<BeltRoute>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ContainerPose {
    pub position: [f32; 3],
    pub heading: f32,
    pub pitch: f32,
}

/// A drawn centerline between two semantic conveyor cells, in render metres.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BeltRoute {
    pub from_cell: u32,
    pub to_cell: u32,
    pub points: Vec<[f32; 3]>,
}

impl RackVisual {
    /// Legacy uniform ladder: `vtc` evenly spaced levels above an open
    /// ground floor, `LEVEL_HEIGHT` per level, rendered by cell extrusion.
    pub fn uniform(vtc: u32) -> Self {
        Self {
            uniform_levels: vtc,
            top_z: (vtc + 1) as f32 * LEVEL_HEIGHT,
            board_floors_z: Vec::new(),
            frames: Vec::new(),
            steel_mm: Vec::new(),
            cell_slots: BTreeMap::new(),
            relay_tote_pos: BTreeMap::new(),
            easyloop_belts: Vec::new(),
        }
    }

    /// Render-space position for the n-th in-rack tote at an anchor cell.
    /// Explicit-slot maps record every in-rack tote at `level 0` of its
    /// anchor cell (each slot is its own one-level rack), so the tote's
    /// actual slot is unrecoverable from playback data; `occupancy_index`
    /// (n-th tote seen at this cell during one instance build) spreads
    /// co-anchored totes across the cell's real slots bottom-up then along
    /// the run. `None` on uniform maps (callers keep the exact legacy
    /// cell-center ladder).
    pub fn in_rack_slot_pos(&self, cell_id: u32, occupancy_index: usize) -> Option<[f32; 3]> {
        let slots = self.cell_slots.get(&cell_id)?;
        slots.get(occupancy_index.min(slots.len().checked_sub(1)?)).copied()
    }

    /// Fallback floor height for a tote at a relay cell without a board-0
    /// mapping: the lowest board, or just above the ground on uniform maps.
    pub fn relay_tote_z(&self) -> f32 {
        self.board_floors_z.first().copied().unwrap_or(0.01)
    }
}
