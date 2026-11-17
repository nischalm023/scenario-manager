//! The generic task scheduler: it owns the task graph, the async
//! executor loop, and the trailing global barrier, and it knows nothing
//! about warehouses — no model state, no resource families, no demand
//! generators. Domain policy reaches it only through opaque surfaces:
//! tasks and their proposals (whose `start_time`/`poisoned`/
//! `admissible` it reads generically) and injected harness closures (sim
//! horizon, stall census). Quiescence it does not compute at all: the
//! runtime's park hook reports every drained run queue, and an
//! acceptance round rides that report.

pub mod barrier;
mod heartbeat;
pub mod prelude;
pub mod task;

use std::time::{Duration, Instant};

use crate::StopSignal;
use crate::model::SimTime;
use crate::model::bot::plan::PlanError;
use crate::sim_types::StopReason;
use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::future::poll_fn;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Poll;

use tokio::pin;
use tokio::runtime::Builder;
use tokio::sync::Notify;
use tokio::task::{LocalSet, spawn_local, unconstrained};
use tokio::time::sleep;

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::model::ids::TaskId;
use crate::signals::Version;
use crate::state::States;
use crate::task::{Deferred as RegisteredDeferred, TaskContext, TaskDefaults};
use barrier::Barrier;
use heartbeat::Heartbeat;
use serde_json::Value;
use task::proposal::{ProposalId, accept};
use task::{
    Completion, CompletionState, ErasedTask, SchedCtx, Task, TaskCell, TaskDependency, TaskFuture, TaskIdentity,
    TaskRunOutcome, drive,
};

// How long the scheduler loop waits, in wall-clock time, for a graph or a
// drained wake before its watchdog wakes it to check the stop signal and the
// stall alarm. A long stretch of task polls delays that wake further.
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(50);

impl Drop for Scheduler {
    fn drop(&mut self) {
        self.drop_all_pending();
    }
}

