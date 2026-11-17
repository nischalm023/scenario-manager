//! Within `studio-protocol`, `problem` is how the service refuses: spec 01 section 3.14's `application/problem+json`
//! body (RFC 9457) and the vocabulary of its `type`. A type joins the vocabulary with the first route, tool or parser
//! that answers it, so the TypeScript union names every type a page can meet and no other; the world-proto-server
//! client's types joined with the API's reading of its errors (spec 04 section 3.8).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::{Key, ScopePath, Seq};
use crate::scenario::FieldConflict;

/// An error as the service answers it: `type`, `title`, `status` and `detail`, and `instance`, the path of the
/// request it refuses, when there is one. The extensions are spec 01 section 3.14's `problems`, `moved` and
/// `retry_after_s`, and the world-proto-server client's `uncertain` (spec 04 section 3.8's `OutcomeUnknown`); `draft`
/// and `revision` arrive with the draft routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "problem.ts")]
pub struct Problem {
    #[serde(rename = "type")]
    pub type_: ProblemType,
    /// The type's summary, the same at every occurrence.
    pub title: String,
    /// The HTTP status of the answer, repeated in the body.
    pub status: u16,
    /// This occurrence, in words a person can act on.
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<ProblemItem>>", optional)]
    pub problems: Vec<ProblemItem>,
    /// With `head-moved`: every scope that moved past the base it was read at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<MovedScope>>", optional)]
    pub moved: Vec<MovedScope>,
    /// With `wps-outcome-unknown`: the scopes whose write is not yet known to have landed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<ScopePath>>", optional)]
    pub uncertain: Vec<ScopePath>,
    /// With a busy answer: wall-clock seconds to wait before asking again, which `Retry-After` repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub retry_after_s: Option<u32>,
    /// With `scenario-changed` (spec 07 section 3.5): each field the edit could not set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<FieldConflict>>", optional)]
    pub conflicts: Vec<FieldConflict>,
}

/// One thing wrong, the same in every spec (spec 01 section 3.14): its `code`, and where it is as far as the finding
/// knows: `ref` a key, `field` a JSON pointer, `knob` a knob's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "problem.ts")]
pub struct ProblemItem {
    pub code: String,
    pub severity: Severity,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ref_: Option<Key>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scope: Option<ScopePath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub knob: Option<String>,
    pub text: String,
}

/// Whether a problem item stops what it is found on (`error`) or only says so (`warning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "problem.ts")]
pub enum Severity {
    Error,
    Warning,
}

/// A scope that moved past the changeset something was built on: its `base`, and the `head` it moved to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "problem.ts")]
pub struct MovedScope {
    pub scope: ScopePath,
    pub base: Seq,
    pub head: Seq,
}

impl Problem {
    /// A problem of `type_`, its title and status the type's own, with no extension.
    pub fn new(type_: ProblemType, detail: impl Into<String>) -> Self {
        Self {
            type_,
            title: type_.title().to_owned(),
            status: type_.status(),
            detail: detail.into(),
            instance: None,
            problems: Vec::new(),
            moved: Vec::new(),
            uncertain: Vec::new(),
            retry_after_s: None,
            conflicts: Vec::new(),
        }
    }

    /// The same problem, naming the request path it refuses.
    pub fn at(self, instance: impl Into<String>) -> Self {
        Self { instance: Some(instance.into()), ..self }
    }
}

