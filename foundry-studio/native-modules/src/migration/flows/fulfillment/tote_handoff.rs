//! Physical tote load/unload boundaries. These tasks change custody only after
//! the HTM's full dwell while retaining its cell reservation for its next task.

use crate::events::{CycleCompleteEvent, CycleKind, EventJournal, SimEvent, ToteMovedEvent};
use crate::flows::transport::HtmPosition;
use crate::model::bot::LegSpec;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::conveyance::{Admission, CollectionClaim, ConveyedTote, RetainedHtm};
use crate::model::conveyor::ConveyorId;
use crate::observer::time_slice::ToteVisualState;
use crate::prelude::*;

#[derive(Display)]
#[display("UnloadTote({tote:?})")]
pub struct UnloadTote {
    pub position: Option<HtmPosition>,
    pub tote: ToteId,
    pub conveyor: ConveyorId,
    pub admission: Option<Admission>,
    pub dwell: SimDuration,
    pub inventory: InventoryService,
    pub bots: BotService,
    pub events: EventJournal,
}

#[async_trait(?Send)]
impl Task for UnloadTote {
    type Proposal = EmptyProposal;
    type Schedule = (HtmPosition, ConveyedTote, RetainedHtm, SimTime);
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let mut position = self.position.take().ok_or_else(|| super::invalid("UnloadTote scheduled twice"))?;
        let mut admission = self.admission.take().ok_or_else(|| super::invalid("UnloadTote has no admission"))?;
        let start = position.action_ready_at.max(admission.at()).max(ctx.barrier().clock().value());
        let at = start + self.dwell;
        let robot = position.cell_reservation.robot();
        admission.validate(at).map_err(|error| super::invalid(error.to_string()))?;
        if admission.tote() != self.tote || admission.robot() != robot || admission.conveyor_id() != self.conveyor {
            return Err(super::invalid("UnloadTote identity differs from its conveyor admission"));
        }
        validate_position(&position, &self.bots, Some(self.tote), start, at)?;
        let tote = self.inventory.tote(self.tote)?;
        if !tote.location_is_through(start, at, ToteLocation::WithRobot { robot_id: robot }) {
            return Err(super::invalid("UnloadTote inventory custody differs from its delivering HTM"));
        }
        self.bots.commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell: position.cell_reservation.cell() },
                requests: &[],
                action_ready_arrival: at,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::with_tote(self.tote),
                carrying_after: None,
            }],
            &self.inventory,
            None,
        )?;
        let (conveyed, retained) = admission.board(at).map_err(|error| super::invalid(error.to_string()))?;
        tote.move_to(ToteLocation::OnConveyance { conveyor_id: admission.conveyor_id() }, at)?;
        self.events.emit(ToteMovedEvent {
            tote_id: self.tote,
            time: at,
            new_state: ToteVisualState::OnConveyor { cell_id: position.cell_reservation.cell().0 },
        });
        position.action_ready_at = at;
        Ok((position, conveyed, retained, at))
    }
}

#[derive(Display)]
#[display("LoadTote({tote:?})")]
pub struct LoadTote {
    pub position: Option<HtmPosition>,
    pub tote: ToteId,
    pub claim: Option<CollectionClaim>,
    pub dwell: SimDuration,
    pub inventory: InventoryService,
    pub bots: BotService,
    pub events: EventJournal,
}

#[async_trait(?Send)]
impl Task for LoadTote {
    type Proposal = EmptyProposal;
    type Schedule = HtmPosition;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<HtmPosition, PlanError> {
        let mut position = self.position.take().ok_or_else(|| super::invalid("LoadTote scheduled twice"))?;
        let mut claim = self.claim.take().ok_or_else(|| super::invalid("LoadTote has no collection claim"))?;
        let start = position.action_ready_at.max(claim.at()).max(ctx.barrier().clock().value());
        let at = start + self.dwell;
        let robot = position.cell_reservation.robot();
        claim.validate(at).map_err(|error| super::invalid(error.to_string()))?;
        if claim.tote() != self.tote || claim.robot() != robot {
            return Err(super::invalid("LoadTote identity differs from its conveyor collection claim"));
        }
        validate_position(&position, &self.bots, None, start, at)?;
        let tote = self.inventory.tote(self.tote)?;
        if !tote.location_is_through(start, at, ToteLocation::OnConveyance { conveyor_id: claim.conveyor_id() }) {
            return Err(super::invalid("LoadTote inventory custody differs from its collection conveyor"));
        }
        self.bots.commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell: position.cell_reservation.cell() },
                requests: &[],
                action_ready_arrival: at,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::empty(),
                carrying_after: Some(self.tote),
            }],
            &self.inventory,
            None,
        )?;
        claim.commit(at).map_err(|error| super::invalid(error.to_string()))?;
        tote.move_to(ToteLocation::WithRobot { robot_id: robot }, at)?;
        self.events.emit(ToteMovedEvent {
            tote_id: self.tote,
            time: at,
            new_state: ToteVisualState::OnBot { bot_id: robot.0 },
        });
        position.action_ready_at = at;
        Ok(position)
    }
}

