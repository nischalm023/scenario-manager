//! Retrieve a matching tote through the VTM/relay/HTM chain, optionally continuing
//! its owned HTM journey to the requested destination before returning custody.

use desgraph::CellId;
use serde_json::json;

use crate::events::{EventJournal, ToteSource};
use crate::flows::transport::{HtmPosition, Navigate};
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::flows::ttp::task::{HtmPickup, VtmForward};
use crate::model::ParkingRegistry;
use crate::model::htm::registry::PickupHeadroom;
use crate::model::navigation::Navigation;
use crate::model::relay::{ProposedRetrieve, RetrieveAdmission, ToteSearchCriteria};
use crate::prelude::*;

#[derive(Display)]
#[display("RetrieveTote(to={target:?})")]
pub struct RetrieveTote {
    pub criteria: ToteSearchCriteria,
    pub target: CellId,
    pub after: SimTime,
    pub relay: RelayService,
    pub inventory: InventoryService,
    pub events: EventJournal,
    pub aim_version: Version,
    pub admission: Option<RetrieveAdmission>,
    pub pickup_headroom: Option<PickupHeadroom>,
}

#[async_trait(?Send)]
impl Task for RetrieveTote {
    type Proposal = ProposedRetrieve;
    type Schedule = HtmCarryingTote;

    async fn propose(&self, owner: ProposalId) -> Result<Self::Proposal, PlanError> {
        Ok(self
            .relay
            .source_retrieve_with_admission(
                self.criteria,
                self.target,
                self.after,
                SimTime::ZERO,
                || None,
                &self.aim_version,
                owner,
                self.admission.as_ref(),
            )
            .await?)
    }

    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let (reservation, rack_slot) = proposal.into_accepted()?;
        let handoff = ctx
            .add_task(
                VtmForward::new(
                    rack_slot,
                    reservation,
                    None,
                    self.relay.clone(),
                    self.inventory.clone(),
                    self.events.clone(),
                )
                .with_source(ToteSource::Pick),
            )
            .await;
        let context = ctx.task_context::<HtmPickup>(json!({}))?;
        let mut pickup = HtmPickup::new(&context, handoff)?;
        if let Some(headroom) = &self.pickup_headroom {
            pickup = pickup.with_headroom(headroom.clone());
        } else if let Some(admission) = &self.admission {
            pickup = pickup.with_admission(admission.clone());
        }
        context.finish()?;
        Ok(ctx.add_task(pickup).await)
    }
}

#[derive(Display)]
#[display("RetrieveAndDeliverTote")]
pub(super) struct RetrieveAndDeliverTote {
    pub retrieval: Option<RetrieveTote>,
    pub navigation: Arc<Navigation>,
    pub bots: BotService,
    pub inventory: InventoryService,
    pub parking: Option<ParkingRegistry>,
    pub events: EventJournal,
}

#[async_trait(?Send)]
impl Task for RetrieveAndDeliverTote {
    type Proposal = EmptyProposal;
    type Schedule = HtmCarryingTote;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let retrieval = self.retrieval.take().ok_or_else(|| super::invalid("retrieval already handed off"))?;
        let destination = retrieval.target;
        let mut tote = ctx.add_task(retrieval).await;
        let position = ctx
            .add_task(Navigate::new(
                HtmPosition {
                    cell_reservation: tote.cell_reservation.clone(),
                    action_ready_at: tote.action_ready_at,
                    orientation: tote.orientation,
                },
                destination,
                Some(tote.tote_id),
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                self.parking.clone(),
                self.events.clone(),
                ctx.barrier().clock(),
            ))
            .await;
        tote.cell_reservation = position.cell_reservation;
        tote.action_ready_at = position.action_ready_at;
        tote.orientation = position.orientation;
        Ok(tote)
    }
}
