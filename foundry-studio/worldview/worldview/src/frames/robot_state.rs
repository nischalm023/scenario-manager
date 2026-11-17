//! Within `frames`, the robot state channel: what each robot is doing, slice by slice, and the state a reader holds
//! from it.
//!
//! **What it is for.** A chart of how a fleet spends its time must be final with the slice it reads, and a viewer
//! that begins at any keyframe must know what every robot is doing there. So the channel states a robot's state the
//! way playback states its position: a keyframe states every robot's state at its instant, and a delta states, for
//! each robot that enters a span in its slice, its states as spans laid end to end from the slice's start to its end.
//! A robot the delta does not name stays all slice in the span it was in when the slice began.
//!
//! **A span lies inside its slice.** One that runs on past the slice's end is cut there, and the next slice to name
//! the robot marks its first span as carrying on that one. A slice ends where the next frame begins: the next slice,
//! or, for the run's last slice, the end block, stamped at the run's stop: the simtime its summary ends it at. So the
//! last slice reaches only as far as the run does, and a run that stops inside a slice cuts every robot's spans there.
//! A span states what was committed when the barrier passed the end of its slice, and nothing a later commit decides:
//! a robot whose next task is committed only later has no task until then. Nothing a slice states is ever taken back.
//!
//! **A span is one stretch in one state, entered once.** Its start is an entry, which a count of entries counts. So
//! two spans side by side may state one state, as two pickups back to back, and a first span not marked as carrying
//! on begins anew even in the state the robot was in. A producer never merges two spans into one, and never splits
//! one inside a slice: only a slice's edge cuts a span, and the mark says so.
//!
//! The states are the ones the fleet reports partition each robot's time into, and they speak the run's own robots
//! and quantities, never a simulator type.

use std::result::Result as StdResult;

use desunits::{RobotId, SimDuration, SimTime};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::container::error::{Error, Result};
use crate::container::payload::Payload;
use crate::frames::{byte_enum, in_id_order};

byte_enum! {
    /// What a waiting robot waits for: the committed fact that holds it where it stands.
    WaitCause as "wait cause" {
        /// Its pickup waits for its tote to reach the relay's diving board: it drove to the relay and arrived first.
        AwaitingDeposit = 1,
        /// It stands at the relay its tote goes to until the deposit reservation its task holds begins: the diving
        /// board free, and the rack slot behind it vacated.
        AwaitingSlot = 2,
        /// It has arrived at a station's presentation cell, and the operator has not begun on its tote.
        AwaitingStationService = 3,
        /// Its next move is committed and has not begun: from the simtime the barrier had reached when the move was
        /// committed to its departure, wherever it stands, as on a presentation cell the operator has finished with.
        AwaitingDeparture = 4,
        /// Its path holds it on a cell between two moves.
        RouteHold = 5,
    }
}

byte_enum! {
    /// What the cell a standing robot stands on is for.
    CellRole as "cell role" {
        /// A cell the map sets aside for charging. The simulator models no charging, so a robot stands here as on
        /// open floor.
        Charging = 1,
        /// A station's presentation cell, where the operator works on the tote a robot brings.
        Presentation = 2,
        /// A station's queue cell, on the way to its presentation cell.
        StationQueue = 3,
        /// A relay's access cell, where a tote passes between a robot and the diving board.
        Relay = 4,
        /// Any other cell: open floor, a highway, a junction, an aisle.
        Route = 5,
    }
}

/// What a robot is doing over a span.
///
/// Three positions on the wire, nested as one element wherever a record names a state: the kind byte, the cause and
/// the cell role. The cause belongs to a wait, the cell role to a wait and to no task, and each is nil for every
/// other kind; a record carrying one its kind has no use for is refused rather than re-encoded as nil.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RobotState {
    /// Taking a tote on with its fork.
    Pickup,
    /// Putting a tote down with its fork.
    Drop,
    /// Lining its fork up with a rack slot or a relay before a pickup or a drop.
    Alignment,
    EmptyTravel,
    LadenTravel,
    /// Travelling to a parking cell, whatever it carries.
    TravelToPark,
    /// Standing on a station's presentation cell while the operator works on its tote.
    StationProcessing,
    /// Standing still for `cause`, a fact its committed task names. `cell` is the role of the cell it stands on, nil
    /// for a robot that waits on no floor cell, as a lift waits on its rail, or on one the producer's map gives no
    /// role.
    Waiting {
        cause: WaitCause,
        cell: Option<CellRole>,
    },
    /// Standing with no committed task, on a cell of role `cell`, nil as for a wait. It does not say why: the robot
    /// may have no work it could do, or work it cannot be given yet, and the recording does not tell the two apart.
    NoTask {
        cell: Option<CellRole>,
    },
}

