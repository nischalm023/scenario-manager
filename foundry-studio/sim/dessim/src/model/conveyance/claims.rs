//! Revocable claims. Dropping an uncommitted claim restores the calendars;
//! committing it transfers custody to explicit handles or a fixed work interval.

use std::sync::Arc;

use desgraph::StationId;

use crate::model::conveyor::ConveyorId;
use crate::model::{RobotId, SimTime, ToteId};

use super::custody::{HtmHandle, PresentationHandle, ToteHandle};
use super::{
    ConveyanceError, ConveyedTote, InventoryPresentation, PairedTransfer, RetainedHtm, Shared, TotePhase, no_earlier,
};

pub struct Admission {
    pub(super) shared: Arc<Shared>,
    pub(super) id: u64,
    pub(super) at: SimTime,
    pub(super) active: bool,
}

impl Admission {
    pub fn at(&self) -> SimTime {
        self.at
    }

    pub fn tote(&self) -> ToteId {
        self.shared.inner.lock().unwrap().loads[&self.id].visit.tote
    }

    pub fn robot(&self) -> RobotId {
        self.shared.inner.lock().unwrap().htms[&self.id].robot
    }

    pub fn conveyor_id(&self) -> ConveyorId {
        self.shared.id
    }

    pub fn validate(&self, at: SimTime) -> Result<(), ConveyanceError> {
        if !self.active {
            return Err(ConveyanceError::SpentClaim);
        }
        no_earlier(at, self.at)
    }

    pub fn board(&mut self, at: SimTime) -> Result<(ConveyedTote, RetainedHtm), ConveyanceError> {
        self.validate(at)?;
        let mut inner = self.shared.inner.lock().unwrap();
        let tote = ConveyedTote { handle: Arc::new(ToteHandle { shared: self.shared.clone(), id: self.id }) };
        let htm = RetainedHtm { handle: Arc::new(HtmHandle { shared: self.shared.clone(), id: self.id }) };
        // Delaying an open-ended claim only shrinks its reserved interval.
        // No successor can have released this still-provisional admission.
        inner.belt.record(self.at, -1);
        inner.belt.record(at, 1);
        inner.retained.record(self.at, -1);
        inner.retained.record(at, 1);
        let load = inner.loads.get_mut(&self.id).unwrap();
        load.visit.boarded_at = Some(at);
        load.visit.phase = TotePhase::AwaitingPresentation;
        load.handle = Arc::downgrade(&tote.handle);
        inner.htms.get_mut(&self.id).unwrap().boarded_at = Some(at);
        self.active = false;
        drop(inner);
        self.shared.version.bump();
        Ok((tote, htm))
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut inner = self.shared.inner.lock().unwrap();
        inner.loads.remove(&self.id);
        inner.htms.remove(&self.id);
        inner.belt.record(self.at, -1);
        inner.retained.record(self.at, -1);
        drop(inner);
        self.shared.version.bump();
    }
}

pub struct PresentationClaim {
    pub(super) shared: Arc<Shared>,
    pub(super) id: u64,
    pub(super) at: SimTime,
    pub(super) tote: ConveyedTote,
    pub(super) active: bool,
}

impl PresentationClaim {
    pub fn station(&self) -> StationId {
        self.shared.inner.lock().unwrap().presentations[&self.id].station
    }

    pub fn at(&self) -> SimTime {
        self.at
    }

    pub fn present(&mut self, actual_ready: SimTime) -> Result<InventoryPresentation, ConveyanceError> {
        if !self.active {
            return Err(ConveyanceError::SpentClaim);
        }
        no_earlier(actual_ready, self.at)?;
        let mut inner = self.shared.inner.lock().unwrap();
        let station = inner.presentations[&self.id].station;
        let slot = inner.slots.get_mut(&station).unwrap();
        slot.record(self.at, -1);
        slot.record(actual_ready, 1);
        let handle = Arc::new(PresentationHandle { shared: self.shared.clone(), id: self.id, tote: self.tote.clone() });
        let held = inner.presentations.get_mut(&self.id).unwrap();
        held.committed = true;
        held.ready_at = actual_ready;
        held.pick_end = actual_ready;
        inner.loads.get_mut(&self.tote.handle.id).unwrap().visit.phase =
            TotePhase::Presented { station, ready_at: actual_ready };
        self.active = false;
        drop(inner);
        self.shared.version.bump();
        Ok(InventoryPresentation { handle })
    }
}

