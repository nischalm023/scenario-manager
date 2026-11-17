//! The studio pages' half of the protocol, generated from this crate's types.
//!
//! This test IS the generator: it writes every type's TypeScript afresh into a scratch folder, then makes
//! `studio/ui/src/protocol/` match it, failing when anything had to change, so a type changed without regenerating
//! stops here rather than at a page that misreads the service. A generated file that no type writes any more is
//! named, never deleted, since the folder may hold hand-written files beside the generated ones. Spec 01 asks for
//! such a drift test (section 7, the CI item); crucible's `tests/ts_bindings.rs` is the same test for its protocol.
//!
//! Run `cargo test -p studio-protocol --test ts_bindings` after changing a type, and commit what it writes.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use studio_protocol::DurationClock;
use studio_protocol::{
    ActiveTurn, ApprovalAnswer, ApprovalDecided, ApprovalExecuted, ApprovalExpired, ApprovalFacts, ApprovalNoted,
    ApprovalRequested, ApprovalState, BlockId, BranchName, CallId, CallState, CheckedBy, Clock, ContextPanel,
    ContextSelection, Conversation, ConversationChanged, ConversationList, ConversationState, ConversationSummary,
    Decision, DeclaredId, DevEntry, DevLive, DevWrite, DevWritten, Dismissed, DraftApplied, DraftDiscarded, DraftOp,
    DraftPart, DraftStale, DraftState, DraftUpdated, EndingCounts, EntityId, EntityRef, EntriesPage, EnvelopeVersion,
    Execution, FactRow, FieldKey, Finding, FindingKind, FlowLayer, GroupMarkCounts, HighlightGroup,
    HighlightGroupInput, ItemAnswer, JournalEntry, JournalKind, JournalSeq, Key, KinPin, KinPins, LiveItem, LiveKind,
    Location, MarkCount, Me, MessageAccepted, MessageSource, MovedScope, NoteKind, ObjectKey, ObjectKind,
    OpenConversation, OpenedConversation, PageContext, Panel, PanelAction, PanelClearInput, PanelCommandEntry,
    PanelCommandResult, PanelEvent, PanelHighlightInput, PanelSelectInput, PanelShowInput, PanelTarget, Pending,
    Person, Pin, Ping, PostAnswers, PostAttribution, PostDecision, PostMessage, Problem, ProblemItem, ProblemType,
    ProgressValue, ProjectedApproval, ProjectedCall, ProjectedDraft, ProjectedQuestion, ProjectedRun, ProjectedTurn,
    Projection, QuestionAnswered, QuestionAsked, QuestionExpired, QuestionItem, QuestionOption, QuestionState, Ref,
    RefPin, ResetReason, ResolvedRef, ResumedBy, RunFinding, RunProgress, RunState, RunStatus, ScopeEventKind,
    ScopeMoved, ScopePath, ScopeSeq, Seq, Severity, SiteBranch, SiteConversation, SiteItem, SiteList, SitePath,
    Snapshot, StaleReason, StreamKind, StreamLive, StreamReset, StreamingBlock, Subject, SubmissionEnded, SystemNote,
    TagName, TextDelta, TextDone, TextKind, TextOffset, ToolFinished, ToolProgress, ToolStarted, TurnFinished,
    TurnNumber, TurnResumed, TurnStarted, TurnState, TurnWaiting, Ulid, Unattributable, Usage, UserMessage, WaitingOn,
    WallClock,
};
use studio_protocol::{AgentModel, AnswersAccepted, CallRead, ModelProvider, QuestionRef, StopAccepted, TurnCalls};
use studio_protocol::{
    Aggregation, AppliedPatch, AttemptControl, AttemptNumber, AttemptRead, ByteCount, CancelAnswer, CancelOutcome,
    CompileRead, CompiledRead, DeclaredMetric, EndedBy, Ending, Endpoint, Expectation, ExportRead, FileFormat,
    FindingPart, FindingSide, Heartbeat, JobSpec, Manifest, ManifestAccepted, MetricBasis, MetricSeries, MetricUnit,
    MetricsFile, PinnedImage, PinnedScenario, Pins, Plan, PlanAccepted, PlanFormat, PlanProgress, PlanRead,
    PlanRequest, PlanRunner, PlanScenario, PlanState, PlannedRun, QueueSnapshot, RecordingRead, RecordingStored,
    ResultsAccepted, ResultsEnded, ResultsReport, RunCancelled, RunCut, RunDetail, RunList, RunPosition,
    RunProgressRead, RunQueue, RunScenario, RunStats, RunSummary, RunnerChosen, RunnerImage, RunnerImages,
    RunnerMessage, RunnerReply, SealedVersion, SeatsRead, SeriesFile, SeriesKind, SeriesPoint, StreamEnded,
    StreamProgress, StreamRefusal, StreamingServer, SubmissionCancelled, SubmitRequest, Submitted, VariedValue,
    ViewerMessage, ViewerRequest, Waiting, WaitingFor, WallMs, WatchTicket, WorkloadRead, WorldCount, WorldRead,
};
use studio_protocol::{AmendDraft, ChangesetOp, ChangesetPart, DraftChecked, DraftList, DraftRead, NewDraft};
use studio_protocol::{
    AppliedTo, CreateScenario, CreatedScenario, DuplicateScenario, Experiment, ExperimentKind, ExperimentRead,
    ExperimentRequest, FieldCatalogue, FieldChange, FieldConflict, FieldEntry, FieldOption, FieldSet, FillPercent,
    FloorTraffic, GenerateRequest, GeneratorSpec, LinesPerOrder, OrderLine, OrderTrays, OrdersPerHour, PanelEventKind,
    PanelEventWritten, Patch, PreloadFraction, Readiness, Release, Repeats, ReportWindow, Revision, RunControls,
    RunCount, RunCounts, RunOverrides, RunSlot, ScenarioContent, ScenarioDiff, ScenarioEdit, ScenarioEdited,
    ScenarioList, ScenarioRead, ScenarioRow, ScenarioSummary, Seed, SetBy, SetInTurn, Setter, SkuCount, SlotCount,
    Sweep, Tally, UnitCut, UnitsPerTote, Variant, VariantNumber, Varied, VariesRequest, VersionNumber, VersionRead,
    VersionState, VersionSummary, Workload, WorkloadHeader, WorkloadItem, WorkloadLine, WorkloadOrder, WorkloadSummary,
};
use studio_protocol::{
    ApprovalRecord, ApprovalVia, Census, CreateSite, DraftMode, DraftStarted, DraftWrite, EvaluationEngine,
    EvaluationEvent, EvaluationNote, EvaluationNumber, EvaluationProblem, EvaluationPrologue, EvaluationRef,
    EvaluationState, FloorRef, GeneratorUsed, KeyChange, KeyChanges, KnobChange, KnobDeclaration, KnobDefault,
    KnobOption, KnobSetBy, KnobSource, KnobUnit, KnobValue, KnobsChanged, KnobsSet, MadeFrom, MadeFromKnob,
    MadeFromList, SetByKind, SetKnobs, SiteMade, StartDraft, TemplateAt, TemplateCard, TemplateDescription,
    TemplateDetail, TemplateDraft, TemplateDraftState, TemplateEvaluation, TemplateList, Thumbnail, ThumbnailState,
    WriteAccepted,
};
use studio_protocol::{
    Bucket, Dimension, MetricCatalogue, MetricDescriptor, MetricMessage, MetricPoint, MetricRequest, MetricSource,
    RunOutcome, TrailingSpec,
};
use studio_protocol::{
    CompareReply, ComparedMetric, ComparedRun, ComparedValue, InputField, InputRow, InputSection, InputValue,
    MetricPins, MetricRow, MetricsReply, NotCompared, PeriodPoint, RunInputs, SameField, SeriesReply, SeriesShape,
    SimWindow, ValueBasis,
};
use studio_protocol::{
    CountRow, EntityCount, FindResults, FlowAllIn, FlowBase, FlowChoice, FlowDiff, FlowDraft, FlowEdge, FlowExecutor,
    FlowFieldDiff, FlowMadeFrom, FlowNode, FlowOrderKind, FlowParameter, FlowParameters, FlowPatch, FlowPlace,
    FlowPlan, FlowRead, FlowReturnRule, FlowRoot, FlowSector, FlowSegment, FlowServedBy, FlowSetting, FlowSetup,
    FlowValidate, FlowValidated, FrameAxes, MapFrame, MapSite, MapSummary, Mark, Mm, RefMarks, Resolution,
    ResolveRequest, UnitMm,
};
use ts_rs::{Config, ExportError, TS};

