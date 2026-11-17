//! Retrieve a tote from rack to relay using the VTM service.

use crate::events::{EventJournal, ToteSource, VtmEndpoint};
use crate::model::inventory::InventoryService;
use std::fmt;

use crate::error::planner::ErrContext;
use crate::flows::ttp::handoff::RelayHandoff;
use crate::model::ToteLocation;
use crate::model::VtmCycleId;
use crate::model::ids::RackSlot;
use crate::model::relay::vtm::{ProposedVtmCycle, VtmAssignment};
use crate::model::relay::{RelayReservation, RelayService, RelayStartCause};
use crate::model::station::StationArrivalSource;
use crate::prelude::*;
use desgraph::{RelayId, ZoneId};

use super::vtm_leg_boundaries;

pub struct VtmForward {
    zone_id: ZoneId,
    relay_id: RelayId,
    tote_id: ToteId,
    rack_slot: RackSlot,
    relay_start: SimTime,
    reservation: Option<RelayReservation<ToteId>>,
    station_arrival_plan: Option<StationArrivalPlan>,
    source: Option<ToteSource>,
    relay: RelayService,
    inventory: InventoryService,
    events: EventJournal,
}

impl VtmForward {
    pub fn new(
        rack_slot: RackSlot,
        reservation: RelayReservation<ToteId>,
        station_arrival_plan: Option<StationArrivalPlan>,
        relay: RelayService,
        inventory: InventoryService,
        events: EventJournal,
    ) -> Self {
        let source = station_arrival_plan.as_ref().map(|plan| match plan.source {
            StationArrivalSource::PickFromRack => ToteSource::Pick,
            StationArrivalSource::PutawayFromRack | StationArrivalSource::PutawayDirect => ToteSource::Putaway,
        });
        Self {
            zone_id: reservation.zone_id,
            relay_id: reservation.relay_id,
            tote_id: reservation.recommendation,
            rack_slot,
            relay_start: reservation.start,
            reservation: Some(reservation),
            station_arrival_plan,
            source,
            relay,
            inventory,
            events,
        }
    }

    /// Supply flow attribution when the retrieval serves a waiting location
    /// or conveyor handover rather than a station-arrival plan.
    pub fn with_source(mut self, source: ToteSource) -> Self {
        self.source = Some(source);
        self
    }
}

impl Drop for VtmForward {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            reservation.cancel();
        }
    }
}

impl fmt::Display for VtmForward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VtmForward(zone={:?} relay={:?} tote={:?})", self.zone_id, self.relay_id, self.tote_id)
    }
}

#[async_trait(?Send)]
impl Task for VtmForward {
    type Proposal = ProposedVtmCycle;
    type Schedule = RelayHandoff;

