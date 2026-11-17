//! Within the Map Creator adapter, `serialize` is **the site becoming records**:
//! one function per file of a Map Creator export, each turning the objects one
//! lens projected into the JSON that file spells.
//!
//! This is the write direction, and it is explicit code rather than a language
//! feature for a reason worth stating once. A lens is a query, and a query's
//! inverse is not unique — many graphs project to the same objects — so there is
//! no `put` to derive. What holds the two directions together is `records`: the
//! same field map this module reads backwards is the one `import` read forwards,
//! and the two round-trip laws, run over the built module, are the proof rather
//! than the promise.
//!
//! The spellings are the export's own, not tidied: a grid coordinate is `"[6,1]"`
//! in `map.json` and `[6, 1]` in `block.json`, adjacency is a four-slot array with
//! nulls in it, `size_info` is four half-extents, `botid` is the string `"null"` on
//! three sites and JSON `null` on two, and a numbered sector is an integer in
//! `map_values` and a string key in `sectors.json`. Every one of those is a fact
//! about the file, so every one comes back as it went in.
//!
//! So is how the file's text spells its JSON: each record, once built, carries
//! what its entry of `import_formatting` records of the order the file wrote its
//! keys in and of the whole numbers it wrote with a decimal point, and the files
//! are written from that and from what the export entity records (`written`).

use crate::dump::Error;
use crate::kinds::FileKind;
use crate::format::source::MAX_MM;
use crate::format::steps::millimetres;
use crate::records::{Field, Join, KeyText, Slot, Spelling};
use crate::source_index::spell_coordinate;
use crate::written::{self, carried, Arrangement, Facts, Spacing, Usual, CARRIED};
use crate::{identity, records};
use rustc_hash::FxHashMap;
use serde_json::{json, Map, Number, Value};
use std::collections::BTreeMap;
use worldlens::{EntityView, Object};

/// Every object every lens projected, plus the join tables the serializers read
/// them through.
///
/// The joins live here rather than in the lenses because a lens field list comes
/// back in identity order with repeats dropped: one folded list of cells cannot
/// say which cell filled which of a charger's three cell-valued fields. So the
/// export runs each lens once, indexes every object by identity, and the
/// serializers join through the index.
pub struct Projection {
    /// Lens name -> its objects, already sorted by the record index each carries.
    pub objects: FxHashMap<&'static str, Vec<Object>>,
    /// What one step of the primary frame is worth in millimetres, read off the
    /// site frame, which is how a cell's position in steps becomes the
    /// millimetres `map.json` states.
    pub unit_mm: f64,
    /// Cell identity -> its grid coordinate and its barcode, the two legacy keys
    /// every other file names a cell by.
    cell_keys: FxHashMap<String, (String, String)>,
    /// Cell identity -> the sector key whose members hold it.
    cell_sector: FxHashMap<String, Value>,
    /// Cell identity -> the legacy block id whose cells hold it.
    cell_block: FxHashMap<String, String>,
    /// Cell identities a dock point stands on.
    dock_cells: FxHashMap<String, ()>,
    /// Block identity -> its legacy `block_id`.
    block_ids: FxHashMap<String, String>,
    /// For each kind of record another record names: the named record's
    /// identity -> its own key, which a reference to it is written back as.
    joins: FxHashMap<Join, FxHashMap<String, String>>,
    /// Sector identity -> its key in the JSON kind `map_values` spells.
    sector_keys: FxHashMap<String, Value>,
    /// The order each place of the files usually states an object's keys in,
    /// off the export entity: what a record states no order of its own
    /// against is written in.
    usual: Usual,
}

impl Projection {
    /// Build the join tables from whatever lenses were run. A table a serializer
    /// needs and no lens filled is an empty table, and a record naming what an
    /// empty table lacks is one `references` refuses before any serializer runs.
    pub fn index(objects: FxHashMap<&'static str, Vec<Object>>, unit_mm: f64) -> Projection {
        let of = |name: &str| objects.get(name).map(Vec::as_slice).unwrap_or_default();
        let mut cell_keys = FxHashMap::default();
        let mut cell_sector = FxHashMap::default();
        let mut cell_block = FxHashMap::default();
        let mut dock_cells = FxHashMap::default();
        let mut block_ids = FxHashMap::default();
        let mut sector_keys = FxHashMap::default();
        for object in of("mapcreator_cell") {
            let ids = external_ids(object);
            let coordinate = text(&ids, "mapcreator_grid_coordinate");
            let barcode = text(&ids, "barcode");
            cell_keys.insert(object.anchor.id.to_string(), (coordinate, barcode));
        }
        for object in of("mapcreator_sector") {
            let key = formatting(object, written::SECTOR).get("key").cloned().unwrap_or(Value::Null);
            sector_keys.insert(object.anchor.id.to_string(), key.clone());
            for member in component(object, "sector").get("members").and_then(Value::as_array).into_iter().flatten() {
                if let Some(member) = member.as_str() {
                    cell_sector.insert(member.to_string(), key.clone());
                }
            }
        }
        for object in of("mapcreator_block") {
            let block_id = text(&external_ids(object), "mapcreator_block_id");
            block_ids.insert(object.anchor.id.to_string(), block_id.clone());
            // The cells a block covers, which is what a cell's `block` field
            // names, wherever its kind keeps them.
            for cell in block_cells(object).as_ref().and_then(Value::as_array).into_iter().flatten() {
                if let Some(cell) = cell.as_str() {
                    cell_block.insert(cell.to_string(), block_id.clone());
                }
            }
        }
        for object in of("mapcreator_dock_point") {
            if let Some(cell) = component(object, "dock_point").get("at_cell").and_then(Value::as_str) {
                dock_cells.insert(cell.to_string(), ());
            }
        }
        let mut joins: FxHashMap<Join, FxHashMap<String, String>> = FxHashMap::default();
        for (join, lens) in [
            (Join::Station, "mapcreator_station"),
            (Join::Zone, "mapcreator_zone"),
            (Join::Profile, "mapcreator_location_profile"),
            (Join::Seat, "mapcreator_pps_seat"),
            (Join::Bin, "mapcreator_pps_bin"),
            (Join::BinType, "mapcreator_pps_bin_type"),
            (Join::Conveyor, "mapcreator_conveyor"),
            (Join::Block, "mapcreator_block"),
        ] {
            let keys = joins.entry(join).or_default();
            for object in of(lens) {
                keys.insert(object.anchor.id.to_string(), text(&external_ids(object), join.key_field()));
            }
        }
        // A table another tool spelled otherwise is read as none: it states how
        // the files were laid out, and the files come back equal without it.
        let usual = of("mapcreator_export")
            .first()
            .and_then(|object| component(object, "mapcreator.export").remove("key_orders"))
            .and_then(|orders| serde_json::from_value(orders).ok())
            .unwrap_or_default();
        Projection {
            objects,
            unit_mm,
            cell_keys,
            cell_sector,
            cell_block,
            dock_cells,
            block_ids,
            joins,
            sector_keys,
            usual,
        }
    }