/// The first line ts-rs writes into every file it generates.
const GENERATED: &str = "// This file was generated by [ts-rs]";

/// The first line of a file this test writes itself, from a type's own rules rather than its shape.
const GENERATED_HERE: &str = "// This file was generated by studio-protocol's tests/ts_bindings.rs";

fn committed_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../studio/ui/src/protocol")
}

/// Every exported type, each with what it carries. A new type joins this list, or the pages never see it.
fn export_every_type(config: &Config) -> Result<(), ExportError> {
    ScopePath::export_all(config)?;
    SitePath::export_all(config)?;
    Seq::export_all(config)?;
    BranchName::export_all(config)?;
    TagName::export_all(config)?;
    Pin::export_all(config)?;
    RefPin::export_all(config)?;
    Location::export_all(config)?;
    KinPin::export_all(config)?;
    KinPins::export_all(config)?;
    Ulid::export_all(config)?;
    ObjectKind::export_all(config)?;
    ObjectKey::export_all(config)?;
    FieldKey::export_all(config)?;
    DeclaredId::export_all(config)?;
    EntityId::export_all(config)?;
    EntityRef::export_all(config)?;
    Key::export_all(config)?;
    Ref::export_all(config)?;
    Problem::export_all(config)?;
    ProblemType::export_all(config)?;
    ProblemItem::export_all(config)?;
    Severity::export_all(config)?;
    MovedScope::export_all(config)?;
    Subject::export_all(config)?;
    Me::export_all(config)?;
    ModelProvider::export_all(config)?;
    AgentModel::export_all(config)?;
    Person::export_all(config)?;
    WallClock::export_all(config)?;
    Clock::export_all(config)?;
    DurationClock::export_all(config)?;
    EnvelopeVersion::export_all(config)?;
    JournalSeq::export_all(config)?;
    JournalKind::export_all(config)?;
    JournalEntry::export_all(config)?;
    LiveKind::export_all(config)?;
    BlockId::export_all(config)?;
    TextOffset::export_all(config)?;
    TextDelta::export_all(config)?;
    LiveItem::export_all(config)?;
    StreamingBlock::export_all(config)?;
    StreamKind::export_all(config)?;
    StreamLive::export_all(config)?;
    Ping::export_all(config)?;
    ResetReason::export_all(config)?;
    StreamReset::export_all(config)?;
    DevEntry::export_all(config)?;
    DevLive::export_all(config)?;
    DevWrite::export_all(config)?;
    DevWritten::export_all(config)?;
    ConversationState::export_all(config)?;
    WaitingOn::export_all(config)?;
    Conversation::export_all(config)?;
    ConversationSummary::export_all(config)?;
    ConversationList::export_all(config)?;
    OpenConversation::export_all(config)?;
    OpenedConversation::export_all(config)?;
    Dismissed::export_all(config)?;
    ConversationChanged::export_all(config)?;
    TurnNumber::export_all(config)?;
    TurnState::export_all(config)?;
    ProjectedTurn::export_all(config)?;
    CallId::export_all(config)?;
    CallState::export_all(config)?;
    ProjectedCall::export_all(config)?;
    QuestionState::export_all(config)?;
    ProjectedQuestion::export_all(config)?;
    ApprovalState::export_all(config)?;
    Execution::export_all(config)?;
    ProjectedApproval::export_all(config)?;
    Unattributable::export_all(config)?;
    ApprovalExecuted::export_all(config)?;
    DraftState::export_all(config)?;
    ProjectedDraft::export_all(config)?;
    ProjectedRun::export_all(config)?;
    Projection::export_all(config)?;
    Pending::export_all(config)?;
    ActiveTurn::export_all(config)?;
    Snapshot::export_all(config)?;
    EntriesPage::export_all(config)?;
    Panel::export_all(config)?;
    PanelAction::export_all(config)?;
    FlowLayer::export_all(config)?;
    PanelTarget::export_all(config)?;
    ScopeSeq::export_all(config)?;
    HighlightGroupInput::export_all(config)?;
    PanelShowInput::export_all(config)?;
    PanelHighlightInput::export_all(config)?;
    PanelSelectInput::export_all(config)?;
    PanelClearInput::export_all(config)?;
    MarkCount::export_all(config)?;
    HighlightGroup::export_all(config)?;
    PanelCommandEntry::export_all(config)?;
    GroupMarkCounts::export_all(config)?;
    PanelCommandResult::export_all(config)?;
    SiteBranch::export_all(config)?;
    SiteConversation::export_all(config)?;
    SiteItem::export_all(config)?;
    SiteList::export_all(config)?;
    ScopeEventKind::export_all(config)?;
    ScopeMoved::export_all(config)?;
    ContextPanel::export_all(config)?;
    ContextSelection::export_all(config)?;
    PageContext::export_all(config)?;
    MessageSource::export_all(config)?;
    PostMessage::export_all(config)?;
    MessageAccepted::export_all(config)?;
    UserMessage::export_all(config)?;
    PanelEvent::export_all(config)?;
    TurnStarted::export_all(config)?;
    TurnWaiting::export_all(config)?;
    ResumedBy::export_all(config)?;
    TurnResumed::export_all(config)?;
    Usage::export_all(config)?;
    TurnFinished::export_all(config)?;
    CallRead::export_all(config)?;
    TurnCalls::export_all(config)?;
    StopAccepted::export_all(config)?;
    TextKind::export_all(config)?;
    ResolvedRef::export_all(config)?;
    TextDone::export_all(config)?;
    SystemNote::export_all(config)?;
    ToolStarted::export_all(config)?;
    ToolProgress::export_all(config)?;
    ToolFinished::export_all(config)?;
    QuestionOption::export_all(config)?;
    QuestionItem::export_all(config)?;
    QuestionAsked::export_all(config)?;
    ItemAnswer::export_all(config)?;
    QuestionAnswered::export_all(config)?;
    QuestionExpired::export_all(config)?;
    PostAnswers::export_all(config)?;
    QuestionRef::export_all(config)?;
    AnswersAccepted::export_all(config)?;
    FactRow::export_all(config)?;
    ApprovalFacts::export_all(config)?;
    ApprovalRequested::export_all(config)?;
    Decision::export_all(config)?;
    ApprovalDecided::export_all(config)?;
    ApprovalExpired::export_all(config)?;
    NoteKind::export_all(config)?;
    ApprovalNoted::export_all(config)?;
    PostDecision::export_all(config)?;
    PostAttribution::export_all(config)?;
    CheckedBy::export_all(config)?;
    DraftOp::export_all(config)?;
    DraftPart::export_all(config)?;
    DraftUpdated::export_all(config)?;
    DraftApplied::export_all(config)?;
    DraftDiscarded::export_all(config)?;
    StaleReason::export_all(config)?;
    DraftStale::export_all(config)?;
    RunState::export_all(config)?;
    RunStatus::export_all(config)?;
    FindingKind::export_all(config)?;
    Finding::export_all(config)?;
    RunFinding::export_all(config)?;
    EndingCounts::export_all(config)?;
    SubmissionEnded::export_all(config)?;
    ProgressValue::export_all(config)?;
    RunProgress::export_all(config)?;
    AttemptNumber::export_all(config)?;
    EndedBy::export_all(config)?;
    FindingPart::export_all(config)?;
    FindingSide::export_all(config)?;
    RunPosition::export_all(config)?;
    Expectation::export_all(config)?;
    Plan::export_all(config)?;
    PlanAccepted::export_all(config)?;
    PlanFormat::export_all(config)?;
    PlanProgress::export_all(config)?;
    PlanRead::export_all(config)?;
    PlanRequest::export_all(config)?;
    PlanRunner::export_all(config)?;
    PlanScenario::export_all(config)?;
    PlanState::export_all(config)?;
    PlannedRun::export_all(config)?;
    QueueSnapshot::export_all(config)?;
    RunnerChosen::export_all(config)?;
    SubmitRequest::export_all(config)?;
    Submitted::export_all(config)?;
    WorldCount::export_all(config)?;
    Aggregation::export_all(config)?;
    AttemptControl::export_all(config)?;
    AppliedPatch::export_all(config)?;
    ByteCount::export_all(config)?;
    CompileRead::export_all(config)?;
    DeclaredMetric::export_all(config)?;
    Endpoint::export_all(config)?;
    Ending::export_all(config)?;
    ExportRead::export_all(config)?;
    FileFormat::export_all(config)?;
    JobSpec::export_all(config)?;
    Manifest::export_all(config)?;
    ManifestAccepted::export_all(config)?;
    Heartbeat::export_all(config)?;
    RecordingStored::export_all(config)?;
    ResultsReport::export_all(config)?;
    ResultsEnded::export_all(config)?;
    ResultsAccepted::export_all(config)?;
    SealedVersion::export_all(config)?;
    MetricBasis::export_all(config)?;
    MetricSeries::export_all(config)?;
    MetricUnit::export_all(config)?;
    MetricsFile::export_all(config)?;
    PinnedImage::export_all(config)?;
    PinnedScenario::export_all(config)?;
    Pins::export_all(config)?;
    SeriesFile::export_all(config)?;
    SeriesKind::export_all(config)?;
    SeriesPoint::export_all(config)?;
    WorkloadRead::export_all(config)?;
    WorldRead::export_all(config)?;
    RunnerMessage::export_all(config)?;
    RunnerReply::export_all(config)?;
    ViewerRequest::export_all(config)?;
    ViewerMessage::export_all(config)?;
    StreamProgress::export_all(config)?;
    StreamEnded::export_all(config)?;
    StreamRefusal::export_all(config)?;
    StreamingServer::export_all(config)?;
    AttemptRead::export_all(config)?;
    CancelAnswer::export_all(config)?;
    CancelOutcome::export_all(config)?;
    CompiledRead::export_all(config)?;
    RecordingRead::export_all(config)?;
    RunCancelled::export_all(config)?;
    RunCut::export_all(config)?;
    RunDetail::export_all(config)?;
    RunList::export_all(config)?;
    RunProgressRead::export_all(config)?;
    RunQueue::export_all(config)?;
    RunStats::export_all(config)?;
    SeatsRead::export_all(config)?;
    RunScenario::export_all(config)?;
    RunSummary::export_all(config)?;
    RunnerImage::export_all(config)?;
    RunnerImages::export_all(config)?;
    SubmissionCancelled::export_all(config)?;
    WatchTicket::export_all(config)?;
    VariedValue::export_all(config)?;
    Waiting::export_all(config)?;
    WaitingFor::export_all(config)?;
    WallMs::export_all(config)?;
    template_types(config)?;
    scenario_types(config)?;
    metric_stream_types(config)?;
    flow_types(config)
}