pub struct Scheduler {
    graph: Rc<RefCell<TaskGraph>>,
    consecutive_blocked: u32,
    stop_signal: Option<Arc<StopSignal>>,
    sim_time_limit: Option<SimTime>,
    // The domain's planned-frontier probe, injected by the composition —
    // the scheduler has no view of sim time beyond the barrier. Required
    // whenever a sim-time limit is set.
    sim_horizon: Option<Box<dyn Fn() -> SimTime>>,
    // True once a stop (signal or sim-time limit) ended the run: teardown
    // then legitimately drops mid-chain work, which the harness's leak
    // audit reads through `stopped()`.
    stopped: bool,
    // Domain census printed when the stall alarm fires; injected so the
    // scheduler stays domain-blind.
    stall_report: Option<Box<dyn Fn() -> String>>,
    // Told the current world time each time the scheduler advances B, so a
    // reader of the event stream learns it in the stream's own order.
    advance_listener: Option<Box<dyn Fn(SimTime)>>,
    accepts: u64,
    evictions_poisoned: u64,
    evictions_poisoned_by_task_type: FxHashMap<&'static str, u64>,
    // The inadmissible earliest start now holding the clock, if one is,
    // and how many distinct holds the run has seen (see `acceptance_round`).
    holding: Option<SimTime>,
    inadmissible_holds: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            graph: Rc::new(RefCell::new(TaskGraph::new(Rc::new(Barrier::new(true))))),
            consecutive_blocked: 0,
            stop_signal: None,
            sim_time_limit: None,
            sim_horizon: None,
            stopped: false,
            stall_report: None,
            advance_listener: None,
            accepts: 0,
            evictions_poisoned: 0,
            evictions_poisoned_by_task_type: FxHashMap::default(),
            holding: None,
            inadmissible_holds: 0,
        }
    }

    /// Replace the barrier's enablement before any task launches.
    pub fn with_barrier_enabled(self, enabled: bool) -> Self {
        self.graph.borrow_mut().barrier = Rc::new(Barrier::new(enabled));
        self
    }

    pub fn with_stop_signal(mut self, signal: Arc<StopSignal>) -> Self {
        self.stop_signal = Some(signal);
        self
    }

    /// Stop the run once the domain's planned frontier reaches `limit`.
    /// Requires a sim-horizon probe ([`Self::with_sim_horizon`]).
    pub fn with_sim_time_limit(mut self, limit: SimTime) -> Self {
        self.sim_time_limit = Some(limit);
        self
    }

    /// The domain's planned-frontier probe, injected by the composition.
    pub fn with_sim_horizon(mut self, horizon: Box<dyn Fn() -> SimTime>) -> Self {
        self.sim_horizon = Some(horizon);
        self
    }

    pub fn with_stall_report(mut self, report: Box<dyn Fn() -> String>) -> Self {
        self.stall_report = Some(report);
        self
    }

    /// Call `listener` with the current world time each time the scheduler
    /// advances B, before any acceptance at the new instant, so whatever the
    /// accepted work reports follows the call.
    pub fn with_advance_listener(mut self, listener: Box<dyn Fn(SimTime)>) -> Self {
        self.advance_listener = Some(listener);
        self
    }

    pub fn barrier(&self) -> Rc<Barrier> {
        self.graph.borrow().barrier.clone()
    }

    /// Run-lived construction context, also used by specifications spawned
    /// through SchedCtx. Configuration can arrive after consumers launch.
    pub fn states(&self) -> States {
        self.graph.borrow().states.clone()
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn run(&mut self) -> Result<(), PlanError> {
        if self.sim_time_limit.is_some() && self.sim_horizon.is_none() {
            return Err(PlanError::TaskGraphInvariant {
                context: "Scheduler",
                detail: "a sim-time limit needs a sim-horizon probe (with_sim_horizon)".into(),
            });
        }
        let parks = Arc::new(Parks::new());
        // This hook is the run's only quiescence source, and it is sound on the
        // current-thread runtime alone (docs/multithreaded-quiescence.md).
        let runtime = Builder::new_current_thread()
            .enable_time()
            .on_thread_park({
                let parks = parks.clone();
                move || parks.bump()
            })
            .build()
            .map_err(|e| PlanError::TaskGraphInvariant {
                context: "Scheduler",
                detail: format!("tokio runtime build failed: {e}"),
            })?;
        let tasks = LocalSet::new();
        let result = tasks.block_on(&runtime, self.supervise(parks));

        drop(tasks);
        drop(runtime);
        if result.is_err() {
            let dropped = self.drop_all_pending();
            if dropped > 0 {
                eprintln!("[scheduler] aborted after error; dropped {dropped} pending tasks");
            }
        }
        result
    }

    // Event-driven: the loop processes on every task-graph wake (proposal
    // publications and commits both bump the graph version), on every
    // report that the executor drained its run queue, plus a watchdog
    // (`WATCHDOG_INTERVAL`) for the harness instruments (stop signal, the
    // stall alarm). A drain report carries one acceptance round: advance the
    // barrier to the earliest admissible pending start time or awaited
    // target, and accept the proposal at it. `biased` keeps the select's poll
    // order fixed.
    async fn supervise(&mut self, parks: Arc<Parks>) -> Result<(), PlanError> {
        enum Wake {
            Graph,
            Drained,
            Watchdog,
        }

        let run_start = Instant::now();
        let mut cycles: u64 = 0;
        let mut heartbeat = Heartbeat::from_env();

        self.launch_pending();
        let graph_version = self.graph.borrow().version.clone();
        let mut last_seen = graph_version.seen().wrapping_sub(1);
        // Set by an acceptance round that found nothing to do: until the
        // graph moves, further drain reports say nothing new, so the loop
        // stops asking the executor for one.
        let mut exhausted = false;
        let mut event = true;
        loop {
            if let Some(e) = self.graph.borrow_mut().fatal.take() {
                return Err(e);
            }

            let seen = graph_version.seen();
            let progressed = seen != last_seen;
            if progressed {
                last_seen = seen;
                self.consecutive_blocked = 0;
                exhausted = false;
            }

            if event {
                cycles += 1;
                if let Some(hb) = heartbeat.as_mut() {
                    let sim = match &self.sim_horizon {
                        Some(probe) => probe(),
                        None => self.graph.borrow().barrier.floor(),
                    };
                    hb.tick(sim, cycles, self.len());
                }
            } else {
                if !progressed && !self.is_empty() {
                    self.consecutive_blocked += 1;
                }
                self.check_alarms()?;
            }

            // The stop signal is checked on EVERY wake: a person, the harness's
            // wall-clock timeout or an observer failure raises it in wall-clock
            // time, and a busy run's continuous event traffic must never starve
            // it — the watchdog tick is only its floor, not its sole carrier.
            if self.stop_signal.as_deref().map_or(false, |s| s.is_stopped()) {
                self.stopped = true;
                let dropped = self.drop_all_pending();
                eprintln!("[scheduler] stopped by signal after {} cycles; dropped {} pending tasks", cycles, dropped);
                break;
            }
            // The sim-time limit is checked on the first pass, before any task
            // is polled, and after that only on a wake the scheduler's own work
            // causes (a graph or a drained wake), never on a watchdog wake:
            // wall-clock time decides where in a stretch of task polls the
            // watchdog wakes, and where a run stops must be decided by its input
            // alone. A drained wake checks it again before its acceptance round.
            if event
                && let Some(limit) = self.sim_time_limit
                && let Some(planned_horizon) = self.planned_horizon_at_limit()
            {
                self.stopped = true;
                if let Some(signal) = &self.stop_signal {
                    signal.stop(StopReason::SimTimeLimit);
                }
                let dropped = self.drop_all_pending();
                eprintln!(
                    "[scheduler] sim-time limit reached at {:.3}s (limit {:.3}s) after {} cycles; dropped {} pending tasks",
                    planned_horizon.as_ms() as f64 / 1000.0,
                    limit.as_ms() as f64 / 1000.0,
                    cycles,
                    dropped
                );
                break;
            }

            if self.is_empty() {
                break;
            }

            let graph_seen = graph_version.seen();
            let parks_seen = parks.seen();
            let wake = tokio::select! {
                biased;
                _ = graph_version.past(graph_seen) => Wake::Graph,
                _ = parks.past(parks_seen), if !exhausted => Wake::Drained,
                _ = sleep(WATCHDOG_INTERVAL) => Wake::Watchdog,
            };
            event = match wake {
                Wake::Graph => true,
                // The park hook having fired IS the quiescence fact: the
                // executor found nothing runnable and no deferred wake, so
                // this round cannot race work that is still to be polled.
                // Both outcomes stay event passes — the watchdog keeps
                // owning the stall alarm's cadence. This wake can be the first
                // to follow the planned horizon reaching the limit; it then
                // skips its round, so no further acceptance round runs, and
                // the next pass stops the run.
                Wake::Drained => {
                    if self.planned_horizon_at_limit().is_none() && !self.acceptance_round()? {
                        exhausted = true;
                    }
                    true
                }
                Wake::Watchdog => {
                    // Re-arm the drain wait: no wake source today can change
                    // what a round reads without a graph event, but an
                    // exogenous one (a real-runtime harness feed) could, and
                    // the retry is a no-op round per tick at worst.
                    exhausted = false;
                    false
                }
            };
        }
        eprintln!(
            "[scheduler] done: {}ms total, {} cycles, {} accepts, {} poison evictions, {} inadmissible holds{}",
            run_start.elapsed().as_millis(),
            cycles,
            self.accepts,
            self.evictions_poisoned,
            self.inadmissible_holds,
            if self.stopped { " (stopped by signal)" } else { "" },
        );
        let mut by_type: Vec<_> = self.evictions_poisoned_by_task_type.iter().collect();
        by_type.sort_unstable_by_key(|&(task_type, _)| *task_type);
        for (task_type, count) in by_type {
            eprintln!("[scheduler] poison evictions: task_type={task_type} count={count}");
        }
        Ok(())
    }

    // One acceptance round at executor quiescence. Poisoned candidacies
    // are evicted first (their owners re-select under the changed state,
    // so a stale tie can never double-book behind an earlier winner).
    // Otherwise B advances to the earliest live candidacy's decision deadline —
    // or first wakes an awaited target at that same instant — and uses
    // the due proposal only to find a family-ranked contest. Exactly one
    // whole proposal accepts per round, so the next round sees the
    // acceptance's consequences before choosing again. The advance never
    // passes ANY live candidacy's deadline: an unresolved minimum holds.
    // Returns false when nothing moved — a hold or the alarm's case,
    // which the counters below tell apart.
    fn acceptance_round(&mut self) -> Result<bool, PlanError> {
        let evicted = self.evict_where(|cell| cell.pending_poisoned());
        if evicted > 0 {
            self.evictions_poisoned += evicted;
            return Ok(true);
        }

        let barrier = self.graph.borrow().barrier.clone();
        let floor = barrier.floor();
        let (live_min, mut due) = {
            let g = self.graph.borrow();
            let mut live_min = None;
            let mut due = Vec::new();
            for (task_id, cell) in g.pending_cells() {
                let Some(deadline) = cell.pending_resolve_by() else { continue };
                let id = cell.pending_id().expect("a pending deadline belongs to a published attempt");
                match live_min {
                    Some(minimum) if deadline > minimum => {}
                    Some(minimum) if deadline == minimum => due.push((task_id, id)),
                    _ => {
                        live_min = Some(deadline);
                        due.clear();
                        due.push((task_id, id));
                    }
                }
            }
            (live_min, due)
        };
        due.sort_unstable_by_key(|&(task_id, _)| task_id);
        let target = barrier.next_target();

        // Equal-time targets go first: advancing wakes every producer
        // waiting on this instant, and executor quiescence lets them publish
        // before a due proposal asks its family to rank the contest.
        if let Some(t) = target.filter(|t| live_min.map_or(true, |minimum| *t <= minimum)) {
            self.holding = None;
            self.advance_to(&barrier, t)?;
            return Ok(true);
        }

        let Some(minimum) = live_min else {
            self.holding = None;
            return Ok(false);
        };
        if minimum < floor {
            return Err(PlanError::TaskGraphInvariant {
                context: "Scheduler",
                detail: format!("live candidacy deadline {minimum} lies behind the settled frontier {floor}"),
            });
        }

        let mut winner = None;
        for (task_id, trigger) in due {
            let cell = locate_cell(&self.graph, task_id)?;
            let ranked = cell.pending_ranked_competitors();
            let contenders = match ranked {
                Some(contenders) => {
                    if !contenders.contains(&trigger) {
                        return Err(PlanError::TaskGraphInvariant {
                            context: "Scheduler",
                            detail: format!("family ranking for trigger {trigger} omitted its own proposal"),
                        });
                    }
                    contenders
                }
                None => vec![trigger],
            };

            for contender in contenders {
                let Ok(contender_cell) = locate_cell(&self.graph, contender.task) else {
                    continue;
                };
                if contender_cell.pending_id() != Some(contender)
                    || contender_cell.pending_poisoned()
                    || !contender_cell.pending_admissible()
                {
                    continue;
                }
                winner = Some(contender);
                break;
            }
            if winner.is_some() {
                break;
            }
        }

        if let Some(winner) = winner {
            // The due deadline is the must-resolve trigger, not the winning
            // contender's score or start. A ranked future contender may be
            // committed here because accepting it resolves this due contest.
            if minimum > floor {
                self.advance_to(&barrier, minimum)?;
            }
            self.accepts += 1;
            self.holding = None;
            accept_attempt(&self.graph, &barrier, winner)?;
            return Ok(true);
        }

        // Count hold EPISODES, not held rounds — the watchdog re-arms the
        // drain wait every 50ms, so one standing hold is re-examined.
        if self.holding != Some(minimum) {
            self.inadmissible_holds += 1;
        }
        self.holding = Some(minimum);
        Ok(false)
    }

    fn advance_to(&mut self, barrier: &Barrier, t: SimTime) -> Result<(), PlanError> {
        // Both callers bound the advance by the minimum live deadline, so no
        // pending candidacy can be stranded behind it. A violation means
        // that bound was computed wrong; past this point the stranded
        // candidacy hard-errors at acceptance, far from the cause — so
        // halt here, at the advance that broke it, in every build.
        let stranded = self
            .graph
            .borrow()
            .pending_cells()
            .find_map(|(task_id, cell)| cell.pending_resolve_by().filter(|s| *s < t).map(|s| (task_id, s)));
        if let Some((task_id, deadline)) = stranded {
            return Err(PlanError::TaskGraphInvariant {
                context: "Scheduler",
                detail: format!(
                    "advance to {t} would strand task {task_id:?}'s candidacy deadline {deadline} in the settled past"
                ),
            });
        }
        barrier.advance_to(t);
        if let Some(listener) = &self.advance_listener {
            listener(barrier.current_world_time());
        }
        Ok(())
    }

    fn evict_where(&mut self, condition: impl Fn(&TaskCell) -> bool) -> u64 {
        let victims: Vec<_> = {
            let g = self.graph.borrow();
            g.pending_cells()
                .filter(|(_, cell)| condition(cell))
                .map(|(task_id, cell)| (task_id, cell.task_type()))
                .collect()
        };
        let mut evicted = 0;
        for (task_id, task_type) in victims {
            if evict_task(&self.graph, task_id) {
                evicted += 1;
                *self.evictions_poisoned_by_task_type.entry(task_type).or_default() += 1;
            }
        }
        evicted
    }

    // The planned horizon, once it has reached the sim-time limit.
    fn planned_horizon_at_limit(&self) -> Option<SimTime> {
        let limit = self.sim_time_limit?;
        let probe = self.sim_horizon.as_ref().expect("run() refuses a limit without a probe");
        let planned_horizon = probe();
        (planned_horizon >= limit).then_some(planned_horizon)
    }

    fn check_alarms(&mut self) -> Result<(), PlanError> {
        const BLOCKED_LIMIT: u32 = 1000;
        if self.consecutive_blocked >= BLOCKED_LIMIT {
            // `holding` names the start the clock is pinned at when an
            // inadmissible candidacy is what stopped the run, which is the
            // difference between "nothing to do" and "one family wedged".
            eprintln!(
                "[scheduler] DEADLOCK pending={} services={} barrier={} holding={:?} inadmissible_holds={}",
                self.len(),
                self.graph.borrow().service_pending,
                self.graph.borrow().barrier.floor(),
                self.holding,
                self.inadmissible_holds,
            );
            for (tid, cell) in self.graph.borrow().pending_cells() {
                if let Some(start) = cell.pending_start_time() {
                    eprintln!("[scheduler]   candidacy {tid:?} start={start} admissible={}", cell.pending_admissible(),);
                }
            }
            for (tid, label) in self.pending_tasks() {
                eprintln!("[scheduler]   task {:?} {}", tid, label);
            }
            if let Some(report) = &self.stall_report {
                eprintln!("{}", report());
            }
            return Err(PlanError::TaskGraphInvariant {
                context: "Scheduler",
                detail: format!(
                    "blocked for {} consecutive passes with no progress — likely deadlock",
                    self.consecutive_blocked,
                ),
            });
        }
        Ok(())
    }

    fn launch_pending(&mut self) {
        let unlaunched: Vec<(TaskId, TaskGroupId)> = {
            let g = self.graph.borrow();
            g.groups
                .values()
                .flat_map(|group| group.entries.iter().map(move |entry| (entry, group.id)))
                .filter(|(entry, _)| matches!(entry.state, TaskEntryState::Pending) && entry.cell.is_none())
                .map(|(entry, group_id)| (entry.task_id, group_id))
                .collect()
        };
        for (task_id, group_id) in unlaunched {
            launch(&self.graph, task_id, group_id);
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// The executor's own count of the moments it had nothing left to run.
///
/// tokio runs the `on_thread_park` hook immediately before the sole
/// worker parks, and it only reaches that park when no task is runnable,
/// no wake is deferred, and the supervise future is not already woken —
/// so each bump is the executor stating that it drained its run queue,
/// which is exactly the fact an acceptance round needs. Waking the
/// supervisor from inside the hook cannot deadlock: work the hook
/// schedules aborts the park instead of sleeping through it. Atomics
/// because the hook contract is `Fn() + Send + Sync`, not because a
/// second thread ever touches this.
struct Parks {
    count: AtomicU64,
    woke: Notify,
}

impl Parks {
    fn new() -> Self {
        Self { count: AtomicU64::new(0), woke: Notify::new() }
    }

    fn seen(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.count.fetch_add(1, Ordering::Release);
        self.woke.notify_waiters();
    }

    // Level-triggered, like every other wait in the crate. `notify_waiters`
    // reaches only listeners already polled or enabled, so this enables the
    // listener BEFORE re-reading the count: a park landing in that window
    // is caught by the re-read instead of being lost.
    async fn past(&self, seen: u64) {
        loop {
            let woke = self.woke.notified();
            pin!(woke);
            woke.as_mut().enable();
            if self.seen() > seen {
                return;
            }
            woke.await;
        }
    }
}

// The pull surface: accept a specific task's published proposal,
// converting its tentatives in place. The barrier floor is enforced
// here — a claim starting in the settled past is a scheduler bug and
// halts the run.
pub(crate) fn accept_task(graph: &Rc<RefCell<TaskGraph>>, barrier: &Barrier, task_id: TaskId) -> Result<(), PlanError> {
    let cell = locate_cell(graph, task_id)?;
    let id = cell.pending_id().ok_or_else(|| PlanError::TaskGraphInvariant {
        context: "Scheduler",
        detail: format!("accept on task {task_id:?} with no published proposal"),
    })?;
    accept_attempt(graph, barrier, id)
}

fn accept_attempt(graph: &Rc<RefCell<TaskGraph>>, barrier: &Barrier, id: ProposalId) -> Result<(), PlanError> {
    let cell = locate_cell(graph, id.task)?;
    let mut proposal = cell.take_pending_attempt(id).ok_or_else(|| PlanError::TaskGraphInvariant {
        context: "Scheduler",
        detail: format!("accept on stale or unpublished proposal {id}"),
    })?;
    if let Some(start) = proposal.start_time() {
        let floor = barrier.floor();
        if start < floor {
            return Err(PlanError::TaskGraphInvariant {
                context: "Scheduler",
                detail: format!("acceptance for {id} starts at {start}, before the settled frontier {floor}"),
            });
        }
    }
    accept(proposal.as_mut()).map_err(|e| PlanError::TaskGraphInvariant {
        context: "Scheduler",
        detail: format!("acceptance failure is a family-service bug: {e}"),
    })?;
    cell.deliver_accepted(proposal);
    Ok(())
}

pub(crate) fn evict_task(graph: &Rc<RefCell<TaskGraph>>, task_id: TaskId) -> bool {
    let Ok(cell) = locate_cell(graph, task_id) else {
        return false;
    };
    let evicted = cell.take_pending_proposal().is_some();
    if evicted {
        cell.wake();
    }
    evicted
}

fn locate_cell(graph: &Rc<RefCell<TaskGraph>>, task_id: TaskId) -> Result<Rc<TaskCell>, PlanError> {
    let g = graph.borrow();
    let entry = g
        .locator
        .get(&task_id)
        .and_then(|&(group_id, idx)| g.groups.get(&group_id).map(|group| &group.entries[idx]))
        .ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "Scheduler",
            detail: format!("proposal operation on unknown task {task_id:?}"),
        })?;
    entry.cell.as_ref().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
        context: "Scheduler",
        detail: format!("proposal operation on never-launched task {task_id:?}"),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskGroupId(pub u32);

impl fmt::Display for TaskGroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "g{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lineage(SmallVec<[u32; 4]>);

impl Lineage {
    fn registered(id: TaskId) -> Self {
        Lineage(SmallVec::from_slice(&[id.0]))
    }

    fn child(&self, counter: u32) -> Self {
        let mut path = self.0.clone();
        path.push(counter);
        Lineage(path)
    }
}

pub enum TaskEntryState {
    Pending,

    Committed,

    Dissolved,

    Canceled,
}

pub struct TaskEntry {
    pub task_id: TaskId,

    slot: Rc<RefCell<Option<Box<dyn ErasedTask>>>>,
    label: String,
    pub state: TaskEntryState,

    lineage: Lineage,

    next_child: u32,

    cell: Option<Rc<TaskCell>>,

    completion: Rc<CompletionState>,

    // Whether this task HOLDS COMPLETION: the run is done when zero such
    // tasks remain pending. Services (perpetual demand generators) don't.
    blocks_completion: bool,
}

impl TaskEntry {
    fn display_label(&self) -> String {
        match self.slot.borrow().as_ref() {
            Some(task) => format!("{task}"),
            None => self.label.clone(),
        }
    }
}

pub struct TaskGroup {
    pub id: TaskGroupId,
    pub entries: Vec<TaskEntry>,
}

impl fmt::Display for TaskGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "TaskGroup({})", self.id)?;
        for entry in &self.entries {
            write!(f, "  {} {}", entry.task_id, entry.display_label())?;
            match &entry.state {
                TaskEntryState::Pending => writeln!(f)?,
                TaskEntryState::Committed => writeln!(f, " [committed]")?,
                TaskEntryState::Dissolved => writeln!(f, " [dissolved]")?,
                TaskEntryState::Canceled => writeln!(f, " [canceled]")?,
            }
        }
        Ok(())
    }
}

