//! A presentation's return drive: an HTM standing at a station with a tote
//! the operator is done with drives to a relay access cell and deposits it
//! there for a VTM to carry back to the rack.
//!
//! Within `task`, `htm_store` chooses nothing — its parent (a presentation's
//! clearance, close-out's store into the buffer, fulfillment's `StoreTote`)
//! picked both the HTM and the relay slot, and spent them, before this task
//! existed. What it owns is WHEN the drive may be planned:
//! [`ProposedHtmStore`] publishes its start to the barrier, and `schedule`
//! waits out the relay access cell locally before it commits the path. A
//! clearance's store may not leave the station's cell after the release, and
//! its path search may not await (scheduler time) commits by robots on its
//! way before its first window is committed; a wait (simtime) that its path
//! plans on the station's cell, behind holds whose ends are already
//! scheduled, is allowed. Once the barrier passes the release before its
//! drive is committed, or its path could leave only after such an await, the
//! run stops with the cause.

use crate::events::{EventJournal, TripEndpoint, TripPurpose, TripStartEvent};
use crate::flows::transport::windows::JourneyWindows;
use crate::flows::transport::{Parking, ParkingOutcome, PathCommitObserver};
use crate::model::ParkingRegistry;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotService, LegSpec};
use crate::model::cell::registry::Vacate;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::relay::RelayService;

use std::fmt;
use std::future::pending;

use super::clearance::{Unparkable, barrier_passes, release_missed};
use crate::error::planner::ClearanceBlock;
use crate::events::ToteSource;
use crate::flows::ttp::handoff::ToteAtRelay;
use crate::model::ToteLocation;
use crate::model::relay::{DEPOSIT_ACCESS_MAX_WAIT, RelayReservation};
use crate::model::reservation::cell_reservation::CellReservation;
use crate::observer::time_slice::ToteVisualState;
use crate::pathfinding::{CollisionConfig, Orientation, PathfinderError, WaypointAction};
use crate::prelude::*;
use desgraph::{CellId, RelayId, StationId};
use desmap::RobotType;
use tokio::select;

// The store leg's admission candidacy. Nothing is selected and nothing is
// spent: the parent claimed this HTM and reserved the relay deposit slot
// before the chain was built, so there is no premise to lose and
// `poisoned` stays false. What this publishes is the drive's physical
// start and, when an incoming journey's need-by asks for it, an earlier
// decision about that already-owned departure.
//
// Whether the relay access cell is free is deliberately NOT on the
// admissible surface. `admissible` may gate only on conditions that
// in-flight work clears WITHOUT a further acceptance, and this one fails
// that test: the cell's holder is often an idle HTM that only moves once
// some other task is accepted — which an inadmissible minimum forbids.
// Since the start self-floors it would sit exactly at B forever and pin
// the whole run. The wait belongs in `schedule`, where it blocks one task
// instead of the clock.
//
// Open question deliberately carried, not settled by this conversion:
// under pull mode a resource ranks its live candidacies by the concrete
// type's own `Ord`, and for store that ranking is genuinely undecided.
// Delivery ranks by arrival at the presentation cell because the CONWIP
// span is defined to end there, and pickup ranks toward meeting the tote
// at the relay. Store could rank by start — free the station cell soonest
// — or by arrival, which is what the zone's deposit timing actually cares
// about. Whoever builds the store family's pull surface has to choose.
pub struct ProposedHtmStore {
    relay_id: RelayId,
    robot: RobotId,
    anchor: SimTime,
    clock: Mut<SimTime>,
    accepted: Option<SimTime>,
    vacate: Option<Vacate>,
}

impl ProposedHtmStore {
    fn new(relay_id: RelayId, robot: RobotId, anchor: SimTime, clock: Mut<SimTime>, vacate: Vacate) -> Self {
        Self { relay_id, robot, anchor, clock, accepted: None, vacate: Some(vacate) }
    }

