//! Transfer stock between two physically presented totes during one operator
//! interval. The order tote stays on its reserved HTM; conveyor custody and
//! both presentation holds remain with the parent after the pick.

use std::fmt;

use desgraph::CellId;
use desgraph::types::RobotType;

use crate::events::{CycleCompleteEvent, CycleKind, EventJournal, OrderCompleteEvent, SimEvent, TotePresentation};
use crate::flows::ttp::demand::order_completion::OrderCompletionTracker;
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotKind, BotService, LegSpec};
use crate::model::conveyance::{Conveyance, InventoryPresentation, PairedTransfer};
use crate::model::inventory::InventoryService;
use crate::model::inventory::error::InventoryError;
use crate::model::{ToteDesignation, ToteLocation};
use crate::prelude::*;
use crate::scenario_gen::demand_pool::DistributionCenterId;

pub struct TransferItems {
    order: Option<HtmCarryingTote>,
    presented: InventoryPresentation,
    conveyance: Conveyance,
    order_cell: CellId,
    source: TotePresentation,
    sku: SkuId,
    qty: u32,
    dwell: SimDuration,
    inventory: InventoryService,
    bots: BotService,
    events: EventJournal,
    completion: OrderCompletionTracker,
    dc_id: Option<DistributionCenterId>,
}

impl TransferItems {
    pub fn new(
        order: HtmCarryingTote,
        presented: InventoryPresentation,
        conveyance: Conveyance,
        order_cell: CellId,
        source: TotePresentation,
        sku: SkuId,
        qty: u32,
        dwell: SimDuration,
        inventory: InventoryService,
        bots: BotService,
        events: EventJournal,
        completion: OrderCompletionTracker,
        dc_id: Option<DistributionCenterId>,
    ) -> Result<Self, PlanError> {
        if qty == 0 || qty > i32::MAX as u32 || dwell == SimDuration::ZERO {
            return Err(invalid("quantity and dwell must be positive and quantity must fit the stock event"));
        }
        if source.tote_id != presented.tote() {
            return Err(invalid("inventory trip metadata names a different tote"));
        }
        Ok(Self {
            order: Some(order),
            presented,
            conveyance,
            order_cell,
            source,
            sku,
            qty,
            dwell,
            inventory,
            bots,
            events,
            completion,
            dc_id,
        })
    }

    fn validate_transfer(&self, order: &HtmCarryingTote, pair: &PairedTransfer) -> Result<(), PlanError> {
        let hold = &order.cell_reservation;
        if hold.cell() != self.order_cell || !hold.window().contains(pair.start) || !hold.window().contains(pair.end) {
            return Err(invalid("order HTM does not hold the selected presentation cell through pick completion"));
        }
        if self.bots.bot_kind(hold.robot()) != Some(BotKind::Htm) {
            return Err(invalid("order tote is not carried by a registered HTM"));
        }
        let mut ready = false;
        self.bots.for_each(BotKind::Htm, |robot, plan| {
            if robot == hold.robot() {
                // The handed-off hold is the robot's current plan frontier,
                // not an old cell reservation preceding already assigned work.
                ready = plan.carrying_tote() == Some(order.tote_id)
                    && plan
                        .availability()
                        .is_some_and(|(at, cell)| at <= order.action_ready_at && cell == self.order_cell)
                    && plan.position_at(pair.start).is_some_and(|wp| wp.cell == self.order_cell)
                    && plan.position_at(pair.end).is_some_and(|wp| wp.cell == self.order_cell)
                    && plan
                        .iter()
                        .filter(|wp| wp.enter > pair.start && wp.enter <= pair.end)
                        .all(|wp| wp.cell == self.order_cell);
            }
        });
        if !ready {
            return Err(invalid("order HTM plan does not show continuous presentation with its tote"));
        }
        let order_tote = self.inventory.tote(order.tote_id)?;
        let stock_tote = self.inventory.tote(self.presented.tote())?;
        if order_tote.designation() != ToteDesignation::Order || stock_tote.designation() != ToteDesignation::Inventory
        {
            return Err(invalid("item transfer requires inventory and order tote designations"));
        }
        if !order_tote.location_is_through(pair.start, pair.end, ToteLocation::WithRobot { robot_id: hold.robot() }) {
            return Err(invalid("order tote leaves its HTM during the pick"));
        }
        if !stock_tote.location_is_through(
            pair.start,
            pair.end,
            ToteLocation::OnConveyance { conveyor_id: self.conveyance.id() },
        ) {
            return Err(invalid("inventory tote is not in this conveyor's custody throughout the pick"));
        }
        if stock_tote.sku_qty_at(self.sku, pair.start) < self.qty
            || stock_tote.sku_qty_at(self.sku, pair.end) < self.qty
        {
            return Err(InventoryError::InsufficientQuantity.into());
        }
        if order_tote.sku_qty_at(self.sku, pair.end).checked_add(self.qty).is_none() {
            return Err(invalid("order tote quantity would overflow"));
        }
        if self.inventory.sku_inventory_at(self.sku, pair.end) < self.qty as isize
            || self.inventory.sku_reserved_at(self.sku, pair.end) < self.qty as isize
        {
            return Err(invalid("picked quantity is not present and reserved in the stock ledger"));
        }
        self.completion
            .validate_pick(self.source.order_id, self.qty)
            .map_err(|order| invalid(format!("order {order:?} is missing or would be over-picked")))?;
        Ok(())
    }
}

