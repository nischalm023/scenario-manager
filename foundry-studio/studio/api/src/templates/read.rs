//! Within `templates`, `read` is the adapter spec 05 section 3.6 asks of the API: a site template read through the
//! world-proto-server client (its scopes each at one changeset, every catalog scope its references pin, the
//! registry's schemas and annotations, and the server's definitions document) turned into the engine's plain
//! `TemplateRead`, field for field but the client's `at` and `exact`. A template scope read below its head is
//! refused, since its labels and its prologue, which names the primary frame, would be the head's; a catalog scope
//! read below its head is kept with a note, its components being the pinned changeset's. A template at given
//! changesets never changes, so each read is kept for the next evaluation.

use std::collections::BTreeMap;
use std::sync::Arc;

use rustc_hash::FxHashSet;
use serde_json::Value;
use site_template::{
    DeclaredEntity, Prologue, Record, RefAnnotation, RegisteredComponent, RegistrySnapshot, ScopeSnapshot, TemplateRead,
};
use studio_protocol::{DeclaredId, EntityRef, KnobDeclaration, Pin, RefPin, ScopePath, Seq, TemplateDescription};
use thiserror::Error;
use wps_client::export::CLOSURE_LIMIT;
use wps_client::snapshot::{Record as ClientRecord, ScopeSnapshot as ClientSnapshot};
use wps_client::walker::{RefForm, components_read, walk};
use wps_client::wps::Wps;
use wps_client::{Client, Prologue as ClientPrologue, WorldStore, WpsError};

use super::Kept;
use super::declarations::{description_of, flows_of, knobs_of};

/// How many template states are kept read at once; past it the kept reads are dropped and read again on demand.
const KEPT_READS: usize = 32;

