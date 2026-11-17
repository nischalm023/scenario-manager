//! Within `worldlens`'s tests, `patterns` is the ENGINE'S OWN ANSWER on the
//! four real documents: every pattern of `data/seed/application-tests/lenses/legacy-wps/world.lens` and every lens
//! over them, run against cencora-v3, walmart-dallas, minimal-putaway2 and the
//! minimal EasyLoop map, against the same two tables `census.rs` reaches by
//! hand.
//!
//! The table is written out in both files on purpose. One of them is the
//! engine and the other is a scan somebody could have written in a loader, and
//! a number they agree on is a number two different readings of the same
//! document produce. A number only one of them produces is a bug, and which
//! file has to change says which bug it is.

mod fixtures;

use fixtures::{document, documents, Fixture};
use std::collections::BTreeSet;
use worldlens::{Library, Matcher};

const LIBRARY: &str = foundry_world_lenses::LEGACY_WPS_LENS;

fn library() -> Library {
    Library::parse(LIBRARY).expect("the shipped pattern library parses")
}

/// One row per pattern of `data/seed/application-tests/lenses/legacy-wps/world.lens`: the count it comes to on
/// cencora-v3, walmart-dallas, minimal-putaway2 and the minimal EasyLoop map,
/// in that order. Every number was worked out twice — once by the scans in
/// `census.rs` and once by the engine in `patterns.rs` — and the two must agree.
///
/// Six rows read zero on all four, and those are the six whose whole claim IS
/// the zero: nobody writes `queue.into`, no station carries `queue` (the two
/// station-queue rows: a station's queue is a lane of its own), every relay of every document stands on
/// a cell, none of the four declares charging equipment, and no document says
/// which station an order carrier's head serves — walmart-dallas and the
/// minimal EasyLoop map each state two such heads and neither names a station on
/// one. Nine more read zero on the three reader fixtures and
/// answer only on the minimal EasyLoop map — `machine` and the five that read
/// what is posed in its frame, which want a machine authored cell by cell
/// rather than stamped from a template; and `building`, `highway` and a
/// `easyloop` naming its `part`, three components the other three documents
/// never write. `lane` and `lane_cell` answer wherever a station has a lane.
const EXPECTED: [(&str, [usize; 4]); 52] = [
    ("belt_designation", [0, 3, 0, 0]),
    ("building_envelope", [0, 0, 0, 1]),
    ("carrier_instance", [0, 75, 0, 10]),
    ("cell_grid", [146, 9, 1, 8]),
    ("cohort_assignment", [19, 1, 1, 2]),
    ("collection_cell", [0, 1, 0, 0]),
    ("derived_cell_of", [8089, 178, 132, 223]),
    ("document_cell", [8773, 184, 132, 251]),
    ("dropoff_cell", [0, 1, 0, 0]),
    ("easyloop_internal_of", [0, 1, 0, 0]),
    ("easyloop_machine", [0, 1, 0, 0]),
    ("easyloop_part", [0, 0, 0, 1]),
    ("exchange_cell", [0, 5, 0, 2]),
    ("floor_cohort", [1, 1, 1, 1]),
    ("flow_highway_cell", [0, 131, 0, 0]),
    ("highway_cell", [0, 0, 0, 32]),
    ("lane", [26, 0, 2, 6]),
    ("lane_cell", [136, 0, 4, 23]),
    ("machine", [0, 0, 0, 1]),
    ("machine_cell", [0, 0, 0, 28]),
    ("machine_pick_place", [0, 0, 0, 2]),
    ("machine_port", [0, 0, 0, 2]),
    ("machine_port_posed", [0, 0, 0, 2]),
    ("machine_station", [0, 1, 0, 0]),
    ("machine_waiting_cell", [0, 0, 0, 12]),
    ("open_cell", [8747, 179, 130, 215]),
    ("order_bay", [0, 2, 0, 5]),
    ("order_presentation", [0, 0, 0, 0]),
    ("charging_cell", [0, 0, 0, 0]),
    ("pick_station", [10, 1, 1, 0]),
    ("placed_station", [26, 4, 2, 3]),
    ("port_of_machine", [0, 5, 0, 0]),
    ("presentation_cell", [26, 0, 2, 3]),
    ("presentation_marked_cell", [26, 0, 2, 2]),
    ("primary_frame_scale", [1, 1, 1, 1]),
    ("queue_cell", [0, 0, 0, 0]),
    ("queue_into_station", [0, 0, 0, 0]),
    ("rack_bay", [228, 20, 6, 11]),
    ("rack_bay_with_relays", [1368, 56, 6, 11]),
    ("run_access", [38, 4, 1, 3]),
    ("sector_group", [19, 1, 1, 2]),
    ("sector_member", [38, 1, 1, 3]),
    ("stamp_instance", [372, 24, 6, 11]),
    ("station_bins", [0, 0, 2, 1]),
    ("station_exit", [26, 0, 2, 3]),
    ("station_presentation_cell", [26, 0, 2, 3]),
    ("station_with_queue", [0, 0, 0, 0]),
    ("stitch_pair", [420, 10, 0, 16]),
    ("storage_position", [10032, 3940, 60, 55]),
    ("unbound_relay", [0, 0, 0, 0]),
    ("vtm_cohort", [38, 1, 1, 3]),
    ("waiting_pool", [0, 1, 0, 0]),
];