    /// One lens's objects, in record order. An empty list is what a lens nobody
    /// ran and a lens nothing matched both come back as, which is the same answer
    /// for the export's purposes: no records.
    pub fn lens(&self, name: &str) -> &[Object] {
        self.objects.get(name).map(Vec::as_slice).unwrap_or_default()
    }

    /// The grid coordinate and the barcode of the cell an identity names, the
    /// two keys a reference to it is written back as: `None` for an identity
    /// that is no cell of this site.
    pub fn cell(&self, id: &str) -> Option<(&str, &str)> {
        self.cell_keys.get(id).map(|(coordinate, barcode)| (coordinate.as_str(), barcode.as_str()))
    }

    /// Every cell of the site, as its identity, its grid coordinate and its
    /// barcode, in no particular order.
    pub fn cells(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.cell_keys.iter().map(|(id, (coordinate, barcode))| (id.as_str(), coordinate.as_str(), barcode.as_str()))
    }

    /// The key a reference to a record of one joined kind is written back as:
    /// `None` for an identity that is no record of that kind.
    pub fn joined(&self, join: Join, id: &str) -> Option<&str> {
        self.joins.get(&join).and_then(|keys| keys.get(id)).map(String::as_str)
    }

    /// The `block_id` a reference to a block is written back as.
    pub fn block_id(&self, id: &str) -> Option<&str> {
        self.block_ids.get(id).map(String::as_str)
    }

    /// The key a reference to a sector is written back as, in the JSON kind the
    /// cells spell it.
    pub fn sector_key(&self, id: &str) -> Option<&Value> {
        self.sector_keys.get(id)
    }

    /// The whole of one file, or `None` when the document says the site never
    /// shipped it.
    pub fn file(&self, kind: FileKind) -> Option<Result<Value, Error>> {
        let form = self.form(kind)?;
        match form.as_str() {
            "empty_array" => return Some(Ok(json!([]))),
            "empty_object" => return Some(Ok(json!({}))),
            _ => {}
        }
        Some(self.records(kind))
    }

    /// Every record of a kind one file owns is in a file the projection writes
    /// records into, or a sentence naming the first that is not: a file the
    /// export entity says the site never shipped, or shipped empty, is written
    /// so whatever records the site holds of its kind, which would leave them
    /// out. Read off the record kinds and the preferences, each owned by its
    /// file, and not off every lens: a sector is the cells' own `sector` too,
    /// so `map.json` writes it whatever the sector files' forms.
    pub fn inventory(&self) -> Result<(), Error> {
        let owned = records::KINDS
            .iter()
            .map(|kind| (kind.lens, kind.file))
            .chain([("mapcreator_sector_preference", FileKind::SectorMxUPreferences)]);
        for (lens, file) in owned {
            let Some(object) = self.lens(lens).first() else { continue };
            let form = match self.form(file).as_deref() {
                None => "the site never shipped".to_string(),
                Some(form @ ("empty_array" | "empty_object")) => {
                    format!("the site shipped empty, as {}", if form == "empty_array" { "`[]`" } else { "`{}`" })
                }
                Some(_) => continue,
            };
            return Err(Error::Document(format!(
                "entity {}{} is a record of {}, which the export entity's inventory says {form}: written as the \
                 inventory states it, the file would leave the record out",
                object.anchor.id,
                named(object),
                file.name()
            )));
        }
        Ok(())
    }

    /// The spacing the export entity records one file's text in.
    pub fn spacing(&self, kind: FileKind) -> Spacing {
        Spacing::from_json(self.export().get("spacing").and_then(|spacings| spacings.get(kind.name())))
    }

    /// What the files' keys and numbers are written back by: the usual orders,
    /// and what the export entity records of everything outside the records.
    pub fn arrangement(&self) -> Arrangement {
        let export = self.export();
        let mut outside = Map::new();
        for name in written::FACTS {
            if let Some(value) = export.get(name) {
                outside.insert(name.to_string(), value.clone());
            }
        }
        // A preference's list is written as its own entry says, wherever its
        // carrier type now puts it.
        let in_a_list = self.form(FileKind::SectorMxUPreferences).as_deref() == Some("array");
        for object in self.lens("mapcreator_sector_preference") {
            let carrier = text(&external_ids(object), "mapcreator_preferred_for");
            let (pointer, _) = written::preference_list(&carrier, in_a_list);
            Facts::moved_to(&formatting(object, written::SECTOR_PREFERENCE), &pointer, &mut outside);
        }
        Arrangement { usual: self.usual.clone(), outside }
    }

    /// One record, built, carrying what its formatting entry records of how
    /// its file wrote it, for the writing to read (`written::CARRIED`).
    fn finish(&self, mut record: Map<String, Value>, formatting: &Map<String, Value>) -> Value {
        if let Some(facts) = carried(formatting) {
            record.insert(CARRIED.to_string(), facts);
        }
        Value::Object(record)
    }

