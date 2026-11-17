//! One opening's staging: within `feed_putaway_stations`, the task that
//! decides where a short putaway station's next empty comes from (a live
//! empty standing elsewhere, or an empty retrieved from the rack), delivers
//! it, and presents it for its fill. Once the last lot's fill is scheduled,
//! nothing will fill a staged empty: a staging whose source is not yet
//! accepted dissolves, and one that has staged its empty hands it to a
//! clearance once a deposit relay is reserved for its tote.
//!
//! TODO(overstuffed): three concerns share this file. Move the proposal's
//! types (`ProposedDirectEmpty`, `EmptySource`, `StageProposal`) into
//! `stage_empty/proposal.rs`, and the wait for a staged empty's fill, with
//! the drain, into `stage_empty/drain.rs`, its tests beside it.

use std::collections::VecDeque;
use std::fmt;

use desgraph::ZoneId;
use serde_json::json;
use tokio::select;

use super::{
    ArrivalQueue, DirectCandidate, PutawaySupply, PutawaySupplyDemand, StagingOpening, StagingTicket, direct_wins,
};
use crate::events::{EventJournal, HtmAssignmentEvent, ToteSource};
use crate::flows::ttp::task::clearance::{Attempt, ClearPresentation, ReturnRelays, Unparkable};
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry, ProposedHtmClaim};
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::station_service::StationService;
use crate::{
    flows::ttp::{handoff::*, task::*},
    model::relay::*,
    prelude::*,
};

// Direct supply's claim on the empty's stay: a productive claim starting
// at the planned departure. It outranks the stay's clearance when it departs
// no later than the clearance (simtime), and loses to it otherwise.
pub struct ProposedDirectEmpty {
    claim: ProposedHtmClaim,
    travel: SimDuration,
}

impl ProposedDirectEmpty {
    fn new(registry: AvailableHtmRegistry, candidate: DirectCandidate, clock: Mut<SimTime>, owner: ProposalId) -> Self {
        Self {
            claim: registry.propose_productive(&candidate.entry, owner, candidate.depart, clock),
            travel: candidate.travel,
        }
    }

    /// The claimed stay, the departure acceptance fixed (the later of the
    /// empty's availability and the opening, floored at the barrier), and
    /// the arrival that departure quotes.
    fn into_accepted(self) -> Result<(AvailableHtm, SimTime, SimTime), PlanError> {
        let (entry, depart) = self.claim.into_accepted()?;
        Ok((entry, depart, depart + self.travel))
    }
}

impl fmt::Display for ProposedDirectEmpty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "direct{{{}}}", self.claim)
    }
}

impl ProposedReservation for ProposedDirectEmpty {
    fn resolve_by(&self) -> Option<SimTime> {
        ProposedReservation::resolve_by(&self.claim)
    }

    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        ProposedReservation::ranked_competitors(&self.claim)
    }

    fn start_time(&self) -> Option<SimTime> {
        ProposedReservation::start_time(&self.claim)
    }

    fn poisoned(&self) -> bool {
        ProposedReservation::poisoned(&self.claim)
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        self.claim.accept()
    }
}

/// Where a `StageEmpty` proposes its station's next empty comes from, held
/// with the lots still to fill.
#[derive(Display)]
#[display("{source}")]
pub struct StageProposal {
    source: EmptySource,
    arrivals: ArrivalQueue,
}

#[derive(Display)]
pub enum EmptySource {
    #[display("{}", _0)]
    Direct(ProposedDirectEmpty),
    #[display("{}", _0)]
    FromRack(ProposedRetrieve),
    /// The arrival sequence was exhausted while the opening was searching;
    /// the task dissolves before this is ever accepted.
    #[display("supply complete")]
    Obsolete,
}

impl StageProposal {
    fn reservation(&mut self) -> Option<&mut dyn ProposedReservation> {
        match &mut self.source {
            EmptySource::Direct(direct) => Some(direct),
            EmptySource::FromRack(rack) => Some(rack),
            EmptySource::Obsolete => None,
        }
    }

    fn as_reservation(&self) -> Option<&dyn ProposedReservation> {
        match &self.source {
            EmptySource::Direct(direct) => Some(direct),
            EmptySource::FromRack(rack) => Some(rack),
            EmptySource::Obsolete => None,
        }
    }
}

