//! Within `crucible`, `spec` is A SUBMISSION MADE LOADABLE: it checks that what
//! arrived is a world spec with every file it names, and lays those bytes back
//! out as a folder that `dessim::harness::prepare` reads exactly as it reads a
//! fixture on a developer's disk. The server lays a submission out twice: once
//! when it arrives, to refuse a spec that cannot run while its client is still
//! holding the socket, and once on the worker from the stored object. The first
//! folder is removed on every path, so a refusal leaves nothing behind.
//!
//! It exists because the server shares no filesystem with whoever submits. The
//! rule it keeps is that nothing is rewritten: the spec's own bytes are written
//! back verbatim and each file lands at the path its reference resolves to, so
//! the recording's embedded spec, world document and workload files are the
//! bytes that were submitted, keyed by the references that were submitted. A
//! recording made here and a recording made by headless desvis from the same
//! files carry the same documents.
//!
//! References climb out of the spec's own directory — cencora's workload sits
//! four levels up — so the spec is not written at the root of the folder but
//! deep enough inside it that every reference still lands within.

use std::io::Error as IoError;
use std::path::{Component, Path, PathBuf};

use base64::DecodeError;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Error as JsonError, Value};
use thiserror::Error;

use dessim::world_spec::WorldSpec;

use crate::protocol::SubmittedSpec;

/// The spec file's name inside the folder a submission is laid out in.
const SPEC_FILE: &str = "spec.json";

/// A submission checked and written out: where the spec is, and what it is
/// called.
#[derive(Debug, Clone)]
pub struct LaidOut {
    /// The spec file, ready to hand to the harness.
    pub spec_path: PathBuf,
    /// What to call the run.
    pub name: String,
}

/// What a submission can fail to be.
#[derive(Debug, Error)]
pub enum SpecError {
    #[error("the spec is not JSON: {0}")]
    NotJson(#[source] JsonError),
    #[error("the spec is not a world spec: {0}")]
    NotAWorldSpec(#[source] JsonError),
    #[error(
        "the spec names `{reference}` and the submission does not carry it; \
         send every file the spec names, keyed by the reference as written"
    )]
    MissingFile { reference: String },
    #[error("the bytes submitted for `{reference}` are not base64: {source}")]
    NotBase64 { reference: String, source: DecodeError },
    #[error("`{reference}` is an absolute path; a spec names its files relative to itself")]
    AbsoluteReference { reference: String },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: IoError },
}

/// Read what a submission names, refusing anything that is not a world spec.
///
/// Parsed twice on purpose. `WorldSpec` is what desvis loads and it refuses
/// unknown fields, so parsing into it here is what makes a misspelt key the
/// submit's failure rather than a queued run that fails minutes later on a
/// worker. It resolves nothing, though — `scenario` is one of five variants
/// with their own file-naming fields — so the references are read off the
/// plain JSON beside it, keyed exactly as the spec writes them.
fn references(text: &str) -> Result<Vec<String>, SpecError> {
    let raw: Value = serde_json::from_str(text).map_err(SpecError::NotJson)?;
    serde_json::from_value::<WorldSpec>(raw.clone()).map_err(SpecError::NotAWorldSpec)?;
    let world =
        raw.get("world").and_then(Value::as_str).expect("a WorldSpec has a world, and it is a path").to_string();
    let mut references = vec![world];
    references.extend(raw.get("scenario").map(workload_references).unwrap_or_default());
    Ok(references)
}

/// Write a submission out under `root`, which the caller owns and removes
/// when the run is over.
pub fn lay_out(submitted: &SubmittedSpec, root: &Path) -> Result<LaidOut, SpecError> {
    let references = references(&submitted.spec)?;

    // Deep enough that the reference climbing furthest out of the spec's own
    // directory still lands inside `root`.
    let depth = references.iter().map(|reference| climbs(reference)).max().unwrap_or(0);
    let spec_dir = (0..depth).fold(root.to_path_buf(), |dir, level| dir.join(format!("d{level}")));
    let spec_path = spec_dir.join(SPEC_FILE);
    write(&spec_path, submitted.spec.as_bytes())?;

    for reference in &references {
        let bytes = bytes_for(submitted, reference)?;
        write(&resolve(&spec_dir, reference), &bytes)?;
    }

    let name = submitted.name.clone().unwrap_or_else(|| "run".to_string());
    Ok(LaidOut { spec_path, name })
}

/// The bytes a submission carries for one reference.
fn bytes_for(submitted: &SubmittedSpec, reference: &str) -> Result<Vec<u8>, SpecError> {
    if Path::new(reference).is_absolute() {
        return Err(SpecError::AbsoluteReference { reference: reference.to_string() });
    }
    let file = submitted
        .files
        .iter()
        .find(|file| file.reference == reference)
        .ok_or_else(|| SpecError::MissingFile { reference: reference.to_string() })?;
    BASE64.decode(&file.bytes).map_err(|source| SpecError::NotBase64 { reference: reference.to_string(), source })
}

/// Every workload file a scenario block names.
///
/// The keys are the ones `dessim::scenario`'s own variants carry; a block
/// naming none of them names no files, which is what a generated or inline
/// scenario does.
fn workload_references(scenario: &Value) -> Vec<String> {
    const SLOTS: [&str; 6] = ["items_csv", "orders_csv", "bulk_csv", "outbound_csv", "onhand_csv", "stock_top_up_csv"];
    SLOTS.iter().filter_map(|slot| scenario.get(slot).and_then(Value::as_str).map(str::to_string)).collect()
}

