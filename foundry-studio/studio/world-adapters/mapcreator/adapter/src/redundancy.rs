//! Within the Map Creator adapter, `redundancy` is **the facts an export
//! states more than once, checked to agree**: a sector's key as each of its
//! cells spells it, `sectors.json` and `sector.json`, each one the export
//! carries, against the cells' own `sector`, a cell's `block` against
//! `block.json`, a cell's `dockPoint` against `dock_point.json`,
//! `vda5050_reference` against the cell it names, and a presentation point's
//! `world_coordinates` against the cell its barcode names.
//!
//! The import keeps each of these in one home and the export rebuilds the
//! other statements from it, which is what keeps the site from holding two
//! answers. That is only exact while the export's statements agree: one that
//! disagrees with itself would come back agreeing, corrected, with nobody
//! told. So a disagreement is refused, naming it.

use crate::dump::{Dump, Error};
use crate::identity::scalar;
use crate::kinds::FileKind;
use crate::layout;
use crate::format::source::{raw_pair, Site};
use crate::records::Spelling;
use crate::source_index::{key_of, spell_coordinate, CellNamed, SourceIndex};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{json, Map, Value};
use worldcanonical::to_canonical_string;

/// Every one of them, or a sentence naming the first disagreement.
pub fn check(site: &Site<'_>, dump: &Dump, index: &SourceIndex) -> Result<(), Error> {
    sector_spellings(site)?;
    sectors(site, dump)?;
    blocks(site, dump, index)?;
    dock_points(site, dump, index)?;
    vendor_reference(site, index)?;
    presentation_points(dump, index)
}

/// One sector's key as each of its cells spells it: the sector records one
/// spelling, the first cell's, and the export writes every cell of it back in
/// that one, so a cell spelling the same key otherwise (`"1"` beside `1`) would
/// come back respelled.
fn sector_spellings(site: &Site<'_>) -> Result<(), Error> {
    let mut first: FxHashMap<String, (&str, &Value)> = FxHashMap::default();
    for cell in &site.cells {
        let Some(sector) = cell.raw.get("sector") else { continue };
        match first.get(&scalar(sector)) {
            Some((coordinate, spelled)) if to_canonical_string(spelled) != to_canonical_string(sector) => {
                return Err(Error::Import(format!(
                    "map.json cell {} states `sector` {sector}, and cell {coordinate} states the same sector as \
                     {spelled}: a sector has one spelling, and the export writes every cell of it back in the first \
                     cell's, so this one would come back changed",
                    cell.coordinate
                )));
            }
            Some(_) => {}
            None => {
                first.insert(scalar(sector), (cell.coordinate.as_str(), sector));
            }
        }
    }
    Ok(())
}

/// The sector index against the cells' own `sector` fields, under each of its
/// two names the export carries: the export writes every spelling it was
/// given back as each key the cells state, with the cells that state it in
/// `map.json` order, so each is checked on its own.
fn sectors(site: &Site<'_>, dump: &Dump) -> Result<(), Error> {
    let mut rebuilt: Map<String, Value> = Map::new();
    for cell in &site.cells {
        if let Some(sector) = cell.raw.get("sector") {
            if let Value::Array(members) = rebuilt.entry(scalar(sector)).or_insert_with(|| json!([])) {
                members.push(json!(cell.coordinate));
            }
        }
    }
    for kind in [FileKind::Sectors, FileKind::Sector] {
        if let Some(stated) = dump.get(kind) {
            sector_index(kind, stated, &rebuilt)?;
        }
    }
    Ok(())
}

