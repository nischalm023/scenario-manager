//! What the viewer draws: where every robot and tote is, which relay cells
//! are reserved, and how much work is queued at each station.
//!
//! A keyframe states all of it for one instant with no earlier frame needed.
//! A delta states one closed slice's changes and nothing else — a robot that
//! did not move has no entry, which is where most of a recording's bytes
//! would otherwise go.
//!
//! A robot's place is its coordinates: whole millimetres in the world
//! document's root 3-D frame, which the scenario block names once. A cell
//! rides beside them and positions nothing — it says which cell a consumer
//! reporting in cells should count the robot against, and a viewer that asks
//! the floor where the robot is has read the record wrong.
//!
//! A mover's slice is a list of steps, each stating where the step left the
//! mover and how long it took; the step's source is wherever the mover was
//! before it, and a slice that cuts a leg in half states the half-way point
//! like any other.

use std::fmt;

use desgraph::types::{CellId, RobotType, StationId};
use desunits::{RobotId, SimDuration, SimTime, ToteId};
use serde::de::{Error as DeError, IgnoredAny, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::container::payload::{Opaque, Payload};
use crate::frames::{byte_enum, in_id_order};

byte_enum! {
    /// Which class of machine a robot is. The simulator's `RobotType` converts
    /// to this and back, and this is the only place that conversion is
    /// written.
    RobotClass as "robot class" {
        /// Horizontal transfer module.
        Htm = 1,
        /// Vertical transfer module.
        Vtm = 2,
        /// Racks-to-picker.
        Rtp = 3,
    }
}

impl From<RobotType> for RobotClass {
    fn from(robot_type: RobotType) -> Self {
        match robot_type {
            RobotType::Htm => Self::Htm,
            RobotType::Vtm => Self::Vtm,
            RobotType::Rtp => Self::Rtp,
        }
    }
}

impl From<RobotClass> for RobotType {
    fn from(class: RobotClass) -> Self {
        match class {
            RobotClass::Htm => Self::Htm,
            RobotClass::Vtm => Self::Vtm,
            RobotClass::Rtp => Self::Rtp,
        }
    }
}

byte_enum! {
    /// What a mover was doing over one step of a slice.
    StepKind as "step kind" {
        Move = 1,
        Reverse = 2,
        Wait = 3,
        Rotate = 4,
    }
}

impl StepKind {
    /// Whether the step takes the mover along its leg. A wait or a rotate
    /// spends time where the mover already stands.
    pub fn travels(self) -> bool {
        matches!(self, Self::Move | Self::Reverse)
    }
}

/// Where a robot is: whole millimetres along the three axes of the world
/// document's root 3-D frame, which the scenario block names once.
///
/// Three positions on the wire, as its own array, and always all three. The
/// vertical one is what lets a recording say where a ladder VTM is without a
/// second vocabulary for height; a machine that never leaves its standing
/// surface simply states that surface's height every time.
///
/// Millimetres are the format's whole precision. A producer holding finer
/// numbers rounds them on the way in, and a consumer interpolating between two
/// of these rounds the result the same way, so two viewers reading one file
/// put a robot in one place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Position {
    pub x_mm: i32,
    pub y_mm: i32,
    pub z_mm: i32,
}

impl Position {
    pub const fn new(x_mm: i32, y_mm: i32, z_mm: i32) -> Self {
        Self { x_mm, y_mm, z_mm }
    }

    /// The point `along` of the way from here to `to`, in whole millimetres —
    /// the one arithmetic every consumer draws a part-finished step with.
    pub fn lerp(self, to: Self, along: f64) -> Self {
        let axis = |from: i32, to: i32| (f64::from(from) + (f64::from(to) - f64::from(from)) * along).round() as i32;
        Self { x_mm: axis(self.x_mm, to.x_mm), y_mm: axis(self.y_mm, to.y_mm), z_mm: axis(self.z_mm, to.z_mm) }
    }
}