/// Spec 06's payloads (brief I1.11): the flow read, the parameter list and flow-validate.
fn flow_types(config: &Config) -> Result<(), ExportError> {
    FlowRead::export_all(config)?;
    FlowNode::export_all(config)?;
    FlowSector::export_all(config)?;
    FlowAllIn::export_all(config)?;
    FlowEdge::export_all(config)?;
    FlowServedBy::export_all(config)?;
    FlowSegment::export_all(config)?;
    FlowPlace::export_all(config)?;
    FlowExecutor::export_all(config)?;
    FlowReturnRule::export_all(config)?;
    FlowPlan::export_all(config)?;
    FlowRoot::export_all(config)?;
    FlowSetting::export_all(config)?;
    FlowSetup::export_all(config)?;
    FlowParameters::export_all(config)?;
    FlowParameter::export_all(config)?;
    FlowChoice::export_all(config)?;
    FlowValidate::export_all(config)?;
    FlowPatch::export_all(config)?;
    FlowOrderKind::export_all(config)?;
    FlowValidated::export_all(config)?;
    FlowDraft::export_all(config)?;
    FlowBase::export_all(config)?;
    FlowMadeFrom::export_all(config)?;
    FlowDiff::export_all(config)?;
    FlowFieldDiff::export_all(config)?;
    draft_types(config)
}

