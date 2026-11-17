//! The two round-trip laws, and the crate's other claims, over the five
//! production site exports, lettered `site-a` to `site-e`, through the built
//! component.
//!
//! These are the adapter's whole claim: give it a site's export, and the export
//! comes back. Everything else — the field maps, the vocabulary's components, the
//! lenses — is machinery in service of that, so the laws run over the five
//! production exports and not only over fixtures: a fixture is a thing somebody
//! wrote to pass, and a site export is a thing that was there first.
//!
//! The exports are production data and are not in the repository, so every
//! test here is ignored by an ordinary run and reads the exports from the
//! directory `MAPCREATOR_DUMPS` names when asked for with `--ignored`. Asked for
//! with the variable unset, each fails naming it: a test that returned early
//! would count as passed, having run nothing. The calls run under
//! [`PRODUCTION`], whose doc comment holds the measurements it is sized from.

use mapcreator_adapter_tests::{
    by_op, edited_in_place, file_json, graph_of, ingested, keyed, lens_library, loaded, map_values, mapcreator,
    millimetre_pair, module_bytes, naming, normalized, production_export, spell_millimetres, unresolved_fields,
    with_every_cell_shifted, with_file, with_map_values, with_one_more_cell, without_one_cell, Ordered, PRODUCTION,
    SITES, UNRESOLVED,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use worldadapt_host::{AdapterError, Budget, Change, Entity, File, Module, RandomIds, Site};
use worldadapt_testkit::{apply, round_trip_within, Deleted};
use worldlens::Matcher;

/// What a production export's calls run under while they are measured: far
/// more than any of them spends, so that a measurement is never cut short.
const MEASURING: Budget =
    Budget { fuel: 1_000_000_000_000, run_fuel: 1_000_000_000_000, memory_bytes: 3 << 30, fresh_ids: 1_000_000 };

/// The site's own scope.
const OWN: &str = "tenant/site";

/// The export the edits are made to, as the crate's own edit tests made them:
/// one floor of 1,867 cells, ten of them off the grid.
const EDITED: &str = "site-c";

/// The reader warnings each site's ingested site is expected to raise, as a
/// phrase of each. One, and it is a fact about a site rather than about the
/// adapter: two of site-d's cells sit close enough together that the nominal
/// legacy grid puts them in one square, and no stitch joins them. Each keeps
/// its own measured position and its own recorded coordinate, so nothing is
/// lost; what the frame cannot do is stand in for them.
const NAMED_WARNINGS: [(&str, &[&str]); 5] = [
    ("site-a", &[]),
    ("site-b", &[]),
    ("site-c", &[]),
    ("site-d", &["folds 2 cells into 1 squares that no stitch joins"]),
    ("site-e", &[]),
];

/// An identity a test writes down for an entity it adds to a site by hand.
const TWIN: &str = "c1000000-0000-4000-8000-000000000001";

fn empty() -> Site {
    Site { scope: OWN.into(), entities: Vec::new() }
}

fn ingest(module: &Module, files: &[File], site: &Site) -> Vec<Change> {
    module.ingest(PRODUCTION, files, site, Box::new(RandomIds)).unwrap_or_else(|error| panic!("{error}"))
}

fn refusal<T: std::fmt::Debug>(result: Result<T, AdapterError>) -> String {
    match result {
        Err(AdapterError::Refused { message, .. }) => message,
        other => panic!("expected the adapter's own refusal, got {other:?}"),
    }
}

/// The site one export's first ingest into an empty site leaves.
fn site_of(module: &Module, site: &str) -> Site {
    ingested(module, &production_export(site), PRODUCTION, OWN)
}

/// The laws hold through an edit: the export, then the edited export, each
/// ingested into the site the one before left.
fn laws_hold_through(module: &Module, files: &[File], edited: &[File]) {
    round_trip_within(module, &[files.to_vec(), edited.to_vec()], &empty(), None, Some(&normalized), PRODUCTION)
        .unwrap_or_else(|broken| panic!("{broken}"));
}

fn entity<'s>(site: &'s Site, id: &str) -> &'s Entity {
    site.entities.iter().find(|entity| entity.id == id).unwrap_or_else(|| panic!("no entity {id}"))
}

/// How many of a site's entities carry one component: a plain scan that never
/// mentions the lens engine.
fn scan(site: &Site, component: &str) -> usize {
    site.entities.iter().filter(|entity| entity.components.contains_key(component)).count()
}

