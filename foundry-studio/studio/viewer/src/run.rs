//! Within `crucible-viewer`, `run` is THE RUN AS THIS BROWSER HOLDS IT: the
//! frames the server has sent, the state they rebuild, and what is true at the
//! instant the cursor points at.
//!
//! It exists so that the drawing half of the viewer asks one question — what
//! does the run say at `t`? — and never touches a frame. Everything that knows
//! about the container format is here, and everything that knows about a GPU
//! is not, which is what lets these answers be checked against desvis's own
//! without a browser (`tests/dumped_rows.rs`).
//!
//! A browser holds only what it was sent. An attachment opens on one keyframe
//! and the deltas after it, so the stretch the cursor can be moved within is
//! that keyframe's instant to where the run stopped; a cursor outside it is
//! the shell's cue to attach again somewhere else. A floor sent with no run
//! (a site's or an evaluation's, spec 03 section 3.6) is held here the same
//! way, as a floor no frame will follow.
//!
//! **A damaged stream stops the run.** Frames are held to the shape the wire
//! promises as they arrive — one frame per message, deltas dense and in order
//! after the keyframe, nothing after the end — and the first breach is kept
//! and answered with from then on, because a viewer that carried on would draw
//! a run nobody recorded.

use std::fmt;

use desunits::{SimDuration, SimTime};
use worldview::frames::declare::Descriptors;
use worldview::frames::scenario::Scenario;
use worldview::{Declare, Delta, End, Error, Frame, Index, IndexRecord, Keyframe, Payload, RunState, Sections, Tag};
use worldview_render::{Census, Drawn, FloorDoc, HeatmapData, Scene, Traffic, slice_visits, visit_counts};

/// What one frame told the viewer, for a shell that wants to say so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    Scenario,
    Index,
    Declaration,
    Keyframe,
    Delta,
    End,
}

/// Why this run can no longer be drawn. A stream that broke its own shape does
/// not mend, so every later answer is this one, and attaching again is the
/// shell's decision rather than this one's.
#[derive(Clone, Debug)]
pub struct Stopped(pub String);

/// The run this browser holds: the scene to draw it on, the frames it was
/// sent, and where the cursor points.
#[derive(Default)]
pub struct Run {
    scene: Option<Scene>,
    /// Whether the floor held was sent ahead with `take_floor` and no scenario
    /// block has run on it yet: the one floor a block without its own may use.
    floor_sent_ahead: bool,
    descriptors: Descriptors,
    slice_width: SimDuration,
    /// Which cells each entity of the world document turned into, which the
    /// viewer picks and outlines by.
    census: Census,
    /// Every record the server sent of the file at or before the attach
    /// instant — what a shell decides its next seek from.
    index: Vec<IndexRecord>,
    opened_at: SimTime,
    /// The state the attachment's own keyframe states, kept so a cursor moving
    /// backwards can be answered by replaying from it.
    opened_on: Option<RunState>,
    slices: Vec<(SimTime, Delta)>,
    /// The instant the end frame stamped: where the run actually stopped,
    /// which is not its last slice's start plus a slice width.
    ended_at: Option<SimTime>,
    cursor: Cursor,
    stopped: Option<Stopped>,
}

impl Run {
    /// Take one worldview frame, header included, exactly as the server sent
    /// it — and nothing else, since one frame is one binary message and bytes
    /// after it are a second frame or damage, either way not this one.
    pub fn push_frame(&mut self, bytes: &[u8]) -> Result<Took, Stopped> {
        if let Some(stopped) = &self.stopped {
            return Err(stopped.clone());
        }
        match self.take(bytes) {
            Ok(took) => Ok(took),
            Err(why) => Err(self.stop(why)),
        }
    }