pub(super) fn validate_position(
    position: &HtmPosition,
    bots: &BotService,
    carrying: Option<ToteId>,
    start: SimTime,
    end: SimTime,
) -> Result<(), PlanError> {
    let hold = &position.cell_reservation;
    if bots.bot_kind(hold.robot()) != Some(BotKind::Htm)
        || !hold.window().contains(start)
        || !hold.window().contains(end)
    {
        return Err(super::invalid("tote handoff requires an HTM cell reservation through dwell completion"));
    }
    let mut ready = false;
    let mut observed = None;
    bots.for_each(BotKind::Htm, |robot, plan| {
        if robot == hold.robot() {
            ready = plan.carrying_tote() == carrying
                && plan.availability().is_some_and(|(at, cell)| at <= position.action_ready_at && cell == hold.cell())
                && plan.position_at(start).is_some_and(|wp| wp.cell == hold.cell())
                && plan.position_at(end).is_some_and(|wp| wp.cell == hold.cell())
                && plan.iter().filter(|wp| wp.enter > start && wp.enter <= end).all(|wp| wp.cell == hold.cell());
            if !ready {
                observed = Some(format!(
                    "availability={:?}, carrying={:?}, start_cell={:?}, end_cell={:?}",
                    plan.availability(),
                    plan.carrying_tote(),
                    plan.position_at(start).map(|wp| wp.cell),
                    plan.position_at(end).map(|wp| wp.cell)
                ));
            }
        }
    });
    if !ready {
        return Err(super::invalid(format!(
            "tote handoff disagrees with the HTM's current position or load: robot={:?} cell={:?} ready={} expected_cargo={carrying:?}; {observed:?}",
            hold.robot(),
            hold.cell(),
            position.action_ready_at
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tote_handoff_tests.rs"]
mod tests;

use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::model::ToteType;
use desgraph::{RobotType, StationId};

pub enum ExtractedOrder {
    Reusable(HtmCarryingTote),
    Removed(HtmPosition),
}

#[derive(Display)]
#[display("ExtractOrder({order_id:?})")]
pub struct ExtractOrder {
    pub order: Option<HtmCarryingTote>,
    pub order_id: OrderId,
    /// None drains at the area exit for the extraction-isolation diagnostic.
    pub station: Option<StationId>,
    pub dwell: SimDuration,
    pub inventory: InventoryService,
    pub bots: BotService,
    pub events: EventJournal,
}

#[async_trait(?Send)]
impl Task for ExtractOrder {
    type Proposal = EmptyProposal;
    type Schedule = ExtractedOrder;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<ExtractedOrder, PlanError> {
        let mut order = self.order.take().ok_or_else(|| super::invalid("ExtractOrder scheduled twice"))?;
        let start = order.action_ready_at.max(ctx.barrier().clock().value());
        let end = start + self.dwell;
        let robot = order.cell_reservation.robot();
        let tote = self.inventory.tote(order.tote_id)?;
        let whole_container = tote.tote_type() == ToteType::Carton;
        let position = HtmPosition {
            cell_reservation: order.cell_reservation.clone(),
            action_ready_at: order.action_ready_at,
            orientation: order.orientation,
        };
        validate_position(&position, &self.bots, Some(order.tote_id), start, end)?;
        if !tote.location_is_through(start, end, ToteLocation::WithRobot { robot_id: robot }) {
            return Err(super::invalid("extraction has no presented order tote"));
        }
        let contents = tote.contents_at(start);
        let qty = contents.iter().map(|(_, qty)| *qty).sum();
        self.bots.commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell: order.cell_reservation.cell() },
                requests: &[],
                action_ready_arrival: end,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::with_tote(order.tote_id),
                carrying_after: (!whole_container).then_some(order.tote_id),
            }],
            &self.inventory,
            None,
        )?;
        for (sku, qty) in contents {
            tote.remove_item(sku, qty, end.as_ms())?;
        }
        if let Some(station) = self.station {
            self.events.emit(SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: robot,
                robot_type: RobotType::Htm,
                kind: CycleKind::OrderExtract { qty },
                station_id: station,
                sim_time_ms: start.as_ms(),
                dwell_ms: self.dwell.as_ms(),
                arrival_source: None,
                dc_id: None,
                presentation_id: Some(order.presentation_id),
                tote_presentation: None,
            }));
        }
        if whole_container {
            tote.move_to(ToteLocation::Extracted, end)?;
            self.events.emit(ToteMovedEvent {
                tote_id: order.tote_id,
                time: end,
                new_state: ToteVisualState::Extracted,
            });
        }
        order.action_ready_at = end;
        if whole_container {
            Ok(ExtractedOrder::Removed(HtmPosition {
                cell_reservation: order.cell_reservation,
                action_ready_at: end,
                orientation: order.orientation,
            }))
        } else {
            Ok(ExtractedOrder::Reusable(order))
        }
    }
}
