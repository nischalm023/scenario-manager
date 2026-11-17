//! Within `activity`, the `log` keeps each robot's milestones from the moment
//! their facts are read until B has passed them and they are built, with what
//! building them carries forward, reads a robot's work as B stands, and names
//! the robots an advance of B has anything to settle for.

use std::collections::BTreeSet;
use std::mem;

use rustc_hash::FxHashMap;

use desgraph::CellId;

use crate::events::TripEndpoint;
use crate::model::{RobotId, SimTime};
use crate::observer::duty::BusyInterval;
use crate::observer::state::SimObserverState;

use super::visits::StationVisits;
use super::{ActivityKind, LegNav, Milestone};

/// One robot's milestones, split by whether B has passed them, and what has
/// been built from those it has.
#[derive(Debug, Default)]
pub(super) struct RobotActivity {
    /// Reported, but not yet behind B.
    pub(super) pending: Vec<Milestone>,
    /// Behind B but not yet built, in `Milestone::order`. Between settlements
    /// this is at most the open mark, waiting for whatever closes it.
    pub(super) marks: Vec<Milestone>,
    /// The latest interval built, which the next one must not overlap.
    pub(super) last: Option<BusyInterval>,
    /// The intervals a slice whose distribution or duty is still to come can
    /// read, in order.
    pub(super) intervals: Vec<BusyInterval>,
    pub(super) carrying: bool,
    /// What the bot holds from each instant a leg departed or a fork
    /// operation ended, from the latest at or before `gap_cursor` on.
    pub(super) carrying_since: Vec<(SimTime, bool)>,
    pub(super) gap_cursor: SimTime,
    due: Option<SimTime>,
}

impl RobotActivity {
    /// What B has to pass before this robot has something to settle: its
    /// earliest pending milestone, or the end of the span it reported last.
    pub(super) fn next_due(&self) -> Option<SimTime> {
        let open_span = match self.marks.last() {
            Some(Milestone::Span { end, .. }) => Some(*end),
            _ => None,
        };
        self.pending.iter().map(Milestone::at).chain(open_span).min()
    }

    /// The robot's work from `from` on as B stands, in order: the intervals
    /// built, then what the open mark says of the time after it, as far as
    /// what is reported allows. A leg runs to its nav's arrival, and on to the
    /// operation it was sent for from when that operation is reported; a
    /// visit runs until a departure closes it; a span is its own extent. Work
    /// nothing has closed yet runs to the end of time.
    pub(super) fn work_from(&self, from: SimTime) -> impl Iterator<Item = BusyInterval> + '_ {
        let first = self.intervals.partition_point(|interval| interval.end <= from);
        self.intervals[first..].iter().copied().chain(self.open_work())
    }

    fn open_work(&self) -> impl Iterator<Item = BusyInterval> {
        let travel = |start, end| BusyInterval { kind: ActivityKind::Travel, start, end };
        let (work, stand) = match self.marks.first() {
            Some(Milestone::Departure { at, nav, .. }) => {
                let arrival = nav.map_or(*at, |nav| nav.arrival);
                let next = self.pending.iter().min_by_key(|milestone| milestone.order());
                let stand = next.and_then(|operation| operation.claims_stand_from(arrival));
                (Some(travel(*at, arrival)), stand.map(|from| travel(from, SimTime::ms(u64::MAX))))
            }
            Some(Milestone::Arrival { at, .. }) => {
                (Some(BusyInterval { kind: ActivityKind::Presentation, start: *at, end: SimTime::ms(u64::MAX) }), None)
            }
            Some(Milestone::Span { kind, start, end, .. }) => {
                (Some(BusyInterval { kind: *kind, start: *start, end: *end }), None)
            }
            None => (None, None),
        };
        work.into_iter().chain(stand).filter(|work| work.end > work.start)
    }
}

#[derive(Debug, Default)]
pub(in crate::observer) struct ActivityLog {
    pub(super) robots: FxHashMap<RobotId, RobotActivity>,
    // ordered: by the instant each robot falls due, so an advance of B takes
    // the robots it settles off the front and visits no other.
    due: BTreeSet<(SimTime, RobotId)>,
    pub(super) visits: StationVisits,
    /// Slices before this one have their distribution.
    pub(super) distribution_upto: usize,
    /// Slices before this one have their D30 duty.
    pub(super) duty_upto: usize,
    pub(super) world_time: SimTime,
}

impl ActivityLog {
    /// Have `robot` settled once B passes `at`, unless it is due sooner.
    pub(super) fn due_at(&mut self, robot: RobotId, at: SimTime) {
        let state = self.robots.entry(robot).or_default();
        if state.due.is_some_and(|due| due <= at) {
            return;
        }
        if let Some(stale) = state.due.replace(at) {
            self.due.remove(&(stale, robot));
        }
        self.due.insert((at, robot));
    }

    pub(super) fn take_due(&mut self, floor: SimTime) -> Vec<RobotId> {
        let later = self.due.split_off(&(floor, RobotId(0)));
        let mut robots: Vec<RobotId> = mem::replace(&mut self.due, later).into_iter().map(|(_, r)| r).collect();
        for robot in &robots {
            if let Some(state) = self.robots.get_mut(robot) {
                state.due = None;
            }
        }
        // ordered: robot id, so journal rows are appended in a run-independent order.
        robots.sort();
        robots
    }
}

impl SimObserverState {
    /// The current world time as this observer has heard it: the simtime the
    /// latest advance of B read from the stream reached, read as of the
    /// observer's place in the stream, which follows the scheduler's order.
    /// No fact read after it may carry an earlier simtime.
    pub(in crate::observer) fn current_world_time(&self) -> SimTime {
        self.activity.world_time
    }

    pub(in crate::observer) fn mark_activity(&mut self, robot: RobotId, milestone: Milestone) {
        self.activity.visits.follow(robot, &milestone);
        self.activity.robots.entry(robot).or_default().pending.push(milestone);
        self.activity.due_at(robot, milestone.at());
    }

    /// Give a leg its nav. A `TripStart` reported just before the nav opened
    /// the leg; a nav no `TripStart` announced, as the fulfillment tasks move
    /// totes between cells, opens its own.
    pub(in crate::observer) fn report_leg(
        &mut self,
        robot: RobotId,
        depart: SimTime,
        leg_nav: LegNav,
        origin: Option<CellId>,
    ) {
        let announced = self.activity.robots.get_mut(&robot).and_then(|state| {
            state.pending.iter_mut().rev().find_map(|milestone| match milestone {
                Milestone::Departure { at, nav: slot @ None, .. } if *at == depart => Some(slot),
                _ => None,
            })
        });
        if let Some(slot) = announced {
            *slot = Some(leg_nav);
            return;
        }
        let from = self.last_trip_to.get(&robot).copied().unwrap_or(TripEndpoint::Parking);
        let leg = Milestone::Departure {
            at: depart,
            nav: Some(leg_nav),
            leaves_visit: false,
            from,
            from_cell: origin,
            known_from: self.current_world_time(),
        };
        self.mark_activity(robot, leg);
    }
}
