//! The first HTM leg of a delivery chain: an idle HTM drives to the relay
//! access cell and lifts off the tote a forward VTM cycle deposited there.
//!
//! Within `task`, `htm_pickup` owns the choice of WHICH idle HTM serves a
//! relay reservation. `propose` ranks the idle pool and publishes the
//! winner as a candidacy; acceptance claims that HTM; `schedule` then
//! reserves the drive and spends the chain state around it.

use crate::events::EventJournal;
use crate::flows::transport::windows::OneWindowJourney;
use crate::flows::ttp::demand::park_idle_htms::vacate_cell;
use crate::model::ParkingRegistry;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotService, LegSpec};
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::relay::{RelayService, RetrieveAdmission};
use crate::task::TaskContext;

use std::fmt;
use std::future::pending;
use std::time::Instant;

use crate::error::planner::ErrContext;
use crate::events::TripEndpoint;
use crate::flows::ttp::handoff::{HtmCarryingTote, RelayHandoff};
use crate::model::ToteLocation;
use crate::model::htm::registry::{AvailableHtm, PickupHeadroom, PickupSelection, ProposedHtmPickup};
use crate::model::relay::RETRIEVE_ACCESS_MAX_WAIT;
use crate::pathfinding::{CellCandidate, CollisionConfig, Orientation, PathfinderError};
use crate::prelude::*;
use desgraph::{CellId, RelayId, WorldPos, ZoneId};
use desmap::RobotType;
use tokio::select;

#[cfg(test)]
#[path = "htm_pickup_tests.rs"]
mod tests;

/// What one pass over the idle pool concluded.
enum SelectionPass {
    /// Nothing usable stands in the pool yet — the caller suspends on a
    /// fleet or relay change and runs another pass.
    Retry,
    Selected(PickupSelection),
}

#[derive(Clone)]
struct HtmPickupCandidate {
    htm: AvailableHtm,
}

impl CellCandidate for HtmPickupCandidate {
    fn cell(&self) -> CellId {
        self.htm.cell()
    }
}

pub struct HtmPickup {
    zone_id: ZoneId,
    handoff: Option<RelayHandoff>,
    clock: Mut<SimTime>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    headroom: Option<PickupHeadroom>,
}

impl HtmPickup {
    pub fn new(ctx: &TaskContext, handoff: RelayHandoff) -> Result<Self, PlanError> {
        let input = world_inputs(ctx.states())?;
        Ok(Self {
            zone_id: handoff.reservation.zone_id,
            handoff: Some(handoff),
            clock: input.clock.clone(),
            navigation: ctx.state()?,
            relay: ctx.state::<RelayService>()?.as_ref().clone(),
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: ctx.state::<InventoryService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: Some(ctx.state::<ParkingRegistry>()?.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            headroom: None,
        })
    }

    pub fn with_admission(self, admission: RetrieveAdmission) -> Self {
        let changed = admission.version().clone();
        self.with_headroom(PickupHeadroom::new(2, Arc::new(move || !admission.is_productive()), changed))
    }

    pub fn with_headroom(mut self, headroom: PickupHeadroom) -> Self {
        assert!(
            self.headroom.is_none(),
            "HtmPickup accepts one headroom policy; choose admission or explicit headroom"
        );
        self.headroom = Some(headroom);
        self
    }

    fn relay_cell(&self, relay_id: RelayId) -> Result<CellId, PlanError> {
        self.relay.htm_access_cell(relay_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmPickup",
            detail: format!("no HTM access cell for relay {:?}", relay_id),
        })
    }

