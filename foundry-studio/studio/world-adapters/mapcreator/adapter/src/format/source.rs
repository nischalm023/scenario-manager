//! Within `format`, `source` is **the export as it actually is**: the
//! nineteen sibling JSON files of one Map Creator site dump, read without
//! correction.
//!
//! Its job is to be literal. The export has no manifest, no schema version, and
//! four different ways of naming a cell (grid coordinate string, raw grid pair,
//! barcode, display name), so this module reads what is there and normalizes
//! nothing that would erase a difference: `"undefined"` stays the string
//! `"undefined"`, the `x == 999` escape hatch stays a grid coordinate of 999, and
//! a cell's `coordinate` string is kept verbatim because it is the export's own
//! primary key and the ledger has to be able to quote it.
//!
//! It also takes the **field census** every file's ledger is checked against: a
//! count of every key path each file carries. Nothing may be dropped silently, so
//! the census is what a missing disposition is measured against — a field family
//! present in a dump and absent from `coverage`'s table fails the run.

use rustc_hash::FxHashMap;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The files a site dump may contain, in the order butler's own upload sequence
/// posts them (zones before the map, the map before everything that references
/// cells). A dump is not required to carry all of them: the RTP sites ship no
/// `block.json`, and five files are empty on every site seen.
pub const EXPORT_FILES: &[&str] = &[
    "zone.json",
    "map.json",
    "block.json",
    "pps.json",
    "tote_storage.json",
    "tote_type.json",
    "conveyor.json",
    "conveyor_mapping.json",
    "charger.json",
    "elevator.json",
    "vtm_zone.json",
    "dock_point.json",
    "ods_excluded.json",
    "sectors.json",
    "sector.json",
    "sectorMxUPreferences.json",
    "fire_emergency.json",
    "queue_data.json",
    "opc_connection.json",
    "zone_controller.json",
    "pps_bin.json",
    "pps_bintype.json",
    "pps_seat.json",
    "pptl.json",
];

/// Key paths below which the census stops descending, because the keys there are
/// *data* rather than field names: a bot-type string, a sector id, a rack type. A
/// census that descended into them would report one "field" per site value and
/// bury the actual field families.
const OPAQUE: &[&str] = &[
    "map.json:[].location_profiles[].bot_orientation_details",
    "map.json:[].location_profiles[].edge_height_clearance",
    "map.json:[].map_values[].barcode_properties",
    "sectors.json:[]",
    "sector.json:[]",
    "sectorMxUPreferences.json",
];

/// One cell of `map.json`'s `map_values`, read into the parts the import needs
/// plus the raw object, which `external_ids` and the ledger quote from.
#[derive(Debug, Clone)]
pub struct Cell {
    /// The export's own primary key, verbatim: `"[7,1]"`.
    pub coordinate: String,
    pub grid: [i64; 2],
    pub world: [f64; 2],
    /// The vendor coordinate, where the cell states one that measures: two
    /// finite numbers of millimetres in range.
    pub vda: Option<[f64; 2]>,
    pub barcode: String,
    /// Half-extents toward directions 0..3, millimetres, where the cell states
    /// a `size_info`: a cell stating none has no footprint.
    pub size_info: Option<[f64; 4]>,
    pub blocked: bool,
    pub zone: Option<String>,
    /// Kept as written, so the string `"undefined"` is distinguishable from a
    /// real sector id.
    pub sector: Option<String>,
    pub block: Option<i64>,
    pub location_profile_id: Option<String>,
    /// Per direction 0..3: the grid coordinate of the connected neighbour.
    pub adjacency: [Option<[i64; 2]>; 4],
    /// Per direction 0..3: `[neighbour exists, bot may move, bot with load may
    /// move]`, decoded by butler at `vda5050_map:get_allowed_movements/1`.
    pub neighbours: [[i64; 3]; 4],
    pub raw: Map<String, Value>,
}

impl Cell {
    /// The cell's footprint in millimetres, each axis the sum of its two
    /// half-extents, where the cell states them.
    pub fn size_mm(&self) -> Option<[f64; 2]> {
        self.size_parts().map(|parts| parts.map(|[one, other]| one + other))
    }

