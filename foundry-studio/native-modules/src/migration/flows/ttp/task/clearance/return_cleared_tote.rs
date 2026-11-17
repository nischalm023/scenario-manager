//! A clearance's return: within `clearance`, the step that sends an HTM a
//! clearance has claimed, still carrying its tote, back to rack storage.
//!
//! It drives the HTM off the station's cell at the clearance's accepted
//! departure to a deposit relay in the racking that owns the tote, and hands
//! it to the ordinary `ReturnTote` chain. The relay is the one a putaway
//! station reserved before the clearance was spawned, when the clearance
//! carries one, unless, when the return is decided, another robot holds that
//! relay's access cell with no departure scheduled and a search reserves
//! another relay then. Otherwise the return searches (`ReturnRelays`), and
//! attempts again after a declined search while the barrier has not passed
//! that departure. Once it has, any relay reserved would have the HTM leave
//! the cell later, and the run stops, naming the relay service's last reason
//! or the missed departure.

use super::return_relay::{Attempt, ReturnRelays};
use super::{Unparkable, barrier_passes, release_missed};
use crate::error::planner::ClearanceBlock;
use crate::events::{DivingBoardSlotWaitEvent, ToteSource};
use crate::flows::ttp::task::ReturnTote;
use crate::model::htm::registry::AvailableHtm;
use crate::model::relay::RelayReservation;
use crate::prelude::*;
use crate::task::TaskContext;
use desgraph::{RelayId, StationId};
use tokio::select;

#[derive(Display)]
#[display("ReturnClearedTote(htm={:?} stn={:?})", entry.robot(), station)]
pub struct ReturnClearedTote {
    entry: AvailableHtm,
    station: StationId,
    source: ToteSource,
    depart: SimTime,
    /// The deposit relay reserved for this return before its clearance was
    /// spawned; given back if the return is dropped unused.
    return_relay: Option<RelayReservation<RackSlot>>,
    relays: ReturnRelays,
    clock: Mut<SimTime>,
}

impl ReturnClearedTote {
    pub fn new(
        context: &TaskContext,
        entry: AvailableHtm,
        station: StationId,
        source: ToteSource,
        depart: SimTime,
        clock: Mut<SimTime>,
    ) -> Result<Self, PlanError> {
        Ok(Self { entry, station, source, depart, return_relay: None, relays: ReturnRelays::new(context)?, clock })
    }

    /// Deposit at `return_relay`, reserved before the clearance was spawned,
    /// instead of searching for a relay in the scheduler step that decides
    /// the departure.
    pub fn with_return_relay(mut self, return_relay: Option<RelayReservation<RackSlot>>) -> Self {
        self.return_relay = return_relay;
        self
    }

    /// The relay reserved before the clearance was spawned, unless, at this
    /// decision (scheduler time), another robot holds its access cell with no
    /// departure scheduled and a search reserves another relay: then that
    /// one, and `reserved` is given back. A declined search keeps `reserved`,
    /// whose store then awaits (scheduler time) the holder's departure being
    /// scheduled before it commits its path, until the barrier passes the
    /// departure (`HtmStore`).
    async fn reserved_unless_blocked(
        &self,
        reserved: RelayReservation<RackSlot>,
        ctx: &SchedCtx,
    ) -> Result<RelayReservation<RackSlot>, PlanError> {
        if !self.held_by_another(reserved.relay_id) {
            return Ok(reserved);
        }
        let mut unparkable = Unparkable::default();
        loop {
            // A squatter's parking awaited below can take the barrier past
            // the departure: nothing is reserved behind it.
            if self.clock.value() > self.depart {
                let cause = self.missed_cause(reserved.relay_id);
                reserved.cancel();
                return Err(self.cannot_depart(cause));
            }
            match self.relays.attempt(&self.entry, self.depart, &mut unparkable, ctx).await {
                Ok(Attempt::Reserved(other)) => {
                    reserved.cancel();
                    return Ok(other);
                }
                Ok(Attempt::SquatterMoved) => continue,
                Ok(Attempt::Declined(_)) => return Ok(reserved),
                Err(error) => {
                    reserved.cancel();
                    return Err(error);
                }
            }
        }
    }

