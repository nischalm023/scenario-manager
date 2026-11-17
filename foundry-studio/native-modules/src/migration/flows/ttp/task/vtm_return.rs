//! Return a handed-off tote from relay to rack using the VTM service.

use crate::events::EventJournal;
use crate::events::{ToteSource, VtmEndpoint};
use crate::flows::ttp::handoff::ToteAtRelay;
use crate::model::VtmCycleId;
use crate::model::ids::RackSlot;
use crate::model::inventory::InventoryService;
use crate::model::relay::vtm::{ProposedVtmCycle, VtmAssignment, VtmService};
use crate::model::{ToteDesignation, ToteLocation};
use crate::prelude::*;
use desgraph::{CellId, RelayId, ZoneId};

use super::vtm_leg_boundaries;

#[derive(Display)]
#[display("VtmReturn(zone={:?})", zone_id)]
pub struct VtmReturn {
    zone_id: ZoneId,
    handoff: Option<ToteAtRelay>,
    vtms: VtmService,
    inventory: InventoryService,
    events: EventJournal,
}

impl VtmReturn {
    pub fn new(handoff: ToteAtRelay, vtms: VtmService, inventory: InventoryService, events: EventJournal) -> Self {
        Self { zone_id: handoff.reservation.zone_id, handoff: Some(handoff), vtms, inventory, events }
    }
}

impl Drop for VtmReturn {
    fn drop(&mut self) {
        if let Some(handoff) = self.handoff.take() {
            handoff.reservation.cancel();
        }
    }
}

#[async_trait(?Send)]
impl Task for VtmReturn {
    type Proposal = ProposedVtmCycle;
    type Schedule = SimTime;

