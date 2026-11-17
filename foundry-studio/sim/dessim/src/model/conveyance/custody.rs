//! Committed custody survives task cancellation. Only explicit clearance and
//! collection bound its occupancy; a lost final handle is recorded for auditing.

use std::sync::Arc;

use desgraph::StationId;

use crate::model::{RobotId, SimTime, ToteId};

use super::{ConveyanceError, CustodyLeak, Shared, TotePhase, no_earlier, same};

#[derive(Clone)]
pub struct ConveyedTote {
    pub(super) handle: Arc<ToteHandle>,
}

pub(super) struct ToteHandle {
    pub shared: Arc<Shared>,
    pub id: u64,
}

impl ConveyedTote {
    pub fn tote(&self) -> ToteId {
        self.handle.shared.inner.lock().unwrap().loads[&self.handle.id].visit.tote
    }

    pub fn phase(&self) -> TotePhase {
        self.handle.shared.inner.lock().unwrap().loads[&self.handle.id].visit.phase
    }

    pub fn collect(&self, htm: &RetainedHtm, at: SimTime) -> Result<(), ConveyanceError> {
        self.collect_claimed(htm, at, None)
    }

    pub(super) fn collect_claimed(
        &self,
        htm: &RetainedHtm,
        at: SimTime,
        claim: Option<u64>,
    ) -> Result<(), ConveyanceError> {
        same(&self.handle.shared, &htm.handle.shared)?;
        let mut inner = self.handle.shared.inner.lock().unwrap();
        let load = &inner.loads[&self.handle.id];
        let TotePhase::Returning { ready_at } = load.visit.phase else {
            return Err(ConveyanceError::WrongState);
        };
        let retained = &inner.htms[&htm.handle.id];
        if retained.collected_at.is_some() || load.collection != claim || retained.collection != claim {
            return Err(ConveyanceError::WrongState);
        }
        no_earlier(at, ready_at.max(retained.boarded_at.ok_or(ConveyanceError::WrongState)?))?;
        let robot = retained.robot;
        inner.loads.get_mut(&self.handle.id).unwrap().visit.phase = TotePhase::Collected { at, robot };
        inner.loads.get_mut(&self.handle.id).unwrap().collection = None;
        inner.htms.get_mut(&htm.handle.id).unwrap().collected_at = Some(at);
        inner.htms.get_mut(&htm.handle.id).unwrap().collection = None;
        inner.belt.record(at, -1);
        inner.retained.record(at, -1);
        drop(inner);
        self.handle.shared.version.bump();
        Ok(())
    }
}

impl Drop for ToteHandle {
    fn drop(&mut self) {
        let mut inner = self.shared.inner.lock().unwrap();
        let load = &inner.loads[&self.id];
        if matches!(load.visit.phase, TotePhase::Collected { .. }) {
            return;
        }
        let tote = load.visit.tote;
        inner.abandoned.push(CustodyLeak::InventoryTote(tote));
        drop(inner);
        self.shared.version.bump();
    }
}

#[derive(Clone)]
pub struct RetainedHtm {
    pub(super) handle: Arc<HtmHandle>,
}

pub(super) struct HtmHandle {
    pub shared: Arc<Shared>,
    pub id: u64,
}

impl RetainedHtm {
    pub fn robot(&self) -> RobotId {
        self.handle.shared.inner.lock().unwrap().htms[&self.handle.id].robot
    }
}

impl Drop for HtmHandle {
    fn drop(&mut self) {
        let mut inner = self.shared.inner.lock().unwrap();
        let htm = &inner.htms[&self.id];
        if htm.collected_at.is_some() {
            return;
        }
        let robot = htm.robot;
        inner.abandoned.push(CustodyLeak::RetainedHtm(robot));
        drop(inner);
        self.shared.version.bump();
    }
}

#[derive(Clone)]
pub struct InventoryPresentation {
    pub(super) handle: Arc<PresentationHandle>,
}

pub(super) struct PresentationHandle {
    pub shared: Arc<Shared>,
    pub id: u64,
    pub tote: ConveyedTote,
}

impl InventoryPresentation {
    pub fn tote(&self) -> ToteId {
        self.handle.tote.tote()
    }

    pub fn station(&self) -> StationId {
        self.handle.shared.inner.lock().unwrap().presentations[&self.handle.id].station
    }

    pub fn ready_at(&self) -> SimTime {
        self.handle.shared.inner.lock().unwrap().presentations[&self.handle.id].ready_at
    }

    pub fn clear(&self, cleared_at: SimTime, return_ready: SimTime) -> Result<(), ConveyanceError> {
        let mut inner = self.handle.shared.inner.lock().unwrap();
        let held = &inner.presentations[&self.handle.id];
        let load = &inner.loads[&held.load];
        if !held.committed || held.cleared_at.is_some() || load.presentation != Some(self.handle.id) {
            return Err(ConveyanceError::WrongState);
        }
        if inner.transfers.values().any(|transfer| transfer.presentation == self.handle.id && !transfer.committed) {
            return Err(ConveyanceError::PendingTransfer);
        }
        no_earlier(cleared_at, held.pick_end.max(held.ready_at))?;
        if return_ready < cleared_at {
            return Err(ConveyanceError::InvalidReturnTime);
        }
        let (station, load_id) = (held.station, held.load);
        inner.presentations.get_mut(&self.handle.id).unwrap().cleared_at = Some(cleared_at);
        inner.slots.get_mut(&station).unwrap().record(cleared_at, -1);
        let load = inner.loads.get_mut(&load_id).unwrap();
        load.visit.phase = TotePhase::Returning { ready_at: return_ready };
        load.presentation = None;
        drop(inner);
        self.handle.shared.version.bump();
        Ok(())
    }
}

impl Drop for PresentationHandle {
    fn drop(&mut self) {
        let mut inner = self.shared.inner.lock().unwrap();
        let held = &inner.presentations[&self.id];
        if held.cleared_at.is_some() {
            return;
        }
        let station = held.station;
        let tote = inner.loads[&held.load].visit.tote;
        inner.abandoned.push(CustodyLeak::Presentation { tote, station });
        drop(inner);
        self.shared.version.bump();
    }
}
