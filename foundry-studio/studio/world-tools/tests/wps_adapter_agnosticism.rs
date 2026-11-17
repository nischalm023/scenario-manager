//! Within the adapter host's tests, `agnosticism` is THE ABSENCE OF A
//! VOCABULARY in the sources under `adapter/`: no component name written into
//! a string literal in the SDK, the host, the test kit or the command line.
//!
//! The rule is the one the plan states for this family: everything that knows
//! a component or a format lives in a module, and what runs modules knows
//! neither (Daniel, 2026-09-17). A module names the components it reads and
//! writes; the code that loads it, checks it and tests it must not, or the day
//! a format's adapter needs a special case is the day the host grows one.
//!
//! The register is the protocol's core names, read from the schema, TOGETHER
//! WITH every component key the example documents carry, because a real site
//! writes names the schema's list does not hold (`easyloop`, `highway`). It is
//! read rather than written down here, because a list in this file would be the
//! vocabulary it forbids.
//!
//! What this gives is a floor and not a proof, as the lens crate's test says of
//! itself, whose scanner this one follows: a name assembled from pieces, or one
//! no document in this tree writes, passes. Prose is exempt on purpose — these
//! charters speak of identities and scopes, and comments are read as comments
//! — and so are the probes and the seed adapter, whose business is naming what
//! they read.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "../../test-support/wps_source.rs"]
mod wps_source;

/// The resolved WPS adapter source.
fn adapter_dir() -> PathBuf {
    wps_source::root().join("adapter")
}

/// The four crates whose sources run, check or test a module, and none of
/// whose business is a component.
const CRATES: [&str; 4] = ["sdk", "host", "testkit", "cli"];

/// Every component name the protocol's schema defines.
fn core_names() -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application/schema/components.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let schema: Value = serde_json::from_str(&text).expect("components.json is JSON");
    let names: BTreeSet<String> = schema["$defs"]["core_name"]["enum"]
        .as_array()
        .expect("components.json#/$defs/core_name enumerates the protocol's component names")
        .iter()
        .map(|name| name.as_str().expect("a component name is a string").to_string())
        .collect();
    assert!(names.len() > 20, "the component register came back with {} names — has the schema moved?", names.len());
    names
}

/// Every key of every component map the example documents write: on records,
/// on template internals, on derived nodes, and in a tiling's `derives`.
fn document_names() -> BTreeSet<String> {
    fn walk(value: &Value, found: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, member) in map {
                    if key == "components" || key == "derives" {
                        if let Value::Object(components) = member {
                            found.extend(components.keys().filter(|name| !name.starts_with('$')).cloned());
                        }
                    }
                    walk(member, found);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, found)),
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    let mut documents = 0;
    for directory in ["examples/walmart-dallas", "examples/walmart-dfw6"] {
        for entry in fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application-tests/wps-sites").join(directory)).expect("an example directory") {
            let path = entry.expect("a directory entry").path();
            if path.to_string_lossy().ends_with(".world.json") {
                let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                walk(&serde_json::from_str(&text).expect("a document is JSON"), &mut found);
                documents += 1;
            }
        }
    }
    assert!(documents >= 6, "only {documents} example documents found — have the examples moved?");
    found
}

/// The names the rule is written against.
fn component_names() -> BTreeSet<String> {
    let mut names = core_names();
    let core = names.len();
    names.extend(document_names());
    assert!(names.len() > core, "the documents carry no name beyond the schema's — are they being read?");
    assert!(names.contains("easyloop"), "the EasyLoop component the example documents carry is missing");
    names
}

/// Whether `needle` stands at `i` in `chars`.
fn at(chars: &[char], i: usize, needle: &str) -> bool {
    needle.chars().enumerate().all(|(offset, want)| chars.get(i + offset) == Some(&want))
}