impl fmt::Display for TransferItems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransferItems(order={:?} station={:?})", self.source.order_id, self.presented.station())
    }
}

#[async_trait(?Send)]
impl Task for TransferItems {
    type Proposal = EmptyProposal;
    type Schedule = (HtmCarryingTote, PairedTransfer);

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let mut order = self.order.take().ok_or_else(|| invalid("scheduled twice"))?;
        let robot = order.cell_reservation.robot();
        let mut claim = self
            .conveyance
            .try_pair(&self.presented, order.tote_id, robot, order.action_ready_at, ctx.barrier().floor(), self.dwell)
            .map_err(|error| invalid(error.to_string()))?;
        let pair = claim.interval();
        self.validate_transfer(&order, &pair)?;
        self.bots.commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell: self.order_cell },
                requests: &[],
                action_ready_arrival: pair.end,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::with_tote(order.tote_id),
                carrying_after: Some(order.tote_id),
            }],
            &self.inventory,
            None,
        )?;
        self.inventory.tote(pair.inventory_tote)?.remove_item(self.sku, self.qty, pair.end.as_ms())?;
        self.inventory.tote(pair.order_tote)?.add_item(self.sku, self.qty, pair.end.as_ms())?;
        for _ in 0..self.qty {
            self.inventory.record_pick_consumed(self.sku, pair.end);
        }
        let completed = self
            .completion
            .record_pick(self.source.order_id, self.qty, pair.end.as_ms())
            .map_err(|order| invalid(format!("order {order:?} could not record its validated pick")))?;
        claim.commit().map_err(|error| invalid(error.to_string()))?;
        self.events.emit(SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: robot,
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: self.sku, qty: self.qty },
            station_id: pair.station,
            sim_time_ms: pair.start.as_ms(),
            dwell_ms: self.dwell.as_ms(),
            arrival_source: None,
            dc_id: self.dc_id.clone(),
            presentation_id: Some(order.presentation_id),
            tote_presentation: Some(self.source),
        }));
        if let Some(completed) = completed {
            self.events.emit(SimEvent::OrderComplete(OrderCompleteEvent {
                order_id: self.source.order_id,
                sim_time_ms: completed,
            }));
        }
        order.action_ready_at = pair.end;
        Ok((order, pair))
    }
}

fn invalid(detail: impl Into<String>) -> PlanError {
    PlanError::TaskGraphInvariant { context: "TransferItems", detail: detail.into() }
}

#[cfg(test)]
#[path = "transfer_items_tests.rs"]
mod tests;