/// Every pattern, on every document, comes to the table.
#[test]
fn the_engine_matches_the_table() {
    let library = library();
    assert_eq!(library.len(), EXPECTED.len(), "the table covers every pattern the library ships");
    for (column, Fixture { name, world, .. }) in documents().into_iter().enumerate() {
        let matcher = Matcher::new(&world.entities);
        for (pattern, wanted) in EXPECTED {
            let parsed = library.get(pattern).unwrap_or_else(|| panic!("`{pattern}` is in the library"));
            assert_eq!(matcher.matches(parsed).len(), wanted[column], "{name}: {pattern}");
        }
    }
}

/// cencora states every one of its 26 presentation points as an override on the
/// floor cell it sits on, and names the station outright — so the pattern that
/// follows the reference and the pattern that only looks at the cell agree, and
/// every station is reached.
#[test]
fn cencora_binds_its_presentation_points_by_reference() {
    let Fixture { world, .. } = document("cencora-v3");
    let library = library();
    let matcher = Matcher::new(&world.entities);
    let explicit = matcher.matches(library.get("presentation_cell").expect("the pattern"));
    assert_eq!(explicit.len(), 26);
    let stations: BTreeSet<_> = explicit.iter().filter_map(|b| b.get("s")).collect();
    assert_eq!(stations.len(), 26, "one station each, none named twice");
    let cells: BTreeSet<_> = explicit.iter().filter_map(|b| b.get("c")).collect();
    assert_eq!(cells.len(), 26);
    // Each of those points IS a cell, which is what an override on a derived
    // floor cell means.
    assert_eq!(matcher.matches(library.get("presentation_marked_cell").expect("the pattern")).len(), 26);
}

/// `queue.into` is in the reader's typed view and in nobody's document, so the
/// pattern that follows it answers zero everywhere. Asserting the zero is what
/// makes the field's disuse a fact rather than an impression.
#[test]
fn no_document_in_the_tree_writes_queue_into() {
    let library = library();
    let pattern = library.get("queue_into_station").expect("the pattern");
    for Fixture { name, world, .. } in documents() {
        let matcher = Matcher::new(&world.entities);
        assert!(matcher.matches(pattern).is_empty(), "{name} writes queue.into");
    }
}

/// A nested pattern folds a stamp's part-rows into one row for the stamp, and
/// that row is anchored at the stamp itself.
///
/// This is the engine's one existential, and the walmart-dallas catalog is what
/// exercises it: its EasyLoop is stamped from a template, so its `easyloop`
/// part hangs under the stamp, and `easyloop_machine` nests
/// `easyloop_internal_of` to reach the stamp through that part. The other
/// reading of a machine — one entity carrying its own frame, its envelope and
/// its belt timing — is `machine`, and the minimal EasyLoop map is the one
/// document authored that way, so the two patterns never answer the same
/// document.
#[test]
fn a_nested_pattern_folds_a_stamps_parts_into_the_stamp() {
    let library = library();
    let Fixture { world, .. } = document("walmart-dallas");
    let matcher = Matcher::new(&world.entities);
    let parts = matcher.matches(library.get("easyloop_internal_of").expect("the pattern"));
    let stamped = matcher.matches(library.get("easyloop_machine").expect("the pattern"));
    assert_eq!((parts.len(), stamped.len()), (1, 1), "nesting folds a stamp's parts into it");
    for binding in &stamped {
        let id = binding.get("m").expect("the anchor bound");
        assert!(world.entities[id].has("stamp"), "the row that survives is anchored at the stamp");
    }
    assert_eq!(
        matcher.matches(library.get("machine").expect("the pattern")).len(),
        0,
        "nothing here is a frame machine"
    );
}