/// Every string literal in one Rust source, each with the line it opens on.
///
/// Character by character, as the lens crate's scanner reads one: `//` to the
/// end of the line, `/* … */` to its match and nested, a raw string (`r`, or
/// `br` and `cr` for its byte and C forms) to the quote with the same run of
/// hashes, an ordinary string to the first quote no backslash precedes, and a
/// char literal skipped whole so a quote inside one opens nothing. An ordinary
/// string comes back with its escapes decoded as the compiler decodes them, so
/// `"po\u{73}e"` is the name it spells; a raw one comes back as written.
fn literals(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let (mut i, mut line, mut comment) = (0usize, 1usize, 0usize);
    while i < chars.len() {
        if chars[i] == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if comment > 0 {
            if at(&chars, i, "/*") {
                comment += 1;
                i += 2;
            } else if at(&chars, i, "*/") {
                comment -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if at(&chars, i, "//") {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if at(&chars, i, "/*") {
            comment = 1;
            i += 2;
            continue;
        }
        let opens_a_token = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        let raw_from = match chars[i] {
            'r' if opens_a_token => Some(i + 1),
            'b' | 'c' if opens_a_token && chars.get(i + 1) == Some(&'r') => Some(i + 2),
            _ => None,
        };
        if let Some(mut j) = raw_from {
            let mut hashes = 0usize;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                let (text, end) = read_raw(&chars, j + 1, hashes);
                found.push((line, text));
                line += line_breaks(&chars[i..end]);
                i = end;
                continue;
            }
        }
        if chars[i] == '\'' {
            if let Some(end) = char_literal_end(&chars, i) {
                i = end;
                continue;
            }
        }
        if chars[i] == '"' {
            let (text, end) = read_quoted(&chars, i + 1);
            found.push((line, text));
            line += line_breaks(&chars[i..end]);
            i = end;
            continue;
        }
        i += 1;
    }
    found
}

/// Where a char literal opening at `from` ends, or `None` when the quote opens
/// a lifetime instead: only a closing quote tells `'a'` from `'a`.
fn char_literal_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from + 1;
    if chars.get(i) == Some(&'\\') {
        i += 1;
        if chars.get(i) == Some(&'u') {
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
        }
    }
    i += 1;
    (chars.get(i) == Some(&'\'')).then_some(i + 1)
}

/// How many lines a stretch of source spans past its first, counted in the
/// source and not in what an escape decodes to.
fn line_breaks(source: &[char]) -> usize {
    source.iter().filter(|c| **c == '\n').count()
}

/// An ordinary string's text, decoded: `\n`, `\r`, `\t`, `\0`, a quote or a
/// backslash escaped, `\x` and two hex digits, `\u{…}`, and a backslash before
/// a line break, which skips the break and the whitespace after it.
fn read_quoted(chars: &[char], from: usize) -> (String, usize) {
    let hex = |digits: &[char]| -> Option<char> {
        let digits: String = digits.iter().filter(|c| **c != '_').collect();
        u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32)
    };
    let mut text = String::new();
    let mut i = from;
    while i < chars.len() {
        match (chars[i], chars.get(i + 1)) {
            ('"', _) => return (text, i + 1),
            ('\\', Some('x')) => {
                text.extend(chars.get(i + 2..i + 4).and_then(hex));
                i += 4;
            }
            ('\\', Some('u')) => {
                let close = chars[i..].iter().position(|c| *c == '}').map_or(chars.len(), |at| i + at);
                text.extend(chars.get(i + 3..close).and_then(hex));
                i = close + 1;
            }
            ('\\', Some('\n' | '\r')) => {
                i += 1;
                while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                    i += 1;
                }
            }
            ('\\', Some(&escaped)) => {
                text.push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    other => other,
                });
                i += 2;
            }
            (c, _) => {
                text.push(c);
                i += 1;
            }
        }
    }
    (text, i)
}

fn read_raw(chars: &[char], from: usize, hashes: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '"' && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#')) {
            return (text, i + 1 + hashes);
        }
        text.push(chars[i]);
        i += 1;
    }
    (text, i)
}

/// Every place one of `names` is spelled out inside a string literal: as the
/// whole literal, which is `entity.raw("pose")`, or as a quoted key inside
/// one, which is the same name in a payload the code builds or matches.
fn named_in(source: &str, names: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    for (line, literal) in literals(source) {
        if names.contains(&literal) {
            found.push(format!("{line}: `{literal}`, the whole literal"));
        }
        for name in names {
            if literal.contains(&format!("\"{name}\"")) {
                found.push(format!("{line}: `\"{name}\"`, a key"));
            }
        }
    }
    found
}

