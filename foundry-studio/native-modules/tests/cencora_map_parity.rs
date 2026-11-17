//! Map parity for cencora: the same site read twice, and the two readings
//! compared cell by cell.
//!
//! Among dessim's integration tests, most take ONE loader's map and check what
//! the simulator does with it. This one takes NEITHER side's word: it compiles
//! `sim/desmap`'s v3 design of cencora and loads the world-protocol document
//! of the same site in one process, and holds the two resulting
//! `desmap::WarehouseMap`s against each other.
//!
//! It exists to gate the landing. `walmart-on-tmv2` replaces the v3 design
//! loader with the world-document loader for cencora, and the promise attached
//! to that swap is that master does not regress: the floor a robot drives, the
//! cells a station presents from, the shelves a VTM reaches must be the same
//! millimetres afterwards. Cell ids cannot carry that comparison -- the two
//! loaders mint their own, in their own order -- so every join here is
//! GEOMETRIC: a cell is identified by its centre rounded to the millimetre, an
//! exchange point by the row and aisle column and side it stands at, a zone by
//! the aisle column its VTM works, and an arc by the centres of its two cells.
//!
//! Every join is also ONE-TO-ONE, because a census is not a comparison: matched
//! things are paired individually and each pair's attributes are compared, so
//! two zones cannot trade their relay lists and two arcs cannot trade their
//! endpoints without the pairing itself failing. `Edge` is destructured field
//! by field here, so a new edge attribute breaks this file at compile time
//! rather than slipping past the arc comparison unread.
//!
//! What is asserted is what actually holds; everything else is printed. The two
//! sides disagree in places -- rack reporting anchors, cell labels, and which
//! robots an arc admits -- and the
//! disagreements are the point of the report: a harness that asserted them into
//! agreement would be hiding the landing's real cost rather than gating it. Each
//! is instead named, measured, and pinned to its exact size, so it cannot grow
//! or change shape unnoticed. Run with `--nocapture` to read it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use desgraph::{
    Cell, CellId, CellKind, DestGate, Edge, EdgeTraversability, HtmDriveDirection, HtmTraversal, StationId, SystemType,
    WarehouseGraphBuilder, WorldPos, ZoneId,
};
use desmap::{RelayPosition, WarehouseDesign, WarehouseMap, Zone};
use dessim::scenario_gen::topology::{Racking, racking_from_map};
use dessim::worldmap::{self, WorldMap};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;

/// 19 zones, each two runs of 6 bays facing one aisle -- the same census
/// `worldproto_cencora.rs` counts the document by.
const ZONES: usize = 19;
const BAYS: usize = ZONES * 2 * 6;

/// One grid column of the Cencora survey, in millimetres: the distance from
/// the VTM aisle to the relay stands beneath the boards.
const COLUMN_MM: i64 = 850;

/// The join key: a cell centre rounded to whole millimetres.
///
/// Both loaders compute centres in `f64` millimetres from the same survey, so
/// the rounding is a bucket and not a tolerance -- two centres that agree
/// physically land in the same bucket, and the matched set is re-checked
/// afterwards for exact `f64` equality so that nothing merely NEARLY agreeing
/// is counted as a match.
type Mm = (i64, i64);

fn key(pos: WorldPos) -> Mm {
    (pos.x_mm.round() as i64, pos.y_mm.round() as i64)
}

fn fmt_mm(k: Mm) -> String {
    format!("({}, {})", k.0, k.1)
}

// ---------------------------------------------------------------------------
// The two arms
// ---------------------------------------------------------------------------

/// The v3 design of cencora, compiled the way master's simulator compiles it,
/// with its cell centres carried into the world document's site frame.
///
/// The grid's millimetres are the document's floor frame: rows run down the
/// drawing. The document states how that floor sits in the site frame (a
/// mirrored y, since the site is drawn north-up), and the world loader places
/// everything there, so the legacy arm is projected through the same
/// declaration before the two are joined by centre. Only the centres move;
/// grid indices, kinds and arcs are the compiler's own.
///
/// Spelled out here rather than taken from `tests/common.rs`: this branch's
/// helper module has no legacy-design loader left, and restoring one belongs to
/// another task.
fn legacy() -> (WarehouseDesign, WarehouseMap) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sim/desmap/tests/fixtures/cencora_v3.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let design: WarehouseDesign =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {} as a v3 design: {e}", path.display()));
    let map = design.compile().unwrap_or_else(|e| panic!("compile {}: {e}", path.display()));
    (design, into_site_frame(map))
}

/// The floor frame's `p_site = translation + scale * p_floor`, read off the
/// document: the one frame whose parent is the primary frame.
fn floor_to_site() -> ([f64; 2], [f64; 2]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cencora_v3.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let doc: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    let primary = doc["primary_frame"].as_str().expect("the document names its primary frame");
    let floors: Vec<&Value> = doc["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter(|e| e["components"]["frame"]["parent"].as_str() == Some(primary))
        .collect();
    assert_eq!(floors.len(), 1, "exactly one frame sits directly in the site frame");
    let transform = &floors[0]["components"]["frame"]["transform_to_parent"];
    let axis = |key: &str, i: usize| transform[key][i].as_f64().unwrap_or_else(|| panic!("{key}[{i}] is a number"));
    ([axis("scale", 0), axis("scale", 1)], [axis("translation", 0), axis("translation", 1)])
}

/// The compiled legacy map with every cell centre moved from floor to site
/// millimetres. Cells are referenced by id everywhere else in the map, so the
/// graph is the only thing rebuilt.
fn into_site_frame(map: WarehouseMap) -> WarehouseMap {
    let (scale, translation) = floor_to_site();
    let project =
        |p: WorldPos| WorldPos { x_mm: translation[0] + scale[0] * p.x_mm, y_mm: translation[1] + scale[1] * p.y_mm };
    let mut builder = WarehouseGraphBuilder::with_capacity(map.graph.cell_count(), map.graph.edge_count());
    for cell in map.graph.cells() {
        let mut cell = cell.clone();
        cell.world_pos = cell.world_pos.map(project);
        builder.add_cell(cell);
    }
    for (from, to, edge) in map.graph.edges() {
        builder.add_edge(from, to, edge.clone());
    }
    WarehouseMap { graph: builder.build(), ..map }
}

/// The world-protocol document of the same site, loaded the way the branch
/// loads it.
fn world() -> WorldMap {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cencora_v3.json");
    worldmap::load(&path).unwrap_or_else(|e| panic!("load {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Classifying each side's cells before any comparison
// ---------------------------------------------------------------------------

/// Rack anchors have no footprint. All remaining cells come from the document's
/// floor; VTM positions do not mint any cells. Check both kind and provenance
/// so a missing floor footprint cannot silently disappear from the parity join.
fn world_minted(world: &WorldMap) -> FxHashSet<CellId> {
    assert_eq!(
        world.cell_sizes_mm.len(),
        world.map.graph.cell_count(),
        "the loader states a footprint for every cell it mints"
    );
    let footprint = |cell: &Cell| {
        *world
            .cell_sizes_mm
            .get(cell.id.0 as usize)
            .unwrap_or_else(|| panic!("cell {:?} has no stated footprint", cell.id))
    };
    let zero = |cell: &Cell| footprint(cell) == [0.0, 0.0];
    let minted_kind = |kind: CellKind| matches!(kind, CellKind::Relay);

    // ordered: printed in a failure message, and hash order would reshuffle it.
    let mut disagree: BTreeMap<String, usize> = BTreeMap::new();
    for cell in world.map.graph.cells() {
        if zero(cell) != minted_kind(cell.kind) {
            *disagree.entry(format!("{:?} at footprint {:?}", cell.kind, footprint(cell))).or_default() += 1;
        }
    }
    assert!(
        disagree.is_empty(),
        "zero footprint no longer means exactly `Relay` on this document, so it no longer \
         identifies the minted cells: {disagree:?}"
    );

    world.map.graph.cells().filter(|cell| zero(cell)).map(|cell| cell.id).collect()
}

/// One side's cells indexed by millimetre centre, plus the centres more than one
/// cell of that same side claims.
struct ByCentre<'a> {
    at: FxHashMap<Mm, &'a Cell>,
    /// Centre -> the cells stacked on it, only where there is more than one.
    // ordered: printed in a failure message, and hash order would reshuffle it.
    stacked: BTreeMap<Mm, Vec<CellId>>,
}

fn by_centre<'a>(cells: impl Iterator<Item = &'a Cell>) -> ByCentre<'a> {
    let mut at: FxHashMap<Mm, &Cell> = FxHashMap::default();
    let mut seen: FxHashMap<Mm, Vec<CellId>> = FxHashMap::default();
    for cell in cells {
        let pos = cell.world_pos.unwrap_or_else(|| panic!("cell {:?} has no centre to join on", cell.id));
        let k = key(pos);
        seen.entry(k).or_default().push(cell.id);
        at.entry(k).or_insert(cell);
    }
    let mut stacked = BTreeMap::new();
    for (k, mut ids) in seen {
        if ids.len() > 1 {
            ids.sort_by_key(|id| id.0);
            stacked.insert(k, ids);
        }
    }
    ByCentre { at, stacked }
}

fn kind_histogram<'a>(cells: impl Iterator<Item = &'a Cell>) -> BTreeMap<String, usize> {
    // ordered: printed as a table, so the kinds come out the same way each run.
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    for cell in cells {
        *hist.entry(format!("{:?}", cell.kind)).or_default() += 1;
    }
    hist
}

