//! Move an already assigned HTM between floor cells, preserving its tote and
//! facing. The parent keeps the robot after arrival and decides its next work.

use std::convert::Infallible;
use std::fmt;
use std::future::pending;
use std::sync::Arc;

use desgraph::{CellId, RobotType};

use super::{commit_plan_requests, move_distance};
use crate::events::{EventJournal, NavEventData, TripEndpoint, TripPurpose, TripStartEvent};
use crate::flows::transport::windows::{JourneyWindows, PathCommitObserver};
use crate::model::ParkingRegistry;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotService, LegSpec};
use crate::model::cell::registry::{NeedBy, Vacate};
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::pathfinding::{CollisionConfig, Orientation, PathCacheKey, PathfinderError};
use crate::prelude::*;

pub struct HtmPosition {
    pub cell_reservation: CellReservation,
    pub action_ready_at: SimTime,
    pub orientation: Orientation,
}

pub struct ProposedNavigate {
    robot: RobotId,
    target: CellId,
    anchor: SimTime,
    clock: Mut<SimTime>,
    accepted: Option<SimTime>,
    vacate: Option<Vacate>,
}

impl ProposedNavigate {
    fn start(&self) -> SimTime {
        self.anchor.max(self.clock.value())
    }
}

impl fmt::Display for ProposedNavigate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "navigate{{htm={:?} to={:?} start={}}}", self.robot, self.target, self.start())
    }
}

impl ProposedReservation for ProposedNavigate {
    fn resolve_by(&self) -> Option<SimTime> {
        Some(
            self.vacate
                .as_ref()
                .map_or_else(|| self.start(), |vacate| vacate.resolve_by(self.start(), self.clock.value())),
        )
    }

    fn start_time(&self) -> Option<SimTime> {
        Some(self.start())
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        self.accepted = Some(self.start());
        // The accepted journey now owns planning; a blocked destination must
        // not leave a due Vacate pinning the global barrier.
        self.vacate.take();
        Ok(())
    }
}

pub struct Navigate {
    commit_observer: Option<PathCommitObserver>,
    position: Option<HtmPosition>,
    target: CellId,
    destinations: Option<Vec<(CellId, TripEndpoint)>>,
    carrying: Option<ToteId>,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    clock: Mut<SimTime>,
    trip: Option<NavigationTrip>,
    not_before: SimTime,
}

impl fmt::Display for Navigate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.destinations {
            Some(destinations) => write!(f, "Navigate(one of {} destinations)", destinations.len()),
            None => write!(f, "Navigate(to={:?})", self.target),
        }
    }
}

struct NavigationTrip {
    purpose: TripPurpose,
    presentation_id: Option<PresentationId>,
    destination: TripEndpoint,
}

