//! A return's deposit relay, within `clearance`: the search for the relay
//! that takes a presented tote home, run by a putaway station before its
//! tote's clearance is spawned (a fill before it is committed, the drain of a
//! staged empty that nothing will fill), and by a clearance's return that
//! holds no reservation. One attempt reserves a relay in the racking that
//! owns the tote, for a drive departing the station's cell at a given
//! departure (simtime), and parks idle empty HTMs off the access cells of the
//! only relays that could take it; a declined attempt says what to await
//! (scheduler time) before the next.

use std::future::pending;
use std::pin::Pin;

use crate::error::ReservationError;
use crate::events::EventJournal;
use crate::flows::transport::{Parking, ParkingOutcome};
use crate::model::ParkingRegistry;
use crate::model::ToteDesignation;
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry};
use crate::model::navigation::Navigation;
use crate::model::relay::RelayReservation;
use crate::prelude::*;
use crate::task::TaskContext;
use desgraph::CellId;
use tokio::select;

use super::{Unparkable, barrier_passes_unheld};

/// The services a return's relay search reads, and the return's drive needs
/// after it.
pub struct ReturnRelays {
    pub(super) navigation: Arc<Navigation>,
    pub(super) relay: RelayService,
    pub(super) bots: BotService,
    pub(super) inventory: InventoryService,
    pub(super) available: AvailableHtmRegistry,
    pub(super) parking: Option<ParkingRegistry>,
    pub(super) events: EventJournal,
}

/// What one attempt came to.
pub enum Attempt {
    Reserved(RelayReservation<RackSlot>),
    /// The registry no longer holds the stay of an idle squatter on an
    /// access cell the tote's relay needs (a parking or a productive claim
    /// took it): attempt again without awaiting.
    SquatterMoved,
    Declined(Declined),
}

/// A declined attempt: the relay service's reason, and what it read.
pub struct Declined {
    pub(super) reason: ReservationError,
    pub(super) seen: Seen,
}

/// What a declined attempt read: once any of it changes, the next attempt
/// may be accepted.
pub struct Seen {
    relay_changed: Pin<Box<dyn Future<Output = ()>>>,
    inventory_seen: u64,
    oracle_seen: Option<u64>,
    fleet_seen: u64,
}

impl ReturnRelays {
    pub fn new(context: &TaskContext) -> Result<Self, PlanError> {
        let parking = context.state::<ParkingRegistry>()?;
        Ok(Self {
            navigation: context.state::<Navigation>()?,
            relay: context.state::<RelayService>()?.as_ref().clone(),
            bots: context.state::<BotService>()?.as_ref().clone(),
            inventory: context.state::<InventoryService>()?.as_ref().clone(),
            available: context.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: context.state::<EventJournal>()?.as_ref().clone(),
        })
    }

