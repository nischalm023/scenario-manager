//! Within `wps-client`, `wire` mirrors world-proto-server's request and answer bodies at master 092d695: scopes
//! phase 3's changeset across a unit (an operation's `scope`, `bases`, `create`, `prologues`, `generators`) and its
//! answer's `scopes` and `created`, the first cut's `operation`, `replayed` and `replace`, and the conveniences'
//! listing `name` and `features`, each optional so an older server still reads; no world-proto crate is linked (spec
//! 04 section 10). Each answer is read here into this crate's own values.

use std::collections::BTreeMap;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use studio_protocol::{EntityId, Pin, ScopePath, Seq};

use crate::draft::{NewScope, Op};
use crate::error::WpsError;
use crate::snapshot::EntityReading;
use crate::value::{
    ArchivedRecord, CommitPlace, CreatedScope, DerivedListing, OpTally, Prologue, Referrer, ScopeListing,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireProtocol {
    pub proto_version: String,
    #[serde(default)]
    pub features: Option<Vec<String>>,
}

/// One row of `GET /scopes` (`wp:server/server/src/api/scopes.rs:57-67`); a family of the registry has no flag.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireScope {
    pub scope: String,
    pub parent: Option<String>,
    pub boundary: Option<bool>,
    pub unit: Option<String>,
    pub head: u64,
    #[serde(default)]
    pub name: Option<String>,
}

impl WireScope {
    /// The scope as a listing, or `None` for a family of the registry, which stands outside the tree.
    pub fn listing(&self) -> Result<Option<ScopeListing>, WpsError> {
        let (Some(boundary), Some(unit)) = (self.boundary, &self.unit) else { return Ok(None) };
        let path = |text: &str| {
            text.parse::<ScopePath>().map_err(|fault| WpsError::Invalid { text: format!("the listing: {fault}") })
        };
        Ok(Some(ScopeListing {
            path: path(&self.scope)?,
            parent: self.parent.as_deref().map(path).transpose()?,
            boundary,
            unit: path(unit)?,
            head: Seq::new(self.head),
            name: self.name.clone(),
        }))
    }
}

/// One operation as a changeset carries it: WPS's own, naming the scope it writes where that is not the scope the
/// changeset is initiated on (`wp:server/core/src/changeset.rs:64-110` at 092d695).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireOp {
    #[serde(flatten)]
    pub op: Op,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopePath>,
}

/// A changeset as `POST /scopes/<path>/-/changesets` takes it (`wp:server/core/src/changeset.rs:262-330` at 092d695):
/// `scope` is the scope it is initiated on and `base` that scope's head as read; `bases` holds the head read of every
/// other scope it writes or only reads, `create` the scopes it creates, `prologues` every other scope's prologue and
/// `generators` the set it expects to run. Each is left out when empty, so a changeset of one scope is the body a
/// server before scopes phase 3 takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireChangeset {
    pub scope: ScopePath,
    pub base: Seq,
    pub author: String,
    pub message: String,
    pub ops: Vec<WireOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prologue: Option<Prologue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    // ordered: one draft always sends one body, its scopes in path order.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bases: BTreeMap<ScopePath, Seq>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub create: Vec<NewScope>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prologues: BTreeMap<ScopePath, Prologue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generators: Option<Vec<WireRanGenerator>>,
}

impl WireChangeset {
    /// A changeset of one scope, as every changeset was before scopes phase 3: its operations name no scope, and it
    /// creates nothing and states no other base, prologue, generator set or operation identifier.
    pub fn of_one_scope(
        scope: ScopePath,
        base: Seq,
        author: impl Into<String>,
        message: impl Into<String>,
        ops: Vec<Op>,
    ) -> Self {
        Self {
            scope,
            base,
            author: author.into(),
            message: message.into(),
            ops: ops.into_iter().map(|op| WireOp { op, scope: None }).collect(),
            prologue: None,
            operation: None,
            bases: BTreeMap::new(),
            create: Vec::new(),
            prologues: BTreeMap::new(),
            generators: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireRanGenerator {
    pub generator: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireGeneration {
    pub digest: String,
    pub derived: u64,
    pub generators: Vec<WireRanGenerator>,
}

/// One reference WPS stored otherwise than it was sent: written `@head`, stored bare
/// (`wp:server/server/src/storage/references.rs:105-116`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Respelled {
    pub entity: String,
    pub component: String,
    pub path: String,
    pub written: String,
    pub stored: String,
}

/// One written scope's entry in an applied changeset's answer (`wp:server/server/src/api/changesets.rs:107-119`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireAppliedScope {
    pub scope: ScopePath,
    pub base: Seq,
    pub seq: Seq,
    pub tally: OpTally,
    #[serde(default)]
    pub archived: Vec<ArchivedRecord>,
    #[serde(default)]
    pub generation: Option<WireGeneration>,
}

/// The 201 a changeset is answered with, a replay's included: since scopes phase 3 an entry per written scope and
/// every scope it created, and for a changeset that wrote exactly one scope that scope's entry flat as well, which is
/// all an older server answers (`wp:server/server/src/api/changesets.rs:73-105` at 092d695).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct WireApplied {
    #[serde(default)]
    pub scopes: Vec<WireAppliedScope>,
    #[serde(default)]
    pub created: Vec<CreatedScope>,
    #[serde(default)]
    pub seq: Option<Seq>,
    #[serde(default)]
    pub tally: Option<OpTally>,
    #[serde(default)]
    pub archived: Vec<ArchivedRecord>,
    #[serde(default)]
    pub generation: Option<WireGeneration>,
    #[serde(default)]
    pub respelled: Vec<Respelled>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub replayed: bool,
}

impl WireApplied {
    /// Each written scope's entry: `scopes` where the server answers them, else the flat fields an older server
    /// answers for the one scope a changeset could write, the scope `sent` was initiated on.
    pub fn parts(&self, sent: &WireChangeset) -> Vec<WireAppliedScope> {
        if !self.scopes.is_empty() {
            return self.scopes.clone();
        }
        match (self.seq, self.tally) {
            (Some(seq), Some(tally)) => vec![WireAppliedScope {
                scope: sent.scope.clone(),
                base: sent.base,
                seq,
                tally,
                archived: self.archived.clone(),
                generation: self.generation.clone(),
            }],
            _ => Vec::new(),
        }
    }
}

/// `POST /scopes/<path>/-/import?base=`: the changeset a document would become, applying nothing.
#[derive(Debug, Clone, Deserialize)]
pub struct WireImport {
    pub changeset: WireChangeset,
    pub tally: OpTally,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// A reference annotation of a published schema: a JSON pointer, or one ending `/*` for each element.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RefAnnotation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expects: Option<String>,
}

/// A component's definition (`wp:server/server/src/registry.rs:189-208`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireDefinition {
    pub component: String,
    pub owner: String,
    pub status: String,
    pub meaning: String,
    #[serde(default)]
    pub asserts: Option<String>,
    #[serde(default)]
    pub generator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireSchemaVersion {
    pub component: String,
    pub version: String,
    pub schema: Value,
    #[serde(default)]
    pub refs: Vec<RefAnnotation>,
}

/// `GET /registry/{c}`: a definition and every version published under it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireComponent {
    pub component: WireDefinition,
    pub versions: Vec<WireSchemaVersion>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireGeneratorVersion {
    pub generator: String,
    pub version: String,
    pub digest: String,
}

/// `GET /registry/generators/{g}` and, without versions, each entry of the listing.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireGenerator {
    pub generator: String,
    #[serde(default)]
    pub versions: Vec<WireGeneratorVersion>,
}

