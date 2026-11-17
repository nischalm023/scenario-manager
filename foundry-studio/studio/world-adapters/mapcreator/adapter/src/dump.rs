//! Within the Map Creator adapter, `dump` is **one Map Creator export as a set of
//! files**: the JSON of every file a site shipped, read from the files a host
//! hands over, and written back as files of the same names.
//!
//! It is the adapter's input and its output, which is why it is one type rather
//! than two. The import reads a dump and the export builds one, and the round
//! trip's first law compares a built dump's files against the files read — so a
//! shape that could hold one and not the other would put the comparison in the
//! caller's hands.
//!
//! What it refuses to do is correct anything. A file that is `[]` on one site
//! and `{}` on another keeps the form it arrived in, a file a site never
//! shipped stays absent rather than becoming an empty one, and a file this
//! crate cannot parse is an error rather than a file quietly skipped — the
//! emptiness and the absence are facts about the site, and the export has to
//! reproduce both. The same goes for what a parse would correct by itself: a
//! key named twice in one object, of which `serde_json` keeps the last, and a
//! `$comment` note, which this adapter strips from everything it writes.
//!
//! Beside each file's JSON it holds what a parse into `serde_json`'s map
//! forgets and the round trip owes back (`written`): the file's keys in the
//! order its text states them, the separators and the whitespace it was
//! written with, and, on a dump the export built, what writes its keys back in
//! that order.

