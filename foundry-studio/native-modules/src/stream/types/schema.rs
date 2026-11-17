//! The JSON Schema of each record type: generated from its struct, checked in beside it as `<type>.jsonschema`, and
//! held by these tests to the struct, to the profile every type keeps, to lines it must refuse and to an example line
//! of each type. It compiles into the tests only, as the schema derive does.

use std::any::type_name;
use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use desunits::SimTime;
use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{CustomerOrder, InboundLot, Sku, StartingTote};

const WRITE: &str = "NATIVE_MODULES_WRITE_SCHEMAS";

/// Stands for a reader's world-time type in the generated schemas: the one definition every world-time field refers
/// to.
enum WorldTimeStandIn {}

impl JsonSchema for WorldTimeStandIn {
    fn schema_name() -> Cow<'static, str> {
        "WorldTime".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "An instant of world time, the simulated clock, in milliseconds from the run's start; at most 2^53 - 1, so that every JSON reader holds it exactly.",
            "type": "integer",
            "minimum": 0,
            "maximum": 9_007_199_254_740_991_u64
        })
    }
}

/// A record type: its name, which names its schema file; its schema; a reading of a line into its struct, with the
/// simulator's `SimTime` for a world time, written back as JSON; and an example line of the type.
struct Case {
    name: &'static str,
    schema: fn() -> Value,
    reread: fn(&str) -> Value,
    example: &'static str,
}

const CASES: [Case; 4] = [
    Case { name: "sku", schema: generated::<Sku>, reread: reread::<Sku>, example: r#"{"id":"2943928","units":237}"# },
    Case {
        name: "inbound_lot",
        schema: generated::<InboundLot<WorldTimeStandIn>>,
        reread: reread::<InboundLot<SimTime>>,
        example: r#"{"id":"lot-0001","known_ms":0,"arrival_ms":3600000,"due_ms":14400000,"station":"station-1","totes":["single","quarter-partitioned"],"lines":[{"sku":"SKU-1","quantity":40},{"sku":"SKU-1","quantity":40,"tote_index":0},{"sku":"SKU-1","quantity":40,"tote_index":1,"compartment":2}]}"#,
    },
    Case {
        name: "customer_order",
        schema: generated::<CustomerOrder<WorldTimeStandIn>>,
        reread: reread::<CustomerOrder<SimTime>>,
        example: r#"{"id":"order-0042","customer":"087ABC004","known_ms":42000,"arrival_ms":60000,"due_ms":3600000,"station":"station-3","lines":[{"sku":"3486829","quantity":2}]}"#,
    },
    Case {
        name: "starting_tote",
        schema: generated::<StartingTote>,
        reread: reread::<StartingTote>,
        example: r#"{"id":"T-00017","contents":[{"sku":"A-114","quantity":12},{"sku":"B-9","quantity":3}]}"#,
    },
];

fn generated<T: JsonSchema>() -> Value {
    let settings = SchemaSettings::draft2020_12().with_transform(RecursiveTransform(keep_profile));
    settings.into_generator().into_root_schema_for::<T>().to_value()
}

/// Holds a generated schema to the profile where the derive departs from it: drops the `null` an `Option` adds, as a
/// record says it has no value by leaving the field out; states the ceiling of a sized integer, which the derive
/// names only as a format; and joins a description's wrapped lines, as rustdoc does.
fn keep_profile(schema: &mut Schema) {
    let Some(object) = schema.as_object_mut() else { return };
    let ceiling = match object.get("format").and_then(Value::as_str) {
        Some("uint8") => Some(u64::from(u8::MAX)),
        Some("uint32") => Some(u64::from(u32::MAX)),
        _ => None,
    };
    if let Some(ceiling) = ceiling {
        object.entry("maximum").or_insert(ceiling.into());
    }
    if let Some(Value::Array(types)) = object.get_mut("type") {
        types.retain(|name| *name != "null");
        if let [only] = types.as_slice() {
            let only = only.clone();
            object.insert("type".into(), only);
        }
    }
    if let Some(Value::Array(branches)) = object.get_mut("anyOf") {
        branches.retain(|branch| branch.get("type").is_none_or(|name| *name != "null"));
        if let [Value::Object(only)] = branches.as_slice() {
            let only = only.clone();
            object.remove("anyOf");
            object.extend(only);
        }
    }
    if let Some(Value::String(text)) = object.get_mut("description") {
        *text = text.split("\n\n").map(|paragraph| paragraph.replace('\n', " ")).collect::<Vec<_>>().join("\n\n");
    }
}

/// A schema as its file holds it: keys sorted, two-space indents, a final newline.
fn text(mut schema: Value) -> String {
    schema.sort_all_objects();
    let mut text = serde_json::to_string_pretty(&schema).expect("a schema writes as JSON");
    text.push('\n');
    text
}

fn file(case: &Case) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/stream/types").join(format!("{}.jsonschema", case.name))
}