/// Spec 01 section 3.11's person's draft changeset (brief I1.11).
fn draft_types(config: &Config) -> Result<(), ExportError> {
    ChangesetOp::export_all(config)?;
    NewDraft::export_all(config)?;
    AmendDraft::export_all(config)?;
    ChangesetPart::export_all(config)?;
    DraftChecked::export_all(config)?;
    DraftRead::export_all(config)?;
    DraftList::export_all(config)?;
    results_types(config)
}

/// Spec 08's result routes (brief I1.10): metrics over a window, a series by period, a run's inputs, a comparison, and
/// a project's pinned metrics.
fn results_types(config: &Config) -> Result<(), ExportError> {
    SimWindow::export_all(config)?;
    ValueBasis::export_all(config)?;
    MetricRow::export_all(config)?;
    MetricsReply::export_all(config)?;
    SeriesShape::export_all(config)?;
    PeriodPoint::export_all(config)?;
    SeriesReply::export_all(config)?;
    InputSection::export_all(config)?;
    InputRow::export_all(config)?;
    RunInputs::export_all(config)?;
    ComparedRun::export_all(config)?;
    InputValue::export_all(config)?;
    InputField::export_all(config)?;
    SameField::export_all(config)?;
    NotCompared::export_all(config)?;
    ComparedValue::export_all(config)?;
    ComparedMetric::export_all(config)?;
    CompareReply::export_all(config)?;
    MetricPins::export_all(config)
}