fn print_histogram(label: &str, hist: &BTreeMap<String, usize>) {
    let total: usize = hist.values().sum();
    println!("  {label} ({total}):");
    for (kind, count) in hist {
        println!("    {kind:<14} {count:>6}");
    }
}

fn sorted<T: Ord + Copy>(set: &FxHashSet<T>) -> Vec<T> {
    let mut v: Vec<T> = set.iter().copied().collect();
    v.sort_unstable();
    v
}

/// The millimetre centres an HTM can get to at all, by the same walk the v3
/// compiler uses to decide it (`desmap::design::reachability`): breadth first
/// from every relay standing cell and every station presentation cell, across
/// any arc the map states.
///
/// It exists to tell a traversability flag that matters from one that cannot:
/// an arc between two cells no HTM can ever stand on is dead floor whatever its
/// flags say, and the two loaders are allowed to label dead floor differently.
fn htm_reachable_centres(map: &WarehouseMap) -> FxHashSet<Mm> {
    let mut visited: FxHashSet<CellId> = FxHashSet::default();
    let mut queue: Vec<CellId> = Vec::new();
    for origin in
        map.relays.iter().map(|r| r.htm_access).chain(map.stations.iter().flat_map(|s| s.presentation_cells()))
    {
        if map.graph.cell(origin).is_some() && visited.insert(origin) {
            queue.push(origin);
        }
    }
    while let Some(current) = queue.pop() {
        for (next, _arc) in map.graph.neighbors(current) {
            if visited.insert(next) {
                queue.push(next);
            }
        }
    }
    visited.iter().map(|id| key(map.cell_world_pos(*id).expect("a reached cell has a centre"))).collect()
}

// ---------------------------------------------------------------------------
// Reading an exchange point, and a zone, without either loader's minted ids
// ---------------------------------------------------------------------------

/// One exchange point read as three millimetre centres: where the HTM stands,
/// the aisle cell the VTM works from, and the cell the shelf itself sits on.
///
/// `RelayId` is minted independently by the two loaders and in a different
/// order, so it cannot pair anything. These three centres can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RelayGeometry {
    standing: Mm,
    vtm: Mm,
    shelf: Mm,
}

impl RelayGeometry {
    /// Which side of the aisle column the HTM stands on: -1 west of it, +1 east.
    fn side(&self) -> i64 {
        (self.standing.0 - self.vtm.0).signum()
    }

    /// The x-separation, in millimetres, between the HTM's standing cell and
    /// the aisle cell the VTM works from. Columns between two cells -- NOT a
    /// path length, and nothing here measures what a robot drives.
    fn reach_mm(&self) -> i64 {
        (self.standing.0 - self.vtm.0).abs()
    }

    /// What pairs this exchange point with the other loader's: the row it
    /// exchanges on, the aisle column its VTM works, and the side of that column
    /// the HTM stands on. No minted id enters the key.
    fn pairing_key(&self) -> (i64, i64, i64) {
        (self.standing.1, self.vtm.0, self.side())
    }
}

fn relay_geometry(map: &WarehouseMap, relay: &RelayPosition) -> RelayGeometry {
    let at = |cell| key(map.cell_world_pos(cell).expect("a relay's cells have centres"));
    let g = RelayGeometry {
        standing: at(relay.htm_access),
        vtm: key(relay.vtm_access.world_pos(&map.graph).expect("VTM access coordinates")),
        shelf: at(relay.position),
    };
    assert_ne!(g.side(), 0, "an exchange point stands in its own aisle column at {}", fmt_mm(g.standing));
    g
}

/// One zone read the way the OTHER loader can recognise it.
///
/// `ZoneId` is minted per loader, so zones join on the aisle a zone's VTM works:
/// the column every one of its relays names as `vtm_access`. That column is one
/// number per zone, and no two zones share it -- both asserted here, because a
/// zone whose relays straddle two aisles would make the join meaningless rather
/// than merely wrong.
struct ZoneReading<'a> {
    zone: &'a Zone,
    relays: Vec<RelayGeometry>,
}

impl ZoneReading<'_> {
    /// The rows this zone exchanges on.
    fn rows(&self) -> BTreeSet<i64> {
        // ordered: compared and printed.
        self.relays.iter().map(|r| r.standing.1).collect()
    }

    /// The HTM standing cells on one side of the zone's aisle column.
    fn standing_on(&self, side: i64) -> BTreeSet<Mm> {
        // ordered: compared and printed.
        self.relays.iter().filter(|r| r.side() == side).map(|r| r.standing).collect()
    }
}

/// Every zone that owns exchange points, indexed by the aisle column its VTM
/// works from. Zones with no relays are left out and returned separately: the v3
/// compiler labels unzoned floor `ZoneId(0)` and that label owns no aisle.
fn zones_by_aisle(map: &WarehouseMap) -> (BTreeMap<i64, ZoneReading<'_>>, Vec<ZoneId>) {
    // ordered: zones are joined, compared and printed by aisle column.
    let mut out: BTreeMap<i64, ZoneReading<'_>> = BTreeMap::new();
    let mut relayless: Vec<ZoneId> = Vec::new();
    for zone in &map.zones {
        if zone.relays.is_empty() {
            relayless.push(zone.id);
            continue;
        }
        let relays: Vec<RelayGeometry> = zone
            .relays
            .iter()
            .map(|id| relay_geometry(map, map.relay(*id).expect("a zone's relays are in the map")))
            .collect();
        // ordered: named in a failure message.
        let columns: BTreeSet<i64> = relays.iter().map(|r| r.vtm.0).collect();
        assert_eq!(
            columns.len(),
            1,
            "zone {:?} owns relays working {} different aisle columns ({columns:?}), so it cannot be \
             joined to the other side by the aisle it serves",
            zone.id,
            columns.len()
        );
        let column = *columns.first().expect("exactly one column");
        assert!(
            out.insert(column, ZoneReading { zone, relays }).is_none(),
            "two zones work the aisle column at {column} mm, so the aisle no longer names one zone"
        );
    }
    (out, relayless)
}

// ---------------------------------------------------------------------------
// 1. Cell centres -- the four sets
// ---------------------------------------------------------------------------