/// Where a tote is, in the terms the viewer draws it in.
///
/// Three positions on the wire — a kind byte, the cell or robot the kind
/// names, and the rack level the kind needs — nested as one element wherever
/// a record names a location, so a keyframe and a delta say it the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToteLocation {
    /// Standing at an induction or presentation cell.
    AtCell(CellId),
    /// Gone from the simulated system and no longer drawn.
    Extracted,
    /// In a rack, at a cell and a shelf level.
    InRack { cell: CellId, level: u16 },
    /// On a relay's diving board.
    AtRelay(CellId),
    /// Carried by a robot; its position follows that robot's.
    OnBot(RobotId),
    /// At its last conveyor landmark, between transits.
    OnConveyor(CellId),
}

impl ToteLocation {
    /// The three scalars the wire carries: kind, the id it names, the level.
    fn parts(self) -> (u8, u32, u16) {
        match self {
            Self::AtCell(cell) => (0, cell.0, 0),
            Self::Extracted => (1, 0, 0),
            Self::InRack { cell, level } => (2, cell.0, level),
            Self::AtRelay(cell) => (3, cell.0, 0),
            Self::OnBot(robot) => (4, robot.0, 0),
            Self::OnConveyor(cell) => (5, cell.0, 0),
        }
    }

    /// Read the three scalars back. A kind reads only the components it
    /// names, so a record carrying a component its kind has no use for —
    /// `Extracted` with a cell id, anything but a rack with a level — is
    /// refused. Accepting it and re-encoding it as a zero would make the
    /// decoder rewrite the file it read.
    fn from_parts(kind: u8, id: u32, level: u16) -> Result<Self, String> {
        let spare =
            |what: &str, value: u64| format!("a tote location of kind {kind} has no {what}, and this one says {value}");
        let location = match kind {
            0 => Self::AtCell(CellId(id)),
            1 if id != 0 => return Err(spare("cell or robot", u64::from(id))),
            1 => Self::Extracted,
            2 => return Ok(Self::InRack { cell: CellId(id), level }),
            3 => Self::AtRelay(CellId(id)),
            4 => Self::OnBot(RobotId(id)),
            5 => Self::OnConveyor(CellId(id)),
            _ => return Err(format!("tote location kind byte {kind} names no location")),
        };
        match level {
            0 => Ok(location),
            level => Err(spare("rack level", u64::from(level))),
        }
    }
}

impl Serialize for ToteLocation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.parts().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToteLocation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (kind, id, level) = <(u8, u32, u16)>::deserialize(deserializer)?;
        Self::from_parts(kind, id, level).map_err(DeError::custom)
    }
}

/// One step of a mover's slice: where the step left the mover, how long the
/// mover spent on it inside this slice, what kind of motion it was, and the
/// cell it arrived in.
///
/// The cell is advisory, like a keyframe robot's. A leg the planner curved
/// arrives at one cell along an arc and is recorded as several steps naming
/// that cell; a slice boundary that cuts a leg states the point it cut at.
/// Both are ordinary steps here, because the coordinates say where the mover
/// is and the cell never has to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub at: Position,
    pub dt: SimDuration,
    pub kind: StepKind,
    pub cell: CellId,
}

/// One robot that moved during a slice, and its steps in order. A robot that
/// stayed put has no entry at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoverSteps {
    pub robot: RobotId,
    /// The chassis heading in milliradians at the instant the slice began;
    /// the steps turn the robot from there. Nil says the producer tracks no
    /// heading for this robot, so a reader keeps the last one it was given.
    pub start_heading_mrad: Option<i32>,
    pub steps: Vec<Step>,
    /// The cell the slice leaves the mover in — advisory, like every cell
    /// here. It is stated rather than taken from the last step because a
    /// slice's end cuts a leg part way: the step names the cell the mover was
    /// travelling to and this names the one it is still in, so a reader that
    /// replayed every delta counts the mover against the same cell as one
    /// that sought to the next keyframe.
    pub cell: CellId,
}

