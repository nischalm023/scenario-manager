//! The loaded HTM's drive: the HTM carrying a tote drives to its station's
//! presentation cell and stands there for the operator.
//!
//! Within `task`, `htm_deliver` chooses nothing — the HTM and the station
//! were both fixed at chain birth. What it owns is WHEN that drive may
//! be planned: [`ProposedHtmDeliver`] publishes its start to the barrier,
//! and `schedule` makes one `reserve_path` call from that start, which
//! foreshortens the drive to the station's queue while another robot holds
//! the presentation cell. A delivery whose HTM stands on an operator's
//! presentation cell, claimed over that stay's clearance, may not await
//! (scheduler time) other robots' commits before its first window: when no
//! window can be committed at its departure it is refused ([`DeliveryRefused`])
//! and the caller hands the HTM back to the clearance.

use crate::flows::transport::windows::JourneyWindows;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::bot::LegSpec;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::station_service::StationService;

use std::fmt;

use super::move_distance;
use crate::error::planner::ErrContext;
use crate::events::*;
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::flows::ttp::inflight::InflightSpan;
use crate::pathfinding::{CollisionConfig, PathfinderError, WaypointAction};
use crate::prelude::*;
use desgraph::types::RobotType;
use desgraph::{CellId, StationId};

const STATION_PRESENTATION_GAP_ABORT: SimDuration = SimDuration::seconds(60);

#[cfg(test)]
#[path = "htm_deliver_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "htm_deliver_queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "htm_deliver_refusal_tests.rs"]
mod refusal_tests;

// The delivery leg's admission candidacy. Nothing is selected here — the
// HTM and the station were both fixed at chain birth — so there is no
// premise to lose and `poisoned` stays false forever; eviction would only
// rebuild an identical candidacy. What it publishes is the drive's start,
// and nothing else. It spends nothing: acceptance records the departure
// instant.
//
// Whether the presentation cell is free is deliberately NOT on the
// admissible surface. `admissible` may gate only on conditions that
// in-flight work clears WITHOUT a further acceptance, and this one fails
// that test: the cell's holder is the HTM whose presentation there ends,
// and moving it needs its next task (a productive claim or the clearance)
// accepted, which an inadmissible minimum forbids. Since the start
// self-floors it would sit exactly at B forever and pin the whole run. The
// wait belongs in `schedule`'s `reserve_path` call, where it blocks one task
// instead of the clock.
pub struct ProposedHtmDeliver {
    station_id: StationId,
    robot: RobotId,
    anchor: SimTime,
    clock: Mut<SimTime>,
    accepted: Option<SimTime>,
}

impl ProposedHtmDeliver {
    pub fn new(station_id: StationId, robot: RobotId, anchor: SimTime, clock: Mut<SimTime>) -> Self {
        Self { station_id, robot, anchor, clock, accepted: None }
    }

    /// The departure instant acceptance settled on.
    fn into_accepted(mut self) -> Result<SimTime, PlanError> {
        self.accepted.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmDeliver",
            detail: "scheduled with a candidacy acceptance never settled".into(),
        })
    }

    fn start(&self) -> SimTime {
        self.anchor.max(self.clock.value())
    }
}

impl fmt::Display for ProposedHtmDeliver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "htm-deliver{{htm={:?} stn={:?} start={}}}", self.robot, self.station_id, self.start())
    }
}

impl ProposedReservation for ProposedHtmDeliver {
    // Scored on the drive's start. Self-flooring: an accepted chain plans
    // its later legs ahead of the settled frontier, so by the time the
    // barrier reaches this candidacy its anchor can already lie behind B.
    // Reporting the floored instant keeps the claim startable — physically
    // the HTM just stands where it is until then.
    fn start_time(&self) -> Option<SimTime> {
        Some(self.start())
    }

    // `admissible` stays the default true — see the type's comment for why
    // the presentation cell cannot be gated here.
    fn accept(&mut self) -> Result<(), AcceptError> {
        self.accepted = Some(self.start());
        Ok(())
    }
}

