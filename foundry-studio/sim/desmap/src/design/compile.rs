//! Compile: `WarehouseDesign` v3 → `WarehouseMap`.

use crate::VtmPosition;
use crate::map::relay_access::restrict_relay_access;
use crate::map::{GridBounds, WarehouseMap, WarehouseMapBuilder};
use crate::station::{PresentationPoint, RelayPosition, Station};
use crate::types::*;
use crate::zone::{RackBlock, Zone};
use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::{
    graph_builder::build_initial_graph,
    grid_geometry::AxisGeometry,
    queue_edges::{restrict_queue_htm, validate_queue_adjacency},
    reachability::narrow_by_reachability,
    types::*,
};

/// Block geometry captured during zone stamping.
pub(super) struct BlockInfo {
    pub id: u32,
    pub top_left: GridPos,
    pub bottom_right: GridPos,
}

impl WarehouseDesign {
    /// Compile the v3 warehouse design into a full `WarehouseMap`.
    pub fn compile(&self) -> Result<WarehouseMap, String> {
        if self.version != 3 {
            return Err(format!("unsupported design version {}; expected 3", self.version));
        }
        let w = self.grid.width;
        let d = self.grid.depth;
        let cell_size = self.grid.cell_size_mm;

        // Axis geometry. Columns are always uniform; rows honour
        // `row_centers_mm` when the design supplies it. Both fall back to
        // `cell_size_mm`, so a design without centers compiles identically to
        // before this existed.
        let xgeo = AxisGeometry::uniform(w, cell_size);
        let ygeo = match &self.grid.row_centers_mm {
            Some(centers) => AxisGeometry::from_centers(centers, d, "row")?,
            None => AxisGeometry::uniform(d, cell_size),
        };

        // Anchoring rack slots needs the row geometry, so this follows `ygeo`.
        let materialized_slots = materialize_rack_geometry(&self.racks, &self.zones, &ygeo)?;

        // Phase 1-4: Stamp cell kinds and collect zone/block/relay data.
        let mut cell_kinds = vec![vec![CellKind::Open; w as usize]; d as usize];
        let mut cell_zones = vec![vec![0u32; w as usize]; d as usize];

        stamp_highways(&mut cell_kinds, &self.highways, w, d);
        let (block_infos, relay_positions, system_type) =
            stamp_zones(&mut cell_kinds, &mut cell_zones, &self.zones, w, d);
        stamp_stations(&mut cell_kinds, &self.stations, w, d);
        stamp_charging_zones(&mut cell_kinds, &self.charging_zones, w, d);
        validate_queue_adjacency(&self.stations)?;
        validate_and_stamp_clips(&mut cell_kinds, &cell_zones, &self.clip_boxes, w, d)?;

        let rack_orient = build_rack_orientations(&block_infos, &cell_kinds);
        let relay_cells: HashSet<u32> = relay_positions.iter().map(|r| r.position.0).collect();

        // Phase 5: Build initial graph, restrict queue HTM, then narrow by reachability.
        let initial_graph = build_initial_graph(
            &cell_kinds,
            &cell_zones,
            &rack_orient,
            &self.highways,
            &self.stations,
            &self.edge_exceptions,
            &self.junction_cells,
            &relay_cells,
            w,
            d,
            &xgeo,
            &ygeo,
        )?;
        let queue_graph = restrict_queue_htm(&initial_graph, &self.stations, w);
        let stations = build_stations(&self.stations, w);
        let graph = narrow_by_reachability(&queue_graph, &relay_positions, &stations);
        let graph = restrict_relay_access(&graph, &relay_positions)?;

        // Assemble.
        let zones = build_zones(
            &self.zones,
            &materialized_slots,
            &cell_zones,
            &self.junction_cells,
            &relay_positions,
            w,
            d,
            &ygeo,
        );
        let blocks = build_blocks(&block_infos, &cell_kinds, w);

        let mut mb = WarehouseMapBuilder::new(system_type)
            .grid_bounds(GridBounds { width: w, depth: d, cell_size_mm: cell_size })
            .graph(graph);
        if let Some(fc) = &self.fleet {
            mb = mb.fleet(*fc);
        }
        for s in stations {
            mb = mb.add_station(s);
        }
        for r in relay_positions {
            mb = mb.add_relay(r);
        }
        for z in zones {
            mb = mb.add_zone(z);
        }
        for b in blocks {
            mb = mb.add_block(b);
        }
        Ok(mb.build())
    }
}