impl MoverSteps {
    /// Where the slice leaves the mover, given where it found it: its last
    /// step's coordinates, or where it began when it took no step.
    pub fn ends_at(&self, began: Position) -> Position {
        self.steps.last().map_or(began, |step| step.at)
    }

    /// The cell the slice leaves the mover heading for, or nil where it
    /// arrived: a last step naming a cell the mover is not in is a leg the
    /// slice's end cut part way.
    pub fn heading_for(&self) -> Option<CellId> {
        self.steps.last().map(|step| step.cell).filter(|cell| *cell != self.cell)
    }
}

/// Where one robot is at a keyframe's instant, what it is, which way it
/// faces, and what it carries. The heading is the robot's at that instant,
/// which is the one the slice beginning there starts with.
///
/// `at` is the whole answer to where the robot is. The two cells beside it are
/// advisory and place nothing: `cell` is the cell the robot is in, which is
/// what a consumer counting robots per cell counts it against, and `toward`
/// the cell it is heading for when the keyframe catches it between two.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RobotRecord {
    pub robot: RobotId,
    pub class: RobotClass,
    pub at: Position,
    pub heading_mrad: Option<i32>,
    pub carrying: Option<ToteId>,
    pub cell: CellId,
    pub toward: Option<CellId>,
}

/// Where one tote is, what it is, and how full it is, at a keyframe's
/// instant.
///
/// The container type and the pool are here as well as on the tote's creation
/// record, so a viewer that seeks into the middle of a run draws an order
/// tote differently from an inventory tote without having read the creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ToteRecord {
    pub tote: ToteId,
    /// 0 tray, 1 tote, 2 carton — the physical container, the same byte
    /// [`ToteCreation`] states.
    pub tote_type: u8,
    /// 0 inventory, 1 order — the pool the tote belongs to for its whole
    /// life, the same byte [`ToteCreation`] states.
    pub designation: u8,
    pub location: ToteLocation,
    pub contents_units: u64,
}

struct ToteRecordForm;

impl<'de> Visitor<'de> for ToteRecordForm {
    type Value = ToteRecord;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a tote, its container type, its pool, where it is, and how much it holds")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut row: A) -> Result<ToteRecord, A::Error> {
        let tote: ToteId = row.next_element()?.ok_or_else(|| DeError::invalid_length(0, &self))?;
        let tote_type: u8 = row.next_element()?.ok_or_else(|| DeError::invalid_length(1, &self))?;
        let designation: u8 = row.next_element()?.ok_or_else(|| DeError::invalid_length(2, &self))?;
        let location: ToteLocation = row.next_element()?.ok_or_else(|| DeError::invalid_length(3, &self))?;
        let contents_units: u64 = row.next_element()?.ok_or_else(|| DeError::invalid_length(4, &self))?;
        if row.next_element::<IgnoredAny>()?.is_some() {
            return Err(DeError::invalid_length(6, &self));
        }
        if location == ToteLocation::Extracted {
            // A keyframe states what is in the run. A tote that has left it is
            // in no keyframe, so one that says otherwise is damage rather than
            // a tote to draw.
            return Err(DeError::custom(format!("tote {tote} has been extracted, and a keyframe states no such tote")));
        }
        Ok(ToteRecord { tote, tote_type, designation, location, contents_units })
    }
}

impl<'de> Deserialize<'de> for ToteRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_tuple(5, ToteRecordForm)
    }
}

/// A tote's birth: the two facts about it that never change, and where it
/// first stands. The location is here so a tote born mid-slice is drawn from
/// this record alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToteCreation {
    pub offset: SimDuration,
    pub tote: ToteId,
    pub tote_type: u8,
    pub designation: u8,
    pub location: ToteLocation,
}

/// A tote's move to a new location, at an offset into the slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToteTransition {
    pub offset: SimDuration,
    pub tote: ToteId,
    pub location: ToteLocation,
}