/// How many entities carry one component and one of this adapter's lookup key
/// fields: what a lens matching a shared component counts, since another of
/// this adapter's kinds may carry the same component.
fn scan_keyed(site: &Site, component: &str, key: &str) -> usize {
    site.entities
        .iter()
        .filter(|entity| entity.components.contains_key(component))
        .filter(|entity| entity.components.get("external_ids").is_some_and(|ids| ids.get(key).is_some()))
        .count()
}

/// One entity's formatting entry under `key`, as the import wrote it.
fn formatting<'e>(components: &'e Map<String, Value>, key: &str) -> Option<&'e Map<String, Value>> {
    components.get("import_formatting")?.get(key)?.as_object()
}

/// The legacy grid frame's transform, and its x intercept and pitch, off one
/// entity's components.
fn grid_fit(components: &Map<String, Value>) -> (Value, f64, f64) {
    let transform = components["frame"]["transform_to_parent"].clone();
    let x = |key: &str| transform[key][0].as_f64().expect("an x axis");
    let (intercept, pitch) = (x("translation"), x("scale"));
    (transform, intercept, pitch)
}

/// Each export ingested into an empty site declares one entity per record, and
/// as many as the crate this adapter was ported from wrote.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_production_export_declares_what_the_crate_wrote() {
    let module = mapcreator();
    for (site, declared) in SITES {
        let changes = ingest(&module, &production_export(site), &empty());
        assert!(changes.iter().all(|change| matches!(change, Change::Declare(_))), "{site}");
        assert_eq!(changes.len(), declared, "{site}");
    }
}

/// Each export's references that name nothing in it are exactly as many as
/// the table beside `SITES` says, so one that stops resolving is seen.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_production_export_keeps_as_many_references_unresolved_as_pinned() {
    let module = mapcreator();
    let counted: Vec<(&str, usize)> =
        UNRESOLVED.iter().map(|(site, _)| (*site, unresolved_fields(&site_of(&module, site)).len())).collect();
    assert_eq!(counted, UNRESOLVED);
}

/// **Both laws**, on every export: ingested into an empty site, applied and
/// projected, the same files come back, the same names and each equal under
/// the crate's normalization; and ingested again into the site that resulted,
/// nothing changes — every identity looked up, none drawn afresh.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn both_laws_hold_on_every_production_export() {
    let module = mapcreator();
    for (site, _) in SITES {
        round_trip_within(&module, &[production_export(site)], &empty(), None, Some(&normalized), PRODUCTION)
            .unwrap_or_else(|broken| panic!("{site}: {broken}"));
    }
}

/// **Byte for byte**: every file of every export comes back as the very bytes
/// it was given — key order, spacing and a whole number written `40.0`
/// included, which the laws' normalization forgives — after a first ingest
/// into an empty site, and after a second ingest of the same files into the
/// site the first left.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_production_export_comes_back_byte_for_byte() {
    let module = mapcreator();
    for (site, _) in SITES {
        let files = production_export(site);
        round_trip_within(&module, &[files.clone(), files], &empty(), None, None, PRODUCTION)
            .unwrap_or_else(|broken| panic!("{site}: {broken}"));
    }
}

/// What each call of each export's round trip spends, printed for the record
/// the budget is sized from: run with `--nocapture`.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn what_each_production_call_spends() {
    let module = mapcreator();
    eprintln!("the module: {} bytes", module_bytes().len());
    for (site, _) in SITES {
        let files = production_export(site);
        let (first, ingested) = module
            .ingest_spending(MEASURING, &files, &empty(), Box::new(RandomIds))
            .unwrap_or_else(|error| panic!("{site}: {error}"));
        let held = apply(&empty(), &first, &mut Deleted::default()).expect("the changes apply");
        let (written, projected) =
            module.project_spending(MEASURING, &held).unwrap_or_else(|error| panic!("{site}: {error}"));
        let (again, settled) = module
            .ingest_spending(MEASURING, &files, &held, Box::new(RandomIds))
            .unwrap_or_else(|error| panic!("{site}: {error}"));
        eprintln!(
            "{site}: {} entities; first ingest {} changes, fuel {}, memory {}; project {} files, fuel {}, memory \
             {}; second ingest {} changes, fuel {}, memory {}",
            held.entities.len(),
            first.len(),
            ingested.fuel,
            ingested.memory_bytes,
            written.len(),
            projected.fuel,
            projected.memory_bytes,
            again.len(),
            settled.fuel,
            settled.memory_bytes
        );
    }
}