#[test]
fn cell_centres() {
    let (design, legacy_map) = legacy();
    let world = world();
    let minted = world_minted(&world);

    println!("\n=== cencora map parity: cell centres ===");
    println!("  legacy compiled cells: {}", legacy_map.graph.cell_count());
    println!("  world loaded cells:    {}", world.map.graph.cell_count());

    // The world side's own two classes, checked before the join so the join
    // compares floor against floor.
    let minted_cells: Vec<&Cell> = world.map.graph.cells().filter(|c| minted.contains(&c.id)).collect();
    let floor_cells: Vec<&Cell> = world.map.graph.cells().filter(|c| !minted.contains(&c.id)).collect();
    println!("\n  -- the world loader's own two classes --");
    println!("  minted point cells (zero footprint): {}", minted_cells.len());
    println!("  tiled floor cells:                   {}", floor_cells.len());
    print_histogram("minted, by kind", &kind_histogram(minted_cells.iter().copied()));
    let minted_racks = minted_cells.iter().filter(|c| c.kind == CellKind::Relay).count();
    assert_eq!(minted_racks, BAYS, "one rack anchor per bay");
    assert_eq!(minted_cells.len(), BAYS, "VTM positions add no cells");
    assert_eq!(floor_cells.len(), 8089 + 3 * BAYS, "the document's derived floor");
    assert_eq!(world.index.cell_entities.len(), world.map.graph.cell_count(), "every cell retains entity provenance");
    let racks = by_centre(minted_cells.iter().copied());
    assert!(racks.stacked.is_empty(), "two bays' rack cells share a centre: {:?}", racks.stacked);
    assert_eq!(racks.at.len(), BAYS, "rack anchors are one per bay and distinct");

    let l = by_centre(legacy_map.graph.cells());
    let w = by_centre(floor_cells.iter().copied());
    assert!(l.stacked.is_empty(), "legacy stacks cells on one centre: {:?}", l.stacked);
    assert!(w.stacked.is_empty(), "the world floor stacks cells on one centre: {:?}", w.stacked);

    let lk: FxHashSet<Mm> = l.at.keys().copied().collect();
    let wk: FxHashSet<Mm> = w.at.keys().copied().collect();
    let matched = sorted(&lk.intersection(&wk).copied().collect());
    let only_legacy = sorted(&lk.difference(&wk).copied().collect());
    let only_world = sorted(&wk.difference(&lk).copied().collect());

    println!("\n  -- the four sets (legacy cells vs world FLOOR cells) --");
    println!("  matched centres:     {}", matched.len());
    println!("  only in legacy:      {}", only_legacy.len());
    println!("  only in world:       {}", only_world.len());
    println!("  held out as minted:  {}", minted_cells.len());

    // The join buckets to the millimetre; this measures what the two loaders
    // actually computed. `worst` is the largest disagreement in either axis and
    // `identical` counts the centres whose `f64` coordinates are equal bit for
    // bit -- a printed "0.0000 mm" would say neither, since anything under
    // 0.00005 mm prints that way.
    let mut worst = 0.0f64;
    let mut identical = 0usize;
    for k in &matched {
        let a = l.at[k].world_pos.unwrap();
        let b = w.at[k].world_pos.unwrap();
        if a.x_mm == b.x_mm && a.y_mm == b.y_mm {
            identical += 1;
        }
        worst = worst.max((a.x_mm - b.x_mm).abs()).max((a.y_mm - b.y_mm).abs());
    }
    println!("  matched centres equal as f64, bit for bit: {identical} of {}", matched.len());
    if identical == matched.len() {
        println!("  worst matched offset: bit-identical (exactly 0, not a rounded print)");
    } else {
        println!("  worst matched offset: {worst:.9} mm ({worst:e} mm)");
    }
    assert!(worst < 0.5, "matched centres disagree by {worst:.9} mm, more than half a millimetre");

    println!("\n  -- residue by cell kind --");
    print_histogram("only in legacy", &kind_histogram(only_legacy.iter().map(|k| l.at[k])));
    print_histogram("only in world", &kind_histogram(only_world.iter().map(|k| w.at[k])));

    // Every world floor cell is a legacy cell: the world side tiles a subset of
    // the same grid, so the only-in-world set is empty and the whole residue is
    // legacy's. That is the gate -- the world loader may not invent floor.
    assert!(only_world.is_empty(), "the world floor has {} cells legacy has nowhere: {:?}", only_world.len(), {
        let mut sample: Vec<String> = only_world.iter().take(12).map(|k| fmt_mm(*k)).collect();
        sample.push("...".into());
        sample
    });
    assert_eq!(matched.len(), floor_cells.len(), "every world floor cell matches a legacy cell");

    // The legacy residue, explained by class. The v3 compiler emits the WHOLE
    // 153 x 96 grid including the cells its clip boxes blanked; the document
    // tiles only the floor that exists.
    // Every legacy-only cell belongs to one of two named classes, and the
    // gate is that the residue is exactly those two at exactly those sizes --
    // a third class, or one of these growing, means the world loader has
    // stopped tiling floor the legacy grid still has.
    //
    //   Blank  the grid positions v3's clip boxes carve out. The compiler keeps
    //          all 153 x 96 and marks them untraversable; the document simply
    //          does not tile them.
    //   Aisle  the VTM corridor beside each relay column. Legacy tiles it as
    //          floor; the world represents VTM access without extra floor cells.
    let residue = kind_histogram(only_legacy.iter().map(|k| l.at[k]));
    println!("\n  legacy grid positions: {} x {} = {}", design.grid.width, design.grid.depth, 153 * 96);
    assert_eq!(legacy_map.graph.cell_count(), 153 * 96, "the v3 compiler keeps every grid position");
    assert_eq!(matched.len() + only_legacy.len(), legacy_map.graph.cell_count());
    assert_eq!(
        residue,
        BTreeMap::from([("Blank".into(), 5573), ("Aisle".into(), ZONES * 18)]),
        "the legacy-only residue is no longer the two explained classes"
    );
}

// ---------------------------------------------------------------------------
// 2. Attributes on the matched cells
// ---------------------------------------------------------------------------

