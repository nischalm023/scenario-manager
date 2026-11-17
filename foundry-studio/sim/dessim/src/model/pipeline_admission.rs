//! Simulation-time CONWIP admission, independent of physical area capacity.

use std::hash::Hash;

use super::SimTime;
use super::area_admission::{AdmissionClaim, AdmissionQuote, AreaAdmission, AreaAdmissionError};
use crate::signals::Version;
use crate::state::{StateError, StateManager, States};

pub type PipelineAdmissionError = AreaAdmissionError;
pub type PipelineQuote = AdmissionQuote;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisitSpan {
    pub start: SimTime,
    pub finish: Option<SimTime>,
    pub slot: usize,
}

pub struct PipelineAdmission<C, V> {
    capacity: AreaAdmission<C, V>,
}

impl<C, V> Clone for PipelineAdmission<C, V> {
    fn clone(&self) -> Self {
        Self { capacity: self.capacity.clone() }
    }
}

impl<C, V> PipelineAdmission<C, V>
where
    C: Clone + Eq + Hash,
    V: Clone + Eq + Hash,
{
    pub fn empty() -> Self {
        Self { capacity: AreaAdmission::empty() }
    }

    pub fn declare_consumer(&self, consumer: C, limit: usize) -> Result<(), PipelineAdmissionError> {
        self.capacity.declare_area(consumer, limit)
    }

    pub fn version(&self) -> &Version {
        self.capacity.version()
    }

    pub fn quote(&self, consumer: &C, after: SimTime) -> Result<Option<PipelineQuote>, PipelineAdmissionError> {
        self.capacity.quote(consumer, after)
    }

    pub fn acquire(
        &self,
        consumer: C,
        visit: V,
        quote: PipelineQuote,
    ) -> Result<PipelinePermit<C, V>, PipelineAdmissionError> {
        let claim = self.capacity.claim(consumer.clone(), visit.clone(), quote)?;
        Ok(PipelinePermit { service: self.clone(), consumer, visit, claim: Some(claim) })
    }

    pub fn span(&self, visit: &V) -> Option<VisitSpan> {
        self.capacity.membership(visit).map(|span| VisitSpan { start: span.entry, finish: span.exit, slot: span.slot })
    }

    pub fn spans(&self, consumer: &C) -> Vec<(V, VisitSpan)> {
        self.capacity
            .memberships(consumer)
            .into_iter()
            .map(|(visit, span)| (visit, VisitSpan { start: span.entry, finish: span.exit, slot: span.slot }))
            .collect()
    }

    pub fn active_spans(&self, consumer: &C, at: SimTime) -> Vec<(V, VisitSpan)> {
        self.spans(consumer)
            .into_iter()
            .filter(|(_, span)| span.start <= at && span.finish.is_none_or(|finish| at < finish))
            .collect()
    }

    pub fn occupancy(&self, consumer: &C, at: SimTime) -> Result<usize, PipelineAdmissionError> {
        self.capacity.occupancy(consumer, at)
    }
}

impl<C, V> StateManager for PipelineAdmission<C, V>
where
    C: Clone + Eq + Hash + 'static,
    V: Clone + Eq + Hash + 'static,
{
    fn create(_: &States) -> Result<Self, StateError> {
        Ok(Self::empty())
    }
}

pub struct PipelinePermit<C: Clone + Eq + Hash, V: Clone + Eq + Hash> {
    service: PipelineAdmission<C, V>,
    consumer: C,
    visit: V,
    claim: Option<AdmissionClaim<C, V>>,
}

impl<C: Clone + Eq + Hash, V: Clone + Eq + Hash> PipelinePermit<C, V> {
    pub fn quoted_start(&self) -> SimTime {
        self.claim.as_ref().expect("unspent permit").entry()
    }

    pub fn start(mut self, actual_start: SimTime) -> Result<PipelineVisit<C, V>, PipelineAdmissionError> {
        self.claim.as_mut().expect("unspent permit").finalize(actual_start)?;
        self.claim.take();
        Ok(PipelineVisit { service: self.service.clone(), consumer: self.consumer.clone(), visit: self.visit.clone() })
    }
}

pub struct PipelineVisit<C, V> {
    service: PipelineAdmission<C, V>,
    consumer: C,
    visit: V,
}

impl<C: Clone + Eq + Hash, V: Clone + Eq + Hash> PipelineVisit<C, V> {
    pub fn finish(self, actual_pick_end: SimTime) -> Result<(), PipelineAdmissionError> {
        self.service.capacity.commit_exit(&self.consumer, &self.visit, actual_pick_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(seconds: u64) -> SimTime {
        SimTime::seconds(seconds)
    }

    #[test]
    fn limit_counts_visits_until_their_paired_pick_finishes() {
        let service = PipelineAdmission::empty();
        service.declare_consumer("station", 2).unwrap();
        let mut visits = Vec::new();
        for visit in 1..=2 {
            let quote = service.quote(&"station", t(0)).unwrap().unwrap();
            visits.push(service.acquire("station", visit, quote).unwrap().start(t(0)).unwrap());
        }
        assert_eq!(service.quote(&"station", t(0)).unwrap(), None);
        assert_eq!(service.active_spans(&"station", t(0)).len(), 2);
        visits.pop().unwrap().finish(t(10)).unwrap();
        assert_eq!(service.quote(&"station", t(0)).unwrap().unwrap().entry, t(10));
    }

    #[test]
    fn future_finish_releases_capacity_in_planning_time() {
        let service = PipelineAdmission::empty();
        service.declare_consumer("station", 1).unwrap();
        let quote = service.quote(&"station", t(5)).unwrap().unwrap();
        let visit = service.acquire("station", 1, quote).unwrap().start(t(5)).unwrap();
        visit.finish(t(50)).unwrap();
        let next = service.quote(&"station", t(10)).unwrap().unwrap();
        assert_eq!(next.entry, t(50));
        assert_eq!(service.span(&1).unwrap().finish, Some(t(50)));
    }

    #[test]
    fn canceled_permit_does_not_consume_conwip() {
        let service = PipelineAdmission::empty();
        service.declare_consumer("station", 1).unwrap();
        let quote = service.quote(&"station", t(0)).unwrap().unwrap();
        let permit = service.acquire("station", 1, quote).unwrap();
        assert_eq!(service.quote(&"station", t(0)).unwrap(), None);
        drop(permit);
        assert_eq!(service.quote(&"station", t(0)).unwrap().unwrap().entry, t(0));
    }
}