/// The site every export's ingest leaves loads in the reference reader with no
/// error and only the warnings named above, and holds what the mapping rules
/// say it must, worked out from the files by plain arithmetic: one cell per
/// record of `map.json`, a sector per sector key the cells state and one per
/// aisle, a queue per queue block, a station component wherever the station's
/// kind is decidable, a presentation point per point, a robot class per bot
/// type the profiles name, a traversability link per stitch, and nothing a
/// generator derived.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_production_site_loads_in_the_reader_and_holds_what_the_mapping_predicts() {
    let module = mapcreator();
    for (site, named) in NAMED_WARNINGS {
        let files = production_export(site);
        let ingested = site_of(&module, site);
        let (world, _scratch) = loaded(&ingested);
        let raised: Vec<String> = world.warnings.iter().map(|warning| warning.to_string()).collect();
        assert_eq!(raised.len(), named.len(), "{site}: the reader raised {raised:?}, and {named:?} are named");
        for (warning, phrase) in raised.iter().zip(named.iter()) {
            assert!(warning.contains(phrase), "{site}: the reader raised `{warning}`, which no named warning matches");
        }

        let census = world.component_census();
        let count = |component: &str| census.get(component).copied().unwrap_or(0);
        let cells = map_values(&files);
        let file = |name: &str| files.iter().any(|f| f.name == name).then(|| file_json(&files, name));
        let sector_keys: BTreeSet<String> = cells.iter().filter_map(|cell| cell.get("sector")).map(scalar).collect();
        let aisles = file("vtm_zone.json")
            .and_then(|zones| zones.get("aisle_info").and_then(Value::as_array).map(Vec::len))
            .unwrap_or(0);
        let queues = file("block.json")
            .and_then(|blocks| blocks.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|block| matches!(block["entity_type"].as_str(), Some("queue" | "sequenced_queue")))
            .count();
        let stations = file("pps.json").and_then(|stations| stations.as_array().cloned()).unwrap_or_default();
        let decidable = stations.iter().filter(|station| kind_is_decidable(station)).count();
        let points: usize = stations
            .iter()
            .map(|station| {
                let listed = |pointer: &str| station.pointer(pointer).and_then(Value::as_array).map_or(0, Vec::len);
                listed("/entities/presentation_points") + listed("/pps_points")
            })
            .sum();
        let bot_types: BTreeSet<String> = file("map.json")
            .and_then(|map| map[0].get("location_profiles").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|profile| profile.get("bot_orientation_details").and_then(Value::as_object))
            .flat_map(|details| details.keys().cloned())
            .collect();
        assert_eq!(count("cell"), cells.len(), "{site}: cells");
        assert_eq!(count("sector"), sector_keys.len() + aisles, "{site}: sectors");
        assert_eq!(count("queue"), queues, "{site}: queues");
        assert_eq!(count("station"), decidable, "{site}: stations");
        assert_eq!(count("presentation_point"), points, "{site}: presentation points");
        assert_eq!(count("robot_class"), bot_types.len(), "{site}: robot classes");
        assert_eq!(count("stitch"), world.edges.iter().filter(|edge| edge.bidirectional).count(), "{site}: links");
        assert_eq!(world.entities.len(), ingested.entities.len(), "{site}: entities");
        assert_eq!(world.derived_count(), 0, "{site}: derived entities");
    }
}

/// A JSON scalar as the string the export would print.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Whether SPEC §10's one station kind is decidable from a station's record: a
/// type naming induction or extraction decides it, and otherwise exactly one of
/// pick and put among its modes does.
fn kind_is_decidable(station: &Value) -> bool {
    let kind = station["type"].as_str().unwrap_or_default().to_ascii_lowercase();
    if kind.contains("induct") || kind.contains("extract") {
        return true;
    }
    let modes = station["allowed_modes"].as_array().cloned().unwrap_or_default();
    modes.iter().filter(|mode| matches!(mode.as_str(), Some("pick" | "put"))).count() == 1
}

/// **The counts, double-entry.** Every lens's object count is counted twice:
/// once by the engine running the adapter's own lens text, and once by a plain
/// scan over the site's entities that never mentions the engine.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_lens_counts_what_a_plain_scan_counts() {
    let module = mapcreator();
    let library = lens_library();
    let definitions = mapcreator_adapter_tests::vocabulary();
    for (site, _) in SITES {
        let ingested = site_of(&module, site);
        let graph = graph_of(&ingested);
        let matcher = Matcher::over(&graph);
        for lens in library.lenses() {
            // A lens is named for its component: `mapcreator_zone` reads the
            // shared `zone`, `mapcreator_station` the still-prefixed
            // `mapcreator.station`; and `mapcreator_sector` the core `sector` an
            // aisle carries too, `mapcreator_cell` the core `cell`, and
            // `mapcreator_presentation_point` the core `presentation_point` a v2
            // point carries empty, on the entities their keys name.
            let own = lens.name().strip_prefix("mapcreator_").expect("a Map Creator lens");
            let scanned = match own {
                "sector" => scan_keyed(&ingested, "sector", "mapcreator_sector"),
                "cell" => scan_keyed(&ingested, "cell", "mapcreator_grid_coordinate"),
                "presentation_point" => scan_keyed(&ingested, "presentation_point", "mapcreator_point_id"),
                own if definitions.contains_key(own) => scan(&ingested, own),
                own => scan(&ingested, &format!("mapcreator.{own}")),
            };
            let engine = matcher.project(lens).len();
            assert_eq!(engine, scanned, "{site}: `{}` and a plain scan disagree", lens.name());
        }
    }
}