/// A stamp is not one of its instances. The count of entities carrying `stamp`
/// and the count of what those stamps put down are answers to two different
/// questions: cencora-v3 carries 182 stamps that put down 372 instances since
/// its relay access was re-cut beneath the boards (2026-09-15), and
/// walmart-dallas eight that put down twenty-four. Both numbers are pinned
/// here, side by side, to keep one from being read as the other.
#[test]
fn a_stamp_is_not_one_of_its_instances() {
    let library = library();
    for (name, stamps, instances) in [("cencora-v3", 182usize, 372usize), ("walmart-dallas", 8, 24)] {
        let Fixture { world, .. } = document(name);
        let matcher = Matcher::new(&world.entities);
        assert_eq!(world.entities.values().filter(|e| e.has("stamp")).count(), stamps);
        assert_eq!(matcher.matches(library.get("stamp_instance").expect("the pattern")).len(), instances);
        assert!(instances > stamps, "{name}: every stamp puts down at least one instance");
    }
}

/// Every relay of every document is bound to a cell, so the negative pattern
/// answers zero — and the positive one answers once per relay, which is what
/// says each relay names exactly one bay and exactly one cell.
#[test]
fn every_relay_stands_on_exactly_one_cell() {
    let library = library();
    for (name, relays) in [("cencora-v3", 1368usize), ("walmart-dallas", 56), ("minimal-putaway2", 6)] {
        let Fixture { world, .. } = document(name);
        let matcher = Matcher::new(&world.entities);
        assert_eq!(matcher.matches(library.get("rack_bay_with_relays").expect("the pattern")).len(), relays);
        assert!(matcher.matches(library.get("unbound_relay").expect("the pattern")).is_empty());
        assert_eq!(world.entities.values().filter(|e| e.has("relay")).count(), relays);
    }
}

/// The minimal EasyLoop map is small enough to say out loud, so this is the
/// document that says what a whole answer looks like — and it is the one
/// document here whose EasyLoop is authored cell by cell, which is the shape
/// the `machine*` patterns were written for.
///
/// The machine is ONE entity carrying its own frame, the `easyloop` envelope
/// and the belt timing; membership is the frame, so the twenty-eight cells
/// posed in it are its members and nothing else is. It carries no `station`
/// component at all, which is why `machine_station` and `pick_station` answer
/// zero here while `machine_pick_place` answers two: the happening moved onto
/// the cell where it happens.
#[test]
fn the_minimal_easyloop_map_reads_end_to_end() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = library();
    let matcher = Matcher::new(&world.entities);
    let count = |name: &str| matcher.matches(library.get(name).expect("the pattern")).len();

    assert_eq!(count("machine"), 1, "one entity IS the machine");
    assert_eq!(count("machine_cell"), 28, "and twenty-eight cells are posed in its frame");
    assert_eq!(count("machine_pick_place"), 2, "two pick places, each a cell carrying the happening");
    assert_eq!(count("machine_port"), 2, "one dropoff and one collection cell");
    assert_eq!(count("machine_waiting_cell"), 12, "the 6+6 of the drawings, one cell at a time");
    assert_eq!(count("lane"), 6, "the inventory loop, one horseshoe per pick place, and one lane per station");
    // 5 + 4 + 4 + 4 + 3 + 3: the inventory lane's four cells under the belt plus
    // the collection port it ends at; three queue cells plus the presentation
    // cell for each pick place; and each station's queue cells plus the
    // presentation cell its lane ends at. The dropoff is the inventory lane's
    // ENTRY cell and not a step of its path (2026-09-04), which is why the
    // inventory lane states five and not six.
    assert_eq!(count("lane_cell"), 23, "and every step of them names a cell by identity");
    assert_eq!(count("easyloop_machine"), 0, "nothing here is stamped");
    assert_eq!(count("machine_station"), 0, "and nothing here carries `station` but the three that are not machines");

    assert_eq!(count("placed_station"), 3, "inventory induction, box induction, extraction");
    assert_eq!(count("presentation_cell"), 3, "the three that name their station");
    assert_eq!(count("presentation_marked_cell"), 2, "and the machine's two, which name none");
    assert_eq!(count("document_cell"), 251);
    assert_eq!(count("rack_bay"), 11);
    assert_eq!(count("rack_bay_with_relays"), 11, "one shelf each, each standing on one cell");
    assert_eq!(count("storage_position"), 55, "five levels a bay");
}

