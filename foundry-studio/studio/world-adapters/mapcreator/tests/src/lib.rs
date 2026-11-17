//! What the Map Creator adapter's tests share: the module, built once per test
//! binary and loaded the way any host loads a module, and the exports a test
//! hands it.
//!
//! The check.py wrapper builds the current source once and supplies that module
//! to every test binary. The converter is never linked into a host test, so
//! what these tests prove is true of the WASM module a host runs.

use site_model::Entity as WorldEntity;
use jsonschema::Validator;
use jsonschema::Resource;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;
use worldadapt_host::{Budget, Change, Engine, Entity, File, ImportPolicy, Module, RandomIds, Site};
use worldadapt_testkit::{apply, Deleted};
use worldlens::{Graph, Library};
use site_model::{EntityId, World};

fn foundry_root() -> PathBuf {
    PathBuf::from(env!("FOUNDRY_APPLICATION_ROOT"))
}

/// The component built by check.py, loaded once per test binary.
pub fn module_bytes() -> &'static [u8] {
    static MODULE: OnceLock<Vec<u8>> = OnceLock::new();
    MODULE.get_or_init(|| {
        let path = std::env::var("FOUNDRY_MAPCREATOR_MODULE").expect("run check.py to build the tested module");
        fs::read(path).expect("the built module")
    })
}

/// The adapter, loaded under the policy that traps every import the contract
/// does not lend, and the default budget.
pub fn mapcreator() -> Module {
    let engine = Engine::new().expect("an engine");
    Module::load(&engine, module_bytes(), ImportPolicy::TrapOnUse, Budget::default()).expect("the adapter loads")
}

/// What the production exports' calls run under. The default budget, sized for
/// a generator's one object, holds every call of the two smallest exports and
/// no call of the three large ones.
///
/// Measured on 2026-10-08 with the 1,296,695-byte module, which writes each
/// file back byte for byte, keeps how a file wrote each record in its
/// `import_formatting`, refuses by name what the fourth fix round refuses and
/// a file whose text it could not write back, and keeps the key orders a site
/// already holds, through `what_each_production_call_spends` (fuel in
/// wasmtime's units; memory is what the instance held when the call returned,
/// its peak, since a WebAssembly memory never shrinks):
///
/// | export | first ingest | projection | second ingest |
/// |---|---|---|---|
/// | site-a | 6.14 G, 186 MB | 6.03 G, 290 MB | 7.55 G, 291 MB |
/// | site-b | 6.20 G, 184 MB | 6.15 G, 299 MB | 7.68 G, 289 MB |
/// | site-c | 1.32 G, 41 MB | 1.30 G, 57 MB | 1.61 G, 62 MB |
/// | site-d | 4.11 G, 131 MB | 4.25 G, 219 MB | 5.13 G, 207 MB |
/// | site-e | 1.29 G, 39 MB | 1.19 G, 52 MB | 1.58 G, 59 MB |
///
/// The second ingest, the export into the site its first ingest made, is the
/// dearest call in fuel, since it reads the whole site as well as the files,
/// and each file twice: as its values, and as its text states them, key order
/// and all; the projection of the largest site is, by a little, the dearest in
/// memory. This allows about two and a half times the most fuel measured and
/// not quite twice the most memory, so a growth of the sites or of the module
/// shows up as a change in these numbers before it fails a test.
pub const PRODUCTION: Budget =
    Budget { fuel: 20_000_000_000, run_fuel: 1_000_000_000_000, memory_bytes: 512 << 20, fresh_ids: 1_000_000 };

/// The variable that names the directory the production exports are unpacked
/// under, one folder per export, named by its letter.
pub const DUMPS: &str = "MAPCREATOR_DUMPS";

/// The five production site exports, lettered `site-a` to `site-e`, each with
/// the number of entities its first ingest declares — the number the crate this
/// adapter was ported from wrote for it, one entity per record and per frame,
/// robot class and stitch. The letters are all the repository knows of them.
pub const SITES: [(&str, usize); 5] =
    [("site-a", 26_734), ("site-b", 28_087), ("site-c", 5_543), ("site-d", 20_952), ("site-e", 5_657)];