impl RobotState {
    /// The three scalars the wire carries: kind, cause, cell role.
    fn parts(self) -> (u8, Option<WaitCause>, Option<CellRole>) {
        match self {
            Self::Pickup => (1, None, None),
            Self::Drop => (2, None, None),
            Self::Alignment => (3, None, None),
            Self::EmptyTravel => (4, None, None),
            Self::LadenTravel => (5, None, None),
            Self::TravelToPark => (6, None, None),
            Self::StationProcessing => (7, None, None),
            Self::Waiting { cause, cell } => (8, Some(cause), cell),
            Self::NoTask { cell } => (9, None, cell),
        }
    }

    fn from_parts(kind: u8, cause: Option<WaitCause>, cell: Option<CellRole>) -> StdResult<Self, String> {
        let state = match kind {
            1 => Self::Pickup,
            2 => Self::Drop,
            3 => Self::Alignment,
            4 => Self::EmptyTravel,
            5 => Self::LadenTravel,
            6 => Self::TravelToPark,
            7 => Self::StationProcessing,
            8 => {
                let cause = cause.ok_or_else(|| "a waiting robot's state names what it waits for".to_owned())?;
                return Ok(Self::Waiting { cause, cell });
            }
            9 => match cause {
                None => return Ok(Self::NoTask { cell }),
                Some(_) => return Err("a robot with no task waits for nothing, and this state names a cause".into()),
            },
            _ => return Err(format!("robot state kind byte {kind} names no state")),
        };
        match (cause, cell) {
            (None, None) => Ok(state),
            _ => Err(format!(
                "a robot state of kind {kind} is no wait and has a task, and this one names a cause or a cell"
            )),
        }
    }
}

impl Serialize for RobotState {
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        self.parts().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RobotState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> StdResult<Self, D::Error> {
        let (kind, cause, cell) = <(u8, Option<WaitCause>, Option<CellRole>)>::deserialize(deserializer)?;
        Self::from_parts(kind, cause, cell).map_err(DeError::custom)
    }
}

/// One stretch of a slice a robot spends in one state: how long it is in that state inside this slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSpan {
    pub dt: SimDuration,
    pub state: RobotState,
}

/// One robot's states over a slice, as spans laid end to end from the slice's start to its end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotSpans {
    pub robot: RobotId,
    /// Whether the first span carries on the one the robot was in as the slice began, rather than beginning with
    /// the slice.
    pub continues: bool,
    pub spans: Vec<StateSpan>,
}

impl RobotSpans {
    /// Refuse spans that do not lie across the slice beginning at `begins` and `width` wide exactly: every instant of
    /// a robot's time has one state, and a slice states only its own. A reader knows the width once the next frame
    /// says where the slice ends.
    pub fn lie_across(&self, begins: SimTime, width: SimDuration) -> Result<()> {
        let off = |detail: String| Error::RobotSpansOffTheSlice { at_ms: begins.as_ms(), robot: self.robot.0, detail };
        let mut covered = SimDuration::ZERO;
        for span in &self.spans {
            covered = covered.checked_add(span.dt).ok_or_else(|| off("the spans add past the end of time".into()))?;
        }
        match covered == width {
            true => Ok(()),
            false => Err(off(format!("the spans last {} ms and the slice {} ms", covered.as_ms(), width.as_ms()))),
        }
    }
}

/// Every robot's state in force at a keyframe's instant, in ascending robot id. Where one of a robot's spans ends
/// at that very instant and the next begins, it states the one beginning, as entering the slice that begins there
/// does (`RunState::enter_slice`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct RobotStateKeyframe(pub Vec<(RobotId, RobotState)>);

/// The spans of every robot that enters a span in one slice, its very start included, in ascending robot id. Its
/// event arrays, the spans, keep their order in time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct RobotStateDelta(pub Vec<RobotSpans>);

impl Serialize for RobotStateKeyframe {
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        in_id_order(&self.0, |(robot, _)| *robot).serialize(serializer)
    }
}

impl Serialize for RobotStateDelta {
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        in_id_order(&self.0, |spans| spans.robot).serialize(serializer)
    }
}

impl Payload for RobotStateKeyframe {
    const NAME: &'static str = "robot state keyframe";
}

impl Payload for RobotStateDelta {
    const NAME: &'static str = "robot state delta";
}

/// Each robot's state at the instant a rebuilt state stands at: what a keyframe states, carried on by every slice
/// taken since. After a whole slice, a robot holds the state the slice ended it in. Entering the next slice then gives
/// each robot whose new span begins at that instant the state it begins in, and only the entered states are what a
/// keyframe written at that instant states.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RobotStates {
    pub robots: FxHashMap<RobotId, RobotState>,
}

