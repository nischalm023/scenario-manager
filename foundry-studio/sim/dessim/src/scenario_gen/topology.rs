//! Within `scenario_gen`, `topology` is the READING of a map's structure: which
//! zones and relays and stations it holds, which of its cells are storage, and
//! how that storage is laid out in millimetres.
//!
//! Most of it exists because a compiled desmap map states its structure only as
//! cell kinds on a grid, so a rack's physical row, the side of the aisle it
//! faces and the height of each of its shelves all have to be rediscovered from
//! adjacency and column alignment. [`Racking`] is where that rediscovery ends: a
//! map that can state its own layout hands one over instead.

use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};

use desgraph::{CellId, CellKind, RelayId, WorldPos, ZoneId};
use desmap::{BlockSide, RelayPosition, Station, VtmPosition, WarehouseMap, Zone, ZoneHolds};
use serde::{Deserialize, Serialize};

use crate::model::SimDuration;
use crate::model::{RackId, ToteDesignation};
use crate::robots::catalog::RobotDeployment;

use super::types::*;

pub(super) fn extract_zone_data(map: &WarehouseMap) -> Vec<ZoneSetupData> {
    map.zones
        .iter()
        .filter(|z| !z.relays.is_empty())
        .map(|z| ZoneSetupData { zone_id: z.id, relay_ids: z.relays.iter().copied().collect() })
        .collect()
}

pub(super) fn extract_relay_data(map: &WarehouseMap) -> Vec<RelaySetupData> {
    map.relays
        .iter()
        .map(|r: &RelayPosition| RelaySetupData {
            relay_id: r.id,
            vtm_access: r.vtm_access.anchor(),
            htm_access: r.htm_access,
            side: r.side,
        })
        .collect()
}

pub(super) fn extract_station_data(map: &WarehouseMap) -> Vec<StationSetupData> {
    map.stations
        .iter()
        .map(|s: &Station| StationSetupData {
            station_id: s.id,
            presentation_cell: s.delivery_cell(),
            bins: s.bins,
            station_type: s.station_type,
            // desmap states it in milliseconds; this is dessim's boundary.
            conveyor_transit: s.conveyor_transit_ms.map(SimDuration::ms),
            order_presentation_cell: s.order_presentation_cell,
            conveyor: s.conveyor_group,
        })
        .collect()
}

/// Per-zone storage cells: Relay cells for relay systems, Storage cells for RTP.
pub(super) fn extract_zone_storage(map: &WarehouseMap) -> BTreeMap<ZoneId, BTreeSet<CellId>> {
    let mut zone_storage: BTreeMap<ZoneId, BTreeSet<CellId>> = BTreeMap::new();
    for zone in &map.zones {
        let cells: BTreeSet<CellId> = zone
            .cells
            .iter()
            .copied()
            .filter(|&cell_id| {
                map.graph.cell(cell_id).map(|c| matches!(c.kind, CellKind::Relay | CellKind::Storage)).unwrap_or(false)
            })
            .collect();
        if !cells.is_empty() {
            zone_storage.insert(zone.id, cells);
        }
    }
    zone_storage
}

/// Build rack_cell -> vtm_access_cell mapping for ALL Relay cells.
///
/// For cells with a RelayPosition, use the position's vtm_access.
/// For orphan rack cells (no RelayPosition), find the adjacent Aisle neighbor.
pub(super) fn build_rack_to_vtm_access(map: &WarehouseMap) -> Result<BTreeMap<CellId, CellId>, String> {
    let mut mapping: BTreeMap<CellId, CellId> =
        map.relays.iter().map(|r| (r.position, r.vtm_access.anchor())).collect();

    for cell in map.graph.cells() {
        if cell.kind != CellKind::Relay {
            continue;
        }
        if mapping.contains_key(&cell.id) {
            continue;
        }
        let aisle = map
            .graph
            .neighbors(cell.id)
            .iter()
            .find(|(nid, _)| map.graph.cell(*nid).map(|c| c.kind == CellKind::Aisle).unwrap_or(false))
            .map(|(nid, _)| *nid)
            .ok_or_else(|| format!("rack cell {:?} has no adjacent Aisle cell — zone layout is broken", cell.id))?;
        mapping.insert(cell.id, aisle);
    }

    Ok(mapping)
}

