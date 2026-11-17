//! Within the mock, `flagged_export` is WPS's flagged export as the mock answers it (Agent Studio's ruling D185): the
//! scope a request names and, with `nested`, the scopes nested in it down to the next boundary, each whole at the
//! changeset `at` names (`requested`); with `derived`, beside a scope's declared entities, the rows a generation left
//! there, a stand-in generation's (`generation`) or those a test planted (`plant`), the scope's records laid over them;
//! and with `referenced`, every entity an entity of the file references, followed until nothing new turns up, and none
//! of the rest of its scope (`referenced`). Written as one file the way WPS writes it.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use serde_json::{Map, Value, json};
use studio_protocol::{EntityId, ScopePath, Seq};

use super::{Inner, MockWps, refuse};
use crate::error::WpsError;
use crate::export::CLOSURE_LIMIT;
use crate::refusal::RouteKind;
use crate::wire::ExportFlags;

mod generation;
mod referenced;
mod requested;

use generation::Generated;
pub use generation::StandInGeneration;
use requested::Request;

impl MockWps {
    /// Leave `id` in `scope`, at its head, as a generation would: a derived entity with `components`, before any
    /// record of the scope's unit is laid over it.
    pub fn plant(&self, scope: &ScopePath, id: EntityId, components: Map<String, Value>) {
        let mut inner = self.lock();
        let state = inner.scopes.get_mut(scope).and_then(|held| held.states.last_mut()).expect("a scope to plant in");
        state.planted.insert(id.to_string(), (id, components));
    }
}

/// One scope the file holds entities from: the changeset it is read at, and the entities `referenced` brought from
/// it, none where the request holds it whole.
struct HeldScope {
    at: Seq,
    // ordered: the file lists a scope's entities by identity.
    brought: Option<BTreeSet<String>>,
}

/// One entity of a scope as the file writes it, and for a planted row with an override laid over it, the components
/// it was planted with, whose references WPS's index keeps beside the override's.
struct Row {
    name: Option<String>,
    components: Map<String, Value>,
    planted: Option<Map<String, Value>>,
}

impl Inner {
    /// WPS's flagged export of `scope` with `flags`, each scope a reference at head from the requested scopes can reach
    /// read at the changeset `named` gives (at its head where `named` gives none). A stand-in generation runs over what
    /// the export reads, and again while the entities `referenced` brings change what it reads.
    pub(super) fn flagged_export(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        named: &BTreeMap<ScopePath, Seq>,
    ) -> Result<Bytes, WpsError> {
        assert!(
            flags.named().iter().any(|(_, set)| *set),
            "a flagged export sets a flag; with none WPS answers the plain export, `export_scope`"
        );
        let request = self.request(scope, flags, named)?;
        let latest = self.latest();
        let (mut generated, mut read) = (Vec::new(), None);
        let held = loop {
            let held = self.held(scope, &request, &generated, &latest)?;
            let Some(generation) = self.generation.as_ref().filter(|_| flags.derived) else { break held };
            let input = self.generation_input(&request, &held, &latest)?;
            if read.as_ref() == Some(&input.read) {
                break held;
            }
            generated = self.generated(generation, scope, &input, &latest)?;
            read = Some(input.read);
        };
        self.written(scope, &request, &held, &generated)
    }

    /// Every entity of `scope` at changeset `at`, by identity: the ones it declares, and with `derived` the rows
    /// `generated` holds in it and the rows a test planted there, the scope's records laid over them and a suppressed
    /// one left out.
    fn rows(
        &self,
        scope: &ScopePath,
        at: Seq,
        derived: bool,
        generated: &[Generated],
    ) -> Result<BTreeMap<String, Row>, WpsError> {
        let (_, state, _) = self.state_at(scope, Some(at))?;
        // ordered: the file lists a scope's entities by identity.
        let mut rows: BTreeMap<String, Row> = (state.entities.iter())
            .map(|(id, (_, name, components))| {
                (id.clone(), Row { name: name.clone(), components: components.clone(), planted: None })
            })
            .collect();
        if !derived {
            return Ok(rows);
        }
        for one in generated.iter().filter(|one| one.scope == *scope) {
            let row = Row { name: one.name.clone(), components: one.components.clone(), planted: None };
            rows.insert(one.id.clone(), row);
        }
        for (id, (_, components)) in &state.planted {
            if state.records.contains_key(&format!("{id}#suppress")) {
                rows.remove(id);
                continue;
            }
            let overlaid = state.records.get(&format!("{id}#override")).and_then(|record| record.get("components"));
            let row = match overlaid.and_then(Value::as_object) {
                Some(overlaid) => {
                    let mut laid = components.clone();
                    laid.extend(overlaid.clone());
                    Row { name: None, components: laid, planted: Some(components.clone()) }
                }
                None => Row { name: None, components: components.clone(), planted: None },
            };
            rows.insert(id.clone(), row);
        }
        Ok(rows)
    }

    /// The file, as WPS writes it: its keys in byte order, the scope the request named, its flags, every entity it
    /// holds by scope and then by identity, and each scope it holds entities from with its changeset and whether it
    /// holds that scope whole.
    fn written(
        &self,
        scope: &ScopePath,
        request: &Request,
        held: &BTreeMap<ScopePath, HeldScope>,
        generated: &[Generated],
    ) -> Result<Bytes, WpsError> {
        let (mut entities, mut scopes) = (Vec::new(), Vec::new());
        for (path, one) in held {
            for (id, row) in self.rows(path, one.at, request.flags.derived, generated)? {
                if one.brought.as_ref().is_some_and(|brought| !brought.contains(&id)) {
                    continue;
                }
                let mut entry = json!({ "components": row.components, "id": id });
                if let Some(name) = row.name {
                    entry["name"] = json!(name);
                }
                entry["scope"] = json!(path);
                entities.push(entry);
            }
            scopes.push(json!({ "changeset": one.at, "scope": path, "whole": one.brought.is_none() }));
        }
        let ExportFlags { nested, derived, referenced } = request.flags;
        let file = json!({
            "derived": derived,
            "entities": entities,
            "nested": nested,
            "referenced": referenced,
            "scope": scope,
            "scopes": scopes,
        });
        Ok(Bytes::from(serde_json::to_vec(&file).expect("an export serializes")))
    }
}

/// The keys a flagged export holds beside `entities` and no document does (`written`), by which WPS's import refuses
/// one.
pub(super) const FLAGGED_FILE_KEYS: [&str; 5] = ["derived", "nested", "referenced", "scope", "scopes"];

/// WPS's refusal of a flagged export of `scope` that would hold entities from more scopes than one export holds.
fn closure_too_large(scope: &ScopePath) -> WpsError {
    let sentence = format!(
        "the flagged export of {scope} would hold entities from more than {CLOSURE_LIMIT} scopes, those it requests \
         and those its references bring entities from, and one export holds at most {CLOSURE_LIMIT}"
    );
    let details = json!({ "scope": scope, "limit": CLOSURE_LIMIT });
    refuse(RouteKind::Export, 422, "closure_too_large", details, sentence)
}

/// WPS's refusal of a flagged export that would read `scope` at two changesets, the lower named first.
fn reached_twice(scope: &ScopePath, one: Seq, other: Seq) -> WpsError {
    let changesets = [one.min(other), one.max(other)];
    let sentence = format!(
        "{scope} would be read at changesets {} and {}, and a flagged export holds each scope at one changeset",
        changesets[0], changesets[1]
    );
    let details = json!({ "scope": scope, "changesets": changesets });
    refuse(RouteKind::Export, 409, "reached_twice", details, sentence)
}

#[cfg(test)]
mod tests;
