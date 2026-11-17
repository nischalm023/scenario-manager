//! Within `worldmap`, `census` is the ROLL of a loaded document: one row per
//! entity the world holds, declared and derived alike, saying what the document
//! makes that entity out of and where on the floor to point at it.
//!
//! It exists so a reader can study what a conversion produced -- which entities
//! somebody wrote down, which a generator derived from them, what each carries
//! -- against the map the simulator runs on, without deriving the document a
//! second time beside it. Nothing in the simulator reads this. It stands one
//! step further out than `WorldIndex`, which answers "which entity is this
//! `CellId`"; this answers "what is in this document at all".
//!
//! A row's place is a reading for a renderer, never a second geometry: every
//! rectangle here is one the survey or the region already produced, and every
//! cell is one the cell table already minted.

use std::sync::Arc;

use desgraph::CellId;
use rustc_hash::FxHashMap;
use site_model::components::product::{CellAddress, Queue};
use site_model::entity::Entity;
use site_model::ids::EntityId;
use site_model::pipeline::World;

use super::cells::CellTable;
use super::racking::Survey;
use super::stations::{Stations, posed_at};
use super::{StepMm, WorldMapError, region_rect};

/// One entity of the loaded world, as a reader meets it.
#[derive(Debug, Clone)]
pub struct EntityRow {
    /// The identity, which is also the provenance: a UUID somebody minted, or
    /// the address a generator produced the entity at (SPEC §3). Ask it
    /// `is_derived()` -- there is no second flag, because there is no second
    /// fact.
    pub id: EntityId,
    /// The label the document carries for people, when it carries one. Never
    /// identity, never unique (SPEC §2).
    pub name: Option<String>,
    /// The entity's component names, in the document's own order, because the
    /// set of components an entity carries IS its type (SPEC §2). The slice
    /// and every name in it are shared between rows carrying the same set: a
    /// site runs to a hundred thousand storage slots and thirty distinct sets.
    pub components: Arc<[Arc<str>]>,
    pub place: EntityPlace,
}

/// Where an entity is, in the terms a renderer already draws in.
#[derive(Debug, Clone)]
pub enum EntityPlace {
    /// The dessim cells this entity is or owns: a cell of the floor, the cell a
    /// relay's shelf is reached from, a station's presentation and queue.
    Cells(Vec<CellId>),
    /// An axis-aligned millimetre rectangle `[min_x, min_y, w, h]`: a tiled or
    /// untiled region, the building envelope — flat on the floor.
    Rect([f64; 4]),
    /// One bay's steel: its footprint rectangle and how high it stands (the
    /// zone's topmost shelf), so a 3D pick can meet the rack where it is
    /// rather than only at its footprint.
    Prism { rect: [f64; 4], top_mm: f64 },
    /// A millimetre position with no extent: a pose on the floor (z 0), a
    /// storage slot on its shelf, a handoff board at its height — so a 3D
    /// pick can meet shelved things where they are.
    Point([f64; 3]),
    /// The document puts this entity nowhere. Not a failure to resolve: a
    /// stitch, an access, a sector, a cohort, a robot, a template and a
    /// product in the catalog each state a relation or a kind, and neither
    /// stands anywhere. On the two site fixtures every placeless row is one of
    /// those.
    Nowhere,
}

