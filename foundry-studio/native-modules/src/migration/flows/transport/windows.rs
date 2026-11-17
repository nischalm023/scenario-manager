//! A task's side of `reserve_path`'s windows: what a robot's journey reports
//! for each window of its path the pathfinder commits short of the target, as
//! it commits (scheduler time), so the robot's plan, the facts of that window
//! and what its observer reads off it go out before the barrier can pass its
//! start, and a picked-up tote's inbound unit rides the hold the window ends
//! in. The task commits the journey's last window itself, through
//! [`JourneyWindows::commit_task`], [`JourneyWindows::trip_start`] and
//! [`JourneyWindows::observe_last`], so the whole journey is one task and one
//! trip whatever the number of windows.

use std::error::Error;

use pathfinder::WindowSink;

use super::{commit_plan_requests, move_distance};
use crate::error::PlanError;
use crate::events::{EventJournal, HtmAssignmentEvent, NavEventData, TripStartEvent};
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::bot::LegSpec;
use crate::model::bot::plan::{BotExpectation, Leg, PlanTaskId, TaskKind, WaypointKind};
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::{RobotId, SimTime, ToteId};
use crate::pathfinding::{Orientation, Path, ReservedPath};
use crate::signals::Mut;
use desgraph::CellId;
use desgraph::types::RobotType;

/// Which window of a route an observer is handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteWindow {
    /// A window the pathfinder ended short of the target, on a queue cell.
    ShortOfTarget,
    /// The window that ends on the target: the route's last.
    Last,
}

/// Called with each window of a route as it is committed, the last included.
/// An error halts planning; it does not roll back the committed route.
pub type PathCommitObserver = Box<dyn FnMut(&Path, RouteWindow) -> Result<(), PlanError>>;

/// One journey's windows: who travels, as which task, carrying what, and the
/// trip its first window announces.
pub struct JourneyWindows<'a> {
    robot: RobotId,
    kind: TaskKind,
    carrying: Option<ToteId>,
    /// The trip this journey makes, announced at its first window's start;
    /// `None` for a journey that announces none.
    trip: Option<TripStartEvent>,
    clock: &'a Mut<SimTime>,
    bots: &'a BotService,
    inventory: &'a InventoryService,
    parking: Option<&'a ParkingRegistry>,
    events: &'a EventJournal,
    /// The map the windows' distances are read off, and the pickup units
    /// the windows move.
    navigation: &'a Navigation,
    /// The task the journey's first window opened, which later windows
    /// extend.
    task: Option<PlanTaskId>,
    /// The last committed window's end cell, facing and arrival: where the
    /// journey's next window starts.
    resumes_from: Option<(CellId, Orientation, SimTime)>,
    /// The first committed window's start: the journey's departure from its
    /// origin (simtime).
    departure: Option<SimTime>,
    observer: Option<PathCommitObserver>,
    /// Whether the robot may wait (simtime) on the journey's origin while no
    /// window can be committed (`WindowSink::allows_origin_wait`).
    allows_origin_wait: bool,
    /// The HTM's assignment, held until the journey commits: for a journey
    /// that may be refused before it commits anything, so a refused one
    /// assigns nothing.
    assignment: Option<HtmAssignmentEvent>,
}

