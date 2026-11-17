//! The frames layer: what the container's sections MEAN — the typed records a
//! run is written as, and the state a reader rebuilds from them.
//!
//! **What it is for.** The container hands over bytes under a section id and
//! has no opinion about them. Everything above it — a viewer drawing an
//! instant, a chart reading a series, the workbook tabulating a run — needs
//! one agreed reading of those bytes, and one place that reading is written
//! down. This module is that place: the scenario block that opens a run, the
//! declarations that extend it, the sections a keyframe and a delta carry,
//! the end block that closes it, and the rebuild ([`state::RunState`]) that
//! turns one keyframe plus the deltas after it into where everything is and
//! what every robot is doing.
//!
//! **Simulator-agnostic.** The vocabulary speaks the run's own ids — cells,
//! robots, totes, stations — and quantities from `desunits`. It knows no
//! simulator type, so a browser reads a recording with this same code, and
//! the parts that are the producer's own travel as [`Opaque`] documents this
//! crate carries and never reads.
//!
//! **The clock.** The container stamps frames with [`Ms`]; a record speaks
//! `SimTime` and `SimDuration`, which serialize as the same u64
//! milliseconds. [`stamp`] and [`instant`] convert at that one boundary.

use desunits::SimTime;

use crate::container::Ms;
use crate::container::error::{Error, Result};
use crate::container::frame::Tag;
use crate::container::index::IndexRecord;
use crate::container::payload::Payload;
use crate::container::sections::Sections;
use crate::container::writer::{AppendSink, Writer};

pub mod declare;
pub mod end;
pub mod heat;
pub mod motion;
pub mod playback;
pub mod robot_state;
pub mod scalars;
pub mod scenario;
pub mod state;
pub mod tables;

use declare::Declare;
use end::End;
use heat::{HeatDelta, HeatKeyframe};
use motion::MotionDelta;
use playback::{PlaybackDelta, PlaybackKeyframe};
use robot_state::{RobotStateDelta, RobotStateKeyframe};
use scalars::{ScalarsDelta, ScalarsKeyframe};
use scenario::Scenario;
use tables::TablesDelta;

/// What the viewer draws: where every robot and tote is.
pub const PLAYBACK: u8 = 1;
/// The chart values, one slot per declared metric.
pub const SCALARS: u8 = 2;
/// The grid counters the heat maps are drawn from.
pub const HEAT: u8 = 3;
/// The journal rows whose instants fall in the slice. Deltas only.
pub const TABLES: u8 = 4;
/// The per-move speed profiles the traffic overlay bins. Deltas only.
pub const MOTION: u8 = 5;
/// What every robot is doing: each robot's state in a keyframe, and in a
/// delta the state spans of each robot that enters a span in its slice. Only
/// in a recording whose producer records the channel, which its scenario
/// block declares.
pub const ROBOT_STATE: u8 = 6;

/// Every section this vocabulary defines, with the name the scenario block
/// declares it under. A consumer takes the ids it knows from here and steps
/// over the rest, so a file written by a later producer still reads.
pub const SECTIONS: [(u8, &str); 6] = [
    (PLAYBACK, "playback"),
    (SCALARS, "scalars"),
    (HEAT, "heat"),
    (TABLES, "tables"),
    (MOTION, "motion"),
    (ROBOT_STATE, "robot_state"),
];

/// The sections a keyframe can carry; a delta can carry every one.
const KEYFRAME_SECTIONS: [u8; 4] = [PLAYBACK, SCALARS, HEAT, ROBOT_STATE];

/// What a section id is called, or `None` for an id this vocabulary does not
/// define.
pub fn section_name(id: u8) -> Option<&'static str> {
    SECTIONS.iter().find(|(section, _)| *section == id).map(|(_, name)| *name)
}

/// The section declarations a scenario block carries, which name the
/// sections every keyframe and every delta of its file carries: every section
/// this vocabulary defines, robot state only when the producer records the
/// channel.
pub fn section_declarations(robot_state: bool) -> Vec<(u8, String)> {
    SECTIONS
        .iter()
        .filter(|(id, _)| robot_state || *id != ROBOT_STATE)
        .map(|(id, name)| (*id, (*name).to_owned()))
        .collect()
}

