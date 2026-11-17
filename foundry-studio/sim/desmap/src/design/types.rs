//! Within `design`, `types` is the AUTHORING-FORMAT data model: the serde
//! shapes of the warehouse design file (cells, stations, edges, zones) that
//! the compiler turns into runtime map entities. Serialization tolerance
//! rules for older design files live here, beside the fields they default.

use serde::{Deserialize, Serialize};

use crate::map::FleetConfig;
use crate::types::*;
use crate::vtm_model::VtmModel;

/// The warehouse design — compact description that compiles to a `WarehouseMap`.
///
/// Version 3 uses parametric zone definitions instead of per-cell kind strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarehouseDesign {
    /// Format version. v3 = zone-based schematic design.
    pub version: u32,
    /// Grid dimensions and cell size.
    pub grid: GridDefinition,
    /// Highway blocks — rectangular regions stamped as Highway cells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highways: Vec<HighwayDefinition>,
    /// Parametric zone definitions (rack regions).
    pub zones: Vec<ZoneDef>,
    /// Rack prototypes referenced by relay-zone `rack_placements`. A
    /// prototype is defined once here and stamped onto any number of zone
    /// sides; unreferenced prototypes are allowed (a prototype library).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub racks: Vec<RackDef>,
    /// PPS station definitions.
    pub stations: Vec<StationDefinition>,
    /// Edge exceptions — only edges that differ from the default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_exceptions: Vec<EdgeException>,
    /// Non-grid junction cells with explicit adjacency.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub junction_cells: Vec<JunctionCell>,
    /// Clip boxes — rectangular regions that blank out unused grid areas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip_boxes: Vec<ClipBox>,
    /// Explicit charging infrastructure (optional; never auto-generated).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charging_zones: Vec<ChargingZone>,
    /// Fleet configuration (optional; defaults based on system type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet: Option<FleetConfig>,
}

/// Grid dimensions, and optionally the real physical position of each row.
///
/// Not `Copy`: `row_centers_mm` is a `Vec`. Nothing relied on `Copy` — every use
/// is a struct literal or a field read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridDefinition {
    pub width: u32,
    pub depth: u32,
    /// Uniform cell pitch. Authoritative for columns, and for rows when
    /// `row_centers_mm` is absent.
    pub cell_size_mm: f64,
    /// Physical center Y of each row in mm, `depth` entries, strictly
    /// increasing. Absent means uniform: `row * cell_size_mm + cell_size_mm / 2`.
    ///
    /// Exists because a single pitch cannot describe a real rack: on cencora,
    /// relay pitch along a rack column alternates 645, 645, 850. Positions
    /// rather than extents, because extents correspond to no dimension on a
    /// drawing and are ambiguous under non-uniform spacing — see
    /// [`crate::design::grid_geometry::AxisGeometry`].
    ///
    /// Columns have no equivalent yet; add one the same way when a map needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_centers_mm: Option<Vec<f64>>,
}

impl GridDefinition {
    /// Uniform grid — the shape every design had before row centers existed.
    pub fn uniform(width: u32, depth: u32, cell_size_mm: f64) -> Self {
        Self { width, depth, cell_size_mm, row_centers_mm: None }
    }
}

/// Direction constraint for a highway lane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaneDirection {
    #[default]
    Bidirectional,
    Eastbound,
    Westbound,
    Northbound,
    Southbound,
    NorthSouthOnly,
    EastWestOnly,
}

/// A highway block — a rectangular region of Highway cells with a uniform
/// lane direction. Multiple blocks can be placed to create complex layouts
/// (e.g., eastbound + gap + westbound).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighwayDefinition {
    /// `[col, row]` of the top-left corner.
    pub origin: [u32; 2],
    /// Width in cells (horizontal extent).
    pub width: u32,
    /// Height in cells (vertical extent / number of lanes).
    pub height: u32,
    /// Lane direction for all cells in this block.
    #[serde(default)]
    pub direction: LaneDirection,
}

/// A rectangular clip box that blanks out a region of the grid.
///
/// Cells inside a clip box are set to `CellKind::Blank` and become
/// non-traversable. Origin is `[col, row]` (top-left of the box).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipBox {
    /// [col, row] of the top-left corner.
    pub origin: [u32; 2],
    /// Width in cells (number of columns).
    pub width: u32,
    /// Height in cells (number of rows).
    pub height: u32,
}