impl RobotStates {
    /// The states a keyframe at `at` states outright. A robot it names twice is damage: two states at one instant
    /// leave a reader choosing between them.
    pub fn from_keyframe(at: SimTime, keyframe: &RobotStateKeyframe) -> Result<Self> {
        let mut robots = FxHashMap::with_capacity_and_hasher(keyframe.0.len(), Default::default());
        for (robot, state) in &keyframe.0 {
            if robots.insert(*robot, *state).is_some() {
                return Err(Error::RobotStatedTwice { at_ms: at.as_ms(), robot: robot.0 });
            }
        }
        Ok(Self { robots })
    }

    /// Refuse a slice beginning at `begins` this state cannot take, before anything is taken in. `width` is the
    /// slice's, when the caller knows where it ends, which is where the next frame begins: then each named robot's
    /// spans must lie across it exactly ([`RobotSpans::lie_across`]).
    pub fn check(&self, begins: SimTime, width: Option<SimDuration>, delta: &RobotStateDelta) -> Result<()> {
        let at_ms = begins.as_ms();
        let mut named: FxHashSet<RobotId> = FxHashSet::default();
        for entry in &delta.0 {
            let robot = entry.robot.0;
            if !named.insert(entry.robot) {
                return Err(Error::RobotStatedTwice { at_ms, robot });
            }
            let held = self.robots.get(&entry.robot).ok_or(Error::SpansOfUnstatedRobot { at_ms, robot })?;
            let off = |detail: String| Error::RobotSpansOffTheSlice { at_ms, robot, detail };
            let first = entry.spans.first().ok_or_else(|| off("it names the robot and states no span".to_owned()))?;
            if entry.continues && first.state != *held {
                return Err(Error::SpanDoesNotCarryOn { at_ms, robot });
            }
            if let Some(index) = entry.spans.iter().position(|span| span.dt == SimDuration::ZERO) {
                return Err(off(format!("span {index} lasts no time")));
            }
            if let Some(width) = width {
                entry.lie_across(begins, width)?;
            }
        }
        Ok(())
    }

    /// Take what a slice states about its own start: the state each named robot begins it in, where that state
    /// begins with the slice.
    pub(crate) fn enter(&mut self, delta: &RobotStateDelta) {
        for entry in delta.0.iter().filter(|entry| !entry.continues) {
            if let Some(first) = entry.spans.first() {
                self.robots.insert(entry.robot, first.state);
            }
        }
    }

    /// Take the state a whole slice ends each named robot in.
    pub fn leave(&mut self, delta: &RobotStateDelta) {
        for entry in &delta.0 {
            if let Some(last) = entry.spans.last() {
                self.robots.insert(entry.robot, last.state);
            }
        }
    }

