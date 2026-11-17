//! Within `observer`, `robot_state` is WHAT EACH ROBOT IS DOING OVER EACH SLICE, as the facts read by the time the
//! slice is made final commit it: the producing half of the recording's robot state channel, in the channel's own
//! states.
//!
//! **What it is for.** A chart of how the fleet spends its time must be final with the slice it reads, and a viewer
//! starting at any keyframe must know what every robot is doing there. So each slice states every robot's spans
//! inside it from what the scheduler had committed when B passed the slice's end, and nothing committed later is
//! written back into a slice already stated.
//!
//! **How a slice is stated.** The handlers note each fact as they read it: a span the fact commits (a nav's travel
//! runs and route holds, a fork operation, a relay wait its drive commits with the operation, a DA50's run along its
//! rail), the operator's work at a station, a station visit from its arrival to the departure that closes it, a nav
//! stamped before it departs, and the role of the cell each nav leaves a D30 standing on. Over a slice, a committed
//! span claims its time first, the operator's work next (a putaway's can begin while the bot still travels to it),
//! then the stand before work outside any visit, from the instant that work's fact is stamped at, then a visit's wait
//! for the operator's work, then the stand before a nav departs, from the instant the nav is stamped at. Time nothing
//! claims is no task, on the cell the robot stands on: the gap between its tasks, the stand after the operator's work
//! up to the instant its next nav is stamped at, a resident D30's stand before its pickup, a DA50's wait before its
//! cycle. Whether such a robot has no work, or work
//! it cannot be given yet, no fact says while B passes it.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rustc_hash::FxHashMap;
use worldview::frames::robot_state::{CellRole, RobotState, WaitCause};

use desgraph::RobotType;

use crate::model::{RobotId, SimDuration, SimTime};
use crate::observer::output::HtmWaitPosition;

/// One robot's spans over one slice, laid end to end from the slice's start to its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RobotSliceSpans {
    pub robot: RobotId,
    /// Whether the first span is the one the robot ended the slice before in.
    pub continues: bool,
    pub spans: Vec<(SimDuration, RobotState)>,
}

/// How strongly a noted span claims a robot's time against one it overlaps; a visit claims what none does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Claim {
    Committed,
    OperatorWork,
    /// The stand where a nav left the robot, before the operator's work that nav brought it for.
    AwaitedWork,
}

/// Two spans committed for one robot over one instant: the facts contradict each other, and the channel cannot state
/// both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommittedSpansOverlap {
    pub robot: RobotId,
    /// The span noted first, as `(start, end, state)`.
    pub first: (SimTime, SimTime, RobotState),
    pub second: (SimTime, SimTime, RobotState),
}

/// Where a stretch of a robot's time takes its state from, so the next slice can tell a span it carries on from a
/// new one in the same state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Noted(u64),
    Visit { visit: u64, served: bool },
    Departure(u64),
    Gap,
}

#[derive(Debug)]
struct Noted {
    start: SimTime,
    end: SimTime,
    state: RobotState,
    claim: Claim,
    id: u64,
}

#[derive(Debug)]
struct Visit {
    arrival: SimTime,
    close: Option<SimTime>,
    /// When the operator's work in it ends, once that work is read.
    served_until: Option<SimTime>,
    cell: Option<CellRole>,
    id: u64,
}

/// The stand from the instant a nav was read to its departure: the move is committed and has not begun.
#[derive(Debug)]
struct Departure {
    read_at: SimTime,
    departs: SimTime,
    id: u64,
}

/// One robot's facts from the earliest instant a slice still to be stated reaches.
#[derive(Debug, Default)]
struct RobotFacts {
    noted: Vec<Noted>,
    visits: Vec<Visit>,
    departures: Vec<Departure>,
    /// The role of the cell each nav leaves the robot standing on, from the instant the nav ends, in time order.
    stands: Vec<(SimTime, Option<CellRole>)>,
    /// The stretch the robot ended the last stated slice in.
    last: Option<(Source, RobotState)>,
}

/// Every robot's facts until the slices they reach are stated.
#[derive(Debug, Default)]
pub(super) struct RobotStateLog {
    robots: FxHashMap<RobotId, RobotFacts>,
    next_id: u64,
    /// The first two committed spans noted over one instant of one robot's time, until the observer reads it.
    overlap: Option<CommittedSpansOverlap>,
}

