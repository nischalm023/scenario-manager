//! Whether `schema/` and these structs still describe the same shape.
//!
//! The JSON Schemas one directory over and the serde structs beside this file
//! are two statements of one thing, written for two audiences: a validator with
//! no Rust toolchain, and this crate. Nothing else makes them agree, and two
//! statements of one shape that nothing compares drift apart quietly — the
//! schema goes on promising a field the reader stopped reading, and a document
//! that validates then fails to load. This module is the comparison: it fails
//! the build the moment they part, naming the component and the field rather
//! than the file.
//!
//! It is a unit test rather than a file in `tests/`, and the reason is
//! mechanical. The field census comes from a
//! `#[cfg_attr(test, derive(schemars::JsonSchema))]` on the component structs,
//! and `cfg(test)` holds only while the crate compiles its own test target; an
//! integration test links the library built *without* it and finds no
//! `JsonSchema` implementation to call. The census has to be derived rather than
//! written by hand — a hand-kept list of field names rots in exactly the way the
//! thing it guards rots.

use crate::components::KNOWN_COMPONENTS;
use crate::components::fleet::{
    Assignment, Carrier, CarrierPool, Cohort, Placement, Robot, RobotClass,
};
use crate::components::product::{
    CarrierType, Demand, Induct, Lineage, LineageEntry, LineageOption, LineageRole, ParamDecl,
    Pick, Port, PresentationPoint, Queue, Repeat, Sector, Stamp, Station, Template, TemplateEntity,
    Waiting,
};
use crate::components::spatial::{
    Cell, Exclusion, Frame, Charging, Pose, Region, Tiling, Transform,
};
use crate::components::structure::{
    Access, AccessCell, CircularArc, Direction, Flow, FlowRule, Level, Rack, Relay, Side, Slot,
    Stitch, StitchRule, Storage,
};
use schemars::{Schema, schema_for};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// The table: one row per `$defs` entry of every component file
// ---------------------------------------------------------------------------

const SPATIAL: &str = "components/spatial.json";
const STRUCTURE: &str = "components/structure.json";
const FLEET: &str = "components/fleet.json";
const PRODUCT: &str = "components/product.json";
const IDENTITY: &str = "components/identity.json";