/// A station's object folds on its bins, its operator seats and its put-to-light
/// modules, not only its points and queues. `site-e` is the only export with a
/// station interior: ten stations, 40 bins, 20 seats and 80 lights —
/// twice as many lights as bins, because `pptl.json` lists its devices in twenty
/// groups, the same ten stations twice over.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_station_folds_its_bins_its_seats_and_its_lights() {
    let module = mapcreator();
    let ingested = site_of(&module, "site-e");
    let graph = graph_of(&ingested);
    let stations = Matcher::over(&graph).project(lens_library().lens("mapcreator_station").expect("the station lens"));
    assert_eq!(stations.len(), 10, "stations");
    let folded = |field: &str| -> usize { stations.iter().map(|s| s.fields().get(field).map_or(0, Vec::len)).sum() };
    assert_eq!((folded("bins"), folded("seats"), folded("lights")), (40, 20, 80));
    for station in &stations {
        for field in ["bins", "seats", "lights"] {
            assert!(station.fields().get(field).is_some_and(|parts| !parts.is_empty()), "a station folded no {field}");
        }
    }
}

/// A bin of a station's put wall reaches its bin type and both its operator
/// seats by identity, and every one of those is an entity of the site: the
/// files that state them are read after the bins, and the references are made
/// once every record has its identity.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_bin_reaches_its_type_and_its_seats_by_identity() {
    let module = mapcreator();
    let ingested = site_of(&module, "site-e");
    let ids: BTreeSet<&str> = ingested.entities.iter().map(|entity| entity.id.as_str()).collect();
    let bins: Vec<&Entity> =
        ingested.entities.iter().filter(|entity| entity.components.contains_key("pps_bin")).collect();
    assert_eq!(bins.len(), 40, "put-wall bins");
    for bin in bins {
        let component = &bin.components["pps_bin"];
        for field in ["station", "bin_type", "front_seat", "back_seat"] {
            let named = component[field].as_str().unwrap_or_else(|| panic!("a bin's `{field}` names nothing"));
            assert!(ids.contains(named), "a bin's `{field}` names {named}, which is no entity of the site");
        }
    }
}

/// A cell names its location profile by identity, and a cell with no profile
/// names nothing.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_cell_names_its_location_profile_by_identity() {
    let module = mapcreator();
    let mut without = 0usize;
    for (site, _) in SITES {
        let ingested = site_of(&module, site);
        let profiles: BTreeSet<&str> = ingested
            .entities
            .iter()
            .filter(|entity| entity.components.contains_key("location_profile"))
            .map(|entity| entity.id.as_str())
            .collect();
        let mut referencing = 0usize;
        for cell in ingested.entities.iter().filter(|entity| entity.components.contains_key("cell")) {
            let component = &cell.components["cell"];
            match component.get("location_profile").and_then(Value::as_str) {
                Some(named) => {
                    assert!(profiles.contains(named), "{site}: a cell names {named}, which is no profile entity");
                    referencing += 1;
                }
                None => {
                    assert!(component.get("location_profile_unresolved").is_none(), "{site}: an undeclared profile");
                    without += 1;
                }
            }
        }
        if site == "site-b" {
            assert_eq!(referencing, 3_140, "site-b's profiled cells");
        }
    }
    assert_eq!(without, 5_148, "cells with no location profile, across the five sites");
}

/// Two entities of the site under one lookup key are an ambiguity the ingest
/// refuses by naming both, rather than moving an identity by guess: two robot
/// classes under one bot version, and two cells under one grid coordinate.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn two_entities_under_one_lookup_key_are_refused_by_name() {
    let module = mapcreator();
    for (site, component) in [("site-b", "robot_class"), ("site-c", "cell")] {
        let mut ingested = site_of(&module, site);
        let original =
            ingested.entities.iter().find(|entity| entity.components.contains_key(component)).expect("one").clone();
        ingested.entities.push(Entity { id: TWIN.into(), ..original.clone() });
        let refused = refusal(module.ingest(PRODUCTION, &production_export(site), &ingested, Box::new(RandomIds)));
        for named in [original.id.as_str(), TWIN, "refuses"] {
            assert!(refused.contains(named), "{site}: {named:?} in {refused}");
        }
    }
}

