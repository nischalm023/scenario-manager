use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use desgraph::CellId;

use crate::CommitPlan;
use crate::RobotId;

use super::super::cell_reservation::{CellReservation, ReservationId};
use super::super::oracle::{ReservationOracle, SafeIntervalId};
use super::super::table::ReservationConflict;
use crate::{SimDuration, SimTime, TimeRange};

fn mint_id(counter: &AtomicU64) -> ReservationId {
    ReservationId::new(counter.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// ScriptedConflicts: cells blocked at scripted time ranges.
// ---------------------------------------------------------------------------

pub struct ScriptedConflicts {
    pub blocks: Vec<(CellId, SimTime, SimTime)>,
    next_id: AtomicU64,
}

impl ScriptedConflicts {
    pub fn new(blocks: Vec<(CellId, SimTime, SimTime)>) -> Self {
        Self { blocks, next_id: AtomicU64::new(1) }
    }
}

impl ReservationOracle for ScriptedConflicts {
    fn earliest_available(
        &self,
        cell: CellId,
        after: SimTime,
        duration: SimDuration,
        _predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        let mut candidate = after;
        loop {
            let mut pushed = false;
            for &(c, block_start, block_end) in &self.blocks {
                if c == cell && candidate < block_end && candidate + duration > block_start {
                    candidate = block_end;
                    pushed = true;
                }
            }
            if !pushed {
                return Some(candidate);
            }
        }
    }

    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        _predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        // Mirror `TableOracle::commit_path` — return only newly created
        // reservations; the predecessor finalize is a side effect on the
        // table, not part of the result vec. (Production callers use the
        // last committed reservation as the new tail.)
        let mut out = Vec::with_capacity(plan.requests.len());
        for req in &plan.requests {
            let window = req.window;
            out.push(CellReservation::new(mint_id(&self.next_id), req.cell, robot, window));
        }
        Ok(out)
    }

    fn entry_intervals(
        &self,
        cell: CellId,
        _after: SimTime,
        _horizon: SimTime,
        _predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId> {
        // Convert blocks on this cell to free intervals.
        let mut blocks: Vec<(SimTime, SimTime)> =
            self.blocks.iter().filter(|(c, _, _)| *c == cell).map(|(_, s, e)| (*s, *e)).collect();
        blocks.sort_by_key(|(s, _)| *s);
        let mut out: Vec<SafeIntervalId> = Vec::new();
        let mut cursor = SimTime::ZERO;
        for (bs, be) in blocks {
            if cursor < bs {
                out.push((cursor, Some(bs)));
            }
            if be > cursor {
                cursor = be;
            }
        }
        out.push((cursor, None));
        out
    }
}

// ---------------------------------------------------------------------------
// InstrumentedOracle: decorator that records calls.
// ---------------------------------------------------------------------------

pub struct InstrumentedOracle<T: ReservationOracle> {
    pub inner: T,
    pub queries: Mutex<Vec<(CellId, SimTime, SimDuration)>>,
    pub commits: Mutex<Vec<(RobotId, CommitPlan)>>,
}

impl<T: ReservationOracle> InstrumentedOracle<T> {
    pub fn new(inner: T) -> Self {
        Self { inner, queries: Mutex::new(Vec::new()), commits: Mutex::new(Vec::new()) }
    }

    pub fn queried_cells(&self) -> Vec<CellId> {
        let mut cells: Vec<CellId> = self.queries.lock().unwrap().iter().map(|q| q.0).collect();
        cells.sort_by_key(|c| c.0);
        cells.dedup();
        cells
    }

    pub fn query_count(&self) -> usize {
        self.queries.lock().unwrap().len()
    }
}

impl<T: ReservationOracle> ReservationOracle for InstrumentedOracle<T> {
    fn earliest_available(
        &self,
        cell: CellId,
        after: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        self.queries.lock().unwrap().push((cell, after, duration));
        self.inner.earliest_available(cell, after, duration, predecessor)
    }

    fn has_conflict(
        &self,
        cell: CellId,
        start: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> bool {
        self.queries.lock().unwrap().push((cell, start, duration));
        self.inner.has_conflict(cell, start, duration, predecessor)
    }

    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        self.commits.lock().unwrap().push((robot, plan.clone()));
        self.inner.commit_path(robot, plan, predecessor)
    }

    fn entry_intervals(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId> {
        self.inner.entry_intervals(cell, after, horizon, predecessor)
    }
}

// ---------------------------------------------------------------------------
// CommitBlockingOracle: always free during planning, hard-fails commit if
// any blocked cell appears in the plan. Used to exercise the allocator's
// commit-conflict path under single-shot semantics (no soft tail).
// ---------------------------------------------------------------------------

pub struct CommitBlockingOracle {
    pub blocked_cells: Vec<CellId>,
    next_id: AtomicU64,
}

impl CommitBlockingOracle {
    pub fn new(blocked_cells: Vec<CellId>) -> Self {
        Self { blocked_cells, next_id: AtomicU64::new(1) }
    }
}

impl ReservationOracle for CommitBlockingOracle {
    fn earliest_available(
        &self,
        _cell: CellId,
        after: SimTime,
        _duration: SimDuration,
        _predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        Some(after)
    }

    fn entry_intervals(
        &self,
        _cell: CellId,
        _after: SimTime,
        _horizon: SimTime,
        _predecessor: Option<&CellReservation>,
    ) -> Vec<SafeIntervalId> {
        vec![(SimTime::ZERO, None)]
    }

    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        _predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        for req in &plan.requests {
            if self.blocked_cells.contains(&req.cell) {
                let window = req.window;
                return Err(ReservationConflict {
                    cell: req.cell,
                    proposed: window,
                    existing_robot: RobotId(999),
                    existing_window: TimeRange::open_ended(req.window.start()),
                });
            }
        }
        let mut out: Vec<CellReservation> = Vec::with_capacity(plan.requests.len());
        for req in &plan.requests {
            let window = req.window;
            out.push(CellReservation::new(mint_id(&self.next_id), req.cell, robot, window));
        }
        Ok(out)
    }
}