impl Proposal for StageProposal {
    fn resolve_by(&self) -> Option<SimTime> {
        self.as_reservation().and_then(ProposedReservation::resolve_by)
    }

    fn ranked_competitors(&self) -> Option<Vec<ProposalId>> {
        self.as_reservation().and_then(ProposedReservation::ranked_competitors)
    }

    fn reservations(&mut self) -> Vec<&mut dyn ProposedReservation> {
        self.reservation().into_iter().collect()
    }

    fn start_time(&self) -> Option<SimTime> {
        self.as_reservation().and_then(ProposedReservation::start_time)
    }

    // Once the last lot's fill is scheduled, nothing will fill the empty this
    // would stage: the scheduler evicts the proposal, and its task dissolves.
    fn poisoned(&self) -> bool {
        self.arrivals.with(VecDeque::is_empty) || self.as_reservation().is_none_or(ProposedReservation::poisoned)
    }

    fn admissible(&self) -> bool {
        self.as_reservation().is_some_and(ProposedReservation::admissible)
    }
}

#[derive(Display)]
#[display("StageEmpty(station={:?})", opening.station_id)]
pub struct StageEmpty {
    arrivals: ArrivalQueue,
    clock: Mut<SimTime>,
    opening: StagingOpening,
    ticket: Option<StagingTicket>,
    supply: Arc<PutawaySupply>,
    direct_bias: SimOffset,
    relay: RelayService,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
}

impl StageEmpty {
    /// The staging of `opening`, holding its ticket, with the demand's
    /// handles.
    pub(super) fn new(
        demand: &PutawaySupplyDemand,
        opening: StagingOpening,
        ticket: StagingTicket,
        clock: Mut<SimTime>,
    ) -> Self {
        Self {
            arrivals: demand.arrivals.clone(),
            clock,
            opening,
            ticket: Some(ticket),
            supply: demand.supply.clone(),
            direct_bias: demand.direct_bias,
            relay: demand.relay.clone(),
            navigation: demand.navigation.clone(),
            bots: demand.bots.clone(),
            inventory: demand.inventory.clone(),
            available: demand.available.clone(),
            parking: demand.parking.clone(),
            events: demand.events.clone(),
            station_service: demand.station_service.clone(),
            station_arrivals: demand.station_arrivals.clone(),
            tripwires: demand.tripwires,
            delivery_max_wait_override: demand.delivery_max_wait_override,
        }
    }
}

#[async_trait(?Send)]
impl Task for StageEmpty {
    type Proposal = StageProposal;
    type Schedule = ();

    // The last lot's fill was scheduled before this opening's source was
    // accepted: nothing would ever fill what it staged.
    fn dissolve(&self) -> Option<()> {
        self.arrivals.with(|queue| queue.is_empty()).then_some(())
    }

    async fn propose(&self, owner: ProposalId) -> Result<StageProposal, PlanError> {
        let StagingOpening { station_cell, after, search_after, .. } = self.opening;
        let proposal = |source| StageProposal { source, arrivals: self.arrivals.clone() };
        loop {
            let arrivals_changed = notified(&self.arrivals);
            if self.arrivals.with(|queue| queue.is_empty()) {
                return Ok(proposal(EmptySource::Obsolete));
            }
            let fleet_seen = self.available.version().seen();
            let inventory_seen = self.inventory.version().seen();
            let relay_changed = notified(self.relay.signal());
            // The comparison lever decides on the settled present, so a rack
            // chain never starts from a decision made past the instant.
            let rack_arrival = if self.supply.rack_allowed_at(self.clock.value()) {
                self.supply
                    .quote_from_rack(station_cell, search_after, after)?
                    .map(|selection| self.supply.rack_arrival(&selection))
            } else {
                None
            };
            let direct = self.supply.best_direct(station_cell, after)?.filter(|candidate| {
                rack_arrival.is_none_or(|rack| direct_wins(candidate.arrival(), rack, after, self.direct_bias))
            });
            if let Some(candidate) = direct {
                return Ok(proposal(EmptySource::Direct(ProposedDirectEmpty::new(
                    self.available.clone(),
                    candidate,
                    self.clock.clone(),
                    owner,
                ))));
            }
            if rack_arrival.is_some() {
                select! {
                    biased;
                    retrieve = self.relay.source_retrieve(
                        ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
                        station_cell,
                        search_after,
                        SimTime::ZERO,
                        || Some(after),
                        self.supply.version(),
                        owner,
                    ) => return Ok(proposal(EmptySource::FromRack(retrieve?))),
                    _ = self.available.version().past(fleet_seen) => {}
                    _ = arrivals_changed => {}
                }
                continue;
            }
            select! {
                biased;
                _ = self.available.version().past(fleet_seen) => {}
                _ = self.inventory.version().past(inventory_seen) => {}
                _ = relay_changed => {}
                _ = arrivals_changed => {}
            }
        }
    }