/// How many `*_unresolved` fields each production export's first ingest
/// writes: references the export makes that name nothing in it, kept as the
/// file wrote them. Both laws pass whether or not a reference resolved, so a
/// reference that stopped resolving shows up here and nowhere else. Site-a's
/// 24 are obstacle-detection exclusions whose tuple names a barcode no cell
/// carries; site-b's 2 are sector preferences naming sectors no cell states.
pub const UNRESOLVED: [(&str, usize); 5] = [("site-a", 24), ("site-b", 2), ("site-c", 0), ("site-d", 0), ("site-e", 0)];

/// One production export's files, read from the directory [`DUMPS`] names.
///
/// Unset, this panics naming the variable, so a test that needs the exports
/// fails on a machine without them rather than passing having run nothing.
/// The exports are production data and never enter the repository.
pub fn production_export(site: &str) -> Vec<File> {
    let root = std::env::var(DUMPS).unwrap_or_else(|_| {
        panic!(
            "{DUMPS} is unset. The five production site exports are not in the repository; set {DUMPS} to the \
             directory they are unpacked under, one folder per export named by its letter, `site-a` to `site-e`, \
             and run again"
        )
    });
    files_in(&PathBuf::from(root).join(site))
}

/// One synthetic export under `fixtures/`, every value in it invented: `current`
/// (the current dialect, all nineteen files), `current-edited` (the same site
/// after the edits `fixtures.rs` lists), `older` (the older dialect's eight
/// files) and `tenths` (a site measured in tenths of a millimetre, with a
/// station).
pub fn fixture(name: &str) -> Vec<File> {
    files_in(&foundry_root().join("data/seed/application-tests/mapcreator").join(name))
}

/// Every file of one export directory, by its name, in name order.
pub fn files_in(directory: &Path) -> Vec<File> {
    let mut files: Vec<File> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| {
            let path = entry.expect("a directory entry").path();
            let name = path.file_name().expect("a file name").to_string_lossy().into_owned();
            File { name, bytes: fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display())) }
        })
        .collect();
    files.sort_by(|one, other| one.name.cmp(&other.name));
    files
}

/// One file brought to the one spelling the laws compare: the normalization the
/// crate this adapter was ported from compared its round trip under, and no
/// other. Object key order goes, because JSON has none; a whole number written
/// `40.0` is the number `40`. An empty array stays an empty array and is not an
/// absent key, a string `"null"` stays a string, and a reordered list is a
/// difference.
///
/// The normalized value is written one value per line, so that where a law
/// finds a difference it names the line, rather than a file Map Creator wrote
/// on one line. A file that is not JSON is compared as it is.
pub fn normalized(_name: &str, bytes: &[u8]) -> Vec<u8> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => serde_json::to_vec_pretty(&normalize(&value)).expect("a JSON value serializes"),
        Err(_) => bytes.to_vec(),
    }
}

/// One value with every difference that is only a spelling normalized away.
pub fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            // ordered: written back in sorted key order, which is what erases
            // the order the file stated its keys in.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|key| (key.clone(), normalize(&map[key]))).collect::<Map<_, _>>())
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && float.abs() < 9.007e15 => Value::from(float as i64),
            _ => value.clone(),
        },
        other => other.clone(),
    }
}

/// The cells one export's `map.json` states, in file order.
pub fn map_values(files: &[File]) -> Vec<Value> {
    let map = file_json(files, "map.json");
    map.get(0).and_then(|floor| floor.get("map_values")).and_then(Value::as_array).cloned().expect("map_values")
}