    fn take(&mut self, bytes: &[u8]) -> Result<Took, String> {
        let said = |failure: Error| failure.to_string();
        let (frame, consumed) = Frame::decode_at(bytes, 0).map_err(said)?;
        if consumed != bytes.len() {
            return Err(format!("a {}-byte message carries one {consumed}-byte frame and more besides", bytes.len()));
        }
        let body = frame.contents(0).map_err(said)?;
        let at = SimTime::ms(frame.header.t.0);
        // A scenario block after the end is the NEXT attachment opening on
        // this canvas, which is the one frame that may follow an end.
        if self.ended_at.is_some() && frame.header.tag != Tag::Scenario {
            return Err(format!("a {:?} frame at {} ms arrived after the run's end", frame.header.tag, at.as_ms()));
        }
        match frame.header.tag {
            Tag::Scenario => {
                self.scenario(&Scenario::from_bytes(&body, 0).map_err(said)?).map_err(said)?;
                Ok(Took::Scenario)
            }
            Tag::Index => {
                self.index = Index::from_block(&body, 0).map_err(said)?.records().to_vec();
                Ok(Took::Index)
            }
            Tag::Declare => {
                self.descriptors.push(&Declare::from_bytes(&body, 0).map_err(said)?);
                // Both states hold a scalars array laid out by the descriptors
                // known when it was built. A metric declared after the
                // keyframe widens that layout, and a state still holding the
                // narrow one refuses the next delta for naming metrics the run
                // never declared.
                if let Some(state) = self.opened_on.as_mut() {
                    state.scalars.describe(&self.descriptors);
                }
                self.cursor.redescribe(&self.descriptors);
                Ok(Took::Declaration)
            }
            Tag::Keyframe => {
                if self.opened_on.is_some() {
                    return Err(format!("a second keyframe arrived at {} ms; an attachment sends one", at.as_ms()));
                }
                let sections = Sections::decode(body.to_vec(), 0).map_err(said)?;
                let keyframe = Keyframe::decode(&sections, 0).map_err(said)?;
                self.opened_at = at;
                self.opened_on = Some(RunState::from_keyframe(at, &keyframe, &self.descriptors).map_err(said)?);
                self.cursor.rewind(self.opened_on.as_ref(), self.scene.as_ref());
                Ok(Took::Keyframe)
            }
            Tag::Delta => {
                let expected = self.next_slice_at().ok_or("a delta arrived before the attachment's keyframe")?;
                if at != expected {
                    return Err(format!(
                        "the slice at {} ms does not follow the one before it, which ends at {} ms",
                        at.as_ms(),
                        expected.as_ms()
                    ));
                }
                let sections = Sections::decode(body.to_vec(), 0).map_err(said)?;
                self.slices.push((at, Delta::decode(&sections, 0).map_err(said)?));
                Ok(Took::Delta)
            }
            Tag::End => {
                End::from_bytes(&body, 0).map_err(said)?;
                if self.last_slice_ends().is_some_and(|last| at > last) {
                    return Err(format!("the run ends at {} ms, past what its last slice reaches", at.as_ms()));
                }
                self.ended_at = Some(at.max(self.opened_at));
                Ok(Took::End)
            }
        }
    }

    /// The scenario block: the floor to draw on, and what the run declared.
    ///
    /// A second one is a second attachment on the same canvas, so everything
    /// the first sent goes — its keyframe, its slices, its index, its end —
    /// and the floor is replaced. A browser compiles no floor of its own, so
    /// the block carries one, or the shell has sent one ahead for it with
    /// `set_floor` (a run's recording from dessim carries none: the floor is
    /// the site's, read by its key). A block with neither is refused even
    /// while the last attachment's floor is held, as `clear` keeps it: that
    /// floor under this one's robots would be a picture of no run at all. A
    /// floor sent ahead serves one block. The census beside it is kept
    /// for the entity explorer and read by nobody yet; it is decoded here
    /// anyway, so a block that carries a damaged one is refused rather than
    /// found out later by whoever first opens that panel.
    fn scenario(&mut self, scenario: &Scenario) -> Result<(), Error> {
        // Both documents are read before either is kept, so a block whose
        // census is damaged does not leave the floor beside it standing.
        let floor = match &scenario.floor {
            Some(floor) => Some(floor.read::<FloorDoc>("the compiled floor")?),
            None if self.floor_sent_ahead => None,
            None => {
                return Err(Error::OpaqueShape {
                    what: "the compiled floor",
                    detail: "the scenario block carries none, none was sent ahead for it, and a browser compiles no floor of its own".into(),
                });
            }
        };
        let census = match &scenario.census {
            Some(census) => census.read::<Census>("the entity census")?,
            None => Census::new(),
        };

        self.slice_width = scenario.slice_width;
        self.descriptors = Descriptors::from_scenario(scenario);
        self.census = census;
        if let Some(floor) = floor {
            self.scene = Some(floor.into_scene());
        }
        self.floor_sent_ahead = false;
        self.forget_attachment();
        Ok(())
    }

