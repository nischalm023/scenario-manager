//! dessim's half of the reservation oracle. The planning trait, `GhostOracle`
//! and the oracle vocabulary live in `pathfinder`; dessim's
//! `ReservationOracle` is that trait plus the change `Version` local
//! navigation waits subscribe to, and `TableOracle` is the crate's table
//! oracle carrying that `Version` and emitting its holds on handoff cells as
//! facts.

use std::pin::Pin;

use desgraph::CellId;
use pathfinder::ReservationOracle as SearchOracle;
use pathfinder::reservation::diagnostics::CellDutyStats;
use pathfinder::reservation::oracle::TableOracle as SearchTableOracle;
pub use pathfinder::reservation::oracle::{CellStatus, GhostOracle, SafeIntervalId};
use pathfinder::reservation::table::{ReservationConflict, ReservationTable, TraceOp};
use pathfinder::{CellReservation, CommitPlan};
use rustc_hash::FxHashSet;

use crate::events::{CellHeldEvent, CellReleasedEvent, EventJournal};
use crate::model::RobotId;
use crate::signals::Version;
use crate::simtime::{SimDuration, SimTime, TimeRange};

/// The crate's planning oracle plus what the flows need from it: the change
/// `Version` local navigation waits subscribe to before reading. The
/// pathfinder sees only the supertrait.
pub trait ReservationOracle: SearchOracle {
    /// Changes after a successful path commit, including the bounding of a
    /// predecessor's open hold.
    fn version(&self) -> Option<&Version> {
        None
    }
}

impl ReservationOracle for GhostOracle {}

impl ReservationOracle for TableOracle {
    fn version(&self) -> Option<&Version> {
        Some(&self.version)
    }
}

// ---------------------------------------------------------------------------
// Real oracle: wraps ReservationTable
// ---------------------------------------------------------------------------

/// Real oracle backed by ReservationTable. Used in production CollisionDes
/// mode: the crate's table oracle, bumping the `Version` on every commit.
pub struct TableOracle {
    inner: SearchTableOracle,
    version: Version,
    handoff_holds: Option<HandoffHolds>,
}

/// The cells where a robot hands a tote over, whose holds the oracle emits as
/// facts as it commits them: the core reports nothing, so what reads the run
/// learns how long those cells were held only from the holds themselves.
struct HandoffHolds {
    cells: FxHashSet<CellId>,
    events: EventJournal,
}

impl HandoffHolds {
    /// Emit what one committed path did to the holds on handoff cells: the
    /// end of the robot's open hold that its first move bounds, then each hold
    /// it opens.
    fn emit(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
        committed: &[CellReservation],
    ) {
        if let (Some(held), Some(at)) = (predecessor, plan.close_predecessor_at)
            && self.cells.contains(&held.cell())
        {
            self.events.emit(CellReleasedEvent { robot_id: robot, cell: held.cell(), at });
        }
        for hold in committed.iter().filter(|hold| self.cells.contains(&hold.cell())) {
            let window = hold.window();
            self.events.emit(CellHeldEvent {
                robot_id: robot,
                cell: hold.cell(),
                from: window.start(),
                until: window.end(),
            });
        }
    }
}

impl TableOracle {
    pub fn new(table: ReservationTable) -> Self {
        Self { inner: SearchTableOracle::new(table), version: Version::new(), handoff_holds: None }
    }

    /// A table oracle that emits each hold it commits on `handoff_cells`,
    /// and the end of each open one, as a fact in `events`.
    pub fn emitting_handoff_holds(
        table: ReservationTable,
        handoff_cells: FxHashSet<CellId>,
        events: EventJournal,
    ) -> Self {
        Self { handoff_holds: Some(HandoffHolds { cells: handoff_cells, events }), ..Self::new(table) }
    }

    pub fn table(&self) -> &ReservationTable {
        self.inner.table()
    }
}

impl Default for TableOracle {
    fn default() -> Self {
        Self::new(ReservationTable::default())
    }
}