    /// The form the export entity says this file arrived in, or `None` for a file
    /// the site never shipped.
    fn form(&self, kind: FileKind) -> Option<String> {
        let object = self.lens("mapcreator_export").first()?;
        component(object, "mapcreator.export").get("files")?.get(kind.name())?.as_str().map(str::to_string)
    }

    /// The export entity's own component, which carries the file inventory and the
    /// per-file wrappers.
    fn export(&self) -> Map<String, Value> {
        self.lens("mapcreator_export").first().map(|object| component(object, "mapcreator.export")).unwrap_or_default()
    }

    fn records(&self, kind: FileKind) -> Result<Value, Error> {
        match kind {
            FileKind::Map => self.map_file(),
            FileKind::Zone => Ok(self.wrapped_rows("mapcreator_zone", records::ZONE, "zone", "zonerec")),
            FileKind::Sectors | FileKind::Sector => Ok(self.sector_index()),
            FileKind::VtmZone => Ok(json!({ "aisle_info": self.aisle_rows() })),
            FileKind::Block => Ok(self.block_rows()),
            FileKind::Pps => self.station_rows(),
            FileKind::Charger => Ok(self.rows("mapcreator_charger", records::CHARGER, "charger")),
            FileKind::DockPoint => Ok(self.rows("mapcreator_dock_point", records::DOCK_POINT, "dock_point")),
            FileKind::OdsExcluded => Ok(json!({ "ods_excluded_list": self.ods_rows() })),
            FileKind::ToteStorage => Ok(self.rows("mapcreator_tote_storage", records::TOTE_STORAGE, "tote_storage")),
            FileKind::ToteType => Ok(self.rows("mapcreator_rack_type", records::RACK_TYPE, "rack_type")),
            FileKind::Conveyor => Ok(self.conveyor_file()),
            FileKind::ConveyorMapping => {
                Ok(self.rows("mapcreator_conveyor_mapping", records::CONVEYOR_MAPPING, "conveyor_mapping"))
            }
            FileKind::SectorMxUPreferences => Ok(self.preference_table()),
            FileKind::PpsBin => {
                Ok(self.enveloped("pps_bin_envelope", self.rows("mapcreator_pps_bin", records::PPS_BIN, "pps_bin")))
            }
            FileKind::PpsBinType => Ok(self.enveloped(
                "pps_bin_type_envelope",
                self.rows("mapcreator_pps_bin_type", records::PPS_BIN_TYPE, "pps_bin_type"),
            )),
            FileKind::PpsSeat => {
                Ok(json!({ "pps_seats": self.rows("mapcreator_pps_seat", records::PPS_SEAT, "pps_seat") }))
            }
            FileKind::Pptl => Ok(self.pptl_groups()),
            // The five files that are empty on every site seen. `form` has already
            // answered for a file the export entity says arrived empty, and an
            // ingest refuses one of these that holds anything at all (`layout`), so
            // an export entity naming one in another form was written some other
            // way. `[]` is what such a file is written as.
            FileKind::Elevator
            | FileKind::FireEmergency
            | FileKind::QueueData
            | FileKind::OpcConnection
            | FileKind::ZoneController => Ok(json!([])),
        }
    }

    // -----------------------------------------------------------------------
    // The field map, read backwards
    // -----------------------------------------------------------------------

    /// One record's own fields, out of the component the import put them in and
    /// the record's formatting entry, which holds what only says how its file
    /// wrote it.
    ///
    /// A field the component does not state is a field the record did not have, so
    /// it is left out rather than written as null: the presence of a key is a fact
    /// about the site (one site-a cell in four states `grid_attribute` and the
    /// rest do not), and this is where that fact comes back.
    pub fn decode(
        &self,
        map: &[Field],
        component: &Map<String, Value>,
        formatting: &Map<String, Value>,
        ids: &Map<String, Value>,
    ) -> Map<String, Value> {
        let mut out = Map::new();
        let runtime = component.get("runtime").and_then(Value::as_object).cloned().unwrap_or_default();
        let missing = |source: &str| absent(formatting, source);
        for field in map {
            match field.slot {
                Slot::Key { external, text } => {
                    if let Some(value) = ids.get(external) {
                        let value = identity::scalar(value);
                        out.insert(field.source.to_string(), spelled_key(&value, text));
                    }
                }
                Slot::Verbatim(_) => {
                    if let Some(value) = component.get(field.target) {
                        out.insert(field.source.to_string(), value.clone());
                    }
                }
                Slot::Runtime(_) => {
                    if let Some(value) = runtime.get(field.target) {
                        out.insert(field.source.to_string(), value.clone());
                    }
                }
                Slot::Written(_) => {
                    if let Some(value) = formatting.get(field.target) {
                        out.insert(field.source.to_string(), value.clone());
                    }
                }
                Slot::Elsewhere(_) => {}
                Slot::Cell(spelling) => {
                    if let Some(value) = self.spell_cell(component, field.target, spelling) {
                        out.insert(field.source.to_string(), value);
                    }
                }
                // A list's references in their own order, and each entry that
                // named nothing back at its place among them.
                // A list holding only entries that named nothing is that list.
                Slot::Cells(spelling) => {
                    let list = component.get(field.target);
                    let unresolved = component.get(&format!("{}_unresolved", field.target));
                    if list.is_some() || unresolved.is_some() {
                        let spelled = in_place(list, unresolved, |cell, _| self.spell_one_cell(cell, spelling));
                        out.insert(field.source.to_string(), Value::Array(spelled));
                    }
                }
                // The reference is written back as the named record's own key,
                // in the JSON kind this field's file fixes; a value that named
                // nothing comes back as the file wrote it. `references` has
                // refused a reference naming no record of the kind.
                Slot::Join(join, text) => {
                    if let Some(named) = component.get(field.target).and_then(Value::as_str) {
                        if let Some(key) = self.joins.get(&join).and_then(|keys| keys.get(named)) {
                            out.insert(field.source.to_string(), spelled_key(key, text));
                        }
                    } else if let Some(value) = component.get(&format!("{}_unresolved", field.target)) {
                        out.insert(field.source.to_string(), value.clone());
                    }
                }
                Slot::Blocks => {
                    let list = component.get(field.target);
                    let unresolved = component.get(&format!("{}_unresolved", field.target));
                    if list.is_some() || unresolved.is_some() {
                        let spelled = in_place(list, unresolved, |block, _| {
                            self.block_ids.get(block.as_str()?).map(|block_id| json!(block_id))
                        });
                        out.insert(field.source.to_string(), Value::Array(spelled));
                    }
                }
                // The field it repeats is written first — the field map's own
                // order says so — so the value is there to be copied unless this
                // row is one where the two differed.
                Slot::Echo { of } => {
                    if missing(field.source) {
                        continue;
                    }
                    if let Some(value) = component.get(field.target).cloned().or_else(|| out.get(of).cloned()) {
                        out.insert(field.source.to_string(), value);
                    }
                }
                Slot::Nested(inner) => {
                    if let Some(nested) = component.get(field.target).and_then(Value::as_object) {
                        let written = formatting.get(field.target).and_then(Value::as_object).cloned();
                        let decoded = self.decode(inner, nested, &written.unwrap_or_default(), ids);
                        out.insert(field.source.to_string(), Value::Object(decoded));
                    }
                }
            }
        }
        out
    }

