//! A deferred last resort for an empty HTM whose previous chain has ended.
//! Productive claims for the same availability win before a bay or path is
//! considered; only accepted parking performs physical planning.
//! `ParkClaimedHtm` is that planning alone, for an HTM whose stay another
//! task's claim already won.

use crate::events::EventJournal;
use crate::flows::ttp::demand::park_idle_htms::park_accepted_htm;
use crate::model::ParkingRegistry;
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry, ProposedLastResort};
use crate::model::navigation::Navigation;
use crate::prelude::*;
use crate::task::TaskContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkingOutcome {
    Parked,
    Superseded,
    Unavailable,
}

#[derive(Clone, Display)]
#[display("Parking(htm={:?})", entry.robot())]
pub struct Parking {
    entry: AvailableHtm,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    clock: Mut<SimTime>,
    retry_at_departure: bool,
}

impl Parking {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry: AvailableHtm,
        navigation: Arc<Navigation>,
        bots: BotService,
        inventory: InventoryService,
        available: AvailableHtmRegistry,
        parking: Option<ParkingRegistry>,
        events: EventJournal,
        clock: Mut<SimTime>,
    ) -> Result<Self, PlanError> {
        if entry.tote.is_some() || !entry.cell_reservation.is_open_ended() {
            return Err(PlanError::TaskGraphInvariant {
                context: "Parking",
                detail: "parking requires an empty HTM with an open-ended resting hold".into(),
            });
        }
        Ok(Self { entry, navigation, bots, inventory, available, parking, events, clock, retry_at_departure: true })
    }

    /// An attempt that finds no bay comes back `Unavailable` once it is
    /// decided (scheduler time), instead of trying again when the barrier
    /// reaches the HTM's departure: for a caller that keeps its own deadline
    /// and tries the other holders itself, as a clearance's return does.
    pub fn without_retry(mut self) -> Self {
        self.retry_at_departure = false;
        self
    }
}

#[async_trait(?Send)]
impl Task for Parking {
    type Proposal = ProposedLastResort;
    type Schedule = ParkingOutcome;

    fn dissolve(&self) -> Option<Self::Schedule> {
        (!self.available.is_available(self.entry.availability_id())).then_some(ParkingOutcome::Superseded)
    }

    async fn propose(&self, owner: ProposalId) -> Result<Self::Proposal, PlanError> {
        Ok(ProposedLastResort::parking(&self.available, &self.navigation, &self.entry, owner, self.clock.clone()))
    }

    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let (entry, depart) = proposal.into_accepted()?;
        let moved = park_accepted_htm(
            &self.navigation,
            &self.available,
            &self.bots,
            &self.inventory,
            self.parking.as_ref(),
            &self.events,
            entry,
            depart,
        )
        .await?;
        if moved {
            return Ok(ParkingOutcome::Parked);
        }
        if self.retry_at_departure && self.clock.value() < depart {
            // An early release decision cannot declare future parking
            // unavailable. The helper restored the original idle claim;
            // productive work can take it while its current dwell finishes.
            ctx.barrier().wait_until(depart).await?;
            // The accepted attempt has finished without moving. A new task
            // competes for the retry and can dissolve before acceptance if
            // productive work already claimed this HTM.
            let mut retry = self.clone();
            retry.retry_at_departure = false;
            return Ok(ctx.add_task(retry).await);
        }
        Ok(ParkingOutcome::Unavailable)
    }
}

/// Park an empty HTM whose stay a won claim already holds (a station's
/// clearance), departing at `depart`. Failure restores the stay, for the
/// clearance to compete again or stop the run.
#[derive(Display)]
#[display("ParkClaimedHtm(htm={:?})", entry.as_ref().map(AvailableHtm::robot))]
pub struct ParkClaimedHtm {
    entry: Option<AvailableHtm>,
    depart: SimTime,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
}

impl ParkClaimedHtm {
    pub fn new(context: &TaskContext, entry: AvailableHtm, depart: SimTime) -> Result<Self, PlanError> {
        let parking = context.state::<ParkingRegistry>()?;
        Ok(Self {
            entry: Some(entry),
            depart,
            navigation: context.state::<Navigation>()?,
            bots: context.state::<BotService>()?.as_ref().clone(),
            inventory: context.state::<InventoryService>()?.as_ref().clone(),
            available: context.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: context.state::<EventJournal>()?.as_ref().clone(),
        })
    }
}

#[async_trait(?Send)]
impl Task for ParkClaimedHtm {
    type Proposal = EmptyProposal;
    type Schedule = ParkingOutcome;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<ParkingOutcome, PlanError> {
        let entry = self.entry.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "ParkClaimedHtm",
            detail: "scheduled twice".into(),
        })?;
        let moved = park_accepted_htm(
            &self.navigation,
            &self.available,
            &self.bots,
            &self.inventory,
            self.parking.as_ref(),
            &self.events,
            entry,
            self.depart,
        )
        .await?;
        Ok(if moved { ParkingOutcome::Parked } else { ParkingOutcome::Unavailable })
    }
}

#[cfg(test)]
mod tests;
