//! Within the Map Creator adapter's tests, `schemas` is **the door the graph
//! server will put these components behind, stood up here first**: every
//! definition under `vocabulary/` is compiled the way the server compiles one,
//! and every payload of one of those components the adapter writes for all five
//! production exports is validated against its own.
//!
//! It exists because a published schema is not a document — it is a gate. The
//! server compiles each registered schema through `jsonschema` and validates
//! every write against it, so a schema that says a station's `pps_id` is a
//! number refuses two of the five sites at the door and nobody finds out until
//! a site is being imported. Compiling the schemas here and running the real
//! payloads through them is what turns "the schema is derived from the field
//! map" from an assertion about two data structures into one about the sites.
//!
//! The negative half matters as much: a schema that accepts everything would
//! pass the positive half without saying anything. So **every** property of
//! every component gets a payload with a value of a kind that property forbids,
//! and the validator has to refuse each one — references and the ordering key
//! included, which means following a `$ref` into the schema's own `$defs` to
//! find out what kind the definition states.
//!
//! The payload half needs the production exports, which are not in the
//! repository: it is ignored by an ordinary run and reads them from the
//! directory `MAPCREATOR_DUMPS` names when asked for with `--ignored`, failing
//! by name without it. The schema half needs no data and always runs.

use jsonschema::Validator;
use mapcreator_adapter_tests::{
    annotated_references, compiled, mapcreator, production_export, seed_refs, seed_schemas, vocabulary, CORE, EXTENDED,
    PRODUCTION, SITES,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use worldadapt_host::{Change, RandomIds, Site};

/// Every schema compiles the way the server compiles one, which is the first
/// thing the server's registry asks of it: `jsonschema` resolves every `$ref`
/// while building and fetches nothing off the machine, so a schema reaching for
/// a definition it does not carry fails here rather than turning up later as a
/// write nobody can validate.
#[test]
fn every_component_schema_compiles() {
    let definitions = vocabulary();
    for definition in definitions.values() {
        compiled(&definition["schema"]);
    }
    assert_eq!(definitions.len(), 21, "the vocabulary's size moved");
}

/// Every payload of a component under `vocabulary/`, or of a core component
/// the import writes Map Creator's fields into, that the adapter writes for all
/// five production exports validates against its component's schema — its own,
/// or the one the test helper derives from the protocol's — and every reference
/// it holds at an annotated path is one the server's reference gate resolves:
/// an entity the same ingest declares, carrying the component the annotation
/// expects where it names one. The counts are pinned too, because a walk that
/// found nothing would pass every validation it ran.
#[test]
#[ignore = "reads the five production exports from the directory MAPCREATOR_DUMPS names"]
fn every_payload_of_all_five_production_exports_validates() {
    let mut definitions = vocabulary();
    let seeded = seed_schemas();
    let mut published = seed_refs();
    for name in EXTENDED {
        let refs = published.remove(name).unwrap_or_else(|| json!([]));
        definitions.insert(name.to_string(), json!({ "schema": seeded[name].clone(), "refs": refs }));
    }
    let validators: Vec<(String, Validator)> =
        definitions.iter().map(|(name, definition)| (name.clone(), compiled(&definition["schema"]))).collect();
    let module = mapcreator();
    let mut validated = 0usize;
    let mut resolved = 0usize;
    for (site, _) in SITES {
        let empty = Site { scope: "tenant/site".into(), entities: Vec::new() };
        let changes = module
            .ingest(PRODUCTION, &production_export(site), &empty, Box::new(RandomIds))
            .unwrap_or_else(|error| panic!("{site}: {error}"));
        let declared: BTreeMap<&str, &Map<String, Value>> = changes
            .iter()
            .filter_map(|change| match change {
                Change::Declare(write) => Some((write.id.as_str(), &write.components)),
                _ => None,
            })
            .collect();
        for (id, components) in &declared {
            for (component, payload) in *components {
                if !definitions.contains_key(component) {
                    continue;
                }
                let (_, validator) = validators
                    .iter()
                    .find(|(name, _)| name == component)
                    .unwrap_or_else(|| panic!("{site}: `{component}` is written and not defined"));
                if let Err(refusal) = validator.validate(payload) {
                    panic!("{site}: entity {id}'s `{component}` payload is refused by its own schema: {refusal}");
                }
                validated += 1;
                for (path, named, expects) in annotated_references(payload, &definitions[component]["refs"]) {
                    let target = named.as_str().and_then(|target| declared.get(target));
                    assert!(
                        target.is_some_and(|target| expects.is_none_or(|expects| target.contains_key(expects))),
                        "{site}: entity {id} holds {named} at `{component}{path}`, which names no entity of the ingest \
                         carrying `{expects:?}`"
                    );
                    resolved += 1;
                }
            }
        }
    }
    // Pinned rather than bounded, so that a mapping change which stops writing a
    // component shows up here as a number rather than as a test that still
    // passes on the payloads it happens to have left. 140,801 since how a file
    // wrote a record moved to `import_formatting`: each of the 70,381 record
    // entities carries its component and its formatting, each of the 14 sectors
    // its formatting alone, and the frames and export entities their 25;
    // 140,831 since the shared `presentation_point` is validated too, which each
    // of the 30 older-dialect station points carries empty beside its own.
    assert_eq!(
        validated, 140_831,
        "the number of payloads of the vocabulary's components across the five exports moved"
    );
    assert_eq!(resolved, 81_332, "the number of annotated references across the five exports moved");
}

/// Every definition annotates each field that holds a reference — a property
/// whose schema is an entity identity, or a list of them — at its JSON
/// pointer, with a component this vocabulary defines as what the target
/// carries, and annotates nothing else: the server indexes a reference only
/// at a path a schema version's annotations name, and refuses a delete that
/// would leave one dangling only there. The joins are among them, a bin's type,
/// its seats and its station included, and every path is one the server's
/// extractor reads: a pointer with at most one trailing `/*`.
#[test]
fn every_reference_field_is_annotated_with_what_its_target_carries() {
    let definitions = vocabulary();
    let mut annotations = 0usize;
    for (component, definition) in &definitions {
        let mut typed = Vec::new();
        reference_paths(&definition["schema"]["properties"], "", &mut typed);
        typed.sort();
        let refs = definition["refs"].as_array().unwrap_or_else(|| panic!("`{component}` states no `refs`"));
        let mut annotated = Vec::new();
        for one in refs {
            let path = one["path"].as_str().expect("a path");
            let expects = one["expects"].as_str().expect("what the target carries");
            assert!(
                read_by_the_server(path),
                "`{component}` annotates {path}, which the server's extractor does not read"
            );
            assert!(
                definitions.contains_key(expects) || CORE.contains(&expects),
                "`{component}` annotates {path} as expecting `{expects}`"
            );
            annotated.push(path.to_string());
            annotations += 1;
        }
        assert_eq!(annotated, typed, "`{component}`'s annotations and its reference fields disagree");
    }
    let bin = definitions["pps_bin"]["refs"].as_array().expect("a bin's annotations");
    for (path, expects) in [
        ("/bin_type", "pps_bin_type"),
        ("/front_seat", "pps_seat"),
        ("/back_seat", "pps_seat"),
        ("/station", "mapcreator.station"),
    ] {
        assert!(bin.contains(&json!({ "path": path, "expects": expects })), "a bin's {path}");
    }
    // 26 since the cell's location profile and zone, and a presentation point's
    // cell and station, moved with the rest of their fields into the protocol's
    // `cell` and `presentation_point`, whose annotations the seed publishes.
    assert_eq!(annotations, 26, "the number of annotated reference fields across the vocabulary moved");
}

/// The JSON pointers of every property of a schema typed as an entity identity
/// (`/field`) or a list of them (`/field/*`), nested objects included.
fn reference_paths(properties: &Value, under: &str, out: &mut Vec<String>) {
    for (name, property) in properties.as_object().into_iter().flatten() {
        let path = format!("{under}/{name}");
        match property.get("$ref").and_then(Value::as_str) {
            Some("#/$defs/entity_id") => out.push(path.clone()),
            Some("#/$defs/entity_ids") => out.push(format!("{path}/*")),
            _ => {}
        }
        if let Some(inner) = property.get("properties") {
            reference_paths(inner, &path, out);
        }
    }
}

/// A JSON pointer of snake-case segments, ending `/*` at most once, which is
/// as far as the server's extractor follows a list.
fn read_by_the_server(path: &str) -> bool {
    let fields = path.strip_suffix("/*").unwrap_or(path);
    fields.strip_prefix('/').is_some_and(|fields| {
        fields.split('/').all(|field| {
            !field.is_empty() && field.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
    })
}

/// Every property of every component refuses a value of a kind that property
/// forbids.
///
/// This is the half that makes the positive half mean something: a schema that
/// accepted everything would validate all 70,420 production payloads and assert
/// nothing. The
/// schemas are deliberately open to *fields* a newer producer adds (SPEC §9), so
/// the one thing they do assert is the **kind** of a field they know — and this is
/// the test that says they assert it, for every field rather than for one.
///
/// One property per component used to be the whole of it: the alphabetically first
/// one that stated a `type` outright, always refused with `null`. That skipped
/// every `$ref`-typed property — the ordering key and every reference field, which
/// is most of what these components carry — so a schema that had lost its
/// `entity_id` references would have passed. Here a `$ref` is followed into the
/// schema's own `$defs` and the definition's kind is what the wrong value is chosen
/// against, nested objects are walked into, and the ordering key's `minimum` and
/// `maximum` are checked besides.
///
/// The one property that states no kind is `legacy_value`, and it states none on
/// purpose: a legacy value comes back in whatever JSON kind the dump wrote it, and
/// a schema that corrected the kind would be the converter deciding what the site
/// meant. The test asserts that is the only one.
#[test]
fn every_property_refuses_a_value_of_a_kind_it_forbids() {
    let mut refused = 0usize;
    let mut untyped = 0usize;
    let mut states_no_kind: BTreeSet<String> = BTreeSet::new();
    for (component, definition) in vocabulary() {
        let schema = &definition["schema"];
        let validator = compiled(schema);
        let defs = schema["$defs"].as_object().cloned().unwrap_or_default();
        let properties = schema["properties"].as_object().expect("properties");
        let mut cases = Vec::new();
        collect_cases(properties, &defs, &[], &mut cases, &mut untyped, &mut states_no_kind, &component);
        assert!(!cases.is_empty(), "`{component}` has no property that states a kind");
        // A payload carrying only a wrong value would be refused by a `required`
        // field it also lacks, and the test would pass without the kind ever being
        // read. So each case starts from a payload that validates — the required
        // fields, filled from what their own schemas say, and those of every
        // object on the way to the case's field — and the wrong value is the one
        // difference between the two.
        for (path, wrong) in cases {
            let control = control_for(schema, &path);
            assert!(
                validator.validate(&control).is_ok(),
                "`{component}`'s required fields on the way to `{}` cannot be filled from its own schema: {control}",
                path.join(".")
            );
            let mut payload = control.as_object().cloned().expect("an object");
            set_at(&mut payload, &path, wrong.clone());
            let payload = Value::Object(payload);
            assert!(
                validator.validate(&payload).is_err(),
                "`{component}` accepts {wrong} for `{}`, so its schema states nothing about that field",
                path.join(".")
            );
            refused += 1;
        }
    }
    // Pinned rather than bounded: a mapping change that stopped deriving a whole
    // component's properties would otherwise leave a test that still passes on the
    // properties it happens to have left.
    // 189 since a bin's seats and type, a light's bin and station, and a mapping
    // row's conveyor became references, the six lists that resolve partly keep
    // their unresolved entries with their places, a preference's sectors the
    // sixth, and five presence facts a record or a file states found a home, a
    // presentation point's stated absence of `world_coordinates` the fifth; 237
    // since every component states how its record's file wrote it, `key_order`
    // and `written_with_point`, and the export entity the usual key orders and
    // each file's layout; 250 since how a file wrote a record left the
    // components that state the site for `import_formatting` — the 22 remaining
    // components less their three each and nine more (66 + 9), the export entity
    // keeping two of the three and taking the empty profile list (3), the
    // sector's spelling component gone (4), and the formatting's 21 entries with
    // their 68 fields come in (89); 220 since the cell's 17 typed properties and
    // a presentation point's 13 moved into the protocol's `cell` and
    // `presentation_point`, and their four legacy values with them; 240 since
    // a zero written `-0.0` is recorded beside a whole number written `40.0`,
    // in each of the 19 record kinds' formatting entries and on the export
    // entity (20); 243 since a preference's entry states how the file wrote
    // its list, as a record's does (3).
    assert_eq!(refused, 243, "the number of typed properties across the vocabulary moved");
    assert_eq!(
        states_no_kind.into_iter().collect::<Vec<String>>(),
        vec!["#/$defs/legacy_value".to_string()],
        "a property states no kind and is not a legacy value"
    );
    assert_eq!(untyped, 21, "the number of legacy-value properties moved");
}

/// The ordering key's bounds are the schema's, not only the converter's.
///
/// `record_index` is where the key a consumer reads back as a JSON number is
/// bounded, so the schema has to refuse a negative one and one past 2^53 − 1 —
/// a key above that range comes back to a reader whose numbers are doubles as a
/// different key, and two records can come back with one key between them.
#[test]
fn the_ordering_key_refuses_a_value_outside_the_exact_integer_range() {
    let formatting = vocabulary().remove("import_formatting").expect("the formatting is defined");
    let validator = compiled(&formatting["schema"]);
    let cell = |index: Value| json!({ "mapcreator_cell": { "index": index } });
    for refused in [json!(-1), json!(9_007_199_254_740_992i64), json!(1.5)] {
        assert!(validator.validate(&cell(refused.clone())).is_err(), "the ordering key accepts {refused}");
    }
    assert!(validator.validate(&cell(json!(9_007_199_254_740_991i64))).is_ok(), "the ceiling itself is a key");
    assert!(validator.validate(&cell(json!(0))).is_ok(), "zero is a key");
}

/// One wrong-kind case per property of one component's schema, walking into a
/// property's own properties and following a `$ref` into the schema's `$defs`.
///
/// A property whose resolved definition states no kind is recorded rather than
/// tested: there is one such definition and the caller asserts which.
fn collect_cases(
    properties: &Map<String, Value>,
    defs: &Map<String, Value>,
    prefix: &[String],
    out: &mut Vec<(Vec<String>, Value)>,
    untyped: &mut usize,
    states_no_kind: &mut BTreeSet<String>,
    component: &str,
) {
    for (name, property) in properties {
        let mut path = prefix.to_vec();
        path.push(name.clone());
        let resolved = match property.get("$ref").and_then(Value::as_str) {
            Some(reference) => {
                let key = reference.strip_prefix("#/$defs/").unwrap_or_else(|| {
                    panic!("`{component}`.{name} names `{reference}`, which is not one of its own definitions")
                });
                defs.get(key).unwrap_or_else(|| panic!("`{component}`.{name} names `{key}` and its $defs carries none"))
            }
            None => property,
        };
        match kinds_of_property(resolved) {
            Some(allowed) => {
                let wrong = candidates()
                    .into_iter()
                    .find(|value| kinds_of_value(value).iter().all(|kind| !allowed.iter().any(|one| one == kind)))
                    .unwrap_or_else(|| panic!("`{component}`.{name} allows every JSON kind: {allowed:?}"));
                out.push((path.clone(), wrong));
            }
            None => {
                *untyped += 1;
                states_no_kind.insert(property.get("$ref").and_then(Value::as_str).unwrap_or(name).to_string());
            }
        }
        if let Some(inner) = resolved.get("properties").and_then(Value::as_object) {
            collect_cases(inner, defs, &path, out, untyped, states_no_kind, component);
        }
    }
}

/// The JSON kinds one resolved property allows, or `None` where it states none.
fn kinds_of_property(property: &Value) -> Option<Vec<String>> {
    match property.get("type") {
        Some(Value::String(one)) => Some(vec![one.clone()]),
        Some(Value::Array(many)) => Some(many.iter().filter_map(Value::as_str).map(str::to_string).collect()),
        _ => None,
    }
}

/// The kinds one value is. A whole number is both a `number` and an `integer`,
/// and a fractional one is only a `number` — which is the difference the ordering
/// key's own type turns on.
fn kinds_of_value(value: &Value) -> Vec<&'static str> {
    match value {
        Value::Null => vec!["null"],
        Value::Bool(_) => vec!["boolean"],
        Value::String(_) => vec!["string"],
        Value::Number(number) if number.as_f64().is_some_and(|n| n.fract() == 0.0) => vec!["number", "integer"],
        Value::Number(_) => vec!["number"],
        Value::Array(_) => vec!["array"],
        Value::Object(_) => vec!["object"],
    }
}

/// One value of each JSON kind, which is what a wrong kind is chosen out of. The
/// whole number and the fractional one are both here because `integer` allows the
/// first and forbids the second.
fn candidates() -> [Value; 7] {
    [json!(null), json!(true), json!("a string"), json!(7), json!(7.5), json!([]), json!({})]
}

/// One value put at the end of a path of object keys, making the objects on the
/// way as it goes: `["a", "b"] -> {"a":{"b":…}}`.
fn set_at(object: &mut Map<String, Value>, path: &[String], value: Value) {
    let Some((key, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        object.insert(key.clone(), value);
        return;
    }
    let inner = object.entry(key.clone()).or_insert_with(|| Value::Object(Map::new()));
    if !inner.is_object() {
        *inner = Value::Object(Map::new());
    }
    set_at(inner.as_object_mut().expect("just made an object"), rest, value);
}

/// The smallest payload one object's schema accepts: every field it requires,
/// filled with a value that field's own schema allows, a `$ref` followed into
/// the root schema's `$defs` to find what that is.
fn required_sample(object: &Value, root: &Value) -> Map<String, Value> {
    let mut payload = Map::new();
    for name in object.get("required").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let name = name.as_str().expect("a required field is named by a string");
        let property = &object["properties"][name];
        assert!(property.is_object(), "`{name}` is required and is no property");
        payload.insert(name.to_string(), allowed_value(resolved(property, root)));
    }
    payload
}

/// A property, or the definition of the root schema's it names.
fn resolved<'s>(property: &'s Value, root: &'s Value) -> &'s Value {
    match property.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let key = reference.strip_prefix("#/$defs/").expect("a reference into the schema's own $defs");
            &root["$defs"][key]
        }
        None => property,
    }
}