/// One row per lens of `data/seed/application-tests/lenses/legacy-wps/world.lens`: the number of objects it projects
/// on cencora-v3, walmart-dallas, minimal-putaway2 and the minimal EasyLoop map.
/// Worked out twice, like the pattern table above -- once by the engine here,
/// once by the anchor scans in `census.rs`.
///
/// Six of the seven names are a pattern's name as well, which is the point of
/// the two keywords: `library.lens("rack_bay")` and `library.get("rack_bay")`
/// answer two different questions about the same subject.
///
/// The EasyLoop rows are where the four documents part company.
/// `easyloop_station` answers only on the minimal map, whose machine is
/// authored cell by cell; `lane` answers wherever a station or a machine
/// states one; and `pick_station` answers everywhere but there,
/// because that map's three stations induct, put away and extract, and none of
/// them is a pick station.
const LENSES: [(&str, [usize; 4]); 7] = [
    ("easyloop_station", [0, 0, 0, 1]),
    ("floor_cohort", [1, 1, 1, 1]),
    ("lane", [26, 0, 2, 6]),
    ("pick_station", [10, 1, 1, 0]),
    ("placed_station", [26, 4, 2, 3]),
    ("rack_bay", [228, 20, 6, 11]),
    ("vtm_cohort", [19, 1, 1, 2]),
];

/// Every lens, on every document, projects the table.
#[test]
fn the_engine_projects_the_lens_table() {
    let library = library();
    assert_eq!(library.lenses().count(), LENSES.len(), "the table covers every lens the library ships");
    for (column, Fixture { name, world, .. }) in documents().into_iter().enumerate() {
        let matcher = Matcher::new(&world.entities);
        for (lens, wanted) in LENSES {
            let registered = library.lens(lens).unwrap_or_else(|| panic!("`{lens}` is in the library"));
            let objects = matcher.project(registered);
            assert_eq!(objects.len(), wanted[column], "{name}: {lens}");
        }
    }
}

/// A lens is a fold and not a filter: the objects of a lens are the distinct
/// entities its anchor variable binds, so an object count is at most the row
/// count of the anchor pattern and is smaller wherever a row repeats an entity.
///
/// cencora is where the two come apart: `vtm_cohort` answers 38 rows over 19
/// cohorts, because each cohort's sector names two runs. Saying both numbers
/// here keeps one from being read as the other -- and the pattern and the lens
/// are asked for under the one name `vtm_cohort`, which is what `get` and
/// `lens` being two lookups buys.
#[test]
fn an_object_count_is_a_count_of_things_not_of_rows() {
    let library = library();
    let Fixture { world, .. } = document("cencora-v3");
    let matcher = Matcher::new(&world.entities);
    let rows = matcher.matches(library.get("vtm_cohort").expect("the pattern")).len();
    let objects = matcher.project(library.lens("vtm_cohort").expect("the lens")).len();
    assert_eq!((rows, objects), (38, 19));

    // And every field of every shipped lens holds entities of the document it
    // was projected from, which is what says a projection invents nothing.
    for lens in library.lenses() {
        for object in matcher.project(lens) {
            assert!(world.entities.contains_key(&object.anchor.id), "{}", lens.name());
            for (field, entities) in object.fields() {
                for view in entities {
                    assert!(world.entities.contains_key(&view.id), "{}: {field}", lens.name());
                }
            }
        }
    }
}