fn stamp_highways(cell_kinds: &mut [Vec<CellKind>], highways: &[HighwayDefinition], w: u32, d: u32) {
    for hd in highways {
        let (cs, rs) = (hd.origin[0] as usize, hd.origin[1] as usize);
        for r in rs..(rs + hd.height as usize).min(d as usize) {
            for c in cs..(cs + hd.width as usize).min(w as usize) {
                cell_kinds[r][c] = CellKind::Highway;
            }
        }
    }
}

fn stamp_zones(
    cell_kinds: &mut [Vec<CellKind>],
    cell_zones: &mut [Vec<u32>],
    zones: &[ZoneDef],
    w: u32,
    d: u32,
) -> (Vec<BlockInfo>, Vec<RelayPosition>, SystemType) {
    let has_relay = zones.iter().any(|z| matches!(z, ZoneDef::Relay(_)));
    let system_type = if has_relay { SystemType::Relay } else { SystemType::Rtp };

    let mut blocks = Vec::new();

    // Phase 1: Stamp all cell kinds and zones. Must complete for ALL zones
    // before relay position generation so that side approaches see the final grid.
    for zd in zones {
        match zd {
            ZoneDef::Relay(rz) => stamp_relay_cells(cell_kinds, cell_zones, rz, w, d, &mut blocks),
            ZoneDef::Rtp(rz) => stamp_rtp(cell_kinds, cell_zones, rz, w, d, &mut blocks),
        }
    }

    // Phase 2: Generate relay positions using the fully-stamped grid.
    let mut relays = Vec::new();
    let mut relay_id = 0u32;
    for zd in zones {
        if let ZoneDef::Relay(rz) = zd {
            generate_relay_positions(cell_kinds, rz, w, d, &mut relays, &mut relay_id);
        }
    }

    (blocks, relays, system_type)
}

/// Phase 1: stamp CellKind::Relay and CellKind::Aisle cells, record block geometry.
fn stamp_relay_cells(
    cell_kinds: &mut [Vec<CellKind>],
    cell_zones: &mut [Vec<u32>],
    rz: &RelayZone,
    w: u32,
    d: u32,
    blocks: &mut Vec<BlockInfo>,
) {
    let (oc, or) = (rz.origin[0], rz.origin[1]);
    let (rack_cols, aisle_col, fw): (Vec<(u32, BlockSide)>, u32, u32) = match rz.sides {
        Sidedness::Dual => (vec![(oc, BlockSide::Left), (oc + 2, BlockSide::Right)], oc + 1, 3),
        Sidedness::LeftOnly => (vec![(oc, BlockSide::Left)], oc + 1, 2),
        Sidedness::RightOnly => (vec![(oc + 1, BlockSide::Right)], oc, 2),
    };

    for &(col, _) in &rack_cols {
        for ro in 0..rz.length {
            let row = or + ro;
            if (row as usize) < d as usize && (col as usize) < w as usize {
                cell_kinds[row as usize][col as usize] = CellKind::Relay;
                cell_zones[row as usize][col as usize] = rz.id;
            }
        }
    }
    for ro in 0..rz.length {
        let row = or + ro;
        if (row as usize) < d as usize && (aisle_col as usize) < w as usize {
            cell_kinds[row as usize][aisle_col as usize] = CellKind::Aisle;
            cell_zones[row as usize][aisle_col as usize] = rz.id;
        }
    }

    blocks.push(BlockInfo {
        id: rz.id,
        top_left: GridPos { x: oc, y: or },
        bottom_right: GridPos { x: oc + fw - 1, y: or + rz.length - 1 },
    });
}

