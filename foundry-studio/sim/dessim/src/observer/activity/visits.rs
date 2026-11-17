//! Within `activity`, `visits` keeps a robot's station visits: what each one
//! is (the cycle it serves, its station, the cell it holds the bot on), which
//! reported operator dwell belongs to which visit, and the hold rows a visit
//! journals when it closes. It hands the robot state channel each visit and
//! each report of the operator's work as it reads them.

use std::collections::VecDeque;

use rustc_hash::FxHashMap;
use worldview::frames::robot_state::RobotState;

use desgraph::{CellId, StationId, StationType};

use crate::events::TripEndpoint;
use crate::model::ids::PresentationId;
use crate::model::{RobotId, SimTime};
use crate::observer::output::HtmWaitCause;
use crate::observer::robot_state::{Claim, role_of};
use crate::observer::state::SimObserverState;

use super::{ActivityKind, ActivityViolation, Milestone};

/// Operator dwell recorded at `CycleComplete`, consumed when the station visit
/// that contains it closes. `presented` stores both resolved endpoints: pick
/// stamps the dwell start; putaway stamps `fill_at`, its end.
///
/// `containment` distinguishes the timestamp contracts. A pick dwell is a
/// real occupancy window wholly inside the visit. Putaway's `dwell_ms` is
/// also its pacing quantity, so `[fill_at - dwell, fill_at)` can begin before
/// the robot arrived; only its end must lie inside the visit.
///
/// `presentation_id` is the cycle the dwell belongs to, carried through so
/// the journal row it produces names the same trip as the legs around it.
#[derive(Debug, Clone, Copy)]
pub(in crate::observer) struct PendingDwell {
    pub(in crate::observer) presented: (SimTime, SimTime),
    pub(in crate::observer) containment: DwellContainment,
    pub(in crate::observer) presentation_id: Option<PresentationId>,
}

/// Presentation attribution read when a station visit's arrival is reported.
/// Offline planning can report a later presentation's future-stamped
/// `TripStart` before the visit closes, so close-time attribution must not
/// read the mutable `current_trip`.
#[derive(Debug, Clone, Copy)]
struct VisitContext {
    presentation_id: Option<PresentationId>,
    station: Option<TripEndpoint>,
    cell: Option<CellId>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::observer) enum DwellContainment {
    FullWindow,
    EndOnly,
}

#[derive(Debug, Default)]
pub(super) struct StationVisits {
    /// One entry per open station visit, keyed by the robot and the arrival
    /// that opened it: visits open as their arrivals are read and close once
    /// B has passed what ends them, so a slot per robot would keep only the
    /// newest.
    open: FxHashMap<(RobotId, SimTime), VisitContext>,
    /// Dwells captured on `CycleComplete`, consumed by the station visit that
    /// contains them. Queued per robot because the offline planner can report
    /// a later cycle's future-stamped completion before an earlier visit
    /// closes, and joined by containment rather than by the order the facts
    /// are read.
    pub(super) pending_dwell: FxHashMap<RobotId, VecDeque<PendingDwell>>,
    /// The arrival that opened each robot's latest station visit reported,
    /// until a departure after it is reported: a dwell reported in between
    /// belongs to that visit.
    latest_arrival: FxHashMap<RobotId, SimTime>,
}

impl StationVisits {
    pub(super) fn follow(&mut self, robot: RobotId, milestone: &Milestone) {
        match *milestone {
            Milestone::Arrival { at, .. } => {
                self.latest_arrival.insert(robot, at);
            }
            // A delivery reports its own inbound departure after its arrival;
            // only a departure after the arrival leaves the visit.
            Milestone::Departure { at, .. } if self.latest_arrival.get(&robot).is_some_and(|&arrival| at > arrival) => {
                self.latest_arrival.remove(&robot);
            }
            _ => {}
        }
    }
}

