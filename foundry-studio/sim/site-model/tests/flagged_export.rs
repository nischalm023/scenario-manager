//! A whole site as WPS generated it, read into a world with nothing generated again (ruling D175).
//!
//! The small files here are written out by hand the way WPS writes one: every entity listed once, generated ones
//! under the identity their generation gave them, and no prologue. Each pins one rule of the reader: which frame is
//! primary, whose child a generated entity is, and what it refuses. The last test holds the reader to the native
//! pipeline over the Walmart Dallas fixture: the entities the native pipeline derives, written out as a flagged
//! export, build the same world again.

use serde_json::{json, Value};
use site_model::pipeline::World;
use site_model::{flagged_export, EntityId, LoadError};
use std::fmt::Display;
use std::path::Path;

const FLOOR: &str = "f0000000-0000-4000-8000-000000000001";
const RACKING: &str = "f0000000-0000-4000-8000-000000000002";
const HIGHWAY: &str = "a0000000-0000-4000-8000-000000000001";
const STAMP: &str = "b0000000-0000-4000-8000-000000000001";
const STATION: &str = "c0000000-0000-4000-8000-000000000001";

fn id(text: &str) -> EntityId {
    text.parse().expect("a literal identity in this test")
}

/// A generated entity's identity: its generator and the key beneath it.
fn at(generator: impl Display, key: impl Display) -> EntityId {
    id(&format!("{generator}/{key}"))
}

/// A flagged export with all three flags, holding `entities`, each given in this file's scope.
fn export(entities: Vec<Value>) -> Value {
    let entities: Vec<Value> = entities
        .into_iter()
        .map(|mut entity| {
            entity["scope"] = json!("customer/c/site/s");
            entity
        })
        .collect();
    json!({
        "derived": true,
        "entities": entities,
        "nested": true,
        "referenced": true,
        "scope": "customer/c/site/s",
        "scopes": [{ "changeset": 1, "scope": "customer/c/site/s", "whole": true }],
    })
}

fn world(entities: Vec<Value>) -> Result<World, LoadError> {
    flagged_export::world(Path::new("export"), &export(entities))
}

/// The primary frame, at one millimetre a step.
fn floor() -> Value {
    json!({ "id": FLOOR, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } })
}

/// A column of three tiles at x = 450, as its tiling's generation leaves it: the tiling and its three cells.
fn highway() -> Vec<Value> {
    let mut entities = vec![json!({ "id": HIGHWAY, "name": "highway", "components": {
        "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 2700] },
        "tiling": { "pitch": [900, 900] }
    }})];
    for row in 0..3 {
        entities.push(json!({ "id": format!("{HIGHWAY}/cell:0,{row}"), "components": {
            "pose": { "frame": FLOOR, "position": [450, 450 + 900 * row], "heading_deg": 0.0 },
            "cell": {}
        }}));
    }
    entities
}

/// The file's entities are the world's, its tiling's cells compile their arcs, and nothing is left over from the
/// native pipeline's records: no override, no suppression, no `expect` pin.
#[test]
fn a_flagged_export_builds_its_world_as_it_stands() {
    let mut entities = vec![floor()];
    entities.extend(highway());
    let world = world(entities).expect("the export builds a world");
    assert_eq!(world.entities.len(), 5);
    assert_eq!(world.frames.primary(), Some(&id(FLOOR)));
    assert_eq!(world.declared.keys().cloned().collect::<Vec<_>>(), vec![id(HIGHWAY), id(FLOOR)]);
    assert_eq!(world.counts.get(&id(HIGHWAY).root()), Some(&3), "three cells counted under the tiling");
    assert_eq!(world.derived_count(), 3);
    assert!(world.overrides.is_empty() && world.suppressions.is_empty() && world.expects.is_empty());
    let cell = at(HIGHWAY, "cell:0,1");
    assert_eq!(world.entities[&cell].parent, Some(id(HIGHWAY)));
    assert_eq!(world.entities[&id(HIGHWAY)].name.as_deref(), Some("highway"));
    // A tiled cell takes its tiling's pitch as its footprint: the parent is what that is read through.
    assert_eq!(world.cells[&cell].footprint.map(|side| side.0), [900, 900]);
    assert!(world.cells[&cell].footprint_stated);
    // Two-way flow over a column of three: each of the two neighbour pairs both ways.
    let arcs: Vec<(EntityId, EntityId)> = world.edges.iter().map(|edge| (edge.from.clone(), edge.to.clone())).collect();
    assert_eq!(arcs.len(), 4, "{arcs:?}");
    assert!(arcs.contains(&(at(HIGHWAY, "cell:0,0"), cell.clone())));
    assert!(arcs.contains(&(cell, at(HIGHWAY, "cell:0,0"))));
}

