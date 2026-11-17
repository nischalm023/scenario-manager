//! The scenario block: everything true of the run before any instant of it.
//!
//! It is the first frame of a file and the first thing on a wire, and it
//! carries what a reader needs to make sense of every frame after it — the
//! run's identity, the slice width and keyframe cadence the stamps follow, the
//! document frame every position is measured in, which section id means what,
//! the documents the run was built from, the id tables that give a cell, a
//! station or a robot a name, and the descriptors that fix the positions in a
//! scalars array and the id of every table.
//!
//! Two of its fields are only ever filled in on a wire: the compiled floor and
//! the entity census, which a server works out from the world document for a
//! browser that cannot compile one. A file states nothing for either.
//!
//! Nothing here is read by this crate beyond the numbers a reader seeks by:
//! the world and the scenario documents travel as the bytes they were read
//! from disk as, and the producer's own state travels as MessagePack it
//! writes and reads back itself.

use std::io::{Read, Seek};
use std::result::Result as StdResult;

use desgraph::types::{CellId, StationId};
use desunits::{RobotId, SimDuration};
use serde::ser::{Error as SerError, SerializeTuple};
use serde::{Deserialize, Serialize, Serializer};

use crate::container::error::{Error, Result};
use crate::container::frame::{Frame, Tag, VERSION};
use crate::container::payload::{Opaque, Payload};
use crate::container::reader::Reader;
use crate::frames::playback::RobotClass;
use crate::frames::scalars::MetricDescriptor;
use crate::frames::tables::TableDescriptor;
use crate::frames::{in_id_order, section_declarations};

/// A document as it was read from disk — the world, the scenario that names
/// it, or a data file the scenario references.
///
/// Its bytes are not MessagePack and this crate never parses them; it carries
/// them so a consumer resolves every reference against the recording and the
/// filesystem never.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl Document {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One robot the run holds, in the id table: which class of machine it is,
/// what the run calls it, and the world document's entity for its class when
/// the producer knows one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotEntry {
    pub robot: RobotId,
    pub name: String,
    pub class: RobotClass,
    pub class_entity: Option<String>,
}

/// The primary frame a recording that carries no world document names, and
/// which the stand-in document such a recording does carry declares.
///
/// A recording states the frame its positions are measured in, and this crate
/// refuses one that names none. A test fixture is a recording like any other,
/// so it names this rather than being excused the rule it is exercising; a
/// recording a simulator wrote names its own document's primary frame.
pub const HAND_BUILT_FRAME: &str = "hand-built-frame";

/// What was run, on what, and how to read the frames after it.
///
/// The id tables are state, so the encoder writes them in ascending id; the
/// descriptor lists are not, because their order is what fixes a metric's
/// slot and is the only place that order is written down.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Scenario {
    /// The container version, restated in-band. The magic ahead of this
    /// block is the authority; this is for provenance and for tools that
    /// print it.
    pub container_version: u8,
    pub run: String,
    pub attempt: u32,
    pub run_name: String,
    pub commit: String,
    pub dirty: bool,
    /// When the run started, in the producer's own words.
    pub timestamp: String,
    pub hostname: String,
    pub slice_width: SimDuration,
    /// Slices between keyframes. A reader takes the cadence from here, never
    /// from configuration.
    pub keyframe_cadence_slices: u32,
    /// The world document's root 3-D frame, by its entity id — the frame every
    /// robot position in this recording is stated in. It is named once, here,
    /// so no record carries it and no consumer has to work out which of the
    /// document's frames a millimetre belongs to. Never empty: a block that
    /// names no frame is refused on the way in.
    pub frame: String,
    /// Which section id carries what, so a consumer skips what it does not
    /// know by id rather than by guessing. In a file these are the sections
    /// every keyframe and every delta carries, each where its kind has it: a
    /// recording that does not state the robot state channel leaves it out.
    pub sections: Vec<(u8, String)>,
    pub world: Document,
    /// The scenario document — the world spec — as read from disk.
    pub scenario: Document,
    /// The files the scenario document references, keyed by the reference
    /// string exactly as it is written there. `None` on a wire, where a
    /// browser has no use for a run's workload and would pay tens of megabytes
    /// for it; a file always states them.
    pub data: Option<Vec<(String, Document)>>,
    /// The simulator's configuration, as MessagePack this crate carries and
    /// does not read.
    pub sim_config: Opaque,
    /// Whatever else the producer's run model needs to rebuild itself
    /// losslessly, in its own MessagePack. `None` on a wire, for the same
    /// reason and because a browser has no run model to rebuild into.
    pub producer: Option<Opaque>,
    pub cells: Vec<(CellId, String)>,
    pub stations: Vec<(StationId, String)>,
    pub robots: Vec<RobotEntry>,
    /// The floor a viewer draws, compiled from [`Self::world`] by whoever has
    /// the simulator's map compiler. A file the simulator writes states `None`
    /// here: the document beside it says the same thing, and a recording that
    /// carried both could have them disagree. A server fills it in for a
    /// browser, which has no map compiler of its own.
    pub floor: Option<Opaque>,
    /// Which cells each entity of the world document turned into, as the same
    /// compile worked out. `None` in a file, for the same reason.
    pub census: Option<Opaque>,
    /// The metrics known at the start, in the order that fixes each value's
    /// position in every scalars array. A declare frame appends to this list
    /// and never renumbers it.
    pub metrics: Vec<MetricDescriptor>,
    pub tables: Vec<TableDescriptor>,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            container_version: VERSION,
            run: String::new(),
            attempt: 0,
            run_name: String::new(),
            commit: String::new(),
            dirty: false,
            timestamp: String::new(),
            hostname: String::new(),
            slice_width: SimDuration::ZERO,
            keyframe_cadence_slices: 0,
            frame: String::new(),
            sections: section_declarations(false),
            world: Document::default(),
            scenario: Document::default(),
            data: None,
            sim_config: Opaque::default(),
            producer: None,
            cells: Vec::new(),
            stations: Vec::new(),
            robots: Vec::new(),
            floor: None,
            census: None,
            metrics: Vec::new(),
            tables: Vec::new(),
        }
    }
}

