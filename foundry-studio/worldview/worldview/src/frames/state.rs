//! One instant of a run, rebuilt from one keyframe and the deltas after it.
//!
//! **What it is for.** Every way of looking at a run asks the same question —
//! what is true at time `t`? — and there must be one answer, whether it was
//! reached by scrubbing to `t` or by playing the run from its start. This
//! module is that one answer: [`RunState`] takes a keyframe and then whole
//! slices, and [`RunState::at`] reads a place inside the slice a scrub landed
//! in without applying it.
//!
//! **Where a state stands.** A state stands at an instant, and that instant
//! is where the next slice begins: a keyframe puts it at the keyframe's own
//! instant, and applying a slice moves it to where that slice ends. So a
//! delta's offsets are always measured from where the state already stands,
//! and the slice a scrub lands in is the one beginning there. Beside where
//! everything is, it holds what every robot is doing, for a recording that
//! states the robot state channel.
//!
//! No geometry lives here, and none is needed: a recording states where every
//! robot is in millimetres, so this module carries those coordinates forward
//! and interpolates between them without ever asking the world document where
//! a cell is.

use std::io::{Read, Seek};

use desgraph::types::{CellId, StationId};
use desunits::{RobotId, SimTime, ToteId};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::container::error::{Error, Result};
use crate::container::reader::Reader;
use crate::frames::declare::{Declare, Descriptors};
use crate::frames::heat::{HeatDelta, HeatKeyframe};
use crate::frames::playback::{
    MoverSteps, PlaybackDelta, PlaybackKeyframe, Position, RobotClass, ToteLocation, ToteTransit,
};
use crate::frames::robot_state::{RobotStateDelta, RobotStates};
use crate::frames::scalars::{ScalarsDelta, ScalarsKeyframe};
use crate::frames::scenario::Scenario;
use crate::frames::{Delta, Keyframe, instant, stamp};

/// Where one robot is, which way it faces, and what it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotPlace {
    pub class: RobotClass,
    pub at: Position,
    /// Which way the robot faces at the instant this state stands at. A
    /// keyframe states it outright, and a delta restates it for every robot
    /// that turned since the slice before.
    pub heading_mrad: Option<i32>,
    pub carrying: Option<ToteId>,
    /// The cell the robot is in, and the one it is heading for when it is
    /// between two. Advisory throughout: a consumer reporting per cell counts
    /// the robot against `cell`, and nothing places the robot from either.
    pub cell: CellId,
    pub toward: Option<CellId>,
}

/// Where one tote is, what it is, and how full it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToteState {
    pub tote_type: u8,
    pub designation: u8,
    pub location: ToteLocation,
    pub contents_units: u64,
}

/// Everything the viewer draws at one instant.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackState {
    pub robots: FxHashMap<RobotId, RobotPlace>,
    /// Every tote that is still in the run. A tote whose location becomes
    /// `Extracted` has left the simulated system and is dropped.
    pub totes: FxHashMap<ToteId, ToteState>,
    pub reserved_relay_cells: FxHashSet<CellId>,
    /// How much work is queued at each station.
    pub stations: FxHashMap<StationId, u32>,
    pub transits_in_flight: Vec<ToteTransit>,
}

/// Every metric's value at one instant, positional per the descriptors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScalarsState {
    pub values: Vec<Option<f64>>,
    /// Which slots add a slice's value onto what came before and which take
    /// it as the newest reading. Slots past the end of the descriptors a
    /// reader has seen accumulate, which is what a counter does.
    accumulates: Vec<bool>,
}

/// Each named layer's per-cell counters at one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeatState {
    pub layers: FxHashMap<String, FxHashMap<CellId, u64>>,
}

/// One run at one instant: where everything is, what every metric says, what
/// every grid counter holds, and what every robot is doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunState {
    /// The instant this state stands at, which is where the next slice
    /// begins.
    pub reached: SimTime,
    pub playback: PlaybackState,
    pub scalars: ScalarsState,
    pub heat: HeatState,
    /// What every robot is doing; nil for a recording that does not state the
    /// robot state channel, or a wire that left it out.
    pub robot_states: Option<RobotStates>,
}

/// Where one robot is at an instant inside a slice: its coordinates, the cell
/// it counts against, and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotAt {
    pub at: Position,
    pub cell: CellId,
    pub heading_mrad: Option<i32>,
}

/// Where everything is at an instant inside a slice — the answer a scrub
/// gives. Which robot carries which tote is here too, as the totes whose
/// location names a robot, and what each robot is doing, as the state does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackAt {
    pub at: SimTime,
    pub robots: FxHashMap<RobotId, RobotAt>,
    pub totes: FxHashMap<ToteId, ToteLocation>,
    pub robot_states: Option<RobotStates>,
}

impl RunState {
    /// The state a keyframe states outright, standing at the keyframe's
    /// instant. The descriptors say which scalars slots accumulate, and a
    /// keyframe naming more slots than the run has declared is damage.
    pub fn from_keyframe(at: SimTime, keyframe: &Keyframe, descriptors: &Descriptors) -> Result<Self> {
        let playback = keyframe.playback.as_ref().map(PlaybackState::from_keyframe).unwrap_or_default();
        let mut scalars = keyframe.scalars.as_ref().map(ScalarsState::from_keyframe).unwrap_or_default();
        scalars.describe(descriptors);
        declared(scalars.values.len(), descriptors.metrics.len())?;
        let heat = keyframe.heat.as_ref().map(HeatState::from_keyframe).unwrap_or_default();
        let robot_states =
            keyframe.robot_state.as_ref().map(|states| RobotStates::from_keyframe(at, states)).transpose()?;
        if let (Some(placed), Some(states)) = (&keyframe.playback, &robot_states) {
            every_placed_robot_stated(at, placed, states)?;
        }
        Ok(Self { reached: at, playback, scalars, heat, robot_states })
    }

    /// Take in what a delta states about the instant its slice begins, which
    /// is where this state stands: the headings of the robots that turned
    /// since the slice before, movers and standing robots alike.
    ///
    /// [`RunState::apply`] does this first. A caller that is only reading
    /// inside the slice does it on its own, because a state standing at an
    /// instant holds the headings of that instant — which is what a keyframe
    /// written there states, and what makes a seek and a replay agree.
    pub fn enter_slice(&mut self, begins: SimTime, delta: &Delta) -> Result<()> {
        self.stands_at(begins)?;
        if let Some((held, spans)) = self.robot_spans(begins, delta)? {
            held.check(begins, None, spans)?;
        }
        self.take_slice_start(delta);
        Ok(())
    }

    /// What a delta states about its slice's start, checked already: the
    /// headings, and the state each robot that enters a span there begins
    /// the slice in.
    fn take_slice_start(&mut self, delta: &Delta) {
        if let Some(playback) = &delta.playback {
            for (robot, heading_mrad) in &playback.headings {
                if let Some(state) = self.playback.robots.get_mut(robot) {
                    state.heading_mrad = *heading_mrad;
                }
            }
        }
        if let (Some(held), Some(spans)) = (&mut self.robot_states, &delta.robot_state) {
            held.enter(spans);
        }
    }