/// A delivery that did not depart: its HTM stands on an operator's
/// presentation cell whose stay it claimed over the clearance, no window
/// could be committed at `departure` (simtime), and its path search may not
/// await (scheduler time) other robots' commits before one is. Nothing of
/// the journey was committed and its in-flight span is withdrawn; the HTM's
/// hold on `origin` is as the claim found it.
#[derive(Debug)]
pub struct DeliveryRefused {
    pub origin: CellId,
    /// The cells on the route held open-ended (no departure scheduled) that
    /// left no window to commit.
    pub held: Vec<CellId>,
    /// The departure acceptance fixed (simtime): the stay's release.
    pub departure: SimTime,
}

impl From<DeliveryRefused> for PlanError {
    fn from(refused: DeliveryRefused) -> Self {
        PlanError::Pathfinder(PathfinderError::OriginWaitRefused { origin: refused.origin, held: refused.held })
    }
}

pub struct HtmDeliver {
    pub station_id: StationId,
    inflight: InflightSpan,
    handoff: Option<HtmCarryingTote>,
    clock: Mut<SimTime>,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    order_presentation: Option<CellId>,
    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
    /// True when the carrier being delivered is an order carrier.
    presents_order: bool,
    /// The HTM's assignment, held until the journey commits. Its presence
    /// makes the delivery refusable: the HTM stands on an operator's
    /// presentation cell whose stay this delivery claimed over the
    /// clearance, nothing departs such a cell later than the operator's
    /// release (simtime), so its path search may not await (scheduler time)
    /// other robots' commits before its first window, and a refused delivery
    /// assigns nothing.
    assignment_on_commit: Option<HtmAssignmentEvent>,
}

impl HtmDeliver {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        station_id: StationId,
        inflight: InflightSpan,
        handoff: HtmCarryingTote,
        clock: Mut<SimTime>,
        navigation: Arc<Navigation>,
        bots: BotService,
        inventory: InventoryService,
        parking: Option<ParkingRegistry>,
        events: EventJournal,
        station_service: Arc<StationService>,
        station_arrivals: StationArrivalDiagnostics,
        order_presentation: Option<CellId>,
        tripwires: bool,
        delivery_max_wait_override: Option<SimDuration>,
    ) -> Self {
        Self {
            station_id,
            inflight,
            handoff: Some(handoff),
            clock,
            presents_order: false,
            assignment_on_commit: None,
            navigation,
            bots,
            inventory,
            parking,
            events,
            station_service,
            station_arrivals,
            order_presentation,
            tripwires,
            delivery_max_wait_override,
        }
    }

    /// Deliver to the station's ORDER presentation point rather than its
    /// presentation cell. At an EasyLoop the two differ: the presentation
    /// cell doubles as the belt's dropoff, and delivering an order carrier
    /// there would board it onto the inventory conveyance.
    pub fn presenting_order(mut self) -> Self {
        self.presents_order = true;
        self
    }

    /// Refuse the delivery rather than leave its HTM waiting on the
    /// presentation cell it departs from: for a claim that superseded that
    /// stay's clearance, which takes the HTM back on a refusal. `assignment`
    /// is announced only once the journey commits.
    pub fn superseding_clearance(mut self, assignment: HtmAssignmentEvent) -> Self {
        self.assignment_on_commit = Some(assignment);
        self
    }

    fn station_cell(&self) -> Result<CellId, PlanError> {
        let cell = if self.presents_order {
            self.order_presentation
        } else {
            self.station_service.presentation_cell(self.station_id)
        };
        cell.ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmDeliver",
            detail: format!(
                "no {} cell for station {:?}",
                if self.presents_order { "order presentation" } else { "presentation" },
                self.station_id
            ),
        })
    }
}

impl fmt::Display for HtmDeliver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HtmDeliver(stn={:?})", self.station_id)
    }
}

#[async_trait(?Send)]
impl Task for HtmDeliver {
    type Proposal = ProposedHtmDeliver;
    type Schedule = Result<HtmCarryingTote, DeliveryRefused>;

