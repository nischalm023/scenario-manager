//! Within `activity`, `intervals` settles a robot's milestones as B passes
//! them, builds the intervals they close, and books what each one is read
//! into: the run's D30 duty totals, the journal's no-task rows for the gaps
//! between them, and the bot inspector's records.

use desgraph::{CellId, RobotType};

use crate::events::TripEndpoint;
use crate::model::{RobotId, SimTime};
use crate::observer::duty::BusyInterval;
use crate::observer::state::SimObserverState;

use super::log::RobotActivity;
use super::{ActivityKind, ActivityViolation, Milestone};

impl RobotActivity {
    /// Where the first departure at or after `instant` left from.
    fn departure_from(&self, instant: SimTime) -> Option<(TripEndpoint, Option<CellId>)> {
        let first = self.marks.partition_point(|m| m.at() < instant);
        let pending = self.pending.iter().filter(|m| m.at() >= instant).min_by_key(|m| m.order());
        self.marks[first..].iter().chain(pending).find_map(|m| match m {
            Milestone::Departure { from, from_cell, .. } => Some((*from, *from_cell)),
            _ => None,
        })
    }

    fn carrying_at(&self, instant: SimTime) -> bool {
        let after = self.carrying_since.partition_point(|(at, _)| *at <= instant);
        after.checked_sub(1).is_some_and(|i| self.carrying_since[i].1)
    }
}

impl SimObserverState {
    /// B advanced to `at`, the current world time from here on: turn what lies
    /// behind it into intervals and their readings, and derive the
    /// distribution of every slice whose start it has passed and the D30 duty
    /// of every slice whose end it has passed. Visits only the robots it has
    /// something to settle for.
    pub(in crate::observer) fn settle_activity(&mut self, at: SimTime) -> Result<(), ActivityViolation> {
        self.activity.world_time = at;
        let floor = self.current_world_time();
        for robot in self.activity.take_due(floor) {
            self.settle_robot(robot, floor)?;
            self.build_robot_intervals(robot, Some(floor))?;
            if let Some(due) = self.activity.robots.get(&robot).and_then(RobotActivity::next_due) {
                self.activity.due_at(robot, due);
            }
        }
        self.derive_settled_distribution(floor);
        self.derive_settled_duty(floor);
        Ok(())
    }

    /// Move `robot`'s milestones behind `floor` into its marks.
    fn settle_robot(&mut self, robot: RobotId, floor: SimTime) -> Result<(), ActivityViolation> {
        let Some(state) = self.activity.robots.get_mut(&robot) else {
            return Ok(());
        };
        let (mut batch, pending): (Vec<Milestone>, Vec<Milestone>) =
            state.pending.drain(..).partition(|m| m.at() < floor);
        state.pending = pending;
        // A fact is never stamped behind B, but a putaway dwell with no visit
        // open is work from the dwell's start, which can lie behind the fill
        // its fact is stamped at. B had passed that work when it was reported,
        // so the slices it falls in were read without it.
        if let Some(behind) = batch.iter().find(|m| m.at() < m.known_from()) {
            return Err(ActivityViolation::WorkBehindBarrier { robot, at: behind.at(), barrier: behind.known_from() });
        }
        if batch.is_empty() {
            return Ok(());
        }
        state.marks.append(&mut batch);
        state.marks.sort_by_key(Milestone::order);
        Ok(())
    }

