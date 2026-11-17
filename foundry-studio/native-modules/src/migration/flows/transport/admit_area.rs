//! Scheduler-mediated acquisition of a simulation-time area slot. Quoting
//! owns no capacity; acceptance owns a cancelable claim for the movement task.

use std::fmt;
use std::hash::Hash;

use crate::model::area_admission::{AdmissionClaim, AdmissionQuote, AreaAdmission};
use crate::prelude::*;
use crate::scheduler::task::proposal::{AcceptError, ProposedReservation};
use crate::task::invalid;

/// Accepts one capacity slot without planning the movement into the area.
pub struct AdmitArea<A, M> {
    service: AreaAdmission<A, M>,
    area: A,
    member: M,
    after: SimTime,
    clock: Mut<SimTime>,
}

impl<A, M> AdmitArea<A, M> {
    pub fn new(service: AreaAdmission<A, M>, area: A, member: M, after: SimTime, clock: Mut<SimTime>) -> Self {
        Self { service, area, member, after, clock }
    }
}

impl<A, M> fmt::Display for AdmitArea<A, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AdmitArea")
    }
}

pub struct ProposedAdmission<A: Clone + Eq + Hash, M: Clone + Eq + Hash> {
    service: AreaAdmission<A, M>,
    area: A,
    member: M,
    quote: AdmissionQuote,
    after: SimTime,
    clock: Mut<SimTime>,
    seen: u64,
    accepted: Option<AdmissionClaim<A, M>>,
}

impl<A: Clone + Eq + Hash, M: Clone + Eq + Hash> ProposedAdmission<A, M> {
    fn accepted_at(&self) -> SimTime {
        self.quote.entry.max(self.after).max(self.clock.value())
    }
}

impl<A: Clone + Eq + Hash, M: Clone + Eq + Hash> fmt::Display for ProposedAdmission<A, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AdmitArea(slot={}, at={})", self.quote.slot, self.accepted_at())
    }
}

impl<A, M> ProposedReservation for ProposedAdmission<A, M>
where
    A: Clone + Eq + Hash + 'static,
    M: Clone + Eq + Hash + 'static,
{
    fn start_time(&self) -> Option<SimTime> {
        Some(self.accepted_at())
    }

    fn poisoned(&self) -> bool {
        self.accepted.is_none() && self.service.version().seen() != self.seen
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        if ProposedReservation::poisoned(self) {
            return Err(AcceptError::Poisoned { detail: "area availability changed before acceptance".into() });
        }
        let quote = AdmissionQuote { entry: self.accepted_at(), slot: self.quote.slot };
        self.accepted = Some(
            self.service
                .claim(self.area.clone(), self.member.clone(), quote)
                .map_err(|error| AcceptError::ActualConflict { detail: format!("area claim failed: {error:?}") })?,
        );
        Ok(())
    }
}

impl<A, M> Drop for ProposedAdmission<A, M>
where
    A: Clone + Eq + Hash,
    M: Clone + Eq + Hash,
{
    fn drop(&mut self) {
        if let Some(mut claim) = self.accepted.take() {
            let _ = claim.cancel();
        }
    }
}

#[async_trait(?Send)]
impl<A, M> Task for AdmitArea<A, M>
where
    A: Clone + Eq + Hash + 'static,
    M: Clone + Eq + Hash + 'static,
{
    type Proposal = ProposedAdmission<A, M>;
    type Schedule = AdmissionClaim<A, M>;

    async fn propose(&self, _: ProposalId) -> Result<Self::Proposal, PlanError> {
        loop {
            let after = self.after.max(self.clock.value());
            let seen = self.service.version().seen();
            let quote = self
                .service
                .quote(&self.area, after)
                .map_err(|error| invalid(format!("area admission quote failed: {error:?}")))?;
            if let Some(quote) = quote {
                return Ok(ProposedAdmission {
                    service: self.service.clone(),
                    area: self.area.clone(),
                    member: self.member.clone(),
                    quote,
                    after: self.after,
                    clock: self.clock.clone(),
                    seen,
                    accepted: None,
                });
            }
            // Recheck after subscribing so a mutation between quote and wait
            // cannot strand this task behind an already-fired version.
            if self.service.version().seen() == seen {
                self.service.version().past(seen).await;
            }
        }
    }

    async fn schedule(&mut self, mut proposal: Self::Proposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        proposal.accepted.take().ok_or_else(|| invalid("area admission scheduled before acceptance"))
    }
}

#[cfg(test)]
mod tests {
    use tokio::runtime::Builder;

    use super::*;

    fn t(seconds: u64) -> SimTime {
        SimTime::seconds(seconds)
    }

    fn proposal(task: &AdmitArea<&'static str, u32>) -> ProposedAdmission<&'static str, u32> {
        Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(Task::propose(task, ProposalId { task: TaskId(0), attempt: 0 }))
            .unwrap()
    }

    #[test]
    fn competing_publications_book_only_the_accepted_slot() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let clock = Mut::new(t(0));
        let first = AdmitArea::new(service.clone(), "area", 1, t(10), clock.clone());
        let second = AdmitArea::new(service.clone(), "area", 2, t(10), clock);
        let mut winner = proposal(&first);
        let mut loser = proposal(&second);

        assert_eq!(service.occupancy(&"area", t(10)).unwrap(), 0);
        ProposedReservation::accept(&mut winner).unwrap();
        assert_eq!(service.occupancy(&"area", t(10)).unwrap(), 1);
        assert!(ProposedReservation::poisoned(&loser));
        assert!(matches!(ProposedReservation::accept(&mut loser), Err(AcceptError::Poisoned { .. })));
        drop(loser);
        assert_eq!(service.occupancy(&"area", t(10)).unwrap(), 1, "dropping an unaccepted loser changes nothing");
        drop(winner);
        assert_eq!(service.occupancy(&"area", t(10)).unwrap(), 0, "cancelled acceptance rolls back its slot");
    }

    #[test]
    fn proposal_uses_committed_future_exit_without_advancing_clock() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let first_quote = service.quote(&"area", t(0)).unwrap().unwrap();
        let mut first = service.claim("area", 1, first_quote).unwrap();
        first.finalize(t(0)).unwrap();
        service.commit_exit(&"area", &1, t(50)).unwrap();

        let task = AdmitArea::new(service, "area", 2, t(10), Mut::new(t(3)));
        let mut proposed = proposal(&task);
        assert_eq!(ProposedReservation::start_time(&proposed), Some(t(50)));
        ProposedReservation::accept(&mut proposed).unwrap();
        assert_eq!(proposed.accepted.as_ref().unwrap().entry(), t(50));
    }

    #[test]
    fn acceptance_respects_clock_advance_after_publication() {
        let service = AreaAdmission::new([("area", 1)]).unwrap();
        let clock = Mut::new(t(5));
        let task = AdmitArea::new(service, "area", 1, t(10), clock.clone());
        let mut proposed = proposal(&task);
        clock.set(t(20));
        ProposedReservation::accept(&mut proposed).unwrap();
        assert_eq!(proposed.accepted.as_ref().unwrap().entry(), t(20));
    }
}
