//! Perform a station operation on a presented tote, then release its HTM:
//! the stay it is released into is returned, so the operation's owner can
//! plan what clears the station (`ClearPresentation`) while the barrier is
//! still short of the operation's end.

use std::fmt;

use crate::events::{EventJournal, OrderCompleteEvent, SimEvent};
use crate::flows::ttp::demand::order_completion::OrderCompletionTracker;
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::flows::ttp::inflight::BinHold;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotService, LegSpec};
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry};
use crate::model::ids::OrderId;
use crate::model::inventory::InventoryService;
use crate::prelude::*;
use crate::scenario_gen::demand_pool::DistributionCenterId;
use desgraph::StationId;
use desgraph::types::RobotType;

pub enum PresentationOp {
    Pick {
        sku: SkuId,
        qty: u32,
        order_id: OrderId,
        dc_id: Option<DistributionCenterId>,
    },

    Putaway,

    /// An empty order tote takes on a completed order's picked units at the
    /// station that picked it. Presence-only: the units left inventory at
    /// their own picks and an order tote holds no modelled inventory, so
    /// `qty` is for the event only.
    OrderLoad {
        order_id: OrderId,
        qty: u32,
    },
    /// A full order tote is emptied at an extraction station and its contents
    /// leave the building. Presence-only, for the same reason.
    OrderExtract {
        order_id: OrderId,
        qty: u32,
    },
}

pub struct Presentation {
    pub station_id: StationId,
    pub op: PresentationOp,
    pub dwell: SimDuration,
    pub handoff: Option<HtmCarryingTote>,
    pub order_completion: Option<OrderCompletionTracker>,
    pub bin_hold: Option<BinHold>,
    pub inventory: InventoryService,
    pub bots: BotService,
    pub available: AvailableHtmRegistry,
    pub events: EventJournal,
}

impl fmt::Display for Presentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match &self.op {
            PresentationOp::Pick { .. } => "pick",
            PresentationOp::Putaway => "putaway",
            PresentationOp::OrderLoad { .. } => "order-load",
            PresentationOp::OrderExtract { .. } => "order-extract",
        };
        write!(f, "Presentation({} stn={:?})", op, self.station_id)
    }
}

