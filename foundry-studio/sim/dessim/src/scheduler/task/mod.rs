pub mod proposal;

use crate::task::{TaskContext, invalid, task_name};
use serde_json::Value;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;

use super::barrier::Barrier;
use super::{TaskGraph, TaskGroupId};
use crate::model::SimTime;
use crate::model::bot::plan::PlanError;
use crate::model::ids::TaskId;
use crate::signals::Version;
use proposal::{Proposal, ProposalId};

#[derive(Clone, Copy, Debug)]
pub struct TaskIdentity {
    pub id: TaskId,
    pub group_id: TaskGroupId,
}

impl fmt::Display for TaskIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "task={} group={}", self.id, self.group_id)
    }
}

// The two-phase contract: `propose` selects and registers a revocable
// candidacy through the task's own service handles — visible to
// acceptance arbitration, consuming no capacity, deregistered on drop.
// Acceptance converts it in place into the held reservation; `schedule`
// spends it and returns the facts a spawning parent may await.
#[async_trait(?Send)]
pub trait Task: fmt::Display {
    type Proposal: Proposal;

    type Schedule: 'static;

    /// Retire obsolete work. The drive asks before each propose, right after
    /// it (proposing can await while the work is taken), and after an evicted
    /// attempt. The typed result resolves dependents without calling
    /// `schedule`.
    fn dissolve(&self) -> Option<Self::Schedule> {
        None
    }

    /// Synchronous factory used only for registered composition roots. Children
    /// are constructed by their parent with ordinary typed arguments.
    fn create(_context: &TaskContext) -> Result<Self, PlanError>
    where
        Self: Sized,
    {
        Err(invalid(format!("{} has no composition root factory", task_name::<Self>())))
    }

    fn holds_completion() -> bool
    where
        Self: Sized,
    {
        true
    }

    fn validate_config(config: &Value) -> Result<(), PlanError>
    where
        Self: Sized,
    {
        if config.as_object().is_some_and(|fields| fields.is_empty()) {
            Ok(())
        } else {
            Err(invalid(format!("{} accepts no configuration", task_name::<Self>())))
        }
    }

    async fn propose(&self, owner: ProposalId) -> Result<Self::Proposal, PlanError>;

    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError>;
}

// The planner's object-safe view. The blanket impl is the one place a
// proposal is downcast back; the drive frame returns each proposal to
// the task that crafted it, so a mismatch is a planner bug.
#[async_trait(?Send)]
pub(crate) trait ErasedTask: fmt::Display {
    fn task_type(&self) -> &'static str;
    fn dissolve(&self) -> Option<Box<dyn Any>>;
    async fn propose(&self, owner: ProposalId) -> Result<Box<dyn Proposal>, PlanError>;
    async fn schedule(&mut self, proposal: Box<dyn Proposal>, ctx: SchedCtx) -> Result<Box<dyn Any>, PlanError>;
}

#[async_trait(?Send)]
impl<T: Task> ErasedTask for T {
    fn task_type(&self) -> &'static str {
        task_name::<T>()
    }

    fn dissolve(&self) -> Option<Box<dyn Any>> {
        Task::dissolve(self).map(|result| Box::new(result) as Box<dyn Any>)
    }

    async fn propose(&self, owner: ProposalId) -> Result<Box<dyn Proposal>, PlanError> {
        Ok(Box::new(Task::propose(self, owner).await?))
    }

    async fn schedule(&mut self, proposal: Box<dyn Proposal>, ctx: SchedCtx) -> Result<Box<dyn Any>, PlanError> {
        let proposal: Box<dyn Any> = proposal;
        let proposal = proposal.downcast::<T::Proposal>().map_err(|_| PlanError::TaskGraphInvariant {
            context: "drive",
            detail: "planner delivered a foreign proposal".into(),
        })?;
        Ok(Box::new(Task::schedule(self, *proposal, ctx).await?))
    }
}

pub(crate) struct TaskCell {
    task_type: &'static str,
    label: RefCell<String>,

    pending_proposal: RefCell<Option<PublishedProposal>>,

    accepted: RefCell<Option<Box<dyn Proposal>>>,

    waker: RefCell<Option<Waker>>,

    version: Version,
}

struct PublishedProposal {
    id: ProposalId,
    proposal: Box<dyn Proposal>,
}

impl TaskCell {
    pub(crate) fn new(version: Version, task_type: &'static str) -> Rc<Self> {
        Rc::new(Self {
            task_type,
            label: RefCell::new(String::new()),
            pending_proposal: RefCell::new(None),
            accepted: RefCell::new(None),
            waker: RefCell::new(None),
            version,
        })
    }

