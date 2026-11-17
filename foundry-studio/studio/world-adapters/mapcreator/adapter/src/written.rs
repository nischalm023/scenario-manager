//! Within the Map Creator adapter, `written` is **how an export's files spell
//! their JSON beyond the values in it**: the order each object states its keys
//! in, the separators between items and between a key and its value, the
//! whitespace around a file's value, a whole number written with a decimal
//! point (`40.0`), and a zero written with a minus sign (`-0.0`). JSON gives
//! none of these a meaning and a parse forgets every one of them, while the
//! round trip owes each file back byte for byte; so the import records each as
//! a fact and the projection writes from the facts.
//!
//! A fact lives on the entity the object it is about is. An object inside a
//! record is the record's: its key order rides in the record's entry of
//! `import_formatting`, never in a component that states facts about the site,
//! and only where it departs from the order its file usually gives an object
//! with the same keys in the same place, which the export entity records once
//! per place and set of keys. Everything outside every record — a file's top
//! level, an envelope, the sector index — is the export entity's, as are a
//! file's separators and whitespace.
//!
//! A place is where an object stands in its file with the list positions left
//! out (`map.json[].map_values[]` is every cell). A pointer is where one value
//! stands, spelled as RFC 6901 spells one: within a record, from the record's
//! own object; on the export entity, after the file's name.

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt;

/// The places a record's own object stands at, one per record kind. The facts
/// of an object at one of these, and of everything inside it, are that
/// record's; a walk from anywhere else stops at them.
pub const RECORD_PLACES: [&str; 19] = [
    "map.json[].map_values[]",
    "map.json[].location_profiles[]",
    "zone.json[]",
    "vtm_zone.json.aisle_info[]",
    "block.json[]",
    "pps.json[]",
    "pps.json[].entities.presentation_points[]",
    "pps.json[].pps_points[]",
    "charger.json[]",
    "dock_point.json[]",
    "ods_excluded.json.ods_excluded_list[]",
    "tote_storage.json[]",
    "tote_type.json[]",
    "conveyor.json.conveyor_data[]",
    "conveyor_mapping.json[]",
    "pps_bin.json.data[]",
    "pps_bintype.json.data[]",
    "pps_seat.json.pps_seats[]",
    "pptl.json[].pptls[]",
];

/// The component a record's formatting rides in, on the entity the record is:
/// how the file it was imported from wrote it, apart from the components that
/// state facts about the site. One entry per record, keyed by the lens its
/// kind is written back through (`mapcreator_cell`), so the formatting of two
/// records an entity carries facts from never mixes, and neither does two
/// importers', whose lenses are named apart.
pub const COMPONENT: &str = "import_formatting";

/// The key a sector's entry takes: its place in the sector index, and the JSON
/// kind the cells spell its key in.
pub const SECTOR: &str = "mapcreator_sector";

/// The key a sector preference's entry takes: its place in
/// `sectorMxUPreferences.json`.
pub const SECTOR_PREFERENCE: &str = "mapcreator_sector_preference";

/// Every key this adapter writes an entry of [`COMPONENT`] under: one per
/// record kind, the sector index's and the preference table's.
pub fn keys() -> impl Iterator<Item = &'static str> {
    crate::records::KINDS.iter().map(|kind| kind.lens).chain([SECTOR, SECTOR_PREFERENCE])
}

/// The key one construct's records take their entry under, which holds the
/// record's ordering key: a record kind's lens, the sector index's or the
/// preference table's. A construct that orders nothing — a frame, the export
/// header — has none, and no other entry's `index` is the record's, whoever
/// wrote it.
pub fn key_of(construct: &str) -> Option<&'static str> {
    match construct {
        "sector" => Some(SECTOR),
        "sector_preference" => Some(SECTOR_PREFERENCE),
        _ => crate::records::kind(construct).map(|kind| kind.lens),
    }
}

/// One record's entry, out of the [`COMPONENT`] an entity carries: empty where
/// it carries none, or none under `key`.
pub fn entry(formatting: Option<&Value>, key: &str) -> Map<String, Value> {
    formatting.and_then(|all| all.get(key)).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// The [`COMPONENT`] one record's entry makes.
pub fn component(key: &str, entry: Map<String, Value>) -> Value {
    let mut out = Map::new();
    out.insert(key.to_string(), Value::Object(entry));
    Value::Object(out)
}

/// One file's JSON as its text states it: every object's keys in the order the
/// file wrote them, which `serde_json`'s map forgets, and every number as the
/// kind it was written as.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Object(Vec<(String, Shape)>),
    Array(Vec<Shape>),
    Scalar(Value),
    /// A number as these very characters, which no `Value` holds: a whole
    /// number past what a double holds exactly, written with a decimal point.
    /// An ingest reads one where a file wrote one ([`read_whole_digits`]) and
    /// the projection writes one where the site holds such a number at a place
    /// its file wrote with a point.
    Spelled(String),
}

impl Shape {
    /// The value one pointer names, spelled as RFC 6901 spells one.
    pub fn at(&self, pointer: &str) -> Option<&Shape> {
        let mut here = self;
        for step in pointer.split('/').skip(1) {
            let step = step.replace("~1", "/").replace("~0", "~");
            here = match here {
                Shape::Object(entries) => &entries.iter().find(|(key, _)| *key == step)?.1,
                Shape::Array(items) => items.get(step.parse::<usize>().ok()?)?,
                Shape::Scalar(_) | Shape::Spelled(_) => return None,
            };
        }
        Some(here)
    }

    /// The keys an object states, in its order.
    fn keys(entries: &[(String, Shape)]) -> Vec<&str> {
        entries.iter().map(|(key, _)| key.as_str()).collect()
    }
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Shape, D::Error> {
        deserializer.deserialize_any(ShapeVisitor)
    }
}

struct ShapeVisitor;

