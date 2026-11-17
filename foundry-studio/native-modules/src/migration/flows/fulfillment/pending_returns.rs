//! Returning inventory totes await whichever retained HTM reaches the
//! collection cell next. Selection stays revocable until its physical
//! readiness time.

use std::sync::Mutex;

use desgraph::{RelayId, StationId, ZoneId};

use crate::events::TripEndpoint;
use crate::model::conveyance::{CollectionClaim, Conveyance, ConveyedTote, RetainedHtm, TotePhase};
use crate::prelude::*;
use crate::scheduler::task::proposal::{AcceptError, ProposedReservation};

pub(super) struct ReturnWork {
    pub conveyed: ConveyedTote,
    pub ready_at: SimTime,
    pub station: StationId,
    pub zone: ZoneId,
    pub source_relay: Option<RelayId>,
    pub presentation_id: PresentationId,
    pub deliver_from: Option<TripEndpoint>,
}

#[derive(Clone)]
pub(super) struct PendingReturns {
    work: Arc<Mutex<Vec<ReturnWork>>>,
    changed: Version,
}

impl PendingReturns {
    pub fn new() -> Self {
        Self { work: Arc::new(Mutex::new(Vec::new())), changed: Version::new() }
    }

    pub fn publish(&self, work: ReturnWork) -> Result<(), PlanError> {
        if work.conveyed.phase() != (TotePhase::Returning { ready_at: work.ready_at }) {
            return Err(super::invalid("return work does not match its conveyor readiness"));
        }
        let mut pending = self.work.lock().unwrap();
        if pending.iter().any(|other| other.conveyed.tote() == work.conveyed.tote()) {
            return Err(super::invalid("conveyor tote was published for collection twice"));
        }
        pending.push(work);
        self.changed.bump();
        Ok(())
    }
}

#[derive(Display)]
#[display("ClaimReturnedTote({:?})", retained.robot())]
pub(super) struct ClaimReturnedTote {
    pending: PendingReturns,
    conveyance: Conveyance,
    retained: RetainedHtm,
    available_at: SimTime,
    clock: Mut<SimTime>,
}

impl ClaimReturnedTote {
    pub fn new(
        pending: PendingReturns,
        conveyance: Conveyance,
        retained: RetainedHtm,
        available_at: SimTime,
        clock: Mut<SimTime>,
    ) -> Self {
        Self { pending, conveyance, retained, available_at, clock }
    }
}

#[derive(Display)]
#[display("return {tote:?} to {:?} at {at}", retained.robot())]
pub(super) struct ProposedReturn {
    pending: PendingReturns,
    conveyance: Conveyance,
    retained: RetainedHtm,
    tote: ToteId,
    at: SimTime,
    seen: u64,
    clock: Mut<SimTime>,
    accepted: Option<(ReturnWork, CollectionClaim)>,
}

impl ProposedReservation for ProposedReturn {
    fn start_time(&self) -> Option<SimTime> {
        Some(self.at.max(self.clock.value()))
    }

    fn poisoned(&self) -> bool {
        // Capacity is small; reselect after any publication or collection.
        // This also distinguishes successive visits of the same physical tote.
        self.pending.changed.seen() != self.seen
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        let mut pending = self.pending.work.lock().unwrap();
        if self.pending.changed.seen() != self.seen {
            return Err(AcceptError::Poisoned { detail: "return selection changed before acceptance".into() });
        }
        let index = pending.iter().position(|work| work.conveyed.tote() == self.tote).ok_or_else(|| {
            AcceptError::ActualConflict { detail: "selected return is missing without a version change".into() }
        })?;
        let claim = self
            .conveyance
            .try_collect(&pending[index].conveyed, &self.retained, self.at.max(self.clock.value()))
            .map_err(|error| AcceptError::ActualConflict { detail: error.to_string() })?
            .ok_or_else(|| AcceptError::ActualConflict {
                detail: "return custody was claimed outside its queue".into(),
            })?;
        self.accepted = Some((pending.swap_remove(index), claim));
        self.pending.changed.bump();
        Ok(())
    }
}

impl Drop for ProposedReturn {
    fn drop(&mut self) {
        if let Some((work, claim)) = self.accepted.take() {
            // Cancellation before schedule restores selection only; physical
            // custody and both capacity reservations remain in force.
            drop(claim);
            let mut pending = self.pending.work.lock().unwrap();
            pending.push(work);
            self.pending.changed.bump();
        }
    }
}

#[async_trait(?Send)]
impl Task for ClaimReturnedTote {
    type Proposal = ProposedReturn;
    type Schedule = (ReturnWork, CollectionClaim);

    async fn propose(&self, _: ProposalId) -> Result<ProposedReturn, PlanError> {
        loop {
            let selected = {
                let pending = self.pending.work.lock().unwrap();
                pending.iter().min_by_key(|work| (work.ready_at, work.conveyed.tote())).map(|work| {
                    (work.conveyed.tote(), work.ready_at.max(self.available_at), self.pending.changed.seen())
                })
            };
            if let Some((tote, at, seen)) = selected {
                return Ok(ProposedReturn {
                    pending: self.pending.clone(),
                    conveyance: self.conveyance.clone(),
                    retained: self.retained.clone(),
                    tote,
                    at,
                    seen,
                    clock: self.clock.clone(),
                    accepted: None,
                });
            }
            let seen = self.pending.changed.seen();
            if self.pending.work.lock().unwrap().is_empty() {
                self.pending.changed.past(seen).await;
            }
        }
    }

    async fn schedule(&mut self, mut proposal: ProposedReturn, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        proposal.accepted.take().ok_or_else(|| super::invalid("return claim scheduled before acceptance"))
    }
}

#[cfg(test)]
#[path = "pending_returns_tests.rs"]
mod tests;
