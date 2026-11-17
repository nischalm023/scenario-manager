//! Named tote custody on one conveyor and its shared presentation resources.
//! Capacity is reserved over physical time, independently of task execution
//! order. Tasks own robot routing and item mutation; this service admits
//! unloading, keeps presentation holds and books the shared operator.

mod calendar;
mod claims;
mod custody;
mod intents;
mod machines;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, Weak};

use desgraph::StationId;
use rustc_hash::FxHashMap;
use thiserror::Error;

use crate::model::conveyor::ConveyorId;
use crate::model::{RobotId, SimDuration, SimTime, ToteId};
use crate::signals::Version;

use calendar::Calendar;
pub use claims::{Admission, CollectionClaim, PresentationClaim, TransferClaim};
use custody::ToteHandle;
pub use custody::{ConveyedTote, InventoryPresentation, RetainedHtm};
use intents::IntentData;
pub use intents::{PresentationApproach, PresentationIntent, PresentationQuote};
pub use machines::Conveyances;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConveyanceError {
    #[error("conveyance capacities and presentation locations must be nonzero and distinct")]
    InvalidEquipment,
    #[error("the tote, robot or presentation belongs to another conveyance")]
    DifferentConveyance,
    #[error("tote {0:?} already has an unfinished conveyor visit")]
    ToteAlreadyAdmitted(ToteId),
    #[error("robot {0:?} is already retained by this conveyor")]
    RobotAlreadyRetained(RobotId),
    #[error("unknown presentation location {0:?}")]
    UnknownPresentation(StationId),
    #[error("claim has already been committed or cancelled")]
    SpentClaim,
    #[error("the tote's current state does not permit this operation")]
    WrongState,
    #[error("operation at {at} precedes its earliest legal time {earliest}")]
    TooEarly { at: SimTime, earliest: SimTime },
    #[error("return availability precedes presentation clearance")]
    InvalidReturnTime,
    #[error("an uncommitted transfer still holds the presentation")]
    PendingTransfer,
    #[error("item transfer needs distinct totes and a positive finite duration")]
    InvalidTransfer,
}

#[derive(Clone)]
pub struct Conveyance {
    shared: Arc<Shared>,
}

struct Shared {
    id: ConveyorId,
    capacity: usize,
    retained_capacity: usize,
    inner: Mutex<Inner>,
    version: Version,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    belt: Calendar,
    retained: Calendar,
    slots: FxHashMap<StationId, Calendar>,
    operator: Calendar,
    loads: FxHashMap<u64, Load>,
    htms: FxHashMap<u64, Htm>,
    presentations: FxHashMap<u64, Presentation>,
    transfers: FxHashMap<u64, Transfer>,
    intents: FxHashMap<u64, IntentData>,
    abandoned: Vec<CustodyLeak>,
}

