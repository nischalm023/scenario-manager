//! Within `agent`, a `permit` is a running driver's place among the process's `STUDIO_TURNS_AT_ONCE` (spec 01 section
//! 3.1): counted and taken in one step before the turn's row commits as running, fresh or resumed, so admissions racing
//! for the last place cannot all pass; given back when it drops, as the driver lets go of the turn at a wait or an
//! ending, or as the commit never happens. It carries the token Stop cancels.

use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard, PoisonError};

use rustc_hash::FxHashMap;
use studio_protocol::{Problem, ProblemType, Ulid};
use tokio_util::sync::CancellationToken;

use super::Agent;
use crate::problem::ProblemAnswer;

/// A turn this process runs: the permit that holds its place and the token Stop cancels.
pub(super) struct Running {
    permit: u64,
    cancel: CancellationToken,
}

impl Agent {
    fn running(&self) -> MutexGuard<'_, FxHashMap<Ulid, Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A place for a driver of `turn` among the process's `STUDIO_TURNS_AT_ONCE` (spec 01 section 3.1), counted and
    /// taken in one step; `None` when that many run. A driver resuming a turn whose last driver has not yet let go
    /// takes that driver's place.
    pub(super) fn permit(self: &Arc<Self>, turn: &Ulid) -> Option<Permit> {
        let mut running = self.running();
        if !running.contains_key(turn) && running.len() >= self.settings.turns_at_once.0 as usize {
            return None;
        }
        let (permit, cancel) = (self.permits.fetch_add(1, Ordering::Relaxed), CancellationToken::new());
        running.insert(turn.clone(), Running { permit, cancel: cancel.clone() });
        Some(Permit { agent: self.clone(), turn: turn.clone(), permit, cancel })
    }

    /// Cancel the turn when this process runs it; false when it runs nowhere here.
    pub(super) fn cancel(&self, turn: &Ulid) -> bool {
        self.running().get(turn).map(|running| running.cancel.cancel()).is_some()
    }
}

/// A driver's place among the process's `STUDIO_TURNS_AT_ONCE`, taken before its turn's row commits as running so that
/// admissions racing for the last place cannot all pass, and given back when it drops: when the driver lets go of the
/// turn at a wait or an ending, or when the commit never happens.
pub(super) struct Permit {
    agent: Arc<Agent>,
    turn: Ulid,
    permit: u64,
    pub cancel: CancellationToken,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut running = self.agent.running();
        if running.get(&self.turn).is_some_and(|held| held.permit == self.permit) {
            running.remove(&self.turn);
        }
    }
}

/// `agent-busy`: the process already runs `STUDIO_TURNS_AT_ONCE` turns, so nothing was changed.
pub(super) fn busy(instance: &str) -> ProblemAnswer {
    let mut problem = Problem::new(ProblemType::AgentBusy, "The agent service is busy; try again in a minute.");
    problem.retry_after_s = Some(60);
    problem.at(instance).into()
}