/// What the crate has to say about one schema `$def`.
enum Shape {
    /// A serde struct. `schemars` derives the property and required census from
    /// it, so the comparison is against the struct itself and not against
    /// somebody's memory of it.
    Struct(fn() -> Value),
    /// A Rust enum, whose serde spellings are the closed value list.
    Enum(fn() -> Value),
    /// A closed value list SPEC §10 fixes and no Rust enum spells: the crate
    /// reads the field as a plain string. Listed so that giving one a Rust enum
    /// later is a change this test notices.
    SpecSourced(&'static str),
    /// A component with no struct at all, because the spec fixes its meaning and
    /// none of its keys.
    FreeMap(&'static str),
}

/// One `$defs` entry, and what the crate says it is.
struct Row {
    /// The component name, when this def *is* a component. A nested def —
    /// `transform`, `level`, `assignment` — names none: it is a shape a
    /// component's field has, not a meaning an entity carries.
    component: Option<&'static str>,
    file: &'static str,
    def: &'static str,
    shape: Shape,
}

fn rows() -> Vec<Row> {
    let component = |name: &'static str, file, def, shape| Row {
        component: Some(name),
        file,
        def,
        shape,
    };
    let nested = |file, def, shape| Row {
        component: None,
        file,
        def,
        shape,
    };
    vec![
        // spatial
        component(
            "frame",
            SPATIAL,
            "frame",
            Shape::Struct(|| value(schema_for!(Frame))),
        ),
        nested(
            SPATIAL,
            "transform",
            Shape::Struct(|| value(schema_for!(Transform))),
        ),
        component(
            "pose",
            SPATIAL,
            "pose",
            Shape::Struct(|| value(schema_for!(Pose))),
        ),
        component(
            "region",
            SPATIAL,
            "region",
            Shape::Struct(|| value(schema_for!(Region))),
        ),
        component(
            "tiling",
            SPATIAL,
            "tiling",
            Shape::Struct(|| value(schema_for!(Tiling))),
        ),
        nested(
            SPATIAL,
            "exclusion",
            Shape::Struct(|| value(schema_for!(Exclusion))),
        ),
        component(
            "cell",
            SPATIAL,
            "cell",
            Shape::Struct(|| value(schema_for!(Cell))),
        ),
        component(
            "charging",
            SPATIAL,
            "charging",
            Shape::Struct(|| value(schema_for!(Charging))),
        ),
        // structure
        nested(STRUCTURE, "side", Shape::Enum(|| value(schema_for!(Side)))),
        nested(
            STRUCTURE,
            "direction",
            Shape::Enum(|| value(schema_for!(Direction))),
        ),
        component(
            "rack",
            STRUCTURE,
            "rack",
            Shape::Struct(|| value(schema_for!(Rack))),
        ),
        nested(
            STRUCTURE,
            "level",
            Shape::Struct(|| value(schema_for!(Level))),
        ),
        component(
            "access",
            STRUCTURE,
            "access",
            Shape::Struct(|| value(schema_for!(Access))),
        ),
        component(
            "access_cell",
            STRUCTURE,
            "access_cell",
            Shape::Struct(|| value(schema_for!(AccessCell))),
        ),
        component(
            "storage",
            STRUCTURE,
            "storage",
            Shape::Struct(|| value(schema_for!(Storage))),
        ),
        component(
            "slot",
            STRUCTURE,
            "slot",
            Shape::Struct(|| value(schema_for!(Slot))),
        ),
        component(
            "relay",
            STRUCTURE,
            "relay",
            Shape::Struct(|| value(schema_for!(Relay))),
        ),
        component(
            "flow",
            STRUCTURE,
            "flow",
            Shape::Struct(|| value(schema_for!(Flow))),
        ),
        nested(
            STRUCTURE,
            "flow_rule",
            Shape::Enum(|| value(schema_for!(FlowRule))),
        ),
        component(
            "stitch",
            STRUCTURE,
            "stitch",
            Shape::Struct(|| value(schema_for!(Stitch))),
        ),
        nested(
            STRUCTURE,
            "circular_arc",
            Shape::Struct(|| value(schema_for!(CircularArc))),
        ),
        nested(
            STRUCTURE,
            "stitch_rule",
            Shape::Enum(|| value(schema_for!(StitchRule))),
        ),
        // fleet
        component(
            "robot_class",
            FLEET,
            "robot_class",
            Shape::Struct(|| value(schema_for!(RobotClass))),
        ),
        component(
            "cohort",
            FLEET,
            "cohort",
            Shape::Struct(|| value(schema_for!(Cohort))),
        ),
        nested(
            FLEET,
            "assignment",
            Shape::Struct(|| value(schema_for!(Assignment))),
        ),
        component(
            "robot",
            FLEET,
            "robot",
            Shape::Struct(|| value(schema_for!(Robot))),
        ),
        component(
            "carrier_pool",
            FLEET,
            "carrier_pool",
            Shape::Struct(|| value(schema_for!(CarrierPool))),
        ),
        nested(
            FLEET,
            "designation",
            Shape::SpecSourced(
                "SPEC §10 makes inventory-versus-order a permanent per-carrier fact; the crate \
                 carries it as the string a pool and a carrier both write",
            ),
        ),
        nested(
            FLEET,
            "placement",
            Shape::Struct(|| value(schema_for!(Placement))),
        ),
        component(
            "carrier",
            FLEET,
            "carrier",
            Shape::Struct(|| value(schema_for!(Carrier))),
        ),
        // product
        component(
            "template",
            PRODUCT,
            "template",
            Shape::Struct(|| value(schema_for!(Template))),
        ),
        nested(
            PRODUCT,
            "param_decl",
            Shape::Struct(|| value(schema_for!(ParamDecl))),
        ),
        nested(
            PRODUCT,
            "template_entity",
            Shape::Struct(|| value(schema_for!(TemplateEntity))),
        ),
        component(
            "stamp",
            PRODUCT,
            "stamp",
            Shape::Struct(|| value(schema_for!(Stamp))),
        ),
        nested(
            PRODUCT,
            "repeat",
            Shape::Struct(|| value(schema_for!(Repeat))),
        ),
        component(
            "station",
            PRODUCT,
            "station",
            Shape::Struct(|| value(schema_for!(Station))),
        ),
        component(
            "port",
            PRODUCT,
            "port",
            Shape::Struct(|| value(schema_for!(Port))),
        ),
        component(
            "queue",
            PRODUCT,
            "queue",
            Shape::Struct(|| value(schema_for!(Queue))),
        ),
        component(
            "waiting",
            PRODUCT,
            "waiting",
            Shape::Struct(|| value(schema_for!(Waiting))),
        ),
        component(
            "pick",
            PRODUCT,
            "pick",
            Shape::Struct(|| value(schema_for!(Pick))),
        ),
        component(
            "induct",
            PRODUCT,
            "induct",
            Shape::Struct(|| value(schema_for!(Induct))),
        ),
        component(
            "presentation_point",
            PRODUCT,
            "presentation_point",
            Shape::Struct(|| value(schema_for!(PresentationPoint))),
        ),
        component(
            "sector",
            PRODUCT,
            "sector",
            Shape::Struct(|| value(schema_for!(Sector))),
        ),
        component(
            "demand",
            PRODUCT,
            "demand",
            Shape::Struct(|| value(schema_for!(Demand))),
        ),
        component(
            "carrier_type",
            PRODUCT,
            "carrier_type",
            Shape::Struct(|| value(schema_for!(CarrierType))),
        ),
        component(
            "lineage",
            PRODUCT,
            "lineage",
            Shape::Struct(|| value(schema_for!(Lineage))),
        ),
        nested(
            PRODUCT,
            "lineage_entry",
            Shape::Struct(|| value(schema_for!(LineageEntry))),
        ),
        nested(
            PRODUCT,
            "lineage_role",
            Shape::Enum(|| value(schema_for!(LineageRole))),
        ),
        nested(
            PRODUCT,
            "lineage_option",
            Shape::Struct(|| value(schema_for!(LineageOption))),
        ),
        // identity
        component(
            "external_ids",
            IDENTITY,
            "external_ids",
            Shape::FreeMap(
                "SPEC §2 fixes the meaning — the identifiers other systems know an entity by — \
                 and fixes no keys, because every key is another system's spelling",
            ),
        ),
    ]
}

/// A field the schema names, the crate has no field for, and nobody has ruled
/// on: the def, the property, and what is waiting to be decided. Checked in both
/// directions, so the day the struct grows the field this list has to lose it.
fn spec_named_without_a_field() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![(
        "cell",
        "legacy_grid_coordinate",
        "SPEC §4 says a cell may carry the coordinate a legacy system recorded, and does not say \
         which integer frame the pair is read in; with two grid frames the pair is ambiguous, so \
         the crate has nothing to read it into yet",
    )]
}

