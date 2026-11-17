//! Within `run`, `cursor` is WHERE THE RUN IS BEING LOOKED AT and what it
//! costs to move there: the state the slices before the cursor rebuild, and
//! the measurement that has to be carried forward with them.
//!
//! It is separate from the frames because moving a cursor is not reading a
//! frame. Playing forward takes each slice in once and keeps the result;
//! dragging backwards cannot un-apply a slice, so it replays from the
//! attachment's own keyframe — and the traffic measured over those slices is
//! rebuilt with it, since a measurement of the first half of a run is not a
//! measurement of the whole one.
//!
//! **Two states, because there are two questions.** What is TRUE at the cursor
//! is what gets drawn, and the cursor's own slice is not applied for it: a
//! slice carries records stamped past its own end — the observer's run-end
//! finalisers land in the last one — and applying the whole slice would draw
//! them before they happened. What the run HAS COUNTED by the cursor is what a
//! chart and the run's visit counters report, and that does include the slice
//! the cursor has just reached the end of.

use desunits::{RobotId, SimTime};
use rustc_hash::FxHashMap;
use worldview::frames::declare::Descriptors;
use worldview::{Delta, RunState};
use worldview_render::{Drawn, Scene, Traffic, from_recording};

/// The slices an attachment holds, and where each one ends.
pub(super) struct Slices<'a> {
    pub(super) held: &'a [(SimTime, Delta)],
    /// Where the run stopped, which is where its last slice ends.
    pub(super) ended_at: Option<SimTime>,
}

impl Slices<'_> {
    /// Where the slice at `index` ends, which is where the next frame begins:
    /// the next slice, or the end block for the run's last, which the run's
    /// stop may cut short. Nil for the newest slice of a run still going,
    /// whose end no frame has said yet.
    pub(super) fn ends(&self, index: usize) -> Option<SimTime> {
        match self.held.get(index + 1) {
            Some((next, _)) => Some(*next),
            None => self.ended_at,
        }
    }
}

/// Where the cursor stands, and everything that had to be walked to get there.
#[derive(Default)]
pub(super) struct Cursor {
    /// The state as the cursor left it, standing at `held[reached].0`.
    standing: Option<RunState>,
    reached: usize,
    /// The same state with the cursor's own slice taken in as well, kept only
    /// while the cursor stands at that slice's end.
    totalled: Option<RunState>,
    /// The HTM traffic measured over the slices taken in so far.
    traffic: Option<Traffic>,
    /// The facing each robot was last drawn with, for the robots a slice
    /// states no heading for — which is desvis's rule, and the reason this is
    /// carried at all.
    facing_before: FxHashMap<RobotId, f32>,
}

impl Cursor {
    /// Put the cursor back at the attachment's keyframe, forgetting every
    /// slice walked and every measurement made over them.
    pub(super) fn rewind(&mut self, opened_on: Option<&RunState>, scene: Option<&Scene>) {
        self.standing = opened_on.cloned();
        self.totalled = None;
        self.traffic = scene.map(Traffic::over);
        self.facing_before.clear();
        self.reached = 0;
    }

    pub(super) fn drawn_at(&mut self, scene: &Scene, slices: &Slices<'_>, t: SimTime) -> Option<Result<Drawn, String>> {
        if let Err(why) = self.walk_to(slices, t).and_then(|()| self.total_through(slices, t)) {
            return Some(Err(why));
        }
        let state = self.standing.as_ref()?;
        let empty = Delta::default();
        let (begins, slice) =
            slices.held.get(self.reached).map(|(at, delta)| (*at, delta)).unwrap_or((state.reached, &empty));
        if let Some(traffic) = self.traffic.as_mut() {
            traffic.reaching(begins, t, state, slice);
        }
        Some(match from_recording(scene, state, slice, t, &self.facing_before) {
            Ok(drawn) => {
                self.facing_before = drawn.robots.iter().map(|robot| (robot.id, robot.heading_rad)).collect();
                Ok(drawn)
            }
            Err(why) => Err(why.to_string()),
        })
    }

    pub(super) fn standing(&self) -> Option<&RunState> {
        self.standing.as_ref()
    }

    /// What a chart reads: the standing state plus the slice just finished.
    pub(super) fn counted(&self) -> Option<&RunState> {
        self.totalled.as_ref().or(self.standing.as_ref())
    }

    pub(super) fn slice<'a>(&self, slices: &'a Slices<'_>) -> Option<&'a Delta> {
        slices.held.get(self.reached).map(|(_, delta)| delta)
    }

    pub(super) fn traffic(&self) -> Option<&Traffic> {
        self.traffic.as_ref()
    }

    /// Take in every slice the cursor has passed the end of, except the last,
    /// which the picture is drawn from.
    fn walk_to(&mut self, slices: &Slices<'_>, t: SimTime) -> Result<(), String> {
        while let Some(begins) = slices.held.get(self.reached).map(|(at, _)| *at) {
            let Some(ends) = slices.ends(self.reached).filter(|_| self.reached + 1 < slices.held.len()) else {
                return Ok(());
            };
            if t < ends {
                return Ok(());
            }
            let Some(state) = self.standing.as_mut() else { return Ok(()) };
            let (_, delta) = &slices.held[self.reached];
            if let Some(traffic) = self.traffic.as_mut() {
                traffic.took(begins, ends, state, delta);
            }
            state
                .apply(begins, ends, delta)
                .map_err(|why| format!("the slice at {} ms could not be taken in: {why}", begins.as_ms()))?;
            self.reached += 1;
        }
        Ok(())
    }

    /// Take the cursor's own slice into a state of its own once the cursor has
    /// reached its end, so a chart counts it without the picture drawing it. A
    /// slice whose end no frame has said yet is not counted, and one the state
    /// refuses is the recording's damage, reported.
    fn total_through(&mut self, slices: &Slices<'_>, t: SimTime) -> Result<(), String> {
        self.totalled = None;
        let Some((begins, delta)) = slices.held.get(self.reached) else { return Ok(()) };
        let Some(ends) = slices.ends(self.reached).filter(|ends| t >= *ends) else { return Ok(()) };
        let mut totalled = self.standing.clone();
        if let Some(state) = totalled.as_mut() {
            state
                .apply(*begins, ends, delta)
                .map_err(|why| format!("the slice at {} ms could not be counted: {why}", begins.as_ms()))?;
        }
        self.totalled = totalled;
        Ok(())
    }

    /// Lay out both states' scalars by the descriptors as they now stand. A
    /// metric declared after the keyframe widens every array the run states
    /// from then on, and a state still laid out for the narrower one refuses
    /// the next slice.
    pub(super) fn redescribe(&mut self, descriptors: &Descriptors) {
        for state in [self.standing.as_mut(), self.totalled.as_mut()].into_iter().flatten() {
            state.scalars.describe(descriptors);
        }
    }

    /// Whether the cursor has to start again from the keyframe to answer `t`:
    /// where the state stands is where its next slice begins, so an instant
    /// before that is one it can no longer be asked about.
    pub(super) fn must_rewind(&self, t: SimTime) -> bool {
        self.standing.as_ref().is_none_or(|state| t < state.reached)
    }
}