#[test]
fn matched_cell_attributes() {
    let (design, legacy_map) = legacy();
    let world = world();
    let minted = world_minted(&world);

    let l = by_centre(legacy_map.graph.cells());
    let w = by_centre(world.map.graph.cells().filter(|c| !minted.contains(&c.id)));
    let matched = sorted(
        &l.at
            .keys()
            .copied()
            .collect::<FxHashSet<Mm>>()
            .intersection(&w.at.keys().copied().collect())
            .copied()
            .collect(),
    );

    println!("\n=== cencora map parity: attributes on {} matched cells ===", matched.len());

    println!("\n  -- kind: legacy x world --");
    // Both loaders write the same `desgraph::CellKind`, so this is not a
    // translation table but the observed correspondence. `Exchange` exists only
    // on the world side (no v3 map mints one): a legacy relay stand is the
    // shelf's own `Relay` cell, so the stands read legacy Relay x world Exchange.
    // ordered: printed.
    let mut cross: BTreeMap<(String, String), usize> = BTreeMap::new();
    for k in &matched {
        *cross.entry((format!("{:?}", l.at[k].kind), format!("{:?}", w.at[k].kind))).or_default() += 1;
    }
    let mut same = 0usize;
    for ((lkind, wkind), count) in &cross {
        let mark = if lkind == wkind { "=" } else { "x" };
        println!("    {mark} legacy {lkind:<14} world {wkind:<14} {count:>6}");
        if lkind == wkind {
            same += count;
        }
    }
    println!("  cells whose kind agrees: {same} of {}", matched.len());

    println!("\n  -- station ownership --");
    // Which cell each side's stations stand on, and WHICH station stands there.
    // The seat exception below names one specific station, so a cell stamped
    // with some other station's id is a disagreement and not an exception.
    let mut seat_owner: FxHashMap<Mm, StationId> = FxHashMap::default();
    for station in &world.map.stations {
        let seat = key(world.map.cell_world_pos(station.position).expect("a station cell has a centre"));
        assert!(
            seat_owner.insert(seat, station.id).is_none(),
            "two world stations are seated on {}, so the seat does not name one station",
            fmt_mm(seat)
        );
    }
    let mut differ: Vec<Mm> = Vec::new();
    let mut seat_only: usize = 0;
    for k in &matched {
        if l.at[k].station_id == w.at[k].station_id {
            continue;
        }
        // The one explained class: the world stamps the station's own seat with
        // THAT station's id; the v3 compiler stamps only the presentation cell
        // and the queue, leaving the seat unowned. Same 26 cells, one more
        // label -- and the label has to be the id of the station whose
        // `position` is this very cell.
        let seated = seat_owner.get(k).copied();
        if l.at[k].station_id.is_none() && seated.is_some() && w.at[k].station_id == seated {
            seat_only += 1;
            continue;
        }
        differ.push(*k);
    }
    println!("  station_id agrees on {} cells", matched.len() - differ.len() - seat_only);
    println!("  the world additionally names the station on its own seat: {seat_only} cells");
    println!("  unexplained disagreements: {}", differ.len());
    for k in differ.iter().take(12) {
        println!(
            "    {} legacy {:?} world {:?} (the station seated here: {:?})",
            fmt_mm(*k),
            l.at[k].station_id,
            w.at[k].station_id,
            seat_owner.get(k)
        );
    }
    assert_eq!(seat_only, world.map.stations.len(), "the seat class is one cell per station");
    assert!(differ.is_empty(), "{} matched cells are owned by different stations", differ.len());

    // World relay identity belongs to the shelf/anchor, while its physical HTM
    // stand is an Exchange cell. Permit this label difference only at the
    // exact access positions, which must coincide between the two maps.
    let relay_stands: FxHashSet<Mm> = world
        .map
        .relays
        .iter()
        .map(|r| key(world.map.cell_world_pos(r.htm_access).expect("relay access has a centre")))
        .collect();
    let kind_differences: FxHashSet<Mm> = matched.iter().copied().filter(|k| l.at[k].kind != w.at[k].kind).collect();
    assert_eq!(kind_differences, relay_stands);
    assert_eq!(same + 684, matched.len());
    assert!(kind_differences.iter().all(|k| { l.at[k].kind == CellKind::Relay && w.at[k].kind == CellKind::Exchange }));

    println!("\n  -- zone --");
    // Legacy labels the relay stands with rack zones; world labels the shelf
    // entities. Compare ownership through the aisle/relay membership below.
    // ordered: printed.
    let mut labels: BTreeMap<(ZoneId, ZoneId), usize> = BTreeMap::new();
    for k in &matched {
        *labels.entry((l.at[k].zone, w.at[k].zone)).or_default() += 1;
    }
    println!("  (legacy zone, world zone) over the matched cells: {labels:?}");
    println!("  zones in the compiled map: legacy {}   world {}", legacy_map.zones.len(), world.map.zones.len());

    // Zones join ONE-TO-ONE on the aisle they serve: the column their VTM works
    // from. Everything below is then compared inside a single pair of zones, so
    // two zones that traded their `relays` lists put the wrong rows and the
    // wrong standing cells under one aisle column and fail here -- which a
    // global census of columns and rows over the whole map cannot see.
    let (lz, l_relayless) = zones_by_aisle(&legacy_map);
    let (wz, w_relayless) = zones_by_aisle(&world.map);
    println!("  zones owning exchange points: legacy {}   world {}", lz.len(), wz.len());
    println!("  zones owning none: legacy {l_relayless:?}   world {w_relayless:?}");
    println!("  legacy carries one extra: ZoneId(0), the label its compiler gives unzoned floor.");
    assert_eq!(lz.len(), ZONES, "19 aisles on the legacy side");
    assert_eq!(wz.len(), ZONES, "19 aisles on the world side");
    assert_eq!(
        lz.keys().collect::<Vec<_>>(),
        wz.keys().collect::<Vec<_>>(),
        "the two sides put their aisles in different columns"
    );

    println!("\n  per zone, joined on the aisle column its VTM works:");
    println!(
        "    {:>9} {:>7} {:>5} {:>5} {:>6} {:>6} {:>10} {:>9} {:>9} {:>9} {:>9}",
        "aisle mm",
        "relays",
        "rows",
        "west",
        "east",
        "vtms",
        "width mm",
        "l shelves",
        "w shelves",
        "l height",
        "w height"
    );
    let mut faults: Vec<String> = Vec::new();
    for (column, lr) in &lz {
        let wr = &wz[column];
        println!(
            "    {:>9} {:>7} {:>5} {:>5} {:>6} {:>6} {:>10} {:>9} {:>9} {:>9} {:>9}",
            column,
            format!("{}/{}", lr.relays.len(), wr.relays.len()),
            format!("{}/{}", lr.rows().len(), wr.rows().len()),
            format!("{}/{}", lr.standing_on(-1).len(), wr.standing_on(-1).len()),
            format!("{}/{}", lr.standing_on(1).len(), wr.standing_on(1).len()),
            format!("{}/{}", lr.zone.vtm_count, wr.zone.vtm_count),
            format!("{:?}/{:?}", lr.zone.zone_width_mm, wr.zone.zone_width_mm),
            lr.zone.real_shelf_count,
            wr.zone.real_shelf_count,
            lr.zone.zone_height_mm,
            wr.zone.zone_height_mm,
        );
        let mut fault = |what: &str| faults.push(format!("aisle {column} mm: {what}"));
        if lr.relays.len() != wr.relays.len() {
            fault(&format!("{} legacy relays against {} world relays", lr.relays.len(), wr.relays.len()));
        }
        if lr.rows() != wr.rows() {
            fault(&format!("exchange rows differ: legacy {:?} world {:?}", lr.rows(), wr.rows()));
        }
        // Compare the physical stands per aisle side, without coordinate shifts.
        for side in [-1i64, 1] {
            let expected = lr.standing_on(side);
            let stated = wr.standing_on(side);
            if expected != stated {
                fault(&format!(
                    "side {side}: {} legacy standing cells do not match the {} world \
                     ones (first difference {:?})",
                    expected.len(),
                    stated.len(),
                    expected.symmetric_difference(&stated).next()
                ));
            }
        }
        if lr.zone.vtm_count != wr.zone.vtm_count {
            fault(&format!("vtm_count legacy {} world {}", lr.zone.vtm_count, wr.zone.vtm_count));
        }
        if lr.zone.zone_width_mm != wr.zone.zone_width_mm {
            fault(&format!("zone_width_mm legacy {:?} world {:?}", lr.zone.zone_width_mm, wr.zone.zone_width_mm));
        }
        if lr.zone.holds != wr.zone.holds {
            fault(&format!("holds legacy {:?} world {:?}", lr.zone.holds, wr.zone.holds));
        }
    }
    for f in faults.iter().take(20) {
        println!("    FAULT {f}");
    }
    assert!(
        faults.is_empty(),
        "{} zones disagree once joined on their aisle: {:?}",
        faults.len(),
        &faults[..faults.len().min(20)]
    );

    // Deliberate, recorded per zone above and never asserted: the document
    // states eleven measured shelves and the topmost as the zone height, and
    // the v3 compiler counts thirteen nominal shelves and sums them.
    println!(
        "  real_shelf_count / zone_height_mm are DELIBERATELY different (13 / 10017 legacy, 11 / 7577 world) \
         and are printed, not asserted."
    );
    // What each side additionally states about a zone, printed because it is
    // not a like-for-like comparison: names are minted per loader, and a zone's
    // `cells` list is the residue on one side and minted points on the other.
    let extras = |zs: &BTreeMap<i64, ZoneReading<'_>>| -> Vec<String> {
        let named = zs.values().filter(|r| r.zone.name.is_some()).count();
        let configured: BTreeSet<String> = zs.values().filter_map(|r| r.zone.vtm_configuration.clone()).collect();
        let cells: usize = zs.values().map(|r| r.zone.cells.len()).sum();
        let slots: usize = zs.values().map(|r| r.zone.slots.as_ref().map_or(0, |s| s.len())).sum();
        vec![
            format!("named {named}/{}", zs.len()),
            format!("vtm_configuration {configured:?}"),
            format!("zone cells {cells}"),
            format!("zone slots {slots}"),
        ]
    };
    println!("  legacy zones also state: {:?}", extras(&lz));
    println!("  world zones also state:  {:?}", extras(&wz));

    let centres = design.grid.row_centers_mm.as_ref().expect("cencora states row centres");
    println!("\n  -- reading [col,row] off any millimetre above --");
    println!(
        "  columns: {} at a uniform {} mm pitch (centre = 425 + 850*col)",
        design.grid.width, design.grid.cell_size_mm
    );
    println!("  rows: {} surveyed centres, first five {:?}", centres.len(), &centres[..5.min(centres.len())]);
}

// ---------------------------------------------------------------------------
// 3. Station anchors
// ---------------------------------------------------------------------------