/// Every closed value list in the component schemas that no Rust enum spells,
/// by the pointer that reaches it, with the values the spec fixes. Pinned here
/// so that a value appearing in the schema is a change somebody made on purpose:
/// SPEC §10 is the source of these vocabularies, and a schema that quietly grows
/// one has invented protocol.
fn carried_unread() -> Vec<(&'static str, &'static [&'static str], &'static str)> {
    vec![
        (
            "cell",
            &[
                "adjacency",
                "barcode_properties",
                "composite_attribute_list",
                "conveyor_direction_attribute",
                "floor",
                "grid_attribute",
                "has_storables",
                "location_profile",
                "location_profile_unresolved",
                "neighbours",
                "runtime",
                "size_info",
                "special",
                "store_status",
                "world_coordinate_reference_neighbour",
                "zone",
                "zone_unresolved",
            ],
            "Map Creator's facts about a cell, which its adapter writes and reads back",
        ),
        (
            "presentation_point",
            &[
                "attributes",
                "light_curtain",
                "presented_entity_allowed_types",
                "presenting_agent_id",
                "presenting_agent_type",
                "station",
                "world_coordinates_unresolved",
            ],
            "Map Creator's facts about a presentation point, which its adapter writes and reads back; the \
             shipped lenses read `station` through the lens engine, which needs no struct",
        ),
    ]
}

fn spec_sourced_enums() -> Vec<(&'static str, &'static str, Value, &'static str)> {
    vec![
        (
            SPATIAL,
            "/$defs/frame/properties/dims/enum",
            json!([2, 3]),
            "SPEC §4: the primary frame is 3-D and a floor or a grid is 2-D; the crate reads a \
             plain integer",
        ),
        (
            STRUCTURE,
            "/$defs/storage/properties/holds/enum",
            json!(["inventory", "order"]),
            "SPEC §10: which side of the operation the steel serves",
        ),
        (
            STRUCTURE,
            "/$defs/flow/if/properties/rule/enum",
            json!(["one_way", "alternating_rows"]),
            "not a vocabulary but a condition — the two rules that cannot leave `direction` \
             unstated; the vocabulary itself is `flow_rule`",
        ),
        (
            FLEET,
            "/$defs/robot_class/properties/locomotion/enum",
            json!(["floor", "ladder"]),
            "SPEC §10; the crate reads a string because absent has to read as `floor` \
             (SPEC-GAPS 48)",
        ),
        (
            FLEET,
            "/$defs/designation/enum",
            json!(["inventory", "order"]),
            "SPEC §10's per-carrier designation",
        ),
        (
            PRODUCT,
            "/$defs/station/properties/kind/enum",
            json!(["pick", "put", "induct", "extract"]),
            "SPEC §10: what happens at a station",
        ),
        (
            PRODUCT,
            "/$defs/port/properties/direction/enum",
            json!(["source", "sink"]),
            "SPEC §10: whether the world gains something here or loses it",
        ),
        (
            PRODUCT,
            "/$defs/queue/properties/discipline/enum",
            json!(["no_reverse"]),
            "SPEC §10: what entering a path commits an agent to",
        ),
        (
            PRODUCT,
            "/$defs/demand/properties/kind/enum",
            json!(["orders", "putaway"]),
            "SPEC §10: which direction of work a source produces",
        ),
        (
            PRODUCT,
            "/$defs/carrier_type/properties/kind/enum",
            json!(["tote", "tray", "box"]),
            "SPEC §10: which physical kind a carrier is",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Reading the two sides
// ---------------------------------------------------------------------------

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application/schema")
}

/// One schema file, parsed. Read at test time from the working tree: no network,
/// no code generation, and no copy of the schema inside the crate to go stale.
fn schema(file: &str) -> Value {
    let path = schema_dir().join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is not readable: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// Every schema file beside `validate.py`, by the `$id` it registers itself
/// under. Discovered rather than listed, so a file added to the directory is a
/// file this test can already resolve a `$ref` into.
fn schemas_by_id() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut directories = vec![(schema_dir(), String::new())];
    while let Some((directory, prefix)) = directories.pop() {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|e| panic!("{} is not readable: {e}", directory.display()));
        for entry in entries {
            let entry = entry.expect("a directory entry");
            let name = entry.file_name().to_string_lossy().to_string();
            let relative = format!("{prefix}{name}");
            if entry.path().is_dir() {
                if !name.starts_with('.') {
                    directories.push((entry.path(), format!("{relative}/")));
                }
            } else if name.ends_with(".json") {
                if let Some(id) = schema(&relative).get("$id").and_then(Value::as_str) {
                    out.insert(id.to_string(), relative);
                }
            }
        }
    }
    out
}

/// A derived schema as plain JSON, which is the form both sides are compared in.
fn value(derived: Schema) -> Value {
    serde_json::to_value(derived).expect("a derived schema is JSON")
}

/// One `$defs` entry of a file.
fn def<'a>(file: &'a Value, name: &str) -> &'a Value {
    file.get("$defs")
        .and_then(|defs| defs.get(name))
        .unwrap_or_else(|| panic!("no `$defs/{name}` in this schema file"))
}