    /// Each robot's state once `elapsed` of a slice has gone by: the span that instant falls in for a robot the
    /// slice names, its last for an instant past them all, and the state it holds for every other robot.
    pub(crate) fn at(&self, elapsed: SimDuration, delta: &RobotStateDelta) -> Self {
        let mut robots = self.robots.clone();
        for entry in &delta.0 {
            let mut reached = SimDuration::ZERO;
            let state = entry.spans.iter().find(|span| {
                reached = reached.saturating_add(span.dt);
                elapsed < reached
            });
            if let Some(span) = state.or(entry.spans.last()) {
                robots.insert(entry.robot, span.state);
            }
        }
        Self { robots }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_TASK_ON_A_CHARGER: RobotState = RobotState::NoTask { cell: Some(CellRole::Charging) };

    fn span(dt_ms: u64, state: RobotState) -> StateSpan {
        StateSpan { dt: SimDuration::ms(dt_ms), state }
    }

    fn held(states: &[(u32, RobotState)]) -> RobotStates {
        RobotStates { robots: states.iter().map(|(robot, state)| (RobotId(*robot), *state)).collect() }
    }

    #[test]
    fn a_state_is_three_positions_with_the_cause_and_the_cell_role_nil_where_its_kind_has_no_use_for_them() {
        let travel = rmp_serde::to_vec(&RobotState::LadenTravel).expect("encodes");
        assert_eq!(travel, vec![0x93, 5, 0xc0, 0xc0], "the kind byte, then two nils");
        let waiting = RobotState::Waiting { cause: WaitCause::RouteHold, cell: Some(CellRole::StationQueue) };
        let bytes = rmp_serde::to_vec(&waiting).expect("encodes");
        assert_eq!(bytes, vec![0x93, 8, 5, 3], "the wait's kind, its cause, the role of the cell it waits on");
        assert_eq!(rmp_serde::from_slice::<RobotState>(&bytes).expect("decodes"), waiting);

        let bytes = rmp_serde::to_vec(&NO_TASK_ON_A_CHARGER).expect("encodes");
        assert_eq!(bytes, vec![0x93, 9, 0xc0, 1], "no task: no cause, and the role of the cell it stands on");
        assert_eq!(rmp_serde::from_slice::<RobotState>(&bytes).expect("decodes"), NO_TASK_ON_A_CHARGER);
        let on_a_rail = RobotState::NoTask { cell: None };
        let bytes = rmp_serde::to_vec(&on_a_rail).expect("encodes");
        assert_eq!(bytes, vec![0x93, 9, 0xc0, 0xc0], "a lift stands on no cell with a role");
        assert_eq!(rmp_serde::from_slice::<RobotState>(&bytes).expect("decodes"), on_a_rail);
    }

    /// A component the kind has no use for would be re-encoded as nil by a reader that accepted it, so the
    /// decoder would rewrite the file it read; a wait with no cause says nothing about why the robot stands, and a
    /// robot with no task stands for no cause.
    #[test]
    fn a_state_naming_what_its_kind_has_no_use_for_or_a_wait_naming_no_cause_is_refused() {
        let refused =
            [(5u8, Some(1u8), None), (3, None, Some(2u8)), (8, None, Some(1)), (9, Some(1), Some(4)), (10, None, None)];
        for (kind, cause, cell) in refused {
            let bytes = rmp_serde::to_vec(&(kind, cause, cell)).expect("encodes");
            assert!(rmp_serde::from_slice::<RobotState>(&bytes).is_err(), "kind {kind} with {cause:?} and {cell:?}");
        }
        let unknown_cause = rmp_serde::to_vec(&(8u8, Some(6u8), None::<u8>)).expect("encodes");
        assert!(rmp_serde::from_slice::<RobotState>(&unknown_cause).is_err(), "cause byte 6 names no cause");
    }

    #[test]
    fn a_delta_names_robots_in_ascending_id_each_with_its_mark_and_its_spans_in_time_order() {
        let delta = RobotStateDelta(vec![
            RobotSpans { robot: RobotId(9), continues: false, spans: vec![span(10_000, RobotState::Alignment)] },
            RobotSpans {
                robot: RobotId(2),
                continues: true,
                spans: vec![span(4_000, RobotState::EmptyTravel), span(6_000, NO_TASK_ON_A_CHARGER)],
            },
        ]);
        let bytes = delta.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x92, "two entries, with no wrapper around them");
        assert_eq!(&bytes[1..4], &[0x93, 2, 0xc3], "the lower id first: three positions, the id, the mark");
        let read = RobotStateDelta::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read.0[0].robot, RobotId(2));
        assert_eq!(read.0[0].spans[0].state, RobotState::EmptyTravel, "a robot's spans keep their order in time");
        assert_eq!(read.0[1], delta.0[0]);