    /// Reserve a deposit relay for the tote `htm` carries off its cell at
    /// `depart`. A tote goes home to the steel of its own pool: an emptied
    /// order tote to the row close-out draws empties from, not to stock.
    pub async fn attempt(
        &self,
        htm: &AvailableHtm,
        depart: SimTime,
        unparkable: &mut Unparkable,
        ctx: &SchedCtx,
    ) -> Result<Attempt, PlanError> {
        let (robot, from, orientation) = (htm.robot(), htm.cell(), htm.orientation);
        let tote = htm.tote.as_ref().ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "ReturnRelays",
            detail: format!("{robot:?} carries no tote to return"),
        })?;
        let designation = self.inventory.tote(tote.tote_id)?.designation();
        let inventory_seen = self.inventory.version().seen();
        let relay_changed = Box::pin(notified(self.relay.signal()));
        let oracle_seen = self.navigation.oracle.version().map(Version::seen);
        unparkable.as_of(oracle_seen);
        let reserved = match designation {
            ToteDesignation::Inventory => {
                self.relay.reserve_relay_for_deposit(from, orientation, depart, robot, None).await
            }
            ToteDesignation::Order => {
                self.relay.reserve_depot_relay_for_deposit(from, orientation, depart, robot, None).await
            }
        };
        let reason = match reserved {
            Ok(deposit) => return Ok(Attempt::Reserved(deposit)),
            Err(
                reason @ (ReservationError::AccessHeld { .. }
                | ReservationError::NoCandidates { .. }
                | ReservationError::BeyondAccessHorizon { .. }
                | ReservationError::HeadroomExhausted { .. }),
            ) => reason,
            Err(error) => return Err(error.into()),
        };
        let held = match &reason {
            ReservationError::AccessHeld { held, .. } => held.as_slice(),
            _ => &[],
        };
        if self.park_a_squatter(held, unparkable, depart, ctx).await? {
            return Ok(Attempt::SquatterMoved);
        }
        // Observed after the parking attempts: a failed one restores its
        // HTM, and that restoration is not the change a search awaits.
        let fleet_seen = self.available.version().seen();
        let seen = Seen { relay_changed, inventory_seen, oracle_seen, fleet_seen };
        Ok(Attempt::Declined(Declined { reason, seen }))
    }

    /// Awaits (scheduler time) what may let a putaway station's next attempt
    /// be accepted after `declined`, an attempt for a drive departing
    /// (simtime) at `depart`, when `lead` of work on the tote ends (a fill's
    /// length; zero for a drain). A change to what the attempt read
    /// (relays, inventory, fleet, paths) wakes it; beyond that, by case:
    /// - relays open beyond the access horizon (beside held access cells or
    ///   not): the start of the earliest work whose departure brings the first
    ///   of them within its horizon is held as an advance target, so the
    ///   barrier moves there with no other work moving it;
    /// - no relay finalized, or admissions unresolved: the relay service
    ///   changes once one is;
    /// - relays' access cells held through the horizon, by holds whose ends are
    ///   scheduled or by idle squatters no bay takes, with no opening named:
    ///   the barrier passing the work's start, after which the next attempt's
    ///   drive departs later. A squatter's parking is decided (scheduler time)
    ///   when the barrier reaches the earlier of `depart` and the start of the
    ///   squatter's stay, or at the barrier if it is already past that, so it
    ///   can bring the barrier there a single time: it is not tried again while
    ///   no path commits and its departure is the same. Beyond that only other
    ///   work moves the barrier, and with none the deadlock alarm reports the
    ///   wait.
    pub async fn next_attempt_due(
        &self,
        declined: Declined,
        depart: SimTime,
        lead: SimDuration,
        ctx: &SchedCtx,
    ) -> Result<(), PlanError> {
        let retry_start = declined.reason.earliest_admitted_departure().map(|depart| depart - lead);
        select! {
            biased;
            _ = self.changed(declined.seen) => {}
            _ = barrier_passes_unheld(ctx.barrier().clock(), depart - lead) => {}
            reached = async {
                match retry_start {
                    Some(start) => ctx.barrier().wait_until(start).await,
                    None => pending().await,
                }
            } => reached?,
        }
        Ok(())
    }

    /// Resolves once something a declined attempt read has changed.
    pub(super) async fn changed(&self, seen: Seen) {
        let Seen { relay_changed, inventory_seen, oracle_seen, fleet_seen } = seen;
        let oracle_version = self.navigation.oracle.version();
        select! {
            _ = self.available.version().past(fleet_seen) => {}
            _ = self.inventory.version().past(inventory_seen) => {}
            _ = relay_changed => {}
            _ = async {
                match oracle_version.zip(oracle_seen) {
                    Some((version, seen)) => version.past(seen).await,
                    None => pending().await,
                }
            } => {}
        }
    }

    /// Robots hold the access cells (`held`, from the declined deposit) of
    /// the only relays that could take the tote. When one of those holders is
    /// an idle empty HTM, a `Parking` task moves it (or a productive claim
    /// takes it first), as `HtmStore` does for its own relay, decided before
    /// the barrier passes `depart`, however late the holder's own stay
    /// starts. A parking that finds no bay comes back at that decision, so
    /// the next holder is tried before the barrier passes `depart`; its stay
    /// joins `unparkable` with the parking's departure, and a stay whose
    /// parking would depart at the same simtime as its failed parking was
    /// planned to depart is passed over.
    /// True when the registry no longer holds some holder's stay.
    async fn park_a_squatter(
        &self,
        held: &[(CellId, RobotId)],
        unparkable: &mut Unparkable,
        depart: SimTime,
        ctx: &SchedCtx,
    ) -> Result<bool, PlanError> {
        let Some(parking) = &self.parking else { return Ok(false) };
        for &(cell, robot) in held {
            let Some(squatter) = self.available.squatter(cell, robot) else { continue };
            let Some(parking_departure) = unparkable.untried_departure(&squatter, ctx.barrier().clock().value()) else {
                continue;
            };
            let stay = squatter.availability_id();
            let hold = squatter.cell_reservation.clone();
            let task = Parking::new(
                squatter,
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                self.available.clone(),
                Some(parking.clone()),
                self.events.clone(),
                ctx.barrier().clock(),
            )?;
            let parked = ctx.add_task(task.without_retry());
            let outcome = select! {
                biased;
                outcome = parked => outcome,
                never = self.navigation.cells.keep_need_by(&hold, depart) => match never {},
            };
            if outcome != ParkingOutcome::Unavailable {
                return Ok(true);
            }
            unparkable.insert(stay, parking_departure);
        }
        Ok(false)
    }
}