impl<'de> Visitor<'de> for ShapeVisitor {
    type Value = Shape;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Shape, E> {
        Ok(Shape::Scalar(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Shape, E> {
        Ok(Shape::Scalar(Value::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Shape, E> {
        Ok(Shape::Scalar(Value::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Shape, E> {
        Ok(Shape::Scalar(Number::from_f64(value).map_or(Value::Null, Value::Number)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Shape, E> {
        Ok(Shape::Scalar(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> Result<Shape, E> {
        Ok(Shape::Scalar(Value::String(value)))
    }

    fn visit_unit<E>(self) -> Result<Shape, E> {
        Ok(Shape::Scalar(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Shape, A::Error> {
        let mut out = Vec::with_capacity(items.size_hint().unwrap_or(0));
        while let Some(item) = items.next_element()? {
            out.push(item);
        }
        Ok(Shape::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Shape, A::Error> {
        let mut out = Vec::new();
        while let Some((key, value)) = entries.next_entry()? {
            out.push((key, value));
        }
        Ok(Shape::Object(out))
    }
}

/// 2^53: every whole number smaller in magnitude is a double exactly.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// Read each whole number a file wrote with a decimal point and a double does
/// not hold exactly, `9007199254740993.0`, by its own digits. A parse reads it
/// as the nearest double, which is neither the number written nor one that
/// writes back as it was, so an export the projection wrote such a number into
/// would be refused when it came back. The shape keeps the characters, and
/// `value`, the same file as a `Value`, takes the integer where 64 bits hold
/// it; past them the number stays the double, and [`Spacing::of`] refuses the
/// file, naming it.
pub fn read_whole_digits(bytes: &[u8], shape: &mut Shape, value: &mut Value) {
    // Almost no file holds a large double, so almost no file is scanned.
    if holds_large(shape) {
        respell(shape, &mut number_spans(bytes), bytes, &mut String::new(), value);
    }
}

/// Whether a number was read as a double of 2^53 or more in magnitude: only
/// such a double can be the nearest to a whole number a double does not hold.
fn large(number: &Number) -> bool {
    number.is_f64() && number.as_f64().is_some_and(|double| double.abs() >= EXACT)
}

fn holds_large(shape: &Shape) -> bool {
    match shape {
        Shape::Object(entries) => entries.iter().any(|(_, child)| holds_large(child)),
        Shape::Array(items) => items.iter().any(holds_large),
        Shape::Scalar(Value::Number(number)) => large(number),
        Shape::Scalar(_) | Shape::Spelled(_) => false,
    }
}

/// Walk a file's shape in the order its text states it, taking each number's
/// span in the text from `numbers`, and respell each whole number past a
/// double there and at its pointer in `value`.
fn respell(
    shape: &mut Shape,
    numbers: &mut impl Iterator<Item = (usize, usize)>,
    text: &[u8],
    pointer: &mut String,
    value: &mut Value,
) {
    match shape {
        Shape::Object(entries) => {
            for (key, child) in entries {
                let at = pointer.len();
                pointer.push('/');
                escape(key, pointer);
                respell(child, numbers, text, pointer, value);
                pointer.truncate(at);
            }
        }
        Shape::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let at = pointer.len();
                pointer.push('/');
                pointer.push_str(&index.to_string());
                respell(item, numbers, text, pointer, value);
                pointer.truncate(at);
            }
        }
        Shape::Scalar(Value::Number(number)) => {
            let Some((from, to)) = numbers.next() else { return };
            if let Some(whole) = large(number).then(|| whole_past_a_double(&text[from..to])).flatten() {
                if let Some(slot) = value.pointer_mut(pointer) {
                    *slot = whole;
                }
                *shape = Shape::Spelled(String::from_utf8_lossy(&text[from..to]).into_owned());
            }
        }
        Shape::Scalar(_) | Shape::Spelled(_) => {}
    }
}

/// The integer a number token spells as `-?digits.0`, where a double does not
/// hold it exactly and 64 bits do.
fn whole_past_a_double(token: &[u8]) -> Option<Value> {
    let whole: i128 = std::str::from_utf8(token.strip_suffix(b".0")?).ok()?.parse().ok()?;
    if whole as f64 as i128 == whole {
        return None;
    }
    i64::try_from(whole).map(Value::from).or_else(|_| u64::try_from(whole).map(Value::from)).ok()
}

/// The spans of a JSON text's numbers, in the order the text states them: a
/// key is a string, so every number in the text is a value.
fn number_spans(text: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut at = 0;
    std::iter::from_fn(move || {
        while at < text.len() {
            match text[at] {
                b'"' => {
                    at += 1;
                    while at < text.len() && text[at] != b'"' {
                        at += if text[at] == b'\\' { 2 } else { 1 };
                    }
                    at += 1;
                }
                b'-' | b'0'..=b'9' => {
                    let from = at;
                    while at < text.len() && matches!(text[at], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                        at += 1;
                    }
                    return Some((from, at));
                }
                _ => at += 1,
            }
        }
        None
    })
}

/// For every place, the orders its objects state their keys in: one per set of
/// keys some object there states, the established ones first (see
/// [`usual_orders`]) and then the sets met most often first. A place whose
/// objects state fewer than two keys has no order to record and is not here.
// ordered: written onto the export entity as JSON, so the same files give the
// same component.
pub type Usual = BTreeMap<String, Vec<Vec<String>>>;

/// The usual orders of every place of every file of an export.
///
/// A set of keys the site's export entity already gives an order for, at a
/// place where some object still states that set, keeps that order: it is
/// established, and the records whose own order departs from it say so. Only
/// a set no order is established for takes the order most of the objects
/// stating it state, a tie to the one met first. So a record removed or
/// reordered never moves the order under records nobody touched, which would
/// patch them; a set no object states any more is dropped. A site holding no
/// orders, a first ingest's, takes every set's from the files.
pub fn usual_orders(files: &BTreeMap<String, Shape>, established: Option<&Usual>) -> Usual {
    let stated = stated_orders(files);
    let Some(established) = established else { return stated };
    fn set(order: &Vec<String>) -> Vec<&str> {
        let mut set: Vec<&str> = order.iter().map(String::as_str).collect();
        set.sort_unstable();
        set
    }
    stated
        .into_iter()
        .map(|(place, orders)| {
            let sets: Vec<Vec<&str>> = orders.iter().map(set).collect();
            let mut usual: Vec<Vec<String>> = established
                .get(&place)
                .into_iter()
                .flatten()
                .filter(|order| sets.contains(&set(order)))
                .cloned()
                .collect();
            let kept: Vec<Vec<&str>> = usual.iter().map(set).collect();
            let fresh: Vec<Vec<String>> = orders.iter().filter(|order| !kept.contains(&set(order))).cloned().collect();
            usual.extend(fresh);
            (place, usual)
        })
        .collect()
}

/// The order most of the objects stating each set of keys at each place state
/// it in, the sets met most often first.
fn stated_orders(files: &BTreeMap<String, Shape>) -> Usual {
    // Per place, each distinct order with how many objects state it and where
    // the first one stood, so that a tie goes to the one met first.
    let mut seen: BTreeMap<String, BTreeMap<Vec<&str>, (usize, usize)>> = BTreeMap::new();
    let mut met = 0usize;
    for (name, shape) in files {
        count(shape, &mut name.clone(), &mut seen, &mut met);
    }
    let mut usual = Usual::new();
    for (place, orders) in seen {
        // Each set of keys: how many objects state it and where the first
        // stood, and its most stated order with the same two numbers.
        let mut sets: BTreeMap<Vec<&str>, ((usize, Reverse<usize>), (usize, Reverse<usize>), Vec<&str>)> =
            BTreeMap::new();
        for (order, (count, first)) in orders {
            let mut set = order.clone();
            set.sort_unstable();
            let (whole, best, chosen) =
                sets.entry(set).or_insert(((0, Reverse(first)), (0, Reverse(first)), Vec::new()));
            *whole = (whole.0 + count, Reverse(whole.1 .0.min(first)));
            if (count, Reverse(first)) > *best {
                *best = (count, Reverse(first));
                *chosen = order;
            }
        }
        let mut entries: Vec<((usize, Reverse<usize>), Vec<&str>)> =
            sets.into_values().map(|(whole, _, order)| (whole, order)).collect();
        // ordered: the set most objects state first, a tie to the one met first.
        entries.sort_by(|one, other| other.0.cmp(&one.0));
        let entries = entries.into_iter().map(|(_, order)| order.into_iter().map(str::to_string).collect()).collect();
        usual.insert(place, entries);
    }
    usual
}

fn count<'s>(
    shape: &'s Shape,
    place: &mut String,
    seen: &mut BTreeMap<String, BTreeMap<Vec<&'s str>, (usize, usize)>>,
    met: &mut usize,
) {
    match shape {
        Shape::Object(entries) => {
            if entries.len() >= 2 {
                let orders = seen.entry(place.clone()).or_default();
                let entry = orders.entry(Shape::keys(entries)).or_insert((0, *met));
                entry.0 += 1;
                *met += 1;
            }
            for (key, child) in entries {
                let at = place.len();
                place.push('.');
                place.push_str(key);
                count(child, place, seen, met);
                place.truncate(at);
            }
        }
        Shape::Array(items) => {
            let at = place.len();
            place.push_str("[]");
            for item in items {
                count(item, place, seen, met);
            }
            place.truncate(at);
        }
        Shape::Scalar(_) | Shape::Spelled(_) => {}
    }
}

/// The order the file usually gives an object at `place` stating `keys`: the
/// one recorded for exactly that set, or for an object whose set no file of the
/// export stated — another tool's edit — the place's first order.
fn expected<'u>(usual: &'u Usual, place: &str, keys: &[&str]) -> Option<&'u [String]> {
    let entries = usual.get(place)?;
    entries
        .iter()
        .find(|order| order.len() == keys.len() && order.iter().all(|key| keys.contains(&key.as_str())))
        .or_else(|| entries.first())
        .map(Vec::as_slice)
}

/// The three facts a record's text states and its values do not: where an
/// object's keys depart from their usual order, the order the file wrote them
/// in; where a whole number was written with a decimal point; and where a zero
/// was written with a minus sign, which a reader holds as the zero without one.
/// Each is keyed by a pointer from the record's own object.
#[derive(Debug, Default, PartialEq)]
pub struct Facts {
    pub key_order: Map<String, Value>,
    pub with_point: Vec<Value>,
    pub negative_zero: Vec<Value>,
}

impl Facts {
    /// The facts of one record's own object, standing at `place`, and of
    /// everything inside it but the records nested in it.
    pub fn of_record(record: &Shape, place: &str, usual: &Usual) -> Facts {
        let mut facts = Facts::default();
        facts.walk(record, &mut place.to_string(), &mut String::new(), usual, true);
        facts
    }

    /// The facts of everything in one file outside its records, each keyed by
    /// the file's name and its pointer in the file.
    pub fn outside_records(name: &str, file: &Shape, usual: &Usual, into: &mut Facts) {
        into.walk(file, &mut name.to_string(), &mut name.to_string(), usual, true);
    }

    fn walk(&mut self, shape: &Shape, place: &mut String, pointer: &mut String, usual: &Usual, start: bool) {
        match shape {
            Shape::Object(entries) => {
                if !start && RECORD_PLACES.contains(&place.as_str()) {
                    return;
                }
                if entries.len() >= 2 {
                    let keys = Shape::keys(entries);
                    let usual = expected(usual, place, &keys);
                    if usual.is_none_or(|order| !order.iter().map(String::as_str).eq(keys.iter().copied())) {
                        self.key_order.insert(pointer.clone(), Value::from(keys));
                    }
                }
                for (key, child) in entries {
                    let (place_at, pointer_at) = (place.len(), pointer.len());
                    place.push('.');
                    place.push_str(key);
                    pointer.push('/');
                    escape(key, pointer);
                    self.walk(child, place, pointer, usual, false);
                    place.truncate(place_at);
                    pointer.truncate(pointer_at);
                }
            }
            Shape::Array(items) => {
                let (place_at, pointer_at) = (place.len(), pointer.len());
                place.push_str("[]");
                for (at, item) in items.iter().enumerate() {
                    pointer.push('/');
                    pointer.push_str(&at.to_string());
                    self.walk(item, place, pointer, usual, false);
                    pointer.truncate(pointer_at);
                }
                place.truncate(place_at);
            }
            Shape::Scalar(Value::Number(number)) => {
                if number.is_f64() && number.as_f64().is_some_and(|float| float.fract() == 0.0) {
                    self.with_point.push(Value::String(pointer.clone()));
                }
                if number.as_f64().is_some_and(|float| float == 0.0 && float.is_sign_negative()) {
                    self.negative_zero.push(Value::String(pointer.clone()));
                }
            }
            Shape::Spelled(_) => self.with_point.push(Value::String(pointer.clone())),
            Shape::Scalar(_) => {}
        }
    }

    /// Leave out the facts of what stands at `pointer` and inside it: a part of
    /// a file that a record's own facts state, such as a preference's list.
    pub fn without(&mut self, pointer: &str) {
        let inside = |at: &str| at == pointer || at.strip_prefix(pointer).is_some_and(|rest| rest.starts_with('/'));
        self.key_order.retain(|at, _| !inside(at));
        for list in [&mut self.with_point, &mut self.negative_zero] {
            list.retain(|at| !at.as_str().is_some_and(inside));
        }
    }

    /// The facts as they stand at `pointer` in a file, from facts keyed from
    /// what stands there: a preference's own, read back onto its list.
    pub fn moved_to(entry: &Map<String, Value>, pointer: &str, into: &mut Map<String, Value>) {
        for name in FACTS {
            let Some(facts) = entry.get(name) else { continue };
            let moved = |at: &str| format!("{pointer}{at}");
            match (
                facts,
                into.entry(name.to_string()).or_insert_with(|| match facts {
                    Value::Object(_) => Value::Object(Map::new()),
                    _ => Value::Array(Vec::new()),
                }),
            ) {
                (Value::Object(orders), Value::Object(out)) => {
                    out.extend(orders.iter().map(|(at, order)| (moved(at), order.clone())));
                }
                (Value::Array(places), Value::Array(out)) => {
                    out.extend(places.iter().filter_map(Value::as_str).map(|at| Value::String(moved(at))));
                }
                _ => {}
            }
        }
    }

    /// Put the facts into a record's formatting entry, or onto the export
    /// entity's component, under `key_order`, `written_with_point` and
    /// `written_negative_zero`, where there are any.
    pub fn stamp(self, into: &mut Map<String, Value>) {
        if !self.key_order.is_empty() {
            into.insert("key_order".into(), Value::Object(self.key_order));
        }
        if !self.with_point.is_empty() {
            into.insert("written_with_point".into(), Value::Array(self.with_point));
        }
        if !self.negative_zero.is_empty() {
            into.insert("written_negative_zero".into(), Value::Array(self.negative_zero));
        }
    }
}

/// Where one sector preference's list stands in `sectorMxUPreferences.json`,
/// as a pointer after the file's name and as a place: under its carrier type,
/// in the file's one object or in that object as the one element of a list.
/// A preference is its list, and the facts of how the list is written are the
/// preference's, keyed from the list, as a record's are keyed from its object.
pub fn preference_list(carrier: &str, in_a_list: bool) -> (String, String) {
    let (pointer, place) = match in_a_list {
        true => ("sectorMxUPreferences.json/0/", "sectorMxUPreferences.json[]."),
        false => ("sectorMxUPreferences.json/", "sectorMxUPreferences.json."),
    };
    let mut pointer = pointer.to_string();
    escape(carrier, &mut pointer);
    (pointer, format!("{place}{carrier}"))
}

/// One key as a pointer step: `~` and `/` escaped, per RFC 6901.
fn escape(key: &str, pointer: &mut String) {
    for character in key.chars() {
        match character {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            other => pointer.push(other),
        }
    }
}

/// The member a built record carries its entry's two facts under on its way
/// to being written: `$comment`, which no file of an export can hold, since an
/// ingest refuses one, and which the writing takes off.
pub const CARRIED: &str = "$comment";

/// The names the facts of how a text spells its values ride under, in a
/// record's formatting entry and on the export entity alike.
pub const FACTS: [&str; 3] = ["key_order", "written_with_point", "written_negative_zero"];

/// The facts one record's formatting entry records, to carry on its built
/// record under [`CARRIED`]: `None` when it records none.
pub fn carried(entry: &Map<String, Value>) -> Option<Value> {
    let mut facts = Map::new();
    for name in FACTS {
        if let Some(value) = entry.get(name) {
            facts.insert(name.to_string(), value.clone());
        }
    }
    (!facts.is_empty()).then_some(Value::Object(facts))
}

/// What a projection writes its files' keys and numbers back by: the usual
/// orders, and the facts of everything outside the records, both off the
/// export entity. Empty for a dump nobody projected, whose files are written
/// as their values hold them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Arrangement {
    pub usual: Usual,
    pub outside: Map<String, Value>,
}

impl Arrangement {
    /// One file's value as it is written: every object's keys in the order its
    /// record's facts, or the export entity's for what is outside every record,
    /// record for its pointer, or else in the usual order of its place and keys;
    /// every number recorded as written with a decimal point as a double, or,
    /// where a double does not hold it exactly, as its own digits with `.0`
    /// after them; and a zero recorded as written with a minus sign with one.
    /// A key no order names, which only another tool's edit adds, follows the
    /// ones it names. A record's facts ride on it under [`CARRIED`].
    pub fn shape(&self, name: &str, value: &Value) -> Shape {
        self.arrange(value, &mut name.to_string(), &mut name.to_string(), &self.outside)
    }

    fn arrange(&self, value: &Value, place: &mut String, pointer: &mut String, facts: &Map<String, Value>) -> Shape {
        match value {
            Value::Object(map) => {
                let carried = map.get(CARRIED).and_then(Value::as_object);
                let mut own = String::new();
                let (facts, pointer) = match carried {
                    Some(carried) => (carried, &mut own),
                    None => (facts, pointer),
                };
                let mut keys: Vec<&str> = map.keys().map(String::as_str).filter(|key| *key != CARRIED).collect();
                let recorded: Option<Vec<&str>> = facts
                    .get("key_order")
                    .and_then(|orders| orders.get(pointer.as_str()))
                    .and_then(Value::as_array)
                    .map(|order| order.iter().filter_map(Value::as_str).collect());
                let order: Option<Vec<&str>> = recorded.or_else(|| {
                    expected(&self.usual, place, &keys).map(|order| order.iter().map(String::as_str).collect())
                });
                if let Some(order) = order {
                    // ordered: a stable sort, so a key no order names keeps its
                    // place among the others after the named ones.
                    keys.sort_by_key(|key| order.iter().position(|named| named == key).unwrap_or(usize::MAX));
                }
                let entries = keys
                    .into_iter()
                    .map(|key| {
                        let (place_at, pointer_at) = (place.len(), pointer.len());
                        place.push('.');
                        place.push_str(key);
                        pointer.push('/');
                        escape(key, pointer);
                        let child = self.arrange(&map[key], place, pointer, facts);
                        place.truncate(place_at);
                        pointer.truncate(pointer_at);
                        (key.to_string(), child)
                    })
                    .collect();
                Shape::Object(entries)
            }
            Value::Array(items) => {
                let (place_at, pointer_at) = (place.len(), pointer.len());
                place.push_str("[]");
                let items = items
                    .iter()
                    .enumerate()
                    .map(|(at, item)| {
                        pointer.push('/');
                        pointer.push_str(&at.to_string());
                        let item = self.arrange(item, place, pointer, facts);
                        pointer.truncate(pointer_at);
                        item
                    })
                    .collect();
                place.truncate(place_at);
                Shape::Array(items)
            }
            Value::Number(number) => {
                let recorded = |name: &str| {
                    facts
                        .get(name)
                        .and_then(Value::as_array)
                        .is_some_and(|places| places.iter().any(|at| at.as_str() == Some(pointer.as_str())))
                };
                if number.as_f64() == Some(0.0) && recorded("written_negative_zero") {
                    return Shape::Scalar(Value::Number(Number::from_f64(-0.0).expect("a finite double")));
                }
                if !recorded("written_with_point") {
                    return Shape::Scalar(value.clone());
                }
                // A whole number past what a double holds exactly, which only
                // another tool's edit puts where the file wrote one with a
                // point, keeps its digits rather than take its double's.
                let whole = number.as_i64().map(i128::from).or_else(|| number.as_u64().map(i128::from));
                match (whole, number.as_f64().and_then(Number::from_f64)) {
                    (Some(whole), Some(float)) if float.as_f64().is_some_and(|double| double as i128 == whole) => {
                        Shape::Scalar(Value::Number(float))
                    }
                    (Some(whole), _) => Shape::Spelled(format!("{whole}.0")),
                    (None, Some(float)) => Shape::Scalar(Value::Number(float)),
                    (None, None) => Shape::Scalar(value.clone()),
                }
            }
            other => Shape::Scalar(other.clone()),
        }
    }
}

/// How one file's text lays its value out: the separator between two items of
/// a list or an object and the one between a key and its value; the
/// indentation, where each item stands on a line of its own indented one unit
/// deeper than the list or object it is in; and the whitespace before and
/// after the value. The compact form, on one line with nothing around it, is
/// what a file stating no other is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spacing {
    pub item: String,
    pub key: String,
    pub indent: Option<String>,
    pub before: String,
    pub after: String,
}

impl Default for Spacing {
    fn default() -> Spacing {
        Spacing { item: ",".into(), key: ":".into(), indent: None, before: String::new(), after: String::new() }
    }
}

impl Spacing {
    /// The separator pairs a file on one line is written with: the compact one,
    /// and the one with a space after each separator, which one production
    /// export's eight files use. An indented file takes the first's item
    /// separator, a line break following it, and the second's key separator.
    const SEPARATORS: [(&'static str, &'static str); 2] = [(",", ":"), (", ", ": ")];

    /// The spacing a file's bytes spell its value in; or, where the value
    /// written in no spacing this module writes is those bytes, what in them
    /// cannot be written back, for a sentence: a string escaped or a number
    /// spelled otherwise than `serde_json` writes it, lines ended with CR LF,
    /// or spacing that is not the same throughout.
    pub fn of(bytes: &[u8], shape: &Shape) -> Result<Spacing, String> {
        let start = bytes.iter().position(|byte| !is_space(*byte)).unwrap_or(bytes.len());
        let end = bytes.iter().rposition(|byte| !is_space(*byte)).map_or(start, |at| at + 1);
        let body = &bytes[start..end];
        let around = |range: &[u8]| String::from_utf8_lossy(range).into_owned();
        // The unit an indented file indents by: what follows its first line
        // break, which is its first item's indentation, one unit deep.
        let indent = body.iter().position(|byte| *byte == b'\n').map(|at| {
            let unit: Vec<u8> =
                body[at + 1..].iter().copied().take_while(|byte| matches!(byte, b' ' | b'\t')).collect();
            String::from_utf8_lossy(&unit).into_owned()
        });
        let mut candidates: Vec<(&str, &str, Option<String>)> =
            Spacing::SEPARATORS.iter().map(|(item, key)| (*item, *key, None)).collect();
        candidates.extend(indent.map(|unit| (Spacing::SEPARATORS[0].0, Spacing::SEPARATORS[1].1, Some(unit))));
        // The text the spacing that agrees with the file the longest writes,
        // which is where a sentence says the two part ways.
        let mut nearest: (usize, Vec<u8>) = (0, Vec::new());
        for (item, key, indent) in candidates {
            let spacing = Spacing {
                item: item.into(),
                key: key.into(),
                indent,
                before: around(&bytes[..start]),
                after: around(&bytes[end..]),
            };
            let mut written = Vec::with_capacity(body.len());
            spacing.write(shape, 0, &mut written);
            if written == body {
                return Ok(spacing);
            }
            let agreed = body.iter().zip(&written).take_while(|(one, other)| one == other).count();
            if agreed >= nearest.0 {
                nearest = (agreed, written);
            }
        }
        Err(unwritable(bytes, start, body, nearest))
    }

    /// The file's whole text.
    pub fn text(&self, shape: &Shape) -> Vec<u8> {
        let mut out = self.before.as_bytes().to_vec();
        self.write(shape, 0, &mut out);
        out.extend(self.after.as_bytes());
        out
    }

    /// One value's text, `depth` lists and objects down.
    fn write(&self, shape: &Shape, depth: usize, out: &mut Vec<u8>) {
        let (open, close, items): (u8, u8, Vec<(Option<&String>, &Shape)>) = match shape {
            Shape::Object(entries) => (b'{', b'}', entries.iter().map(|(key, value)| (Some(key), value)).collect()),
            Shape::Array(items) => (b'[', b']', items.iter().map(|item| (None, item)).collect()),
            Shape::Scalar(value) => {
                serde_json::to_writer(&mut *out, value).expect("a JSON scalar serializes");
                return;
            }
            Shape::Spelled(text) => {
                out.extend(text.as_bytes());
                return;
            }
        };
        out.push(open);
        for (at, (key, value)) in items.iter().enumerate() {
            if at > 0 {
                out.extend(self.item.as_bytes());
            }
            self.line(depth + 1, out);
            if let Some(key) = key {
                serde_json::to_writer(&mut *out, key).expect("a string serializes");
                out.extend(self.key.as_bytes());
            }
            self.write(value, depth + 1, out);
        }
        if !items.is_empty() {
            self.line(depth, out);
        }
        out.push(close);
    }

    /// A line break and `depth` units of indentation, in an indented spacing.
    fn line(&self, depth: usize, out: &mut Vec<u8>) {
        if let Some(unit) = &self.indent {
            out.push(b'\n');
            for _ in 0..depth {
                out.extend(unit.as_bytes());
            }
        }
    }

    /// The spacing as the export entity records it: only what differs from the
    /// compact form, so a compact file records nothing.
    pub fn to_json(&self) -> Option<Value> {
        let mut out = Map::new();
        if (self.item.as_str(), self.key.as_str()) != Spacing::SEPARATORS[0] {
            out.insert("separators".into(), Value::from(vec![self.item.clone(), self.key.clone()]));
        }
        if let Some(unit) = &self.indent {
            out.insert("indent".into(), Value::String(unit.clone()));
        }
        for (name, text) in [("before", &self.before), ("after", &self.after)] {
            if !text.is_empty() {
                out.insert(name.into(), Value::String(text.clone()));
            }
        }
        (!out.is_empty()).then_some(Value::Object(out))
    }

    /// The spacing the export entity records for one file.
    pub fn from_json(value: Option<&Value>) -> Spacing {
        let mut spacing = Spacing::default();
        let Some(value) = value else { return spacing };
        if let Some([item, key]) = value.get("separators").and_then(Value::as_array).map(Vec::as_slice) {
            if let (Some(item), Some(key)) = (item.as_str(), key.as_str()) {
                spacing.item = item.into();
                spacing.key = key.into();
            }
        }
        spacing.indent = value.get("indent").and_then(Value::as_str).map(str::to_string);
        for (name, text) in [("before", &mut spacing.before), ("after", &mut spacing.after)] {
            if let Some(stated) = value.get(name).and_then(Value::as_str) {
                *text = stated.into();
            }
        }
        spacing
    }
}

/// The four bytes JSON allows between tokens.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// What in a file's text no spacing this module writes reproduces, for a
/// sentence: where the file and the spacing that agrees with it the longest
/// part ways, and the token there — a string, a number, or the whitespace
/// between tokens. `agreed` is how far the two agree and `nearest` that
/// spacing's text.
fn unwritable(bytes: &[u8], start: usize, body: &[u8], (agreed, nearest): (usize, Vec<u8>)) -> String {
    // The line and the column, counted in bytes, of one place in the body.
    let place = |offset: usize| {
        let at = start + offset;
        let line_start = bytes[..at].iter().rposition(|byte| *byte == b'\n').map_or(0, |newline| newline + 1);
        (1 + bytes[..line_start].iter().filter(|byte| **byte == b'\n').count(), at - line_start + 1)
    };
    // A piece of the text as it stands, but for the line breaks and tabs in
    // it, which are spelled out so the sentence stays on one line.
    let shown = |text: &[u8]| {
        let text = String::from_utf8_lossy(text);
        let mut out = String::new();
        for character in text.chars().take(40) {
            match character {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                other => out.push(other),
            }
        }
        if text.chars().count() > 40 {
            out.push('…');
        }
        out
    };
    // A string or a number the adapter writes otherwise, at the place the two
    // part or just before it, as where `-0` is written `-0.0`; else spacing.
    let respelled = |(from, to): (usize, usize)| -> Option<(usize, usize, Vec<u8>)> {
        let token = &body[from..to];
        let written = match token.first()? {
            b'"' => serde_json::to_vec(&serde_json::from_slice::<String>(token).ok()?).ok()?,
            b'-' | b'0'..=b'9' => serde_json::to_vec(&serde_json::from_slice::<Value>(token).ok()?).ok()?,
            _ => return None,
        };
        (written != token).then_some((from, to, written))
    };
    let here = token_at(body, agreed);
    let value = respelled(here).or_else(|| {
        let before = token_at(body, agreed.checked_sub(1)?);
        (before.1 == agreed).then_some(before).and_then(respelled)
    });
    if let Some((from, to, written)) = value {
        let (line, column) = place(from);
        let what = if body[from] == b'"' { "string" } else { "number" };
        let forgets = if body[from] == b'"' { "how it was escaped" } else { "how it was spelled" };
        return format!(
            "at line {line}, column {column}, it spells a {what} `{}`, which this adapter writes `{}`: a parse keeps \
             the {what} and forgets {forgets}",
            shown(&body[from..to]),
            shown(&written)
        );
    }
    let (line, column) = place(agreed);
    if body[here.0..here.1].contains(&b'\r') || body.get(agreed) == Some(&b'\r') {
        return format!(
            "it ends its lines with CR LF (`\\r\\n`), first at line {line}, and this adapter writes a line break as \
             LF alone"
        );
    }
    let context = agreed.saturating_sub(8);
    format!(
        "its spacing at line {line}, column {column} is not the spacing it is written in up to there: it has `{}` \
         where that spacing writes `{}`",
        shown(&body[context..(agreed + 12).min(body.len())]),
        shown(&nearest[context.min(nearest.len())..(agreed + 12).min(nearest.len())])
    )
}

/// The span of the token of a JSON text that `at` falls in: a string, a
/// number, a literal, one punctuation byte, or a run of whitespace.
fn token_at(text: &[u8], at: usize) -> (usize, usize) {
    let mut from = 0;
    while from < text.len() {
        let to = match text[from] {
            b'"' => {
                let mut end = from + 1;
                while end < text.len() && text[end] != b'"' {
                    end += if text[end] == b'\\' { 2 } else { 1 };
                }
                (end + 1).min(text.len())
            }
            byte if is_space(byte) => from + text[from..].iter().take_while(|byte| is_space(**byte)).count(),
            b'-' | b'0'..=b'9' | b'a'..=b'z' => {
                from + text[from..]
                    .iter()
                    .take_while(|byte| matches!(byte, b'-' | b'+' | b'.' | b'0'..=b'9' | b'a'..=b'z' | b'E'))
                    .count()
            }
            _ => from + 1,
        };
        if at < to {
            return (from, to);
        }
        from = to;
    }
    (text.len(), text.len())
}

#[cfg(test)]
mod tests {
    use super::Usual;

    use super::{carried, usual_orders, Arrangement, Facts, Shape, Spacing, CARRIED, RECORD_PLACES};
    use crate::layout::record_place;
    use crate::records::KINDS;
    use serde_json::{json, Map, Value};
    use std::collections::BTreeMap;

    fn shape(text: &str) -> Shape {
        serde_json::from_str(text).expect("JSON")
    }

    /// Every flat file's records stand at a place a walk stops at, so no
    /// record's facts are recorded twice.
    #[test]
    fn every_flat_record_place_is_a_record_place() {
        for kind in KINDS {
            let place = record_place(kind.file);
            if place.ends_with("[]") {
                assert!(RECORD_PLACES.contains(&place.as_str()), "{place}");
            }
        }
    }

    /// The records of `text`, each carrying the facts its file states, as the
    /// projection builds them: read by a parse that forgets key order.
    fn carrying(text: &str, place: &str, usual: &Usual) -> Value {
        let Shape::Array(rows) = shape(text) else { panic!("a list") };
        let mut read: Value = serde_json::from_str(text).expect("JSON");
        for (row, shape) in read.as_array_mut().expect("a list").iter_mut().zip(&rows) {
            let mut component = Map::new();
            Facts::of_record(shape, place, usual).stamp(&mut component);
            if let Some(facts) = carried(&component) {
                row.as_object_mut().expect("a record").insert(CARRIED.into(), facts);
            }
        }
        read
    }

    /// An object departing from the usual order of its place and keys is the
    /// one whose order is recorded, and writing the facts back gives every
    /// object its own order again.
    #[test]
    fn only_a_departing_order_is_recorded_and_every_order_comes_back() {
        let text = r#"[{"b":1,"a":2},{"b":3,"a":4},{"a":5,"b":6},{"c":1,"a":2,"b":3}]"#;
        let usual = usual_orders(&BTreeMap::from([("charger.json".to_string(), shape(text))]), None);
        assert_eq!(usual["charger.json[]"], [vec!["b", "a"], vec!["c", "a", "b"]]);
        let Shape::Array(rows) = shape(text) else { panic!("a list") };
        let departing: Vec<bool> =
            rows.iter().map(|row| !Facts::of_record(row, "charger.json[]", &usual).key_order.is_empty()).collect();
        assert_eq!(departing, [false, false, true, false]);
        let read = carrying(text, "charger.json[]", &usual);
        let arrangement = Arrangement { usual, outside: Map::new() };
        assert_eq!(Spacing::default().text(&arrangement.shape("charger.json", &read)), text.as_bytes());
    }

    /// An order the site already holds for a set of keys stays while an object
    /// still states the set, though most objects now state another, so the
    /// records nobody touched keep their facts; a set no object states any
    /// more is dropped, and one new to the files takes its most stated order.
    #[test]
    fn an_established_order_stays_while_its_set_is_stated() {
        let files = |text: &str| BTreeMap::from([("charger.json".to_string(), shape(text))]);
        let before = r#"[{"id":1,"status":"idle"},{"status":"idle","id":2},{"status":"idle","id":3},{"a":1,"b":2}]"#;
        let established = usual_orders(&files(before), None);
        assert_eq!(established["charger.json[]"], [vec!["status", "id"], vec!["a", "b"]]);
        // The third record gone: the two left tie, and the established order
        // stays rather than go to the one met first.
        let after = r#"[{"id":1,"status":"idle"},{"status":"idle","id":2},{"c":1,"d":2}]"#;
        let usual = usual_orders(&files(after), Some(&established));
        assert_eq!(usual["charger.json[]"], [vec!["status", "id"], vec!["c", "d"]]);
        assert_eq!(usual_orders(&files(after), None)["charger.json[]"][0], ["id", "status"]);
        let Shape::Array(rows) = shape(after) else { panic!("a list") };
        let departing: Vec<bool> =
            rows.iter().map(|row| !Facts::of_record(row, "charger.json[]", &usual).key_order.is_empty()).collect();
        assert_eq!(departing, [true, false, false]);
    }

    /// A whole number written with a decimal point is recorded where it
    /// stands, and comes back written with one; one written without stays so.
    #[test]
    fn a_whole_number_written_with_a_point_comes_back_with_one() {
        let text = r#"[{"count":3,"dimension":{"height":40.0,"width":12.5}}]"#;
        let Shape::Array(rows) = shape(text) else { panic!("a list") };
        let facts = Facts::of_record(&rows[0], "pps_bintype.json.data[]", &BTreeMap::new());
        assert_eq!(facts.with_point, [json!("/dimension/height")]);
        let mut read = carrying(text, "pps_bintype.json.data[]", &BTreeMap::new());
        // As a host reads it back: the whole number as an integer.
        read[0]["dimension"]["height"] = json!(40);
        let arrangement = Arrangement::default();
        assert_eq!(Spacing::default().text(&arrangement.shape("pps_bintype.json", &read)), text.as_bytes());
    }

    /// A zero written with a minus sign is recorded where it stands and comes
    /// back with one, though a reader holds it as 0; and a whole number
    /// recorded as written with a point that a double does not hold exactly,
    /// which only another tool's edit puts there, keeps its own digits.
    #[test]
    fn a_negative_zero_and_a_whole_number_past_a_double_come_back_as_written() {
        let text = r#"[{"charge":-0.0,"charger_id":5.0}]"#;
        let Shape::Array(rows) = shape(text) else { panic!("a list") };
        let facts = Facts::of_record(&rows[0], "charger.json[]", &BTreeMap::new());
        assert_eq!(facts.negative_zero, [json!("/charge")]);
        let mut read = carrying(text, "charger.json[]", &BTreeMap::new());
        // As a host holds them: the zero without its sign, the whole number as
        // an integer.
        read[0]["charge"] = json!(0);
        read[0]["charger_id"] = json!(5);
        let arrangement = Arrangement::default();
        assert_eq!(Spacing::default().text(&arrangement.shape("charger.json", &read)), text.as_bytes());
        read[0]["charger_id"] = json!(9_007_199_254_740_993_u64);
        assert_eq!(
            Spacing::default().text(&arrangement.shape("charger.json", &read)),
            br#"[{"charge":-0.0,"charger_id":9007199254740993.0}]"#
        );
    }

    /// A whole number past a double written with a point is read by its
    /// digits: the value takes the integer, the shape the characters, the
    /// record's facts say it was written with a point, and it is written back
    /// so. One past 64 bits stays the double, and the file is refused.
    #[test]
    fn a_whole_number_past_a_double_written_with_a_point_is_read_by_its_digits() {
        let text = r#"[{"a":9007199254740993.0,"b":[-9007199254740993.0,12345678901234567890.0],"c":9007199254740994.0,"d":1e+300,"e":"9007199254740993.0"}]"#;
        let mut read: Value = serde_json::from_str(text).expect("JSON");
        let mut file = shape(text);
        super::read_whole_digits(text.as_bytes(), &mut file, &mut read);
        assert_eq!(read[0]["a"], json!(9_007_199_254_740_993_u64));
        assert_eq!(read[0]["b"], json!([-9_007_199_254_740_993_i64, 12_345_678_901_234_567_890_u64]));
        assert_eq!(read[0]["c"], json!(9_007_199_254_740_994.0));
        assert_eq!(file.at("/0/b/0"), Some(&Shape::Spelled("-9007199254740993.0".into())));
        assert_eq!(Spacing::of(text.as_bytes(), &file), Ok(Spacing::default()));
        let Shape::Array(rows) = &file else { panic!("a list") };
        let mut entry = Map::new();
        Facts::of_record(&rows[0], "charger.json[]", &BTreeMap::new()).stamp(&mut entry);
        assert_eq!(entry["written_with_point"], json!(["/a", "/b/0", "/b/1", "/c", "/d"]));
        // As a host holds the record: the exact double as its integer.
        read[0]["c"] = json!(9_007_199_254_740_994_u64);
        read[0].as_object_mut().expect("a record").insert(CARRIED.into(), carried(&entry).expect("facts"));
        let written = Spacing::default().text(&Arrangement::default().shape("charger.json", &read));
        assert_eq!(String::from_utf8_lossy(&written), text);
        let past = "[123456789012345678901234567890.0]";
        let mut read: Value = serde_json::from_str(past).expect("JSON");
        let mut file = shape(past);
        super::read_whole_digits(past.as_bytes(), &mut file, &mut read);
        let refusal = Spacing::of(past.as_bytes(), &file).expect_err("no spacing writes it");
        assert!(refusal.contains("it spells a number `123456789012345678901234567890.0`"), "{refusal}");
    }

    /// A file's separators and the whitespace around its value are read off
    /// its bytes and written back; a file no spacing writes is answered with
    /// where it parts from the nearest one, and what is there.
    #[test]
    fn a_layout_is_read_off_the_bytes_and_written_back() {
        for text in [
            r#"[{"b": 1, "a": [1, 2]}]"#,
            "[{\"a\":1}]\n",
            "{}",
            " [] ",
            "[\n  {\n    \"b\": [],\n    \"a\": {}\n  },\n  3\n]\n",
            "[]\n",
            "[\n\t{\n\t\t\"a\": -0.0\n\t}\n]\r\n",
        ] {
            let shape = shape(text);
            let spacing = Spacing::of(text.as_bytes(), &shape).expect("a spacing this module writes");
            assert_eq!(spacing.text(&shape), text.as_bytes());
            assert_eq!(Spacing::from_json(spacing.to_json().as_ref()), spacing);
        }
        assert_eq!(Spacing::of(b"[1,2]", &shape("[1,2]")).ok().and_then(|spacing| spacing.to_json()), None);
        for (text, says) in [
            ("[\n  1,\n    2\n]", "its spacing at line 3, column 3 is not the spacing it is written in up to there"),
            (r#"[{"charger_id":5, "mode":"auto"}]"#, "its spacing at line 1, column 18"),
            (r#"{"a" : 1}"#, "its spacing at line 1, column 5"),
            ("[\r\n  1,\r\n  2\r\n]", "it ends its lines with CR LF (`\\r\\n`), first at line 1"),
            (r#"["\u0041"]"#, r#"at line 1, column 2, it spells a string `"\u0041"`, which this adapter writes `"A"`"#),
            (r#"{"url":"a\/b"}"#, r#"it spells a string `"a\/b"`, which this adapter writes `"a/b"`"#),
            ("[1E2]", "at line 1, column 2, it spells a number `1E2`, which this adapter writes `100.0`"),
            (r#"{"w":40.00}"#, "it spells a number `40.00`, which this adapter writes `40.0`"),
            ("[-0]", "at line 1, column 2, it spells a number `-0`, which this adapter writes `-0.0`"),
        ] {
            let refusal = Spacing::of(text.as_bytes(), &shape(text)).expect_err("no spacing writes it");
            assert!(refusal.contains(says), "{text}: {says:?} is not in {refusal}");
        }
    }

    /// A pointer reaches the value it names, keys escaped as RFC 6901 escapes
    /// them.
    #[test]
    fn a_pointer_names_one_value() {
        let file = shape(r#"[{"a/b":{"c~d":[7,8]}}]"#);
        assert_eq!(file.at("/0/a~1b/c~0d/1"), Some(&Shape::Scalar(json!(8))));
        assert_eq!(file.at(""), Some(&file));
        assert_eq!(file.at("/1"), None);
    }
}