/// A problem's `type`: the slugs of spec 01 section 3.14 that the service's routes answer so far, spec 03 section 3.8's
/// for the reference grammar, and those spec 04 section 3.8 maps the world-proto-server client's errors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "problem.ts")]
pub enum ProblemType {
    IdentityMissing,
    NotFound,
    ValidationFailed,
    IdempotencyKeyMissing,
    IdempotencyKeyReused,
    IdempotencyKeyInFlight,
    NotConversationOwner,
    ConversationClosed,
    TurnRunning,
    ProjectRequired,
    RefMalformed,
    RefPinConflicts,
    RefNeedsVersion,
    HeadMoved,
    GeneratorsMoved,
    ExportNotClosed,
    PinUnresolved,
    SiteExists,
    WpsNotReady,
    WpsUnavailable,
    WpsOutcomeUnknown,
    WpsFault,
    ComputeBusy,
    /// A fault on the service's side ("the fault is ours"); its detail never carries the fault's internals.
    Internal,
    KnobChanged,
    DraftNotOpen,
    EvaluationStale,
    /// A site's document will not load in the simulator's reader or compile to a floor (spec 03 section 3.8).
    FloorCompileFailed,
    /// Spec 03 section 3.8's for an address or a command: no such scope or object; a reference naming nothing at the
    /// location; a location the panel does not show.
    LocationUnresolved,
    RefUnresolved,
    PanelMismatch,
    ScenarioChanged,
    NameTaken,
    ScenarioDeleted,
    PlanStale,
    DraftChanged,
    StaleDraft,
    NotDraftOwner,
    DraftApplying,
    PlanNotReady,
    ExperimentSubmitted,
    NoRunnerImage,
    NotFetched,
    ManifestDiffers,
    RunnerTokenInvalid,
    NotThisRun,
    NoRecording,
    /// A heartbeat or a recording for an attempt that has ended: nothing of it is kept.
    AttemptEnded,
    /// A recording other than the one its attempt took first, which stands: nothing of it is kept.
    RecordingDiffers,
    /// A run whose latest attempt has no live stream to watch: no streaming server was alive when it was dispatched,
    /// or the one it streams to is no longer.
    NotStreamed,
    /// Task 3's, the agent loop's (spec 01 sections 3.4, 3.8 and 3.14): an answer or decision for a turn that no
    /// longer waits on it; an answer, decision or attribution after the first; a conversation past its size gate; the
    /// process's turns all taken; a task not built yet, its detail naming the task; the model unreachable at all.
    TurnNotWaiting,
    QuestionClosed,
    ApprovalClosed,
    ConversationTooLong,
    AgentBusy,
    NotBuilt,
    ModelUnavailable,
    /// A failed tool's: an asking tool in an unattended turn; what was approved moved on; the tool's own limits; a
    /// panel command with no panel to act on; a conversation another provider began; a call the turn's budget stopped,
    /// or never let run (spec 01 section 3.4, "The loop's budgets").
    ApprovalRequired,
    StaleApproval,
    ToolTimeout,
    ResultTooLarge,
    NoPanelContext,
    ProviderChanged,
    TurnBudget,
}

