//! Within `wps-client`, `error` is spec 04 section 3.8: the one typed error every call answers, the problem a refusal
//! or a finding of `check` becomes, and how each reaches spec 01 as a problem type. `refusal` reads WPS's answers
//! into these by their codes; this module only says what they are and what the user is told.

use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use studio_protocol::{EntityId, ScopePath, Seq};
use thiserror::Error;

use crate::refusal::Unresolved;
use crate::value::{Capability, GeneratorSet};

/// Every way a call to world-proto-server, or this client's own check of one, can fail.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum WpsError {
    /// (scope, base, head) for every scope that moved past the base a draft was built on.
    #[error("{}; nothing was applied", moved_cause(moved))]
    HeadMoved { moved: Vec<(ScopePath, Seq, Seq)> },
    #[error("the generators that derive this site changed after it was evaluated; evaluate and approve again")]
    GeneratorsMoved { expected: GeneratorSet, found: GeneratorSet },
    #[error("the generators that derive {scope} kept changing while it was written; nothing was written: {text}")]
    RegistryMoved { scope: ScopePath, text: String },
    /// WPS's first refusal: it stops at one (`wp:server/server/src/storage/apply.rs:1062-1076`).
    #[error("world-proto-server refused the change: {}", problem.text)]
    Refused { problem: Box<Problem> },
    #[error("{}", problems_sentence("the change", problems))]
    Problems { problems: Vec<Problem> },
    #[error("{}", problems_sentence("the export", problems))]
    ExportNotClosed { problems: Vec<Problem> },
    #[error("{text}")]
    NotFound { text: String },
    /// The resolve route's refusal in a resolution's place, read by its code as what it says of the reference's
    /// target (section 3.7, step 2), with WPS's sentence.
    #[error("{text}")]
    Unresolvable { unresolved: Unresolved, text: String },
    /// A pin that names no changeset of a scope, or a fixed cut that leaves a scope out.
    #[error("{text}")]
    PinUnresolved { scope: ScopePath, text: String },
    #[error("{scope} already exists and is not empty, at changeset {head}")]
    ScopeExists { scope: ScopePath, head: Seq },
    #[error("{}", unsupported_sentence(*capability))]
    Unsupported { capability: Capability },
    #[error("this change is {bytes} bytes, larger than world-proto-server accepts ({limit} bytes)")]
    TooLarge { bytes: u64, limit: u64 },
    #[error("world-proto-server did not answer after {attempts} tries; nothing was written: {last}")]
    Unavailable { attempts: u8, last: String },
    #[error("not yet known whether the change to {} landed", spoken(scopes))]
    OutcomeUnknown { scopes: Vec<ScopePath> },
    #[error("world-proto-server failed on its side ({status}): {text}")]
    ServerFault { status: u16, text: String },
    /// A request this client should never have sent: the fault is ours.
    #[error("Agent Studio sent world-proto-server a request it refused as malformed: {text}")]
    Invalid { text: String },
    /// Another body sent under an operation identifier WPS holds (`operation_reused`), which names the scope its
    /// changeset was initiated on and each scope it wrote; one draft makes one body, so the fault is ours.
    #[error("{}", reused_sentence(operation, initiated, named))]
    OperationReused { operation: String, initiated: ScopePath, named: Vec<(ScopePath, Seq)> },
    #[error("{entity}'s `{component}` does not read as its type: {detail}")]
    Decode { entity: EntityId, component: String, detail: String },
    /// The configured server is not one this client can work with; the client stops at start.
    #[error("world-proto-server cannot be used: {text}")]
    Incompatible { text: String },
    /// Spec 01's compute pool is full (section 3.15); `retry_after` is wall clock.
    #[error("Agent Studio is busy computing; try again in {} s", retry_after.as_secs().max(1))]
    ComputeBusy { retry_after: Duration },
    /// A computation ended without an answer, on spec 01's compute pool or as a cache's flight: it panicked, or was
    /// dropped before it answered, a fault of ours and no answer of world-proto-server's.
    #[error("a computation of Agent Studio's ended without an answer: it panicked, or was dropped before it answered")]
    ComputeLost,
}

/// Every scope that moved past the base a draft was built on, each with both changesets.
fn moved_cause(moved: &[(ScopePath, Seq, Seq)]) -> String {
    let each: Vec<String> =
        moved.iter().map(|(scope, base, head)| format!("{scope} moved on (changeset {base} to {head})")).collect();
    format!("{} after this draft was built", spoken(&each))
}