/// Where each station stands, presents from, and is queued into -- in
/// millimetres, joined by the station's own position and never by `StationId`.
/// The v3 fixture writes stations as `[col,row]` pairs, so a systematic
/// half-cell offset between the loaders would surface here first.
#[test]
fn station_anchors() {
    let (_design, legacy_map) = legacy();
    let world = world();

    // Each station's lanes: a presentation centre and its queue's centres.
    let anchors = |map: &WarehouseMap| -> BTreeMap<Mm, Vec<(Mm, Vec<Mm>)>> {
        // ordered: joined and printed by position centre.
        let mut out = BTreeMap::new();
        for station in &map.stations {
            let at = |cell| key(map.cell_world_pos(cell).expect("a station's cells have centres"));
            let points = station
                .presentation_points
                .iter()
                .map(|point| (at(point.presentation_cell), point.queue_path.iter().map(|c| at(*c)).collect()))
                .collect();
            out.insert(at(station.position), points);
        }
        out
    };

    let l = anchors(&legacy_map);
    let w = anchors(&world.map);

    println!("\n=== cencora map parity: station anchors ===");
    println!("  legacy stations: {}   world stations: {}", legacy_map.stations.len(), world.map.stations.len());
    assert_eq!(l.len(), legacy_map.stations.len(), "two legacy stations share a position centre");
    assert_eq!(w.len(), world.map.stations.len(), "two world stations share a position centre");

    let lk: FxHashSet<Mm> = l.keys().copied().collect();
    let wk: FxHashSet<Mm> = w.keys().copied().collect();
    let only_legacy = sorted(&lk.difference(&wk).copied().collect());
    let only_world = sorted(&wk.difference(&lk).copied().collect());
    println!("  station positions only in legacy: {:?}", only_legacy.iter().map(|k| fmt_mm(*k)).collect::<Vec<_>>());
    println!("  station positions only in world:  {:?}", only_world.iter().map(|k| fmt_mm(*k)).collect::<Vec<_>>());

    let mut bad = Vec::new();
    for (pos, legacy_lanes) in &l {
        let Some(world_lanes) = w.get(pos) else { continue };
        if legacy_lanes != world_lanes {
            bad.push(*pos);
            println!("  station at {}:", fmt_mm(*pos));
            for (side, lanes) in [("legacy", legacy_lanes), ("world ", world_lanes)] {
                for (presentation, queue) in lanes {
                    let queue: Vec<_> = queue.iter().map(|k| fmt_mm(*k)).collect();
                    println!("    {side} presentation {} queue {queue:?}", fmt_mm(*presentation));
                }
            }
        }
    }
    let queue_cells: usize = l.values().flatten().map(|(_, q)| q.len()).sum();
    println!(
        "  matched by position with identical presentation cell and queue path: {}",
        l.len() - bad.len() - only_legacy.len()
    );
    println!("  queue cells compared: {queue_cells}");

    assert!(only_legacy.is_empty() && only_world.is_empty(), "station positions do not correspond");
    assert!(bad.is_empty(), "{} stations differ in presentation cell or queue path", bad.len());
}

// ---------------------------------------------------------------------------
// 4. Relay shelves, and where an HTM stands to reach one
// ---------------------------------------------------------------------------

/// Require all 684 physical HTM access positions to match legacy Cencora.
///
/// Each exchange point is paired with its counterpart by (the row it exchanges
/// on, the aisle column its VTM works, the side of that column the HTM stands
/// on), the pairing is asserted one-to-one across all 684, and every claim
/// below is then made about a PAIR. Comparing sorted per-row distributions
/// instead would pass while `htm_access` was rotated among the relays of a row,
/// which is a different set of shelves reached from a different set of cells.
///
/// The 850 mm figures below are X-SEPARATIONS between two cell
/// centres -- the columns between the HTM's standing cell and the aisle cell
/// the VTM works from -- and not path lengths. Nothing here measures what a
/// robot drives; what a robot drives is the arc set, in `arcs`.
#[test]
fn relay_access_cells() {
    let (_design, legacy_map) = legacy();
    let world = world();

    println!("\n=== cencora map parity: relays ===");
    println!("  relay positions: legacy {}   world {}", legacy_map.relays.len(), world.map.relays.len());
    assert_eq!(legacy_map.relays.len(), world.map.relays.len(), "the two sides count different exchange points");
    assert_eq!(world.map.relays.len(), 684, "684 exchange points");

    let read = |map: &WarehouseMap| -> BTreeMap<(i64, i64, i64), RelayGeometry> {
        // ordered: paired, walked and printed by row, then aisle column, then side.
        let mut out: BTreeMap<(i64, i64, i64), RelayGeometry> = BTreeMap::new();
        for relay in &map.relays {
            let g = relay_geometry(map, relay);
            assert!(
                out.insert(g.pairing_key(), g).is_none(),
                "two exchange points share the row/aisle/side key {:?}, so they cannot be paired individually",
                g.pairing_key()
            );
        }
        out
    };
    let lr = read(&legacy_map);
    let wr = read(&world.map);
    println!("  exchange points with a distinct (row, aisle column, side): legacy {}   world {}", lr.len(), wr.len());
    assert_eq!(lr.len(), 684, "the legacy exchange points do not have 684 distinct row/aisle/side keys");
    assert_eq!(wr.len(), 684, "the world exchange points do not have 684 distinct row/aisle/side keys");
    assert_eq!(
        lr.keys().collect::<Vec<_>>(),
        wr.keys().collect::<Vec<_>>(),
        "the two sides' exchange points do not pair one-to-one on (row, aisle column, side)"
    );

    let ls: FxHashSet<Mm> = lr.values().map(|g| g.standing).collect();
    let ws: FxHashSet<Mm> = wr.values().map(|g| g.standing).collect();
    println!("  distinct HTM standing cells: legacy {}   world {}", ls.len(), ws.len());
    assert_eq!(ls.len(), 684, "one standing cell per legacy exchange point");
    assert_eq!(ws.len(), 684, "one standing cell per world exchange point");
    println!("  standing cells at the SAME millimetre on both sides: {}", ls.intersection(&ws).count());

    // Every claim below is about a PAIR of exchange points, not about two
    // distributions that happen to have the same shape.
    // ordered: printed.
    let mut offsets: BTreeMap<i64, usize> = BTreeMap::new();
    // ordered: printed.
    let mut lreach: BTreeMap<i64, usize> = BTreeMap::new();
    let mut wreach: BTreeMap<i64, usize> = BTreeMap::new();
    let mut shelf_reach: BTreeMap<i64, usize> = BTreeMap::new();
    // ordered: printed and pinned.
    let mut shelf_row_gap: BTreeMap<i64, usize> = BTreeMap::new();
    let mut faults: Vec<String> = Vec::new();
    for (k, l) in &lr {
        let w = &wr[k];
        let side = k.2;
        *offsets.entry(w.standing.0 - l.standing.0).or_default() += 1;
        *lreach.entry(l.reach_mm()).or_default() += 1;
        *wreach.entry(w.reach_mm()).or_default() += 1;
        *shelf_reach.entry(w.shelf.0 - w.vtm.0).or_default() += 1;
        *shelf_row_gap.entry(w.shelf.1 - w.standing.1).or_default() += 1;
        let mut fault = |what: String| faults.push(format!("relay at row {} aisle {} side {side}: {what}", k.0, k.1));

        // The row, to the millimetre. It is part of the key, so this restates
        // the pairing on the world side rather than assuming it.
        if l.standing.1 != w.standing.1 || l.standing.1 != k.0 {
            fault(format!("rows {} legacy, {} world", l.standing.1, w.standing.1));
        }
        if w.standing != l.standing {
            fault(format!("standing cells differ: legacy {:?}, world {:?}", l.standing, w.standing));
        }
        if w.side() != l.side() || (w.shelf.0 - w.vtm.0).signum() != side {
            fault("the stand or shelf is on the wrong side of the aisle".into());
        }
        if w.standing.0 != w.shelf.0 {
            fault("the HTM stand is not beneath the shelf column".into());
        }
    }
    for f in faults.iter().take(20) {
        println!("    FAULT {f}");
    }
    assert!(
        faults.is_empty(),
        "{} of the 684 paired exchange points disagree: {:?}",
        faults.len(),
        &faults[..faults.len().min(20)]
    );

    println!("  column offset world - legacy, over all 684 paired (mm -> how many):");
    for (delta, count) in &offsets {
        println!("    {delta:>6} mm  {count:>4}");
    }
    println!("  x-separation from the HTM's standing cell to the VTM's aisle cell (mm -> how many):");
    println!("    legacy {lreach:?}");
    println!("    world  {wreach:?}");
    println!("  x-separation from the world shelf point to that aisle cell (signed mm -> how many): {shelf_reach:?}");
    println!(
        "  row gap from the world shelf point to its own standing cell (signed mm -> how many): {shelf_row_gap:?}"
    );
    // Each world bay mints ONE rack point and three stands face it, 645 mm
    // apart along the run, so a stand sits at one of exactly three row gaps from
    // its own bay's point and each gap is taken by exactly one stand per bay.
    // That ties a standing cell to the bay it serves: moving `htm_access` from
    // one bay's stand to another's lands it at a gap no bay uses and fails here.
    // It does NOT distinguish the three stands of ONE bay from each other --
    // they share the bay's only minted point, and what separates them is which
    // shelves they reach, which is not map geometry and is not compared here.
    // The three gaps are stated along the run in floor millimetres; the floor's
    // frame carries their sign into the site, where the comparison happens.
    let (scale, _) = floor_to_site();
    let in_site = |gap_mm: i64| (scale[1] * gap_mm as f64).round() as i64;
    assert_eq!(
        shelf_row_gap,
        BTreeMap::from([(in_site(-543), BAYS), (in_site(102), BAYS), (in_site(747), BAYS)]),
        "a world standing cell no longer sits at one of the three stand offsets from its own bay's rack point"
    );

    assert_eq!(lreach, BTreeMap::from([(COLUMN_MM, 684)]));
    assert_eq!(wreach, lreach, "HTM stands must be under the same shelf column as legacy");
    assert_eq!(offsets, BTreeMap::from([(0, 684)]));
    assert_eq!(ls, ws, "all 684 physical access positions must match");

    // The shelves themselves. Legacy makes the relay cell and the standing cell
    // one and the same; the world puts the shelf on its own minted point.
    let shelf = |map: &WarehouseMap| -> FxHashSet<Mm> {
        map.relays.iter().map(|r| key(map.cell_world_pos(r.position).expect("a relay cell has a centre"))).collect()
    };
    let lshelf = shelf(&legacy_map);
    let wshelf = shelf(&world.map);
    println!("  distinct relay CELLS: legacy {}   world {}", lshelf.len(), wshelf.len());
    println!("  relay cells at the same millimetre on both sides: {}", lshelf.intersection(&wshelf).count());
    println!(
        "  legacy relay cell == its own standing cell: {}",
        legacy_map.relays.iter().all(|r| r.position == r.htm_access)
    );
    println!(
        "  world relay cell == its own standing cell: {}",
        world.map.relays.iter().all(|r| r.position == r.htm_access)
    );
}