/// The sections each kind of frame in a file carries, keyframes' then
/// deltas', from what its scenario block declares: this vocabulary's sections
/// by their names, robot state among them or not. Any other list would leave
/// the frames after it carrying sections nothing names, or naming sections no
/// frame carries.
fn carried_sections(declared: &[(u8, String)]) -> Result<(Vec<u8>, Vec<u8>)> {
    if declared != section_declarations(true) && declared != section_declarations(false) {
        return Err(Error::RefusedFrame {
            tag: Tag::Scenario,
            detail: "a scenario block declares this vocabulary's sections by their names, robot state among them or not",
        });
    }
    let delta: Vec<u8> = declared.iter().map(|(id, _)| *id).collect();
    let keyframe = delta.iter().copied().filter(|id| KEYFRAME_SECTIONS.contains(id)).collect();
    Ok((keyframe, delta))
}

/// The instant a frame is stamped with, from the instant a record speaks in.
pub fn stamp(t: SimTime) -> Ms {
    Ms(t.as_ms())
}

/// The instant a record speaks in, from the one a frame is stamped with.
pub fn instant(t: Ms) -> SimTime {
    SimTime::ms(t.0)
}

/// Declare an enum that travels as one byte.
///
/// Serde's own enum form is a one-entry map keyed by the variant name, which
/// the positional convention forbids and which costs bytes in every record.
/// This writes the byte, and refuses a byte no variant answers to rather than
/// silently picking one.
macro_rules! byte_enum {
    (
        $(#[$outer:meta])*
        $name:ident as $what:literal {
            $($(#[$inner:meta])* $variant:ident = $byte:literal),+ $(,)?
        }
    ) => {
        $(#[$outer])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$inner])* $variant = $byte),+
        }

        impl $name {
            /// The byte this value travels as.
            pub fn as_byte(self) -> u8 {
                self as u8
            }

            /// What a byte names, or `None` for one no variant answers to.
            pub fn from_byte(byte: u8) -> Option<Self> {
                Some(match byte {
                    $($byte => Self::$variant,)+
                    _ => return None,
                })
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_u8(self.as_byte())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> ::std::result::Result<Self, D::Error> {
                let byte = <u8 as serde::Deserialize>::deserialize(deserializer)?;
                Self::from_byte(byte).ok_or_else(|| {
                    <D::Error as serde::de::Error>::custom(format!("{} byte {byte} names no {}", $what, $what))
                })
            }
        }
    };
}

pub(crate) use byte_enum;

/// A view of `items` in ascending key order, for a record whose array is
/// state rather than events: the encoder writes through it, so two producers
/// that gathered the same state in different orders write the same bytes.
pub(crate) fn in_id_order<T, K: Ord, F: Fn(&T) -> K>(items: &[T], key: F) -> Vec<&T> {
    let mut view: Vec<&T> = items.iter().collect();
    view.sort_by_key(|item| key(item));
    view
}

/// Everything true at one instant, with no earlier frame needed.
///
/// Every section is optional because a filtered frame — what a server sends a
/// browser that asked for less — carries only the sections that were asked
/// for. A file always writes the first three, and robot state as well when
/// its scenario block declares it.
///
/// This is an assembly of section bodies and is not itself a record: a frame
/// on the wire is the section table and the bodies, which is what
/// [`Keyframe::encode`] hands over.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keyframe {
    pub playback: Option<PlaybackKeyframe>,
    pub scalars: Option<ScalarsKeyframe>,
    pub heat: Option<HeatKeyframe>,
    pub robot_state: Option<RobotStateKeyframe>,
}

/// What one closed slice changed, section by section. The slice ends where the
/// next frame begins, and the last slice where the end block is stamped: at
/// the run's stop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delta {
    pub playback: Option<PlaybackDelta>,
    pub scalars: Option<ScalarsDelta>,
    pub heat: Option<HeatDelta>,
    pub tables: Option<TablesDelta>,
    pub motion: Option<MotionDelta>,
    pub robot_state: Option<RobotStateDelta>,
}

impl Keyframe {
    /// The sections to hand the container's writer, in ascending id.
    pub fn encode(&self) -> Result<Vec<(u8, Vec<u8>)>> {
        let mut sections = Vec::with_capacity(4);
        encode_into(&mut sections, PLAYBACK, self.playback.as_ref())?;
        encode_into(&mut sections, SCALARS, self.scalars.as_ref())?;
        encode_into(&mut sections, HEAT, self.heat.as_ref())?;
        encode_into(&mut sections, ROBOT_STATE, self.robot_state.as_ref())?;
        Ok(sections)
    }

    /// Read one keyframe's sections. A section the frame does not carry
    /// decodes as `None`; a section this vocabulary does not name is left
    /// alone.
    pub fn decode(sections: &Sections, offset: u64) -> Result<Self> {
        Ok(Self {
            playback: decode_from(sections, PLAYBACK, offset)?,
            scalars: decode_from(sections, SCALARS, offset)?,
            heat: decode_from(sections, HEAT, offset)?,
            robot_state: decode_from(sections, ROBOT_STATE, offset)?,
        })
    }
}

impl Delta {
    /// The sections to hand the container's writer, in ascending id.
    pub fn encode(&self) -> Result<Vec<(u8, Vec<u8>)>> {
        let mut sections = Vec::with_capacity(6);
        encode_into(&mut sections, PLAYBACK, self.playback.as_ref())?;
        encode_into(&mut sections, SCALARS, self.scalars.as_ref())?;
        encode_into(&mut sections, HEAT, self.heat.as_ref())?;
        encode_into(&mut sections, TABLES, self.tables.as_ref())?;
        encode_into(&mut sections, MOTION, self.motion.as_ref())?;
        encode_into(&mut sections, ROBOT_STATE, self.robot_state.as_ref())?;
        Ok(sections)
    }

    pub fn decode(sections: &Sections, offset: u64) -> Result<Self> {
        Ok(Self {
            playback: decode_from(sections, PLAYBACK, offset)?,
            scalars: decode_from(sections, SCALARS, offset)?,
            heat: decode_from(sections, HEAT, offset)?,
            tables: decode_from(sections, TABLES, offset)?,
            motion: decode_from(sections, MOTION, offset)?,
            robot_state: decode_from(sections, ROBOT_STATE, offset)?,
        })
    }
}

fn encode_into<P: Payload>(sections: &mut Vec<(u8, Vec<u8>)>, id: u8, body: Option<&P>) -> Result<()> {
    if let Some(body) = body {
        sections.push((id, body.to_bytes()?));
    }
    Ok(())
}

fn decode_from<P: Payload>(sections: &Sections, id: u8, offset: u64) -> Result<Option<P>> {
    sections.get(id).map(|bytes| P::from_bytes(bytes, offset)).transpose()
}

/// Writing a run as the frames this vocabulary defines.
///
/// An extension of the container's writer rather than a change to it: the
/// container stays free of every type here, and a producer that speaks the
/// vocabulary writes a whole run without ever laying out a section table.
pub trait WriteFrames<S: AppendSink> {
    /// The scenario block, which opens the file and carries everything a
    /// reader needs before any instant of the run. Its section declarations
    /// fix which sections every keyframe and delta after it carries.
    fn append_scenario_block(&mut self, scenario: &Scenario) -> Result<IndexRecord>;

    /// One descriptor declared after the scenario block, stamped with the
    /// start of the slice it precedes.
    fn append_declare(&mut self, t: SimTime, declare: &Declare) -> Result<IndexRecord>;

    /// Write a keyframe, which carries every keyframe section the scenario
    /// block declared. An empty section is a present section with an empty
    /// body, never a section left out: a file whose frames differ in which
    /// sections they carry would make a consumer guess whether a run stopped
    /// recording something or simply had nothing to say.
    fn append_keyframe(&mut self, t: SimTime, keyframe: &Keyframe) -> Result<IndexRecord>;

    /// Write a delta, which carries every section the scenario block
    /// declared — the same ones in every slice, empty ones included.
    fn append_delta(&mut self, t: SimTime, delta: &Delta) -> Result<IndexRecord>;

    /// Close the file with the end block, the index block and the trailer,
    /// and hand the sink back.
    fn finish_with(self, t: SimTime, end: &End) -> Result<S>;
}

impl<S: AppendSink> WriteFrames<S> for Writer<S> {
    fn append_scenario_block(&mut self, scenario: &Scenario) -> Result<IndexRecord> {
        let (keyframe, delta) = carried_sections(&scenario.sections)?;
        let record = self.append_scenario(Ms::ZERO, &scenario.to_bytes()?)?;
        self.carry_sections(keyframe, delta);
        Ok(record)
    }

    fn append_declare(&mut self, t: SimTime, declare: &Declare) -> Result<IndexRecord> {
        self.append_payload(Tag::Declare, stamp(t), declare)
    }

    fn append_keyframe(&mut self, t: SimTime, keyframe: &Keyframe) -> Result<IndexRecord> {
        let sections = keyframe.encode()?;
        whole_frame(Tag::Keyframe, &sections, &[PLAYBACK, SCALARS, HEAT])?;
        append_sections(self, Tag::Keyframe, t, sections)
    }

    fn append_delta(&mut self, t: SimTime, delta: &Delta) -> Result<IndexRecord> {
        let sections = delta.encode()?;
        whole_frame(Tag::Delta, &sections, &[PLAYBACK, SCALARS, HEAT, TABLES, MOTION])?;
        append_sections(self, Tag::Delta, t, sections)
    }

    fn finish_with(self, t: SimTime, end: &End) -> Result<S> {
        let body = end.to_bytes()?;
        self.finish(stamp(t), &body)
    }
}

/// Refuse a frame a file cannot hold: one missing a section every recording
/// carries.
///
/// An assembly leaves sections out for the wire, where a server sends a
/// browser only what it asked for. A file is not that: every keyframe in it
/// carries playback, scalars and heat and every delta those, tables and
/// motion, so a consumer reading a run never has to tell a section that says
/// nothing from one that was never sent. Robot state rides in every frame of
/// a file that declares it, which the writer holds the file to as well.
fn whole_frame(tag: Tag, sections: &[(u8, Vec<u8>)], wanted: &[u8]) -> Result<()> {
    for id in wanted {
        if !sections.iter().any(|(section, _)| section == id) {
            let detail = match tag {
                Tag::Keyframe => "a keyframe in a file carries playback, scalars and heat, empty ones included",
                _ => "a delta in a file carries playback, scalars, heat, tables and motion, empty ones included",
            };
            return Err(Error::RefusedFrame { tag, detail });
        }
    }
    Ok(())
}

fn append_sections<S: AppendSink>(
    writer: &mut Writer<S>,
    tag: Tag,
    t: SimTime,
    sections: Vec<(u8, Vec<u8>)>,
) -> Result<IndexRecord> {
    let borrowed: Vec<(u8, &[u8])> = sections.iter().map(|(id, body)| (*id, body.as_slice())).collect();
    writer.append_sections(tag, stamp(t), &borrowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_lays_its_sections_out_in_ascending_id_and_omits_the_ones_it_lacks() {
        let delta = Delta {
            playback: Some(PlaybackDelta::default()),
            motion: Some(MotionDelta::default()),
            ..Delta::default()
        };
        let encoded = delta.encode().expect("encodes");
        assert_eq!(encoded.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![PLAYBACK, MOTION]);
    }

    #[test]
    fn a_filtered_frame_decodes_with_none_where_its_sections_were_taken_out() {
        let keyframe = Keyframe { scalars: Some(ScalarsKeyframe::default()), ..Keyframe::default() };
        let encoded = keyframe.encode().expect("encodes");
        let borrowed: Vec<(u8, &[u8])> = encoded.iter().map(|(id, body)| (*id, body.as_slice())).collect();
        let sections = Sections::decode(Sections::encode(&borrowed), 0).expect("decodes");
        let read = Keyframe::decode(&sections, 0).expect("reads");
        assert_eq!(read, keyframe);
        assert_eq!(read.playback, None, "a section the frame does not carry is absent, not empty");
    }

    /// A scenario block carrying nothing but the frame every recording names.
    fn hand_built_scenario() -> Scenario {
        Scenario { frame: scenario::HAND_BUILT_FRAME.to_owned(), ..Scenario::default() }
    }

    #[test]
    fn a_frame_a_file_cannot_hold_is_refused_by_the_writer_and_not_written() {
        let mut writer = Writer::new(Vec::new());
        writer.append_scenario_block(&hand_built_scenario()).expect("the scenario block opens the file");
        let error = writer
            .append_keyframe(
                SimTime::ZERO,
                &Keyframe { playback: Some(PlaybackKeyframe::default()), ..Keyframe::default() },
            )
            .expect_err("a keyframe in a file carries all three sections");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Keyframe, .. }), "got {error}");
        let error = writer.append_delta(SimTime::ZERO, &Delta::default()).expect_err("and a delta all five");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Delta, .. }), "got {error}");
        assert_eq!(writer.frames_written(), 1, "neither frame reached the file");
    }

    #[test]
    fn the_section_ids_and_names_are_the_ones_the_scenario_block_declares() {
        assert_eq!(section_name(PLAYBACK), Some("playback"));
        assert_eq!(section_name(ROBOT_STATE), Some("robot_state"));
        assert_eq!(section_name(7), None);
        let without = section_declarations(false);
        assert_eq!(without.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
        assert_eq!(section_declarations(true).len(), 6, "and robot state when the producer records it");
    }

    fn whole_keyframe(robot_state: Option<RobotStateKeyframe>) -> Keyframe {
        Keyframe {
            playback: Some(PlaybackKeyframe::default()),
            scalars: Some(ScalarsKeyframe::default()),
            heat: Some(HeatKeyframe::default()),
            robot_state,
        }
    }

    fn whole_delta(robot_state: Option<RobotStateDelta>) -> Delta {
        Delta {
            playback: Some(PlaybackDelta::default()),
            scalars: Some(ScalarsDelta::default()),
            heat: Some(HeatDelta::default()),
            tables: Some(TablesDelta::default()),
            motion: Some(MotionDelta::default()),
            robot_state,
        }
    }

    /// The scenario block's declarations say whether the file states the robot state channel, and every keyframe
    /// and delta after it is held to that: a frame carrying robot state the file never declared, or leaving out
    /// robot state it did, would make a reader guess.
    #[test]
    fn a_file_carries_robot_state_in_every_frame_exactly_when_its_scenario_block_declares_it() {
        let declaring = Scenario { sections: section_declarations(true), ..hand_built_scenario() };
        let mut writer = Writer::new(Vec::new());
        writer.append_scenario_block(&declaring).expect("the scenario block opens the file");
        let error = writer.append_keyframe(SimTime::ZERO, &whole_keyframe(None)).expect_err("declared, not carried");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Keyframe, .. }), "got {error}");
        writer.append_keyframe(SimTime::ZERO, &whole_keyframe(Some(RobotStateKeyframe::default()))).expect("carried");
        let error = writer.append_delta(SimTime::ZERO, &whole_delta(None)).expect_err("a delta carries it too");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Delta, .. }), "got {error}");
        writer.append_delta(SimTime::ZERO, &whole_delta(Some(RobotStateDelta::default()))).expect("carried");

        let mut writer = Writer::new(Vec::new());
        writer.append_scenario_block(&hand_built_scenario()).expect("a block that does not declare it");
        let error = writer
            .append_keyframe(SimTime::ZERO, &whole_keyframe(Some(RobotStateKeyframe::default())))
            .expect_err("robot state the file never declared");
        assert!(matches!(error, Error::RefusedFrame { tag: Tag::Keyframe, .. }), "got {error}");
        writer.append_keyframe(SimTime::ZERO, &whole_keyframe(None)).expect("the five sections every recording has");
    }

    #[test]
    fn a_scenario_block_declaring_other_sections_than_this_vocabularys_is_refused_and_not_written() {
        for sections in [
            Vec::new(),
            vec![(PLAYBACK, "playback".to_owned())],
            vec![
                (1, "playback".into()),
                (2, "scalars".into()),
                (3, "heat".into()),
                (4, "tables".into()),
                (5, "moves".into()),
            ],
            [section_declarations(false), vec![(7, "later".to_owned())]].concat(),
        ] {
            let mut writer = Writer::new(Vec::new());
            let error = writer
                .append_scenario_block(&Scenario { sections: sections.clone(), ..hand_built_scenario() })
                .expect_err("the frames after it would carry sections nothing names");
            assert!(matches!(error, Error::RefusedFrame { tag: Tag::Scenario, .. }), "{sections:?}: {error}");
            assert_eq!(writer.frames_written(), 0);
        }
    }

    #[test]
    fn a_keyframe_and_a_delta_lay_robot_state_out_last_and_read_it_back() {
        let keyframe = whole_keyframe(Some(RobotStateKeyframe::default()));
        let encoded = keyframe.encode().expect("encodes");
        assert_eq!(encoded.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![PLAYBACK, SCALARS, HEAT, ROBOT_STATE]);
        let borrowed: Vec<(u8, &[u8])> = encoded.iter().map(|(id, body)| (*id, body.as_slice())).collect();
        let read = Keyframe::decode(&Sections::decode(Sections::encode(&borrowed), 0).expect("decodes"), 0);
        assert_eq!(read.expect("reads"), keyframe);
        let delta = whole_delta(Some(RobotStateDelta::default()));
        let ids: Vec<u8> = delta.encode().expect("encodes").iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![PLAYBACK, SCALARS, HEAT, TABLES, MOTION, ROBOT_STATE]);
    }
}