    /// The legacy spelling of the cell one component field references, or of the
    /// verbatim value the import kept when the reference named nothing. A
    /// reference naming no cell of the site is one `references` has refused.
    fn spell_cell(&self, component: &Map<String, Value>, target: &str, spelling: Spelling) -> Option<Value> {
        if let Some(value) = component.get(target) {
            return self.spell_one_cell(value, spelling);
        }
        component.get(&format!("{target}_unresolved")).cloned()
    }

    fn spell_one_cell(&self, value: &Value, spelling: Spelling) -> Option<Value> {
        let id = value.as_str()?;
        let (coordinate, barcode) = self.cell_keys.get(id)?;
        match spelling {
            Spelling::Barcode => Some(Value::String(barcode.clone())),
            other => spell_coordinate(coordinate, other),
        }
    }

    // -----------------------------------------------------------------------
    // The files
    // -----------------------------------------------------------------------

    /// A flat array of records, one per object of a lens.
    fn rows(&self, lens: &str, map: &[Field], component_name: &str) -> Value {
        Value::Array(
            self.lens(lens)
                .iter()
                .map(|object| {
                    let component = component(object, component_name);
                    let formatting = formatting(object, lens);
                    self.finish(self.decode(map, &component, &formatting, &external_ids(object)), &formatting)
                })
                .collect(),
        )
    }

    /// A flat array of records each wrapped in a one-key envelope, which is how
    /// `zone.json` states a zone: `{"zonerec": {...}}`.
    fn wrapped_rows(&self, lens: &str, map: &[Field], component_name: &str, wrapper: &str) -> Value {
        Value::Array(
            self.lens(lens)
                .iter()
                .map(|object| {
                    let component = component(object, component_name);
                    let formatting = formatting(object, lens);
                    let mut envelope = Map::new();
                    envelope.insert(
                        wrapper.to_string(),
                        Value::Object(self.decode(map, &component, &formatting, &external_ids(object))),
                    );
                    self.finish(envelope, &formatting)
                })
                .collect(),
        )
    }

    /// `map.json`: one object with the floor, the cells, the permission bundles and
    /// the vendor sample point.
    fn map_file(&self) -> Result<Value, Error> {
        let frames = self.lens("mapcreator_frame");
        let floor = frames
            .iter()
            .find(|object| component(object, "mapcreator.frame").get("role").and_then(Value::as_str) == Some("floor"))
            .ok_or_else(|| Error::Document("no floor frame, so the site states no floor to write".to_string()))?;
        let floor_component = component(floor, "mapcreator.frame");
        let mut out = Map::new();
        if let Some(floor_id) = floor_component.get("floor_id") {
            out.insert("floor_id".into(), floor_id.clone());
        }
        out.insert("map_values".into(), Value::Array(self.cell_rows(&floor.anchor.id.to_string())?));
        // The profile list is written where it has profiles, and where the file
        // stated it empty, which the export entity records.
        let profiles: Vec<Value> = self
            .lens("mapcreator_location_profile")
            .iter()
            .map(|object| {
                let component = component(object, "location_profile");
                let formatting = formatting(object, "mapcreator_location_profile");
                let record = self.decode(records::LOCATION_PROFILE, &component, &formatting, &external_ids(object));
                self.finish(record, &formatting)
            })
            .collect();
        let stated_empty = self.export().get("location_profiles_empty") == Some(&json!(true));
        if !profiles.is_empty() || stated_empty {
            out.insert("location_profiles".into(), Value::Array(profiles));
        }
        // `vda5050_reference` is the named cell's own two fields on every site, so
        // both halves come off the cell rather than out of a stored copy.
        if let Some(barcode) = floor_component.get("vda5050_reference_barcode").and_then(Value::as_str) {
            if let Some(cell) =
                self.lens("mapcreator_cell").iter().find(|object| text(&external_ids(object), "barcode") == barcode)
            {
                let mut reference = Map::new();
                reference.insert("barcode".into(), json!(barcode));
                if let Some(vda) = external_ids(cell).get("vda_world_coordinate") {
                    reference.insert("vda5050_coordinate".into(), vda.clone());
                }
                out.insert("vda5050_reference".into(), Value::Object(reference));
            }
        }
        Ok(Value::Array(vec![Value::Object(out)]))
    }