    async fn search_deposit(&self, ctx: &SchedCtx) -> Result<RelayReservation<RackSlot>, PlanError> {
        let mut unparkable = Unparkable::default();
        let mut declined_for = None;
        loop {
            if self.clock.value() > self.depart {
                let cause = declined_for.map_or(ClearanceBlock::DepartureMissed, ClearanceBlock::NoDepositRelay);
                return Err(self.cannot_depart(cause));
            }
            let declined = match self.relays.attempt(&self.entry, self.depart, &mut unparkable, ctx).await? {
                Attempt::Reserved(deposit) => return Ok(deposit),
                Attempt::SquatterMoved => {
                    declined_for = None;
                    continue;
                }
                Attempt::Declined(declined) => declined,
            };
            declined_for = Some(declined.reason);
            select! {
                _ = self.relays.changed(declined.seen) => {}
                passed = barrier_passes(ctx.barrier(), self.depart) => passed?,
            }
        }
    }

    /// Whether a robot other than this return's holds `relay`'s access cell
    /// with no departure scheduled.
    fn held_by_another(&self, relay: RelayId) -> bool {
        self.relays.relay.htm_access_holder(relay).is_some_and(|holder| holder != self.entry.robot())
    }

    /// Why the barrier passed the departure with `relay` reserved and no
    /// drive committed.
    fn missed_cause(&self, relay: RelayId) -> ClearanceBlock {
        release_missed(self.relays.relay.htm_access_holder(relay), self.entry.robot())
    }

    fn cannot_depart(&self, cause: ClearanceBlock) -> PlanError {
        PlanError::ClearanceCannotDepart {
            station: self.station,
            robot: self.entry.robot(),
            departure: self.depart,
            cause,
        }
    }
}

impl Drop for ReturnClearedTote {
    fn drop(&mut self) {
        if let Some(unused) = self.return_relay.take() {
            unused.cancel();
        }
    }
}

#[async_trait(?Send)]
impl Task for ReturnClearedTote {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let carried = self.entry.tote.clone().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "ReturnClearedTote",
            detail: format!("{:?} was cleared as a return but carries no tote", self.entry.robot()),
        })?;
        let deposit = match self.return_relay.take() {
            Some(reserved) => self.reserved_unless_blocked(reserved, &ctx).await?,
            None => self.search_deposit(&ctx).await?,
        };
        // A declined search for another relay can await parkings past the
        // departure: nothing is announced behind the barrier.
        if self.clock.value() > self.depart {
            let cause = self.missed_cause(deposit.relay_id);
            deposit.cancel();
            return Err(self.cannot_depart(cause));
        }
        let relays = &self.relays;
        relays.events.emit(DivingBoardSlotWaitEvent {
            relay_id: deposit.relay_id,
            wait_ms: deposit.slot_wait.as_ms(),
            sim_time_ms: deposit.start.as_ms(),
        });
        let entry = &self.entry;
        ctx.add_task(ReturnTote {
            station_id: self.station,
            robot_id: entry.robot(),
            tote_id: carried.tote_id,
            predecessor: entry.cell_reservation.clone(),
            action_ready: self.depart,
            departs_by: Some(self.depart),
            orientation: entry.orientation,
            deposit: Some(deposit),
            presentation_id: carried.presentation_id,
            tote_source: self.source,
            navigation: relays.navigation.clone(),
            relay: relays.relay.clone(),
            bots: relays.bots.clone(),
            inventory: relays.inventory.clone(),
            available: relays.available.clone(),
            parking: relays.parking.clone(),
            events: relays.events.clone(),
        })
        .await;
        Ok(())
    }
}

#[cfg(test)]
#[path = "return_cleared_tote_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "return_cleared_tote_reserved_tests.rs"]
mod reserved_tests;