// Shared with every task's SchedCtx so `add_task` registers and
// launches successors mid-schedule. Single-threaded: every borrow is
// scoped, and none is held while task code runs.
pub(crate) struct TaskGraph {
    states: States,
    task_defaults: TaskDefaults,
    groups: FxHashMap<TaskGroupId, TaskGroup>,

    locator: FxHashMap<TaskId, (TaskGroupId, usize)>,
    next_group_id: u32,
    next_task_id: u32,

    // Completion-holding pending tasks; `is_empty` and the natural-quiescence
    // break read this. Services count separately and never gate completion.
    pending: usize,
    service_pending: usize,

    commits: u64,

    fatal: Option<PlanError>,
    version: Version,

    // The trailing global barrier, shared into every SchedCtx at launch.
    pub(crate) barrier: Rc<Barrier>,
}

impl TaskGraph {
    fn new(barrier: Rc<Barrier>) -> Self {
        Self {
            states: States::default(),
            task_defaults: TaskDefaults::default(),
            groups: FxHashMap::default(),
            locator: FxHashMap::default(),
            next_group_id: 0,
            next_task_id: 0,
            pending: 0,
            service_pending: 0,
            commits: 0,
            fatal: None,
            version: Version::new(),
            barrier,
        }
    }

    fn pending_cells(&self) -> impl Iterator<Item = (TaskId, &Rc<TaskCell>)> {
        self.groups
            .values()
            .flat_map(|group| group.entries.iter())
            .filter(|entry| matches!(entry.state, TaskEntryState::Pending))
            .filter_map(|entry| entry.cell.as_ref().map(|cell| (entry.task_id, cell)))
    }