    pub(crate) fn label(&self) -> String {
        self.label.borrow().clone()
    }

    pub(crate) fn task_type(&self) -> &'static str {
        self.task_type
    }

    pub(crate) fn take_pending_proposal(&self) -> Option<Box<dyn Proposal>> {
        self.pending_proposal.borrow_mut().take().map(|published| published.proposal)
    }

    pub(crate) fn take_pending_attempt(&self, id: ProposalId) -> Option<Box<dyn Proposal>> {
        let mut pending = self.pending_proposal.borrow_mut();
        if pending.as_ref().is_some_and(|published| published.id == id) {
            pending.take().map(|published| published.proposal)
        } else {
            None
        }
    }

    pub(crate) fn pending_id(&self) -> Option<ProposalId> {
        self.pending_proposal.borrow().as_ref().map(|published| published.id)
    }

    pub(crate) fn pending_ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        self.pending_proposal.borrow().as_ref().and_then(|published| published.proposal.ranked_competitors())
    }

    pub(crate) fn deliver_accepted(&self, proposal: Box<dyn Proposal>) {
        let prior = self.accepted.borrow_mut().replace(proposal);
        debug_assert!(prior.is_none(), "acceptance delivered twice to one promotion point");
        self.wake();
    }

    pub(crate) fn wake(&self) {
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }

    // The scheduler's generic views of the published candidacy, if any.
    pub(crate) fn pending_resolve_by(&self) -> Option<SimTime> {
        self.pending_proposal.borrow().as_ref().and_then(|published| {
            match (published.proposal.resolve_by(), published.proposal.start_time()) {
                (Some(decision), Some(start)) => Some(decision.min(start)),
                (decision, start) => decision.or(start),
            }
        })
    }

    pub(crate) fn pending_start_time(&self) -> Option<SimTime> {
        self.pending_proposal.borrow().as_ref().and_then(|published| published.proposal.start_time())
    }

    pub(crate) fn pending_poisoned(&self) -> bool {
        self.pending_proposal.borrow().as_ref().is_some_and(|published| published.proposal.poisoned())
    }

    pub(crate) fn pending_admissible(&self) -> bool {
        self.pending_proposal.borrow().as_ref().map_or(true, |published| published.proposal.admissible())
    }
}

pub struct SchedCtx {
    identity: TaskIdentity,
    cell: Rc<TaskCell>,
    graph: Rc<RefCell<TaskGraph>>,
    barrier: Rc<Barrier>,
}

impl SchedCtx {
    pub(crate) fn new(
        identity: TaskIdentity,
        cell: Rc<TaskCell>,
        graph: Rc<RefCell<TaskGraph>>,
        barrier: Rc<Barrier>,
    ) -> Self {
        Self { identity, cell, graph, barrier }
    }

    pub fn identity(&self) -> TaskIdentity {
        self.identity
    }

    /// The trailing global barrier — the scheduler's settled present.
    pub fn barrier(&self) -> &Barrier {
        &self.barrier
    }

    /// Accept a specific task's published proposal: the pull surface a
    /// family service or demand uses when its resource frees. Refusal is
    /// a family-service bug and aborts the run.
    pub fn accept_proposal(&self, task: TaskId) -> Result<(), PlanError> {
        super::accept_task(&self.graph, &self.barrier, task)
    }

    /// Evict a specific task's published proposal; the owner wakes and
    /// re-proposes under current state. Returns false if none is pending.
    pub fn evict_proposal(&self, task: TaskId) -> bool {
        super::evict_task(&self.graph, task)
    }

    fn await_accept(&self, owner: ProposalId, proposal: Box<dyn Proposal>) -> AwaitAccept<'_> {
        AwaitAccept { cell: &self.cell, owner, proposal: Some(proposal), published: false }
    }

    pub fn add_task<T: Task + 'static>(&self, task: T) -> Completion<T::Schedule> {
        Completion::typed(TaskGraph::spawn(&self.graph, self.identity, Box::new(task)))
    }

    /// Resolve policy defaults for an ordinary child constructor. The parent
    /// passes runtime facts directly and calls `finish` after reading parameters.
    pub fn task_context<T: Task>(&self, config: Value) -> Result<TaskContext, PlanError> {
        let graph = self.graph.borrow();
        TaskContext::new::<T>(graph.states.clone(), &graph.task_defaults, config)
    }

    pub(crate) fn record_label(&self, label: String) {
        *self.cell.label.borrow_mut() = label;
    }
}