    /// The robot states this state holds and the spans a delta states, once
    /// the two agree that the recording states the channel. A delta that
    /// states it where the keyframe did not, or leaves it out where the
    /// keyframe stated it, is a recording that stopped or started recording
    /// it part way, which no reader can make one reading of.
    fn robot_spans<'a>(
        &'a self,
        begins: SimTime,
        delta: &'a Delta,
    ) -> Result<Option<(&'a RobotStates, &'a RobotStateDelta)>> {
        let at_ms = begins.as_ms();
        match (&self.robot_states, &delta.robot_state) {
            (Some(held), Some(spans)) => Ok(Some((held, spans))),
            (None, None) => Ok(None),
            (Some(_), None) => {
                Err(Error::RobotStateNotInEveryFrame { at_ms, detail: "states no robot state where the keyframe did" })
            }
            (None, Some(_)) => {
                Err(Error::RobotStateNotInEveryFrame { at_ms, detail: "states robot state where the keyframe did not" })
            }
        }
    }

    /// Take in one whole slice: the one beginning at `begins`, which is where
    /// this state stands, and ending at `ends`, which is where it then
    /// stands. A caller passes the delta's own instant and the next frame's:
    /// for the run's last slice, the end block's, which is the run's stop.
    /// The robot state channel's spans must lie across exactly that stretch.
    ///
    /// Nothing changes until everything that can be refused has been refused:
    /// a delta this state cannot take leaves it exactly as it was, so a
    /// caller that reports the damage and reads on is holding a state the
    /// file really described.
    pub fn apply(&mut self, begins: SimTime, ends: SimTime, delta: &Delta) -> Result<()> {
        self.stands_at(begins)?;
        if ends < begins {
            return Err(Error::SliceEndsBeforeItBegins { begins_ms: begins.as_ms(), ends_ms: ends.as_ms() });
        }
        if let Some(playback) = &delta.playback {
            for mover in &playback.movers {
                let (began, _) = self.stood(mover.robot, begins)?;
                walkable(mover, began, begins)?;
            }
            introduced(&self.playback.totes, begins, ends, playback)?;
        }
        if let Some(scalars) = &delta.scalars {
            declared(scalars.values.len(), self.scalars.slots())?;
        }
        let raised = match &delta.heat {
            Some(heat) => self.heat.raised(heat)?,
            None => Vec::new(),
        };
        if let Some((held, spans)) = self.robot_spans(begins, delta)? {
            held.check(begins, Some(ends - begins), spans)?;
        }

        self.take_slice_start(delta);
        if let Some(playback) = &delta.playback {
            self.playback.apply(begins, ends, playback);
        }
        if let Some(scalars) = &delta.scalars {
            self.scalars.apply(scalars);
        }
        self.heat.commit(raised);
        if let (Some(held), Some(spans)) = (&mut self.robot_states, &delta.robot_state) {
            held.leave(spans);
        }
        self.reached = ends;
        Ok(())
    }

    /// Where everything is at `t`, inside the slice beginning at `begins`,
    /// which is where this state stands. Nothing is applied: the state stays
    /// at the slice's start, so a caller can ask for another instant of the
    /// same slice.
    pub fn at(&self, begins: SimTime, t: SimTime, delta: &Delta) -> Result<PlaybackAt> {
        self.stands_at(begins)?;
        if t < begins {
            return Err(Error::InstantOutsideSlice { t_ms: t.as_ms(), slice_start_ms: begins.as_ms() });
        }
        let elapsed_ms = t.as_ms() - begins.as_ms();
        let mut robots: FxHashMap<RobotId, RobotAt> = self
            .playback
            .robots
            .iter()
            .map(|(robot, state)| {
                (*robot, RobotAt { at: state.at, cell: state.cell, heading_mrad: state.heading_mrad })
            })
            .collect();
        let mut totes: FxHashMap<ToteId, ToteLocation> =
            self.playback.totes.iter().map(|(tote, state)| (*tote, state.location)).collect();

        if let Some(playback) = &delta.playback {
            introduced(&self.playback.totes, begins, t, playback)?;
            // The slice's own headings come first: they are true from its
            // start, whether or not the robot went anywhere afterwards.
            for (robot, heading_mrad) in &playback.headings {
                if let Some(held) = robots.get_mut(robot) {
                    held.heading_mrad = *heading_mrad;
                }
            }
            for mover in &playback.movers {
                let stood = self.stood(mover.robot, begins)?;
                walkable(mover, stood.0, begins)?;
                let (at, cell) = step_place(mover, stood, elapsed_ms)?;
                let held = robots.get(&mover.robot).and_then(|held| held.heading_mrad);
                robots.insert(mover.robot, RobotAt { at, cell, heading_mrad: mover.start_heading_mrad.or(held) });
            }
            for placement in placements(begins, t, playback) {
                match placement.location {
                    ToteLocation::Extracted => totes.remove(&placement.tote),
                    location => totes.insert(placement.tote, location),
                };
            }
        }
        // A delta that states no robot state says nothing about it: the
        // default delta a reader passes at the state's own instant says
        // nothing about anything.
        let robot_states = match (&self.robot_states, &delta.robot_state) {
            (Some(held), None) => Some(held.clone()),
            _ => match self.robot_spans(begins, delta)? {
                Some((held, spans)) => {
                    held.check(begins, None, spans)?;
                    Some(held.at(t - begins, spans))
                }
                None => None,
            },
        };
        Ok(PlaybackAt { at: t, robots, totes, robot_states })
    }

    /// Refuse a delta that is not the next one for this state. A slice's
    /// offsets are measured from where the state stands, so reading another
    /// slice's would place everything by a clock it never ran on.
    fn stands_at(&self, begins: SimTime) -> Result<()> {
        match begins == self.reached {
            true => Ok(()),
            false => Err(Error::DeltaNotAtState { begins_ms: begins.as_ms(), reached_ms: self.reached.as_ms() }),
        }
    }

    /// Where a robot the delta names stands and the cell it stands in, or a
    /// complaint naming the run's fault: a mover no keyframe ever placed
    /// cannot be drawn.
    fn stood(&self, robot: RobotId, at: SimTime) -> Result<(Position, CellId)> {
        self.playback
            .robots
            .get(&robot)
            .map(|state| (state.at, state.cell))
            .ok_or(Error::DeltaMovesUnplacedRobot { at_ms: at.as_ms(), robot: robot.0 })
    }
}

/// Refuse a keyframe that places a robot and states no state for it, or
/// states one for a robot it does not place: a keyframe states every robot's
/// state at its instant, so a reader beginning there knows what each robot it
/// draws is doing.
fn every_placed_robot_stated(at: SimTime, placed: &PlaybackKeyframe, states: &RobotStates) -> Result<()> {
    let disagree =
        |robot: RobotId, detail| Error::KeyframeRobotStatesDisagree { at_ms: at.as_ms(), robot: robot.0, detail };
    if let Some(record) = placed.robots.iter().find(|record| !states.robots.contains_key(&record.robot)) {
        return Err(disagree(record.robot, "places and states no state for"));
    }
    let placed_ids: FxHashSet<RobotId> = placed.robots.iter().map(|record| record.robot).collect();
    match states.robots.keys().find(|robot| !placed_ids.contains(robot)) {
        Some(robot) => Err(disagree(*robot, "states the state of, and does not place,")),
        None => Ok(()),
    }
}

/// Refuse a scalars array that names metrics the run never declared. A
/// shorter array is ordinary — the slots after it had not been declared when
/// the frame was written — but a longer one has no descriptor to say what its
/// last values count or how they combine.
fn declared(slots: usize, declared: usize) -> Result<()> {
    match slots <= declared {
        true => Ok(()),
        false => Err(Error::ScalarsPastDescriptors { slots, declared }),
    }
}

/// Refuse a mover a slice cannot have walked.
///
/// A mover is a robot that MOVED — a robot that stayed put has no entry at all
/// — so one stating no steps says nothing about where it went, and yet its own
/// `cell` would still carry the robot's occupancy to a cell it never travelled
/// to. And a step of no duration that also changes the position says the robot
/// crossed that distance in no time; a reader drawing the slice would have it
/// at the far end at the slice's opening instant. A step of no duration that
/// leaves the position where it was is ordinary — it is how a turn or a wait
/// of no measured length is written — so only the moving one is refused.
fn walkable(mover: &MoverSteps, began: Position, at: SimTime) -> Result<()> {
    let (at_ms, robot) = (at.as_ms(), mover.robot.0);
    if mover.steps.is_empty() {
        return Err(Error::MoverWithNoSteps { at_ms, robot });
    }
    let mut before = began;
    for (step, taken) in mover.steps.iter().enumerate() {
        if taken.dt.as_ms() == 0 && taken.at != before {
            return Err(Error::StepOfNoDurationMoves { at_ms, robot, step });
        }
        before = taken.at;
    }
    Ok(())
}

