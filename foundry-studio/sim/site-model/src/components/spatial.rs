//! Where things are: the components that place an entity in space and that turn
//! an area into the cells an agent can stand on.
//!
//! This is the half of the vocabulary SPEC §4 and §5 govern — declared
//! coordinate systems, poses and regions expressed in their steps, the tiling
//! rule that derives a region's cells and stamps its components onto them, the
//! carve-outs a tiling states to leave part of its own region untiled, and the
//! marker that identifies a robot charging cell. Nothing here knows about
//! rack or station behavior; it describes floor capabilities, rectangles and
//! the frames they are measured in.

use crate::ids::EntityId;
use crate::units::{Coord, Degrees, Steps};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A declared coordinate system (SPEC §4). One frame per world is primary —
/// 3-D, world-fixed, and the only one that says what a step is worth — and
/// every other frame reaches it through a chain of `transform_to_parent`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Frame {
    /// Absent on the primary frame and only there.
    #[serde(default)]
    pub parent: Option<EntityId>,
    pub dims: u8,
    pub units: String,
    /// What one integer step of this format's lengths is worth physically. The
    /// primary frame declares it and no other frame may (SPEC §4): a step
    /// becomes physical in exactly one place, and every other frame takes its
    /// step from its transform chain.
    #[serde(default)]
    pub unit_mm: Option<f64>,
    /// Positions *declared in* this frame must be integers; projections into it
    /// round half-up and are understood to be approximate (SPEC §4).
    #[serde(default)]
    pub integer_only: bool,
    /// The robot vendor whose install this frame belongs to. A vendor frame is
    /// not a kind of frame — just a frame so tagged (SPEC §4), which is why
    /// nothing in the pipeline branches on it; it is here so that a reader of
    /// the typed view can see what the document says.
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub transform_to_parent: Option<Transform>,
}

impl Frame {
    /// Whether one step of this frame measures a length. A frame whose `units`
    /// are `index` is the legacy lattice of SPEC §4: something to project into,
    /// never something to author geometry in.
    pub fn measures_length(&self) -> bool {
        self.units != LATTICE_UNITS
    }
}

/// What a frame writes for `units` when one step of it is a lattice index
/// rather than a length (SPEC §4).
pub const LATTICE_UNITS: &str = "index";

/// `p_parent = translation + scale ⊙ p_frame`, per axis. A 2-D frame under a
/// 3-D parent embeds as `[x, y] → [x, y, 0]` before the translation, which is
/// why its translation carries three entries and its scale two.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Transform {
    #[serde(default)]
    pub translation: Vec<f64>,
    #[serde(default)]
    pub scale: Vec<f64>,
}

/// A position, and optionally a heading, in some frame (SPEC §5). Inside a
/// template the frame is absent: the pose is in the template's own local frame
/// and acquires a frame when a stamp places it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Pose {
    #[serde(default)]
    pub frame: Option<EntityId>,
    pub position: Vec<Coord>,
    #[serde(default)]
    pub heading_deg: Degrees,
}

impl Pose {
    /// The planar part of the position. v0 geometry is planar; a 3-D position's
    /// third entry is its elevation and takes part in no rectangle arithmetic.
    pub fn xy(&self) -> Option<[Coord; 2]> {
        match self.position.len() {
            0 | 1 => None,
            _ => Some([self.position[0], self.position[1]]),
        }
    }
}

/// A rectangle in a frame, anchored at its minimum corner, measured in that
/// frame's steps (SPEC §5).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Region {
    #[serde(default)]
    pub frame: Option<EntityId>,
    pub origin: [Steps; 2],
    pub size: [Steps; 2],
}

impl Region {
    pub fn max_corner(&self) -> [Steps; 2] {
        [
            Steps(self.origin[0].0 + self.size[0].0),
            Steps(self.origin[1].0 + self.size[1].0),
        ]
    }
}

/// A rectangle a tiling leaves untiled: anchored at its minimum corner, stated
/// relative to its region's origin, in the region's frame's steps. Each derived
/// cell whose centre falls strictly inside it is removed — from this tiling
/// only, which is the point: what a tiling derives is a function of the tiling
/// alone (SPEC §5).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Exclusion {
    pub origin: [Steps; 2],
    pub size: [Steps; 2],
}