/// One spelling of the sector index against the index the cells give.
fn sector_index(kind: FileKind, stated: &Value, rebuilt: &Map<String, Value>) -> Result<(), Error> {
    if stated.as_array().is_some_and(Vec::is_empty) || stated.as_object().is_some_and(Map::is_empty) {
        return Ok(());
    }
    let table = match stated.as_array().map(Vec::as_slice) {
        Some([Value::Object(table)]) => table,
        _ => {
            return Err(Error::Import(format!(
                "{kind} is not a list holding one object of sectors, and the export rebuilds it as one from the \
                 cells' own `sector` fields, so it would come back changed"
            )));
        }
    };
    let disagreement = rebuilt
        .iter()
        .find_map(|(key, members)| match table.get(key) {
            None => Some(match list_len(members) {
                1 => format!("does not list sector `{key}`, which 1 cell states"),
                many => format!("does not list sector `{key}`, which {many} cells state"),
            }),
            Some(listed) if listed != members => {
                Some(format!("lists sector `{key}` other than its cells do: {}", first_difference(listed, members)))
            }
            Some(_) => None,
        })
        .or_else(|| {
            table
                .keys()
                .find(|key| !rebuilt.contains_key(*key))
                .map(|key| format!("lists sector `{key}`, which no cell states"))
        });
    match disagreement {
        Some(disagreement) => Err(Error::Import(format!(
            "{kind} {disagreement}: the export rebuilds {kind} from the cells' own `sector` fields, so it would come \
             back agreeing with them"
        ))),
        None => Ok(()),
    }
}

/// A cell's `block` against `block.json`: the export writes each cell the block
/// whose resolved cells hold it, the last such block in the file, as a number
/// where its id is one.
fn blocks(site: &Site<'_>, dump: &Dump, index: &SourceIndex) -> Result<(), Error> {
    let mut holder: FxHashMap<String, String> = FxHashMap::default();
    for block in dump.get(FileKind::Block).and_then(Value::as_array).into_iter().flatten() {
        let Some(details) = block.get("entity_details") else { continue };
        let block_id = details.get("block_id").map(scalar).unwrap_or_default();
        for point in details.get("coordinates").and_then(Value::as_array).into_iter().flatten() {
            if let Some(coordinate) = raw_pair(point).map(key_of).filter(|key| index.cell_by_coordinate(key).is_some())
            {
                holder.insert(coordinate, block_id.clone());
            }
        }
    }
    for cell in &site.cells {
        let rebuilt = holder
            .get(&cell.coordinate)
            .map(|block_id| block_id.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(block_id)));
        let stated = cell.raw.get("block");
        if !same(stated, rebuilt.as_ref()) {
            return Err(Error::Import(format!(
                "map.json cell {} states {}, and block.json {}: the export rebuilds a cell's `block` from block.json, \
                 so it would come back {}",
                cell.coordinate,
                stated.map_or("no `block`".to_string(), |block| format!("`block` {block}")),
                rebuilt.as_ref().map_or("puts it in no block".to_string(), |block| format!("puts it in block {block}")),
                rebuilt.as_ref().map_or("without one".to_string(), |block| format!("as {block}"))
            )));
        }
    }
    Ok(())
}

/// A cell's `dockPoint` against `dock_point.json`: the export writes `true` on
/// each cell a dock point stands on, and nothing on any other.
fn dock_points(site: &Site<'_>, dump: &Dump, index: &SourceIndex) -> Result<(), Error> {
    let mut docked: FxHashSet<String> = FxHashSet::default();
    if let Some(file) = dump.get(FileKind::DockPoint) {
        for (_, row) in layout::flat_rows(FileKind::DockPoint, file) {
            if let Some(coordinate) = row.get("dock_coordinate").and_then(Value::as_str) {
                if index.cell_by_coordinate(coordinate).is_some() {
                    docked.insert(coordinate.to_string());
                }
            }
        }
    }
    for cell in &site.cells {
        let rebuilt = docked.contains(&cell.coordinate).then(|| json!(true));
        let stated = cell.raw.get("dockPoint");
        if !same(stated, rebuilt.as_ref()) {
            return Err(Error::Import(format!(
                "map.json cell {} states {}, and dock_point.json {}: the export rebuilds a cell's `dockPoint` from \
                 dock_point.json, so it would come back {}",
                cell.coordinate,
                stated.map_or("no `dockPoint`".to_string(), |flag| format!("`dockPoint` {flag}")),
                if rebuilt.is_some() { "stands a dock point on it" } else { "stands no dock point on it" },
                if rebuilt.is_some() { "with `dockPoint` true" } else { "without one" }
            )));
        }
    }
    Ok(())
}

