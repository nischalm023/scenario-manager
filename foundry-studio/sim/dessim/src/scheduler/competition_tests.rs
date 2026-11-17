//! Scheduler-level contracts for opaque family arbitration and dissolution.
use super::*;
use crate::scheduler::task::proposal::{AcceptError, EmptyProposal, Proposal, ProposalId, ProposedReservation};
use async_trait::async_trait;
use std::cell::Cell;

#[derive(Default)]
struct Family {
    claims: RefCell<FxHashMap<ProposalId, (bool, SimTime)>>,
    awarded: Cell<bool>,
}

struct Claim {
    id: ProposalId,
    start: SimTime,
    family: Option<Rc<Family>>,
    accepted: Rc<RefCell<Vec<TaskId>>>,
}
impl fmt::Display for Claim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "claim {}", self.id)
    }
}
impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(family) = &self.family {
            family.claims.borrow_mut().remove(&self.id);
        }
    }
}
impl ProposedReservation for Claim {
    fn start_time(&self) -> Option<SimTime> {
        Some(self.start)
    }
    fn poisoned(&self) -> bool {
        self.family.as_ref().is_some_and(|family| family.awarded.get())
    }
    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        let family = self.family.as_ref()?;
        let mut candidates: Vec<_> = family.claims.borrow().iter().map(|(&id, &rank)| (rank, id)).collect();
        candidates.sort_unstable();
        Some(candidates.into_iter().map(|(_, id)| id).collect())
    }
    fn accept(&mut self) -> Result<(), AcceptError> {
        if let Some(family) = &self.family {
            assert!(!family.awarded.replace(true));
        }
        self.accepted.borrow_mut().push(self.id.task);
        Ok(())
    }
}

struct Composite {
    decision: Option<SimTime>,
    claim: Claim,
    whole_admissible: bool,
}
impl fmt::Display for Composite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.claim.fmt(f)
    }
}
impl Proposal for Composite {
    fn resolve_by(&self) -> Option<SimTime> {
        self.decision.or_else(|| self.start_time())
    }

    fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
        vec![&mut self.claim]
    }
    fn start_time(&self) -> Option<SimTime> {
        ProposedReservation::start_time(&self.claim)
    }
    fn poisoned(&self) -> bool {
        ProposedReservation::poisoned(&self.claim)
    }
    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        ProposedReservation::ranked_competitors(&self.claim)
    }
    fn admissible(&self) -> bool {
        self.whole_admissible
    }
}

struct Candidate {
    decision: Option<SimTime>,
    whole_admissible: bool,
    start: SimTime,
    ranked_last: bool,
    family: Option<Rc<Family>>,
    accepted: Rc<RefCell<Vec<TaskId>>>,
    scheduled: Rc<Cell<u32>>,
}
impl fmt::Display for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "candidate at {}", self.start)
    }
}
#[async_trait(?Send)]
impl Task for Candidate {
    type Proposal = Composite;
    type Schedule = bool;
    fn dissolve(&self) -> Option<bool> {
        self.family.as_ref().is_some_and(|family| family.awarded.get()).then_some(false)
    }
    async fn propose(&self, id: ProposalId) -> Result<Composite, PlanError> {
        if let Some(family) = &self.family {
            family.claims.borrow_mut().insert(id, (self.ranked_last, self.start));
        }
        Ok(Composite {
            decision: self.decision,
            claim: Claim { id, start: self.start, family: self.family.clone(), accepted: self.accepted.clone() },
            whole_admissible: self.whole_admissible,
        })
    }
    async fn schedule(&mut self, _: Composite, _: SchedCtx) -> Result<bool, PlanError> {
        self.scheduled.set(self.scheduled.get() + 1);
        Ok(true)
    }
}

#[test]
fn no_family_keeps_start_then_task_order() {
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let scheduled = Rc::new(Cell::new(0));
    let mut scheduler = Scheduler::new();
    for start in [20, 10, 10] {
        scheduler.add_task(Candidate {
            decision: None,
            whole_admissible: true,
            start: SimTime::ms(start),
            ranked_last: false,
            family: None,
            accepted: accepted.clone(),
            scheduled: scheduled.clone(),
        });
    }
    scheduler.run().unwrap();
    assert_eq!(*accepted.borrow(), vec![TaskId(1), TaskId(2), TaskId(0)]);
    assert_eq!(scheduled.get(), 3);
    assert_eq!(scheduler.barrier().floor(), SimTime::ms(20));
}

#[test]
fn a_later_family_winner_dissolves_the_loser_without_invoking_schedule() {
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let family = Rc::new(Family::default());
    let low_scheduled = Rc::new(Cell::new(0));
    let mut scheduler = Scheduler::new();
    let low = scheduler.add_task(Candidate {
        decision: None,
        whole_admissible: true,
        start: SimTime::ms(10),
        ranked_last: true,
        family: Some(family.clone()),
        accepted: accepted.clone(),
        scheduled: low_scheduled.clone(),
    });
    let high = scheduler.add_task(Candidate {
        decision: None,
        whole_admissible: true,
        start: SimTime::ms(20),
        ranked_last: false,
        family: Some(family.clone()),
        accepted: accepted.clone(),
        scheduled: Rc::new(Cell::new(0)),
    });
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    assert!(!runtime.block_on(low));
    assert!(runtime.block_on(high));
    assert_eq!(*accepted.borrow(), vec![TaskId(1)]);
    assert_eq!(low_scheduled.get(), 0);
    assert_eq!(scheduler.barrier().floor(), SimTime::ms(10), "due time triggers acceptance, not winning start");
    assert!(family.claims.borrow().is_empty(), "every attempt releases its registry entry");
}