    /// One `map_values` record per cell: the field map's own fields plus the eight
    /// the core protocol took. `floor` is the floor frame's identity, the frame
    /// every position `map.json` states is in.
    fn cell_rows(&self, floor: &str) -> Result<Vec<Value>, Error> {
        self.lens("mapcreator_cell")
            .iter()
            .map(|object| {
                let core = component(object, "cell");
                let formatting = formatting(object, "mapcreator_cell");
                let ids = external_ids(object);
                let id = object.anchor.id.to_string();
                let mut record = self.decode(records::CELL, &core, &formatting, &ids);
                if let Some(world) = self.world_coordinate(object, floor)? {
                    record.insert("world_coordinate".into(), json!(world));
                }
                if let Some(vda) = ids.get("vda_world_coordinate") {
                    record.insert("vda_world_coordinate".into(), vda.clone());
                }
                if let Some(blocked) = core.get("blocked") {
                    record.insert("blocked".into(), blocked.clone());
                }
                if let Some(sector) = self.cell_sector.get(&id) {
                    record.insert("sector".into(), sector.clone());
                }
                if let Some(block) = self.cell_block.get(&id) {
                    // `map_values` states the block as an integer and
                    // `block.json` as a string; the integer is what goes here.
                    record
                        .insert("block".into(), block.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(block)));
                }
                if self.dock_cells.contains_key(&id) {
                    record.insert("dockPoint".into(), json!(true));
                }
                Ok(self.finish(record, &formatting))
            })
            .collect()
    }

    /// A cell's position as `map.json` states one, where its pose states one:
    /// in the floor frame, two axes, each a finite length no further than
    /// `MAX_MM` from the origin. Any other pose would be written as another
    /// position — read in the wrong frame, cut to two axes, or a zero for what
    /// is no number — so it is refused, naming the cell.
    fn world_coordinate(&self, object: &Object, floor: &str) -> Result<Option<String>, Error> {
        let pose = component(object, "pose");
        let Some(position) = pose.get("position") else { return Ok(None) };
        let refuse = |why: String| {
            Error::Document(format!(
                "entity {}{} {why}, so map.json's `world_coordinate` could not be written back",
                object.anchor.id,
                named(object)
            ))
        };
        match pose.get("frame").and_then(Value::as_str) {
            Some(frame) if frame == floor => {}
            other => {
                return Err(refuse(format!(
                    "states its `pose` in {}, where map.json states a cell's position in the floor frame, {floor}",
                    other.map_or_else(|| "no frame".to_string(), |frame| format!("frame {frame}"))
                )))
            }
        }
        let axes = match position.as_array().map(Vec::as_slice) {
            Some([x, y]) => x.as_f64().zip(y.as_f64()).map(|(x, y)| [x, y]),
            _ => None,
        };
        let in_range = |steps: &f64| {
            let mm = millimetres(*steps, self.unit_mm);
            mm.is_finite() && mm.abs() <= MAX_MM
        };
        match axes.filter(|axes| axes.iter().all(in_range)) {
            Some(axes) => Ok(Some(spell_millimetres(axes, self.unit_mm))),
            None => Err(refuse(format!(
                "states its `pose.position` as {position}, where map.json states two finite numbers of millimetres \
                 no further than {MAX_MM} from the origin"
            ))),
        }
    }

    /// `sectors.json`: one object mapping each sector's key to its member cells, in
    /// `map_values` order. The inverse of the cell `sector` field, rebuilt from the
    /// sector entities rather than stored twice.
    fn sector_index(&self) -> Value {
        // The cells' own file order, which is the order each list has to come back
        // in. Read off each cell's ordering key, because a lens list is identity
        // ordered and the identities are random.
        let mut order: FxHashMap<String, i64> = FxHashMap::default();
        for object in self.lens("mapcreator_cell") {
            let index = formatting(object, "mapcreator_cell").get("index").and_then(Value::as_i64).unwrap_or(0);
            order.insert(object.anchor.id.to_string(), index);
        }
        let mut table = Map::new();
        for object in self.lens("mapcreator_sector") {
            let key = formatting(object, written::SECTOR).get("key").cloned().unwrap_or(Value::Null);
            let key = identity::scalar(&key);
            let mut members: Vec<(i64, String)> = component(object, "sector")
                .get("members")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|id| {
                    let coordinate = self.cell_keys.get(id)?.0.clone();
                    Some((*order.get(id).unwrap_or(&0), coordinate))
                })
                .collect();
            // ordered: the file lists a sector's cells in map_values order, so the
            // list is sorted by each cell's own record index before it is written.
            members.sort();
            table.insert(key, Value::Array(members.into_iter().map(|(_, c)| json!(c)).collect()));
        }
        Value::Array(vec![Value::Object(table)])
    }

    fn aisle_rows(&self) -> Vec<Value> {
        self.lens("mapcreator_aisle")
            .iter()
            .map(|object| {
                let mapcreator = component(object, "aisle");
                let formatting = formatting(object, "mapcreator_aisle");
                let ids = external_ids(object);
                let mut record = self.decode(records::AISLE, &mapcreator, &formatting, &ids);
                // The entries the file spelled as raw pairs, by their place in
                // the list; every other resolved entry is a coordinate string.
                let as_pairs: Vec<u64> = formatting
                    .get("io_point_coordinates_as_pairs")
                    .and_then(Value::as_array)
                    .map(|places| places.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default();
                let members = component(object, "sector").get("members").cloned();
                let points = in_place(members.as_ref(), mapcreator.get("io_point_coordinates_unresolved"), |id, at| {
                    let (coordinate, _) = self.cell_keys.get(id.as_str()?)?;
                    let pair = as_pairs.contains(&(at as u64));
                    spell_coordinate(coordinate, if pair { Spelling::RawPair } else { Spelling::Bracketed })
                });
                record.insert("io_point_coordinates".into(), Value::Array(points));
                self.finish(record, &formatting)
            })
            .collect()
    }

    /// `block.json`: the envelope the file writes, with the block's cells back as
    /// raw grid pairs.
    fn block_rows(&self) -> Value {
        Value::Array(
            self.lens("mapcreator_block")
                .iter()
                .map(|object| {
                    let mapcreator = component(object, "block");
                    let formatting = formatting(object, "mapcreator_block");
                    let ids = external_ids(object);
                    let mut details = self.decode(records::BLOCK, &mapcreator, &formatting, &ids);
                    let cells = block_cells(object);
                    let coordinates = in_place(cells.as_ref(), mapcreator.get("coordinates_unresolved"), |cell, _| {
                        let (coordinate, _) = self.cell_keys.get(cell.as_str()?)?;
                        spell_coordinate(coordinate, Spelling::RawPair)
                    });
                    details.insert("coordinates".into(), Value::Array(coordinates));
                    let mut record = Map::new();
                    if let Some(kind) = mapcreator.get("entity_type") {
                        record.insert("entity_type".into(), kind.clone());
                    }
                    record.insert("entity_details".into(), Value::Object(details));
                    self.finish(record, &formatting)
                })
                .collect(),
        )
    }

    /// `pps.json`: one record per station, with its parts nested the way its own
    /// dialect nests them.
    fn station_rows(&self) -> Result<Value, Error> {
        let mut rows = Vec::new();
        for object in self.lens("mapcreator_station") {
            let mapcreator = component(object, "mapcreator.station");
            let formatting = formatting(object, "mapcreator_station");
            // The shape the import read this record by, which the import wrote
            // down. Guessing it from the presence of `served_by` or `system` wrote
            // a v3 station with neither back in the v2 spelling.
            let shape = formatting.get("dialect_shape").and_then(Value::as_str).ok_or_else(|| {
                Error::Document(format!(
                    "station {} carries no `dialect_shape`, so nothing says which of pps.json's two \
                     dialects to write it back in",
                    object.anchor.id
                ))
            })?;
            let v3 = shape == "v3";
            let map = if v3 { records::STATION_V3 } else { records::STATION_V2 };
            let mut record = self.decode(map, &mapcreator, &formatting, &external_ids(object));
            if let Some(role) = mapcreator.get("role") {
                record.insert("type".into(), role.clone());
            }
            if v3 {
                let mut entities = Map::new();
                let points: Vec<Value> = self
                    .folded(object, "presentation_points", "mapcreator_presentation_point")
                    .into_iter()
                    .map(|view| self.presentation_record(&view))
                    .collect();
                if !(points.is_empty() && absent(&formatting, "entities.presentation_points")) {
                    entities.insert("presentation_points".into(), Value::Array(points));
                }
                let (served, unresolved) = (mapcreator.get("served_by"), mapcreator.get("served_by_unresolved"));
                if served.is_some() || unresolved.is_some() {
                    let spelled = in_place(served, unresolved, |block, _| {
                        self.block_ids.get(block.as_str()?).map(|block_id| json!(block_id))
                    });
                    entities.insert("blocks".into(), Value::Array(spelled));
                }
                record.insert("entities".into(), Value::Object(entities));
            } else {
                let points: Vec<Value> = self
                    .folded(object, "points", "mapcreator_station_point")
                    .into_iter()
                    .map(|view| {
                        let component = view
                            .components
                            .get("station_point")
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        let formatting =
                            written::entry(view.components.get(written::COMPONENT), "mapcreator_station_point");
                        let ids =
                            view.components.get("external_ids").and_then(Value::as_object).cloned().unwrap_or_default();
                        let record = self.decode(records::STATION_POINT, &component, &formatting, &ids);
                        self.finish(record, &formatting)
                    })
                    .collect();
                if !(points.is_empty() && absent(&formatting, "pps_points")) {
                    record.insert("pps_points".into(), Value::Array(points));
                }
            }
            rows.push(self.finish(record, &formatting));
        }
        Ok(Value::Array(rows))
    }

    /// One v3 presentation point's record, out of the entity the station folded on.
    fn presentation_record(&self, view: &EntityView) -> Value {
        let component =
            view.components.get("presentation_point").and_then(Value::as_object).cloned().unwrap_or_default();
        let formatting = written::entry(view.components.get(written::COMPONENT), "mapcreator_presentation_point");
        let ids = view.components.get("external_ids").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut record = self.decode(records::PRESENTATION_POINT, &component, &formatting, &ids);
        if let Some(purpose) = component.get("for") {
            record.insert("presented_for".into(), purpose.clone());
        }
        // `world_coordinates` is a GRID coordinate under a name that says world,
        // and it names the cell the barcode names, which the import held it to,
        // so it comes off that cell rather than out of a stored copy: except
        // where the point stated none, which its formatting records, and where
        // its barcode named no cell, when the copy the import kept comes back.
        if let Some(Value::Object(attributes)) = record.get_mut("attributes") {
            let cell = component
                .get("attributes")
                .and_then(Value::as_object)
                .and_then(|nested| nested.get("cell"))
                .and_then(Value::as_str);
            let stated = if absent(&formatting, "attributes.world_coordinates") {
                None
            } else if let Some(kept) = component.get("world_coordinates_unresolved") {
                Some(kept.clone())
            } else {
                cell.and_then(|cell| self.cell_keys.get(cell))
                    .and_then(|(coordinate, _)| spell_coordinate(coordinate, Spelling::RawPair))
            };
            if let Some(world) = stated {
                attributes.insert("world_coordinates".into(), world);
            }
        }
        self.finish(record, &formatting)
    }

    /// The entities one lens field folded onto an object, in the order the fold
    /// gave them, re-sorted by each part's own record index, which its
    /// formatting entry under `lens` states, never another entry's.
    fn folded(&self, object: &Object, field: &str, lens: &str) -> Vec<EntityView> {
        let mut views: Vec<(i64, EntityView)> = object
            .fields()
            .get(field)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|view| {
                let formatting = written::entry(view.components.get(written::COMPONENT), lens);
                let index = formatting.get("index").and_then(Value::as_i64).unwrap_or(0);
                (index, view.clone())
            })
            .collect();
        // ordered: a fold list is identity-ordered and the identities are random,
        // so the record order rides in the formatting and is restored here.
        views.sort_by_key(|(index, _)| *index);
        views.into_iter().map(|(_, view)| view).collect()
    }

    fn ods_rows(&self) -> Vec<Value> {
        self.lens("mapcreator_ods_exclusion")
            .iter()
            .map(|object| {
                let mapcreator = component(object, "ods_exclusion");
                let formatting = formatting(object, "mapcreator_ods_exclusion");
                let mut record = self.decode(records::ODS_EXCLUSION, &mapcreator, &formatting, &external_ids(object));
                // The tuple is rebuilt from the cell's barcode and the heading,
                // which is what makes the join an identity rather than a string —
                // except where the barcode named no cell, and then the string the
                // dump wrote is what comes back.
                let heading = mapcreator.get("heading").and_then(Value::as_str).unwrap_or_default();
                let barcode = mapcreator
                    .get("cell")
                    .and_then(Value::as_str)
                    .and_then(|id| self.cell_keys.get(id))
                    .map(|(_, barcode)| barcode.clone())
                    .or_else(|| mapcreator.get("cell_barcode_unresolved").and_then(Value::as_str).map(str::to_string));
                match (barcode, mapcreator.get("tuple_unparsed").and_then(Value::as_str)) {
                    (_, Some(unparsed)) => {
                        record.insert("ods_tuple".into(), json!(unparsed));
                    }
                    (Some(barcode), None) => {
                        record.insert("ods_tuple".into(), json!(format!("{barcode}--{heading}")));
                    }
                    (None, None) => {}
                }
                self.finish(record, &formatting)
            })
            .collect()
    }

    fn conveyor_file(&self) -> Value {
        let mut out = Map::new();
        if let Some(header) = self.export().get("conveyor_header").and_then(Value::as_object) {
            for (key, value) in header {
                out.insert(key.clone(), value.clone());
            }
        }
        let rows: Vec<Value> = self
            .lens("mapcreator_conveyor")
            .iter()
            .map(|object| {
                let mapcreator = component(object, "conveyor");
                let formatting = formatting(object, "mapcreator_conveyor");
                let ids = external_ids(object);
                self.finish(self.decode(records::CONVEYOR, &mapcreator, &formatting, &ids), &formatting)
            })
            .collect();
        out.insert("conveyor_data".into(), Value::Array(rows));
        Value::Object(out)
    }

    /// `sectorMxUPreferences.json`: one object of carrier types, or that object
    /// as the one element of a list, which is the form the export entity says
    /// the file arrived in. Each carrier's list is its sectors' keys, in the
    /// cells' spelling, with each key that named no sector back at its place.
    fn preference_table(&self) -> Value {
        let mut table = Map::new();
        for object in self.lens("mapcreator_sector_preference") {
            let mapcreator = component(object, "sector_preference");
            let ids = external_ids(object);
            let carrier = text(&ids, "mapcreator_preferred_for");
            let sectors = in_place(mapcreator.get("prefers"), mapcreator.get("prefers_unresolved"), |sector, _| {
                self.sector_keys.get(sector.as_str()?).cloned()
            });
            table.insert(carrier, Value::Array(sectors));
        }
        if self.form(FileKind::SectorMxUPreferences).as_deref() == Some("array") {
            return Value::Array(vec![Value::Object(table)]);
        }
        Value::Object(table)
    }

    /// A file whose model data sits inside a checked-in HTTP request envelope.
    fn enveloped(&self, wrapper: &str, data: Value) -> Value {
        let mut out = Map::new();
        if let Some(envelope) = self.export().get(wrapper).and_then(Value::as_object) {
            for (key, value) in envelope {
                out.insert(key.clone(), value.clone());
            }
        }
        out.insert("data".into(), data);
        Value::Object(out)
    }

    /// `pptl.json`: the devices back in their groups. Each device carries its
    /// group's place in the file, and the export entity the places of the groups
    /// that hold no device, so every group comes back where it stood.
    fn pptl_groups(&self) -> Value {
        // ordered: the file states its devices in groups and the group is not one
        // station's — site-e lists ten stations in twenty groups, the same
        // ten twice over — so each device carries its group and the map is walked
        // in group order.
        let mut groups: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
        for empty in self.export().get("pptl_empty_groups").and_then(Value::as_array).into_iter().flatten() {
            if let Some(group) = empty.as_i64() {
                groups.entry(group).or_default();
            }
        }
        for object in self.lens("mapcreator_pptl") {
            let mapcreator = component(object, "pptl");
            let formatting = formatting(object, "mapcreator_pptl");
            let ids = external_ids(object);
            let group = formatting.get("group").and_then(Value::as_i64).unwrap_or(0);
            let record = self.decode(records::PPTL, &mapcreator, &formatting, &ids);
            groups.entry(group).or_default().push(self.finish(record, &formatting));
        }
        Value::Array(
            groups
                .into_values()
                .map(|devices| {
                    let mut group = Map::new();
                    group.insert("pptls".into(), Value::Array(devices));
                    Value::Object(group)
                })
                .collect(),
        )
    }
}

