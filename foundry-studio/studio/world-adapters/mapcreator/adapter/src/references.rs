//! Within the Map Creator adapter, `references` is **every reference the
//! projection reads, resolved before a file is written**: a record's references
//! to cells and to records of other kinds, the blocks that serve a station, a
//! block's, a sector's and an aisle's cells, the station a point sits in, a
//! preference's sectors and the floor's vendor sample point, each checked
//! against the index the serializers write it back through.
//!
//! It exists because a serializer that cannot spell a reference has nothing
//! honest to write: it would leave the field out, drop the entry from its list
//! or the point from its station's record, or write an identity where the file
//! keeps a key. The import writes only references that resolve, so a site holds
//! one that does not only after another tool edits it, which on a server the
//! reference gate the vocabulary's annotations arm mostly refuses first. This is
//! the second line, and the one in any host: such a site is refused, naming the
//! referring record, the field and what it names; so is a reference the file
//! could only spell ambiguously, a cell named by a barcode another cell carries
//! too, or a seat or bin of another station than the naming record's; and so is
//! a cell, named or not, whose grid coordinate the export cannot spell.

use crate::dump::Error;
use crate::export::LENSES;
use crate::identity::{constructs_of, scalar};
use crate::kinds::FileKind;
use crate::format::source::bracket_pair;
use crate::records::{self, Field, Join, Slot, Spelling};
use crate::serialize::{block_cells_field, formatting, Projection};
use crate::written;
use rustc_hash::FxHashMap;
use serde_json::{Map, Value};
use uuid::Uuid;
use worldlens::Object;

/// Every reference the projection reads names a record it can write back, or
/// a sentence naming the first that does not.
pub fn check(projection: &Projection) -> Result<(), Error> {
    let targets = Targets::of(projection);
    targets.vendor_samples()?;
    for kind in records::KINDS {
        for object in projection.lens(kind.lens) {
            let Some(component) = payload(object, kind.component) else { continue };
            let record = Record { object, name: kind.component, file: kind.file, component };
            // A station's fields are read by the table of the dialect it is
            // written in, as the serializer reads them.
            let map = match kind.construct {
                "station" if is_v3(&formatting(object, kind.lens)) => records::STATION_V3,
                "station" => records::STATION_V2,
                _ => kind.fields,
            };
            targets.fields(&record, map, component, &[])?;
            targets.beside_the_field_map(&record, kind.construct)?;
        }
    }
    // A cell's `sector` names one sector and the sector index lists it once,
    // so the sectors' members are a partition of the cells they hold: a cell
    // held twice would be listed twice while it names one.
    let mut holder: FxHashMap<&str, &Object> = FxHashMap::default();
    for object in projection.lens("mapcreator_sector") {
        let Some(component) = payload(object, "sector") else { continue };
        let record = Record { object, name: "sector", file: FileKind::Sectors, component };
        targets.list(&record, "members", "the sector index", |named| targets.cell(named, Spelling::Bracketed))?;
        for member in component.get("members").and_then(Value::as_array).into_iter().flatten() {
            let Some(cell) = member.as_str() else { continue };
            if let Some(first) = holder.insert(cell, object) {
                let held = if first.anchor.id == object.anchor.id {
                    "twice".to_string()
                } else {
                    format!("and so does entity {}{}", first.anchor.id, key_of(first))
                };
                return Err(Error::Document(format!(
                    "entity {}{} names cell {cell} in `sector.members` {held}, and a cell's `sector` in map.json \
                     names one sector, which lists it once: the sector index would list it where the cell does \
                     not say it is",
                    object.anchor.id,
                    key_of(object)
                )));
            }
        }
    }
    for object in projection.lens("mapcreator_sector_preference") {
        let name = "sector_preference";
        let Some(component) = payload(object, name) else { continue };
        let record = Record { object, name, file: FileKind::SectorMxUPreferences, component };
        placements(&record, "prefers", component.get("prefers"), component.get("prefers_unresolved"), None)?;
        targets.list(&record, "prefers", "sectorMxUPreferences.json", |named| targets.sector(named))?;
    }
    emitted_keys(projection)?;
    // A cell nothing names is still written into map.json under its own grid
    // coordinate, which the next ingest reads only in the export's spelling.
    for object in projection.lens("mapcreator_cell") {
        if let Some((coordinate, _)) = projection.cell(&object.anchor.id.to_string()) {
            if bracket_pair(coordinate).is_none() {
                return Err(Error::Document(format!(
                    "entity {}{} carries the grid coordinate {coordinate:?} in \
                     `external_ids.mapcreator_grid_coordinate`, {UNSPELLABLE}, so map.json's `coordinate` could not \
                     be written back as one an ingest reads",
                    object.anchor.id,
                    key_of(object)
                )));
            }
        }
    }
    Ok(())
}

