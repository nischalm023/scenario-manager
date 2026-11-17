//! Store a held tote in the requested rack pool and return its physical arrival time.

use std::future::pending;

use desgraph::StationId;
use tokio::select;

use crate::error::ReservationError;
use crate::events::{EventJournal, ToteSource};
use crate::flows::transport::PathCommitObserver;
use crate::flows::ttp::demand::park_idle_htms::parking_relief;
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::flows::ttp::task::{HtmStore, VtmReturn};
use crate::model::ParkingRegistry;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::navigation::Navigation;
use crate::prelude::*;
use crate::scenario_gen::RackUse;

#[derive(Display)]
#[display("StoreTote({pool:?})")]
pub struct StoreTote {
    pub handoff: Option<HtmCarryingTote>,
    pub commit_observer: Option<PathCommitObserver>,
    pub pool: RackUse,
    pub station: StationId,
    pub navigation: Arc<Navigation>,
    pub relay: RelayService,
    pub bots: BotService,
    pub inventory: InventoryService,
    pub available: AvailableHtmRegistry,
    pub parking: Option<ParkingRegistry>,
    pub events: EventJournal,
}

#[async_trait(?Send)]
impl Task for StoreTote {
    type Proposal = EmptyProposal;
    type Schedule = SimTime;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<SimTime, PlanError> {
        let handoff = self.handoff.take().ok_or_else(|| super::invalid("StoreTote scheduled twice"))?;
        let robot = handoff.cell_reservation.robot();
        let from = handoff.cell_reservation.cell();
        let deposit = loop {
            let oracle_version = self.navigation.oracle.version();
            let oracle_seen = oracle_version.map(Version::seen);
            let relay_changed = notified(self.relay.signal());
            let inventory_seen = self.inventory.version().seen();
            let after = handoff.action_ready_at.max(ctx.barrier().clock().value());
            let relief = parking_relief(
                &self.navigation,
                &self.available,
                &self.bots,
                &self.inventory,
                self.parking.as_ref(),
                &self.events,
            );
            let reservation = match self.pool {
                RackUse::Inventory => {
                    self.relay.reserve_relay_for_deposit(from, handoff.orientation, after, robot, Some(&relief)).await
                }
                RackUse::OrderDepot => {
                    self.relay
                        .reserve_depot_relay_for_deposit(from, handoff.orientation, after, robot, Some(&relief))
                        .await
                }
                RackUse::OrderBuffer => {
                    self.relay
                        .reserve_buffer_relay_for_deposit(from, handoff.orientation, after, robot, Some(&relief))
                        .await
                }
            };
            match reservation {
                Ok(deposit) => break deposit,
                Err(
                    ReservationError::NoCandidates { .. }
                    | ReservationError::BeyondAccessHorizon { .. }
                    | ReservationError::HeadroomExhausted { .. },
                ) => {
                    // Failed synchronous relief takes and restores an idle HTM,
                    // advancing the fleet version without clearing any floor.
                    // Wait for a subsequent change, not our own restoration.
                    let fleet_seen = self.available.version().seen();
                    select! {
                        _ = relay_changed => {},
                        _ = self.inventory.version().past(inventory_seen) => {},
                        _ = self.available.version().past(fleet_seen) => {},
                        _ = async {
                            match oracle_version.zip(oracle_seen) {
                                Some((version, seen)) => version.past(seen).await,
                                None => pending().await,
                            }
                        } => {},
                    }
                }
                Err(error) => return Err(error.into()),
            }
        };
        let mut store = HtmStore::new(
            self.station,
            robot,
            handoff.tote_id,
            handoff.cell_reservation,
            handoff.action_ready_at,
            deposit,
            Some(handoff.presentation_id),
            ToteSource::Pick,
            ctx.barrier().clock(),
            self.navigation.clone(),
            self.relay.clone(),
            self.bots.clone(),
            self.inventory.clone(),
            self.available.clone(),
            self.parking.clone(),
            self.events.clone(),
        )
        .with_orientation(handoff.orientation);
        if let Some(observer) = self.commit_observer.take() {
            store = store.with_commit_observer(observer);
        }
        let stored = ctx.add_task(store).await;
        Ok(ctx
            .add_task(VtmReturn::new(
                stored,
                self.relay.vtm_service.clone(),
                self.inventory.clone(),
                self.events.clone(),
            ))
            .await)
    }
}

#[cfg(test)]
#[path = "store_tote_tests.rs"]
mod tests;
