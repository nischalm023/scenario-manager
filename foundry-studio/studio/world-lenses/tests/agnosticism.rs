//! Within Foundry's application tests, `agnosticism` is the ABSENCE OF A VOCABULARY in
//! the shared engine's sources: no component name written into a string literal
//! under `src`, and no typed component view imported there.
//!
//! The rule behind it is Daniel's of 2026-09-16: the engine reads recorded
//! facts — which components an entity carries, the values in them, the entities
//! its fields name, and how a derivation address nests — and names no component
//! while doing it. A pattern names components; the code that runs one must not.
//!
//! The second rule is the one that pins the ruling's other half. Geometry came
//! into this crate through the reader's typed views: `Pose` to resolve a
//! position, `Region` to build a rectangle. Both are gone, and an import of
//! either is how they would come back — so the import is what is refused, and
//! the refusal names the two tests as one wall.
//!
//! The register is the protocol's own core names TOGETHER WITH every component
//! key the four surveyed documents carry, because a real map writes names the
//! schema's list does not hold: `easyloop` is what the shipped library reads
//! throughout, and a product's name is exactly as special-caseable as a core
//! one.
//!
//! What this gives is a floor and not a proof. A name assembled from pieces, or
//! read out of a pattern at run time, passes — as it should, because that is
//! what running a pattern is. A name no document in this tree writes and the
//! schema does not state passes too, because nothing here knows it is a name.
//! Prose is exempt on purpose: these charters talk
//! about poses and frames constantly and should keep doing so, and the scanner
//! reads comments as comments. An example inside a refusal message passes too:
//! the parser tells an author to write `ref_walk(c, m, "pose.frame", …)`, which
//! is a name in a literal and is teaching rather than deciding.
//!
//! The view rule has limits of the same kind. It reads text: a module alias is
//! caught where it is declared (`use site_model::components as c;`) and not
//! where `c::spatial::Pose` is later written, and a path a macro spells is not
//! read at all. Both are floors a reviewer's eye is still owed.

mod fixtures;

use fixtures::{documents, Fixture};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every component name this version of the protocol defines, read out of the
/// schema rather than listed here: the point is that this crate carries no such
/// list, and a test that carried one would be the thing it forbids.
fn core_names() -> BTreeSet<String> {
    let path: PathBuf =
        [crate_dir().as_path(), Path::new("../../data/seed/application/schema/components.json")].iter().collect();
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

/// The names the rule is written against: the schema's core register and every
/// component key the four surveyed documents carry.
///
/// The documents are read for this rather than a second list being written
/// down, for the reason the schema is: a list here is the vocabulary this file
/// forbids. What they add is the names the schema does not state — `easyloop`,
/// the one the shipped library reads on every machine row, among them.
fn component_names() -> BTreeSet<String> {
    let mut names = core_names();
    let core = names.len();
    for Fixture { world, .. } in documents() {
        for entity in world.entities.values() {
            names.extend(entity.components.keys().cloned());
        }
    }
    assert!(
        names.len() > core,
        "the four documents carry no component name beyond the {core} the schema states — are they loading?"
    );
    assert!(names.contains("easyloop"), "the component the shipped library reads on every machine row is missing");
    names
}

/// Whether `needle` stands at `i` in `chars`.
fn at(chars: &[char], i: usize, needle: &str) -> bool {
    needle.chars().enumerate().all(|(offset, want)| chars.get(i + offset) == Some(&want))
}

/// Every string literal in one Rust source, each with the line it opens on, and
/// the source with its comments blanked out.
///
/// Character by character, because telling a literal from a comment is a
/// lexical question and no parser is wanted: `//` runs to the end of the line,
/// `/* … */` to its match and nests, a raw string to the quote with the same run
/// of hashes, an ordinary string to the first quote no backslash precedes.
/// Literal text comes back exactly as written, escapes and all, because a JSON
/// key inside an ordinary Rust string is written `\"pose\"` and inside a raw
/// string `"pose"`, and un-escaping would hide which was written.
fn read(source: &str) -> (Vec<(usize, String)>, String) {
    let chars: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut code = String::new();
    let (mut i, mut line, mut comment) = (0usize, 1usize, 0usize);
    while i < chars.len() {
        if chars[i] == '\n' {
            line += 1;
            code.push('\n');
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
        // A raw string opens with `r`, any run of `#`, then the quote — and the
        // `r` has to start a token, or the one ending an identifier just before
        // a string would open a raw string that never closes.
        let opens_a_token = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if chars[i] == 'r' && opens_a_token {
            let (mut j, mut hashes) = (i + 1, 0usize);
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                let (text, end) = read_raw(&chars, j + 1, hashes);
                line += text.matches('\n').count();
                literals.push((line, text));
                i = end;
                continue;
            }
        }
        // A char literal, before the quote case, because two of them in
        // `parse.rs` hold a quote (`if c == '"'`). Read as a string opening,
        // that quote runs to the next one in the file and swaps code for
        // literal in every run after it, so a component name written anywhere
        // downstream is read as code and never checked.
        if chars[i] == '\'' {
            if let Some(end) = char_literal_end(&chars, i) {
                i = end;
                continue;
            }
        }
        if chars[i] == '"' {
            let (text, end) = read_quoted(&chars, i + 1);
            line += text.matches('\n').count();
            literals.push((line, text));
            i = end;
            continue;
        }
        code.push(chars[i]);
        i += 1;
    }
    (literals, code)
}

/// Where a char literal opening at `from` ends, or `None` when that quote opens
/// a LIFETIME instead and the source continues at the next character.
///
/// The closing quote is the whole of what tells the two apart: `'a'` is a
/// character and `'a` is a lifetime, and Rust has no place a quote may follow a
/// lifetime. A char literal holds no newline, so skipping one loses no line.
fn char_literal_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from + 1;
    if chars.get(i) == Some(&'\\') {
        i += 1;
        // `\u{7f}` is the one escape longer than the character it escapes.
        if chars.get(i) == Some(&'u') {
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
        }
    }
    i += 1;
    (chars.get(i) == Some(&'\'')).then_some(i + 1)
}