/// Why a grid coordinate cannot be written back: every file spells one, as a
/// string or as a pair, from the two integers `[x,y]` spells.
const UNSPELLABLE: &str = "which is not a grid pair spelled `[x,y]`, two whole numbers in their shortest form";

/// One record whose references are read: the entity, the component they are
/// in, and the file the record is written into.
#[derive(Clone, Copy)]
struct Record<'p> {
    object: &'p Object,
    name: &'static str,
    file: FileKind,
    component: &'p Map<String, Value>,
}

/// Why the projection could not write one reference back.
enum Fault {
    /// It names no record of the kind it is written back as.
    Missing(&'static str),
    /// A list of references is held as something other than a list.
    NotAList,
    /// It names a cell by a barcode several cells carry, at these coordinates.
    Shared(String, Vec<String>),
    /// It names a cell whose grid coordinate, which every file spells a cell
    /// from as a string or as a pair, is not one the export can spell.
    Unspellable(String),
    /// It names a seat or a bin written at another station than the naming
    /// record: the station the target is written at, then the record's own.
    Elsewhere(Join, String, String),
    /// It names a station of the other dialect from the point's: `true` for a
    /// presentation point, which only a station of the current dialect keeps.
    Dialect(bool),
}

/// What a reference can name, read off the projection's own indexes.
struct Targets<'p> {
    projection: &'p Projection,
    /// A barcode -> the grid coordinates of the cells carrying it.
    barcodes: FxHashMap<&'p str, Vec<&'p str>>,
    /// A station's identity -> whether its record is written in the current
    /// dialect, which keeps presentation points, rather than the older one,
    /// which keeps `pps_points`.
    stations: FxHashMap<String, bool>,
    /// A seat's or a bin's identity -> the station it is written at, as the
    /// key its own record's `pps_id` comes back as.
    parts: FxHashMap<String, Option<String>>,
}