    /// The two half-extents each axis of the footprint sums. Direction 0 points
    /// toward -y and 2 toward +y; 1 toward +x and 3 toward -x
    /// (research/mapcreator-format.md §4.5).
    pub fn size_parts(&self) -> Option<[[f64; 2]; 2]> {
        self.size_info.map(|half| [[half[1], half[3]], [half[0], half[2]]])
    }

    /// Whether this cell uses the `x == 999` escape hatch — "no grid position, y
    /// is a synthetic sequence number". Its millimetre position is still real.
    pub fn is_off_grid(&self) -> bool {
        self.grid[0] == 999
    }

    /// Whether this cell sits on the `[500,500]…` re-initialization diagonal,
    /// which every site marks `special: true` and every charger points at.
    pub fn is_reinit(&self) -> bool {
        !self.is_off_grid() && (self.grid[0] >= 500 || self.grid[1] >= 500)
    }

    /// Whether this cell's grid coordinate is a real lattice position, and so
    /// belongs in the fit that gives the nominal grid frame its transform.
    pub fn is_on_grid(&self) -> bool {
        !self.is_off_grid() && !self.is_reinit()
    }
}

/// One named bundle of per-bot-type permissions, `map.json`'s one genuinely
/// well-shaped table.
#[derive(Debug, Clone)]
pub struct LocationProfile {
    pub id: String,
    /// Bot type to whether that type's detail map is non-empty. An empty object
    /// means the type is forbidden here; a type absent from the map altogether
    /// says nothing at all, which is a different thing again (see `coverage`).
    pub bots: BTreeMap<String, bool>,
}

/// One site dump, borrowing the files it was read from rather than copying them:
/// an export is the largest thing an ingest holds besides the site.
#[derive(Debug)]
pub struct Site<'a> {
    /// Every file that was present and parsed, by file name.
    pub files: &'a BTreeMap<String, Value>,
    /// Files named in `EXPORT_FILES` that this dump does not carry.
    pub absent: Vec<String>,
    pub floor_id: i64,
    pub cells: Vec<Cell>,
    pub location_profiles: Vec<LocationProfile>,
    /// The single sample point the export offers in place of a frame transform.
    pub vda5050_reference: Option<Value>,
}

