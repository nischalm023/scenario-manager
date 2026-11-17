//! The cell's registry, within `cell`: for each open-ended hold, the
//! proposals that would vacate it and the entrants that need that decided.
//!
//! A proposal whose task would move a robot off the hold it stands on
//! registers a `Vacate`: "I'd vacate this hold at simtime t". An entrant
//! blocked on the hold, in scheduler time, reads the hold's earliest
//! `Vacate` (`vacating`) and registers a `NeedBy` answering it: "decide it
//! before the barrier passes simtime T". That `Vacate`'s decision deadline
//! is then its earliest need-by, never later than its own start and never
//! behind the barrier. A `NeedBy` stays live only while the `Vacate` it
//! answered does: once that `Vacate` is replaced (scheduler time), the
//! entrant answers the replacement anew, reading the replacement's simtime
//! rather than the old one's. A registration promises a pending decision,
//! never that the cell is free; both kinds drop themselves with their owner.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use desgraph::CellId;
use rustc_hash::FxHashMap;

use crate::model::SimTime;
use crate::model::reservation::cell_reservation::{CellReservation, ReservationId};
use crate::model::reservation::oracle::ReservationOracle;
use crate::scheduler::task::proposal::ProposalId;
use crate::signals::Version;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VacateId {
    hold: ReservationId,
    owner: ProposalId,
}

/// The earliest `Vacate` on an open hold, as an entrant reads it.
#[derive(Clone, Copy)]
pub struct Vacating {
    id: VacateId,
    pub at: SimTime,
    cell: CellId,
}

struct VacateEntry {
    at: SimTime,
    need_bys: FxHashMap<u64, SimTime>,
}

#[derive(Default)]
struct State {
    holds: FxHashMap<ReservationId, FxHashMap<ProposalId, VacateEntry>>,
    next_need_by: u64,
}

#[derive(Clone)]
pub struct CellRegistry {
    state: Arc<Mutex<State>>,
    changed: Version,
    oracle: Arc<dyn ReservationOracle>,
}

impl CellRegistry {
    pub fn new(oracle: Arc<dyn ReservationOracle>) -> Self {
        Self { state: Arc::new(Mutex::new(State::default())), changed: Version::new(), oracle }
    }

    fn hold_is_open(&self, cell: CellId, id: ReservationId) -> bool {
        self.oracle.open_ended_reservation(cell).is_some_and(|hold| hold.id() == id)
    }

    pub fn changed(&self) -> &Version {
        &self.changed
    }

    /// The `Vacate`s registered on any hold, open or closed: a closed hold
    /// hides its `Vacate`s from `vacating`, so a test counts them here.
    #[cfg(any(test, feature = "test-support"))]
    pub fn vacate_count(&self) -> usize {
        self.state.lock().unwrap().holds.values().map(FxHashMap::len).sum()
    }

    /// `owner`'s task would vacate `hold` at simtime `at`.
    pub fn vacate(&self, hold: &CellReservation, owner: ProposalId, at: SimTime) -> Vacate {
        let id = VacateId { hold: hold.id(), owner };
        let mut state = self.state.lock().unwrap();
        let vacates = state.holds.entry(id.hold).or_default();
        assert!(!vacates.contains_key(&owner), "one attempt registers its Vacate on a hold once");
        vacates.insert(owner, VacateEntry { at, need_bys: FxHashMap::default() });
        drop(state);
        self.changed.bump();
        Vacate { registry: self.clone(), id, cell: hold.cell() }
    }

    /// The earliest `Vacate` on the caller's exact hold, not just its cell or
    /// robot, while the oracle still holds it open-ended.
    pub fn vacating(&self, hold: &CellReservation) -> Option<Vacating> {
        if !self.hold_is_open(hold.cell(), hold.id()) {
            return None;
        }
        let state = self.state.lock().unwrap();
        let vacates = state.holds.get(&hold.id())?;
        let (&owner, entry) = vacates.iter().min_by_key(|(owner, entry)| (entry.at, **owner))?;
        Some(Vacating { id: VacateId { hold: hold.id(), owner }, at: entry.at, cell: hold.cell() })
    }