impl SimObserverState {
    /// Record an operator dwell. The visit the robot's reports last opened
    /// claims a dwell inside it when the visit closes; a dwell outside any
    /// visit, as the fulfillment tasks report their station work, is itself
    /// the robot's work at the station.
    pub(in crate::observer) fn mark_dwell(&mut self, robot: RobotId, dwell: PendingDwell) {
        let anchor = match dwell.containment {
            DwellContainment::FullWindow => dwell.presented.0,
            DwellContainment::EndOnly => dwell.presented.1,
        };
        let visit = self.activity.visits.latest_arrival.get(&robot).copied().filter(|&arrival| anchor >= arrival);
        self.note_operator_work(robot, dwell.presented, visit);
        if visit.is_some() {
            self.activity.visits.pending_dwell.entry(robot).or_default().push_back(dwell);
        } else {
            let (start, end) = dwell.presented;
            let work = Milestone::Span {
                kind: ActivityKind::Presentation,
                start,
                end,
                carries_after: None,
                resident_since: None,
                known_from: self.current_world_time(),
            };
            self.mark_activity(robot, work);
        }
    }

    /// State the operator's work in the robot state channel: inside the visit
    /// that arrived at `visit`, from the bot's arrival on, since a putaway's
    /// work can begin while the bot still travels to it. With no visit, as
    /// the fulfillment tasks report their station work, all of it, and the
    /// stand before it, where the nav that brought the bot here left it, is
    /// the wait for it, as a visit's is.
    fn note_operator_work(&mut self, robot: RobotId, (start, end): (SimTime, SimTime), visit: Option<SimTime>) {
        let start = visit.map_or(start, |arrival| start.max(arrival));
        self.robot_state_log.note(robot, (start, end), RobotState::StationProcessing, Claim::OperatorWork);
        match visit {
            Some(arrival) => self.robot_state_log.serve_visit(robot, arrival, end),
            None => self.robot_state_log.await_work(robot, start, self.current_world_time()),
        }
    }

    pub(in crate::observer) fn open_station_visit(&mut self, robot: RobotId, station_id: StationId, arrival: SimTime) {
        let trip = self.current_trip.get(&robot).copied();
        let cell = self.config.station_cells.get(&station_id).copied();
        self.activity.visits.open.insert(
            (robot, arrival),
            VisitContext {
                presentation_id: trip.and_then(|ctx| ctx.presentation_id),
                station: Some(TripEndpoint::Station(station_id)),
                cell,
            },
        );
        let role = cell.and_then(|cell| role_of(self.hold_position(cell)));
        self.robot_state_log.open_visit(robot, arrival, role);
    }

    /// The end of the operator dwell recorded for `robot`'s open station
    /// visit, if one was recorded. Peeked, not consumed: the visit is closed
    /// with it and `close_station_visit` still has to take it.
    pub(super) fn recorded_dwell_end(&self, robot: RobotId, arrival: SimTime) -> Option<SimTime> {
        // The visit has no close, so any dwell ending at or after the arrival
        // is one it could contain.
        self.activity
            .visits
            .pending_dwell
            .get(&robot)
            .and_then(|queue| queue.iter().find(|d| d.presented.1 >= arrival))
            .map(|d| d.presented.1)
    }