/// Phase 2: generate relay positions only where the rack's outer side is accessible.
fn generate_relay_positions(
    cell_kinds: &[Vec<CellKind>],
    rz: &RelayZone,
    w: u32,
    d: u32,
    relays: &mut Vec<RelayPosition>,
    relay_id: &mut u32,
) {
    let (oc, or) = (rz.origin[0], rz.origin[1]);
    let (rack_cols, aisle_col): (Vec<(u32, BlockSide)>, u32) = match rz.sides {
        Sidedness::Dual => (vec![(oc, BlockSide::Left), (oc + 2, BlockSide::Right)], oc + 1),
        Sidedness::LeftOnly => (vec![(oc, BlockSide::Left)], oc + 1),
        Sidedness::RightOnly => (vec![(oc + 1, BlockSide::Right)], oc),
    };

    for &(col, side) in &rack_cols {
        for ro in 0..rz.length {
            let row = or + ro;
            if !has_outer_side_approach(cell_kinds, col, row, side, w, d) {
                continue;
            }
            relays.push(RelayPosition {
                id: RelayId(*relay_id),
                position: CellId(row * w + col),
                htm_access: CellId(row * w + col),
                vtm_access: VtmPosition::Cell(CellId(row * w + aisle_col)),
                block: BlockId(rz.id),
                side,
            });
            *relay_id += 1;
        }
    }
}

/// A north-south rack relay is served through its outer side. The opposite
/// neighbor is the VTM aisle; north/south open cells are end caps and cannot
/// support the perpendicular HTM entry and back-out maneuver.
fn has_outer_side_approach(cell_kinds: &[Vec<CellKind>], col: u32, row: u32, side: BlockSide, w: u32, d: u32) -> bool {
    let approach_col = match side {
        BlockSide::Left => col.checked_sub(1),
        BlockSide::Right => col.checked_add(1).filter(|candidate| *candidate < w),
    };
    row < d
        && approach_col.is_some_and(|approach_col| {
            matches!(
                cell_kinds[row as usize][approach_col as usize],
                CellKind::Open | CellKind::Highway | CellKind::Charging
            )
        })
}

fn stamp_rtp(
    cell_kinds: &mut [Vec<CellKind>],
    cell_zones: &mut [Vec<u32>],
    rz: &RtpZone,
    w: u32,
    d: u32,
    blocks: &mut Vec<BlockInfo>,
) {
    let (oc, or) = (rz.origin[0], rz.origin[1]);
    for ro in 0..rz.rows {
        for co in 0..rz.cols {
            let (row, col) = (or + ro, oc + co);
            if (row as usize) < d as usize && (col as usize) < w as usize {
                cell_kinds[row as usize][col as usize] = CellKind::Storage;
                cell_zones[row as usize][col as usize] = rz.id;
            }
        }
    }
    blocks.push(BlockInfo {
        id: rz.id,
        top_left: GridPos { x: oc, y: or },
        bottom_right: GridPos { x: oc + rz.cols - 1, y: or + rz.rows - 1 },
    });
}

fn stamp_stations(cell_kinds: &mut [Vec<CellKind>], stations: &[StationDefinition], w: u32, d: u32) {
    let mut stamp = |pos: [u32; 2], kind: CellKind| {
        let (c, r) = (pos[0] as usize, pos[1] as usize);
        if r < d as usize && c < w as usize {
            cell_kinds[r][c] = kind;
        }
    };
    for sd in stations {
        stamp(sd.position, CellKind::StationArea);
        stamp(sd.presentation_cell, CellKind::Presentation);
        for qc in &sd.queue_path {
            stamp(*qc, CellKind::QueueCell);
        }
    }
}

fn stamp_charging_zones(cell_kinds: &mut [Vec<CellKind>], zones: &[ChargingZone], w: u32, d: u32) {
    for zone in zones {
        let (cs, rs) = (zone.origin[0] as usize, zone.origin[1] as usize);
        for r in rs..(rs + zone.height as usize).min(d as usize) {
            for c in cs..(cs + zone.width as usize).min(w as usize) {
                cell_kinds[r][c] = CellKind::Charging;
            }
        }
    }
}

