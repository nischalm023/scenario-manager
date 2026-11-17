//! Within `wps-client`, `snapshot` is what a read answers (spec 04 section 3.2): one scope's authored form at a
//! changeset, a site's scopes read together at one cut with the lookups consumers need over them, WPS's single entity
//! read, a site's flagged export with the changeset of each scope it holds entities from, and the event a moved scope
//! yields.

use std::collections::BTreeMap;

use bytes::Bytes;
use rustc_hash::FxHashMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use studio_protocol::{DeclaredId, EntityId, ScopePath, Seq, TagName};
use thiserror::Error;

use crate::error::WpsError;
use crate::value::{CommitPlace, Exactness, ExternalValue, OpTally, Prologue, UnitCut};

/// A declared entity as its scope authored it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredEntity {
    pub id: DeclaredId,
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

/// A live record of a scope: an override of a derived entity's components, or its suppression.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Override { target: EntityId, components: Map<String, Value> },
    Suppress { target: EntityId },
}

impl Record {
    pub fn target(&self) -> &EntityId {
        match self {
            Self::Override { target, .. } | Self::Suppress { target } => target,
        }
    }
}

/// One scope's authored form at a changeset: its declared entities and live records, never a derived entity.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeSnapshot {
    pub scope: ScopePath,
    pub at: Seq,
    pub exact: Exactness,
    pub prologue: Option<Prologue>,
    pub entities: Vec<DeclaredEntity>,
    pub records: Vec<Record>,
}

/// WPS's single entity read (`wp:server/server/src/api/entities.rs:174-203`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityReading {
    pub id: EntityId,
    pub provenance: String,
    pub name: Option<String>,
    pub components: Map<String, Value>,
    pub component_versions: Map<String, Value>,
    pub generation: Option<Value>,
    pub created_seq: Seq,
    pub deleted_seq: Option<Seq>,
}

/// A component type a consumer spec owns, decoded from a snapshot by its registered name.
pub trait Component: Serialize + DeserializeOwned {
    const NAME: &'static str;
}

/// Two entities of one site carrying one external key, which ruling D73's lookup cannot choose between.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{system} = {value} is carried by {first} in {first_scope} and by {second} in {second_scope}")]
pub struct DuplicateKey {
    pub system: String,
    pub value: String,
    pub first: DeclaredId,
    pub first_scope: ScopePath,
    pub second: DeclaredId,
    pub second_scope: ScopePath,
}

/// A site's scopes read at one cut, indexed by the entity each scope declares.
#[derive(Debug, Clone)]
pub struct UnitSnapshot {
    pub cut: UnitCut,
    // ordered: consumers walk a site's scopes in path order, and an export lists them so.
    pub scopes: BTreeMap<ScopePath, ScopeSnapshot>,
    owners: FxHashMap<DeclaredId, (ScopePath, usize)>,
}

impl UnitSnapshot {
    pub fn new(cut: UnitCut, scopes: BTreeMap<ScopePath, ScopeSnapshot>) -> Self {
        let mut owners = FxHashMap::default();
        for (path, snapshot) in &scopes {
            for (index, entity) in snapshot.entities.iter().enumerate() {
                owners.insert(entity.id, (path.clone(), index));
            }
        }
        Self { cut, scopes, owners }
    }

    /// The scope an entity lives in; a derived address lives where its generator's anchor is declared.
    pub fn owner_of(&self, id: &EntityId) -> Option<&ScopePath> {
        self.owners.get(&id.declared()).map(|(scope, _)| scope)
    }

    pub fn get(&self, id: &DeclaredId) -> Option<(&ScopePath, &DeclaredEntity)> {
        let (scope, index) = self.owners.get(id)?;
        Some((scope, &self.scopes[scope].entities[*index]))
    }

    /// Every entity carrying `C`, decoded, in scope order and then in each scope's order.
    pub fn all<C: Component>(&self) -> Result<Vec<(ScopePath, DeclaredId, C)>, WpsError> {
        let mut found = Vec::new();
        for (scope, snapshot) in &self.scopes {
            for entity in &snapshot.entities {
                let Some(payload) = entity.components.get(C::NAME) else { continue };
                let decoded = serde_json::from_value(payload.clone()).map_err(|fault| WpsError::Decode {
                    entity: entity.id.into(),
                    component: C::NAME.to_owned(),
                    detail: fault.to_string(),
                })?;
                found.push((scope.clone(), entity.id, decoded));
            }
        }
        Ok(found)
    }

    /// Every entity by its `external_ids` value under `system`, or the first key two entities share.
    pub fn by_external_id(
        &self,
        system: &str,
    ) -> Result<FxHashMap<ExternalValue, (ScopePath, DeclaredId)>, Box<DuplicateKey>> {
        let mut index: FxHashMap<ExternalValue, (ScopePath, DeclaredId)> = FxHashMap::default();
        for (scope, snapshot) in &self.scopes {
            for entity in &snapshot.entities {
                let Some(value) = entity.components.get("external_ids").and_then(|ids| ids.get(system)) else {
                    continue;
                };
                let key = match value {
                    Value::String(text) => ExternalValue::Text(text.clone()),
                    Value::Number(number) => ExternalValue::Number(number.clone()),
                    Value::Bool(flag) => ExternalValue::Flag(*flag),
                    _ => continue,
                };
                if let Some((first_scope, first)) = index.get(&key) {
                    return Err(Box::new(DuplicateKey {
                        system: system.to_owned(),
                        value: value.to_string(),
                        first: *first,
                        first_scope: first_scope.clone(),
                        second: entity.id,
                        second_scope: scope.clone(),
                    }));
                }
                index.insert(key, (scope.clone(), entity.id));
            }
        }
        Ok(index)
    }
}

/// Which reference walk vouched for the closure check: the annotations alone, or with the client's own list,
/// named by its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "walk")]
pub enum Walked {
    Annotations,
    AnnotationsAndList { list: String },
}

/// The flagged export held to a cut, as `Client::export` answers it and a run reads it (rulings D40 and D185): the
/// whole site of the cut as WPS generated it with every entity it references, held to the cut and closed, and what it
/// holds.
#[derive(Debug, Clone, PartialEq)]
pub struct CutExport {
    pub bytes: Bytes,
    pub sha256: String,
    /// Every scope the file holds entities from, whole or in part, at the changeset it was read at.
    // ordered: the file's `scopes` and a runner's manifest list scopes in path order.
    pub scopes: BTreeMap<ScopePath, Seq>,
    pub tag: Option<TagName>,
    pub exact: Exactness,
    pub walked: Walked,
    /// The scope that holds each declared entity the file lists, and so each entity generated from it.
    pub holders: FxHashMap<DeclaredId, ScopePath>,
}

/// A hint that a scope moved, never the record of it: the changeset log is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeEvent {
    /// WPS applied changeset `seq`; `gap` when it skips past the last one this stream yielded. `commit` is the place
    /// every scope one changeset wrote shares, so the moves of several watched scopes read as one changeset.
    Moved { scope: ScopePath, seq: Seq, author: String, tally: OpTally, gap: bool, commit: Option<CommitPlace> },
    /// After a reconnect, a read of the head found the scope had moved while the stream was down.
    Resynced { scope: ScopePath, head: Seq },
}
