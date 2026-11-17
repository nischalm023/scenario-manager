//! Within `wps-client`, `value` is spec 04 section 3.1, the names and value types `studio-protocol` does not already
//! hold: the version a reference across a boundary takes, the reference a consumer states, the author and document
//! identities, a scope's prologue, a site's cut, the capability words, and WPS's listing, log and generation answers.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use rustc_hash::FxHashSet;
use semver::Version as SemVer;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Number;
use studio_protocol::{BranchName, DeclaredId, EntityId, GrammarError, Pin, RefPin, ScopePath, Seq, TagName};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValueError {
    #[error("`{0}` is a branch, and a reference across a boundary names a changeset or a tag, never a branch")]
    Branch(String),
    #[error("`head` follows the referrer's branch, and a reference across a boundary names a changeset or a tag")]
    Head,
    #[error("an author is the person who acts, and is never blank")]
    BlankAuthor,
    #[error(transparent)]
    Grammar(#[from] GrammarError),
}

/// The only pins a lateral reference takes: a changeset of the target's scope, or a tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Version {
    Seq(Seq),
    Tag(TagName),
}

impl TryFrom<Pin> for Version {
    type Error = ValueError;

    fn try_from(pin: Pin) -> Result<Self, ValueError> {
        match pin {
            Pin::Changeset(seq) => Ok(Self::Seq(seq)),
            Pin::Tag(name) => Ok(Self::Tag(name)),
            Pin::Branch(name) => Err(ValueError::Branch(name.to_string())),
        }
    }
}

impl FromStr for Version {
    type Err = ValueError;

    fn from_str(text: &str) -> Result<Self, ValueError> {
        match text.parse::<RefPin>()? {
            RefPin::Head => Err(ValueError::Head),
            RefPin::Pin(pin) => pin.try_into(),
        }
    }
}

impl Display for Version {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seq(seq) => write!(out, "{seq}"),
            Self::Tag(name) => write!(out, "tag:{name}"),
        }
    }
}

/// What a consumer states about a reference, which `DraftBuilder::reference` spells (spec 04 section 3.7): an entity
/// of the referrer's own scope, of another scope of the same site at head, or under another boundary at a version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reference {
    Local(EntityId),
    Kin(EntityId),
    Lateral(EntityId, Version),
}

impl Reference {
    /// A lateral reference at `pin`, refused when the pin is `head` or a branch.
    pub fn lateral(id: EntityId, pin: RefPin) -> Result<Self, ValueError> {
        match pin {
            RefPin::Head => Err(ValueError::Head),
            RefPin::Pin(pin) => Ok(Self::Lateral(id, pin.try_into()?)),
        }
    }

    pub fn id(&self) -> &EntityId {
        match self {
            Self::Local(id) | Self::Kin(id) | Self::Lateral(id, _) => id,
        }
    }

    /// The reference as a payload holds it: bare, `@head`, or `@` and its version.
    pub fn spelled(&self) -> String {
        match self {
            Self::Local(id) => id.to_string(),
            Self::Kin(id) => format!("{id}@head"),
            Self::Lateral(id, version) => format!("{id}@{version}"),
        }
    }
}

/// The authenticated person a changeset is written by, handed over by spec 01.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Author(String);

