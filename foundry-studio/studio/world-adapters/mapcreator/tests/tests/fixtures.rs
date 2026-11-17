//! What the Map Creator adapter must keep being true of, shown on synthetic
//! exports whose every value is invented, through the built component, under
//! the default budget.
//!
//! The four exports under `fixtures/`: `current`, a small site in the current
//! dialect with all nineteen files and every record kind that dialect has;
//! `current-edited`, the same site edited as Map Creator writes an edit: one
//! cell removed, its four neighbours no longer naming it, one cell added, one
//! station field changed, a zone no cell names renamed and a zone a cell names
//! renamed; `older`, a small site in the older dialect's eight files; and
//! `tenths`, a site measured in tenths of a millimetre, with a station and its
//! presentation point. Between them they write every component the import
//! writes.

use jsonschema::Validator;
use mapcreator_adapter_tests::{
    annotated_references, by_op, compiled, file_json, fixture, ingested, keyed, loaded, map_values, mapcreator,
    millimetre_pair, module_bytes, naming, normalized, seed_refs, seed_schemas, spell_millimetres, unresolved_fields,
    vocabulary, with_file, with_map_values, CORE, EXTENDED,
};
use serde_json::{json, Map, Value};
use site_model::{Coord, EntityId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use worldadapt_host::{AdapterError, Budget, Change, Engine, Entity, File, ImportPolicy, Module, RandomIds, Site};
use worldadapt_testkit::{apply, assert_round_trip, round_trip, Deleted, FixedIds};

/// The site's own scope, and a scope it pins.
const OWN: &str = "tenant/site";
const PINNED: &str = "module/catalog";

/// The four exports.
const FIXTURES: [&str; 4] = ["current", "current-edited", "older", "tenths"];

fn empty() -> Site {
    Site { scope: OWN.into(), entities: Vec::new() }
}

fn ingest(module: &Module, files: &[File], site: &Site) -> Result<Vec<Change>, AdapterError> {
    module.ingest(Budget::default(), files, site, Box::new(RandomIds))
}

/// The site one fixture's first ingest into an empty site leaves.
fn site_of(module: &Module, name: &str) -> Site {
    ingested(module, &fixture(name), Budget::default(), OWN)
}

/// The adapter's own sentence refusing a call.
fn refusal<T: Debug>(result: Result<T, AdapterError>) -> String {
    match result {
        Err(AdapterError::Refused { message, .. }) => message,
        other => panic!("expected the adapter's own refusal, got {other:?}"),
    }
}

fn entity<'s>(site: &'s Site, id: &str) -> &'s Entity {
    site.entities.iter().find(|entity| entity.id == id).unwrap_or_else(|| panic!("no entity {id}"))
}

fn entity_mut<'s>(site: &'s mut Site, id: &str) -> &'s mut Entity {
    site.entities.iter_mut().find(|entity| entity.id == id).unwrap_or_else(|| panic!("no entity {id}"))
}

fn component_mut<'s>(site: &'s mut Site, id: &str, name: &str) -> &'s mut Value {
    entity_mut(site, id).components.get_mut(name).unwrap_or_else(|| panic!("{id} carries no {name}"))
}

/// Whether two sets of files are one export under the laws' normalization.
fn same_files(given: &[File], back: &[File]) -> bool {
    let spelled = |files: &[File]| -> BTreeMap<String, Vec<u8>> {
        files.iter().map(|file| (file.name.clone(), normalized(&file.name, &file.bytes))).collect()
    };
    spelled(given) == spelled(back)
}

/// The declares an ingest proposes, by the components each writes.
fn declared(changes: &[Change]) -> Vec<&Map<String, Value>> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Declare(write) => Some(&write.components),
            _ => None,
        })
        .collect()
}

/// **Byte for byte**: every fixture's files come back as the very bytes they
/// were given — key order, indentation, the newline after the value and a
/// whole number written `125.0` included, which the laws' normalization
/// forgives — after a first ingest into an empty site, and after a second
/// ingest of the same files into the site the first left.
#[test]
fn every_fixture_comes_back_byte_for_byte() {
    let module = mapcreator();
    for name in FIXTURES {
        assert_round_trip(&module, &[fixture(name), fixture(name)], &empty(), None, None);
    }
}

/// Both laws, from an empty site, on every fixture: the current site and then
/// the same site edited, the older dialect, and the site measured in tenths.
#[test]
fn both_laws_hold_from_an_empty_site_on_every_fixture() {
    let module = mapcreator();
    assert_round_trip(&module, &[fixture("current"), fixture("current-edited")], &empty(), None, Some(&normalized));
    assert_round_trip(&module, &[fixture("older")], &empty(), None, Some(&normalized));
    assert_round_trip(&module, &[fixture("tenths")], &empty(), None, Some(&normalized));
}

/// Both laws, from a site that already holds what the adapter must leave
/// alone, and every one of those entities comes through exactly as it was:
/// another system's frame; a lookalike cell with every component a cell has
/// and none of the adapter's keys, which is never written out; an entity
/// another system knows by the barcode of one of the export's cells; and a
/// pinned scope's template.
#[test]
fn both_laws_hold_from_a_site_that_holds_entities_the_adapter_does_not_own() {
    let module = mapcreator();
    let held = |id: &str, scope: &str, components: Value| Entity {
        id: id.into(),
        name: Some(format!("held {id}")),
        scope: scope.into(),
        derived: false,
        components: components.as_object().expect("an object").clone(),
    };
    let elsewhere = "d0000000-0000-4000-8000-000000000001";
    let neighbourhood = vec![
        held(elsewhere, OWN, json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } })),
        held(
            "d0000000-0000-4000-8000-000000000002",
            OWN,
            json!({
                "cell": { "traversable_by": [], "size_info": [625, 625, 625, 625] },
                "pose": { "frame": elsewhere, "position": [28750, 6250] },
                "import_formatting": { "mapcreator_cell": { "index": 0 } },
                "external_ids": { "wms_location": "W-17" }
            }),
        ),
        held("d0000000-0000-4000-8000-000000000003", OWN, json!({ "external_ids": { "barcode": "611.117" } })),
        held("e0000000-0000-4000-8000-000000000004", PINNED, json!({ "template": { "entities": [] } })),
    ];
    let site = Site { scope: OWN.into(), entities: neighbourhood.clone() };
    let exports = [fixture("current"), fixture("current-edited")];
    let trips = assert_round_trip(&module, &exports, &site, None, Some(&normalized));
    for before in &neighbourhood {
        assert_eq!(entity(&trips[1].site, &before.id), before, "{} was touched", before.id);
    }
}

/// The edited export, ingested into the site the current one left, proposes
/// exactly what its edits make, edits Map Creator itself writes: the removed
/// cell and its stitches deleted, its sector rewritten, and its four
/// neighbours, which no longer name it, one patch each to `cell`;
/// the added cell one declare; the changed station one patch to
/// `mapcreator.station`; the renamed zone no cell names, one delete and one
/// declare; and the renamed zone a cell names, one delete, one declare, and
/// that cell re-pointed at the new zone in its patch. Nothing else of the
/// floor is rewritten. The export entity is patched too: the added cell states
/// a set of keys no cell of the file stated before and the removed one took a
/// set no other cell states, so its table of the order the file usually gives
/// each set of keys changes.
#[test]
fn the_edited_export_proposes_exactly_the_changes_its_edits_make() {
    let module = mapcreator();
    let site = site_of(&module, "current");
    let changes = ingest(&module, &fixture("current-edited"), &site).expect("an ingest");

    let gone = keyed(&site, "mapcreator_grid_coordinate", "[3,2]").id.clone();
    let named_by = naming(&site, &gone);
    let stitches: BTreeSet<String> =
        named_by.iter().filter(|id| entity(&site, id).components.contains_key("stitch")).cloned().collect();
    assert_eq!(stitches.len(), 3, "the removed cell's stitches: {stitches:?}");
    let renamed = keyed(&site, "mapcreator_zone_id", "Spare Bay").id.clone();
    let gate = keyed(&site, "mapcreator_zone_id", "Gate East ").id.clone();
    let station = keyed(&site, "mapcreator_pps_id", "31").id.clone();
    let sector = keyed(&site, "mapcreator_sector", "1").id.clone();
    let export = keyed(&site, "mapcreator_export", "site").id.clone();
    assert_eq!(named_by.difference(&stitches).cloned().collect::<BTreeSet<_>>(), BTreeSet::from([sector.clone()]));
    let neighbours: Vec<String> = ["[3,1]", "[2,2]", "[4,2]", "[3,3]"]
        .iter()
        .map(|coordinate| keyed(&site, "mapcreator_grid_coordinate", coordinate).id.clone())
        .collect();

    let deleted: BTreeSet<&str> = by_op(&changes, "delete");
    let expected: BTreeSet<&str> =
        stitches.iter().map(String::as_str).chain([gone.as_str(), renamed.as_str(), gate.as_str()]).collect();
    assert_eq!(deleted, expected);
    let mut patched: Vec<(&str, Vec<&String>)> = changes
        .iter()
        .filter_map(|change| match change {
            Change::Patch(write) => Some((write.id.as_str(), write.components.keys().collect())),
            _ => None,
        })
        .collect();
    patched.sort();
    let mut expected = vec![
        (station.as_str(), vec!["mapcreator.station"]),
        (sector.as_str(), vec!["sector"]),
        (export.as_str(), vec!["mapcreator.export"]),
    ];
    expected.extend(neighbours.iter().map(|cell| (cell.as_str(), vec!["cell"])));
    expected.sort();
    assert_eq!(
        patched
            .iter()
            .map(|(id, names)| (*id, names.iter().map(|n| n.as_str()).collect::<Vec<_>>()))
            .collect::<Vec<_>>(),
        expected
    );
    let declares = declared(&changes);
    assert_eq!(declares.len(), 3, "{declares:?}");
    assert!(declares.iter().any(|c| c["external_ids"]["mapcreator_grid_coordinate"] == json!("[999,4]")));
    assert!(declares.iter().any(|c| c["external_ids"]["mapcreator_zone_id"] == json!("Spare Bay West")));
    let new_gate = changes
        .iter()
        .find_map(|change| match change {
            Change::Declare(write) if write.components["external_ids"]["mapcreator_zone_id"] == json!("Gate East") => {
                Some(write.id.clone())
            }
            _ => None,
        })
        .expect("the renamed zone is declared");
    let repointed = changes
        .iter()
        .find_map(|change| match change {
            Change::Patch(write) if write.id == neighbours[3] => Some(&write.components["cell"]["zone"]),
            _ => None,
        })
        .expect("the cell naming the renamed zone is patched");
    assert_eq!(repointed, &json!(new_gate));
}

/// Every reference a fixture keeps unresolved is one it names nothing with on
/// purpose, and no other: a reference that stopped resolving would otherwise
/// pass both laws unseen, since its legacy value comes back as written.
#[test]
fn every_unresolved_reference_is_one_the_fixture_makes_on_purpose() {
    let module = mapcreator();
    let current = [
        "mapcreator_aisle_id=A2 aisle.io_point_coordinates_unresolved",
        "mapcreator_block_id=10 block.coordinates_unresolved",
        "mapcreator_charger_id=6 charger.entry_cell_unresolved",
        "mapcreator_ods_tuple=000.042--1 ods_exclusion.cell_barcode_unresolved",
        "mapcreator_of_conveyor=3 mapcreator_step_id=C3-S3 conveyor_mapping.cell_unresolved",
        "mapcreator_of_conveyor=3 mapcreator_step_id=C3-S3 conveyor_mapping.station_unresolved",
        "mapcreator_pps_id=32 mapcreator.station.served_by_unresolved",
        "mapcreator_preferred_for=rack-A sector_preference.prefers_unresolved",
        "mapcreator_tote_location=TL-0004 tote_storage.io_point.cell_unresolved",
    ];
    for (name, expected) in
        [("current", &current[..]), ("current-edited", &current[..]), ("older", &[]), ("tenths", &[])]
    {
        assert_eq!(unresolved_fields(&site_of(&module, name)), expected, "{name}");
    }
}

/// A reference lands on the entity of the record it names: a charger's three
/// cells, a storage position's io-point and relay cells, a mapping row's
/// conveyor and station.
#[test]
fn a_named_reference_lands_on_the_entity_it_names() {
    let module = mapcreator();
    let site = site_of(&module, "current");
    let cell = |coordinate: &str| json!(keyed(&site, "mapcreator_grid_coordinate", coordinate).id);
    let charger = &keyed(&site, "mapcreator_charger_id", "5").components["charger"];
    assert_eq!(
        (&charger["at_cell"], &charger["entry_cell"], &charger["reinit_cell"]),
        (&cell("[1,3]"), &cell("[1,2]"), &cell("[500,500]"))
    );
    let storage = &keyed(&site, "mapcreator_tote_location", "TL-0002").components["tote_storage"];
    assert_eq!((&storage["io_point"]["cell"], &storage["relay_cell"]), (&cell("[1,1]"), &cell("[1,2]")));
    let row = site
        .entities
        .iter()
        .find(|e| {
            e.components.get("external_ids").and_then(|ids| ids.get("mapcreator_step_id")) == Some(&json!("C3-S2"))
        })
        .expect("a mapping row");
    let mapping = &row.components["conveyor_mapping"];
    assert_eq!(mapping["conveyor_coordinates"]["conveyor"], json!(keyed(&site, "mapcreator_conveyor_id", "3").id));
    assert_eq!(mapping["station"], json!(keyed(&site, "mapcreator_pps_id", "31").id));
    assert_eq!(mapping["cell"], cell("[4,1]"));
}

/// The same files ingested into the site they made ask for nothing, so no
/// record ever becomes a second entity.
#[test]
fn the_same_files_into_the_same_site_never_make_a_second_entity() {
    let module = mapcreator();
    for name in FIXTURES {
        let site = site_of(&module, name);
        let again = ingest(&module, &fixture(name), &site).expect("an ingest");
        assert!(again.is_empty(), "{name}: {again:?}");
        assert_eq!(apply(&site, &again, &mut Deleted::default()).expect("nothing to apply"), site);
    }
}

/// A queue block's core queue states its path and nothing beside it: since
/// 2026-09-02 a queue's path is the run it admits on, and SPEC §10 gives it no
/// depth to state the same thing a second time.
#[test]
fn a_queue_states_its_path_alone() {
    let module = mapcreator();
    let site = site_of(&module, "current");
    let queues: Vec<&Value> = site.entities.iter().filter_map(|entity| entity.components.get("queue")).collect();
    assert!(!queues.is_empty(), "the fixture holds no queue block, so the test proves nothing");
    for queue in queues {
        let fields: Vec<&str> = queue.as_object().expect("a queue is an object").keys().map(String::as_str).collect();
        assert_eq!(fields, ["path"], "{queue}");
    }
}