/// Which side of its aisle each rack sits on.
///
/// A VTM cycle is side-bound; the relay inventory index uses this map to
/// pair a rack only with same-side relays.
///
/// A relay already names the rack cell it sits on (`RelayPosition::position`),
/// so that answers most racks outright. Rack cells with no relay of their own
/// — cells a map keeps as storage but reaches through no handoff shelf — fall
/// back to the column heuristic: on a map whose relays stand on the rack cells
/// themselves, every rack column in a zone shares its grid_x with a relay
/// column on the same side (`desmap::design::compile::stamp_relay_cells`). The
/// direct key is what keeps this working for a map whose handoff cell is out in
/// the lane instead, where the two columns differ (`worldmap`, and
/// `sim/dessim/PORT-NOTES.md` §5.1).
///
/// Returns a `RackId → BlockSide` map covering every rack in `rack_cells` that
/// either carries a relay or shares a column with one. Racks answering to
/// neither are omitted (the caller decides whether to require them).
pub(super) fn build_rack_side(
    map: &WarehouseMap,
    rack_cells: &BTreeMap<RackId, CellId>,
) -> BTreeMap<RackId, BlockSide> {
    let mut cell_side: FxHashMap<CellId, BlockSide> = FxHashMap::default();
    // Column grid_x → side, grouped per zone. Two zones can share a grid_x
    // (different aisles at different rows), so we key by (zone, grid_x). Both
    // the rack column and the handoff column are recorded: they are the same
    // column on a desmap-compiled map and different ones when the handoff cell
    // sits out in the lane, and a rack cell with no relay of its own has to
    // find its column either way.
    let mut col_side: FxHashMap<(ZoneId, u32), BlockSide> = FxHashMap::default();
    for r in &map.relays {
        cell_side.insert(r.position, r.side);
        for anchor in [r.position, r.htm_access] {
            let Some(cell) = map.graph.cell(anchor) else { continue };
            let Some(gp) = cell.grid_pos else { continue };
            col_side.insert((cell.zone, gp.x), r.side);
        }
    }

    rack_cells
        .iter()
        .filter_map(|(&rack_id, &cell_id)| {
            if let Some(&side) = cell_side.get(&cell_id) {
                return Some((rack_id, side));
            }
            let cell = map.graph.cell(cell_id)?;
            let gp = cell.grid_pos?;
            let side = col_side.get(&(cell.zone, gp.x)).copied()?;
            Some((rack_id, side))
        })
        .collect()
}

/// A storage column ("rack") placed on the map.
///
/// `cell` anchors zone membership, VTM access, and grid attribution.
/// `world_pos` overrides the anchor cell's physical position along the rack
/// row; it is `Some` only for entries in zones that decouple slot layout
/// from the grid footprint. `floor_m` is the slot's fork-interface height
/// for zones with an explicit level table (each slot is then its own rack
/// entity); `None` means heights come from the uniform per-column model.
/// `side` is which side of its aisle the rack sits on, which a VTM cycle is
/// bound by.
#[derive(Clone, Copy, Debug)]
pub struct RackColumn {
    pub cell: CellId,
    pub world_pos: Option<WorldPos>,
    pub floor_m: Option<f64>,
    pub side: Option<BlockSide>,
    /// What this slot's steel is for, and therefore what starts in it.
    pub uses: RackUse,
}

/// What one storage position is FOR.
///
/// The map states two things and this is their product: what the BAY holds
/// (world protocol's `storage.holds`, per bay) and what the AISLE holds (the
/// same statement, when every bay of the aisle agrees). Three answers come out
/// of two, and each starts life differently, which is the whole reason the
/// distinction is drawn:
///
/// | bay | aisle | this | at t=0 |
/// |---|---|---|---|
/// | inventory | inventory | `Inventory` | a stock tote |
/// | order | inventory | `OrderDepot` | an EMPTY order tote |
/// | order | order | `OrderBuffer` | nothing |
///
/// The depot and the buffer are both order racking and neither may take stock,
/// but a depot slot is where an unused order tote WAITS and a buffer slot is
/// capacity for a closed order that has not been picked yet. Seeding a buffer
/// slot spends that capacity; leaving a depot slot empty means close-out has
/// no tote to load an order into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RackUse {
    /// Stock. What every map said before order buffers existed.
    #[default]
    Inventory,
    /// The row of empty order totes close-out draws on.
    OrderDepot,
    /// The order buffer: where a closed order rests until it is due out.
    OrderBuffer,
}

