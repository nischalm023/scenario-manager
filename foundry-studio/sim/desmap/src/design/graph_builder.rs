//! Build the initial warehouse graph with traversability flags.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::graph::{Cell, DestGate, Edge, WarehouseGraph, WarehouseGraphBuilder};
use crate::types::*;

use super::grid_geometry::AxisGeometry;
use super::types::*;

/// Build the full cell+edge graph before reachability narrowing.
///
/// `relay_cells` contains CellId.0 values of relay rack cells that have
/// HTM access points (relay positions). Only these cells allow HTM traversal,
/// and only to/from Open/Highway neighbors.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_initial_graph(
    cell_kinds: &[Vec<CellKind>],
    cell_zones: &[Vec<u32>],
    rack_orient: &BTreeMap<(u32, u32), RackOrientation>,
    highways: &[HighwayDefinition],
    station_defs: &[StationDefinition],
    edge_exceptions: &[EdgeException],
    junction_cells: &[JunctionCell],
    relay_cells: &HashSet<u32>,
    w: u32,
    d: u32,
    xgeo: &AxisGeometry,
    ygeo: &AxisGeometry,
) -> Result<WarehouseGraph, String> {
    let (gc, jc) = ((w * d) as usize, junction_cells.len());
    let mut builder = WarehouseGraphBuilder::with_capacity(gc + jc, gc * 4 + jc * 8);

    let station_owners = build_station_owner_map(station_defs, w);
    add_grid_cells(&mut builder, cell_kinds, cell_zones, rack_orient, &station_owners, w, d, xgeo, ygeo);
    add_junction_cells(&mut builder, junction_cells, w * d)?;

    let exc_map = build_exception_map(edge_exceptions, w);
    let (row_lane_dirs, col_lane_dirs) = build_lane_dir_maps(highways);
    let (presentation, position, pres_dir) = build_station_lookups(station_defs, w, &row_lane_dirs);

    add_cardinal_edges(
        &mut builder,
        cell_kinds,
        cell_zones,
        &exc_map,
        &row_lane_dirs,
        &col_lane_dirs,
        &presentation,
        &position,
        &pres_dir,
        relay_cells,
        rack_orient,
        // Traversability/flag template only. Per-edge distance comes from the
        // axis geometry, since it differs by direction and by index.
        Edge::uniform(0),
        w,
        d,
        xgeo,
        ygeo,
    );
    for jn in junction_cells {
        for je in &jn.edges {
            let e = Edge {
                traversability: je.traversability,
                distance_mm: je.distance_mm,
                dest_only: None,
                vtm_zone_gate: None,
                curve: None,
                htm_drive: Default::default(),
            };
            builder.add_edge(CellId(jn.id), CellId(je.target), e);
            if je.bidirectional {
                builder.add_edge(CellId(je.target), CellId(jn.id), e);
            }
        }
    }
    Ok(builder.build())
}

fn add_grid_cells(
    builder: &mut WarehouseGraphBuilder,
    cell_kinds: &[Vec<CellKind>],
    cell_zones: &[Vec<u32>],
    rack_orient: &BTreeMap<(u32, u32), RackOrientation>,
    station_owners: &HashMap<u32, StationId>,
    w: u32,
    d: u32,
    xgeo: &AxisGeometry,
    ygeo: &AxisGeometry,
) {
    for row in 0..d {
        for col in 0..w {
            let id = row * w + col;
            builder.add_cell(Cell {
                id: CellId(id),
                kind: cell_kinds[row as usize][col as usize],
                zone: ZoneId(cell_zones[row as usize][col as usize]),
                sector: None,
                grid_pos: Some(GridPos { x: col, y: row }),
                world_pos: Some(WorldPos { x_mm: xgeo.center(col), y_mm: ygeo.center(row) }),
                barcode: None,
                htm_rotation_forbidden: false,
                rack_orientation: rack_orient.get(&(row, col)).copied(),
                station_id: station_owners.get(&id).copied(),
            });
        }
    }
}