/// A list written back in the order its file stated it: the entries that
/// resolved, spelled by `spell` in their own order, and each that named
/// nothing, `{at, value}`, back at the place it held among them. An entry
/// `spell` cannot spell would be left out, which is why `references` refuses a
/// site holding one before any list is written.
fn in_place(
    resolved: Option<&Value>,
    unresolved: Option<&Value>,
    mut spell: impl FnMut(&Value, usize) -> Option<Value>,
) -> Vec<Value> {
    let mut kept: Vec<(u64, Value)> = unresolved
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some((entry.get("at")?.as_u64()?, entry.get("value")?.clone())))
        .collect();
    // ordered: each kept entry goes back at its own place, so they are met in
    // the order of their places.
    kept.sort_by_key(|(at, _)| *at);
    let mut kept = kept.into_iter().peekable();
    let mut resolved = resolved.and_then(Value::as_array).into_iter().flatten();
    let mut out = Vec::new();
    loop {
        let here = out.len() as u64;
        if let Some((_, value)) = kept.next_if(|(at, _)| *at <= here) {
            out.push(value);
        } else if let Some(entry) = resolved.next() {
            if let Some(spelled) = spell(entry, out.len()) {
                out.push(spelled);
            }
        } else if let Some((_, value)) = kept.next() {
            out.push(value);
        } else {
            return out;
        }
    }
}