fn reread<T: DeserializeOwned + Serialize>(line: &str) -> Value {
    let record: T =
        serde_json::from_str(line).unwrap_or_else(|error| panic!("{} refuses {line}: {error}", type_name::<T>()));
    serde_json::to_value(record).expect("a record writes as JSON")
}

/// Each way `schema`, found at `at`, breaks the profile: a `null` or `number` type, an integer without a stated
/// range, an object open to unknown fields, a definition or property without a description.
fn breaches(schema: &Value, at: &str, found: &mut Vec<String>) {
    let types: Vec<&Value> = match schema.get("type") {
        Some(Value::Array(names)) => names.iter().collect(),
        named => named.into_iter().collect(),
    };
    for kind in ["null", "number"] {
        if types.iter().any(|name| **name == kind) {
            found.push(format!("{at}: a {kind} type"));
        }
    }
    let ranged = schema.get("minimum").is_some() && schema.get("maximum").is_some();
    if types.iter().any(|name| **name == "integer") && !ranged {
        found.push(format!("{at}: an integer without a stated range"));
    }
    let is_object = types.iter().any(|name| **name == "object") || schema.get("properties").is_some();
    if is_object && schema.get("additionalProperties") != Some(&Value::Bool(false)) {
        found.push(format!("{at}: an object open to unknown fields"));
    }
    for keyword in ["$defs", "properties"] {
        for (name, member) in schema.get(keyword).and_then(Value::as_object).into_iter().flatten() {
            let at = format!("{at}/{keyword}/{name}");
            if !member.get("description").is_some_and(Value::is_string) {
                found.push(format!("{at}: no description"));
            }
            breaches(member, &at, found);
        }
    }
    for keyword in ["items", "additionalProperties", "not"] {
        if let Some(member) = schema.get(keyword).filter(|member| member.is_object()) {
            breaches(member, &format!("{at}/{keyword}"), found);
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        for (index, member) in schema.get(keyword).and_then(Value::as_array).into_iter().flatten().enumerate() {
            breaches(member, &format!("{at}/{keyword}/{index}"), found);
        }
    }
}

fn holds_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(values) => values.iter().any(holds_null),
        Value::Object(members) => members.values().any(holds_null),
        _ => false,
    }
}

/// A struct changed without its file being regenerated fails here, naming the file; with
/// `NATIVE_MODULES_WRITE_SCHEMAS` set, the test writes each file instead.
#[test]
fn each_checked_in_schema_is_the_one_its_struct_generates() {
    for case in &CASES {
        let generated = text((case.schema)());
        let file = file(case);
        if env::var_os(WRITE).is_some() {
            fs::write(&file, &generated).unwrap_or_else(|error| panic!("{} is not written: {error}", file.display()));
            continue;
        }
        let checked_in =
            fs::read_to_string(&file).unwrap_or_else(|error| panic!("{} is not read: {error}", file.display()));
        assert!(
            checked_in == generated,
            "{} is not the schema its struct generates; {WRITE}=1 rewrites it",
            file.display()
        );
    }
}