/// The role the channel gives a cell the observer placed a robot on, none for one the map does not place.
pub(super) fn role_of(position: HtmWaitPosition) -> Option<CellRole> {
    match position {
        HtmWaitPosition::Charging => Some(CellRole::Charging),
        HtmWaitPosition::Presentation => Some(CellRole::Presentation),
        HtmWaitPosition::StationQueue => Some(CellRole::StationQueue),
        HtmWaitPosition::Relay => Some(CellRole::Relay),
        HtmWaitPosition::Route => Some(CellRole::Route),
        HtmWaitPosition::Unknown => None,
    }
}

impl RobotStateLog {
    /// A fact commits `robot` to `state` over `[start, end)`. A committed span over an instant another committed span
    /// already covers is kept as the overlap the observer stops the run on.
    pub(super) fn note(&mut self, robot: RobotId, (start, end): (SimTime, SimTime), state: RobotState, claim: Claim) {
        if end <= start {
            return;
        }
        let id = self.id();
        let noted = &mut self.robots.entry(robot).or_default().noted;
        if claim == Claim::Committed && self.overlap.is_none() {
            let clash =
                noted.iter().find(|held| held.claim == Claim::Committed && held.start < end && start < held.end);
            self.overlap = clash.map(|held| CommittedSpansOverlap {
                robot,
                first: (held.start, held.end, held.state),
                second: (start, end, state),
            });
        }
        noted.push(Noted { start, end, state, claim, id });
    }

    /// The first two committed spans noted over one instant of a robot's time, if any were.
    pub(super) fn take_overlap(&mut self) -> Option<CommittedSpansOverlap> {
        self.overlap.take()
    }

    /// The operator's work outside any visit begins at `start`, read with B at `read_at`: from then, or from
    /// where the robot's last nav left it if that is later, it stands awaiting the work.
    pub(super) fn await_work(&mut self, robot: RobotId, start: SimTime, read_at: SimTime) {
        let stands = &self.robots.entry(robot).or_default().stands;
        let standing = stands.partition_point(|(since, _)| *since <= start).checked_sub(1).map(|index| stands[index]);
        if let Some((since, cell)) = standing {
            let awaiting = RobotState::Waiting { cause: WaitCause::AwaitingStationService, cell };
            self.note(robot, (since.max(read_at), start), awaiting, Claim::AwaitedWork);
        }
    }

    /// A nav that departs at `departs` was read with B at `read_at`: from then the robot's next move is committed,
    /// and it stands awaiting the departure wherever it stands.
    pub(super) fn await_departure(&mut self, robot: RobotId, read_at: SimTime, departs: SimTime) {
        if departs > read_at {
            let id = self.id();
            self.robots.entry(robot).or_default().departures.push(Departure { read_at, departs, id });
        }
    }

    /// From `since` on, `robot` stands on a cell of role `cell`, until a nav moves it again.
    pub(super) fn stand(&mut self, robot: RobotId, since: SimTime, cell: Option<CellRole>) {
        let stands = &mut self.robots.entry(robot).or_default().stands;
        let at = stands.partition_point(|(from, _)| *from <= since);
        stands.insert(at, (since, cell));
    }

    pub(super) fn open_visit(&mut self, robot: RobotId, arrival: SimTime, cell: Option<CellRole>) {
        let id = self.id();
        let visits = &mut self.robots.entry(robot).or_default().visits;
        visits.push(Visit { arrival, close: None, served_until: None, cell, id });
    }

    /// The operator's work in `robot`'s open visit that arrived at `arrival` ends at `until`.
    pub(super) fn serve_visit(&mut self, robot: RobotId, arrival: SimTime, until: SimTime) {
        if let Some(visit) = self.open_visit_mut(robot, arrival) {
            visit.served_until = Some(visit.served_until.map_or(until, |served| served.max(until)));
        }
    }

    pub(super) fn close_visit(&mut self, robot: RobotId, arrival: SimTime, close: SimTime) {
        if let Some(visit) = self.open_visit_mut(robot, arrival) {
            visit.close = Some(close);
        }
    }

    /// Forget the open visit that arrived at `arrival`: it holds the robot for no time.
    pub(super) fn discard_visit(&mut self, robot: RobotId, arrival: SimTime) {
        if let Some(facts) = self.robots.get_mut(&robot) {
            facts.visits.retain(|visit| visit.arrival != arrival || visit.close.is_some());
        }
    }

