//! Within `site-template`, `read` is the engine's input (spec 05 section 3.6, `TemplateRead`): a site template's
//! scopes, the catalog scopes it pins and the registry, each at one version, as plain values. The API builds them
//! from world-proto-server through its client; a test builds them from documents on disk with
//! [`ScopeSnapshot::from_document`]. Nothing here reads a server.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use jsonschema::{Resource, Validator};
use rustc_hash::FxHashMap;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use studio_protocol::{DeclaredId, EntityId, GrammarError, ScopePath, Seq, TagName};
use thiserror::Error;

/// The URN the registry's schemas reach the server's definitions document by (world-proto `server-defs.json`).
pub const DEFS_URN: &str = "urn:world-graph-server:defs";

/// The protocol's comment, which every consumer ignores wherever it stands (world-proto SPEC section 2), a
/// component map's keys included: never a component, so never registered.
const COMMENT: &str = "$comment";

/// Everything one evaluation reads, each scope at one changeset.
#[derive(Debug)]
pub struct TemplateRead {
    pub template: ScopePath,
    /// Null until world-proto-server has tags (spec 05 section 2, input 2).
    pub tag: Option<TagName>,
    /// The top scope and every sub-scope (`map`, `flow`, `fleet`), each with its prologue, entities and records.
    pub scopes: Vec<ScopeSnapshot>,
    /// Every catalog scope the template pins, at its pinned version.
    pub catalog: Vec<ScopeSnapshot>,
    pub registry: RegistrySnapshot,
}

/// One scope at one changeset (spec 04 section 3.2's `ScopeSnapshot`, its `exact` flag left to the client).
#[derive(Debug, Clone)]
pub struct ScopeSnapshot {
    pub scope: ScopePath,
    pub seq: Seq,
    pub prologue: Option<Prologue>,
    pub entities: Vec<DeclaredEntity>,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredEntity {
    pub id: DeclaredId,
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

/// A record aimed at a derived entity: an override of its components, or its suppression.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Override { target: EntityId, components: Map<String, Value> },
    Suppress { target: EntityId },
}

/// A document's identity: a random v4 UUID, minted once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocId(pub DeclaredId);

impl DocId {
    pub fn mint() -> Self {
        Self(DeclaredId::mint())
    }
}

impl Display for DocId {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

impl FromStr for DocId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self).map_err(|fault: GrammarError| fault.to_string())
    }
}

impl Serialize for DocId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DocId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(DeError::custom)
    }
}

/// A scope's prologue: the `doc` block and `primary_frame` its export opens with (spec 04 section 3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prologue {
    pub doc: DocId,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub primary_frame: Option<EntityId>,
    #[serde(default)]
    pub depends_on: Vec<DocId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{scope}: {why}")]
pub struct SnapshotFault {
    pub scope: String,
    pub why: String,
}

impl ScopeSnapshot {
    /// Read a world document, as world-proto-server imports one into a scope, into the snapshot of that scope at
    /// `seq`: its `doc` block and `primary_frame` as the prologue, each declaration as an entity, and each
    /// `derivation` record as an override or a suppression.
    pub fn from_document(scope: ScopePath, seq: Seq, document: &Value) -> Result<ScopeSnapshot, SnapshotFault> {
        let refuse = |why: String| SnapshotFault { scope: scope.to_string(), why };
        let doc = document.get("doc").ok_or_else(|| refuse("the document has no `doc` block".into()))?;
        let field = |name: &str| doc.get(name).and_then(Value::as_str).map(str::to_owned);
        let prologue = Prologue {
            doc: field("id").ok_or_else(|| refuse("`doc.id` is missing".into()))?.parse().map_err(refuse)?,
            kind: field("kind"),
            name: field("name"),
            primary_frame: match document.get("primary_frame").and_then(Value::as_str) {
                Some(id) => Some(id.parse().map_err(|fault: GrammarError| refuse(fault.to_string()))?),
                None => None,
            },
            depends_on: serde_json::from_value(doc.get("depends_on").cloned().unwrap_or(Value::Array(Vec::new())))
                .map_err(|fault| refuse(fault.to_string()))?,
        };
        let mut entities = Vec::new();
        let mut records = Vec::new();
        for entry in document.get("entities").and_then(Value::as_array).into_iter().flatten() {
            let components = entry.get("components").and_then(Value::as_object).cloned().unwrap_or_default();
            if let Some(derivation) = entry.get("derivation") {
                let generator = derivation.get("generator").and_then(Value::as_str).unwrap_or_default();
                let key = derivation.get("key").and_then(Value::as_str).unwrap_or_default();
                let target: EntityId =
                    format!("{generator}/{key}").parse().map_err(|fault: GrammarError| refuse(fault.to_string()))?;
                let suppress = entry.get("suppress").and_then(Value::as_bool).unwrap_or(false);
                records.push(if suppress {
                    Record::Suppress { target }
                } else {
                    Record::Override { target, components }
                });
                continue;
            }
            if entry.get("patch").is_some() || entry.get("suppress").is_some() {
                return Err(refuse("a scope snapshot holds declarations and derivation records, never a patch".into()));
            }
            let id = entry.get("id").and_then(Value::as_str).ok_or_else(|| refuse("an entity has no `id`".into()))?;
            entities.push(DeclaredEntity {
                id: id.parse().map_err(|fault: GrammarError| refuse(fault.to_string()))?,
                name: entry.get("name").and_then(Value::as_str).map(str::to_owned),
                components,
            });
        }
        Ok(ScopeSnapshot { scope, seq, prologue: Some(prologue), entities, records })
    }
}