    fn add_root(&mut self, task: Box<dyn ErasedTask>, blocks_completion: bool) -> Rc<CompletionState> {
        let group_id = TaskGroupId(self.next_group_id);
        self.next_group_id += 1;
        let task_id = TaskId(self.next_task_id);
        self.next_task_id += 1;

        self.locator.insert(task_id, (group_id, 0));
        let lineage = Lineage::registered(task_id);
        let label = format!("{task}");
        if blocks_completion {
            self.pending += 1;
        } else {
            self.service_pending += 1;
        }
        let completion = CompletionState::new();
        self.groups.insert(
            group_id,
            TaskGroup {
                id: group_id,
                entries: vec![TaskEntry {
                    task_id,
                    slot: Rc::new(RefCell::new(Some(task))),
                    label,
                    state: TaskEntryState::Pending,
                    lineage,
                    next_child: 0,
                    cell: None,
                    completion: completion.clone(),
                    blocks_completion,
                }],
            },
        );
        completion
    }

    pub(crate) fn spawn(
        graph: &Rc<RefCell<TaskGraph>>,
        parent: TaskIdentity,
        task: Box<dyn ErasedTask>,
    ) -> Rc<CompletionState> {
        let (task_id, completion) = graph.borrow_mut().insert_child(parent, task);
        launch(graph, task_id, parent.group_id);
        completion
    }

    fn insert_child(&mut self, parent: TaskIdentity, task: Box<dyn ErasedTask>) -> (TaskId, Rc<CompletionState>) {
        let task_id = TaskId(self.next_task_id);
        self.next_task_id += 1;

        let lineage = {
            let &(pgroup, pidx) = self.locator.get(&parent.id).expect("spawning parent is registered");
            debug_assert_eq!(pgroup, parent.group_id);
            let pentry = &mut self.groups.get_mut(&pgroup).expect("parent group").entries[pidx];
            let lineage = pentry.lineage.child(pentry.next_child);
            pentry.next_child += 1;
            lineage
        };

        let completion = CompletionState::new();
        let label = format!("{task}");
        let group = self.groups.get_mut(&parent.group_id).expect("parent group");
        self.locator.insert(task_id, (parent.group_id, group.entries.len()));
        self.pending += 1;
        group.entries.push(TaskEntry {
            task_id,
            slot: Rc::new(RefCell::new(Some(task))),
            label,
            state: TaskEntryState::Pending,
            lineage,
            next_child: 0,
            cell: None,
            completion: completion.clone(),
            // Children are real work, whoever spawned them.
            blocks_completion: true,
        });
        (task_id, completion)
    }