impl<'a> Site<'a> {
    /// One site from the export's files already parsed, which is the only way an
    /// adapter is ever handed one: a module reads no directory.
    pub fn from_files(files: &'a BTreeMap<String, Value>) -> Result<Site<'a>, String> {
        let absent: Vec<String> =
            EXPORT_FILES.iter().filter(|file| !files.contains_key(**file)).map(|file| (*file).to_string()).collect();

        let map = match files.get("map.json").and_then(|v| v.as_array()).map(Vec::as_slice) {
            Some([floor]) => floor.as_object(),
            _ => None,
        }
        .ok_or_else(|| "map.json is not a one-element array of objects".to_string())?;

        let floor_id = map
            .get("floor_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| "map.json states no floor_id that is a whole number".to_string())?;
        let cells = map
            .get("map_values")
            .and_then(Value::as_array)
            .ok_or_else(|| "map.json has no map_values array".to_string())?
            .iter()
            .map(read_cell)
            .collect::<Result<Vec<Cell>, String>>()?;
        let location_profiles = map
            .get("location_profiles")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(read_profile).collect::<Result<Vec<LocationProfile>, String>>())
            .transpose()?
            .unwrap_or_default();

        Ok(Site {
            vda5050_reference: map.get("vda5050_reference").cloned(),
            files,
            absent,
            floor_id,
            cells,
            location_profiles,
        })
    }

    /// Every key path every file carries, with how many times it occurs. Paths
    /// are written `file.json:a[].b`, and the array marker is flat — one entry
    /// per field family, not one per element.
    ///
    /// Every file that is present also gets an entry under its bare name, so the
    /// five files that are `[]` on every site seen — `elevator`,
    /// `fire_emergency`, `queue_data`, `opc_connection`, `zone_controller` — are
    /// still things the ledger has to have a disposition for. They name real
    /// organizational needs, and a file whose contents are empty today is not a
    /// file an importer may say nothing about.
    pub fn census(&self) -> BTreeMap<String, usize> {
        // Counted by hash and sorted once at the end, along one path that grows
        // and shrinks as the walk does: a large export visits millions of keys
        // and carries a few hundred distinct paths.
        let mut counts: FxHashMap<String, usize> = FxHashMap::default();
        let mut path = String::new();
        for (file, value) in self.files {
            count(&mut counts, file);
            path.clear();
            path.push_str(file);
            path.push(':');
            walk(&mut path, value, &mut counts);
        }
        counts.into_iter().collect()
    }

    /// Cells by their `coordinate` string, which is how every other file in the
    /// export points at one.
    pub fn by_coordinate(&self) -> BTreeMap<&str, &Cell> {
        self.cells.iter().map(|c| (c.coordinate.as_str(), c)).collect()
    }

    /// Cells by barcode — the export's alternate key, and the one `charger.json`,
    /// `ods_excluded.json`, `dock_point.json` and v2 stations use.
    pub fn by_barcode(&self) -> BTreeMap<&str, &Cell> {
        self.cells.iter().map(|c| (c.barcode.as_str(), c)).collect()
    }
}

fn count(counts: &mut FxHashMap<String, usize>, path: &str) {
    match counts.get_mut(path) {
        Some(seen) => *seen += 1,
        None => {
            counts.insert(path.to_string(), 1);
        }
    }
}

/// Count every key path below `path`, which the walk extends for each step
/// down and gives back as it was on the way out.
fn walk(path: &mut String, value: &Value, counts: &mut FxHashMap<String, usize>) {
    let normalized = path.trim_end_matches(':');
    if OPAQUE.contains(&normalized) {
        // An opaque path reached as an object key was already counted by the
        // parent that named it; one reached as a file root or an array element
        // was not, and is counted here so the coverage table still has to
        // dispose of it.
        if path.ends_with(':') || path.ends_with("[]") {
            count(counts, normalized);
        }
        return;
    }
    let stem = path.len();
    match value {
        Value::Object(map) => {
            let at_file_root = path.ends_with(':');
            for (key, sub) in map {
                if !at_file_root {
                    path.push('.');
                }
                push_key(path, key);
                count(counts, path);
                walk(path, sub, counts);
                path.truncate(stem);
            }
        }
        Value::Array(items) => {
            path.push_str("[]");
            for item in items {
                walk(path, item, counts);
            }
            path.truncate(stem);
        }
        _ => {}
    }
}

/// One object key onto a census path, each character the path itself spells
/// with — `.`, `[`, `]`, `:` and the backslash — escaped by a backslash, so a
/// key such as `"map_values[].barcode"` is a path of its own rather than the
/// covered one it spells, and is refused as a field family nobody has decided
/// about.
fn push_key(path: &mut String, key: &str) {
    if !key.contains(['.', '[', ']', ':', '\\']) {
        path.push_str(key);
        return;
    }
    for character in key.chars() {
        if matches!(character, '.' | '[' | ']' | ':' | '\\') {
            path.push('\\');
        }
        path.push(character);
    }
}

/// One profile's permissions, each bot type's an object of headings, empty
/// where the type is forbidden. A table or a bot's permissions of any other
/// JSON kind is refused, naming the profile and the bot: read as forbidden,
/// a role the export permits would leave `cell.traversable_by` on every cell
/// naming the profile, while the table itself rides back unchanged.
fn read_profile(value: &Value) -> Result<LocationProfile, String> {
    let object = value.as_object();
    let id = object.and_then(|o| o.get("location_profile_id")).and_then(Value::as_str).unwrap_or_default().to_string();
    let refuse = |what: String, stated: &Value| {
        format!(
            "map.json location profile {id:?} states {what} as {}, where the format keeps an object of headings, \
             empty where a bot type is forbidden: read as forbidding, it would take a permission the export states \
             off every cell naming the profile",
            kind_of(stated)
        )
    };
    let bots = match object.and_then(|o| o.get("bot_orientation_details")) {
        None => BTreeMap::new(),
        Some(Value::Object(details)) => details
            .iter()
            .map(|(bot, detail)| match detail.as_object() {
                Some(headings) => Ok((bot.clone(), !headings.is_empty())),
                None => Err(refuse(format!("bot type {bot:?}'s permissions"), detail)),
            })
            .collect::<Result<_, String>>()?,
        Some(other) => return Err(refuse("`bot_orientation_details`".to_string(), other)),
    };
    Ok(LocationProfile { id, bots })
}

/// The JSON kind of a value, for a sentence that should not quote a large one.
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

fn read_cell(value: &Value) -> Result<Cell, String> {
    let raw = value.as_object().ok_or_else(|| "a map_values entry of map.json is not an object".to_string())?.clone();
    let coordinate = raw
        .get("coordinate")
        .and_then(Value::as_str)
        .ok_or_else(|| "a cell of map.json has no coordinate string".to_string())?
        .to_string();
    let grid = bracket_pair(&coordinate).ok_or_else(|| {
        format!(
            "map.json cell {coordinate:?} states a `coordinate` that is not a grid pair spelled `[x,y]`, two whole \
             numbers in their shortest form no further than {MAX_WHOLE} from zero: every other file names a cell by \
             that spelling or by the pair it spells, and a cell spelled otherwise would be named by neither"
        )
    })?;
    let stated = raw
        .get("world_coordinate")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("map.json cell {coordinate} has no world_coordinate"))?;
    let world = millimetre_pair(stated).ok_or_else(|| {
        format!(
            "map.json cell {coordinate} states `world_coordinate` as {stated:?}, which is not a pair of millimetres"
        )
    })?;
    if !world.iter().all(|axis| axis.is_finite() && axis.abs() <= MAX_MM) {
        return Err(format!(
            "map.json cell {coordinate} states `world_coordinate` as {stated:?}, and a position is a finite number \
             of millimetres no further than {MAX_MM} from the origin on each axis, the most a site counting tenths \
             of a millimetre can count exactly"
        ));
    }
    let spelled = millimetre_spelling(world);
    if spelled != stated {
        return Err(format!(
            "map.json cell {coordinate} states `world_coordinate` as {stated:?}, which the export would write back \
             as {spelled:?}: it spells a position `[x,y]` with each number in its shortest form, so the file would \
             not come back as it went in"
        ));
    }
    // A vendor coordinate that is not two finite numbers of millimetres in
    // range measures nothing: it rides back as the file wrote it, and it is no
    // vote in the vendor frame's fit.
    let vda = raw
        .get("vda_world_coordinate")
        .and_then(Value::as_str)
        .and_then(millimetre_pair)
        .filter(|pair| pair.iter().all(|axis| axis.is_finite() && axis.abs() <= MAX_MM));
    let barcode = raw
        .get("barcode")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("map.json cell {coordinate} has no barcode"))?
        .to_string();

    let size_info = match raw.get("size_info") {
        Some(stated) => Some(half_extents(stated).map_err(|why| format!("map.json cell {coordinate} states {why}"))?),
        None => None,
    };
    if size_info.iter().flatten().any(|half| half.abs() > MAX_MM / 2.0) {
        return Err(format!(
            "map.json cell {coordinate} states a `size_info` half-extent beyond {}, and a footprint is at most \
             {MAX_MM} mm across, the most a site counting tenths of a millimetre can count exactly",
            MAX_MM / 2.0
        ));
    }

    let adjacency =
        adjacency(raw.get("adjacency")).map_err(|why| format!("map.json cell {coordinate} states {why}"))?;
    let neighbours = match raw.get("neighbours") {
        Some(stated) => grades(stated).map_err(|why| format!("map.json cell {coordinate} states {why}"))?,
        // A cell naming a neighbour states the grades its travel is read off;
        // read as all zero, a connection the export states would vanish.
        None if adjacency.iter().any(Option::is_some) => {
            return Err(format!(
                "map.json cell {coordinate} names a neighbour in `adjacency` and states no `neighbours`, where the \
                 format keeps the grades travel to each neighbour is read off: read as zero, the connection would \
                 be missing from the graph"
            ))
        }
        None => [[0; 3]; 4],
    };

    Ok(Cell {
        coordinate,
        grid,
        world,
        vda,
        barcode,
        size_info,
        blocked: raw.get("blocked").and_then(Value::as_bool).unwrap_or(false),
        zone: raw.get("zone").and_then(Value::as_str).map(str::to_string),
        sector: raw.get("sector").map(scalar_string),
        block: raw.get("block").and_then(Value::as_i64),
        location_profile_id: raw.get("location_profile_id").and_then(Value::as_str).map(str::to_string),
        adjacency,
        neighbours,
        raw,
    })
}