    // Selection only — the candidacy holds the chosen VTM's frontier
    // generation as its premise, and spends nothing. A propose error
    // drops the task, whose Drop cancels the held relay reservation.
    async fn propose(&self, _owner: ProposalId) -> Result<ProposedVtmCycle, PlanError> {
        let vtm_service = &self.relay.vtm_service;
        let assignment = vtm_service
            .select_for_forward(self.zone_id, self.relay_id, self.rack_slot, self.relay_start)
            .ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "VtmForward",
                detail: format!("no VTM in zone {:?} for forward from {:?}", self.zone_id, self.rack_slot),
            })?;
        let source_cell =
            self.relay.vtm_service.rack_cell(self.rack_slot.rack_id).ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "VtmForward",
                detail: format!("no cell mapped for rack {:?}", self.rack_slot.rack_id),
            })?;
        let relay_vtm_cell =
            self.relay.vtm_service.relay_cell(self.relay_id).ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "VtmForward",
                detail: format!("no VTM access cell for relay {:?}", self.relay_id),
            })?;
        let slot_endpoint =
            VtmEndpoint::Slot { rack_id: self.rack_slot.rack_id, cell: source_cell, pos: assignment.slot_pos };
        let relay_endpoint =
            VtmEndpoint::Relay { relay_id: self.relay_id, cell: relay_vtm_cell, pos: assignment.relay_pos };
        // A forward ends at the relay.
        let complete_endpoint = relay_endpoint.clone();
        Ok(ProposedVtmCycle::new(vtm_service.clone(), assignment, slot_endpoint, relay_endpoint, complete_endpoint))
    }

    // The cycle committed at acceptance; schedule spends the chain state
    // around the accepted assignment.
    async fn schedule(&mut self, proposal: ProposedVtmCycle, _ctx: SchedCtx) -> Result<RelayHandoff, PlanError> {
        let presentation_id = self.events.next_presentation_id();
        let assignment = &proposal.assignment;
        let vtm_deposit = assignment.vtm_arrival;

        // How much later than the slot start this chain was admitted
        // against the tote actually lands. The selector meets `relay_start`
        // when a VTM can and returns its earliest possible arrival when
        // none can, so this is never negative and is zero on an on-time
        // cycle. An HTM that drove to meet the deposit and found the board
        // empty was reading the far end of this number.
        let quote_slip = vtm_deposit.max(self.relay_start) - self.relay_start;
        self.relay.record_deposit_quote(quote_slip);
        crate::trace_event!(
            metric,
            vtm_deposit_quote_slip,
            relay = self.relay_id,
            vtm = assignment.vtm_id,
            relay_start = self.relay_start,
            deposit = vtm_deposit,
            slip = quote_slip,
        );

        let pickup_at = assignment.vtm_start
            + SimDuration::from_secs(assignment.cycle.first_travel_secs)
            + SimDuration::from_secs(assignment.cycle.rack_alignment_secs)
            + SimDuration::from_secs(assignment.cycle.rack_transfer_secs);
        let tote = self.inventory.tote(self.tote_id)?;
        tote.move_to(ToteLocation::WithRobot { robot_id: assignment.vtm_id }, pickup_at).err_context("VtmForward")?;
        tote.move_to(ToteLocation::AtRelay { relay_id: self.relay_id }, vtm_deposit).err_context("VtmForward")?;

        let vtm_cycle_id = self.events.next_vtm_cycle_id();
        self.emit_cycle(
            assignment,
            proposal.slot_endpoint.clone(),
            proposal.relay_endpoint.clone(),
            vtm_cycle_id,
            presentation_id,
        )?;

        let reservation = self.reservation.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "VtmForward",
            detail: "reservation consumed twice".into(),
        })?;
        let estimated_htm_to_relay = self.relay.htm_to_relay_estimate(self.relay_id);
        Ok(RelayHandoff {
            reservation,
            deposit_time: vtm_deposit,
            estimated_htm_to_relay,
            station_arrival_plan: self.station_arrival_plan,
            presentation_id,
        })
    }
}