fn validate_and_stamp_clips(
    cell_kinds: &mut [Vec<CellKind>],
    cell_zones: &[Vec<u32>],
    clip_boxes: &[ClipBox],
    w: u32,
    d: u32,
) -> Result<(), String> {
    for (i, cb) in clip_boxes.iter().enumerate() {
        let (oc, or) = (cb.origin[0] as usize, cb.origin[1] as usize);
        for row in or..or + cb.height as usize {
            for col in oc..oc + cb.width as usize {
                if row < d as usize && col < w as usize {
                    if cell_zones[row][col] != 0 {
                        return Err(format!(
                            "clip_boxes[{}] (origin [{}, {}], {}x{}) overlaps zone {} at cell ({}, {})",
                            i, cb.origin[0], cb.origin[1], cb.width, cb.height, cell_zones[row][col], col, row,
                        ));
                    }
                    cell_kinds[row][col] = CellKind::Blank;
                }
            }
        }
    }
    Ok(())
}

fn build_rack_orientations(
    block_infos: &[BlockInfo],
    cell_kinds: &[Vec<CellKind>],
) -> BTreeMap<(u32, u32), RackOrientation> {
    let mut lookup = BTreeMap::new();
    for bi in block_infos {
        let bw = bi.bottom_right.x - bi.top_left.x + 1;
        let bh = bi.bottom_right.y - bi.top_left.y + 1;
        let orient = if bh >= bw { RackOrientation::NorthSouth } else { RackOrientation::EastWest };
        for row in bi.top_left.y..=bi.bottom_right.y {
            for col in bi.top_left.x..=bi.bottom_right.x {
                if matches!(cell_kinds[row as usize][col as usize], CellKind::Storage | CellKind::Relay) {
                    lookup.insert((row, col), orient);
                }
            }
        }
    }
    lookup
}

fn build_stations(defs: &[StationDefinition], w: u32) -> Vec<Station> {
    defs.iter()
        .map(|sd| {
            let pos = CellId(sd.position[1] * w + sd.position[0]);
            let pc = CellId(sd.presentation_cell[1] * w + sd.presentation_cell[0]);
            let qp = sd.queue_path.iter().map(|p| CellId(p[1] * w + p[0])).collect();
            Station {
                id: StationId(sd.id),
                outbound_role: None,
                position: pos,
                // A v3 design states one presentation cell per station.
                presentation_points: vec![PresentationPoint { presentation_cell: pc, queue_path: qp }],
                edge: sd.edge,
                // Same reason as the ports below: a v3 design has no fixed
                // conveyance, so it has no handover but the presentation.
                handoff_queues: Vec::new(),
                exit_direction: sd.exit_direction,
                bins: sd.bins,
                station_type: sd.station_type,
                // A `StationDefinition` has no ports: fixed conveyance is a
                // world-protocol concept and this is the v3 design compiler.
                dropoff_cell: None,
                collection_cell: None,
                order_presentation_cell: None,
                conveyance_accepts: None,
                conveyor_transit_ms: None,
                conveyor_group: None,
            }
        })
        .collect()
}