/// Two sites get different identities for the same records: nothing about a
/// record decides what its entity is called, only the host's draw does.
#[test]
fn two_sites_get_different_identities_for_the_same_records() {
    let module = mapcreator();
    let declare = || -> BTreeSet<String> {
        ingest(&module, &fixture("current"), &empty()).expect("an ingest").iter().map(|c| c.id().to_string()).collect()
    };
    let (first, second) = (declare(), declare());
    assert_eq!(first.len(), second.len());
    assert!(first.is_disjoint(&second));
}

/// A number spelled two ways is one key: files spelling a station's id `31.0`
/// and a charger's `5.0` find the entities the site holds under `31` and `5`,
/// declare and delete nothing, and come back byte for byte. What they change is
/// how the two ids are written, which the station's and the charger's own
/// formatting record, and nothing else.
#[test]
fn a_key_number_spelled_two_ways_is_one_key() {
    let module = mapcreator();
    let current = fixture("current");
    let site = site_of(&module, "current");
    let respelled = |files: &[File], name: &str, from: &str, to: &str| {
        with_text(files, name, |text| {
            assert!(text.contains(from), "{name} has no {from:?}");
            text.replacen(from, to, 1)
        })
    };
    let respelled = respelled(
        &respelled(&current, "pps.json", "\"pps_id\": 31,", "\"pps_id\": 31.0,"),
        "charger.json",
        "\"charger_id\": 5,",
        "\"charger_id\": 5.0,",
    );
    let changes = ingest(&module, &respelled, &site).expect("an ingest");
    let patched = by_op(&changes, "patch");
    assert_eq!(patched.len(), changes.len(), "every change is a patch: {changes:?}");
    let written = ["mapcreator_pps_id", "mapcreator_charger_id"]
        .into_iter()
        .zip(["31", "5"])
        .map(|(key, value)| keyed(&site, key, value).id.as_str())
        .collect::<BTreeSet<&str>>();
    assert_eq!(patched, written);
    assert_round_trip(&module, &[current, respelled], &empty(), None, None);
}

/// A conveyor-mapping row is keyed by its conveyor and its step as two values,
/// so two rows whose halves would join into one string are two entities.
#[test]
fn two_mapping_rows_whose_halves_join_alike_are_two_entities() {
    let module = mapcreator();
    let current = fixture("current");
    let mut rows = file_json(&current, "conveyor_mapping.json");
    for (conveyor, step, cell) in [("3", "C3/S9", [1, 1]), ("3/C3", "S9", [2, 1])] {
        rows.as_array_mut().expect("a list").push(json!({
            "go_coordinates": cell, "HAI_coordinates": "HC-39", "location_type": "conveyor_track",
            "conveyor_coordinates": { "conveyor_id": conveyor, "step_id": step }
        }));
    }
    let edited = with_file(&current, "conveyor_mapping.json", &rows);
    let trips = assert_round_trip(&module, &[edited], &empty(), None, Some(&normalized));
    let rows = trips[0].site.entities.iter().filter(|e| e.components.contains_key("conveyor_mapping"));
    assert_eq!(rows.count(), 5);
}

/// Every new entity takes an identity the host issued: handed a list the test
/// wrote down, the declares carry exactly that list, each identity once.
#[test]
fn every_new_identity_is_one_the_host_issued() {
    let module = mapcreator();
    let count = ingest(&module, &fixture("tenths"), &empty()).expect("an ingest").len();
    let listed: Vec<String> = (1..=count).map(|n| format!("f0000000-0000-4000-8000-{n:012}")).collect();
    let ids = FixedIds::parse(&listed.iter().map(String::as_str).collect::<Vec<_>>());
    let changes =
        module.ingest(Budget::default(), &fixture("tenths"), &empty(), Box::new(ids.clone())).expect("an ingest");
    let carried: BTreeSet<&str> = changes.iter().map(Change::id).collect();
    assert_eq!(carried, listed.iter().map(String::as_str).collect::<BTreeSet<_>>());
    assert_eq!(ids.remaining(), 0);
}

/// An entity of a pinned scope that carries the adapter's key is written out
/// like any other, and files that say the same of it ask for nothing; files
/// that would change it, or hold no record for it, are refused, naming the
/// key, the entity and the scope.
#[test]
fn a_pinned_entity_is_read_and_a_change_to_it_is_refused_naming_key_entity_and_scope() {
    let module = mapcreator();
    let current = fixture("current");
    let mut site = site_of(&module, "current");
    let zone = keyed(&site, "mapcreator_zone_id", "North Hall").id.clone();
    entity_mut(&mut site, &zone).scope = PINNED.into();
    assert!(same_files(&current, &module.project(Budget::default(), &site).expect("a projection")));
    assert!(ingest(&module, &current, &site).expect("an ingest").is_empty());

    let mut zones = file_json(&current, "zone.json");
    zones[0]["zonerec"]["paused"] = json!(true);
    let changed = refusal(ingest(&module, &with_file(&current, "zone.json", &zones), &site));
    zones.as_array_mut().expect("a list").remove(0);
    let gone = refusal(ingest(&module, &with_file(&current, "zone.json", &zones), &site));
    for (refused, says) in [(changed, "would change entity"), (gone, "hold no record for entity")] {
        for named in [says, zone.as_str(), "the zone North Hall", PINNED] {
            assert!(refused.contains(named), "{named:?} in {refused}");
        }
    }

    // A change of the site's own that writes a reference to the pinned zone is
    // refused: the server takes a reference into a pinned scope only with the
    // pin it is read at, and the adapter writes a bare identity.
    let mut cells = map_values(&current);
    cells[0]["store_status"] = json!(1);
    let refused = refusal(ingest(&module, &with_map_values(&current, cells), &site));
    let says = format!("(the cell 1 [1,1]) name entity {zone} (the zone North Hall) at `cell.zone`");
    for named in [says.as_str(), PINNED, "the server would refuse the change"] {
        assert!(refused.contains(named), "{named:?} in {refused}");
    }
}