impl Drop for PresentationClaim {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut inner = self.shared.inner.lock().unwrap();
        let held = inner.presentations.remove(&self.id).unwrap();
        inner.slots.get_mut(&held.station).unwrap().record(self.at, -1);
        let load = inner.loads.get_mut(&held.load).unwrap();
        load.visit.phase = held.prior_phase;
        load.presentation = None;
        drop(inner);
        self.shared.version.bump();
    }
}

pub struct TransferClaim {
    pub(super) shared: Arc<Shared>,
    pub(super) id: u64,
    pub(super) pair: PairedTransfer,
    pub(super) presentation: InventoryPresentation,
    pub(super) active: bool,
}

impl TransferClaim {
    pub fn interval(&self) -> PairedTransfer {
        self.pair
    }

    pub fn commit(&mut self) -> Result<PairedTransfer, ConveyanceError> {
        if !self.active {
            return Err(ConveyanceError::SpentClaim);
        }
        let mut inner = self.shared.inner.lock().unwrap();
        let held = &inner.presentations[&self.presentation.handle.id];
        let load = &inner.loads[&held.load];
        if !held.committed || held.cleared_at.is_some() || load.presentation != Some(self.presentation.handle.id) {
            return Err(ConveyanceError::WrongState);
        }
        no_earlier(self.pair.start, held.ready_at)?;
        inner.transfers.get_mut(&self.id).unwrap().committed = true;
        let held = inner.presentations.get_mut(&self.presentation.handle.id).unwrap();
        held.pick_end = held.pick_end.max(self.pair.end);
        self.active = false;
        drop(inner);
        self.shared.version.bump();
        Ok(self.pair)
    }
}

impl Drop for TransferClaim {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut inner = self.shared.inner.lock().unwrap();
        inner.transfers.remove(&self.id);
        inner.operator.record(self.pair.start, -1);
        inner.operator.record(self.pair.end, 1);
        drop(inner);
        self.shared.version.bump();
    }
}

/// Exclusive custody selection while a retained HTM travels to collection.
/// Cancellation returns both candidates to their prior state; only commit
/// releases their physical capacity at the actual collection time.
pub struct CollectionClaim {
    pub(super) shared: Arc<Shared>,
    pub(super) id: u64,
    pub(super) at: SimTime,
    pub(super) tote: ConveyedTote,
    pub(super) htm: RetainedHtm,
    pub(super) active: bool,
}

impl CollectionClaim {
    pub fn at(&self) -> SimTime {
        self.at
    }

    pub fn tote(&self) -> ToteId {
        self.tote.tote()
    }

    pub fn robot(&self) -> RobotId {
        self.htm.robot()
    }

    pub fn conveyor_id(&self) -> ConveyorId {
        self.shared.id
    }

    pub fn validate(&self, at: SimTime) -> Result<(), ConveyanceError> {
        if !self.active {
            return Err(ConveyanceError::SpentClaim);
        }
        no_earlier(at, self.at)?;
        let inner = self.shared.inner.lock().unwrap();
        let load = &inner.loads[&self.tote.handle.id];
        let htm = &inner.htms[&self.htm.handle.id];
        if !matches!(load.visit.phase, TotePhase::Returning { .. })
            || load.collection != Some(self.id)
            || htm.collection != Some(self.id)
            || htm.collected_at.is_some()
        {
            return Err(ConveyanceError::WrongState);
        }
        Ok(())
    }

    pub fn commit(&mut self, actual_collection_at: SimTime) -> Result<(), ConveyanceError> {
        self.validate(actual_collection_at)?;
        self.tote.collect_claimed(&self.htm, actual_collection_at, Some(self.id))?;
        self.active = false;
        Ok(())
    }
}

impl Drop for CollectionClaim {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut inner = self.shared.inner.lock().unwrap();
        let load = inner.loads.get_mut(&self.tote.handle.id).unwrap();
        debug_assert_eq!(load.collection, Some(self.id));
        load.collection = None;
        let htm = inner.htms.get_mut(&self.htm.handle.id).unwrap();
        debug_assert_eq!(htm.collection, Some(self.id));
        htm.collection = None;
        drop(inner);
        self.shared.version.bump();
    }
}