/// A run of axis values in the site frame's steps, spelled the way a Map
/// Creator file states a millimetre coordinate: a bracketed list with the steps
/// carried back into millimetres, and a whole number written as a whole number.
///
/// The steps are divided by the whole number of them a millimetre holds, rather
/// than multiplied by a step's binary approximation of a tenth, so that a
/// position written in tenths comes back as the decimal it was written as.
pub fn spell_millimetres(steps: impl IntoIterator<Item = f64>, unit_mm: f64) -> String {
    let parts: Vec<String> = steps
        .into_iter()
        .map(|steps| {
            let mm = millimetres(steps, unit_mm);
            if (mm - mm.round()).abs() < 1e-9 {
                format!("{}", mm.round() as i64)
            } else {
                format!("{mm}")
            }
        })
        .collect();
    format!("[{}]", parts.join(","))
}

/// The cells one block covers, from the list its kind keeps them in: a queue
/// block's are its core `queue.path`, and every other block's are on its own
/// `block.cells`, since the core has no field to state them in. A
/// `queue` another system put on a block of another kind is not read.
fn block_cells(object: &Object) -> Option<Value> {
    let (name, field) = block_cells_field(object);
    object.anchor.components.get(name).and_then(|component| component.get(field)).cloned()
}

/// The component and the field one block keeps its cells in, by its kind.
pub fn block_cells_field(object: &Object) -> (&'static str, &'static str) {
    let kind = object.anchor.components.get("block").and_then(|block| block.get("entity_type"));
    if kind.and_then(Value::as_str).is_some_and(|kind| records::QUEUE_KINDS.contains(&kind)) {
        ("queue", "path")
    } else {
        ("block", "cells")
    }
}