fn add_junction_cells(
    builder: &mut WarehouseGraphBuilder,
    junctions: &[JunctionCell],
    grid_id_max: u32,
) -> Result<(), String> {
    for jc in junctions {
        if jc.id < grid_id_max {
            return Err(format!("junction cell id {} collides with grid id space (0..{})", jc.id, grid_id_max));
        }
        builder.add_cell(Cell {
            id: CellId(jc.id),
            kind: jc.kind,
            zone: ZoneId(jc.zone),
            sector: None,
            grid_pos: None,
            world_pos: Some(WorldPos { x_mm: jc.world_pos[0], y_mm: jc.world_pos[1] }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
    }
    Ok(())
}

fn build_exception_map<'a>(exc: &'a [EdgeException], w: u32) -> BTreeMap<(u32, u32), &'a EdgeException> {
    exc.iter().map(|e| ((e.from[1] * w + e.from[0], e.to[1] * w + e.to[0]), e)).collect()
}

fn build_lane_dir_maps(
    highways: &[HighwayDefinition],
) -> (HashMap<(u32, u32), LaneDirection>, HashMap<(u32, u32), LaneDirection>) {
    let mut row_dirs = HashMap::new();
    let mut col_dirs = HashMap::new();
    for hd in highways {
        if hd.direction != LaneDirection::Bidirectional {
            if hd.width > hd.height {
                for r in hd.origin[1]..hd.origin[1] + hd.height {
                    for c in hd.origin[0]..hd.origin[0] + hd.width {
                        row_dirs.insert((c, r), hd.direction);
                    }
                }
            } else if hd.height > hd.width {
                for r in hd.origin[1]..hd.origin[1] + hd.height {
                    for c in hd.origin[0]..hd.origin[0] + hd.width {
                        col_dirs.insert((c, r), hd.direction);
                    }
                }
            }
        }
    }
    (row_dirs, col_dirs)
}

fn build_station_owner_map(defs: &[StationDefinition], w: u32) -> HashMap<u32, StationId> {
    let mut owners = HashMap::new();
    for sd in defs {
        let sid = StationId(sd.id);
        owners.insert(sd.presentation_cell[1] * w + sd.presentation_cell[0], sid);
        for qc in &sd.queue_path {
            owners.insert(qc[1] * w + qc[0], sid);
        }
    }
    owners
}

fn build_station_lookups(
    defs: &[StationDefinition],
    w: u32,
    row_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
) -> (HashSet<u32>, HashSet<u32>, HashMap<u32, LaneDirection>) {
    let mut presentation = HashSet::new();
    let mut position = HashSet::new();
    let mut pres_dir = HashMap::new();
    for sd in defs {
        let pres = sd.presentation_cell[1] * w + sd.presentation_cell[0];
        presentation.insert(pres);
        position.insert(sd.position[1] * w + sd.position[0]);
        if let Some(&dir) = row_lane_dirs.get(&(sd.presentation_cell[0], sd.presentation_cell[1])) {
            pres_dir.insert(pres, dir);
        }
    }
    (presentation, position, pres_dir)
}

#[allow(clippy::too_many_arguments)]
fn add_cardinal_edges(
    builder: &mut WarehouseGraphBuilder,
    cell_kinds: &[Vec<CellKind>],
    cell_zones: &[Vec<u32>],
    exc: &BTreeMap<(u32, u32), &EdgeException>,
    row_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
    col_lane_dirs: &HashMap<(u32, u32), LaneDirection>,
    presentation: &HashSet<u32>,
    position: &HashSet<u32>,
    pres_dir: &HashMap<u32, LaneDirection>,
    relay_cells: &HashSet<u32>,
    rack_orient: &BTreeMap<(u32, u32), RackOrientation>,
    default_edge: Edge,
    w: u32,
    d: u32,
    xgeo: &AxisGeometry,
    ygeo: &AxisGeometry,
) {
    for row in 0..d {
        for col in 0..w {
            let id = row * w + col;
            let ka = cell_kinds[row as usize][col as usize];
            if ka == CellKind::Blank {
                continue;
            }

            if col + 1 < w {
                let kb = cell_kinds[row as usize][(col + 1) as usize];
                if kb != CellKind::Blank {
                    let right = row * w + col + 1;
                    let ovr = htm_block_override(ka, kb, id, right, relay_cells, rack_orient, true, w);
                    let (hf, hr) = horizontal_lane_overrides(ovr, row_lane_dirs, col_lane_dirs, row, col);
                    let (df, dr) = horizontal_dest_only(id, right, presentation, position, pres_dir);
                    let (zf, zr) = horizontal_vtm_zone_gate(cell_zones, row, col, col + 1, ka, kb);
                    // Column pitch: center-to-center along x.
                    let dist = xgeo.pitch(col, col + 1).round() as u64;
                    add_edge_pair(builder, id, right, exc, default_edge, dist, hf, hr, df, dr, zf, zr);
                }
            }
            if row + 1 < d {
                let kb = cell_kinds[(row + 1) as usize][col as usize];
                if kb != CellKind::Blank {
                    let down = (row + 1) * w + col;
                    let ovr = htm_block_override(ka, kb, id, down, relay_cells, rack_orient, false, w);
                    let (mut hf, mut hr) = vertical_lane_overrides(ovr, row_lane_dirs, col_lane_dirs, row, col);
                    if ovr.is_none() {
                        // Block HTM entry into a presentation cell from a non-station,
                        // non-queue neighbor. Queue cells are the legitimate entry path
                        // for stations with a vertical (N/S) queue; without this exemption
                        // the queue-head → presentation edge would be blocked here even
                        // though restrict_queue_htm later promotes it to the allowed corridor.
                        // (Horizontal queues are governed by lane direction instead.)
                        if presentation.contains(&down) && !position.contains(&id) && ka != CellKind::QueueCell {
                            hf = Some(HtmTraversal::None);
                        }
                        if presentation.contains(&id) && !position.contains(&down) && kb != CellKind::QueueCell {
                            hr = Some(HtmTraversal::None);
                        }
                    }
                    let (zf, zr) = vertical_horizontal_vtm_zone_gate(cell_zones, row, col, ka, kb);
                    // Row pitch: center-to-center along y. This is the term that
                    // alternates 645/645/850 on a calibrated cencora rack column.
                    let dist = ygeo.pitch(row, row + 1).round() as u64;
                    add_edge_pair(builder, id, down, exc, default_edge, dist, hf, hr, None, None, zf, zr);
                }
            }
        }
    }
}

/// Determine HTM traversability for an edge between cells `id_a` (kind `ka`)
/// and `id_b` (kind `kb`).
///
/// Aisle, StationArea, Storage always block. Relay cells block unless they have
/// a relay position AND their neighbor is Open/Highway (the HTM ingress/egress)
/// AND the edge is along the rack's side (not the end cap).
/// Relay-to-Relay edges are always blocked.
/// Charging-to-Charging edges are blocked: each charging bay is accessed
/// from the adjacent aisle, never by traversing another charger.
fn htm_block_override(
    ka: CellKind,
    kb: CellKind,
    id_a: u32,
    id_b: u32,
    relay_cells: &HashSet<u32>,
    rack_orient: &BTreeMap<(u32, u32), RackOrientation>,
    is_horizontal: bool,
    w: u32,
) -> Option<HtmTraversal> {
    let blocked_a = matches!(ka, CellKind::Aisle | CellKind::StationArea | CellKind::Storage);
    let blocked_b = matches!(kb, CellKind::Aisle | CellKind::StationArea | CellKind::Storage);
    if blocked_a || blocked_b {
        return Some(HtmTraversal::None);
    }

    if ka == CellKind::Charging && kb == CellKind::Charging {
        return Some(HtmTraversal::None);
    }

    let relay_a = ka == CellKind::Relay;
    let relay_b = kb == CellKind::Relay;
    if !relay_a && !relay_b {
        return None; // Neither side is a relay cell — no override.
    }

    // Relay-to-Relay: always blocked.
    if relay_a && relay_b {
        return Some(HtmTraversal::None);
    }

    let open_like = |k: CellKind| matches!(k, CellKind::Open | CellKind::Highway | CellKind::Charging);

    // Relay-to-Open/Highway: allowed only if the relay cell has a relay position
    // AND the edge goes through the rack's side (not the end cap).
    // NorthSouth racks: side access is horizontal, end caps are vertical.
    // EastWest racks: side access is vertical, end caps are horizontal.
    if relay_a && open_like(kb) {
        if !relay_cells.contains(&id_a) {
            return Some(HtmTraversal::None);
        }
        let is_side = is_side_access(id_a, rack_orient, is_horizontal, w);
        return if is_side { None } else { Some(HtmTraversal::None) };
    }
    if relay_b && open_like(ka) {
        if !relay_cells.contains(&id_b) {
            return Some(HtmTraversal::None);
        }
        let is_side = is_side_access(id_b, rack_orient, is_horizontal, w);
        return if is_side { None } else { Some(HtmTraversal::None) };
    }

    // Relay adjacent to anything else (QueueCell, Presentation, etc.): blocked.
    Some(HtmTraversal::None)
}

/// Check whether an edge direction represents side access for a relay cell's rack.
/// NorthSouth racks have side access via horizontal edges.
/// EastWest racks have side access via vertical edges.
/// If no orientation is known, defaults to allowing access.
fn is_side_access(
    relay_id: u32,
    rack_orient: &BTreeMap<(u32, u32), RackOrientation>,
    is_horizontal: bool,
    w: u32,
) -> bool {
    let (row, col) = (relay_id / w, relay_id % w);
    match rack_orient.get(&(row, col)) {
        Some(RackOrientation::NorthSouth) => is_horizontal,
        Some(RackOrientation::EastWest) => !is_horizontal,
        None => true,
    }
}

fn horizontal_lane_overrides(
    base: Option<HtmTraversal>,
    row_dirs: &HashMap<(u32, u32), LaneDirection>,
    col_dirs: &HashMap<(u32, u32), LaneDirection>,
    row: u32,
    col: u32,
) -> (Option<HtmTraversal>, Option<HtmTraversal>) {
    let (mut f, mut r) = (base, base);
    if base.is_none() {
        // Row-lane directions apply only inside the explicit highway
        // rectangle. Earlier this was keyed by row alone, which accidentally
        // extended highway rules out to cells outside the highway on the same rows.
        let left_dir = row_dirs.get(&(col, row)).copied();
        let right_dir = row_dirs.get(&(col + 1, row)).copied();
        for d in [left_dir, right_dir].into_iter().flatten() {
            match d {
                LaneDirection::Westbound => f = Some(HtmTraversal::None),
                LaneDirection::Eastbound => r = Some(HtmTraversal::None),
                LaneDirection::NorthSouthOnly => {
                    f = Some(HtmTraversal::None);
                    r = Some(HtmTraversal::None);
                }
                _ => {}
            }
        }
        let ns_here = col_dirs.get(&(col, row)).copied() == Some(LaneDirection::NorthSouthOnly);
        let ns_right = col_dirs.get(&(col + 1, row)).copied() == Some(LaneDirection::NorthSouthOnly);
        if ns_here || ns_right {
            f = Some(HtmTraversal::None);
            r = Some(HtmTraversal::None);
        }
    }
    (f, r)
}

fn vertical_lane_overrides(
    base: Option<HtmTraversal>,
    row_dirs: &HashMap<(u32, u32), LaneDirection>,
    col_dirs: &HashMap<(u32, u32), LaneDirection>,
    row: u32,
    col: u32,
) -> (Option<HtmTraversal>, Option<HtmTraversal>) {
    let (mut f, mut r) = (base, base);
    if base.is_none() {
        let ew_top = row_dirs.get(&(col, row)).copied() == Some(LaneDirection::EastWestOnly);
        let ew_bot = row_dirs.get(&(col, row + 1)).copied() == Some(LaneDirection::EastWestOnly);
        if ew_top || ew_bot {
            f = Some(HtmTraversal::None);
            r = Some(HtmTraversal::None);
        }

        let top = col_dirs.get(&(col, row)).copied();
        let bot = col_dirs.get(&(col, row + 1)).copied();

        let block_south = matches!(top, Some(LaneDirection::Northbound | LaneDirection::EastWestOnly))
            || matches!(bot, Some(LaneDirection::Northbound | LaneDirection::EastWestOnly));
        let block_north = matches!(top, Some(LaneDirection::Southbound | LaneDirection::EastWestOnly))
            || matches!(bot, Some(LaneDirection::Southbound | LaneDirection::EastWestOnly));

        if block_south {
            f = Some(HtmTraversal::None);
        }
        if block_north {
            r = Some(HtmTraversal::None);
        }
    }
    (f, r)
}

fn horizontal_dest_only(
    id: u32,
    right: u32,
    presentation: &HashSet<u32>,
    position: &HashSet<u32>,
    pres_dir: &HashMap<u32, LaneDirection>,
) -> (Option<CellId>, Option<CellId>) {
    let fwd = if presentation.contains(&right) && !position.contains(&id) {
        match pres_dir.get(&right) {
            Some(LaneDirection::Eastbound) => Some(CellId(right)),
            _ if !pres_dir.contains_key(&right) => Some(CellId(right)),
            _ => None,
        }
    } else {
        None
    };
    let rev = if presentation.contains(&id) && !position.contains(&right) {
        match pres_dir.get(&id) {
            Some(LaneDirection::Westbound) => Some(CellId(id)),
            _ if !pres_dir.contains_key(&id) => Some(CellId(id)),
            _ => None,
        }
    } else {
        None
    };
    (fwd, rev)
}

fn horizontal_vtm_zone_gate(
    cz: &[Vec<u32>],
    row: u32,
    col: u32,
    col_r: u32,
    ka: CellKind,
    kb: CellKind,
) -> (Option<ZoneId>, Option<ZoneId>) {
    let f = {
        let z = cz[row as usize][col_r as usize];
        if z > 0 && kb == CellKind::Relay { Some(ZoneId(z)) } else { None }
    };
    let r = {
        let z = cz[row as usize][col as usize];
        if z > 0 && ka == CellKind::Relay { Some(ZoneId(z)) } else { None }
    };
    (f, r)
}

fn vertical_horizontal_vtm_zone_gate(
    cz: &[Vec<u32>],
    row: u32,
    col: u32,
    ka: CellKind,
    kb: CellKind,
) -> (Option<ZoneId>, Option<ZoneId>) {
    let f = {
        let z = cz[(row + 1) as usize][col as usize];
        if z > 0 && kb == CellKind::Relay { Some(ZoneId(z)) } else { None }
    };
    let r = {
        let z = cz[row as usize][col as usize];
        if z > 0 && ka == CellKind::Relay { Some(ZoneId(z)) } else { None }
    };
    (f, r)
}

/// Add both directions of one cardinal edge.
///
/// `default_distance_mm` is the center-to-center distance for this pair, taken
/// from the axis geometry by the caller (which knows the direction and the
/// indices). It applies to both directions, and is the fallback when an
/// `EdgeException` matches but omits `distance_mm`. `default` supplies the
/// traversability/flag template only — its own `distance_mm` is always replaced.
#[allow(clippy::too_many_arguments)]
fn add_edge_pair(
    builder: &mut WarehouseGraphBuilder,
    id_a: u32,
    id_b: u32,
    exc: &BTreeMap<(u32, u32), &EdgeException>,
    default: Edge,
    default_distance_mm: u64,
    hf: Option<HtmTraversal>,
    hr: Option<HtmTraversal>,
    df: Option<CellId>,
    dr: Option<CellId>,
    zf: Option<ZoneId>,
    zr: Option<ZoneId>,
) {
    let resolve = |a, b| {
        exc.get(&(a, b)).map_or(Edge { distance_mm: default_distance_mm, ..default }, |e| Edge {
            traversability: e.traversability,
            distance_mm: e.distance_mm.unwrap_or(default_distance_mm),
            dest_only: None,
            vtm_zone_gate: None,
            curve: None,
            htm_drive: Default::default(),
        })
    };
    let mut fwd = resolve(id_a, id_b);
    let mut rev = resolve(id_b, id_a);
    if let Some(h) = hf {
        fwd.traversability.htm_traversal = h;
    }
    if let Some(h) = hr {
        rev.traversability.htm_traversal = h;
    }
    if let Some(cell) = df {
        fwd.dest_only = Some(DestGate::Cell(cell));
    }
    if let Some(cell) = dr {
        rev.dest_only = Some(DestGate::Cell(cell));
    }
    if zf.is_some() {
        fwd.vtm_zone_gate = zf;
    }
    if zr.is_some() {
        rev.vtm_zone_gate = zr;
    }
    builder.add_edge(CellId(id_a), CellId(id_b), fwd);
    builder.add_edge(CellId(id_b), CellId(id_a), rev);
}
