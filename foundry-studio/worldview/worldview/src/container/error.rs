//! Every way reading or writing a `.worldview` can fail.
//!
//! One enum for the whole crate, because a caller handling a bad file handles
//! it in one place. Every variant that describes damaged bytes names the file
//! offset those bytes start at, so a report says where to look. Nothing here
//! is raised by panicking: bytes arriving from a file, an object store or a
//! socket are input, and input never aborts the process.

use std::io::Error as IoError;
use std::result::Result as StdResult;

use thiserror::Error;

use crate::container::Ms;
use crate::container::frame::Tag;

/// What went wrong reading or writing a `.worldview`.
#[derive(Debug, Error)]
pub enum Error {
    #[error("not a .worldview: {found}")]
    NotAWorldview { found: String },

    #[error("this crate reads .worldview container version {expected}; the file says version {found}")]
    UnsupportedVersion { found: u8, expected: u8 },

    #[error("frame at offset {offset}: a frame header is 20 bytes and only {available} remain")]
    ShortHeader { offset: u64, available: usize },

    #[error(
        "frame at offset {offset}: the header declares a {declared}-byte payload and only {available} bytes remain"
    )]
    ShortPayload { offset: u64, declared: u32, available: usize },

    #[error("frame at offset {offset}: the header records crc32c {recorded:#010x}, the bytes hash to {computed:#010x}")]
    ChecksumMismatch { offset: u64, recorded: u32, computed: u32 },

    #[error("frame at offset {offset}: tag byte {tag:#04x} names no frame kind")]
    UnknownTag { offset: u64, tag: u8 },

    #[error("frame at offset {offset}: flags {flags:#04x} set bits this container does not define")]
    UnknownFlags { offset: u64, flags: u8 },

    #[error("frame at offset {offset}: the two reserved header bytes hold {reserved:#06x} and must hold zero")]
    ReservedNonZero { offset: u64, reserved: u16 },

    #[error(
        "frame at offset {offset}: the header declares a {declared}-byte payload, past the {limit}-byte ceiling a stored payload is allowed"
    )]
    PayloadTooLarge { offset: u64, declared: u64, limit: u32 },

    #[error("frame at offset {offset}: decompressing the payload failed: {detail}")]
    Decompress { offset: u64, detail: String },

    #[error(
        "frame at offset {offset}: the payload decompresses past the {limit}-byte ceiling a frame's contents are allowed"
    )]
    DecompressedTooLarge { offset: u64, limit: usize },

    #[error("frame at offset {offset}: the payload is not the MessagePack value a {expected} frame carries: {detail}")]
    PayloadShape { offset: u64, expected: &'static str, detail: String },

    #[error("the {what} this file carries is not the MessagePack the caller asked to read it as: {detail}")]
    OpaqueShape { what: &'static str, detail: String },

    #[error("compressing a payload failed: {detail}")]
    Compress { detail: String },

    #[error(
        "a {len}-byte payload is past the {limit}-byte ceiling a frame's contents are allowed, and no reader would take it back"
    )]
    BodyTooLarge { len: usize, limit: usize },

    #[error("the writer refuses a {tag} frame: {detail}")]
    RefusedFrame { tag: Tag, detail: &'static str },

    #[error("the writer stopped at a write that failed and appends nothing more: {detail}")]
    WriterFailed { detail: String },

    #[error("zstd compression is not built for wasm32; a browser decodes frames, it does not write them")]
    CompressionUnavailable,

    #[error("encoding a {what} payload as MessagePack failed: {detail}")]
    Encode { what: &'static str, detail: String },

    #[error("frame at offset {offset}: the section table is not one contiguous run of bodies: {detail}")]
    BadSectionTable { offset: u64, detail: String },

    #[error("frame at offset {offset}: a {tag} frame carries one payload of its own, not sections")]
    NotASectionedFrame { offset: u64, tag: Tag },

    #[error("index block at offset {offset}: its records do not describe the frames ahead of it: {detail}")]
    BadIndexBlock { offset: u64, detail: String },

    #[error(
        "frame at offset {offset}: the scenario frame carries the magic and opens a file or a stream, so it is the first frame and there is only one"
    )]
    ScenarioNotFirst { offset: u64 },

    #[error("frame at offset {offset}: a frame beginning at {t} cannot follow one beginning at {previous}: {detail}")]
    FrameOutOfOrder { offset: u64, t: Ms, previous: Ms, detail: &'static str },

    #[error("the file carries no keyframe, so no instant can be reached")]
    NoKeyframe,

    #[error("a delta whose slice begins at {begins_ms} ms is not the next one for a state standing at {reached_ms} ms")]
    DeltaNotAtState { begins_ms: u64, reached_ms: u64 },

    #[error("a slice beginning at {begins_ms} ms cannot end at {ends_ms} ms: a run is read forward")]
    SliceEndsBeforeItBegins { begins_ms: u64, ends_ms: u64 },

    #[error("{t_ms} ms is outside the slice beginning at {slice_start_ms} ms, so this delta does not describe it")]
    InstantOutsideSlice { t_ms: u64, slice_start_ms: u64 },

    #[error("the slice beginning at {at_ms} ms moves robot {robot}, which no keyframe placed")]
    DeltaMovesUnplacedRobot { at_ms: u64, robot: u32 },

    #[error(
        "the slice beginning at {at_ms} ms lists robot {robot} as a mover and states no step for it, so it \
         says nothing about where the robot went"
    )]
    MoverWithNoSteps { at_ms: u64, robot: u32 },

    #[error(
        "in the slice beginning at {at_ms} ms, step {step} of robot {robot} takes no time and changes its \
         position, so the robot would cross that distance in no time at all"
    )]
    StepOfNoDurationMoves { at_ms: u64, robot: u32, step: usize },

    #[error("the steps of robot {robot} add past the end of simulated time, so this slice cannot be walked")]
    StepsOverflow { robot: u32 },

    #[error("the frame at {at_ms} ms states the state of robot {robot} twice, and a robot is in one state at a time")]
    RobotStatedTwice { at_ms: u64, robot: u32 },

    #[error("the slice beginning at {at_ms} ms states spans for robot {robot}, whose state no keyframe stated")]
    SpansOfUnstatedRobot { at_ms: u64, robot: u32 },

    #[error(
        "the spans of robot {robot} in the slice beginning at {at_ms} ms do not lay end to end across it: {detail}"
    )]
    RobotSpansOffTheSlice { at_ms: u64, robot: u32, detail: String },

    #[error(
        "the slice beginning at {at_ms} ms says robot {robot}'s first span carries on the state the robot was in, \
         and names another state"
    )]
    SpanDoesNotCarryOn { at_ms: u64, robot: u32 },

    #[error(
        "the slice beginning at {at_ms} ms {detail}: a recording states robot state in every keyframe and delta or \
         in none"
    )]
    RobotStateNotInEveryFrame { at_ms: u64, detail: &'static str },

    #[error(
        "the keyframe at {at_ms} ms {detail} robot {robot}, and a keyframe states the state of every robot it places"
    )]
    KeyframeRobotStatesDisagree { at_ms: u64, robot: u32, detail: &'static str },

    #[error(
        "the slice beginning at {at_ms} ms {what} container {tote}, which no keyframe stated and no creation \
         record introduced, so what kind of container it is was never said"
    )]
    DeltaMovesUnknownTote { at_ms: u64, tote: u32, what: &'static str },

    #[error("a {what} byte of {found} names no kind of container this format has")]
    UnknownContainerByte { what: &'static str, found: u8 },

    #[error("the run places something on cell {cell}, which the floor it is drawn on does not have")]
    PlaceOffTheFloor { cell: u32 },

    #[error(
        "frame at offset {offset}: the scenario block names no frame, so nothing says what the run's \
         positions are measured from"
    )]
    RecordingNamesNoFrame { offset: u64 },

    #[error("a scalars array of {slots} slots names metrics past the {declared} this run has declared")]
    ScalarsPastDescriptors { slots: usize, declared: usize },

    #[error("heat layer {layer}: the count for cell {cell} passes what a counter can hold")]
    CounterOverflow { layer: String, cell: u32 },

    #[error("table {table}: {detail}")]
    BadSplice { table: u16, detail: String },

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: IoError,
    },
}

impl Error {
    /// Wrap an I/O failure with the thing that was being read or written, so
    /// the message says which file and which step, not just "file not found".
    pub(crate) fn io(context: impl Into<String>, source: IoError) -> Self {
        Self::Io { context: context.into(), source }
    }
}

/// The crate's result type.
pub type Result<T> = StdResult<T, Error>;
