//! Within `worldmap`, `cells` is the CELL TABLE: every place a robot or a tote
//! can be in dessim's world, the dense `CellId` minted for it, and the arcs
//! between them.
//!
//! It answers one question the protocol deliberately leaves to a consumer —
//! *what counts as a place* — twice, once per family:
//!
//! - A cell of a cell_grid is a place, and it keeps the identity the document
//!   gave it. The cell an HTM stands on to work a handoff shelf is one of these
//!   and nothing else: a relay binds a cell of the floor it travels, so there is
//!   no second entity at that spot for this module to reconcile.
//! - A bay's steel anchors storage and relay reporting without being floor.
//!   It receives one cell tied to the rack entity. VTM coordinates are carried
//!   separately; the loader creates no extra aisle cells for them.

use site_model::LoadError;
use std::cmp::Reverse;
use std::collections::BTreeMap;

use desgraph::{
    Cell, CellId, CellKind, CircularArc, DestGate, Edge, EdgeTraversability, HtmTraversal, RackOrientation, RelayId,
    StationId, WarehouseGraph, WarehouseGraphBuilder, WorldPos, ZoneId,
};
use site_model::components::spatial::Cell as WorldCell;
use site_model::components::spatial::Pose;
use site_model::ids::EntityId;
use site_model::pipeline::World;

use super::racking::Survey;
use super::{StepMm, WorldMapError};

/// Every dessim cell of a world, in the order their identities were minted.
pub struct CellTable {
    /// The travelled floor's extent as distinct centre columns and rows, and
    /// its tightest inter-centre gap -- what survives of the ordinal lattice
    /// now that no cell carries a grid position. Feeds the map header
    /// (`GridBounds`) only; nothing ranks or searches on it.
    pub floor_columns: u32,
    pub floor_rows: u32,
    pub tightest_step_mm: f64,
    cells: Vec<Cell>,
    arcs: Vec<(CellId, CellId, Edge)>,
    /// Which protocol entity each cell stands for, including rack anchors.
    pub cell_entities: BTreeMap<CellId, EntityId>,
    pub cells_by_entity: BTreeMap<EntityId, CellId>,
    /// The bay's steel, by the rack entity that is that bay.
    pub rack_cells: BTreeMap<EntityId, CellId>,
    /// The relay each exchange point became, by the bay it stands on and the
    /// floor cell its shelves bound. Both halves are needed: neighbouring bays
    /// share the cell somebody stands on where relay pitch and cell pitch are
    /// incommensurate, and their shelves are still different steel.
    pub relays_by_stand: BTreeMap<(EntityId, EntityId), RelayId>,
    /// The protocol relays each dessim relay gathers — one per shelf of the
    /// stack standing there.
    pub relay_shelves: BTreeMap<RelayId, Vec<EntityId>>,
    pub centres: BTreeMap<CellId, [f64; 2]>,
    /// Each cell's true tile in millimetres, id-indexed; `[0, 0]` for the
    /// minted exchange cells, which are points rather than floor.
    pub cell_sizes_mm: Vec<[f64; 2]>,
    /// Cell pose headings in the world frame, keyed by the same dense CellId.
    pub cell_headings_rad: Vec<f64>,
    /// Whether the DOCUMENT fixed each cell's tile, id-indexed alongside
    /// `cell_sizes_mm`. False for a cell this module mints, and false for a
    /// document cell that states no `size` and inherits no tiling pitch —
    /// the reader hands those SPEC §5's 900 mm default, so their entry in
    /// `cell_sizes_mm` is a square nobody declared. A consumer that has to
    /// tell declared floor from a bare point (desvis, when it decides which
    /// coplanar tier a cell draws on) must read this and not the size.
    pub cell_footprint_stated: Vec<bool>,
    /// The most common arc length in the graph: how far one move goes.
    pub nominal_step_mm: Option<f64>,
    /// The cells a station may name: everything a floor robot drives on.
    pub way_cells: Vec<CellId>,
}