impl Serialize for Scenario {
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        if self.container_version != VERSION {
            // The magic ahead of this block says which version the file is.
            // A block saying another would leave a reader with two answers
            // and no way to tell which the producer meant.
            return Err(SerError::custom(format!(
                "this crate writes container version {VERSION} and this block says {}",
                self.container_version
            )));
        }
        let mut row = serializer.serialize_tuple(24)?;
        row.serialize_element(&self.container_version)?;
        row.serialize_element(&self.run)?;
        row.serialize_element(&self.attempt)?;
        row.serialize_element(&self.run_name)?;
        row.serialize_element(&self.commit)?;
        row.serialize_element(&self.dirty)?;
        row.serialize_element(&self.timestamp)?;
        row.serialize_element(&self.hostname)?;
        row.serialize_element(&self.slice_width)?;
        row.serialize_element(&self.keyframe_cadence_slices)?;
        row.serialize_element(&self.frame)?;
        row.serialize_element(&self.sections)?;
        row.serialize_element(&self.world)?;
        row.serialize_element(&self.scenario)?;
        row.serialize_element(&self.data)?;
        row.serialize_element(&self.sim_config)?;
        row.serialize_element(&self.producer)?;
        row.serialize_element(&in_id_order(&self.cells, |(cell, _)| *cell))?;
        row.serialize_element(&in_id_order(&self.stations, |(station, _)| *station))?;
        row.serialize_element(&in_id_order(&self.robots, |entry| entry.robot))?;
        row.serialize_element(&self.floor)?;
        row.serialize_element(&self.census)?;
        row.serialize_element(&self.metrics)?;
        row.serialize_element(&self.tables)?;
        row.end()
    }
}

impl Scenario {
    /// Read the scenario block out of the frame that opens a recording. Its
    /// offset is zero by the format's own rule: the block carries the magic,
    /// so no other frame can come first.
    ///
    /// A block whose version is not the one this crate reads is refused here
    /// rather than half-understood, even though the magic ahead of it passed:
    /// the two disagreeing is itself the fault.
    pub fn read_from<R: Read + Seek>(reader: &mut Reader<R>) -> Result<Self> {
        Self::from_frame(&reader.frame_at(0)?)
    }

    /// The same, out of a recording's first frame already in hand: a reader
    /// taking a recording's bytes as they come has no file to read it from.
    /// A first frame that is not the scenario frame opens nothing.
    pub fn from_frame(frame: &Frame) -> Result<Self> {
        if frame.header.tag != Tag::Scenario {
            return Err(Error::ScenarioNotFirst { offset: 0 });
        }
        let scenario = Self::from_bytes(&frame.contents(0)?, 0)?;
        match scenario.container_version == VERSION {
            true => Ok(scenario),
            false => Err(Error::UnsupportedVersion { found: scenario.container_version, expected: VERSION }),
        }
    }
}

impl Payload for Scenario {
    const NAME: &'static str = "scenario block";

    /// Decode as any payload does, and then hold the block to the one rule a
    /// reader cannot make good on its own: it names the frame every position
    /// after it is measured in. A block naming none leaves every millimetre in
    /// the file measured from an origin nothing states, so it is refused here
    /// — on a wire as well as in a file, since both arrive through this.
    fn from_bytes(bytes: &[u8], offset: u64) -> Result<Self> {
        let scenario: Self = rmp_serde::from_slice(bytes).map_err(|e| Error::PayloadShape {
            offset,
            expected: Self::NAME,
            detail: e.to_string(),
        })?;
        match scenario.frame.is_empty() {
            true => Err(Error::RecordingNamesNoFrame { offset }),
            false => Ok(scenario),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::Ms;

    #[test]
    fn the_scenario_block_is_twenty_four_positions_with_its_documents_byte_for_byte() {
        let scenario = Scenario {
            run: "run-1".to_owned(),
            attempt: 2,
            slice_width: SimDuration::seconds(10),
            keyframe_cadence_slices: 30,
            world: Document(vec![b'{', b'}']),
            data: Some(vec![("racks.csv".to_owned(), Document(vec![1, 2, 3]))]),
            sim_config: Opaque::of(&vec![1u32, 2, 3], "sim config").expect("encodes"),
            robots: vec![RobotEntry {
                robot: RobotId(4),
                name: "htm-4".to_owned(),
                class: RobotClass::Htm,
                class_entity: None,
            }],
            frame: HAND_BUILT_FRAME.to_owned(),
            ..Scenario::default()
        };
        let bytes = scenario.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0xdc, "more than fifteen positions, so a sixteen-bit array header");
        assert_eq!(&bytes[1..3], &[0x00, 24], "twenty-four positions");
        assert_eq!(bytes[3], VERSION, "the container version opens the block");

        let read = Scenario::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read, scenario);
        assert_eq!(read.world.0, b"{}", "the world document comes back as the bytes it was read as");
        assert_eq!(read.sim_config.read::<Vec<u32>>("sim config").expect("reads back"), vec![1, 2, 3]);
        assert_eq!((read.floor, read.census), (None, None), "a producer states neither; a server fills both in");
    }

