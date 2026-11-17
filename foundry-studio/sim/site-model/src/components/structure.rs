//! The built structure of a site and the connectivity rules over it: racks and
//! what they project, the storage they hold, the junction a run is worked from,
//! the flow policy inside one expansion, and the stitches that join two
//! expansions.
//!
//! These are the components that turn declared geometry into a graph. A rack is
//! a generator in the same sense a cell_grid is (SPEC §5) — its enumerated
//! `levels` project one relay per position of every relay level and one slot per
//! storage position — and `flow` and `stitch` are the only two ways an arc ever
//! comes into existence, one inside an expansion and one between two of them.
//! The relay's own binding to the floor is not an arc at all: it is the
//! [`AccessCell`] component naming a cell that already exists.

use crate::components::product::CellAddress;
use crate::ids::EntityId;
use crate::units::Steps;
use serde::{Deserialize, Serialize};

/// Which face of a rack is meant. Sidedness is physics, not policy: a rack is
/// reachable from one side and that is a fact about the steel.
///
/// One thing states a face and nothing else does: the [`Access`], naming the
/// side storage is worked from. Nothing derives from it — it is read by the
/// SPEC §7 checks and by a motion model, which is why this type carries no
/// axis arithmetic. Where a relay stands is geometry on its level, not a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub enum Side {
    #[serde(rename = "+x")]
    PlusX,
    #[serde(rename = "-x")]
    MinusX,
    #[serde(rename = "+y")]
    PlusY,
    #[serde(rename = "-y")]
    MinusY,
}

/// One bay of racking: pure steel, and nothing about who serves it.
///
/// A rack's vertical truth is one list, `levels`, enumerated bottom-up, because
/// real bays are not uniformly spaced and a nominal pitch would be a second
/// number that could disagree with the survey. An entry carrying `positions` is
/// a relay level and derives the handoff shelves; an entry without derives
/// storage. What a rack deliberately does *not* state is which side anybody
/// reaches it from: that is the junction between this run and the region it is
/// worked from, and it belongs to an [`Access`]. A rack reachable from both
/// sides is one rack with two accesses, not two logical racks.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Rack {
    pub slots_wide: usize,
    pub slots_deep: usize,
    #[serde(default)]
    pub slot_pitch: Option<[Steps; 2]>,
    /// Extent at heading 0, anchored with its minimum corner at the rack's pose.
    pub footprint: [Steps; 2],
    /// Every shelf this bay has, bottom-up and strictly increasing. Relay and
    /// slot keys both index this list, so a level number names the same physical
    /// shelf whatever kind of entry sits below it (SPEC §5).
    pub levels: Vec<Level>,
}

/// One entry of a rack's `levels`: a height, and either the relays that protrude
/// from it or the storage it holds — never both.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Level {
    /// How far above the floor at the rack's pose this shelf sits.
    pub height: Steps,
    /// Where this level's shelves stand, each an `[along, out]` offset from the
    /// bay's own origin corner in the bay's own frame. Present makes it a relay
    /// level; absent makes it a storage level.
    ///
    /// Which side of the steel a shelf protrudes on is the second number: a
    /// shelf on the low-`y` face stands at an `out` at or below zero, one on the
    /// high-`y` face at an `out` at or beyond the footprint's depth. That is the
    /// same statement the retired `face` enum made, said as geometry, and the
    /// pair composes through the rack's pose like every other position (SPEC §5).
    #[serde(default)]
    pub positions: Option<Vec<[Steps; 2]>>,
    /// How far from a relay of this level a cell may sit and still be the one it
    /// binds.
    #[serde(default)]
    pub reach: Option<Steps>,
}

/// A relay level's two statements, read together because a relay cannot be
/// placed or bound without both.
#[derive(Debug, Clone, Copy)]
pub struct RelayLevel<'a> {
    pub positions: &'a [[Steps; 2]],
    pub reach: Steps,
}

impl Level {
    /// Whether this entry is a relay level. The discriminator is `positions`,
    /// so a reader can tell what `slot:{level},…` addresses without consulting a
    /// second field (SPEC §5).
    pub fn is_relay(&self) -> bool {
        self.positions.is_some()
    }

    /// The relay level's two statements, or `None` when this is storage or when
    /// the entry is a relay level missing one of them.
    pub fn relay(&self) -> Option<RelayLevel<'_>> {
        Some(RelayLevel {
            positions: self.positions.as_deref()?,
            reach: self.reach?,
        })
    }
}

impl Rack {
    /// The lowest shelf, which is the clearance anything driving under this bay
    /// has (SPEC §7).
    pub fn lowest_level(&self) -> Option<Steps> {
        self.levels.first().map(|level| level.height)
    }

    /// Every level with its index, relay levels only.
    pub fn relay_levels(&self) -> impl Iterator<Item = (usize, &Level)> {
        self.levels
            .iter()
            .enumerate()
            .filter(|(_, level)| level.is_relay())
    }

    /// Every level with its index, storage levels only.
    pub fn storage_levels(&self) -> impl Iterator<Item = (usize, &Level)> {
        self.levels
            .iter()
            .enumerate()
            .filter(|(_, level)| !level.is_relay())
    }
}