    /// The departure instant acceptance settled on.
    fn into_accepted(mut self) -> Result<SimTime, PlanError> {
        self.accepted.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmStore",
            detail: "scheduled with a candidacy acceptance never settled".into(),
        })
    }

    fn start(&self) -> SimTime {
        self.anchor.max(self.clock.value())
    }
}

impl fmt::Display for ProposedHtmStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "htm-store{{htm={:?} relay={:?} start={}}}", self.robot, self.relay_id, self.start())
    }
}

impl ProposedReservation for ProposedHtmStore {
    fn resolve_by(&self) -> Option<SimTime> {
        Some(
            self.vacate
                .as_ref()
                .map_or_else(|| self.start(), |vacate| vacate.resolve_by(self.start(), self.clock.value())),
        )
    }

    // Scored on the drive's start. Self-flooring: an accepted chain plans
    // its later legs ahead of the settled frontier, so by the time the
    // barrier reaches this candidacy its anchor can already lie behind B.
    // Reporting the floored instant keeps the claim startable — physically
    // the HTM just stands where it is until then.
    fn start_time(&self) -> Option<SimTime> {
        Some(self.start())
    }

    // `admissible` stays the default true — see the type's comment for why
    // the relay access cell cannot be gated here.
    fn accept(&mut self) -> Result<(), AcceptError> {
        self.accepted = Some(self.start());
        // From acceptance on, planning belongs to schedule. In particular, a
        // blocked relay must not leave a due Vacate pinning the global clock.
        self.vacate.take();
        Ok(())
    }
}

#[derive(Display)]
#[display("HtmStore(stn={:?} htm={:?})", station_id, robot_id)]
pub struct HtmStore {
    commit_observer: Option<PathCommitObserver>,
    pub station_id: StationId,
    robot_id: RobotId,
    tote_id: ToteId,
    predecessor: Option<CellReservation>,
    predecessor_action_ready: SimTime,
    from_orientation: Orientation,
    departs_by: Option<SimTime>,
    deposit: Option<RelayReservation<RackSlot>>,
    presentation_id: Option<PresentationId>,
    tote_source: ToteSource,
    clock: Mut<SimTime>,
    navigation: Arc<Navigation>,
    relay: RelayService,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
}

impl HtmStore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        station_id: StationId,
        robot_id: RobotId,
        tote_id: ToteId,
        predecessor: CellReservation,
        predecessor_action_ready: SimTime,
        deposit: RelayReservation<RackSlot>,
        presentation_id: Option<PresentationId>,
        tote_source: ToteSource,
        clock: Mut<SimTime>,
        navigation: Arc<Navigation>,
        relay: RelayService,
        bots: BotService,
        inventory: InventoryService,
        available: AvailableHtmRegistry,
        parking: Option<ParkingRegistry>,
        events: EventJournal,
    ) -> Self {
        Self {
            commit_observer: None,
            station_id,
            robot_id,
            tote_id,
            predecessor: Some(predecessor),
            predecessor_action_ready,
            from_orientation: Orientation::default(),
            departs_by: None,
            deposit: Some(deposit),
            presentation_id,
            tote_source,
            clock,
            navigation,
            relay,
            bots,
            inventory,
            available,
            parking,
            events,
        }
    }

    /// Observe the exact committed departure, before deposit or event work.
    pub fn with_commit_observer(mut self, observer: PathCommitObserver) -> Self {
        self.commit_observer = Some(observer);
        self
    }

    /// Preserve a curved path's tangent after the preceding station dwell.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.from_orientation = orientation;
        self
    }

    /// Leave the station's cell no later than `departs_by` (simtime), a
    /// clearance's release, or stop the run.
    pub fn departing_by(mut self, departs_by: Option<SimTime>) -> Self {
        self.departs_by = departs_by;
        self
    }

    fn cannot_depart(&self, departure: SimTime, cause: ClearanceBlock) -> PlanError {
        PlanError::ClearanceCannotDepart { station: self.station_id, robot: self.robot_id, departure, cause }
    }

    fn deposit_relay_id(&self) -> Result<RelayId, PlanError> {
        self.deposit.as_ref().map(|d| d.relay_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmStore",
            detail: "read after its relay reservation was consumed".into(),
        })
    }
}