/// One cell on its way into the table, before it has an identity.
struct Draft {
    centre: [f64; 2],
    /// The cell's tile in millimetres. Document cells carry their true
    /// footprint; minted rack cells get a nominal marker, since
    /// they are points of exchange rather than floor.
    size: [f64; 2],
    /// Whether the document stated that size. See
    /// `CellTable::cell_footprint_stated`.
    size_stated: bool,
    heading_rad: f64,
    rack_orientation: Option<RackOrientation>,
    kind: CellKind,
    zone: ZoneId,
    entity: EntityId,
    /// Rank breaks ties inside one lattice square so identity minting stays
    /// deterministic when two families land on the same coordinate.
    rank: u8,
}

impl CellTable {
    /// Stamp a station's ownership onto the cells it works over. Station
    /// resolution needs the table, so the labels arrive after the cells do.
    pub fn decorate(&mut self, decorations: &[(CellId, CellKind, StationId)]) {
        for &(cell, kind, station) in decorations {
            if let Some(entry) = self.cells.get_mut(cell.0 as usize) {
                entry.kind = kind;
                entry.station_id = Some(station);
            }
        }
    }

    /// Label the cells an HTM stands on to exchange a tote with a relay
    /// `Exchange`. A relay binds an ordinary floor cell, so the label arrives
    /// once the relays exist. Only `Open` floor is relabelled: a charger or a
    /// station's cell keeps the kind that says more about it.
    pub fn stamp_relay_access(&mut self, access: impl IntoIterator<Item = CellId>) {
        for cell in access {
            if let Some(entry) = self.cells.get_mut(cell.0 as usize)
                && entry.kind == CellKind::Open
            {
                entry.kind = CellKind::Exchange;
            }
        }
    }

    pub fn graph(&self) -> WarehouseGraph {
        let mut builder = WarehouseGraphBuilder::with_capacity(self.cells.len(), self.arcs.len());
        for cell in &self.cells {
            builder.add_cell(cell.clone());
        }
        for (from, to, edge) in &self.arcs {
            builder.add_edge(*from, *to, *edge);
        }
        builder.build()
    }

    pub fn cell_kind(&self, cell: CellId) -> Option<CellKind> {
        self.cells.get(cell.0 as usize).map(|c| c.kind)
    }
}

