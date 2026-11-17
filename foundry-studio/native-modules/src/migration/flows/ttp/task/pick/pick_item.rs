//! One admitted inventory pick: retrieve its tote, deliver the HTM, then
//! perform the operator transfer and release the presentation.

use super::station::*;
use crate::events::{EventJournal, ToteSource};
use crate::flows::ttp::task::clearance::ClearPresentation;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry};
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::station_service::StationService;
use crate::{
    flows::ttp::{demand::*, inflight::*, task::*},
    model::relay::*,
    prelude::*,
};
use serde_json::json;

pub struct PickLine {
    pub order: OrderId,
    pub sku: SkuId,
    pub qty: u32,
    pub dc: Option<DistributionCenterId>,
}

#[derive(Display)]
#[display("PickItem(order={:?} sku={:?} station={:?})", line.order, line.sku, station.id)]
pub struct PickItem {
    pub line: PickLine,
    pub station: Arc<StationState>,
    pub relay: RelayService,
    pub navigation: Arc<Navigation>,
    pub bots: BotService,
    pub inventory: InventoryService,
    pub available: AvailableHtmRegistry,
    pub parking: Option<ParkingRegistry>,
    pub events: EventJournal,
    pub station_service: Arc<StationService>,
    pub station_arrivals: StationArrivalDiagnostics,
    pub tripwires: bool,
    pub delivery_max_wait_override: Option<SimDuration>,
    pub after: SimTime,
    pub arrival_floor: SimTime,
    pub bin_hold: BinHold,
    pub completion: OrderCompletionTracker,
    pub boarding: BeltBoarding,
}

#[async_trait(?Send)]
impl Task for PickItem {
    type Proposal = ProposedRetrieve;
    type Schedule = ();

    async fn propose(&self, owner: ProposalId) -> Result<ProposedRetrieve, PlanError> {
        Ok(self
            .relay
            .source_retrieve(
                ToteSearchCriteria::ContainsSku { sku: self.line.sku },
                self.station.presentation_cell,
                self.after,
                self.arrival_floor,
                || self.station.arrival_aim(),
                self.station.version(),
                owner,
            )
            .await?)
    }

    async fn schedule(&mut self, proposal: ProposedRetrieve, ctx: SchedCtx) -> Result<(), PlanError> {
        let quoted_arrival = proposal.selection.completion_time;
        let (reservation, rack_slot) = proposal.into_accepted()?;
        let inflight = self.station.open_pick_span(reservation.start);
        self.station.book_presentation(quoted_arrival);

        let plan = StationArrivalPlan::new(
            self.station.id,
            StationArrivalSource::PickFromRack,
            quoted_arrival,
            quoted_arrival,
        );

        let handoff = ctx
            .add_task(VtmForward::new(
                rack_slot,
                reservation,
                Some(plan),
                self.relay.clone(),
                self.inventory.clone(),
                self.events.clone(),
            ))
            .await;
        let context = ctx.task_context::<HtmPickup>(json!({}))?;
        let pickup = HtmPickup::new(&context, handoff)?;
        context.finish()?;
        let carrying = ctx.add_task(pickup).await;
        let carrying = ctx
            .add_task(HtmDeliver::new(
                self.station.id,
                inflight,
                carrying,
                ctx.barrier().clock(),
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                self.parking.clone(),
                self.events.clone(),
                self.station_service.clone(),
                self.station_arrivals.clone(),
                None,
                self.tripwires,
                self.delivery_max_wait_override,
            ))
            .await?;

        let available = ctx
            .add_task(Presentation {
                station_id: self.station.id,
                op: PresentationOp::Pick {
                    sku: self.line.sku,
                    qty: self.line.qty,
                    order_id: self.line.order,
                    dc_id: self.line.dc.clone(),
                },
                dwell: self.station.pick_dwell(),
                handoff: Some(carrying),
                order_completion: Some(self.completion.clone()),
                bin_hold: Some(self.bin_hold.clone()),
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                available: self.available.clone(),
                events: self.events.clone(),
            })
            .await;
        self.clear_station(available, &ctx)
    }
}

impl PickItem {
    /// Leaves the HTM the pick released to whatever takes it off the
    /// station's cell. A tote the station's belt boards is LoopDeposit's to
    /// take: LoopDeposit boards it once the belt has room (scheduler time),
    /// and a clearance would send it home instead of onto the belt. Any other
    /// gets a clearance.
    fn clear_station(&self, available: AvailableHtm, ctx: &SchedCtx) -> Result<(), PlanError> {
        let boards_belt = match &available.tote {
            Some(carried) => {
                self.boarding.boards(available.cell(), self.inventory.tote(carried.tote_id)?.designation())
            }
            None => false,
        };
        if !boards_belt {
            ctx.add_task(ClearPresentation::new(
                available,
                self.station.id,
                ToteSource::Pick,
                self.navigation.clone(),
                self.available.clone(),
                ctx.barrier().clock(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "pick_item_tests.rs"]
mod tests;