impl SearchOracle for TableOracle {
    fn earliest_available(
        &self,
        cell: CellId,
        after: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> Option<SimTime> {
        self.inner.earliest_available(cell, after, duration, predecessor)
    }

    fn has_conflict(
        &self,
        cell: CellId,
        start: SimTime,
        duration: SimDuration,
        predecessor: Option<&CellReservation>,
    ) -> bool {
        self.inner.has_conflict(cell, start, duration, predecessor)
    }

    fn has_edge_conflict(
        &self,
        from: CellId,
        to: CellId,
        start: SimTime,
        duration: SimDuration,
        acting_robot: Option<RobotId>,
    ) -> bool {
        self.inner.has_edge_conflict(from, to, start, duration, acting_robot)
    }

    fn commit_edge(&self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        self.inner.commit_edge(from, to, robot, window);
    }

    fn release_edge_window(&self, from: CellId, to: CellId, robot: RobotId, window: TimeRange) {
        self.inner.release_edge_window(from, to, robot, window);
    }

    fn commit_path(
        &self,
        robot: RobotId,
        plan: &CommitPlan,
        predecessor: Option<&CellReservation>,
    ) -> Result<Vec<CellReservation>, ReservationConflict> {
        let reservations = self.inner.commit_path(robot, plan, predecessor)?;
        self.version.bump();
        if let Some(handoff_holds) = &self.handoff_holds {
            handoff_holds.emit(robot, plan, predecessor, &reservations);
        }
        Ok(reservations)
    }

    fn find_overlaps(&self) -> Vec<(CellId, RobotId, TimeRange, RobotId, TimeRange)> {
        self.inner.find_overlaps()
    }

    fn robot_has_reservation(&self, robot: RobotId, cell: CellId, at: SimTime) -> bool {
        self.inner.robot_has_reservation(robot, cell, at)
    }

    fn reservations_for_cell(&self, cell: CellId) -> Vec<CellReservation> {
        self.inner.reservations_for_cell(cell)
    }

    fn cell_duty_stats(&self, cell: CellId, horizon_ms: u64) -> CellDutyStats {
        self.inner.cell_duty_stats(cell, horizon_ms)
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

    fn entry_intervals_into(
        &self,
        cell: CellId,
        after: SimTime,
        horizon: SimTime,
        predecessor: Option<&CellReservation>,
        out: &mut Vec<SafeIntervalId>,
    ) {
        self.inner.entry_intervals_into(cell, after, horizon, predecessor, out);
    }

    fn open_ended_reservation(&self, cell: CellId) -> Option<CellReservation> {
        self.inner.open_ended_reservation(cell)
    }

    fn cell_status(&self, cell: CellId) -> CellStatus {
        match self.inner.open_ended_reservation(cell).map(|hold| hold.robot()) {
            Some(robot) => CellStatus::PartiallyPlanned { robot },
            None => CellStatus::Schedulable,
        }
    }

    fn open_ended(&self) -> Vec<CellReservation> {
        self.inner.open_ended()
    }

    fn commit_count(&self) -> Option<u64> {
        self.inner.commit_count()
    }

    fn commit_touching<'a>(&'a self, cells: &'a [CellId], seen: u64) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        self.inner.commit_touching(cells, seen)
    }

    fn enable_trace(&self) {
        self.inner.enable_trace();
    }

    fn drain_trace(&self) -> Vec<TraceOp> {
        self.inner.drain_trace()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use pathfinder::{MoveOp, ReservationRequest};

    use super::*;
    use crate::events::{EventSink, SimEvent};

    /// A committed path emits each hold it opens on a handoff cell, and the
    /// end its first move puts to the robot's open hold there; a hold on any
    /// other cell is no fact.
    #[test]
    fn a_committed_path_emits_its_holds_on_handoff_cells() {
        let (tx, facts) = mpsc::channel();
        let sink: Arc<dyn EventSink> = Arc::new(tx);
        let handoff_cells = [CellId(1), CellId(3)].into_iter().collect();
        let oracle = TableOracle::emitting_handoff_holds(
            ReservationTable::default(),
            handoff_cells,
            EventJournal::new(Some(sink)),
        );
        let robot = RobotId(5);
        let standing = ReservationRequest::spanning(CellId(1), SimTime::ZERO, None);
        let standing = oracle.commit_path(robot, &CommitPlan::single(standing), None).unwrap().remove(0);
        let (second, twelfth, fourteenth) = (SimTime::seconds(10), SimTime::seconds(12), SimTime::seconds(14));
        let path = CommitPlan {
            requests: vec![
                ReservationRequest::spanning(CellId(2), second, Some(fourteenth)),
                ReservationRequest::spanning(CellId(3), twelfth, None),
            ],
            move_ops: vec![
                Some(MoveOp { source_cell: CellId(1), dest_cell: CellId(2), window: TimeRange::new(second, twelfth) }),
                Some(MoveOp {
                    source_cell: CellId(2),
                    dest_cell: CellId(3),
                    window: TimeRange::new(twelfth, fourteenth),
                }),
            ],
            close_predecessor_at: Some(twelfth),
        };
        oracle.commit_path(robot, &path, Some(&standing)).unwrap();
        let held =
            |cell, from, until| SimEvent::CellHeld(CellHeldEvent { robot_id: robot, cell: CellId(cell), from, until });
        assert_eq!(
            facts.try_iter().collect::<Vec<_>>(),
            [
                held(1, SimTime::ZERO, None),
                SimEvent::CellReleased(CellReleasedEvent { robot_id: robot, cell: CellId(1), at: twelfth }),
                held(3, twelfth, None),
            ]
        );
    }
}