    // Selection only — the candidacy holds the chosen VTM's frontier
    // generation as its premise, and spends nothing. A propose error
    // drops the task, whose Drop cancels the handoff's relay hold. A zone
    // with no VTMs can never produce one — zone membership is static —
    // so an empty selection is infeasibility, not "not yet".
    async fn propose(&self, _owner: ProposalId) -> Result<ProposedVtmCycle, PlanError> {
        let handoff = self.handoff.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "VtmReturn",
            detail: "proposed after its handoff was consumed".into(),
        })?;
        let vtm_service = &self.vtms;
        let assignment = vtm_service
            .select_for_return(self.zone_id, handoff.relay_id, handoff.rack_slot, handoff.deposit_time)
            .ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "VtmReturn",
                detail: format!("no VTM in zone {:?} for return to {:?}", self.zone_id, handoff.rack_slot),
            })?;
        let rack_cell =
            self.vtms.rack_cell(handoff.rack_slot.rack_id).ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "VtmReturn",
                detail: format!("no cell for rack {:?}", handoff.rack_slot.rack_id),
            })?;
        let relay_vtm_cell = self.vtms.relay_cell(handoff.relay_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "VtmReturn",
            detail: format!("no VTM access cell for relay {:?}", handoff.relay_id),
        })?;
        let relay_endpoint =
            VtmEndpoint::Relay { relay_id: handoff.relay_id, cell: relay_vtm_cell, pos: assignment.relay_pos };
        let slot_endpoint =
            VtmEndpoint::Slot { rack_id: handoff.rack_slot.rack_id, cell: rack_cell, pos: assignment.slot_pos };
        // A return ends at the rack slot.
        let complete_endpoint = slot_endpoint.clone();
        Ok(ProposedVtmCycle::new(vtm_service.clone(), assignment, slot_endpoint, relay_endpoint, complete_endpoint))
    }

    // The cycle committed at acceptance; schedule spends the chain state
    // around the accepted assignment.
    async fn schedule(&mut self, proposal: ProposedVtmCycle, _ctx: SchedCtx) -> Result<SimTime, PlanError> {
        let handoff = self.handoff.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "VtmReturn",
            detail: "scheduled after its handoff was consumed".into(),
        })?;
        let (tote_id, relay_id, rack_slot, deposit_time, presentation_id, tote_source) = (
            handoff.tote_id,
            handoff.relay_id,
            handoff.rack_slot,
            handoff.deposit_time,
            handoff.presentation_id,
            handoff.tote_source,
        );
        let assignment = &proposal.assignment;
        let vtm_start = assignment.vtm_start;
        let vtm_complete = assignment.vtm_arrival;
        let rack_cell = self.vtms.rack_cell(rack_slot.rack_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "VtmReturn",
            detail: format!("no cell for rack {:?}", rack_slot.rack_id),
        })?;

        // The outbound relay hold is the edge from HtmStore to VtmReturn;
        // it closes when the tote leaves the relay buffer.
        let t_relay_pickup_done = vtm_start
            + SimDuration::from_secs(
                assignment.cycle.first_travel_secs
                    + assignment.cycle.relay_alignment_secs
                    + assignment.cycle.relay_transfer_secs,
            );
        // Match the operation stream's per-phase rounding without changing
        // the relay reservation's existing end-time calculation above.
        let pickup_at = vtm_start
            + SimDuration::from_secs(assignment.cycle.first_travel_secs)
            + SimDuration::from_secs(assignment.cycle.relay_alignment_secs)
            + SimDuration::from_secs(assignment.cycle.relay_transfer_secs);
        let tote = self.inventory.tote(tote_id)?;
        tote.move_to(ToteLocation::WithRobot { robot_id: assignment.vtm_id }, pickup_at)?;
        tote.move_to(ToteLocation::InRack { rack_id: rack_slot.rack_id, z_index: rack_slot.level }, vtm_complete)?;
        // Publish occupied custody before releasing the destination claim.
        // The relay itself still becomes available at its earlier pickup time.
        handoff.reservation.finalize(t_relay_pickup_done).map_err(|e| PlanError::TaskGraphInvariant {
            context: "VtmReturn",
            detail: format!("relay finalize failed: {:?}", e),
        })?;
        // A picked inventory tote's residual stock was never removed from the
        // ledger, and order contents are already picked. Only inbound stock is new.
        if tote_source == ToteSource::Putaway && tote.designation() == ToteDesignation::Inventory {
            for (sku, qty) in tote.contents_at(vtm_complete) {
                for _ in 0..qty {
                    self.inventory.record_inventory_added(sku, vtm_complete);
                }
            }
        }

        let vtm_cycle_id = self.events.next_vtm_cycle_id();

        self.emit_cycle(
            assignment,
            &ToteReturn { tote_id, relay_id, rack_slot, rack_cell, deposit_time, presentation_id, tote_source },
            proposal.relay_endpoint.clone(),
            proposal.slot_endpoint.clone(),
            vtm_cycle_id,
        )?;

        Ok(vtm_complete)
    }
}

// The handoff scalars a return cycle's event stream narrates, copied out
// before the reservation is consumed.
struct ToteReturn {
    tote_id: ToteId,
    relay_id: RelayId,
    rack_slot: RackSlot,
    rack_cell: CellId,
    deposit_time: SimTime,
    presentation_id: Option<PresentationId>,
    tote_source: ToteSource,
}

