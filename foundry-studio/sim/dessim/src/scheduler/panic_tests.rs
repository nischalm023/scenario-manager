//! Task panics abort the run and release waiting parents instead of leaving
//! detached executor failures disguised as scheduler deadlocks.

use std::cell::Cell;
use std::fmt;
use std::panic::panic_any;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::time::sleep;

use super::Scheduler;
use super::task::proposal::{EmptyProposal, ProposalId};
use super::task::{SchedCtx, Task};
use crate::error::PlanError;
use crate::sim_types::{StopReason, StopSignal};
use crate::task::TaskContext;

struct TestDeadline(Arc<StopSignal>);

impl fmt::Display for TestDeadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestDeadline")
    }
}

#[async_trait(?Send)]
impl Task for TestDeadline {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        sleep(Duration::from_millis(100)).await;
        self.0.stop(StopReason::WallTimeout);
        Ok(())
    }
}

fn bounded_scheduler() -> Scheduler {
    let stop = StopSignal::new();
    let mut scheduler = Scheduler::new().with_stop_signal(stop.clone());
    scheduler.add_service(TestDeadline(stop));
    scheduler
}

struct FailingChild {
    in_propose: bool,
    dropped: Rc<Cell<bool>>,
}

impl fmt::Display for FailingChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FailingChild")
    }
}

impl Drop for FailingChild {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

#[async_trait(?Send)]
impl Task for FailingChild {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        if self.in_propose {
            panic_any(String::from("proposal failed"));
        }
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        panic!("schedule failed");
    }
}

struct WaitingParent {
    child: Option<FailingChild>,
    finished: Rc<Cell<bool>>,
}

impl fmt::Display for WaitingParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WaitingParent")
    }
}

#[async_trait(?Send)]
impl Task for WaitingParent {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.add_task(self.child.take().unwrap()).await;
        self.finished.set(true);
        Ok(())
    }
}

#[test]
fn child_panics_abort_with_original_context_and_release_waiting_parent() {
    for in_propose in [true, false] {
        let mut scheduler = bounded_scheduler();
        let dropped = Rc::new(Cell::new(false));
        let finished = Rc::new(Cell::new(false));
        scheduler.add_task(WaitingParent {
            child: Some(FailingChild { in_propose, dropped: dropped.clone() }),
            finished: finished.clone(),
        });

        let error = scheduler.run().expect_err("a child panic must abort, not wait for the test deadline");
        let PlanError::TaskGraphInvariant { context, detail } = error else { panic!("unexpected error") };
        assert_eq!(context, "task panic");
        assert!(detail.contains("FailingChild"), "{detail}");
        assert!(detail.contains(if in_propose { "proposal failed" } else { "schedule failed" }), "{detail}");
        assert!(dropped.get(), "panicked task retains its resources");
        assert!(!finished.get(), "parent continued past a failed child");
        assert!(scheduler.is_empty(), "pending parent survived fatal teardown");
        assert!(!scheduler.stopped(), "panic was hidden by a wall timeout");
    }
}

struct Setup;

impl fmt::Display for Setup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Setup")
    }
}

#[async_trait(?Send)]
impl Task for Setup {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        Ok(())
    }
}

struct FailingFactory;

impl fmt::Display for FailingFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FailingFactory")
    }
}

#[async_trait(?Send)]
impl Task for FailingFactory {
    type Proposal = EmptyProposal;
    type Schedule = ();

    fn create(_: &TaskContext) -> Result<Self, PlanError> {
        panic!("dependent root factory failed");
    }

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        unreachable!()
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        unreachable!()
    }
}

#[test]
fn deferred_root_factory_panic_is_reported_after_setup() {
    let mut scheduler = bounded_scheduler();
    let setup = scheduler.add_task(Setup);
    scheduler.add_registered_after::<FailingFactory>(json!({}), vec![setup.dependency()]).unwrap();

    let error = scheduler.run().expect_err("a deferred factory panic must abort the run");
    let message = error.to_string();
    assert!(message.contains("FailingFactory"), "{message}");
    assert!(message.contains("dependent root factory failed"), "{message}");
    assert!(scheduler.is_empty());
    assert!(!scheduler.stopped());
}