/// The four neighbours an `adjacency` states, one slot per direction: `null`
/// for none, or the neighbour's grid pair. A cell stating no `adjacency` names
/// none. Any other shape is refused, since read as no neighbour it would drop
/// a connection the export states while the field rides back unchanged.
fn adjacency(stated: Option<&Value>) -> Result<[Option<[i64; 2]>; 4], String> {
    let mut slots = [None; 4];
    let Some(stated) = stated else { return Ok(slots) };
    let entries = four(stated, "adjacency", "a neighbour's grid pair or null")?;
    for (at, entry) in entries.iter().enumerate() {
        if entry.is_null() {
            continue;
        }
        slots[at] = Some(whole_pair(entry).ok_or_else(|| {
            format!(
                "`adjacency` entry {} as {entry}, where the format keeps null or a grid pair of two whole numbers: \
                 read as no neighbour, the connection would be missing from the graph",
                at + 1
            )
        })?);
    }
    Ok(slots)
}

/// The grades a `neighbours` states, one triple per direction, each `[neighbour
/// exists, bot may move, bot with load may move]` and each grade 0 or 1. Any
/// other shape is refused, since a grade read as zero denies travel the export
/// grants.
fn grades(stated: &Value) -> Result<[[i64; 3]; 4], String> {
    let entries = four(stated, "neighbours", "a triple of grades")?;
    let mut grades = [[0; 3]; 4];
    for (at, entry) in entries.iter().enumerate() {
        let triple = entry.as_array().filter(|triple| triple.len() == 3);
        let read = triple.and_then(|triple| {
            let mut out = [0; 3];
            for (slot, grade) in triple.iter().enumerate() {
                out[slot] = whole_number(grade).filter(|grade| *grade == 0 || *grade == 1)?;
            }
            Some(out)
        });
        grades[at] = read.ok_or_else(|| {
            format!(
                "`neighbours` entry {} as {entry}, where the format keeps a triple of grades, each 0 or 1: read \
                 otherwise, travel to that neighbour would be granted or denied where the export says otherwise",
                at + 1
            )
        })?;
    }
    Ok(grades)
}