impl Inner {
    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("conveyance claim identity exhausted");
        id
    }

    fn reserve_presentation(
        &mut self,
        shared: &Arc<Shared>,
        tote: &ConveyedTote,
        station: StationId,
        at: SimTime,
    ) -> PresentationClaim {
        let prior_phase = self.loads[&tote.handle.id].visit.phase;
        let id = self.id();
        self.slots.get_mut(&station).unwrap().record(at, 1);
        self.presentations.insert(
            id,
            Presentation {
                load: tote.handle.id,
                station,
                ready_at: at,
                committed: false,
                prior_phase,
                cleared_at: None,
                pick_end: at,
            },
        );
        let load = self.loads.get_mut(&tote.handle.id).unwrap();
        load.presentation = Some(id);
        load.visit.phase = TotePhase::PresentationReserved;
        PresentationClaim { shared: shared.clone(), id, at, tote: tote.clone(), active: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TotePhase {
    Admitted,
    AwaitingPresentation,
    PresentationReserved,
    Presented { station: StationId, ready_at: SimTime },
    Returning { ready_at: SimTime },
    Collected { at: SimTime, robot: RobotId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToteVisit {
    pub tote: ToteId,
    pub admitted_at: SimTime,
    pub boarded_at: Option<SimTime>,
    pub phase: TotePhase,
}

struct Load {
    visit: ToteVisit,
    presentation: Option<u64>,
    collection: Option<u64>,
    handle: Weak<ToteHandle>,
}

struct Htm {
    robot: RobotId,
    boarded_at: Option<SimTime>,
    collected_at: Option<SimTime>,
    collection: Option<u64>,
}

struct Presentation {
    load: u64,
    station: StationId,
    ready_at: SimTime,
    committed: bool,
    prior_phase: TotePhase,
    cleared_at: Option<SimTime>,
    pick_end: SimTime,
}

struct Transfer {
    presentation: u64,
    pair: PairedTransfer,
    committed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairedTransfer {
    pub conveyor: ConveyorId,
    pub station: StationId,
    pub inventory_tote: ToteId,
    pub order_tote: ToteId,
    pub order_robot: RobotId,
    pub start: SimTime,
    pub end: SimTime,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CustodyLeak {
    InventoryTote(ToteId),
    RetainedHtm(RobotId),
    Presentation { tote: ToteId, station: StationId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occupancy {
    pub conveyor: usize,
    pub retained_htms: usize,
    pub presentations: usize,
    pub operator: usize,
}

pub struct ReturningTote {
    pub tote: ConveyedTote,
    pub ready_at: SimTime,
}

impl Conveyance {
    pub fn new(
        id: ConveyorId,
        capacity: usize,
        retained_capacity: usize,
        stations: impl IntoIterator<Item = StationId>,
    ) -> Result<Self, ConveyanceError> {
        let mut inner = Inner::default();
        for station in stations {
            if inner.slots.insert(station, Calendar::default()).is_some() {
                return Err(ConveyanceError::InvalidEquipment);
            }
        }
        if capacity == 0 || retained_capacity == 0 || inner.slots.is_empty() {
            return Err(ConveyanceError::InvalidEquipment);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                id,
                capacity,
                retained_capacity,
                inner: Mutex::new(inner),
                version: Version::new(),
            }),
        })
    }

    pub fn id(&self) -> ConveyorId {
        self.shared.id
    }

    pub fn version(&self) -> &Version {
        &self.shared.version
    }

    pub fn try_admit(
        &self,
        tote: ToteId,
        robot: RobotId,
        not_before: SimTime,
    ) -> Result<Option<Admission>, ConveyanceError> {
        let mut inner = self.shared.inner.lock().unwrap();
        let mut floor = not_before;
        for load in inner.loads.values().filter(|load| load.visit.tote == tote) {
            match load.visit.phase {
                TotePhase::Collected { at, .. } => floor = floor.max(at),
                _ => return Err(ConveyanceError::ToteAlreadyAdmitted(tote)),
            }
        }
        for htm in inner.htms.values().filter(|htm| htm.robot == robot) {
            floor = floor.max(htm.collected_at.ok_or(ConveyanceError::RobotAlreadyRetained(robot))?);
        }
        let Some(belt_at) = inner.belt.open_after(self.shared.capacity, floor) else { return Ok(None) };
        let Some(htm_at) = inner.retained.open_after(self.shared.retained_capacity, floor) else { return Ok(None) };
        let at = belt_at.max(htm_at);
        let id = inner.id();
        inner.belt.record(at, 1);
        inner.retained.record(at, 1);
        inner.loads.insert(
            id,
            Load {
                visit: ToteVisit { tote, admitted_at: at, boarded_at: None, phase: TotePhase::Admitted },
                presentation: None,
                collection: None,
                handle: Weak::new(),
            },
        );
        inner.htms.insert(id, Htm { robot, boarded_at: None, collected_at: None, collection: None });
        drop(inner);
        self.version().bump();
        Ok(Some(Admission { shared: self.shared.clone(), id, at, active: true }))
    }

    pub fn try_present(
        &self,
        tote: &ConveyedTote,
        station: StationId,
        inventory_ready: SimTime,
    ) -> Result<Option<PresentationClaim>, ConveyanceError> {
        self.try_present_any(tote, [station], inventory_ready)
    }

    /// Claim the earliest available eligible presentation in one atomic step.
    /// Equal readiness times prefer the caller's first station, so preference
    /// is explicit and independent of hash iteration or scheduling races.
    pub fn try_present_any(
        &self,
        tote: &ConveyedTote,
        stations: impl IntoIterator<Item = StationId>,
        inventory_ready: SimTime,
    ) -> Result<Option<PresentationClaim>, ConveyanceError> {
        same(&self.shared, &tote.handle.shared)?;
        let mut inner = self.shared.inner.lock().unwrap();
        let load = &inner.loads[&tote.handle.id];
        if load.collection.is_some() {
            return Err(ConveyanceError::WrongState);
        }
        let prior_phase = load.visit.phase;
        let earliest = match prior_phase {
            TotePhase::AwaitingPresentation => load.visit.boarded_at.unwrap(),
            TotePhase::Returning { ready_at } => ready_at,
            _ => return Err(ConveyanceError::WrongState),
        };
        let floor = inventory_ready.max(earliest);
        let mut selected: Option<(StationId, SimTime)> = None;
        for station in stations {
            let slot = inner.slots.get(&station).ok_or(ConveyanceError::UnknownPresentation(station))?;
            if let Some(at) = slot.open_after(1, floor) {
                if selected.is_none_or(|(_, best)| at < best) {
                    selected = Some((station, at));
                }
            }
        }
        let Some((station, at)) = selected else { return Ok(None) };
        let claim = inner.reserve_presentation(&self.shared, tote, station, at);
        drop(inner);
        self.version().bump();
        Ok(Some(claim))
    }

    pub fn try_pair(
        &self,
        presentation: &InventoryPresentation,
        order_tote: ToteId,
        order_robot: RobotId,
        order_ready: SimTime,
        not_before: SimTime,
        duration: SimDuration,
    ) -> Result<TransferClaim, ConveyanceError> {
        same(&self.shared, &presentation.handle.shared)?;
        let mut inner = self.shared.inner.lock().unwrap();
        let held = &inner.presentations[&presentation.handle.id];
        let load = &inner.loads[&held.load];
        if !held.committed || held.cleared_at.is_some() || load.presentation != Some(presentation.handle.id) {
            return Err(ConveyanceError::WrongState);
        }
        if order_tote == load.visit.tote || duration == SimDuration::ZERO {
            return Err(ConveyanceError::InvalidTransfer);
        }
        let floor = held.ready_at.max(held.pick_end).max(order_ready).max(not_before);
        let start = inner.operator.interval_after(floor, duration).ok_or(ConveyanceError::InvalidTransfer)?;
        let end = start.checked_add(duration).ok_or(ConveyanceError::InvalidTransfer)?;
        let pair = PairedTransfer {
            conveyor: self.id(),
            station: held.station,
            inventory_tote: load.visit.tote,
            order_tote,
            order_robot,
            start,
            end,
        };
        let id = inner.id();
        inner.operator.record(start, 1);
        inner.operator.record(end, -1);
        inner.transfers.insert(id, Transfer { presentation: presentation.handle.id, pair, committed: false });
        drop(inner);
        self.version().bump();
        Ok(TransferClaim { shared: self.shared.clone(), id, pair, presentation: presentation.clone(), active: true })
    }

    /// Reserve a returning tote and its collector before committing robot
    /// travel. The claim holds selection, not a premature capacity release.
    pub fn try_collect(
        &self,
        tote: &ConveyedTote,
        htm: &RetainedHtm,
        not_before: SimTime,
    ) -> Result<Option<CollectionClaim>, ConveyanceError> {
        same(&self.shared, &tote.handle.shared)?;
        same(&self.shared, &htm.handle.shared)?;
        let mut inner = self.shared.inner.lock().unwrap();
        let load = &inner.loads[&tote.handle.id];
        let TotePhase::Returning { ready_at } = load.visit.phase else {
            return Err(ConveyanceError::WrongState);
        };
        let retained = &inner.htms[&htm.handle.id];
        if retained.collected_at.is_some() {
            return Err(ConveyanceError::WrongState);
        }
        if load.collection.is_some() || retained.collection.is_some() {
            return Ok(None);
        }
        let at = not_before.max(ready_at).max(retained.boarded_at.ok_or(ConveyanceError::WrongState)?);
        let id = inner.id();
        inner.loads.get_mut(&tote.handle.id).unwrap().collection = Some(id);
        inner.htms.get_mut(&htm.handle.id).unwrap().collection = Some(id);
        drop(inner);
        self.version().bump();
        Ok(Some(CollectionClaim {
            shared: self.shared.clone(),
            id,
            at,
            tote: tote.clone(),
            htm: htm.clone(),
            active: true,
        }))
    }

    pub fn returning(&self) -> Vec<ReturningTote> {
        let inner = self.shared.inner.lock().unwrap();
        let mut returning: Vec<_> = inner
            .loads
            .iter()
            .filter_map(|(&id, load)| {
                let TotePhase::Returning { ready_at } = load.visit.phase else { return None };
                if load.collection.is_some() {
                    return None;
                }
                Some((ready_at, load.visit.tote, id, load.handle.upgrade()?))
            })
            .collect();
        returning.sort_by_key(|(at, tote, id, _)| (*at, *tote, *id));
        returning
            .into_iter()
            .map(|(ready_at, _, _, handle)| ReturningTote { tote: ConveyedTote { handle }, ready_at })
            .collect()
    }

    pub fn occupancy_at(&self, at: SimTime) -> Occupancy {
        let inner = self.shared.inner.lock().unwrap();
        Occupancy {
            conveyor: inner.belt.count_at(at),
            retained_htms: inner.retained.count_at(at),
            presentations: inner.slots.values().map(|slot| slot.count_at(at)).sum(),
            operator: inner.operator.count_at(at),
        }
    }

    /// Sorted transitions from the capacity calendars themselves, including
    /// provisional claims. Diagnostic integrations must not infer these from
    /// the separate visit and transfer histories they are checking.
    pub fn occupancy_change_times(&self) -> Vec<SimTime> {
        let inner = self.shared.inner.lock().unwrap();
        let mut times: Vec<_> = inner
            .belt
            .change_times()
            .chain(inner.retained.change_times())
            .chain(inner.operator.change_times())
            .chain(inner.slots.values().flat_map(Calendar::change_times))
            .collect();
        times.sort_unstable();
        times.dedup();
        times
    }

    pub fn visits(&self) -> Vec<ToteVisit> {
        let inner = self.shared.inner.lock().unwrap();
        let mut visits: Vec<_> = inner.loads.iter().map(|(&id, load)| (id, load.visit)).collect();
        visits.sort_by_key(|(id, _)| *id);
        visits.into_iter().map(|(_, visit)| visit).collect()
    }

    pub fn transfers(&self) -> Vec<PairedTransfer> {
        let inner = self.shared.inner.lock().unwrap();
        let mut transfers: Vec<_> = inner
            .transfers
            .iter()
            .filter(|(_, transfer)| transfer.committed)
            .map(|(&id, transfer)| (id, transfer.pair))
            .collect();
        transfers.sort_by_key(|(id, _)| *id);
        transfers.into_iter().map(|(_, pair)| pair).collect()
    }

    pub fn abandoned_custody(&self) -> Vec<CustodyLeak> {
        let mut leaks = self.shared.inner.lock().unwrap().abandoned.clone();
        leaks.sort();
        leaks
    }

    pub fn unfinished(&self) -> usize {
        let inner = self.shared.inner.lock().unwrap();
        inner.loads.values().filter(|load| !matches!(load.visit.phase, TotePhase::Collected { .. })).count()
            + inner.htms.values().filter(|htm| htm.collected_at.is_none()).count()
    }
}

fn same(a: &Arc<Shared>, b: &Arc<Shared>) -> Result<(), ConveyanceError> {
    if Arc::ptr_eq(a, b) { Ok(()) } else { Err(ConveyanceError::DifferentConveyance) }
}

fn no_earlier(at: SimTime, earliest: SimTime) -> Result<(), ConveyanceError> {
    if at < earliest { Err(ConveyanceError::TooEarly { at, earliest }) } else { Ok(()) }
}
