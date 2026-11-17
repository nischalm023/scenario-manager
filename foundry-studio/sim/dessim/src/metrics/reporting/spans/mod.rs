//! Within `reporting`, `spans` is THE SPAN ENGINE: the robot state channel a recording states, read once and turned
//! into windows of time in a state, each final once the reader has taken the slice past its end, and never revised.
//!
//! Many metrics are time in a state, so each is a short definition here (`definitions`): the robots it measures and
//! the category each of their states counts under. The engine does the rest once for all of them: `reader` holds
//! each robot's state, seeds from a keyframe and closes at the run's stop; `windows` apportions the time to windows and
//! sends each window out once it is final. Between them they hold each robot's state and the windows still open,
//! never the rows since simtime 0.

mod definitions;
mod reader;
mod windows;

use thiserror::Error;
use worldview::Error as WorldviewError;

use crate::model::{RobotId, SimTime};

pub use definitions::{D30StateDefinition, D30WaitDefinition, Da50StateDefinition, Da50WaitDefinition, SpanDefinition};
pub use reader::{SpanEvent, SpanReader};
pub use windows::{SpanWindow, SpanWindows};

/// Why the engine cannot read a recording's robot state channel on.
#[derive(Debug, Error)]
pub enum SpanError {
    /// The channel's own rules refuse a keyframe or a slice.
    #[error(transparent)]
    Channel(#[from] WorldviewError),
    #[error("the keyframe at {at} states robot {robot}, which the recording's roster does not name")]
    OffRoster { at: SimTime, robot: RobotId },
    #[error("the keyframe at {at} states no state for robot {robot}, and a reader begins knowing every robot's")]
    Unstated { at: SimTime, robot: RobotId },
    #[error("a slice beginning at {begins}, where the next slice begins at {next}")]
    NotNextSlice { begins: SimTime, next: SimTime },
    #[error("the run ends at {run_end}, outside the last slice stated, from {from} to {to}")]
    RunEndOutsideLastSlice { run_end: SimTime, from: SimTime, to: SimTime },
    #[error("something told after the run's end, at {run_end}")]
    AfterRunEnd { run_end: SimTime },
    #[error("slices or windows of no width")]
    ZeroWidth,
    #[error("time at {at} told outside the windows still open: before the reader began, or in one gone out")]
    OutsideOpenWindows { at: SimTime },
    #[error("a definition counts time under {category}, which it does not list")]
    UnlistedCategory { category: String },
}