/// `GET /registry/generators`: the generator family's head and its definitions.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WireGenerators {
    pub at: u64,
    pub generators: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireEntity {
    pub id: EntityId,
    pub provenance: String,
    #[serde(default)]
    pub name: Option<String>,
    pub components: Map<String, Value>,
    #[serde(default)]
    pub component_versions: Map<String, Value>,
    #[serde(default)]
    pub generation: Option<Value>,
    pub created_seq: Seq,
    pub deleted_seq: Option<Seq>,
}

impl From<WireEntity> for EntityReading {
    fn from(wire: WireEntity) -> Self {
        Self {
            id: wire.id,
            provenance: wire.provenance,
            name: wire.name,
            components: wire.components,
            component_versions: wire.component_versions,
            generation: wire.generation,
            created_seq: wire.created_seq,
            deleted_seq: wire.deleted_seq,
        }
    }
}

/// An entity's whole trail in its scope (`GET /scopes/<path>/-/history/<id>`), read for whether an id is spent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WireTrail {
    pub created_seq: Seq,
    pub deleted_seq: Option<Seq>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireListed {
    pub id: EntityId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub component_names: Vec<String>,
}

impl From<WireListed> for DerivedListing {
    fn from(wire: WireListed) -> Self {
        Self { id: wire.id, name: wire.name, component_names: wire.component_names }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireReferrer {
    pub scope: ScopePath,
    pub from: EntityId,
    pub component: String,
    pub field_path: String,
    pub pin: Option<u64>,
}

impl From<WireReferrer> for Referrer {
    fn from(wire: WireReferrer) -> Self {
        Self {
            from: wire.from,
            scope: wire.scope,
            component: wire.component,
            path: wire.field_path,
            pin: wire.pin.map(|seq| Pin::Changeset(Seq::new(seq))),
        }
    }
}

/// `GET /scopes/<path>/-/resolve?ref=`: the scope that owns a reference's target, and the pin it reads at.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Resolution {
    pub scope: ScopePath,
    pub id: EntityId,
    pub pin: Option<Seq>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityQuery {
    pub at: Option<Seq>,
    pub under: Option<EntityId>,
    pub provenance: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u16>,
}

/// One scope's export as received: the changeset it reflects (`x-base-version`) and its bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct RawExport {
    pub base: Seq,
    pub bytes: Bytes,
}

/// The flags of WPS's scope export (Agent Studio's ruling D185), each widening what the export of one scope holds and
/// none depending on another: `nested` adds the scope's nested scopes, down to the next boundary, each whole;
/// `derived` adds the entities WPS's generators derived in every scope the file holds; `referenced` adds every entity
/// an entity in the file references, followed until nothing new turns up, and none of the rest of its scope. The
/// request states each as `1` or `0`, and the file each as `true` or `false`, under these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFlags {
    pub nested: bool,
    pub derived: bool,
    pub referenced: bool,
}

impl ExportFlags {
    /// What a run reads: a site whole, as WPS generated it, with every entity it references.
    pub const ALL: ExportFlags = ExportFlags { nested: true, derived: true, referenced: true };

    /// Each flag under the name the request and the file give it.
    pub fn named(self) -> [(&'static str, bool); 3] {
        [("nested", self.nested), ("derived", self.derived), ("referenced", self.referenced)]
    }
}

/// One server-sent `changeset` event; `commit` since scopes phase 3, absent from an older server and null on a
/// family of the registry's feed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WireEvent {
    pub seq: Seq,
    pub author: String,
    pub tally: OpTally,
    #[serde(default)]
    pub commit: Option<CommitPlace>,
}
