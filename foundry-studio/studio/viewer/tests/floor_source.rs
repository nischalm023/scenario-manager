//! The floor-only source over a made site (spec 03 section 3.6, what the
//! viewer needs; section 8 statement 7, its half that needs no browser): the
//! L-Shape at its defaults as the template engine evaluated it
//! (`tests/fixtures/l_shape/world.json`, written by `site-template` on
//! studio/poc at 12cbc9938), compiled the one way a server compiles it,
//! written as the floor document and read back as a browser reads it.
//!
//! The browser's half, the same floor drawn by the wasm viewer headless, is
//! `studio/viewer/js/test/floor.test.mjs`.

use std::fs::read;
use std::path::Path;

use crucible_viewer::outline::cell_sides;
use crucible_viewer::roll::Roll;
use desgraph::CellId;
use dessim::worldmap::census::EntityPlace;
use dessim::worldmap::{WorldMap, load_from_bytes};
use worldview_render::{FloorAndRoll, FloorDoc, RollEntry};

fn world() -> WorldMap {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/l_shape/world.json");
    load_from_bytes(&read(path).expect("the fixture is checked in")).expect("the made L-Shape loads")
}

/// The floor and roll a browser reads, through the document a server writes.
fn sent(world: &WorldMap) -> (FloorDoc, Vec<RollEntry>) {
    let compiled = worldview_floor::compile(world).expect("the made L-Shape states a floor");
    FloorAndRoll::read(&FloorAndRoll::write(&compiled.floor, &compiled.roll()).expect("writes")).expect("reads")
}

/// The keys of the entities with `component` that stand on cells.
fn standing_with(world: &WorldMap, component: &str) -> Vec<(String, Vec<CellId>)> {
    let rows = world.entities.iter().filter(|row| row.components.iter().any(|name| &**name == component));
    rows.filter_map(|row| match &row.place {
        EntityPlace::Cells(cells) => Some((row.id.to_string(), cells.clone())),
        _ => None,
    })
    .collect()
}

#[test]
fn the_floor_document_carries_the_floor_and_the_roll_compiled() {
    let world = world();
    let compiled = worldview_floor::compile(&world).expect("compiles");
    let (floor, roll) = sent(&world);
    let packed = |floor: &FloorDoc| rmp_serde::to_vec(floor).expect("encodes");
    assert_eq!(packed(&floor), packed(&compiled.floor), "the floor, byte for byte");
    assert_eq!(roll, compiled.roll(), "the roll, entity for entity");
    let cells: Vec<&RollEntry> = roll.iter().filter(|entry| entry.is_cell).collect();
    assert!(cells.len() > 1_000 && cells.iter().all(|entry| entry.cells.len() == 1), "each cell marked, on itself");
    assert!(floor.cells.iter().flatten().count() > 1_000, "the made L-Shape's floor, not an empty one");
}

/// Statement 7: a pick over a station's cell lists the station before the
/// cell, for every station of the made site, and the cell itself last, as
/// the roll marks it. Only the lane into the station's cell comes before the
/// station: it stands on fewer cells.
#[test]
fn a_stations_cell_answers_the_station_before_the_cell() {
    let world = world();
    let entries = sent(&world).1;
    let marked_cell = |key: &str| entries.iter().any(|entry| entry.key == key && entry.is_cell);
    let roll = Roll::new(entries.clone());
    let stations = standing_with(&world, "station");
    assert_eq!(stations.len(), 6, "four pick stations and two putaway stations");
    let lanes: Vec<String> = standing_with(&world, "queue").into_iter().map(|(key, _)| key).collect();
    for (station, cells) in &stations {
        for &cell in cells {
            let keys = roll.keys_at(cell);
            let at = keys.iter().position(|key| key == station).unwrap_or_else(|| panic!("cell {cell:?}: {keys:?}"));
            assert!(keys[..at].iter().all(|key| lanes.contains(key)), "cell {cell:?}: only a lane before, {keys:?}");
            let last = keys.last().expect("the cell itself");
            assert!(marked_cell(last), "cell {cell:?}: the cell itself comes last, {keys:?}");
            assert_eq!(roll.cells_of(last), [cell]);
        }
    }
}

/// A station is outlined cell by cell, each side once: a side two of its
/// cells share is one stroke, not two and not none.
#[test]
fn a_station_is_outlined_cell_by_cell_each_side_once() {
    let world = world();
    let (floor, roll) = sent(&world);
    let roll = Roll::new(roll);
    for (station, cells) in standing_with(&world, "station") {
        let (marked, standing) = roll.cells_of_all([station.as_str()]);
        assert_eq!((marked.len(), standing), (cells.len(), 1));
        let sides = cell_sides(&marked, &floor.cells);
        assert!(sides.len() < 4 * cells.len(), "{station}: its cells share sides, each drawn once");
        let each: usize = marked.iter().map(|&cell| cell_sides(&[cell], &floor.cells).len()).sum();
        assert_eq!(each, 4 * cells.len(), "{station}: every cell outlined on all four of its sides");
    }
}