    /// Close one station visit `[open, close)`, splitting it at the operator
    /// dwell the `CycleComplete` fact recorded. `close` is the departure that
    /// ends the visit or, when the run stopped inside it with no departure
    /// scheduled, the current world time, before which a departure scheduled
    /// later could not start, or the dwell's end if that is later. The dwell
    /// must lie inside the visit; a dwell that escapes it is a modeling bug and
    /// fails here, where the visit is known.
    pub(super) fn close_station_visit(
        &mut self,
        robot: RobotId,
        open: SimTime,
        close: SimTime,
        closed_into_departure: bool,
        carrying_cargo: bool,
    ) -> Result<(), ActivityViolation> {
        let mut visit = self.activity.visits.open.remove(&(robot, open));
        self.robot_state_log.close_visit(robot, open, close);
        let visit_station = visit.as_ref().and_then(|ctx| match ctx.station {
            Some(TripEndpoint::Station(id)) => Some(id),
            _ => None,
        });
        let contained = |d: &PendingDwell| match d.containment {
            DwellContainment::FullWindow => open <= d.presented.0 && d.presented.1 <= close,
            DwellContainment::EndOnly => open <= d.presented.1 && d.presented.1 <= close,
        };
        let pending = self.activity.visits.pending_dwell.get_mut(&robot).and_then(|queue| {
            let index = queue.iter().position(contained)?;
            queue.remove(index)
        });
        if self.activity.visits.pending_dwell.get(&robot).is_some_and(|queue| queue.is_empty()) {
            self.activity.visits.pending_dwell.remove(&robot);
        }
        if let (Some(ctx), Some(dwell)) = (visit.as_mut(), pending.as_ref()) {
            ctx.presentation_id = dwell.presentation_id;
        }
        // Ruling #49's tripwire. A dwell that overlaps this visit without
        // falling inside it escaped the visit that produced it; one wholly
        // outside belongs to another visit. Checked whether or not this visit
        // found its own dwell, so an escaping one queued behind it is caught.
        let escaped = self.activity.visits.pending_dwell.get(&robot).and_then(|queue| {
            queue.iter().find(|d| d.presented.0 < close && d.presented.1 > open).map(|d| d.presented)
        });
        if let Some(dwell) = escaped {
            return Err(ActivityViolation::DwellEscapesVisit { robot, dwell, visit: (open, close) });
        }
        let Some(PendingDwell { presented, .. }) = pending else {
            // No dwell is legitimate on the end-of-run seam: a stop (sim-time
            // cap, wall-timeout, UI) cancels chains between the HtmDeliver
            // commit, which already reported the arrival, and the Presentation
            // commit, which would have recorded the dwell. Nobody served the
            // bot, so the whole visit, to the current world time, awaits
            // station service.
            //
            // Mid-run, a visit to a station whose operator works on the bot in
            // front of them always closes with that work recorded, and
            // anything else is a modeling bug. A putaway induction is not such
            // a station: it fills totes at its own pace, and a bot that
            // brought one can be called back to the rack before its turn comes
            // — an unfilled visit there is the pacing, not a lost fact.
            let paced_by_the_station =
                matches!(visit_station.and_then(|id| self.config.station_types.get(&id)), Some(StationType::Putaway));
            if closed_into_departure && !paced_by_the_station {
                return Err(ActivityViolation::VisitWithoutDwell { robot, departure: close });
            }
            if !closed_into_departure {
                self.push_station_hold_rows(robot, open, None, close, carrying_cargo, visit);
            }
            return Ok(());
        };
        self.push_station_hold_rows(robot, open, Some(presented), close, carrying_cargo, visit);
        Ok(())
    }

    /// Journal the hold before a presentation dwell and the stand after it:
    /// the bot on the presentation cell until the visit's close. With no
    /// dwell recorded, nobody served the bot, and the hold is the whole visit.
    ///
    /// This is what shrinks `D30State::StationProcessing` to the recorded
    /// dwell (#272, Daniel 2026-07-30). The report's gap rule is untouched:
    /// with the pre-dwell hold journaled, the dwell is the one stretch of the
    /// visit no row covers and is bounded by the arrival and return facts at
    /// the same station endpoint.
    ///
    /// Station, cell, and presentation id come from the context captured when
    /// the visit opened. The mutable current trip may already describe a later
    /// future-stamped visit when this one closes.
    fn push_station_hold_rows(
        &mut self,
        robot_id: RobotId,
        open: SimTime,
        presented: Option<(SimTime, SimTime)>,
        close: SimTime,
        carrying_cargo: bool,
        visit: Option<VisitContext>,
    ) {
        let station = self.idle_location(visit.and_then(|ctx| ctx.station));
        let cell = visit.and_then(|ctx| ctx.cell);
        let presentation_id = visit.and_then(|ctx| ctx.presentation_id).map(|id| id.0);
        let awaiting_until = presented.map_or(close, |(dwell_start, _)| dwell_start);
        self.push_htm_hold_row(
            robot_id,
            (open, awaiting_until - open),
            (station, station),
            HtmWaitCause::AwaitingStationService,
            cell,
            carrying_cargo,
            presentation_id,
        );
        if let Some((_, dwell_end)) = presented {
            self.push_htm_hold_row(
                robot_id,
                (dwell_end, close - dwell_end),
                (station, station),
                HtmWaitCause::AwaitingDeparture,
                cell,
                carrying_cargo,
                presentation_id,
            );
        }
    }

    /// Forget a visit with no extent: it holds the bot for no time and
    /// journals nothing.
    pub(super) fn discard_station_visit(&mut self, robot: RobotId, arrival: SimTime) {
        self.activity.visits.open.remove(&(robot, arrival));
        self.robot_state_log.discard_visit(robot, arrival);
    }
}