impl RackUse {
    /// Read the two statements together. `bay` is the bay's own `holds`;
    /// `zone` is its aisle's, which is `Order` only when every bay agrees.
    pub fn read(bay: ZoneHolds, zone: ZoneHolds) -> Self {
        match (bay, zone) {
            (ZoneHolds::Inventory, _) => Self::Inventory,
            (ZoneHolds::Order, ZoneHolds::Order) => Self::OrderBuffer,
            (ZoneHolds::Order, ZoneHolds::Inventory) => Self::OrderDepot,
        }
    }

    /// Which tote pool may rest here. `None` for a buffer slot, whose
    /// occupant is a closed order rather than a member of either pool.
    pub fn pool(self) -> Option<ToteDesignation> {
        match self {
            Self::Inventory => Some(ToteDesignation::Inventory),
            Self::OrderDepot => Some(ToteDesignation::Order),
            Self::OrderBuffer => None,
        }
    }

    /// Whether stock may be stored here. Only inventory racking.
    pub fn takes_stock(self) -> bool {
        matches!(self, Self::Inventory)
    }

    /// Which ZONES hold racking of this kind. The buffer is whole aisles and
    /// is skipped wholesale by every reservation that is not a buffer release;
    /// the depot lives inside inventory aisles, so it is reached through the
    /// same zones stock is.
    pub fn racking(self) -> ZoneHolds {
        match self {
            Self::Inventory | Self::OrderDepot => ZoneHolds::Inventory,
            Self::OrderBuffer => ZoneHolds::Order,
        }
    }
}

/// How a map's storage is laid out: what the racks are, where each one is,
/// where a VTM meets each handoff shelf, and how many totes a rack holds.
///
/// This is the seam between "which map am I running" and the rest of scenario
/// generation. A desmap-compiled map derives it from grid geometry
/// ([`racking_from_map`]); a world-protocol document states it outright, and
/// its loader builds this value directly instead of leaving the grid to be
/// re-read. Everything downstream consumes the layout and never asks where it
/// came from.
#[derive(Clone, Debug, Default)]
pub struct Racking {
    /// Source models and cohort counts, resolved after run settings are final.
    pub robot_deployment: RobotDeployment,
    /// `RackId(i)` is the index into this list.
    pub columns: Vec<RackColumn>,
    /// Relay → the exchange position along the row where a VTM meets its
    /// lowest board. Absent → the relay's standing cell centre.
    ///
    /// Keyed by relay rather than by cell because the two are not one-to-one:
    /// where board pitch and lane pitch are incommensurate, several handoff
    /// positions of one run share the lane cell somebody stands on, and their
    /// boards still sit at different places along the run.
    pub relay_exchange_pos: BTreeMap<RelayId, WorldPos>,
    /// Zone → the heights, bottom-up, of the stack of boards each handoff
    /// position carries. The lowest is the height the exchange happens at.
    pub board_floors_m: BTreeMap<ZoneId, Vec<f64>>,
    /// Tote positions per rack.
    pub vertical_tote_capacity: u32,
    /// Initial-position candidates in rail coordinates, grouped by zone.
    /// Empty for grid maps, which use their existing aisle cells.
    pub vtm_start_positions: FxHashMap<ZoneId, Vec<VtmPosition>>,
}