/// The payload one case starts from: the component's required fields, and
/// those of every object on the path to the case's field, so that the wrong
/// value put there is the one thing the schema can refuse.
fn control_for(root: &Value, path: &[String]) -> Value {
    let mut control = required_sample(root, root);
    let mut object = root;
    let mut at: Vec<String> = Vec::new();
    for step in &path[..path.len().saturating_sub(1)] {
        object = resolved(&object["properties"][step], root);
        at.push(step.clone());
        for (name, value) in required_sample(object, root) {
            let mut place = at.clone();
            place.push(name);
            set_at(&mut control, &place, value);
        }
        if !object["required"].is_array() {
            // An object on the way that requires nothing is still an object.
            let here = at.clone();
            if get_at(&control, &here).is_none() {
                set_at(&mut control, &here, json!({}));
            }
        }
    }
    Value::Object(control)
}

/// The value at the end of a path of object keys, if every step is there.
fn get_at<'v>(object: &'v Map<String, Value>, path: &[String]) -> Option<&'v Value> {
    let (key, rest) = path.split_first()?;
    let value = object.get(key)?;
    if rest.is_empty() {
        return Some(value);
    }
    get_at(value.as_object()?, rest)
}

/// One value a property allows: the first of its `enum` where it states one, and
/// otherwise the plainest value of the kind it states, or of the first kind it
/// states where it states several.
fn allowed_value(property: &Value) -> Value {
    if let Some(first) = property.get("enum").and_then(Value::as_array).and_then(|members| members.first()) {
        return first.clone();
    }
    let kind = match property.get("type") {
        Some(Value::Array(kinds)) => kinds.first().and_then(Value::as_str),
        other => other.and_then(Value::as_str),
    };
    match kind {
        Some("string") => json!("x"),
        Some("integer") | Some("number") => json!(0),
        Some("boolean") => json!(false),
        Some("array") => json!([]),
        _ => json!({}),
    }
}