impl Navigate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        position: HtmPosition,
        target: CellId,
        carrying: Option<ToteId>,
        navigation: Arc<Navigation>,
        bots: BotService,
        inventory: InventoryService,
        parking: Option<ParkingRegistry>,
        events: EventJournal,
        clock: Mut<SimTime>,
    ) -> Self {
        Self {
            commit_observer: None,
            position: Some(position),
            target,
            destinations: None,
            carrying,
            navigation,
            bots,
            inventory,
            parking,
            events,
            clock,
            trip: None,
            not_before: SimTime::ZERO,
        }
    }

    /// Observe the exact committed route before any further task work. A
    /// same-cell journey has no route commit and drops the observer unused.
    pub fn with_commit_observer(mut self, observer: PathCommitObserver) -> Self {
        self.commit_observer = Some(observer);
        self
    }

    /// Keeps a need-by on the `Vacate` of the hold another robot has on
    /// `target`, open-ended, for as long as this journey's `reserve_path` is
    /// pending (scheduler time): on each change to the holds or to the cells'
    /// registry, it answers again once the need-by it holds is no longer live,
    /// so a Vacate registered after the call began, or a new holder's, is
    /// answered too. It never completes; dropping it when the call ends
    /// withdraws the need-by.
    async fn keep_target_need_by(
        &self,
        position: &HtmPosition,
        target: CellId,
        depart: SimTime,
    ) -> Result<Infallible, PlanError> {
        let mut need_by: Option<NeedBy> = None;
        loop {
            let version = self.navigation.oracle.version();
            let seen = version.map(Version::seen);
            let cells_seen = self.navigation.cells.changed().seen();
            if need_by.as_ref().is_none_or(|need_by| !need_by.is_live()) {
                need_by = self.need_destination_vacated(position, depart.max(self.clock.value()), [target])?;
            }
            let holds_changed = async {
                match version.zip(seen) {
                    Some((version, seen)) => version.past(seen).await,
                    None => pending().await,
                }
            };
            tokio::select! {
                _ = holds_changed => {},
                _ = self.navigation.cells.changed().past(cells_seen) => {},
            }
        }
    }

    /// Needs one holder decided before the barrier advances (a need-by at the
    /// barrier's simtime): of the other robots with a `Vacate` pending that
    /// hold open-ended a cell of `cells` this robot can reach, the one that
    /// ranks first by the later of this robot's empty-floor arrival there and
    /// the time the holder is ready to depart (both simtime), ties to the lower
    /// cell. The need-by owns no departure task or robot assignment.
    fn need_destination_vacated(
        &self,
        position: &HtmPosition,
        depart: SimTime,
        cells: impl IntoIterator<Item = CellId>,
    ) -> Result<Option<NeedBy>, PlanError> {
        let mut chosen = None;
        for cell in cells {
            let holds: Vec<_> = self
                .navigation
                .oracle
                .reservations_for_cell(cell)
                .into_iter()
                .filter(|hold| hold.robot() != position.cell_reservation.robot() && hold.is_open_ended())
                .collect();
            if holds.is_empty() {
                continue;
            }
            let travel = match self.navigation.cache(position.cell_reservation.robot())?.travel_time(PathCacheKey {
                from: position.cell_reservation.cell(),
                from_orientation: position.orientation,
                to: cell,
                carrying_cargo: self.carrying.is_some(),
            }) {
                Ok(travel) => travel,
                Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            for hold in holds {
                if let Some(vacating) = self.navigation.cells.vacating(&hold) {
                    let rank = ((depart + travel).max(vacating.at), cell);
                    if chosen.as_ref().is_none_or(|(best, _)| rank < *best) {
                        chosen = Some((rank, vacating));
                    }
                }
            }
        }
        Ok(chosen.and_then(|(_, vacating)| self.navigation.cells.need_by(vacating, self.clock.value())))
    }

    /// Declare the work this leg approaches. The parent knows its semantic
    /// destination; generic floor movement does not infer one from a cell.
    pub fn with_trip(
        mut self,
        purpose: TripPurpose,
        presentation_id: Option<PresentationId>,
        destination: TripEndpoint,
    ) -> Self {
        self.trip = Some(NavigationTrip { purpose, presentation_id, destination });
        self
    }

    /// Choose among equivalent destinations when the robot can actually
    /// depart. The supplied endpoints also identify the resulting trip.
    pub fn with_destinations(mut self, destinations: Vec<(CellId, TripEndpoint)>) -> Self {
        self.destinations = Some(destinations);
        self
    }

    /// Delay this new journey without changing when the previous action
    /// actually finished. Collision routing may still require a later departure.
    pub fn not_before(mut self, at: SimTime) -> Self {
        self.not_before = at;
        self
    }

    /// The journey's target and its departure (simtime). For a target a
    /// finite hold already bounds far ahead, the departure moves toward that
    /// clearance, so it need not spend the search's horizon waiting at the
    /// source, and alternatives are chosen again at the later departure. A
    /// finite hold, not the open-ended one the pathfinder awaits; the
    /// pathfinder applies the same rule after its own await, and each only
    /// moves a start later, so the later of the two stands.
    async fn choose_target(
        &self,
        position: &HtmPosition,
        mut depart: SimTime,
        max_wait: SimDuration,
    ) -> Result<(CellId, Option<TripEndpoint>, SimTime), PlanError> {
        let from = position.cell_reservation.cell();
        loop {
            let (target, destination) = match &self.destinations {
                Some(destinations) => {
                    let (cell, endpoint, chosen_at) =
                        self.await_free_destination(position, depart, destinations).await?;
                    depart = chosen_at;
                    (cell, Some(endpoint))
                }
                None => (self.target, None),
            };
            // The oracle's actual trailing free interval, including any
            // required gap after the preceding hold.
            let clearance = (from != target)
                .then(|| self.navigation.oracle.entry_intervals(target, depart, SimTime::EOT, None))
                .into_iter()
                .flatten()
                .find_map(|(start, end)| end.is_none().then_some(start.max(depart)));
            if let Some(clearance) = clearance.filter(|at| *at >= depart + max_wait) {
                let travel = self.navigation.cache(position.cell_reservation.robot())?.travel_time(PathCacheKey {
                    from,
                    from_orientation: position.orientation,
                    to: target,
                    carrying_cargo: self.carrying.is_some(),
                })?;
                if travel < max_wait && clearance.saturating_sub(travel) > depart {
                    depart = clearance.saturating_sub(travel);
                    if self.destinations.is_some() {
                        continue;
                    }
                }
            }
            return Ok((target, destination, depart));
        }
    }

    /// The alternative that frees first, and the departure (simtime) it is
    /// chosen at. The pathfinder awaits one target's holder, so the choice
    /// among alternatives awaits here, and meanwhile a need-by asks for one
    /// holder's next task decided.
    async fn await_free_destination(
        &self,
        position: &HtmPosition,
        mut depart: SimTime,
        destinations: &[(CellId, TripEndpoint)],
    ) -> Result<(CellId, TripEndpoint, SimTime), PlanError> {
        let oracle = &self.navigation.oracle;
        let mut need_by: Option<NeedBy> = None;
        loop {
            // The await below lets the barrier pass this departure.
            depart = depart.max(self.clock.value());
            let version = oracle.version();
            let seen = version.map(Version::seen);
            let cells_seen = self.navigation.cells.changed().seen();
            if let Some((cell, endpoint)) = self.available_destination(position, depart, destinations)? {
                return Ok((cell, endpoint, depart));
            }
            if need_by.as_ref().is_some_and(|need_by| !need_by.is_live()) {
                need_by = None;
            }
            if need_by.is_none() {
                need_by =
                    self.need_destination_vacated(position, depart, destinations.iter().map(|(cell, _)| *cell))?;
            }
            let Some((version, seen)) = version.zip(seen) else {
                return Err(PlanError::TaskGraphInvariant {
                    context: "Navigate",
                    detail: "occupied destinations have no reservation change notification".into(),
                });
            };
            tokio::select! {
                _ = version.past(seen) => {},
                _ = self.navigation.cells.changed().past(cells_seen) => {},
            }
        }
    }

    fn available_destination(
        &self,
        position: &HtmPosition,
        after: SimTime,
        destinations: &[(CellId, TripEndpoint)],
    ) -> Result<Option<(CellId, TripEndpoint)>, PlanError> {
        if destinations.is_empty() {
            return Err(PlanError::TaskGraphInvariant {
                context: "Navigate",
                detail: "destination alternatives are empty".into(),
            });
        }
        let robot = position.cell_reservation.robot();
        let cache = self.navigation.cache(robot)?;
        let mut reachable = false;
        let mut selected = None;
        for &(cell, endpoint) in destinations {
            let travel = if cell == position.cell_reservation.cell() {
                SimDuration::ZERO
            } else {
                match cache.travel_time(PathCacheKey {
                    from: position.cell_reservation.cell(),
                    from_orientation: position.orientation,
                    to: cell,
                    carrying_cargo: self.carrying.is_some(),
                }) {
                    Ok(travel) => travel,
                    Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => continue,
                    Err(error) => return Err(error.into()),
                }
            };
            reachable = true;
            let holds = self.navigation.oracle.reservations_for_cell(cell);
            if holds.iter().any(|hold| hold.robot() != robot && hold.is_open_ended()) {
                continue;
            }
            let available = holds
                .iter()
                .filter(|hold| hold.robot() != robot)
                .filter_map(|hold| hold.window().end())
                .max()
                .unwrap_or(after);
            let rank = ((after + travel).max(available), cell);
            if selected.as_ref().is_none_or(|(previous, _)| rank < *previous) {
                selected = Some((rank, endpoint));
            }
        }
        if !reachable {
            return Err(PlanError::TaskGraphInvariant {
                context: "Navigate",
                detail: "no reachable destination alternative".into(),
            });
        }
        Ok(selected.map(|((_, cell), endpoint)| (cell, endpoint)))
    }

    fn position(&self) -> Result<&HtmPosition, PlanError> {
        self.position.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "Navigate",
            detail: "robot position was already handed to the next task".into(),
        })
    }
}