/// One metric of an attempt as the streaming server serves it (the metric streams, November 2026).
fn metric_stream_types(config: &Config) -> Result<(), ExportError> {
    TrailingSpec::export_all(config)?;
    Bucket::export_all(config)?;
    Dimension::export_all(config)?;
    MetricSource::export_all(config)?;
    MetricDescriptor::export_all(config)?;
    MetricCatalogue::export_all(config)?;
    MetricPoint::export_all(config)?;
    MetricRequest::export_all(config)?;
    RunOutcome::export_all(config)?;
    MetricMessage::export_all(config)
}

/// Spec 05's payloads (brief I1.07b) and the approval a page reads after a press.
fn template_types(config: &Config) -> Result<(), ExportError> {
    KnobUnit::export_all(config)?;
    KnobValue::export_all(config)?;
    KnobOption::export_all(config)?;
    KnobDefault::export_all(config)?;
    KnobDeclaration::export_all(config)?;
    TemplateDescription::export_all(config)?;
    Census::export_all(config)?;
    EvaluationProblem::export_all(config)?;
    EvaluationNote::export_all(config)?;
    ThumbnailState::export_all(config)?;
    Thumbnail::export_all(config)?;
    TemplateCard::export_all(config)?;
    TemplateList::export_all(config)?;
    TemplateDetail::export_all(config)?;
    TemplateDraftState::export_all(config)?;
    DraftMode::export_all(config)?;
    EvaluationState::export_all(config)?;
    SetByKind::export_all(config)?;
    KnobSetBy::export_all(config)?;
    EvaluationNumber::export_all(config)?;
    EvaluationRef::export_all(config)?;
    TemplateAt::export_all(config)?;
    StartDraft::export_all(config)?;
    DraftStarted::export_all(config)?;
    SiteMade::export_all(config)?;
    DraftWrite::export_all(config)?;
    TemplateDraft::export_all(config)?;
    SetKnobs::export_all(config)?;
    KnobsSet::export_all(config)?;
    EvaluationPrologue::export_all(config)?;
    GeneratorUsed::export_all(config)?;
    EvaluationEngine::export_all(config)?;
    KeyChange::export_all(config)?;
    KeyChanges::export_all(config)?;
    FloorRef::export_all(config)?;
    KnobSource::export_all(config)?;
    TemplateEvaluation::export_all(config)?;
    EvaluationEvent::export_all(config)?;
    KnobChange::export_all(config)?;
    KnobsChanged::export_all(config)?;
    CreateSite::export_all(config)?;
    WriteAccepted::export_all(config)?;
    MadeFromKnob::export_all(config)?;
    MadeFrom::export_all(config)?;
    MadeFromList::export_all(config)?;
    ApprovalVia::export_all(config)?;
    ApprovalRecord::export_all(config)?;
    ApprovalAnswer::export_all(config)?;
    map_types(config)
}