impl Drop for HtmStore {
    fn drop(&mut self) {
        if let Some(deposit) = self.deposit.take() {
            deposit.cancel();
        }
    }
}

#[async_trait(?Send)]
impl Task for HtmStore {
    type Proposal = ProposedHtmStore;
    type Schedule = ToteAtRelay;

    // Nothing to select — the candidacy just publishes the drive's start,
    // and spends nothing.
    async fn propose(&self, owner: ProposalId) -> Result<ProposedHtmStore, PlanError> {
        let relay = self.deposit_relay_id()?;
        let source = self.predecessor.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmStore",
            detail: "proposed after its source hold was consumed".into(),
        })?;
        let vacate = self.navigation.cells.vacate(source, owner, self.predecessor_action_ready);
        Ok(ProposedHtmStore::new(relay, self.robot_id, self.predecessor_action_ready, self.clock.clone(), vacate))
    }

    async fn schedule(&mut self, proposal: ProposedHtmStore, ctx: SchedCtx) -> Result<ToteAtRelay, PlanError> {
        let commit_observer = self.commit_observer.take();
        let oracle = &self.navigation.oracle;
        // The presentation's action-ready instant reaches here through the
        // candidacy, floored at whatever the barrier had settled by
        // acceptance.
        let mut depart = proposal.into_accepted()?.max(self.clock.value());

        let (deposit_relay_id, deposit_rack_slot, slot_start) = {
            let deposit = self.deposit.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmStore",
                detail: "scheduled after its relay reservation was consumed".into(),
            })?;
            (deposit.relay_id, deposit.recommendation, deposit.start)
        };

        let relay_cell = self.relay.htm_access_cell(deposit_relay_id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmStore",
            detail: format!("no HTM access cell for relay {:?}", deposit_relay_id),
        })?;

        // The predecessor hold stays in `self` until the path commits, so a
        // failed reserve_path leaves this task's own state describing what
        // it still holds.
        let predecessor = self
            .predecessor
            .as_ref()
            .ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "HtmStore",
                detail: "scheduled after its predecessor hold was consumed".into(),
            })?
            .clone();
        let from_cell = predecessor.cell();

        crate::debug_log!("[HtmStore] {:?} reserving {:?} → {:?} at {}", self.robot_id, from_cell, relay_cell, depart);
        let mut reservation_config = CollisionConfig {
            self_robot: self.robot_id,
            predecessor: Some(predecessor),
            destination: None,
            sim_start: depart,
            dest_dwell: None,
            collision_depth: 8,
            window_size: 0,
            replan_interval: 0,
            max_wait: DEPOSIT_ACCESS_MAX_WAIT,
            max_iterations: 5_000_000,
            motion_model: self.navigation.model(self.robot_id)?.kind,
        };
        let drive = self.bots.drive(self.robot_id, true)?;
        let mut windows = JourneyWindows::new(
            self.robot_id,
            TaskKind::Store,
            Some(self.tote_id),
            Some(TripStartEvent {
                robot_id: self.robot_id,
                robot_type: RobotType::Htm,
                purpose: TripPurpose::ToReturnRelay,
                presentation_id: self.presentation_id,
                from: Some(TripEndpoint::Station(self.station_id)),
                to: TripEndpoint::Relay(deposit_relay_id),
                sim_time_ms: depart.as_ms(),
            }),
            &self.clock,
            &self.bots,
            &self.inventory,
            self.parking.as_ref(),
            &self.events,
            &self.navigation,
        )
        .observed_by(commit_observer);
        if self.departs_by.is_some() {
            windows = windows.refusing_origin_wait();
        }
        let mut unparkable = Unparkable::default();
        let reserved = loop {
            if let Some(release) = self.departs_by
                && self.clock.value() > release
            {
                let cause = release_missed(self.relay.htm_access_holder(deposit_relay_id), self.robot_id);
                return Err(self.cannot_depart(release, cause));
            }
            depart = depart.max(self.clock.value());
            reservation_config.sim_start = depart;
            // The gate on the relay access cell: another robot holding it
            // admits no arrival, so wait for that robot's next leg to bound
            // the hold. Local on purpose: see ProposedHtmStore's comment.
            match self.relay.htm_access_holder(deposit_relay_id) {
                Some(blocking_robot) if blocking_robot != self.robot_id => {
                    crate::debug_log!(
                        "[HtmStore.wait] {:?} relay={:?} cell={:?} blocked by {:?}",
                        self.robot_id,
                        deposit_relay_id,
                        relay_cell,
                        blocking_robot
                    );
                    // A relay may have become an idle destination after this
                    // store selected it. Let productive claims compete before
                    // requesting a physical relocation of its empty resident.
                    //
                    // The oracle's version is read before the parking: a
                    // failed parking commits no path, so it cannot wake the
                    // await below itself, and a path that other work commits
                    // while the parking is decided (scheduler time), which
                    // may free a bay, wakes it.
                    let oracle_version = oracle.version();
                    let oracle_seen = oracle_version.map(Version::seen);
                    unparkable.as_of(oracle_seen);
                    if let Some(parking) = &self.parking
                        && let Some(entry) = self.available.squatter(relay_cell, blocking_robot)
                        && let Some(parking_departure) = unparkable.untried_departure(&entry, self.clock.value())
                    {
                        let (stay, hold) = (entry.availability_id(), entry.cell_reservation.clone());
                        let task = Parking::new(
                            entry,
                            self.navigation.clone(),
                            self.bots.clone(),
                            self.inventory.clone(),
                            self.available.clone(),
                            Some(parking.clone()),
                            self.events.clone(),
                            self.clock.clone(),
                        )?;
                        // A clearance's store needs the squatter's parking
                        // decided before the barrier passes the release,
                        // however late its stay starts, and back at that
                        // decision (scheduler time) when it finds no bay, so
                        // that the await below sees the barrier pass the
                        // release. Such a stay is not parked again until a
                        // path commits or the barrier passes the departure
                        // its failed parking was decided for (`Unparkable`):
                        // its take and give-back would wake every other such
                        // store to park its own squatter again, without end.
                        let outcome = match self.departs_by {
                            Some(release) => select! {
                                biased;
                                outcome = ctx.add_task(task.without_retry()) => outcome,
                                never = self.navigation.cells.keep_need_by(&hold, release) => match never {},
                            },
                            None => ctx.add_task(task).await,
                        };
                        match outcome {
                            ParkingOutcome::Unavailable if self.departs_by.is_some() => {
                                unparkable.insert(stay, parking_departure);
                            }
                            ParkingOutcome::Unavailable => {}
                            ParkingOutcome::Parked | ParkingOutcome::Superseded => continue,
                        }
                    }
                    // Failed relief restores availability and bumps its version.
                    // Observe after that attempt so our own take/restore cannot
                    // wake this wait forever. Recheck a holder changed while awaiting.
                    let fleet_seen = self.available.version().seen();
                    let relay_changed = notified(self.relay.signal());
                    if self.relay.htm_access_holder(deposit_relay_id) != Some(blocking_robot) {
                        continue;
                    }
                    select! {
                        biased;
                        _ = self.available.version().past(fleet_seen) => {}
                        _ = relay_changed => {}
                        _ = async {
                            match oracle_version.zip(oracle_seen) {
                                Some((version, seen)) => version.past(seen).await,
                                None => pending().await,
                            }
                        } => {}
                        passed = async {
                            match self.departs_by {
                                Some(release) => barrier_passes(ctx.barrier(), release).await,
                                None => pending().await,
                            }
                        } => passed?,
                    }
                    continue;
                }
                _ => {}
            }
            match self
                .navigation
                .pathfinder
                .reserve_path(
                    from_cell,
                    self.from_orientation,
                    relay_cell,
                    drive.as_ref(),
                    oracle.as_ref(),
                    &reservation_config,
                    &mut windows,
                )
                .await
            {
                Ok(r) => break r,
                Err(PathfinderError::OriginWaitRefused { .. }) if let Some(release) = self.departs_by => {
                    return Err(self.cannot_depart(release, ClearanceBlock::WayHeld));
                }
                Err(e) => {
                    if matches!(e, PathfinderError::UnreachableWithinHorizon { .. }) {
                        dump_horizon_miss(
                            &self.navigation,
                            self.robot_id,
                            self.station_id,
                            deposit_relay_id,
                            from_cell,
                            self.from_orientation,
                            relay_cell,
                            depart,
                            &reservation_config,
                        );
                    }
                    return Err(PlanError::TaskGraphInvariant {
                        context: "HtmStore",
                        detail: format!(
                            "reserve_path: {:?} {:?} → {:?} at {}: {}",
                            self.robot_id, from_cell, relay_cell, depart, e
                        ),
                    });
                }
            }
        };
        windows.observe_last(&reserved.path)?;
        self.predecessor.take();

        let tail = reserved.committed_reservations.last().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmStore",
            detail: "reserve_path returned no reservations".into(),
        })?;
        let path = reserved.path;
        let start = path.waypoints.first().map_or(depart, |waypoint| waypoint.start_time());
        // The departure from the station's cell is bounded; the journey's
        // later windows start whenever they do. The path search plans from a
        // start (simtime) no later than the release and refuses to await
        // (scheduler time) before its first window, so that window starts no
        // later.
        if let Some(release) = self.departs_by
            && let Some(departure) = windows.departure(&path).filter(|&departure| departure > release)
        {
            return Err(PlanError::TaskGraphInvariant {
                context: "HtmStore",
                detail: format!("a clearance's store committed a departure at {departure}, after {release}"),
            });
        }

        let path_requests = super::commit_plan_requests(&path.waypoints, None);
        let arrival = start + path.travel_time;
        // Deposit begins at the later of HTM arrival and the slot becoming
        // valid; the forks then take `htm_deposit_time` to set the tote down.
        // FIXME(pathfinding): "arrive early, wait at the cell" is not physically
        // accurate — a laden HTM cannot enter the access cell while the relay
        // slot is occupied; entry should be gated by slot_start too. Needs
        // reserve_path to gain an arrive-after constraint (see #153).
        let deposit_start = arrival.max(slot_start);
        let deposit_done = deposit_start + self.bots.model(self.robot_id)?.figures.deposit_time();
        windows.commit_task(&[LegSpec {
            leg: Leg::RelayDeposit,
            requests: &path_requests,
            action_ready_arrival: deposit_done,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation { tote: Some(self.tote_id), ..BotExpectation::empty() },
            carrying_after: None,
        }])?;

        self.inventory
            .tote(self.tote_id)?
            .move_to(ToteLocation::AtRelay { relay_id: deposit_relay_id }, deposit_done)?;

        self.available.register(
            tail,
            deposit_done,
            path.waypoints.last().map(|waypoint| waypoint.orientation).unwrap_or(self.from_orientation),
            None,
        )?;

        let robot_id = self.robot_id;
        {
            use super::move_distance;
            use crate::events::*;
            windows.trip_start(start);
            self.events.emit(NavEventData {
                robot_id,
                robot_type: RobotType::Htm,
                waypoints: path.waypoints.clone(),
                time: start,
                distance_mm: move_distance(&path, &self.navigation.map),
                carrying_cargo: true,
            });
            let relay_htm_cell = self.relay.htm_access_cell(deposit_relay_id).map(|c| c.0).unwrap_or(0);
            self.events.emit(ToteMovedEvent {
                tote_id: self.tote_id,
                time: deposit_done,
                new_state: ToteVisualState::AtRelay { cell_id: relay_htm_cell },
            });
            // The fork-Put itself, always: an operation happened whether or not
            // the bot waited for the slot. `wait` is the stretch of the leg
            // after the D30 reached the access cell, so it never overlaps the
            // leg's Travel journal rows, and it is zero when the slot was
            // valid on arrival.
            self.events.emit(HtmRelayDepositEvent {
                robot_id,
                relay_id: deposit_relay_id,
                start: deposit_start,
                end: deposit_done,
                wait: deposit_start - arrival,
            });
        }

        let deposit = self.deposit.take().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "HtmStore",
            detail: "relay reservation consumed twice".into(),
        })?;
        Ok(ToteAtRelay {
            tote_id: self.tote_id,
            relay_id: deposit_relay_id,
            rack_slot: deposit_rack_slot,
            reservation: deposit,
            deposit_time: deposit_done,
            presentation_id: self.presentation_id,
            tote_source: self.tote_source,
        })
    }
}