fn reused_sentence(operation: &str, initiated: &ScopePath, named: &[(ScopePath, Seq)]) -> String {
    let each: Vec<String> = named.iter().map(|(scope, seq)| format!("changeset {seq} of {scope}")).collect();
    let stored = match each.is_empty() {
        true => format!("a changeset initiated on {initiated} that wrote no scope"),
        false => format!("{}, initiated on {initiated}", spoken(&each)),
    };
    format!("operation `{operation}` already names {stored}, sent with another body; nothing was applied")
}

fn problems_sentence(what: &str, problems: &[Problem]) -> String {
    let each: Vec<&str> = problems.iter().map(|problem| problem.text.as_str()).collect();
    format!("{what} has {} problem(s): {}", problems.len(), each.join("; "))
}

fn unsupported_sentence(capability: Capability) -> String {
    let what = match capability {
        Capability::Tags => "Tags are",
        Capability::Branches => "Branches other than main are",
        Capability::Preview => "The server's preview is",
        Capability::ExportFlags => "The export's flags (the feature `export_flags`) are",
        Capability::MultiScopeChangesets => "A changeset across a site's scopes is",
        Capability::ExactReplace => "Exact replace is",
        Capability::OperationId => "The operation identifier is",
        Capability::StructuredRefusals => "A code and details beside each refusal's sentence are",
        Capability::ReadsAtVersion => {
            return "A site read at one changeset, its other scopes placed by commit order, is not available in \
                    world-proto-server yet; name each scope's changeset instead (a fixed cut)"
                .to_owned();
        }
        _ => "This capability is",
    };
    format!("{what} not available in world-proto-server yet")
}

pub(crate) fn spoken<T: Display>(items: &[T]) -> String {
    match items {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => {
            let rest: Vec<String> = rest.iter().map(T::to_string).collect();
            format!("{} and {last}", rest.join(", "))
        }
    }
}

/// One thing wrong with a change, naming where it is as far as the finding knows, and for WPS's `schema_invalid` the
/// schema version it validated against, which the schema cache compares with its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    pub scope: Option<ScopePath>,
    pub entity: Option<EntityId>,
    pub component: Option<String>,
    pub path: Option<String>,
    pub version: Option<String>,
    pub text: String,
}

impl Problem {
    pub fn new(kind: ProblemKind, text: impl Into<String>) -> Self {
        Self { kind, scope: None, entity: None, component: None, path: None, version: None, text: text.into() }
    }

    pub fn in_scope(self, scope: &ScopePath) -> Self {
        Self { scope: Some(scope.clone()), ..self }
    }

    pub fn on(self, entity: EntityId) -> Self {
        Self { entity: Some(entity), ..self }
    }

    pub fn at_field(self, component: &str, path: &str) -> Self {
        Self { component: Some(component.to_owned()), path: Some(path.to_owned()), ..self }
    }

    /// `/{component}{path}`, spec 01's `field`.
    pub fn field(&self) -> Option<String> {
        self.component.as_ref().map(|component| format!("/{component}{}", self.path.as_deref().unwrap_or_default()))
    }
}

/// A problem as spec 01's problem item: `code`, `ref`, `scope`, `field`, `text`.
impl Serialize for Problem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut item = serializer.serialize_map(None)?;
        item.serialize_entry("code", &self.kind)?;
        item.serialize_entry("ref", &self.entity)?;
        item.serialize_entry("scope", &self.scope)?;
        item.serialize_entry("field", &self.field())?;
        item.serialize_entry("text", &self.text)?;
        item.end()
    }
}