    /// What a server sends: the two documents a browser has no use for state
    /// nothing at all, as one MessagePack nil apiece rather than an empty
    /// array and an empty binary value.
    #[test]
    fn a_wire_scenario_states_nil_for_the_data_files_and_the_producer_document() {
        let sent = Scenario { data: None, producer: None, frame: HAND_BUILT_FRAME.to_owned(), ..Scenario::default() };
        let bytes = sent.to_bytes().expect("encodes");
        // Positions 14 and 16 of the block, behind the two-byte array header.
        let positions: Vec<u8> = bytes[3..].to_vec();
        assert!(positions.contains(&0xc0), "nil is on the wire");
        let read = Scenario::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!((read.data, read.producer), (None, None));
    }

    #[test]
    fn the_compiled_floor_and_census_travel_as_two_documents_this_crate_does_not_read() {
        let sent = Scenario {
            floor: Some(Opaque::of(&vec![9u32, 8], "floor").expect("encodes")),
            census: Some(Opaque::of(&vec![("entity-1", vec![3u32])], "census").expect("encodes")),
            frame: HAND_BUILT_FRAME.to_owned(),
            ..Scenario::default()
        };
        let read = Scenario::from_bytes(&sent.to_bytes().expect("encodes"), 0).expect("decodes");
        assert_eq!(read.floor.expect("the floor came through").read::<Vec<u32>>("floor").expect("reads"), vec![9, 8]);
        assert_eq!(read.census, sent.census, "and the census byte for byte");
    }

    /// Every position after the block is measured in the frame the block
    /// names, so a block naming none is refused where it is decoded — which is
    /// the same door a file and a wire both come through.
    #[test]
    fn a_scenario_block_that_names_no_frame_is_refused_on_the_way_in() {
        let nameless = Scenario::default();
        assert_eq!(nameless.frame, "", "a default states no frame of its own");
        let bytes = nameless.to_bytes().expect("the encoder writes what it is given");
        let error = Scenario::from_bytes(&bytes, 96).expect_err("nothing says what its millimetres measure from");
        assert!(matches!(error, Error::RecordingNamesNoFrame { offset: 96 }), "got {error}");

        let named = Scenario { frame: HAND_BUILT_FRAME.to_owned(), ..Scenario::default() };
        let read = Scenario::from_bytes(&named.to_bytes().expect("encodes"), 0).expect("a block that names one reads");
        assert_eq!(read.frame, HAND_BUILT_FRAME);
    }

    /// A recording opens on its first frame, and only the scenario frame opens
    /// one: a delta carrying a scenario block's very bytes opens nothing.
    #[test]
    fn only_the_scenario_frame_opens_a_recording() {
        let block = Scenario { frame: HAND_BUILT_FRAME.to_owned(), ..Scenario::default() }.to_bytes().expect("encodes");
        let opening = Frame::build(Tag::Scenario, Ms::ZERO, &block, true).expect("builds");
        assert_eq!(Scenario::from_frame(&opening).expect("the scenario frame opens it").frame, HAND_BUILT_FRAME);
        let delta = Frame::build(Tag::Delta, Ms::ZERO, &block, true).expect("builds");
        let error = Scenario::from_frame(&delta).expect_err("a delta is not the scenario frame");
        assert!(matches!(error, Error::ScenarioNotFirst { offset: 0 }), "got {error}");
    }

    #[test]
    fn a_robot_entry_states_its_class_as_one_byte() {
        let entry =
            RobotEntry { robot: RobotId(7), name: "vtm-7".to_owned(), class: RobotClass::Vtm, class_entity: None };
        let bytes = rmp_serde::to_vec(&entry).expect("encodes");
        assert_eq!(bytes[0], 0x94, "four positions");
        assert_eq!(bytes[1], 7, "robot id");
        assert_eq!(bytes[bytes.len() - 2..], [RobotClass::Vtm.as_byte(), 0xc0], "the class byte, then no entity");
        assert_eq!(rmp_serde::from_slice::<RobotEntry>(&bytes).expect("decodes"), entry);
    }
}
