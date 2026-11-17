//! Registered roots use synchronous Task factories. Only explicit setup
//! dependencies defer construction until the scheduler completes prerequisites.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::runtime::Builder;

use dessim::prelude::*;
use dessim::state::{StateError, StateManager, States};
use dessim::task::{TaskContext, TaskDefaults, invalid, registered};
use serde_json::{Value, json};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

struct Counter(AtomicUsize);

impl StateManager for Counter {
    fn create(_: &States) -> Result<Self, StateError> {
        CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Self(AtomicUsize::new(0)))
    }
}

#[dessim::task]
#[derive(Display)]
#[display("Increment")]
struct Increment {
    counter: Arc<Counter>,
    amount: usize,
}

#[async_trait(?Send)]
impl Task for Increment {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        Ok(Self { counter: ctx.state()?, amount: ctx.parameter("amount", 1)? })
    }

    type Proposal = EmptyProposal;
    type Schedule = (usize, Arc<Counter>);

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        let fields = config.as_object().ok_or_else(|| invalid("expected object"))?;
        if fields.keys().any(|name| name != "amount")
            || fields.get("amount").is_some_and(|value| value.as_u64().is_none())
        {
            return Err(invalid("amount must be a nonnegative integer"));
        }
        Ok(())
    }

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let total = self.counter.0.fetch_add(self.amount, Ordering::SeqCst) + self.amount;
        Ok((total, self.counter.clone()))
    }
}

#[test]
fn descriptors_are_static_and_root_factories_are_synchronous() {
    assert_eq!(registered("Increment").unwrap().name, "Increment");
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0, "registration must not create managers");
    let defaults: TaskDefaults = serde_json::from_value(json!({"Increment": {"amount": 7}})).unwrap();
    let mut scheduler = Scheduler::new().with_task_defaults(defaults).unwrap();
    let first = scheduler.add_registered::<Increment>(json!({})).unwrap();
    let second = scheduler.add_registered::<Increment>(json!({"amount": 2})).unwrap();
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1, "root factories run synchronously and share managers");
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    let first = runtime.block_on(first);
    let second = runtime.block_on(second);
    assert_eq!(first.0, 7);
    assert_eq!(second.0, 9);
    assert!(Arc::ptr_eq(&first.1, &second.1));
}

#[test]
fn invalid_names_and_config_are_rejected_before_construction() {
    assert!(registered("Typo").is_err());
    let mut scheduler = Scheduler::new();
    assert!(scheduler.add_registered::<Increment>(json!({"amount": "seven"})).is_err());
    assert!(scheduler.add_registered::<Increment>(json!({"typo": 1})).is_err());
    let defaults: TaskDefaults = serde_json::from_value(json!({"UnknownChild": {}})).unwrap();
    assert!(scheduler.with_task_defaults(defaults).is_ok(), "unregistered children can have defaults");
    let defaults: TaskDefaults = serde_json::from_value(json!({"UnknownChild": []})).unwrap();
    assert!(Scheduler::new().with_task_defaults(defaults).is_err());
}

struct SetupProbe {
    ready: AtomicBool,
    consumers: AtomicUsize,
}

impl StateManager for SetupProbe {
    fn create(_: &States) -> Result<Self, StateError> {
        Ok(Self { ready: AtomicBool::new(false), consumers: AtomicUsize::new(0) })
    }
}

#[dessim::task]
#[derive(Display)]
#[display("Prepare")]
struct Prepare {
    probe: Arc<SetupProbe>,
}

#[async_trait(?Send)]
impl Task for Prepare {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        Ok(Self { probe: ctx.state()? })
    }

    type Proposal = EmptyProposal;
    type Schedule = Arc<SetupProbe>;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        tokio::task::yield_now().await;
        self.probe.ready.store(true, Ordering::SeqCst);
        Ok(self.probe.clone())
    }
}

#[dessim::task]
#[derive(Display)]
#[display("ConsumePrepared")]
struct ConsumePrepared {
    probe: Arc<SetupProbe>,
}

#[async_trait(?Send)]
impl Task for ConsumePrepared {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let probe: Arc<SetupProbe> = ctx.state()?;
        if !probe.ready.load(Ordering::SeqCst) {
            return Err(invalid("consumer constructed before setup completed"));
        }
        Ok(Self { probe })
    }

    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        self.probe.consumers.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn multiple_roots_wait_for_setup_without_consuming_its_typed_result() {
    let mut scheduler = Scheduler::new();
    let setup = scheduler.add_registered::<Prepare>(json!({})).unwrap();
    for _ in 0..2 {
        scheduler.add_registered_after::<ConsumePrepared>(json!({}), vec![setup.dependency()]).unwrap();
    }
    scheduler.run().unwrap();
    let probe = Builder::new_current_thread().build().unwrap().block_on(setup);
    assert_eq!(probe.consumers.load(Ordering::SeqCst), 2);
}

#[derive(Display)]
#[display("OrdinaryChild")]
struct OrdinaryChild;

#[async_trait(?Send)]
impl Task for OrdinaryChild {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        Ok(())
    }
}

#[test]
fn ordinary_child_has_no_implicit_root_factory() {
    let mut scheduler = Scheduler::new();
    let error = scheduler.add_registered::<OrdinaryChild>(json!({})).err().unwrap();
    assert!(error.to_string().contains("OrdinaryChild has no composition root factory"));
    assert!(scheduler.is_empty());
}
