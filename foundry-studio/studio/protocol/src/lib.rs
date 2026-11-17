//! Agent Studio's protocol: the value types the API service, the agent loop and the studio pages exchange. Each is
//! written once here and generated into TypeScript under `studio/ui/src/protocol/` by `tests/ts_bindings.rs`, so a page
//! cannot hold a shape the service does not send.
//!
//! It holds spec 03 section 3.1's reference grammar (`grammar`), spec 03 section 3.3's panel commands (`panel`),
//! spec 01 section 3.5's journal envelope (`journal`) and the stream's live-only items (`stream`), the bodies of the
//! journal's kinds with the request bodies of their routes (a person's `message`, a `turn` and its `tool` calls, a
//! `question`, an `approval`, a `draft` changeset, a simulation `run`), spec 01 section 3.14's problem body
//! (`problem`), and the payloads of the routes the API mounts so far (`person`; a site's conversations in
//! `conversation`, spec 01 sections 3.6 and 3.17; how a tab reads one in `snapshot`, and what its entries name in
//! `projection`; the site list in `site` and a followed scope's moves in `scope_events`; spec 05's templates in
//! `template`, a template draft in `template_draft`, one evaluation in `template_evaluation`, Create site and a site's
//! made-from records in `made_from`, and an approval as a page reads it in `approval`; spec 03's map summary, marks,
//! resolve and find in `map`), and the local test writer's (`dev`). Spec 07's scenarios, experiments and generated
//! workloads are `scenario`, `experiment` and `workload`, and a panel's own act told to the agent is `panel_event`.
//! Spec 06's flow reads are `flow`, and a person's draft changeset as its routes carry it (spec 01 section 3.11) is
//! `changeset_draft`. Spec 08's submission plan is `plan`, its run routes' answers `runs`, and the runner contract its
//! runners and the API share is `runner`.
//! An ended run's results, its inputs as values and a comparison of runs are `results` (spec 08 sections 3.9 to 3.12).
//! A run's live stream of messages and frames is `run_stream` (ruling D174), and one metric of a run computed by the
//! streaming server from its recording and served as its own stream is `metric_stream`.
//! The JSON form a digest is taken over, and the flow plan's digest (spec 06 section 3.3), are `canonical`.

pub mod approval;
pub mod canonical;
pub mod changeset_draft;
pub mod clock;
pub mod conversation;
pub mod dev;
pub mod draft;
pub mod experiment;
pub mod flow;
pub mod grammar;
pub mod journal;
pub mod made_from;
pub mod map;
pub mod message;
pub mod metric_stream;
pub mod page;
pub mod panel;
pub mod panel_event;
pub mod person;
pub mod plan;
pub mod problem;
pub mod projection;
pub mod question;
pub mod results;
pub mod run;
pub mod run_stream;
pub mod runner;
pub mod runs;
pub mod scenario;
pub mod scope_events;
pub mod site;
pub mod snapshot;
pub mod stream;
pub mod template;
pub mod template_draft;
pub mod template_evaluation;
pub mod tool;
pub mod turn;
pub mod workload;