/// Refuse a slice that moves or fills a container the run never introduced.
///
/// What kind of container something is, and which flow it belongs to, are
/// stated exactly twice: by a keyframe that lists it, and by its own creation
/// record. A transition or a contents record naming a container neither of
/// those introduced leaves a viewer with a place and no thing to put there,
/// and the only ways on are to invent a kind or to draw nothing — so the
/// recording is wrong and says so here, before any of it is taken in.
fn introduced(held: &FxHashMap<ToteId, ToteState>, begins: SimTime, cut: SimTime, delta: &PlaybackDelta) -> Result<()> {
    let mut known: FxHashSet<ToteId> = held.keys().copied().collect();
    let refuse = |tote: ToteId, what| Err(Error::DeltaMovesUnknownTote { at_ms: begins.as_ms(), tote: tote.0, what });
    for placement in placements(begins, cut, delta) {
        match placement.born {
            Some(_) => {
                known.insert(placement.tote);
            }
            None if !known.contains(&placement.tote) => return refuse(placement.tote, "moves"),
            None => {}
        }
    }
    let elapsed_ms = cut.as_ms().saturating_sub(begins.as_ms());
    for contents in delta.tote_contents.iter().filter(|contents| contents.offset.as_ms() <= elapsed_ms) {
        if !known.contains(&contents.tote) {
            return refuse(contents.tote, "fills");
        }
    }
    Ok(())
}

/// Where a mover is once `elapsed_ms` of its slice have passed, and the cell
/// it counts against there.
///
/// The steps state whole millimetres at both ends of the one the instant falls
/// in, so the answer is a straight interpolation between them rounded back to
/// whole millimetres — the same arithmetic wherever it is asked for, which is
/// what makes a seek and a replay put the robot in one place. A mover part-way
/// along a step counts against the cell it left until it arrives, because that
/// is the cell it still occupies.
fn step_place(mover: &MoverSteps, stood: (Position, CellId), elapsed_ms: u64) -> Result<(Position, CellId)> {
    let (began, began_cell) = stood;
    let mut before = began;
    let mut source = began_cell;
    let mut walked_ms = 0u64;
    for step in &mover.steps {
        let step_ms = step.dt.as_ms();
        // The durations come out of a frame, so they are added with a check:
        // steps that reach past the end of time are damage, not a wrapped
        // clock and a mover drawn somewhere it never was.
        let ends_ms = walked_ms.checked_add(step_ms).ok_or(Error::StepsOverflow { robot: mover.robot.0 })?;
        if elapsed_ms < ends_ms {
            if !step.kind.travels() {
                // A wait or a turn spends its time where the step leaves the
                // mover, so its own coordinates are the whole answer.
                return Ok((step.at, step.cell));
            }
            // A step of no duration cannot be the one holding the instant, so
            // this arm only keeps the division from dividing by nothing.
            let along = match step_ms {
                0 => 0.0,
                span => (elapsed_ms - walked_ms) as f64 / span as f64,
            };
            return Ok((before.lerp(step.at, along), source));
        }
        walked_ms = ends_ms;
        before = step.at;
        source = step.cell;
    }
    Ok((mover.ends_at(began), mover.cell))
}

impl PlaybackState {
    /// The state a keyframe states outright.
    pub fn from_keyframe(keyframe: &PlaybackKeyframe) -> Self {
        Self {
            robots: keyframe
                .robots
                .iter()
                .map(|robot| {
                    (
                        robot.robot,
                        RobotPlace {
                            class: robot.class,
                            at: robot.at,
                            heading_mrad: robot.heading_mrad,
                            carrying: robot.carrying,
                            cell: robot.cell,
                            toward: robot.toward,
                        },
                    )
                })
                .collect(),
            totes: keyframe
                .totes
                .iter()
                .map(|tote| {
                    (
                        tote.tote,
                        ToteState {
                            tote_type: tote.tote_type,
                            designation: tote.designation,
                            location: tote.location,
                            contents_units: tote.contents_units,
                        },
                    )
                })
                .collect(),
            reserved_relay_cells: keyframe.reserved_relay_cells.iter().copied().collect(),
            stations: keyframe.stations.iter().map(|station| (station.station, station.pipeline_depth)).collect(),
            transits_in_flight: keyframe.transits_in_flight.clone(),
        }
    }

    /// Take one closed slice, from `start` to `end`. A tote's three kinds of
    /// move — its creation, a transit coming to rest, a transition — sit in
    /// three arrays with clocks of their own, so they are merged and applied
    /// in the order they happened; for everything else the last record the
    /// producer wrote for a thing is the one that stands.
    fn apply(&mut self, begins: SimTime, ends: SimTime, delta: &PlaybackDelta) {
        for placement in placements(begins, ends, delta) {
            self.place(placement.tote, placement.location, placement.born);
        }
        for contents in &delta.tote_contents {
            if let Some(tote) = self.totes.get_mut(&contents.tote) {
                tote.contents_units = contents.units;
            }
        }
        for relay in &delta.relay_transitions {
            match relay.reserved {
                true => self.reserved_relay_cells.insert(relay.cell),
                false => self.reserved_relay_cells.remove(&relay.cell),
            };
        }
        for change in &delta.station_changes {
            self.stations.insert(change.station, change.pipeline_depth);
        }
        for mover in &delta.movers {
            // Every mover was checked against the state before anything here
            // was touched, so one missing now cannot happen.
            let Some(robot) = self.robots.get_mut(&mover.robot) else { continue };
            robot.at = mover.ends_at(robot.at);
            robot.cell = mover.cell;
            robot.toward = mover.heading_for();
            if let Some(heading) = mover.start_heading_mrad {
                robot.heading_mrad = Some(heading);
            }
        }
        // A journey that has landed by this instant is a place, not a
        // journey; the rest are still under way, and a keyframe here would
        // state them — each of them once, however many slices it spans.
        self.transits_in_flight.retain(|transit| transit.end > ends);
        for transit in delta.tote_transits.iter().filter(|transit| transit.end > ends) {
            if !self.transits_in_flight.contains(transit) {
                self.transits_in_flight.push(*transit);
            }
        }
    }

    /// Put a tote at a location, keeping the robots in step with it: the
    /// robot the tote was riding stops carrying it, and the robot it moves
    /// onto starts. A keyframe states both sides of that pairing, so this is
    /// how a run reached by applying deltas ends up holding what the next
    /// keyframe would have said.
    /// `born` is the container type and pool a CREATION record states, and
    /// nothing else states them. A container this state has not met and no
    /// creation introduces is refused a slice earlier, by
    /// [`introduced`] — there is no kind to draw it as, and inventing one
    /// would draw a carton as a tote and say nothing.
    fn place(&mut self, tote: ToteId, location: ToteLocation, born: Option<(u8, u8)>) {
        let previous = match (self.totes.get_mut(&tote), born) {
            (Some(record), born) => {
                let was = record.location;
                record.location = location;
                // A creation restates the two permanent facts as it places the
                // container. It never resets the units, which the container's
                // own contents records carry.
                if let Some((tote_type, designation)) = born {
                    record.tote_type = tote_type;
                    record.designation = designation;
                }
                Some(was)
            }
            (None, Some((tote_type, designation))) => {
                self.totes.insert(tote, ToteState { tote_type, designation, location, contents_units: 0 });
                None
            }
            (None, None) => return,
        };
        if let Some(ToteLocation::OnBot(carrier)) = previous
            && let Some(robot) = self.robots.get_mut(&carrier)
            && robot.carrying == Some(tote)
        {
            robot.carrying = None;
        }
        if let ToteLocation::OnBot(carrier) = location
            && let Some(robot) = self.robots.get_mut(&carrier)
        {
            robot.carrying = Some(tote);
        }
        if location == ToteLocation::Extracted {
            // Extracted is where a tote goes when it leaves the run, and
            // nothing draws it or asks about it afterwards.
            self.totes.remove(&tote);
        }
    }
}