use serde_json::Deserializer as JsonDeserializer;
use crate::kinds::FileKind;
use crate::written::{self, Arrangement, Shape, Spacing};
use serde::de::{DeserializeSeed, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::map::Entry;
use serde_json::{Map, Number, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use worldadapt_sdk::File;

/// Everything that can go wrong reading or writing a dump, or converting one.
#[derive(Debug)]
pub enum Error {
    /// A file is there and is not JSON. A file this adapter cannot read is a
    /// field family it cannot account for.
    File { name: String, cause: String },
    /// A file no Map Creator export carries.
    UnknownFile { name: String },
    /// Two files under one name, of which a reader would keep one.
    RepeatedFile { name: String },
    /// A key named twice in one object, of which a reader keeps one value.
    KeyTwice { name: String, key: String, at: String },
    /// A `$comment` key, which nothing this adapter writes can carry back.
    Note { name: String, at: String },
    /// A file whose text no spacing this adapter writes reproduces, and what
    /// in it does not: the export would give the file back otherwise.
    Unwritable { name: String, what: String },
    /// The dump has no `map.json`, so there is no floor to convert.
    NoMap,
    /// The importer refused the dump. Its own sentence, verbatim.
    Import(String),
    /// A lens text would not parse.
    Lens(String),
    /// The site does not carry what a Map Creator export needs: no export
    /// entity, a station with no dialect, a reference that names nothing.
    Document(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::File { name, cause } => write!(f, "{name} is not JSON: {cause}"),
            Error::UnknownFile { name } => {
                write!(f, "{name} is not a file any Map Creator export carries")
            }
            Error::RepeatedFile { name } => write!(f, "{name} was handed over twice, and an export has one of each"),
            Error::KeyTwice { name, key, at } => write!(
                f,
                "{name} names the key `{key}` twice in one object, at {at}: a reader keeps one of the two values, so \
                 the other would be lost without a word"
            ),
            Error::Note { name, at } => write!(
                f,
                "{name} carries a `$comment` note at {at}: this adapter strips every `$comment` from what it \
                 compares and writes, so the export could not give the note back"
            ),
            Error::Unwritable { name, what } => write!(
                f,
                "{name} is written in a way this adapter cannot write back byte for byte: {what}. The export would \
                 give the file back otherwise than it was handed over, so the ingest refuses it"
            ),
            Error::NoMap => write!(f, "the export carries no map.json, so it states no floor"),
            Error::Import(message) => write!(f, "{message}"),
            Error::Lens(message) => write!(f, "{message}"),
            Error::Document(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

/// So that `?` on any step works inside a direction, whose error is the sentence
/// a host shows a person.
impl From<Error> for String {
    fn from(error: Error) -> String {
        error.to_string()
    }
}

/// One Map Creator export.
///
/// It has no name. The crate this adapter was ported from named a dump after the
/// directory it was read from; an adapter is handed files and no directory, and
/// a site holds one export, so nothing here needs one.
#[derive(Debug, Clone, PartialEq)]
pub struct Dump {
    /// Every file the dump carries, by file name. A file that is absent is
    /// absent from this map; one that is present and empty is here with its own
    /// empty form, `[]` or `{}`, because the two are different facts about the
    /// site and the export reproduces whichever it found.
    files: BTreeMap<String, Value>,
    /// Each file read from bytes, as its text states it, by file name: the
    /// import records its key orders and its numbers' spellings off this.
    shapes: BTreeMap<String, Shape>,
    /// The spacing each file's text was written in, by file name: read off the
    /// bytes a host handed over, or set by the export from what the import
    /// recorded. A file not here is written compact, with nothing around it.
    spacings: BTreeMap<String, Spacing>,
    /// On a dump the export built, what its files' keys and numbers are written
    /// back by.
    arrangement: Arrangement,
}

impl Dump {
    /// Read the files a host handed over.
    ///
    /// A file this adapter has no name for is **refused by name**, whatever it
    /// holds. A Map Creator export that has grown a file is a field family
    /// nobody has decided about, and reading the nineteen it knows would convert
    /// the site and report success while the new file produced neither state nor
    /// a difference. A file that is not JSON is refused by name for the same
    /// reason, and so is a second file under one name. So is a file whose text
    /// this adapter could not write back byte for byte — lines ended with CR
    /// LF, spacing that changes partway, a string escaped or a number spelled
    /// otherwise than `serde_json` writes it — naming what in it cannot be:
    /// the export would give it back changed without a word.
    pub fn read(files: &[File]) -> Result<Dump, Error> {
        let mut parsed = BTreeMap::new();
        let mut shapes = BTreeMap::new();
        let mut spacings = BTreeMap::new();
        for file in files {
            if FileKind::of(&file.name).is_none() {
                return Err(Error::UnknownFile { name: file.name.clone() });
            }
            let mut value = parse(&file.name, &file.bytes)?;
            if parsed.contains_key(&file.name) {
                return Err(Error::RepeatedFile { name: file.name.clone() });
            }
            // The same text read again keeping its order, which the first read
            // has already found to be JSON.
            let mut shape: Shape = serde_json::from_slice(&file.bytes)
                .map_err(|cause| Error::File { name: file.name.clone(), cause: cause.to_string() })?;
            written::read_whole_digits(&file.bytes, &mut shape, &mut value);
            let spacing =
                Spacing::of(&file.bytes, &shape).map_err(|what| Error::Unwritable { name: file.name.clone(), what })?;
            parsed.insert(file.name.clone(), value);
            spacings.insert(file.name.clone(), spacing);
            shapes.insert(file.name.clone(), shape);
        }
        let mut dump = Dump::from_files(parsed)?;
        dump.shapes = shapes;
        dump.spacings = spacings;
        Ok(dump)
    }

    /// One dump from file name to JSON.
    pub fn from_files(files: BTreeMap<String, Value>) -> Result<Dump, Error> {
        for file in files.keys() {
            if FileKind::of(file).is_none() {
                return Err(Error::UnknownFile { name: file.clone() });
            }
        }
        if !files.contains_key(FileKind::Map.name()) {
            return Err(Error::NoMap);
        }
        Ok(Dump { files, shapes: BTreeMap::new(), spacings: BTreeMap::new(), arrangement: Arrangement::default() })
    }

    /// The JSON of one file, or `None` when this dump does not carry it.
    pub fn get(&self, kind: FileKind) -> Option<&Value> {
        self.files.get(kind.name())
    }

    /// Every file this dump carries, in upload order.
    pub fn present(&self) -> Vec<FileKind> {
        FileKind::ALL.into_iter().filter(|kind| self.files.contains_key(kind.name())).collect()
    }

    /// Whether a file is present and has no records in it. An empty file still
    /// states something — the site shipped it — and the form it is empty in,
    /// `[]` or `{}`, is part of that.
    pub fn is_empty_file(&self, kind: FileKind) -> bool {
        match self.get(kind) {
            Some(Value::Array(items)) => items.is_empty(),
            Some(Value::Object(map)) => map.is_empty(),
            _ => false,
        }
    }

    /// Every file's JSON, by file name — what `from_files` takes back.
    pub fn to_files(&self) -> &BTreeMap<String, Value> {
        &self.files
    }

    /// Put one file's JSON in, as the export builds a dump up file by file.
    pub fn set(&mut self, kind: FileKind, value: Value) {
        self.files.insert(kind.name().to_string(), value);
    }

    /// One file read from bytes as its text states it, or `None` for a file
    /// this dump was not read from bytes for.
    pub fn shape(&self, kind: FileKind) -> Option<&Shape> {
        self.shapes.get(kind.name())
    }

    /// Every file read from bytes as its text states it, by file name.
    pub fn shapes(&self) -> &BTreeMap<String, Shape> {
        &self.shapes
    }

    /// The spacing one file's text was written in, when it was one this adapter
    /// writes.
    pub fn spacing(&self, kind: FileKind) -> Option<&Spacing> {
        self.spacings.get(kind.name())
    }

    /// Say which spacing one file is to be written in.
    pub fn set_spacing(&mut self, kind: FileKind, spacing: Spacing) {
        self.spacings.insert(kind.name().to_string(), spacing);
    }

    /// Say what the files' keys and numbers are written back by.
    pub fn set_arrangement(&mut self, arrangement: Arrangement) {
        self.arrangement = arrangement;
    }

    /// Every file, as a host writes it out: in its own spacing, its keys in the
    /// order the file was read in or the arrangement records, its numbers as
    /// they were written. A file with no spacing is one compact line, which is
    /// how Map Creator writes most of them.
    pub fn write(&self) -> Vec<File> {
        self.files
            .iter()
            .map(|(name, value)| {
                let spacing = self.spacings.get(name).cloned().unwrap_or_default();
                let text = match self.shapes.get(name) {
                    Some(shape) => spacing.text(shape),
                    None => spacing.text(&self.arrangement.shape(name, value)),
                };
                File::new(name.clone(), text)
            })
            .collect()
    }
}

/// One file's JSON, read as `serde_json` reads it and refused, naming where,
/// at a key named twice in one object or at a `$comment` note.
fn parse(name: &str, bytes: &[u8]) -> Result<Value, Error> {
    let found = RefCell::new(None);
    let mut reader = JsonDeserializer::from_slice(bytes);
    let read = Checked { found: &found }.deserialize(&mut reader).and_then(|value| reader.end().map(|()| value));
    match (read, found.into_inner()) {
        (Ok(value), _) => Ok(value),
        (Err(_), Some(Found { twice, mut steps })) => {
            steps.reverse();
            let mut at = format!("{name}:");
            for step in &steps {
                if step != "[]" && !at.ends_with(':') {
                    at.push('.');
                }
                at.push_str(step);
            }
            if steps.is_empty() {
                at = format!("the top of {name}");
            }
            Err(match twice {
                Some(key) => Error::KeyTwice { name: name.to_string(), key, at },
                None => Error::Note { name: name.to_string(), at },
            })
        }
        (Err(cause), None) => Err(Error::File { name: name.to_string(), cause: cause.to_string() }),
    }
}

/// What the parse stopped at: the key named twice, or none for a note, and the
/// steps to it, innermost first, each object key or `[]` for an array element.
struct Found {
    twice: Option<String>,
    steps: Vec<String>,
}

/// A value read as `serde_json`'s own `Value` reads one, which stops at what
/// [`parse`] refuses and leaves a [`Found`] saying where. The path is collected
/// only on the way out of a refusal, so a file that parses pays nothing for it.
#[derive(Clone, Copy)]
struct Checked<'a> {
    found: &'a RefCell<Option<Found>>,
}

impl Checked<'_> {
    /// A refusal on its way out of one step, which the step adds to the path.
    fn within<E>(self, step: &str, error: E) -> E {
        if let Some(found) = self.found.borrow_mut().as_mut() {
            found.steps.push(step.to_string());
        }
        error
    }
}

impl<'de> DeserializeSeed<'de> for Checked<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Checked<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Value, A::Error> {
        let mut out = Vec::with_capacity(items.size_hint().unwrap_or(0));
        while let Some(item) = items.next_element_seed(self).map_err(|error| self.within("[]", error))? {
            out.push(item);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut out = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if key == "$comment" {
                *self.found.borrow_mut() = Some(Found { twice: None, steps: vec![key] });
                return Err(A::Error::custom("a note"));
            }
            let value = entries.next_value_seed(self).map_err(|error| self.within(&key, error))?;
            match out.entry(key) {
                Entry::Vacant(slot) => {
                    slot.insert(value);
                }
                Entry::Occupied(slot) => {
                    *self.found.borrow_mut() = Some(Found { twice: Some(slot.key().clone()), steps: Vec::new() });
                    return Err(A::Error::custom("a key named twice"));
                }
            }
        }
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::{Dump, Error};
    use crate::kinds::FileKind;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use worldadapt_sdk::File;

    fn one_map() -> BTreeMap<String, Value> {
        let mut files = BTreeMap::new();
        files.insert("map.json".to_string(), json!([{ "floor_id": 1 }]));
        files
    }

    #[test]
    fn a_dump_needs_a_map() {
        assert!(matches!(Dump::from_files(BTreeMap::new()), Err(Error::NoMap)));
        assert!(Dump::from_files(one_map()).is_ok());
    }

    #[test]
    fn a_file_no_export_carries_is_refused() {
        let mut files = one_map();
        files.insert("racks.json".to_string(), json!([]));
        assert!(matches!(Dump::from_files(files), Err(Error::UnknownFile { .. })));
    }

    /// The two empty forms are two different facts, and both survive a round
    /// trip through the type.
    #[test]
    fn an_empty_file_keeps_the_form_it_arrived_in() {
        let mut files = one_map();
        files.insert("elevator.json".to_string(), json!([]));
        files.insert("conveyor.json".to_string(), json!({}));
        let dump = Dump::from_files(files).expect("a map is there");
        assert!(dump.is_empty_file(FileKind::Elevator));
        assert!(dump.is_empty_file(FileKind::Conveyor));
        assert_eq!(dump.get(FileKind::Elevator), Some(&json!([])));
        assert_eq!(dump.get(FileKind::Conveyor), Some(&json!({})));
        assert!(!dump.is_empty_file(FileKind::Map));
        // Absence is not emptiness.
        assert_eq!(dump.get(FileKind::Pptl), None);
        assert!(!dump.is_empty_file(FileKind::Pptl));
    }

    /// Every file handed over is either one the format names or refused by its
    /// own name: an unknown name, a file that is not JSON, and a name handed
    /// over twice.
    #[test]
    fn every_file_handed_over_is_read_or_refused_by_name() {
        let map = File::new("map.json", b"[{\"floor_id\": 1}]".to_vec());
        assert!(Dump::read(std::slice::from_ref(&map)).is_ok());
        for (extra, says) in [
            (File::new("new_feature.json", b"[]".to_vec()), "new_feature.json is not a file"),
            (File::new("README.md", b"not the format".to_vec()), "README.md is not a file"),
            (File::new("zone.json", b"[{".to_vec()), "zone.json is not JSON"),
            (map.clone(), "map.json was handed over twice"),
        ] {
            let refusal = Dump::read(&[map.clone(), extra]).expect_err("a refusal").to_string();
            assert!(refusal.starts_with(says), "{refusal}");
        }
    }

    /// A file reads as `serde_json` reads it, numbers, escapes and nesting
    /// alike; a key named twice in one object and a `$comment` note are each
    /// refused, naming the file and where.
    #[test]
    fn a_key_named_twice_and_a_note_are_refused_where_they_stand() {
        let text =
            br#"[{"a": [1, 2.5, -0.0, 1e300, 18446744073709551615], "b": {"c": "tab\there \"quoted\"\n", "d": null}}]"#;
        assert_eq!(
            super::parse("x.json", text).expect("it parses"),
            serde_json::from_slice::<Value>(text).expect("JSON")
        );
        for (text, says) in [
            (&br#"[{"a": 1, "a": 2}]"#[..], "x.json names the key `a` twice in one object, at x.json:[]:"),
            (br#"{"a": 1, "a": 2}"#, "x.json names the key `a` twice in one object, at the top of x.json:"),
            (
                br#"[{"b": {"c": [{"d": 1, "e": 2, "d": 3}]}}]"#,
                "x.json names the key `d` twice in one object, at x.json:[].b.c[]:",
            ),
            (br#"{"a": [{"$comment": "note"}]}"#, "x.json carries a `$comment` note at x.json:a[].$comment:"),
            (br#"[1, 2] 3"#, "x.json is not JSON"),
        ] {
            let refusal = super::parse("x.json", text).expect_err("a refusal").to_string();
            assert!(refusal.starts_with(says), "{refusal}");
        }
    }

    /// A dump goes back out as one compact line per file, under the names it
    /// came in under.
    #[test]
    fn a_dump_is_written_as_one_line_per_file() {
        let dump = Dump::from_files(one_map()).expect("a map is there");
        assert_eq!(dump.write(), [File::new("map.json", b"[{\"floor_id\":1}]".to_vec())]);
    }
}
