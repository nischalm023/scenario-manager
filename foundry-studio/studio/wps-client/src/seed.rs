//! Within `wps-client`, `seed` is spec 04 section 3.13's mechanism: one known set of templates and catalog items put
//! into a fresh development WPS by one command, `studio-seed`. The entries are spec 05's (I1.07 writes them); this
//! module checks our vocabulary is registered as design/vocabulary/ holds it, skips every entry whose scope already
//! holds a changeset, never overwriting, and writes the rest through WPS's own import. An entry's document is written
//! against the documents it can reference (a flow names its map's sectors), so the seed writes nothing at all when a
//! scope that an entry to write can reference holds no changeset seeded from the manifest's text of its document. It
//! defines no component and writes no Postgres row.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use studio_protocol::{Kinship, ScopePath, Seq};
use thiserror::Error;

use crate::client::{Client, WorldStore};
use crate::error::WpsError;
use crate::kinship::ScopeTree;
use crate::site::{ensure_prefixes, is_root};
use crate::value::Author;
use crate::wps::Wps;

/// The manifest's own version.
pub const MANIFEST_VERSION: u32 = 1;
/// The version every component of our vocabulary is published at while the format is unreleased.
pub const VOCABULARY_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedManifest {
    pub v: u32,
    pub vocabulary: Vec<String>,
    pub entries: Vec<SeedEntry>,
}

/// One world document under the manifest's folder, written into one scope.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedEntry {
    pub scope: ScopePath,
    pub document: String,
}

/// What a run wrote and skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedReport {
    pub written: Vec<(ScopePath, Seq)>,
    pub skipped: Vec<ScopePath>,
    pub warnings: Vec<String>,
}

/// Why the seed stopped; each names what stopped it.
#[derive(Debug, Error)]
pub enum SeedError {
    #[error("the manifest {path}: {why}")]
    Manifest { path: PathBuf, why: String },
    #[error(
        "component `{component}` {why}; reset the development WPS (dev.sh --stop --db), whose own seed registers \
         Agent Studio's vocabulary"
    )]
    Vocabulary { component: String, why: String },
    #[error("entry {scope} ({document}) stopped the seed: {why}")]
    Entry { scope: ScopePath, document: String, why: String },
    /// A held scope that an entry still to write can reference, holding no changeset seeded from this manifest's text
    /// of its document: seeded from another text, or written otherwise.
    #[error(
        "{held} holds no changeset seeded from this manifest's text of {document}, so the seed wrote nothing, not even \
         {unwritten}, which can reference it. Where the WPS's database is in dev.sh's own Postgres container, \
         ./dev.sh --stop --db and the next start seed afresh, deleting that stack's runs and the container's rows; a \
         WPS database kept anywhere else must be reset where it is kept"
    )]
    UnmatchedHeldScope { held: ScopePath, document: String, unwritten: ScopePath },
    #[error(transparent)]
    Wps(#[from] WpsError),
}

fn manifest_fault(path: &Path, why: impl ToString) -> SeedError {
    SeedError::Manifest { path: path.to_path_buf(), why: why.to_string() }
}

/// The definition file of `component` under `vocabulary`, at any depth.
fn definition_file(vocabulary: &Path, component: &str) -> Option<PathBuf> {
    let wanted = format!("{component}.json");
    let mut folders = vec![vocabulary.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.file_name().is_some_and(|name| name == wanted.as_str()) {
                return Some(path);
            }
        }
    }
    None
}

async fn check_vocabulary<S: Wps>(wps: &S, names: &[String], vocabulary: Option<&Path>) -> Result<(), SeedError> {
    for name in names {
        let stop = |why: String| SeedError::Vocabulary { component: name.clone(), why };
        let Some(registered) = wps.component(name).await? else {
            return Err(stop("is not registered in world-proto-server".to_owned()));
        };
        let Some(published) = registered.versions.iter().find(|version| version.version == VOCABULARY_VERSION) else {
            return Err(stop(format!("has no schema published at {VOCABULARY_VERSION}")));
        };
        let Some(folder) = vocabulary else { continue };
        let file = definition_file(folder, name)
            .ok_or_else(|| stop(format!("has no definition file under {}", folder.display())))?;
        let text = fs::read_to_string(&file).map_err(|fault| stop(format!("{}: {fault}", file.display())))?;
        let defined: Value =
            serde_json::from_str(&text).map_err(|fault| stop(format!("{}: {fault}", file.display())))?;
        let refs = serde_json::to_value(&published.refs).unwrap_or(Value::Null);
        if defined.get("schema") != Some(&published.schema)
            || defined.get("refs").unwrap_or(&Value::Array(Vec::new())) != &refs
        {
            return Err(stop(format!("is registered with a schema other than {}", file.display())));
        }
    }
    Ok(())
}

/// The identities of the documents `document`'s prologue says it references into (`doc.depends_on`).
fn depends_on(document: &Value) -> impl Iterator<Item = &str> {
    document.pointer("/doc/depends_on").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
}