/// A rectangular block of charging cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChargingZone {
    /// [col, row] of top-left.
    pub origin: [u32; 2],
    /// Width in cells (columns).
    pub width: u32,
    /// Height in cells (rows).
    pub height: u32,
}

/// Parametric zone definition — tagged enum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ZoneDef {
    Relay(RelayZone),
    Rtp(RtpZone),
}

/// A relay zone in the "sandwich" model.
///
/// Each zone contains its own aisle column. The `sides` field determines shape:
/// - **Dual** (default): rack @ origin_col, aisle @ origin_col+1, rack @ origin_col+2 (3 cols)
/// - **LeftOnly**: rack @ origin_col, aisle @ origin_col+1 (2 cols)
/// - **RightOnly**: aisle @ origin_col, rack @ origin_col+1 (2 cols)
///
/// Origin is always the top-left cell of the zone footprint. Relay positions
/// are created on each rack column with `vtm_access` pointing to the aisle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayZone {
    pub id: u32,
    /// [col, row] of top-left of the zone footprint.
    pub origin: [u32; 2],
    /// Number of rack rows (height of the zone).
    pub length: u32,
    /// Which sides of the aisle have rack columns. Default Dual.
    #[serde(default, skip_serializing_if = "is_dual")]
    pub sides: Sidedness,
    /// Number of totes stacked vertically per rack position. Default 8.
    /// Java equivalent: `verticalToteCapacity` (typically 8-12).
    #[serde(default = "default_vtc")]
    pub vertical_tote_capacity: u32,
    /// Physical height of the racking in this zone, in mm. Default 9000.
    /// VTM vertical travel spans 0 to this height.
    #[serde(default = "default_zone_height_mm")]
    pub zone_height_mm: u64,
    /// Physical horizontal length of the aisle in this zone, in mm.
    /// When None, derived from zone length × cell_size_mm at compile time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_width_mm: Option<u64>,
    /// Actual number of physical shelves per rack column. Default 10.
    /// Used for VTM motion model; independent of vertical_tote_capacity (which
    /// may be inflated as a hack for inventory capacity).
    #[serde(default = "default_real_shelf_count")]
    pub real_shelf_count: u16,
    /// Stampings of rack prototypes onto this zone's rack sides. When
    /// non-empty, the uniform `zone_height_mm`/`real_shelf_count`/
    /// `vertical_tote_capacity` model is bypassed: each storage slot of a
    /// placed prototype becomes its own rack entity in the sim, VTM travel
    /// targets the slot's exact (x, y), and the relay exchange height comes
    /// from the diving-board slots. The compiler materializes placements
    /// (in authored order) into the compiled zone's slot list, deriving
    /// each slot's anchor cell from its position along the run.
    ///
    /// Replaces the earlier inline `slots` field, which duplicated one rack
    /// design into every zone; a design that still carries `slots` fails to
    /// parse ("unknown field") and needs its slot lists hoisted into a
    /// prototype here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rack_placements: Vec<RackPlacement>,
    /// VTM hardware model for this zone.
    #[serde(default, skip_serializing_if = "is_default_vtm")]
    pub vtm_model: VtmModel,
    /// Per-zone VTM count override. When `None`, uses model default (2 for A5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vtm_count: Option<u32>,
}

/// One explicitly encoded slot of a relay zone in the **compiled** map: a
/// storage slot or a diving-board (relay exchange) slot. Produced by
/// materializing the zone's rack placements — not part of the authored
/// design, where slot geometry lives on [`RackDef`] without cells.
///
/// `y` is the slot's beam/floor height — the VTM **fork-interface** height,
/// where the forks slide under a tote. It is deliberately not a slot
/// midpoint: the A5 motion model travels to the fork height and folds the
/// pick/lift micro-motion into its fork-time constants (the spec sheet's
/// "First-Tier Height of Buffer Position" 390 mm is likewise a floor).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotDef {
    /// Absolute grid cell this slot anchors to, `[col, row]` — a rack cell
    /// of the zone; for diving boards, the relay cell carrying the board
    /// stack. Anchors zone membership, aisle side, and VTM access. Derived
    /// at materialization (nearest cell center along the run), never
    /// authored: grid anchoring is how a slot meets the grid-generated cell
    /// world, not part of the slot's identity.
    pub cell: [u32; 2],
    /// Position along the rack row, mm. One shared frame per zone (any
    /// consistent origin, e.g. the row's left edge); only relative spacing
    /// matters — the span is normalized onto `zone_width_mm` for VTM
    /// kinematics.
    pub x: f64,
    /// Beam/floor height, mm above ground (fork-interface height).
    pub y: f64,
    /// This is a diving board — a relay/buffer exchange slot, not storage.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diving_board: bool,
    /// Index of this board within its relay cell's exchange stack, bottom-up
    /// from 0. Required on diving boards (relay cells can carry several
    /// boards; the sim currently exchanges on board 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diving_board_slot: Option<u32>,
}