/// Whether a record's formatting says its record did not state one field,
/// which the export would otherwise write back.
fn absent(formatting: &Map<String, Value>, source: &str) -> bool {
    formatting.get("absent").and_then(Value::as_array).is_some_and(|fields| fields.iter().any(|field| field == source))
}

/// One object's formatting entry: how the file wrote the record the object is,
/// under the lens it is written back through.
pub fn formatting(object: &Object, lens: &str) -> Map<String, Value> {
    written::entry(object.anchor.components.get(written::COMPONENT), lens)
}

/// The lookup key an object's record is known by, for a sentence: " (the
/// cell 1 [3,2])", or nothing for an object that carries none.
fn named(object: &Object) -> String {
    identity::constructs_of(&external_ids(object))
        .into_iter()
        .next()
        .map(|(_, key)| format!(" (the {key})"))
        .unwrap_or_default()
}

/// One component's JSON off an object's anchor, or an empty object when the anchor
/// does not carry it.
pub fn component(object: &Object, name: &str) -> Map<String, Value> {
    object.anchor.components.get(name).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// An object's `external_ids`, which is where every legacy key rides.
pub fn external_ids(object: &Object) -> Map<String, Value> {
    component(object, "external_ids")
}

/// A legacy key as its file spells it. `external_ids` carries every legacy key
/// as a string, a number in its canonical spelling; the file states some of
/// them as numbers, and a number that will not parse stays the string rather
/// than becoming a zero.
fn spelled_key(key: &str, text: KeyText) -> Value {
    match text {
        KeyText::Number => number_of(key).unwrap_or_else(|| json!(key)),
        KeyText::Text => json!(key),
    }
}

/// The JSON number a canonical spelling names: an integer where it is one, and
/// otherwise the double it spells.
fn number_of(text: &str) -> Option<Value> {
    if let Ok(integer) = text.parse::<i64>() {
        return Some(Value::from(integer));
    }
    if let Ok(integer) = text.parse::<u64>() {
        return Some(Value::from(integer));
    }
    text.parse::<f64>().ok().and_then(Number::from_f64).map(Value::Number)
}

fn text(ids: &Map<String, Value>, key: &str) -> String {
    ids.get(key).map(identity::scalar).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::Projection;
    use crate::records::Spelling;
    use rustc_hash::FxHashMap;
    use serde_json::json;

    /// A millimetre position comes back as the export spells it: a bracketed pair
    /// of whole numbers, with the primary frame's step size multiplied back in.
    #[test]
    fn a_position_in_steps_comes_back_as_millimetres() {
        use super::spell_millimetres;
        assert_eq!(spell_millimetres([-46500.0, 13500.0], 1.0), "[-46500,13500]");
        assert_eq!(spell_millimetres([-465003.0, 135000.0], 0.1), "[-46500.3,13500]");
    }

    /// A numeric key comes back as the number its canonical spelling names,
    /// fractions included, and a key that is no number stays the string.
    #[test]
    fn a_numeric_key_comes_back_as_its_number() {
        use super::number_of;
        assert_eq!(number_of("31"), Some(json!(31)));
        assert_eq!(number_of("31.5"), Some(json!(31.5)));
        assert_eq!(number_of("18446744073709551615"), Some(json!(u64::MAX)));
        assert_eq!(number_of("C3"), None);
    }

    /// A reference this projection holds no cell for spells nothing rather than
    /// spelling a wrong coordinate, and the cell index answers it as no cell,
    /// which is what `references` refuses a site by.
    #[test]
    fn an_unknown_cell_reference_spells_nothing() {
        let projection = Projection::index(FxHashMap::default(), 1.0);
        assert_eq!(projection.spell_one_cell(&json!("not-an-id"), Spelling::Barcode), None);
        assert_eq!(projection.spell_one_cell(&json!(7), Spelling::Bracketed), None);
        assert_eq!(projection.cell("not-an-id"), None);
    }
}
