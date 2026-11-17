//! Within `spans`, `reader` is THE CHANNEL READ ONCE FOR EVERY DEFINITION: each robot's state, seeded from a keyframe
//! and carried through every slice, told as the time each robot spent in each state.
//!
//! **When each thing is told.** A slice ends where the next frame begins, so its inside is told once the reader has
//! taken the next slice or the run's end, which the end block states at the run's stop, cutting the last slice there.
//! Only then are the slice's spans held to its width and its inside told. Once the reader has taken a slice, every
//! robot's time before its start is told.
//!
//! **Seeding.** A keyframe states each robot's state at its instant, so a reader seeded there tells every robot's time
//! from there on as a reader from simtime 0 tells it.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;
use worldview::frames::playback::RobotClass;
use worldview::frames::robot_state::{RobotSpans, RobotState, RobotStateDelta, RobotStateKeyframe, RobotStates};

use crate::model::{RobotId, SimDuration, SimTime};

use super::SpanError;

/// What a reader tells about the robots' time, in the order it learns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanEvent {
    /// A robot of `class` spent `[from, to)` in `state`.
    Spent { class: RobotClass, state: RobotState, from: SimTime, to: SimTime },
    /// Every robot's time before `at` is told.
    Through(SimTime),
    /// The run ended at `at`, and every robot's time before it is told.
    RunEnd(SimTime),
}

#[derive(Debug)]
struct Robot {
    class: RobotClass,
    /// Its state where the reader has told through.
    state: RobotState,
}

/// A slice taken whose inside is not yet told.
#[derive(Debug)]
struct Pending {
    begins: SimTime,
    named: FxHashMap<RobotId, RobotSpans>,
}

impl Pending {
    /// Refuse the slice unless every robot it names has spans laid end to end up to `ends`, where the next frame says
    /// it ends.
    fn lies_across(&self, ends: SimTime) -> Result<(), SpanError> {
        for spans in self.named.values() {
            spans.lie_across(self.begins, ends - self.begins)?;
        }
        Ok(())
    }
}

/// One reading of a recording's robot state channel, from a keyframe on, for every definition at once: it holds
/// each robot's state, and the newest slice, whose end only the frame the reader takes next states.
#[derive(Debug)]
pub struct SpanReader {
    width: SimDuration,
    // ordered: robots are told in id order, so two readers of one recording tell the same events in the same order.
    robots: BTreeMap<RobotId, Robot>,
    /// The state each robot ends the newest slice taken in, which the channel's rules check the next slice against.
    held: RobotStates,
    next: SimTime,
    pending: Option<Pending>,
    run_end: Option<SimTime>,
}

impl SpanReader {
    /// A reader beginning at the keyframe at `at`, over the robots `roster` names, in a recording whose slices are
    /// `slice_width` wide. The keyframe states every robot the roster names, and no other.
    pub fn seeded(
        roster: impl IntoIterator<Item = (RobotId, RobotClass)>,
        slice_width: SimDuration,
        at: SimTime,
        keyframe: &RobotStateKeyframe,
    ) -> Result<Self, SpanError> {
        if slice_width == SimDuration::ZERO {
            return Err(SpanError::ZeroWidth);
        }
        let held = RobotStates::from_keyframe(at, keyframe)?;
        let mut robots = BTreeMap::new();
        for (robot, class) in roster {
            let unstated = SpanError::Unstated { at, robot };
            let state = *held.robots.get(&robot).ok_or(unstated)?;
            robots.insert(robot, Robot { class, state });
        }
        if let Some(robot) = held.robots.keys().filter(|robot| !robots.contains_key(robot)).min() {
            return Err(SpanError::OffRoster { at, robot: *robot });
        }
        Ok(Self { width: slice_width, robots, held, next: at, pending: None, run_end: None })
    }

    /// Take the slice beginning at `begins`, where the last one taken ended or, for the first, at the keyframe's
    /// instant: tell the inside of the slice before, which the run has gone past. A slice out of turn, or one the
    /// channel's rules refuse, is refused before anything is told, and so is a slice before it whose spans do not lie
    /// across its whole width.
    pub fn slice(&mut self, begins: SimTime, delta: RobotStateDelta) -> Result<Vec<SpanEvent>, SpanError> {
        self.before_run_end()?;
        if begins != self.next {
            return Err(SpanError::NotNextSlice { begins, next: self.next });
        }
        self.held.check(begins, None, &delta)?;
        let mut events = Vec::new();
        if let Some(pending) = self.pending.take() {
            pending.lies_across(begins)?;
            self.tell_inside(&pending, begins, &mut events);
        }
        events.push(SpanEvent::Through(begins));
        self.held.leave(&delta);
        self.pending = Some(Pending { begins, named: delta.0.into_iter().map(|entry| (entry.robot, entry)).collect() });
        self.next = begins + self.width;
        Ok(events)
    }

    /// The run stopped at `run_end`, which ends the last slice taken: tell that slice's inside up to there, once its
    /// spans are held to lie across it exactly.
    pub fn end(&mut self, run_end: SimTime) -> Result<Vec<SpanEvent>, SpanError> {
        self.before_run_end()?;
        let from = self.pending.as_ref().map_or(self.next, |pending| pending.begins);
        if run_end < from || run_end > self.next {
            return Err(SpanError::RunEndOutsideLastSlice { run_end, from, to: self.next });
        }
        let mut events = Vec::new();
        if let Some(pending) = self.pending.take() {
            pending.lies_across(run_end)?;
            self.tell_inside(&pending, run_end, &mut events);
        }
        events.push(SpanEvent::RunEnd(run_end));
        self.run_end = Some(run_end);
        Ok(events)
    }

    fn before_run_end(&self) -> Result<(), SpanError> {
        match self.run_end {
            Some(run_end) => Err(SpanError::AfterRunEnd { run_end }),
            None => Ok(()),
        }
    }

    /// Tell the inside of `pending` up to `until`, the slice's end or the run's: each robot's time in the spans the
    /// slice states for it, or in the state it holds all slice.
    fn tell_inside(&mut self, pending: &Pending, until: SimTime, events: &mut Vec<SpanEvent>) {
        for (id, robot) in &mut self.robots {
            let Some(named) = pending.named.get(id) else {
                robot.spend(pending.begins, until, events);
                continue;
            };
            let mut at = pending.begins;
            for span in &named.spans {
                if at >= until {
                    break;
                }
                robot.state = span.state;
                robot.spend(at, (at + span.dt).min(until), events);
                at = at + span.dt;
            }
        }
    }
}

impl Robot {
    fn spend(&self, from: SimTime, to: SimTime, events: &mut Vec<SpanEvent>) {
        if to > from {
            events.push(SpanEvent::Spent { class: self.class, state: self.state, from, to });
        }
    }
}