/// `vda5050_reference` against the cell it names: the export writes the
/// reference back as that cell's barcode and its vendor coordinate, so it has
/// to name exactly one cell and say what that cell says.
fn vendor_reference(site: &Site<'_>, index: &SourceIndex) -> Result<(), Error> {
    let Some(reference) = &site.vda5050_reference else { return Ok(()) };
    let barcode = reference.get("barcode");
    let cell = match barcode.map(|barcode| index.cell(barcode, Spelling::Barcode)) {
        Some(CellNamed::One(cell)) => cell,
        Some(CellNamed::Shared(barcode)) => {
            let cells: Vec<&str> =
                site.cells.iter().filter(|cell| cell.barcode == barcode).map(|cell| cell.coordinate.as_str()).collect();
            return Err(Error::Import(format!(
                "map.json's `vda5050_reference` names the barcode {barcode}, which {} cells carry, {}: which of them \
                 it means would be a guess, and the import refuses rather than make it",
                cells.len(),
                cells.join(" and ")
            )));
        }
        Some(CellNamed::Nothing) | None => {
            return Err(Error::Import(format!(
                "map.json's `vda5050_reference` names {}, which no cell carries: the export rebuilds the reference \
                 from the cell it names, so it would not come back",
                barcode.map_or("no barcode".to_string(), |barcode| format!("the barcode {barcode}"))
            )));
        }
    };
    let named = site.cells.iter().find(|candidate| index.cell_by_coordinate(&candidate.coordinate) == Some(cell));
    let mut rebuilt = Map::new();
    rebuilt.insert("barcode".into(), barcode.cloned().unwrap_or(Value::Null));
    if let Some(vendor) = named.and_then(|cell| cell.raw.get("vda_world_coordinate")) {
        rebuilt.insert("vda5050_coordinate".into(), vendor.clone());
    }
    let rebuilt = Value::Object(rebuilt);
    if !same(Some(reference), Some(&rebuilt)) {
        return Err(Error::Import(format!(
            "map.json's `vda5050_reference` states {reference}, and the cell it names gives {rebuilt}: the export \
             rebuilds the reference from that cell's barcode and vendor coordinate, so it would come back as the \
             cell says"
        )));
    }
    Ok(())
}

/// A presentation point's `attributes.world_coordinates` against the cell its
/// `attributes.barcode` names: the export writes the coordinates back as that
/// cell's grid pair, so where the barcode names one cell they have to be that
/// pair. Where it names no cell, or the point states no barcode, the import
/// keeps the coordinates as written; a barcode two cells carry the import
/// refuses with the point itself.
fn presentation_points(dump: &Dump, index: &SourceIndex) -> Result<(), Error> {
    for (at, station) in dump.get(FileKind::Pps).and_then(Value::as_array).into_iter().flatten().enumerate() {
        let points = station.get("entities").and_then(|entities| entities.get("presentation_points"));
        for (place, point) in points.and_then(Value::as_array).into_iter().flatten().enumerate() {
            let Some(attributes) = point.get("attributes") else { continue };
            let (Some(stated), Some(barcode)) = (attributes.get("world_coordinates"), attributes.get("barcode")) else {
                continue;
            };
            let CellNamed::One(cell) = index.cell(barcode, Spelling::Barcode) else { continue };
            let Some(coordinate) = index.coordinate(cell) else { continue };
            let rebuilt = spell_coordinate(coordinate, Spelling::RawPair);
            if !same(Some(stated), rebuilt.as_ref()) {
                return Err(Error::Import(format!(
                    "pps.json record {}, presentation point {} names the barcode {barcode}, whose cell is {}, and \
                     states `world_coordinates` {stated}: the export rebuilds `world_coordinates` from the cell the \
                     barcode names, so it would come back as {}",
                    at + 1,
                    place + 1,
                    coordinate,
                    rebuilt.map_or("nothing".to_string(), |pair| pair.to_string())
                )));
            }
        }
    }
    Ok(())
}