/// A per-direction list as exactly four entries, one per direction, or what
/// keeps it from being one.
fn four<'v>(stated: &'v Value, field: &str, entry: &str) -> Result<&'v [Value], String> {
    match stated.as_array() {
        Some(entries) if entries.len() == 4 => Ok(entries),
        Some(entries) => Err(format!(
            "`{field}` with {} entries, {stated}, where the format keeps four, one per direction, each {entry}: read \
             {}, a direction's connection would be one the export does not state",
            entries.len(),
            if entries.len() < 4 { "with nothing for each it lacks" } else { "as its first four" }
        )),
        None => Err(format!(
            "`{field}` as {stated}, where the format keeps a list of four entries, one per direction, each {entry}"
        )),
    }
}

/// The four half-extents a `size_info` states, or what keeps it from stating
/// them: no list, a list of another length, or an entry that is no number. Read
/// with a zero for what it lacked, or without what it added, the cell's
/// footprint, which is summed from the four, would be one the export does not
/// state.
fn half_extents(stated: &Value) -> Result<[f64; 4], String> {
    let entries = stated.as_array().ok_or_else(|| {
        format!(
            "`size_info` as {stated}, where the format keeps a list of four half-extents: read as stating none, the \
             cell could not be told from one whose export states no `size_info`"
        )
    })?;
    if entries.len() != 4 {
        return Err(format!(
            "`size_info` with {} entries, {stated}, where the format keeps four half-extents: read {}, the cell's \
             footprint would be one the export does not state",
            entries.len(),
            if entries.len() < 4 { "with a zero for each it lacks" } else { "as its first four" }
        ));
    }
    let mut halves = [0.0; 4];
    for (at, entry) in entries.iter().enumerate() {
        halves[at] = entry.as_f64().filter(|half| half.is_finite()).ok_or_else(|| {
            format!(
                "`size_info` entry {} as {entry}, where the format keeps a number of millimetres: read as zero, the \
                 cell's footprint would be one the export does not state",
                at + 1
            )
        })?;
    }
    Ok(halves)
}

/// The furthest a length may lie from zero, in millimetres: a length counts
/// steps of as little as a tenth of a millimetre, and a step count travels as
/// a JSON number, which consumers read as a double, exact to 2^53 − 1.
pub const MAX_MM: f64 = 9_007_199_254_740_991.0 / 10.0;