impl RelayZone {
    /// Slots per rack entity. With placed rack prototypes every storage
    /// slot is its own directly-encoded rack, so this is 1; otherwise the
    /// explicit `vertical_tote_capacity` (slots stacked per column).
    pub fn effective_vertical_tote_capacity(&self) -> u32 {
        if self.has_explicit_slot_geometry() { 1 } else { self.vertical_tote_capacity }
    }

    /// Whether this zone's rack geometry is explicitly encoded per slot
    /// rather than derived from the uniform vertical model.
    pub fn has_explicit_slot_geometry(&self) -> bool {
        !self.rack_placements.is_empty()
    }
}

/// A rack prototype: one physical rack unit — the four-legged steel frame
/// — as pure slot geometry, independent of any zone. A [`RackPlacement`]
/// stamps a prototype onto a relay zone's rack side, and a full rack run
/// is tiled from repeated placements (`x_offset_mm` stepping by
/// `length_mm`); the compiler then materializes the prototype's slots into
/// the zone's compiled slot list, deriving each slot's anchor cell (the
/// grid-generated, HTM-traversable place that carries it) from the slot's
/// position along the run. Slots materialize in list order — the order is
/// load-bearing: it fixes the sim's rack enumeration.
///
/// **Frame convention: bottom-left.** The prototype's own frame has x = 0
/// at the **left end of the steel envelope** (not the first slot — leg
/// margins are encoded in the slot xs) and y = 0 at the **floor**. Corner
/// origins keep survey-drawing values verbatim, keep tiling arithmetic
/// additive (`next_offset = offset + length_mm`), and stay stable when a
/// prototype gains slots — a center origin would shift with content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackDef {
    /// Unique name; `RackPlacement::rack` references it.
    pub name: String,
    /// Length of the steel envelope along the run, mm — from the frame's
    /// x = 0 to its right end. Declares the physical extent independently
    /// of slot positions: tiling steps by it, overlap validation uses it,
    /// and every slot x must lie within `[0, length_mm]`. Adjacent tiled
    /// racks touch (shared legs), so consecutive offsets differ by exactly
    /// `length_mm`.
    pub length_mm: f64,
    /// Slot geometry in materialization order. Convention (and what the
    /// cencora survey data uses): diving-board levels first, then storage
    /// levels, each level bottom-up with x ascending.
    pub slots: Vec<RackSlotDef>,
}

/// One slot of a rack prototype: [`SlotDef`] geometry without an anchor
/// cell — the cell is derived at materialization, not authored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackSlotDef {
    /// Position along the run, mm, from the rack frame's left end
    /// (bottom-left convention, see [`RackDef`]); a placement's
    /// `x_offset_mm` shifts it into the zone-shared frame. Must lie within
    /// the rack's `[0, length_mm]` envelope.
    pub x: f64,
    /// Beam/floor height, mm above the floor (fork-interface height; see
    /// [`SlotDef::y`]).
    pub y: f64,
    /// This is a diving board — a relay/buffer exchange slot, not storage.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diving_board: bool,
    /// Index within the exchange stack at this slot's cell, bottom-up from
    /// 0. Required on diving boards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diving_board_slot: Option<u32>,
}

/// One stamping of a rack prototype onto a relay zone's rack side.
/// Multiple placements per side stamp short bays along a run; they
/// materialize in authored order, prototype slots in prototype order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackPlacement {
    /// Name of a [`RackDef`] in [`WarehouseDesign::racks`].
    pub rack: String,
    /// Which side of the zone's aisle (must be a rack side under the
    /// zone's `sides`).
    pub side: BlockSide,
    /// Distance from the start of the zone's run to this rack's **left
    /// end** (bottom-left convention), mm: the shift applied to every
    /// prototype slot x into the zone-shared frame. Tile a run by stepping
    /// consecutive placements by the rack's `length_mm`. Default 0.
    #[serde(default, skip_serializing_if = "is_zero_mm")]
    pub x_offset_mm: f64,
}