/// Two optional values that are one value as the laws compare them: a whole
/// number spelled with a fraction is the number.
fn same(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => to_canonical_string(left) == to_canonical_string(right),
        (None, None) => true,
        _ => false,
    }
}

fn list_len(value: &Value) -> usize {
    value.as_array().map_or(0, Vec::len)
}

/// Where a list the file states first parts from the one the cells give.
fn first_difference(stated: &Value, rebuilt: &Value) -> String {
    let (stated, rebuilt) = (stated.as_array(), rebuilt.as_array());
    let (Some(stated), Some(rebuilt)) = (stated, rebuilt) else {
        return "it is not a list of cells".to_string();
    };
    match stated.iter().zip(rebuilt).position(|(one, other)| one != other) {
        Some(at) => format!("entry {} is {}, where the cells give {}", at + 1, stated[at], rebuilt[at]),
        None => format!("it lists {} cells, where the cells give {}", stated.len(), rebuilt.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::check;
    use crate::dump::Dump;
    use crate::format::source::Site;
    use crate::source_index::SourceIndex;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn refusal(edit: impl Fn(&mut BTreeMap<String, Value>)) -> Option<String> {
        let mut files: BTreeMap<String, Value> = BTreeMap::new();
        let cell = |coordinate: &str, barcode: &str, extra: Value| {
            let mut cell = json!({ "coordinate": coordinate, "barcode": barcode, "world_coordinate": "[0,0]" });
            for (key, value) in extra.as_object().expect("an object") {
                cell[key] = value.clone();
            }
            cell
        };
        files.insert(
            "map.json".into(),
            json!([{ "floor_id": 1,
                "vda5050_reference": { "barcode": "B1", "vda5050_coordinate": "[5,5]" },
                "map_values": [
                    cell("[1,1]", "B1", json!({ "sector": 1, "block": 7, "vda_world_coordinate": "[5,5]" })),
                    cell("[2,1]", "B2", json!({ "sector": "undefined", "dockPoint": true })),
                ] }]),
        );
        files.insert("sectors.json".into(), json!([{ "1": ["[1,1]"], "undefined": ["[2,1]"] }]));
        files.insert("block.json".into(), json!([{ "entity_details": { "block_id": "7", "coordinates": [[1, 1]] } }]));
        files.insert("dock_point.json".into(), json!([{ "id": "D1", "dock_coordinate": "[2,1]" }]));
        edit(&mut files);
        let dump = Dump::from_files(files).expect("a dump");
        let site = Site::from_files(dump.to_files()).expect("a site");
        let mut index = SourceIndex::default();
        for (at, cell) in site.cells.iter().enumerate() {
            let id = Uuid::parse_str(&format!("c0000000-0000-4000-8000-{:012}", at + 1)).expect("a UUID");
            index.add_cell(&cell.coordinate, &cell.barcode, id, cell.world);
        }
        check(&site, &dump, &index).err().map(|refusal| refusal.to_string())
    }

    /// One v3 station with one presentation point on the cell barcode `B1`
    /// names, stating `world_coordinates` as given.
    fn station_with_a_point(world_coordinates: Value) -> Value {
        json!([{ "pps_id": 1, "entities": { "presentation_points": [
            { "point_id": "P1", "attributes": { "barcode": "B1", "world_coordinates": world_coordinates } }
        ] } }])
    }

    /// An export whose statements agree passes, and each that disagrees is
    /// refused naming the disagreement: under both names of the sector index,
    /// each checked on its own, and a presentation point's coordinates against
    /// the cell its barcode names, unless the barcode names none.
    #[test]
    fn each_redundant_part_that_disagrees_is_refused_naming_it() {
        assert_eq!(refusal(|_| {}), None);
        assert_eq!(refusal(|files| drop(files.insert("pps.json".into(), station_with_a_point(json!([1, 1]))))), None);
        let elsewhere = |files: &mut BTreeMap<String, Value>| {
            let mut stations = station_with_a_point(json!([9, 9]));
            stations[0]["entities"]["presentation_points"][0]["attributes"]["barcode"] = json!("B9");
            files.insert("pps.json".into(), stations);
        };
        assert_eq!(refusal(elsewhere), None);
        let both = |files: &mut BTreeMap<String, Value>| {
            files.insert("sector.json".into(), json!([{ "1": ["[1,1]"], "undefined": ["[2,1]"] }]));
        };
        assert_eq!(refusal(both), None);
        for (edit, says) in [
            (
                Box::new(|files: &mut BTreeMap<String, Value>| {
                    files.insert("sectors.json".into(), json!([{ "1": ["[1,1]"] }]));
                }) as Box<dyn Fn(&mut BTreeMap<String, Value>)>,
                "sectors.json does not list sector `undefined`, which 1 cell states",
            ),
            (
                Box::new(|files| {
                    files.insert("sectors.json".into(), json!([{ "1": ["[1,1]", "[2,1]"], "undefined": ["[2,1]"] }]));
                }),
                "sectors.json lists sector `1` other than its cells do: it lists 2 cells, where the cells give 1",
            ),
            (
                Box::new(|files| {
                    files.insert("sectors.json".into(), json!([{ "1": ["[1,1]"], "undefined": ["[2,1]"], "4": [] }]));
                }),
                "sectors.json lists sector `4`, which no cell states",
            ),
            (
                Box::new(|files| {
                    files.insert("sector.json".into(), json!([{ "1": ["[1,1]"] }]));
                }),
                "sector.json does not list sector `undefined`, which 1 cell states",
            ),
            (
                Box::new(|files| {
                    files.insert("block.json".into(), json!([]));
                }),
                "map.json cell [1,1] states `block` 7, and block.json puts it in no block",
            ),
            (
                Box::new(|files| {
                    files.insert("dock_point.json".into(), json!([]));
                }),
                "map.json cell [2,1] states `dockPoint` true, and dock_point.json stands no dock point on it",
            ),
            (
                Box::new(|files| {
                    files.get_mut("map.json").expect("a map")[0]["vda5050_reference"]["barcode"] = json!("B9");
                }),
                "map.json's `vda5050_reference` names the barcode \"B9\", which no cell carries",
            ),
            (
                Box::new(|files| {
                    files.get_mut("map.json").expect("a map")[0]["vda5050_reference"]["vda5050_coordinate"] =
                        json!("[6,6]");
                }),
                "map.json's `vda5050_reference` states",
            ),
            (
                Box::new(|files| {
                    files.get_mut("map.json").expect("a map")[0]["map_values"][1]["sector"] = json!("1");
                    files.insert("sectors.json".into(), json!([{ "1": ["[1,1]", "[2,1]"] }]));
                }),
                "map.json cell [2,1] states `sector` \"1\", and cell [1,1] states the same sector as 1",
            ),
            (
                Box::new(|files| {
                    files.insert("pps.json".into(), station_with_a_point(json!([2, 1])));
                }),
                "pps.json record 1, presentation point 1 names the barcode \"B1\", whose cell is [1,1], and states \
                 `world_coordinates` [2,1]: the export rebuilds `world_coordinates` from the cell the barcode names, \
                 so it would come back as [1,1]",
            ),
            (
                Box::new(|files| {
                    files.insert("pps.json".into(), station_with_a_point(json!("[1,1]")));
                }),
                "pps.json record 1, presentation point 1 names the barcode \"B1\", whose cell is [1,1], and states \
                 `world_coordinates` \"[1,1]\"",
            ),
        ] {
            let refused = refusal(edit).unwrap_or_else(|| panic!("{says}: no refusal"));
            assert!(refused.starts_with(says), "{says:?}: {refused}");
        }
    }
}