/// An entity that carries the adapter's key and lacks what its kind is
/// written back from is refused by both directions, naming it: a cell without
/// its pose, a station without the dialect it is written in, a frame without
/// its role, a queue block without its queue, a presentation point without
/// the core component its purpose is on, and an entity carrying two kinds'
/// keys.
#[test]
fn an_entity_that_cannot_be_read_back_is_refused_by_both_directions_naming_it() {
    let module = mapcreator();
    let current = fixture("current");
    let site = site_of(&module, "current");
    let cell = keyed(&site, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    let station = keyed(&site, "mapcreator_pps_id", "31").id.clone();
    let frame = keyed(&site, "mapcreator_frame", "legacy-grid").id.clone();
    let zone = keyed(&site, "mapcreator_zone_id", "North Hall").id.clone();
    let queue = keyed(&site, "mapcreator_block_id", "7").id.clone();
    let point = site
        .entities
        .iter()
        .find(|entity| entity.components["external_ids"].get("mapcreator_point_id").is_some())
        .expect("a presentation point")
        .id
        .clone();
    let broken = |edit: &dyn Fn(&mut Site)| {
        let mut site = site.clone();
        edit(&mut site);
        site
    };
    for (site, named, says) in [
        (broken(&|s| drop(entity_mut(s, &cell).components.remove("pose"))), &cell, "has no `pose` component"),
        (
            broken(&|s| {
                let formatting = &mut component_mut(s, &station, "import_formatting")["mapcreator_station"];
                drop(formatting.as_object_mut().unwrap().remove("dialect_shape"))
            }),
            &station,
            "no `import_formatting.mapcreator_station.dialect_shape`",
        ),
        (
            broken(&|s| drop(component_mut(s, &frame, "mapcreator.frame").as_object_mut().unwrap().remove("role"))),
            &frame,
            "no `mapcreator.frame.role`",
        ),
        (broken(&|s| drop(entity_mut(s, &queue).components.remove("queue"))), &queue, "has no `queue` component"),
        (
            broken(&|s| drop(entity_mut(s, &point).components.remove("presentation_point"))),
            &point,
            "has no `presentation_point` component",
        ),
        (
            broken(&|s| component_mut(s, &zone, "external_ids")["mapcreator_block_id"] = json!("77")),
            &zone,
            "of a zone and of a block",
        ),
    ] {
        let projected = refusal(module.project(Budget::default(), &site));
        let ingested = refusal(ingest(&module, &current, &site));
        for refused in [projected, ingested] {
            assert!(refused.contains(named.as_str()) && refused.contains(says), "{says:?}: {refused}");
        }
    }
}

/// Everything the table says an entity is written back from beyond what the
/// test above takes off, taken off one entity of a synthetic site, is refused
/// by both directions naming the entity and what it lacks: a cell's `blocked`
/// and barcode, a station's `pps_id`, a point's station, a light's group, a
/// preference's sectors, a mapping row's coordinates and its conveyor, an
/// exclusion's heading and cell, a bin's and a seat's station, the site
/// frame's step, and every record's ordering key. Without each, the projection
/// would drop the record, write it short or out of its order, or write one the
/// ingest refuses. A site with no site frame at all is not written out.
#[test]
fn every_field_an_entity_is_written_back_from_is_refused_when_it_is_gone() {
    let module = mapcreator();
    let sites: Vec<(&str, Site)> = ["current", "older", "tenths"].map(|name| (name, site_of(&module, name))).into();
    let mut cases: Vec<(String, Vec<String>, String)> = [
        ("cell", &["blocked"][..], "no `cell.blocked`"),
        ("external_ids", &["barcode"], "no `external_ids.barcode`"),
        ("import_formatting", &["mapcreator_station.pps_id"], "no `import_formatting.mapcreator_station.pps_id`"),
        ("presentation_point", &["station"], "no `presentation_point.station`"),
        ("station_point", &["station"], "no `station_point.station`"),
        ("import_formatting", &["mapcreator_pptl.group"], "no `import_formatting.mapcreator_pptl.group`"),
        ("sector_preference", &["prefers"], "no `sector_preference.prefers`"),
        ("conveyor_mapping", &["conveyor_coordinates"], "no `conveyor_mapping.conveyor_coordinates`"),
        (
            "conveyor_mapping",
            &["conveyor_coordinates.conveyor", "conveyor_coordinates.conveyor_unresolved"],
            "neither `conveyor_mapping.conveyor_coordinates.conveyor` nor",
        ),
        ("ods_exclusion", &["heading"], "no `ods_exclusion.heading`"),
        ("ods_exclusion", &["cell", "cell_barcode_unresolved"], "neither `ods_exclusion.cell` nor"),
        ("pps_bin", &["station", "station_unresolved"], "neither `pps_bin.station` nor"),
        ("pps_seat", &["station", "station_unresolved"], "neither `pps_seat.station` nor"),
        ("frame", &["unit_mm"], "no `frame.unit_mm`"),
    ]
    .into_iter()
    .map(|(component, paths, says)| {
        (component.to_string(), paths.iter().map(|path| path.to_string()).collect(), says.to_string())
    })
    .collect();
    let formatting = &vocabulary()["import_formatting"]["schema"]["properties"];
    for (key, entry) in formatting.as_object().expect("the formatting's entries") {
        if entry["required"].as_array().is_some_and(|required| required.contains(&json!("index"))) {
            let says = format!("no `import_formatting.{key}.index`");
            cases.push(("import_formatting".to_string(), vec![format!("{key}.index")], says));
        }
    }
    assert_eq!(cases.len(), 14 + 19, "every record kind's ordering key is among the cases");
    for (component, paths, says) in cases {
        // The first entity of a fixture whose component carries the first field.
        let top = paths[0].split('.').next().expect("a field").to_string();
        let carries = |entity: &Entity| entity.components.get(&component).and_then(|c| c.get(&top)).is_some();
        let (name, site) = sites
            .iter()
            .find(|(_, site)| site.entities.iter().any(carries))
            .unwrap_or_else(|| panic!("no fixture carries `{component}.{top}`, so this proves nothing"));
        let mut broken = site.clone();
        let entity = broken.entities.iter_mut().find(|entity| carries(entity)).expect("the entity found above");
        let id = entity.id.clone();
        for path in &paths {
            let (outer, last) =
                path.rsplit_once('.').map_or((None, path.as_str()), |(outer, last)| (Some(outer), last));
            let payload = entity.components.get_mut(&component).expect("the component");
            let holder = match outer {
                Some(outer) => payload.pointer_mut(&format!("/{}", outer.replace('.', "/"))),
                None => Some(payload),
            };
            holder.and_then(Value::as_object_mut).map(|object| object.remove(last));
        }
        let projected = refusal(module.project(Budget::default(), &broken));
        let ingested = refusal(ingest(&module, &fixture(name), &broken));
        for refused in [projected, ingested] {
            assert!(refused.contains(&id) && refused.contains(&says), "{component} {paths:?}: {refused}");
        }
    }

    let mut frameless = site_of(&module, "current");
    let site_frame = keyed(&frameless, "mapcreator_frame", "site").id.clone();
    frameless.entities.retain(|entity| entity.id != site_frame);
    let refused = refusal(module.project(Budget::default(), &frameless));
    assert!(refused.starts_with("this site holds no Map Creator site frame stating `frame.unit_mm`"), "{refused}");
}

/// A note somebody added to a component never causes a change by itself and
/// never reaches a file; a note on a component the adapter does rewrite is
/// replaced with the rest of it.
#[test]
fn an_authors_note_never_causes_a_change_and_never_reaches_a_file() {
    let module = mapcreator();
    let current = fixture("current");
    let mut site = site_of(&module, "current");
    let cell = keyed(&site, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    component_mut(&mut site, &cell, "cell")["$comment"] = json!("resurveyed in spring");
    component_mut(&mut site, &cell, "external_ids")["$comment"] = json!("keys from the survey");
    assert!(ingest(&module, &current, &site).expect("an ingest").is_empty());
    let files = module.project(Budget::default(), &site).expect("a projection");
    assert!(same_files(&current, &files));
    assert!(files.iter().all(|file| !String::from_utf8_lossy(&file.bytes).contains("$comment")));

    let station = keyed(&site, "mapcreator_pps_id", "31").id.clone();
    component_mut(&mut site, &station, "mapcreator.station")["$comment"] = json!("checked on the floor");
    let changes = ingest(&module, &fixture("current-edited"), &site).expect("an ingest");
    let rewritten = changes
        .iter()
        .find_map(|change| match change {
            Change::Patch(write) if write.id == station => Some(&write.components["mapcreator.station"]),
            _ => None,
        })
        .expect("the station is rewritten");
    assert!(rewritten.get("$comment").is_none(), "{rewritten}");
}

/// A component the files would stop giving an entity is refused, naming the
/// entity and the component: no change can remove a component. A component
/// the import gives some shapes of record only, which another tool put on a
/// record of another shape, is left alone.
#[test]
fn a_component_the_files_stop_giving_an_entity_is_refused_naming_both() {
    let module = mapcreator();
    let current = fixture("current");
    let mut site = site_of(&module, "current");
    let station = keyed(&site, "mapcreator_pps_id", "31").id.clone();
    let mut stations = file_json(&current, "pps.json");
    stations[0]["location"] = json!([9, 9]);
    let refused = refusal(ingest(&module, &with_file(&current, "pps.json", &stations), &site));
    assert!(refused.contains(&station) && refused.contains("`pose` component"), "{refused}");

    let several = keyed(&site, "mapcreator_pps_id", "32").id.clone();
    entity_mut(&mut site, &several).components.insert("station".into(), json!({ "kind": "put" }));
    assert!(ingest(&module, &current, &site).expect("an ingest").is_empty());
}

/// A component the import writes for one shape of record only, which the
/// record's own component shows this adapter wrote for the shape the record
/// had, is refused when the files give the record a shape that gets none:
/// left on, it would say a waiting block is still a queue, or a station of two
/// modes still a pick station. The refusal names the entity, the component and
/// the two shapes.
#[test]
fn a_component_written_for_a_records_earlier_shape_is_refused_when_its_shape_changes() {
    let module = mapcreator();
    let current = fixture("current");
    let site = site_of(&module, "current");
    let block = keyed(&site, "mapcreator_block_id", "7").id.clone();
    let waiting = with_edit(&current, "block.json", |rows| rows[0]["entity_type"] = json!("waiting"));
    let refused = refusal(ingest(&module, &waiting, &site));
    for says in [block.as_str(), "`queue` component", "`entity_type` \"queue\"", "`entity_type` \"waiting\""] {
        assert!(refused.contains(says), "{says:?} in {refused}");
    }

    let station = keyed(&site, "mapcreator_pps_id", "31").id.clone();
    let both = with_edit(&current, "pps.json", |rows| rows[0]["allowed_modes"] = json!(["pick", "put"]));
    let refused = refusal(ingest(&module, &both, &site));
    for says in [
        station.as_str(),
        "`station` component",
        "`type` \"manual\" with `allowed_modes` [\"pick\"]",
        "`type` \"manual\" with `allowed_modes` [\"pick\",\"put\"]",
    ] {
        assert!(refused.contains(says), "{says:?} in {refused}");
    }
}

/// An entity the files no longer hold a record for, that another system still
/// knows by a key of its own, is refused rather than deleted, naming the
/// entity and the key.
#[test]
fn an_entity_another_system_still_knows_is_refused_rather_than_deleted() {
    let module = mapcreator();
    let mut site = site_of(&module, "current");
    let renamed = keyed(&site, "mapcreator_zone_id", "Spare Bay").id.clone();
    component_mut(&mut site, &renamed, "external_ids")["wms_zone"] = json!("W-9");
    let refused = refusal(ingest(&module, &fixture("current-edited"), &site));
    assert!(refused.contains(&renamed) && refused.contains("`wms_zone`"), "{refused}");
}

/// A key path no disposition covers is refused, naming it with its count; a
/// key inside a record that no field map row disposes of is refused naming it.
#[test]
fn a_field_family_nobody_has_disposed_of_is_refused_naming_it() {
    let module = mapcreator();
    let current = fixture("current");
    let mut map = file_json(&current, "map.json");
    map[0]["gantry_calibration"] = json!({ "offset": 3 });
    let refused = refusal(ingest(&module, &with_file(&current, "map.json", &map), &empty()));
    assert!(refused.starts_with("the export carries 2 key path(s) that no disposition covers"), "{refused}");
    assert!(refused.contains("map.json:[].gantry_calibration (once)"), "{refused}");

    // A key spelling a covered path is a path of its own, on a wrapper as on
    // the floor, and not the covered one it spells.
    let mut map = file_json(&current, "map.json");
    map[0]["map_values[].barcode"] = json!("discarded");
    let refused = refusal(ingest(&module, &with_file(&current, "map.json", &map), &empty()));
    assert!(refused.contains("map.json:[].map_values\\[\\]\\.barcode (once)"), "{refused}");
    let mut zones = file_json(&current, "zone.json");
    zones[0]["zonerec.zone_id"] = json!("discarded");
    let refused = refusal(ingest(&module, &with_file(&current, "zone.json", &zones), &empty()));
    assert!(refused.contains("zone.json:[].zonerec\\.zone_id (once)"), "{refused}");

    let mut chargers = file_json(&current, "charger.json");
    chargers[0]["gantry_mode"] = json!(1);
    let refused = refusal(ingest(&module, &with_file(&current, "charger.json", &chargers), &empty()));
    assert!(refused.contains("a record carries `gantry_mode`"), "{refused}");
}

/// The same export with one file's text rewritten, for a shape no JSON value
/// can hold, such as a key named twice.
fn with_text(files: &[File], name: &str, edit: impl Fn(&str) -> String) -> Vec<File> {
    let mut files = files.to_vec();
    let file = files.iter_mut().find(|file| file.name == name).unwrap_or_else(|| panic!("no {name}"));
    file.bytes = edit(&String::from_utf8_lossy(&file.bytes)).into_bytes();
    files
}

/// The same export with one file's JSON changed by `edit`.
fn with_edit(files: &[File], name: &str, edit: impl Fn(&mut Value)) -> Vec<File> {
    let mut value = file_json(files, name);
    edit(&mut value);
    with_file(files, name, &value)
}

/// The same export with one more file, written as Map Creator writes one.
fn with_added(files: &[File], name: &str, value: &Value) -> Vec<File> {
    let mut files = files.to_vec();
    files.push(File { name: name.into(), bytes: serde_json::to_vec(value).expect("JSON serializes") });
    files
}

/// Every export shape the adapter cannot give back exactly, each made from
/// the `current` export, with the words its refusal must say: which file,
/// which record, which rule.
fn refused_shapes() -> Vec<(&'static str, Vec<File>, Vec<&'static str>)> {
    let current = fixture("current");
    let older = fixture("older");
    let tenths = fixture("tenths");
    let at_first_cell = |world: &str| {
        let mut cells = map_values(&current);
        cells[0]["world_coordinate"] = json!(world);
        with_map_values(&current, cells)
    };
    let at_first_tenths_cell = |world: &str| {
        let mut cells = map_values(&tenths);
        cells[0]["world_coordinate"] = json!(world);
        with_map_values(&tenths, cells)
    };
    let with_barcode = |files: &[File], at: usize, barcode: &str| {
        let mut cells = map_values(files);
        cells[at]["barcode"] = json!(barcode);
        with_map_values(files, cells)
    };
    let with_size_info = |size_info: Value| {
        let mut cells = map_values(&current);
        cells[0]["size_info"] = size_info;
        with_map_values(&current, cells)
    };
    let mut shapes = vec![
        (
            "a charger naming a barcode two cells carry",
            with_barcode(&current, 11, "613.317"),
            vec!["charger.json record 1 names the barcode 613.317 in `charger_location`, which 2 cells carry, [1,3] and [4,3]"],
        ),
        (
            "a station of the older dialect naming a barcode two cells carry",
            with_barcode(&older, 1, "732.321"),
            vec!["pps.json record 2 names the barcode 732.321 in `location`, which 2 cells carry, [2,1] and [3,2]"],
        ),
        (
            "a vendor sample point naming a barcode two cells carry",
            with_barcode(&current, 1, "611.117"),
            vec!["map.json's `vda5050_reference` names the barcode 611.117, which 2 cells carry, [1,1] and [2,1]"],
        ),
        (
            "a vendor sample point naming no cell",
            with_edit(&current, "map.json", |map| map[0]["vda5050_reference"]["barcode"] = json!("990.919")),
            vec!["map.json's `vda5050_reference` names the barcode \"990.919\", which no cell carries"],
        ),
        (
            "a vendor sample point disagreeing with the cell it names",
            with_edit(&current, "map.json", |map| map[0]["vda5050_reference"]["vda5050_coordinate"] = json!("[1,1]")),
            vec!["map.json's `vda5050_reference` states", "the export rebuilds the reference from that cell"],
        ),
        (
            "a sector index disagreeing with the cells",
            with_edit(&current, "sectors.json", |index| {
                index[0]["1"].as_array_mut().unwrap().retain(|cell| cell != "[3,2]")
            }),
            vec!["sectors.json lists sector `1` other than its cells do: entry 7 is \"[4,2]\", where the cells give \"[3,2]\""],
        ),
        (
            "the sector index under both its names, the second disagreeing with the cells",
            with_added(&current, "sector.json", &{
                let mut index = file_json(&current, "sectors.json");
                index[0]["1"].as_array_mut().unwrap().retain(|cell| cell != "[3,2]");
                index
            }),
            vec!["sector.json lists sector `1` other than its cells do: entry 7 is \"[4,2]\", where the cells give \"[3,2]\""],
        ),
        (
            "a cell's block disagreeing with block.json",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                cells[1]["block"] = json!(7);
                cells
            }),
            vec!["map.json cell [2,1] states `block` 7, and block.json puts it in no block"],
        ),
        (
            "a cell's dock flag disagreeing with dock_point.json",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                cells[0]["dockPoint"] = json!(true);
                cells
            }),
            vec!["map.json cell [1,1] states `dockPoint` true, and dock_point.json stands no dock point on it"],
        ),
        (
            "a position spelled with a needless fraction",
            at_first_cell("[28750.0,6250]"),
            vec!["map.json cell [1,1] states `world_coordinate` as \"[28750.0,6250]\", which the export would write back as \"[28750,6250]\""],
        ),
        ("a position spelled with a space", at_first_cell("[28750, 6250]"), vec!["would write back as \"[28750,6250]\""]),
        ("a position spelled with a negative zero", at_first_cell("[-0,6250]"), vec!["would write back as \"[0,6250]\""]),
        ("a position spelled with an exponent", at_first_cell("[2.875e4,6250]"), vec!["would write back as \"[28750,6250]\""]),
        (
            "a position that is not a number",
            at_first_cell("[NaN,6250]"),
            vec!["map.json cell [1,1] states `world_coordinate` as \"[NaN,6250]\", and a position is a finite number"],
        ),
        (
            "a position too far to count in tenths",
            at_first_cell("[100000000000000000000,6250]"),
            vec!["a position is a finite number of millimetres no further than"],
        ),
        (
            "a position a ten-millionth of a millimetre off a whole one",
            at_first_cell("[28750.0000001,6250]"),
            vec![
                "1 length(s) of the export land on no step this adapter counts in",
                "the worst is a length of 28750.0000001 mm on map.json cell [1,1], which would be counted as 28750 mm",
            ],
        ),
        (
            "a position one double off a whole millimetre",
            at_first_cell("[28750.000000000004,6250]"),
            vec!["the worst is a length of 28750.000000000004 mm on map.json cell [1,1], which would be counted as 28750 mm"],
        ),
        (
            "a position one double off a tenth, on a site counting tenths",
            at_first_tenths_cell("[2250.8000000000006,1750.6]"),
            vec!["the worst is a length of 2250.8000000000006 mm on map.json cell [1,1], which would be counted as 2250.8 mm"],
        ),
        (
            "a quarter of a millimetre beside a tenth",
            at_first_tenths_cell("[2250.25,1750.6]"),
            vec![
                "the export states a length of 2250.25 mm on map.json cell [1,1], which a step of 0.25 mm states and a \
                 tenth of a millimetre does not, and one of 1750.6 mm on map.json cell [1,1], which a step of 0.25 mm \
                 does not",
            ],
        ),
        (
            "a `size_info` of three half-extents",
            with_size_info(json!([625, 625, 625])),
            vec![
                "map.json cell [1,1] states `size_info` with 3 entries, [625,625,625], where the format keeps four \
                 half-extents: read with a zero for each it lacks, the cell's footprint would be one the export does \
                 not state",
            ],
        ),
        (
            "a `size_info` of five half-extents",
            with_size_info(json!([625, 625, 625, 625, 625])),
            vec!["map.json cell [1,1] states `size_info` with 5 entries", "read as its first four"],
        ),
        (
            "a `size_info` entry that is no number",
            with_size_info(json!([625, "bad", 625, 625])),
            vec![
                "map.json cell [1,1] states `size_info` entry 2 as \"bad\", where the format keeps a number of \
                 millimetres: read as zero",
            ],
        ),
        (
            "a `size_info` entry that is null",
            with_size_info(json!([625, 625, null, 625])),
            vec!["map.json cell [1,1] states `size_info` entry 3 as null"],
        ),
        (
            "a `size_info` that is no list",
            with_size_info(json!({ "north": 625 })),
            vec![
                "map.json cell [1,1] states `size_info` as {\"north\":625}, where the format keeps a list of four \
                 half-extents: read as stating none, the cell could not be told from one whose export states no \
                 `size_info`",
            ],
        ),
        (
            "a `size_info` that is null",
            with_size_info(json!(null)),
            vec!["map.json cell [1,1] states `size_info` as null, where the format keeps a list of four half-extents"],
        ),
        (
            "a footprint's half-extent finer than a tenth",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                cells[0]["size_info"] = json!([625.05, 625, 625, 625]);
                cells
            }),
            vec!["the worst is a length of 625.05 mm on map.json cell [1,1], which would be counted as 625.1 mm"],
        ),
    ];
    shapes.extend(vec![
        (
            "a block with no `entity_type`",
            with_edit(&current, "block.json", |rows| drop(rows[0].as_object_mut().unwrap().remove("entity_type"))),
            vec!["block.json record 1 states no `entity_type`"],
        ),
        (
            "a cell with no `blocked`",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                drop(cells[0].as_object_mut().unwrap().remove("blocked"));
                cells
            }),
            vec!["map.json cell [1,1] states no `blocked`"],
        ),
        (
            "a station `type` that is not a string",
            with_edit(&current, "pps.json", |rows| rows[0]["type"] = json!(5)),
            vec!["pps.json record 1 states `type` as a number (5), where the format fixes a string"],
        ),
        (
            "a presentation point whose `world_coordinates` name another cell than its barcode",
            with_edit(&current, "pps.json", |rows| {
                rows[0]["entities"]["presentation_points"][0]["attributes"]["world_coordinates"] = json!([9, 9])
            }),
            vec![
                "pps.json record 1, presentation point 1 names the barcode \"621.127\", whose cell is [2,1], and \
                 states `world_coordinates` [9,9]: the export rebuilds `world_coordinates` from the cell the barcode \
                 names, so it would come back as [2,1]",
            ],
        ),
        (
            "a presentation point whose `world_coordinates` are spelled as a string",
            with_edit(&current, "pps.json", |rows| {
                rows[0]["entities"]["presentation_points"][0]["attributes"]["world_coordinates"] = json!("[2,1]")
            }),
            vec!["states `world_coordinates` \"[2,1]\": the export rebuilds `world_coordinates`"],
        ),
        (
            "a presentation point whose `presented_for` is not a string",
            with_edit(&current, "pps.json", |rows| {
                rows[0]["entities"]["presentation_points"][0]["presented_for"] = json!(null)
            }),
            vec!["pps.json record 1, presentation point 1 states `presented_for` as null, where the format fixes a string"],
        ),
        (
            "one sector spelled two ways by two cells",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                cells[1]["sector"] = json!("1");
                cells
            }),
            vec!["map.json cell [2,1] states `sector` \"1\", and cell [1,1] states the same sector as 1"],
        ),
        (
            "a sector preference that is not a list",
            with_edit(&current, "sectorMxUPreferences.json", |file| file["rack-A"] = json!(5)),
            vec!["sectorMxUPreferences.json gives carrier type `rack-A` a number (5), where the format keeps a list"],
        ),
        (
            "`sectorMxUPreferences.json` as a list of two objects",
            with_edit(&current, "sectorMxUPreferences.json", |file| *file = json!([file.clone(), {}])),
            vec!["sectorMxUPreferences.json is a list of 2 entries"],
        ),
        (
            "a record key in the other JSON kind from its field's fixed one",
            with_edit(&current, "charger.json", |rows| rows[1]["charger_id"] = json!("6")),
            vec!["charger.json record 2 states `charger_id` as a string (\"6\"), and charger.json fixes it as a number"],
        ),
        (
            "a zone id that is a number",
            with_edit(&current, "zone.json", |rows| rows[0]["zonerec"]["zone_id"] = json!(3)),
            vec!["zone.json record 1 states `zone_id` as a number (3), and zone.json fixes it as a string"],
        ),
        (
            "a block id that is a number",
            with_edit(&current, "block.json", |rows| rows[0]["entity_details"]["block_id"] = json!(7)),
            vec!["block.json record 1 states `block_id` as a number (7)"],
        ),
        (
            "a conveyor id that is a string",
            with_edit(&current, "conveyor.json", |file| file["conveyor_data"][0]["conveyor_id"] = json!("3")),
            vec!["conveyor.json record 1 states `conveyor_id` as a string (\"3\"), and conveyor.json fixes it as a number"],
        ),
        (
            "a station point's position id that is a string",
            with_edit(&older, "pps.json", |rows| rows[0]["pps_points"][0]["position_id"] = json!("1")),
            vec!["pps.json record 1, point 1 states `position_id` as a string (\"1\")"],
        ),
        (
            "a bin with no station id",
            with_edit(&older, "pps_bin.json", |file| drop(file["data"][1].as_object_mut().unwrap().remove("pps_id"))),
            vec!["pps_bin.json record 2 states no `pps_id`, the station half of the key a record of pps_bin.json is known by"],
        ),
        (
            "a zone with no key",
            with_edit(&current, "zone.json", |rows| drop(rows[2]["zonerec"].as_object_mut().unwrap().remove("zone_id"))),
            vec!["zone.json record 3 states no `zone_id`"],
        ),
        (
            "a charger with no key",
            with_edit(&current, "charger.json", |rows| drop(rows[0].as_object_mut().unwrap().remove("charger_id"))),
            vec!["charger.json record 1 states no `charger_id`"],
        ),
        (
            "a storage position whose io point is null",
            with_edit(&current, "tote_storage.json", |rows| rows[1]["io_point"] = json!(null)),
            vec!["tote_storage.json record 2 states `io_point` as null, where the format nests an object"],
        ),
        (
            "a presentation point whose attributes are not an object",
            with_edit(&current, "pps.json", |rows| rows[0]["entities"]["presentation_points"][0]["attributes"] = json!(4)),
            vec!["pps.json record 1, presentation point 1 states `attributes` as a number (4)"],
        ),
        (
            "map.json with two floors",
            with_edit(&current, "map.json", |map| {
                let second = map[0].clone();
                map.as_array_mut().unwrap().push(second);
            }),
            vec!["map.json states 2 floor objects, and an export is one floor"],
        ),
        (
            "a floor id that is a string",
            with_edit(&current, "map.json", |map| map[0]["floor_id"] = json!("1")),
            vec!["map.json states `floor_id` as a string (\"1\"), and the format fixes a whole number"],
        ),
        (
            "a floor id that is a float",
            with_edit(&current, "map.json", |map| map[0]["floor_id"] = json!(1.0)),
            vec!["map.json states `floor_id` as a number (1.0)"],
        ),
        (
            "no floor id",
            with_edit(&current, "map.json", |map| drop(map[0].as_object_mut().unwrap().remove("floor_id"))),
            vec!["map.json states no `floor_id`"],
        ),
        (
            "a flat file that is null",
            with_edit(&current, "charger.json", |file| *file = json!(null)),
            vec!["charger.json is null, where charger.json keeps a list of records"],
        ),
        (
            "a flat file that is a scalar",
            with_edit(&current, "dock_point.json", |file| *file = json!("none")),
            vec!["dock_point.json is a string (\"none\"), where dock_point.json keeps a list of records"],
        ),
        (
            "an empty-on-every-site file holding a scalar",
            with_edit(&current, "elevator.json", |file| *file = json!([1])),
            vec!["elevator.json holds a list, and every export seen ships it empty"],
        ),
        (
            "a flat file holding a scalar",
            with_edit(&current, "charger.json", |file| file.as_array_mut().unwrap().push(json!(1))),
            vec!["charger.json holds a number (1) as its entry 3, where charger.json keeps a record"],
        ),
        (
            "a record list missing from its file",
            with_edit(&current, "conveyor.json", |file| drop(file.as_object_mut().unwrap().remove("conveyor_data"))),
            vec!["conveyor.json states no `conveyor_data` list"],
        ),
        (
            "a record list that is not a list",
            with_edit(&current, "vtm_zone.json", |file| file["aisle_info"] = json!({})),
            vec!["vtm_zone.json's `aisle_info` is an object, where vtm_zone.json keeps a list of records"],
        ),
        (
            "a key named twice in one object",
            with_text(&current, "zone.json", |text| {
                text.replacen("\"zone_id\": \"North Hall\",", "\"zone_id\": \"North Hall\", \"zone_id\": \"Annex\",", 1)
            }),
            vec!["zone.json names the key `zone_id` twice in one object, at zone.json:[].zonerec"],
        ),
        (
            "a `$comment` note in a record",
            with_edit(&current, "map.json", |map| map[0]["map_values"][0]["$comment"] = json!("resurveyed")),
            vec!["map.json carries a `$comment` note at map.json:[].map_values[].$comment"],
        ),
        (
            "a `$comment` note where the census does not look",
            with_edit(&current, "map.json", |map| {
                map[0]["location_profiles"][0]["bot_orientation_details"]["$comment"] = json!("per the vendor")
            }),
            vec!["at map.json:[].location_profiles[].bot_orientation_details.$comment"],
        ),
        (
            "a cell coordinate spelled with a space",
            with_edit(&current, "map.json", |map| map[0]["map_values"][0]["coordinate"] = json!("[1, 1]")),
            vec!["map.json cell \"[1, 1]\" states a `coordinate` that is not a grid pair spelled `[x,y]`"],
        ),
        (
            "a cell coordinate no double holds exactly",
            with_edit(&current, "map.json", |map| map[0]["map_values"][0]["coordinate"] = json!("[9007199254740993,1]")),
            vec!["map.json cell \"[9007199254740993,1]\" states a `coordinate` that is not a grid pair"],
        ),
        (
            "a field in a JSON kind its component's schema does not take",
            with_edit(&current, "pps.json", |stations| {
                stations[0]["entities"]["presentation_points"][0]["attributes"]["presentation_height"] = json!("820mm")
            }),
            vec![
                "pps.json states the field it carries in `presentation_point.attributes.presentation_height` as \
                 \"820mm\", where that component's published schema takes number",
                "(the presentation_point 31 P1)",
            ],
        ),
        (
            "a cell's flag the protocol's `cell` takes as a boolean, written as a number",
            with_edit(&current, "map.json", |map| map[0]["map_values"][0]["special"] = json!(0)),
            vec![
                "map.json states the field it carries in `cell.special` as 0, where that component's published \
                 schema takes boolean",
                "(the cell 1 [1,1])",
            ],
        ),
        (
            "a point's light curtain the protocol's `presentation_point` takes as a boolean, written as null",
            with_edit(&current, "pps.json", |stations| {
                stations[0]["entities"]["presentation_points"][0]["light_curtain"] = json!(null)
            }),
            vec![
                "pps.json states the field it carries in `presentation_point.light_curtain` as null, where that \
                 component's published schema takes boolean",
                "(the presentation_point 31 P1)",
            ],
        ),
        (
            "a cell on another floor than its file's",
            with_edit(&tenths, "map.json", |map| map[0]["map_values"][0]["floor"] = json!(3)),
            vec!["map.json cell [1,1] states `floor` 3, while the file states `floor_id` 2"],
        ),
        (
            "a bot type's permissions that are no object",
            with_edit(&current, "map.json", |map| {
                map[0]["location_profiles"][0]["bot_orientation_details"]["VTM_HAI_Z9_E1"] = json!([])
            }),
            vec!["map.json location profile \"LP-open\" states bot type \"VTM_HAI_Z9_E1\"'s permissions as a list"],
        ),
        (
            "a permission table that is no object",
            with_edit(&current, "map.json", |map| map[0]["location_profiles"][1]["bot_orientation_details"] = json!(null)),
            vec!["map.json location profile \"LP-narrow\" states `bot_orientation_details` as null"],
        ),
        (
            "an adjacency entry that is no grid pair",
            with_edit(&current, "map.json", |map| map[0]["map_values"][0]["adjacency"][1] = json!([2.5, 1])),
            vec!["map.json cell [1,1] states `adjacency` entry 2 as [2.5,1], where the format keeps null or a grid pair"],
        ),
        (
            "a `neighbours` list shorter than four",
            with_edit(&current, "map.json", |map| {
                map[0]["map_values"][0]["neighbours"].as_array_mut().expect("a list").truncate(3)
            }),
            vec!["map.json cell [1,1] states `neighbours` with 3 entries", "where the format keeps four"],
        ),
    ]);
    shapes
}