/// One record of a slice that puts a tote somewhere, and when in the slice it
/// takes effect.
struct Placement {
    offset_ms: u64,
    /// Which kind of record this is, which is the order two of them sharing
    /// an instant are applied in: a creation, then a transit coming to rest,
    /// then a transition.
    rank: u8,
    tote: ToteId,
    location: ToteLocation,
    /// The container type and the pool a creation states; the other two kinds
    /// say nothing about either.
    born: Option<(u8, u8)>,
}

/// Every move the slice makes up to `cut`, in the order they happen.
///
/// A transit is keyed by the instant it comes to rest, since that is when the
/// tote is at its landmark: one that ended before the slice opened has
/// already landed and sorts first, and one whose end is past `cut` has not
/// landed at all and is left out — the tote is still travelling, and the
/// state keeps the journey in flight instead.
fn placements(begins: SimTime, cut: SimTime, delta: &PlaybackDelta) -> Vec<Placement> {
    let mut moves =
        Vec::with_capacity(delta.tote_creations.len() + delta.tote_transits.len() + delta.tote_transitions.len());
    let elapsed_ms = cut.as_ms().saturating_sub(begins.as_ms());
    for creation in delta.tote_creations.iter().filter(|creation| creation.offset.as_ms() <= elapsed_ms) {
        moves.push(Placement {
            offset_ms: creation.offset.as_ms(),
            rank: 0,
            tote: creation.tote,
            location: creation.location,
            born: Some((creation.tote_type, creation.designation)),
        });
    }
    for transit in delta.tote_transits.iter().filter(|transit| transit.end <= cut) {
        moves.push(Placement {
            offset_ms: transit.end.as_ms().saturating_sub(begins.as_ms()),
            rank: 1,
            tote: transit.tote,
            location: ToteLocation::OnConveyor(transit.to_cell),
            born: None,
        });
    }
    for transition in delta.tote_transitions.iter().filter(|transition| transition.offset.as_ms() <= elapsed_ms) {
        moves.push(Placement {
            offset_ms: transition.offset.as_ms(),
            rank: 2,
            tote: transition.tote,
            location: transition.location,
            born: None,
        });
    }
    // Stable, and by nothing but the instant and the kind, so records of one
    // kind sharing an instant stay in the order the run emitted them.
    moves.sort_by_key(|placement| (placement.offset_ms, placement.rank));
    moves
}

impl ScalarsState {
    pub fn from_keyframe(keyframe: &ScalarsKeyframe) -> Self {
        Self { values: keyframe.values.clone(), accumulates: Vec::new() }
    }

    /// How many metrics the run had declared when this state was described,
    /// which is how many slots a frame may state.
    pub fn slots(&self) -> usize {
        self.accumulates.len()
    }

    /// Say which metric sits in each slot, so a gauge is read as a reading
    /// and not as a sum of readings.
    pub fn describe(&mut self, descriptors: &Descriptors) {
        self.accumulates =
            descriptors.metrics.iter().map(|metric| metric.semantics.aggregation.accumulates()).collect();
    }

    /// Take one slice's values: an accumulating metric's add onto the running
    /// total, a gauge's replace the reading before it. A slot the slice says
    /// nothing about keeps what it held, and one nothing has ever said stays
    /// absent.
    pub fn apply(&mut self, delta: &ScalarsDelta) {
        if delta.values.len() > self.values.len() {
            self.values.resize(delta.values.len(), None);
        }
        for (slot, stated) in delta.values.iter().enumerate() {
            let Some(value) = stated else { continue };
            let running = self.values[slot].filter(|_| self.accumulates.get(slot).copied().unwrap_or(true));
            self.values[slot] = Some(running.unwrap_or(0.0) + value);
        }
    }
}

impl HeatState {
    pub fn from_keyframe(keyframe: &HeatKeyframe) -> Self {
        let layers =
            keyframe.layers.iter().map(|layer| (layer.name.clone(), layer.counts.iter().copied().collect())).collect();
        Self { layers }
    }

    /// What one slice's increments make of the counters, worked out before
    /// anything is changed.
    ///
    /// A counter that passes what it can hold says the file is damaged — no
    /// run counts a cell eighteen quintillion times — and a half-added state
    /// would be worse than the complaint, so the additions are checked here
    /// and committed only once they have all come out.
    pub fn raised<'a>(&self, delta: &'a HeatDelta) -> Result<Vec<(&'a str, CellId, u64)>> {
        let mut raised: FxHashMap<(&str, CellId), u64> = FxHashMap::default();
        for layer in &delta.layers {
            for (cell, increment) in &layer.increments {
                let held = match raised.get(&(layer.name.as_str(), *cell)) {
                    Some(running) => *running,
                    None => self.layers.get(&layer.name).and_then(|counts| counts.get(cell)).copied().unwrap_or(0),
                };
                let count = held
                    .checked_add(*increment)
                    .ok_or_else(|| Error::CounterOverflow { layer: layer.name.clone(), cell: cell.0 })?;
                raised.insert((layer.name.as_str(), *cell), count);
            }
        }
        Ok(raised.into_iter().map(|((name, cell), count)| (name, cell, count)).collect())
    }

    /// Take the counts [`HeatState::raised`] worked out, creating a layer the
    /// first time it appears.
    pub fn commit(&mut self, raised: Vec<(&str, CellId, u64)>) {
        for (name, cell, count) in raised {
            match self.layers.get_mut(name) {
                Some(counts) => {
                    counts.insert(cell, count);
                }
                None => {
                    self.layers.insert(name.to_owned(), FxHashMap::from_iter([(cell, count)]));
                }
            }
        }
    }
}

/// The state at `t`, and where everything is inside the slice `t` falls in —
/// what a viewer asks for when somebody drags the scrubber.
///
/// The scenario block is read for the descriptors its metrics fix, which
/// costs one frame. A caller scrubbing a run holds the scenario block already
/// and should call [`state_at_described`] instead of reading it again at
/// every tick.
pub fn state_at<R: Read + Seek>(reader: &mut Reader<R>, t: SimTime) -> Result<(RunState, PlaybackAt)> {
    let described = Descriptors::from_scenario(&Scenario::read_from(reader)?);
    state_at_described(reader, t, &described)
}

/// The same answer for a caller that already knows what the run declared at
/// its start.
///
/// This is the container's seek plan carried out: the keyframe at or before
/// `t`, every declaration made by then, then the deltas, all but the last
/// applied whole and the last read into. A `t` before the run's first
/// keyframe answers with that keyframe, as the seek itself does.
///
/// The state that comes back stands at the start of the slice `t` falls in;
/// what happens inside that slice is in the places beside it.
pub fn state_at_described<R: Read + Seek>(
    reader: &mut Reader<R>,
    t: SimTime,
    described: &Descriptors,
) -> Result<(RunState, PlaybackAt)> {
    let plan = reader.seek(stamp(t))?;
    let mut descriptors = described.clone();
    for record in &plan.declares {
        descriptors.push(&reader.payload_at::<Declare>(record)?);
    }

    let sections = reader.sections_at(&plan.keyframe)?;
    let keyframe = Keyframe::decode(&sections, plan.keyframe.offset)?;
    let mut state = RunState::from_keyframe(instant(plan.keyframe.t), &keyframe, &descriptors)?;

    let Some((last, whole)) = plan.deltas.split_last() else {
        let reached = state.reached;
        let at = state.at(reached, t.max(reached), &Delta::default())?;
        return Ok((state, at));
    };
    for (index, record) in whole.iter().enumerate() {
        let sections = reader.sections_at(record)?;
        let delta = Delta::decode(&sections, record.offset)?;
        // Each delta states where its own slice begins, and the next frame
        // says where it ends; the last whole one is followed by the delta the
        // instant falls in.
        let ends = whole.get(index + 1).unwrap_or(last).t;
        state.apply(instant(record.t), instant(ends), &delta)?;
    }
    let sections = reader.sections_at(last)?;
    let delta = Delta::decode(&sections, last.offset)?;
    // The last slice is not applied, but the state stands at its start, so
    // what that delta says about its start belongs to the state.
    let begins = instant(last.t);
    state.enter_slice(begins, &delta)?;
    let at = state.at(begins, t.max(begins), &delta)?;
    Ok((state, at))
}