    async fn clear_route_holder(
        &self,
        from: CellId,
        orientation: Orientation,
        to: CellId,
        robot: RobotId,
        depart: SimTime,
        oracle_seen: Option<u64>,
        fleet_seen: u64,
    ) -> Result<bool, PlanError> {
        let oracle = &self.navigation.oracle;
        let route = self.navigation.pathfinder.find_path(
            from,
            orientation,
            to,
            self.navigation.model(robot)?.estimate[0].as_ref(),
            depart,
        )?;
        let mut cells: Vec<_> = route.waypoints.iter().map(|waypoint| waypoint.cell).collect();
        cells.dedup();
        let mut busy_holder = false;
        for cell in cells {
            for hold in oracle.reservations_for_cell(cell) {
                if hold.robot() == robot || !hold.is_open_ended() {
                    continue;
                }
                if self.available.contains(hold.robot()) {
                    if vacate_cell(
                        &self.navigation,
                        &self.available,
                        &self.bots,
                        &self.inventory,
                        self.parking.as_ref(),
                        &self.events,
                        hold.robot(),
                        cell,
                        depart,
                    )
                    .await?
                    {
                        return Ok(true);
                    }
                } else {
                    busy_holder = true;
                }
            }
        }
        // A claimed holder needs its own next task to close this reservation.
        // Waiting after acceptance lets that task run without bypassing it.
        if busy_holder && let Some((version, seen)) = oracle.version().zip(oracle_seen) {
            select! {
                biased;
                _ = version.past(seen) => {}
                _ = self.available.version().past(fleet_seen) => {}
            }
            return Ok(true);
        }
        Ok(false)
    }

    // One pass over the idle pool: reject the relay access cell's holder
    // when it cannot serve, rank every empty HTM by real path time to the
    // relay, and derive when the winner's drive starts. Reads only — the
    // winner is still in the pool when this returns. The count it returns
    // alongside is how many candidates the pass had to rank.
    fn select_once(
        &self,
        relay_id: RelayId,
        relay_cell: CellId,
        relay_grid: WorldPos,
        deposit_time: SimTime,
    ) -> Result<(SelectionPass, usize), PlanError> {
        let required_holder = match self.relay.htm_access_holder(relay_id) {
            Some(blocking_robot) => {
                let holder = self.available.peek_matching(|htm| htm.robot() == blocking_robot).into_iter().next();
                match holder {
                    Some(htm) if htm.tote.is_none() => Some(blocking_robot),
                    _ => {
                        crate::debug_log!(
                            "[HtmPickup.wait] relay={:?} relay_cell={:?} holder={:?} holder_usable=false deposit={}ms relay_res={:?}",
                            relay_id,
                            relay_cell,
                            blocking_robot,
                            deposit_time.as_ms(),
                            self.navigation.oracle.reservations_for_cell(relay_cell)
                        );
                        return Ok((SelectionPass::Retry, 0));
                    }
                }
            }
            None => None,
        };

        let mut candidates = {
            crate::perf_span!("htm_pickup_candidates");
            self.available.find_nearest(
                relay_grid,
                |htm| {
                    htm.tote.is_none()
                        && match required_holder {
                            Some(robot) => htm.robot() == robot,
                            None => true,
                        }
                        && !self.navigation.htm_departure_is_blocked(
                            htm.robot(),
                            htm.cell(),
                            relay_cell,
                            false,
                            htm.available_at().max(self.clock.value()),
                        )
                },
                usize::MAX,
            )
        };
        let considered = candidates.len();
        if candidates.is_empty() {
            crate::debug_log!(
                "[HtmPickup.wait] relay={:?} relay_cell={:?} deposit={}ms reason=no_empty_htm_candidates",
                relay_id,
                relay_cell,
                deposit_time.as_ms()
            );
            return Ok((SelectionPass::Retry, considered));
        }

        if required_holder.is_some() && considered != 1 {
            return Err(PlanError::TaskGraphInvariant {
                context: "HtmPickup",
                detail: format!("relay cell {:?} holder filter returned {} candidates", relay_cell, considered),
            });
        }

        crate::perf_span!("htm_pickup_prescreen_refine");
        let mut delayed: Option<PickupSelection> = None;
        while !candidates.is_empty() {
            let selected = self
                .navigation
                .select_htm_to(
                    candidates.iter().cloned().map(|htm| HtmPickupCandidate { htm }),
                    relay_cell,
                    |candidate| candidate.htm.robot(),
                    |candidate| candidate.htm.orientation,
                    false,
                    |c, travel| {
                        let earliest_pickup = c.htm.available_at() + travel;
                        Some(earliest_pickup.max(deposit_time))
                    },
                )
                .err_context("HtmPickup")?;

            let htm = selected.candidate.htm;
            let travel = selected.travel_time;
            let resident = htm.cell() == relay_cell;
            let mut depart_anchor = deposit_time.saturating_sub(travel).max(htm.available_at());
            if let Some(headroom) = &self.headroom {
                let Some(allowed_at) =
                    headroom.earliest_allowed_at(&self.available, depart_anchor.max(self.clock.value()))
                else {
                    return Ok((SelectionPass::Retry, considered));
                };
                depart_anchor = allowed_at;
            }
            if self.navigation.htm_departure_is_blocked(
                htm.robot(),
                htm.cell(),
                relay_cell,
                false,
                depart_anchor.max(self.clock.value()),
            ) {
                candidates.retain(|candidate| candidate.robot() != htm.robot());
                continue;
            }
            if self.navigation.htm_departure_exceeds_horizon(
                htm.robot(),
                htm.cell(),
                relay_cell,
                false,
                depart_anchor.max(self.clock.value()),
                RETRIEVE_ACCESS_MAX_WAIT,
            ) {
                candidates.retain(|candidate| candidate.robot() != htm.robot());
                if let Some(mut opening) = self.navigation.htm_departure_opening(
                    htm.robot(),
                    htm.cell(),
                    relay_cell,
                    false,
                    depart_anchor.max(self.clock.value()),
                ) {
                    if let Some(headroom) = &self.headroom {
                        let Some(allowed) = headroom.earliest_allowed_at(&self.available, opening) else { continue };
                        opening = allowed;
                    }
                    if delayed
                        .as_ref()
                        .is_none_or(|previous| opening + travel < previous.depart_anchor + previous.travel)
                    {
                        delayed = Some(PickupSelection { htm, travel, depart_anchor: opening, resident });
                    }
                }
                continue;
            }
            return Ok((SelectionPass::Selected(PickupSelection { htm, travel, depart_anchor, resident }), considered));
        }
        // A finite departure opening yields a revocable future candidacy when
        // no HTM can leave sooner. Expiry alone emits no oracle change to wake Retry.
        Ok((delayed.map_or(SelectionPass::Retry, SelectionPass::Selected), considered))
    }
}