/// The one frame stating `unit_mm` is primary, whichever frame it is; one nested inside it is not.
#[test]
fn the_primary_frame_is_the_frame_that_states_unit_mm() {
    let racking = json!({ "id": RACKING, "components": { "frame": {
        "parent": FLOOR, "dims": 2, "units": "mm", "transform_to_parent": { "translation": [100, 0], "scale": [1, 1] }
    }}});
    let world = world(vec![racking, floor()]).expect("the export builds a world");
    assert_eq!(world.frames.primary(), Some(&id(FLOOR)));
    assert_eq!(world.frames.unit_mm(), Some(1.0));
}

/// Two frames stating `unit_mm` would make a step worth two things, so the file is refused naming both.
#[test]
fn two_frames_stating_unit_mm_are_refused() {
    let second = json!({ "id": RACKING, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 10 } } });
    match world(vec![floor(), second]) {
        Err(LoadError::UnitOnTwoFrames { first, second }) => {
            assert_eq!((first, second), (id(FLOOR), id(RACKING)));
        }
        other => panic!("two frames stating unit_mm load: {other:?}"),
    }
}

/// With no frame stating `unit_mm` there is no primary frame; a cell that needs the 900 mm default then has no step
/// count for it, and the file is refused there rather than read at an assumed scale.
#[test]
fn a_file_without_a_unit_has_no_primary_frame() {
    let unitless = json!({ "id": FLOOR, "components": { "frame": { "dims": 2, "units": "mm" } } });
    let world = world(vec![unitless.clone()]).expect("a file of one frame needs no unit");
    assert_eq!(world.frames.primary(), None);

    let point = json!({ "id": STATION, "components": {
        "pose": { "frame": FLOOR, "position": [450, 450] }, "cell": {}
    }});
    let refused = flagged_export::world(Path::new("export"), &export(vec![unitless, point]));
    assert!(matches!(refused, Err(LoadError::CellSizeWithoutUnit { .. })), "{refused:?}");
}

/// A generated entity's parent is its nearest ancestor in the file: one step up where WPS carries it, and further
/// up where it does not, as when an instance is suppressed and the internals stamped under it survive.
#[test]
fn a_generated_entity_takes_its_nearest_ancestor_as_parent() {
    let template_internal =
        |key: &str, components: Value| json!({ "id": format!("{STAMP}/{key}"), "components": components });
    let entities = vec![
        floor(),
        json!({ "id": STAMP, "components": { "external_ids": { "test": "stamp" } } }),
        // `instance:0` itself is absent: suppressed, or never written by the generation.
        template_internal(
            "instance:0/local:aisle",
            json!({
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 900] },
                "tiling": { "pitch": [900, 900] }
            }),
        ),
        template_internal(
            "instance:0/local:aisle/cell:0,0",
            json!({
                "pose": { "frame": FLOOR, "position": [450, 450] }, "cell": {}
            }),
        ),
    ];
    let world = world(entities).expect("the export builds a world");
    let aisle = at(STAMP, "instance:0/local:aisle");
    assert_eq!(world.entities[&aisle].parent, Some(id(STAMP)), "the stamp, two steps up");
    assert_eq!(world.entities[&at(STAMP, "instance:0/local:aisle/cell:0,0")].parent, Some(aisle));
}

/// A generated entity whose generator the file does not hold has no ancestor at all, and is refused naming both,
/// rather than read without the tiling its footprint comes from. A site that references another scope's derived
/// entity gets such a file from WPS: `referenced` brings the entity alone, from a scope the file holds in part.
#[test]
fn a_generated_entity_without_its_generator_is_refused() {
    let orphan = json!({ "id": format!("{HIGHWAY}/cell:0,0"), "scope": "catalog/c/lib", "components": {
        "pose": { "frame": FLOOR, "position": [450, 450] }, "cell": {}
    }});
    let mut file = export(vec![floor()]);
    file["entities"].as_array_mut().expect("the entities").insert(0, orphan);
    file["scopes"] = json!([
        { "changeset": 1, "scope": "catalog/c/lib", "whole": false },
        { "changeset": 1, "scope": "customer/c/site/s", "whole": true },
    ]);
    let refused = flagged_export::world(Path::new("export"), &file).expect_err("an orphan does not load");
    let text = refused.to_string();
    match refused {
        LoadError::GeneratedWithoutGenerator { entity, root } => {
            assert_eq!((entity, root), (at(HIGHWAY, "cell:0,0"), id(HIGHWAY)));
            assert!(text.contains(&format!("{HIGHWAY}/cell:0,0")) && text.contains(HIGHWAY), "{text}");
        }
        other => panic!("refused another way: {other:?}"),
    }
}

