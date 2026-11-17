//! Display impls for simulation events (concise event logging).

use std::fmt;

use desgraph::RobotType;

use super::{CycleKind, DispatchTrigger, ResourceEventKind, SimEvent, VtmCycleDirection, VtmIdleReason};

/// Format sim time as M:SS.mmm for concise event logs.
fn fmt_sim_time(f: &mut fmt::Formatter<'_>, ms: u64) -> fmt::Result {
    let secs = ms / 1000;
    let millis = ms % 1000;
    let mins = secs / 60;
    let rem_secs = secs % 60;
    write!(f, "{:3}:{:02}.{:03}", mins, rem_secs, millis)
}

impl fmt::Display for SimEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimEvent::Nav(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.time.as_ms())?;
                write!(f, "] NAV {:?} {:?} {}wp {:.0}mm", e.robot_type, e.robot_id, e.waypoints.len(), e.distance_mm,)
            }
            SimEvent::VtmNav(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.kind.start().as_ms())?;
                use crate::events::VtmNavKind;
                // Timestamp and duration are both instant-clock; a diverged
                // geometric window is appended explicitly. No suffix outside
                // Teleport.
                match &e.kind {
                    VtmNavKind::Alignment { at, tote_id, start, end } => {
                        write!(
                            f,
                            "] VTM_NAV {:?} {} alignment at c{} tote={:?} {}ms",
                            e.robot_id,
                            e.vtm_cycle_id,
                            at.cell().0,
                            tote_id,
                            (end.as_ms() - start.as_ms()),
                        )?;
                    }
                    VtmNavKind::Fork { op, at, tote_id, start, end } => {
                        write!(
                            f,
                            "] VTM_NAV {:?} {} fork {:?} at c{} tote={:?} {}ms",
                            e.robot_id,
                            e.vtm_cycle_id,
                            op,
                            at.cell().0,
                            tote_id,
                            (end.as_ms() - start.as_ms()),
                        )?;
                    }
                    VtmNavKind::RailMove { from, to, distance_mm, start, end, .. } => {
                        write!(
                            f,
                            "] VTM_NAV {:?} {} railmove c{}->c{} {:.0}mm {}ms",
                            e.robot_id,
                            e.vtm_cycle_id,
                            from.cell().0,
                            to.cell().0,
                            distance_mm,
                            (end.as_ms() - start.as_ms()),
                        )?;
                    }
                }
                if e.geo_start != e.kind.start() || e.geo_end != e.kind.end() {
                    write!(f, " geo=[{}ms+{}ms]", e.geo_start.as_ms(), (e.geo_end - e.geo_start).as_ms())?;
                }
                Ok(())
            }
            SimEvent::CycleComplete(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                let (label, qty) = match e.kind {
                    CycleKind::Pick { qty, .. } => ("picks", qty),
                    CycleKind::Putaway { qty, .. } => ("putaways", qty),
                    CycleKind::OrderLoad { qty } => ("order-load", qty),
                    CycleKind::OrderExtract { qty } => ("order-extract", qty),
                };
                write!(f, "] CYCLE {:?} {:?} station={:?} {}={}", e.robot_type, e.robot_id, e.station_id, label, qty)
            }
            SimEvent::VtmCycle(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                let dir = match e.direction {
                    VtmCycleDirection::Forward => "fwd",
                    VtmCycleDirection::Return => "ret",
                };
                write!(
                    f,
                    "] VTM_CYCLE {:?} {} zone={:?} {} empty={}ms loaded={}ms fork={}ms buffer={}ms total={}ms",
                    e.vtm_id,
                    e.vtm_cycle_id,
                    e.zone_id,
                    dir,
                    e.empty_leg_ms,
                    e.loaded_leg_ms,
                    e.fork_ms,
                    e.buffer_ms,
                    e.total_cycle_ms
                )
            }
            SimEvent::VtmOperation(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(
                    f,
                    "] DA50_OPERATION {:?} {} {:?} tote={:?} payload={:?}",
                    e.vtm_id, e.vtm_cycle_id, e.kind, e.tote_id, e.payload,
                )
            }
            SimEvent::Resource(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(f, "] RESOURCE {}", e.kind)
            }
            SimEvent::InitialPositions(e) => {
                let htms = e.positions.iter().filter(|(_, _, t, _)| *t == RobotType::Htm).count();
                let vtms = e.positions.iter().filter(|(_, _, t, _)| *t == RobotType::Vtm).count();
                let rtps = e.positions.iter().filter(|(_, _, t, _)| *t == RobotType::Rtp).count();
                write!(f, "[  0:00.000] INITIAL_POSITIONS {} HTMs, {} VTMs, {} RTPs", htms, vtms, rtps)
            }
            SimEvent::OrderComplete(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(f, "] ORDER_COMPLETE {:?}", e.order_id)
            }
            SimEvent::OrderDispatch(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(
                    f,
                    "] ORDER_DISPATCH {:?} -> station={:?} score={} depth={}/{} {:?}",
                    e.order_id, e.station_id, e.score, e.pipeline_depth_before, e.pipeline_target, e.trigger,
                )
            }
            SimEvent::HtmDispatch(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(
                    f,
                    "] HTM_DISPATCH {:?} tote={:?} station={:?} relay={:?}",
                    e.robot_id, e.tote_id, e.station_id, e.relay_id,
                )
            }
            SimEvent::PipelineSnapshot(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(
                    f,
                    "] PIPELINE station={:?} scheduled={} nft={}ms max_inflight={}",
                    e.station_id, e.scheduled_picks_count, e.next_free_time_ms, e.max_inflight_picks,
                )
            }
            SimEvent::HtmAssignment(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.dispatch_time.as_ms())?;
                write!(
                    f,
                    "] HTM_ASSIGNMENT {:?} station={:?} tote={:?} source={} quoted_arrival={}",
                    e.robot_id, e.station_id, e.tote_id, e.source, e.quoted_arrival,
                )
            }
            SimEvent::HtmArrival(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.arrival.as_ms())?;
                write!(f, "] HTM_ARRIVAL {:?} station={:?}", e.robot_id, e.station_id)
            }
            SimEvent::DispatchRejection(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.at.as_ms())?;
                write!(f, "] DISPATCH_REJECT station={:?} reason={} count={}", e.station_id, e.reason, e.count)?;
                match &e.detail {
                    Some(detail) => write!(f, " ({detail})"),
                    None => Ok(()),
                }
            }
            SimEvent::OrderStationPhase(e) => write!(
                f,
                "ORDER_STATION_PHASE order={:?} conveyor={:?} presentation={:?} phase={:?} t={}",
                e.order_id, e.conveyor_id, e.presentation_id, e.phase, e.at
            ),
            SimEvent::PresentationReadiness(e) => write!(
                f,
                "PRESENTATION_READY order={:?} pick={} conveyor={:?} presentation={:?} stream={:?} t={}",
                e.order_id, e.pick_index, e.conveyor_id, e.presentation_id, e.stream, e.ready_at
            ),
            SimEvent::InventoryVisitPhase(e) => write!(
                f,
                "INVENTORY_VISIT_PHASE order={:?} pick={} conveyor={:?} phase={:?} t={}",
                e.order_id, e.pick_index, e.conveyor_id, e.phase, e.at
            ),
            SimEvent::BotPlanSnapshot(e) => {
                write!(f, "BOT_PLAN {:?} legs={}", e.robot_id, e.legs.len())
            }
            SimEvent::BarrierAdvanced(at) => {
                write!(f, "[")?;
                fmt_sim_time(f, at.as_ms())?;
                write!(f, "] BARRIER_ADVANCED")
            }
            SimEvent::CellHeld(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.from.as_ms())?;
                write!(f, "] CELL_HELD {:?} c{} until={:?}", e.robot_id, e.cell.0, e.until)
            }
            SimEvent::CellReleased(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.at.as_ms())?;
                write!(f, "] CELL_RELEASED {:?} c{}", e.robot_id, e.cell.0)
            }
            SimEvent::RelayStateChange(e) => {
                write!(f, "RELAY_STATE cell={} reserved={} t={}", e.cell_id, e.reserved, e.time)
            }
            SimEvent::ToteCreated(e) => {
                write!(f, "ToteCreated({:?}, {:?}, {:?}, {:?})", e.tote_id, e.tote_type, e.initial_state, e.time)
            }
            SimEvent::ToteContentsChanged(e) => {
                write!(f, "TOTE_CONTENTS tote={:?} units={} t={}", e.tote_id, e.units, e.at)
            }
            SimEvent::ToteInitialStates(e) => {
                write!(f, "TOTE_INITIAL count={}", e.totes.len())
            }
            SimEvent::StripePlan(e) => {
                write!(f, "STRIPE_PLAN destinations={} stations={}", e.entries.len(), e.stations.len())
            }
            SimEvent::ToteTransit(e) => {
                write!(
                    f,
                    "TOTE_TRANSIT {:?} {:?}->{:?} {}..{}ms",
                    e.tote_id,
                    e.from_cell,
                    e.to_cell,
                    e.start.as_ms(),
                    e.end.as_ms()
                )
            }
            SimEvent::ToteMoved(e) => {
                write!(f, "TOTE_MOVED tote={:?} state={:?} t={}", e.tote_id, e.new_state, e.time)
            }
            SimEvent::ArrivalError(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(
                    f,
                    "] ARRIVAL_ERROR {:?} station={:?} target={}ms actual={}ms err={:+}ms",
                    e.order_id, e.station_id, e.target_arrival_ms, e.actual_arrival_ms, e.error_ms,
                )
            }
            SimEvent::WatchdogTimeout(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(
                    f,
                    "] WATCHDOG_TIMEOUT task={:?} order={:?} station={:?} timeout={:.1}s",
                    e.task_id, e.order_id, e.station_id, e.timeout_secs,
                )
            }
            SimEvent::TripStart(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(f, "] TRIP_START {:?} {:?} {:?}", e.robot_type, e.robot_id, e.purpose)
            }
            SimEvent::Induction(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(f, "] INDUCTION station={:?} tote={:?} cases={}", e.station_id, e.tote_id, e.case_count)
            }
            SimEvent::VtmIdleReason(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.at.as_ms())?;
                write!(f, "] VTM_IDLE {:?} {} ", e.robot_id, e.vtm_cycle_id)?;
                match e.reason {
                    VtmIdleReason::Continuous => write!(f, "continuous"),
                    VtmIdleReason::AwaitingDemand { gap, relay_start } => {
                        write!(f, "awaiting_demand gap={}ms relay_start={}ms", gap.as_ms(), relay_start.as_ms())
                    }
                    VtmIdleReason::AwaitingTote { gap, relay_start } => {
                        write!(f, "awaiting_tote gap={}ms relay_start={}ms", gap.as_ms(), relay_start.as_ms())
                    }
                    VtmIdleReason::AwaitingSlot { gap, relay_start } => {
                        write!(f, "awaiting_slot gap={}ms relay_start={}ms", gap.as_ms(), relay_start.as_ms())
                    }
                    VtmIdleReason::AwaitingHtmDeposit { gap, deposit_time } => {
                        write!(f, "awaiting_htm_deposit gap={}ms deposit_time={}ms", gap.as_ms(), deposit_time.as_ms())
                    }
                }
            }
            SimEvent::InventoryDelta(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.at.as_ms())?;
                write!(f, "] INV_DELTA {:+}", e.delta)
            }
            SimEvent::InventoryMove(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.at.as_ms())?;
                write!(f, "] INV_MOVE tote={:?} skus={}", e.tote_id, e.contents.len())
            }
            SimEvent::HtmRelayPickup(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.start.as_ms())?;
                write!(f, "] RELAY_PICKUP {:?} relay={:?} wait={}ms", e.robot_id, e.relay_id, e.wait.as_ms())
            }
            SimEvent::HtmRelayDeposit(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.start.as_ms())?;
                write!(f, "] RELAY_DEPOSIT {:?} relay={:?} wait={}ms", e.robot_id, e.relay_id, e.wait.as_ms())
            }
            SimEvent::DivingBoardSlotWait(e) => {
                write!(f, "[")?;
                fmt_sim_time(f, e.sim_time_ms)?;
                write!(f, "] DBOARD_SLOT_WAIT relay={:?} wait={}ms", e.relay_id, e.wait_ms)
            }
        }
    }
}