    /// Build every interval of `robot`'s whose close B has passed: each mark
    /// the next one bounds, and a span on its own once B is past its end.
    /// With no floor the stream has closed, and the last mark is closed by
    /// the rules for a run that stopped mid-task.
    fn build_robot_intervals(&mut self, robot: RobotId, floor: Option<SimTime>) -> Result<(), ActivityViolation> {
        let end_of_run = self.latest_sim_time;
        let Some(state) = self.activity.robots.get(&robot) else {
            return Ok(());
        };
        let buildable = match (floor, state.marks.last()) {
            (None, _) => state.marks.len(),
            (Some(floor), Some(Milestone::Span { end, .. })) if *end < floor => state.marks.len(),
            _ => state.marks.len().saturating_sub(1),
        };
        if buildable == 0 && floor.is_some() {
            return Ok(());
        }
        let window: Vec<Milestone> = state.marks[..(buildable + 1).min(state.marks.len())].to_vec();
        let mut carrying = state.carrying;

        let mut fresh: Vec<BusyInterval> = Vec::new();
        // The stands before operations the bot stood resident for, which the
        // operations' own facts journal.
        let mut stands: Vec<(SimTime, SimTime)> = Vec::new();
        let mut carrying_since: Vec<(SimTime, bool)> = Vec::new();
        for (index, mark) in window.iter().take(buildable).enumerate() {
            let next_mark = window.get(index + 1);
            let next = next_mark.map(Milestone::at);
            match *mark {
                Milestone::Span { kind, start, end, carries_after, resident_since, .. } => {
                    stands.extend(resident_since.map(|since| (since, start)));
                    fresh.push(BusyInterval { kind, start, end });
                    if let Some(after) = carries_after {
                        carrying = after;
                        carrying_since.push((end, after));
                    }
                }
                // The stand before the operation the leg was sent for is the
                // leg's from when that operation was reported; with none
                // following, the leg ends with its nav, or sooner if the next
                // leg departs first.
                Milestone::Departure { at, nav, .. } => {
                    if let Some(nav) = nav {
                        carrying = nav.carrying;
                        carrying_since.push((at, nav.carrying));
                    }
                    let travel = |start, end| BusyInterval { kind: ActivityKind::Travel, start, end };
                    let arrival = nav.map_or(at, |nav| nav.arrival);
                    match next_mark.and_then(|next| Some((next, next.claims_stand_from(arrival)?))) {
                        Some((operation, claimed)) if claimed > arrival => {
                            fresh.extend([travel(at, arrival), travel(claimed, operation.at())]);
                        }
                        Some((operation, _)) => fresh.push(travel(at, operation.at())),
                        None => fresh.push(travel(at, next.map_or(arrival, |next| arrival.min(next)))),
                    }
                }
                Milestone::Arrival { at, .. } => match next {
                    Some(end) if end > at => {
                        fresh.push(BusyInterval { kind: ActivityKind::Presentation, start: at, end });
                        self.close_station_visit(robot, at, end, true, carrying)?;
                    }
                    Some(_) => {
                        self.discard_station_visit(robot, at);
                    }
                    // The run stopped inside the visit with no departure
                    // scheduled. A departure scheduled later could start no
                    // earlier than the current world time, nor before the
                    // recorded dwell ends, so the bot stands at the station
                    // until the later of the two. An arrival at or past the
                    // current world time with no dwell recorded makes a visit
                    // with no extent, and it is discarded.
                    None => {
                        let world_time = self.current_world_time();
                        let end = self.recorded_dwell_end(robot, at).map_or(world_time, |dwell| dwell.max(world_time));
                        if end > at {
                            fresh.push(BusyInterval { kind: ActivityKind::Presentation, start: at, end });
                            self.close_station_visit(robot, at, end, false, carrying)?;
                        } else {
                            self.discard_station_visit(robot, at);
                        }
                    }
                },
            }
        }
        fresh.retain(|interval| interval.end > interval.start);
        let mut covered: Vec<(SimTime, SimTime)> =
            fresh.iter().map(|interval| (interval.start, interval.end)).chain(stands).filter(|(s, e)| e > s).collect();
        covered.sort();

        let next_read = self.activity.distribution_upto.min(self.activity.duty_upto);
        let next_sample = SimTime::ms(next_read as u64 * self.config.slice_width_ms);
        let Some(state) = self.activity.robots.get_mut(&robot) else {
            return Ok(());
        };
        state.carrying = carrying;
        let adjoining: Vec<BusyInterval> = state.last.into_iter().chain(fresh.iter().copied()).collect();
        if let Some(pair) = adjoining.windows(2).find(|pair| pair[0].end > pair[1].start) {
            return Err(ActivityViolation::InsideTwoTasks { robot, first: pair[0], second: pair[1] });
        }
        state.last = fresh.last().copied().or(state.last);
        // No slice still to be derived reads before `next_sample`, nor the
        // closing distribution before the run's latest instant so far.
        let stale = next_sample.min(end_of_run);
        state.intervals.retain(|interval| interval.end > stale);
        state.intervals.extend(fresh.iter().copied().filter(|interval| interval.end > stale));

        // A DA50's duty is booked from its rail legs by `record_vtm_leg`, which
        // reads both of a leg's clocks; here its intervals serve the
        // distribution and the inspector alone. A D30's share of each slice is
        // derived as B passes the slice's end, and the run's slices reach as
        // far as its work does.
        if self.output.robot_types.get(&robot) == Some(&RobotType::Htm) {
            for interval in &fresh {
                self.record_interval(robot, *interval);
                self.extend_slices_through(((interval.end.as_ms() - 1) / self.config.slice_width_ms) as usize);
            }
            self.journal_activity_gaps(robot, &covered, carrying_since, floor.is_none());
        }
        for interval in &fresh {
            self.record_robot_activity(robot, *interval);
        }
        if let Some(state) = self.activity.robots.get_mut(&robot) {
            state.marks.drain(..buildable);
        }
        Ok(())
    }