impl ProblemType {
    /// The type's usual status; `pin-unresolved` is 404 for a pin in an address and 422 for one stored in a body,
    /// which its answer sets.
    pub fn status(self) -> u16 {
        match self {
            Self::IdempotencyKeyMissing => 400,
            Self::IdentityMissing | Self::RunnerTokenInvalid => 401,
            Self::NotConversationOwner | Self::NotDraftOwner | Self::NotThisRun | Self::ApprovalRequired => 403,
            Self::NotFound | Self::PinUnresolved | Self::LocationUnresolved => 404,
            Self::IdempotencyKeyInFlight
            | Self::ConversationClosed
            | Self::TurnRunning
            | Self::HeadMoved
            | Self::GeneratorsMoved
            | Self::SiteExists
            | Self::KnobChanged
            | Self::DraftNotOpen
            | Self::EvaluationStale
            | Self::ScenarioChanged
            | Self::NameTaken
            | Self::ScenarioDeleted
            | Self::PlanStale
            | Self::DraftChanged
            | Self::StaleDraft
            | Self::DraftApplying
            | Self::PlanNotReady
            | Self::ExperimentSubmitted
            | Self::NoRunnerImage
            | Self::NotFetched
            | Self::ManifestDiffers
            | Self::NoRecording
            | Self::AttemptEnded
            | Self::RecordingDiffers
            | Self::NotStreamed
            | Self::TurnNotWaiting
            | Self::QuestionClosed
            | Self::ApprovalClosed
            | Self::ConversationTooLong
            | Self::StaleApproval
            | Self::ProviderChanged => 409,
            Self::ProjectRequired
            | Self::ValidationFailed
            | Self::IdempotencyKeyReused
            | Self::RefMalformed
            | Self::RefPinConflicts
            | Self::RefNeedsVersion
            | Self::ExportNotClosed
            | Self::FloorCompileFailed
            | Self::RefUnresolved
            | Self::PanelMismatch
            | Self::ResultTooLarge
            | Self::NoPanelContext => 422,
            Self::Internal => 500,
            Self::WpsNotReady | Self::NotBuilt => 501,
            Self::WpsOutcomeUnknown | Self::WpsFault => 502,
            Self::WpsUnavailable | Self::ComputeBusy | Self::AgentBusy | Self::ModelUnavailable => 503,
            Self::ToolTimeout => 504,
            Self::TurnBudget => 429,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::IdentityMissing => "Not signed in",
            Self::NotFound => "Nothing is here",
            Self::ValidationFailed => "The request failed a check",
            Self::IdempotencyKeyMissing => "The request needs an Idempotency-Key",
            Self::IdempotencyKeyReused => "That Idempotency-Key was used for another request",
            Self::IdempotencyKeyInFlight => "Still working on your last press",
            Self::NotConversationOwner => "This new site's conversation is another person's",
            Self::ConversationClosed => "This conversation is closed",
            Self::TurnRunning => "The agent is working",
            Self::ProjectRequired => "A conversation needs a project, and there is none",
            Self::RefMalformed => "Not a reference Agent Studio can read",
            Self::RefPinConflicts => "A reference inside the site carries a pin",
            Self::RefNeedsVersion => "A reference outside the site needs a version",
            Self::HeadMoved => "The site moved on after this was read",
            Self::GeneratorsMoved => "The generators that derive this site changed",
            Self::ExportNotClosed => "This site cannot be exported whole",
            Self::PinUnresolved => "That changeset does not exist",
            Self::SiteExists => "A site of that name already exists",
            Self::WpsNotReady => "world-proto-server cannot do this yet",
            Self::WpsUnavailable => "world-proto-server did not answer",
            Self::WpsOutcomeUnknown => "Not yet known whether the change landed",
            Self::WpsFault => "world-proto-server failed on its side",
            Self::ComputeBusy => "Agent Studio is busy computing",
            Self::Internal => "The fault is ours",
            Self::KnobChanged => "A knob was set by someone else after you read it",
            Self::DraftNotOpen => "This template draft takes no change now",
            Self::EvaluationStale => "Only the draft's latest ready evaluation can be written",
            Self::FloorCompileFailed => "This map cannot be drawn",
            Self::LocationUnresolved => "There is no such scope or object",
            Self::RefUnresolved => "A reference names nothing at this location",
            Self::PanelMismatch => "That panel does not show this",
            Self::ScenarioChanged => "The scenario changed after this edit's version was read",
            Self::NameTaken => "This site already has a scenario of that name",
            Self::ScenarioDeleted => "This scenario was deleted",
            Self::PlanStale => "The scenario changed after these runs were prepared",
            Self::DraftChanged => "The draft changed after you read it",
            Self::StaleDraft => "This draft can no longer be applied",
            Self::NotDraftOwner => "This draft is another person's",
            Self::DraftApplying => "This draft is being applied",
            Self::PlanNotReady => "These runs are not ready to submit",
            Self::ExperimentSubmitted => "These runs were already submitted",
            Self::NoRunnerImage => "No runner image is registered to run these",
            Self::NotFetched => "The runner has not handed in what it read yet",
            Self::ManifestDiffers => "This attempt already handed in another manifest",
            Self::RunnerTokenInvalid => "This token does not open this attempt",
            Self::NotThisRun => "This scenario version is not this runner's run's",
            Self::NoRecording => "This attempt stored no recording",
            Self::AttemptEnded => "This attempt has ended",
            Self::RecordingDiffers => "This attempt took another recording already",
            Self::NotStreamed => "This run has no live stream to watch",
            Self::TurnNotWaiting => "The agent's turn no longer waits on this",
            Self::QuestionClosed => "This question was already answered or has expired",
            Self::ApprovalClosed => "This approval was already decided or has expired",
            Self::ConversationTooLong => "This conversation is too long for the agent",
            Self::AgentBusy => "The agent service is busy",
            Self::NotBuilt => "Not built yet",
            Self::ModelUnavailable => "The agent's model cannot be reached",
            Self::ApprovalRequired => "This needs a person's approval, and nobody is attached to give it",
            Self::StaleApproval => "What was approved moved on after the approval",
            Self::ToolTimeout => "The tool took too long",
            Self::ResultTooLarge => "The tool's answer is too large",
            Self::NoPanelContext => "There is no panel to act on",
            Self::ProviderChanged => "This conversation began with another model provider",
            Self::TurnBudget => "The turn reached its budget",
        }
    }
}