pub use approval::{
    ApprovalAnswer, ApprovalDecided, ApprovalExecuted, ApprovalExpired, ApprovalFacts, ApprovalNoted, ApprovalRecord,
    ApprovalRequested, ApprovalVia, Decision, FactRow, NoteKind, PostAttribution, PostDecision, Unattributable,
};
pub use canonical::{flow_plan_digest, write_canonical};
pub use changeset_draft::{AmendDraft, ChangesetOp, ChangesetPart, DraftChecked, DraftList, DraftRead, NewDraft};
pub use clock::{Clock, DurationClock, WallClock};
pub use conversation::{
    Conversation, ConversationChanged, ConversationList, ConversationState, ConversationSummary, Dismissed,
    OpenConversation, OpenedConversation, WaitingOn,
};
pub use dev::{DevEntry, DevLive, DevWrite, DevWritten};
pub use draft::{CheckedBy, DraftApplied, DraftDiscarded, DraftOp, DraftPart, DraftStale, DraftUpdated, StaleReason};
pub use experiment::{
    AppliedTo, Experiment, ExperimentKind, ExperimentRead, ExperimentRequest, Patch, Repeats, RunOverrides, RunSlot,
    Sweep, Variant, VariantNumber, Varied, VariesRequest,
};
pub use flow::{
    FlowAllIn, FlowBase, FlowChoice, FlowDiff, FlowDraft, FlowEdge, FlowExecutor, FlowFieldDiff, FlowMadeFrom,
    FlowNode, FlowOrderKind, FlowParameter, FlowParameters, FlowPatch, FlowPlace, FlowPlan, FlowRead, FlowReturnRule,
    FlowRoot, FlowSector, FlowSegment, FlowServedBy, FlowSetting, FlowSetup, FlowValidate, FlowValidated,
};
pub use grammar::{
    BranchName, DeclaredId, EntityId, EntityRef, FieldKey, GrammarError, Key, KinPin, KinPins, Kinship, Location,
    ObjectKey, ObjectKind, Pin, PinRuleBreach, PinRuleError, Ref, RefPin, ScopePath, Seq, SitePath, TagName, Ulid,
};
pub use journal::{EnvelopeVersion, JournalEntry, JournalKind, JournalSeq};
pub use made_from::{CreateSite, MadeFrom, MadeFromKnob, MadeFromList, WriteAccepted};
pub use map::{
    CountRow, EntityCount, FindResults, FrameAxes, MapFrame, MapSite, MapSummary, Mark, Mm, RefMarks, Resolution,
    ResolveRequest, UnitMm,
};
pub use message::{MessageAccepted, MessageSource, PostMessage, UserMessage};
pub use metric_stream::{
    Bucket, Dimension, METRIC_STREAM_FORMAT, MetricCatalogue, MetricDescriptor, MetricMessage, MetricPoint,
    MetricRequest, MetricSource, RunOutcome, TrailingSpec,
};
pub use page::{ContextPanel, ContextSelection, PageContext};
pub use panel::{
    FlowLayer, GroupMarkCounts, HighlightGroup, HighlightGroupInput, MarkCount, Panel, PanelAction, PanelClearInput,
    PanelCommandEntry, PanelCommandResult, PanelHighlightInput, PanelSelectInput, PanelShowInput, PanelTarget,
    ScopeSeq,
};
pub use panel_event::{PanelEvent, PanelEventKind, PanelEventWritten};
pub use person::{AgentModel, BlankSubject, Me, ModelProvider, Person, Subject};
pub use plan::{
    Expectation, Plan, PlanAccepted, PlanFormat, PlanProgress, PlanRead, PlanRequest, PlanRunner, PlanScenario,
    PlanState, PlannedRun, QueueSnapshot, RunnerChosen, SubmitRequest, Submitted, WorldCount,
};
pub use problem::{MovedScope, Problem, ProblemItem, ProblemType, Severity};
pub use projection::{
    ApprovalState, CallId, CallState, DraftState, Execution, ProjectedApproval, ProjectedCall, ProjectedDraft,
    ProjectedQuestion, ProjectedRun, ProjectedTurn, Projection, QuestionState, TurnNumber, TurnState,
};
pub use question::{
    AnswersAccepted, ItemAnswer, PostAnswers, QuestionAnswered, QuestionAsked, QuestionExpired, QuestionItem,
    QuestionOption, QuestionRef,
};
pub use results::{
    CompareReply, ComparedMetric, ComparedRun, ComparedValue, InputField, InputRow, InputSection, InputValue,
    MetricPins, MetricRow, MetricsReply, NotCompared, PeriodPoint, RunInputs, SameField, SeriesReply, SeriesShape,
    SimWindow, ValueBasis,
};
pub use run::{
    AttemptNumber, EndedBy, EndingCounts, Finding, FindingKind, FindingPart, FindingSide, ProgressValue, RunFinding,
    RunPosition, RunProgress, RunState, RunStatus, SubmissionEnded,
};
pub use run_stream::{
    RunnerMessage, RunnerReply, STREAM_FORMAT, StreamEnded, StreamProgress, StreamRefusal, ViewerMessage, ViewerRequest,
};
pub use runner::{
    Aggregation, AppliedPatch, AttemptControl, ByteCount, CompileRead, DELIVERY_BUDGET, DeclaredMetric, Ending,
    Endpoint, ExportRead, FORMAT, FileFormat, HOLD_VAR, Heartbeat, JobSpec, Manifest, ManifestAccepted, MetricBasis,
    MetricSeries, MetricUnit, MetricsFile, PinnedImage, PinnedScenario, Pins, RUNNER_SETTINGS, RecordingStored,
    ResultsAccepted, ResultsEnded, ResultsReport, SealedVersion, SeriesFile, SeriesKind, SeriesPoint, StreamingServer,
    THREADS_VAR, TOKEN_VAR, WorkloadRead, WorldRead,
};
pub use runs::{
    AttemptRead, CancelAnswer, CancelOutcome, CompiledRead, RecordingRead, RunCancelled, RunCut, RunDetail, RunList,
    RunProgressRead, RunQueue, RunScenario, RunStats, RunSummary, RunnerImage, RunnerImages, SeatsRead,
    SubmissionCancelled, VariedValue, Waiting, WaitingFor, WallMs, WatchTicket,
};
pub use scenario::{
    CreateScenario, CreatedScenario, DuplicateScenario, FieldCatalogue, FieldChange, FieldConflict, FieldEntry,
    FieldOption, FieldSet, FillPercent, FloorTraffic, GeneratorSpec, LinesPerOrder, OrderTrays, OrdersPerHour, Release,
    ReportWindow, Revision, RunControls, RunCount, RunCounts, ScenarioContent, ScenarioDiff, ScenarioEdit,
    ScenarioEdited, ScenarioList, ScenarioRead, ScenarioRow, ScenarioSummary, Seed, SetBy, SetInTurn, Setter, SkuCount,
    UnitsPerTote, VersionNumber, VersionRead, VersionState, VersionSummary, Workload,
};
pub use scope_events::{ScopeEventKind, ScopeMoved};
pub use site::{SiteBranch, SiteConversation, SiteItem, SiteList};
pub use snapshot::{ActiveTurn, EntriesPage, Pending, Snapshot};
pub use stream::{
    BlockId, LiveItem, LiveKind, Ping, ResetReason, StreamKind, StreamLive, StreamReset, StreamingBlock, TextDelta,
    TextOffset,
};
pub use template::{
    Census, EvaluationNote, EvaluationProblem, KnobDeclaration, KnobDefault, KnobOption, KnobUnit, KnobValue,
    TemplateCard, TemplateDescription, TemplateDetail, TemplateList, Thumbnail, ThumbnailState,
};
pub use template_draft::{
    DraftMode, DraftStarted, DraftWrite, EvaluationEvent, EvaluationNumber, EvaluationRef, EvaluationState, KnobChange,
    KnobSetBy, KnobsChanged, KnobsSet, SetByKind, SetKnobs, SiteMade, StartDraft, TemplateAt, TemplateDraft,
    TemplateDraftState,
};
pub use template_evaluation::{
    EvaluationEngine, EvaluationPrologue, FloorRef, GeneratorUsed, KeyChange, KeyChanges, KnobSource,
    TemplateEvaluation,
};
pub use tool::{ToolFinished, ToolProgress, ToolStarted};
pub use turn::{
    CallRead, ResolvedRef, ResumedBy, StopAccepted, SystemNote, TextDone, TextKind, TurnCalls, TurnFinished,
    TurnResumed, TurnStarted, TurnWaiting, Usage,
};
pub use workload::{
    GenerateRequest, OrderLine, PreloadFraction, Readiness, SlotCount, Tally, UnitCut, WorkloadHeader, WorkloadItem,
    WorkloadLine, WorkloadOrder, WorkloadSummary,
};
