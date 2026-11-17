//! Within `metrics`, `worldview` is THE RECORDING: the one place a run
//! becomes a `.worldview` file, slice by slice as the run goes, and a
//! `.worldview` file becomes a [`PerfProfile`] again.
//!
//! **What it is for.** Everything downstream of a run — desvis's playback and
//! charts, the workbook, `compare`, `repeat`, the traffic report — is written
//! against the run model and nothing else. A run has to leave the process to
//! reach any of them, and this module is the only crossing: [`write`] states
//! a run as the format's frames, [`read`] puts the run model back together
//! from them. Nothing else in the simulator knows the format exists.
//!
//! **What each part does.** [`wire`] swaps values between the observer's
//! vocabulary and the format's records; [`journals`] turns the typed journals
//! into declared rows and back; [`summary`] packs the simulator's own
//! documents, leaving the journals out of the one at the end.
//!
//! **Why it is written as the run goes.** The observer makes a slice final
//! once B has passed its end, and nothing writes into it afterwards
//! (`observer::finality`), so a slice can be stated the moment it is final: a
//! run that dies leaves every slice before the break, and what reads a run
//! live reads the same frames the file holds.

pub mod journals;
pub mod read;
pub mod summary;
pub mod wire;
pub mod write;

pub use read::{IncomingRecording, Recording, read_worldview};
pub use write::{DEFAULT_KEYFRAME_CADENCE_SLICES, RecordingError, RunRecording, RunScenario};

/// The extension a recording is written with, with no dot.
pub const EXTENSION: &str = "worldview";