/// Validate per-zone rack geometry: explicit slot lists.
/// Stamp each relay zone's rack placements into an explicit slot list with
/// **absolute** anchor cells, ready for the compiled [`Zone::slots`].
///
/// This is where prototype geometry meets the grid: a prototype slot has no
/// cell of its own — its anchor cell is materialized here from the slot's
/// position along the run. Column from the placement side (mirror of
/// `stamp_relay_cells`: Dual → Left 0 / Right 2, LeftOnly → 0,
/// RightOnly → 1); row by nearest row *center* on the zone-shared x frame:
///
/// ```text
/// t      = (x - x_min) / (x_max - x_min)   // over the zone's ENTIRE set
/// target = center(first_row) + t * (center(last_row) - center(first_row))
/// row    = the zone row whose center is nearest `target`  // ties upward
/// ```
///
/// The slot extent is anchored onto the zone's row-center extent, then the
/// nearest center wins — so anchoring reads real row positions rather than row
/// *indices*. On a uniform axis centers are linear in index, which collapses
/// this to the `round(t * (length - 1))` it replaces (ties upward matches
/// `f64::round`), so uniform maps are unaffected. On a `row_centers_mm` axis
/// the two differ: index-proportional anchoring assumes even pitch, and
/// cencora's alternates 645/645/850. It agrees there only because 18 boards
/// meet 18 rows — one board per row makes both rules the identity.
///
/// Keep the zone-wide frame: a per-side frame silently moves anchor cells.
///
/// No upper bound is enforced on x: slot positions legitimately overhang
/// `zone_width_mm` (the survey data does); downstream normalizes spans.
fn materialize_rack_geometry(
    racks: &[RackDef],
    zone_defs: &[ZoneDef],
    ygeo: &AxisGeometry,
) -> Result<BTreeMap<u32, Vec<SlotDef>>, String> {
    let mut by_name: BTreeMap<&str, &RackDef> = BTreeMap::new();
    for rack in racks {
        if by_name.insert(rack.name.as_str(), rack).is_some() {
            return Err(format!("racks: duplicate rack name \"{}\"", rack.name));
        }
        if !rack.length_mm.is_finite() || rack.length_mm <= 0.0 {
            return Err(format!("rack \"{}\": length_mm must be finite and positive", rack.name));
        }
        for (index, slot) in rack.slots.iter().enumerate() {
            match (slot.diving_board, slot.diving_board_slot) {
                (true, None) => {
                    return Err(format!(
                        "rack \"{}\": slots[{}] diving board needs diving_board_slot",
                        rack.name, index
                    ));
                }
                (false, Some(_)) => {
                    return Err(format!(
                        "rack \"{}\": slots[{}] diving_board_slot is only valid on diving boards",
                        rack.name, index
                    ));
                }
                _ => {}
            }
            if !slot.x.is_finite() || !slot.y.is_finite() {
                return Err(format!("rack \"{}\": slots[{}] x/y must be finite", rack.name, index));
            }
            // Bottom-left frame: x measures from the steel's left end, so
            // a slot outside [0, length_mm] hangs off the frame.
            if slot.x < 0.0 || slot.x > rack.length_mm {
                return Err(format!(
                    "rack \"{}\": slots[{}] x {} is outside the rack envelope [0, {}]",
                    rack.name, index, slot.x, rack.length_mm
                ));
            }
        }
    }

    let relay_zones: Vec<_> =
        zone_defs.iter().filter_map(|zd| if let ZoneDef::Relay(rz) = zd { Some(rz) } else { None }).collect();

    // Explicit slot geometry switches the sim to flat per-slot racks
    // (vtc = 1), which is a map-wide mode: mixing explicit and implicit
    // relay zones would give the implicit ones one slot per column.
    let with_geometry = relay_zones.iter().filter(|rz| rz.has_explicit_slot_geometry()).count();
    if with_geometry > 0 && with_geometry < relay_zones.len() {
        return Err(format!(
            "rack_placements must be set on all relay zones or none ({} of {} have it)",
            with_geometry,
            relay_zones.len()
        ));
    }

    let mut materialized: BTreeMap<u32, Vec<SlotDef>> = BTreeMap::new();
    for rz in &relay_zones {
        if rz.rack_placements.is_empty() {
            continue;
        }
        // Collect (col_offset, x, prototype slot) across all placements in
        // authored order; anchor rows need the zone-wide x extent, so rows
        // are derived after the full set is known.
        let mut placed: Vec<(u32, f64, &RackSlotDef)> = Vec::new();
        // Steel envelopes per side, for overlap checking: (side, start, end, index).
        let mut extents: Vec<(BlockSide, f64, f64, usize)> = Vec::new();
        for (index, placement) in rz.rack_placements.iter().enumerate() {
            let Some(rack) = by_name.get(placement.rack.as_str()) else {
                return Err(format!(
                    "zone {}: rack_placements[{}] references unknown rack \"{}\"",
                    rz.id, index, placement.rack
                ));
            };
            let col_offset = match (rz.sides, placement.side) {
                (Sidedness::Dual, BlockSide::Left) => 0,
                (Sidedness::Dual, BlockSide::Right) => 2,
                (Sidedness::LeftOnly, BlockSide::Left) => 0,
                (Sidedness::RightOnly, BlockSide::Right) => 1,
                (sides, side) => {
                    return Err(format!(
                        "zone {}: rack_placements[{}] side {:?} is not a rack side of a {:?} zone",
                        rz.id, index, side, sides
                    ));
                }
            };
            if !placement.x_offset_mm.is_finite() || placement.x_offset_mm < 0.0 {
                return Err(format!(
                    "zone {}: rack_placements[{}] x_offset_mm must be finite and non-negative",
                    rz.id, index
                ));
            }
            extents.push((placement.side, placement.x_offset_mm, placement.x_offset_mm + rack.length_mm, index));
            for slot in &rack.slots {
                placed.push((col_offset, placement.x_offset_mm + slot.x, slot));
            }
        }

        // Steel can't overlap: on each side the envelopes must be disjoint.
        // Touching is fine — tiled racks share legs, so consecutive offsets
        // differ by exactly length_mm.
        extents.sort_by(|a, b| (a.0 as u8, a.1).partial_cmp(&(b.0 as u8, b.1)).unwrap());
        for pair in extents.windows(2) {
            let [(side_a, _, end_a, index_a), (side_b, start_b, _, index_b)] = pair else { unreachable!() };
            if side_a == side_b && start_b < end_a {
                return Err(format!(
                    "zone {}: rack_placements[{}] and [{}] overlap on side {:?} ({} < {})",
                    rz.id, index_a, index_b, side_a, start_b, end_a
                ));
            }
        }

        if !placed.iter().any(|(_, _, s)| !s.diving_board) {
            return Err(format!("zone {}: materialized rack geometry has no storage slot", rz.id));
        }

        let x_min = placed.iter().map(|&(_, x, _)| x).fold(f64::INFINITY, f64::min);
        let x_max = placed.iter().map(|&(_, x, _)| x).fold(f64::NEG_INFINITY, f64::max);
        let span = x_max - x_min;
        let first_row = rz.origin[1];
        let last_row = first_row + rz.length.saturating_sub(1);
        let (y_first, y_last) = (ygeo.center(first_row), ygeo.center(last_row));
        // Nearest row center to the anchored slot position. `<=` lets the later
        // row win a tie, matching the `f64::round` this replaces.
        let row_for = |x: f64| {
            let t = if span > 0.0 { (x - x_min) / span } else { 0.5 };
            let target = y_first + t * (y_last - y_first);
            let mut best = (first_row, f64::INFINITY);
            for row in first_row..=last_row {
                let distance = (ygeo.center(row) - target).abs();
                if distance <= best.1 {
                    best = (row, distance);
                }
            }
            best.0
        };
        let slots: Vec<SlotDef> = placed
            .iter()
            .map(|&(col_offset, x, slot)| SlotDef {
                cell: [rz.origin[0] + col_offset, row_for(x)],
                x,
                y: slot.y,
                diving_board: slot.diving_board,
                diving_board_slot: slot.diving_board_slot,
            })
            .collect();

        // Board indices per relay cell: unique and contiguous from 0.
        let mut board_stacks: BTreeMap<[u32; 2], Vec<u32>> = BTreeMap::new();
        for slot in &slots {
            if let Some(board) = slot.diving_board_slot {
                board_stacks.entry(slot.cell).or_default().push(board);
            }
        }
        for (cell, mut stack) in board_stacks {
            stack.sort_unstable();
            if stack.iter().enumerate().any(|(expected, &board)| board != expected as u32) {
                return Err(format!(
                    "zone {}: diving_board_slot indices at cell {:?} must be contiguous from 0 (got {:?})",
                    rz.id, cell, stack
                ));
            }
        }

        materialized.insert(rz.id, slots);
    }
    Ok(materialized)
}