/// Read every entity of the world into a row.
///
/// The order is `world.entities`' own: `EntityId` sorts declared before
/// derived and then by identity, so walking the map in key order already
/// hands a reader the grouping they want and nothing here re-sorts.
pub(super) fn build(
    world: &World,
    survey: &Survey,
    table: &CellTable,
    stations: &Stations,
    unit: StepMm,
) -> Result<Vec<EntityRow>, WorldMapError> {
    let surveyed = Surveyed::of(survey, stations);
    // One shared slice per distinct component set, and one shared string per
    // distinct name, so a hundred thousand slots that all read `slot` cost
    // two allocations between them.
    let mut names: FxHashMap<String, Arc<str>> = FxHashMap::default();
    let mut sets: FxHashMap<String, Arc<[Arc<str>]>> = FxHashMap::default();
    let mut rows = Vec::with_capacity(world.entities.len());
    for entity in world.entities.values() {
        let mut joined = String::new();
        for name in entity.components.keys() {
            if !joined.is_empty() {
                joined.push('+');
            }
            joined.push_str(name);
        }
        let components = match sets.get(&joined) {
            Some(shared) => Arc::clone(shared),
            None => {
                let shared: Arc<[Arc<str>]> = entity
                    .components
                    .keys()
                    .map(|name| match names.get(name.as_str()) {
                        Some(interned) => Arc::clone(interned),
                        None => {
                            let interned: Arc<str> = Arc::from(name.as_str());
                            names.insert(name.clone(), Arc::clone(&interned));
                            interned
                        }
                    })
                    .collect();
                sets.insert(joined, Arc::clone(&shared));
                shared
            }
        };
        rows.push(EntityRow {
            id: entity.id.clone(),
            name: entity.name.clone(),
            components,
            place: place_of(world, entity, table, &surveyed, unit)?,
        });
    }
    Ok(rows)
}

/// The geometry the loader has already produced, keyed by the entity it
/// belongs to. Reading it back is what keeps a row's place the same geometry
/// the simulator schedules against and the renderer draws.
struct Surveyed {
    /// One bay's steel rectangle and its zone's standing height, by the rack
    /// entity that is that bay.
    bays: FxHashMap<EntityId, ([f64; 4], f64)>,
    /// One storage position's millimetre centre and shelf height, by the
    /// slot entity.
    slots: FxHashMap<EntityId, [f64; 3]>,
    /// One handoff board's exchange position and height, by the protocol
    /// relay that shelf is. The board, not the stand cell: a shelf's place is
    /// up the rack where the shelf is.
    relay_boards: FxHashMap<EntityId, [f64; 3]>,
    /// The union footprint and tallest standing height of every bay a
    /// declaring root derived, by that root. A rack-run stamp has no
    /// geometry of its own, but it IS somewhere — everywhere its bays stand.
    derived_steel: FxHashMap<EntityId, ([f64; 4], f64)>,
    /// Every cell a station claims -- presentation, queue, standing area --
    /// by the station entity.
    station_cells: FxHashMap<EntityId, Vec<CellId>>,
}

impl Surveyed {
    fn of(survey: &Survey, stations: &Stations) -> Surveyed {
        let mut out = Surveyed {
            bays: FxHashMap::default(),
            slots: FxHashMap::default(),
            relay_boards: FxHashMap::default(),
            derived_steel: FxHashMap::default(),
            station_cells: FxHashMap::default(),
        };
        for zone in &survey.zones {
            for bay in &zone.bays {
                out.bays.insert(bay.rack.clone(), (bay.rect_mm, zone.height_mm));
                let root = EntityId::from(bay.rack.root());
                let union = out.derived_steel.entry(root).or_insert((bay.rect_mm, 0.0));
                let [ux, uy, uw, uh] = union.0;
                let [bx, by, bw, bh] = bay.rect_mm;
                let (nx, ny) = (ux.min(bx), uy.min(by));
                union.0 = [nx, ny, (ux + uw).max(bx + bw) - nx, (uy + uh).max(by + bh) - ny];
                union.1 = union.1.max(zone.height_mm);
                for slot in &bay.slots {
                    out.slots.insert(slot.entity.clone(), [slot.position[0], slot.position[1], slot.floor_m * 1000.0]);
                }
                for stand in &bay.stands {
                    // `shelves` runs bottom-up in step with `board_floors_m`,
                    // so each relay gets the height of its own board.
                    for (level, shelf) in stand.shelves.iter().enumerate() {
                        let floor_mm = stand.board_floors_m.get(level).copied().unwrap_or(0.0) * 1000.0;
                        out.relay_boards.insert(shelf.clone(), [stand.exchange[0], stand.exchange[1], floor_mm]);
                    }
                }
            }
        }
        for &(cell, _, id) in &stations.decorations {
            let Some(entity) = stations.by_id.get(&id) else { continue };
            out.station_cells.entry(entity.clone()).or_default().push(cell);
        }
        out
    }
}