#[cfg(test)]
mod tests {
    use desunits::SimDuration;

    use super::*;
    use crate::frames::heat::{HeatLayer, HeatLayerDelta};
    use crate::frames::playback::{
        RobotRecord, Step, StepKind, ToteContents, ToteCreation, ToteRecord, ToteTransit, ToteTransition,
    };
    use crate::frames::robot_state::{
        CellRole, RobotSpans, RobotState, RobotStateDelta, RobotStateKeyframe, StateSpan,
    };
    use crate::frames::scalars::{AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit};

    /// One metric of a given class, which is all a scalars test needs of it.
    fn metric(aggregation: AggregationClass) -> MetricDescriptor {
        MetricDescriptor {
            name: "metric".to_owned(),
            label: "Metric".to_owned(),
            description: String::new(),
            semantics: MetricSemantics { unit: MetricUnit::Count, denominator_description: None, aggregation },
        }
    }

    const SLICE: SimTime = SimTime::ms(10_000);

    fn one_tote_on_a_conveyor() -> RunState {
        let keyframe = Keyframe {
            playback: Some(PlaybackKeyframe {
                totes: vec![ToteRecord {
                    tote: ToteId(1),
                    tote_type: 1,
                    designation: 0,
                    location: ToteLocation::OnConveyor(CellId(9)),
                    contents_units: 0,
                }],
                ..PlaybackKeyframe::default()
            }),
            ..Keyframe::default()
        };
        RunState::from_keyframe(SLICE, &keyframe, &Descriptors::default()).expect("a keyframe of one slot")
    }

    /// A robot in cell `cell`, standing `cell` metres along the x axis: the
    /// coordinates are the state's own truth, and the cell rides beside them.
    fn one_robot_at(cell: u32) -> RunState {
        let keyframe = Keyframe {
            playback: Some(PlaybackKeyframe {
                robots: vec![RobotRecord {
                    robot: RobotId(1),
                    class: RobotClass::Htm,
                    at: Position::new(cell as i32 * 1_000, 0, 0),
                    heading_mrad: Some(0),
                    carrying: None,
                    cell: CellId(cell),
                    toward: None,
                }],
                ..PlaybackKeyframe::default()
            }),
            ..Keyframe::default()
        };
        RunState::from_keyframe(SLICE, &keyframe, &Descriptors::default()).expect("a keyframe of one slot")
    }

    /// Two two-second steps: into cell 5 at x = 5,000 mm, then into cell 6,
    /// which the slice cuts at `ends_at_mm` rather than at the cell.
    fn walking(ends_at_mm: i32) -> Delta {
        let step = |x: i32, cell: u32| Step {
            at: Position::new(x, 0, 0),
            dt: SimDuration::ms(2_000),
            kind: StepKind::Move,
            cell: CellId(cell),
        };
        Delta {
            playback: Some(PlaybackDelta {
                movers: vec![MoverSteps {
                    robot: RobotId(1),
                    start_heading_mrad: Some(1_571),
                    steps: vec![step(5_000, 5), step(ends_at_mm, 6)],
                    cell: CellId(6),
                }],
                ..PlaybackDelta::default()
            }),
            ..Delta::default()
        }
    }