/// Read a compiled desmap map's storage layout out of its grid geometry.
///
/// Every step here is a rediscovery: which cells are storage, which physical
/// row a cell belongs to, which side of the aisle that row is on, how high each
/// shelf sits. A world document states all four, which is why its loader builds
/// a [`Racking`] directly rather than coming through here.
pub fn racking_from_map(map: &WarehouseMap, vertical_tote_capacity: u32) -> Result<Racking, String> {
    let is_relay = !map.relays.is_empty();
    let storage_cells = collect_storage_cells(map, is_relay);
    let layout = build_rack_layout(map, &storage_cells)?;
    let cells: BTreeMap<RackId, CellId> =
        layout.columns.iter().enumerate().map(|(i, r)| (RackId(i as u32), r.cell)).collect();
    let sides = build_rack_side(map, &cells);
    let columns = layout
        .columns
        .iter()
        .enumerate()
        .map(|(i, rack)| RackColumn { side: sides.get(&RackId(i as u32)).copied(), ..*rack })
        .collect();
    // desmap puts a relay on the rack cell it serves, so its cell-keyed
    // exchange positions carry over one for one.
    let relay_exchange_pos = map
        .relays
        .iter()
        .filter_map(|relay| Some((relay.id, *layout.relay_exchange_pos.get(&relay.position)?)))
        .collect();
    Ok(Racking {
        robot_deployment: RobotDeployment::default(),
        columns,
        relay_exchange_pos,
        board_floors_m: board_floors_from_zones(map),
        vertical_tote_capacity,
        vtm_start_positions: FxHashMap::default(),
    })
}

/// The distinct board heights of each zone, bottom-up, read off the zone's
/// materialized slot list. A zone with no explicit slots states no board
/// heights, and the VTM model falls back to its own buffer constant.
fn board_floors_from_zones(map: &WarehouseMap) -> BTreeMap<ZoneId, Vec<f64>> {
    map.zones
        .iter()
        .filter_map(|zone| {
            let slots = zone.slots.as_ref()?;
            let mut boards: Vec<(u32, f64)> = slots
                .iter()
                .filter(|s| s.diving_board)
                .map(|s| (s.diving_board_slot.unwrap_or(0), s.y / 1000.0))
                .collect();
            boards.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap()));
            boards.dedup();
            let floors: Vec<f64> = boards.into_iter().map(|(_, y)| y).collect();
            (!floors.is_empty()).then_some((zone.id, floors))
        })
        .collect()
}

/// Output of [`build_rack_layout`]: the storage racks plus, for explicit-slot
/// zones, each relay cell's board-0 exchange position — the physical spot
/// along the row where the VTM meets the diving board. Both are in the same
/// interpolated grid-mm frame, so VTM cycle distances and the docked
/// positions stay consistent.
pub(super) struct RackLayout {
    pub columns: Vec<RackColumn>,
    pub relay_exchange_pos: BTreeMap<CellId, WorldPos>,
}