/// Where to point at one entity. The order of the tests is the answer's
/// order of preference, not a search: an entity that is several of these at
/// once gets the reading with the most extent behind it.
fn place_of(
    world: &World,
    entity: &Entity,
    table: &CellTable,
    surveyed: &Surveyed,
    unit: StepMm,
) -> Result<EntityPlace, WorldMapError> {
    let id = &entity.id;
    // A bay ahead of a cell: the loader mints an EXTENTLESS cell for a bay's
    // steel (`cells.rs`, rank 1), so reading that cell's tile back would
    // outline nothing. The surveyed rectangle is where the steel stands, and
    // the zone's height is how far up it stands there.
    if let Some(&(rect, top_mm)) = surveyed.bays.get(id) {
        return Ok(EntityPlace::Prism { rect, top_mm });
    }
    // A pick happening on a cell does not turn that cell into the whole
    // station. Its own footprint must remain directly inspectable.
    if let Some(cells) = surveyed.station_cells.get(id)
        && !entity.has("cell")
    {
        return Ok(EntityPlace::Cells(cells.clone()));
    }
    // Storage positions ahead of cells only because there are so many of them
    // -- nine entities in ten on a real site is a slot -- and a slot is never
    // any of the things below. Answering them on a hash lookup keeps the roll
    // off the cell table's ordered search.
    if let Some(&at) = surveyed.slots.get(id) {
        return Ok(EntityPlace::Point(at));
    }
    if let Some(&cell) = table.cells_by_entity.get(id) {
        return Ok(EntityPlace::Cells(vec![cell]));
    }
    // A relay is a shelf partway up a bay, and its place is the board itself
    // at its height. The cell an HTM stands on to reach it is one hover away
    // (the inspector's relay selection still outlines it).
    if let Some(&board) = surveyed.relay_boards.get(id) {
        return Ok(EntityPlace::Point(board));
    }
    if let Some(rect) = region_rect(world, entity, unit)? {
        return Ok(EntityPlace::Rect(rect));
    }
    // A declaring root whose derivation put steel down wears that steel's
    // union footprint — a rack-run stamp answers with its whole run, not the
    // single pose point its own components state. Ahead of the pose test for
    // exactly that reason.
    if let Some(&(rect, top_mm)) = surveyed.derived_steel.get(id) {
        return Ok(EntityPlace::Prism { rect, top_mm });
    }
    if entity.has("queue") {
        let cells = queue_cells(entity, table);
        if !cells.is_empty() {
            return Ok(EntityPlace::Cells(cells));
        }
    }
    if let Some([x, y]) = posed_at(world, entity, unit) {
        return Ok(EntityPlace::Point([x, y, 0.0]));
    }
    Ok(EntityPlace::Nowhere)
}

/// The cells a `queue` names by reference, in path order.
///
/// No queue that reaches this states a step as a position: before the census
/// runs, the loader reads every queue but a tiling's as a lane, and a lane may
/// not state a position. A queue on a tiling is placed by its region above.
/// Nothing refuses here: a roll of a document must not reject a document the
/// simulator accepted, so an address this cannot follow leaves the queue with
/// no place rather than the load with an error.
fn queue_cells(entity: &Entity, table: &CellTable) -> Vec<CellId> {
    let Ok(Some(queue)) = entity.component::<Queue>("queue") else { return Vec::new() };
    queue
        .path
        .iter()
        .filter_map(|address| match address {
            CellAddress::Id(id) => table.cells_by_entity.get(id).copied(),
            CellAddress::Derived(raw) => {
                raw.parse().ok().and_then(|derived| table.cells_by_entity.get(&EntityId::from(derived)).copied())
            }
            CellAddress::Local(_) | CellAddress::Handle(_) | CellAddress::At(_) => None,
        })
        .collect()
}
