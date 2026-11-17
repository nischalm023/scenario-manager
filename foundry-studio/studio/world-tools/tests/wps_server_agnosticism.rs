//! Agnosticism: that no component name is written into a string literal in the
//! server's sources, into the vocabulary it shares with its clients, or into a
//! column or constraint of its layout — and no adapter's name, no file name an
//! adapter writes, and no spelling of Map Creator into those sources either.
//!
//! That sentence is the whole of what these tests give, and it is narrower than
//! the ruling behind them. Daniel's agnosticism ruling of 2026-09-09 says the
//! server knows nothing about any specific component, field path or entity
//! kind — only the registry, the scopes, the changesets and the
//! entity/component graph are its business — and no test can check a claim
//! that wide. What these check is the shape the ruling is lost in: somebody
//! reaches for a name that happens to work, and it lands in the code as a
//! written-down string. So a name spelled out where a decision can read it
//! fails here, and everything else is out of reach — a name assembled from
//! pieces, read out of a file at run time, or spelled in a `const` these do
//! not scan, all pass. That is a floor, not a proof.
//!
//! Prose is exempt on purpose. The charters here talk about poses and tilings
//! constantly and should keep doing so; a name only becomes a decision when it
//! is a literal. There is no whitelist of English words behind that, only the
//! rule that a comment is not code and a sentence is not a name.
//!
//! **Scope: WPS `server/src` and `core/src`.** Native application expansion now
//! lives in Foundry's tooling. This retained guard covers the server and shared
//! wire types; it does not claim to scan every WPS crate. `core` is in scope
//! because it is the vocabulary both sides speak — `EntityRef`, scopes,
//! changeset operations — and a component name appearing there would put one
//! in the wire format.
//!
//! What the store may know: the keys of a document entry — the five it stores
//! and the one it drops — and the keys of the block a document opens with,
//! because parsing a document means knowing what it is made of. None of them is
//! a component name today, so the exemption below is a statement of the
//! boundary rather than a live escape hatch — see `ENTRY_KEYS` and
//! `DROPPED_KEY` in `storage/documents.rs`. The legacy `primary_frame` prologue
//! field remains pending the separate scope-format migration; this lexical
//! scan does not prove that the remaining wire format is application-agnostic.
//!
//! No database, and no `mod common`: these read files.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[path = "../../test-support/wps_source.rs"]
mod wps_source;

/// The document entry keys the store is allowed to know by name, because
/// `storage::documents` has to read them to parse a world document at all.
///
/// Kept in step with `ENTRY_KEYS` and `DROPPED_KEY` there by hand — the five
/// keys it stores and the one it drops (`expect`, DESIGN.md §9). None of them
/// is a component name today, so this subtraction removes nothing — it is here
/// to say where the boundary is, and it would start doing work the day the
/// vocabulary minted a component called `name`.
const ENTRY_KEYS: [&str; 6] = ["id", "name", "components", "suppress", "derivation", "expect"];

/// The SQL files that ARE the layout, and so must not name a component in a
/// column or a constraint: the tables themselves, and the column moves that
/// carry an older database up to them.
const LAYOUT_FILES: [&str; 2] = ["schema.sql", "upgrades.sql"];

/// The resolved WPS server source; the application vocabulary stays in Foundry.
fn crate_dir() -> PathBuf {
    wps_source::root().join("server/server")
}

/// Every component name this version of the protocol defines.
///
/// Read out of the protocol's schema layer rather than listed here: the point
/// is that the server carries no such list, and a test that carried one would
/// be the very thing it forbids — as well as going stale the day the spec mints
/// a name.
fn component_names() -> BTreeSet<String> {
    let path: PathBuf = [Path::new(env!("CARGO_MANIFEST_DIR")), Path::new("../../data/seed/application/schema/components.json")].iter().collect();
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let schema: Value = serde_json::from_str(&text).expect("components.json is JSON");
    let names = schema["$defs"]["core_name"]["enum"]
        .as_array()
        .expect("components.json#/$defs/core_name enumerates the protocol's component names");
    let names: BTreeSet<String> =
        names.iter().map(|name| name.as_str().expect("a component name is a string").to_string()).collect();
    assert!(
        names.len() > 20,
        "the protocol's component register came back with {} names, which is not \
         a vocabulary — has `$defs.core_name.enum` moved?",
        names.len()
    );
    names
}

