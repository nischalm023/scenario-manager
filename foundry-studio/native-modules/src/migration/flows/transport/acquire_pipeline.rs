//! Scheduler-mediated acquisition of a CONWIP pipeline permit.

use std::{fmt, hash::Hash};

use crate::model::pipeline_admission::{PipelineAdmission, PipelinePermit, PipelineQuote};
use crate::prelude::*;
use crate::scheduler::task::proposal::{AcceptError, ProposedReservation};
use crate::task::invalid;

pub struct AcquirePipeline<C, V> {
    service: PipelineAdmission<C, V>,
    consumer: C,
    visit: V,
    after: SimTime,
    clock: Mut<SimTime>,
}

impl<C, V> AcquirePipeline<C, V> {
    pub fn new(service: PipelineAdmission<C, V>, consumer: C, visit: V, after: SimTime, clock: Mut<SimTime>) -> Self {
        Self { service, consumer, visit, after, clock }
    }
}

impl<C, V> fmt::Display for AcquirePipeline<C, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AcquirePipeline")
    }
}

pub struct ProposedPipeline<C: Clone + Eq + Hash, V: Clone + Eq + Hash> {
    service: PipelineAdmission<C, V>,
    consumer: C,
    visit: V,
    quote: PipelineQuote,
    after: SimTime,
    clock: Mut<SimTime>,
    seen: u64,
    accepted: Option<PipelinePermit<C, V>>,
}

impl<C: Clone + Eq + Hash, V: Clone + Eq + Hash> ProposedPipeline<C, V> {
    fn accepted_at(&self) -> SimTime {
        self.quote.entry.max(self.after).max(self.clock.value())
    }
}

impl<C: Clone + Eq + Hash, V: Clone + Eq + Hash> fmt::Display for ProposedPipeline<C, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AcquirePipeline(slot={}, at={})", self.quote.slot, self.accepted_at())
    }
}

impl<C, V> ProposedReservation for ProposedPipeline<C, V>
where
    C: Clone + Eq + Hash + 'static,
    V: Clone + Eq + Hash + 'static,
{
    fn start_time(&self) -> Option<SimTime> {
        Some(self.accepted_at())
    }
    fn poisoned(&self) -> bool {
        self.accepted.is_none() && self.service.version().seen() != self.seen
    }
    fn accept(&mut self) -> Result<(), AcceptError> {
        if ProposedReservation::poisoned(self) {
            return Err(AcceptError::Poisoned { detail: "pipeline availability changed before acceptance".into() });
        }
        let quote = PipelineQuote { entry: self.accepted_at(), slot: self.quote.slot };
        self.accepted =
            Some(self.service.acquire(self.consumer.clone(), self.visit.clone(), quote).map_err(|error| {
                AcceptError::ActualConflict { detail: format!("pipeline permit failed: {error:?}") }
            })?);
        Ok(())
    }
}

#[async_trait(?Send)]
impl<C, V> Task for AcquirePipeline<C, V>
where
    C: Clone + Eq + Hash + 'static,
    V: Clone + Eq + Hash + 'static,
{
    type Proposal = ProposedPipeline<C, V>;
    type Schedule = PipelinePermit<C, V>;

    async fn propose(&self, _: ProposalId) -> Result<Self::Proposal, PlanError> {
        loop {
            let after = self.after.max(self.clock.value());
            let seen = self.service.version().seen();
            if let Some(quote) = self
                .service
                .quote(&self.consumer, after)
                .map_err(|error| invalid(format!("pipeline quote failed: {error:?}")))?
            {
                return Ok(ProposedPipeline {
                    service: self.service.clone(),
                    consumer: self.consumer.clone(),
                    visit: self.visit.clone(),
                    quote,
                    after: self.after,
                    clock: self.clock.clone(),
                    seen,
                    accepted: None,
                });
            }
            if self.service.version().seen() == seen {
                self.service.version().past(seen).await;
            }
        }
    }

    async fn schedule(&mut self, mut proposal: Self::Proposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        proposal.accepted.take().ok_or_else(|| invalid("pipeline permit scheduled before acceptance"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Builder;

    fn propose(task: &AcquirePipeline<&'static str, u32>) -> ProposedPipeline<&'static str, u32> {
        Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(Task::propose(task, ProposalId { task: TaskId(0), attempt: 0 }))
            .unwrap()
    }

    #[test]
    fn competing_tasks_only_book_on_acceptance() {
        let service = PipelineAdmission::empty();
        service.declare_consumer("station", 1).unwrap();
        let clock = Mut::new(SimTime::ZERO);
        let first = AcquirePipeline::new(service.clone(), "station", 1, SimTime::ZERO, clock.clone());
        let second = AcquirePipeline::new(service.clone(), "station", 2, SimTime::ZERO, clock);
        let mut winner = propose(&first);
        let mut loser = propose(&second);
        assert_eq!(service.occupancy(&"station", SimTime::ZERO).unwrap(), 0);
        ProposedReservation::accept(&mut winner).unwrap();
        assert!(ProposedReservation::poisoned(&loser));
        assert!(ProposedReservation::accept(&mut loser).is_err());
        drop(winner);
        assert_eq!(service.occupancy(&"station", SimTime::ZERO).unwrap(), 0);
    }
}
