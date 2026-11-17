use tokio::runtime::Builder;

use super::*;
use crate::model::conveyor::ConveyorId;

fn returning(service: &Conveyance, id: u32, ready: u64) -> (ReturnWork, RetainedHtm) {
    let (conveyed, htm) =
        service.try_admit(ToteId(id), RobotId(id), SimTime::ZERO).unwrap().unwrap().board(SimTime::ZERO).unwrap();
    let held =
        service.try_present(&conveyed, StationId(id), SimTime::ZERO).unwrap().unwrap().present(SimTime::ZERO).unwrap();
    let ready_at = SimTime::seconds(ready);
    held.clear(SimTime::ZERO, ready_at).unwrap();
    (
        ReturnWork {
            conveyed,
            ready_at,
            station: StationId(id),
            zone: ZoneId(id),
            source_relay: Some(RelayId(id)),
            presentation_id: PresentationId(id as u64),
            deliver_from: Some(TripEndpoint::Station(StationId(id))),
        },
        htm,
    )
}

fn service() -> Conveyance {
    Conveyance::new(ConveyorId(0), 2, 2, [StationId(1), StationId(2)]).unwrap()
}

fn proposal(task: &ClaimReturnedTote) -> ProposedReturn {
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(Task::propose(task, ProposalId { task: TaskId(0), attempt: 0 }))
        .unwrap()
}

#[test]
fn earlier_return_poisons_a_future_selection_and_cancellation_keeps_capacity() {
    let service = service();
    let returns = PendingReturns::new();
    let (later, head) = returning(&service, 1, 100);
    let (earlier, _tail) = returning(&service, 2, 50);
    returns.publish(later).unwrap();
    let task = ClaimReturnedTote::new(returns.clone(), service.clone(), head, SimTime::ZERO, Mut::new(SimTime::ZERO));
    let mut old = proposal(&task);
    assert_eq!(ProposedReservation::start_time(&old), Some(SimTime::seconds(100)));
    returns.publish(earlier).unwrap();
    assert!(ProposedReservation::poisoned(&old));
    assert!(matches!(ProposedReservation::accept(&mut old), Err(AcceptError::Poisoned { .. })));
    drop(old);
    let mut selected = proposal(&task);
    assert_eq!(selected.tote, ToteId(2));
    assert_eq!(service.returning().len(), 2, "proposing cannot spend custody");
    ProposedReservation::accept(&mut selected).unwrap();
    assert_eq!(service.returning().len(), 1);
    assert_eq!(service.occupancy_at(SimTime::seconds(100)).conveyor, 2);
    assert_eq!(service.occupancy_at(SimTime::seconds(100)).retained_htms, 2);
    drop(selected);
    assert_eq!(service.returning().len(), 2, "cancelled acceptance restores selection");
    assert_eq!(returns.work.lock().unwrap().len(), 2);
    assert!(service.abandoned_custody().is_empty());
}

#[test]
fn competing_claims_cannot_assign_one_tote_twice_and_ties_use_tote_id() {
    let service = service();
    let returns = PendingReturns::new();
    let (a, first) = returning(&service, 1, 50);
    let (b, second) = returning(&service, 2, 50);
    returns.publish(b).unwrap();
    returns.publish(a).unwrap();
    let first = ClaimReturnedTote::new(returns.clone(), service.clone(), first, SimTime::ZERO, Mut::new(SimTime::ZERO));
    let second =
        ClaimReturnedTote::new(returns.clone(), service.clone(), second, SimTime::ZERO, Mut::new(SimTime::ZERO));
    let mut first_claim = proposal(&first);
    let mut second_claim = proposal(&second);
    assert_eq!((first_claim.tote, second_claim.tote), (ToteId(1), ToteId(1)));
    ProposedReservation::accept(&mut first_claim).unwrap();
    assert!(ProposedReservation::poisoned(&second_claim));
    assert!(ProposedReservation::accept(&mut second_claim).is_err());
    let replacement = proposal(&second);
    assert_eq!(replacement.tote, ToteId(2));
}

#[derive(Display)]
#[display("PublishReturn")]
struct PublishReturn {
    returns: PendingReturns,
    work: Option<ReturnWork>,
}

#[async_trait(?Send)]
impl Task for PublishReturn {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(SimTime::seconds(10)).await?;
        self.returns.publish(self.work.take().unwrap())
    }
}

#[test]
fn scheduler_reselects_when_an_earlier_return_is_published_before_acceptance() {
    let service = service();
    let returns = PendingReturns::new();
    let (later, head) = returning(&service, 1, 100);
    let (earlier, _tail) = returning(&service, 2, 50);
    returns.publish(later).unwrap();
    let mut scheduler = Scheduler::new();
    let assigned = scheduler.add_task(ClaimReturnedTote::new(
        returns.clone(),
        service.clone(),
        head,
        SimTime::ZERO,
        scheduler.barrier().clock(),
    ));
    scheduler.add_task(PublishReturn { returns, work: Some(earlier) });
    scheduler.run().unwrap();
    let (work, claim) = Builder::new_current_thread().build().unwrap().block_on(assigned);
    assert_eq!(work.conveyed.tote(), ToteId(2));
    assert_eq!(claim.robot(), RobotId(1));
    assert_eq!(claim.at(), SimTime::seconds(50));
    assert_eq!(scheduler.barrier().floor(), SimTime::seconds(50));
}