#[async_trait(?Send)]
impl Task for Presentation {
    type Proposal = EmptyProposal;
    type Schedule = AvailableHtm;

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, _ctx: SchedCtx) -> Result<AvailableHtm, PlanError> {
        let upstream = self.handoff.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "Presentation",
            detail: "scheduled twice".into(),
        })?;
        let tote_id = upstream.tote_id;
        let arrival_source = upstream.station_arrival_plan.map(|plan| plan.source);
        let presentation_id = upstream.presentation_id;
        let source_zone_id = upstream.zone_id;
        let source_relay_id = upstream.source_relay_id;
        let cell_reservation = upstream.cell_reservation;
        let htm_id = cell_reservation.robot();

        let arrival = upstream.action_ready_at;
        let presentation_time = match &self.bin_hold {
            Some(hold) if hold.bin_free() > arrival => {
                crate::trace_event!(
                    reject,
                    pick_bin_wait,
                    station = self.station_id,
                    arrival = arrival,
                    bin_free = hold.bin_free(),
                );
                hold.bin_free()
            }
            _ => arrival,
        };
        let presentation_done = presentation_time + self.dwell;
        if !matches!(self.op, PresentationOp::Putaway) {
            self.bots.commit_task(
                htm_id,
                TaskKind::Deliver,
                &[LegSpec {
                    leg: Leg::StationWork { cell: cell_reservation.cell() },
                    requests: &[],
                    action_ready_arrival: presentation_done,
                    wp_kind: WaypointKind::Present,
                    expect: BotExpectation::with_tote(tote_id),
                    carrying_after: Some(tote_id),
                }],
                &self.inventory,
                None,
            )?;
        }

        let mut order_completed: Option<(OrderId, u64)> = None;

        match &self.op {
            PresentationOp::Pick { sku, qty, order_id, .. } => {
                let tote = self.inventory.tote(tote_id)?;
                tote.remove_item(*sku, *qty, presentation_time.as_ms())?;
                self.inventory.record_pick_consumed(*sku, presentation_time);

                if let Some(hold) = &self.bin_hold {
                    hold.record_pick_done(presentation_done);
                }
                if let Some(tracker) = &self.order_completion {
                    order_completed = tracker
                        .record_pick(*order_id, *qty, presentation_time.as_ms())
                        .map_err(|id| PlanError::TaskGraphInvariant {
                            context: "Presentation",
                            detail: format!(
                                "pick for order {id:?} not registered in OrderCompletionTracker \
                                 (register missing at start_picking, or order over-picked)"
                            ),
                        })?
                        .map(|completion_ms| (*order_id, completion_ms));
                }
            }
            PresentationOp::Putaway => {}
            // Presence-only, and for the same reason as `Putaway`: nothing to
            // mutate. Unlike `Putaway` these emit their own CycleComplete
            // below, since no later generator owns the event.
            PresentationOp::OrderLoad { .. } | PresentationOp::OrderExtract { .. } => {}
        }

        if let PresentationOp::Pick { sku, qty, order_id, dc_id } = &self.op {
            use crate::events::*;
            let source_relay_id = source_relay_id.ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "Presentation",
                detail: format!("pick presentation {:?} for tote {:?} has no source relay", presentation_id, tote_id),
            })?;
            self.events.emit(SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: htm_id,
                robot_type: RobotType::Htm,
                kind: CycleKind::Pick { sku: *sku, qty: *qty },
                station_id: self.station_id,
                sim_time_ms: presentation_time.as_ms(),
                dwell_ms: self.dwell.as_ms(),
                arrival_source,
                dc_id: dc_id.clone(),
                presentation_id: Some(presentation_id),
                tote_presentation: Some(TotePresentation {
                    order_id: *order_id,
                    tote_id,
                    source_zone_id,
                    source_relay_id,
                    presentation_id,
                }),
            }));
        }

        // Order-tote presentations need no source relay: the tote is carried
        // station-to-station, so there is none to name.
        if let PresentationOp::OrderLoad { order_id, qty } | PresentationOp::OrderExtract { order_id, qty } = &self.op {
            use crate::events::*;
            let kind = match &self.op {
                PresentationOp::OrderLoad { .. } => CycleKind::OrderLoad { qty: *qty },
                _ => CycleKind::OrderExtract { qty: *qty },
            };
            let _ = order_id;
            self.events.emit(SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: htm_id,
                robot_type: RobotType::Htm,
                kind,
                station_id: self.station_id,
                sim_time_ms: presentation_time.as_ms(),
                dwell_ms: self.dwell.as_ms(),
                arrival_source,
                dc_id: None,
                presentation_id: Some(presentation_id),
                tote_presentation: None,
            }));
        }

        if let Some((order_id, completion_ms)) = order_completed {
            if let Some(hold) = &self.bin_hold {
                hold.close(presentation_done);
            }
            self.events.emit(SimEvent::OrderComplete(OrderCompleteEvent { order_id, sim_time_ms: completion_ms }));
        }

        crate::debug_log!(
            "[Presentation] op={} stn={:?} time={}ms done={}ms register={} cell={:?} htm={:?} open={}",
            match &self.op {
                PresentationOp::Pick { .. } => "pick",
                PresentationOp::Putaway => "putaway",
                PresentationOp::OrderLoad { .. } => "order-load",
                PresentationOp::OrderExtract { .. } => "order-extract",
            },
            self.station_id,
            presentation_time.as_ms(),
            presentation_done.as_ms(),
            cell_reservation.id(),
            cell_reservation.cell(),
            htm_id,
            cell_reservation.is_open_ended()
        );
        {
            let op = match &self.op {
                PresentationOp::Pick { .. } => "pick",
                PresentationOp::Putaway => "putaway",
                PresentationOp::OrderLoad { .. } => "order-load",
                PresentationOp::OrderExtract { .. } => "order-extract",
            };
            crate::trace_event!(
                commit,
                presentation_committed,
                station = self.station_id,
                htm = htm_id,
                op = op,
                presentation_time = presentation_time,
                presentation_done = presentation_done,
            );
        }

        Ok(self.available.register(cell_reservation, presentation_done, upstream.orientation, Some(presentation_id))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_model::{TestModelConfig, build_model, register_loaded_htm};
    use crate::model::{ToteDesignation, ToteLocation, ToteType};
    use desgraph::{CellId, RelayId, ZoneId};
    use tokio::runtime::Builder;

    #[test]
    fn planning_a_pick_returns_future_availability_without_advancing_simtime() {
        let (model, oracle) = build_model(TestModelConfig::default());
        let robot = RobotId(60);
        let tote = ToteId(700);
        model
            .inventory
            .register_tote(
                tote,
                ToteType::Tote,
                ToteDesignation::Inventory,
                ToteLocation::WithRobot { robot_id: robot },
            )
            .unwrap();
        model.inventory.tote(tote).unwrap().add_item(SkuId(1), 1, 0).unwrap();
        model.inventory.seed_inventory_from_totes();
        assert!(model.inventory.reserve_for_order(&[(SkuId(1), 1)], SimTime::ZERO));
        register_loaded_htm(&model, &oracle, robot, CellId(10), tote);
        let source = model.available_htm.take(robot).unwrap();
        let mut scheduler = Scheduler::new();
        let presentation = scheduler.add_task(Presentation {
            station_id: StationId(1),
            op: PresentationOp::Pick { sku: SkuId(1), qty: 1, order_id: OrderId(1), dc_id: None },
            dwell: SimDuration::seconds(7),
            handoff: Some(HtmCarryingTote {
                tote_id: tote,
                zone_id: ZoneId(1),
                cell_reservation: source.cell_reservation,
                action_ready_at: SimTime::seconds(100),
                orientation: source.orientation,
                station_arrival_plan: None,
                presentation_id: PresentationId(1),
                source_relay_id: Some(RelayId(1)),
                deliver_from: None,
            }),
            order_completion: None,
            bin_hold: None,
            inventory: model.inventory.clone(),
            bots: model.bot_service.clone(),
            available: model.available_htm.clone(),
            events: model.events.clone(),
        });
        scheduler.run().unwrap();
        let runtime = Builder::new_current_thread().build().unwrap();
        let available = runtime.block_on(presentation);
        assert_eq!(scheduler.barrier().floor(), SimTime::ZERO);
        assert_eq!(available.action_ready_at, SimTime::seconds(107));
    }
}