fn build_zones(
    zone_defs: &[ZoneDef],
    materialized_slots: &BTreeMap<u32, Vec<SlotDef>>,
    cell_zones: &[Vec<u32>],
    junctions: &[JunctionCell],
    relays: &[RelayPosition],
    w: u32,
    d: u32,
    ygeo: &AxisGeometry,
) -> Vec<Zone> {
    let mut names: BTreeMap<u32, String> = BTreeMap::new();
    names.insert(0, "zone_0".into());
    // vtm_count per zone: relay zones use vtm_count override or default 2; others 0.
    let mut vtm_counts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut zone_heights: BTreeMap<u32, u64> = BTreeMap::new();
    let mut zone_widths: BTreeMap<u32, Option<u64>> = BTreeMap::new();
    let mut shelf_counts: BTreeMap<u32, u16> = BTreeMap::new();
    let mut slot_defs: BTreeMap<u32, Vec<SlotDef>> = BTreeMap::new();
    for zd in zone_defs {
        match zd {
            ZoneDef::Relay(r) => {
                names.insert(r.id, format!("zone_{}", r.id));
                vtm_counts.insert(r.id, r.vtm_count.unwrap_or(2));
                zone_heights.insert(r.id, r.zone_height_mm);
                // A relay zone's aisle runs along its rows, so its physical length is the
                // row span. Identical to `length * cell_size_mm` on a uniform grid; on a
                // calibrated one it cannot know the real margin past the outermost relay
                // center (425mm on a cencora unit), so fixtures that need the exact
                // figure set `zone_width_mm` explicitly.
                let derived = ygeo.span(r.origin[1], r.origin[1] + r.length - 1).round() as u64;
                zone_widths.insert(r.id, r.zone_width_mm.or(Some(derived)));
                shelf_counts.insert(r.id, r.real_shelf_count);
                if let Some(slots) = materialized_slots.get(&r.id) {
                    // Rack placements, already materialized to absolute cells.
                    slot_defs.insert(r.id, slots.clone());
                }
            }
            ZoneDef::Rtp(r) => {
                names.insert(r.id, format!("zone_{}", r.id));
            }
        }
    }

    let mut cells_by_zone: BTreeMap<u32, Vec<CellId>> = BTreeMap::new();
    for row in 0..d {
        for col in 0..w {
            cells_by_zone.entry(cell_zones[row as usize][col as usize]).or_default().push(CellId(row * w + col));
        }
    }
    for jc in junctions {
        cells_by_zone.entry(jc.zone).or_default().push(CellId(jc.id));
    }

    let mut relays_by_zone: BTreeMap<u32, Vec<RelayId>> = BTreeMap::new();
    for r in relays {
        let z = cell_zones[(r.vtm_access.anchor().0 / w) as usize][(r.vtm_access.anchor().0 % w) as usize];
        relays_by_zone.entry(z).or_default().push(r.id);
    }

    let mut all_ids: BTreeSet<u32> = cells_by_zone.keys().copied().collect();
    all_ids.insert(0);
    all_ids
        .iter()
        .map(|&zid| Zone {
            id: ZoneId(zid),
            name: names.get(&zid).cloned().or_else(|| Some(format!("zone_{}", zid))),
            cells: cells_by_zone.get(&zid).cloned().unwrap_or_default(),
            relays: relays_by_zone.get(&zid).cloned().unwrap_or_default(),
            vtm_count: vtm_counts.get(&zid).copied().unwrap_or(0),
            // v3 designs have no buffers; every zone they compile is inventory.
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: zone_heights.get(&zid).copied().unwrap_or(9000),
            zone_width_mm: zone_widths.get(&zid).copied().flatten(),
            real_shelf_count: shelf_counts.get(&zid).copied().unwrap_or(10),
            slots: slot_defs.get(&zid).cloned(),
        })
        .collect()
}

fn build_blocks(infos: &[BlockInfo], cell_kinds: &[Vec<CellKind>], w: u32) -> Vec<RackBlock> {
    infos
        .iter()
        .map(|bi| {
            let mut rack = Vec::new();
            let mut aisle = Vec::new();
            for row in bi.top_left.y..=bi.bottom_right.y {
                for col in bi.top_left.x..=bi.bottom_right.x {
                    let id = CellId(row * w + col);
                    match cell_kinds[row as usize][col as usize] {
                        CellKind::Storage | CellKind::Relay => rack.push(id),
                        CellKind::Aisle => aisle.push(id),
                        CellKind::Blank => {}
                        _ => rack.push(id),
                    }
                }
            }
            RackBlock {
                id: BlockId(bi.id),
                top_left: bi.top_left,
                bottom_right: bi.bottom_right,
                rack_cells: rack,
                aisle_cells: aisle,
            }
        })
        .collect()
}