// ---------------------------------------------------------------------------
// 5. Storage positions and shelf heights
// ---------------------------------------------------------------------------

/// The storage the two sides offer a VTM: one entry per slot, with its height.
///
/// The design memo expected this to be the harness's one deliberate divergence
/// (a surveyed eleven-shelf list against a grid-derived thirteen). The slots
/// themselves turn out to agree; what diverges is the ZONE-level shelf
/// summary, and that is recorded below rather than asserted either way.
#[test]
fn storage_positions_and_shelf_heights() {
    let (_design, legacy_map) = legacy();
    let world = world();

    let legacy_racking: Racking = racking_from_map(&legacy_map, world.racking.vertical_tote_capacity)
        .expect("rediscover the v3 design's racking from its grid");

    let slots = |racking: &Racking| -> Vec<(i64, i64, i64)> {
        let mut v: Vec<(i64, i64, i64)> = racking
            .columns
            .iter()
            .map(|c| {
                let pos = c.world_pos.expect("a storage position has a centre");
                (pos.x_mm.round() as i64, pos.y_mm.round() as i64, (c.floor_m.unwrap_or(0.0) * 1000.0).round() as i64)
            })
            .collect();
        v.sort_unstable();
        v
    };

    println!("\n=== cencora map parity: storage positions and shelf heights ===");
    println!("  storage positions: legacy {}   world {}", legacy_racking.columns.len(), world.racking.columns.len());
    let ls = slots(&legacy_racking);
    let ws = slots(&world.racking);
    assert_eq!(ls.len(), ws.len(), "the two sides offer a different number of storage positions");
    let lset: FxHashSet<(i64, i64, i64)> = ls.iter().copied().collect();
    let wset: FxHashSet<(i64, i64, i64)> = ws.iter().copied().collect();
    println!("  distinct (x mm, y mm, floor mm) triples: legacy {}   world {}", lset.len(), wset.len());
    println!("  triples identical to the millimetre on both sides: {}", lset.intersection(&wset).count());

    // Where they are not identical, by how much. Column and height agree
    // exactly, so slots pair off inside a (column, height) group and the only
    // question left is the row.
    // ordered: grouped and printed by column and height.
    let mut group_l: BTreeMap<(i64, i64), Vec<i64>> = BTreeMap::new();
    let mut group_w: BTreeMap<(i64, i64), Vec<i64>> = BTreeMap::new();
    for (x, y, f) in &ls {
        group_l.entry((*x, *f)).or_default().push(*y);
    }
    for (x, y, f) in &ws {
        group_w.entry((*x, *f)).or_default().push(*y);
    }
    for ys in group_l.values_mut().chain(group_w.values_mut()) {
        ys.sort_unstable();
    }
    println!("  (column, height) groups: legacy {}   world {}", group_l.len(), group_w.len());
    assert_eq!(
        group_l.keys().collect::<Vec<_>>(),
        group_w.keys().collect::<Vec<_>>(),
        "the two sides put storage in different columns or at different heights"
    );
    // ordered: printed.
    let mut row_offsets: BTreeMap<i64, usize> = BTreeMap::new();
    for (g, lys) in &group_l {
        let wys = &group_w[g];
        assert_eq!(
            lys.len(),
            wys.len(),
            "column {} at height {} holds {} legacy and {} world slots",
            g.0,
            g.1,
            lys.len(),
            wys.len()
        );
        for (ly, wy) in lys.iter().zip(wys) {
            *row_offsets.entry(wy - ly).or_default() += 1;
        }
    }
    println!("  row offset world - legacy over all {} slots (mm -> how many):", ls.len());
    for (delta, count) in &row_offsets {
        println!("    {delta:>6} mm  {count:>5}");
    }
    let worst_row = row_offsets.keys().map(|d| d.abs()).max().unwrap_or(0);
    println!("  worst row offset: {worst_row} mm");
    println!("  Not a rounding: the offset climbs monotonically along each run,");
    println!("  from -3 mm at the first slot to +207 mm at the twenty-fourth. The");
    println!("  v3 compiler anchors each slot on a grid row centre; the document");
    println!("  states the surveyed position, and the two pitches diverge over the");
    println!("  6-bay run. What agrees exactly is the column and the height.");
    // The gate: the same count of slots, in the same columns, at the same
    // heights, and no further along the run than the drift measured today. 207
    // mm is a PIN on what the two loaders currently disagree by, not a
    // tolerance anybody chose -- a larger drift is a new disagreement and fails.
    assert!(worst_row <= 207, "storage rows moved by {worst_row} mm, past the 207 mm drift this pins");

    let heights = |racking: &Racking| -> Vec<f64> {
        let mut v: Vec<f64> = racking.columns.iter().filter_map(|c| c.floor_m).collect();
        v.sort_by(f64::total_cmp);
        v.dedup();
        v
    };
    println!("  distinct floor heights: legacy {:?}", heights(&legacy_racking));
    println!("  distinct floor heights: world  {:?}", heights(&world.racking));
    println!(
        "  vertical tote capacity: legacy {}  world {}",
        legacy_racking.vertical_tote_capacity, world.racking.vertical_tote_capacity
    );

    // The zone-level summary, which is where the two readings of the shelf list
    // part company. RECORDED, not gated: the world document states the survey
    // and the v3 compiler derives a taller nominal stack from grid geometry.
    let zone_shape = |map: &WarehouseMap| -> Vec<(u16, u64, Option<u64>, usize)> {
        let mut counts: BTreeMap<(u16, u64, Option<u64>), usize> = BTreeMap::new();
        for zone in &map.zones {
            *counts.entry((zone.real_shelf_count, zone.zone_height_mm, zone.zone_width_mm)).or_default() += 1;
        }
        counts.into_iter().map(|((a, b, c), n)| (a, b, c, n)).collect()
    };
    println!("  zone summary (shelves, height mm, width mm, how many zones):");
    println!("    legacy: {:?}", zone_shape(&legacy_map));
    println!("    world:  {:?}", zone_shape(&world.map));
    println!("  Deliberate: the document states eleven measured shelf heights and");
    println!("  the topmost as the zone height; the v3 compiler counts thirteen");
    println!("  nominal shelves and sums them. Recorded here, never asserted equal.");
}

// ---------------------------------------------------------------------------
// 6. Arcs
// ---------------------------------------------------------------------------

/// Every attribute `desgraph::Edge` carries, read for one arc.
///
/// The two id-bearing attributes are translated out of their loader's own
/// numbering before they are compared: `dest_only` names a cell, so it is held
/// as that cell's millimetre centre, and `vtm_zone_gate` names a zone, so it is
/// held as the aisle column that zone's VTM works. Comparing the raw ids would
/// compare two independent minting orders and mean nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ArcAttributes {
    distance_mm: u64,
    vtm_traversal: bool,
    htm_traversal: &'static str,
    htm_reverse: bool,
    rtp_traversal: bool,
    dest_only: Option<GateAt>,
    vtm_zone_gate: Option<i64>,
    curve: Option<(Mm, bool)>,
}

