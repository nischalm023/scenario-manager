use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Newtype IDs
// ---------------------------------------------------------------------------

/// Opaque cell identity. NOT a coordinate. Supports special/junction cells
/// that have no grid position.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellId(pub u32);

// Debug emits the test-helper constructor form (`c(N)`) so panic output from
// `assert_trace!` / `assert_waypoints!` pastes back into the fh_sipp test
// suite verbatim. Production logging gains brevity (no `CellId(...)` wrapper).
impl std::fmt::Debug for CellId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "c({})", self.0)
    }
}

macro_rules! display_id {
    ($id:ty, $prefix:literal) => {
        impl std::fmt::Display for $id {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

display_id!(CellId, "c");

/// Station identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StationId(pub u32);
display_id!(StationId, "stn");

/// Zone identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ZoneId(pub u32);
display_id!(ZoneId, "z");

/// Sector identity (secondary partitioning, optional).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SectorId(pub u32);
display_id!(SectorId, "sector");

/// Relay position identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RelayId(pub u32);
display_id!(RelayId, "rly");

/// Floor identity (single floor for now).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FloorId(pub u32);
display_id!(FloorId, "floor");

/// Rack block identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BlockId(pub u32);
display_id!(BlockId, "block");

// ---------------------------------------------------------------------------
// Coordinate types
// ---------------------------------------------------------------------------

/// Logical grid position. Integer ordinals. Optional on cells -- special/junction
/// cells may not have one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GridPos {
    pub x: u32,
    pub y: u32,
}

/// Physical world position in millimeters. Optional on cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldPos {
    pub x_mm: f64,
    pub y_mm: f64,
}

// ---------------------------------------------------------------------------
// Cell classification
// ---------------------------------------------------------------------------

/// What kind of navigable cell this is. Set by the ingestion layer (parametric
/// generator or GSB loader). Used by pathfinder for robot-type traversability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellKind {
    /// Rack/storage position.
    Storage,
    /// VTM operating corridor between rack blocks. HTMs are forbidden from
    /// entering these cells.
    Aisle,
    /// Open/untyped cell (default). Traversable by all robot types.
    #[serde(alias = "Parking")]
    Open,
    /// Highway lane (transit corridor, VTMs cannot use).
    Highway,
    /// Station infrastructure / PPS area.
    StationArea,
    /// Relay rack position (RELAY system). Each relay cell is a shelf where
    /// VTMs and HTMs exchange totes. Relay rack blocks are typically 2 columns
    /// wide (VTMs access from adjacent aisles), so every cell in the block is
    /// an edge cell with its own RelayPosition — nothing but edge.
    Relay,
    /// Special junction node (cross-aisle navigation node).
    Junction,
    /// Non-traversable blank cell (clipped out of the usable grid).
    Blank,
    /// Cell where HTM presents tote to operator/arm.
    Presentation,
    /// Cell in the HTM queue path leading to the presentation cell.
    QueueCell,
    /// Designated HTM charging cell (future). Traversable by HTMs (same as Open).
    Charging,
    /// A carrier handoff cell: the place an HTM occupies to exchange a tote
    /// with a belt, a dock or a relay (an EasyLoop's dropoff/collection cells,
    /// and the cell an HTM stands on beneath a relay's shelves). Traversable
    /// by HTMs (same as Open) -- occupancy discipline is the engine's, not the
    /// kind's. Reintroduced 2026-08-18; the v3 compiler
    /// had this kind for relay exchange docks and it retired with that
    /// runtime.
    Exchange,
}

// ---------------------------------------------------------------------------
// Edge traversability
// ---------------------------------------------------------------------------

/// HTM traversal capability for an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HtmTraversal {
    /// HTM cannot traverse this edge.
    None,
    /// Only unladen HTM can traverse (laden blocked -- e.g. lengthwise rack edge).
    UnladenOnly,
    /// Both laden and unladen HTM can traverse.
    Both,
}

/// Per-directed-edge traversability flags, computed during map compilation.
///
/// These flags are the SINGLE SOURCE OF TRUTH for which robot types can
/// traverse an edge. Downstream consumers (pathfinder, visualizer) read
/// these flags directly instead of implementing their own cell-kind logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeTraversability {
    /// Can a VTM traverse this edge?
    pub vtm_traversal: bool,
    /// Can an HTM traverse this edge (and in which cargo states)?
    pub htm_traversal: HtmTraversal,
    /// Can an RTP robot traverse this edge?
    pub rtp_traversal: bool,
}

impl EdgeTraversability {
    /// All robot types enabled -- used as the initial state before
    /// reachability analysis narrows flags down.
    pub const fn fully_traversable() -> Self {
        Self { vtm_traversal: true, htm_traversal: HtmTraversal::Both, rtp_traversal: true }
    }
}

// ---------------------------------------------------------------------------
// Warehouse edge
// ---------------------------------------------------------------------------

/// Operational type of a PPS station.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StationType {
    #[default]
    Pick,
    Putaway,
    /// A carrier source: an empty HTM arrives and leaves carrying an empty
    /// order box. No pick or putaway demand ever routes here.
    BoxInduction,
    /// An outbound handoff: an HTM leaves with the emptied tote or unladen.
    /// The station's role distinguishes final dispense from box finishing.
    Extraction,
    /// Pickup of an existing container from downstream equipment. Unlike
    /// BoxInduction, this does not create a new empty order container.
    ContainerPickup,
}

/// Which edge of the rectangular grid a station is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StationEdge {
    Bottom,
    Top,
    Left,
    Right,
}

/// Which side of a rack block a relay position or rack access faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlockSide {
    Left,
    Right,
}

/// System type: RTP (racks-to-picker) or RELAY (VTM+HTM).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SystemType {
    Rtp,
    Relay,
}

/// Robot type for traversability filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RobotType {
    /// Horizontal Transfer Module (aisle+highway, rack limited when laden).
    Htm,
    /// Vertical Transfer Module (aisle-only, no highway or rack cells).
    Vtm,
    /// Racks-to-Picker (unrestricted).
    Rtp,
}

/// Which direction a rack row runs. Used to determine which edges are
/// "lengthwise" (parallel to the row) and therefore blocked for laden HTMs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RackOrientation {
    /// Rack row runs North-South (vertical columns). N-S edges are lengthwise.
    NorthSouth,
    /// Rack row runs East-West (horizontal rows). E-W edges are lengthwise.
    EastWest,
}

/// Cardinal direction for HTM departure after presenting at a station.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CardinalDirection {
    North,
    South,
    East,
    West,
}