    /// A floor with no run: whatever run was held is forgotten, declarations
    /// and census and all, as a scenario block for another run would forget
    /// it. Its roll comes in its own document and is the viewer's to keep.
    pub fn take_floor(&mut self, scene: Scene) {
        self.descriptors = Descriptors::default();
        self.slice_width = SimDuration::default();
        self.census = Census::new();
        self.scene = Some(scene);
        self.floor_sent_ahead = true;
        self.stopped = None;
        self.forget_attachment();
    }

    /// Forget every frame this attachment sent, keeping the floor. The shell
    /// calls this before attaching somewhere else, so the frames that follow
    /// start a window of their own instead of joining one they have no
    /// continuity with.
    pub fn clear(&mut self) {
        self.stopped = None;
        self.forget_attachment();
    }

    fn forget_attachment(&mut self) {
        self.opened_on = None;
        self.opened_at = SimTime::ZERO;
        self.slices.clear();
        self.index.clear();
        self.ended_at = None;
        self.cursor.rewind(self.opened_on.as_ref(), self.scene.as_ref());
    }

    pub fn scene(&self) -> Option<&Scene> {
        self.scene.as_ref()
    }

    pub fn ended(&self) -> bool {
        self.ended_at.is_some()
    }

    pub fn index(&self) -> &[IndexRecord] {
        &self.index
    }

    pub fn census(&self) -> &Census {
        &self.census
    }

    pub fn stopped(&self) -> Option<&Stopped> {
        self.stopped.as_ref()
    }

    /// The stretch the cursor can be moved within without attaching again:
    /// the attachment's keyframe, to where the run stopped. Nil until a
    /// keyframe has arrived.
    ///
    /// The end frame's own instant is what the run reached. A run that stopped
    /// part-way through its last slice goes no further, and a window to that
    /// slice's nominal end would offer time no frame describes.
    pub fn held_window(&self) -> Option<(SimTime, SimTime)> {
        self.opened_on.as_ref()?;
        let reaches = self.ended_at.or_else(|| self.last_slice_ends()).unwrap_or(self.opened_at);
        Some((self.opened_at, reaches))
    }

    /// What the run says at `t`, in the renderer's words. Nil before a floor
    /// and a keyframe have both arrived.
    pub fn drawn_at(&mut self, t: SimTime) -> Option<Result<Drawn, Stopped>> {
        if let Some(stopped) = &self.stopped {
            return Some(Err(stopped.clone()));
        }
        let t = self.held(t)?;
        if self.cursor.must_rewind(t) {
            self.cursor.rewind(self.opened_on.as_ref(), self.scene.as_ref());
        }
        let slices = Slices { held: &self.slices, ended_at: self.ended_at };
        let drawn = self.cursor.drawn_at(self.scene.as_ref()?, &slices, t)?;
        Some(drawn.map_err(|why| self.stop(why)))
    }

    /// The overlay the cursor's own slice paints — desvis's cursor view — as
    /// against the whole run's counters in [`Self::visits_so_far`].
    pub fn slice_overlay(&self) -> Option<HeatmapData> {
        let state = self.cursor.standing()?;
        let empty = Delta::default();
        let slices = Slices { held: &self.slices, ended_at: self.ended_at };
        let slice = self.cursor.slice(&slices).unwrap_or(&empty);
        Some(slice_visits(state, slice, self.scene.as_ref()?.cells().len()))
    }

