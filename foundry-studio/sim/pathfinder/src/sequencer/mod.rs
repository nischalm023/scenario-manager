//! Who plans next: a [`Sequencer`] grants one turn per search-and-commit cycle, the only
//! thing a `reserve_path` is meant to await. It orders planning in scheduler time, without advancing simtime. A
//! blocked robot asks again when a commit frees a cell it needs. Nothing drives it yet: the
//! pathfinder trait takes a sequencer in a later step.

use async_trait::async_trait;
use desgraph::CellId;

use crate::{PathfinderError, RobotId};

mod round_robin;

pub use round_robin::RoundRobin;

// TODO: Progress scratch arena, I think these allocations might add up over time since these are per-turn, per-robot.
// But measure and find out.

/// The planning result of a granted turn; a failed search or commit is `Err` at `end_turn`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The path was committed in scheduler time, including releases of `freed`
    /// cells. The robot need not have reached the destination in simtime.
    Arrived { freed: Vec<CellId> },
    /// Planning could not commit. `by` names blocking robots; `needs` names
    /// cells whose release could allow another attempt. Empty `needs` lets
    /// RoundRobin retry after any successful commit.
    Blocked { by: Vec<RobotId>, needs: Vec<CellId> },
}

/// What a granted turn is for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "a turn must be taken and ended"]
pub enum Turn {
    Advance,
}

#[derive(Debug)]
#[must_use = "a granted turn must be returned to its sequencer's end_turn"]
pub struct TurnGuard {
    robot_id: RobotId,
    passed: bool,
}

impl TurnGuard {
    /// Issue a guard after granting a turn. Public so sequencers implemented
    /// outside this crate can participate in the same turn protocol.
    pub fn new(robot_id: RobotId) -> Self {
        Self { robot_id, passed: false }
    }

    pub fn robot_id(&self) -> RobotId {
        self.robot_id
    }

    /// Explicitly disarm the `Drop` panic, indicating that the turn was properly ended with `end_turn`.
    ///
    /// [`Sequencer`] implementations *must* call this on [`Sequencer::end_turn`].
    pub fn disarm(&mut self) {
        self.passed = true;
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        // A guard dropped by a panic is that panic's business: a second one here would abort the process
        // past the scheduler's catch. Only a quiet drop is the bug this names.
        if !self.passed && !std::thread::panicking() {
            panic!("TurnGuard for robot {self:?} dropped without being passed to end_turn");
        }
    }
}

/// Orders search-and-commit cycles in scheduler time. A caller may have at
/// most one outstanding request or held turn per robot on this sequencer.
///
/// Dropping an ungranted request cancels it. Once granted, the caller must
/// return the guard to the same sequencer's `end_turn`, including on a search
/// or commit error, before asking again. Implementations disarm it there.
/// The guard checks completion on drop; it does not authenticate its issuer.
#[async_trait(?Send)]
pub trait Sequencer: Send + Sync {
    /// Request permission to plan from `at` to `to`. RoundRobin orders requests
    /// by their first poll, ignores these cells, and rejects duplicate requests.
    async fn next_turn(&self, robot: RobotId, at: CellId, to: CellId) -> (Turn, TurnGuard);

    /// Finish the planning turn with its commit result, without waiting for
    /// physical arrival in simtime.
    fn end_turn(&self, guard: TurnGuard, progress: &Result<Progress, PathfinderError>);
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::TurnGuard;
    use crate::RobotId;

    /// A turn dropped by a panic lets that panic unwind; the guard adds no second one, which would abort.
    #[test]
    fn a_guard_dropped_while_unwinding_does_not_panic_again() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = TurnGuard::new(RobotId(1));
            panic!("the engine failed mid-turn");
        }));
        assert!(result.is_err());
    }

    #[test]
    #[should_panic(expected = "dropped without being passed to end_turn")]
    fn a_guard_dropped_quietly_panics() {
        drop(TurnGuard::new(RobotId(1)));
    }
}