/// A millimetre position spelled as the export spells one: `"[x,y]"`, each
/// number in its shortest form, a whole number without a fraction.
pub fn millimetre_spelling(pair: [f64; 2]) -> String {
    let axis = |mm: f64| if mm.fract() == 0.0 { format!("{}", mm as i64) } else { format!("{mm}") };
    format!("[{},{}]", axis(pair[0]), axis(pair[1]))
}

/// `"[7,1]"` — the export's most common coordinate spelling, and the only one
/// it writes: two integers in their shortest form, no further than
/// [`MAX_WHOLE`] from zero. Any other spelling is no grid pair, since a pair
/// read out of it would be written back spelled otherwise, or as another pair.
pub fn bracket_pair(text: &str) -> Option<[i64; 2]> {
    let inner = text.strip_prefix('[')?.strip_suffix(']')?;
    let (x, y) = inner.split_once(',')?;
    let pair = [x.parse::<i64>().ok()?, y.parse::<i64>().ok()?];
    let in_range = pair.iter().all(|index| index.unsigned_abs() <= MAX_WHOLE as u64);
    (in_range && coordinate_key(pair) == text).then_some(pair)
}

/// `"[11475.3,-3825]"` — a measured position in millimetres, which unlike a grid
/// coordinate need not be a whole number: a site that measures in tenths of a
/// millimetre states tenths, and reading them as whole numbers would move every
/// cell.
pub fn millimetre_pair(text: &str) -> Option<[f64; 2]> {
    let inner = text.trim().trim_start_matches('[').trim_end_matches(']');
    let mut parts = inner.split(',');
    let x = parts.next()?.trim().parse::<f64>().ok()?;
    let y = parts.next()?.trim().parse::<f64>().ok()?;
    parts.next().is_none().then_some([x, y])
}

/// `[7, 1]` — the raw-array spelling `block.json`, `pps.json` v3 and
/// `vtm_zone.json` use for the same thing. A reference names a cell only spelled
/// as the export writes the pair back, two integers, so `[7.0, 1]` names none
/// and is kept as the file wrote it.
pub fn raw_pair(value: &Value) -> Option<[i64; 2]> {
    let items = value.as_array()?;
    if items.len() != 2 {
        return None;
    }
    Some([items[0].as_i64()?, items[1].as_i64()?])
}

/// A grid pair read as two whole numbers in either JSON spelling, `[7, 1]` or
/// `[7.0, 1]`: how an `adjacency` entry is read, since the connection is
/// derived from it while the entry itself rides back as the file wrote it.
fn whole_pair(value: &Value) -> Option<[i64; 2]> {
    match value.as_array()?.as_slice() {
        [x, y] => Some([whole_number(x)?, whole_number(y)?]),
        _ => None,
    }
}

/// The furthest a whole number the export counts with — a grid index, a grade
/// — may lie from zero: it travels as a JSON number, which consumers read as a
/// double, exact to 2^53 − 1.
pub const MAX_WHOLE: i64 = 9_007_199_254_740_991;

/// A whole number in either JSON spelling the export writes one in: an
/// integer, `1`, or a number with a decimal point whose value is whole, `1.0`.
/// Both are the same number, so both read the same; a fraction, or a number
/// beyond [`MAX_WHOLE`], is none.
pub fn whole_number(value: &Value) -> Option<i64> {
    let whole = match value.as_i64() {
        Some(integer) => integer,
        None => {
            let float = value.as_f64().filter(|float| float.fract() == 0.0 && float.abs() <= MAX_WHOLE as f64)?;
            float as i64
        }
    };
    (whole.unsigned_abs() <= MAX_WHOLE as u64).then_some(whole)
}

/// A coordinate key in the export's canonical `"[x,y]"` spelling, whichever
/// spelling it arrived in.
pub fn coordinate_key(pair: [i64; 2]) -> String {
    format!("[{},{}]", pair[0], pair[1])
}

