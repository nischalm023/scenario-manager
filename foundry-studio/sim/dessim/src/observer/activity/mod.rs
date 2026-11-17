//! Within `observer`, `activity` is what a robot was doing, as intervals: one
//! per stretch the robot spent inside a scheduled task, bounded by the task
//! milestones the stream reports. A robot has no state; idle is the gap
//! between its tasks. D30 duty, the per-slice distribution, the journal's
//! no-task rows and the bot inspector's timeline are read from these
//! intervals; a DA50's duty is booked from its rail legs directly.
//!
//! Milestones are sorted, not paired as they are read. A task schedules a whole
//! chain of future instants in one step and reports its facts in the order its
//! code writes them, so `HtmDeliver` reports its station arrival before the
//! departure that precedes it. An interval is built once the barrier B has
//! passed what closes it: only behind B is a robot's milestone list complete.

mod distribution;
mod intervals;
mod log;
mod milestone;
mod slice_duty;
#[cfg(test)]
mod tests;
mod visits;

pub(super) use log::ActivityLog;
pub(super) use milestone::{LegNav, Milestone};
pub(super) use visits::{DwellContainment, PendingDwell};

use std::fmt;

use serde::{Deserialize, Serialize};

use thiserror::Error;

use crate::model::{RobotId, SimTime};
use crate::observer::duty::BusyInterval;

/// What a robot was doing across one interval. `Gap` is not an interval: it is
/// the absence of the other three, which is what idle is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActivityKind {
    Gap,
    /// A leg of a task, from its departure to the milestone it exists to reach.
    Travel,
    /// The tote changing hands: an HTM's fork-Get or fork-Put at a diving
    /// board, a lift's alignment and fork at an endpoint.
    ForkOp,
    /// A station visit, from the bot's arrival at the presentation cell to its
    /// departure on the return leg.
    Presentation,
}

impl ActivityKind {
    /// The suffix of the per-slice distribution column that counts the robots
    /// in this kind (`htm_state_<suffix>`, `vtm_state_<suffix>`).
    pub fn column_suffix(self) -> &'static str {
        match self {
            Self::Gap => "Gap",
            Self::Travel => "Travel",
            Self::ForkOp => "ForkOp",
            Self::Presentation => "Presentation",
        }
    }
}

impl fmt::Display for ActivityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Gap => "Idle",
            Self::Travel => "Travel",
            Self::ForkOp => "Fork op",
            Self::Presentation => "Presentation",
        })
    }
}

/// An invariant of a robot's work the reported facts break. Nothing read from
/// facts that contradict each other can be trusted, so the run stops on it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ActivityViolation {
    #[error("{robot:?} reported work from {at} after B had reached {barrier}")]
    WorkBehindBarrier { robot: RobotId, at: SimTime, barrier: SimTime },
    #[error("{robot:?} is inside two scheduled tasks at once: {first:?} overlaps {second:?}")]
    InsideTwoTasks { robot: RobotId, first: BusyInterval, second: BusyInterval },
    #[error("dwell window {dwell:?} escapes the station visit {visit:?} for {robot:?}")]
    DwellEscapesVisit { robot: RobotId, dwell: (SimTime, SimTime), visit: (SimTime, SimTime) },
    #[error("station visit for {robot:?} closed into a departure at {departure} with no recorded dwell")]
    VisitWithoutDwell { robot: RobotId, departure: SimTime },
}