impl<'p> Targets<'p> {
    fn of(projection: &'p Projection) -> Targets<'p> {
        let mut barcodes: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
        for (_, coordinate, barcode) in projection.cells() {
            barcodes.entry(barcode).or_default().push(coordinate);
        }
        let stations = projection
            .lens("mapcreator_station")
            .iter()
            .map(|object| (object.anchor.id.to_string(), is_v3(&formatting(object, "mapcreator_station"))))
            .collect();
        let mut parts = FxHashMap::default();
        for (lens, name) in [("mapcreator_pps_seat", "pps_seat"), ("mapcreator_pps_bin", "pps_bin")] {
            for object in projection.lens(lens) {
                let station = payload(object, name).and_then(|component| written_station(projection, component));
                parts.insert(object.anchor.id.to_string(), station);
            }
        }
        Targets { projection, barcodes, stations, parts }
    }

    /// The floor's vendor sample point: the barcode of exactly one cell, whose
    /// own fields `map.json`'s `vda5050_reference` is written back from.
    fn vendor_samples(&self) -> Result<(), Error> {
        for object in self.projection.lens("mapcreator_frame") {
            let Some(sample) =
                payload(object, "mapcreator.frame").and_then(|frame| frame.get("vda5050_reference_barcode"))
            else {
                continue;
            };
            let carrying =
                sample.as_str().and_then(|barcode| self.barcodes.get(barcode)).map_or(&[][..], Vec::as_slice);
            let why = match carrying {
                [_] => continue,
                [] => "which no cell of this site carries".to_string(),
                many => {
                    let many: Vec<String> = many.iter().map(|at| at.to_string()).collect();
                    format!(
                        "which {} cells carry, {}: which of them it means would be a guess",
                        many.len(),
                        listed(&many)
                    )
                }
            };
            return Err(Error::Document(format!(
                "entity {}{} names the barcode {sample} in `mapcreator.frame.vda5050_reference_barcode`, {why}, so \
                 map.json's `vda5050_reference` could not be written back",
                object.anchor.id,
                key_of(object)
            )));
        }
        Ok(())
    }

    /// The references one field map reads off a component, nested objects
    /// included; `outer` holds the fields of the objects being read inside.
    fn fields(
        &self,
        record: &Record,
        map: &'static [Field],
        component: &Map<String, Value>,
        outer: &[&'static Field],
    ) -> Result<(), Error> {
        for field in map {
            if matches!(field.slot, Slot::Cells(_) | Slot::Blocks) {
                let kept = component.get(&format!("{}_unresolved", field.target));
                placements(record, &outer_path(outer, field), component.get(field.target), kept, None)?;
            }
            let Some(value) = component.get(field.target) else { continue };
            let refuse = |entry: Option<usize>, named: &Value, fault: Fault| {
                let path: Vec<&str> = outer.iter().chain([&field]).map(|field| field.target).collect();
                let source: Vec<&str> = outer.iter().chain([&field]).map(|field| field.source).collect();
                let written = format!("{}'s `{}`", record.file, source.join("."));
                self.refusal(record, &path.join("."), entry, named, fault, &written)
            };
            match field.slot {
                Slot::Cell(spelling) => {
                    beside_its_kept_value(record, component, field.target, &outer_path(outer, field))?;
                    if let Some(fault) = self.cell(value, spelling) {
                        return Err(refuse(None, value, fault));
                    }
                }
                Slot::Cells(spelling) => self.entries(value, |named| self.cell(named, spelling), &refuse)?,
                Slot::Join(join, _) => {
                    beside_its_kept_value(record, component, field.target, &outer_path(outer, field))?;
                    if let Some(fault) = self.joined(record.component, value, join) {
                        return Err(refuse(None, value, fault));
                    }
                }
                Slot::Blocks => self.entries(value, |named| self.block(named), &refuse)?,
                Slot::Nested(inner) => {
                    if let Some(nested) = value.as_object() {
                        let deeper: Vec<&'static Field> = outer.iter().copied().chain([field]).collect();
                        self.fields(record, inner, nested, &deeper)?;
                    }
                }
                Slot::Key { .. }
                | Slot::Verbatim(_)
                | Slot::Runtime(_)
                | Slot::Written(_)
                | Slot::Echo { .. }
                | Slot::Elsewhere(_) => {}
            }
        }
        Ok(())
    }

    /// The references a record kind's serializer reads besides its field map:
    /// a station's served blocks, a block's and an aisle's cells, the station a
    /// point is written inside, and the cell an exclusion's tuple is rebuilt
    /// from.
    fn beside_the_field_map(&self, record: &Record, construct: &str) -> Result<(), Error> {
        let component = record.component;
        match construct {
            "station" if is_v3(&formatting(record.object, "mapcreator_station")) => {
                placements(
                    record,
                    "served_by",
                    component.get("served_by"),
                    component.get("served_by_unresolved"),
                    None,
                )?;
                self.list(record, "served_by", "pps.json's `entities.blocks`", |named| self.block(named))
            }
            "block" => {
                let (name, field) = block_cells_field(record.object);
                let cells = payload(record.object, name);
                let kept = component.get("coordinates_unresolved");
                placements(record, "coordinates", cells.and_then(|cells| cells.get(field)), kept, None)?;
                let Some(cells) = cells else { return Ok(()) };
                let holder = Record { name, component: cells, ..*record };
                self.list(&holder, field, "block.json's `entity_details.coordinates`", |named| {
                    self.cell(named, Spelling::RawPair)
                })
            }
            "aisle" => {
                let sector = payload(record.object, "sector");
                let kept = component.get("io_point_coordinates_unresolved");
                let formatting = formatting(record.object, "mapcreator_aisle");
                let pairs = formatting.get("io_point_coordinates_as_pairs");
                placements(
                    record,
                    "io_point_coordinates",
                    sector.and_then(|sector| sector.get("members")),
                    kept,
                    pairs,
                )?;
                let Some(sector) = sector else { return Ok(()) };
                let holder = Record { name: "sector", component: sector, ..*record };
                self.list(&holder, "members", "vtm_zone.json's `io_point_coordinates`", |named| {
                    self.cell(named, Spelling::Bracketed)
                })
            }
            "presentation_point" => {
                // The coordinates are kept only where the barcode named no
                // cell; beside a cell, which would be written is a guess.
                let cell = component.get("attributes").and_then(|attributes| attributes.get("cell"));
                if cell.is_some() && component.contains_key("world_coordinates_unresolved") {
                    return Err(both(
                        record,
                        "attributes.cell",
                        "world_coordinates_unresolved",
                        "the coordinates the import keeps only where the point's barcode named no cell",
                    ));
                }
                self.one(record, "station", "pps.json's `entities.presentation_points`", |named| {
                    self.station(named, true)
                })
            }
            "station_point" => {
                self.one(record, "station", "pps.json's `pps_points`", |named| self.station(named, false))
            }
            "ods_exclusion" => {
                // A tuple kept whole is one the import could not split, so it
                // stands beside neither a cell, a barcode nor a heading.
                if component.contains_key("tuple_unparsed") {
                    let split = ["cell", "cell_barcode_unresolved", "heading"];
                    return match split.into_iter().find(|field| component.contains_key(*field)) {
                        Some(field) => Err(both(
                            record,
                            field,
                            "tuple_unparsed",
                            "the tuple the import keeps whole only where it could not split it",
                        )),
                        None => Ok(()),
                    };
                }
                if component.contains_key("cell") && component.contains_key("cell_barcode_unresolved") {
                    return Err(both(
                        record,
                        "cell",
                        "cell_barcode_unresolved",
                        "the barcode the import keeps only where it named no cell",
                    ));
                }
                self.one(record, "cell", "ods_excluded.json's `ods_tuple`", |named| self.cell(named, Spelling::Barcode))
            }
            _ => Ok(()),
        }
    }

    /// One reference a record's component holds under `field`, where it holds
    /// one.
    fn one(
        &self,
        record: &Record,
        field: &str,
        written: &str,
        fault: impl Fn(&Value) -> Option<Fault>,
    ) -> Result<(), Error> {
        match record.component.get(field) {
            Some(named) => match fault(named) {
                Some(fault) => Err(self.refusal(record, field, None, named, fault, written)),
                None => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// Each entry of a list of references a record's component holds under
    /// `field`, where it holds one.
    fn list(
        &self,
        record: &Record,
        field: &str,
        written: &str,
        fault: impl Fn(&Value) -> Option<Fault>,
    ) -> Result<(), Error> {
        let Some(list) = record.component.get(field) else { return Ok(()) };
        self.entries(list, fault, &|entry, named, fault| self.refusal(record, field, entry, named, fault, written))
    }

    /// Each entry of one list of references, or the refusal of the first that
    /// faults, or of a list that is no list at all.
    fn entries(
        &self,
        list: &Value,
        fault: impl Fn(&Value) -> Option<Fault>,
        refuse: &dyn Fn(Option<usize>, &Value, Fault) -> Error,
    ) -> Result<(), Error> {
        let Some(entries) = list.as_array() else { return Err(refuse(None, list, Fault::NotAList)) };
        for (entry, named) in entries.iter().enumerate() {
            if let Some(fault) = fault(named) {
                return Err(refuse(Some(entry), named, fault));
            }
        }
        Ok(())
    }

    /// A reference to one cell: a cell of the site whose grid coordinate the
    /// export can spell, since a file writes it from that coordinate as a
    /// string or a pair, or a point beside the barcode naming it; and, where
    /// the file names a cell by its barcode, the one cell carrying it.
    fn cell(&self, named: &Value, spelling: Spelling) -> Option<Fault> {
        let Some((coordinate, barcode)) = named.as_str().and_then(|id| self.projection.cell(id)) else {
            return Some(Fault::Missing("cell"));
        };
        if bracket_pair(coordinate).is_none() {
            return Some(Fault::Unspellable(coordinate.to_string()));
        }
        match self.barcodes.get(barcode) {
            Some(carrying) if spelling == Spelling::Barcode && carrying.len() > 1 => {
                Some(Fault::Shared(barcode.to_string(), carrying.iter().map(|at| at.to_string()).collect()))
            }
            _ => None,
        }
    }

    /// A reference to a record of a joined kind, and, for a seat or a bin, one
    /// written at the naming record's own station: a file names those among the
    /// parts of that station, so one of another station has no spelling there.
    fn joined(&self, component: &Map<String, Value>, named: &Value, join: Join) -> Option<Fault> {
        let Some(id) = named.as_str().filter(|id| self.projection.joined(join, id).is_some()) else {
            return Some(Fault::Missing(join.what()));
        };
        if !matches!(join, Join::Seat | Join::Bin) {
            return None;
        }
        match (self.parts.get(id).cloned().flatten(), written_station(self.projection, component)) {
            (Some(theirs), Some(ours)) if theirs != ours => Some(Fault::Elsewhere(join, theirs, ours)),
            _ => None,
        }
    }

    fn block(&self, named: &Value) -> Option<Fault> {
        named.as_str().and_then(|id| self.projection.block_id(id)).is_none().then_some(Fault::Missing("block"))
    }

    fn sector(&self, named: &Value) -> Option<Fault> {
        named.as_str().and_then(|id| self.projection.sector_key(id)).is_none().then_some(Fault::Missing("sector"))
    }

    /// A point's station: one of the site's stations, written in the dialect
    /// that keeps points of the point's kind.
    fn station(&self, named: &Value, presentation: bool) -> Option<Fault> {
        match named.as_str().and_then(|id| self.stations.get(id)) {
            None => Some(Fault::Missing("station")),
            Some(current) if *current != presentation => Some(Fault::Dialect(presentation)),
            Some(_) => None,
        }
    }

    /// The sentence refusing one reference: the referring entity, the field,
    /// what it names, and why the projection could not write it back as
    /// `written`.
    fn refusal(
        &self,
        record: &Record,
        field: &str,
        entry: Option<usize>,
        named: &Value,
        fault: Fault,
        written: &str,
    ) -> Error {
        let what = match named.as_str() {
            Some(id) if Uuid::parse_str(id).is_ok() => format!("entity {id}"),
            _ => named.to_string(),
        };
        let place = match entry {
            Some(entry) => format!("as entry {} of `{}.{field}`", entry + 1, record.name),
            None => format!("in `{}.{field}`", record.name),
        };
        let why = match fault {
            Fault::Missing(kind) => {
                let instead = named.as_str().and_then(|id| self.what_it_is(id)).map(|it| format!(" but {it}"));
                format!("which is no {kind} of this site{}", instead.unwrap_or_default())
            }
            Fault::NotAList => "where the projection reads a list of references".to_string(),
            Fault::Shared(barcode, carrying) => format!(
                "a cell whose barcode, {barcode}, {} cells carry, {}: {} names the cell by its barcode, which would \
                 name each of them",
                carrying.len(),
                listed(&carrying),
                record.file
            ),
            Fault::Unspellable(coordinate) => format!("a cell whose grid coordinate, {coordinate:?}, {UNSPELLABLE}"),
            Fault::Elsewhere(join, theirs, ours) => format!(
                "{} {} written at station {theirs}, while this record is written at station {ours}: {} names it among \
                 the parts of the record's own station",
                if join == Join::Seat { "an" } else { "a" },
                join.what(),
                record.file
            ),
            Fault::Dialect(presentation) => format!(
                "a station written in pps.json's {} dialect, whose record keeps no {}",
                if presentation { "older" } else { "current" },
                if presentation { "presentation points" } else { "`pps_points`" }
            ),
        };
        Error::Document(format!(
            "entity {}{} names {what} {place}, {why}, so {written} could not be written back",
            record.object.anchor.id,
            key_of(record.object)
        ))
    }

    /// What one identity is among the records the projection writes, for a
    /// sentence: "the zone North Hall".
    fn what_it_is(&self, id: &str) -> Option<String> {
        let object = LENSES
            .iter()
            .flat_map(|lens| self.projection.lens(lens))
            .find(|object| object.anchor.id.to_string() == id)?;
        let ids = payload(object, "external_ids").cloned().unwrap_or_default();
        constructs_of(&ids).into_iter().next().map(|(_, key)| format!("the {key}"))
    }
}

/// The key each record is written back under, as the next ingest reads it
/// off the files, is the key it is known by and named by, and no two records
/// are written under one: a station's own `pps_id` agrees with the key every
/// record naming it is written with, no two sectors spell one key, and no two
/// parts are written under one station with one name, which a tool that
/// re-points a part's station can leave. The files would otherwise name a
/// record by a key it is not written under, or state one record twice.
fn emitted_keys(projection: &Projection) -> Result<(), Error> {
    for object in projection.lens("mapcreator_station") {
        let formatting = formatting(object, "mapcreator_station");
        let known = projection.joined(Join::Station, &object.anchor.id.to_string()).unwrap_or_default();
        if let Some(own) = formatting.get("pps_id").filter(|own| scalar(own) != known) {
            return Err(Error::Document(format!(
                "entity {}{} states `import_formatting.mapcreator_station.pps_id` {own}, while every record naming \
                 it is written with the key it is known by, {known}: pps.json would state station {own} and the \
                 records naming it station {known}",
                object.anchor.id,
                key_of(object)
            )));
        }
    }
    let sectors = projection.lens("mapcreator_sector").iter().map(|object| {
        let key = formatting(object, written::SECTOR).get("key").map(scalar);
        (object, key)
    });
    once_each(sectors, "the sector")?;
    let station_of = |component: Option<&Map<String, Value>>| {
        let named = component?.get("station")?.as_str()?;
        projection.joined(Join::Station, named).map(str::to_string)
    };
    let own = |object: &Object, field: &str| payload(object, "external_ids").and_then(|ids| ids.get(field)).map(scalar);
    for (lens, name, field, what) in [
        ("mapcreator_pps_bin", "pps_bin", "mapcreator_bin_id", "the bin"),
        ("mapcreator_pps_seat", "pps_seat", "mapcreator_seat_name", "the operator seat"),
    ] {
        let parts = projection.lens(lens).iter().map(|object| {
            let station = payload(object, name).and_then(|part| written_station(projection, part));
            (object, station.zip(own(object, field)).map(|(station, own)| format!("{own} of station {station}")))
        });
        once_each(parts, what)?;
    }
    for (lens, name, field, what) in [
        ("mapcreator_presentation_point", "presentation_point", "mapcreator_point_id", "the point"),
        ("mapcreator_station_point", "station_point", "mapcreator_position_id", "the station point"),
    ] {
        let points = projection.lens(lens).iter().map(|object| {
            let station = station_of(payload(object, name));
            (object, station.zip(own(object, field)).map(|(station, own)| format!("{own} of station {station}")))
        });
        once_each(points, what)?;
    }
    let rows = projection.lens("mapcreator_conveyor_mapping").iter().map(|object| {
        let joined = payload(object, "conveyor_mapping")
            .and_then(|row| row.get("conveyor_coordinates"))
            .and_then(|nested| nested.get("conveyor"))
            .and_then(Value::as_str)
            .and_then(|named| projection.joined(Join::Conveyor, named));
        let key = joined
            .zip(own(object, "mapcreator_step_id"))
            .map(|(conveyor, step)| format!("{step} of conveyor {conveyor}"));
        (object, key)
    });
    once_each(rows, "the conveyor mapping row")
}

/// Each record written under its own key, or the sentence naming two that
/// would be written under one.
fn once_each<'p>(records: impl Iterator<Item = (&'p Object, Option<String>)>, what: &str) -> Result<(), Error> {
    let mut written: FxHashMap<String, &Object> = FxHashMap::default();
    for (object, key) in records {
        let Some(key) = key else { continue };
        if let Some(first) = written.get(&key) {
            return Err(Error::Document(format!(
                "entities {}{} and {}{} would both be written as {what} {key}: the files would state one record \
                 twice, which no ingest can tell apart",
                first.anchor.id,
                key_of(first),
                object.anchor.id,
                key_of(object)
            )));
        }
        written.insert(key, object);
    }
    Ok(())
}

/// The entries a list kept where they named nothing go back each at its own
/// place among the list's entries, so each is `{at, value}`, no two take one
/// place, and every place lies within the list the resolved and the kept
/// entries make together; so does each place an aisle records an entry as
/// spelled a pair at, which must be a resolved entry's. Otherwise the list
/// would be written with an entry moved, dropped or out of its order.
fn placements(
    record: &Record,
    field: &str,
    resolved: Option<&Value>,
    kept: Option<&Value>,
    pairs: Option<&Value>,
) -> Result<(), Error> {
    let refuse = |why: String| {
        Error::Document(format!(
            "entity {}{} keeps `{}.{field}`'s entries {why}, so {}'s `{field}` could not be written back in the \
             order the file stated it",
            record.object.anchor.id,
            key_of(record.object),
            record.name,
            record.file
        ))
    };
    let resolved = resolved.and_then(Value::as_array).map_or(0, Vec::len);
    let kept = match kept {
        None => &[][..],
        Some(Value::Array(kept)) => kept.as_slice(),
        Some(other) => return Err(refuse(format!("that named nothing as {other}, where a list of them is kept"))),
    };
    let total = resolved + kept.len();
    let mut taken: FxHashMap<u64, ()> = FxHashMap::default();
    for entry in kept {
        let at = match entry.as_object() {
            Some(fields) if fields.len() == 2 && fields.contains_key("value") => {
                fields.get("at").and_then(Value::as_u64)
            }
            _ => None,
        };
        let Some(at) = at else {
            return Err(refuse(format!("that named nothing with the entry {entry}, where each is `{{at, value}}`")));
        };
        if at >= total as u64 {
            return Err(refuse(format!(
                "that named nothing with one at place {at}, beyond the {total} the list holds"
            )));
        }
        if taken.insert(at, ()).is_some() {
            return Err(refuse(format!("that named nothing with two at place {at}")));
        }
    }
    for place in pairs.and_then(Value::as_array).into_iter().flatten() {
        match place.as_u64() {
            Some(at) if at < total as u64 && !taken.contains_key(&at) => {}
            _ => {
                return Err(refuse(format!(
                    "spelled as pairs at place {place}, which is no resolved entry of the {total} the list holds"
                )))
            }
        }
    }
    Ok(())
}

/// A reference beside the value the import keeps where a reference named
/// nothing: the one or the other is written back, never both, so a record
/// stating both refuses rather than having one chosen for it.
fn beside_its_kept_value(
    record: &Record,
    component: &Map<String, Value>,
    target: &str,
    path: &str,
) -> Result<(), Error> {
    if component.contains_key(&format!("{target}_unresolved")) {
        return Err(both(
            record,
            path,
            &format!("{path}_unresolved"),
            "the value the import keeps only where the file's reference named nothing",
        ));
    }
    Ok(())
}

/// A field's path through the nested objects being read, for a sentence.
fn outer_path(outer: &[&'static Field], field: &Field) -> String {
    let path: Vec<&str> = outer.iter().map(|outer| outer.target).chain([field.target]).collect();
    path.join(".")
}

/// The sentence refusing a record that states a resolved reference beside
/// the value the import keeps only where one named nothing.
fn both(record: &Record, resolved: &str, kept: &str, what: &str) -> Error {
    Error::Document(format!(
        "entity {}{} states both `{}.{resolved}` and `{}.{kept}`, {what}, so which of the two {} gets would be a \
         guess",
        record.object.anchor.id,
        key_of(record.object),
        record.name,
        record.name,
        record.file
    ))
}

/// The station a seat, a bin or a light is written at: the key its own
/// record's `pps_id` comes back as, from its station reference or from the
/// value the import kept where that named no station.
fn written_station(projection: &Projection, component: &Map<String, Value>) -> Option<String> {
    match component.get("station") {
        Some(station) => station.as_str().and_then(|id| projection.joined(Join::Station, id)).map(str::to_string),
        None => component.get("station_unresolved").map(scalar),
    }
}

/// A station record's dialect, as the serializer reads it off the record's
/// formatting.
fn is_v3(formatting: &Map<String, Value>) -> bool {
    formatting.get("dialect_shape").and_then(Value::as_str) == Some("v3")
}

/// One component of an object's anchor, borrowed.
fn payload<'o>(object: &'o Object, name: &str) -> Option<&'o Map<String, Value>> {
    object.anchor.components.get(name).and_then(Value::as_object)
}

/// The lookup key an object's record is known by, for a sentence: " (the
/// charger 5)", or nothing for an object that carries none.
fn key_of(object: &Object) -> String {
    let ids = payload(object, "external_ids").cloned().unwrap_or_default();
    constructs_of(&ids).into_iter().next().map(|(_, key)| format!(" (the {key})")).unwrap_or_default()
}

/// Grid coordinates in a sentence, in order: "[1,3] and [4,3]".
fn listed(coordinates: &[String]) -> String {
    let mut sorted = coordinates.to_vec();
    sorted.sort_unstable();
    sorted.join(" and ")
}