/// Refuses, before anything is written, an entry to write beside a held scope it can reference that holds no
/// changeset seeded from this manifest's text of its document; a held scope no entry to write can reference is not
/// asked about. An entry can reference at head a scope kin to its own, and a scope of another unit by a pin into its
/// document. The check trusts each document's `depends_on` to name every document it pins into; world-proto-server
/// does not check that. It asks only about the scopes an entry to write can reference directly, and does not follow
/// what such a scope itself pins: a new entry kin to a map seeded from this manifest's text passes the check although
/// a catalog item that map stamps was seeded from another text, and world-proto-server may then refuse it partway
/// through the writes. Nor does it establish that the stored document still satisfies the entries to write after
/// later edits to it, or that no other writer acts between the check and the writes.
async fn check_referable_held_scopes<S: Wps>(
    client: &Client<S>,
    entries: &[SeedEntry],
    documents: &[(Value, String)],
    tree: &ScopeTree,
    held: &FxHashSet<ScopePath>,
) -> Result<(), SeedError> {
    let unwritten: Vec<(&SeedEntry, &Value)> = entries
        .iter()
        .zip(documents)
        .filter(|(entry, _)| !held.contains(&entry.scope))
        .map(|(entry, (document, _))| (entry, document))
        .collect();
    for (entry, (document, operation)) in entries.iter().zip(documents) {
        if !held.contains(&entry.scope) {
            continue;
        }
        let id = document.pointer("/doc/id").and_then(Value::as_str);
        let can_reference = |(pending, its_document): &&(&SeedEntry, &Value)| {
            tree.kinship(&pending.scope, &entry.scope) == Kinship::Kin
                || depends_on(its_document).any(|named| Some(named) == id)
        };
        let Some((pending, _)) = unwritten.iter().find(can_reference) else { continue };
        if client.wps().changeset_named(&entry.scope, operation).await?.is_none() {
            return Err(SeedError::UnmatchedHeldScope {
                held: entry.scope.clone(),
                document: entry.document.clone(),
                unwritten: pending.scope.clone(),
            });
        }
    }
    Ok(())
}

/// Seed the server `client` speaks to from the manifest at `manifest`, recording `commit` in each message.
pub async fn run<S: Wps>(
    client: &Client<S>,
    manifest: &Path,
    vocabulary: Option<&Path>,
    commit: &str,
) -> Result<SeedReport, SeedError> {
    let text = fs::read_to_string(manifest).map_err(|fault| manifest_fault(manifest, fault))?;
    let parsed: SeedManifest = serde_json::from_str(&text).map_err(|fault| manifest_fault(manifest, fault))?;
    if parsed.v != MANIFEST_VERSION {
        return Err(manifest_fault(manifest, format!("`v` is {}, and this seed reads {MANIFEST_VERSION}", parsed.v)));
    }
    check_vocabulary(client.wps(), &parsed.vocabulary, vocabulary).await?;
    let folder = manifest.parent().unwrap_or(Path::new("."));
    let mut documents = Vec::new();
    let mut entry_of_doc: FxHashMap<String, usize> = FxHashMap::default();
    for (index, entry) in parsed.entries.iter().enumerate() {
        let path = folder.join(&entry.document);
        let stop = |why: String| SeedError::Entry { scope: entry.scope.clone(), document: entry.document.clone(), why };
        let text = fs::read_to_string(&path).map_err(|fault| stop(format!("{}: {fault}", path.display())))?;
        let document: Value =
            serde_json::from_str(&text).map_err(|fault| stop(format!("{}: {fault}", path.display())))?;
        if let Some(id) = document.pointer("/doc/id").and_then(Value::as_str) {
            entry_of_doc.insert(id.to_owned(), index);
        }
        // The entry's scope and document name its one changeset, so its operation identifier is never another's.
        let digest = hex::encode(Sha256::digest(format!("{}\n{text}", entry.scope).as_bytes()));
        documents.push((document, format!("seed-{}", &digest[..16])));
    }
    for (index, (entry, (document, _))) in parsed.entries.iter().zip(&documents).enumerate() {
        if let Some(later) = depends_on(document).find(|id| entry_of_doc.get(*id).is_some_and(|at| *at > index)) {
            let why = format!("its document depends on {later}, which a later entry writes");
            return Err(SeedError::Entry { scope: entry.scope.clone(), document: entry.document.clone(), why });
        }
    }
    let tree = client.tree().await?;
    let mut held: FxHashSet<ScopePath> = parsed
        .entries
        .iter()
        .map(|entry| &entry.scope)
        .filter(|scope| tree.head(scope).is_some_and(|head| head > Seq::new(0)))
        .cloned()
        .collect();
    check_referable_held_scopes(client, &parsed.entries, &documents, &tree, &held).await?;
    let mut report = SeedReport::default();
    for (entry, (document, operation)) in parsed.entries.iter().zip(&documents) {
        let stop = |why: String| SeedError::Entry { scope: entry.scope.clone(), document: entry.document.clone(), why };
        // One read of the tree serves the loop: after it, only the loop's own writes make a scope held (other writers
        // left aside, as for the check), and `held` takes them in.
        if held.contains(&entry.scope) {
            report.skipped.push(entry.scope.clone());
            continue;
        }
        // An ancestor an earlier entry filled (a template's root above its `map`) is kept with only its flag checked;
        // the entry's own scope is made, or reused while empty.
        ensure_prefixes(client, &entry.scope).await.map_err(|fault| stop(fault.to_string()))?;
        client.ensure_scope(&entry.scope, is_root(&entry.scope)).await.map_err(|fault| stop(fault.to_string()))?;
        let proposal = client
            .propose_import(&entry.scope, Seq::new(0), document)
            .await
            .map_err(|fault| stop(fault.to_string()))?;
        report.warnings.extend(proposal.warnings.iter().map(|warning| format!("{}: {warning}", entry.scope)));
        let mut draft = proposal.draft;
        draft.author = Author::new("dev-seed").expect("a word");
        draft.message = format!("Seeded from {} at foundry {commit}", entry.document);
        draft.operation = operation.clone();
        let applied = client.apply(&draft).await.map_err(|fault| stop(fault.to_string()))?;
        let seqs: BTreeMap<ScopePath, Seq> = applied.seqs;
        held.extend(seqs.keys().cloned());
        report.written.extend(seqs);
    }
    Ok(report)
}