#[async_trait(?Send)]
impl Task for Navigate {
    type Proposal = ProposedNavigate;
    type Schedule = HtmPosition;

    async fn propose(&self, owner: ProposalId) -> Result<ProposedNavigate, PlanError> {
        let position = self.position()?;
        let anchor = position.action_ready_at.max(self.not_before);
        let from = position.cell_reservation.cell();
        let leaves_source = self.destinations.as_ref().map_or(from != self.target, |destinations| {
            !destinations.is_empty() && destinations.iter().all(|(cell, _)| *cell != from)
        });
        Ok(ProposedNavigate {
            robot: position.cell_reservation.robot(),
            target: self.target,
            anchor,
            clock: self.clock.clone(),
            accepted: None,
            vacate: leaves_source.then(|| self.navigation.cells.vacate(&position.cell_reservation, owner, anchor)),
        })
    }

    async fn schedule(&mut self, proposal: ProposedNavigate, _: SchedCtx) -> Result<HtmPosition, PlanError> {
        let depart = proposal
            .accepted
            .ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "Navigate",
                detail: "departure was not accepted".into(),
            })?
            .max(self.clock.value());
        let commit_observer = self.commit_observer.take();
        let position = self.position()?;
        let robot = position.cell_reservation.robot();
        let from = position.cell_reservation.cell();
        let from_orientation = position.orientation;
        let mut orientation = from_orientation;
        let mut tail = position.cell_reservation.clone();
        let mut requests = Vec::new();
        let mut path = None;

        let oracle = &self.navigation.oracle;
        let max_wait = SimDuration::seconds(900);
        let (target, destination, depart) = self.choose_target(position, depart, max_wait).await?;
        // A journey to the robot's own cell arrives as it departs.
        let mut arrival = depart;
        let config = if from == target {
            None
        } else {
            Some(CollisionConfig {
                self_robot: robot,
                predecessor: Some(position.cell_reservation.clone()),
                destination: None,
                sim_start: depart,
                dest_dwell: None,
                collision_depth: 8,
                window_size: 0,
                replan_interval: 0,
                max_wait,
                max_iterations: 5_000_000,
                motion_model: self.navigation.model(robot)?.kind,
            })
        };
        let trip = self.trip.as_ref().map(|trip| TripStartEvent {
            robot_id: robot,
            robot_type: RobotType::Htm,
            purpose: trip.purpose,
            presentation_id: trip.presentation_id,
            from: None,
            to: destination.unwrap_or(trip.destination),
            sim_time_ms: depart.as_ms(),
        });
        let mut windows = JourneyWindows::new(
            robot,
            TaskKind::Navigate,
            self.carrying,
            trip,
            &self.clock,
            &self.bots,
            &self.inventory,
            self.parking.as_ref(),
            &self.events,
            &self.navigation,
        )
        .observed_by(commit_observer);
        let reserved = match config {
            Some(config) => {
                let drive = self.bots.drive(robot, self.carrying.is_some())?;
                let journey = self.navigation.pathfinder.reserve_path(
                    from,
                    from_orientation,
                    target,
                    drive.as_ref(),
                    oracle.as_ref(),
                    &config,
                    &mut windows,
                );
                // The pathfinder awaits the target's holder; meanwhile a
                // need-by asks for the holder's next task decided. Polled
                // first, so the need-by stands before the first search.
                tokio::select! {
                    biased;
                    kept = self.keep_target_need_by(position, target, depart) => match kept? {},
                    reserved = journey => Some(reserved?),
                }
            }
            None => None,
        };
        let mut last_start = depart;
        if let Some(reserved) = reserved {
            windows.observe_last(&reserved.path)?;
            let endpoint = reserved.path.waypoints.last().ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "Navigate",
                detail: "pathfinder returned no destination waypoint".into(),
            })?;
            tail = reserved.committed_reservations.last().cloned().ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "Navigate",
                detail: "pathfinder committed no destination reservation".into(),
            })?;
            if endpoint.cell != target || tail.cell() != target {
                return Err(PlanError::TaskGraphInvariant {
                    context: "Navigate",
                    detail: format!("path ended outside target {:?}", target),
                });
            }
            last_start = reserved.path.waypoints.first().map_or(depart, |waypoint| waypoint.start_time());
            arrival = last_start + reserved.path.travel_time;
            orientation = endpoint.orientation;
            requests = commit_plan_requests(&reserved.path.waypoints, None);
            path = Some(reserved.path);
        }

        windows.commit_task(&[LegSpec {
            leg: Leg::Travel,
            requests: &requests,
            action_ready_arrival: arrival,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation { tote: self.carrying, ..BotExpectation::empty() },
            carrying_after: self.carrying,
        }])?;
        windows.trip_start(last_start);
        drop(windows);
        self.position.take();

        if let Some(path) = path {
            self.events.emit(NavEventData {
                robot_id: robot,
                robot_type: RobotType::Htm,
                distance_mm: move_distance(&path, &self.navigation.map),
                waypoints: path.waypoints,
                time: last_start,
                carrying_cargo: self.carrying.is_some(),
            });
        }
        Ok(HtmPosition { cell_reservation: tail, action_ready_at: arrival, orientation })
    }
}

#[cfg(test)]
#[path = "navigate_tests.rs"]
mod tests;