    async fn schedule(&mut self, proposal: StageProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        let StagingOpening { station_id, .. } = self.opening;
        let ticket = self
            .ticket
            .take()
            .ok_or_else(|| PlanError::TaskGraphInvariant { context: "StageEmpty", detail: "scheduled twice".into() })?;

        let carrying = match proposal.source {
            EmptySource::Obsolete => {
                return Err(PlanError::TaskGraphInvariant {
                    context: "StageEmpty",
                    detail: "obsolete supply accepted".into(),
                });
            }
            EmptySource::Direct(direct) => {
                let (entry, ready, arrival) = direct.into_accepted()?;
                let robot = entry.robot();
                let tote_id = entry
                    .tote
                    .as_ref()
                    .ok_or_else(|| PlanError::TaskGraphInvariant {
                        context: "StageEmpty",
                        detail: format!("direct empty {robot:?} carries no tote"),
                    })?
                    .tote_id;
                let plan = StationArrivalPlan::new(
                    station_id,
                    StationArrivalSource::PutawayDirect,
                    self.opening.after,
                    arrival,
                )
                .with_htm_source_ready_at(entry.action_ready_at);
                let assignment = HtmAssignmentEvent {
                    robot_id: robot,
                    station_id,
                    tote_id: Some(tote_id),
                    source: StationArrivalSource::PutawayDirect,
                    relay_cell: None,
                    dispatch_time: ready,
                    quoted_arrival: arrival,
                };
                let inflight = ticket.open_span(ready);
                // An empty on an operator's presentation cell was claimed over
                // that stay's clearance, which takes it back if the delivery
                // cannot depart at the release. Every clearance off a station
                // other than a putaway station's returns its tote as Pick's,
                // and `best_direct` never takes a putaway station's empty.
                let clearance =
                    self.station_service.station_at_cell(entry.cell()).map(|station| (station, entry.clone()));
                // The delivery departs at the departure the claim was
                // accepted for: an empty standing idle since before the
                // opening leaves at the opening, not at its own availability.
                let handoff = HtmCarryingTote {
                    tote_id,
                    zone_id: ZoneId(0),
                    cell_reservation: entry.cell_reservation,
                    action_ready_at: ready,
                    orientation: entry.orientation,
                    station_arrival_plan: Some(plan),
                    presentation_id: self.events.next_presentation_id(),
                    source_relay_id: None,
                    deliver_from: None,
                };
                let deliver = HtmDeliver::new(
                    station_id,
                    inflight,
                    handoff,
                    ctx.barrier().clock(),
                    self.navigation.clone(),
                    self.bots.clone(),
                    self.inventory.clone(),
                    self.parking.clone(),
                    self.events.clone(),
                    self.station_service.clone(),
                    self.station_arrivals.clone(),
                    None,
                    self.tripwires,
                    self.delivery_max_wait_override,
                );
                let deliver = if clearance.is_some() {
                    deliver.superseding_clearance(assignment)
                } else {
                    self.events.emit(assignment);
                    deliver
                };
                match (ctx.add_task(deliver).await, clearance) {
                    (Ok(carrying), _) => carrying,
                    (Err(refused), Some((station, stay))) => {
                        return ClearPresentation::take_back(&ctx, stay, station, ToteSource::Pick, refused.departure);
                    }
                    (Err(refused), None) => return Err(refused.into()),
                }
            }
            EmptySource::FromRack(retrieve) => {
                let plan = StationArrivalPlan {
                    quoted_relay_pickup_at: Some(retrieve.selection.path_start_at),
                    quoted_htm_to_relay: Some(retrieve.selection.estimated_htm_to_relay),
                    ..StationArrivalPlan::new(
                        station_id,
                        StationArrivalSource::PutawayFromRack,
                        self.opening.after,
                        self.supply.rack_arrival(&retrieve.selection),
                    )
                };
                let (reservation, rack_slot) = retrieve.into_accepted()?;
                let inflight = ticket.open_span(reservation.start);
                let handoff = ctx
                    .add_task(VtmForward::new(
                        rack_slot,
                        reservation,
                        Some(plan),
                        self.relay.clone(),
                        self.inventory.clone(),
                        self.events.clone(),
                    ))
                    .await;
                let context = ctx.task_context::<HtmPickup>(json!({}))?;
                let pickup = HtmPickup::new(&context, handoff)?;
                context.finish()?;
                let carrying = ctx.add_task(pickup).await;
                ctx.add_task(HtmDeliver::new(
                    station_id,
                    inflight,
                    carrying,
                    ctx.barrier().clock(),
                    self.navigation.clone(),
                    self.bots.clone(),
                    self.inventory.clone(),
                    self.parking.clone(),
                    self.events.clone(),
                    self.station_service.clone(),
                    self.station_arrivals.clone(),
                    None,
                    self.tripwires,
                    self.delivery_max_wait_override,
                ))
                .await?
            }
        };

        let staged = ctx
            .add_task(Presentation {
                station_id,
                op: PresentationOp::Putaway,
                dwell: SimDuration::ZERO,
                handoff: Some(carrying),
                order_completion: None,
                bin_hold: None,
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                available: self.available.clone(),
                events: self.events.clone(),
            })
            .await;
        // A staged empty is supply awaiting its fill; the fill re-registers it
        // and `PutawayInduction` clears it from there. Once the last lot's
        // fill has been scheduled nothing will fill it, so it is drained from
        // here rather than left holding the station's cell.
        loop {
            let fleet_seen = self.available.version().seen();
            let arrivals_changed = notified(&self.arrivals);
            if !self.available.is_available(staged.availability_id()) {
                return Ok(());
            }
            if self.arrivals.with(|queue| queue.is_empty()) {
                return self.drain(staged, &ctx).await;
            }
            select! {
                _ = self.available.version().past(fleet_seen) => {}
                _ = arrivals_changed => {}
            }
        }
    }
}