/// The cells an arc's destination gate admits, as their millimetre centres.
///
/// One for an approach into an operator's standing spot; two for a machine's
/// door, which admits the robots bound for either end of its belt. Held as
/// centres for the same reason `dest_only` is: comparing raw cell ids would
/// compare two independent minting orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GateAt {
    Cell(Mm),
    Handovers([Mm; 2]),
}

fn htm_name(t: HtmTraversal) -> &'static str {
    match t {
        HtmTraversal::None => "None",
        HtmTraversal::UnladenOnly => "UnladenOnly",
        HtmTraversal::Both => "Both",
    }
}

/// Which attributes two readings of the same arc disagree on.
fn arc_disagreements(l: &ArcAttributes, w: &ArcAttributes) -> Vec<&'static str> {
    let mut out = Vec::new();
    if l.distance_mm != w.distance_mm {
        out.push("distance_mm");
    }
    if l.vtm_traversal != w.vtm_traversal {
        out.push("traversability.vtm_traversal");
    }
    if l.htm_traversal != w.htm_traversal {
        out.push("traversability.htm_traversal");
    }
    if l.htm_reverse != w.htm_reverse {
        out.push("htm_drive");
    }
    if l.rtp_traversal != w.rtp_traversal {
        out.push("traversability.rtp_traversal");
    }
    if l.dest_only != w.dest_only {
        out.push("dest_only");
    }
    if l.curve != w.curve {
        out.push("curve");
    }
    if l.vtm_zone_gate != w.vtm_zone_gate {
        out.push("vtm_zone_gate");
    }
    out
}

/// One map's arcs, keyed by the ordered pair of cell centres.
///
/// A cell pair may appear at most once: two arcs joining the same ordered pair
/// at different distances would make this map's contents depend on which one
/// the iteration reached last, so a duplicate fails here rather than being
/// resolved by hash order.
fn arcs_by_cell_pair(map: &WarehouseMap, zone_column: &FxHashMap<ZoneId, i64>) -> FxHashMap<(Mm, Mm), ArcAttributes> {
    let at = |cell| key(map.cell_world_pos(cell).expect("an arc's cells have centres"));
    let mut out: FxHashMap<(Mm, Mm), ArcAttributes> = FxHashMap::default();
    // ordered: printed in a failure message.
    let mut duplicates: BTreeSet<(Mm, Mm)> = BTreeSet::new();
    for (from, to, edge) in map.graph.edges() {
        // Destructured, not field-picked: a new attribute on `Edge` or on
        // `EdgeTraversability` stops this file compiling instead of silently
        // going uncompared.
        let Edge { traversability, distance_mm, dest_only, vtm_zone_gate, curve, htm_drive } = *edge;
        let EdgeTraversability { vtm_traversal, htm_traversal, rtp_traversal } = traversability;
        let attributes = ArcAttributes {
            curve: curve.map(|arc| (key(arc.center), arc.clockwise)),
            distance_mm,
            vtm_traversal,
            htm_traversal: htm_name(htm_traversal),
            htm_reverse: htm_drive == HtmDriveDirection::Reverse,
            rtp_traversal,
            dest_only: dest_only.map(|gate| match gate {
                DestGate::Cell(cell) => GateAt::Cell(at(cell)),
                DestGate::Handovers([a, b]) => GateAt::Handovers([at(a), at(b)]),
            }),
            vtm_zone_gate: vtm_zone_gate.map(|z| {
                *zone_column
                    .get(&z)
                    .unwrap_or_else(|| panic!("an arc gates on zone {z:?}, which works no aisle column"))
            }),
        };
        if out.insert((at(from), at(to)), attributes).is_some() {
            duplicates.insert((at(from), at(to)));
        }
    }
    assert!(
        duplicates.is_empty(),
        "{} ordered cell pairs carry more than one arc, so which arc's attributes are compared would \
         depend on iteration order: {:?}",
        duplicates.len(),
        duplicates.iter().take(10).map(|(a, b)| format!("{} -> {}", fmt_mm(*a), fmt_mm(*b))).collect::<Vec<_>>()
    );
    assert_eq!(out.len(), map.graph.edge_count(), "the arcs do not have one distinct cell pair each");
    out
}