struct Producer {
    task: Option<Candidate>,
    at: SimTime,
}
impl fmt::Display for Producer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "producer at {}", self.at)
    }
}
#[async_trait(?Send)]
impl Task for Producer {
    type Proposal = EmptyProposal;
    type Schedule = bool;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<bool, PlanError> {
        ctx.barrier().wait_until(self.at).await?;
        Ok(ctx.add_task(self.task.take().unwrap()).await)
    }
}

#[test]
fn an_equal_time_producer_publishes_before_the_due_contest_is_decided() {
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let family = Rc::new(Family::default());
    let mut scheduler = Scheduler::new();
    scheduler.add_task(Candidate {
        decision: None,
        whole_admissible: true,
        start: SimTime::ms(10),
        ranked_last: true,
        family: Some(family.clone()),
        accepted: accepted.clone(),
        scheduled: Rc::new(Cell::new(0)),
    });
    scheduler.add_task(Producer {
        at: SimTime::ms(10),
        task: Some(Candidate {
            decision: None,
            whole_admissible: true,
            start: SimTime::ms(20),
            ranked_last: false,
            family: Some(family),
            accepted: accepted.clone(),
            scheduled: Rc::new(Cell::new(0)),
        }),
    });
    scheduler.run().unwrap();
    assert_eq!(*accepted.borrow(), vec![TaskId(2)], "new work at the trigger instant participates");
}

#[test]
fn family_priority_cannot_accept_an_inadmissible_whole_proposal() {
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let family = Rc::new(Family::default());
    let mut scheduler = Scheduler::new();
    scheduler.add_task(Candidate {
        decision: None,
        whole_admissible: true,
        start: SimTime::ms(10),
        ranked_last: true,
        family: Some(family.clone()),
        accepted: accepted.clone(),
        scheduled: Rc::new(Cell::new(0)),
    });
    scheduler.add_task(Candidate {
        decision: None,
        whole_admissible: false,
        start: SimTime::ms(20),
        ranked_last: false,
        family: Some(family),
        accepted: accepted.clone(),
        scheduled: Rc::new(Cell::new(0)),
    });
    scheduler.run().unwrap();
    assert_eq!(*accepted.borrow(), vec![TaskId(0)], "the preferred claim's other preconditions are not met");
}

#[test]
fn early_decision_does_not_advance_past_unrelated_due_work() {
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let mut scheduler = Scheduler::new();
    for (start, decision) in [(100, Some(SimTime::ZERO)), (10, None)] {
        scheduler.add_task(Candidate {
            decision,
            whole_admissible: true,
            start: SimTime::ms(start),
            ranked_last: false,
            family: None,
            accepted: accepted.clone(),
            scheduled: Rc::new(Cell::new(0)),
        });
    }
    scheduler.run().unwrap();
    assert_eq!(*accepted.borrow(), vec![TaskId(0), TaskId(1)]);
    assert_eq!(scheduler.barrier().floor(), SimTime::ms(10));
}

#[test]
fn decision_deadline_never_overrides_an_earlier_physical_start() {
    let mut scheduler = Scheduler::new();
    scheduler.add_task(Candidate {
        decision: Some(SimTime::ms(200)),
        whole_admissible: true,
        start: SimTime::ms(100),
        ranked_last: false,
        family: None,
        accepted: Rc::new(RefCell::new(Vec::new())),
        scheduled: Rc::new(Cell::new(0)),
    });
    scheduler.run().unwrap();
    assert_eq!(scheduler.barrier().floor(), SimTime::ms(100));
}

#[test]
fn an_early_decision_cannot_make_a_past_physical_start_legal() {
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let mut scheduler = Scheduler::new();
    scheduler.barrier().advance_to(SimTime::ms(20));
    scheduler.add_task(Candidate {
        decision: Some(SimTime::ms(20)),
        whole_admissible: true,
        start: SimTime::ms(10),
        ranked_last: false,
        family: None,
        accepted: accepted.clone(),
        scheduled: Rc::new(Cell::new(0)),
    });
    assert!(scheduler.run().is_err());
    assert!(accepted.borrow().is_empty());
}

#[derive(derive_more::Display)]
#[display("ExpiresWhileProposing")]
struct ExpiresWhileProposing {
    obsolete: Cell<bool>,
}

#[async_trait(?Send)]
impl Task for ExpiresWhileProposing {
    type Proposal = EmptyProposal;
    type Schedule = bool;
    fn dissolve(&self) -> Option<bool> {
        self.obsolete.get().then_some(false)
    }
    // Suspends once, so the task expires while its proposal is under
    // construction.
    #[allow(clippy::disallowed_methods)]
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        tokio::task::yield_now().await;
        self.obsolete.set(true);
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<bool, PlanError> {
        panic!("work that expired before acceptance must not schedule");
    }
}

#[test]
fn expiry_during_proposal_construction_dissolves_before_resource_free_acceptance() {
    let mut scheduler = Scheduler::new();
    let completion = scheduler.add_task(ExpiresWhileProposing { obsolete: Cell::new(false) });
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    assert!(!runtime.block_on(completion));
}