    /// Each robot's spans over `[from, to)`, for every robot of `fleet` and every robot a fact names, in ascending
    /// id, leaving out a robot that carries one span on across the whole slice. The facts the slice used up are
    /// forgotten, so each slice is stated once and in order.
    pub(super) fn state_slice(
        &mut self,
        from: SimTime,
        to: SimTime,
        fleet: &HashMap<RobotId, RobotType>,
    ) -> Vec<RobotSliceSpans> {
        let mut robots: Vec<RobotId> = fleet.keys().chain(self.robots.keys()).copied().collect();
        // ordered: robot id, so a slice names its robots in a run-independent order.
        robots.sort_unstable();
        robots.dedup();
        let mut named = Vec::new();
        for robot in robots {
            let on_rail = fleet.get(&robot) == Some(&RobotType::Vtm);
            let facts = self.robots.entry(robot).or_default();
            named.extend(facts.state_slice(robot, on_rail, from, to));
            facts.forget_before(to);
        }
        named
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn open_visit_mut(&mut self, robot: RobotId, arrival: SimTime) -> Option<&mut Visit> {
        let visits = self.robots.get_mut(&robot)?.visits.iter_mut();
        visits.into_iter().find(|visit| visit.arrival == arrival && visit.close.is_none())
    }
}

impl RobotFacts {
    fn state_slice(&mut self, robot: RobotId, on_rail: bool, from: SimTime, to: SimTime) -> Option<RobotSliceSpans> {
        let mut cuts = vec![from, to];
        for noted in self.noted.iter().filter(|noted| noted.start < to && noted.end > from) {
            cuts.extend([noted.start, noted.end]);
        }
        for visit in self.visits.iter().filter(|visit| visit.arrival < to && visit.close.is_none_or(|c| c > from)) {
            cuts.extend([Some(visit.arrival), visit.close, visit.served_until].into_iter().flatten());
        }
        cuts.extend(self.departures.iter().flat_map(|departure| [departure.read_at, departure.departs]));
        cuts.extend(self.stands.iter().map(|(since, _)| *since));
        cuts.retain(|cut| (from..=to).contains(cut));
        cuts.sort_unstable();
        cuts.dedup();

        let mut stretches: Vec<(SimTime, SimTime, Source, RobotState)> = Vec::new();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let (source, state) = self.at(start, on_rail);
            match stretches.last_mut() {
                Some(last) if (last.2, last.3) == (source, state) => last.1 = end,
                _ => stretches.push((start, end, source, state)),
            }
        }
        let (first, last) = (stretches.first()?, stretches.last()?);
        let continues = self.last == Some((first.2, first.3));
        self.last = Some((last.2, last.3));
        if continues && stretches.len() == 1 {
            return None;
        }
        let spans = stretches.iter().map(|(start, end, _, state)| (*end - *start, *state)).collect();
        Some(RobotSliceSpans { robot, continues, spans })
    }

    /// What the robot is doing at `t`, and where that comes from.
    fn at(&self, t: SimTime, on_rail: bool) -> (Source, RobotState) {
        let covering = self.noted.iter().filter(|noted| noted.start <= t && t < noted.end);
        if let Some(noted) = covering.min_by_key(|noted| (noted.claim, noted.id)) {
            return (Source::Noted(noted.id), noted.state);
        }
        let visits = self.visits.iter().filter(|visit| visit.arrival <= t && visit.close.is_none_or(|c| t < c));
        let visit = visits.max_by_key(|visit| visit.arrival);
        let served = visit.is_some_and(|visit| visit.served_until.is_some_and(|until| until <= t));
        let cell = match visit {
            Some(visit) if !served => {
                let awaiting = RobotState::Waiting { cause: WaitCause::AwaitingStationService, cell: visit.cell };
                return (Source::Visit { visit: visit.id, served }, awaiting);
            }
            Some(visit) => visit.cell,
            None => {
                let standing = self.stands.partition_point(|(since, _)| *since <= t).checked_sub(1);
                standing.and_then(|index| self.stands[index].1).filter(|_| !on_rail)
            }
        };
        let departing = self.departures.iter().filter(|departure| departure.read_at <= t && t < departure.departs);
        if let Some(departure) = departing.min_by_key(|departure| departure.id) {
            let awaiting = RobotState::Waiting { cause: WaitCause::AwaitingDeparture, cell };
            return (Source::Departure(departure.id), awaiting);
        }
        let source = visit.map_or(Source::Gap, |visit| Source::Visit { visit: visit.id, served });
        (source, RobotState::NoTask { cell })
    }

    /// Forget what no slice from `to` on can read: spans, visits and departures over by then, and every stand but the
    /// one the robot is on at `to`.
    fn forget_before(&mut self, to: SimTime) {
        self.noted.retain(|noted| noted.end > to);
        self.visits.retain(|visit| visit.close.is_none_or(|close| close > to));
        self.departures.retain(|departure| departure.departs > to);
        let in_force = self.stands.partition_point(|(since, _)| *since <= to).saturating_sub(1);
        self.stands.drain(..in_force);
    }
}