impl fmt::Display for ResourceEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceEventKind::RelayReservationSuccess => write!(f, "RelayReserveOK"),
            ResourceEventKind::RelayReservationFailure => write!(f, "RelayReserveFAIL"),
            ResourceEventKind::ToteClaimSuccess => write!(f, "ToteClaimOK"),
            ResourceEventKind::ToteClaimFailure => write!(f, "ToteClaimFAIL"),
            ResourceEventKind::BackpressureTrigger => write!(f, "Backpressure"),
            ResourceEventKind::PathfinderQuery => write!(f, "PathfinderQuery"),
            ResourceEventKind::PathfinderFailure => write!(f, "PathfinderFAIL"),
            ResourceEventKind::VtmScoreForwardSelected => write!(f, "VtmForwardSelected"),
            ResourceEventKind::VtmScoreReturnSelected => write!(f, "VtmReturnSelected"),
            ResourceEventKind::VtmCrossZoneAssignment => write!(f, "VtmCrossZone"),
            ResourceEventKind::VtmScoringDuration { duration_ns } => {
                write!(f, "VtmScoring {}us", duration_ns / 1000)
            }
            ResourceEventKind::ZonePressureSnapshot { zone_id, pressure } => {
                write!(f, "ZonePressure z={} p={:.2}", zone_id, pressure)
            }
            ResourceEventKind::StarvationAgeBonusApplied => write!(f, "StarvationBonus"),
            ResourceEventKind::DeadlockHeadroomTriggered => write!(f, "DeadlockHeadroom"),
            ResourceEventKind::VtmForwardCompleted { cycle_ms } => {
                write!(f, "VtmForwardDone {}ms", cycle_ms)
            }
            ResourceEventKind::VtmReturnCompleted { cycle_ms } => {
                write!(f, "VtmReturnDone {}ms", cycle_ms)
            }
            ResourceEventKind::HtmPickupCompleted { cycle_ms } => {
                write!(f, "HtmPickupDone {}ms", cycle_ms)
            }
            ResourceEventKind::HtmReturnCompleted { cycle_ms, .. } => {
                write!(f, "HtmReturnDone {}ms", cycle_ms)
            }
            ResourceEventKind::HtmOperatorWaitCompleted { wait_ms } => {
                write!(f, "HtmOpWait {}ms", wait_ms)
            }
            ResourceEventKind::HtmReturnRelayRetry => write!(f, "HtmReturnRetry"),
            ResourceEventKind::OperatorQueueWait => write!(f, "OperatorQueueWait"),
        }
    }
}

impl fmt::Display for DispatchTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DispatchTrigger::Bootstrap => write!(f, "Bootstrap"),
            DispatchTrigger::HtmDeparture => write!(f, "HtmDeparture"),
            DispatchTrigger::HtmIdle => write!(f, "HtmIdle"),
            DispatchTrigger::Periodic => write!(f, "Periodic"),
        }
    }
}
