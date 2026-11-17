//! Runtime handoffs remain typed and owned while late task construction reads
//! the composition's defaults and resolves the run's shared managers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dessim::composition::{Composition, Definition, RootTask};
use dessim::prelude::*;
use dessim::state::{StateError, StateManager, States};
use dessim::task::{TaskContext, TaskDefaults, invalid, registered};
use serde_json::{Value, json};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

struct TransferCount(AtomicUsize);

impl StateManager for TransferCount {
    fn create(_: &States) -> Result<Self, StateError> {
        CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Self(AtomicUsize::new(0)))
    }
}

// Deliberately neither Clone nor Deserialize: this represents an owned handoff.
struct ToteHandoff {
    tote: Box<str>,
    reservation: Rc<()>,
}

#[derive(Display)]
#[display("ApplyTestHandoff")]
struct ApplyTestHandoff {
    handoff: Option<ToteHandoff>,
    count: Arc<TransferCount>,
    amount: usize,
}

impl ApplyTestHandoff {
    fn new(ctx: &TaskContext, handoff: ToteHandoff) -> Result<Self, PlanError> {
        Ok(Self { handoff: Some(handoff), count: ctx.state()?, amount: ctx.parameter("amount", 1)? })
    }
}

#[async_trait(?Send)]
impl Task for ApplyTestHandoff {
    type Proposal = EmptyProposal;
    type Schedule = (ToteHandoff, usize, Arc<TransferCount>);

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
        let total = self.count.0.fetch_add(self.amount, Ordering::SeqCst) + self.amount;
        Ok((self.handoff.take().unwrap(), total, self.count.clone()))
    }
}

type TransferResult = <ApplyTestHandoff as Task>::Schedule;

#[derive(Display)]
#[display("SequenceTransfers")]
struct SequenceTransfers {
    handoff: Option<ToteHandoff>,
    seen: Rc<RefCell<Vec<TransferResult>>>,
}

#[async_trait(?Send)]
impl Task for SequenceTransfers {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let context = ctx.task_context::<ApplyTestHandoff>(json!({}))?;
        let task = ApplyTestHandoff::new(&context, self.handoff.take().unwrap())?;
        context.finish()?;
        let first = ctx.add_task(task).await;
        let second_input = ToteHandoff { tote: "second".into(), reservation: first.0.reservation.clone() };
        let context = ctx.task_context::<ApplyTestHandoff>(json!({"amount": 2}))?;
        let task = ApplyTestHandoff::new(&context, second_input)?;
        context.finish()?;
        let second = ctx.add_task(task).await;
        self.seen.borrow_mut().extend([first, second]);
        Ok(())
    }
}

#[test]
fn late_children_preserve_owned_inputs_share_managers_and_resolve_defaults() {
    let defaults: TaskDefaults = serde_json::from_value(json!({"ApplyTestHandoff": {"amount": 7}})).unwrap();
    let mut scheduler = Scheduler::new().with_task_defaults(defaults).unwrap();
    let reservation = Rc::new(());
    let seen = Rc::new(RefCell::new(Vec::new()));
    scheduler.add_task(SequenceTransfers {
        handoff: Some(ToteHandoff { tote: "first".into(), reservation: reservation.clone() }),
        seen: seen.clone(),
    });
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    scheduler.run().unwrap();
    let seen = seen.borrow();
    assert_eq!(seen.iter().map(|entry| entry.0.tote.as_ref()).collect::<Vec<_>>(), ["first", "second"]);
    assert_eq!(seen.iter().map(|entry| entry.1).collect::<Vec<_>>(), [7, 9]);
    assert!(seen.iter().all(|entry| Rc::ptr_eq(&entry.0.reservation, &reservation)));
    assert!(Arc::ptr_eq(&seen[0].2, &seen[1].2));
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);
}

#[test]
fn ordinary_children_cannot_be_selected_by_json() {
    assert!(registered("ApplyTestHandoff").is_err(), "child requires no registry entry");
    let decoded = serde_json::from_value::<RootTask>(json!({"type": "ApplyTestHandoff", "config": {"amount": 7}}));
    assert!(decoded.unwrap_err().to_string().contains("unknown task"));
    let composition = Composition::Inline(Definition {
        tasks: vec![RootTask { task_type: "ApplyTestHandoff".into(), config: json!({}), after: Vec::new() }],
        task_defaults: TaskDefaults::default(),
    });
    assert!(composition.resolve().unwrap_err().to_string().contains("unknown task"));
}

#[test]
fn unregistered_child_defaults_are_validated_when_the_child_is_constructed() {
    let defaults: TaskDefaults = serde_json::from_value(json!({"ApplyTestHandoff": {"typo": 7}})).unwrap();
    let mut scheduler = Scheduler::new().with_task_defaults(defaults).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    scheduler.add_task(SequenceTransfers {
        handoff: Some(ToteHandoff { tote: "first".into(), reservation: Rc::new(()) }),
        seen: seen.clone(),
    });
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("amount must be a nonnegative integer"));
    assert!(seen.borrow().is_empty());
}
