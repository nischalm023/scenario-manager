//! The 21-aisle Cencora DA50 world (`cencora_v3_da50_aisles21.json`, cut by
//! `scripts/add_cencora_aisles.py` from the 19-aisle document of record) loads
//! as the 20-aisle world plus one zone standing alone in the L portion, with
//! the floor taken out where the annex's other eight aisles stand: the same
//! zone shape, no floor cell in the two regions of aisles 22 to 29, and every
//! station and relay cell still reachable around them.

use std::collections::VecDeque;
use std::path::PathBuf;

use desgraph::CellId;
use dessim::worldmap::{self, WorldMap};
use rustc_hash::FxHashSet;

fn aisles21() -> WorldMap {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cencora_v3_da50_aisles21.json");
    worldmap::load(&path).expect("load the committed 21-aisle cencora DA50 world document")
}

/// 21 zones, each a sandwich of two six-bay runs facing one aisle.
const ZONES: usize = 21;
const BAYS: usize = ZONES * 2 * 6;

/// Every cell the loader derives. The document held 9,484 before aisles 22 to
/// 29 were taken out, and their two regions take 1,619 band cells: 23 columns
/// by 19 rows east of aisle 21 (437), and 44 columns by 27 rows south of the
/// corridor between the annex rows (1,188, less the 6 a put station's clip
/// had already taken).
const CELLS: usize = 9_484 - 437 - (1_188 - 6);

/// The two regions of aisles 22 to 29 as x from, x to, y from, y to, in the
/// site frame the floor mirrors into (site y = 79,140 - floor y).
const ANNEX_REGIONS: [[f64; 4]; 2] = [
    // Aisles 22 to 24: east of aisle 21's east lane, floor y 40,040 to 56,190.
    [114_750.0, 134_300.0, 22_950.0, 39_100.0],
    // Aisles 25 to 29: south of the corridor between the annex rows, floor y
    // 56,190 to 79,140.
    [96_900.0, 134_300.0, 0.0, 22_950.0],
];

#[test]
fn aisle_21_is_one_more_of_the_same() {
    let world = aisles21();
    assert_eq!(world.map.zones.len(), ZONES);
    for zone in &world.map.zones {
        assert_eq!(zone.cells.len(), 12, "twelve rack anchors per zone");
        assert_eq!(zone.relays.len(), 36, "12 bays x 3 board positions");
        assert_eq!(zone.vtm_count, 2, "one cohort of two DA50s per zone");
    }
    assert_eq!(world.map.vtm_count(), ZONES * 2);
    assert_eq!(world.map.relays.len(), BAYS * 3);
}

#[test]
fn the_floor_extent_stops_at_the_corridor_between_the_annex_rows() {
    let world = aisles21();
    let bounds = world.map.grid_bounds.expect("the header extent");
    assert_eq!(bounds.width, 158, "aisle 21 is cut out of the L portion, so no new columns");
    // The extent counts distinct row centres, not millimetres: the strip's lane
    // patches stand on their own 645 mm rows, eighteen of them, below the
    // corridor, where the main row's lanes already contribute their eighteen.
    // The region of aisles 25 to 29 takes the L's last 27 rows, and no other
    // floor reaches that far south.
    assert_eq!(bounds.depth, 96 + 18 - 27);
    assert_eq!(bounds.cell_size_mm, 850.0);
}

#[test]
fn no_floor_where_aisles_22_to_29_stand() {
    let world = aisles21();
    assert_eq!(world.map.graph.cells().count(), CELLS);
    for cell in world.map.graph.cells() {
        let at = cell.world_pos.expect("every cell has a centre");
        for [x0, x1, y0, y1] in ANNEX_REGIONS {
            assert!(
                !(x0 < at.x_mm && at.x_mm < x1 && y0 < at.y_mm && at.y_mm < y1),
                "cell {:?} at ({}, {}) stands where aisles 22 to 29 do",
                cell.id,
                at.x_mm,
                at.y_mm
            );
        }
    }
}

#[test]
fn every_relay_cell_of_aisle_21_has_a_way_in() {
    let world = aisles21();
    // Found by name: zones are numbered by position, and aisle 21 is not last.
    let aisle_21 = world
        .map
        .zones
        .iter()
        .find(|zone| zone.name.as_deref() == Some("inner aisle: aisle 21"))
        .expect("aisle 21's zone");
    assert_eq!(aisle_21.relays.len(), 36);
    for relay_id in &aisle_21.relays {
        let relay = world.map.relays.iter().find(|r| r.id == *relay_id).expect("the zone's relay is on the map");
        assert!(
            !world.map.graph.neighbors(relay.htm_access).is_empty(),
            "relay {:?} of aisle 21 stands on cell {:?} with no arc into it",
            relay.id,
            relay.htm_access
        );
    }
}

/// The carve runs right up to put stations 23 to 26 and closes the east side
/// of aisle 21's east lane, so nothing may be left cut off: from one station
/// every station and every relay cell is still reached.
#[test]
fn every_station_and_relay_cell_is_still_reachable() {
    let world = aisles21();
    let map = &world.map;
    let start = map.stations[0].delivery_cell();
    let mut seen: FxHashSet<CellId> = FxHashSet::default();
    seen.insert(start);
    let mut queue = VecDeque::from([start]);
    while let Some(cell) = queue.pop_front() {
        for (next, _) in map.graph.neighbors(cell) {
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    for station in &map.stations {
        assert!(station.presentation_cells().all(|cell| seen.contains(&cell)), "station {:?} is cut off", station.id);
    }
    for relay in &map.relays {
        assert!(seen.contains(&relay.htm_access), "relay {:?} on cell {:?} is cut off", relay.id, relay.htm_access);
    }
}