impl VtmReturn {
    // The return cycle's full event stream: idle-reason attribution, the
    // six nav legs (geometric boundaries where the padded cycle diverges),
    // the fork operations, tote visuals, and the per-cycle KPI row.
    // Observation only — it changes no model state.
    fn emit_cycle(
        &self,
        assignment: &VtmAssignment,
        ret: &ToteReturn,
        relay_endpoint: VtmEndpoint,
        slot_endpoint: VtmEndpoint,
        vtm_cycle_id: VtmCycleId,
    ) -> Result<(), PlanError> {
        use crate::events::*;
        use crate::observer::time_slice::ToteVisualState;

        let &ToteReturn { tote_id, relay_id, rack_slot, rack_cell, deposit_time, presentation_id, tote_source } = ret;
        let vtm_id = assignment.vtm_id;
        let cycle = &assignment.cycle;
        let geo_cycle = &assignment.geo_cycle;
        let cycle_duration = assignment.cycle_duration;
        let vtm_start = assignment.vtm_start;
        let vtm_complete = assignment.vtm_arrival;
        let from_endpoint = assignment.from_endpoint.clone();
        let rack_pos = assignment.slot_pos;
        let relay_pos = assignment.relay_pos;

        let t_at_relay = vtm_start + SimDuration::from_secs(cycle.first_travel_secs);
        let t_relay_aligned = t_at_relay + SimDuration::from_secs(cycle.relay_alignment_secs);
        let t_relay_pickup_done = t_relay_aligned + SimDuration::from_secs(cycle.relay_transfer_secs);
        let t_at_rack = t_relay_pickup_done + SimDuration::from_secs(cycle.second_travel_secs);
        let t_rack_aligned = t_at_rack + SimDuration::from_secs(cycle.rack_alignment_secs);

        let geo_diverged = geo_cycle != cycle;
        let geo = if geo_diverged {
            vtm_leg_boundaries(
                vtm_start,
                [
                    geo_cycle.first_travel_secs,
                    geo_cycle.relay_alignment_secs,
                    geo_cycle.relay_transfer_secs,
                    geo_cycle.second_travel_secs,
                    geo_cycle.rack_alignment_secs,
                    geo_cycle.rack_transfer_secs,
                ],
            )
        } else {
            [vtm_start, t_at_relay, t_relay_aligned, t_relay_pickup_done, t_at_rack, t_rack_aligned, vtm_complete]
        };
        let [
            geo_start,
            geo_at_relay,
            geo_relay_aligned,
            geo_relay_pickup_done,
            geo_at_rack,
            geo_rack_aligned,
            geo_complete,
        ] = geo;

        let payload = TotePayloadState::from_empty(self.inventory.tote(tote_id)?.is_empty_at(t_relay_pickup_done));

        let current_pos = from_endpoint.pos();
        let approach_dist_mm =
            ((relay_pos.x_mm - current_pos.x_mm).powi(2) + (relay_pos.y_mm - current_pos.y_mm).powi(2)).sqrt();
        let loaded_dist_mm =
            ((rack_pos.x_mm - relay_pos.x_mm).powi(2) + (rack_pos.y_mm - relay_pos.y_mm).powi(2)).sqrt();

        debug_assert!(
            vtm_start >= assignment.available_at,
            "vtm_start {vtm_start:?} < available_at {:?} — dispatcher invariant violated",
            assignment.available_at,
        );
        let reason = if vtm_start == assignment.available_at {
            VtmIdleReason::Continuous
        } else {
            VtmIdleReason::AwaitingHtmDeposit { gap: vtm_start - assignment.available_at, deposit_time }
        };
        let chain_id = presentation_id;
        let direction = VtmCycleDirection::Return;
        self.events.emit(SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: vtm_id,
            vtm_cycle_id,
            at: vtm_start,
            reason,
            presentation_id: chain_id,
            direction,
            relay_id,
            geo_diverged,
        }));

        let nav_event = |kind, geo_start, geo_end| {
            self.events.emit(SimEvent::VtmNav(VtmNavEventData {
                robot_id: vtm_id,
                kind,
                geo_start,
                geo_end,
                vtm_cycle_id,
                presentation_id: chain_id,
                direction,
            }));
        };
        nav_event(
            VtmNavKind::RailMove {
                from: from_endpoint,
                to: relay_endpoint.clone(),
                tote_id: None,
                start: vtm_start,
                end: t_at_relay,
                distance_mm: approach_dist_mm as u64,
            },
            geo_start,
            geo_at_relay,
        );
        nav_event(
            VtmNavKind::Alignment {
                at: relay_endpoint.clone(),
                tote_id: None,
                start: t_at_relay,
                end: t_relay_aligned,
            },
            geo_at_relay,
            geo_relay_aligned,
        );
        nav_event(
            VtmNavKind::Fork {
                op: ForkOp::Get,
                at: relay_endpoint.clone(),
                tote_id,
                start: t_relay_aligned,
                end: t_relay_pickup_done,
            },
            geo_relay_aligned,
            geo_relay_pickup_done,
        );
        nav_event(
            VtmNavKind::RailMove {
                from: relay_endpoint,
                to: slot_endpoint.clone(),
                tote_id: Some(tote_id),
                start: t_relay_pickup_done,
                end: t_at_rack,
                distance_mm: loaded_dist_mm as u64,
            },
            geo_relay_pickup_done,
            geo_at_rack,
        );
        nav_event(
            VtmNavKind::Alignment {
                at: slot_endpoint.clone(),
                tote_id: Some(tote_id),
                start: t_at_rack,
                end: t_rack_aligned,
            },
            geo_at_rack,
            geo_rack_aligned,
        );
        nav_event(
            VtmNavKind::Fork { op: ForkOp::Put, at: slot_endpoint, tote_id, start: t_rack_aligned, end: vtm_complete },
            geo_rack_aligned,
            geo_complete,
        );

        self.events.emit(SimEvent::VtmOperation(VtmOperationEvent {
            sim_time_ms: t_relay_pickup_done.as_ms(),
            vtm_id,
            vtm_cycle_id,
            zone_id: self.zone_id,
            kind: VtmOperationKind::PickupRelay,
            rack_id: rack_slot.rack_id,
            rack_level: rack_slot.level,
            relay_id,
            tote_id,
            payload,
            presentation_id: chain_id,
        }));
        self.events.emit(SimEvent::VtmOperation(VtmOperationEvent {
            sim_time_ms: vtm_complete.as_ms(),
            vtm_id,
            vtm_cycle_id,
            zone_id: self.zone_id,
            kind: VtmOperationKind::DropStorage,
            rack_id: rack_slot.rack_id,
            rack_level: rack_slot.level,
            relay_id,
            tote_id,
            payload,
            presentation_id: chain_id,
        }));

        self.events.emit(ToteMovedEvent {
            tote_id,
            time: t_relay_pickup_done,
            new_state: ToteVisualState::OnBot { bot_id: vtm_id.0 },
        });
        self.events.emit(ToteMovedEvent {
            tote_id,
            time: vtm_complete,
            new_state: ToteVisualState::InRack { cell_id: rack_cell.0, level: rack_slot.level },
        });
        self.events.emit(SimEvent::Resource(ResourceEvent {
            kind: ResourceEventKind::VtmReturnCompleted { cycle_ms: cycle_duration.as_ms() },
            sim_time_ms: vtm_complete.as_ms(),
        }));

        {
            let empty_ms = (geo_at_relay - geo_start).as_ms();
            let relay_alignment_ms = (geo_relay_aligned - geo_at_relay).as_ms();
            let relay_transfer_ms = (geo_relay_pickup_done - geo_relay_aligned).as_ms();
            let loaded_ms = (geo_at_rack - geo_relay_pickup_done).as_ms();
            let rack_alignment_ms = (geo_rack_aligned - geo_at_rack).as_ms();
            let fork_ms = (geo_complete - geo_rack_aligned).as_ms();
            let buffer_ms = relay_alignment_ms + relay_transfer_ms;
            self.events.emit(SimEvent::VtmCycle(VtmCycleEvent {
                vtm_id,
                vtm_cycle_id,
                zone_id: self.zone_id,
                direction: VtmCycleDirection::Return,
                empty_leg_ms: empty_ms,
                loaded_leg_ms: loaded_ms,
                fork_ms,
                buffer_ms,
                rack_alignment_ms,
                relay_alignment_ms,
                relay_transfer_ms,
                total_cycle_ms: empty_ms + loaded_ms + rack_alignment_ms + fork_ms + buffer_ms,
                sim_time_ms: vtm_start.as_ms(),
                relay_id,
                rack_id: rack_slot.rack_id,
                tote_id,
                presentation_id: chain_id,
                pre_cycle_wait_ms: (vtm_start - assignment.available_at).as_ms(),
                wait_reason: reason.kind(),
                tote_source,
            }));
        }
        Ok(())
    }
}