/// A field family nobody has disposed of is refused, naming the path.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn an_export_with_a_key_path_nobody_has_disposed_of_is_refused() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let mut map = file_json(&files, "map.json");
    map[0]["gantry_calibration"] = json!({ "offset": 3 });
    let refused =
        refusal(module.ingest(PRODUCTION, &with_file(&files, "map.json", &map), &empty(), Box::new(RandomIds)));
    assert!(refused.contains("map.json:[].gantry_calibration (once)"), "{refused}");
}

/// The ordering key is written at the root of a record's formatting entry and
/// nowhere else: not in a component that states what the site is, and not
/// inside an object of a record, since the objects inside a record are not
/// records and an open schema would accept a nested key, so this is what
/// catches one.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn the_ordering_key_is_written_in_a_records_formatting_and_nowhere_else() {
    fn nested(value: &Value, at: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let path = format!("{at}.{key}");
                    if key == "index" {
                        out.push(path.clone());
                    }
                    nested(child, &path, out);
                }
            }
            Value::Array(items) => {
                items.iter().enumerate().for_each(|(i, item)| nested(item, &format!("{at}[{i}]"), out))
            }
            _ => {}
        }
    }
    let module = mapcreator();
    for (site, _) in SITES {
        let mut roots = 0usize;
        let mut misplaced: Vec<String> = Vec::new();
        for entity in site_of(&module, site).entities {
            for (component, payload) in &entity.components {
                let mut found = Vec::new();
                nested(payload, component, &mut found);
                for path in found {
                    let in_an_entry = component == "import_formatting"
                        && path.strip_prefix("import_formatting.").is_some_and(|rest| rest.matches('.').count() == 1);
                    if in_an_entry {
                        roots += 1;
                    } else {
                        misplaced.push(format!("{}'s `{path}`", entity.id));
                    }
                }
            }
        }
        assert!(roots > 0, "{site}: no component states an ordering key, so this test proves nothing");
        assert!(
            misplaced.is_empty(),
            "{site}: ordering keys where nothing orders anything: {:?}",
            &misplaced[..3.min(misplaced.len())]
        );
    }
}

/// A cell stating a floor other than its file's own is refused, naming the
/// cell and both floors: one export is one floor, and the cell's pose would
/// put it on the file's floor while the cell says another.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_cell_on_another_floor_than_its_file_is_refused() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let mut map = file_json(&files, "map.json");
    let stated = map[0]["floor_id"].as_i64().expect("the file states a floor");
    map[0]["map_values"][7]["floor"] = json!(stated + 1);
    let empty = Site { scope: OWN.into(), entities: Vec::new() };
    let refused = refusal(module.ingest(PRODUCTION, &with_file(&files, "map.json", &map), &empty, Box::new(RandomIds)));
    let says = format!("states `floor` {}, while the file states `floor_id` {stated}", stated + 1);
    assert!(refused.contains(&says), "{says:?} in {refused}");
}

/// A v3 station with neither served queues nor a system is written back in
/// the v3 spelling: the dialect is the one the import recorded, not a guess
/// from the fields a record happens to carry.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_v3_station_with_no_served_queues_and_no_system_stays_v3() {
    let module = mapcreator();
    let mut ingested = site_of(&module, "site-a");
    let mut stripped = 0usize;
    for entity in &mut ingested.entities {
        let v3 = formatting(&entity.components, "mapcreator_station")
            .is_some_and(|written| written.get("dialect_shape").and_then(Value::as_str) == Some("v3"));
        let Some(station) = entity.components.get_mut("mapcreator.station").and_then(Value::as_object_mut) else {
            continue;
        };
        if v3 {
            station.remove("served_by");
            station.remove("system");
            stripped += 1;
        }
    }
    assert!(stripped > 0, "no v3 station to strip");
    let files = module.project(PRODUCTION, &ingested).expect("the projection");
    let stations = file_json(&files, "pps.json");
    let stations = stations.as_array().expect("pps.json");
    assert_eq!(stations.len(), stripped, "one record per station");
    for station in stations {
        assert!(station.get("entities").is_some(), "a v3 station came back without `entities`");
        assert!(station.get("pps_points").is_none(), "a v3 station came back with the v2 point list");
        assert!(station.get("location").is_some_and(Value::is_array), "a v3 `location` is a raw grid pair");
    }
}

