//! Return one tote through its reserved relay slot: the HTM deposits it and
//! then a VTM restores it to rack storage.

use crate::events::{EventJournal, ToteSource};
use crate::model::ParkingRegistry;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::navigation::Navigation;
use crate::model::relay::RelayReservation;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::pathfinding::Orientation;
use crate::{flows::ttp::task::*, prelude::*};
use desgraph::StationId;

#[derive(Display)]
#[display("ReturnTote(tote={:?} stn={:?})", tote_id, station_id)]
pub struct ReturnTote {
    pub station_id: StationId,
    pub robot_id: RobotId,
    pub tote_id: ToteId,
    pub predecessor: CellReservation,
    pub action_ready: SimTime,
    /// The latest the drive may leave the station's cell (simtime), when a
    /// clearance sends the tote home from its release: the run stops rather
    /// than have it leave later.
    pub departs_by: Option<SimTime>,
    pub orientation: Orientation,
    pub deposit: Option<RelayReservation<RackSlot>>,
    pub presentation_id: Option<PresentationId>,
    pub tote_source: ToteSource,
    pub navigation: Arc<Navigation>,
    pub relay: RelayService,
    pub bots: BotService,
    pub inventory: InventoryService,
    pub available: AvailableHtmRegistry,
    pub parking: Option<ParkingRegistry>,
    pub events: EventJournal,
}

impl Drop for ReturnTote {
    fn drop(&mut self) {
        if let Some(deposit) = self.deposit.take() {
            deposit.cancel();
        }
    }
}

#[async_trait(?Send)]
impl Task for ReturnTote {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let deposit = self.deposit.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "ReturnTote",
            detail: "scheduled after its relay reservation was consumed".into(),
        })?;
        let at_relay = ctx
            .add_task(
                HtmStore::new(
                    self.station_id,
                    self.robot_id,
                    self.tote_id,
                    self.predecessor.clone(),
                    self.action_ready,
                    deposit,
                    self.presentation_id,
                    self.tote_source,
                    ctx.barrier().clock(),
                    self.navigation.clone(),
                    self.relay.clone(),
                    self.bots.clone(),
                    self.inventory.clone(),
                    self.available.clone(),
                    self.parking.clone(),
                    self.events.clone(),
                )
                .with_orientation(self.orientation)
                .departing_by(self.departs_by),
            )
            .await;
        ctx.add_task(VtmReturn::new(
            at_relay,
            self.relay.vtm_service.clone(),
            self.inventory.clone(),
            self.events.clone(),
        ))
        .await;
        Ok(())
    }
}