/// Every export shape the adapter carries that the four fixtures do not
/// show, each made from one of them: the export goes in, and the same export
/// comes back.
fn carried_shapes() -> Vec<(&'static str, Vec<File>)> {
    let current = fixture("current");
    let tenths = fixture("tenths");
    let older = fixture("older");
    vec![
        (
            "a dock point with no `dock_barcode`",
            with_edit(&current, "dock_point.json", |rows| {
                drop(rows[0].as_object_mut().unwrap().remove("dock_barcode"))
            }),
        ),
        (
            "a v3 station with no `entities.presentation_points`",
            with_edit(&current, "pps.json", |rows| {
                drop(rows[1]["entities"].as_object_mut().unwrap().remove("presentation_points"))
            }),
        ),
        (
            "a v2 station with no `pps_points`",
            with_edit(&older, "pps.json", |rows| drop(rows[1].as_object_mut().unwrap().remove("pps_points"))),
        ),
        (
            "a station with no `type`",
            with_edit(&current, "pps.json", |rows| drop(rows[0].as_object_mut().unwrap().remove("type"))),
        ),
        (
            "map.json with an empty `location_profiles`",
            with_edit(&older, "map.json", |map| map[0]["location_profiles"] = json!([])),
        ),
        (
            "a `pptl.json` group with no devices",
            with_edit(&older, "pptl.json", |groups| groups.as_array_mut().unwrap().insert(1, json!({ "pptls": [] }))),
        ),
        (
            "`sectorMxUPreferences.json` as a list holding its object",
            with_edit(&current, "sectorMxUPreferences.json", |file| *file = json!([file.clone()])),
        ),
        (
            "the sector index under both its names, each agreeing with the cells",
            with_added(&current, "sector.json", &file_json(&current, "sectors.json")),
        ),
        (
            "an aisle whose io points name an unknown cell in the middle",
            with_edit(&current, "vtm_zone.json", |file| {
                file["aisle_info"][0]["io_point_coordinates"] = json!(["[1,1]", "[8,8]", "[1,2]"])
            }),
        ),
        (
            "a block whose cells name an unknown one in the middle",
            with_edit(&current, "block.json", |rows| {
                rows[0]["entity_details"]["coordinates"] = json!([[1, 1], [9, 9], [1, 2]])
            }),
        ),
        (
            "a station whose served blocks name an unknown one in the middle",
            with_edit(&current, "pps.json", |rows| rows[0]["entities"]["blocks"] = json!(["7", "11", "8"])),
        ),
        (
            "an aisle whose io points are raw pairs",
            with_edit(&current, "vtm_zone.json", |file| {
                file["aisle_info"][0]["io_point_coordinates"] = json!([[1, 1], [1, 2]])
            }),
        ),
        (
            "an aisle whose io points mix the two spellings and an unknown cell",
            with_edit(&current, "vtm_zone.json", |file| {
                file["aisle_info"][1]["io_point_coordinates"] = json!([[4, 1], "[8,8]", "[1,1]", [8, 9]])
            }),
        ),
        (
            "a station whose blocks name an unknown one in the middle, spelled as a number",
            with_edit(&current, "pps.json", |rows| rows[1]["blocks"] = json!(["8", 11, "7"])),
        ),
        (
            "a station of the older dialect whose queue names an unknown barcode in the middle",
            with_edit(&older, "pps.json", |rows| rows[0]["queue_barcodes"] = json!(["721.211", "790.009", "731.311"])),
        ),
        (
            "a reference spelled in another JSON kind than its file fixes",
            with_edit(&older, "pptl.json", |groups| groups[0]["pptls"][0]["pps_id"] = json!(1)),
        ),
        (
            "a seat's station spelled in another JSON kind than its file fixes",
            with_edit(&older, "pps_seat.json", |file| file["pps_seats"][1]["pps_id"] = json!(1)),
        ),
        (
            "a cell reference spelled other than the cell's own coordinate",
            with_edit(&current, "tote_storage.json", |rows| rows[0]["io_point"]["coordinate"] = json!("[1, 1]")),
        ),
        (
            "two cells sharing a barcode nothing names",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                cells[11]["barcode"] = json!("641.147");
                cells
            }),
        ),
        (
            "a key named like a length inside a value the export states whole, on a site counting tenths",
            with_map_values(&tenths, {
                let mut cells = map_values(&tenths);
                cells[0]["barcode_properties"] = json!({ "position": [1, 2], "size": 0.25 });
                cells
            }),
        ),
        (
            "a fraction of a millimetre under a key named like a length, on a millimetre site",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                cells[1]["barcode_properties"] = json!({ "size": 12.05 });
                cells
            }),
        ),
        (
            "a presentation point stating no `world_coordinates`",
            with_edit(&current, "pps.json", |rows| {
                let attributes = rows[0]["entities"]["presentation_points"][0]["attributes"].as_object_mut().unwrap();
                drop(attributes.remove("world_coordinates"));
            }),
        ),
        (
            "a presentation point whose barcode names no cell, stating `world_coordinates`",
            with_edit(&current, "pps.json", |rows| {
                rows[0]["entities"]["presentation_points"][0]["attributes"]["barcode"] = json!("690.990")
            }),
        ),
        (
            "a presentation point whose barcode names no cell, stating no `world_coordinates`",
            with_edit(&current, "pps.json", |rows| {
                let attributes = rows[0]["entities"]["presentation_points"][0]["attributes"].as_object_mut().unwrap();
                attributes.insert("barcode".into(), json!("690.990"));
                drop(attributes.remove("world_coordinates"));
            }),
        ),
        (
            "a cell stating no `size_info`",
            with_map_values(&current, {
                let mut cells = map_values(&current);
                drop(cells[1].as_object_mut().unwrap().remove("size_info"));
                cells
            }),
        ),
        (
            "a footprint of two tenths whose sum in binary is no tenth",
            with_map_values(&tenths, {
                let mut cells = map_values(&tenths);
                cells[0]["size_info"] = json!([625.1, 625.5, 625.2, 625.5]);
                cells
            }),
        ),
    ]
}