/// One component as the registry defines it: its schema at the version read and its reference annotations.
#[derive(Debug, Clone)]
pub struct RegisteredComponent {
    pub name: String,
    pub version: String,
    /// `None` where the snapshot carries the annotations alone.
    pub schema: Option<Value>,
    pub refs: Vec<RefAnnotation>,
}

/// A field that holds a reference, as a JSON pointer whose `*` stands for every element of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefAnnotation {
    pub path: String,
    #[serde(default)]
    pub expects: Option<String>,
}

/// The registry as the engine reads it: every component involved, its schema compiled once.
pub struct RegistrySnapshot {
    components: FxHashMap<String, (RegisteredComponent, Option<Validator>)>,
}

impl fmt::Debug for RegistrySnapshot {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&String> = self.components.keys().collect();
        names.sort();
        out.debug_struct("RegistrySnapshot").field("components", &names).finish()
    }
}

impl RegistrySnapshot {
    /// Compile each schema with the server's definitions document `defs` in reach, as the server compiles them.
    pub fn new(components: Vec<RegisteredComponent>, defs: Option<&Value>) -> Result<Self, SnapshotFault> {
        let resource = match defs {
            Some(defs) => Some(
                Resource::from_contents(defs.clone())
                    .map_err(|fault| SnapshotFault { scope: DEFS_URN.into(), why: fault.to_string() })?,
            ),
            None => None,
        };
        let mut compiled = FxHashMap::default();
        for component in components {
            let validator = match &component.schema {
                Some(schema) => {
                    let mut options = jsonschema::options();
                    if let Some(resource) = &resource {
                        options.with_resource(DEFS_URN, resource.clone());
                    }
                    Some(options.build(schema).map_err(|fault| SnapshotFault {
                        scope: component.name.clone(),
                        why: format!("its schema does not compile: {fault}"),
                    })?)
                }
                None => None,
            };
            compiled.insert(component.name.clone(), (component, validator));
        }
        Ok(Self { components: compiled })
    }

    pub fn component(&self, name: &str) -> Option<&RegisteredComponent> {
        self.components.get(name).map(|(component, _)| component)
    }

    pub fn refs(&self, name: &str) -> &[RefAnnotation] {
        self.component(name).map_or(&[], |component| component.refs.as_slice())
    }

    /// Whether a component map may hold `name`: the registry defines it, or it is the protocol's comment.
    pub fn defines(&self, name: &str) -> bool {
        name == COMMENT || self.components.contains_key(name)
    }

    /// Validate one component value as world-proto-server does: a name the registry does not define is refused,
    /// one it defines without a schema (its annotations alone) has nothing to fail, and otherwise the first fault
    /// of the schema comes back in words.
    pub fn validate(&self, name: &str, value: &Value) -> Result<(), String> {
        match self.components.get(name) {
            Some((_, Some(validator))) => match validator.iter_errors(value).next() {
                Some(fault) => Err(format!("`{name}` fails its schema at `{}`: {fault}", fault.instance_path)),
                None => Ok(()),
            },
            Some((_, None)) => Ok(()),
            None if name == COMMENT => Ok(()),
            None => Err(format!("`{name}` is no component the registry defines")),
        }
    }

    pub fn versions(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .components
            .values()
            .filter(|(_, validator)| validator.is_some())
            .map(|(component, _)| (component.name.clone(), component.version.clone()))
            .collect();
        out.sort();
        out
    }
}