impl Author {
    pub fn new(name: impl Into<String>) -> Result<Self, ValueError> {
        let name = name.into();
        if name.trim().is_empty() { Err(ValueError::BlankAuthor) } else { Ok(Self(name)) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Author {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(DeError::custom)
    }
}

/// A document's identity: a lower-case UUID, as the protocol's `doc_id` is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(String);

impl DocId {
    pub fn mint() -> Self {
        Self(DeclaredId::mint().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DocId {
    type Err = ValueError;

    fn from_str(text: &str) -> Result<Self, ValueError> {
        Ok(Self(text.parse::<DeclaredId>()?.to_string()))
    }
}

impl Display for DocId {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Serialize for DocId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DocId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(DeError::custom)
    }
}

/// What makes a scope's export a document the simulator loads, in WPS's changeset form
/// (`wp:server/core/src/prologue.rs:25-46`): its id, kind, display name, primary frame and dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prologue {
    pub doc: DocId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_frame: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<DocId>,
}

/// Whether a read is the scope as it stood, or its components only beside the head's labels and prologue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exactness {
    Exact,
    ComponentsOnly { reason: String },
}

impl Exactness {
    /// Until exact history lands, a read below the head carries the head's labels and prologue. The primary frame is
    /// no prologue's to name but the frame whose component states `unit_mm` (ruling D177), so it is the changeset's.
    pub fn below_head(scope: &ScopePath, at: Seq) -> Self {
        Self::ComponentsOnly {
            reason: format!(
                "{scope} at changeset {at} is read below its head: its components are that changeset's, and so is its \
                 primary frame, the frame that states `unit_mm`; its labels and its prologue are the head's"
            ),
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact)
    }

    /// The weaker of two: a cut is exact only when every read in it is.
    pub fn and(self, other: Exactness) -> Exactness {
        match (self, other) {
            (Self::Exact, other) => other,
            (weaker, _) => weaker,
        }
    }
}

/// A site's scopes resolved once at one pin: each scope's changeset, which every later read of the cut takes as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "UnitCutWire", try_from = "UnitCutWire")]
pub struct UnitCut {
    pub unit: ScopePath,
    pub requested: Pin,
    pub branch: BranchName,
    pub seqs: BTreeMap<ScopePath, Seq>,
    pub exact: Exactness,
}

#[derive(Serialize, Deserialize)]
struct UnitCutWire {
    unit: ScopePath,
    requested: Pin,
    branch: BranchName,
    seqs: BTreeMap<ScopePath, Seq>,
    exact: bool,
    exact_reason: Option<String>,
}

impl From<UnitCut> for UnitCutWire {
    fn from(cut: UnitCut) -> Self {
        let (exact, exact_reason) = match cut.exact {
            Exactness::Exact => (true, None),
            Exactness::ComponentsOnly { reason } => (false, Some(reason)),
        };
        Self { unit: cut.unit, requested: cut.requested, branch: cut.branch, seqs: cut.seqs, exact, exact_reason }
    }
}

impl TryFrom<UnitCutWire> for UnitCut {
    type Error = String;

    fn try_from(wire: UnitCutWire) -> Result<Self, String> {
        let exact = match (wire.exact, wire.exact_reason) {
            (true, None) => Exactness::Exact,
            (false, Some(reason)) => Exactness::ComponentsOnly { reason },
            (true, Some(_)) => return Err("an exact cut states no `exact_reason`".to_owned()),
            (false, None) => return Err("a cut that is not exact states its `exact_reason`".to_owned()),
        };
        Ok(Self { unit: wire.unit, requested: wire.requested, branch: wire.branch, seqs: wire.seqs, exact })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    Generation,
    PinKeepingGeneration,
    GeneratorModules,
    ExactHistory,
    ReadsAtVersion,
    CompleteAnnotations,
    ScopePaths,
    KinReferences,
    MultiScopeChangesets,
    Tags,
    Branches,
    ExportFlags,
    Preview,
    GeneratorPrecondition,
    StructuredRefusals,
    OperationId,
    ExactReplace,
    ListingNames,
    RegistryRevision,
}

/// The words of `GET /protocol`'s `features` this client knows, each with the capability it names: those served
/// since 60aa424, then each later phase's as world-proto's `server/DESIGN.md` section 16 names it.
const FEATURE_WORDS: &[(&str, Capability)] = &[
    ("generation", Capability::Generation),
    ("scope_paths", Capability::ScopePaths),
    ("pin_rule", Capability::KinReferences),
    ("registry_revision", Capability::RegistryRevision),
    ("scope_listing_names", Capability::ListingNames),
    ("exact_replace", Capability::ExactReplace),
    ("operation_id", Capability::OperationId),
    ("scope_changesets", Capability::MultiScopeChangesets),
    ("structured_refusals", Capability::StructuredRefusals),
    ("exact_history", Capability::ExactHistory),
    ("reads_at_version", Capability::ReadsAtVersion),
    ("tags", Capability::Tags),
    ("export_flags", Capability::ExportFlags),
    ("preview", Capability::Preview),
];

impl Capability {
    /// The capability a word of `GET /protocol`'s `features` names; `None` for a word this client does not know,
    /// which it ignores.
    pub fn from_feature(word: &str) -> Option<Self> {
        FEATURE_WORDS.iter().find(|(known, _)| *known == word).map(|(_, capability)| *capability)
    }

    /// The word `features` lists this capability by, for the capabilities a server states rather than one the
    /// probe finds.
    pub fn feature_word(self) -> Option<&'static str> {
        FEATURE_WORDS.iter().find(|(_, capability)| *capability == self).map(|(word, _)| *word)
    }
}

/// What the probe found the configured server to have landed (spec 04 section 3.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub landed: FxHashSet<Capability>,
    pub server_version: String,
    pub proto_version: String,
}

impl Capabilities {
    pub fn has(&self, capability: Capability) -> bool {
        self.landed.contains(&capability)
    }
}

/// One scope of `GET /scopes`; `name` is its prologue's, `None` until the listing carries names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeListing {
    pub path: ScopePath,
    pub parent: Option<ScopePath>,
    pub boundary: bool,
    pub unit: ScopePath,
    pub head: Seq,
    pub name: Option<String>,
}