    /// Once the stream has closed: settle and build every robot's work,
    /// including the roster's robots that never moved, and derive the
    /// distribution and the D30 duty of every slice B never passed from the
    /// final intervals.
    pub(in crate::observer) fn build_activity_intervals(&mut self) -> Result<(), ActivityViolation> {
        let mut robots: Vec<RobotId> =
            self.output.robot_types.keys().chain(self.activity.robots.keys()).copied().collect();
        // ordered: robot id, so booking order is run-independent.
        robots.sort_by_key(|robot| robot.0);
        robots.dedup();
        for robot in robots {
            self.activity.robots.entry(robot).or_default();
            self.settle_robot(robot, SimTime::ms(u64::MAX))?;
            self.build_robot_intervals(robot, None)?;
        }
        self.derive_final_distribution();
        self.derive_final_duty();
        Ok(())
    }

    /// Journal as no-task wait rows the gaps between the stretches `covered`
    /// already accounts for, in order, and on the final pass the gap to the
    /// end of the run. The bot stood where the departure ending a gap left
    /// from, or with none, where its last leg took it, and it held what the
    /// last change in `carried` says.
    fn journal_activity_gaps(
        &mut self,
        robot: RobotId,
        covered: &[(SimTime, SimTime)],
        carried: Vec<(SimTime, bool)>,
        final_pass: bool,
    ) {
        let end_of_run = self.latest_sim_time;
        let Some(state) = self.activity.robots.get_mut(&robot) else {
            return;
        };
        state.carrying_since.extend(carried);
        let mut cursor = state.gap_cursor;
        let mut gaps = Vec::new();
        let trailing = final_pass.then_some((end_of_run, end_of_run));
        for (start, end) in covered.iter().copied().chain(trailing) {
            if start > cursor {
                gaps.push((cursor, start, state.carrying_at(cursor), state.departure_from(start)));
            }
            cursor = cursor.max(end);
        }
        state.gap_cursor = cursor;
        let superseded = state.carrying_since.partition_point(|(at, _)| *at <= cursor).saturating_sub(1);
        state.carrying_since.drain(..superseded);
        for (start, end, carrying, from) in gaps {
            let (location, cell) = match from {
                Some((from, from_cell)) => (self.idle_location(Some(from)), from_cell),
                None => (
                    self.idle_location(self.last_trip_to.get(&robot).copied()),
                    self.bot_positions.get(&robot.0).copied().map(CellId),
                ),
            };
            self.push_htm_no_task_wait_row(robot, start, end - start, location, cell, carrying);
        }
    }
}