/// Lay out storage racks over the storage cells.
///
/// Default: one rack column per cell, in `storage_cells` order — `RackId(i)`
/// is the index into `columns`, so this preserves the historical
/// rack-per-cell enumeration exactly when no zone opts in.
///
/// For relay zones with explicit `slots`, **every storage slot is its own
/// directly encoded rack**: the slot's anchor `cell` places it on a grid
/// rack column, its `x` (one shared frame per zone) sets the physical
/// position along the row, and its `y` the fork-interface height.
/// Diving-board slots are relay exchange positions, not racks; the lowest
/// board (`diving_board_slot == 0`) at each cell yields that relay's
/// interpolated exchange position in `relay_exchange_pos`. Storage x and
/// board x share one frame, so both normalize onto the run consistently.
pub(super) fn build_rack_layout(map: &WarehouseMap, storage_cells: &[CellId]) -> Result<RackLayout, String> {
    let zone_by_id: FxHashMap<ZoneId, &Zone> = map.zones.iter().map(|z| (z.id, z)).collect();

    // Group key for cells in explicit-slot zones: (zone, grid_x) identifies
    // one physical rack row (a vertical run of cells along the aisle).
    let group_key = |cell_id: CellId| -> Option<(ZoneId, u32)> {
        let cell = map.graph.cell(cell_id)?;
        if !zone_by_id.get(&cell.zone).is_some_and(|z| z.slots.is_some()) {
            return None;
        }
        Some((cell.zone, cell.grid_pos?.x))
    };

    let mut groups: BTreeMap<(ZoneId, u32), Vec<CellId>> = BTreeMap::new();
    for &cell_id in storage_cells {
        if let Some(key) = group_key(cell_id) {
            groups.entry(key).or_default().push(cell_id);
        }
    }

    let mut columns = Vec::with_capacity(storage_cells.len());
    let mut relay_exchange_pos: BTreeMap<CellId, WorldPos> = BTreeMap::new();
    for &cell_id in storage_cells {
        let Some(key) = group_key(cell_id) else {
            columns.push(RackColumn {
                cell: cell_id,
                world_pos: None,
                floor_m: None,
                side: None,
                uses: RackUse::Inventory,
            });
            continue;
        };
        // Taking the group is what makes the first cell of a run the only
        // one that emits it; later cells of the same run find it gone.
        let Some(cells) = groups.remove(&key) else { continue };
        let slots = zone_by_id
            .get(&key.0)
            .and_then(|zone| zone.slots.as_ref())
            .ok_or_else(|| format!("zone {:?}: column {} was grouped without a slot table", key.0, key.1))?;
        let span = || -> Option<(WorldPos, WorldPos)> {
            let first = map.graph.cell(*cells.first()?)?.world_pos?;
            let last = map.graph.cell(*cells.last()?)?.world_pos?;
            Some((first, last))
        }();

        // The zone-wide x frame, normalized onto this run's physical span;
        // relative spacing is preserved through the zone-width normalization
        // in VTM kinematics.
        let x_min = slots.iter().map(|s| s.x).fold(f64::INFINITY, f64::min);
        let x_max = slots.iter().map(|s| s.x).fold(f64::NEG_INFINITY, f64::max);
        let x_span = x_max - x_min;

        // Grid rows covered by this run, for resolving each slot's anchor
        // cell from its authored [col, row].
        let row_to_cell: FxHashMap<u32, CellId> =
            cells.iter().filter_map(|&cid| Some((map.graph.cell(cid)?.grid_pos?.y, cid))).collect();

        // Interpolate a slot's `x` (shared frame) onto the run's physical span.
        let interp = |x: f64| -> Option<WorldPos> {
            let t = if x_span > 0.0 { ((x - x_min) / x_span).clamp(0.0, 1.0) } else { 0.5 };
            span.map(|(first, last)| WorldPos {
                x_mm: first.x_mm + (last.x_mm - first.x_mm) * t,
                y_mm: first.y_mm + (last.y_mm - first.y_mm) * t,
            })
        };

        for slot in slots.iter().filter(|s| s.cell[0] == key.1) {
            let &anchor = row_to_cell.get(&slot.cell[1]).ok_or_else(|| {
                format!("zone {:?}: slot anchor cell {:?} is not a rack cell of column {}", key.0, slot.cell, key.1)
            })?;
            if slot.diving_board {
                // Board 0 is the exchange level the VTM meets; higher boards
                // are carried as floors elsewhere for the two-lane model.
                if slot.diving_board_slot == Some(0) {
                    if let Some(pos) = interp(slot.x) {
                        relay_exchange_pos.insert(anchor, pos);
                    }
                }
            } else {
                columns.push(RackColumn {
                    cell: anchor,
                    world_pos: interp(slot.x),
                    floor_m: Some(slot.y / 1000.0),
                    side: None,
                    // A desmap-compiled map states no holds; everything is
                    // inventory, which is what it meant before the kind existed.
                    uses: RackUse::Inventory,
                });
            }
        }
    }
    Ok(RackLayout { columns, relay_exchange_pos })
}

/// Collect all usable storage cells from the map, sorted by CellId.
pub(super) fn collect_storage_cells(map: &WarehouseMap, is_relay: bool) -> Vec<CellId> {
    let mut cells: Vec<CellId> = if is_relay {
        map.graph.cells().filter(|c| matches!(c.kind, CellKind::Storage | CellKind::Relay)).map(|c| c.id).collect()
    } else {
        map.graph.cells().filter(|c| c.kind == CellKind::Storage).map(|c| c.id).collect()
    };
    cells.sort();
    cells
}