    /// A conveyor hands a tote to a robot inside one slice — the induction
    /// path every cencora run walks. The transit lands first and the
    /// transition takes the tote off the conveyor after it, so the slice ends
    /// with the tote on the robot.
    #[test]
    fn a_transit_landing_before_a_transition_leaves_the_transition_standing() {
        let mut state = one_tote_on_a_conveyor();
        let delta = Delta {
            playback: Some(PlaybackDelta {
                tote_transitions: vec![ToteTransition {
                    offset: SimDuration::ms(5_000),
                    tote: ToteId(1),
                    location: ToteLocation::OnBot(RobotId(0)),
                }],
                tote_transits: vec![ToteTransit {
                    tote: ToteId(1),
                    from_cell: CellId(9),
                    to_cell: CellId(10),
                    start: SLICE,
                    end: SLICE.saturating_add(SimDuration::ms(2_000)),
                }],
                ..PlaybackDelta::default()
            }),
            ..Delta::default()
        };
        state.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &delta).expect("applies");
        assert_eq!(state.playback.totes[&ToteId(1)].location, ToteLocation::OnBot(RobotId(0)));
        assert!(state.playback.transits_in_flight.is_empty(), "the journey landed inside the slice");
    }

    /// A slice that cuts a leg in half states the point it cut at, so reading
    /// into the slice interpolates between whole millimetres and counts the
    /// mover against the cell it has not yet left.
    #[test]
    fn a_mover_part_way_through_a_step_is_between_its_two_stated_points() {
        let state = one_robot_at(4);
        let delta = walking(5_600);
        let quarter = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(1_000)), &delta).expect("reads the slice");
        assert_eq!(
            (quarter.robots[&RobotId(1)].at, quarter.robots[&RobotId(1)].cell),
            (Position::new(4_500, 0, 0), CellId(4)),
            "half way from cell 4's 4,000 mm to cell 5's 5,000 mm, and still counted in cell 4"
        );
        let later = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(3_000)), &delta).expect("reads the slice");
        assert_eq!(
            (later.robots[&RobotId(1)].at, later.robots[&RobotId(1)].cell),
            (Position::new(5_300, 0, 0), CellId(5)),
            "half way through the last step, which the slice cut at 5,600 mm"
        );
        assert_eq!(later.robots[&RobotId(1)].heading_mrad, Some(1_571), "the delta's heading, not the keyframe's");
    }

    #[test]
    fn where_a_slice_leaves_a_mover_is_where_the_next_slice_finds_it() {
        let mut state = one_robot_at(4);
        let delta = walking(5_600);
        let at_end = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(4_000)), &delta).expect("reads the slice");
        state.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &delta).expect("applies");
        assert_eq!(state.playback.robots[&RobotId(1)].at, at_end.robots[&RobotId(1)].at);
        assert_eq!(state.playback.robots[&RobotId(1)].at, Position::new(5_600, 0, 0));
        assert_eq!(state.playback.robots[&RobotId(1)].cell, CellId(6), "the cut leg's step named the cell it entered");
    }

    #[test]
    fn a_robot_that_turns_without_moving_is_turned_by_the_slice_it_turned_in() {
        let mut state = one_robot_at(4);
        let turning = Delta {
            playback: Some(PlaybackDelta { headings: vec![(RobotId(1), Some(3_142))], ..PlaybackDelta::default() }),
            ..Delta::default()
        };
        let inside = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(1)), &turning).expect("reads the slice");
        assert_eq!(inside.robots[&RobotId(1)].heading_mrad, Some(3_142), "true from the slice's first instant");
        state.enter_slice(SLICE, &turning).expect("the slice begins where the state stands");
        assert_eq!(
            state.playback.robots[&RobotId(1)].heading_mrad,
            Some(3_142),
            "and part of the state standing where that slice begins"
        );
    }

    /// A journey that is still running when the slice closes has not put its
    /// tote anywhere: the tote is travelling, and the state carries the
    /// journey forward instead of parking the tote at its destination.
    #[test]
    fn a_transit_reaching_past_its_slice_lands_in_no_slice_it_only_crosses() {
        let mut state = one_tote_on_a_conveyor();
        let crossing = ToteTransit {
            tote: ToteId(1),
            from_cell: CellId(9),
            to_cell: CellId(10),
            start: SLICE,
            end: SLICE.saturating_add(SimDuration::ms(25_000)),
        };
        let delta = Delta {
            playback: Some(PlaybackDelta { tote_transits: vec![crossing], ..PlaybackDelta::default() }),
            ..Delta::default()
        };
        let ends = SLICE.saturating_add(SimDuration::ms(10_000));
        let inside = state.at(SLICE, ends, &delta).expect("reads the slice");
        assert_eq!(inside.totes[&ToteId(1)], ToteLocation::OnConveyor(CellId(9)), "still where it set off from");
        state.apply(SLICE, ends, &delta).expect("applies");
        assert_eq!(state.playback.totes[&ToteId(1)].location, ToteLocation::OnConveyor(CellId(9)));
        assert_eq!(state.playback.transits_in_flight, vec![crossing], "the journey is carried forward");

        // The next slice states the same journey, and a keyframe here would
        // state it once — so taking it twice would draw two of it.
        state.apply(ends, ends.saturating_add(SimDuration::ms(10_000)), &delta).expect("applies again");
        assert_eq!(state.playback.transits_in_flight, vec![crossing], "and only once");
    }

    /// A tote created and moved in one slice, its records sharing an instant.
    /// The creation is what puts the tote in the run, so it happens first
    /// however the producer wrote the arrays.
    #[test]
    fn a_creation_sharing_an_instant_with_a_move_and_a_fill_happens_first() {
        let mut state = RunState::default();
        let tote = ToteId(4);
        let at = SimDuration::ms(2_500);
        let delta = Delta {
            playback: Some(PlaybackDelta {
                tote_creations: vec![ToteCreation {
                    offset: at,
                    tote,
                    tote_type: 2,
                    designation: 1,
                    location: ToteLocation::AtCell(CellId(7)),
                }],
                tote_transitions: vec![ToteTransition { offset: at, tote, location: ToteLocation::AtRelay(CellId(8)) }],
                tote_contents: vec![ToteContents { offset: at, tote, units: 6 }],
                ..PlaybackDelta::default()
            }),
            ..Delta::default()
        };
        state.apply(SimTime::ZERO, SimTime::ms(10_000), &delta).expect("applies");
        let held = state.playback.totes[&tote];
        assert_eq!(held.location, ToteLocation::AtRelay(CellId(8)), "the move lands on the tote the creation made");
        assert_eq!((held.tote_type, held.designation, held.contents_units), (2, 1, 6));
    }

    /// A transition or a contents record naming a container the run never
    /// introduced is damage: what kind of container it is was never said, and
    /// a reader that picked one would draw a carton as a tote and say nothing.
    #[test]
    fn a_slice_that_moves_a_container_no_creation_introduced_is_an_error() {
        let tote = ToteId(4);
        let moved = |playback| Delta { playback: Some(playback), ..Delta::default() };
        let transition = moved(PlaybackDelta {
            tote_transitions: vec![ToteTransition {
                offset: SimDuration::ZERO,
                tote,
                location: ToteLocation::AtCell(CellId(7)),
            }],
            ..PlaybackDelta::default()
        });
        let filled = moved(PlaybackDelta {
            tote_contents: vec![ToteContents { offset: SimDuration::ZERO, tote, units: 6 }],
            ..PlaybackDelta::default()
        });

        for (delta, what) in [(&transition, "moves"), (&filled, "fills")] {
            let mut state = RunState::default();
            let refused = state.apply(SimTime::ZERO, SimTime::ms(10_000), delta).expect_err("no such container");
            assert!(matches!(refused, Error::DeltaMovesUnknownTote { tote: 4, .. }), "{refused}");
            assert!(refused.to_string().contains(what), "the message says what the slice did: {refused}");
            assert!(state.playback.totes.is_empty(), "and nothing was taken in");
            let read = state.at(SimTime::ZERO, SimTime::ms(5_000), delta);
            assert!(read.is_err(), "a seek into the same slice refuses it too");
        }

        // The same records are fine once a creation has introduced it, and the
        // kind they draw it as is the creation's, never a default.
        let mut state = RunState::default();
        let born = moved(PlaybackDelta {
            tote_creations: vec![ToteCreation {
                offset: SimDuration::ZERO,
                tote,
                tote_type: 2,
                designation: 1,
                location: ToteLocation::AtCell(CellId(7)),
            }],
            tote_transitions: transition.playback.clone().expect("stated").tote_transitions,
            tote_contents: filled.playback.clone().expect("stated").tote_contents,
            ..PlaybackDelta::default()
        });
        state.apply(SimTime::ZERO, SimTime::ms(10_000), &born).expect("the creation introduces it");
        assert_eq!((state.playback.totes[&tote].tote_type, state.playback.totes[&tote].contents_units), (2, 6));
    }

    #[test]
    fn a_reversing_step_travels_its_leg_and_a_turning_one_holds_where_it_stands() {
        let state = one_robot_at(4);
        let step = |x: i32, cell: u32, kind| Step {
            at: Position::new(x, 0, 0),
            dt: SimDuration::ms(1_000),
            kind,
            cell: CellId(cell),
        };
        let delta = Delta {
            playback: Some(PlaybackDelta {
                movers: vec![MoverSteps {
                    robot: RobotId(1),
                    start_heading_mrad: None,
                    steps: vec![step(4_000, 4, StepKind::Rotate), step(3_000, 3, StepKind::Reverse)],
                    cell: CellId(3),
                }],
                ..PlaybackDelta::default()
            }),
            ..Delta::default()
        };
        let turning = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(500)), &delta).expect("reads the slice");
        assert_eq!(turning.robots[&RobotId(1)].at, Position::new(4_000, 0, 0), "a turn spends its time where it is");
        let backing = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(1_500)), &delta).expect("reads the slice");
        assert_eq!(backing.robots[&RobotId(1)].at, Position::new(3_500, 0, 0), "a reverse travels its leg like a move");
    }

    #[test]
    fn a_gauge_takes_the_newest_reading_and_every_other_class_adds() {
        for class in [
            AggregationClass::Additive,
            AggregationClass::Interval,
            AggregationClass::Ratio,
            AggregationClass::DistinctCount,
            AggregationClass::SnapshotGauge,
        ] {
            let descriptors = Descriptors { metrics: vec![metric(class)], tables: Vec::new() };
            let keyframe =
                Keyframe { scalars: Some(ScalarsKeyframe { values: vec![Some(4.0)] }), ..Keyframe::default() };
            let mut state = RunState::from_keyframe(SimTime::ZERO, &keyframe, &descriptors).expect("a keyframe");
            let delta = Delta { scalars: Some(ScalarsDelta { values: vec![Some(3.0)] }), ..Delta::default() };
            state.apply(SimTime::ZERO, SLICE, &delta).expect("applies");
            let expected = match class {
                AggregationClass::SnapshotGauge => 3.0,
                _ => 7.0,
            };
            assert_eq!(state.scalars.values, vec![Some(expected)], "{class:?}");
        }
    }

    #[test]
    fn a_frame_stating_more_metrics_than_the_run_declared_is_damage() {
        let descriptors = Descriptors { metrics: vec![metric(AggregationClass::Additive)], tables: Vec::new() };
        let long =
            Keyframe { scalars: Some(ScalarsKeyframe { values: vec![Some(1.0), Some(2.0)] }), ..Keyframe::default() };
        let error = RunState::from_keyframe(SimTime::ZERO, &long, &descriptors).expect_err("two slots, one metric");
        assert!(matches!(error, Error::ScalarsPastDescriptors { slots: 2, declared: 1 }), "got {error}");

        let short = Keyframe { scalars: Some(ScalarsKeyframe::default()), ..Keyframe::default() };
        let mut state = RunState::from_keyframe(SimTime::ZERO, &short, &descriptors).expect("a shorter frame reads");
        let delta = Delta { scalars: Some(ScalarsDelta { values: vec![Some(1.0), Some(2.0)] }), ..Delta::default() };
        let error = state.apply(SimTime::ZERO, SLICE, &delta).expect_err("and a delta is refused the same way");
        assert!(matches!(error, Error::ScalarsPastDescriptors { slots: 2, declared: 1 }), "got {error}");
    }

    #[test]
    fn a_delta_the_state_refuses_leaves_it_exactly_as_it_was() {
        let mut state = one_tote_on_a_conveyor();
        let before = state.clone();
        let delta = Delta {
            playback: Some(PlaybackDelta {
                movers: vec![MoverSteps {
                    robot: RobotId(9),
                    start_heading_mrad: None,
                    steps: vec![Step {
                        at: Position::new(1_000, 0, 0),
                        dt: SimDuration::ms(1),
                        kind: StepKind::Move,
                        cell: CellId(1),
                    }],
                    cell: CellId(1),
                }],
                tote_transitions: vec![ToteTransition {
                    offset: SimDuration::ms(1),
                    tote: ToteId(1),
                    location: ToteLocation::AtCell(CellId(2)),
                }],
                ..PlaybackDelta::default()
            }),
            heat: Some(HeatDelta {
                layers: vec![HeatLayerDelta { name: "nav".to_owned(), increments: vec![(CellId(1), 1)] }],
            }),
            ..Delta::default()
        };
        let error = state.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &delta).expect_err("robot 9");
        assert!(matches!(error, Error::DeltaMovesUnplacedRobot { robot: 9, .. }), "got {error}");
        assert_eq!(state, before, "the tote the same delta moved stayed where it was");
    }

    /// A mover is a robot that MOVED, so a mover with no steps says nothing
    /// about where the robot went while still carrying it to another cell,
    /// and a step of no duration that changes the position has the robot
    /// cross that distance in no time. Both are refused on the way in, on the
    /// replay path and on the seek path alike.
    #[test]
    fn a_mover_a_slice_cannot_have_walked_is_refused_before_anything_is_taken_in() {
        let delta = |steps: Vec<Step>| Delta {
            playback: Some(PlaybackDelta {
                movers: vec![MoverSteps { robot: RobotId(1), start_heading_mrad: None, steps, cell: CellId(9) }],
                ..PlaybackDelta::default()
            }),
            ..Delta::default()
        };
        let step = |x: i32, dt_ms: u64| Step {
            at: Position::new(x, 0, 0),
            dt: SimDuration::ms(dt_ms),
            kind: StepKind::Move,
            cell: CellId(5),
        };
        let ends = SLICE.saturating_add(SimDuration::ms(10_000));

        let empty = delta(Vec::new());
        let mut state = one_robot_at(1);
        let before = state.clone();
        let error = state.apply(SLICE, ends, &empty).expect_err("a mover that states no step");
        assert!(matches!(error, Error::MoverWithNoSteps { robot: 1, .. }), "got {error}");
        assert_eq!(state, before, "and the robot is left on the cell the keyframe placed it on");
        let error = state.at(SLICE, ends, &empty).expect_err("a seek refuses it too");
        assert!(matches!(error, Error::MoverWithNoSteps { robot: 1, .. }), "got {error}");

        let teleport = delta(vec![step(9_000, 0)]);
        let error = state.apply(SLICE, ends, &teleport).expect_err("no time, and a metres-long move");
        assert!(matches!(error, Error::StepOfNoDurationMoves { robot: 1, step: 0, .. }), "got {error}");
        let error = state.at(SLICE, ends, &teleport).expect_err("a seek refuses it too");
        assert!(matches!(error, Error::StepOfNoDurationMoves { robot: 1, step: 0, .. }), "got {error}");

        // A step of no duration that leaves the position where it was is how a
        // turn or a wait of no measured length is written, and stays ordinary.
        let stood_still = delta(vec![step(1_000, 0), step(4_000, 2_000)]);
        state.apply(SLICE, ends, &stood_still).expect("a step of no length that moves nothing is fine");
    }

    #[test]
    fn a_counter_that_cannot_hold_its_increment_is_damage_and_not_a_count_that_stops() {
        let keyframe = Keyframe {
            heat: Some(HeatKeyframe {
                layers: vec![HeatLayer { name: "nav".to_owned(), counts: vec![(CellId(1), u64::MAX)] }],
            }),
            ..Keyframe::default()
        };
        let mut state = RunState::from_keyframe(SLICE, &keyframe, &Descriptors::default()).expect("a keyframe");
        let delta = Delta {
            heat: Some(HeatDelta {
                layers: vec![HeatLayerDelta { name: "nav".to_owned(), increments: vec![(CellId(1), 1)] }],
            }),
            ..Delta::default()
        };
        let ends = SLICE.saturating_add(SimDuration::ms(1));
        let error = state.apply(SLICE, ends, &delta).expect_err("no run counts a cell that often");
        assert!(matches!(error, Error::CounterOverflow { cell: 1, .. }), "got {error}");
        assert_eq!(state.heat.layers["nav"][&CellId(1)], u64::MAX, "and the counter is left as it was");
    }

    #[test]
    fn a_delta_that_moves_a_robot_no_keyframe_placed_is_an_error_and_not_a_guess() {
        let mut state = RunState::default();
        let error = state.apply(SimTime::ZERO, SLICE, &walking(6_000)).expect_err("nothing placed robot 1");
        assert!(matches!(error, Error::DeltaMovesUnplacedRobot { robot: 1, .. }), "got {error}");
        let error = state.at(SimTime::ZERO, SLICE, &walking(6_000)).expect_err("nor can it be drawn");
        assert!(matches!(error, Error::DeltaMovesUnplacedRobot { robot: 1, .. }), "got {error}");
    }

    #[test]
    fn a_delta_that_does_not_begin_where_the_state_stands_is_refused() {
        let mut state = one_robot_at(4);
        let error = state.apply(SimTime::ZERO, SLICE, &Delta::default()).expect_err("that is another slice");
        assert!(matches!(error, Error::DeltaNotAtState { begins_ms: 0, reached_ms: 10_000 }), "got {error}");
        let error = state.at(SimTime::ZERO, SLICE, &Delta::default()).expect_err("and cannot be read into");
        assert!(matches!(error, Error::DeltaNotAtState { begins_ms: 0, reached_ms: 10_000 }), "got {error}");
        let ends = SimTime::ms(1);
        let error = state.apply(SLICE, ends, &Delta::default()).expect_err("a run is read forward");
        assert!(matches!(error, Error::SliceEndsBeforeItBegins { ends_ms: 1, .. }), "got {error}");
        let error = state.at(SLICE, SimTime::ZERO, &Delta::default()).expect_err("and answered forward");
        assert!(matches!(error, Error::InstantOutsideSlice { t_ms: 0, slice_start_ms: 10_000 }), "got {error}");
    }

    #[test]
    fn a_tote_filled_and_then_extracted_inside_one_slice_leaves_the_run() {
        let mut state = one_tote_on_a_conveyor();
        let delta = Delta {
            playback: Some(PlaybackDelta {
                tote_contents: vec![ToteContents { offset: SimDuration::ms(10), tote: ToteId(1), units: 5 }],
                tote_transitions: vec![ToteTransition {
                    offset: SimDuration::ms(20),
                    tote: ToteId(1),
                    location: ToteLocation::Extracted,
                }],
                ..PlaybackDelta::default()
            }),
            ..Delta::default()
        };
        let inside = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(15)), &delta).expect("reads the slice");
        assert!(inside.totes.contains_key(&ToteId(1)), "the tote is still there before it is extracted");
        state.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &delta).expect("applies");
        assert!(!state.playback.totes.contains_key(&ToteId(1)), "an extracted tote is gone from the run");
    }

    /// One HTM placed at cell 4, standing still waiting for a task, as a keyframe of a recording that states the
    /// robot state channel says it.
    fn one_robot_waiting() -> Keyframe {
        let placed = one_robot_at(4);
        let playback = PlaybackKeyframe {
            robots: vec![RobotRecord {
                robot: RobotId(1),
                class: RobotClass::Htm,
                at: placed.playback.robots[&RobotId(1)].at,
                heading_mrad: Some(0),
                carrying: None,
                cell: CellId(4),
                toward: None,
            }],
            ..PlaybackKeyframe::default()
        };
        let waiting = RobotState::NoTask { cell: Some(CellRole::Charging) };
        Keyframe {
            playback: Some(playback),
            robot_state: Some(RobotStateKeyframe(vec![(RobotId(1), waiting)])),
            ..Keyframe::default()
        }
    }

    fn spans(continues: bool, spans: &[(u64, RobotState)]) -> Delta {
        let spans = spans.iter().map(|(dt_ms, state)| StateSpan { dt: SimDuration::ms(*dt_ms), state: *state });
        Delta {
            robot_state: Some(RobotStateDelta(vec![RobotSpans {
                robot: RobotId(1),
                continues,
                spans: spans.collect(),
            }])),
            ..Delta::default()
        }
    }

    /// A keyframe states every robot's state at its instant, so a reader beginning there knows what each robot it
    /// draws is doing: one that places a robot it gives no state, or gives a state to a robot it does not place, is
    /// damage.
    #[test]
    fn a_keyframe_whose_robot_states_and_places_name_different_robots_is_refused() {
        let mut keyframe = one_robot_waiting();
        RunState::from_keyframe(SLICE, &keyframe, &Descriptors::default()).expect("every placed robot stated");

        keyframe.robot_state = Some(RobotStateKeyframe::default());
        let error = RunState::from_keyframe(SLICE, &keyframe, &Descriptors::default()).expect_err("robot 1 unstated");
        assert!(matches!(error, Error::KeyframeRobotStatesDisagree { robot: 1, .. }), "got {error}");

        keyframe.robot_state =
            Some(RobotStateKeyframe(vec![(RobotId(1), RobotState::Drop), (RobotId(2), RobotState::Drop)]));
        let error = RunState::from_keyframe(SLICE, &keyframe, &Descriptors::default()).expect_err("robot 2 unplaced");
        assert!(matches!(error, Error::KeyframeRobotStatesDisagree { robot: 2, .. }), "got {error}");
    }

    /// A recording states robot state in every keyframe and delta or in none: one that stopped or started part way
    /// has no one reading. A delta that states nothing at all, which a reader passes at the state's own instant,
    /// is still read into, and answers what the state holds.
    #[test]
    fn a_delta_that_states_robot_state_where_the_keyframe_did_not_or_the_reverse_is_refused() {
        let ends = SLICE.saturating_add(SimDuration::ms(10_000));
        let mut stated = RunState::from_keyframe(SLICE, &one_robot_waiting(), &Descriptors::default()).expect("reads");
        let error = stated.apply(SLICE, ends, &Delta::default()).expect_err("the keyframe stated robot state");
        assert!(matches!(error, Error::RobotStateNotInEveryFrame { at_ms: 10_000, .. }), "got {error}");
        let error = stated.enter_slice(SLICE, &Delta::default()).expect_err("and a slice entered is held to it too");
        assert!(matches!(error, Error::RobotStateNotInEveryFrame { .. }), "got {error}");
        let held = stated.at(SLICE, SLICE, &Delta::default()).expect("a default delta says nothing").robot_states;
        assert_eq!(held, stated.robot_states, "and is answered with what the state holds");

        let mut unstated = one_robot_at(4);
        let delta = spans(true, &[(10_000, RobotState::Drop)]);
        let error = unstated.apply(SLICE, ends, &delta).expect_err("the keyframe stated none");
        assert!(matches!(error, Error::RobotStateNotInEveryFrame { .. }), "got {error}");
        let error = unstated.at(SLICE, SLICE, &delta).expect_err("nor can it be read into");
        assert!(matches!(error, Error::RobotStateNotInEveryFrame { .. }), "got {error}");
    }

    /// Spans that leave part of the slice without a state are refused like every other damage: before the slice's
    /// headings or anything else is taken in, so the state is left exactly as it was.
    #[test]
    fn a_slice_whose_spans_leave_part_of_it_stateless_is_refused_before_anything_is_taken_in() {
        let mut state = RunState::from_keyframe(SLICE, &one_robot_waiting(), &Descriptors::default()).expect("reads");
        let before = state.clone();
        let mut short = spans(false, &[(4_000, RobotState::EmptyTravel)]);
        short.playback = Some(PlaybackDelta { headings: vec![(RobotId(1), Some(1_571))], ..PlaybackDelta::default() });
        let error = state.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &short).expect_err("4 s of 10 s");
        assert!(matches!(error, Error::RobotSpansOffTheSlice { robot: 1, .. }), "got {error}");
        assert_eq!(state, before, "the heading the same slice states was not taken either");
    }

    /// The robot waits, then travels: a whole slice leaves it in the state its last span states, and a seek into the
    /// slice answers the span the instant falls in.
    #[test]
    fn a_slice_taken_in_leaves_a_robot_in_the_state_its_last_span_states() {
        let waiting = RobotState::NoTask { cell: Some(CellRole::Charging) };
        let mut state = RunState::from_keyframe(SLICE, &one_robot_waiting(), &Descriptors::default()).expect("reads");
        let delta = spans(true, &[(3_000, waiting), (7_000, RobotState::EmptyTravel)]);
        let inside = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(2_999)), &delta).expect("reads the slice");
        assert_eq!(inside.robot_states.expect("stated").robots[&RobotId(1)], waiting);
        let later = state.at(SLICE, SLICE.saturating_add(SimDuration::ms(3_000)), &delta).expect("reads the slice");
        assert_eq!(later.robot_states.expect("stated").robots[&RobotId(1)], RobotState::EmptyTravel);
        state.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &delta).expect("applies");
        assert_eq!(state.robot_states.expect("stated").robots[&RobotId(1)], RobotState::EmptyTravel);
    }

    /// A run that stops 4 s into a 10 s slice: the end block, stamped at the stop, is the frame after the last
    /// delta, so the last slice is 4 s wide and its spans end at the stop. A reader taking it up to the end block
    /// stands at the stop; one taking it a whole width on, past the run, is refused.
    #[test]
    fn the_runs_last_slice_ends_at_the_end_block_and_its_spans_end_with_it() {
        let stop = SLICE.saturating_add(SimDuration::ms(4_000));
        let last = spans(false, &[(1_000, RobotState::EmptyTravel), (3_000, RobotState::Pickup)]);
        let mut state = RunState::from_keyframe(SLICE, &one_robot_waiting(), &Descriptors::default()).expect("reads");
        state.apply(SLICE, stop, &last).expect("the spans lie across the slice up to the run's stop");
        assert_eq!(
            (state.reached, state.robot_states.expect("stated").robots[&RobotId(1)]),
            (stop, RobotState::Pickup)
        );

        let mut whole = RunState::from_keyframe(SLICE, &one_robot_waiting(), &Descriptors::default()).expect("reads");
        let error = whole.apply(SLICE, SLICE.saturating_add(SimDuration::ms(10_000)), &last).expect_err("past the run");
        assert!(error.to_string().contains("the spans last 4000 ms and the slice 10000 ms"), "got {error}");
    }
}