/// The names the scan actually looks for: the register, less the entry keys the
/// store is allowed to spell.
///
/// Subtracted here rather than skipped at each rule, so no rule can forget the
/// exemption and no rule can widen it.
fn watched_names() -> BTreeSet<String> {
    let mut names = component_names();
    for key in ENTRY_KEYS {
        names.remove(key);
    }
    names
}

/// Whether `needle` stands at `i` in `chars`.
fn at(chars: &[char], i: usize, needle: &str) -> bool {
    needle.chars().enumerate().all(|(offset, want)| chars.get(i + offset) == Some(&want))
}

/// Every string literal in one Rust source, each with the line it opens on.
///
/// Character by character, because what has to be told apart is a literal from
/// a comment and from code, and that is a lexical question rather than a
/// syntactic one — no parser is involved and none is wanted. The state machine
/// reads `//` to end of line, `/* … */` to its match (nested, as Rust nests
/// them), a raw string `r#"…"#` to the quote with the same run of hashes, and
/// an ordinary `"…"` to the first quote no backslash precedes.
///
/// The text comes back exactly as it stands in the source, escapes and all,
/// because the escapes are part of what is being looked for: a JSON key inside
/// an ordinary Rust string is written `\"name\"` and inside a raw string
/// `"name"`, and un-escaping would hide which of the two was written.
fn literals(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut comment_depth = 0usize;
    while i < chars.len() {
        if chars[i] == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if comment_depth > 0 {
            if at(&chars, i, "/*") {
                comment_depth += 1;
                i += 2;
            } else if at(&chars, i, "*/") {
                comment_depth -= 1;
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
            comment_depth = 1;
            i += 2;
            continue;
        }
        // A raw string opens with `r`, any run of `#`, then the quote — and the
        // `r` has to start a token, or the one ending an identifier just before
        // a string (`author"`) would open a raw string that never closes.
        let opens_a_token = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if chars[i] == 'r' && opens_a_token {
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                let (text, end) = read_raw(&chars, j + 1, hashes);
                found.push((line, text.clone()));
                line += text.matches('\n').count();
                i = end;
                continue;
            }
        }
        if chars[i] == '"' {
            let (text, end) = read_quoted(&chars, i + 1);
            found.push((line, text.clone()));
            line += text.matches('\n').count();
            i = end;
            continue;
        }
        i += 1;
    }
    found
}

