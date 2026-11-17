//! Within `write`, `error` is WHY A RECORDING COULD NOT BE WRITTEN. The
//! recording is the run's output, so the run stops on each of these.

use desunits::{RobotId, SimDuration, SimTime};
use rmp_serde::encode::Error as EncodeError;
use thiserror::Error;
use worldview::container::error::Error as ContainerError;

use crate::metrics::streams::StreamError;

/// Why a run's recording could not be written.
#[derive(Debug, Error)]
pub enum RecordingError {
    /// Every position is measured in the world document's primary frame, and
    /// the run names none.
    #[error("the run names no primary frame, so its positions would be measured from nowhere")]
    NoPrimaryFrame,
    /// The recording's file could not be made.
    #[error("opening the recording: {0}")]
    Open(#[source] ContainerError),
    /// The caller handed the run a recorder of its own and a file to record
    /// into, and a run is recorded once.
    #[error("the run was handed both a recorder and a file to record into, and a run is recorded once")]
    RecorderAndFile,
    /// The sink did not take a frame: a disk filling up, a folder that went
    /// away, a reader that hung up.
    #[error("writing {frame} at {at}: {source}")]
    Write {
        frame: &'static str,
        at: SimTime,
        #[source]
        source: ContainerError,
    },
    /// A robot stands at no number, or past what the format's 32-bit
    /// millimetres reach. Rounding it in would draw the robot somewhere it
    /// never was.
    #[error(
        "robot {robot:?} at {at} stands at {axis} = {value} mm, which is not a whole millimetre a recording can state"
    )]
    Coordinate { robot: RobotId, at: SimTime, axis: &'static str, value: f64 },
    /// The observer stated a change past the end of the slice it closes, which
    /// a reader would apply in a slice that never saw it.
    #[error("{what} {which} is stated {offset} into the slice at {slice_start}, which is {width} wide")]
    PastItsSlice { what: &'static str, which: u64, offset: SimDuration, slice_start: SimTime, width: SimDuration },
    /// A container is born with no move at its birth instant, so the
    /// recording has no place to state for it.
    #[error("tote {tote} is born {offset} into the slice at {slice_start} with no move at that instant")]
    BirthWithNoPlace { tote: u32, offset: SimDuration, slice_start: SimTime },
    /// The observer stated a robot's state the robot state channel cannot
    /// carry: for a robot the playback never placed, so no keyframe states
    /// it, or none for a robot it places.
    #[error("robot {robot:?} at {at}: {detail}")]
    RobotState { robot: RobotId, at: SimTime, detail: &'static str },
    /// The run stopped at another instant than the one its last slice ends
    /// at. A reader ends the last slice at the end block, so the robots' spans
    /// would not lie across it.
    #[error("the run stopped at {stop}, and its last slice ends at {last_ends}")]
    EndOffTheLastSlice { stop: SimTime, last_ends: SimTime },
    /// The run states rows for a journal no table of the recording declares.
    #[error("the run states rows for {0}, which no table declares")]
    UndeclaredTable(String),
    /// A frame holds more of one journal's rows than a table block counts.
    #[error("{0} has more rows than one table block can count")]
    TooManyRows(String),
    /// One of the simulator's own documents would not encode.
    #[error("encoding the {what}: {source}")]
    Encoding {
        what: &'static str,
        #[source]
        source: EncodeError,
    },
    /// The journals' rows would not encode.
    #[error("encoding the journals: {0}")]
    Journals(#[source] StreamError),
}