impl Drop for HtmPickup {
    fn drop(&mut self) {
        if let Some(handoff) = self.handoff.take() {
            handoff.reservation.cancel_after(handoff.deposit_time);
        }
    }
}

impl fmt::Display for HtmPickup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HtmPickup(zone={:?})", self.zone_id)
    }
}

#[async_trait(?Send)]
impl Task for HtmPickup {
    type Proposal = ProposedHtmPickup;
    type Schedule = HtmCarryingTote;

    // Selection only — the candidacy holds the registry's version count as
    // its premise, and spends nothing. A propose error drops the task,
    // whose Drop cancels the held relay reservation.
    async fn propose(&self, owner: ProposalId) -> Result<ProposedHtmPickup, PlanError> {
        let (relay_id, deposit_time) = {
            let h = self.handoff.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmPickup",
                detail: "proposed after its handoff was consumed".into(),
            })?;
            (h.reservation.relay_id, h.deposit_time)
        };
        let relay_cell = self.relay_cell(relay_id)?;
        let relay_grid =
            self.navigation.map.cell_world_pos(relay_cell).ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmPickup",
                detail: format!("no grid pos for relay cell {:?}", relay_cell),
            })?;

        let (selection, seen) = loop {
            // Subscribe before reading, so a change during the pass wakes
            // the wait below instead of being missed. No await runs inside
            // the pass, so `fleet_seen` is exactly the version it saw —
            // which is what the candidacy records as its premise.
            let fleet_seen = self.available.version().seen();
            let headroom_seen = self.headroom.as_ref().map(|headroom| headroom.version().seen());
            let oracle_version = self.navigation.oracle.version();
            let oracle_seen = oracle_version.map(Version::seen);
            let relay_changed = notified(self.relay.signal());

            let started = Instant::now();
            let (pass, considered) = self.select_once(relay_id, relay_cell, relay_grid, deposit_time)?;
            let elapsed = started.elapsed();
            self.available.record_selection(elapsed);
            crate::trace_event!(
                metric,
                htm_pickup_selection,
                task = owner.task,
                nanos = elapsed.as_nanos(),
                candidates = considered,
            );

            match pass {
                SelectionPass::Selected(selection) => break (selection, fleet_seen),
                SelectionPass::Retry => {
                    select! {
                        biased;
                        _ = self.available.version().past(fleet_seen) => {}
                        _ = relay_changed => {}
                        _ = async {
                            match self.headroom.as_ref().zip(headroom_seen) {
                                Some((headroom, seen)) => headroom.version().past(seen).await,
                                None => pending().await,
                            }
                        } => {}
                        _ = async {
                            match oracle_version.zip(oracle_seen) {
                                Some((version, seen)) => version.past(seen).await,
                                None => pending().await,
                            }
                        } => {}
                    }
                }
            }
        };

        let mut proposal = ProposedHtmPickup::new(
            owner,
            self.available.clone(),
            self.navigation.clone(),
            relay_cell,
            selection,
            seen,
            self.clock.clone(),
        )
        .with_route_horizon(RETRIEVE_ACCESS_MAX_WAIT);
        if let Some(headroom) = &self.headroom {
            proposal = proposal.with_headroom(headroom.clone());
        }
        Ok(proposal)
    }

    // The HTM was claimed at acceptance; schedule spends the drive and the
    // chain state around it.
    async fn schedule(&mut self, proposal: ProposedHtmPickup, _ctx: SchedCtx) -> Result<HtmCarryingTote, PlanError> {
        let oracle = &self.navigation.oracle;

        let (tote_id, zone_id, relay_id, deposit_time, station_arrival_plan, presentation_id) = {
            let h = self.handoff.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmPickup",
                detail: "scheduled after its handoff was consumed".into(),
            })?;
            (
                h.reservation.recommendation,
                h.reservation.zone_id,
                h.reservation.relay_id,
                h.deposit_time,
                h.station_arrival_plan,
                h.presentation_id,
            )
        };
        let relay_cell = self.relay_cell(relay_id)?;

        let htm_travel_est = proposal.travel_estimate();
        let already_at_relay = proposal.resident();
        // The claim landed at acceptance, ahead of every path commit below:
        // a rival taking this HTM now costs the candidacy, never a
        // committed path whose robot went elsewhere.
        let (htm, mut htm_depart) = proposal.into_accepted()?;
        let htm_id = htm.robot();
        let htm_available_pos = htm.cell();
        let htm_available_at = htm.available_at();

        crate::trace_event!(
            metric,
            htm_pickup_actual_travel,
            relay = relay_id,
            htm = htm_id,
            resident = already_at_relay,
            travel = htm_travel_est,
        );

        let (path, path_tail, htm_relay_arrival) = if already_at_relay {
            (None, None, htm_available_at)
        } else {
            let reserved = loop {
                htm_depart = htm_depart.max(self.clock.value());
                let fleet_seen = self.available.version().seen();
                let oracle_seen = oracle.version().map(Version::seen);
                let relay_changed = notified(self.relay.signal());
                // Backstop for a new holder after acceptance; before acceptance
                // a destination holder invalidates the selection instead.
                if let Some(blocker) = oracle
                    .reservations_for_cell(relay_cell)
                    .into_iter()
                    .find(|r| r.robot() != htm_id && r.is_open_ended())
                {
                    crate::debug_log!(
                        "[HtmPickup.wait] relay={:?} relay_cell={:?} open_ended_holder={:?} depart={}ms",
                        relay_id,
                        relay_cell,
                        blocker.robot(),
                        htm_depart.as_ms()
                    );
                    select! {
                        biased;
                        _ = self.available.version().past(fleet_seen) => {}
                        _ = relay_changed => {}
                        _ = async {
                            match oracle.version().zip(oracle_seen) {
                                Some((version, seen)) => version.past(seen).await,
                                None => pending().await,
                            }
                        } => {}
                    }
                    continue;
                }
                match {
                    crate::perf_span!("htm_pickup_reserve_path");
                    let config = CollisionConfig {
                        self_robot: htm_id,
                        predecessor: Some(htm.cell_reservation.clone()),
                        destination: None,
                        sim_start: htm_depart,
                        dest_dwell: None,
                        collision_depth: 8,
                        window_size: 0,
                        replan_interval: 0,
                        max_wait: RETRIEVE_ACCESS_MAX_WAIT,
                        max_iterations: 5_000_000,
                        motion_model: self.navigation.model(htm_id)?.kind,
                    };
                    let drive = self.bots.drive(htm_id, false)?;
                    self.navigation
                        .pathfinder
                        .reserve_path(
                            htm_available_pos,
                            htm.orientation,
                            relay_cell,
                            drive.as_ref(),
                            oracle.as_ref(),
                            &config,
                            // The retry below restarts from the origin, so
                            // a window short of the relay is refused by name.
                            &mut OneWindowJourney { task: "HtmPickup", events: &self.events },
                        )
                        .await
                } {
                    Ok(reserved) => break reserved,
                    Err(PathfinderError::CommitConflict { from, to }) => {
                        eprintln!(
                            "[HtmPickup.conflict] htm={:?} from={:?} to={:?} relay={:?} relay_cell={:?} depart={}ms relay_res={:?} from_res={:?}",
                            htm_id,
                            from,
                            to,
                            relay_id,
                            relay_cell,
                            htm_depart.as_ms(),
                            oracle.reservations_for_cell(relay_cell),
                            oracle.reservations_for_cell(htm_available_pos)
                        );
                        return Err(PathfinderError::CommitConflict { from, to }).err_context("HtmPickup");
                    }
                    Err(e) => {
                        if matches!(e, PathfinderError::UnreachableWithinHorizon { .. })
                            && self
                                .clear_route_holder(
                                    htm_available_pos,
                                    htm.orientation,
                                    relay_cell,
                                    htm_id,
                                    htm_depart,
                                    oracle_seen,
                                    fleet_seen,
                                )
                                .await?
                        {
                            continue;
                        }
                        eprintln!(
                            "[HtmPickup.path_error] htm={:?} from={:?} relay={:?} relay_cell={:?} depart={}ms err={:?} relay_res={:?} from_res={:?}",
                            htm_id,
                            htm_available_pos,
                            relay_id,
                            relay_cell,
                            htm_depart.as_ms(),
                            e,
                            oracle.reservations_for_cell(relay_cell),
                            oracle.reservations_for_cell(htm_available_pos)
                        );
                        return Err(e).err_context("HtmPickup");
                    }
                }
            };
            let tail =
                reserved.committed_reservations.last().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
                    context: "HtmPickup",
                    detail: "reserve_path returned no reservations".into(),
                })?;
            let start = reserved.path.waypoints.first().map_or(htm_depart, |waypoint| waypoint.start_time());
            let arrival = start + reserved.path.travel_time;
            (Some(reserved.path), Some(tail), arrival)
        };

        // The drive starts when its window does, which the pathfinder may
        // have committed after an await; a resident HTM's start is the
        // accepted one, floored at acceptance to the barrier.
        let dispatch = path.as_ref().and_then(|path| path.waypoints.first()).map_or(htm_depart, |wp| wp.start_time());
        // The pickup never precedes the drive's own start.
        let pickup = htm_relay_arrival.max(deposit_time).max(dispatch);
        let pickup_done = pickup + self.bots.model(htm_id)?.figures.pickup_time();
        let tail = path_tail.unwrap_or_else(|| htm.cell_reservation.clone());

        let handoff = self.handoff.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmPickup",
            detail: "handoff consumed twice".into(),
        })?;
        handoff.reservation.finalize_pickup(pickup_done, &tail)?;

        use crate::pathfinding::ReservationRequest;
        let path_requests: Vec<ReservationRequest> =
            if let Some(ref p) = path { super::commit_plan_requests(&p.waypoints, None) } else { Vec::new() };
        let relay_leg = LegSpec {
            leg: Leg::RelayApproach,
            requests: &path_requests,
            action_ready_arrival: pickup_done,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation::empty(),
            carrying_after: Some(tote_id),
        };
        self.bots.commit_task(htm_id, TaskKind::Deliver, &[relay_leg], &self.inventory, self.parking.as_ref())?;

        crate::debug_log!(
            "[HtmPickup] tail={} cell={:?} htm={:?} open={} at_relay={}",
            tail.id(),
            tail.cell(),
            htm_id,
            tail.is_open_ended(),
            already_at_relay
        );
        let station_arrival_plan = station_arrival_plan.map(|mut plan| {
            plan.htm_source_ready_at = Some(htm_available_at);
            plan.htm_pickup_from = Some(htm_available_pos);
            plan.htm_pickup_to = Some(relay_cell);
            plan.htm_pickup_travel = Some(if already_at_relay { SimDuration::ZERO } else { htm_travel_est });
            plan.relay_pickup_at = Some(pickup);
            plan
        });

        let robot_id = htm_id;
        self.inventory.tote(tote_id)?.move_to(ToteLocation::WithRobot { robot_id: htm_id }, pickup_done)?;

        {
            use crate::events::*;
            use crate::observer::time_slice::ToteVisualState;
            if let Some(plan) = &station_arrival_plan {
                self.events.emit(HtmAssignmentEvent {
                    robot_id: htm_id,
                    station_id: plan.station_id,
                    tote_id: Some(tote_id),
                    source: plan.source,
                    relay_cell: Some(relay_cell.0),
                    dispatch_time: dispatch,
                    quoted_arrival: plan.quoted_arrival,
                });
            }
            if let Some(ref path) = path {
                use super::move_distance;
                self.events.emit(TripStartEvent {
                    robot_id,
                    robot_type: RobotType::Htm,
                    purpose: TripPurpose::ToRelay,
                    presentation_id: Some(presentation_id),
                    from: None,
                    to: TripEndpoint::Relay(relay_id),
                    sim_time_ms: dispatch.as_ms(),
                });
                self.events.emit(NavEventData {
                    robot_id,
                    robot_type: RobotType::Htm,
                    waypoints: path.waypoints.clone(),
                    time: dispatch,
                    distance_mm: move_distance(path, &self.navigation.map),
                    carrying_cargo: false,
                });
            }
            self.events.emit(ToteMovedEvent {
                tote_id,
                time: pickup_done,
                new_state: ToteVisualState::OnBot { bot_id: htm_id.0 },
            });
            self.events.emit(RelayStateChangeEvent { cell_id: relay_cell.0, reserved: false, time: pickup_done });

            self.events.emit(HtmRelayPickupEvent {
                robot_id,
                relay_id,
                start: pickup,
                end: pickup_done,
                wait: pickup - htm_relay_arrival,
                resident: already_at_relay,
            });
        }

        Ok(HtmCarryingTote {
            tote_id,
            zone_id,
            cell_reservation: tail,
            action_ready_at: pickup_done,
            orientation: path
                .as_ref()
                .and_then(|path| path.waypoints.last())
                .map(|waypoint| waypoint.orientation)
                .unwrap_or(htm.orientation),
            station_arrival_plan,
            presentation_id,
            source_relay_id: Some(relay_id),
            deliver_from: Some(TripEndpoint::Relay(relay_id)),
        })
    }
}