        let keyframe = RobotStateKeyframe(vec![(RobotId(5), RobotState::Drop), (RobotId(1), NO_TASK_ON_A_CHARGER)]);
        let read = RobotStateKeyframe::from_bytes(&keyframe.to_bytes().expect("encodes"), 0).expect("decodes");
        assert_eq!(read.0, vec![(RobotId(1), NO_TASK_ON_A_CHARGER), (RobotId(5), RobotState::Drop)], "state, by id");
    }

    #[test]
    fn a_keyframe_stating_one_robot_twice_is_refused() {
        let twice = RobotStateKeyframe(vec![(RobotId(3), RobotState::Drop), (RobotId(3), RobotState::Pickup)]);
        let error = RobotStates::from_keyframe(SimTime::ms(30_000), &twice).expect_err("two states at one instant");
        assert!(matches!(error, Error::RobotStatedTwice { at_ms: 30_000, robot: 3 }), "got {error}");
    }

    /// The four ways a slice's spans cannot describe a robot's slice, each refused before anything is taken in.
    #[test]
    fn spans_that_do_not_lay_end_to_end_across_their_slice_are_refused() {
        let states = held(&[(1, NO_TASK_ON_A_CHARGER)]);
        let width = Some(SimDuration::ms(10_000));
        let entry = |continues, spans: Vec<StateSpan>| {
            RobotStateDelta(vec![RobotSpans { robot: RobotId(1), continues, spans }])
        };
        let refused = |delta: &RobotStateDelta| states.check(SimTime::ms(20_000), width, delta).expect_err("refused");

        let short = refused(&entry(true, vec![span(4_000, NO_TASK_ON_A_CHARGER)]));
        assert!(matches!(short, Error::RobotSpansOffTheSlice { robot: 1, .. }), "a slice of 10 s, 4 s stated: {short}");
        let long = refused(&entry(false, vec![span(6_000, RobotState::Pickup), span(6_000, RobotState::Drop)]));
        assert!(long.to_string().contains("12000 ms"), "a span reaching past the slice's end: {long}");
        let instant = refused(&entry(false, vec![span(0, RobotState::Pickup), span(10_000, RobotState::Drop)]));
        assert!(instant.to_string().contains("no time"), "{instant}");
        let silent = refused(&entry(false, Vec::new()));
        assert!(silent.to_string().contains("no span"), "{silent}");

        let whole = entry(false, vec![span(3_000, RobotState::Pickup), span(7_000, RobotState::LadenTravel)]);
        states.check(SimTime::ms(20_000), width, &whole).expect("spans laid end to end across the slice");
    }

    #[test]
    fn a_span_marked_as_carrying_on_must_state_the_state_it_carries_on() {
        let states = held(&[(4, RobotState::EmptyTravel)]);
        let delta = |state| {
            RobotStateDelta(vec![RobotSpans { robot: RobotId(4), continues: true, spans: vec![span(10_000, state)] }])
        };
        let error = states.check(SimTime::ZERO, None, &delta(RobotState::LadenTravel)).expect_err("another state");
        assert!(matches!(error, Error::SpanDoesNotCarryOn { at_ms: 0, robot: 4 }), "got {error}");
        states.check(SimTime::ZERO, None, &delta(RobotState::EmptyTravel)).expect("the same travel, carried on");
    }

    #[test]
    fn a_slice_naming_a_robot_twice_or_one_no_keyframe_stated_is_refused() {
        let states = held(&[(1, RobotState::Pickup)]);
        let spans =
            |robot| RobotSpans { robot: RobotId(robot), continues: true, spans: vec![span(5, RobotState::Pickup)] };
        let error = states.check(SimTime::ZERO, None, &RobotStateDelta(vec![spans(1), spans(1)])).expect_err("twice");
        assert!(matches!(error, Error::RobotStatedTwice { robot: 1, .. }), "got {error}");
        let error = states.check(SimTime::ZERO, None, &RobotStateDelta(vec![spans(2)])).expect_err("never stated");
        assert!(matches!(error, Error::SpansOfUnstatedRobot { robot: 2, .. }), "got {error}");
    }

    /// Robot 1 waits all slice and is not named; robot 2 finishes a pickup and travels laden; robot 3's travel ends
    /// exactly as the slice begins, so its new state begins with the slice and entering the slice takes it in.
    #[test]
    fn a_slice_entered_left_and_read_inside_gives_each_robot_the_state_its_spans_say() {
        let mut states = held(&[(1, NO_TASK_ON_A_CHARGER), (2, RobotState::Pickup), (3, RobotState::LadenTravel)]);
        let delta = RobotStateDelta(vec![
            RobotSpans {
                robot: RobotId(2),
                continues: true,
                spans: vec![span(2_000, RobotState::Pickup), span(8_000, RobotState::LadenTravel)],
            },
            RobotSpans { robot: RobotId(3), continues: false, spans: vec![span(10_000, RobotState::Drop)] },
        ]);
        states.check(SimTime::ZERO, Some(SimDuration::ms(10_000)), &delta).expect("a slice this state can take");

        let early = states.at(SimDuration::ms(1_999), &delta);
        assert_eq!(early.robots[&RobotId(2)], RobotState::Pickup, "still in the span carried on");
        let later = states.at(SimDuration::ms(2_000), &delta);
        assert_eq!(later.robots[&RobotId(2)], RobotState::LadenTravel, "a span begins where the one before ends");
        assert_eq!(later.robots[&RobotId(1)], NO_TASK_ON_A_CHARGER, "a robot the slice does not name holds its state");
        assert_eq!(states.at(SimDuration::ms(10_000), &delta).robots[&RobotId(2)], RobotState::LadenTravel);

        states.enter(&delta);
        assert_eq!(states.robots[&RobotId(3)], RobotState::Drop, "the state that begins with the slice");
        assert_eq!(states.robots[&RobotId(2)], RobotState::Pickup, "a span carried on changes nothing at the start");
        states.leave(&delta);
        assert_eq!(states.robots[&RobotId(2)], RobotState::LadenTravel, "the state the slice ends it in");
        assert_eq!(states.robots[&RobotId(1)], NO_TASK_ON_A_CHARGER);
    }
}