    /// The entrant needs `vacating` decided before the barrier passes simtime
    /// `by`.
    pub fn need_by(&self, vacating: Vacating, by: SimTime) -> Option<NeedBy> {
        if !self.hold_is_open(vacating.cell, vacating.id.hold) {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        let token = state.next_need_by;
        state.next_need_by += 1;
        state.holds.get_mut(&vacating.id.hold)?.get_mut(&vacating.id.owner)?.need_bys.insert(token, by);
        drop(state);
        self.changed.bump();
        Some(NeedBy { registry: self.clone(), id: vacating.id, cell: vacating.cell, token })
    }

    /// Keeps a need-by at simtime `by` on `hold`'s earliest `Vacate`, and
    /// answers a replacement anew, for an entrant that waits on the hold in
    /// scheduler time. It never resolves; dropping it withdraws the need-by.
    pub async fn keep_need_by(&self, hold: &CellReservation, by: SimTime) -> Infallible {
        let mut need_by: Option<NeedBy> = None;
        loop {
            let seen = self.changed.seen();
            if !need_by.as_ref().is_some_and(NeedBy::is_live) {
                need_by = self.vacating(hold).and_then(|vacating| self.need_by(vacating, by));
            }
            self.changed.past(seen).await;
        }
    }
}

/// A proposal's registration that its task would vacate a hold.
pub struct Vacate {
    registry: CellRegistry,
    id: VacateId,
    cell: CellId,
}

impl Vacate {
    /// The decision deadline: the earliest need-by answering this `Vacate`
    /// while its hold stays open, else the proposal's own `start`; never
    /// behind `floor`, the barrier.
    pub fn resolve_by(&self, start: SimTime, floor: SimTime) -> SimTime {
        let state = self.registry.state.lock().unwrap();
        let needed = state
            .holds
            .get(&self.id.hold)
            .and_then(|vacates| vacates.get(&self.id.owner))
            .and_then(|entry| entry.need_bys.values().min().copied());
        drop(state);
        match needed {
            Some(by) if self.registry.hold_is_open(self.cell, self.id.hold) => by.min(start).max(floor),
            _ => start.max(floor),
        }
    }
}

impl Drop for Vacate {
    fn drop(&mut self) {
        let mut state = self.registry.state.lock().unwrap();
        if let Some(vacates) = state.holds.get_mut(&self.id.hold) {
            vacates.remove(&self.id.owner);
            if vacates.is_empty() {
                state.holds.remove(&self.id.hold);
            }
        }
        drop(state);
        self.registry.changed.bump();
    }
}

/// An entrant's registration that it needs a `Vacate` decided before the
/// barrier passes a given simtime.
pub struct NeedBy {
    registry: CellRegistry,
    id: VacateId,
    cell: CellId,
    token: u64,
}

impl NeedBy {
    /// Whether the hold is still open and the `Vacate` this answered is still
    /// registered.
    pub fn is_live(&self) -> bool {
        self.registry.hold_is_open(self.cell, self.id.hold)
            && self.registry.state.lock().unwrap().holds.get(&self.id.hold).is_some_and(|vacates| {
                vacates.get(&self.id.owner).is_some_and(|entry| entry.need_bys.contains_key(&self.token))
            })
    }
}

impl Drop for NeedBy {
    fn drop(&mut self) {
        let mut state = self.registry.state.lock().unwrap();
        if let Some(entry) = state.holds.get_mut(&self.id.hold).and_then(|vacates| vacates.get_mut(&self.id.owner)) {
            entry.need_bys.remove(&self.token);
        }
        drop(state);
        self.registry.changed.bump();
    }
}

#[cfg(test)]
mod tests;