    pub fn visits_so_far(&self) -> Option<HeatmapData> {
        Some(visit_counts(&self.cursor.counted()?.heat, self.scene.as_ref()?.cells().len()))
    }

    /// Measured from the attachment's keyframe to the cursor, never earlier.
    pub fn traffic(&self) -> Option<&Traffic> {
        self.cursor.traffic()
    }

    /// One metric's running value where the cursor stands, by the name the run
    /// declared it under.
    pub fn metric(&self, name: &str) -> Option<f64> {
        let slot = self.descriptors.metric_slot(name)?;
        self.cursor.counted()?.scalars.values.get(slot).copied().flatten()
    }

    /// Every metric this run declared, in the order their slots run. A chart
    /// offers what the run itself says it measures, rather than a list written
    /// somewhere else that a later run would leave stale.
    pub fn metric_names(&self) -> Vec<String> {
        self.descriptors.metrics.iter().map(|metric| metric.name.clone()).collect()
    }

    /// One metric's running value at the END of every slice this attachment
    /// holds, as `(t_ms, value)` pairs.
    ///
    /// This is the live half of a chart: the server's `series` answers for
    /// everything up to the attach instant, and each slice taken in after it
    /// adds a point, once the next frame says where the slice ends. The instant
    /// is the slice's end because a delta stamped at a slice's start says what
    /// that slice did, which is true only once it has finished — the reading
    /// the server's `series` takes over the file. Only the scalars are walked,
    /// so where the cursor stands is left alone.
    ///
    /// A slice that stated nothing about the metric adds no point, by the same
    /// rule `series::only_stated` follows over the file. The two halves meet in
    /// one line, so a metric a run states intermittently — one measuring
    /// something that comes and goes — has to read the same on both sides of
    /// the join: a gap before it and an unbroken flat run after it would be two
    /// different claims drawn as one line.
    pub fn metric_series(&self, name: &str) -> Vec<(u64, f64)> {
        let Some((slot, opened)) = self.descriptors.metric_slot(name).zip(self.opened_on.as_ref()) else {
            return Vec::new();
        };
        let mut values = opened.scalars.clone();
        let slices = Slices { held: &self.slices, ended_at: self.ended_at };
        let mut points = Vec::with_capacity(self.slices.len());
        for (index, (_, delta)) in self.slices.iter().enumerate() {
            let stated = match &delta.scalars {
                Some(scalars) => {
                    values.apply(scalars);
                    scalars.values.get(slot).copied().flatten().is_some()
                }
                None => false,
            };
            if let (true, Some(value), Some(ends)) =
                (stated, values.values.get(slot).copied().flatten(), slices.ends(index))
            {
                points.push((ends.as_ms(), value));
            }
        }
        points
    }

    /// `t` clamped into the held window, or nil when nothing is held.
    fn held(&self, t: SimTime) -> Option<SimTime> {
        let (from, to) = self.held_window()?;
        Some(t.clamp(from, to))
    }

    /// The instant the next delta must be stamped with, or nil before the
    /// keyframe: a slice begins where the one before it ends, and the wire
    /// promises them dense. Nominal for the last one, because the end frame
    /// that would shorten it has not arrived while deltas still are.
    fn next_slice_at(&self) -> Option<SimTime> {
        self.opened_on.as_ref()?;
        Some(match self.slices.last() {
            Some((begins, _)) => begins.saturating_add(self.slice_width),
            None => self.opened_at,
        })
    }

    fn last_slice_ends(&self) -> Option<SimTime> {
        let (begins, _) = self.slices.last()?;
        Some(begins.saturating_add(self.slice_width))
    }

    /// Keep the first thing that went wrong and answer with it from now on.
    fn stop(&mut self, why: String) -> Stopped {
        self.stopped.get_or_insert(Stopped(why)).clone()
    }
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

mod cursor;
#[cfg(test)]
mod tests;

use cursor::{Cursor, Slices};