    fn complete(&mut self, task_id: TaskId, schedule: Box<dyn Any>) {
        self.finish(task_id, schedule, false);
    }

    fn finish(&mut self, task_id: TaskId, schedule: Box<dyn Any>, dissolved: bool) {
        if let Some(&(group_id, idx)) = self.locator.get(&task_id) {
            if let Some(group) = self.groups.get_mut(&group_id) {
                let entry = &mut group.entries[idx];
                if matches!(entry.state, TaskEntryState::Pending) {
                    entry.state = if dissolved { TaskEntryState::Dissolved } else { TaskEntryState::Committed };
                    entry.cell = None;
                    entry.completion.complete(schedule);
                    if entry.blocks_completion {
                        self.pending -= 1;
                    } else {
                        self.service_pending -= 1;
                    }
                    if !dissolved {
                        self.commits += 1;
                    }
                    self.version.bump();
                }
            }
        }
    }
}

fn launch(graph: &Rc<RefCell<TaskGraph>>, task_id: TaskId, group_id: TaskGroupId) {
    let (slot, cell, barrier) = {
        let g = &mut *graph.borrow_mut();
        let Some(&(gid, idx)) = g.locator.get(&task_id) else {
            return;
        };
        debug_assert_eq!(gid, group_id);
        let entry = &mut g.groups.get_mut(&gid).expect("locator points at a registered group").entries[idx];
        let task_type = entry.slot.borrow().as_ref().expect("launching a task with an empty slot").task_type();
        let cell = TaskCell::new(g.version.clone(), task_type);
        entry.cell = Some(cell.clone());
        (entry.slot.clone(), cell, g.barrier.clone())
    };
    let ctx = SchedCtx::new(TaskIdentity { id: task_id, group_id }, cell.clone(), graph.clone(), barrier);
    let mut lifecycle = native_lifecycle(slot, ctx);
    let graph = graph.clone();
    // Outside tokio's cooperative budget: its forced mid-poll pauses serve
    // I/O servers, not a sim, and they would make a task look idle to
    // wake-counting drain detection (the multithread roadmap). Explicit
    // yields (a windowed pathfinder between windows) are unaffected.
    spawn_local(unconstrained(async move {
        // A detached Tokio task otherwise swallows its panic and leaves its
        // graph entry pending forever. Never resume a panicked lifecycle:
        // publish the failure and let the supervisor tear the whole run down.
        let result = poll_fn(|cx| match catch_unwind(AssertUnwindSafe(|| lifecycle.as_mut().poll(cx))) {
            Ok(result) => result,
            Err(panic) => {
                let message = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic payload");
                Poll::Ready(Err(PlanError::TaskGraphInvariant {
                    context: "task panic",
                    detail: format!("task {task_id} {} panicked: {message}", cell.label()),
                }))
            }
        })
        .await;
        match result {
            Ok(TaskRunOutcome::Scheduled(schedule)) => {
                crate::debug_log!("[scheduler] COMMITTED {:?} {}", task_id, cell.label());
                crate::trace_event!(commit, task_committed, task = task_id, group = group_id, label = cell.label());
                graph.borrow_mut().complete(task_id, schedule);
            }
            Ok(TaskRunOutcome::Dissolved(result)) => {
                crate::debug_log!("[scheduler] DISSOLVED {:?} {}", task_id, cell.label());
                graph.borrow_mut().finish(task_id, result, true);
            }
            Err(e) => {
                let mut g = graph.borrow_mut();
                if g.fatal.is_none() {
                    g.fatal = Some(e);
                    g.version.bump();
                }
            }
        }
    }));
}

impl Scheduler {
    pub fn with_task_defaults(self, defaults: TaskDefaults) -> Result<Self, PlanError> {
        defaults.validate()?;
        self.graph.borrow_mut().task_defaults = defaults;
        Ok(self)
    }

    pub fn add_registered<T: Task + 'static>(&mut self, config: Value) -> Result<Completion<T::Schedule>, PlanError> {
        self.add_registered_after::<T>(config, Vec::new())
    }

    pub fn add_registered_after<T: Task + 'static>(
        &mut self,
        config: Value,
        after: Vec<TaskDependency>,
    ) -> Result<Completion<T::Schedule>, PlanError> {
        let context = {
            let graph = self.graph.borrow();
            TaskContext::new::<T>(graph.states.clone(), &graph.task_defaults, config)?
        };
        let task: Box<dyn ErasedTask> = if after.is_empty() {
            let task = T::create(&context)?;
            context.finish()?;
            Box::new(task)
        } else {
            Box::new(RegisteredDeferred::<T>::new(context, after))
        };
        Ok(Completion::typed(self.graph.borrow_mut().add_root(task, T::holds_completion())))
    }

    pub fn add_task<T: Task + 'static>(&mut self, task: T) -> Completion<T::Schedule> {
        Completion::typed(self.graph.borrow_mut().add_root(Box::new(task), true))
    }

    // A service is a perpetual demand generator: it spawns completion-holding
    // work but never holds completion itself, and yields no schedule. The run
    // ends by natural quiescence of the work, with services still live. No
    // run adds one today; the scheduler's own tests and the task tests'
    // wall-clock deadline do.
    #[cfg(any(test, feature = "test-support"))]
    pub fn add_service<T: Task<Schedule = ()> + 'static>(&mut self, task: T) {
        self.graph.borrow_mut().add_root(Box::new(task), false);
    }

    pub fn drop_all_pending(&mut self) -> usize {
        let g = &mut *self.graph.borrow_mut();
        let mut dropped = 0;
        for group in g.groups.values_mut() {
            for entry in &mut group.entries {
                if matches!(entry.state, TaskEntryState::Pending) {
                    entry.slot.borrow_mut().take();
                    entry.state = TaskEntryState::Canceled;
                    entry.cell = None;
                    entry.completion.cancel();
                    if entry.blocks_completion {
                        g.pending -= 1;
                    } else {
                        g.service_pending -= 1;
                    }
                    dropped += 1;
                }
            }
        }
        dropped
    }

    pub fn group_of(&self, task_id: TaskId) -> Option<TaskGroupId> {
        self.graph.borrow().locator.get(&task_id).map(|(group_id, _)| *group_id)
    }

    fn pending_tasks(&self) -> Vec<(TaskId, String)> {
        let g = self.graph.borrow();
        g.groups
            .values()
            .flat_map(|group| group.entries.iter())
            .filter(|entry| matches!(entry.state, TaskEntryState::Pending))
            .map(|entry| {
                let service = if entry.blocks_completion { "" } else { " [service]" };
                (entry.task_id, format!("{}{}", entry.display_label(), service))
            })
            .collect()
    }

    pub fn complete(&mut self, task_id: TaskId, schedule: Box<dyn Any>) {
        self.graph.borrow_mut().complete(task_id, schedule);
    }

    pub fn task_label(&self, task_id: TaskId) -> Option<String> {
        let g = self.graph.borrow();
        let &(group_id, idx) = g.locator.get(&task_id)?;
        let entry = g.groups.get(&group_id)?.entries.get(idx)?;
        Some(entry.display_label())
    }

    pub fn dump_group(&self, id: TaskGroupId) -> Option<String> {
        self.graph.borrow().groups.get(&id).map(|group| group.to_string())
    }

    pub fn is_empty(&self) -> bool {
        self.graph.borrow().pending == 0
    }

    pub fn len(&self) -> usize {
        self.graph.borrow().pending
    }
}