/// A JSON scalar as the string the export would print, so `"undefined"` and 12
/// are both quotable in a ledger without losing which one it was.
fn scalar_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use super::Site;

    use super::{bracket_pair, coordinate_key, millimetre_pair, raw_pair};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn the_three_coordinate_spellings_read_the_same_pair() {
        assert_eq!(bracket_pair("[7,1]"), Some([7, 1]));
        assert_eq!(raw_pair(&json!([7, 1])), Some([7, 1]));
        assert_eq!(coordinate_key([7, 1]), "[7,1]");
    }

    /// A measured position keeps its tenths, where a grid coordinate is a whole
    /// number.
    #[test]
    fn a_millimetre_position_keeps_its_fraction() {
        assert_eq!(millimetre_pair("[2250.8,-1750.6]"), Some([2250.8, -1750.6]));
        assert_eq!(millimetre_pair("[2250,1750]"), Some([2250.0, 1750.0]));
        assert_eq!(millimetre_pair("[1,2,3]"), None);
    }

    /// A position is spelled as the export spells it back: whole numbers bare,
    /// fractions in their shortest form, the sign of zero gone.
    #[test]
    fn a_millimetre_position_is_spelled_as_the_export_spells_it() {
        use super::millimetre_spelling;
        assert_eq!(millimetre_spelling([28750.0, -6250.0]), "[28750,-6250]");
        assert_eq!(millimetre_spelling([2250.8, -0.0]), "[2250.8,0]");
        assert_eq!(millimetre_spelling([0.1, 1e3]), "[0.1,1000]");
    }

    /// A `size_info` is four numbers or it is refused, naming what it is
    /// instead: never read with zeros for what it lacks.
    #[test]
    fn a_size_info_is_four_numbers_or_refused() {
        use super::half_extents;
        assert_eq!(half_extents(&json!([600, 650, 625.5, 625])), Ok([600.0, 650.0, 625.5, 625.0]));
        for (stated, says) in [
            (json!([625, 625, 625]), "with 3 entries, [625,625,625], where the format keeps four"),
            (json!([625, 625, 625, 625, 625]), "with 5 entries"),
            (json!([625, "bad", 625, 625]), "entry 2 as \"bad\", where the format keeps a number"),
            (json!([625, 625, null, 625]), "entry 3 as null"),
            (json!(null), "as null, where the format keeps a list of four half-extents"),
            (json!("625,625,625,625"), "as \"625,625,625,625\", where the format keeps a list"),
        ] {
            let refused = half_extents(&stated).expect_err("a refusal");
            assert!(refused.contains(says), "{says:?}: {refused}");
        }
    }

    /// A whole number reads the same spelled `1` or `1.0`, and a fraction, a
    /// number past 2^53 − 1 or something that is no number reads as none.
    #[test]
    fn a_whole_number_reads_the_same_in_both_spellings() {
        use super::{whole_number, MAX_WHOLE};
        assert_eq!(whole_number(&json!(1)), Some(1));
        assert_eq!(whole_number(&json!(1.0)), Some(1));
        assert_eq!(whole_number(&json!(-3.0)), Some(-3));
        assert_eq!(whole_number(&json!(MAX_WHOLE)), Some(MAX_WHOLE));
        for none in [json!(1.5), json!(MAX_WHOLE + 1), json!(1e300), json!(u64::MAX), json!("1"), json!(null)] {
            assert_eq!(whole_number(&none), None, "{none}");
        }
        assert_eq!(super::whole_pair(&json!([1.0, 2])), Some([1, 2]));
        assert_eq!(super::whole_pair(&json!([1.5, 2])), None);
        // A reference is not read so: it names a cell spelled as the export
        // writes one back, or it is kept as the file wrote it.
        assert_eq!(raw_pair(&json!([1.0, 2])), None);
    }

    /// An `adjacency` and a `neighbours` are four entries each, an adjacency
    /// entry null or a grid pair and a grade triple three grades of 0 or 1,
    /// whole numbers in either spelling; any other shape is refused naming the
    /// entry, and a cell naming a neighbour without grades is refused.
    #[test]
    fn an_adjacency_and_its_grades_are_four_entries_of_their_shape_or_refused() {
        use super::read_cell;
        let cell = |adjacency: Value, neighbours: Value| {
            let mut cell = json!({ "coordinate": "[1,1]", "world_coordinate": "[0,0]", "barcode": "B" });
            if !adjacency.is_null() {
                cell["adjacency"] = adjacency;
            }
            if !neighbours.is_null() {
                cell["neighbours"] = neighbours;
            }
            read_cell(&cell)
        };
        let grades = json!([[1, 1, 1], [1.0, 1, 0], [0, 0, 0], [0, 0, 0]]);
        let read = cell(json!([[1, 0], [2.0, 1], null, null]), grades.clone()).expect("a cell");
        assert_eq!(read.adjacency, [Some([1, 0]), Some([2, 1]), None, None]);
        assert_eq!(read.neighbours, [[1, 1, 1], [1, 1, 0], [0, 0, 0], [0, 0, 0]]);
        for (adjacency, neighbours, says) in [
            (json!([[1, 0], [2.5, 1], null, null]), grades.clone(), "`adjacency` entry 2 as [2.5,1]"),
            (json!([[1, 0], "[2,1]", null, null]), grades.clone(), "`adjacency` entry 2 as \"[2,1]\""),
            (json!([[1, 0], null, null]), grades.clone(), "`adjacency` with 3 entries"),
            (json!({ "0": [1, 0] }), grades.clone(), "`adjacency` as {\"0\":[1,0]}, where the format keeps a list"),
            (
                json!([[1, 0], null, null, null]),
                json!([[1, 1, 1], [0, 0, 0], [0, 0, 0]]),
                "`neighbours` with 3 entries",
            ),
            (json!([[1, 0], null, null, null]), json!([[1, 1], [0, 0, 0], [0, 0, 0], [0, 0, 0]]), "entry 1 as [1,1]"),
            (
                json!([[1, 0], null, null, null]),
                json!([[1, 2, 1], [0, 0, 0], [0, 0, 0], [0, 0, 0]]),
                "entry 1 as [1,2,1]",
            ),
            (json!([[1, 0], null, null, null]), json!(null), "names a neighbour in `adjacency` and states no"),
        ] {
            let refused = cell(adjacency, neighbours).expect_err("a refusal");
            assert!(refused.starts_with("map.json cell [1,1] "), "{refused}");
            assert!(refused.contains(says), "{says:?}: {refused}");
        }
        assert_eq!(cell(json!(null), json!(null)).expect("a cell").adjacency, [None; 4]);
    }

    /// A grid coordinate is read only in the one spelling the export writes,
    /// so the pair read out of it is written back as the very string: no
    /// whitespace, sign of zero, leading zero, decimal point or fraction, and
    /// nothing a double could not hold exactly.
    #[test]
    fn a_grid_coordinate_is_read_only_in_the_spelling_the_export_writes() {
        assert_eq!(bracket_pair("[-7,0]"), Some([-7, 0]));
        assert_eq!(bracket_pair("[9007199254740991,1]"), Some([9_007_199_254_740_991, 1]));
        for other in [
            "7,1",
            "[7, 1]",
            " [7,1]",
            "[-0,1]",
            "[07,1]",
            "[7.0,1]",
            "[7.5,1]",
            "[NaN,1]",
            "[9007199254740993,1]",
            "[1e3,1]",
            "[+7,1]",
        ] {
            assert_eq!(bracket_pair(other), None, "{other}");
        }
    }

    /// A key spelling a path names a field family of its own, never the
    /// covered one it spells.
    #[test]
    fn a_key_spelling_a_census_path_is_a_path_of_its_own() {
        let files: BTreeMap<String, Value> = [(
            "zone.json".to_string(),
            json!([{ "zonerec": { "zone_id": "A" }, "zonerec.zone_id": "B", "a:b\\c": 1 }]),
        )]
        .into_iter()
        .collect();
        let site = Site {
            files: &files,
            absent: vec![],
            floor_id: 1,
            cells: vec![],
            location_profiles: vec![],
            vda5050_reference: None,
        };
        let census = site.census();
        assert_eq!(census.get("zone.json:[].zonerec.zone_id"), Some(&1));
        assert_eq!(census.get("zone.json:[].zonerec\\.zone_id"), Some(&1));
        assert_eq!(census.get("zone.json:[].a\\:b\\\\c"), Some(&1));
    }

    #[test]
    fn a_triple_is_not_a_coordinate() {
        assert_eq!(bracket_pair("[7,1,3]"), None);
        assert_eq!(raw_pair(&json!([7, 1, 3])), None);
    }

    /// The escape hatch and the re-init diagonal are ordinary pairs as far as
    /// parsing goes; what they mean is `Cell`'s business.
    #[test]
    fn the_magic_coordinates_parse_as_written() {
        assert_eq!(bracket_pair("[999,350]"), Some([999, 350]));
        assert_eq!(bracket_pair("[500,500]"), Some([500, 500]));
    }
}