/// A shared component another tool puts on an entity of this adapter's, naming
/// a station, is not a part of the station's record: a `presentation_point`
/// naming station 31, put on a cell, is not folded into pps.json, and a
/// `pps_bin` naming station 1, put on a cell of the older site, is not folded
/// into it nor written into pps_bin.json. The files come back as they went in,
/// and the same files ingested again change nothing.
#[test]
fn a_shared_component_another_tool_puts_on_a_record_names_no_station_part() {
    let module = mapcreator();
    for (export, station, part, value) in [
        ("current", "31", "presentation_point", json!({ "for": "pick" })),
        ("older", "1", "pps_bin", json!({ "direction": "front" })),
    ] {
        let files = fixture(export);
        let mut site = site_of(&module, export);
        let station = keyed(&site, "mapcreator_pps_id", station).id.clone();
        let cell = keyed(&site, "mapcreator_grid_coordinate", "[1,1]").id.clone();
        let mut foreign = value;
        foreign["station"] = json!(station);
        entity_mut(&mut site, &cell).components.insert(part.into(), foreign);
        let back = module.project(Budget::default(), &site).expect("a projection");
        assert!(same_files(&files, &back), "{export}: the {part} another tool put on a cell was written out");
        let again = ingest(&module, &files, &site).expect("an ingest");
        assert!(again.is_empty(), "{export}: {again:?}");
    }
}

/// A join between two records comes back from the reference, not from a kept
/// string: re-pointed by another tool, a bin's seat and type, a light's bin, and
/// the station a station's whole interior is moved to come back as the keys of
/// the records they now name. A bin's and a seat's station is half of its key
/// besides, and the file takes the reference. The interior moves whole because
/// a file names a seat or a bin among the parts of the naming record's own
/// station, so a bin at one station naming a seat at another is refused.
#[test]
fn a_reference_another_tool_repoints_comes_back_as_the_named_records_key() {
    let module = mapcreator();
    let mut site = site_of(&module, "older");
    let bin = keyed(&site, "mapcreator_bin_id", "B1").id.clone();
    let light = keyed(&site, "mapcreator_peripheral_id", "L-1").id.clone();
    let named = |site: &Site, key: &str, value: &str| json!(keyed(site, key, value).id);
    let (back_seat, large, second_bin, second_station) = (
        named(&site, "mapcreator_seat_name", "back-1"),
        named(&site, "mapcreator_bin_type_id", "large"),
        named(&site, "mapcreator_bin_id", "B2"),
        named(&site, "mapcreator_pps_id", "2"),
    );
    for entity in &mut site.entities {
        for part in ["pps_bin", "pps_seat", "pptl"] {
            if let Some(component) = entity.components.get_mut(part) {
                component["station"] = second_station.clone();
            }
        }
    }
    let wall = component_mut(&mut site, &bin, "pps_bin");
    wall["front_seat"] = back_seat;
    wall["bin_type"] = large;
    component_mut(&mut site, &light, "pptl")["bin"] = second_bin;
    let files = module.project(Budget::default(), &site).expect("a projection");
    let bins = file_json(&files, "pps_bin.json");
    assert_eq!(
        (&bins["data"][0]["front_seat_name"], &bins["data"][0]["type"], &bins["data"][0]["pps_id"]),
        (&json!("back-1"), &json!("large"), &json!("2"))
    );
    let seats = file_json(&files, "pps_seat.json");
    assert_eq!(&seats["pps_seats"][0]["pps_id"], &json!("2"));
    let lights = file_json(&files, "pptl.json");
    assert_eq!((&lights[0]["pptls"][0]["pps_bin_id"], &lights[0]["pptls"][0]["pps_id"]), (&json!("B2"), &json!("2")));
}

/// A reference the projection reads and could not write back refuses the
/// projection before a file is written, naming the referring entity, the field
/// and what it names: a joined record another tool deleted (the bin type a bin
/// names), a cell deleted (a dock point's), a station deleted (the one a
/// presentation point sits in), a block deleted (one a station's queues list),
/// the vendor sample point's barcode carried by no cell or by two, a reference
/// to a record of another kind, a point in a station written in the other
/// dialect, a bin at one station naming a seat of another, and a charger naming
/// a cell by a barcode another cell now carries too.
#[test]
fn a_reference_the_projection_cannot_write_back_is_refused_by_name() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    let older = site_of(&module, "older");
    let id = |site: &Site, key: &str, value: &str| keyed(site, key, value).id.clone();
    let cell = |coordinate: &str| id(&current, "mapcreator_grid_coordinate", coordinate);
    let without = |site: &Site, gone: &str| {
        let mut site = site.clone();
        site.entities.retain(|entity| entity.id != gone);
        site
    };
    let edited = |site: &Site, edit: &dyn Fn(&mut Site)| {
        let mut site = site.clone();
        edit(&mut site);
        site
    };
    let point = current
        .entities
        .iter()
        .find(|entity| entity.components.get("external_ids").is_some_and(|ids| ids["mapcreator_of_pps"] == json!("31")))
        .expect("station 31's presentation point")
        .id
        .clone();
    let floor = id(&current, "mapcreator_floor", "1");
    let (station, small, bin, front_seat) = (
        id(&current, "mapcreator_pps_id", "31"),
        id(&older, "mapcreator_bin_type_id", "small"),
        id(&older, "mapcreator_bin_id", "B1"),
        id(&older, "mapcreator_seat_name", "front-1"),
    );
    let (dock, profile, charger, block) = (
        id(&current, "mapcreator_dock_point_id", "642.247"),
        id(&current, "mapcreator_location_profile_id", "LP-open"),
        id(&current, "mapcreator_charger_id", "5"),
        id(&current, "mapcreator_block_id", "8"),
    );
    let barcode = |coordinate: &str, barcode: &'static str| {
        let at = cell(coordinate);
        edited(&current, &move |site| component_mut(site, &at, "external_ids")["barcode"] = json!(barcode))
    };
    let cases: Vec<(&str, Site, Vec<String>)> = vec![
        (
            "a joined record deleted",
            without(&older, &small),
            vec![format!(
                "entity {bin} (the pps_bin 1 B1) names entity {small} in `pps_bin.bin_type`, which is no \
                 bin geometry type of this site, so pps_bin.json's `type` could not be written back"
            )],
        ),
        (
            "a cell deleted",
            without(&current, &cell("[4,2]")),
            vec![format!(
                "entity {dock} (the dock_point 642.247) names entity {} in `dock_point.at_cell`, which is \
                 no cell of this site, so dock_point.json's `dock_coordinate` could not be written back",
                cell("[4,2]")
            )],
        ),
        (
            "a station deleted",
            without(&current, &station),
            vec![format!(
                "entity {point} (the presentation_point 31 P1) names entity {station} in \
                 `presentation_point.station`, which is no station of this site, so pps.json's \
                 `entities.presentation_points` could not be written back"
            )],
        ),
        (
            "a block deleted",
            without(&current, &block),
            vec![format!(
                "entity {station} (the station 31) names entity {block} as entry 2 of `mapcreator.station.served_by`, \
                 which is no block of this site, so pps.json's `entities.blocks` could not be written back"
            )],
        ),
        (
            "the vendor sample's cell given another barcode",
            barcode("[1,1]", "611.199"),
            vec![format!(
                "entity {floor} (the floor 1) names the barcode \"611.117\" in \
                 `mapcreator.frame.vda5050_reference_barcode`, which no cell of this site carries, so map.json's \
                 `vda5050_reference` could not be written back"
            )],
        ),
        (
            "the vendor sample's barcode carried by two cells",
            barcode("[2,1]", "611.117"),
            vec!["which 2 cells carry, [1,1] and [2,1]: which of them it means would be a guess".to_string()],
        ),
        (
            "a reference to a record of another kind",
            edited(&current, &|site| {
                let first = keyed(site, "mapcreator_grid_coordinate", "[1,1]").id.clone();
                component_mut(site, &first, "cell")["zone"] = json!(profile);
            }),
            vec![format!(
                "entity {} (the cell 1 [1,1]) names entity {profile} in `cell.zone`, which is no zone of \
                 this site but the location_profile LP-open, so map.json's `zone` could not be written back",
                cell("[1,1]")
            )],
        ),
        (
            "a point in a station written in the other dialect",
            edited(&current, &|site| {
                component_mut(site, &station, "import_formatting")["mapcreator_station"]["dialect_shape"] = json!("v2")
            }),
            vec![format!(
                "entity {point} (the presentation_point 31 P1) names entity {station} in \
                 `presentation_point.station`, a station written in pps.json's older dialect, whose record \
                 keeps no presentation points"
            )],
        ),
        (
            "a bin at one station naming a seat of another",
            edited(&older, &|site| {
                let second = keyed(site, "mapcreator_pps_id", "2").id.clone();
                component_mut(site, &bin, "pps_bin")["station"] = json!(second);
            }),
            vec![format!(
                "entity {bin} (the pps_bin 1 B1) names entity {front_seat} in `pps_bin.front_seat`, an \
                 operator seat written at station 1, while this record is written at station 2: pps_bin.json names it \
                 among the parts of the record's own station, so pps_bin.json's `front_seat_name` could not be \
                 written back"
            )],
        ),
        (
            "a block's cell whose grid coordinate cannot be spelled",
            edited(&current, &|site| {
                let second = keyed(site, "mapcreator_grid_coordinate", "[1,2]").id.clone();
                component_mut(site, &second, "external_ids")["mapcreator_grid_coordinate"] = json!("not-a-pair");
            }),
            vec![format!(
                "entity {} (the block 7) names entity {} as entry 2 of `queue.path`, a cell whose grid coordinate, \
                 \"not-a-pair\", which is not a grid pair spelled `[x,y]`, two whole numbers in their shortest form, \
                 so block.json's `entity_details.coordinates` could not be written back",
                id(&current, "mapcreator_block_id", "7"),
                cell("[1,2]")
            )],
        ),
        (
            "a cell nothing names whose grid coordinate cannot be spelled",
            edited(&current, &|site| {
                let last = keyed(site, "mapcreator_grid_coordinate", "[3,2]").id.clone();
                component_mut(site, &last, "external_ids")["mapcreator_grid_coordinate"] = json!("[3, 2]");
                for entity in &mut site.entities {
                    if let Some(members) = entity.components.get_mut("sector").and_then(|s| s.get_mut("members")) {
                        members.as_array_mut().expect("a list").retain(|member| member != &json!(last));
                    }
                }
            }),
            vec![format!(
                "entity {} (the cell 1 [3, 2]) carries the grid coordinate \"[3, 2]\" in `external_ids.mapcreator_grid_coordinate`, \
                 which is not a grid pair spelled `[x,y]`",
                cell("[3,2]")
            )],
        ),
        (
            "a cell named by a barcode another cell now carries too",
            barcode("[4,3]", "613.317"),
            vec![format!(
                "entity {charger} (the charger 5) names entity {} in `charger.at_cell`, a cell whose \
                 barcode, 613.317, 2 cells carry, [1,3] and [4,3]: charger.json names the cell by its barcode",
                cell("[1,3]")
            )],
        ),
    ];
    for (case, site, says) in cases {
        let refused = refusal(module.project(Budget::default(), &site));
        for phrase in says {
            assert!(refused.contains(&phrase), "{case}: {phrase:?} is not in {refused}");
        }
    }
}

/// A record of a kind one file owns, added by another tool to a site whose
/// export shipped that file absent or empty, refuses the projection naming
/// the record and the file, rather than being left out of the file.
#[test]
fn a_record_whose_file_the_inventory_lists_absent_or_empty_is_refused_by_name() {
    let module = mapcreator();
    let charger = keyed(&site_of(&module, "current"), "mapcreator_charger_id", "5").clone();
    let mut older = site_of(&module, "older");
    older.entities.push(charger.clone());
    let refused = refusal(module.project(Budget::default(), &older));
    let says = format!(
        "entity {} (the charger 5) is a record of charger.json, which the export entity's inventory says the site \
         never shipped: written as the inventory states it, the file would leave the record out",
        charger.id
    );
    assert!(refused.contains(&says), "{refused}");
    let emptied = with_edit(&fixture("current"), "charger.json", |file| *file = json!([]));
    let mut site = ingested(&module, &emptied, Budget::default(), OWN);
    site.entities.push(charger);
    let refused = refusal(module.project(Budget::default(), &site));
    assert!(refused.contains("which the export entity's inventory says the site shipped empty, as `[]`"), "{refused}");
}

/// A cell's position is written as `map.json` states one only from a pose in
/// the floor frame of two finite axes in range; a pose in another frame, of
/// three axes, with an axis that is no number or too far out refuses the
/// projection naming the cell, rather than being written as another position.
#[test]
fn a_cell_pose_map_json_cannot_state_is_refused_by_name() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    let cell = keyed(&current, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    let site_frame = keyed(&current, "mapcreator_frame", "site").id.clone();
    let frame = |to: Value| move |pose: &mut Value| pose["frame"] = to.clone();
    let position = |to: Value| move |pose: &mut Value| pose["position"] = to.clone();
    let cases: Vec<(Box<dyn Fn(&mut Value)>, String)> = vec![
        (Box::new(frame(json!(site_frame))), format!("states its `pose` in frame {site_frame}")),
        (
            Box::new(|pose: &mut Value| drop(pose.as_object_mut().expect("a pose").remove("frame"))),
            "states its `pose` in no frame".to_string(),
        ),
        (Box::new(position(json!([28750, 6250, 5]))), "states its `pose.position` as [28750,6250,5]".to_string()),
        (Box::new(position(json!(["a", 6250]))), "states its `pose.position` as [\"a\",6250]".to_string()),
        (Box::new(position(json!([1e300, 6250]))), "states its `pose.position` as [1e+300,6250]".to_string()),
    ];
    for (edit, says) in cases {
        let mut site = current.clone();
        edit(component_mut(&mut site, &cell, "pose"));
        let refused = refusal(module.project(Budget::default(), &site));
        let named = format!("entity {cell} (the cell 1 [1,1]) {says}");
        assert!(refused.contains(&named), "{named:?} is not in {refused}");
        assert!(refused.ends_with("so map.json's `world_coordinate` could not be written back"), "{refused}");
    }
}