/// Spec 03 sections 3.5 and 3.6's payloads (brief I1.12).
fn map_types(config: &Config) -> Result<(), ExportError> {
    Mm::export_all(config)?;
    UnitMm::export_all(config)?;
    EntityCount::export_all(config)?;
    MapSite::export_all(config)?;
    MapFrame::export_all(config)?;
    FrameAxes::export_all(config)?;
    CountRow::export_all(config)?;
    MapSummary::export_all(config)?;
    Mark::export_all(config)?;
    ResolveRequest::export_all(config)?;
    RefMarks::export_all(config)?;
    Resolution::export_all(config)?;
    FindResults::export_all(config)
}

/// Spec 07's scenarios, experiments and workloads, and spec 01's panel events.
fn scenario_types(config: &Config) -> Result<(), ExportError> {
    CreateScenario::export_all(config)?;
    CreatedScenario::export_all(config)?;
    DuplicateScenario::export_all(config)?;
    FieldCatalogue::export_all(config)?;
    FieldChange::export_all(config)?;
    FieldConflict::export_all(config)?;
    FieldEntry::export_all(config)?;
    FieldOption::export_all(config)?;
    FieldSet::export_all(config)?;
    FillPercent::export_all(config)?;
    FloorTraffic::export_all(config)?;
    GeneratorSpec::export_all(config)?;
    LinesPerOrder::export_all(config)?;
    OrderTrays::export_all(config)?;
    OrdersPerHour::export_all(config)?;
    Release::export_all(config)?;
    ReportWindow::export_all(config)?;
    Revision::export_all(config)?;
    RunControls::export_all(config)?;
    RunCount::export_all(config)?;
    RunCounts::export_all(config)?;
    ScenarioContent::export_all(config)?;
    ScenarioDiff::export_all(config)?;
    ScenarioEdit::export_all(config)?;
    ScenarioEdited::export_all(config)?;
    ScenarioList::export_all(config)?;
    ScenarioRead::export_all(config)?;
    ScenarioRow::export_all(config)?;
    ScenarioSummary::export_all(config)?;
    Seed::export_all(config)?;
    SetBy::export_all(config)?;
    SetInTurn::export_all(config)?;
    Setter::export_all(config)?;
    SkuCount::export_all(config)?;
    UnitsPerTote::export_all(config)?;
    VersionNumber::export_all(config)?;
    VersionRead::export_all(config)?;
    VersionState::export_all(config)?;
    VersionSummary::export_all(config)?;
    Workload::export_all(config)?;
    AppliedTo::export_all(config)?;
    Experiment::export_all(config)?;
    ExperimentKind::export_all(config)?;
    ExperimentRead::export_all(config)?;
    ExperimentRequest::export_all(config)?;
    Patch::export_all(config)?;
    Repeats::export_all(config)?;
    RunOverrides::export_all(config)?;
    RunSlot::export_all(config)?;
    Sweep::export_all(config)?;
    Variant::export_all(config)?;
    VariantNumber::export_all(config)?;
    Varied::export_all(config)?;
    VariesRequest::export_all(config)?;
    GenerateRequest::export_all(config)?;
    OrderLine::export_all(config)?;
    PreloadFraction::export_all(config)?;
    Readiness::export_all(config)?;
    SlotCount::export_all(config)?;
    Tally::export_all(config)?;
    UnitCut::export_all(config)?;
    WorkloadHeader::export_all(config)?;
    WorkloadItem::export_all(config)?;
    WorkloadLine::export_all(config)?;
    WorkloadOrder::export_all(config)?;
    WorkloadSummary::export_all(config)?;
    PanelEventKind::export_all(config)?;
    PanelEventWritten::export_all(config)
}