/// Build the table: take the document's cells, mint the racking's, derive the
/// lattice, hand out identities, then compile the arcs.
pub(super) fn build(world: &World, survey: &Survey, unit: StepMm) -> Result<CellTable, WorldMapError> {
    let mut drafts: Vec<Draft> = Vec::new();

    for (id, geometry) in &world.cells {
        let pose = world.entities[id].component::<Pose>("pose").map_err(LoadError::from)?;
        let heading_rad = if let Some(pose) = pose {
            let local = pose.heading_deg.rotate([1.0, 0.0]);
            let direction = match pose.frame.as_ref() {
                Some(frame) => world.frames.direction_to_primary(frame, local)?,
                None => local,
            };
            direction[1].atan2(direction[0])
        } else {
            0.0
        };
        drafts.push(Draft {
            centre: unit.point(geometry.centre),
            size: [unit.length(geometry.footprint[0]), unit.length(geometry.footprint[1])],
            size_stated: geometry.footprint_stated,
            heading_rad,
            rack_orientation: None,
            kind: way_kind(world, id.clone()),
            zone: ZoneId(0),
            entity: id.clone(),
            rank: 0,
        });
    }

    for (index, zone) in survey.zones.iter().enumerate() {
        let zone_id = ZoneId(index as u32 + 1);
        for bay in &zone.bays {
            let kind = if bay.stands.is_empty() { CellKind::Storage } else { CellKind::Relay };
            drafts.push(Draft {
                centre: bay.centre,
                size: [0.0, 0.0],
                size_stated: false,
                heading_rad: 0.0,
                rack_orientation: Some(if (bay.centre[0] - bay.aisle_centre[0]).abs() > 0.1 {
                    RackOrientation::NorthSouth
                } else {
                    RackOrientation::EastWest
                }),
                kind,
                zone: zone_id,
                entity: bay.rack.clone(),
                rank: 1,
            });
        }
    }

    if drafts.is_empty() {
        return Err(WorldMapError::NoCells);
    }

    // What used to be an ordinal lattice is now two numbers for the map
    // header: the travelled floor's distinct columns and rows, and its
    // tightest inter-centre gap. No ordinals are minted -- a cell's position
    // is its millimetre centre, and everything that used to count squares
    // (the A* heuristic, the parking registry, the station look-back, HTM
    // dispatch) measures millimetres now.
    let (floor_columns, floor_rows, tightest_step_mm) =
        floor_extent(drafts.iter().filter(|d| d.rank == 0).map(|d| d.centre));
    // Row-major over whole-millimetre centres, so `CellId` order still runs
    // the way desmap's `row * width + col` did and the rack enumeration
    // downstream lands on the same bays. (Off-lane cells used to snap to the
    // nearest lane row before sorting; they now sort at their own centres,
    // which reorders ids between rows but never within one.)
    let mut order: Vec<usize> = (0..drafts.len()).collect();
    order.sort_by_key(|&i| {
        let centre = drafts[i].centre;
        (centre[1].round() as i64, centre[0].round() as i64, drafts[i].rank, drafts[i].entity.clone())
    });

    let mut table = CellTable {
        floor_columns,
        floor_rows,
        tightest_step_mm,
        cells: Vec::with_capacity(drafts.len()),
        arcs: Vec::new(),
        cell_entities: BTreeMap::new(),
        cells_by_entity: BTreeMap::new(),
        rack_cells: BTreeMap::new(),
        relays_by_stand: BTreeMap::new(),
        relay_shelves: BTreeMap::new(),
        centres: BTreeMap::new(),
        cell_sizes_mm: Vec::new(),
        cell_headings_rad: Vec::new(),
        cell_footprint_stated: Vec::new(),
        nominal_step_mm: None,
        way_cells: Vec::new(),
    };

    for (next, &draft_index) in order.iter().enumerate() {
        let draft = &drafts[draft_index];
        let id = CellId(next as u32);
        table.cells.push(Cell {
            id,
            kind: draft.kind,
            zone: draft.zone,
            sector: None,
            // The protocol positions cells in millimetres and never in
            // ordinals; nothing downstream needs one any more.
            grid_pos: None,
            world_pos: Some(WorldPos { x_mm: draft.centre[0], y_mm: draft.centre[1] }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: draft.rack_orientation,
            station_id: None,
        });
        table.centres.insert(id, draft.centre);
        table.cell_sizes_mm.push(draft.size);
        table.cell_headings_rad.push(draft.heading_rad);
        table.cell_footprint_stated.push(draft.size_stated);
        table.cell_entities.insert(id, draft.entity.clone());
        table.cells_by_entity.insert(draft.entity.clone(), id);
        if draft.rank == 0 {
            table.way_cells.push(id);
        }
    }

    // Racking indices, now that the bays have identities. A bay's steel is
    // reported as a cell of its own, so it is not one of the document's cells
    // and does not belong in `cell_entities`' way half.
    for zone in &survey.zones {
        for bay in &zone.bays {
            if let Some(&cell) = table.cells_by_entity.get(&bay.rack) {
                table.rack_cells.insert(bay.rack.clone(), cell);
            }
        }
    }
    number_relays(&mut table, survey);

    compile_arcs(world, &mut table)?;
    Ok(table)
}

/// A cell's kind. The `charging` marker identifies charging infrastructure.
/// A tiled region with `flow` is a highway; everything else a floor robot drives
/// on is open floor until a station claims it.
fn way_kind(world: &World, cell: EntityId) -> CellKind {
    if world.entities.get(&cell).is_some_and(|e| e.has("charging")) {
        return CellKind::Charging;
    }
    // Same ruled shape again for two more statements about what the floor IS:
    // a cell an operator is presented at, and a cell where carriers change
    // hands with fixed conveyance (an EasyLoop's dropoff/collection cells). The
    // handoff cell has its kind back: `Exchange`, reintroduced for exactly
    // this (the v3 relay-dock kind, "a place an HTM occupies, not a shelf").
    // A relay's access cell is the other exchange cell; a relay binds it, so
    // `stamp_relay_access` labels it once the relays exist.
    if world.entities.get(&cell).is_some_and(|e| e.has("presentation_point")) {
        return CellKind::Presentation;
    }
    if world.entities.get(&cell).is_some_and(|e| e.has("port")) {
        return CellKind::Exchange;
    }
    // The `highway` marker is the same marker shape as `charging`: a statement
    // about what the floor IS, with no routing semantics of its own (unlike
    // `flow`, which directs arcs). A 7-lane highway is bidirectional until
    // someone rules lane directions.
    if world.entities.get(&cell).is_some_and(|e| e.has("highway")) {
        return CellKind::Highway;
    }
    // Derivedness lives on the identity itself -- a derived id IS its
    // address, and the generator is the address's root.
    let generator = match &cell {
        EntityId::Derived(address) => EntityId::from(address.root()),
        EntityId::Declared(_) => return CellKind::Open,
    };
    match world.entities.get(&generator) {
        Some(region) if region.has("flow") => CellKind::Highway,
        _ => CellKind::Open,
    }
}

/// Hand out relay identities in zone order, then along the run, then along the
/// bay's face — the same order desmap's compile-order counter produced, so a
/// reader comparing two runs sees relay 0 in the same place.
fn number_relays(table: &mut CellTable, survey: &Survey) {
    let mut next = 0u32;
    for zone in &survey.zones {
        for bay in &zone.bays {
            for stand in &bay.stands {
                let id = RelayId(next);
                next += 1;
                table.relays_by_stand.insert((bay.rack.clone(), stand.cell.clone()), id);
                table.relay_shelves.insert(id, stand.shelves.clone());
            }
        }
    }
}

/// Turn the world's arcs into dessim's edges. Flow arcs are directed one at a
/// time; stitch links are bidirectional.
fn compile_arcs(world: &World, table: &mut CellTable) -> Result<(), WorldMapError> {
    let mut arcs = Vec::with_capacity(world.edges.len());
    for edge in &world.edges {
        let (Some(&from), Some(&to)) = (table.cells_by_entity.get(&edge.from), table.cells_by_entity.get(&edge.to))
        else {
            continue;
        };
        let (Some(a), Some(b)) = (table.centres.get(&from), table.centres.get(&to)) else {
            continue;
        };
        let unit = world.frames.unit_mm().ok_or_else(|| WorldMapError::Unreadable {
            entity: edge.owner.clone(),
            message: "arc geometry needs primary-frame millimetres".into(),
        })?;
        let curve = edge.curve.as_ref().map(|arc| CircularArc {
            center: WorldPos { x_mm: arc.center[0].0 as f64 * unit, y_mm: arc.center[1].0 as f64 * unit },
            clockwise: arc.clockwise,
            start: arc.start.map(|p| WorldPos { x_mm: p[0].0 as f64 * unit, y_mm: p[1].0 as f64 * unit }),
            end: arc.end.map(|p| WorldPos { x_mm: p[0].0 as f64 * unit, y_mm: p[1].0 as f64 * unit }),
        });
        let from_pos = WorldPos { x_mm: a[0], y_mm: a[1] };
        let to_pos = WorldPos { x_mm: b[0], y_mm: b[1] };
        let distance_mm =
            curve.map_or_else(|| (a[0] - b[0]).hypot(a[1] - b[1]), |arc| arc.length_mm(from_pos, to_pos)).round();
        if !distance_mm.is_finite() || distance_mm <= 0.0 {
            return Err(WorldMapError::Unreadable {
                entity: edge.from.clone(),
                message: format!(
                    "the arc `{}` has invalid travel distance {distance_mm} mm; \
                     pathfinding requires a finite positive distance",
                    edge.key
                ),
            });
        }
        let traversability = traversability(world, edge.from.clone(), edge.to.clone());
        // A directly linked pick place is a destination, not a shortcut.
        // Gate only its entering edge; upstream waiting branches stay usable.
        let gate = |cell, entity: &EntityId| {
            (curve.is_some() && world.entities.get(entity).is_some_and(|entity| entity.has("pick")))
                .then_some(DestGate::Cell(cell))
        };
        let weight = Edge {
            traversability,
            distance_mm: distance_mm as u64,
            dest_only: gate(to, &edge.to),
            vtm_zone_gate: None,
            curve,
            htm_drive: Default::default(),
        };
        arcs.push((from, to, weight));
        if edge.bidirectional {
            arcs.push((
                to,
                from,
                Edge { curve: curve.map(CircularArc::reversed), dest_only: gate(from, &edge.from), ..weight },
            ));
        }
    }
    arcs.sort_by_key(|(from, to, _)| (from.0, to.0));
    for pair in arcs.windows(2) {
        if pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1 && pair[0].2 != pair[1].2 {
            return Err(WorldMapError::Unreadable {
                entity: table.cell_entities[&pair[0].0].clone(),
                message: "two links between the same cells disagree on geometry or traversal".into(),
            });
        }
    }
    arcs.dedup_by_key(|(from, to, _)| (from.0, to.0));
    table.nominal_step_mm = modal_arc_length_mm(&arcs);
    table.arcs = arcs;
    Ok(())
}

/// Who may cross an arc: the classes both of its cells admit. This is physics
/// read straight off `cell.traversable_by` (SPEC §7), not the cell-kind pair
/// table desmap needed because its cells carried no capability of their own.
fn traversability(world: &World, from: EntityId, to: EntityId) -> EdgeTraversability {
    let classes = |id: EntityId| -> Vec<String> {
        world
            .entities
            .get(&id)
            .and_then(|entity| entity.component::<WorldCell>("cell").ok().flatten())
            .map(|cell| cell.traversable_by)
            .unwrap_or_default()
    };
    let (a, b) = (classes(from), classes(to));
    let admits =
        |role: &str| (a.is_empty() || a.iter().any(|c| c == role)) && (b.is_empty() || b.iter().any(|c| c == role));
    EdgeTraversability {
        vtm_traversal: admits("vtm"),
        htm_traversal: if admits("htm") { HtmTraversal::Both } else { HtmTraversal::None },
        rtp_traversal: admits("rtp"),
    }
}

/// The arc length that turns up most often — the step this map is mostly made
/// of. Ties break toward the shorter step, which keeps a derived traversal time
/// from being generous about a distance most moves do not cover.
fn modal_arc_length_mm(arcs: &[(CellId, CellId, Edge)]) -> Option<f64> {
    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    for (_, _, edge) in arcs {
        *counts.entry(edge.distance_mm).or_default() += 1;
    }
    counts.into_iter().max_by_key(|&(distance, count)| (count, Reverse(distance))).map(|(distance, _)| distance as f64)
}

/// Distinct whole-millimetre columns and rows of the travelled floor, plus the
/// tightest gap between two neighbouring centres on either axis. The header
/// numbers `GridBounds` wants, computed without minting any ordinal.
fn floor_extent(centres: impl IntoIterator<Item = [f64; 2]>) -> (u32, u32, f64) {
    let mut xs: Vec<i64> = Vec::new();
    let mut ys: Vec<i64> = Vec::new();
    for c in centres {
        xs.push(c[0].round() as i64);
        ys.push(c[1].round() as i64);
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let tightest = |sorted: &[i64]| -> Option<f64> {
        sorted
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) as f64)
            .filter(|step| *step > 0.0)
            .fold(None, |acc: Option<f64>, step| Some(acc.map_or(step, |best| best.min(step))))
    };
    let step = tightest(&xs).into_iter().chain(tightest(&ys)).fold(f64::INFINITY, f64::min);
    // dessim's historical fallback spacing, for a one-cell world.
    let step = if step.is_finite() { step } else { 1000.0 };
    // Per-axis centre LISTS used to come back too, feeding the compat map's
    // GridAxes; both the axes and their consumers went with the v3 sunset
    // (the renderer frames each cell's own tile now), so only the header
    // numbers survive.
    (xs.len() as u32, ys.len() as u32, step)
}
