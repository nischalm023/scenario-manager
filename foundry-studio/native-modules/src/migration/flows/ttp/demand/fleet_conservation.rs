//! Check the available-HTM registry against the physical fleet throughout a run.

use crate::model::bot::{BotKind, BotService};
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::prelude::*;
use crate::task;
use crate::task::TaskContext;

#[derive(Display)]
#[display("FleetConservation")]
#[task]
pub struct FleetConservation {
    bots: BotService,
    available: AvailableHtmRegistry,
}

#[async_trait(?Send)]
impl Task for FleetConservation {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        Ok(Self {
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
        })
    }

    fn holds_completion() -> bool {
        false
    }
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        loop {
            let seen = self.available.version().seen();
            let total = self.bots.count_kind(BotKind::Htm);
            if self.available.len() > total {
                return Err(PlanError::TaskGraphInvariant {
                    context: "conservation",
                    detail: format!("registry has {} idle HTMs but only {total} total exist", self.available.len()),
                });
            }
            self.available.version().past(seen).await;
        }
    }
}