impl Exclusion {
    /// Whether this carve-out takes out the cell centred at `centre`, in a
    /// tiling whose region is anchored at `region_origin`.
    ///
    /// Both arrive in the steps of the region's own frame — a tiling writes its
    /// cells' poses in that frame — so nothing converts here. Strictly inside is
    /// the test SPEC §5 gives exclusions: a cell centred exactly on an edge
    /// survives.
    ///
    /// `None` says the far corner does not fit in a step count. The caller
    /// turns that into a load error, because the only other answer available is
    /// the verdict of a rectangle that wrapped around.
    pub fn contains_strictly(&self, region_origin: [Steps; 2], centre: [Coord; 2]) -> Option<bool> {
        let mut inside = true;
        for axis in 0..2 {
            // Both axes are read before the answer is returned, so a corner
            // that does not fit is reported even when the other axis has
            // already put the centre outside.
            let min = region_origin[axis].0.checked_add(self.origin[axis].0)?;
            let max = min.checked_add(self.size[axis].0)?;
            inside &= centre[axis].0 > min && centre[axis].0 < max;
        }
        Some(inside)
    }
}

/// The component that makes a region a cell_grid: a pitch, and the whole
/// component map every cell it derives carries (SPEC §5).
///
/// `derives` is how an area-level fact is stated once instead of per cell —
/// these cells admit HTMs, these cells have chargers — and it is a component map
/// rather than one component's defaults, so a tiling can say anything about its
/// cells that a record could.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Tiling {
    pub pitch: [Steps; 2],
    /// The parts of its own region this tiling leaves untiled — the loading
    /// dock the floor runs past, the footprint the steel stands on. A tiling
    /// states its own holes so that no other entity has to reach into this one
    /// to punch them, and so that a hole meant for one tiling cannot take cells
    /// out of another that happens to overlap it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<Exclusion>,
    #[serde(default)]
    pub derives: Option<BTreeMap<String, Value>>,
}

/// A graph node an agent can occupy.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Cell {
    /// The agent classes that physically fit here — capability, not policy
    /// (SPEC §7).
    #[serde(default)]
    pub traversable_by: Vec<String>,
    #[serde(default)]
    pub blocked: bool,
    /// A hand-declared cell may state its own extent; a derived cell's is
    /// its tile.
    #[serde(default)]
    pub size: Option<[Steps; 2]>,
}

/// A robot charging point. A marker with no fields, declared individually or
/// through a tiling's `derives` (SPEC §5, §10). It denotes charging equipment,
/// not a generic idle destination; charge-state dispatch is a simulator policy.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Charging {}

/// The footprint a cell falls back to when neither a tiling pitch nor a declared
/// `size` says otherwise (SPEC §5).
///
/// It is stated in millimetres because it is the one physical constant the
/// suffix purge leaves standing: a default cannot be a step count until a
/// document says what a step is, so the reader turns it into steps through the
/// primary frame's `unit_mm`.
pub const DEFAULT_CELL_FOOTPRINT_MM: f64 = 900.0;

/// Half a step: the unit every box below is held in.
///
/// A cell is anchored at its centre and reaches half its footprint each way, so
/// an odd-sized footprint puts an edge halfway between two steps. Doubling the
/// lattice is what keeps that edge exact — every gap and every tolerance test
/// below is integer arithmetic with nothing to round, which is the whole point
/// of counting steps instead of carrying millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HalfSteps(pub i64);

impl HalfSteps {
    pub fn of(steps: Steps) -> HalfSteps {
        HalfSteps(steps.0 * 2)
    }
}

/// An axis-aligned box in the primary frame, used for every geometric comparison
/// the pipeline makes between two expansions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub min: [HalfSteps; 2],
    pub max: [HalfSteps; 2],
}

impl Bounds {
    pub fn around(centre: [Steps; 2], footprint: [Steps; 2]) -> Bounds {
        Bounds {
            min: [
                HalfSteps(centre[0].0 * 2 - footprint[0].0),
                HalfSteps(centre[1].0 * 2 - footprint[1].0),
            ],
            max: [
                HalfSteps(centre[0].0 * 2 + footprint[0].0),
                HalfSteps(centre[1].0 * 2 + footprint[1].0),
            ],
        }
    }

    pub fn union(self, other: Bounds) -> Bounds {
        Bounds {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    /// The box's own centre, in half-steps. `min + max` is four times the
    /// centre, so halving it is exact.
    pub fn centre(self) -> [HalfSteps; 2] {
        [
            HalfSteps((self.min[0].0 + self.max[0].0) / 2),
            HalfSteps((self.min[1].0 + self.max[1].0) / 2),
        ]
    }

    /// Edge-to-edge separation on one axis, in half-steps: positive when the two
    /// boxes are apart, zero when they touch, negative by the overlap when they
    /// intersect.
    pub fn separation(self, other: Bounds, axis: usize) -> HalfSteps {
        let lo = self.min[axis].max(other.min[axis]);
        let hi = self.max[axis].min(other.max[axis]);
        HalfSteps(lo.0 - hi.0)
    }

    /// The edge that faces the given direction along one axis, in half-steps.
    pub fn edge(self, axis: usize, toward_plus: bool) -> HalfSteps {
        if toward_plus {
            self.max[axis]
        } else {
            self.min[axis]
        }
    }
}