/// A changeset's commit place: one number every changeset of a world takes from one server-wide sequence as it
/// commits, shared by the parts of one changeset across several scopes, compared only within a unit and never an
/// address (spec 04 section 1, its clocks; `wp:server/DESIGN.md` section 8 at 092d695).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitPlace(u64);

impl CommitPlace {
    pub fn new(place: u64) -> Self {
        Self(place)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    /// The place the changeset to commit next takes, in commit order.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// One scope a changeset created, as its answer names it (scopes phase 3): its parent, its flag and its unit's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedScope {
    pub scope: ScopePath,
    pub parent: Option<ScopePath>,
    pub boundary: bool,
    pub unit: ScopePath,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpTally {
    pub declared: u32,
    pub patched: u32,
    #[serde(default)]
    pub replaced: u32,
    pub deleted: u32,
    pub overridden: u32,
    pub suppressed: u32,
}

/// One changeset of a scope's log (`wp:server/server/src/api/changesets.rs:167-177`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub seq: Seq,
    pub author: String,
    pub message: String,
    pub tally: OpTally,
    /// RFC 3339, wall clock.
    pub applied_at: String,
    #[serde(default)]
    pub operation: Option<String>,
}

/// One page of a listing; an empty `items` ends the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedListing {
    pub id: EntityId,
    pub name: Option<String>,
    pub component_names: Vec<String>,
}

/// One live reference to an entity: who holds it, in which scope, component and field, and at what pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referrer {
    pub from: EntityId,
    pub scope: ScopePath,
    pub component: String,
    pub path: String,
    pub pin: Option<Pin>,
}

/// A value of `external_ids`: never an object (`wp:proto/schema/components/identity.json`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalValue {
    Text(String),
    Number(Number),
    Flag(bool),
}

/// The generators a registry runs: each name's highest published version and its module digest.
// ordered: a generator set is compared, stored in drafts and written into warnings in name order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratorSet(pub BTreeMap<String, (SemVer, String)>);

/// What WPS's generation ran for one changeset (`wp:server/server/src/api/changesets.rs:105-118`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationRan {
    pub digest: String,
    pub derived: u64,
    pub generators: GeneratorSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivedRecord {
    pub target: EntityId,
    pub kind: String,
    pub why: String,
}