/// One value per WPS refusal code the client places a problem by, named as WPS names it (`wp:server/DESIGN.md`
/// section 10 at cd4923516), then one per finding of `check` and of the export's closure check, then `Other` for a code
/// this client does not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProblemKind {
    UnregisteredComponent,
    NoPublishedVersion,
    SchemaInvalid,
    PatchMissingTarget,
    DeleteMissingTarget,
    ReplaceMissingTarget,
    ReplaceDerived,
    DeclareOnDeleted,
    OwnedElsewhere,
    DuplicateOp,
    RecordTargetNotDerived,
    LeftDangling,
    BadRefValue,
    Undeclared,
    Dangling,
    PinWithinUnit,
    HeadAcrossUnits,
    BranchAcrossUnits,
    TwoPartAcrossUnits,
    PinInTwoParts,
    TagNotHeld,
    PinBeyondHead,
    PinnedDangling,
    GenerationRefused,
    GenerationOverBudget,
    GenerationInputPastCeiling,
    GenerationInputUnread,
    GenerationOutputPastCeiling,
    GenerationOverlaidPastCeiling,
    GenerationPinnedTwice,
    GenerationLeftPinnedTwice,
    GenerationHeldTwice,
    GenerationPinNotHeld,
    StagedAddressNotDerived,
    LensOpOutsideTheRegistry,
    AdapterOpOutsideTheRegistry,
    GeneratorOpOutsideTheRegistry,
    EntityOpInTheRegistry,
    OpOfAnotherFamily,
    HoldsNul,
    PrologueHoldsNul,
    OutsideTheInitiatingScope,
    CreationUnflagged,
    CreatedTwice,
    BaseUnstated,
    CreatedBaseNotZero,
    StatedTwice,
    ExpectedTwice,
    RegistryWritesItselfAlone,
    PinCycle,
    ParentMissing,
    BadReference,
    LateralNeedsVersion,
    PinnedInsideBoundary,
    PinnedTwice,
    HeldTwice,
    ClosureTooLarge,
    DocumentFlaggedExport,
    OutsideSubtree,
    MissingBase,
    DuplicateWrite,
    SpentId,
    NotLive,
    MissingPrologue,
    Unregistered,
    Nul,
    TooLarge,
    Other,
}

impl Display for ProblemKind {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        let code = serde_json::to_value(self).map_err(|_| fmt::Error)?;
        out.write_str(code.as_str().unwrap_or("other"))
    }
}

/// What spec 01 answers for an error: its problem type, the status, the detail a person reads, the items, `moved`
/// for a stale draft, and `uncertain` for the scopes whose write is not yet known to have landed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiProblem {
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub status: u16,
    pub detail: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<Problem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moved: Vec<Moved>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uncertain: Vec<ScopePath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    pub scope: ScopePath,
    pub base: Seq,
    pub head: Seq,
}

impl WpsError {
    /// Spec 01 section 3.14's problem for this error (spec 04 section 3.8's table); the types `head-moved` to
    /// `compute-busy` join `studio_protocol::ProblemType` with the routes that first answer them.
    pub fn api_problem(&self) -> ApiProblem {
        let (type_, status) = match self {
            Self::HeadMoved { .. } => ("head-moved", 409),
            Self::GeneratorsMoved { .. } | Self::RegistryMoved { .. } => ("generators-moved", 409),
            Self::Refused { .. } | Self::Problems { .. } | Self::TooLarge { .. } => ("validation-failed", 422),
            Self::ExportNotClosed { .. } => ("export-not-closed", 422),
            Self::NotFound { .. } => ("not-found", 404),
            Self::Unresolvable { unresolved: Unresolved::Owned { problem: None, .. }, .. } => {
                ("validation-failed", 422)
            }
            Self::Unresolvable { .. } => ("not-found", 404),
            Self::PinUnresolved { .. } => ("pin-unresolved", 404),
            Self::ScopeExists { .. } => ("site-exists", 409),
            Self::Unsupported { .. } | Self::Incompatible { .. } => ("wps-not-ready", 501),
            Self::Unavailable { .. } => ("wps-unavailable", 503),
            Self::OutcomeUnknown { .. } => ("wps-outcome-unknown", 502),
            Self::ServerFault { .. } => ("wps-fault", 502),
            Self::Invalid { .. } | Self::OperationReused { .. } | Self::Decode { .. } => ("internal", 500),
            Self::ComputeBusy { .. } => ("compute-busy", 503),
            Self::ComputeLost => ("internal", 500),
        };
        let problems = match self {
            Self::Refused { problem } => vec![(**problem).clone()],
            Self::Problems { problems } | Self::ExportNotClosed { problems } => problems.clone(),
            _ => Vec::new(),
        };
        let moved = match self {
            Self::HeadMoved { moved } => moved
                .iter()
                .map(|(scope, base, head)| Moved { scope: scope.clone(), base: *base, head: *head })
                .collect(),
            _ => Vec::new(),
        };
        let uncertain = match self {
            Self::OutcomeUnknown { scopes } => scopes.clone(),
            _ => Vec::new(),
        };
        let detail = match self {
            Self::Invalid { .. } | Self::OperationReused { .. } | Self::Decode { .. } => {
                "Agent Studio could not talk to world-proto-server about this; the fault is ours.".to_owned()
            }
            Self::ComputeLost => {
                "Agent Studio's computation for this ended without an answer; the fault is ours.".to_owned()
            }
            other => other.to_string(),
        };
        ApiProblem { type_, status, detail, problems, moved, uncertain }
    }
}