/// One file of an export, parsed.
pub fn file_json(files: &[File], name: &str) -> Value {
    let file = files.iter().find(|file| file.name == name).unwrap_or_else(|| panic!("the export carries no {name}"));
    serde_json::from_slice(&file.bytes).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The same export with one file's JSON replaced, written as Map Creator writes
/// it: one compact line.
pub fn with_file(files: &[File], name: &str, value: &Value) -> Vec<File> {
    let bytes = serde_json::to_vec(value).expect("JSON serializes");
    files
        .iter()
        .map(
            |file| {
                if file.name == name {
                    File { name: file.name.clone(), bytes: bytes.clone() }
                } else {
                    file.clone()
                }
            },
        )
        .collect()
}

/// The same export with `map.json`'s cells replaced.
pub fn with_map_values(files: &[File], records: Vec<Value>) -> Vec<File> {
    let mut map = file_json(files, "map.json");
    map[0]["map_values"] = Value::Array(records);
    with_file(files, "map.json", &map)
}

/// One file's JSON as its text orders it: every object's keys in the order the
/// file states them, which `serde_json`'s map, sorted, forgets. The production
/// edits are made on this, as Map Creator makes an edit, so what an edit does
/// not touch is written back as it stood and no record is rewritten for it.
#[derive(Debug, Clone, PartialEq)]
pub enum Ordered {
    Object(Vec<(String, Ordered)>),
    Array(Vec<Ordered>),
    Scalar(Value),
}

impl Ordered {
    /// A value an edit writes, its objects' keys in `serde_json`'s order.
    pub fn of(value: &Value) -> Ordered {
        match value {
            Value::Object(map) => {
                Ordered::Object(map.iter().map(|(key, value)| (key.clone(), Ordered::of(value))).collect())
            }
            Value::Array(items) => Ordered::Array(items.iter().map(Ordered::of).collect()),
            scalar => Ordered::Scalar(scalar.clone()),
        }
    }

    /// The value under one key of an object.
    pub fn get(&self, key: &str) -> Option<&Ordered> {
        match self {
            Ordered::Object(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Ordered> {
        match self {
            Ordered::Object(entries) => entries.iter_mut().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    /// One key of an object set where it stands, or added last.
    pub fn set(&mut self, key: &str, value: Ordered) {
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => match self {
                Ordered::Object(entries) => entries.push((key.to_string(), value)),
                _ => panic!("{key} set on what is no object"),
            },
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Ordered::Object(entries) = self {
            entries.retain(|(name, _)| name != key);
        }
    }

    /// The items of a list.
    pub fn items_mut(&mut self) -> Option<&mut Vec<Ordered>> {
        match self {
            Ordered::Array(items) => Some(items),
            _ => None,
        }
    }

    /// A scalar's value.
    pub fn value(&self) -> Option<&Value> {
        match self {
            Ordered::Scalar(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        self.value().and_then(Value::as_str)
    }

    /// The text, on one line, with `item` between two items and `key` after a
    /// key.
    fn write(&self, item: &str, key: &str, out: &mut Vec<u8>) {
        let (open, close, entries): (u8, u8, Vec<(Option<&String>, &Ordered)>) = match self {
            Ordered::Object(entries) => (b'{', b'}', entries.iter().map(|(name, value)| (Some(name), value)).collect()),
            Ordered::Array(items) => (b'[', b']', items.iter().map(|value| (None, value)).collect()),
            Ordered::Scalar(value) => {
                serde_json::to_writer(&mut *out, value).expect("a JSON scalar serializes");
                return;
            }
        };
        out.push(open);
        for (at, (name, value)) in entries.into_iter().enumerate() {
            if at > 0 {
                out.extend(item.as_bytes());
            }
            if let Some(name) = name {
                serde_json::to_writer(&mut *out, name).expect("a string serializes");
                out.extend(key.as_bytes());
            }
            value.write(item, key, out);
        }
        out.push(close);
    }
}

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Ordered, D::Error> {
        deserializer.deserialize_any(OrderedVisitor)
    }
}

struct OrderedVisitor;

impl<'de> Visitor<'de> for OrderedVisitor {
    type Value = Ordered;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Number::from_f64(value).map_or(Value::Null, Value::Number)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::String(value.to_string())))
    }

    fn visit_unit<E>(self) -> Result<Ordered, E> {
        Ok(Ordered::Scalar(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Ordered, A::Error> {
        let mut out = Vec::new();
        while let Some(item) = items.next_element()? {
            out.push(item);
        }
        Ok(Ordered::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Ordered, A::Error> {
        let mut out = Vec::new();
        while let Some((key, value)) = entries.next_entry()? {
            out.push((key, value));
        }
        Ok(Ordered::Object(out))
    }
}

/// One file of a production export edited in place, as Map Creator writes an
/// edit: read keeping every object's key order, changed by `edit`, and written
/// back on one line in the separators it was written in, compact or with a
/// space after each. Every production file is one line in one of the two.
pub fn edited_in_place(files: &[File], name: &str, edit: impl FnOnce(&mut Ordered)) -> Vec<File> {
    let mut files = files.to_vec();
    let file = files.iter_mut().find(|file| file.name == name).unwrap_or_else(|| panic!("no {name}"));
    let mut value: Ordered = serde_json::from_slice(&file.bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
    let (item, key) = [(",", ":"), (", ", ": ")]
        .into_iter()
        .find(|(item, key)| {
            let mut text = Vec::new();
            value.write(item, key, &mut text);
            text == file.bytes
        })
        .unwrap_or_else(|| panic!("{name} is not one line in either pair of separators"));
    edit(&mut value);
    let mut text = Vec::new();
    value.write(item, key, &mut text);
    file.bytes = text;
    files
}

/// The same export with the cell at `at` taken out, edited in place, and the
/// grid coordinate of the cell that went. The sector index loses the cell too,
/// as Map Creator writes it: the index is the inverse of the cells' own
/// `sector` fields.
pub fn without_one_cell(files: &[File], at: usize) -> (String, Vec<File>) {
    let mut coordinate = String::new();
    let files = edited_in_place(files, "map.json", |map| {
        let cells = map_values_of(map);
        let gone = cells.remove(at);
        coordinate = gone.get("coordinate").and_then(Ordered::as_str).expect("a cell states its coordinate").into();
    });
    let files = with_sector_index(&files, |index| {
        let Ordered::Object(entries) = index else { panic!("the sector index is one object") };
        for (key, members) in entries.iter_mut() {
            let members = members.items_mut().expect("a sector lists its cells");
            members.retain(|member| member.as_str() != Some(coordinate.as_str()));
            if members.is_empty() {
                return Some(key.clone());
            }
        }
        None
    });
    (coordinate, files)
}

/// The cells of a `map.json` read keeping its order.
fn map_values_of(map: &mut Ordered) -> &mut Vec<Ordered> {
    let floor = map.items_mut().and_then(|floors| floors.first_mut()).expect("map.json states a floor");
    floor.get_mut("map_values").and_then(Ordered::items_mut).expect("map_values")
}

/// The same export with its sector index changed in place by `edit`, which
/// answers the key of a sector it emptied, for the index to drop: a sector no
/// cell names has no entry.
fn with_sector_index(files: &[File], edit: impl FnOnce(&mut Ordered) -> Option<String>) -> Vec<File> {
    let Some(name) = ["sectors.json", "sector.json"].into_iter().find(|name| files.iter().any(|f| f.name == *name))
    else {
        return files.to_vec();
    };
    edited_in_place(files, name, |index| {
        let table = index.items_mut().and_then(|tables| tables.first_mut()).expect("the sector index is one object");
        if let Some(emptied) = edit(table) {
            table.remove(&emptied);
        }
    })
}

/// The same export with one cell more, after the one at `at`, edited in
/// place, and its grid coordinate.
///
/// The new cell copies its neighbour and takes Map Creator's own off-grid
/// spelling — `x == 999`, "no grid position" — and states no neighbour, so it
/// joins no stitch and takes no part in the grid fit. Its coordinate and
/// barcode are invented.
pub fn with_one_more_cell(files: &[File], at: usize) -> (String, Vec<File>) {
    let coordinate = "[999,4242]".to_string();
    let (mut after, mut sector) = (String::new(), None);
    let files = edited_in_place(files, "map.json", |map| {
        let cells = map_values_of(map);
        let mut fresh = cells[at].clone();
        fresh.set("coordinate", Ordered::of(&json!(coordinate)));
        fresh.set("barcode", Ordered::of(&json!("999.424")));
        fresh.set("adjacency", Ordered::of(&json!([null, null, null, null])));
        fresh.set("neighbours", Ordered::of(&json!([[0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]])));
        fresh.remove("vda_world_coordinate");
        after = cells[at].get("coordinate").and_then(Ordered::as_str).expect("a cell states its coordinate").into();
        sector = fresh.get("sector").and_then(Ordered::value).map(|key| match key {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        });
        cells.insert(at + 1, fresh);
    });
    let files = with_sector_index(&files, |index| {
        let members = index.get_mut(sector.as_deref()?)?.items_mut()?;
        let at = members.iter().position(|member| member.as_str() == Some(after.as_str()))?;
        members.insert(at + 1, Ordered::of(&json!(coordinate)));
        None
    });
    (coordinate, files)
}

/// The two millimetre values a cell's `"[x,y]"` position states.
pub fn millimetre_pair(text: &str) -> [f64; 2] {
    let inner = text.trim().trim_start_matches('[').trim_end_matches(']');
    let axes: Vec<f64> = inner.split(',').map(|part| part.trim().parse().expect("a millimetre number")).collect();
    [axes[0], axes[1]]
}

/// A millimetre position spelled the way `map.json` states one.
pub fn spell_millimetres(at: [f64; 2]) -> String {
    let axis = |mm: f64| if mm.fract() == 0.0 { format!("{}", mm as i64) } else { format!("{mm}") };
    format!("[{},{}]", axis(at[0]), axis(at[1]))
}

/// The same export with every cell's measured x moved by `by` millimetres: the
/// whole floor, on-grid cells and off-grid ones alike.
pub fn with_every_cell_shifted(files: &[File], by: f64) -> Vec<File> {
    let mut records = map_values(files);
    for record in &mut records {
        let [x, y] = millimetre_pair(record["world_coordinate"].as_str().expect("a cell states where it is"));
        record["world_coordinate"] = Value::String(spell_millimetres([x + by, y]));
    }
    with_map_values(files, records)
}

/// The site an export's first ingest into an empty site leaves.
pub fn ingested(module: &Module, files: &[File], budget: Budget, scope: &str) -> Site {
    let empty = Site { scope: scope.into(), entities: Vec::new() };
    let changes = module.ingest(budget, files, &empty, Box::new(RandomIds)).expect("the export ingests");
    apply(&empty, &changes, &mut Deleted::default()).expect("the changes apply")
}

/// The one entity of a site whose `external_ids` holds `key` as `value`.
pub fn keyed<'s>(site: &'s Site, key: &str, value: &str) -> &'s Entity {
    let carries =
        |entity: &&Entity| entity.components.get("external_ids").and_then(|ids| ids.get(key)) == Some(&json!(value));
    let mut found = site.entities.iter().filter(carries);
    let entity = found.next().unwrap_or_else(|| panic!("no entity carries {key} {value}"));
    assert!(found.next().is_none(), "two entities carry {key} {value}");
    entity
}

/// Every entity of a site whose components name `id` somewhere: the entities
/// that reference it.
pub fn naming(site: &Site, id: &str) -> BTreeSet<String> {
    fn names(value: &Value, id: &str) -> bool {
        match value {
            Value::String(text) => text == id,
            Value::Array(items) => items.iter().any(|item| names(item, id)),
            Value::Object(map) => map.values().any(|item| names(item, id)),
            _ => false,
        }
    }
    site.entities
        .iter()
        .filter(|entity| entity.id != id && entity.components.values().any(|value| names(value, id)))
        .map(|entity| entity.id.clone())
        .collect()
}

/// Every `*_unresolved` field of a site, at any depth of any component, each
/// as the entity's own keys and the field's path: a reference the import kept
/// as the file wrote it because it named nothing. Sorted, so a test can name
/// the exact set it expects.
pub fn unresolved_fields(site: &Site) -> Vec<String> {
    fn walk(value: &Value, path: &str, out: &mut Vec<String>) {
        if let Value::Object(map) = value {
            for (key, child) in map {
                let here = format!("{path}.{key}");
                if key.ends_with("_unresolved") {
                    out.push(here.clone());
                }
                walk(child, &here, out);
            }
        }
    }
    let mut out = Vec::new();
    for entity in &site.entities {
        let mut found = Vec::new();
        for (name, component) in &entity.components {
            walk(component, name, &mut found);
        }
        if found.is_empty() {
            continue;
        }
        let ids = entity.components.get("external_ids").and_then(Value::as_object).cloned().unwrap_or_default();
        let keys: Vec<String> = ids
            .iter()
            .filter(|(name, _)| name.starts_with("mapcreator_"))
            .map(|(name, value)| format!("{name}={}", value.as_str().unwrap_or_default()))
            .collect();
        out.extend(found.into_iter().map(|path| format!("{} {path}", keys.join(" "))));
    }
    out.sort();
    out
}

/// One site written as a world document whose primary frame is the adapter's
/// site frame, and loaded by the reference reader. The scratch directory the
/// document was written to comes back with the world, since the reader loads
/// from a path. The document requires the two components a consumer has to
/// understand to read the site at all, the frames and the stitches, as the
/// document the crate this adapter was ported from wrote did.
pub fn loaded(site: &Site) -> (World, TempDir) {
    let primary = keyed(site, "mapcreator_frame", "site").id.clone();
    let entities: Vec<Value> = site
        .entities
        .iter()
        .map(|entity| {
            let mut record = json!({ "id": entity.id, "components": entity.components });
            if let Some(name) = &entity.name {
                record["name"] = json!(name);
            }
            record
        })
        .collect();
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-0000000000f1", "kind": "world", "name": "an ingested site" },
        "primary_frame": primary,
        "required_components": ["frame", "stitch"],
        "entities": entities
    });
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("site.world.json");
    fs::write(&path, serde_json::to_vec(&document).expect("JSON")).expect("the document writes");
    let (_layers, world) = site_model::load(&path).unwrap_or_else(|error| panic!("the site does not load: {error}"));
    (world, scratch)
}

/// The adapter's own lens library, which the tests run over a site to count
/// what a plain scan of its entities counts.
pub fn lens_library() -> Library {
    let path = foundry_root().join("data/seed/application/lenses/mapcreator.worldlens");
    Library::parse(&fs::read_to_string(path).expect("the lens text")).expect("the lens text parses")
}

/// A site's entities as a lens graph, as a host holding the site has them.
pub fn graph_of(site: &Site) -> Graph<'static> {
    let entities = site
        .entities
        .iter()
        .map(|held| {
            let id: EntityId = held.id.parse().expect("an identity");
            let mut entity = WorldEntity::new(id.clone());
            entity.name = held.name.clone();
            entity.components = held.components.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
            (id, entity)
        })
        .collect();
    Graph::of_entities(entities)
}

/// The core components the import writes, beside the ones under `vocabulary/`.
pub const CORE: [&str; 12] = [
    "cell",
    "external_ids",
    "frame",
    "lineage",
    "pose",
    "presentation_point",
    "queue",
    "robot_class",
    "sector",
    "station",
    "stitch",
    "waiting",
];

/// The core components the import writes Map Creator's fields into, beside
/// the core's own: their schemas and their reference annotations are the
/// test fixtures, and the adapter's own unit tests hold its field maps to both.
pub const EXTENDED: [&str; 2] = ["cell", "presentation_point"];

/// The definitions under `vocabulary/`, by component.
pub fn vocabulary() -> BTreeMap<String, Value> {
    let directory = foundry_root().join("data/seed/application/mapcreator/components");
    fs::read_dir(&directory)
        .expect("the vocabulary directory")
        .map(|entry| {
            let path = entry.expect("an entry").path();
            let definition: Value = serde_json::from_slice(&fs::read(&path).expect("a definition")).expect("JSON");
            (definition["component"].as_str().expect("a name").to_string(), definition)
        })
        .collect()
}

/// The references one payload holds at the paths a definition's `refs`
/// annotate, read as the server's extractor reads them: the value a pointer
/// names, or each element of the list a pointer ending `/*` names; each with
/// the path it was read at and the component its target is expected to carry,
/// where the annotation names one.
pub fn annotated_references<'p>(payload: &'p Value, refs: &'p Value) -> Vec<(String, &'p Value, Option<&'p str>)> {
    let mut out = Vec::new();
    for one in refs.as_array().into_iter().flatten() {
        let path = one["path"].as_str().expect("an annotation states its path");
        let expects = one["expects"].as_str();
        match path.strip_suffix("/*") {
            None => out.extend(payload.pointer(path).map(|named| (path.to_string(), named, expects))),
            Some(list) => {
                let entries = payload.pointer(list).and_then(Value::as_array).into_iter().flatten();
                out.extend(entries.enumerate().map(|(at, named)| (format!("{list}/{at}"), named, expects)));
            }
        }
    }
    out
}

/// Test-only reference annotations for the core application components.
pub fn seed_refs() -> BTreeMap<String, Value> {
    let repository = foundry_root();
    let bytes = fs::read(repository.join("data/seed/application/core-components.json"))
        .expect("the core component definitions");
    let core: BTreeMap<String, Value> = serde_json::from_slice(&bytes).expect("the core component definitions");
    core.into_iter().map(|(name, definition)| (name, definition["refs"].clone())).collect()
}

/// The test schema for each core component the import
/// writes, derived from the test protocol schema layer.
pub fn seed_schemas() -> BTreeMap<String, Value> {
    let repository = foundry_root();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let out = scratch.path().join("schemas.json");
    let script = repository.join("scripts/derive-component-schemas.py");
    let ran = Command::new("python3").arg(&script).arg("--out").arg(&out).args(CORE).output().expect("python3 runs");
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    serde_json::from_slice(&fs::read(&out).expect("the schemas")).expect("JSON")
}

/// One schema compiled the way the server compiles one: with the server's own
/// definitions in scope, and nothing fetched from anywhere.
pub fn compiled(schema: &Value) -> Validator {
    let repository = PathBuf::from(std::env::var("FOUNDRY_WPS_ROOT").expect("run check.py with --world-proto"));
    let definitions: Value =
        serde_json::from_slice(&fs::read(repository.join("server/server/server-defs.json")).expect("server-defs.json"))
            .expect("JSON");
    let resource = Resource::from_contents(definitions).expect("a draft the validator knows");
    jsonschema::options()
        .with_resource("urn:world-graph-server:defs", resource)
        .build(schema)
        .unwrap_or_else(|error| panic!("a schema does not compile: {error}"))
}

/// The identities a list of changes names, by operation.
pub fn by_op<'c>(changes: &'c [Change], op: &str) -> BTreeSet<&'c str> {
    changes.iter().filter(|change| change.op() == op).map(Change::id).collect()
}

#[cfg(test)]
mod tests {
    use super::{normalize, normalized};
    use serde_json::json;

    #[test]
    fn normalization_erases_key_order_and_whole_number_spelling() {
        assert_eq!(normalize(&json!({ "b": 1, "a": 2 })), normalize(&json!({ "a": 2, "b": 1 })));
        assert_eq!(normalize(&json!(40.0)), json!(40));
        assert_eq!(normalize(&json!([97.9, 40.0])), json!([97.9, 40]));
        assert_eq!(normalized("a.json", br#"{"b":40.0,"a":[1]}"#), normalized("a.json", br#"{"a":[1],"b":40}"#));
    }

    /// What normalization deliberately does NOT erase. Each of these is a fact
    /// about a site that a tidier comparison would hide.
    #[test]
    fn normalization_keeps_every_difference_that_is_a_fact() {
        assert_ne!(normalize(&json!("null")), normalize(&json!(null)));
        assert_ne!(normalize(&json!([])), normalize(&json!({})));
        assert_ne!(normalize(&json!({ "a": [] })), normalize(&json!({})));
        assert_ne!(normalize(&json!(1)), normalize(&json!("1")));
        assert_ne!(normalize(&json!([1, 2])), normalize(&json!([2, 1])));
    }
}