/// An ordinary string literal's text, from just past its opening quote, and
/// where the source continues after the closing one.
fn read_quoted(chars: &[char], from: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            // Both characters, so `\"` survives into the text and a search for
            // a JSON key finds it there.
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

/// A raw string literal's text, from just past its opening quote, and where the
/// source continues after the closing quote and its run of `hashes`.
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

/// The words that follow a `->` or `->>` in one literal, quotes stepped over.
///
/// Postgres reaches into a `jsonb` payload with those two operators, and what
/// sits to the right of one is a key — the thing the server would be naming if
/// it named a component at all. Any quoting around the key is stepped over
/// rather than required, so `->'pose'`, `->> "pose"` and a bare `-> pose` are
/// one case.
fn after_arrows(literal: &str) -> Vec<String> {
    let chars: Vec<char> = literal.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !at(&chars, i, "->") {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        while matches!(chars.get(j), Some('>' | ' ' | '\t' | '\'' | '"' | '\\')) {
            j += 1;
        }
        let mut word = String::new();
        while let Some(&c) = chars.get(j) {
            if !(c.is_ascii_alphanumeric() || c == '_' || c == '.') {
                break;
            }
            word.push(c);
            j += 1;
        }
        if !word.is_empty() {
            words.push(word);
        }
        i += 2;
    }
    words
}

/// Every place one of `names` is spelled out inside a string literal of
/// `source`, as `line: what was found`.
///
/// Four forms, because a name reaches a decision in four spellings and the
/// literal's own text is only the first of them. A whole literal equal to a
/// name is the plain case — `row.get("pose")`. A name in single quotes is a SQL
/// literal, which is how a name would reach a `where` clause or a `check`
/// constraint. A name in double quotes inside the literal is a JSON key, the
/// form it takes in a payload the server builds or matches. And a name after a
/// `jsonb` arrow is the server reaching into a payload it is not supposed to
/// understand.
fn offences_in(source: &str, names: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    for (line, literal) in literals(source) {
        if names.contains(&literal) {
            found.push(format!("{line}: `{literal}`, the whole literal"));
        }
        for name in names {
            if literal.contains(&format!("'{name}'")) {
                found.push(format!("{line}: `'{name}'`, a SQL literal"));
            }
            // Two spellings of the same key: a raw string writes the quotes
            // plainly, an ordinary one escapes each of them.
            if literal.contains(&format!("\"{name}\"")) || literal.contains(&format!("\\\"{name}\\\"")) {
                found.push(format!("{line}: `\"{name}\"`, a JSON key"));
            }
        }
        for word in after_arrows(&literal) {
            if names.contains(&word) {
                found.push(format!("{line}: `{word}`, read out of a payload by a jsonb arrow"));
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
        let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
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

#[test]
fn no_string_literal_in_the_server_or_the_shared_vocabulary_names_a_component() {
    let names = watched_names();
    let mut offences = Vec::new();
    for root in ["src", "../core/src"] {
        let root: PathBuf = [crate_dir().as_path(), Path::new(root)].iter().collect();
        for file in rust_files(&root) {
            let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            for offence in offences_in(&text, &names) {
                offences.push(format!("{}:{offence}", file.display()));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the server spells {} component name(s) it is not allowed to know (the \
         agnosticism ruling of 2026-09-09). Move the knowledge to a client, or \
         make it a fact the registry carries:\n{}",
        offences.len(),
        offences.join("\n")
    );
}

#[test]
fn no_column_or_constraint_in_the_layout_names_a_component() {
    // The same absence, one layer down: a column called `pose_x`, or a check
    // constraint listing component names, would be the layout deciding what a
    // component is — which is what `component_def` and its `jsonb` payloads
    // exist to avoid (schema.sql's own header). Comments are stripped first,
    // because the header explains the layout in the vocabulary's own words and
    // should keep doing so.
    let names = watched_names();
    let mut offences = Vec::new();
    for file in LAYOUT_FILES {
        let path: PathBuf = [crate_dir().as_path(), Path::new(file)].iter().collect();
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (number, line) in text.lines().enumerate() {
            let code = line.split("--").next().unwrap_or_default();
            for word in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if names.contains(word) {
                    offences.push(format!("{file}:{}: `{word}`", number + 1));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the layout names {} component(s) (the agnosticism ruling of \
         2026-09-09):\n{}",
        offences.len(),
        offences.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Adapters and formats. The ruling reaches a team's format as it reaches a
// component: an adapter is a module a team registers and the server runs
// (DESIGN.md §7, amended 2026-10-02), so the server knows no adapter by name
// and no format by its files. A second watch list is scanned exactly as the
// component names are: what the repository's own adapters call themselves,
// read out of their sources — the WPS round-trip probe and Foundry's adapters — the file
// names the round-trip probe writes, and the phrase Map Creator in the spellings
// a name takes.
// ---------------------------------------------------------------------------

/// The round-trip probe replaces the removed application adapter in this guard.
/// Host fault probes are unit-test inputs, not application formats.
const ROUNDTRIP_PROBE: &str = "adapter/host/probes/records";

/// The phrase Map Creator, in each spelling a literal could give it.
const MAP_CREATOR: [&str; 7] =
    ["Map Creator", "MapCreator", "mapCreator", "mapcreator", "map_creator", "map-creator", "MAP_CREATOR"];

/// One of the repository's own adapters, as its sources state it.
struct OwnAdapter {
    /// What its `NAME` constant holds — the name the SDK's `Adapter` trait
    /// asks for and a registry holds the module under.
    name: String,
    /// The files it writes, each the first argument of a `File::new` call: a
    /// literal, or a constant its sources define as one.
    files: Vec<String>,
}

/// Every adapter crate under `root` — a directory holding a `Cargo.toml` and a
/// `src` whose sources state a `NAME` — in name order, build products skipped.
fn own_adapters(root: &Path) -> Vec<OwnAdapter> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
                stack.push(path);
            }
        }
        if !dir.join("Cargo.toml").is_file() || !dir.join("src").is_dir() {
            continue;
        }
        let sources: Vec<String> = rust_files(&dir.join("src"))
            .iter()
            .map(|file| fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display())))
            .collect();
        let Some(name) = sources.iter().find_map(|text| constant(text, "NAME")) else {
            continue;
        };
        let files = sources.iter().flat_map(|text| written_files(text, &sources)).collect();
        found.push(OwnAdapter { name, files });
    }
    found.sort_by(|one, other| one.name.cmp(&other.name));
    found
}

/// The literal a source gives the constant `name`: the first one opening on the
/// line that declares it.
fn constant(source: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let declares = format!("const {name}:");
    literals(source)
        .into_iter()
        .find(|(line, _)| lines.get(line - 1).is_some_and(|text| text.contains(&declares)))
        .map(|(_, literal)| literal)
}

/// The names of the files one source writes, reading the first argument of
/// each `File::new` call as a literal or as a constant `sources` define.
fn written_files(source: &str, sources: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for (at, call) in source.match_indices("File::new(") {
        let argument = source[at + call.len()..].trim_start();
        if let Some(quoted) = argument.strip_prefix('"') {
            names.extend(quoted.split_once('"').map(|(literal, _)| literal.to_string()));
            continue;
        }
        let path: String = argument.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':').collect();
        let name = path.rsplit("::").next().unwrap_or_default();
        names.extend(sources.iter().find_map(|text| constant(text, name)));
    }
    names
}

/// The second watch list: these adapters' names, the round-trip probe's file names, and
/// Map Creator's spellings.
fn adapter_watch() -> BTreeSet<String> {
    let mut names: BTreeSet<String> = MAP_CREATOR.iter().map(|spelling| spelling.to_string()).collect();
    let roots = [
        (wps_source::root().join(ROUNDTRIP_PROBE), true),
        (PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../world-adapters"), false),
    ];
    for (root, watch_files) in roots {
        let adapters = own_adapters(&root);
        assert!(!adapters.is_empty(), "no adapter sources found under {}", root.display());
        for adapter in adapters {
            names.insert(adapter.name);
            if watch_files {
                names.extend(adapter.files);
            }
        }
    }
    names
}

/// The repository root, from this crate's place in it.
fn repository() -> PathBuf {
    [crate_dir().as_path(), Path::new("../..")].iter().collect()
}

#[test]
fn no_string_literal_in_the_server_or_the_shared_vocabulary_names_an_adapter_or_a_format() {
    // A reading that found nothing would pass every scan below, so it is
    // checked first: the probes include an adapter that writes a file.
    let seed = own_adapters(&repository().join(ROUNDTRIP_PROBE));
    assert!(
        seed.iter().any(|adapter| !adapter.files.is_empty()),
        "the test sources gave {} adapter(s) and no file name — has the SDK's `NAME` or `File::new` moved?",
        seed.len()
    );
    let names = adapter_watch();
    let mut offences = Vec::new();
    for root in ["src", "../core/src"] {
        let root: PathBuf = [crate_dir().as_path(), Path::new(root)].iter().collect();
        for file in rust_files(&root) {
            let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            for offence in offences_in(&text, &names) {
                offences.push(format!("{}:{offence}", file.display()));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the server spells {} adapter or format name(s) it is not allowed to know: a format is its team's, and \
         registering an adapter teaches the server none (DESIGN.md §7, amended 2026-10-02). Move the knowledge \
         into the adapter:\n{}",
        offences.len(),
        offences.join("\n")
    );
}

/// The second list catches what it is for: an adapter's name, a file one
/// writes and Map Creator, planted in a branch the way a shortcut would put
/// them, each named as the component scan names what it finds.
#[test]
fn a_planted_adapter_name_file_name_or_map_creator_is_caught() {
    let seed = own_adapters(&repository().join(ROUNDTRIP_PROBE));
    let adapter = seed.iter().find(|adapter| !adapter.files.is_empty()).expect("a test adapter that writes a file");
    let source = format!(
        "fn route(adapter: &str, file: &str) -> bool {{\n    adapter == {:?} || file == {:?} || adapter == \"Map Creator\"\n}}",
        adapter.name, adapter.files[0]
    );
    assert_eq!(
        offences_in(&source, &adapter_watch()),
        vec![
            format!("2: `{}`, the whole literal", adapter.name),
            format!("2: `{}`, the whole literal", adapter.files[0]),
            "2: `Map Creator`, the whole literal".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// The scanner itself, over snippets written here. The scan above is only worth
// its run if it can fail, and every way it can fail runs through `offences_in`.
// ---------------------------------------------------------------------------

/// A register to scan against, small enough to read: two of the protocol's own
/// names and one dotted vendor name.
fn register() -> BTreeSet<String> {
    ["pose", "storage", "quicktron.easyloop"].into_iter().map(str::to_string).collect()
}

#[test]
fn a_name_as_the_whole_literal_is_caught() {
    let found = offences_in(r#"let payload = row.get("pose");"#, &register());
    assert_eq!(found, vec!["1: `pose`, the whole literal".to_string()]);
}

#[test]
fn a_name_as_a_sql_literal_is_caught() {
    let found =
        offences_in(r#"sqlx::query("select 1 from component_instance where component = 'pose'");"#, &register());
    assert_eq!(found, vec!["1: `'pose'`, a SQL literal".to_string()]);
}

#[test]
fn a_name_as_a_json_key_is_caught() {
    // As it appears in a Rust source: the quotes around the key are escaped,
    // and that backslash is exactly what a whole-literal check reads past.
    let found = offences_in("let body = \"{\\\"pose\\\": {\\\"position\\\": [0, 0]}}\";", &register());
    assert_eq!(found, vec!["1: `\"pose\"`, a JSON key".to_string()]);
}

#[test]
fn a_name_after_a_jsonb_arrow_is_caught() {
    let source = "sqlx::query(\n    \"select ci.payload -> 'pose' from component_instance ci\",\n);";
    let found = offences_in(source, &register());
    assert_eq!(
        found,
        vec!["2: `'pose'`, a SQL literal".to_string(), "2: `pose`, read out of a payload by a jsonb arrow".to_string(),],
        "the same name is both, and a scan that reported one form would go \
         quiet the day the other spelling is used"
    );
    // The spelling neither of the other rules sees: no quotes at all.
    assert_eq!(
        offences_in("let q = \"select payload ->> pose from t\";", &register()),
        vec!["1: `pose`, read out of a payload by a jsonb arrow".to_string()]
    );
}

#[test]
fn a_dotted_vendor_name_is_caught_like_any_other() {
    // Ownership is not a rung and it is not an exemption either: a namespaced
    // name is a component name, and the server may not spell it.
    let found = offences_in(r#"let c = "quicktron.easyloop";"#, &register());
    assert_eq!(found, vec!["1: `quicktron.easyloop`, the whole literal".to_string()]);
    assert_eq!(
        offences_in("let q = \"select payload -> 'quicktron.easyloop' from t\";", &register()),
        vec![
            "1: `'quicktron.easyloop'`, a SQL literal".to_string(),
            "1: `quicktron.easyloop`, read out of a payload by a jsonb arrow".to_string(),
        ],
        "the dot is part of the name, so the word after an arrow is read whole"
    );
}

#[test]
fn an_english_sentence_using_a_name_as_a_word_is_not_caught() {
    // `storage` is in the register and is also an ordinary English word. What
    // separates the two is the form, not a list of words to forgive: a
    // sentence is not a literal equal to the name, not a quoted key, and not
    // the right-hand side of an arrow.
    assert!(
        offences_in(r#"let why = "the upload exceeded the storage this scope is allowed";"#, &register(),).is_empty()
    );
}

#[test]
fn a_name_in_a_comment_is_not_caught() {
    // Prose may name a component freely — the charters here do it constantly.
    assert!(offences_in("/// reads `pose.position` off the target, as \"pose\".\nfn f() {}", &register(),).is_empty());
    assert!(
        offences_in("/* block: \"pose\" */\nfn f() {}", &register()).is_empty(),
        "a block comment is a comment too"
    );
    assert!(
        offences_in(r#"let url = "http://host/x"; // "pose""#, &register()).is_empty(),
        "a `//` inside a literal does not open a comment, and the comment after \
         the literal is dropped"
    );
}

#[test]
fn a_multi_line_literal_and_a_raw_string_are_read_whole() {
    // The SQL in this crate is written as one literal across many lines, and
    // that is where a name would sit. A line-at-a-time scan saw the opening
    // line and read the rest as code.
    let source =
        "let q = sqlx::query(\n    \"select 1\n     from component_instance\n     where component = 'pose'\",\n);";
    assert_eq!(
        offences_in(source, &register()),
        vec!["2: `'pose'`, a SQL literal".to_string()],
        "the literal opens on line 2 and the name sits on line 4; the line \
         reported is the literal's, which is what a reader greps for"
    );
    assert_eq!(
        offences_in("let j = r#\"{\"pose\": {}}\"#;", &register()),
        vec!["1: `\"pose\"`, a JSON key".to_string()],
        "a raw string writes its JSON keys with no backslash, and the same \
         search has to find them"
    );
}
