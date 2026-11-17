//! Within the Map Creator adapter, `layout` is **where each file of an export
//! keeps its records, checked before anything reads them**: every container
//! in the JSON kind the format fixes, every record an object, every object a
//! record nests an object, and every key a record is written back with present
//! and in the kind its file fixes.
//!
//! It exists because the readers after it take a container of the wrong kind
//! for an empty one and a missing key for an empty string, and the export then
//! writes back an empty list or an invented key: an export that goes in one
//! shape and comes back another, with nobody told. Each check is a refusal
//! naming the file, the record and the rule. A file in one of its two empty
//! forms, `[]` or `{}`, holds no records and is carried as that form.

use crate::dump::{Dump, Error};
use crate::kinds::FileKind;
use crate::records::{self, Field, KeyText, Slot};
use serde_json::{Map, Value};

/// Where a flat file keeps its records: in the file itself, under one key of
/// the file, or under one key of each group the file lists.
enum Holds {
    Itself,
    Under(&'static str),
    Groups(&'static str),
}

fn holds(file: FileKind) -> Option<Holds> {
    match file {
        FileKind::Charger
        | FileKind::DockPoint
        | FileKind::ToteStorage
        | FileKind::ToteType
        | FileKind::ConveyorMapping => Some(Holds::Itself),
        FileKind::OdsExcluded => Some(Holds::Under("ods_excluded_list")),
        FileKind::Conveyor => Some(Holds::Under("conveyor_data")),
        FileKind::PpsBin | FileKind::PpsBinType => Some(Holds::Under("data")),
        FileKind::PpsSeat => Some(Holds::Under("pps_seats")),
        FileKind::Pptl => Some(Holds::Groups("pptls")),
        _ => None,
    }
}

/// The place a flat file's records stand at, spelled as `written` spells a
/// place: the file's name, `.key` into an object and `[]` into a list.
pub fn record_place(file: FileKind) -> String {
    match holds(file) {
        Some(Holds::Itself) => format!("{file}[]"),
        Some(Holds::Under(key)) => format!("{file}.{key}[]"),
        Some(Holds::Groups(key)) => format!("{file}[].{key}[]"),
        None => file.to_string(),
    }
}

/// Where one record of a flat file stands in it, as a pointer: by its group,
/// which only `pptl.json` has, and its place in the list that holds it.
pub fn record_pointer(file: FileKind, group: usize, at: usize) -> String {
    match holds(file) {
        Some(Holds::Under(key)) => format!("/{key}/{at}"),
        Some(Holds::Groups(key)) => format!("/{group}/{key}/{at}"),
        Some(Holds::Itself) | None => format!("/{at}"),
    }
}

/// The records of one flat file, each with its group in the file (which only
/// `pptl.json` has), in file order. Read after [`check`] has passed, which is
/// what makes every one an object.
pub fn flat_rows(file: FileKind, value: &Value) -> Vec<(usize, &Map<String, Value>)> {
    fn objects(list: Option<&Value>) -> Vec<&Map<String, Value>> {
        list.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_object).collect()
    }
    match holds(file) {
        Some(Holds::Itself) => objects(Some(value)).into_iter().map(|row| (0, row)).collect(),
        Some(Holds::Under(key)) => objects(value.get(key)).into_iter().map(|row| (0, row)).collect(),
        Some(Holds::Groups(key)) => objects(Some(value))
            .into_iter()
            .enumerate()
            .flat_map(|(group, list)| objects(list.get(key)).into_iter().map(move |row| (group, row)))
            .collect(),
        None => Vec::new(),
    }
}

/// Every file of the export laid out as the format lays it out, or a sentence
/// naming the first place it is not.
pub fn check(dump: &Dump) -> Result<(), Error> {
    for kind in dump.present() {
        let Some(value) = dump.get(kind) else { continue };
        let empty = value.as_array().is_some_and(Vec::is_empty) || value.as_object().is_some_and(Map::is_empty);
        if empty && kind != FileKind::Map {
            continue;
        }
        let file = kind.name();
        match kind {
            FileKind::Zone => {
                for (at, row) in list_of_records(value, file, file)?.into_iter().enumerate() {
                    let place = format!("{file} record {}", at + 1);
                    let zone = object_in(row, "zonerec", &place)?;
                    fields(&place, file, records::ZONE, zone)?;
                }
            }
            FileKind::Map => map(value)?,
            FileKind::SectorMxUPreferences => preferences(value)?,
            FileKind::Block => {
                for (at, row) in list_of_records(value, file, file)?.into_iter().enumerate() {
                    let place = format!("{file} record {}", at + 1);
                    let details = object_in(row, "entity_details", &place)?;
                    fields(&place, file, records::BLOCK, details)?;
                    list_in(details, "coordinates", &place)?;
                    stated(&place, file, row, "entity_type", Value::is_string, "a string")?;
                }
            }
            FileKind::Pps => {
                for (at, row) in list_of_records(value, file, file)?.into_iter().enumerate() {
                    station(&format!("{file} record {}", at + 1), row)?;
                }
            }
            FileKind::VtmZone => {
                let aisles =
                    list_of_records(under(value, file, "aisle_info")?, file, &format!("{file}'s `aisle_info`"))?;
                for (at, row) in aisles.into_iter().enumerate() {
                    let place = format!("{file} aisle {}", at + 1);
                    fields(&place, file, records::AISLE, row)?;
                    list_in(row, "io_point_coordinates", &place)?;
                }
            }
            // Empty on every export seen, so a record in one is a field family
            // nobody has decided about, whatever kind of value it is.
            FileKind::Elevator
            | FileKind::FireEmergency
            | FileKind::QueueData
            | FileKind::OpcConnection
            | FileKind::ZoneController => {
                return Err(Error::Import(format!(
                    "{file} holds {}, and every export seen ships it empty, so where its contents would go is \
                     undecided",
                    kind_of(value)
                )));
            }
            other => {
                if let Some(holds) = holds(other) {
                    flat(other, value, holds)?;
                }
            }
        }
    }
    Ok(())
}

/// `map.json`: exactly one floor object, its `floor_id` a whole number, its
/// cells and permission bundles records, and its vendor sample point an
/// object.
fn map(value: &Value) -> Result<(), Error> {
    let file = "map.json";
    let floors = value.as_array().ok_or_else(|| {
        Error::Import(format!("{file} is {}, where an export keeps a list holding one floor", kind_of(value)))
    })?;
    if floors.len() != 1 {
        return Err(Error::Import(format!(
            "{file} states {} floor objects, and an export is one floor: the projection would write the first \
             back and drop the rest",
            floors.len()
        )));
    }
    let floor = floors[0].as_object().ok_or_else(|| {
        Error::Import(format!("{file}'s floor is {}, where the format keeps an object", kind_of(&floors[0])))
    })?;
    match floor.get("floor_id") {
        Some(Value::Number(number)) if number.is_i64() => {}
        Some(other) => {
            return Err(Error::Import(format!(
                "{file} states `floor_id` as {}, and the format fixes a whole number: the export writes the floor \
                 id back as one, so it would come back changed",
                described(other)
            )));
        }
        None => {
            return Err(Error::Import(format!(
                "{file} states no `floor_id`, and the export writes one back: it would come back with an invented \
                 floor"
            )));
        }
    }
    let cells = list_of_records(
        floor.get("map_values").ok_or_else(|| Error::Import(format!("{file} has no `map_values` list")))?,
        file,
        &format!("{file}'s `map_values`"),
    )?;
    for (at, cell) in cells.into_iter().enumerate() {
        let place = match cell.get("coordinate").and_then(Value::as_str) {
            Some(coordinate) => format!("{file} cell {coordinate}"),
            None => format!("{file} cell {}", at + 1),
        };
        fields(&place, file, records::CELL, cell)?;
        stated(&place, file, cell, "blocked", Value::is_boolean, "true or false")?;
    }
    if let Some(profiles) = floor.get("location_profiles") {
        for (at, profile) in
            list_of_records(profiles, file, &format!("{file}'s `location_profiles`"))?.into_iter().enumerate()
        {
            fields(&format!("{file} location profile {}", at + 1), file, records::LOCATION_PROFILE, profile)?;
        }
    }
    if let Some(reference) = floor.get("vda5050_reference") {
        if !reference.is_object() {
            return Err(Error::Import(format!(
                "{file} states `vda5050_reference` as {}, where the format nests an object",
                described(reference)
            )));
        }
    }
    Ok(())
}

/// `sectorMxUPreferences.json`: one object of carrier types, or that object
/// as the one element of a list, each carrier's preference a list.
fn preferences(value: &Value) -> Result<(), Error> {
    let file = "sectorMxUPreferences.json";
    let table = match value {
        Value::Object(table) => table,
        Value::Array(items) => match items.as_slice() {
            [Value::Object(table)] => table,
            _ => {
                return Err(Error::Import(format!(
                    "{file} is a list of {} entries, and the format keeps one object of carrier types, alone or as \
                     the one entry of a list",
                    items.len()
                )));
            }
        },
        other => {
            return Err(Error::Import(format!(
                "{file} is {}, where the format keeps one object of carrier types",
                described(other)
            )));
        }
    };
    for (carrier, preferred) in table {
        if !preferred.is_array() {
            return Err(Error::Import(format!(
                "{file} gives carrier type `{carrier}` {}, where the format keeps a list of sectors: read as an empty \
                 one, it would come back as `[]`",
                described(preferred)
            )));
        }
    }
    Ok(())
}

/// One station of `pps.json`, in whichever dialect it is written: its id
/// present, and its parts in the lists the dialect keeps them in.
fn station(place: &str, row: &Map<String, Value>) -> Result<(), Error> {
    let file = "pps.json";
    if !row.contains_key("pps_id") {
        return Err(Error::Import(format!(
            "{place} states no `pps_id`, the key a station is known by: the import would key it by an empty string \
             and the export would write one back"
        )));
    }
    let v3 = row.contains_key("entities");
    let map = if v3 { records::STATION_V3 } else { records::STATION_V2 };
    fields(place, file, map, row)?;
    if let Some(kind) = row.get("type") {
        if !kind.is_string() {
            return Err(Error::Import(format!(
                "{place} states `type` as {}, where the format fixes a string: the export writes a station's type \
                 back as one, so it would come back as `\"\"`",
                described(kind)
            )));
        }
    }
    if v3 {
        let entities = object_in(row, "entities", place)?;
        if let Some(points) = entities.get("presentation_points") {
            let what = format!("{place}'s `entities.presentation_points`");
            for (at, point) in list_of_records(points, file, &what)?.into_iter().enumerate() {
                let point_place = format!("{place}, presentation point {}", at + 1);
                fields(&point_place, file, records::PRESENTATION_POINT, point)?;
                // A point's purpose lives in the core `presentation_point.for`,
                // which holds a string, and the export writes `presented_for`
                // back from it alone.
                if let Some(purpose) = point.get("presented_for").filter(|purpose| !purpose.is_string()) {
                    return Err(Error::Import(format!(
                        "{point_place} states `presented_for` as {}, where the format fixes a string: the export \
                         writes a point's purpose back from the core `presentation_point.for`, which holds a string, \
                         so the point would come back without `presented_for`",
                        described(purpose)
                    )));
                }
            }
        }
        if entities.contains_key("blocks") {
            list_in(entities, "blocks", &format!("{place}'s `entities`"))?;
        }
    } else if let Some(points) = row.get("pps_points") {
        for (at, point) in list_of_records(points, file, &format!("{place}'s `pps_points`"))?.into_iter().enumerate() {
            fields(&format!("{place}, point {}", at + 1), file, records::STATION_POINT, point)?;
        }
    }
    Ok(())
}

/// One flat file: its records where the format keeps them, each with the keys
/// its field map writes back.
fn flat(kind: FileKind, value: &Value, holds: Holds) -> Result<(), Error> {
    let file = kind.name();
    let map = records::KINDS.iter().find(|record| record.file == kind).map(|record| record.fields).unwrap_or_default();
    let mut rows: Vec<(String, &Map<String, Value>)> = Vec::new();
    match holds {
        Holds::Itself => {
            for (at, row) in list_of_records(value, file, file)?.into_iter().enumerate() {
                rows.push((format!("{file} record {}", at + 1), row));
            }
        }
        Holds::Under(key) => {
            for (at, row) in
                list_of_records(under(value, file, key)?, file, &format!("{file}'s `{key}`"))?.into_iter().enumerate()
            {
                rows.push((format!("{file} record {}", at + 1), row));
            }
        }
        Holds::Groups(key) => {
            for (group, row) in list_of_records(value, file, file)?.into_iter().enumerate() {
                let place = format!("{file} group {}", group + 1);
                for (at, device) in list_of_records(list_in(row, key, &place)?, file, &place)?.into_iter().enumerate() {
                    rows.push((format!("{place}, device {}", at + 1), device));
                }
            }
        }
    }
    for (place, row) in rows {
        fields(&place, file, map, row)?;
        match kind {
            FileKind::OdsExcluded => key(&place, file, row, "ods_tuple", KeyText::Text)?,
            // The row's key is its conveyor and its step. The step is a key the
            // field map checks; the conveyor is a reference too, which a value
            // of any kind is carried as, and only its absence is refused.
            FileKind::ConveyorMapping => {
                if !object_in(row, "conveyor_coordinates", &place)?.contains_key("conveyor_id") {
                    return Err(Error::Import(format!(
                        "{place}'s `conveyor_coordinates` states no `conveyor_id`, half of the key a mapping row is \
                         known by: the import would key it by an empty string and the export would write one back"
                    )));
                }
            }
            // A bin's and a seat's station likewise: half of the key and a
            // reference besides.
            FileKind::PpsBin | FileKind::PpsSeat => {
                if !row.contains_key("pps_id") {
                    return Err(Error::Import(format!(
                        "{place} states no `pps_id`, the station half of the key a record of {file} is known by: \
                         the import would key it by an empty string and the export would write one back"
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// One record against its field map: every key the export writes back present
/// and in its file's kind, every nested object an object, and every list a
/// list.
fn fields(place: &str, file: &str, map: &[Field], record: &Map<String, Value>) -> Result<(), Error> {
    for field in map {
        match field.slot {
            Slot::Key { text, .. } => key(place, file, record, field.source, text)?,
            Slot::Nested(inner) => {
                if let Some(nested) = record.get(field.source) {
                    let nested = nested.as_object().ok_or_else(|| {
                        Error::Import(format!(
                            "{place} states `{}` as {}, where the format nests an object: read as an empty one, it \
                             would come back as `{{}}`",
                            field.source,
                            described(nested)
                        ))
                    })?;
                    fields(&format!("{place}'s `{}`", field.source), file, inner, nested)?;
                }
            }
            Slot::Cells(_) | Slot::Blocks => {
                if record.contains_key(field.source) {
                    list_in(record, field.source, place)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// One field every record of a kind states, which the export writes every
/// such record back with, present and of the kind the format fixes; read
/// otherwise, it would come back invented.
fn stated(
    place: &str,
    file: &str,
    record: &Map<String, Value>,
    name: &str,
    fixed: fn(&Value) -> bool,
    kind: &str,
) -> Result<(), Error> {
    match record.get(name) {
        Some(value) if fixed(value) => Ok(()),
        Some(value) => Err(Error::Import(format!(
            "{place} states `{name}` as {}, and {file} fixes it as {kind}: the export writes it back as one, so it \
             would come back changed",
            described(value)
        ))),
        None => Err(Error::Import(format!(
            "{place} states no `{name}`, and the export writes every such record of {file} back with one: it would \
             come back with an invented `{name}`"
        ))),
    }
}

/// One key the export writes back, present and in the JSON kind its file
/// fixes: `external_ids` carries every key as a string, and the export spells
/// it back in that kind.
fn key(place: &str, file: &str, record: &Map<String, Value>, name: &str, kind: KeyText) -> Result<(), Error> {
    let Some(value) = record.get(name) else {
        return Err(Error::Import(format!(
            "{place} states no `{name}`, and the export writes every such record of {file} back with one: it would \
             come back with an invented `{name}`"
        )));
    };
    let fixed = match kind {
        KeyText::Text => value.is_string(),
        KeyText::Number => value.is_number(),
    };
    if !fixed {
        return Err(Error::Import(format!(
            "{place} states `{name}` as {}, and {file} fixes it as {}: the export writes a key back in its file's \
             kind, so it would come back changed",
            described(value),
            match kind {
                KeyText::Text => "a string",
                KeyText::Number => "a number",
            }
        )));
    }
    Ok(())
}

/// A list every element of which is a record.
fn list_of_records<'v>(value: &'v Value, file: &str, what: &str) -> Result<Vec<&'v Map<String, Value>>, Error> {
    let items = value.as_array().ok_or_else(|| {
        Error::Import(format!("{what} is {}, where {file} keeps a list of records", described(value)))
    })?;
    items
        .iter()
        .enumerate()
        .map(|(at, item)| {
            item.as_object().ok_or_else(|| {
                Error::Import(format!(
                    "{what} holds {} as its entry {}, where {file} keeps a record",
                    described(item),
                    at + 1
                ))
            })
        })
        .collect()
}

/// The value under one key of a file that is an object.
fn under<'v>(value: &'v Value, file: &str, key: &str) -> Result<&'v Value, Error> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Import(format!("{file} is {}, where the format keeps an object", described(value))))?;
    object.get(key).ok_or_else(|| {
        Error::Import(format!(
            "{file} states no `{key}` list, and the export writes one back: it would come back with an empty one"
        ))
    })
}

/// The object one record nests under a key the format requires.
fn object_in<'v>(record: &'v Map<String, Value>, key: &str, place: &str) -> Result<&'v Map<String, Value>, Error> {
    match record.get(key) {
        Some(Value::Object(object)) => Ok(object),
        Some(other) => Err(Error::Import(format!(
            "{place} states `{key}` as {}, where the format nests an object: read as an empty one, it would come \
             back as `{{}}`",
            described(other)
        ))),
        None => Err(Error::Import(format!(
            "{place} has no `{key}` object, which the export writes back: it would come back with an empty one"
        ))),
    }
}

/// The list one record keeps under a key, which the export writes back.
fn list_in<'v>(record: &'v Map<String, Value>, key: &str, place: &str) -> Result<&'v Value, Error> {
    match record.get(key) {
        Some(list @ Value::Array(_)) => Ok(list),
        Some(other) => Err(Error::Import(format!(
            "{place} states `{key}` as {}, where the format keeps a list: read as an empty one, it would come back \
             as `[]`",
            described(other)
        ))),
        None => Err(Error::Import(format!(
            "{place} has no `{key}` list, which the export writes back: it would come back with an empty one"
        ))),
    }
}

/// A value's JSON kind, said in words: "null", "a number".
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// A value's kind and, for a scalar, the value itself, for a sentence that
/// lets a person find it.
fn described(value: &Value) -> String {
    match value {
        Value::Number(_) | Value::String(_) | Value::Bool(_) => format!("{} ({value})", kind_of(value)),
        other => kind_of(other).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::check;
    use crate::dump::Dump;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    fn dump(files: &[(&str, Value)]) -> Dump {
        let mut all: BTreeMap<String, Value> = BTreeMap::new();
        all.insert("map.json".into(), json!([{ "floor_id": 1, "map_values": [] }]));
        for (name, value) in files {
            all.insert(name.to_string(), value.clone());
        }
        Dump::from_files(all).expect("a dump")
    }

    /// Each place the format fixes a shape refuses a value of another, naming
    /// the file, the record and the rule.
    #[test]
    fn a_container_record_or_key_of_the_wrong_shape_is_refused_where_it_stands() {
        for (files, says) in [
            (vec![("charger.json", json!(null))], "charger.json is null, where charger.json keeps a list of records"),
            (vec![("charger.json", json!(7))], "charger.json is a number (7), where"),
            (vec![("elevator.json", json!([1]))], "elevator.json holds a list, and every export seen ships it empty"),
            (
                vec![("charger.json", json!([1]))],
                "charger.json holds a number (1) as its entry 1, where charger.json keeps a record",
            ),
            (
                vec![("charger.json", json!([{ "charger_id": "7" }]))],
                "charger.json record 1 states `charger_id` as a string (\"7\"), and charger.json fixes it as a number",
            ),
            (
                vec![("zone.json", json!([{ "zonerec": { "zone_id": 3 } }]))],
                "zone.json record 1 states `zone_id` as a number (3), and zone.json fixes it as a string",
            ),
            (vec![("zone.json", json!([{ "zonerec": {} }]))], "zone.json record 1 states no `zone_id`"),
            (
                vec![("zone.json", json!([{ "zonerec": null }]))],
                "zone.json record 1 states `zonerec` as null, where the format nests an object",
            ),
            (
                vec![("tote_storage.json", json!([{ "location": "T1", "io_point": null }]))],
                "tote_storage.json record 1 states `io_point` as null",
            ),
            (vec![("conveyor.json", json!({ "version": "2" }))], "conveyor.json states no `conveyor_data` list"),
            (
                vec![("pptl.json", json!([{ "pptls": [{ "peripheral_id": 1, "pps_id": "1" }] }]))],
                "pptl.json group 1, device 1 states `peripheral_id` as a number (1)",
            ),
            (
                vec![(
                    "pps.json",
                    json!([{ "pps_id": 1, "entities": { "presentation_points": [{ "point_id": "P1", "attributes": 4 }] } }]),
                )],
                "pps.json record 1, presentation point 1 states `attributes` as a number (4)",
            ),
            (
                vec![(
                    "conveyor_mapping.json",
                    json!([{ "conveyor_coordinates": { "conveyor_id": "3", "step_id": 4 } }]),
                )],
                "conveyor_mapping.json record 1's `conveyor_coordinates` states `step_id` as a number (4)",
            ),
            (
                vec![("conveyor_mapping.json", json!([{ "conveyor_coordinates": { "step_id": "S" } }]))],
                "conveyor_mapping.json record 1's `conveyor_coordinates` states no `conveyor_id`",
            ),
            (
                vec![("map.json", json!([{ "floor_id": 1, "map_values": [] }, { "floor_id": 2, "map_values": [] }]))],
                "map.json states 2 floor objects",
            ),
            (
                vec![("map.json", json!([{ "floor_id": "2", "map_values": [] }]))],
                "map.json states `floor_id` as a string (\"2\"), and the format fixes a whole number",
            ),
            (
                vec![("map.json", json!([{ "floor_id": 2.0, "map_values": [] }]))],
                "map.json states `floor_id` as a number (2.0)",
            ),
            (vec![("map.json", json!([{ "map_values": [] }]))], "map.json states no `floor_id`"),
            (
                vec![("block.json", json!([{ "entity_details": { "block_id": "7", "coordinates": [] } }]))],
                "block.json record 1 states no `entity_type`",
            ),
            (
                vec![(
                    "block.json",
                    json!([{ "entity_type": 3, "entity_details": { "block_id": "7", "coordinates": [] } }]),
                )],
                "block.json record 1 states `entity_type` as a number (3), and block.json fixes it as a string",
            ),
            (vec![("pps.json", json!([{ "pps_id": 1, "type": null }]))], "pps.json record 1 states `type` as null"),
            (
                vec![(
                    "pps.json",
                    json!([{ "pps_id": 1, "entities": { "presentation_points": [{ "point_id": "P1", "presented_for": 3 }] } }]),
                )],
                "pps.json record 1, presentation point 1 states `presented_for` as a number (3), where the format fixes \
                 a string",
            ),
            (
                vec![(
                    "map.json",
                    json!([{ "floor_id": 1, "map_values": [{ "coordinate": "[1,1]", "barcode": "B1", "world_coordinate": "[0,0]" }] }]),
                )],
                "map.json cell [1,1] states no `blocked`",
            ),
        ] {
            let refusal = check(&dump(&files)).expect_err("a refusal").to_string();
            assert!(refusal.starts_with(says), "{says:?}: {refusal}");
        }
        assert!(check(&dump(&[("charger.json", json!([])), ("conveyor.json", json!({}))])).is_ok());
    }
}