fn native_lifecycle(slot: Rc<RefCell<Option<Box<dyn ErasedTask>>>>, ctx: SchedCtx) -> TaskFuture {
    Box::pin(async move {
        let task = slot.borrow_mut().take().expect("native lifecycle polled with an empty slot");
        ctx_label(&ctx, &task);
        drive(task, ctx).await
    })
}

fn ctx_label(ctx: &SchedCtx, task: &Box<dyn ErasedTask>) {
    ctx.record_label(format!("{task}"));
}

#[cfg(test)]
mod panic_tests;

#[cfg(test)]
mod competition_tests;

#[cfg(test)]
mod tests {

    use super::*;

    use std::cell::Cell;
    use std::future::pending;
    use std::rc::Rc;

    use tokio::runtime::Runtime;
    use tokio::task::yield_now;
    use tokio::time::{advance, pause};

    use async_trait::async_trait;

    use crate::scheduler::task::proposal::{AcceptError, EmptyProposal, Proposal, ProposalId, ProposedReservation};
    use crate::signals::{Mut, notified};

    #[test]
    fn new_scheduler_is_empty() {
        let ts = Scheduler::new();
        assert!(ts.is_empty());
        assert_eq!(ts.len(), 0);
    }

    #[derive(Default)]
    struct TentativeLedger {
        accepted: Cell<u32>,
        dropped_unaccepted: Cell<u32>,
    }

    struct ProbeTentative {
        ledger: Rc<TentativeLedger>,
        start: SimTime,
        consumed: bool,
    }