fn is_zero_mm(v: &f64) -> bool {
    *v == 0.0
}

/// An RTP zone: a rectangular block of rack storage cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RtpZone {
    pub id: u32,
    /// [col, row] of top-left.
    pub origin: [u32; 2],
    /// Number of rack rows.
    pub rows: u32,
    /// Number of rack columns.
    pub cols: u32,
}

/// Sidedness of a relay zone — which sides of the aisle have rack columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sidedness {
    #[default]
    Dual,
    LeftOnly,
    RightOnly,
}

fn is_dual(s: &Sidedness) -> bool {
    *s == Sidedness::Dual
}

fn is_default_vtm(m: &VtmModel) -> bool {
    *m == VtmModel::A5
}

pub(crate) fn default_vtc() -> u32 {
    8
}

fn default_zone_height_mm() -> u64 {
    9000
}

fn default_real_shelf_count() -> u16 {
    10
}

/// Station definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationDefinition {
    pub id: u32,
    /// Grid position [col, row] of the station cell.
    pub position: [u32; 2],
    /// Grid position [col, row] where HTM presents tote to operator/arm.
    #[serde(alias = "access_position")]
    pub presentation_cell: [u32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<StationEdge>,
    /// Ordered approach path, entry first. Last cell must adjoin presentation_cell.
    #[serde(default)]
    pub queue_path: Vec<[u32; 2]>,
    /// Cardinal direction HTM departs after presenting.
    #[serde(default = "default_station_exit_direction")]
    pub exit_direction: CardinalDirection,
    /// Per-station bin count override. When `None`, the system default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bins: Option<usize>,
    /// Operational station type. Default: Pick (backward compatible).
    #[serde(default)]
    pub station_type: StationType,
}

fn default_station_exit_direction() -> CardinalDirection {
    CardinalDirection::East
}

/// An edge that differs from the default (cardinal, unrestricted, cell_size_mm distance).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeException {
    /// Source grid position [col, row].
    pub from: [u32; 2],
    /// Target grid position [col, row].
    pub to: [u32; 2],
    /// Override traversability.
    pub traversability: EdgeTraversability,
    /// Override distance. If `None`, uses `cell_size_mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_mm: Option<u64>,
}

/// A non-grid cell with explicit edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JunctionCell {
    /// Cell ID (must be >= width * depth to avoid collision with grid IDs).
    pub id: u32,
    /// World position in mm.
    pub world_pos: [f64; 2],
    pub kind: CellKind,
    pub zone: u32,
    /// Explicit edges from this junction cell.
    pub edges: Vec<JunctionEdge>,
}

/// An edge from a junction cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JunctionEdge {
    /// Target cell ID (can be grid cell or another junction).
    pub target: u32,
    pub traversability: EdgeTraversability,
    pub distance_mm: u64,
    /// Whether this edge is bidirectional.
    #[serde(default = "default_true")]
    pub bidirectional: bool,
}

fn default_true() -> bool {
    true
}

pub fn cell_kind_to_char(kind: CellKind) -> char {
    match kind {
        CellKind::Storage => 'S',
        CellKind::Aisle => 'A',
        CellKind::Open => '.',
        CellKind::Highway => 'H',
        CellKind::StationArea => 'P',
        CellKind::Relay => 'R',
        CellKind::Junction => 'J',
        CellKind::Blank => ' ',
        CellKind::Presentation => 'T',
        CellKind::QueueCell => 'Q',
        CellKind::Charging => 'C',
        CellKind::Exchange => 'X',
    }
}

/// Encode a row of CellKinds as a compact string (for debugging).
pub fn encode_cell_kinds(kinds: &[CellKind]) -> String {
    kinds.iter().map(|k| cell_kind_to_char(*k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_station_definition_defaults_new_choreography_fields() {
        let station: StationDefinition = serde_json::from_str(
            r#"{
                "id": 7,
                "position": [3, 4],
                "access_position": [3, 3]
            }"#,
        )
        .unwrap();

        assert_eq!(station.presentation_cell, [3, 3]);
        assert!(station.queue_path.is_empty());
        assert_eq!(station.exit_direction, CardinalDirection::East);
        assert_eq!(station.station_type, StationType::Pick);
    }
}