// The blame dump for a deposit leg that found no plan within its wait
// horizon: the oracle's view of both endpoints, then per-cell entry
// intervals along the static path, printed before the run halts.
#[allow(clippy::too_many_arguments)]
fn dump_horizon_miss(
    navigation: &Navigation,
    robot_id: RobotId,
    station_id: StationId,
    relay_id: RelayId,
    from_cell: CellId,
    from_orientation: Orientation,
    relay_cell: CellId,
    depart: SimTime,
    config: &CollisionConfig,
) {
    let oracle = &navigation.oracle;
    eprintln!(
        "[HtmStore.horizon] htm={:?} station={:?} relay={:?} from={:?} to={:?} depart={}ms",
        robot_id,
        station_id,
        relay_id,
        from_cell,
        relay_cell,
        depart.as_ms()
    );
    eprintln!(
        "[HtmStore.horizon] from_status={:?} to_status={:?}",
        oracle.cell_status(from_cell),
        oracle.cell_status(relay_cell)
    );
    eprintln!("[HtmStore.horizon] to_res={:?}", oracle.reservations_for_cell(relay_cell));
    let Ok(model) = navigation.model(robot_id) else {
        eprintln!("[HtmStore.horizon] robot has no model binding");
        return;
    };
    match navigation.pathfinder.find_path(from_cell, from_orientation, relay_cell, model.estimate[1].as_ref(), depart) {
        Ok(path) => {
            eprintln!(
                "[HtmStore.horizon] static_path travel={}ms waypoints={}",
                path.travel_time.as_ms(),
                path.waypoints.len()
            );
            let horizon = depart + config.max_wait;
            let mut last_cell = None;
            for wp in &path.waypoints {
                let is_path_cell = matches!(
                    wp.action,
                    WaypointAction::PathStart { .. } | WaypointAction::Move { .. } | WaypointAction::PathEnd { .. }
                );
                if !is_path_cell || last_cell == Some(wp.cell) {
                    continue;
                }
                last_cell = Some(wp.cell);
                let intervals = oracle.entry_intervals(wp.cell, depart, horizon, config.predecessor.as_ref());
                let status = oracle.cell_status(wp.cell);
                eprintln!("[HtmStore.horizon.cell] cell={:?} status={:?} intervals={:?}", wp.cell, status, intervals);
            }
        }
        Err(static_err) => {
            eprintln!("[HtmStore.horizon] static_path_err={static_err}");
        }
    }
}

#[cfg(test)]
#[path = "htm_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "htm_store_release_tests.rs"]
mod release_tests;