/// Every field a component's schema requires is one the projection refuses to
/// write a record without: stripped from every entity carrying its component,
/// on the first export whose site carries one, the projection refuses rather
/// than writing a record it cannot spell.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_required_field_is_one_the_projection_refuses_without() {
    let module = mapcreator();
    let sites: Vec<Site> = SITES.iter().map(|(site, _)| site_of(&module, site)).collect();
    // Each as the component, the entry of the formatting it is in where it is
    // in one, and the field: a formatting entry states its own `required`.
    let mut demanded: Vec<(String, Option<String>, String)> = Vec::new();
    for (component, definition) in mapcreator_adapter_tests::vocabulary() {
        let schema = &definition["schema"];
        for field in schema["required"].as_array().cloned().unwrap_or_default() {
            demanded.push((component.clone(), None, field.as_str().expect("a field").to_string()));
        }
        if component == "import_formatting" {
            for (key, entry) in schema["properties"].as_object().expect("the entries") {
                for field in entry["required"].as_array().cloned().unwrap_or_default() {
                    let field = field.as_str().expect("a field").to_string();
                    demanded.push((component.clone(), Some(key.clone()), field));
                }
            }
        }
    }
    // 29 since a presentation point's `station` is the shared `presentation_point`'s,
    // which other producers write without one: the adapter's own table still
    // refuses a Map Creator point without it, which a fixture test strips.
    assert_eq!(demanded.len(), 29, "the required fields across the vocabulary: {demanded:?}");
    /// The object one demanded field is in: the component, or its entry.
    fn holder<'p>(payload: &'p mut Value, entry: Option<&str>) -> Option<&'p mut Map<String, Value>> {
        match entry {
            Some(key) => payload.get_mut(key)?.as_object_mut(),
            None => payload.as_object_mut(),
        }
    }
    for (component, entry, field) in demanded {
        let carries = |site: &&Site| {
            site.entities.iter().any(|entity| {
                let payload = entity.components.get(&component);
                entry.as_ref().map_or(payload.is_some(), |key| payload.and_then(|p| p.get(key)).is_some())
            })
        };
        let ingested = sites.iter().find(carries).unwrap_or_else(|| panic!("no export's site carries `{component}`"));
        let mut broken = ingested.clone();
        let mut stripped = 0usize;
        for entity in &mut broken.entities {
            if let Some(payload) = entity.components.get_mut(&component).and_then(|p| holder(p, entry.as_deref())) {
                stripped += usize::from(payload.remove(&field).is_some());
            }
        }
        assert!(stripped > 0, "no `{component}` carrying `{field}`, so this proves nothing");
        let refused = refusal(module.project(PRODUCTION, &broken));
        assert!(refused.contains(&field), "{refused}");
    }
}

/// A dock point whose barcode differs from its id comes back keeping both: the
/// barcode is written from the id only where the two agree.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_dock_point_whose_barcode_differs_from_its_id_keeps_both() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let mut rows = file_json(&files, "dock_point.json");
    rows[0]["dock_barcode"] = json!("dock-restickered");
    laws_hold_through(&module, &files, &with_file(&files, "dock_point.json", &rows));
}

/// Deleting one cell deletes that cell and the stitches it was part of, patches
/// exactly the entities that named it, and rewrites no other cell.
///
/// This is what the record ordering key exists for. When a record's order rode
/// as its position in the file, deleting one cell rewrote 1,766 others, each
/// moved up one place.
///
/// The edits here and in the tests below are made in place, as Map Creator
/// writes an edit (`edited_in_place`): every object keeps its keys in its own
/// order and the file keeps its separators, so an ingest has nothing to patch
/// that the edit did not change. The export entity is not patched: the cell
/// that goes leaves a set of keys other cells still state.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn deleting_one_production_cell_deletes_it_and_its_stitches_alone() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let site = ingested(&module, &files, PRODUCTION, OWN);
    let (coordinate, edited) = without_one_cell(&files, 900);
    let gone = keyed(&site, "mapcreator_grid_coordinate", &coordinate).id.clone();
    let named_by = naming(&site, &gone);
    let stitches: BTreeSet<String> =
        named_by.iter().filter(|id| entity(&site, id).components.contains_key("stitch")).cloned().collect();
    assert!(!stitches.is_empty(), "the cell chosen is in no stitch, so the test proves less than it says");

    let changes = ingest(&module, &edited, &site);
    assert!(by_op(&changes, "declare").is_empty(), "{changes:?}");
    let deleted: BTreeSet<String> = by_op(&changes, "delete").into_iter().map(str::to_string).collect();
    let expected: BTreeSet<String> = stitches.iter().cloned().chain([gone.clone()]).collect();
    assert_eq!(deleted, expected, "the entities a one-cell deletion removes");
    let patched: BTreeSet<String> = by_op(&changes, "patch").into_iter().map(str::to_string).collect();
    let referencing: BTreeSet<String> = named_by.difference(&stitches).cloned().collect();
    assert_eq!(patched, referencing, "the entities a one-cell deletion rewrites are those that named the cell");
    assert!(patched.iter().all(|id| !entity(&site, id).components.contains_key("cell")), "{patched:?}");
    laws_hold_through(&module, &files, &edited);
}