/// Every `.rs` file under `root`, in a stable order so a failure reads the same
/// way twice.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_string_literal_under_adapter_names_a_component() {
    let names = component_names();
    let mut scanned = 0;
    let mut offences = Vec::new();
    for name in CRATES {
        for file in rust_files(&adapter_dir().join(name).join("src")) {
            let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            offences.extend(named_in(&text, &names).into_iter().map(|offence| format!("{}:{offence}", file.display())));
            scanned += 1;
        }
    }
    assert!(scanned >= 12, "only {scanned} sources scanned under adapter/ — has the layout moved?");
    assert!(
        offences.is_empty(),
        "the adapter family spells {} component name(s) it is not allowed to know: everything that knows a \
         component lives in a module, and what loads, checks and tests one names none:\n{}",
        offences.len(),
        offences.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The scanner itself, over snippets written here: a scan is only worth its run
// if it can fail.
// ---------------------------------------------------------------------------

fn register() -> BTreeSet<String> {
    ["pose", "external_ids", "quicktron.easyloop"].into_iter().map(str::to_string).collect()
}

#[test]
fn a_name_as_the_whole_literal_or_as_a_key_is_caught() {
    assert_eq!(named_in(r#"let p = entity.raw("pose");"#, &register()), ["1: `pose`, the whole literal"]);
    assert_eq!(named_in("let body = \"{\\\"pose\\\": {}}\";", &register()), ["1: `\"pose\"`, a key"]);
    assert_eq!(named_in(r##"let j = r#"{"external_ids": {}}"#;"##, &register()), ["1: `\"external_ids\"`, a key"]);
    assert_eq!(
        named_in(r#"let c = "quicktron.easyloop";"#, &register()),
        ["1: `quicktron.easyloop`, the whole literal"]
    );
}

/// A name spelled with escapes is the name it spells, and a byte string or a
/// C string, raw or not, is read like any other string.
#[test]
fn a_name_spelled_with_escapes_or_in_a_byte_or_c_string_is_caught() {
    assert_eq!(named_in(r#"let p = "po\u{73}e";"#, &register()), ["1: `pose`, the whole literal"]);
    assert_eq!(named_in(r#"let p = "\x70ose";"#, &register()), ["1: `pose`, the whole literal"]);
    assert_eq!(named_in("let p = \"po\\\n    se\";", &register()), ["1: `pose`, the whole literal"]);
    assert_eq!(named_in(r#"let b = b"external_ids";"#, &register()), ["1: `external_ids`, the whole literal"]);
    assert_eq!(named_in(r##"let j = br#"{"pose":{}}"#;"##, &register()), ["1: `\"pose\"`, a key"]);
    assert_eq!(named_in(r#"let c = cr"pose";"#, &register()), ["1: `pose`, the whole literal"]);
}

/// A line is counted where the source breaks, not where an escape decodes to
/// a line break, so a name after either is placed on its own line.
#[test]
fn a_decoded_line_break_counts_no_line() {
    let source = "let s = \"one\\ntwo\";\nlet p = \"pose\";";
    assert_eq!(named_in(source, &register()), ["2: `pose`, the whole literal"]);
}

#[test]
fn prose_and_comments_are_not_caught() {
    assert!(named_in("/// looks a cell up by its external_ids, as \"pose\" is.\nfn f() {}", &register()).is_empty());
    assert!(named_in("/* \"pose\" */\nfn f() {}", &register()).is_empty());
    assert!(named_in(r#"let why = "a pose of mind";"#, &register()).is_empty());
}

/// A quote inside a char literal opens no string, so what follows it is still
/// read as the code it is; a lifetime is not a char literal at all.
#[test]
fn a_name_after_a_char_literal_or_a_lifetime_is_still_caught() {
    let source = r#"fn f<'a>(s: &'a str, c: char) -> bool {
    if c == '"' { return true; }
    let p = "pose";
    false
}"#;
    assert_eq!(named_in(source, &register()), ["3: `pose`, the whole literal"]);
}