/// A tote's timed travel between conveyance cells. Absolute instants, not
/// offsets, so every slice the travel overlaps can place the tote on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToteTransit {
    pub tote: ToteId,
    pub from_cell: CellId,
    pub to_cell: CellId,
    pub start: SimTime,
    pub end: SimTime,
}

/// How many units a tote holds, from an offset into the slice onward.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToteContents {
    pub offset: SimDuration,
    pub tote: ToteId,
    pub units: u64,
}

/// A relay cell becoming reserved, or stopping being reserved, at an offset
/// into the slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayTransition {
    pub cell: CellId,
    pub offset: SimDuration,
    pub reserved: bool,
}

/// How much work is queued at one station, at a keyframe's instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationState {
    pub station: StationId,
    /// Robots dispatched to the station and not yet served.
    pub pipeline_depth: u32,
}

/// A station's queue depth changing, at an offset into the slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationChange {
    pub offset: SimDuration,
    pub station: StationId,
    pub pipeline_depth: u32,
}

/// Everything needed to draw one instant, with no earlier frame.
///
/// Its arrays are state, so the encoder writes them in ascending id and two
/// producers that gathered the same instant in different orders write the
/// same bytes. A tote that has been extracted is dropped on the way out and
/// refused on the way in: it has left the run, and nothing draws it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct PlaybackKeyframe {
    pub robots: Vec<RobotRecord>,
    pub totes: Vec<ToteRecord>,
    pub reserved_relay_cells: Vec<CellId>,
    pub stations: Vec<StationState>,
    /// The conveyor journeys under way at this instant: every transit whose
    /// end is still ahead of it. A tote part-way along a conveyor stands
    /// nowhere, so without these a viewer attaching here draws it parked at
    /// its destination until the next journey moves it.
    pub transits_in_flight: Vec<ToteTransit>,
}

impl Serialize for PlaybackKeyframe {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut row = serializer.serialize_tuple(5)?;
        row.serialize_element(&in_id_order(&self.robots, |robot| robot.robot))?;
        let in_the_run: Vec<&ToteRecord> =
            self.totes.iter().filter(|tote| tote.location != ToteLocation::Extracted).collect();
        row.serialize_element(&in_id_order(&in_the_run, |tote| tote.tote))?;
        row.serialize_element(&in_id_order(&self.reserved_relay_cells, |cell| *cell))?;
        row.serialize_element(&in_id_order(&self.stations, |station| station.station))?;
        row.serialize_element(&in_id_order(&self.transits_in_flight, |transit| {
            (transit.tote, transit.start.as_ms(), transit.end.as_ms())
        }))?;
        row.end()
    }
}

/// One closed slice's changes.
///
/// Its event arrays keep the producer's order — by offset, then by the order
/// the run emitted them — because that is the order they happened in. Its one
/// state array, the headings, is written in ascending id like a keyframe's.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct PlaybackDelta {
    pub movers: Vec<MoverSteps>,
    /// Every robot whose heading at this slice's start differs from the one
    /// the reader was holding — a robot that turned without moving as much as
    /// one that walked. A nil says the producer no longer tracks a heading
    /// for that robot, so the reader forgets the one it held; a mover's own
    /// nil heading says nothing at all and leaves it standing.
    pub headings: Vec<(RobotId, Option<i32>)>,
    pub tote_creations: Vec<ToteCreation>,
    pub tote_transitions: Vec<ToteTransition>,
    pub tote_transits: Vec<ToteTransit>,
    pub tote_contents: Vec<ToteContents>,
    pub relay_transitions: Vec<RelayTransition>,
    pub station_changes: Vec<StationChange>,
    /// The producer's own station panel records for the slice, carried
    /// through for the workbook. Nothing here reads them and playback does
    /// not need them.
    pub station_display: Opaque,
}