/// A cell held by two sectors, or twice by one, refuses the projection naming
/// it and the sectors, since its own `sector` names one and the sector index
/// would list it twice.
#[test]
fn a_cell_two_sectors_hold_is_refused_by_name() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    let cell = keyed(&current, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    let (one, undefined) = (
        keyed(&current, "mapcreator_sector", "1").id.clone(),
        keyed(&current, "mapcreator_sector", "undefined").id.clone(),
    );
    for (sector, says) in [(&undefined, format!("and so does entity {one}")), (&one, "twice".to_string())] {
        let mut site = current.clone();
        component_mut(&mut site, sector, "sector")["members"].as_array_mut().expect("a list").push(json!(cell));
        let refused = refusal(module.project(Budget::default(), &site));
        let named = format!("names cell {cell} in `sector.members` {says}");
        assert!(refused.contains(&format!("entity {sector}")) && refused.contains(&named), "{named:?}: {refused}");
    }
}

/// A list holding only entries that named nothing comes back as those
/// entries; a reference stated beside the value the import keeps only where
/// one named nothing refuses the projection naming both fields, rather than
/// one of the two being written for it.
#[test]
fn a_kept_value_alone_comes_back_and_beside_its_reference_is_refused() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    let station = keyed(&current, "mapcreator_pps_id", "31").id.clone();
    let mut site = current.clone();
    let held = component_mut(&mut site, &station, "mapcreator.station").as_object_mut().expect("a station");
    for field in ["served_by", "blocks"] {
        held.remove(field);
        held.insert(format!("{field}_unresolved"), json!([{ "at": 0, "value": format!("{field}-77") }]));
    }
    let files = module.project(Budget::default(), &site).expect("a projection");
    let stations = file_json(&files, "pps.json");
    let written = stations.as_array().expect("records").iter().find(|record| record["pps_id"] == json!(31));
    let written = written.expect("station 31");
    assert_eq!((&written["entities"]["blocks"], &written["blocks"]), (&json!(["served_by-77"]), &json!(["blocks-77"])));

    let charger = keyed(&current, "mapcreator_charger_id", "5").id.clone();
    let point = current
        .entities
        .iter()
        .find(|entity| entity.components.get("external_ids").is_some_and(|ids| ids["mapcreator_of_pps"] == json!("31")))
        .expect("station 31's presentation point")
        .id
        .clone();
    let unsplit = keyed(&current, "mapcreator_ods_tuple", "unsplit-tuple").id.clone();
    let cell = keyed(&current, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    let cases: Vec<(&str, &str, Value, String)> = vec![
        (
            &charger,
            "charger",
            json!({ "at_cell_unresolved": "613.317" }),
            "states both `charger.at_cell` and `charger.at_cell_unresolved`".into(),
        ),
        (
            &point,
            "presentation_point",
            json!({ "world_coordinates_unresolved": [9, 9] }),
            "states both `presentation_point.attributes.cell` and `presentation_point.world_coordinates_unresolved`"
                .into(),
        ),
        (
            &unsplit,
            "ods_exclusion",
            json!({ "cell": cell }),
            "states both `ods_exclusion.cell` and `ods_exclusion.tuple_unparsed`".into(),
        ),
    ];
    for (id, name, added, says) in cases {
        let mut site = current.clone();
        let held = component_mut(&mut site, id, name).as_object_mut().expect("a component");
        held.extend(added.as_object().expect("fields").clone());
        let refused = refusal(module.project(Budget::default(), &site));
        assert!(refused.contains(&says) && refused.contains("would be a guess"), "{says:?}: {refused}");
    }
}

/// Every record is written under the key it is known and named by, and no
/// two under one: a station's own `pps_id` edited away from its key, two
/// sectors spelling one key, and two seats re-pointed to one station under
/// one name each refuse the projection naming the records.
#[test]
fn every_record_is_written_under_its_own_key_or_refused() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    let older = site_of(&module, "older");
    let station = keyed(&current, "mapcreator_pps_id", "31").id.clone();
    let undefined = keyed(&current, "mapcreator_sector", "undefined").id.clone();
    let seat = keyed(&older, "mapcreator_seat_name", "front-1").clone();
    let mut renumbered = current.clone();
    component_mut(&mut renumbered, &station, "import_formatting")["mapcreator_station"]["pps_id"] = json!(32);
    let mut respelled = current.clone();
    component_mut(&mut respelled, &undefined, "import_formatting")["mapcreator_sector"]["key"] = json!(1);
    let mut twice = older.clone();
    let mut second = seat.clone();
    second.id = "f0000000-0000-4000-8000-0000000000f3".into();
    second.components.get_mut("external_ids").expect("keys")["mapcreator_of_pps"] = json!("2");
    twice.entities.push(second);
    for (site, says) in [
        (
            renumbered,
            format!(
                "entity {station} (the station 31) states `import_formatting.mapcreator_station.pps_id` 32, while \
                 every record naming it is written with the key it is known by, 31"
            ),
        ),
        (respelled, format!("and {undefined} (the sector undefined) would both be written as the sector 1")),
        (twice, "would both be written as the operator seat front-1 of station 1".to_string()),
    ] {
        let refused = refusal(module.project(Budget::default(), &site));
        assert!(refused.contains(&says), "{says:?} is not in {refused}");
    }
}

/// The entries a list kept where they named nothing are each `{at, value}`,
/// no two at one place, every place within the list: a kept entry beyond the
/// list, two at one place, or one of another shape refuses the projection
/// naming the record and the field, rather than being moved into a place.
#[test]
fn a_kept_entry_out_of_its_list_is_refused_by_name() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    let block = keyed(&current, "mapcreator_block_id", "10").id.clone();
    for (kept, says) in [
        (json!([{ "at": 100, "value": [9, 9] }]), "with one at place 100, beyond the 2 the list holds"),
        (json!([{ "at": 1, "value": [9, 9] }, { "at": 1, "value": [8, 8] }]), "with two at place 1"),
        (json!([{ "at": "1", "value": [9, 9] }]), "with the entry {\"at\":\"1\",\"value\":[9,9]}"),
    ] {
        let mut site = current.clone();
        component_mut(&mut site, &block, "block")["coordinates_unresolved"] = kept;
        let refused = refusal(module.project(Budget::default(), &site));
        let named =
            format!("entity {block} (the block 10) keeps `block.coordinates`'s entries that named nothing {says}");
        assert!(refused.contains(&named), "{named:?} is not in {refused}");
    }
}