/// Every arc compared by BOTH its endpoints and EVERY attribute it carries, and
/// the legacy-only bucket pinned by the cell kinds its arcs join.
///
/// Endpoints and distance alone are not the arc: `traversability` decides which
/// robots may take it and in which cargo state, `dest_only` closes it to all but
/// one destination, and `vtm_zone_gate` closes it to all but one zone's work. An
/// arc that agreed on geometry and disagreed on any of those is a different
/// floor for a robot. RTP flags, station-footprint HTM flags, and VTM-only zone
/// labels on HTM relay ingresses differ. Relay ingress destination gates and
/// reverse exits must agree after restoring the physical access positions.
///
/// The legacy-only bucket is pinned by an exact endpoint-kind histogram rather
/// than by its size, because a size alone lets a world arc be traded for a
/// legacy arc it has nothing to do with: drop one world arc, gain one
/// legacy-only arc, and a count-and-subset gate never notices.
#[test]
fn arcs() {
    let (_design, legacy_map) = legacy();
    let world = world();

    let column_of = |map: &WarehouseMap| -> FxHashMap<ZoneId, i64> {
        zones_by_aisle(map).0.iter().map(|(column, reading)| (reading.zone.id, *column)).collect()
    };

    println!("\n=== cencora map parity: arcs ===");
    println!("  arcs: legacy {}   world {}", legacy_map.graph.edge_count(), world.map.graph.edge_count());
    // Three terminal stands per bay, each with ingress and reverse egress.
    assert_eq!(world.map.graph.edge_count(), 30372 + 3 * BAYS * 2);

    let l_arcs = arcs_by_cell_pair(&legacy_map, &column_of(&legacy_map));
    let w_arcs = arcs_by_cell_pair(&world.map, &column_of(&world.map));
    println!("  distinct ordered cell pairs: legacy {}   world {}", l_arcs.len(), w_arcs.len());

    // Legacy's cells by centre, used below to name an arc's endpoints by kind.
    let l_cells = by_centre(legacy_map.graph.cells());
    assert!(l_cells.stacked.is_empty(), "legacy stacks cells on one centre: {:?}", l_cells.stacked);
    let kind_at = |k: Mm| format!("{:?}", l_cells.at[&k].kind);

    // Every attribute on every arc both sides state, compared pair by pair.
    // ordered: walked, printed and classified in a stable order.
    let mut disagreeing: BTreeMap<&'static str, Vec<(Mm, Mm)>> = BTreeMap::new();
    let mut shared = 0usize;
    let mut both: Vec<(Mm, Mm)> = w_arcs.keys().filter(|p| l_arcs.contains_key(*p)).copied().collect();
    both.sort_unstable();
    for pair in &both {
        shared += 1;
        for attribute in arc_disagreements(&l_arcs[pair], &w_arcs[pair]) {
            disagreeing.entry(attribute).or_default().push(*pair);
        }
    }
    println!("  arcs both sides state: {shared}");
    println!("  attributes compared per arc: distance_mm, traversability (vtm / htm / rtp), dest_only, vtm_zone_gate");
    println!("  arcs disagreeing, by attribute:");
    if disagreeing.is_empty() {
        println!("    (none)");
    }
    for (attribute, pairs) in &disagreeing {
        println!("    {attribute:<32} {:>6}", pairs.len());
        for pair in pairs.iter().take(4) {
            println!(
                "      {} -> {}: legacy {:?} world {:?}",
                fmt_mm(pair.0),
                fmt_mm(pair.1),
                l_arcs[pair],
                w_arcs[pair]
            );
        }
    }

    println!("\n  -- relay agreement and the three explained attribute classes --");
    let class = |attribute: &str| -> &[(Mm, Mm)] { disagreeing.get(attribute).map(Vec::as_slice).unwrap_or_default() };

    // Every physical relay stand has one gated forward ingress. Its inverse
    // directed edge is the reverse-drive exit over the same segment.
    let relay_stands: FxHashSet<Mm> = world
        .map
        .relays
        .iter()
        .map(|relay| key(world.map.cell_world_pos(relay.htm_access).expect("relay stand has a centre")))
        .collect();
    let relay_entries: FxHashSet<(Mm, Mm)> = both
        .iter()
        .copied()
        .filter(|pair| {
            relay_stands.contains(&pair.1)
                && w_arcs[pair].dest_only == Some(GateAt::Cell(pair.1))
                && !w_arcs[pair].htm_reverse
        })
        .collect();
    let relay_exits: FxHashSet<(Mm, Mm)> = both
        .iter()
        .copied()
        .filter(|pair| relay_stands.contains(&pair.0) && w_arcs[pair].dest_only.is_none() && w_arcs[pair].htm_reverse)
        .collect();
    println!("  terminal relay ingress gates: {}", relay_entries.len());
    println!("  terminal relay reverse exits:  {}", relay_exits.len());
    assert_eq!(relay_stands.len(), 684, "one physical HTM stand per exchange point");
    assert_eq!(relay_entries.len(), 684, "each relay stand has one gated forward ingress");
    assert_eq!(relay_exits.len(), 684, "each relay stand has one reverse egress");
    assert!(
        relay_entries.iter().all(|&(from, stand)| relay_exits.contains(&(stand, from))),
        "each relay ingress must pair with the reverse egress over the same segment"
    );
    assert!(class("dest_only").is_empty(), "relay ingress destination gates must match legacy");
    assert!(class("htm_drive").is_empty(), "relay reverse egress must match legacy");

    // Legacy labels entry into any Relay cell with a VTM-zone gate, even on
    // HTM-only edges. Both pathfinders apply this gate only to VTMs, which
    // neither map admits on these edges. Pin its exact location and value.
    assert_eq!(class("vtm_zone_gate").iter().copied().collect::<FxHashSet<_>>(), relay_entries);
    let legacy_columns = column_of(&legacy_map);
    assert!(class("vtm_zone_gate").iter().all(|p| {
        !l_arcs[p].vtm_traversal
            && !w_arcs[p].vtm_traversal
            && l_arcs[p].vtm_zone_gate == Some(legacy_columns[&l_cells.at[&p.1].zone])
            && w_arcs[p].vtm_zone_gate.is_none()
    }));

    // rtp_traversal: legacy true, world false, on EVERY arc both sides state.
    // cencora is a RELAY site -- VTMs and HTMs, no racks-to-picker fleet -- and
    // its document says so cell by cell: every floor cell states
    // `traversable_by: ["htm"]`, so the world loader reads "no RTP here" off
    // the survey. The v3 compiler starts every edge at
    // `EdgeTraversability::fully_traversable()` and only clears the RTP flag
    // for cells its RTP walk cannot reach; that walk starts at every station
    // and crosses any edge, so the initial `true` survives across the connected
    // floor. Nothing reads the flag unless an RTP robot asks
    // (`model::bot::robot`, `RobotType::Rtp`), and this site deploys none.
    println!("  system_type: legacy {:?}   world {:?}", legacy_map.system_type, world.map.system_type);
    println!(
        "  rtp_traversal disagreements: {} (legacy true, world false, on every shared arc)",
        class("traversability.rtp_traversal").len()
    );
    assert!(
        matches!(legacy_map.system_type, SystemType::Relay) && matches!(world.map.system_type, SystemType::Relay),
        "the RTP-flag class is explained by cencora being a relay site, and one of the two maps no longer says so"
    );
    assert!(
        w_arcs.values().all(|a| !a.rtp_traversal),
        "the world side no longer closes every arc to RTP robots, so the RTP class is not one whole-map class"
    );
    assert!(
        class("traversability.rtp_traversal").iter().all(|p| l_arcs[p].rtp_traversal && !w_arcs[p].rtp_traversal),
        "an RTP-flag disagreement runs the other way: the world opens an arc legacy closes"
    );
    assert_eq!(
        class("traversability.rtp_traversal").len(),
        shared,
        "the RTP flag no longer disagrees on every shared arc, so it is no longer one whole-map class"
    );

    // Legacy closes HTM travel touching StationArea; the world document
    // makes those footprints HTM-traversable. The retired parking strip now
    // agrees on both sides as ordinary floor, removing its old 1,114-arc
    // disagreement. Compare each remaining pair, not just the total.
    let reachable = htm_reachable_centres(&world.map);
    let stranded: Vec<&Cell> = world
        .map
        .graph
        .cells()
        .filter(|c| !reachable.contains(&key(c.world_pos.expect("a cell has a centre"))))
        .collect();
    println!("  world cells no HTM can reach from a relay standing cell or a presentation cell: {}", stranded.len());
    print_histogram("stranded world cells, by kind", &kind_histogram(stranded.iter().copied()));
    println!(
        "  htm_traversal disagreements: {} (legacy None, world Both)",
        class("traversability.htm_traversal").len()
    );
    assert!(
        class("traversability.htm_traversal")
            .iter()
            .all(|p| l_arcs[p].htm_traversal == "None" && w_arcs[p].htm_traversal == "Both"),
        "an HTM-flag disagreement runs the other way: legacy opens an arc to HTMs that the world closes"
    );
    // ordered: printed and pinned.
    let mut opened: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut opened_cells: FxHashSet<Mm> = FxHashSet::default();
    let mut opened_live = 0usize;
    for pair in class("traversability.htm_traversal") {
        *opened.entry((kind_at(pair.0), kind_at(pair.1))).or_default() += 1;
        opened_cells.insert(pair.0);
        opened_cells.insert(pair.1);
        if reachable.contains(&pair.0) && reachable.contains(&pair.1) {
            opened_live += 1;
        }
    }
    println!("  the arcs the world opens to HTMs and legacy closes, by the kinds they join:");
    for ((a, b), count) in &opened {
        println!("    {a:<14} -> {b:<14} {count:>6}");
    }
    println!("  distinct cells they touch: {}", opened_cells.len());
    println!(
        "  of those arcs, ones whose BOTH cells an HTM can reach in the world graph: {opened_live} of {}",
        class("traversability.htm_traversal").len()
    );
    println!(
        "  COST, recorded not gated away: the world map permits HTMs across a station footprint; \
         the v3 map closes travel touching StationArea."
    );
    assert_eq!(
        opened,
        BTreeMap::from([(("Open".into(), "StationArea".into()), 37), (("StationArea".into(), "Open".into()), 37),]),
        "the arcs the world opens to HTMs and the v3 compiler closes are no longer exactly \
         the station footprints, at exactly these counts"
    );

    assert_eq!(
        disagreeing.keys().copied().collect::<Vec<&str>>(),
        vec!["traversability.htm_traversal", "traversability.rtp_traversal", "vtm_zone_gate"],
        "an arc attribute outside the three explained classes disagrees between the two loaders"
    );

    let mut w_only: Vec<(Mm, Mm)> = w_arcs.keys().filter(|p| !l_arcs.contains_key(*p)).copied().collect();
    w_only.sort_unstable();
    let l_only: Vec<(Mm, Mm)> = l_arcs.keys().filter(|p| !w_arcs.contains_key(*p)).copied().collect();
    println!("  cell pairs joined only in legacy: {}", l_only.len());
    // Which cells those arcs touch, so the legacy-only bucket is explained by
    // class rather than by size. Every one of them should touch a cell the
    // world floor does not have -- a blanked position, a relay column, an
    // aisle -- or a station queue the world's one-way discipline closed.
    // ordered: printed and pinned.
    let mut endpoints: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (a, b) in &l_only {
        *endpoints.entry((kind_at(*a), kind_at(*b))).or_default() += 1;
    }
    println!("  legacy-only arcs by the kinds they join:");
    for ((a, b), count) in &endpoints {
        println!("    {a:<14} -> {b:<14} {count:>6}");
    }
    println!("  cell pairs joined only in world:  {}", w_only.len());
    for (a, b) in w_only.iter().take(10) {
        println!("    world only: {} -> {}", fmt_mm(*a), fmt_mm(*b));
    }

    // The gate: the world side connects nothing legacy does not, at the same
    // stated distance and under the same traversal rules. Its arcs are a
    // SUBSET, because the world floor is a subset of the legacy grid and its
    // queue discipline drops more edges.
    assert!(w_only.is_empty(), "{} arcs join cells legacy leaves unjoined", w_only.len());
    assert_eq!(shared, w_arcs.len(), "every world arc is a legacy arc carrying the same attributes");

    // And the residue is pinned by CLASS AND SIZE, one entry per pair of cell
    // kinds. Trading a world arc away for some unrelated legacy arc moves a
    // number in this table, which an assertion on the bucket's total would not
    // see.
    assert_eq!(
        endpoints,
        BTreeMap::from([
            (("Aisle".into(), "Aisle".into()), 646),
            (("Aisle".into(), "Relay".into()), 684),
            (("Relay".into(), "Aisle".into()), 684),
            (("Relay".into(), "Relay".into()), 1292),
        ]),
        "the legacy-only arcs no longer match the VTM corridors and relay columns"
    );
}
