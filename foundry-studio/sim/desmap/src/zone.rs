//! Within `desmap`, `zone` is the COMPILED spatial-partition model: the
//! `Zone`/`Sector`/`RackBlock` entities a `WarehouseDesign` compiles into,
//! carrying each partition's cells, relays, VTM deployment, and materialized
//! per-slot rack geometry for the sim to consume.

use serde::{Deserialize, Serialize};

use crate::design::SlotDef;
use crate::types::*;

/// What the racking of a zone is FOR.
///
/// Two aisles of identical steel differ only in this: an order buffer is the
/// inventory racking with what it holds replaced (world protocol's `storage`
/// component, whose `holds` is `inventory` or `order`). The distinction is not
/// cosmetic -- inventory seeding must not fill a buffer, an order tote must not
/// be stored as inventory, and the two are worked by different VTM hardware --
/// so it rides on the zone rather than being inferred from a name.
///
/// Absent in a map that never says: everything is inventory, which is what
/// every map said before buffers existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneHolds {
    /// Inventory: SKUs waiting to be picked.
    #[default]
    Inventory,
    /// The order buffer: containers of picked goods waiting to go out.
    Order,
}

/// A zone -- an operational partition of the warehouse floor.
/// VTMs are zone-bound and cannot leave their assigned zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub id: ZoneId,
    /// Human-readable name (e.g. "zone_block_1_block_2").
    pub name: Option<String>,
    /// Cells belonging to this zone (coverage area).
    pub cells: Vec<CellId>,
    /// Relay positions owned by this zone.
    pub relays: Vec<RelayId>,
    /// Number of VTMs assigned to this zone. Derived from RelayZone.vtm_count
    /// (default 2 for A5 model). Zero for non-relay zones.
    #[serde(default)]
    pub vtm_count: u32,
    /// What this zone's racking holds. See [`ZoneHolds`].
    #[serde(default, skip_serializing_if = "is_inventory")]
    pub holds: ZoneHolds,
    /// The VTM hardware working this zone, when the map states it: the world
    /// protocol's `robot_class.configuration` for the cohort assigned here.
    /// Buffer aisles are worked by grippers, inventory aisles by vacuum heads,
    /// and the two cannot substitute for each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vtm_configuration: Option<String>,
    /// Physical racking height in mm.
    #[serde(default)]
    pub zone_height_mm: u64,
    /// Physical aisle length in mm. None = derived from zone geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_width_mm: Option<u64>,
    /// Actual physical shelf count per rack column.
    #[serde(default)]
    pub real_shelf_count: u16,
    /// Explicit per-slot rack geometry: every storage and diving-board slot
    /// with its own directly encoded position and **absolute** anchor cell,
    /// materialized by the compiler from the zone's rack placements. See
    /// `design::SlotDef`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<SlotDef>>,
}

impl Zone {
    /// Number of VTMs this zone deploys: `vtm_count` for relay zones, 0 for
    /// zones without relays. This is THE spawn predicate — dessim's
    /// `assign_vtm_zones` and `WarehouseMap::vtm_count` both derive from it,
    /// so spawn counts and reported counts cannot drift.
    pub fn vtms_deployed(&self) -> u32 {
        if self.relays.is_empty() { 0 } else { self.vtm_count }
    }
}

/// A sector -- a secondary partition orthogonal to zones.
/// Used for inventory organization, not navigation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sector {
    pub id: SectorId,
    pub cells: Vec<CellId>,
}

/// A rack block definition from the parametric generator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RackBlock {
    pub id: BlockId,
    /// Upper-left corner of the block in grid coordinates.
    pub top_left: GridPos,
    /// Lower-right corner.
    pub bottom_right: GridPos,
    /// All rack cell IDs within this block (bounding box minus aisles).
    ///
    /// In RELAY system type, some of these cells may be `CellKind::Relay`
    /// rather than `CellKind::Storage`. Relay positions sit on the rack edge
    /// columns and are valid tote storage locations (cantilever shelves).
    /// This matches the Java `getRackPositions()` behavior. Use `CellKind`
    /// to distinguish pure storage from relay cells.
    pub rack_cells: Vec<CellId>,
    /// Adjacent aisle cell IDs.
    pub aisle_cells: Vec<CellId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_zone() {
        let z = Zone {
            id: ZoneId(0),
            name: Some("zone_block_1_block_2".into()),
            cells: vec![CellId(0), CellId(1), CellId(2)],
            relays: vec![RelayId(0), RelayId(1)],
            vtm_count: 2,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 9000,
            zone_width_mm: Some(24000),
            real_shelf_count: 10,
            slots: Some(vec![
                SlotDef { cell: [0, 0], x: 356.0, y: 394.0, diving_board: true, diving_board_slot: Some(0) },
                SlotDef { cell: [0, 0], x: 320.0, y: 1580.0, diving_board: false, diving_board_slot: None },
            ]),
        };
        let json = serde_json::to_string(&z).unwrap();
        let z2: Zone = serde_json::from_str(&json).unwrap();
        assert_eq!(z, z2);
    }

    #[test]
    fn serde_round_trip_rack_block() {
        let b = RackBlock {
            id: BlockId(1),
            top_left: GridPos { x: 4, y: 10 },
            bottom_right: GridPos { x: 5, y: 19 },
            rack_cells: vec![CellId(100), CellId(101)],
            aisle_cells: vec![CellId(200)],
        };
        let json = serde_json::to_string(&b).unwrap();
        let b2: RackBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(b, b2);
    }
}

/// `holds` is omitted from serialized maps when it says what every pre-buffer
/// map said implicitly.
fn is_inventory(holds: &ZoneHolds) -> bool {
    matches!(holds, ZoneHolds::Inventory)
}
