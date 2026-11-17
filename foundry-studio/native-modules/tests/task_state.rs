//! Manager construction is synchronous and lazy. Real scheduled parents can
//! introduce a manager after other tasks have already changed shared state.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dessim::prelude::*;
use dessim::state::{StateError, StateManager, States};

struct Counter(AtomicUsize);

impl StateManager for Counter {
    fn create(states: &States) -> Result<Self, StateError> {
        let initial = states.configuration::<usize>()?;
        Ok(Self(AtomicUsize::new(*initial)))
    }
}

struct Later {
    counter: Arc<Counter>,
    observed_at_construction: usize,
}

impl StateManager for Later {
    fn create(states: &States) -> Result<Self, StateError> {
        let counter = states.get::<Counter>()?;
        let observed_at_construction = counter.0.load(Ordering::Relaxed);
        Ok(Self { counter, observed_at_construction })
    }
}

#[derive(Display)]
#[display("Count")]
struct Count(Arc<Counter>);

#[async_trait(?Send)]
impl Task for Count {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        self.0.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Display)]
#[display("SpawnLater")]
struct SpawnLater {
    states: States,
    seen: Rc<RefCell<Option<Arc<Later>>>>,
}

#[async_trait(?Send)]
impl Task for SpawnLater {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let counter = self.states.get::<Counter>()?;
        ctx.add_task(Count(counter)).await;
        let later = self.states.get::<Later>()?;
        ctx.add_task(Count(later.counter.clone())).await;
        *self.seen.borrow_mut() = Some(later);
        Ok(())
    }
}

#[test]
fn late_tasks_construct_managers_from_current_shared_state() {
    let mut scheduler = Scheduler::new();
    let states = scheduler.states();
    states.configure(10usize).unwrap();
    assert!(states.existing::<Counter>().is_none());
    assert!(states.existing::<Later>().is_none());
    let seen = Rc::new(RefCell::new(None));
    scheduler.add_task(SpawnLater { states: states.clone(), seen: seen.clone() });
    scheduler.run().unwrap();
    let seen = seen.borrow();
    let later = seen.as_ref().unwrap();
    assert_eq!(later.observed_at_construction, 11);
    assert_eq!(later.counter.0.load(Ordering::Relaxed), 12);
    assert!(Arc::ptr_eq(&later.counter, &states.get::<Counter>().unwrap()));
    assert!(Arc::ptr_eq(later, &states.get::<Later>().unwrap()));
}

#[test]
fn construction_inputs_cannot_be_replaced() {
    let states = States::default();
    states.configure(10usize).unwrap();
    let counter = states.get::<Counter>().unwrap();
    assert!(matches!(states.configure(99usize), Err(StateError::AlreadyConfigured { .. })));
    assert_eq!(counter.0.load(Ordering::Relaxed), 10);
    assert!(Arc::ptr_eq(&counter, &states.get::<Counter>().unwrap()));
}

struct A;
struct B;

impl StateManager for A {
    fn create(states: &States) -> Result<Self, StateError> {
        states.get::<B>()?;
        Ok(Self)
    }
}

impl StateManager for B {
    fn create(states: &States) -> Result<Self, StateError> {
        states.get::<A>()?;
        Ok(Self)
    }
}

#[test]
fn dependency_cycles_fail_without_poisoning_other_construction() {
    let states = States::default();
    let error = states.get::<A>().err().unwrap();
    assert!(matches!(error, StateError::Cycle(_)));
    let message = error.to_string();
    assert!(message.contains("A ->") && message.contains("B ->"), "{message}");
    states.configure(10usize).unwrap();
    assert_eq!(states.get::<Counter>().unwrap().0.load(Ordering::Relaxed), 10);
    assert!(matches!(states.get::<B>(), Err(StateError::Cycle(_))));
}

#[test]
fn missing_input_fails_immediately_and_can_be_supplied_before_retry() {
    let states = States::default();
    assert!(matches!(states.get::<Counter>(), Err(StateError::MissingInput(_))));
    states.configure(3usize).unwrap();
    assert_eq!(states.get::<Counter>().unwrap().0.load(Ordering::Relaxed), 3);
}

#[test]
fn separate_runs_do_not_share_managers() {
    let first = States::default();
    let second = States::default();
    first.configure(3usize).unwrap();
    second.configure(8usize).unwrap();
    let a = first.get::<Counter>().unwrap();
    let b = second.get::<Counter>().unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(a.0.load(Ordering::Relaxed), 3);
    assert_eq!(b.0.load(Ordering::Relaxed), 8);
}

#[test]
fn reporting_inspection_does_not_construct_and_signals_late_availability() {
    let states = States::default();
    let initialized = states.initialization_version();
    let before = initialized.seen();
    assert!(states.existing::<Counter>().is_none());
    states.configure(3usize).unwrap();
    assert_eq!(initialized.seen(), before);
    let counter = states.get::<Counter>().unwrap();
    assert_ne!(initialized.seen(), before);
    assert!(Arc::ptr_eq(&counter, &states.existing::<Counter>().unwrap()));
    let ready = initialized.seen();
    states.get::<Counter>().unwrap();
    assert_eq!(initialized.seen(), ready);
}