/// Each carried scope is read at one changeset, so an identity listed twice is refused.
#[test]
fn an_identity_listed_twice_is_refused() {
    let refused = world(vec![floor(), floor()]);
    assert!(matches!(&refused, Err(LoadError::ListedTwice { entity }) if *entity == id(FLOOR)), "{refused:?}");
}

/// A queue whose step names a cell the file does not hold dangles: whatever removed it, WPS left nothing to stand
/// on there.
#[test]
fn a_queue_step_onto_a_cell_the_file_lacks_dangles() {
    let mut entities = vec![floor()];
    entities.extend(highway());
    entities.push(json!({ "id": STATION, "components": { "queue": { "path": [
        { "generator": HIGHWAY, "key": "cell:0,0" },
        { "generator": HIGHWAY, "key": "cell:0,5" }
    ]}}}));
    let refused = world(entities);
    assert!(matches!(refused, Err(LoadError::QueuePathDangling { index: 1, .. })), "{refused:?}");
}

/// A placeholder outside a template stands for nothing, here as in a document.
#[test]
fn a_placeholder_left_outside_a_template_is_refused() {
    let mut entities = vec![floor()];
    entities.extend(highway());
    entities[1]["components"]["region"]["origin"] = json!({ "$param": "origin" });
    let refused = world(entities);
    assert!(matches!(refused, Err(LoadError::UnresolvedPlaceholder { .. })), "{refused:?}");
}

/// A parameter standing for a reference stays its stamp's inside a template, and the file builds; outside one it is
/// refused, naming where it stands. So wps-client, which reads one inside a template as no reference, lets none into
/// a run anywhere else.
#[test]
fn a_parameter_for_a_reference_is_refused_outside_a_template_only() {
    let mut entities = vec![floor()];
    entities.extend(highway());
    let internal = json!({ "local": "tile", "components": { "cell": { "zone": { "$param": "zone" } } } });
    let template = json!({ "template": { "params": { "zone": {} }, "entities": [internal] } });
    entities.push(json!({ "id": "d0000000-0000-4000-8000-000000000001", "components": template }));
    world(entities.clone()).expect("a parameter inside a template is its stamp's");
    entities[2]["components"]["cell"]["zone"] = json!({ "$param": "zone" });
    match world(entities) {
        Err(LoadError::UnresolvedPlaceholder { component, field, .. }) => {
            assert_eq!((&*component, &*field), ("cell", "zone"))
        }
        other => panic!("{other:?}"),
    }
}

/// An entry that does not read as an entity is refused naming its place in the list.
#[test]
fn an_entity_that_does_not_read_is_refused_by_its_place() {
    let refused = world(vec![floor(), json!({ "id": "not an identity", "components": {} })]);
    match refused {
        Err(LoadError::Shape { message, .. }) => assert!(message.contains("entity #1"), "{message}"),
        other => panic!("{other:?}"),
    }
}