/// A field of a component this adapter writes that no Map Creator file has a
/// place for, at its top, in its `runtime` or in an object it nests, is
/// refused by both directions, naming the entity and the field: the files
/// would leave it out, and an ingest would write the component without it. So
/// is a formatting entry under another key than the record's own, and a field
/// of the record's own entry. Another system's component is never read so,
/// nor a field the protocol states of a shared component on its own account.
#[test]
fn a_field_no_file_has_a_place_for_is_refused_by_both_directions() {
    let module = mapcreator();
    let current = fixture("current");
    let site = site_of(&module, "current");
    let cell = keyed(&site, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    let point = site
        .entities
        .iter()
        .find(|entity| entity.components.get("external_ids").is_some_and(|ids| ids["mapcreator_of_pps"] == json!("31")))
        .expect("station 31's presentation point")
        .id
        .clone();
    for (id, component, path, says) in [
        (&cell, "cell", &["new_property"][..], "`cell.new_property`"),
        (&cell, "cell", &["runtime", "lit"][..], "`cell.runtime.lit`"),
        (&point, "presentation_point", &["attributes", "tilt"][..], "`presentation_point.attributes.tilt`"),
        (
            &cell,
            "import_formatting",
            &["mapcreator_cell", "new_property"][..],
            "`import_formatting.mapcreator_cell.new_property`",
        ),
        (&cell, "import_formatting", &["mapcreator_zone"][..], "`import_formatting.mapcreator_zone`"),
    ] {
        let mut edited = site.clone();
        let mut held = component_mut(&mut edited, id, component);
        for step in &path[..path.len() - 1] {
            held = &mut held[*step];
        }
        held[path[path.len() - 1]] = json!(1);
        let projected = refusal(module.project(Budget::default(), &edited));
        let ingested = refusal(ingest(&module, &current, &edited));
        for refused in [projected, ingested] {
            let named = format!("entity {id} carries {says}, a field this adapter has no place for");
            assert!(refused.contains(&named), "{named:?} is not in {refused}");
        }
    }
    let mut foreign = site.clone();
    let held = entity_mut(&mut foreign, &cell);
    held.components.insert("wms.slot".into(), json!({ "new_property": 1 }));
    held.components.get_mut("cell").expect("a cell")["legacy_grid_coordinate"] = json!([1, 1]);
    assert!(same_files(&current, &module.project(Budget::default(), &foreign).expect("a projection")));
}

/// Another importer's entry of a record's `import_formatting`, beside this
/// adapter's own, is that importer's: the projection writes the same files
/// past it, the same files ingested again change nothing, and an edit to the
/// record patches its formatting with the other entry kept beside its own.
#[test]
fn another_importers_formatting_entry_is_kept_by_both_directions() {
    let module = mapcreator();
    let current = fixture("current");
    let mut site = site_of(&module, "current");
    let cell = keyed(&site, "mapcreator_grid_coordinate", "[1,1]").id.clone();
    component_mut(&mut site, &cell, "import_formatting")["other_importer_cell"] = json!({ "index": 91 });
    assert!(same_files(&current, &module.project(Budget::default(), &site).expect("a projection")));
    let again = ingest(&module, &current, &site).expect("an ingest");
    assert!(again.is_empty(), "{again:?}");
    // The cell's first two keys written the other way round, which its own
    // entry records as a key order departing from the file's usual one.
    let edited = with_text(&current, "map.json", |text| {
        let usual = "\"coordinate\": \"[1,1]\",\n        \"barcode\": \"611.117\",";
        assert!(text.contains(usual), "the first cell's text");
        text.replacen(usual, "\"barcode\": \"611.117\",\n        \"coordinate\": \"[1,1]\",", 1)
    });
    let changes = ingest(&module, &edited, &site).expect("an ingest");
    let patched = changes
        .iter()
        .find_map(|change| match change {
            Change::Patch(write) if write.id == cell => Some(write),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no patch to the cell: {changes:?}"));
    let formatting = &patched.components["import_formatting"];
    assert_eq!(formatting["other_importer_cell"], json!({ "index": 91 }), "{formatting}");
    assert!(formatting["mapcreator_cell"].get("key_order").is_some(), "{formatting}");
}

/// A second export inventory, site frame or floor frame, under a lookup key of
/// its own, refuses the projection naming both, rather than one being written
/// and the other's facts left out.
#[test]
fn a_second_inventory_site_frame_or_floor_frame_is_refused_by_name() {
    let module = mapcreator();
    let current = site_of(&module, "current");
    for (key, value, second_key, says) in [
        ("mapcreator_export", "site", json!({ "mapcreator_export": "other" }), "export inventory"),
        ("mapcreator_frame", "site", json!({ "mapcreator_frame": "site-2" }), "site frame"),
        ("mapcreator_floor", "1", json!({ "mapcreator_floor": "2" }), "floor frame"),
    ] {
        let first = keyed(&current, key, value).clone();
        let mut second = first.clone();
        second.id = "f0000000-0000-4000-8000-0000000000f2".into();
        second.components.insert("external_ids".into(), second_key);
        let mut site = current.clone();
        site.entities.push(second);
        let refused = refusal(module.project(Budget::default(), &site));
        // Named in identity order, and the first's identity is random.
        for phrase in [first.id.as_str(), "f0000000-0000-4000-8000-0000000000f2", says, "and an export has one"] {
            assert!(refused.contains(phrase), "{key}: {phrase:?} is not in {refused}");
        }
    }
}

/// A preference naming a sector the site no longer holds is refused by the
/// projection, which would otherwise have written the sector's identity into
/// `sectorMxUPreferences.json` as if it were a sector key; so is one naming a
/// record that is no sector. A key the file wrote for a sector no cell states
/// comes back at its place, as the fixture's own `rack-A` list shows.
#[test]
fn a_preference_naming_no_sector_of_the_site_is_refused_rather_than_written_as_an_identity() {
    let module = mapcreator();
    let current = fixture("current");
    let site = site_of(&module, "current");
    let preference = keyed(&site, "mapcreator_preferred_for", "rack-A").id.clone();
    let sector = keyed(&site, "mapcreator_sector", "1").id.clone();
    let prefers = &entity(&site, &preference).components["sector_preference"];
    assert_eq!(
        (&prefers["prefers"], &prefers["prefers_unresolved"]),
        (&json!([sector]), &json!([{ "at": 1, "value": 5 }])),
        "rack-A prefers sector 1, and keeps the key 5 no cell states at its place"
    );
    let files = module.project(Budget::default(), &site).expect("a projection");
    assert_eq!(file_json(&files, "sectorMxUPreferences.json"), file_json(&current, "sectorMxUPreferences.json"));

    let mut gone = site.clone();
    gone.entities.retain(|entity| entity.id != sector);
    let refused = refusal(module.project(Budget::default(), &gone));
    let says = format!(
        "entity {preference} (the sector_preference rack-A) names entity {sector} as entry 1 of \
         `sector_preference.prefers`, which is no sector of this site, so sectorMxUPreferences.json could \
         not be written back"
    );
    assert!(refused.contains(&says), "{refused}");

    let mut another = site.clone();
    let zone = keyed(&site, "mapcreator_zone_id", "North Hall").id.clone();
    component_mut(&mut another, &preference, "sector_preference")["prefers"] = json!([zone]);
    let refused = refusal(module.project(Budget::default(), &another));
    assert!(refused.contains("which is no sector of this site but the zone North Hall"), "{refused}");
}

/// Every export shape the adapter carries comes back as it went in, a second
/// ingest of it asks for nothing, and every component of its own vocabulary it
/// writes validates against its schema under `vocabulary/`.
#[test]
fn every_export_shape_the_adapter_carries_comes_back() {
    let module = mapcreator();
    let validators: BTreeMap<String, Validator> =
        vocabulary().into_iter().map(|(name, definition)| (name, compiled(&definition["schema"]))).collect();
    for (shape, files) in carried_shapes() {
        let trips = round_trip(&module, &[files], &empty(), None, Some(&normalized))
            .unwrap_or_else(|broken| panic!("{shape}: {broken}"));
        for entity in &trips[0].site.entities {
            for (component, payload) in &entity.components {
                let Some(validator) = validators.get(component) else { continue };
                if let Err(refusal) = validator.validate(payload) {
                    panic!("{shape}: a `{component}` payload is refused by its schema: {refusal}\n{payload}");
                }
            }
        }
    }
}

/// Every export shape the adapter cannot give back exactly is refused at
/// ingest, by a sentence that names where the shape is and why.
#[test]
fn every_export_shape_the_adapter_cannot_give_back_is_refused_by_name() {
    let module = mapcreator();
    for (shape, files, says) in refused_shapes() {
        let refused = refusal(ingest(&module, &files, &empty()));
        for phrase in says {
            assert!(refused.contains(phrase), "{shape}: {phrase:?} is not in {refused}");
        }
    }
}

/// A whole number spelled with a decimal point, `1.0`, is the number `1`: an
/// export spelling every adjacency index and every grade so declares the very
/// stitches the plain export does, and still comes back byte for byte, each
/// `1.0` as it was written.
#[test]
fn a_whole_number_spelled_with_a_decimal_point_is_the_same_connection() {
    let module = mapcreator();
    let current = fixture("current");
    let decimal = |value: &mut Value| {
        for number in value.as_array_mut().into_iter().flatten() {
            if let Some(whole) = number.as_i64() {
                *number = json!(whole as f64);
            }
        }
    };
    let mut cells = map_values(&current);
    for cell in &mut cells {
        for entry in cell["adjacency"].as_array_mut().into_iter().flatten() {
            decimal(entry);
        }
        for triple in cell["neighbours"].as_array_mut().into_iter().flatten() {
            decimal(triple);
        }
    }
    let respelled = with_map_values(&current, cells);
    assert!(String::from_utf8_lossy(&respelled.iter().find(|file| file.name == "map.json").expect("map").bytes)
        .contains("[1.0,1.0,1.0]"));
    let stitches = |files: &[File]| -> BTreeSet<String> {
        declared(&ingest(&module, files, &empty()).expect("an ingest"))
            .into_iter()
            .filter(|components| components.contains_key("stitch"))
            .map(|components| components["external_ids"]["mapcreator_stitch"].as_str().expect("a key").to_string())
            .collect()
    };
    let plain = stitches(&current);
    assert!(!plain.is_empty());
    assert_eq!(stitches(&respelled), plain);
    assert_round_trip(&module, &[respelled.clone(), respelled], &empty(), None, None);
}

/// A vendor coordinate that measures nothing — not two finite numbers, or one
/// too far from the origin for a step count to hold — rides back as the file
/// wrote it and is no vote in the vendor frame's fit: alone it declares no
/// frame, and beside one that measures, the frame is the one that one states.
#[test]
fn an_unusable_vendor_coordinate_is_no_vote_in_the_vendor_frame() {
    let module = mapcreator();
    let older = fixture("older");
    let respelled = |vda: [&str; 6]| {
        let mut map = file_json(&older, "map.json");
        for (cell, stated) in map[0]["map_values"].as_array_mut().expect("cells").iter_mut().zip(vda) {
            cell["vda_world_coordinate"] = json!(stated);
        }
        // The sample point names the first cell, and states its coordinate.
        map[0]["vda5050_reference"]["vda5050_coordinate"] = json!(vda[0]);
        with_file(&older, "map.json", &map)
    };
    let vendor = |files: &[File]| -> Option<Value> {
        ingested(&module, files, Budget::default(), OWN)
            .entities
            .iter()
            .find(|entity| entity.components["external_ids"].get("mapcreator_frame") == Some(&json!("vendor/vda5050")))
            .map(|entity| entity.components["frame"]["transform_to_parent"]["translation"].clone())
    };
    assert_eq!(vendor(&older), Some(json!([-410000, 520000])));
    for unusable in ["[NaN,NaN]", "[-1e300,1e300]", "[inf,0]"] {
        let files = respelled([unusable; 6]);
        assert_eq!(vendor(&files), None, "{unusable}");
        assert_round_trip(&module, &[files], &empty(), None, Some(&normalized));
    }
    let nan = "[NaN,NaN]";
    let mixed = respelled([nan, nan, nan, nan, "[1e300,0]", "[420300,525200]"]);
    assert_eq!(vendor(&mixed), Some(json!([-410000, 520000])));
    assert_round_trip(&module, &[mixed], &empty(), None, Some(&normalized));
}

/// A movement mode a station names twice is one mode: the station is still a
/// pick station in the core, and its list comes back as it went in.
#[test]
fn a_mode_named_twice_is_one_mode() {
    let module = mapcreator();
    let current = fixture("current");
    let twice = with_edit(&current, "pps.json", |stations| stations[0]["allowed_modes"] = json!(["pick", "pick"]));
    let site = ingested(&module, &twice, Budget::default(), OWN);
    let station = keyed(&site, "mapcreator_pps_id", "31");
    assert_eq!(station.components.get("station").and_then(|core| core.get("kind")), Some(&json!("pick")));
    assert_round_trip(&module, &[twice], &empty(), None, Some(&normalized));
}

/// A legacy grid fit whose slope rounds to nothing at the thousandth a frame
/// is written to declares no grid frame, rather than one of no scale that no
/// cell can be projected into; the export still comes back.
#[test]
fn a_grid_fit_rounding_to_no_scale_declares_no_grid_frame() {
    let module = mapcreator();
    let cell = |coordinate: &str, world: &str, barcode: &str| json!({ "coordinate": coordinate, "world_coordinate": world, "barcode": barcode, "blocked": false });
    let map = json!([{ "floor_id": 1, "map_values": [
        cell("[-1000,-1000]", "[0,0]", "B1"),
        cell("[1,1]", "[0.1,0.1]", "B2"),
    ] }]);
    let files = vec![File { name: "map.json".into(), bytes: serde_json::to_vec(&map).expect("JSON serializes") }];
    let site = ingested(&module, &files, Budget::default(), OWN);
    let grid = site.entities.iter().find(|entity| {
        entity.components.get("external_ids").and_then(|ids| ids.get("mapcreator_frame")) == Some(&json!("legacy-grid"))
    });
    assert!(grid.is_none(), "{grid:?}");
    assert_round_trip(&module, &[files], &empty(), None, Some(&normalized));
}

/// The order a file states its records in is read off each record's own
/// entry of its formatting: an `index` another component of the entity
/// carries, another system's or one under Map Creator's name, moves no record
/// and asks for no change.
#[test]
fn an_index_on_another_component_orders_nothing() {
    let module = mapcreator();
    let current = fixture("current");
    let mut site = site_of(&module, "current");
    let last = keyed(&site, "mapcreator_grid_coordinate", "[999,3]").id.clone();
    entity_mut(&mut site, &last).components.insert("mapcreator.aaa".into(), json!({ "index": -1 }));
    let second = site
        .entities
        .iter()
        .find(|entity| {
            entity.components.get("external_ids").is_some_and(|ids| {
                ids.get("mapcreator_of_pps") == Some(&json!("32"))
                    && ids.get("mapcreator_point_id") == Some(&json!("P1"))
            })
        })
        .expect("station 32's second point")
        .id
        .clone();
    entity_mut(&mut site, &second).components.insert("aaa.metadata".into(), json!({ "index": -1 }));
    assert!(same_files(&current, &module.project(Budget::default(), &site).expect("a projection")));
    assert!(ingest(&module, &current, &site).expect("an ingest").is_empty());
}

/// A stitch joins two cells whose adjacencies name each other: where one end
/// names the other and the other's opposite slot names a third cell, the
/// other end's grades are toward that third cell, so no stitch is read off
/// them; the adjacencies still come back as they went in.
#[test]
fn a_stitch_needs_both_ends_to_name_each_other() {
    let module = mapcreator();
    let free = json!([1, 1, 1]);
    let none = json!([0, 0, 0]);
    let cell = |x: i64, adjacency: Value, neighbours: Value| {
        json!({
            "coordinate": format!("[{x},1]"), "world_coordinate": format!("[{},1000]", x * 1000),
            "barcode": format!("B{x}"), "blocked": false, "adjacency": adjacency, "neighbours": neighbours
        })
    };
    let export = |b_names: Value| {
        let map = json!([{ "floor_id": 1, "map_values": [
            cell(1, json!([null, null, null, [2, 1]]), json!([none, none, none, free])),
            cell(2, json!([null, b_names, null, null]), json!([none, free, none, none])),
            cell(3, json!([null, null, null, null]), json!([none, none, none, none])),
        ] }]);
        vec![File { name: "map.json".into(), bytes: serde_json::to_vec(&map).expect("JSON serializes") }]
    };
    let stitches = |files: &[File]| -> Vec<String> {
        declared(&ingest(&module, files, &empty()).expect("an ingest"))
            .into_iter()
            .filter(|components| components.contains_key("stitch"))
            .map(|components| components["external_ids"]["mapcreator_stitch"].as_str().expect("a key").to_string())
            .collect()
    };
    assert_eq!(stitches(&export(json!([1, 1]))), ["[1,1]|[2,1]"]);
    assert!(stitches(&export(json!([3, 1]))).is_empty());
    assert_round_trip(&module, &[export(json!([3, 1]))], &empty(), None, Some(&normalized));
}

/// A cell's footprint is the per-axis sum of the four half-extents its
/// `size_info` states, and a cell stating no `size_info` states no footprint:
/// its core `cell.size` is absent, as the half-extents are, rather than summed
/// from zeros.
#[test]
fn a_cell_stating_no_size_info_states_no_footprint() {
    let module = mapcreator();
    let mut cells = map_values(&fixture("current"));
    drop(cells[1].as_object_mut().expect("a cell").remove("size_info"));
    let site = ingested(&module, &with_map_values(&fixture("current"), cells), Budget::default(), OWN);
    let bare = keyed(&site, "mapcreator_grid_coordinate", "[2,1]");
    assert_eq!((bare.components["cell"].get("size"), bare.components["cell"].get("size_info")), (None, None));
    let sized = keyed(&site, "mapcreator_grid_coordinate", "[1,1]");
    assert_eq!(sized.components["cell"]["size"], json!([1250, 1250]));
}

/// A measured value no step states exactly would be rounded onto one, so it
/// is refused, saying how many there are and naming the worst, with what it
/// would be counted as.
#[test]
fn a_measurement_finer_than_a_tenth_of_a_millimetre_is_refused() {
    let module = mapcreator();
    let tenths = fixture("tenths");
    let mut cells = map_values(&tenths);
    cells[0]["world_coordinate"] = json!("[2250.85,1750.6]");
    let refused = refusal(ingest(&module, &with_map_values(&tenths, cells), &empty()));
    assert_eq!(
        refused,
        "1 length(s) of the export land on no step this adapter counts in, 1, 0.5, 0.25 or 0.1 mm, so the site could \
         not state them without rounding: the worst is a length of 2250.85 mm on map.json cell [1,1], which would be \
         counted as 2250.9 mm"
    );
}

/// A site keeps the unit it counts in: an export needing a finer one is
/// refused, naming the length and the unit, so another system's pose counted
/// in the site's steps is never rescaled; and an export a coarser unit would do
/// for keeps the site's own, so the site frame is not rewritten.
#[test]
fn a_site_keeps_its_unit_and_an_export_needing_a_finer_one_is_refused() {
    let module = mapcreator();
    let mut site = site_of(&module, "current");
    let site_frame = keyed(&site, "mapcreator_frame", "site").id.clone();
    site.entities.push(Entity {
        id: "d0000000-0000-4000-8000-000000000009".into(),
        name: Some("another system's rack".into()),
        scope: OWN.into(),
        derived: false,
        components: json!({ "pose": { "frame": site_frame, "position": [100, 0, 0] } }).as_object().unwrap().clone(),
    });
    let mut cells = map_values(&fixture("current"));
    cells[0]["world_coordinate"] = json!("[28750.5,6250]");
    let refused = refusal(ingest(&module, &with_map_values(&fixture("current"), cells), &site));
    assert!(
        refused.starts_with(
            "the export states a length of 28750.5 mm on map.json cell [1,1], which the site's step of 1 mm cannot \
             state exactly"
        ),
        "{refused}"
    );

    let tenths = fixture("tenths");
    let site = site_of(&module, "tenths");
    let site_frame = keyed(&site, "mapcreator_frame", "site").id.clone();
    let mut cells = map_values(&tenths);
    for cell in &mut cells {
        let [x, y] = millimetre_pair(cell["world_coordinate"].as_str().unwrap());
        cell["world_coordinate"] = json!(spell_millimetres([x.round(), y.round()]));
    }
    let whole = with_map_values(&tenths, cells);
    let changes = ingest(&module, &whole, &site).expect("an ingest");
    assert!(!changes.is_empty() && changes.iter().all(|change| change.id() != site_frame), "{changes:?}");
    assert_round_trip(&module, &[tenths, whole], &empty(), None, Some(&normalized));

    // A site frame counting a step no length can be carried back from is
    // refused, naming the frame, the step it counts and the four this takes.
    let mut site = site_of(&module, "current");
    let site_frame = keyed(&site, "mapcreator_frame", "site").id.clone();
    component_mut(&mut site, &site_frame, "frame")["unit_mm"] = json!(0.3);
    let refused = refusal(ingest(&module, &fixture("current"), &site));
    for says in [site_frame.as_str(), "states `frame.unit_mm` 0.3", "steps of 1, 0.5, 0.25 or 0.1 mm"] {
        assert!(refused.contains(says), "{says:?} in {refused}");
    }
    // The projection holds the site to the same steps, rather than dividing a
    // count by a step it cannot carry back exactly.
    let refused = refusal(module.project(Budget::default(), &site));
    for says in ["states `frame.unit_mm` as 0.3", "steps of 1, 0.5, 0.25 or 0.1 mm"] {
        assert!(refused.contains(says), "{says:?} in {refused}");
    }
}

/// A site no Map Creator export was ingested into is refused by the
/// projection, with a sentence saying what this adapter writes out.
#[test]
fn a_site_no_export_was_ingested_into_is_not_written_out() {
    let module = mapcreator();
    let refused = refusal(module.project(Budget::default(), &empty()));
    assert_eq!(
        refused,
        "this site holds no Map Creator export entity, and this adapter writes out only a site that was ingested \
         from a Map Creator export"
    );
}

/// Removing one record patches no record nobody touched: of three chargers,
/// the first writing its keys otherwise than the other two, the third removed
/// leaves the two left at one each, and the order the site already holds for
/// their keys stays, so the first keeps its own order and the second none,
/// and the one change is the removed charger's delete. The files come back
/// byte for byte before and after.
#[test]
fn removing_one_record_patches_no_record_nobody_touched() {
    let module = mapcreator();
    let current = fixture("current");
    let chargers = |text: &str| {
        let mut files = current.clone();
        let file = files.iter_mut().find(|file| file.name == "charger.json").expect("charger.json");
        file.bytes = text.as_bytes().to_vec();
        files
    };
    let three = chargers(
        r#"[{"charger_id":1,"status":"idle"},{"status":"idle","charger_id":2},{"status":"idle","charger_id":3}]"#,
    );
    let two = chargers(r#"[{"charger_id":1,"status":"idle"},{"status":"idle","charger_id":2}]"#);
    let site = ingested(&module, &three, Budget::default(), OWN);
    let removed = keyed(&site, "mapcreator_charger_id", "3").id.clone();
    let changes = ingest(&module, &two, &site).expect("an ingest");
    assert_eq!(changes, [Change::Delete(removed)], "{changes:?}");
    assert_round_trip(&module, &[three, two], &empty(), None, None);
}

/// A preference's list is written as its own formatting says, under whatever
/// carrier type the preference has: a sector key the file spelled `1.0` in
/// `rack-A`'s list comes back so, byte for byte, and comes back `1.0` in the
/// list of `crate` after another tool renames the carrier type, with nothing
/// of it left on the export entity under the old name.
#[test]
fn a_preferences_spelling_moves_with_its_carrier_type() {
    let module = mapcreator();
    let current = fixture("current");
    let pointed = with_text(&current, "sectorMxUPreferences.json", |text| {
        assert!(text.contains("\n    1,\n"), "rack-A's first sector");
        text.replacen("\n    1,\n", "\n    1.0,\n", 1)
    });
    assert_round_trip(&module, &[pointed.clone(), pointed.clone()], &empty(), None, None);
    let mut site = ingested(&module, &pointed, Budget::default(), OWN);
    let export = keyed(&site, "mapcreator_export", "site");
    let outside = export.components["mapcreator.export"].get("written_with_point").cloned().unwrap_or_default();
    assert!(!outside.to_string().contains("sectorMxUPreferences.json"), "the export entity keeps {outside}");
    let preference = keyed(&site, "mapcreator_preferred_for", "rack-A").id.clone();
    component_mut(&mut site, &preference, "external_ids")["mapcreator_preferred_for"] = json!("crate");
    let files = module.project(Budget::default(), &site).expect("a projection");
    let table = &files.iter().find(|file| file.name == "sectorMxUPreferences.json").expect("the file").bytes;
    let text = String::from_utf8_lossy(table);
    assert!(text.contains("\"crate\": [\n    1.0,\n    5\n  ]"), "{text}");
}

/// A zero written `-0.0` comes back with its sign, though the site holds it as
/// 0; and a whole number the file wrote with a point keeps its own digits when
/// another tool makes it one a double does not hold exactly, rather than come
/// back as that double's. The export that number is written into comes back
/// in by its digits: into the site it was projected from, keeping the
/// charger's identity, and into an empty site, each time going out again as
/// the same bytes and settling.
#[test]
fn a_negative_zero_and_a_whole_number_past_a_double_come_back_as_written() {
    let module = mapcreator();
    let current = fixture("current");
    let respelled = |from: &str, to: &str| {
        with_text(&current, "charger.json", |text| {
            assert!(text.contains(from), "charger.json has no {from:?}");
            text.replacen(from, to, 1)
        })
    };
    let signed = respelled("\"charge\": 64.5,", "\"charge\": -0.0,");
    assert_round_trip(&module, &[signed.clone(), signed], &empty(), None, None);
    let pointed = respelled("\"charger_id\": 5,", "\"charger_id\": 5.0,");
    let mut site = ingested(&module, &pointed, Budget::default(), OWN);
    let charger = keyed(&site, "mapcreator_charger_id", "5").id.clone();
    component_mut(&mut site, &charger, "external_ids")["mapcreator_charger_id"] = json!("9007199254740993");
    let files = module.project(Budget::default(), &site).expect("a projection");
    let chargers = &files.iter().find(|file| file.name == "charger.json").expect("charger.json").bytes;
    let text = String::from_utf8_lossy(chargers);
    assert!(text.contains("\"charger_id\": 9007199254740993.0,"), "{text}");
    let trips = assert_round_trip(&module, &[files.clone(), files.clone()], &site, None, None);
    assert_eq!(keyed(&trips[0].site, "mapcreator_charger_id", "9007199254740993").id, charger);
    assert_round_trip(&module, &[files.clone(), files], &empty(), None, None);
}

/// A file whose text the adapter could not write back byte for byte is
/// refused at ingest, naming the file and what in its text cannot be written
/// back, rather than given back otherwise without a word: one line indented
/// one space deeper than the rest, lines ended with CR LF, a key written
/// without the space after its colon that every other key has, a slash
/// escaped as `\/`, a number spelled `120.50`, and a zero spelled `-0`.
#[test]
fn a_file_whose_text_cannot_be_written_back_is_refused_by_name() {
    let module = mapcreator();
    let current = fixture("current");
    let older = fixture("older");
    let respelled = |files: &[File], name: &str, from: &str, to: &str| {
        with_text(files, name, |text| {
            assert!(text.contains(from), "{name} has no {from:?}");
            text.replacen(from, to, 1)
        })
    };
    for (files, says) in [
        (
            respelled(&current, "charger.json", "\n    \"charger_id\": 5,", "\n     \"charger_id\": 5,"),
            "charger.json is written in a way this adapter cannot write back byte for byte: its spacing at line \
             3, column 5 is not the spacing it is written in up to there",
        ),
        (
            with_text(&current, "zone.json", |text| text.replace('\n', "\r\n")),
            "zone.json is written in a way this adapter cannot write back byte for byte: it ends its lines with CR \
             LF (`\\r\\n`), first at line 1",
        ),
        (
            respelled(&current, "charger.json", "\"mode\": \"auto\"", "\"mode\":\"auto\""),
            "charger.json is written in a way this adapter cannot write back byte for byte: its spacing at line \
             11, column 12",
        ),
        (
            respelled(&older, "pps_bin.json", "http://bins", "http:\\/\\/bins"),
            "pps_bin.json is written in a way this adapter cannot write back byte for byte: at line 2, column 10, \
             it spells a string `\"http:\\/\\/bins.example.invalid/api/bins\"`, which this adapter writes \
             `\"http://bins.example.invalid/api/bins\"`",
        ),
        (
            respelled(&current, "tote_type.json", "\"rack_width\": 120.5,", "\"rack_width\": 120.50,"),
            "tote_type.json is written in a way this adapter cannot write back byte for byte: at line 4, column \
             19, it spells a number `120.50`, which this adapter writes `120.5`",
        ),
        (
            respelled(&current, "charger.json", "\"charger_direction\": 2,", "\"charger_direction\": -0,"),
            "charger.json is written in a way this adapter cannot write back byte for byte: at line 7, column \
             26, it spells a number `-0`, which this adapter writes `-0.0`",
        ),
    ] {
        let refused = refusal(ingest(&module, &files, &empty()));
        assert!(refused.contains(says), "{says:?} is not in {refused}");
    }
}

/// What the adapter cannot read among the files is refused by name: a file no
/// Map Creator export carries, an export without its `map.json`, and a file
/// that is not JSON.
#[test]
fn an_unknown_file_a_missing_map_and_a_file_that_is_not_json_are_refused_by_name() {
    let module = mapcreator();
    let current = fixture("current");
    let mut extra = current.clone();
    extra.push(File { name: "notes.txt".into(), bytes: b"left by the surveyor".to_vec() });
    let unknown = refusal(ingest(&module, &extra, &empty()));
    assert_eq!(unknown, "notes.txt is not a file any Map Creator export carries");
    let without: Vec<File> = current.iter().filter(|file| file.name != "map.json").cloned().collect();
    assert_eq!(refusal(ingest(&module, &without, &empty())), "the export carries no map.json, so it states no floor");
    let mut broken = current.clone();
    broken.iter_mut().find(|file| file.name == "zone.json").expect("a zone file").bytes = b"[{\"zonerec\":".to_vec();
    let not_json = refusal(ingest(&module, &broken, &empty()));
    assert!(not_json.starts_with("zone.json is not JSON"), "{not_json}");
}

/// The module loads under the policy that defines every import its toolchain
/// linked beyond the contract's own as a trap, and runs both directions
/// without reaching one: a call that did would stop with the host naming the
/// import.
#[test]
fn the_module_runs_both_directions_with_every_import_beyond_its_own_trapped() {
    let engine = Engine::new().expect("an engine");
    let module =
        Module::load(&engine, module_bytes(), ImportPolicy::TrapOnUse, Budget::default()).expect("the module loads");
    let trapped: Vec<&String> = module.imports().iter().filter(|name| !name.starts_with("goworld:adapter/")).collect();
    eprintln!("imports defined as traps: {trapped:?}");
    assert_round_trip(&module, &[fixture("current"), fixture("current-edited")], &empty(), None, Some(&normalized));
}

/// The site every fixture's ingest leaves loads in the reference reader with
/// no error, and holds what the mapping says it must: one cell per record of
/// `map.json`, one link of the traversability graph per stitch, and nothing a
/// generator derived.
#[test]
fn the_site_an_ingest_leaves_loads_in_the_reference_reader() {
    let module = mapcreator();
    for name in FIXTURES {
        let site = site_of(&module, name);
        let (world, _scratch) = loaded(&site);
        let census = world.component_census();
        assert_eq!(census.get("cell").copied(), Some(map_values(&fixture(name)).len()), "{name}");
        let links = world.edges.iter().filter(|edge| edge.bidirectional).count();
        assert_eq!(census.get("stitch").copied().unwrap_or(0), links, "{name}");
        assert_eq!(world.derived_count(), 0, "{name}");
    }
}

/// The site measured in tenths counts tenths of a millimetre, its legacy grid
/// frame is stated in those steps, and the reference reader projects every
/// on-grid cell through that frame onto the square the export records for it.
#[test]
fn the_tenths_site_counts_tenths_and_its_grid_frame_puts_every_cell_on_its_own_square() {
    let module = mapcreator();
    let site = site_of(&module, "tenths");
    let site_frame = keyed(&site, "mapcreator_frame", "site");
    assert_eq!(site_frame.components["frame"]["unit_mm"], json!(0.1));
    let (world, _scratch) = loaded(&site);
    let grid: EntityId = keyed(&site, "mapcreator_frame", "legacy-grid").id.parse().expect("an identity");
    let mut cells = 0;
    for entity in site.entities.iter().filter(|entity| entity.components.contains_key("cell")) {
        let pose = &entity.components["pose"];
        let frame: EntityId = pose["frame"].as_str().expect("a frame").parse().expect("an identity");
        let at = |axis: usize| Coord(pose["position"][axis].as_i64().expect("a whole number of steps"));
        let primary = world.frames.to_primary(&frame, [at(0), at(1)]).expect("the cell resolves");
        let square = world.frames.from_primary(&grid, primary).expect("the cell projects");
        let recorded = entity.components["external_ids"]["mapcreator_grid_coordinate"].as_str().expect("a key");
        assert_eq!(format!("[{},{}]", square[0].0, square[1].0), recorded, "{}", entity.id);
        cells += 1;
    }
    assert_eq!(cells, 6);

    // A station and its presentation point stand on the cells they name, at a
    // tenth: each pose is its cell's, step for step.
    let pose_of = |entity: &Entity| entity.components["pose"]["position"].clone();
    let station = keyed(&site, "mapcreator_pps_id", "81");
    let point = site
        .entities
        .iter()
        .find(|e| e.components["external_ids"].get("mapcreator_point_id").is_some())
        .expect("a point");
    assert_eq!(pose_of(station), pose_of(keyed(&site, "mapcreator_grid_coordinate", "[2,1]")));
    assert_eq!(pose_of(point), pose_of(keyed(&site, "mapcreator_grid_coordinate", "[2,2]")));
    assert_eq!(pose_of(station), json!([35013, 17506]));
}

/// The fixtures between them write every component the import writes — each
/// of the components under `vocabulary/` and each core one —
/// and no other, so no record kind goes untested.
#[test]
fn the_fixtures_between_them_write_every_component_the_import_writes() {
    let module = mapcreator();
    let mut written: BTreeSet<String> = BTreeSet::new();
    for name in FIXTURES {
        for components in declared(&ingest(&module, &fixture(name), &empty()).expect("an ingest")) {
            written.extend(components.keys().cloned());
        }
    }
    let expected: BTreeSet<String> = vocabulary().into_keys().chain(CORE.iter().map(|name| name.to_string())).collect();
    assert_eq!(written, expected);
}

/// Every reference a fixture's ingest writes at a path its definition
/// annotates — one of the vocabulary's, or one the seed publishes with a core
/// component the import writes Map Creator's fields into — is one the server's
/// reference gate resolves: an entity the same ingest declares, carrying the
/// component the annotation expects where it names one. A legacy value at an
/// annotated path would be refused by the server as no reference.
#[test]
fn every_annotated_reference_an_ingest_writes_names_an_entity_it_declares() {
    let module = mapcreator();
    let mut annotations: BTreeMap<String, Value> =
        vocabulary().into_iter().map(|(name, definition)| (name, definition["refs"].clone())).collect();
    annotations.extend(seed_refs().into_iter().filter(|(name, _)| EXTENDED.contains(&name.as_str())));
    let mut resolved = 0usize;
    for name in FIXTURES {
        let changes = ingest(&module, &fixture(name), &empty()).expect("an ingest");
        let declared: BTreeMap<&str, &Map<String, Value>> = changes
            .iter()
            .filter_map(|change| match change {
                Change::Declare(write) => Some((write.id.as_str(), &write.components)),
                _ => None,
            })
            .collect();
        for (id, components) in &declared {
            for (component, payload) in *components {
                let Some(refs) = annotations.get(component) else { continue };
                for (path, named, expects) in annotated_references(payload, refs) {
                    let target = named.as_str().and_then(|target| declared.get(target));
                    assert!(
                        target.is_some_and(|target| expects.is_none_or(|expects| target.contains_key(expects))),
                        "{name}: entity {id} holds {named} at `{component}{path}`, which names no entity of the \
                         ingest carrying `{expects:?}`"
                    );
                    resolved += 1;
                }
            }
        }
    }
    assert_eq!(resolved, 164, "the number of annotated references the fixtures write moved");
}

/// Every component every fixture's ingest writes validates against its
/// schema: one of its own vocabulary against its file under `vocabulary/`, a core
/// one against its test schema.
#[test]
fn every_component_the_adapter_writes_validates_against_its_schema() {
    let module = mapcreator();
    let mut validators: BTreeMap<String, Validator> =
        vocabulary().into_iter().map(|(name, definition)| (name, compiled(&definition["schema"]))).collect();
    validators.extend(seed_schemas().into_iter().map(|(name, schema)| (name, compiled(&schema))));
    let mut validated: BTreeMap<String, usize> = BTreeMap::new();
    for name in FIXTURES {
        for components in declared(&ingest(&module, &fixture(name), &empty()).expect("an ingest")) {
            for (component, payload) in components {
                let validator = validators.get(component).unwrap_or_else(|| panic!("no schema for {component}"));
                if let Err(refusal) = validator.validate(payload) {
                    panic!("{name}: a `{component}` payload is refused by its schema: {refusal}\n{payload}");
                }
                *validated.entry(component.clone()).or_default() += 1;
            }
        }
    }
    assert_eq!(validated.len(), validators.len(), "every schema validated something: {validated:?}");
}