impl Serialize for PlaybackDelta {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut row = serializer.serialize_tuple(9)?;
        row.serialize_element(&self.movers)?;
        row.serialize_element(&in_id_order(&self.headings, |(robot, _)| *robot))?;
        row.serialize_element(&self.tote_creations)?;
        row.serialize_element(&self.tote_transitions)?;
        row.serialize_element(&self.tote_transits)?;
        row.serialize_element(&self.tote_contents)?;
        row.serialize_element(&self.relay_transitions)?;
        row.serialize_element(&self.station_changes)?;
        row.serialize_element(&self.station_display)?;
        row.end()
    }
}

impl Payload for PlaybackKeyframe {
    const NAME: &'static str = "playback keyframe";
}

impl Payload for PlaybackDelta {
    const NAME: &'static str = "playback delta";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_robot_record_is_seven_positions_with_the_coordinates_nested_and_the_cells_beside_them() {
        let record = RobotRecord {
            robot: RobotId(7),
            class: RobotClass::Vtm,
            at: Position::new(1_200, -450, 9_000),
            heading_mrad: Some(1_571),
            carrying: Some(ToteId(3)),
            cell: CellId(9),
            toward: None,
        };
        let bytes = rmp_serde::to_vec(&record).expect("encodes");
        assert_eq!(bytes[0], 0x97, "a seven-element array");
        assert_eq!((bytes[1], bytes[2], bytes[3]), (7, 2, 0x93), "robot, class, then three coordinates nested");
        assert_eq!(rmp_serde::from_slice::<RobotRecord>(&bytes).expect("decodes"), record);