/// Inserting one cell declares one entity, under a key between its
/// neighbours', and rewrites no other cell.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn inserting_one_production_cell_declares_one_entity_and_rewrites_no_other_cell() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let site = ingested(&module, &files, PRODUCTION, OWN);
    let (coordinate, edited) = with_one_more_cell(&files, 900);

    let changes = ingest(&module, &edited, &site);
    assert!(by_op(&changes, "delete").is_empty(), "{changes:?}");
    let declared: Vec<&Change> = changes.iter().filter(|change| change.op() == "declare").collect();
    let [Change::Declare(cell)] = declared.as_slice() else { panic!("expected one declare, got {declared:?}") };
    assert_eq!(cell.components["external_ids"]["mapcreator_grid_coordinate"], json!(coordinate));
    let export = keyed(&site, "mapcreator_export", "site").id.clone();
    for change in &changes {
        let Change::Patch(patch) = change else { continue };
        if patch.id == export {
            continue;
        }
        assert!(!entity(&site, &patch.id).components.contains_key("cell"), "a cell was rewritten");
        let mentions = serde_json::to_string(&patch.components).expect("JSON").contains(&cell.id);
        assert!(mentions, "{} was rewritten and does not name the new cell", patch.id);
    }
    let key = cell.components["import_formatting"]["mapcreator_cell"]["index"].as_i64().expect("an ordering key");
    let keys: Vec<i64> = site
        .entities
        .iter()
        .filter_map(|entity| formatting(&entity.components, "mapcreator_cell")?.get("index")?.as_i64())
        .collect();
    assert!(keys.iter().any(|other| *other < key) && keys.iter().any(|other| *other > key), "{key}");
    laws_hold_through(&module, &files, &edited);
}

/// Changing one field of one record patches that record's entity, naming the
/// one component the field lives in, and nothing else.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn changing_one_production_record_patches_that_record_alone() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let site = ingested(&module, &files, PRODUCTION, OWN);
    let mut pps_id = String::new();
    let edited = edited_in_place(&files, "pps.json", |stations| {
        let station = &mut stations.items_mut().expect("a list of stations")[0];
        let direction = station.get("pick_direction").and_then(Ordered::value).and_then(Value::as_i64);
        let direction = direction.expect("a v2 station states its pick direction");
        station.set("pick_direction", Ordered::of(&json!((direction + 1) % 4)));
        pps_id = match station.get("pps_id").and_then(Ordered::value) {
            Some(Value::String(text)) => text.clone(),
            other => other.expect("a station states its pps_id").to_string(),
        };
    });

    let changes = ingest(&module, &edited, &site);
    let [Change::Patch(patch)] = changes.as_slice() else { panic!("expected one patch, got {changes:?}") };
    assert_eq!(patch.id, keyed(&site, "mapcreator_pps_id", &pps_id).id);
    assert_eq!(patch.components.keys().collect::<Vec<_>>(), ["mapcreator.station"]);
    laws_hold_through(&module, &files, &edited);
}

/// The legacy grid frame survives a one-cell edit untouched, and is refitted
/// when the whole floor moves.
///
/// Its transform is a least-squares fit over every on-grid cell, so it moves
/// whenever any cell does, and it is the frame every cell's grid projection
/// goes through. A transform that still places every cell in the same square
/// is kept; one that no longer does is refitted.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_one_cell_edit_leaves_the_production_grid_frame_alone_and_a_moved_floor_refits_it() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let site = ingested(&module, &files, PRODUCTION, OWN);
    let grid = keyed(&site, "mapcreator_frame", "legacy-grid");
    let (held, intercept, pitch) = grid_fit(&grid.components);
    let (_, deleted) = without_one_cell(&files, 900);
    for edited in [deleted.clone(), with_one_more_cell(&files, 900).1] {
        let changes = ingest(&module, &edited, &site);
        assert!(!by_op(&changes, "patch").contains(grid.id.as_str()), "a one-cell edit rewrote the grid frame");
    }

    // Keeping it is load-bearing: fitted afresh, the edited export's transform
    // is a different one.
    let fresh = ingested(&module, &deleted, PRODUCTION, OWN);
    let (refitted, _, _) = grid_fit(&keyed(&fresh, "mapcreator_frame", "legacy-grid").components);
    assert_ne!(held, refitted, "the fit does not move on a one-cell deletion, so this test proves nothing");

    // The whole floor moved two metres on x: the intercept follows the floor
    // and the pitch is the pitch it was.
    let changes = ingest(&module, &with_every_cell_shifted(&files, 2_000.0), &site);
    let moved = changes
        .iter()
        .find_map(|change| match change {
            Change::Patch(patch) if patch.id == grid.id => Some(patch),
            _ => None,
        })
        .expect("the grid frame is refitted");
    let (_, after, after_pitch) = grid_fit(&moved.components);
    assert!((after - (intercept + 2_000.0)).abs() < 0.01, "the x intercept went from {intercept} to {after}");
    assert!((after_pitch - pitch).abs() < 1e-9, "the x pitch went from {pitch} to {after_pitch}");
}