/// How far above its own directory a reference ever reaches.
///
/// Walked rather than counted, because `a/../b.csv` has a `..` in it and never
/// leaves the directory it started in, while `../../x` leaves it twice over.
fn climbs(reference: &str) -> usize {
    let mut below = 0i64;
    let mut highest = 0i64;
    for part in Path::new(reference).components() {
        match part {
            Component::ParentDir => below -= 1,
            Component::CurDir => {}
            _ => below += 1,
        }
        highest = highest.max(-below);
    }
    highest.max(0) as usize
}

/// Where a reference written beside `spec_dir` lands, with `..` resolved as
/// text rather than by asking the filesystem — nothing exists yet to ask.
fn resolve(spec_dir: &Path, reference: &str) -> PathBuf {
    let mut path = spec_dir.to_path_buf();
    for part in Path::new(reference).components() {
        match part {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            other => path.push(other),
        }
    }
    path
}

/// Write one file, making the directories above it.
fn write(path: &Path, bytes: &[u8]) -> Result<(), SpecError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SpecError::Write { path: parent.to_path_buf(), source })?;
    }
    std::fs::write(path, bytes).map_err(|source| SpecError::Write { path: path.to_path_buf(), source })
}

#[cfg(test)]
mod tests {
    use super::{SpecError, climbs, lay_out, resolve, workload_references};
    use crate::protocol::{SubmittedFile, SubmittedSpec};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use std::path::Path;

    fn file(reference: &str, bytes: &[u8]) -> SubmittedFile {
        SubmittedFile { reference: reference.to_string(), bytes: BASE64.encode(bytes) }
    }

    #[test]
    fn a_reference_climbing_out_of_the_spec_directory_still_lands_in_the_folder() {
        // cencora's workload sits four levels above its spec. Written at the
        // root of the folder, `../../../../libs/...` would land outside it.
        let spec = r#"{"world":"w.json","scenario":{"items_csv":"../../../../data/items.csv","bulk_csv":"../../../../data/bulk.csv"}}"#;
        let submitted = SubmittedSpec {
            spec: spec.to_string(),
            files: vec![
                file("w.json", b"{}"),
                file("../../../../data/items.csv", b"sku\n"),
                file("../../../../data/bulk.csv", b"sku,qty\n"),
            ],
            name: Some("cencora".to_string()),
        };
        let root = tempfile::tempdir().expect("a folder");
        let laid = lay_out(&submitted, root.path()).expect("the submission lays out");

        assert!(laid.spec_path.starts_with(root.path()));
        assert_eq!(std::fs::read_to_string(&laid.spec_path).expect("the spec"), spec);
        let bulk = resolve(laid.spec_path.parent().expect("a directory"), "../../../../data/bulk.csv");
        assert!(bulk.starts_with(root.path()), "{} should sit inside the folder", bulk.display());
        assert_eq!(std::fs::read_to_string(&bulk).expect("the workload"), "sku,qty\n");
        assert_eq!(std::fs::read_to_string(resolve(laid.spec_path.parent().unwrap(), "w.json")).unwrap(), "{}");
    }

    #[test]
    fn a_spec_that_is_not_a_world_spec_is_refused() {
        // `WorldSpec` refuses unknown fields, so a misspelt key is the submit's
        // failure rather than a queued run that fails on a worker minutes on.
        let misspelt = SubmittedSpec {
            spec: r#"{"world":"w.json","setttings":{}}"#.to_string(),
            files: vec![file("w.json", b"{}")],
            name: None,
        };
        let root = tempfile::tempdir().expect("a folder");
        assert!(matches!(lay_out(&misspelt, root.path()), Err(SpecError::NotAWorldSpec(_))));

        let not_json = SubmittedSpec { spec: "{".to_string(), files: Vec::new(), name: None };
        assert!(matches!(lay_out(&not_json, root.path()), Err(SpecError::NotJson(_))));

        // A spec that IS one passes without a folder being made anywhere.
        let good = SubmittedSpec {
            spec: r#"{"world":"w.json","scenario":{"items_csv":"i.csv","orders_csv":"o.csv"}}"#.to_string(),
            files: vec![file("w.json", b"{}"), file("i.csv", b"sku\n"), file("o.csv", b"order\n")],
            name: Some("named".to_string()),
        };
        assert_eq!(lay_out(&good, root.path()).expect("a world spec with its files").name, "named");
    }

    #[test]
    fn a_spec_whose_files_are_not_all_there_is_refused() {
        let submitted = SubmittedSpec {
            spec: r#"{"world":"w.json","scenario":{"items_csv":"items.csv","orders_csv":"orders.csv"}}"#.to_string(),
            files: vec![file("w.json", b"{}")],
            name: None,
        };
        let root = tempfile::tempdir().expect("a folder");
        let refused = lay_out(&submitted, root.path()).expect_err("items.csv is missing");
        assert!(matches!(refused, SpecError::MissingFile { ref reference } if reference == "items.csv"));
    }

    #[test]
    fn every_workload_slot_a_scenario_can_name_is_read() {
        let scenario = serde_json::json!({
            "items_csv": "i.csv", "bulk_csv": "b.csv", "outbound_csv": "o.csv",
            "onhand_csv": "h.csv", "stock_top_up_csv": "t.csv", "max_pallet_units": 50
        });
        let mut named = workload_references(&scenario);
        named.sort();
        assert_eq!(named, ["b.csv", "h.csv", "i.csv", "o.csv", "t.csv"]);
    }

    #[test]
    fn a_climb_is_counted_at_its_deepest_point() {
        assert_eq!(climbs("w.json"), 0);
        assert_eq!(climbs("../a/b.csv"), 1);
        assert_eq!(climbs("../../../../libs/x/y.csv"), 4);
        // Descending and climbing back only reaches the spec's own directory.
        assert_eq!(climbs("a/../b.csv"), 0);
        assert_eq!(resolve(Path::new("/root/d0/d1"), "../../x.csv"), Path::new("/root/x.csv"));
    }
}
