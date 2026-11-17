//! # worldview
//!
//! The `.worldview` a simulation run is recorded into: how a frame is laid
//! out, where the frames sit, and how to get from a simulated instant to the
//! bytes that describe it.
//!
//! **What it is for.** One artifact per run carries everything the run means,
//! and every way of looking at a run is one cheap operation over it: watch it
//! while it executes, leave and rejoin, scrub backward and forward, read a
//! crashed run's prefix, and rebuild the workbook afterwards — none of them
//! decoding from the start.
//!
//! **What is here.** [`container`] — the frames, the sections inside a
//! keyframe or a delta, the index block, the trailer, the writer, the reader,
//! recovery and compression — and [`frames`], the vocabulary above it: the
//! scenario block, declarations, playback, scalars, heat, tables, motion, the
//! end block, and the state a reader rebuilds from a keyframe and the deltas
//! after it. The container carries payload bytes and does not read them;
//! everything that gives them meaning is in `frames`.
//!
//! Nothing here depends on the simulator, which pulls in crates that do not
//! build for wasm32, so a browser decodes frames with this same code. The
//! parts of a run whose types belong to the simulator travel as
//! [`Opaque`] MessagePack this crate carries and does not read.

pub mod container;
pub mod frames;

pub use container::Ms;
pub use container::error::{Error, Result};
pub use container::frame::{Frame, FrameHeader, MAGIC, Tag, VERSION};
pub use container::index::{Index, IndexRecord, TRAILER_MAGIC};
pub use container::payload::{Opaque, Payload};
pub use container::reader::{Reader, SeekPlan};
pub use container::sections::Sections;
pub use container::writer::{AppendSink, Writer};
pub use frames::declare::{Declare, Descriptors};
pub use frames::end::End;
pub use frames::scenario::{Document, Scenario};
pub use frames::state::{PlaybackAt, PlaybackState, RunState, state_at, state_at_described};
pub use frames::{Delta, Keyframe, WriteFrames};
