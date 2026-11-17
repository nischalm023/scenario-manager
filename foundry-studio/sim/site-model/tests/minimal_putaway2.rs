//! A real desmap fixture, converted and then carried into v0.6: one world
//! document, no scenario wrapper.
//!
//! This is the case that found the hole the Dallas slice could not: its rack row
//! stands *inside* the floor it serves. Through v0.5 that mattered to the stitch
//! machinery, because the row's access minted six cells of its own and they had
//! to be matched back into the floor; under v0.6 the relays reach for cells that
//! are already there, which is the same fact with one entity instead of two and
//! no seam to get wrong. Pinning it here keeps the bare-document path covered
//! too — a document with no `layers` list loads standalone.
//!
//! The fixture beside this file is the v0.5 example under `examples/`, carried
//! over by the mechanical transform v0.6 states: suffixes off the length fields,
//! the rack's four vertical fields folded into one `levels` list, and the
//! boards-access folded into the relay level whose face and reach it used to
//! state. Its primary frame declares `unit_mm: 1`, so every number the
//! conversion measured is the number it still says.

use std::path::{Path, PathBuf};
use site_model::components::product::{CellAddress, Queue};
use site_model::components::structure::AccessCell;
use site_model::{Coord, EntityId};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/seed/application-tests/native/fixtures/minimal-putaway2/minimal_putaway2.json")
}

fn id(text: &str) -> EntityId {
    text.parse().expect("a literal identity in this test")
}

/// The identity a generator derives at one key: the generator, then the chain of
/// segments beneath it. That address *is* what the derived entity is named by
/// (SPEC §3), so a test writes it exactly as a document would.
fn at(generator: impl std::fmt::Display, key: impl std::fmt::Display) -> EntityId {
    format!("{generator}/{key}")
        .parse()
        .expect("a well-formed derivation address in this test")
}

const FLOOR: &str = "5abcf41a-ec51-4029-9793-0734da893e7f";
const RACK_ROW: &str = "1d931662-e00e-4cb8-b431-38d5c08f119d";
const ACCESS: &str = "8c5b0e91-4a72-4d38-b6ef-2170f9c4ad35";
const LEGACY_GRID: &str = "4e96b8ce-6e9a-43aa-b10a-4faaf8776398";

#[test]
fn a_bare_world_document_loads_with_no_scenario_around_it() {
    let (set, world) = site_model::load(&fixture()).expect("the fixture loads");
    assert!(set.layers.is_empty(), "the document composes nothing");
    // 19 declarations, the two stations' lanes among them, plus two
    // presentation-point overrides. The former empty idle-floor sector is
    // gone; its underlying floor cells remain.
    assert_eq!(world.entities.len() - world.derived_count(), 19);
    assert_eq!(world.entities.len(), 19 + 222);

    // Its `required_components` names every load-bearing component it uses, so
    // the load-bearing check has nothing to say; every relay finds its cell, so
    // nothing here is steel nobody reaches.
    assert_eq!(world.warnings.len(), 0, "{:?}", world.warnings);
}

#[test]
fn the_floor_and_the_rack_row_expand_to_what_the_document_implies() {
    let (_, world) = site_model::load(&fixture()).expect("the fixture loads");
    let count = |generator: &str| {
        world
            .counts
            .get(&generator.parse().expect("a literal UUID in this test"))
            .copied()
            .unwrap_or(0)
    };
    // Floor: 8000/1000 = 8 columns x 18000/1000 = 18 rows = 144, less the twelve
    // the zone-footprint exclusion covers — the rack column 6 and the aisle
    // column 7, rows 2 to 7 in both.
    assert_eq!(count(FLOOR), 132);
    // Rack row: 6 instances, each an instance entity + its `rack` internal +
    // 1 relay + 10 storage levels x 1 wide x 1 deep = 10 slots. The relay level
    // sits between two storage levels in the list, so the count is what it was
    // and only the keys moved.
    assert_eq!(count(RACK_ROW), 6 * 13);
    // The access derives nothing at all: it is a statement about who works this
    // face, and SPEC §7's checks are its only consumers.
    assert_eq!(count(ACCESS), 0);
    // Two cohorts: 10 HTMs and 2 VTMs.
    assert_eq!(count("584dc0c9-79f9-4d06-b893-448306c3dd1c"), 10);
    assert_eq!(count("9c128a5c-73ea-4c2e-a9b1-5a54a4ab54bd"), 2);
    assert_eq!(world.suppressions.len(), 12);
}