    // Nothing to select — the candidacy just publishes the drive's start,
    // and spends nothing.
    async fn propose(&self, _owner: ProposalId) -> Result<ProposedHtmDeliver, PlanError> {
        let h = self.handoff.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmDeliver",
            detail: "proposed after its handoff was consumed".into(),
        })?;
        Ok(ProposedHtmDeliver::new(self.station_id, h.cell_reservation.robot(), h.action_ready_at, self.clock.clone()))
    }

    async fn schedule(
        &mut self,
        proposal: ProposedHtmDeliver,
        _ctx: SchedCtx,
    ) -> Result<Result<HtmCarryingTote, DeliveryRefused>, PlanError> {
        let oracle = &self.navigation.oracle;

        let (
            tote_id,
            zone_id,
            htm_id,
            from_cell,
            from_orientation,
            station_arrival_plan,
            presentation_id,
            source_relay_id,
            deliver_from,
        ) = {
            let u = self.handoff.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmDeliver",
                detail: "scheduled after its handoff was consumed".into(),
            })?;
            (
                u.tote_id,
                u.zone_id,
                u.cell_reservation.robot(),
                u.cell_reservation.cell(),
                u.orientation,
                u.station_arrival_plan,
                u.presentation_id,
                u.source_relay_id,
                u.deliver_from,
            )
        };
        crate::debug_log!("[HtmDeliver] peek cell={:?} htm={:?}", from_cell, htm_id);

        let station_cell = self.station_cell()?;
        // The handoff's action-ready instant reaches here through the
        // candidacy, floored at whatever the barrier had settled by
        // acceptance.
        let depart = proposal.into_accepted()?;

        let predecessor = self
            .handoff
            .as_ref()
            .ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmDeliver",
                detail: "scheduled after its handoff was consumed".into(),
            })?
            .cell_reservation
            .clone();
        let reservation_config = CollisionConfig {
            self_robot: htm_id,
            predecessor: Some(predecessor.clone()),
            destination: None,
            sim_start: depart,
            dest_dwell: None,
            collision_depth: 8,
            window_size: 0,
            replan_interval: 0,
            max_wait: self.delivery_max_wait_override.unwrap_or_else(|| station_delivery_max_wait(self.tripwires)),
            max_iterations: 5_000_000,
            motion_model: self.navigation.model(htm_id)?.kind,
        };
        let drive = self.bots.drive(htm_id, true)?;
        // While another robot holds the station cell, or a queue cell
        // feeding it, open-ended, the pathfinder commits the drive in
        // windows ending in the station's queue, and each goes out as it
        // commits (scheduler time).
        let mut windows = JourneyWindows::new(
            htm_id,
            TaskKind::Deliver,
            Some(tote_id),
            Some(TripStartEvent {
                robot_id: htm_id,
                robot_type: RobotType::Htm,
                purpose: TripPurpose::ToStation,
                presentation_id: Some(presentation_id),
                from: deliver_from,
                to: TripEndpoint::Station(self.station_id),
                sim_time_ms: depart.as_ms(),
            }),
            &self.clock,
            &self.bots,
            &self.inventory,
            self.parking.as_ref(),
            &self.events,
            &self.navigation,
        );
        if let Some(assignment) = self.assignment_on_commit.take() {
            windows = windows.refusing_origin_wait().assigning_on_commit(assignment);
        }
        let reserved = match {
            crate::perf_span!("htm_deliver_reserve_path");
            self.navigation
                .pathfinder
                .reserve_path(
                    from_cell,
                    from_orientation,
                    station_cell,
                    drive.as_ref(),
                    oracle.as_ref(),
                    &reservation_config,
                    &mut windows,
                )
                .await
        } {
            Ok(reserved) => reserved,
            Err(PathfinderError::OriginWaitRefused { origin, held }) => {
                self.inflight.cancel();
                return Ok(Err(DeliveryRefused { origin, held, departure: depart }));
            }
            Err(PathfinderError::CommitConflict { from, to }) => {
                eprintln!(
                    "[HtmDeliver.conflict] htm={:?} from={:?} to={:?} depart={}ms station_cell={:?}",
                    htm_id,
                    from,
                    to,
                    depart.as_ms(),
                    station_cell
                );
                eprintln!(
                    "[HtmDeliver.conflict] station reservations: {:?}",
                    oracle.reservations_for_cell(station_cell)
                );
                eprintln!("[HtmDeliver.conflict] from reservations: {:?}", oracle.reservations_for_cell(from_cell));
                return Err(PathfinderError::CommitConflict { from, to }).err_context("HtmDeliver");
            }
            Err(e) => {
                if matches!(
                    e,
                    PathfinderError::UnreachableWithinHorizon { .. }
                        | PathfinderError::DestinationExceedsMaxWait { .. }
                        | PathfinderError::IterationLimit { .. }
                ) {
                    // The failing search started no earlier than the barrier,
                    // from where the last window ended when one was committed.
                    let (from_cell, from_orientation, start) =
                        windows.resumes_from().unwrap_or((from_cell, from_orientation, reservation_config.sim_start));
                    let depart = start.max(self.clock.value());
                    let horizon = depart + reservation_config.max_wait;
                    eprintln!(
                        "[HtmDeliver.horizon] htm={:?} stn={:?} from={:?} to={:?} depart={}ms horizon={}ms station_status={:?} station_res={:?} station_intervals={:?}",
                        htm_id,
                        self.station_id,
                        from_cell,
                        station_cell,
                        depart.as_ms(),
                        horizon.as_ms(),
                        oracle.cell_status(station_cell),
                        oracle.reservations_for_cell(station_cell),
                        oracle.entry_intervals(station_cell, depart, horizon, None),
                    );
                    match self.navigation.pathfinder.find_path(
                        from_cell,
                        from_orientation,
                        station_cell,
                        self.navigation.model(htm_id)?.estimate[1].as_ref(),
                        depart,
                    ) {
                        Ok(path) => {
                            let mut blocked = Vec::new();
                            let mut edge_blocked = Vec::new();
                            let mut source_blocked = Vec::new();
                            let mut prev_cell = None;
                            for wp in &path.waypoints {
                                if let WaypointAction::Move { start, end, .. } = wp.action {
                                    if let Some(from) = prev_cell {
                                        let dur = end - start;
                                        if oracle.has_edge_conflict(from, wp.cell, start, dur, Some(htm_id)) {
                                            edge_blocked.push((from, wp.cell, start.as_ms(), end.as_ms()));
                                        }
                                        let predecessor = if from == from_cell {
                                            reservation_config.predecessor.as_ref()
                                        } else {
                                            None
                                        };
                                        if oracle.earliest_available(from, start, end - start, predecessor)
                                            != Some(start)
                                        {
                                            source_blocked.push((from, start.as_ms(), end.as_ms()));
                                        }
                                    }
                                } else if matches!(
                                    wp.action,
                                    WaypointAction::Rotate { .. } | WaypointAction::Wait { .. }
                                ) {
                                    let start = wp.start_time();
                                    let end = wp.end_time();
                                    let predecessor = if wp.cell == from_cell {
                                        reservation_config.predecessor.as_ref()
                                    } else {
                                        None
                                    };
                                    if oracle.earliest_available(wp.cell, start, end - start, predecessor)
                                        != Some(start)
                                    {
                                        source_blocked.push((wp.cell, start.as_ms(), end.as_ms()));
                                    }
                                }
                                let t = wp.end_time();
                                if t < depart || t > horizon {
                                    prev_cell = Some(wp.cell);
                                    continue;
                                }
                                let predecessor =
                                    if wp.cell == from_cell { reservation_config.predecessor.as_ref() } else { None };
                                let intervals = oracle.entry_intervals(wp.cell, t, horizon, predecessor);
                                let admitted = intervals
                                    .iter()
                                    .any(|(start, end)| *start <= t && end.map(|end| t < end).unwrap_or(true));
                                if !admitted {
                                    blocked.push((
                                        wp.cell,
                                        t.as_ms(),
                                        intervals.into_iter().take(3).collect::<Vec<_>>(),
                                    ));
                                }
                                if blocked.len() >= 12 {
                                    break;
                                }
                                prev_cell = Some(wp.cell);
                            }
                            eprintln!(
                                "[HtmDeliver.horizon.static] travel={}ms waypoints={} first_blocked={:?} first_edge_blocked={:?} first_source_blocked={:?}",
                                path.travel_time.as_ms(),
                                path.waypoints.len(),
                                blocked,
                                edge_blocked.into_iter().take(12).collect::<Vec<_>>(),
                                source_blocked.into_iter().take(12).collect::<Vec<_>>()
                            );
                        }
                        Err(static_err) => {
                            eprintln!("[HtmDeliver.horizon.static] static find_path failed: {static_err}");
                        }
                    }
                }
                return Err(e).err_context("HtmDeliver");
            }
        };
        // The last window's own start: the departure when the drive was one
        // window, the end of its wait in the queue otherwise.
        let last_start = reserved.path.waypoints.first().map_or(depart, |waypoint| waypoint.start_time());

        let tail = reserved.committed_reservations.last().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmDeliver",
            detail: "reserve_path returned no reservations".into(),
        })?;
        let station_reservation = reserved
            .committed_reservations
            .iter()
            .rev()
            .find(|reservation| reservation.cell() == station_cell)
            .ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmDeliver",
                detail: format!("reserve_path returned no reservation for station cell {:?}", station_cell),
            })?;
        let station_arrival = last_start + reserved.path.travel_time;
        if self.tripwires {
            if let Some((previous_end, gap, prior_count)) =
                self.station_service.gap_before(self.station_id, station_reservation)
            {
                if prior_count >= 3 && gap > STATION_PRESENTATION_GAP_ABORT {
                    return Err(PlanError::TaskGraphInvariant {
                        context: "HtmDeliver",
                        detail: format!(
                            "station presentation gap exceeds diagnostic cap: stn={:?} cell={:?} htm={:?} source={:?} previous_end={} new_start={} actual_arrival={} gap={} cap={} delivery_from={:?} delivery_to={:?} depart={} quoted={:?} target={:?} quoted_relay_pickup={:?} actual_relay_pickup={:?} htm_ready={:?} htm_pickup_from={:?} htm_pickup_to={:?} quoted_htm_to_relay={:?} actual_htm_pickup_travel={:?} actual_htm_pickup_wait={:?}",
                            self.station_id,
                            station_cell,
                            htm_id,
                            station_arrival_plan.map(|plan| plan.source),
                            previous_end,
                            station_reservation.window().start(),
                            station_arrival,
                            gap,
                            STATION_PRESENTATION_GAP_ABORT,
                            from_cell,
                            station_cell,
                            depart,
                            station_arrival_plan.map(|plan| plan.quoted_arrival),
                            station_arrival_plan.map(|plan| plan.target_arrival),
                            station_arrival_plan.and_then(|plan| plan.quoted_relay_pickup_at),
                            station_arrival_plan.and_then(|plan| plan.relay_pickup_at),
                            station_arrival_plan.and_then(|plan| plan.htm_source_ready_at),
                            station_arrival_plan.and_then(|plan| plan.htm_pickup_from),
                            station_arrival_plan.and_then(|plan| plan.htm_pickup_to),
                            station_arrival_plan.and_then(|plan| plan.quoted_htm_to_relay),
                            station_arrival_plan.and_then(|plan| plan.htm_pickup_travel),
                            station_arrival_plan
                                .and_then(|plan| plan.relay_pickup_at.zip(plan.htm_source_ready_at))
                                .map(|(pickup, ready)| pickup - ready),
                        ),
                    });
                }
            }
        }

        // A held assignment no window short of the station announced goes
        // out before the arrival that closes it.
        windows.announce_assignment();
        if let Some(plan) = station_arrival_plan {
            self.station_arrivals.record(plan, htm_id, station_arrival);
            self.events.emit(HtmArrivalEvent {
                robot_id: htm_id,
                station_id: plan.station_id,
                arrival: station_arrival,
            });
        }
        let path = reserved.path;

        self.handoff.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmDeliver",
            detail: "handoff consumed twice".into(),
        })?;

        self.inflight.record_arrival(&path);

        let path_requests = super::commit_plan_requests(&path.waypoints, None);
        windows.commit_task(&[LegSpec {
            leg: Leg::StationApproach,
            requests: &path_requests,
            action_ready_arrival: station_arrival,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation { tote: Some(tote_id), ..BotExpectation::empty() },
            carrying_after: Some(tote_id),
        }])?;

        crate::debug_log!(
            "[HtmDeliver] tail={} cell={:?} htm={:?} open={}",
            tail.id(),
            tail.cell(),
            htm_id,
            tail.is_open_ended()
        );

        windows.trip_start(last_start);
        self.events.emit(NavEventData {
            robot_id: htm_id,
            robot_type: RobotType::Htm,
            waypoints: path.waypoints.clone(),
            time: last_start,
            distance_mm: move_distance(&path, &self.navigation.map),
            carrying_cargo: true,
        });

        Ok(Ok(HtmCarryingTote {
            tote_id,
            zone_id,
            cell_reservation: tail,
            action_ready_at: station_arrival,
            orientation: path.waypoints.last().map(|waypoint| waypoint.orientation).unwrap_or(from_orientation),
            station_arrival_plan,
            presentation_id,
            source_relay_id,
            deliver_from: Some(TripEndpoint::Station(self.station_id)),
        }))
    }
}