impl StageEmpty {
    /// Hands `staged`, an empty that nothing will fill, to a clearance once a
    /// deposit relay is reserved for its tote, by the attempt a fill makes
    /// before it is committed. No departure is scheduled for the HTM until
    /// then (scheduler time): after each declined attempt this awaits
    /// `ReturnRelays::next_attempt_due`. The attempt that succeeds reserves
    /// the relay for a drive departing (simtime) at the stay's start, or at
    /// the barrier as that attempt is made if the barrier is past the start,
    /// and the HTM stands on the station's cell until that departure. The
    /// clearance's claim starts at that same departure, and the barrier
    /// passes no live claim's start before the claim is decided, so the HTM
    /// departs at the departure its relay was reserved for.
    async fn drain(&self, staged: AvailableHtm, ctx: &SchedCtx) -> Result<(), PlanError> {
        let context = ctx.task_context::<Self>(json!({}))?;
        let relays = ReturnRelays::new(&context)?;
        context.finish()?;
        let mut unparkable = Unparkable::default();
        loop {
            // Once another task takes the stay (scheduler time), the HTM is
            // that task's to move: the drain reserves no relay for it.
            if !self.available.is_available(staged.availability_id()) {
                return Ok(());
            }
            let depart = staged.action_ready_at.max(ctx.barrier().clock().value());
            let declined = match relays.attempt(&staged, depart, &mut unparkable, ctx).await? {
                Attempt::Reserved(return_relay) => {
                    let station = self.opening.station_id;
                    return ClearPresentation::spawn(ctx, staged, station, ToteSource::Putaway, return_relay);
                }
                Attempt::SquatterMoved => continue,
                Attempt::Declined(declined) => declined,
            };
            relays.next_attempt_due(declined, depart, SimDuration::ZERO, ctx).await?;
        }
    }
}

#[cfg(test)]
#[path = "stage_empty_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stage_empty_drain_tests.rs"]
mod drain_tests;