/// An ordinary string literal's text, from just past its opening quote, and
/// where the source continues after the closing one.
fn read_quoted(chars: &[char], from: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            // Both characters, so `\"` survives into the text.
            text.push(chars[i]);
            if let Some(&next) = chars.get(i + 1) {
                text.push(next);
            }
            i += 2;
            continue;
        }
        if chars[i] == '"' {
            return (text, i + 1);
        }
        text.push(chars[i]);
        i += 1;
    }
    (text, i)
}

/// A raw string literal's text and where the source continues after it.
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

/// Every place one of `names` is spelled out inside a string literal, as
/// `line: what was found`.
///
/// Two forms, because a name reaches a decision in two: a whole literal equal to
/// a name is `entity.raw("pose")`, and a name in quotes inside a literal is the
/// same key in a payload the code builds or matches.
fn named_in(source: &str, names: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    for (line, literal) in read(source).0 {
        if names.contains(&literal) {
            found.push(format!("{line}: `{literal}`, the whole literal"));
        }
        for name in names {
            if literal.contains(&format!("\"{name}\"")) || literal.contains(&format!("\\\"{name}\\\"")) {
                found.push(format!("{line}: `\"{name}\"`, a key"));
            }
        }
    }
    found
}

/// Every mention of application component views in code.
fn views_in(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (number, line) in read(source).1.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for start in 0..chars.len() {
            if !at(&chars, start, "components::") {
                continue;
            }
            let item: String =
                chars[start..].iter().take_while(|c| c.is_alphanumeric() || **c == '_' || **c == ':').collect();
            found.push(format!("{}: {item}", number + 1));
        }
        // The module named without `::` after it: an alias (`components as c`)
        // or a group import (`{components, …}`) is the module coming in whole,
        // and every view with it. Caught where it is declared; a use of the
        // alias elsewhere spells no `components` and is not, which the module
        // doc states.
        if line.contains("site_model") {
            for start in 0..chars.len() {
                let word = at(&chars, start, "components")
                    && !start.checked_sub(1).is_some_and(|i| chars[i].is_alphanumeric() || chars[i] == '_');
                if !word {
                    continue;
                }
                let after = chars.get(start + "components".len()).copied();
                if after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') {
                    continue;
                }
                found.push(format!("{}: `components` imported as a module", number + 1));
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
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[path = "../../test-support/wps_source.rs"]
mod wps_source;

fn sources() -> Vec<PathBuf> {
    let root = wps_source::root().join("lens/rust/src");
    let files = rust_files(&root);
    assert!(files.len() > 5, "only {} sources found under src — has the layout moved?", files.len());
    files
}

#[test]
fn no_string_literal_in_the_engine_names_a_component() {
    let names = component_names();
    let mut offences = Vec::new();
    for file in sources() {
        let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for offence in named_in(&text, &names) {
            offences.push(format!("{}:{offence}", file.display()));
        }
    }
    assert!(
        offences.is_empty(),
        "the engine spells {} component name(s) it is not allowed to know (the ruling of \
         2026-09-16: it reads recorded facts and names no component). A pattern names \
         components; the code that runs one does not:\n{}",
        offences.len(),
        offences.join("\n")
    );
}

#[test]
fn no_typed_component_view_is_imported_into_the_engine() {
    let mut offences = Vec::new();
    for file in sources() {
        let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for offence in views_in(&text) {
            offences.push(format!("{}:{offence}", file.display()));
        }
    }
    assert!(
        offences.is_empty(),
        "the engine reads {} typed component view(s). This is how geometry was here \
         before 2026-09-16 — `Pose` to resolve a position, `Region` to build a \
         rectangle — and it is how it would come back. No component view is allowed:\n{}",
        offences.len(),
        offences.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The scanner itself, over snippets written here: a scan is only worth its run
// if it can fail, and every way it can fail runs through the two functions
// above.
// ---------------------------------------------------------------------------

/// A register small enough to read: two of the protocol's own names and one
/// dotted vendor name.
fn register() -> BTreeSet<String> {
    ["pose", "storage", "quicktron.easyloop"].into_iter().map(str::to_string).collect()
}

#[test]
fn a_name_as_the_whole_literal_or_as_a_key_is_caught() {
    assert_eq!(named_in(r#"let p = entity.raw("pose");"#, &register()), ["1: `pose`, the whole literal"]);
    // As it appears in a Rust source: the quotes around the key are escaped,
    // and that backslash is what a whole-literal check reads past.
    assert_eq!(named_in("let body = \"{\\\"pose\\\": {}}\";", &register()), ["1: `\"pose\"`, a key"]);
    assert_eq!(named_in(r##"let j = r#"{"pose": {}}"#;"##, &register()), ["1: `\"pose\"`, a key"]);
    // Ownership is not an exemption: a namespaced name is a component name.
    assert_eq!(
        named_in(r#"let c = "quicktron.easyloop";"#, &register()),
        ["1: `quicktron.easyloop`, the whole literal"]
    );
}

#[test]
fn prose_and_a_field_path_example_are_not_caught() {
    // `storage` is in the register and is also an ordinary English word. What
    // separates the two is the form, not a list of words to forgive.
    assert!(named_in(r#"let why = "more than the storage this scope allows";"#, &register()).is_empty());
    assert!(named_in("/// reads `pose.position`, as \"pose\".\nfn f() {}", &register()).is_empty());
    assert!(named_in("/* block: \"pose\" */\nfn f() {}", &register()).is_empty(), "a block comment is a comment");
    // A field path is not the component's name: which leading run of it names a
    // component is decided against each entity, so a path in a message teaches
    // a spelling rather than deciding one.
    assert!(named_in(r##"let m = "as in ref_walk(c, m, \"pose.frame\")";"##, &register()).is_empty());
}

/// A quote inside a char literal does not open a string, so what follows it is
/// still read as the code it is.
///
/// This is the case the scanner was blind to: `parse.rs` writes `'"'` twice, and
/// reading either quote as a string's opening flips literal and code apart for
/// every run after it — roughly five hundred lines of the parser, the one file
/// where a special-cased built-in would most plausibly come back.
#[test]
fn a_name_after_a_char_literal_is_still_caught() {
    let quoted = r#"fn f(c: char) -> bool {
    if c == '"' { return true; }
    let p = "pose";
    false
}"#;
    assert_eq!(named_in(quoted, &register()), ["3: `pose`, the whole literal"]);

    // The escaped forms, and a lifetime — which opens with the same character
    // and is not a literal at all, so reading it as one would swallow the code
    // up to the next quote.
    let escaped = r#"fn f<'a>(s: &'a str) -> char {
    let q = '\'';
    let b = '\\';
    let n = '\u{7f}';
    let _ = "storage";
    q
}"#;
    assert_eq!(named_in(escaped, &register()), ["5: `storage`, the whole literal"]);
}

#[test]
fn every_component_view_on_a_line_is_caught() {
    let together = "use site_model::components::product::CellAddress; use site_model::components::spatial::Pose;";
    assert_eq!(views_in(together), ["1: components::product::CellAddress", "1: components::spatial::Pose"]);
}

/// The module coming in whole, under another name or inside a group, is every
/// view coming in, and it says `components` without the `::` the item scan
/// looks for.
#[test]
fn the_module_imported_whole_is_caught_where_it_is_declared() {
    assert_eq!(views_in("use site_model::components as c;"), ["1: `components` imported as a module"]);
    assert_eq!(views_in("use site_model::{components, EntityId};"), ["1: `components` imported as a module"]);
    assert!(views_in("use site_model::EntityId;").is_empty());
}

#[test]
fn a_typed_view_is_caught_but_prose_is_not() {
    assert_eq!(views_in("use site_model::components::spatial::Pose;"), ["1: components::spatial::Pose"]);
    assert!(
        views_in("// once read site_model::components::spatial::Region").is_empty(),
        "prose may name what the crate stopped reading"
    );
}