pub(crate) struct CompletionState {
    done: Cell<bool>,
    canceled: Cell<bool>,
    version: Version,
    schedule: RefCell<Option<Box<dyn Any>>>,
    waker: RefCell<Option<Waker>>,
}

impl CompletionState {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            done: Cell::new(false),
            canceled: Cell::new(false),
            version: Version::new(),
            schedule: RefCell::new(None),
            waker: RefCell::new(None),
        })
    }

    pub(crate) fn complete(&self, schedule: Box<dyn Any>) {
        *self.schedule.borrow_mut() = Some(schedule);
        self.done.set(true);
        self.version.bump();
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }

    pub(crate) fn cancel(&self) {
        self.canceled.set(true);
        self.done.set(true);
        self.version.bump();
    }
}

// Resolves to the spawned task's Schedule once it commits. Optional to
// await — the spawn is already effective; teardown cancellation leaves
// it pending, and the runtime drop reaps the awaiting parent with it.
pub struct Completion<S> {
    state: Rc<CompletionState>,
    schedule: PhantomData<fn() -> S>,
}

/// Broadcast completion dependency; unlike `Completion<S>`, this does not
/// consume the task's typed return value and supports multiple dependent roots.
#[derive(Clone)]
pub struct TaskDependency(Rc<CompletionState>);

impl TaskDependency {
    pub async fn wait(&self) -> Result<(), PlanError> {
        loop {
            let seen = self.0.version.seen();
            if self.0.canceled.get() {
                return Err(invalid("a prerequisite task was canceled"));
            }
            if self.0.done.get() {
                return Ok(());
            }
            self.0.version.past(seen).await;
        }
    }
}

impl<S> Completion<S> {
    pub fn dependency(&self) -> TaskDependency {
        TaskDependency(self.state.clone())
    }

    pub(crate) fn typed(state: Rc<CompletionState>) -> Self {
        Completion { state, schedule: PhantomData }
    }
}

impl<S: 'static> Future for Completion<S> {
    type Output = S;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<S> {
        if self.state.done.get() {
            if let Some(schedule) = self.state.schedule.borrow_mut().take() {
                match schedule.downcast::<S>() {
                    Ok(schedule) => return Poll::Ready(*schedule),
                    Err(_) => debug_assert!(false, "completion schedule type mismatch"),
                }
            }
            Poll::Pending
        } else {
            *self.state.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

enum AcceptOutcome {
    Accepted(Box<dyn Proposal>),

    Evicted,
}

struct AwaitAccept<'a> {
    owner: ProposalId,
    cell: &'a TaskCell,
    proposal: Option<Box<dyn Proposal>>,
    published: bool,
}

impl Future for AwaitAccept<'_> {
    type Output = AcceptOutcome;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(proposal) = self.proposal.take() {
            let prior = self.cell.pending_proposal.borrow_mut().replace(PublishedProposal { id: self.owner, proposal });
            debug_assert!(prior.is_none(), "two proposals published to one promotion point");
            self.published = true;
            self.cell.version.bump();
        }
        if let Some(proposal) = self.cell.accepted.borrow_mut().take() {
            return Poll::Ready(AcceptOutcome::Accepted(proposal));
        }
        if self.published && self.cell.pending_proposal.borrow().is_none() {
            return Poll::Ready(AcceptOutcome::Evicted);
        }
        *self.cell.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

pub(crate) enum TaskRunOutcome {
    Scheduled(Box<dyn Any>),
    Dissolved(Box<dyn Any>),
}

pub(crate) type TaskFuture = Pin<Box<dyn Future<Output = Result<TaskRunOutcome, PlanError>>>>;

pub(crate) async fn drive(mut task: Box<dyn ErasedTask>, ctx: SchedCtx) -> Result<TaskRunOutcome, PlanError> {
    let mut attempt: u32 = 0;
    let proposal = loop {
        if let Some(result) = task.dissolve() {
            return Ok(TaskRunOutcome::Dissolved(result));
        }
        let owner = ProposalId { task: ctx.identity().id, attempt };
        attempt += 1;
        let mut proposal = task.propose(owner).await?;
        // Proposing can await; the work it serves may have been taken by
        // another task meanwhile, and a proposal with no reservations would
        // otherwise schedule on the spot.
        if let Some(result) = task.dissolve() {
            return Ok(TaskRunOutcome::Dissolved(result));
        }
        if proposal.reservations().is_empty() {
            break proposal;
        }
        match ctx.await_accept(owner, proposal).await {
            AcceptOutcome::Accepted(proposal) => break proposal,
            AcceptOutcome::Evicted => continue,
        }
    };
    task.schedule(proposal, ctx).await.map(TaskRunOutcome::Scheduled)
}