/// An off-grid cell that lands in a different square refits the frame, even
/// when every on-grid cell keeps its square: the fit reads on-grid cells only,
/// and the reader projects every cell.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn an_off_grid_production_cell_in_a_different_square_refits_the_grid_frame() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let site = ingested(&module, &files, PRODUCTION, OWN);
    let grid = keyed(&site, "mapcreator_frame", "legacy-grid");
    let (_, held_intercept, held_pitch) = grid_fit(&grid.components);
    let (_, deleted) = without_one_cell(&files, 900);
    let fresh = ingested(&module, &deleted, PRODUCTION, OWN);
    let (_, fresh_intercept, fresh_pitch) = grid_fit(&keyed(&fresh, "mapcreator_frame", "legacy-grid").components);
    let square = |intercept: f64, pitch: f64, world: f64| ((world - intercept) / pitch + 0.5).floor() as i64;
    let apart = (-60_000..60_000)
        .map(f64::from)
        .find(|world| square(held_intercept, held_pitch, *world) != square(fresh_intercept, fresh_pitch, *world))
        .expect("no whole millimetre of this floor is in a different square under the two fits");

    let mut records = map_values(&deleted);
    let off_grid = records
        .iter()
        .position(|record| record["coordinate"].as_str().is_some_and(|c| c.starts_with("[999,")))
        .expect("the export states off-grid cells");
    let [_, y] = millimetre_pair(records[off_grid]["world_coordinate"].as_str().expect("a position"));
    records[off_grid]["world_coordinate"] = Value::String(spell_millimetres([apart, y]));
    records[off_grid].as_object_mut().expect("a cell record").remove("vda_world_coordinate");
    let edited = with_map_values(&deleted, records);

    let changes = ingest(&module, &edited, &site);
    assert!(by_op(&changes, "patch").contains(grid.id.as_str()), "the frame was kept with a cell in another square");
}

/// A key the site states outside JSON's exact-integer range is read as no key:
/// the record that carried it takes the key a first import gives it, every
/// other cell keeps its own, and nothing overflows.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn a_production_key_past_the_exact_integer_range_is_replaced_rather_than_added_to() {
    let module = mapcreator();
    let files = production_export(EDITED);
    let mut site = ingested(&module, &files, PRODUCTION, OWN);
    let index = |entity: &Entity| formatting(&entity.components, "mapcreator_cell")?.get("index")?.as_i64();
    let largest = site.entities.iter().filter_map(index).max().expect("cells state ordering keys");
    let unreadable = site.entities.iter_mut().find(|entity| index(entity) == Some(largest)).expect("the last cell");
    unreadable.components.get_mut("import_formatting").expect("a cell's formatting")["mapcreator_cell"]["index"] =
        json!(i64::MAX);
    let unreadable = unreadable.id.clone();
    let (_, appended) = with_one_more_cell(&files, map_values(&files).len() - 1);

    let changes = ingest(&module, &appended, &site);
    let mut cells_patched = Vec::new();
    for change in &changes {
        match change {
            Change::Patch(patch) if formatting(&patch.components, "mapcreator_cell").is_some() => {
                let key = patch.components["import_formatting"]["mapcreator_cell"]["index"].clone();
                cells_patched.push((patch.id.clone(), key));
            }
            Change::Declare(cell) => {
                let key = cell.components["import_formatting"]["mapcreator_cell"]["index"].as_i64().expect("a key");
                assert!(key > largest, "the appended cell's key {key} is not after the last, {largest}");
            }
            _ => {}
        }
    }
    assert_eq!(cells_patched, [(unreadable, json!(largest))], "the cells whose key moved");
}