/// The refusals after which a viewer stops attaching, from `StreamRefusal::is_final_for_viewer`, so a page reads the
/// rule from the protocol rather than keeping its own list.
fn export_refusal_rules(dir: &Path) {
    let reasons: Vec<String> = StreamRefusal::ALL
        .into_iter()
        .filter(|refusal| refusal.is_final_for_viewer())
        .map(|refusal| format!("\"{}\"", refusal.reason()))
        .collect();
    let text = format!(
        "{GENERATED_HERE}. Do not edit this file manually.\n\
         import type {{ StreamRefusal }} from \"./run_stream\";\n\n\
         /** The refusals after which a viewer stops attaching: another attach would meet the same one. */\n\
         export const VIEWER_FINAL: ReadonlySet<StreamRefusal> = new Set<StreamRefusal>([{}]);\n",
        reasons.join(", ")
    );
    fs::write(dir.join("run_stream_rules.ts"), text).expect("the refusal rules are written");
}

/// Every generated file under `dir`, by its path relative to `dir`.
fn generated_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    // ordered: the drift report lists files in path order.
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.map(|entry| entry.expect("a folder entry reads")) {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if let Ok(text) = fs::read_to_string(&path)
                && (text.starts_with(GENERATED) || text.starts_with(GENERATED_HERE))
            {
                found.insert(path.strip_prefix(dir).expect("under the folder").to_path_buf(), text);
            }
        }
    }
    found
}

