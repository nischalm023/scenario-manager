//! Step 1 of SPEC §8: turning a scenario file into the ordered list of documents
//! it composes.
//!
//! A scenario says what a run *is* by naming layers in order, so this step is
//! where "what does this scenario mean" stops being a question about the file
//! system. It resolves each path against the scenario's own directory, refuses a
//! layer listed twice or reachable from itself, and checks that every document's
//! stated dependencies are actually present — because a document that references
//! into a catalog nobody loaded would otherwise fail later, as a pile of
//! dangling references rather than one missing file.

use crate::document::Document;
use crate::error::LoadError;
use crate::ids::DocId;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// A scenario and the documents it composes, in the order the pipeline reads
/// them.
#[derive(Debug, Clone)]
pub struct LayerSet {
    pub scenario: Document,
    pub layers: Vec<Document>,
}

impl LayerSet {
    /// Every document taking part, layers first and the scenario last: a
    /// scenario may carry records of its own, and they are the final word.
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.layers.iter().chain(std::iter::once(&self.scenario))
    }
}

/// Read a scenario and everything it composes.
pub fn resolve(scenario_path: &Path) -> Result<LayerSet, LoadError> {
    resolve_from(Document::load(scenario_path)?)
}

/// Everything a scenario already read composes, each layer read from beside
/// the file the scenario was read from: for a caller that has read the file
/// once already, to tell what it is.
pub fn resolve_from(scenario: Document) -> Result<LayerSet, LoadError> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    let mut layers = Vec::new();
    collect(&scenario, &mut seen, &mut stack, &mut layers)?;
    check_dependencies(&scenario, &layers)?;
    Ok(LayerSet { scenario, layers })
}

fn collect(
    from: &Document,
    seen: &mut BTreeSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
    out: &mut Vec<Document>,
) -> Result<(), LoadError> {
    let base = from
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let here = normalize(&from.path);
    stack.push(here);
    for layer in &from.layers {
        let path = normalize(&base.join(layer));
        if stack.contains(&path) {
            stack.pop();
            return Err(LoadError::LayerCycle { path });
        }
        if !seen.insert(path.clone()) {
            stack.pop();
            return Err(LoadError::DuplicateLayer { path });
        }
        let document = Document::load(&path)?;
        // A layer may itself compose layers; the stack above is what stops that
        // from becoming an infinite regress.
        collect(&document, seen, stack, out)?;
        out.push(document);
    }
    stack.pop();
    Ok(())
}

/// Lexical normalization only — `a/./b` and `a/b` are the same layer. This
/// deliberately does not touch the file system, so a scenario that lists a
/// missing file reports the path as written.
fn normalize(path: &Path) -> PathBuf {
    let mut parts: Vec<OsString> = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only a real name can be cancelled; a leading `..` has nothing
                // above it and must survive, or the path stops naming the file.
                match parts.last().map(|p| p.as_os_str() == "..") {
                    Some(false) => {
                        parts.pop();
                    }
                    _ => parts.push("..".into()),
                }
            }
            other => parts.push(other.as_os_str().to_os_string()),
        }
    }
    parts.iter().collect()
}

/// `doc.depends_on` names the documents this one references into; all of them
/// must be present for the composition to load standalone (SPEC §8).
fn check_dependencies(scenario: &Document, layers: &[Document]) -> Result<(), LoadError> {
    let present: BTreeSet<DocId> = layers
        .iter()
        .map(|d| d.meta.id)
        .chain(std::iter::once(scenario.meta.id))
        .collect();
    for document in layers.iter().chain(std::iter::once(scenario)) {
        for dependency in &document.meta.depends_on {
            if !present.contains(dependency) {
                return Err(LoadError::MissingDependency {
                    doc: document.meta.id,
                    dependency: *dependency,
                });
            }
        }
    }
    Ok(())
}