/// The reified junction between a run of racking and the region it is worked
/// from.
///
/// Four fields are the whole of it and it derives nothing at all: it is a
/// statement, and SPEC §7's checks and the motion model are its consumers. That
/// is what makes a world expand identically with or without a fleet layered in —
/// nothing derived reads `serves`. The `face` here is the face *storage* is
/// reached from; the face a relay protrudes toward lives on its relay level,
/// because in a rack | aisle | rack zone the two are opposite.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Access {
    /// The run: a stamp whose instances are the bays.
    pub rack: EntityId,
    /// The region this face is worked from.
    pub along: EntityId,
    /// Which face of the bay, in the run template's local frame. The stamp's
    /// heading turns it, like all stamped geometry.
    pub face: Side,
    /// The classes that work this face. Absent means unrestricted.
    #[serde(default)]
    pub serves: Option<Vec<EntityId>>,
}

/// What a rack or buffer may hold. Two racks of identical steel differ only in
/// this: the order buffer is the inventory rack template with `holds` replaced.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Storage {
    pub holds: String,
    #[serde(default)]
    pub carrier_type: Option<EntityId>,
}

/// One storage position of a rack, carried by an entity the rack derived.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Slot {
    pub rack: EntityId,
    /// Which entry of the rack's `levels` this shelf is.
    pub level: usize,
    pub col: usize,
    pub depth: usize,
}

/// One handoff shelf: which rack it belongs to, which of that rack's levels it
/// stands on, how high that is, and where on the bay it sits.
///
/// It does not name the cell an HTM stands on — that is the [`AccessCell`]
/// beside it, which the binding search fills in after the cells that survived
/// suppression are known (SPEC §8 step 5).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Relay {
    pub rack: EntityId,
    /// Which entry of the rack's `levels` this shelf is.
    pub level: usize,
    /// Which of that level's positions, in the order they are written.
    pub index: usize,
    pub height: Steps,
    /// The entry of its level's `positions` that derived it: `[along, out]` from
    /// the bay's origin corner, in the bay's own frame. It is an offset, not a
    /// world position — the rack's pose is what carries it into a frame.
    pub offset: [Steps; 2],
}

/// The binding: the one cell an HTM occupies to exchange with a relay.
///
/// Nothing is minted for it. It names an ordinary cell of the cell_grid the HTM
/// travels — found by the binding search, or stated by an override, which are
/// the same component either way (SPEC §5).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct AccessCell {
    pub cell: CellAddress,
}

/// Which way traffic runs inside one generator's cells — a policy statement, not
/// a physical one (SPEC §7).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Flow {
    pub rule: FlowRule,
    #[serde(default)]
    pub direction: Option<Direction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FlowRule {
    /// Along-axis arcs in the stated direction only; crossing from one row to
    /// the next stays two-way.
    OneWay,
    /// Every orthogonal-neighbour arc, both ways.
    TwoWay,
    /// Rows alternate along-axis direction, even rows running the stated way.
    AlternatingRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub enum Direction {
    #[serde(rename = "+x")]
    PlusX,
    #[serde(rename = "-x")]
    MinusX,
    #[serde(rename = "+y")]
    PlusY,
    #[serde(rename = "-y")]
    MinusY,
}

impl Direction {
    /// The axis traffic runs along: 0 for x, 1 for y.
    pub fn axis(self) -> usize {
        match self {
            Direction::PlusX | Direction::MinusX => 0,
            Direction::PlusY | Direction::MinusY => 1,
        }
    }

    /// Whether it runs toward increasing indices on that axis.
    pub fn forward(self) -> bool {
        matches!(self, Direction::PlusX | Direction::PlusY)
    }
}

/// A reified linking rule between two expansions' cells (SPEC §5). Nothing links
/// without one: two cell_grids that touch are two cell_grids that touch.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Stitch {
    /// One endpoint. Any entity whose expansion contributes cells will do — a
    /// region, a stamp — which is how a stamped station's cells bind to the
    /// floor they stand in.
    pub a: EntityId,
    pub b: EntityId,
    pub rule: StitchRule,
    #[serde(default)]
    pub tolerance: Option<Steps>,
    #[serde(default)]
    pub pairs: Option<Vec<Vec<CellAddress>>>,
    /// False means each explicit pair runs only from its first cell to its second.
    #[serde(
        default = "default_bidirectional",
        skip_serializing_if = "is_bidirectional"
    )]
    pub bidirectional: bool,
    /// Circular geometry between the declared endpoints; no extra cells are generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CircularArc>,
}

fn default_bidirectional() -> bool {
    true
}
fn is_bidirectional(value: &bool) -> bool {
    *value
}

/// A circle's centre and traversal sense, in a stated frame. Its endpoints are
/// the stitch's cells unless tangent points are stated. Optional straight leads
/// join those points to the cells. Reflection reverses traversal sense.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct CircularArc {
    pub frame: EntityId,
    pub center: [Steps; 2],
    pub clockwise: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<[Steps; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<[Steps; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum StitchRule {
    /// Nearest facing cell within tolerance, many-to-one by design.
    Contact,
    /// Exactly the stated pairs, used where contact would be wrong. A pair names
    /// two cells in the one address vocabulary SPEC §3 defines, the same one a
    /// queue path uses.
    Explicit,
}