/// Every schema keeps the profile, and a world-time field that several types carry reads the same in each and is
/// required by all of them or by none. A shared field is found by its reference to the `WorldTime` definition, so the
/// test names no wire name.
#[test]
fn each_schema_keeps_the_profile_and_each_shared_time_reads_alike_in_every_type() {
    let schemas: Vec<(&str, Value)> = CASES.iter().map(|case| (case.name, (case.schema)())).collect();
    let mut found = Vec::new();
    for (name, schema) in &schemas {
        if !schema.get("description").is_some_and(Value::is_string) {
            found.push(format!("{name}: no description"));
        }
        if holds_null(schema) {
            found.push(format!("{name}: a null value"));
        }
        breaches(schema, name, &mut found);
    }
    let time = Value::from("#/$defs/WorldTime");
    let requires = |schema: &Value, field: &str| {
        schema["required"].as_array().is_some_and(|names| names.iter().any(|name| name.as_str() == Some(field)))
    };
    for (index, (first, schema)) in schemas.iter().enumerate() {
        for (field, property) in schema["properties"].as_object().into_iter().flatten() {
            if property.get("$ref") != Some(&time) {
                continue;
            }
            for (second, other) in &schemas[index + 1..] {
                let Some(carried) = other["properties"].get(field) else { continue };
                if carried != property || requires(other, field) != requires(schema, field) {
                    found.push(format!("{field} reads otherwise in {first} and in {second}"));
                }
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// Lines a type's schema refuses, each breaking the one rule its comment names.
const REFUSED: [(&str, &str); 16] = [
    ("sku", r#"{"id":"S","units":1,"weight_kg":2}"#), // an unknown field
    ("sku", r#"["S",1]"#),                            // a record written as a list of its values
    ("sku", r#"{"id":"S","units":4294967296}"#),      // a count beyond 32 bits
    ("inbound_lot", r#"{"id":"L","lines":[]}"#),      // a lot of no lines
    ("inbound_lot", r#"{"id":"L","known_ms":null,"lines":[{"sku":"S","quantity":1}]}"#), // a null for an absent time
    ("inbound_lot", r#"{"id":"L","arrival_ms":-1,"lines":[{"sku":"S","quantity":1}]}"#), // a time before the run's start
    ("inbound_lot", r#"{"id":"L","lines":[{"sku":"S","quantity":0}]}"#),                 // a line of no units
    ("inbound_lot", r#"{"id":"L","lines":[{"sku":"S","quantity":1,"compartment":2}]}"#), // a compartment without its tote
    ("inbound_lot", r#"{"id":"L","totes":[],"lines":[{"sku":"S","quantity":1}]}"#),      // built totes stated as none
    ("inbound_lot", r#"{"id":"L","totes":["double"],"lines":[{"sku":"S","quantity":1,"tote_index":0}]}"#), // an unknown partition
    ("inbound_lot", r#"{"id":"L","totes":[{"single":null}],"lines":[{"sku":"S","quantity":1,"tote_index":0}]}"#), // a partition written as an object
    (
        "inbound_lot",
        r#"{"id":"L","totes":["quarter-partitioned"],"lines":[{"sku":"S","quantity":1,"tote_index":0,"compartment":4}]}"#,
    ), // a compartment past 3
    ("customer_order", r#"{"id":"O","lines":[{"sku":"S","quantity":0}]}"#), // an order line of no units
    ("customer_order", r#"{"id":"O","known_ms":1.5,"lines":[{"sku":"S","quantity":1}]}"#), // a time that is no integer
    ("customer_order", r#"{"id":"O","due_ms":9007199254740992,"lines":[{"sku":"S","quantity":1}]}"#), // a time past 2^53 - 1
    ("starting_tote", r#"{"id":"T","contents":[]}"#), // a tote that holds nothing
];

/// The schema is what validates a stream's bytes, so it must refuse each thing the contract rules out that the struct
/// alone would take or misread.
#[test]
fn each_schema_refuses_what_its_contract_rules_out() {
    for (name, line) in REFUSED {
        let case = CASES.iter().find(|case| case.name == name).expect("a refused line names a type");
        let line: Value = serde_json::from_str(line).expect("a refused line is JSON");
        assert!(jsonschema::validate(&(case.schema)(), &line).is_err(), "{name}'s schema takes {line}");
    }
}

/// Each example is read with the simulator's `SimTime` for a world time, as the simulator will read its streams.
#[test]
fn each_example_line_is_taken_by_its_schema_and_read_and_written_back_unchanged() {
    for case in &CASES {
        let line: Value = serde_json::from_str(case.example).expect("an example is JSON");
        if let Err(error) = jsonschema::validate(&(case.schema)(), &line) {
            panic!("{}'s schema refuses its example: {error}", case.name);
        }
        assert_eq!((case.reread)(case.example), line, "{}'s struct writes its example back otherwise", case.name);
    }
}