impl<'a> JourneyWindows<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        robot: RobotId,
        kind: TaskKind,
        carrying: Option<ToteId>,
        trip: Option<TripStartEvent>,
        clock: &'a Mut<SimTime>,
        bots: &'a BotService,
        inventory: &'a InventoryService,
        parking: Option<&'a ParkingRegistry>,
        events: &'a EventJournal,
        navigation: &'a Navigation,
    ) -> Self {
        Self {
            robot,
            kind,
            carrying,
            trip,
            clock,
            bots,
            inventory,
            parking,
            events,
            navigation,
            task: None,
            resumes_from: None,
            departure: None,
            observer: None,
            allows_origin_wait: true,
            assignment: None,
        }
    }

    /// Hand every window of the route to `observer` as it commits; the task
    /// hands it the last through [`Self::observe_last`].
    pub fn observed_by(mut self, observer: Option<PathCommitObserver>) -> Self {
        self.observer = observer;
        self
    }

    /// Where the journey's next window starts, once a window short of the
    /// target is committed: its end cell, facing and arrival (simtime).
    pub fn resumes_from(&self) -> Option<(CellId, Orientation, SimTime)> {
        self.resumes_from
    }

    /// The journey's departure from its origin (simtime): its first window's
    /// start, `last`'s when no window short of the target was committed.
    pub fn departure(&self, last: &Path) -> Option<SimTime> {
        self.departure.or_else(|| last.waypoints.first().map(|waypoint| waypoint.start_time()))
    }

    /// A journey that may not await (scheduler time) other robots' commits
    /// with nothing committed: where `reserve_path` would, it returns
    /// `OriginWaitRefused`, and the stay on the origin stays the task's to
    /// end.
    pub fn refusing_origin_wait(mut self) -> Self {
        self.allows_origin_wait = false;
        self
    }

    /// Hold the HTM's `assignment` until the journey commits: a window short
    /// of the target announces it before its trip, and the task announces it
    /// through [`Self::announce_assignment`] once the last window is found.
    pub fn assigning_on_commit(mut self, assignment: HtmAssignmentEvent) -> Self {
        self.assignment = Some(assignment);
        self
    }

    /// Announce the held assignment, unless a window already announced it.
    pub fn announce_assignment(&mut self) {
        if let Some(assignment) = self.assignment.take() {
            self.events.emit(assignment);
        }
    }

    /// Hand the window that ends on the target to the observer.
    pub fn observe_last(&mut self, path: &Path) -> Result<(), PlanError> {
        self.observer.as_mut().map_or(Ok(()), |observer| observer(path, RouteWindow::Last))
    }

    /// Commit the journey's last [`LegSpec`]s: into the task its first window opened,
    /// or as the task when the journey was one window.
    pub fn commit_task(&mut self, legs: &[LegSpec<'_>]) -> Result<(), PlanError> {
        match self.task {
            Some(task) => self.bots.extend_task(self.robot, task, legs, self.inventory, self.parking),
            None => {
                self.task = Some(self.bots.commit_task(self.robot, self.kind, legs, self.inventory, self.parking)?);
                Ok(())
            }
        }
    }

    /// Announce the trip at `at`, unless a window already announced it.
    pub fn trip_start(&mut self, at: SimTime) {
        if let Some(trip) = self.trip.take() {
            self.events.emit(TripStartEvent { sim_time_ms: at.as_ms(), ..trip });
        }
    }

    fn report(&mut self, window: &ReservedPath, closed: Option<&CellReservation>) -> Result<(), PlanError> {
        // The unit moves before anything this window reports can read the
        // zone's headroom; the window onto the target releases it.
        if let (Some(closed), Some(queue_hold)) = (closed, window.committed_reservations.last()) {
            self.navigation.pickup_units.follow(closed, queue_hold);
        }
        let waypoints = &window.path.waypoints;
        let (Some(first), Some(last)) = (waypoints.first(), waypoints.last()) else {
            return Err(PlanError::TaskGraphInvariant {
                context: "JourneyWindows",
                detail: "a committed window has no waypoints".into(),
            });
        };
        let (start, arrival) = (first.start_time(), last.end_time());
        self.departure.get_or_insert(start);
        self.resumes_from = Some((last.cell, last.orientation, arrival));
        if let Some(observer) = &mut self.observer {
            observer(&window.path, RouteWindow::ShortOfTarget)?;
        }
        let requests = commit_plan_requests(waypoints, None);
        self.commit_task(&[LegSpec {
            leg: Leg::Travel,
            requests: &requests,
            action_ready_arrival: arrival,
            wp_kind: WaypointKind::Travel,
            expect: BotExpectation { tote: self.carrying, ..BotExpectation::empty() },
            carrying_after: self.carrying,
        }])?;
        self.announce_assignment();
        self.trip_start(start);
        self.events.emit(NavEventData {
            robot_id: self.robot,
            robot_type: RobotType::Htm,
            waypoints: waypoints.clone(),
            time: start,
            distance_mm: move_distance(&window.path, &self.navigation.map),
            carrying_cargo: self.carrying.is_some(),
        });
        Ok(())
    }
}

impl WindowSink for JourneyWindows<'_> {
    fn barrier(&self) -> SimTime {
        self.clock.value()
    }

    fn committed(
        &mut self,
        window: &ReservedPath,
        closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.report(window, closed).map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
    }

    fn allows_origin_wait(&self) -> bool {
        self.allows_origin_wait
    }
}

/// The side of `reserve_path` for a journey that is always one window: its
/// target has no queue (a parking cell), its caller skips a target whose
/// queue is held (`LoopCollect`), or its caller's retry restarts from the
/// origin and so cannot resume a window (`HtmPickup`). A window short of the
/// target is a defect there, refused by name. The barrier is the journal's:
/// the B every fact the journey emits is checked against.
pub struct OneWindowJourney<'a> {
    pub task: &'static str,
    pub events: &'a EventJournal,
}

impl WindowSink for OneWindowJourney<'_> {
    fn barrier(&self) -> SimTime {
        self.events.barrier()
    }

    fn committed(
        &mut self,
        window: &ReservedPath,
        _closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let end = window.path.waypoints.last().map(|waypoint| waypoint.cell);
        Err(format!("{}'s journey is one window, and it stopped short on {end:?}", self.task).into())
    }
}
