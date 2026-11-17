//! A synthetic run to read back: frames written the way a producer will write
//! them, from a seed, so a test asks for a cadence and a length and gets a
//! file.
//!
//! The builder keeps no state of its own beyond the seed — every section body
//! is a pure function of the frame it belongs to — so a test can say what the
//! bytes at any instant should be without replaying anything.

#![allow(dead_code)]

/// The same run written in the vocabulary's own records, for the tests that
/// read what a frame means rather than where its bytes sit.
pub mod vocabulary;

use serde::{Deserialize, Serialize};
use worldview::container::frame::Tag;
use worldview::container::writer::Writer;
use worldview::{AppendSink, Ms, Payload};

/// Ten seconds, the slice width the cencora runs use.
pub const SLICE_WIDTH_MS: u64 = 10_000;

/// The section ids the fixture writes, which are the ones the vocabulary will
/// declare: what the viewer draws, the chart values, the heat grid, and the
/// journal rows a delta carries.
pub const PLAYBACK: u8 = 1;
pub const SCALARS: u8 = 2;
pub const HEAT: u8 = 3;
pub const TABLES: u8 = 4;

/// The scenario block's stand-in: the container carries it and never reads
/// it, so a plausible shape is enough.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    pub slice_width_ms: u64,
    pub cadence: u32,
    pub run: String,
}

impl Payload for Scenario {
    const NAME: &'static str = "scenario";
}

/// The end block's stand-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ending {
    pub frames: u64,
    pub reason: String,
}

impl Payload for Ending {
    const NAME: &'static str = "end";
}

/// What run to build.
#[derive(Clone, Copy, Debug)]
pub struct RunShape {
    /// Slices between keyframes. One means every slice carries a keyframe.
    pub cadence: u32,
    pub slices: u32,
    pub seed: u64,
    /// Whether to write the index block, the end frame and the trailer.
    pub finished: bool,
}

impl Default for RunShape {
    fn default() -> Self {
        Self { cadence: 30, slices: 90, seed: 7, finished: true }
    }
}

/// One run's bytes and the instants its frames were stamped with.
pub struct BuiltRun {
    pub bytes: Vec<u8>,
    pub shape: RunShape,
    /// Every slice's start instant, in order.
    pub slices: Vec<Ms>,
    /// The instants a keyframe states.
    pub keyframes: Vec<Ms>,
    /// The instants a declare frame was written at.
    pub declares: Vec<Ms>,
}

/// One section body: a pure function of the frame it belongs to, so a test
/// says what it expects without keeping the run's state.
pub fn body(tag: Tag, slice: u32, section: u8, seed: u64) -> Vec<u8> {
    let mix = seed.wrapping_add(u64::from(slice) * 31).wrapping_add(u64::from(section) * 7).wrapping_add(match tag {
        Tag::Keyframe => 1_009,
        _ => 0,
    });
    let len = 16 + (mix % 48) as usize;
    (0..len).map(|i| ((mix as usize + i * 13) % 251) as u8).collect()
}

/// The sections a keyframe or a delta of one slice carries.
pub fn sections(tag: Tag, slice: u32, seed: u64) -> Vec<(u8, Vec<u8>)> {
    let ids: &[u8] = match tag {
        Tag::Keyframe => &[PLAYBACK, SCALARS, HEAT],
        _ if slice.is_multiple_of(3) => &[PLAYBACK, SCALARS, TABLES],
        _ => &[PLAYBACK, SCALARS],
    };
    ids.iter().map(|id| (*id, body(tag, slice, *id, seed))).collect()
}

/// Whether a declare frame precedes this slice. Declarations arrive as a run
/// discovers what it is recording, so the fixture scatters a few.
pub fn declares_before(slice: u32) -> bool {
    slice == 0 || slice % 37 == 11
}

/// Write a whole run and hand back its bytes.
pub fn build(shape: RunShape) -> BuiltRun {
    let mut bytes = Vec::new();
    let mut slices = Vec::new();
    let mut keyframes = Vec::new();
    let mut declares = Vec::new();
    {
        let mut writer = Writer::new(&mut bytes);
        let scenario =
            Scenario { slice_width_ms: SLICE_WIDTH_MS, cadence: shape.cadence, run: format!("seed {}", shape.seed) };
        writer
            .append_scenario(Ms::ZERO, &scenario.to_bytes().expect("the scenario encodes"))
            .expect("the scenario frame opens the file");

        for slice in 0..shape.slices {
            let at = Ms(u64::from(slice) * SLICE_WIDTH_MS);
            slices.push(at);
            if declares_before(slice) {
                let declared = Ending { frames: u64::from(slice), reason: format!("declared at slice {slice}") };
                writer.append_payload(Tag::Declare, at, &declared).expect("a declaration");
                declares.push(at);
            }
            if slice.is_multiple_of(shape.cadence) {
                append(&mut writer, Tag::Keyframe, slice, at, shape.seed);
                keyframes.push(at);
            }
            append(&mut writer, Tag::Delta, slice, at, shape.seed);
        }

        if shape.finished {
            let last = slices.last().copied().unwrap_or(Ms::ZERO);
            let ending = Ending { frames: writer.frames_written(), reason: "ran out of orders".to_owned() };
            writer.finish(last, &ending.to_bytes().expect("the ending encodes")).expect("the file closes");
        }
    }
    BuiltRun { bytes, shape, slices, keyframes, declares }
}

fn append<S: AppendSink>(writer: &mut Writer<S>, tag: Tag, slice: u32, at: Ms, seed: u64) {
    let owned = sections(tag, slice, seed);
    let borrowed: Vec<(u8, &[u8])> = owned.iter().map(|(id, body)| (*id, body.as_slice())).collect();
    writer.append_sections(tag, at, &borrowed).expect("a keyframe or a delta");
}