/// The keys alone choose the reader: a flagged export names the scope it exports under `scope`, as a string, beside
/// its flags, and holds no prologue; an authored document holds `proto_version` and `doc` and none of a flagged
/// export's keys. A file holding a flagged export's keys beside a prologue's is refused naming both, whatever they
/// hold, rather than read one way and lose what the other reads; so is a `scope` that names no scope.
#[test]
fn a_file_marked_both_flagged_and_authored_is_refused() {
    let here = Path::new("file");
    let shape = |file: &Value| flagged_export::is_flagged_export(here, file);
    let authored = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "d0000000-0000-4000-8000-000000000001" },
        "primary_frame": FLOOR,
    });
    assert!(matches!(shape(&export(vec![floor()])), Ok(true)));
    assert!(matches!(shape(&authored), Ok(false)));

    let marked = |flagged: &[(&str, Value)], headers: &[&str]| {
        let mut file = json!({ "entities": [] });
        for (key, value) in flagged {
            file[*key] = value.clone();
        }
        for header in headers {
            file[*header] = authored[*header].clone();
        }
        file
    };
    let site = json!("customer/c/site/s");
    let both = [
        (marked(&[("scope", site.clone())], &["proto_version", "doc"]), "`doc` and `proto_version`"),
        (marked(&[("nested", json!(true))], &["primary_frame"]), "`primary_frame`"),
        (marked(&[("scopes", json!([])), ("scope", site)], &["doc"]), "`doc`"),
    ];
    for (file, named) in both {
        match shape(&file) {
            Err(error @ LoadError::FlaggedBesideAuthored { .. }) => {
                let text = error.to_string();
                assert!(text.contains("flagged export") && text.contains(named), "{text}");
            }
            other => panic!("{file} is not refused: {other:?}"),
        }
    }
    for unnamed in [marked(&[("derived", json!(true))], &[]), marked(&[("scope", json!(3))], &[])] {
        assert!(matches!(shape(&unnamed), Err(LoadError::Shape { .. })), "{unnamed}");
    }
}

/// Ruling D185: a world is the site whole as WPS generated it, with every entity it references, so only a flagged
/// export stating all three of its flags builds one; any other is refused naming each flag it leaves unset and the
/// request that sets them, and one stating a flag as neither `true` nor `false` is refused by name.
#[test]
fn a_flagged_export_without_all_three_flags_is_refused_naming_each_unset() {
    let mut unset = export(vec![floor()]);
    unset["nested"] = json!(false);
    unset.as_object_mut().expect("an object").remove("referenced");
    match flagged_export::world(Path::new("export"), &unset) {
        Err(error @ LoadError::ExportFlagsUnset { .. }) => {
            let text = error.to_string();
            assert!(text.contains("`nested` and `referenced`") && !text.contains("`derived`"), "{text}");
            assert!(text.contains("?nested=1&derived=1&referenced=1"), "{text}");
        }
        other => panic!("a flagged export with flags unset builds: {other:?}"),
    }
    let mut spelled = export(vec![floor()]);
    spelled["derived"] = json!("1");
    match flagged_export::world(Path::new("export"), &spelled) {
        Err(LoadError::Shape { message, .. }) => assert!(message.contains("`derived`"), "{message}"),
        other => panic!("a flag stated as a string builds: {other:?}"),
    }
}

/// Walmart Dallas as the native pipeline derives it, written out as a flagged export, builds the same world: the
/// same entities with the same components, names and parents, the same cells and arcs, primary frame and counts.
/// A relay's binding is left out unless a record stated it, so the reader binds it again as WPS's file needs.
#[test]
fn the_native_world_written_out_builds_the_same_world() {
    let scenario = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/seed/application-tests/native/fixtures/walmart-dallas/scenario.json");
    let (_, native) = site_model::load(&scenario).expect("the native fixture loads");
    let file = flagged_export::written_out(&native, "customer/c/site/s");
    let read = flagged_export::world(Path::new("dallas"), &file).expect("the written-out world builds");

    assert_eq!(read.entities.keys().collect::<Vec<_>>(), native.entities.keys().collect::<Vec<_>>());
    for (id, entity) in &native.entities {
        let built = &read.entities[id];
        assert_eq!(built.components, entity.components, "{id}'s components");
        assert_eq!(built.name, entity.name, "{id}'s name");
        assert_eq!(built.parent, entity.parent, "{id}'s parent");
    }
    assert_eq!(read.edges, native.edges);
    assert_eq!(read.cells.keys().collect::<Vec<_>>(), native.cells.keys().collect::<Vec<_>>());
    for (id, cell) in &native.cells {
        let built = &read.cells[id];
        assert_eq!(
            (built.centre, built.footprint, built.footprint_stated),
            (cell.centre, cell.footprint, cell.footprint_stated)
        );
    }
    assert_eq!(read.frames.primary(), native.frames.primary());
    assert_eq!(read.counts, native.counts);
    assert_eq!(read.declared.keys().collect::<Vec<_>>(), native.declared.keys().collect::<Vec<_>>());
    for warning in &read.warnings {
        assert!(native.warnings.contains(warning), "a warning the native load did not give: {warning}");
    }
}