/// The field names an object schema states, leaving out the `$`-prefixed
/// annotation keys: `$comment` sits among fields as freely as it sits among
/// components and is not one (SPEC §2).
fn properties(object: &Value) -> BTreeSet<String> {
    object
        .get("properties")
        .and_then(Value::as_object)
        .map(|map| {
            map.keys()
                .filter(|key| !key.starts_with('$'))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn required(object: &Value) -> BTreeSet<String> {
    object
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The values a closed list admits, in either of the two spellings a schema
/// writes them: `enum` on one line, or a `oneOf` of `const` branches, which is
/// what a derived schema produces for an enum whose variants carry doc comments.
fn closed_values(object: &Value) -> Option<Vec<Value>> {
    if let Some(items) = object.get("enum").and_then(Value::as_array) {
        return Some(items.clone());
    }
    let branches = object.get("oneOf")?.as_array()?;
    let values: Vec<Value> = branches
        .iter()
        .filter_map(|branch| branch.get("const").cloned())
        .collect();
    (values.len() == branches.len()).then_some(values)
}

/// Whether an object schema leaves room for keys it does not name.
fn is_open(object: &Value) -> bool {
    !matches!(object.get("additionalProperties"), Some(Value::Bool(false)))
        && !matches!(
            object.get("unevaluatedProperties"),
            Some(Value::Bool(false))
        )
}

// ---------------------------------------------------------------------------
// The comparisons
// ---------------------------------------------------------------------------

/// The register: one list of component names, stated in four places.
///
/// `KNOWN_COMPONENTS` decides which documents this crate will read at all — a
/// name missing from it refuses any document that requires it — so the three
/// schema statements and it have to be the same list, exactly, in both
/// directions.
#[test]
fn the_register_names_the_same_components_everywhere() {
    let components = schema("components.json");
    let crate_side: BTreeSet<String> = KNOWN_COMPONENTS.iter().map(|n| n.to_string()).collect();
    assert_eq!(
        crate_side.len(),
        KNOWN_COMPONENTS.len(),
        "`KNOWN_COMPONENTS` lists a name twice"
    );

    let core_name: BTreeSet<String> = def(&components, "core_name")
        .get("enum")
        .and_then(Value::as_array)
        .expect("`core_name` is an enum")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    assert_eq!(
        core_name, crate_side,
        "`core_name` against KNOWN_COMPONENTS"
    );
    assert_eq!(
        properties(def(&components, "set")),
        crate_side,
        "the lenient component map against KNOWN_COMPONENTS"
    );
    assert_eq!(
        properties(def(&components, "set_strict")),
        crate_side,
        "the strict component map against KNOWN_COMPONENTS"
    );

    // Every component in the table is a component in the register, and every
    // component in the register has a row: a component added to the crate
    // without a row would otherwise skip every check below in silence.
    let tabled: BTreeSet<String> = rows()
        .iter()
        .filter_map(|row| row.component.map(str::to_string))
        .collect();
    assert_eq!(tabled, crate_side, "the table against KNOWN_COMPONENTS");
}

/// Each component's `$ref` lands where the table says its definition lives.
///
/// The register is a list of names pointing at definitions in other files, and
/// the pointer is as much a claim as the name is: a `$ref` aimed at the wrong
/// file compares the wrong two things and passes.
#[test]
fn each_components_reference_resolves_to_the_file_the_table_names() {
    let components = schema("components.json");
    let by_id = schemas_by_id();
    let set = def(&components, "set")
        .get("properties")
        .and_then(Value::as_object)
        .expect("the component map states its components");

    for row in rows() {
        let Some(name) = row.component else { continue };
        let reference = set[name]
            .get("$ref")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("`{name}` in the component map is not a $ref"));
        let (id, fragment) = reference
            .split_once('#')
            .unwrap_or_else(|| panic!("`{reference}` names no definition"));
        let file = by_id
            .get(id)
            .unwrap_or_else(|| panic!("no schema file registers `{id}`"));
        assert_eq!(*file, row.file, "where `{name}` is defined");
        assert_eq!(
            fragment,
            format!("/$defs/{}", row.def),
            "which definition `{name}` points at"
        );
    }
}

/// Every definition in the component files is somebody's business.
///
/// Without this, adding a `$defs` entry to a schema file and forgetting the
/// crate half reads as agreement, because nothing would look for it.
#[test]
fn every_definition_in_the_component_files_has_a_row() {
    let mut tabled: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for row in rows() {
        tabled.entry(row.file).or_default().insert(row.def);
    }
    for file in [SPATIAL, STRUCTURE, FLEET, PRODUCT, IDENTITY] {
        let defined: BTreeSet<String> = schema(file)
            .get("$defs")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{file} defines nothing"))
            .keys()
            .cloned()
            .collect();
        let tabled: BTreeSet<String> = tabled
            .get(file)
            .map(|names| names.iter().map(|n| n.to_string()).collect())
            .unwrap_or_default();
        assert_eq!(defined, tabled, "the definitions in {file}");
    }
}

/// A schema states the fields the struct states, and requires the fields the
/// struct cannot default.
///
/// Both halves are one mechanical rule read two ways, which is what makes them
/// checkable: a field is `required` in the schema exactly when its struct field
/// carries no `#[serde(default)]`, because that is the rule the schemas were
/// written to.
#[test]
fn each_definitions_fields_are_the_structs_fields() {
    let exempt = spec_named_without_a_field();
    let unread = carried_unread();
    for row in rows() {
        let Shape::Struct(derive) = row.shape else {
            continue;
        };
        let schema_side = schema(row.file);
        let schema_side = def(&schema_side, row.def);
        let crate_side = derive();

        let mut waiting_on_a_ruling: BTreeSet<String> =
            exempt.iter().filter(|(def, ..)| *def == row.def).map(|(_, field, _)| field.to_string()).collect();
        for (_, fields, why) in unread.iter().filter(|(def, ..)| *def == row.def) {
            for field in *fields {
                assert!(
                    properties(schema_side).contains(*field),
                    "`{}.{field}` is listed as carried unread ({why}) and the schema no longer names it",
                    row.def
                );
                assert!(
                    !properties(&crate_side).contains(*field),
                    "`{}.{field}` has a struct field now, so it is read: delete it from `carried_unread`",
                    row.def
                );
                waiting_on_a_ruling.insert(field.to_string());
            }
        }
        for field in &waiting_on_a_ruling {
            assert!(
                properties(schema_side).contains(field),
                "`{}.{field}` is listed as waiting on a ruling and the schema no longer names it",
                row.def
            );
            assert!(
                !properties(&crate_side).contains(field),
                "`{}.{field}` has a struct field now — the ruling happened, so delete it from \
                 `spec_named_without_a_field`",
                row.def
            );
        }

        let schema_fields: BTreeSet<String> =
            properties(schema_side).difference(&waiting_on_a_ruling).cloned().collect();
        assert_eq!(schema_fields, properties(&crate_side), "the fields of `{}`", row.def);
        assert_eq!(required(schema_side), required(&crate_side), "the fields `{}` requires", row.def);

        // A `#[serde(flatten)]` catch-all is not a field and cannot be compared
        // as one. What it means is that the object carries keys the struct does
        // not name, so the schema has to leave room for them.
        if is_open(&crate_side) && crate_side.get("additionalProperties") == Some(&json!(true)) {
            assert!(
                is_open(schema_side),
                "`{}` collects unnamed keys in a struct field and the schema closes the object",
                row.def
            );
        }
    }
}

/// A closed list of values means the same values on both sides.
///
/// Every one of them is reached in the same walk, so a list nobody accounted for
/// fails rather than passing unseen: an enum this crate has a Rust type for is
/// compared against that type's serde spellings, and one only the spec fixes is
/// compared against the values written down beside it here.
#[test]
fn every_closed_value_list_is_either_a_rust_enum_or_declared_spec_sourced() {
    let mut expected: BTreeMap<(&str, String), Value> = BTreeMap::new();
    for (file, pointer, values, _) in spec_sourced_enums() {
        expected.insert((file, pointer.to_string()), values);
    }
    for row in rows() {
        match row.shape {
            Shape::Enum(derive) => {
                let values = closed_values(&derive())
                    .unwrap_or_else(|| panic!("`{}` derives no closed value list", row.def));
                expected.insert(
                    (row.file, format!("/$defs/{}/enum", row.def)),
                    Value::Array(values),
                );
            }
            // A definition that is nothing but a value list, with no Rust enum
            // spelling it, still has to be one of the lists pinned above.
            Shape::SpecSourced(why) => assert!(
                expected.contains_key(&(row.file, format!("/$defs/{}/enum", row.def))),
                "`{}` has no Rust enum ({why}), so its values belong in `spec_sourced_enums`",
                row.def
            ),
            Shape::Struct(_) | Shape::FreeMap(_) => {}
        }
    }

    let mut found: BTreeMap<(&str, String), Value> = BTreeMap::new();
    for file in [SPATIAL, STRUCTURE, FLEET, PRODUCT, IDENTITY] {
        let mut sites = Vec::new();
        collect_closed_lists(&schema(file), String::new(), &mut sites);
        for (pointer, values) in sites {
            found.insert((file, pointer), values);
        }
    }

    assert_eq!(
        found.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "every closed value list in the component schemas has to be accounted for: a Rust enum \
         compares against its serde spellings, anything else is listed as spec-sourced"
    );
    for (site, values) in &found {
        assert_eq!(
            values, &expected[site],
            "the values at {}{}",
            site.0, site.1
        );
    }
}

/// Every `enum` keyword under a schema, by the pointer that reaches it.
fn collect_closed_lists(value: &Value, pointer: String, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_pointer = format!("{pointer}/{key}");
                if key == "enum" {
                    out.push((child_pointer, child.clone()));
                } else {
                    collect_closed_lists(child, child_pointer, out);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_closed_lists(child, format!("{pointer}/{index}"), out);
            }
        }
        _ => {}
    }
}

/// A component with no struct behind it is one whose keys the spec does not fix.
///
/// This is the check that keeps `external_ids` honest. The crate registers the
/// name and parses nothing, and that is only defensible while the schema agrees
/// there is nothing to parse: the day it starts naming properties, the crate
/// owes it a struct.
#[test]
fn a_component_with_no_struct_is_a_map_the_schema_fixes_no_keys_of() {
    for row in rows() {
        let Shape::FreeMap(why) = row.shape else {
            continue;
        };
        let file = schema(row.file);
        let definition = def(&file, row.def);
        assert_eq!(
            definition.get("type"),
            Some(&json!("object")),
            "`{}` has no struct because {why}, which only holds for an object",
            row.def
        );
        assert!(
            properties(definition).is_empty(),
            "`{}` names properties now, so it needs a struct: {why} no longer holds",
            row.def
        );
        assert!(
            is_open(definition),
            "`{}` has no struct because {why}, so it cannot be closed",
            row.def
        );
    }
}