        let heading_for = RobotRecord { toward: Some(CellId(10)), ..record };
        let bytes = rmp_serde::to_vec(&heading_for).expect("encodes");
        assert_eq!(rmp_serde::from_slice::<RobotRecord>(&bytes).expect("decodes"), heading_for);
    }

    #[test]
    fn a_position_is_three_whole_millimetres_and_a_negative_one_survives_the_wire() {
        let place = Position::new(-1, 2, 3);
        let bytes = rmp_serde::to_vec(&place).expect("encodes");
        assert_eq!(bytes[0], 0x93, "three elements");
        assert_eq!(rmp_serde::from_slice::<Position>(&bytes).expect("decodes"), place);
    }

    /// Half a millimetre goes away from zero on both axes, and the vertical
    /// one interpolates like the other two: a ladder VTM part-way up is drawn
    /// part-way up.
    #[test]
    fn interpolating_two_positions_lands_on_whole_millimetres() {
        let from = Position::new(0, 0, 0);
        assert_eq!(from.lerp(Position::new(3, 30, 300), 0.5), Position::new(2, 15, 150));
        assert_eq!(Position::new(0, 0, 0).lerp(Position::new(-3, 0, 0), 0.5), Position::new(-2, 0, 0));
        assert_eq!(from.lerp(Position::new(100, 0, 0), 0.0), from);
        assert_eq!(from.lerp(Position::new(100, 0, 0), 1.0), Position::new(100, 0, 0));
    }

    #[test]
    fn a_mover_ends_its_slice_where_its_last_step_left_it() {
        let step = |x: i32, cell: u32, kind| Step {
            at: Position::new(x, 0, 0),
            dt: SimDuration::ms(800),
            kind,
            cell: CellId(cell),
        };
        let mover = MoverSteps {
            robot: RobotId(1),
            start_heading_mrad: None,
            steps: vec![step(400, 4, StepKind::Move), step(900, 5, StepKind::Move)],
            cell: CellId(5),
        };
        let began = Position::new(0, 0, 0);
        assert_eq!(mover.ends_at(began), Position::new(900, 0, 0));
        assert_eq!(mover.heading_for(), None, "the last step arrived in the cell the mover is left in");
        let cut = MoverSteps { cell: CellId(4), ..mover.clone() };
        assert_eq!(cut.heading_for(), Some(CellId(5)), "a leg the slice's end cut leaves the mover heading on");
        let still = MoverSteps { steps: Vec::new(), ..mover };
        assert_eq!(still.ends_at(began), began);
    }

    #[test]
    fn a_tote_record_nests_the_one_location_encoding_beside_the_type_and_the_pool() {
        let record = ToteRecord {
            tote: ToteId(4),
            tote_type: 1,
            designation: 1,
            location: ToteLocation::InRack { cell: CellId(11), level: 2 },
            contents_units: 6,
        };
        let bytes = rmp_serde::to_vec(&record).expect("encodes");
        assert_eq!(bytes, vec![0x95, 4, 1, 1, 0x93, 2, 11, 2, 6], "five positions, the location nested in the fourth");
        assert_eq!(rmp_serde::from_slice::<ToteRecord>(&bytes).expect("decodes"), record);
    }

    #[test]
    fn every_tote_location_kind_survives_the_wire_and_a_spare_component_is_refused() {
        for location in [
            ToteLocation::AtCell(CellId(1)),
            ToteLocation::Extracted,
            ToteLocation::InRack { cell: CellId(2), level: 5 },
            ToteLocation::AtRelay(CellId(3)),
            ToteLocation::OnBot(RobotId(4)),
            ToteLocation::OnConveyor(CellId(5)),
        ] {
            let bytes = rmp_serde::to_vec(&location).expect("encodes");
            assert_eq!(rmp_serde::from_slice::<ToteLocation>(&bytes).expect("decodes"), location);
        }
        for (kind, id, level) in [(1u8, 99u32, 0u16), (1, 0, 7), (0, 4, 7), (3, 4, 7), (4, 4, 7), (5, 4, 7), (9, 0, 0)]
        {
            let bytes = rmp_serde::to_vec(&(kind, id, level)).expect("encodes");
            let read = rmp_serde::from_slice::<ToteLocation>(&bytes);
            assert!(read.is_err(), "kind {kind} accepted id {id} and level {level} and would re-encode them as zeros");
        }
    }

    #[test]
    fn a_step_kind_byte_no_variant_answers_to_is_an_error_not_a_panic() {
        let bytes = rmp_serde::to_vec(&(Position::default(), SimDuration::ms(1), 9u8, CellId(1))).expect("encodes");
        assert!(rmp_serde::from_slice::<Step>(&bytes).is_err());
        assert_eq!(StepKind::from_byte(1), Some(StepKind::Move));
        assert_eq!(RobotClass::from_byte(0), None);
    }

    #[test]
    fn the_encoder_puts_a_keyframes_state_in_ascending_id_and_leaves_a_deltas_events_alone() {
        let robot = |id: u32| RobotRecord {
            robot: RobotId(id),
            class: RobotClass::Htm,
            at: Position::new(id as i32 * 100, 0, 0),
            heading_mrad: None,
            carrying: None,
            cell: CellId(id),
            toward: None,
        };
        let keyframe = PlaybackKeyframe {
            robots: vec![robot(9), robot(2)],
            reserved_relay_cells: vec![CellId(8), CellId(1)],
            ..PlaybackKeyframe::default()
        };
        let read = PlaybackKeyframe::from_bytes(&keyframe.to_bytes().expect("encodes"), 0).expect("decodes");
        assert_eq!(read.robots, vec![robot(2), robot(9)], "the robots come back in ascending id");
        assert_eq!(read.reserved_relay_cells, vec![CellId(1), CellId(8)]);

        let event = |offset: u64| ToteTransition {
            offset: SimDuration::ms(offset),
            tote: ToteId(1),
            location: ToteLocation::AtCell(CellId(1)),
        };
        let delta = PlaybackDelta {
            tote_transitions: vec![event(900), event(100)],
            headings: vec![(RobotId(6), Some(10)), (RobotId(2), None)],
            ..PlaybackDelta::default()
        };
        let read = PlaybackDelta::from_bytes(&delta.to_bytes().expect("encodes"), 0).expect("decodes");
        assert_eq!(read.tote_transitions, vec![event(900), event(100)], "an event array keeps the producer's order");
        assert_eq!(read.headings, vec![(RobotId(2), None), (RobotId(6), Some(10))], "the headings are state");
    }
}