impl VtmForward {
    // The forward cycle's full event stream: idle-reason attribution, the
    // six nav legs (geometric boundaries where the padded cycle diverges),
    // the fork operations, tote visuals, and the per-cycle KPI row.
    // Observation only — it changes no model state.
    fn emit_cycle(
        &self,
        assignment: &VtmAssignment,
        slot_endpoint: VtmEndpoint,
        relay_endpoint: VtmEndpoint,
        vtm_cycle_id: VtmCycleId,
        presentation_id: PresentationId,
    ) -> Result<(), PlanError> {
        use crate::events::*;
        use crate::observer::time_slice::ToteVisualState;

        let vtm_id = assignment.vtm_id;
        let cycle = &assignment.cycle;
        let geo_cycle = &assignment.geo_cycle;
        let cycle_duration = assignment.cycle_duration;
        let vtm_start = assignment.vtm_start;
        let vtm_deposit = assignment.vtm_arrival;
        let from_endpoint = assignment.from_endpoint.clone();
        let rack_pos = assignment.slot_pos;
        let relay_pos = assignment.relay_pos;

        let t_at_rack = vtm_start + SimDuration::from_secs(cycle.first_travel_secs);
        let t_rack_aligned = t_at_rack + SimDuration::from_secs(cycle.rack_alignment_secs);
        let t_rack_pickup_done = t_rack_aligned + SimDuration::from_secs(cycle.rack_transfer_secs);
        let t_at_relay = t_rack_pickup_done + SimDuration::from_secs(cycle.second_travel_secs);
        let t_relay_aligned = t_at_relay + SimDuration::from_secs(cycle.relay_alignment_secs);

        let geo_diverged = geo_cycle != cycle;
        let geo = if geo_diverged {
            vtm_leg_boundaries(
                vtm_start,
                [
                    geo_cycle.first_travel_secs,
                    geo_cycle.rack_alignment_secs,
                    geo_cycle.rack_transfer_secs,
                    geo_cycle.second_travel_secs,
                    geo_cycle.relay_alignment_secs,
                    geo_cycle.relay_transfer_secs,
                ],
            )
        } else {
            [vtm_start, t_at_rack, t_rack_aligned, t_rack_pickup_done, t_at_relay, t_relay_aligned, vtm_deposit]
        };
        let [
            geo_start,
            geo_at_rack,
            geo_rack_aligned,
            geo_rack_pickup_done,
            geo_at_relay,
            geo_relay_aligned,
            geo_deposit,
        ] = geo;

        let payload = TotePayloadState::from_empty(self.inventory.tote(self.tote_id)?.is_empty_at(t_rack_pickup_done));

        let current_pos = from_endpoint.pos();
        let approach_dist_mm =
            ((rack_pos.x_mm - current_pos.x_mm).powi(2) + (rack_pos.y_mm - current_pos.y_mm).powi(2)).sqrt();
        let loaded_dist_mm =
            ((relay_pos.x_mm - rack_pos.x_mm).powi(2) + (relay_pos.y_mm - rack_pos.y_mm).powi(2)).sqrt();

        debug_assert!(
            vtm_start >= assignment.available_at,
            "vtm_start {vtm_start:?} < available_at {:?} — dispatcher invariant violated",
            assignment.available_at,
        );
        let reason = if vtm_start == assignment.available_at {
            VtmIdleReason::Continuous
        } else {
            let gap = vtm_start - assignment.available_at;
            let relay_start = self.relay_start;
            let cause = self
                .reservation
                .as_ref()
                .ok_or_else(|| PlanError::TaskGraphInvariant {
                    context: "VtmForward",
                    detail: "reservation consumed before the idle reason was emitted".into(),
                })?
                .start_cause
                .ok_or_else(|| PlanError::TaskGraphInvariant {
                    context: "VtmForward",
                    detail: "retrieve reservation carries no forward-floor start cause".into(),
                })?;
            match cause {
                RelayStartCause::Demand => VtmIdleReason::AwaitingDemand { gap, relay_start },
                RelayStartCause::Tote => VtmIdleReason::AwaitingTote { gap, relay_start },
                RelayStartCause::Slot => VtmIdleReason::AwaitingSlot { gap, relay_start },
            }
        };
        let chain_id = Some(presentation_id);
        let direction = VtmCycleDirection::Forward;
        self.events.emit(SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: vtm_id,
            vtm_cycle_id,
            at: vtm_start,
            reason,
            presentation_id: chain_id,
            direction,
            relay_id: self.relay_id,
            geo_diverged,
        }));

        let nav = |kind, geo_start, geo_end| {
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
        nav(
            VtmNavKind::RailMove {
                from: from_endpoint,
                to: slot_endpoint.clone(),
                tote_id: None,
                start: vtm_start,
                end: t_at_rack,
                distance_mm: approach_dist_mm as u64,
            },
            geo_start,
            geo_at_rack,
        );
        nav(
            VtmNavKind::Alignment { at: slot_endpoint.clone(), tote_id: None, start: t_at_rack, end: t_rack_aligned },
            geo_at_rack,
            geo_rack_aligned,
        );
        nav(
            VtmNavKind::Fork {
                op: ForkOp::Get,
                at: slot_endpoint.clone(),
                tote_id: self.tote_id,
                start: t_rack_aligned,
                end: t_rack_pickup_done,
            },
            geo_rack_aligned,
            geo_rack_pickup_done,
        );
        nav(
            VtmNavKind::RailMove {
                from: slot_endpoint,
                to: relay_endpoint.clone(),
                tote_id: Some(self.tote_id),
                start: t_rack_pickup_done,
                end: t_at_relay,
                distance_mm: loaded_dist_mm as u64,
            },
            geo_rack_pickup_done,
            geo_at_relay,
        );
        nav(
            VtmNavKind::Alignment {
                at: relay_endpoint.clone(),
                tote_id: Some(self.tote_id),
                start: t_at_relay,
                end: t_relay_aligned,
            },
            geo_at_relay,
            geo_relay_aligned,
        );
        nav(
            VtmNavKind::Fork {
                op: ForkOp::Put,
                at: relay_endpoint,
                tote_id: self.tote_id,
                start: t_relay_aligned,
                end: vtm_deposit,
            },
            geo_relay_aligned,
            geo_deposit,
        );

        self.events.emit(SimEvent::VtmOperation(VtmOperationEvent {
            sim_time_ms: t_rack_pickup_done.as_ms(),
            vtm_id,
            vtm_cycle_id,
            zone_id: self.zone_id,
            kind: VtmOperationKind::PickupStorage,
            rack_id: self.rack_slot.rack_id,
            rack_level: self.rack_slot.level,
            relay_id: self.relay_id,
            tote_id: self.tote_id,
            payload,
            presentation_id: chain_id,
        }));
        self.events.emit(SimEvent::VtmOperation(VtmOperationEvent {
            sim_time_ms: vtm_deposit.as_ms(),
            vtm_id,
            vtm_cycle_id,
            zone_id: self.zone_id,
            kind: VtmOperationKind::DropRelay,
            rack_id: self.rack_slot.rack_id,
            rack_level: self.rack_slot.level,
            relay_id: self.relay_id,
            tote_id: self.tote_id,
            payload,
            presentation_id: chain_id,
        }));

        self.events.emit(ToteMovedEvent {
            tote_id: self.tote_id,
            time: t_rack_pickup_done,
            new_state: ToteVisualState::OnBot { bot_id: vtm_id.0 },
        });
        let relay_htm_cell = self.relay.htm_access_cell(self.relay_id).map(|c| c.0).unwrap_or(0);
        self.events.emit(ToteMovedEvent {
            tote_id: self.tote_id,
            time: vtm_deposit,
            new_state: ToteVisualState::AtRelay { cell_id: relay_htm_cell },
        });
        self.events.emit(RelayStateChangeEvent { cell_id: relay_htm_cell, reserved: true, time: vtm_deposit });
        self.events.emit(SimEvent::Resource(ResourceEvent {
            kind: ResourceEventKind::VtmForwardCompleted { cycle_ms: cycle_duration.as_ms() },
            sim_time_ms: vtm_deposit.as_ms(),
        }));

        {
            let empty_ms = (geo_at_rack - geo_start).as_ms();
            let rack_alignment_ms = (geo_rack_aligned - geo_at_rack).as_ms();
            let fork_ms = (geo_rack_pickup_done - geo_rack_aligned).as_ms();
            let loaded_ms = (geo_at_relay - geo_rack_pickup_done).as_ms();
            let relay_alignment_ms = (geo_relay_aligned - geo_at_relay).as_ms();
            let relay_transfer_ms = (geo_deposit - geo_relay_aligned).as_ms();
            let buffer_ms = relay_alignment_ms + relay_transfer_ms;
            let tote_source = self.source.ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "VtmForward",
                detail: "forward cycle needs explicit flow attribution or a station-arrival plan".into(),
            })?;
            self.events.emit(SimEvent::VtmCycle(VtmCycleEvent {
                vtm_id,
                vtm_cycle_id,
                zone_id: self.zone_id,
                direction: VtmCycleDirection::Forward,
                empty_leg_ms: empty_ms,
                loaded_leg_ms: loaded_ms,
                fork_ms,
                buffer_ms,
                rack_alignment_ms,
                relay_alignment_ms,
                relay_transfer_ms,
                total_cycle_ms: empty_ms + loaded_ms + rack_alignment_ms + fork_ms + buffer_ms,
                sim_time_ms: vtm_start.as_ms(),
                relay_id: self.relay_id,
                rack_id: self.rack_slot.rack_id,
                tote_id: self.tote_id,
                presentation_id: chain_id,
                pre_cycle_wait_ms: (vtm_start - assignment.available_at).as_ms(),
                wait_reason: reason.kind(),
                tote_source,
            }));
        }
        Ok(())
    }
}