/// The legacy frame hands every floor cell back the old grid index it was
/// converted from. That is the whole job the frame was kept for — "old
/// `[col,row]` citations have a declared meaning" is only true if the
/// declaration reproduces the citations.
///
/// The old fixture put the cell of `[c,r]` at millimetres `[1000c+500,
/// 1000r+500]` — an index names a cell and a cell's place is its centre — and
/// this floor tiles the same 1000 mm pitch from `[0,0]`, so `cell:c,r` is
/// centred on exactly that point. The frame declares that same arithmetic:
/// scale `[1000, 1000]`, translation `[500, 500]`. Projecting a centre back
/// into it subtracts the translation, divides by the scale and rounds half-up
/// (SPEC §4), so `(5500 − 500) / 1000 = 5` exactly. Declaring translation
/// `[0, 0]` instead would make the frame say index `[c,r]` sits on the cell's
/// minimum corner, and the same centre would come back as `5500 / 1000 = 5.5
/// → 6`: every cell one index high on both axes, for a legacy consumer that
/// has no way to notice.
#[test]
fn every_floor_cell_projects_onto_the_old_grid_index_it_came_from() {
    let (_, world) = site_model::load(&fixture()).expect("the fixture loads");
    let grid = id(LEGACY_GRID);

    // The cell an HTM stands on to work bay 2's relay: old grid [5,4].
    let centre = world.cells[&at(FLOOR, "cell:5,4")].centre;
    assert_eq!([centre[0].0, centre[1].0], [5500, 4500]);
    assert_eq!(
        world
            .frames
            .from_primary(&grid, centre)
            .expect("it projects"),
        [Coord(5), Coord(4)]
    );
    // And back the other way with nothing to round: index [5,4] IS that centre.
    assert_eq!(
        world
            .frames
            .to_primary(&grid, [Coord(5), Coord(4)])
            .expect("it pushes"),
        centre
    );

    // Not one cell but all 132: every surviving tile of the floor reports the
    // index the old file addressed it by.
    let mut checked = 0;
    for row in 0i64..18 {
        for col in 0i64..8 {
            let key = format!("cell:{col},{row}");
            let Some(geometry) = world.cells.get(&at(FLOOR, &key)) else {
                // The only cells missing are the zone footprint's: the rack
                // column 6 and the aisle column 7, rows 2 to 7 of both.
                assert!(
                    (6..8).contains(&col) && (2..8).contains(&row),
                    "{key} vanished"
                );
                continue;
            };
            assert_eq!(
                world
                    .frames
                    .from_primary(&grid, geometry.centre)
                    .expect("it projects"),
                [Coord(col), Coord(row)],
                "{key} projects onto a square that is not its own"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 132);
}

/// Every relay binds the floor cell an HTM stands on to work it — and that cell
/// is an ordinary cell of the floor, not one the rack minted beside itself.
#[test]
fn every_relay_binds_the_floor_cell_in_front_of_it() {
    let (_, world) = site_model::load(&fixture()).expect("the fixture loads");
    // The stamp is rotated 90°, so the relay level's +y face looks along -x:
    // bay i's relay point sits at [6000, 2500 + 1000i] on the face, and the
    // nearest cell outward of it within the level's 600-step reach is the floor
    // cell centred 500 away at [5500, 2500 + 1000i].
    for bay in 0..6 {
        let relay = at(RACK_ROW, format!("instance:{bay}/local:rack/relay:1,0"));
        let binding: AccessCell = world.entities[&relay]
            .require("access_cell")
            .expect("the relay bound a cell");
        let CellAddress::Id(cell) = binding.cell else {
            panic!("the search binds a cell by its own identity");
        };
        assert_eq!(
            cell,
            at(FLOOR, format!("cell:5,{}", 2 + bay)),
            "bay {bay}'s relay bound something other than the cell beneath it"
        );
    }
    // Nothing is minted and nothing is linked: a binding is a component naming a
    // cell, not an arc, so the floor's own two-way arcs are the whole graph.
    assert_eq!(world.edges.iter().filter(|e| e.bidirectional).count(), 0);
}

#[test]
fn the_floor_compiles_two_way_around_the_rack() {
    let (_, world) = site_model::load(&fixture()).expect("the fixture loads");
    // The floor declares no flow, so it is two-way (SPEC §5). An 8 x 18 grid has
    // 7 x 18 = 126 horizontal and 8 x 17 = 136 vertical neighbour pairs, 262 in
    // all. The exclusion covers columns 6 and 7, rows 2 to 7. Column 7 is the
    // last one, so the twelve excluded cells sit against the east edge: they
    // take out 12 horizontal pairs (rows 2 to 7 each lose 5-6 and 6-7) and 14
    // vertical ones (each column loses the seven runs from row 1 through row 8),
    // leaving 114 + 122 = 236 pairs and 472 arcs. Nothing else contributes: the
    // v0.5 load carried six more, the links a boards-access made.
    assert_eq!(world.edges.len(), 472);

    // And the graph really routes around the zone: nothing crosses the rack
    // column or the aisle beside it, where the exclusion bit.
    for key in ["cell:6,4", "cell:7,4"] {
        let hole = at(FLOOR, key);
        assert!(!world.entities.contains_key(&hole));
        assert!(!world
            .edges
            .iter()
            .any(|edge| edge.from == hole || edge.to == hole));
    }
}

/// Both stations' lanes queue over the *floor's* cells rather than cells of
/// their own, which is the case a bare key string could never have expressed:
/// the address names the floor generator, and the station is somewhere else
/// entirely. Each lane ends at its station's presentation cell.
#[test]
fn both_station_lanes_queue_over_floor_cells_that_exist() {
    let (_, world) = site_model::load(&fixture()).expect("the fixture loads");
    for (lane, step, presentation) in [
        (
            "a5c47601-4c99-451f-90df-452961151ee7",
            "cell:3,0",
            "cell:4,0",
        ),
        (
            "8d821130-9026-49d4-8fbf-d180406891de",
            "cell:3,8",
            "cell:4,8",
        ),
    ] {
        let queue: Queue = world.entities[&id(lane)]
            .require("queue")
            .expect("the lane's queue");
        let cells: Vec<EntityId> = queue
            .path
            .iter()
            .map(|address| {
                let CellAddress::Derived(raw) = address else {
                    panic!("a local handle outside any template");
                };
                EntityId::from(raw.parse().expect("a well-formed address"))
            })
            .collect();
        assert_eq!(cells, [at(FLOOR, step), at(FLOOR, presentation)]);
        assert!(
            cells.iter().all(|cell| world.entities[cell].has("cell")),
            "{lane}'s steps are floor cells"
        );
    }
}

/// `external_ids` is data, not identity, and this crate knows nothing about it:
/// no typed struct, no check, no rule. What matters is that it survives — parse,
/// expansion, bake — byte for byte, because a legacy identifier that silently
/// stops round-tripping is exactly how a converted fixture loses its join back
/// to the source it came from.
#[test]
fn external_ids_ride_through_untouched() {
    let (set, world) = site_model::load(&fixture()).expect("the fixture loads");
    // All six carriers: the old zone record became steel, a hole in the floor and
    // a grouping, and its id rides on each of the three; the two stations keep
    // theirs; and the VTM class carries the only thing the old map names about a
    // robot. Removed idle-floor annotations have no simulation identity.
    let expected = [
        (
            "1d931662-e00e-4cb8-b431-38d5c08f119d",
            serde_json::json!({ "desmap_zone_id": 1, "desmap_rack_side": "Left" }),
        ),
        (
            "f4520a1e-8447-486a-8f9e-58cc4dcea8c9",
            serde_json::json!({ "desmap_zone_id": 1 }),
        ),
        (
            "3f655773-e134-4e02-931c-0844275744fb",
            serde_json::json!({ "desmap_zone_id": 1 }),
        ),
        (
            "9d8faf9e-f8c7-453c-b093-e153cf44d484",
            serde_json::json!({ "desmap_station_id": 1 }),
        ),
        (
            "76e971d8-e998-4e8b-b8f2-b2800f4d2fd1",
            serde_json::json!({ "desmap_station_id": 2 }),
        ),
        (
            "3e804d73-0f51-41df-9988-baa99d3e42d0",
            serde_json::json!({ "desmap_vtm_model": "A5" }),
        ),
    ];
    for (entity, value) in &expected {
        assert_eq!(
            world.entities[&id(entity)].components["external_ids"],
            *value
        );
    }
    assert_eq!(world.component_census().get("external_ids"), Some(&6));

    // And out the other side: the expanded form's records carry the same values.
    let document = site_model::expanded::emit(&set, &world);
    let records = document["entities"].as_array().expect("records");
    for (entity, value) in &expected {
        let record = records
            .iter()
            .find(|r| r["id"] == *entity)
            .expect("a declaration for this entity");
        assert_eq!(record["components"]["external_ids"], *value);
    }
}

/// What a robot class says about where it came from, now that who made a product
/// is a party with a role rather than a bare string on the class.
///
/// The conversion knew one thing about the VTM — the model name the old map
/// carries — and the migration put the maker it had invented in the one place a
/// party belongs. The capabilities stay flat on the class, exactly once, which
/// is the rule an option may describe and never override (SPEC §10.1).
#[test]
fn a_robot_class_says_who_made_it_in_its_lineage() {
    let (_, world) = site_model::load(&fixture()).expect("the fixture loads");
    let vtm = &world.entities[&id("3e804d73-0f51-41df-9988-baa99d3e42d0")];
    assert_eq!(
        vtm.components["lineage"],
        serde_json::json!([
            { "party": "generic", "role": "manufacturer", "designation": "VTM" }
        ])
    );
    assert!(vtm.components["robot_class"].get("manufacturer").is_none());
    assert_eq!(vtm.components["robot_class"]["height"], 3000);
}