/// Spec 01 section 8, statement 11: the generated TypeScript equals the checked-in copy.
#[test]
fn the_pages_protocol_is_what_the_service_says() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_nanos();
    let fresh = env::temp_dir().join(format!("studio-protocol-ts-{}-{nanos}", process::id()));
    // ts-rs would render a 64-bit integer as `bigint`, but every value here crosses the wire as JSON, which
    // `JSON.parse` hands back as a plain `number`.
    let config = Config::new().with_large_int("number").with_out_dir(&fresh);
    export_every_type(&config).expect("every type exports");
    export_refusal_rules(&fresh);

    let generated = generated_files(&fresh);
    let committed_dir = committed_dir();
    let committed = generated_files(&committed_dir);
    let mut drift = Vec::new();
    for (path, text) in &generated {
        if committed.get(path) != Some(text) {
            let target = committed_dir.join(path);
            fs::create_dir_all(target.parent().expect("a file has a folder")).expect("the folder is made");
            fs::write(&target, text).expect("the binding is written");
            drift.push(format!("rewritten: {}", path.display()));
        }
    }
    for path in committed.keys().filter(|path| !generated.contains_key(*path)) {
        drift.push(format!("stale, delete it: {}", path.display()));
    }
    fs::remove_dir_all(&fresh).expect("the scratch folder is removed");

    assert!(
        drift.is_empty(),
        "{} is out of date; this test has just rewritten it: read the change and commit it\n{}",
        committed_dir.display(),
        drift.join("\n")
    );
}