/// A template read whole for the engine, with what the panels show of it.
pub struct Read {
    pub template: TemplateRead,
    /// What the reads only partly hold: each catalog scope read below its head.
    pub notes: Vec<String>,
    pub description: TemplateDescription,
    /// The knob declarations, in the order the form shows them (groups by their lowest `order`, then by name).
    pub knobs: Vec<KnobDeclaration>,
    /// The names of the flow scope's material edges.
    pub flows: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ReadFault {
    #[error(transparent)]
    Wps(#[from] WpsError),
    /// A template scope the client could read only below its head, or a catalog scope pinned at two changesets.
    #[error("{0}")]
    Inexact(String),
    /// What the template's scopes hold does not read as a template.
    #[error("{0}")]
    Template(String),
    /// The template's scopes, with the catalog scopes its pins reach, are more than one read carries.
    #[error("the template and the catalog its pins reach hold more than {0} scopes")]
    TooLarge(usize),
}

/// The template `template` at `seqs`, one changeset per scope, kept or read now; refused before any scope is read when
/// its scopes are more than one flagged export of WPS's holds (`CLOSURE_LIMIT`).
pub async fn read<S: Wps>(
    kept: &Kept,
    client: &Client<S>,
    template: &ScopePath,
    seqs: &BTreeMap<ScopePath, Seq>,
) -> Result<Arc<Read>, ReadFault> {
    let key = (template.clone(), seqs.clone());
    if let Some(read) = kept.reads.lock().expect("the kept reads' lock").get(&key) {
        return Ok(read.clone());
    }
    if seqs.len() > CLOSURE_LIMIT {
        return Err(ReadFault::TooLarge(CLOSURE_LIMIT));
    }
    let mut own = Vec::new();
    for (scope, seq) in seqs {
        let snapshot = client.read(scope, *seq).await?;
        if !snapshot.exact.is_exact() {
            let text = format!(
                "The template moved after this draft read it: {scope} stands past changeset {seq}, and a read below \
                 the head carries the head's labels and prologue"
            );
            return Err(ReadFault::Inexact(text));
        }
        own.push(snapshot);
    }
    let (catalog, notes) = catalog_of(client, &own).await?;
    let registry = registry_of(kept, client, own.iter().chain(&catalog)).await?;
    let description = description_of(template, &own)?;
    let knobs = knobs_of(template, &own)?;
    let flows = flows_of(&own);
    let read = Arc::new(Read {
        template: TemplateRead {
            template: template.clone(),
            tag: None,
            scopes: own.into_iter().map(snapshot).collect::<Result<_, _>>()?,
            catalog: catalog.into_iter().map(snapshot).collect::<Result<_, _>>()?,
            registry,
        },
        notes,
        description,
        knobs,
        flows,
    });
    let mut reads = kept.reads.lock().expect("the kept reads' lock");
    if reads.len() >= KEPT_READS {
        reads.clear();
    }
    reads.insert(key, read.clone());
    Ok(read)
}

/// Every catalog scope the template's references pin, each at the pinned changeset, followed through the catalog's
/// own pins until none is new: the client's walker finds each pinned reference, world-proto-server names the scope
/// that owns it. Refused when the template's scopes and the catalog's together are more than one flagged export of
/// WPS's holds (`CLOSURE_LIMIT`).
async fn catalog_of<S: Wps>(
    client: &Client<S>,
    own: &[ClientSnapshot],
) -> Result<(Vec<ClientSnapshot>, Vec<String>), ReadFault> {
    let (mut catalog, mut notes): (Vec<ClientSnapshot>, Vec<String>) = (Vec::new(), Vec::new());
    let mut held: FxHashSet<DeclaredId> = own.iter().flat_map(|scope| scope.entities.iter().map(|e| e.id)).collect();
    let mut frontier: Vec<ClientSnapshot> = own.to_vec();
    loop {
        ensure_components(client, &frontier).await?;
        let registry = client.registry().snapshot();
        // ordered: the next hop's scopes are read in path order.
        let mut next: BTreeMap<ScopePath, Seq> = BTreeMap::new();
        for scope in &frontier {
            for entity in &scope.entities {
                for (name, payload) in &entity.components {
                    for occurrence in walk(name, payload, &*registry) {
                        let RefForm::Text(text) = occurrence.form else { continue };
                        let Ok(reference) = text.parse::<EntityRef>() else { continue };
                        let Some(RefPin::Pin(Pin::Changeset(seq))) = reference.pin() else { continue };
                        if held.contains(&reference.id().declared()) {
                            continue;
                        }
                        let owner = client.wps().resolve(&scope.scope, &text).await?.scope;
                        let read_at = catalog.iter().find(|read| read.scope == owner).map(|read| read.at);
                        match next.get(&owner).copied().or(read_at) {
                            Some(at) if at != *seq => {
                                let text =
                                    format!("{owner} is pinned at changeset {at} and, through `{text}`, at {seq}");
                                return Err(ReadFault::Inexact(text));
                            }
                            Some(_) => {}
                            None => {
                                next.insert(owner, *seq);
                            }
                        }
                    }
                }
            }
        }
        if next.is_empty() {
            return Ok((catalog, notes));
        }
        if own.len() + catalog.len() + next.len() > CLOSURE_LIMIT {
            return Err(ReadFault::TooLarge(CLOSURE_LIMIT));
        }
        frontier = Vec::new();
        for (scope, seq) in next {
            let snapshot = client.read(&scope, seq).await?;
            if !snapshot.exact.is_exact() {
                notes.push(format!(
                    "{scope} is read at changeset {seq}, below its head: its components are that changeset's, its \
                     labels the head's"
                ));
            }
            held.extend(snapshot.entities.iter().map(|entity| entity.id));
            frontier.push(snapshot.clone());
            catalog.push(snapshot);
        }
    }
}

/// Have the client's schema cache hold every component these scopes carry, nested ones included.
async fn ensure_components<S: Wps>(client: &Client<S>, scopes: &[ClientSnapshot]) -> Result<(), WpsError> {
    let mut names: Vec<String> = scopes
        .iter()
        .flat_map(|scope| scope.entities.iter())
        .flat_map(|entity| entity.components.iter().flat_map(|(name, payload)| components_read(name, payload)))
        .collect();
    names.push("external_ids".into());
    names.sort();
    names.dedup();
    for name in names {
        client.registry().ensure(client.wps(), &name).await?;
    }
    Ok(())
}

/// The registry as the engine reads it: every component the scopes carry at the version the client's cache holds,
/// with that version's schema and annotations, compiled beside the server's definitions document.
async fn registry_of<'a, S: Wps>(
    kept: &Kept,
    client: &Client<S>,
    scopes: impl Iterator<Item = &'a ClientSnapshot>,
) -> Result<RegistrySnapshot, ReadFault> {
    let scopes: Vec<ClientSnapshot> = scopes.cloned().collect();
    ensure_components(client, &scopes).await?;
    let cache = client.registry().snapshot();
    let mut names: Vec<&String> = cache.components.keys().collect();
    names.sort();
    let mut components = Vec::new();
    for name in names {
        let entry = &cache.components[name];
        let Some(version) = entry.latest.as_ref().map(ToString::to_string) else { continue };
        let schema = schema_of(kept, client, name, &version).await?;
        let refs = entry.refs.iter().map(|r| RefAnnotation { path: r.path.clone(), expects: r.expects.clone() });
        components.push(RegisteredComponent { name: name.clone(), version, schema, refs: refs.collect() });
    }
    let defs = client.wps().protocol_defs().await?;
    RegistrySnapshot::new(components, Some(&defs)).map_err(|fault| ReadFault::Template(fault.to_string()))
}

/// The schema `name` was published with at `version`, kept once fetched, since a published version never changes.
async fn schema_of<S: Wps>(
    kept: &Kept,
    client: &Client<S>,
    name: &str,
    version: &str,
) -> Result<Option<Value>, WpsError> {
    let key = (name.to_owned(), version.to_owned());
    if let Some(schema) = kept.schemas.lock().expect("the kept schemas' lock").get(&key) {
        return Ok(Some(schema.clone()));
    }
    let Some(component) = client.wps().component(name).await? else { return Ok(None) };
    let Some(published) = component.versions.into_iter().find(|published| published.version == version) else {
        return Ok(None);
    };
    kept.schemas.lock().expect("the kept schemas' lock").insert(key, published.schema.clone());
    Ok(Some(published.schema))
}

/// The adapter proper: a scope as the client read it, as the engine reads it; `at` becomes `seq`, and `exact` was
/// settled by the caller.
fn snapshot(read: ClientSnapshot) -> Result<ScopeSnapshot, ReadFault> {
    let prologue = read.prologue.map(prologue).transpose()?;
    let entities = read
        .entities
        .into_iter()
        .map(|entity| DeclaredEntity { id: entity.id, name: entity.name, components: entity.components })
        .collect();
    let records = read
        .records
        .into_iter()
        .map(|record| match record {
            ClientRecord::Override { target, components } => Record::Override { target, components },
            ClientRecord::Suppress { target } => Record::Suppress { target },
        })
        .collect();
    Ok(ScopeSnapshot { scope: read.scope, seq: read.at, prologue, entities, records })
}

/// A prologue, its document ids moved from the client's text form to the engine's UUID form.
pub fn prologue(read: ClientPrologue) -> Result<Prologue, ReadFault> {
    let doc_id =
        |id: &str| id.parse().map_err(|fault: String| ReadFault::Template(format!("a prologue's id: {fault}")));
    Ok(Prologue {
        doc: doc_id(read.doc.as_str())?,
        kind: read.kind,
        name: read.name,
        primary_frame: read.primary_frame,
        depends_on: read.depends_on.iter().map(|id| doc_id(id.as_str())).collect::<Result<_, _>>()?,
    })
}

#[cfg(test)]
mod tests;