    impl fmt::Display for ProbeTentative {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "probe-tentative")
        }
    }

    impl Drop for ProbeTentative {
        fn drop(&mut self) {
            if !self.consumed {
                self.ledger.dropped_unaccepted.set(self.ledger.dropped_unaccepted.get() + 1);
            }
        }
    }

    impl ProposedReservation for ProbeTentative {
        fn start_time(&self) -> Option<SimTime> {
            Some(self.start)
        }

        fn accept(&mut self) -> Result<(), AcceptError> {
            self.consumed = true;
            self.ledger.accepted.set(self.ledger.accepted.get() + 1);
            Ok(())
        }
    }

    struct ProbeProposal {
        tentatives: Vec<ProbeTentative>,
    }

    impl fmt::Display for ProbeProposal {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "ProbeProposal({})", self.tentatives.len())
        }
    }

    impl Proposal for ProbeProposal {
        fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
            self.tentatives.iter_mut().map(|t| t as &mut dyn ProposedReservation).collect()
        }

        fn start_time(&self) -> Option<SimTime> {
            self.tentatives.iter().filter_map(|t| ProposedReservation::start_time(t)).min()
        }
    }

    struct ProposingMock {
        ledger: Rc<TentativeLedger>,
        start: SimTime,
    }

    impl fmt::Display for ProposingMock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "Proposing")
        }
    }

    #[async_trait(?Send)]
    impl Task for ProposingMock {
        type Proposal = ProbeProposal;
        type Schedule = ();

        async fn propose(&self, _owner: ProposalId) -> Result<ProbeProposal, PlanError> {
            Ok(ProbeProposal {
                tentatives: vec![ProbeTentative { ledger: self.ledger.clone(), start: self.start, consumed: false }],
            })
        }

        async fn schedule(&mut self, _proposal: ProbeProposal, _ctx: SchedCtx) -> Result<(), PlanError> {
            Ok(())
        }
    }

    fn suspended_proposing_task(ts: &mut Scheduler, ledger: &Rc<TentativeLedger>) -> TaskId {
        ts.add_task(ProposingMock { ledger: ledger.clone(), start: SimTime::ms(500) });
        let g = ts.graph.borrow();
        g.locator.keys().copied().next().expect("the proposing root is registered")
    }

    fn settle(ts: &mut Scheduler, local: &LocalSet, rt: &Runtime) {
        let version = ts.graph.borrow().version.clone();
        let seen = version.seen();
        local.block_on(rt, async {
            ts.launch_pending();
            version.past(seen).await;
        });
    }

    fn test_runtime() -> (Runtime, LocalSet) {
        (Builder::new_current_thread().build().expect("test runtime"), LocalSet::new())
    }

    // The same runtime with the park hook `Scheduler::run` installs, for
    // tests that consume the drain reports rather than the task graph.
    fn parking_runtime() -> (Runtime, LocalSet, Arc<Parks>) {
        let parks = Arc::new(Parks::new());
        let runtime = Builder::new_current_thread()
            .on_thread_park({
                let parks = parks.clone();
                move || parks.bump()
            })
            .build()
            .expect("test runtime");
        (runtime, LocalSet::new(), parks)
    }

    #[test]
    fn reservation_proposal_suspends_then_accepts_then_schedules() {
        let (rt, local) = test_runtime();
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        let id = suspended_proposing_task(&mut ts, &ledger);

        settle(&mut ts, &local, &rt);
        assert_eq!(ts.len(), 1, "a suspended task stays pending");
        assert_eq!(ledger.accepted.get(), 0, "nothing accepted before the scheduler acts");

        accept_task(&ts.graph, &ts.barrier(), id).expect("accepts the published proposal");
        assert_eq!(ledger.accepted.get(), 1);

        settle(&mut ts, &local, &rt);
        assert!(ts.is_empty(), "acceptance reached schedule and the task committed");
        let g = ts.graph.borrow();
        let group = g.groups.values().next().unwrap();
        assert!(matches!(group.entries[0].state, TaskEntryState::Committed));
    }

    #[test]
    fn runtime_drop_releases_the_unaccepted_proposal() {
        let (rt, local) = test_runtime();
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        suspended_proposing_task(&mut ts, &ledger);

        settle(&mut ts, &local, &rt);
        let dropped = ts.drop_all_pending();
        assert_eq!(dropped, 1);
        assert_eq!(ledger.accepted.get(), 0, "cancellation accepts nothing");

        drop(local);
        drop(rt);
        assert_eq!(ledger.dropped_unaccepted.get(), 1, "the tentative was released by the runtime drop");
    }

    #[test]
    fn evicted_proposal_re_proposes_fresh() {
        let (rt, local) = test_runtime();
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        let id = suspended_proposing_task(&mut ts, &ledger);

        settle(&mut ts, &local, &rt);
        assert!(evict_task(&ts.graph, id), "a published proposal was evicted");
        settle(&mut ts, &local, &rt);
        assert_eq!(ledger.dropped_unaccepted.get(), 1, "the evicted tentative dropped unaccepted");

        accept_task(&ts.graph, &ts.barrier(), id).expect("the fresh attempt accepts");
        settle(&mut ts, &local, &rt);
        assert!(ts.is_empty(), "the fresh attempt scheduled");
        assert_eq!(ledger.accepted.get(), 1, "exactly the fresh tentative accepted");
    }

    struct PoisonOnceMock {
        label: &'static str,
        proposed: Cell<bool>,
    }

    impl fmt::Display for PoisonOnceMock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.label)
        }
    }

    struct PoisonOnceProposal(bool);

    impl fmt::Display for PoisonOnceProposal {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("poison-once-proposal")
        }
    }

    impl ProposedReservation for PoisonOnceProposal {
        fn start_time(&self) -> Option<SimTime> {
            Some(SimTime::ZERO)
        }

        fn poisoned(&self) -> bool {
            self.0
        }

        fn accept(&mut self) -> Result<(), AcceptError> {
            Ok(())
        }
    }

    #[async_trait(?Send)]
    impl Task for PoisonOnceMock {
        type Proposal = PoisonOnceProposal;
        type Schedule = ();

        async fn propose(&self, _owner: ProposalId) -> Result<Self::Proposal, PlanError> {
            Ok(PoisonOnceProposal(!self.proposed.replace(true)))
        }

        async fn schedule(&mut self, _proposal: Self::Proposal, _ctx: SchedCtx) -> Result<(), PlanError> {
            Ok(())
        }
    }

    #[test]
    fn poison_evictions_count_retired_attempts_by_static_task_type() {
        let mut ts = Scheduler::new();
        for label in ["first instance", "second instance"] {
            ts.add_task(PoisonOnceMock { label, proposed: Cell::new(false) });
        }
        let ledger = Rc::new(TentativeLedger::default());
        ts.add_task(ProposingMock { ledger: ledger.clone(), start: SimTime::ms(500) });

        ts.run().expect("the two poisoned attempts retire and all three tasks schedule");

        assert_eq!(ts.evictions_poisoned, 2);
        assert_eq!(ts.evictions_poisoned_by_task_type.len(), 1, "unpoisoned types have no entry");
        assert_eq!(ts.evictions_poisoned_by_task_type.get("PoisonOnceMock"), Some(&2));
        assert_eq!(ts.accepts, 3, "the fresh proposals retain ordinary acceptance behavior");
        assert_eq!(ledger.accepted.get(), 1);
    }

    #[test]
    fn accept_without_published_proposal_errors() {
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        let id = suspended_proposing_task(&mut ts, &ledger);

        let err = accept_task(&ts.graph, &ts.barrier(), id).expect_err("no proposal published");
        assert!(matches!(err, PlanError::TaskGraphInvariant { context: "Scheduler", .. }));
        ts.drop_all_pending();
    }

    #[test]
    fn acceptance_behind_the_settled_frontier_is_refused() {
        let (rt, local) = test_runtime();
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        let id = suspended_proposing_task(&mut ts, &ledger);

        settle(&mut ts, &local, &rt);
        ts.barrier().advance_to(SimTime::ms(1_000));
        let err = accept_task(&ts.graph, &ts.barrier(), id).expect_err("a 500ms start sits behind the frontier");
        assert!(matches!(err, PlanError::TaskGraphInvariant { context: "Scheduler", .. }));
        ts.drop_all_pending();
    }

    #[test]
    fn run_accepts_the_published_proposal_at_its_start_instant() {
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        ts.add_task(ProposingMock { ledger: ledger.clone(), start: SimTime::ms(500) });

        ts.run().expect("the acceptance round accepts and the task commits");
        assert_eq!(ledger.accepted.get(), 1, "the barrier accepted the candidacy at its start");
        assert_eq!(ts.barrier().floor(), SimTime::ms(500), "B advanced to the accepted start");
    }

    struct GatedMock {
        instant: SimTime,
    }

    impl fmt::Display for GatedMock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "GatedMock")
        }
    }

    #[async_trait(?Send)]
    impl Task for GatedMock {
        type Proposal = EmptyProposal;
        type Schedule = ();

        async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
            Ok(EmptyProposal)
        }

        async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
            ctx.barrier().wait_until(self.instant).await
        }
    }

    struct SuspendedService {
        version: Version,
    }

    impl fmt::Display for SuspendedService {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "SuspendedService")
        }
    }

    #[async_trait(?Send)]
    impl Task for SuspendedService {
        type Proposal = EmptyProposal;
        type Schedule = ();

        async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
            Ok(EmptyProposal)
        }

        async fn schedule(&mut self, _proposal: EmptyProposal, _ctx: SchedCtx) -> Result<(), PlanError> {
            let seen = self.version.seen();
            self.version.past(seen).await;
            Ok(())
        }
    }

    #[test]
    fn run_advances_the_barrier_to_a_gated_task_at_quiescence() {
        let mut ts = Scheduler::new();
        ts.add_task(GatedMock { instant: SimTime::ms(7_200_000) });
        ts.run().expect("quiescence advances the barrier and the gate opens");
        assert_eq!(ts.barrier().floor(), SimTime::ms(7_200_000));
    }

    /// The listener hears every instant B advances to, in order, and hears
    /// each one before the acceptance at that instant: whatever the accepted
    /// work reports follows the call.
    #[test]
    fn the_advance_listener_hears_each_advance_before_the_acceptance_at_it() {
        let ledger = Rc::new(TentativeLedger::default());
        let heard = Rc::new(RefCell::new(Vec::new()));
        let mut ts = Scheduler::new().with_advance_listener(Box::new({
            let (ledger, heard) = (ledger.clone(), heard.clone());
            move |at| heard.borrow_mut().push((at, ledger.accepted.get()))
        }));
        ts.add_task(ProposingMock { ledger: ledger.clone(), start: SimTime::ms(500) });
        ts.add_task(GatedMock { instant: SimTime::ms(7_200_000) });
        ts.run().expect("the acceptance and the gate both pass");
        assert_eq!(*heard.borrow(), [(SimTime::ms(500), 0), (SimTime::ms(7_200_000), 1)]);
    }

    #[test]
    fn a_suspended_service_does_not_hold_the_barrier_back() {
        let mut ts = Scheduler::new();
        ts.add_service(SuspendedService { version: Version::new() });
        ts.add_task(GatedMock { instant: SimTime::ms(100) });
        ts.run().expect("services hold no completion and no advance");
        assert_eq!(ts.barrier().floor(), SimTime::ms(100));
    }

    // A candidacy its family cannot take yet: `admissible` is false on the
    // first read and true from the second on. In the real system that flip
    // comes from in-flight work freeing the family (zone inbound
    // headroom); standing alone here, it spares the test a second family.
    struct HeldTentative {
        ledger: Rc<TentativeLedger>,
        reads: Rc<Cell<u32>>,
        start: SimTime,
        consumed: bool,
    }

    impl fmt::Display for HeldTentative {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "held-tentative")
        }
    }

    impl Drop for HeldTentative {
        fn drop(&mut self) {
            if !self.consumed {
                self.ledger.dropped_unaccepted.set(self.ledger.dropped_unaccepted.get() + 1);
            }
        }
    }

    impl ProposedReservation for HeldTentative {
        fn start_time(&self) -> Option<SimTime> {
            Some(self.start)
        }

        fn admissible(&self) -> bool {
            let reads = self.reads.get();
            self.reads.set(reads + 1);
            reads > 0
        }

        fn accept(&mut self) -> Result<(), AcceptError> {
            self.consumed = true;
            self.ledger.accepted.set(self.ledger.accepted.get() + 1);
            Ok(())
        }
    }

    struct HeldMock {
        ledger: Rc<TentativeLedger>,
        reads: Rc<Cell<u32>>,
        start: SimTime,
    }

    impl fmt::Display for HeldMock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "HeldMock")
        }
    }

    #[async_trait(?Send)]
    impl Task for HeldMock {
        type Proposal = HeldTentative;
        type Schedule = ();

        async fn propose(&self, _owner: ProposalId) -> Result<HeldTentative, PlanError> {
            Ok(HeldTentative {
                ledger: self.ledger.clone(),
                reads: self.reads.clone(),
                start: self.start,
                consumed: false,
            })
        }

        async fn schedule(&mut self, _proposal: HeldTentative, _ctx: SchedCtx) -> Result<(), PlanError> {
            Ok(())
        }
    }

    // An inadmissible earliest start HOLDS the clock: the round neither
    // accepts it, nor evicts it, nor advances past it to the later awaited
    // target — stepping over it to that target is what deadlocked the run
    // at sim 58s. The Ok is the load-bearing half of the assertions: after
    // a step-over to 200ms, accepting the 100ms start would sit behind the
    // settled frontier and `accept_task` would halt the run instead.
    #[test]
    fn an_inadmissible_earliest_start_holds_the_clock() {
        let mut ts = Scheduler::new();
        let ledger = Rc::new(TentativeLedger::default());
        let reads = Rc::new(Cell::new(0u32));
        ts.add_task(HeldMock { ledger: ledger.clone(), reads: reads.clone(), start: SimTime::ms(100) });
        ts.add_task(GatedMock { instant: SimTime::ms(200) });

        ts.run().expect("the hold broke, both tasks committed");
        assert_eq!(ts.inadmissible_holds, 1, "one hold episode, however many rounds re-read it");
        assert_eq!(ledger.accepted.get(), 1, "the held candidacy accepted once, at its own start");
        assert_eq!(ledger.dropped_unaccepted.get(), 0, "a hold is not an eviction");
        assert_eq!(ts.barrier().floor(), SimTime::ms(200), "B reached the gate only after the 100ms acceptance");
    }

    // Once accepted, it carries the planned horizon to the sim-time limit and
    // past it in two steps with a yield between, as a bot's planning spans
    // executor batches, then awaits something the run never provides, so no
    // graph wake follows. At the yield, tokio's paused clock, standing in for
    // wall-clock time, moves by `watchdog_clock_advance`: past the watchdog's
    // interval, it plays a machine slow enough for the watchdog to wake
    // between the two steps.
    struct PlansPastTheLimit {
        ledger: Rc<TentativeLedger>,
        horizon: Rc<Cell<SimTime>>,
        start: SimTime,
        steps: [SimTime; 2],
        watchdog_clock_advance: Duration,
    }

    impl fmt::Display for PlansPastTheLimit {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "PlansPastTheLimit")
        }
    }

    #[async_trait(?Send)]
    impl Task for PlansPastTheLimit {
        type Proposal = ProbeProposal;
        type Schedule = ();

        async fn propose(&self, _owner: ProposalId) -> Result<ProbeProposal, PlanError> {
            // Pausing tokio's clock needs a running current-thread runtime, so
            // it happens here, at the first poll inside the one `run` builds.
            // A paused clock also jumps to the next timer when the runtime
            // would park with nothing to run, but here it never parks: each
            // time it would, the park hook's drain report wakes the scheduler
            // loop, whose drain wait stays armed until the stop. So the
            // watchdog wakes only if `schedule` moves the clock.
            pause();
            Ok(ProbeProposal {
                tentatives: vec![ProbeTentative { ledger: self.ledger.clone(), start: self.start, consumed: false }],
            })
        }

        async fn schedule(&mut self, _proposal: ProbeProposal, _ctx: SchedCtx) -> Result<(), PlanError> {
            self.horizon.set(self.steps[0]);
            advance(self.watchdog_clock_advance).await;
            self.horizon.set(self.steps[1]);
            pending().await
        }
    }

    // The accepted count, the barrier and how far the planning got, for a run
    // whose sim-time limit the first acceptance's planning reaches while a
    // second proposal is due; then how many watchdog wakes the stall alarm
    // counted since the task graph last moved.
    fn run_to_the_limit(watchdog_clock_advance: Duration) -> ((u32, SimTime, SimTime), u32) {
        let ledger = Rc::new(TentativeLedger::default());
        let horizon = Rc::new(Cell::new(SimTime::ZERO));
        let mut ts = Scheduler::new().with_sim_time_limit(SimTime::ms(1_000)).with_sim_horizon(Box::new({
            let horizon = horizon.clone();
            move || horizon.get()
        }));
        ts.add_task(PlansPastTheLimit {
            ledger: ledger.clone(),
            horizon: horizon.clone(),
            start: SimTime::ms(100),
            steps: [SimTime::ms(1_000), SimTime::ms(1_100)],
            watchdog_clock_advance,
        });
        ts.add_task(ProposingMock { ledger: ledger.clone(), start: SimTime::ms(200) });
        ts.run().expect("the run stops on its sim-time limit");
        assert!(ts.stopped(), "the limit, not completion, ended the run");
        ((ledger.accepted.get(), ts.barrier().floor(), horizon.get()), ts.consecutive_blocked)
    }

    // Where a run stops on its sim-time limit is decided by the scheduler's
    // work alone: a watchdog wake between the planning steps that reach the
    // limit neither cuts that planning short nor changes what is accepted,
    // and the proposal due at simtime 200 ms is not accepted once the planned
    // horizon has reached the limit. The clock moves by twice the watchdog's
    // interval because tokio rounds a timer's deadline up to its next
    // millisecond.
    #[test]
    fn the_watchdog_waking_does_not_move_where_the_sim_time_limit_stops_the_run() {
        let stop = (1, SimTime::ms(100), SimTime::ms(1_100));
        assert_eq!(run_to_the_limit(Duration::ZERO), (stop, 0), "no watchdog wake in the stretch");
        assert_eq!(run_to_the_limit(WATCHDOG_INTERVAL * 2), (stop, 1), "one watchdog wake in the stretch");
    }

    // tokio's LocalSet polls at most 61 tasks per scheduling round, so a
    // drain report has to cover every woken task, however many rounds the
    // drain takes — it must never precede the last woken task's poll. The
    // park hook cannot fire while a task is runnable or a wake is
    // deferred, so the report waits out as many rounds as it takes.
    // Test harness drives executor rotations deliberately.
    #[allow(clippy::disallowed_methods)]
    #[test]
    fn quiescence_wait_outlasts_the_executors_poll_budget() {
        let (rt, local, parks) = parking_runtime();
        local.block_on(&rt, async {
            let signal = Mut::new(0u32);
            let started = Rc::new(Cell::new(0usize));
            let ran = Rc::new(Cell::new(0usize));

            for _ in 0..100 {
                let signal = signal.clone();
                let started = started.clone();
                let ran = ran.clone();
                spawn_local(async move {
                    started.set(started.get() + 1);
                    loop {
                        let changed = notified(&signal);
                        if signal.value() == 1 {
                            break;
                        }
                        changed.await;
                    }
                    ran.set(ran.get() + 1);
                });
            }

            for _ in 0..20 {
                if started.get() == 100 {
                    break;
                }
                yield_now().await;
            }
            assert_eq!(started.get(), 100, "every waiter suspended on the signal before it moves");

            // Delivers all 100 notifications synchronously, so every waiter
            // is runnable before the wait below snapshots the park count.
            signal.set(1);

            let seen = parks.seen();
            parks.past(seen).await;
            assert_eq!(ran.get(), 100, "the drain report preceded the poll of {} woken tasks", 100 - ran.get());
        });
    }
}
