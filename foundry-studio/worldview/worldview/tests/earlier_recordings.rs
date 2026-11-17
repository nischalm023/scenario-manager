//! Recordings written before the robot state channel, by the producer then on main, read with this crate as they did.
//!
//! The channel is a section of its own, so it changes no record of the five sections every recording carries, and
//! the container version stays at 1. These are the recordings the repository holds from before it: a reader that
//! knows the channel must read every frame of them, mean by each existing section exactly what was written, and say
//! of the robot state that the recording does not state it rather than that every robot is doing nothing.

use std::path::{Path, PathBuf};

use worldview::container::frame::Tag;
use worldview::frames::state::RunState;
use worldview::frames::{Delta, Keyframe, ROBOT_STATE, instant, section_declarations};
use worldview::{Declare, Descriptors, Reader, Scenario};

/// Every recording the repository commits, each written before the channel existed.
///
/// Their scenario blocks were re-encoded once since (2026-11-09) around their world documents, rewritten so that a
/// station's queue is a lane entity of its own, which is the one way the loader reads; the index, end frame and
/// trailer follow the new offsets. Every keyframe, delta and declaration is still the earlier writer's, byte for
/// byte, and those are what this file reads.
fn earlier_recordings() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        "studio/simrunner/fixtures/canned/recording.worldview",
        "studio/viewer/tests/fixtures/minimal_easyloop_flow/recording.worldview",
    ]
    .into_iter()
    .map(|path| root.join(path))
    .collect()
}

/// Every keyframe and delta decodes, carries no robot state, and re-encodes to the very bytes it was read from, so
/// each of the five sections reads as what its producer wrote; and the run replays through a state that holds no
/// robot state from its first keyframe to its last slice.
#[test]
fn every_frame_of_a_recording_from_before_the_channel_reads_as_it_was_written() {
    for path in earlier_recordings() {
        let name = path.display().to_string();
        let mut reader = Reader::open(&path).unwrap_or_else(|e| panic!("{name} opens: {e}"));
        let scenario = Scenario::read_from(&mut reader).unwrap_or_else(|e| panic!("{name}'s scenario block: {e}"));
        assert_eq!(scenario.sections, section_declarations(false), "{name} declares the five sections it carries");

        let mut descriptors = Descriptors::from_scenario(&scenario);
        let records = reader.index().records().to_vec();
        let mut state: Option<RunState> = None;
        let mut deltas = 0;
        for (index, record) in records.iter().enumerate() {
            if record.tag == Tag::Declare {
                descriptors.push(&reader.payload_at::<Declare>(record).expect("a declaration reads"));
                continue;
            }
            if !record.tag.carries_sections() {
                continue;
            }
            let sections = reader.sections_at(record).unwrap_or_else(|e| panic!("{name} at {}: {e}", record.offset));
            assert_eq!(sections.get(ROBOT_STATE), None, "{name} carries no robot state at {} ms", record.t.0);
            let stored: Vec<(u8, Vec<u8>)> = sections.iter().map(|(id, body)| (id, body.to_vec())).collect();
            let (rewritten, at) = match record.tag {
                Tag::Keyframe => {
                    let keyframe = Keyframe::decode(&sections, record.offset).expect("a keyframe decodes");
                    state = Some(RunState::from_keyframe(instant(record.t), &keyframe, &descriptors).expect("seeds"));
                    (keyframe.encode().expect("re-encodes"), record.t)
                }
                _ => {
                    let delta = Delta::decode(&sections, record.offset).expect("a delta decodes");
                    let ends = records[index + 1..]
                        .iter()
                        .find(|next| next.tag != Tag::Declare)
                        .map_or(record.t, |next| next.t)
                        .max(record.t);
                    let replayed = state.as_mut().expect("a delta follows a keyframe");
                    replayed.apply(instant(record.t), instant(ends), &delta).expect("the slice applies");
                    deltas += 1;
                    (delta.encode().expect("re-encodes"), record.t)
                }
            };
            assert_eq!(rewritten, stored, "{name}: the frame at {} ms re-encodes to the bytes it was read from", at.0);
        }
        let state = state.expect("the recording has a keyframe");
        assert!(deltas > 0, "{name} has slices to replay");
        assert_eq!(state.robot_states, None, "{name} states no robot state, which is not every robot doing nothing");
    }
}
