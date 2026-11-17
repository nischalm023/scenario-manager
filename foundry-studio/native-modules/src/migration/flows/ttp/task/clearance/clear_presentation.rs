//! The clearance's claim: within `clearance`, the task that holds the
//! last-resort claim on the standing HTM's stay and, when that claim wins,
//! plans the HTM's next task.
//!
//! The claim starts when the operation ends (simtime), or, for a staged empty
//! that nothing will fill, at the departure its deposit relay was reserved
//! for. It carries a `Vacate` on the station cell's hold, as every
//! last-resort claim has; a need-by an entrant
//! places on that hold brings the claim's decision forward (scheduler time)
//! without moving that start. A productive claim on the same stay (direct
//! putaway supply, a pickup) that departs no later than the clearance's
//! start wins the contest and the clearance dissolves; one that would depart
//! later loses to it. The winning clearance takes the HTM home with its tote
//! (`ReturnClearedTote`) or parks it (`ParkClaimedHtm`). A direct supply that
//! won but could not depart at the release hands the HTM back
//! (`ClearPresentation::take_back`), and the clearance takes it home at that
//! departure, as if it had won. A putaway station's clearance carries the
//! deposit relay reserved before it was spawned, by the fill before it was
//! committed or by the drain of an empty nothing will fill: its return
//! deposits there, and a productive claim that wins the contest leaves the
//! clearance to give it back as it dissolves.

use std::future::pending;

use super::barrier_passes;
use super::return_cleared_tote::ReturnClearedTote;
use crate::error::planner::ClearanceBlock;
use crate::events::ToteSource;
use crate::flows::transport::{ParkClaimedHtm, ParkingOutcome};
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry, ProposedLastResort};
use crate::model::navigation::Navigation;
use crate::model::relay::RelayReservation;
use crate::prelude::*;
use desgraph::StationId;
use serde_json::json;
use tokio::select;

#[derive(Display)]
#[display("ClearPresentation(htm={:?} station={:?})", entry.robot(), station)]
pub struct ClearPresentation {
    entry: AvailableHtm,
    station: StationId,
    source: ToteSource,
    /// The deposit relay reserved for the tote's return before this
    /// clearance was spawned; given back when the clearance is dropped
    /// without taking the HTM home.
    return_relay: Option<RelayReservation<RackSlot>>,
    navigation: Arc<Navigation>,
    available: AvailableHtmRegistry,
    clock: Mut<SimTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearanceOutcome {
    /// This clearance planned the HTM's next task: its return home, or its
    /// drive to a parking bay.
    Cleared,
    /// Another claim on the stay was accepted first (scheduler time).
    Superseded,
}

impl ClearPresentation {
    pub fn new(
        entry: AvailableHtm,
        station: StationId,
        source: ToteSource,
        navigation: Arc<Navigation>,
        available: AvailableHtmRegistry,
        clock: Mut<SimTime>,
    ) -> Self {
        Self { entry, station, source, return_relay: None, navigation, available, clock }
    }

    /// Clear the stay `entry`, whose tote's return holds `return_relay`. The
    /// relay is given back if the clearance cannot be built.
    pub fn spawn(
        ctx: &SchedCtx,
        entry: AvailableHtm,
        station: StationId,
        source: ToteSource,
        return_relay: RelayReservation<RackSlot>,
    ) -> Result<(), PlanError> {
        let built = ctx.task_context::<Self>(json!({})).and_then(|context| {
            let task = Self::new(
                entry,
                station,
                source,
                context.state::<Navigation>()?,
                context.state::<AvailableHtmRegistry>()?.as_ref().clone(),
                ctx.barrier().clock(),
            );
            context.finish()?;
            Ok(task)
        });
        match built {
            Ok(mut task) => {
                task.return_relay = Some(return_relay);
                ctx.add_task(task);
                Ok(())
            }
            Err(error) => {
                return_relay.cancel();
                Err(error)
            }
        }
    }

    /// The clearance tried again after no bay took its HTM, which carries no
    /// tote and so holds no return relay.
    fn again(&self) -> Self {
        Self::new(
            self.entry.clone(),
            self.station,
            self.source,
            self.navigation.clone(),
            self.available.clone(),
            self.clock.clone(),
        )
    }

    /// Take back the stay `entry` that a direct supply claimed over this
    /// clearance and then could not depart on: no window of its drive could
    /// be committed at `depart`, the release (`DeliveryRefused`). The
    /// clearance takes the HTM home with its tote at that departure,
    /// uncontested, in the same scheduler-time instant as the refusal, so the
    /// barrier has not passed it; the HTM never re-enters the registry, so no
    /// claim can contest it again at this release. A refusal at any other
    /// departure is a defect in the direct supply, and stops the run with a
    /// task-graph invariant error. If no deposit relay can take the tote,
    /// `ReturnClearedTote`'s own typed stop stands. A hand-back never carries
    /// a return relay: only a putaway station reserves one, and a direct
    /// supply never takes a putaway station's stay.
    pub fn take_back(
        ctx: &SchedCtx,
        entry: AvailableHtm,
        station: StationId,
        source: ToteSource,
        depart: SimTime,
    ) -> Result<(), PlanError> {
        if entry.tote.is_none() {
            return Err(PlanError::TaskGraphInvariant {
                context: "ClearPresentation",
                detail: format!("a direct supply handed back {:?} without a tote", entry.robot()),
            });
        }
        if depart != entry.action_ready_at {
            return Err(PlanError::TaskGraphInvariant {
                context: "ClearPresentation",
                detail: format!(
                    "a direct supply handed back {:?} refused at {depart}, not at its release {}",
                    entry.robot(),
                    entry.action_ready_at
                ),
            });
        }
        let context = ctx.task_context::<ReturnClearedTote>(json!({}))?;
        let child = ReturnClearedTote::new(&context, entry, station, source, depart, ctx.barrier().clock())?;
        context.finish()?;
        ctx.add_task(child);
        Ok(())
    }
}

#[async_trait(?Send)]
impl Task for ClearPresentation {
    type Proposal = ProposedLastResort;
    type Schedule = ClearanceOutcome;

    fn dissolve(&self) -> Option<ClearanceOutcome> {
        (!self.available.is_available(self.entry.availability_id())).then_some(ClearanceOutcome::Superseded)
    }

    async fn propose(&self, owner: ProposalId) -> Result<ProposedLastResort, PlanError> {
        Ok(ProposedLastResort::clearance(&self.available, &self.navigation, &self.entry, owner, self.clock.clone()))
    }

    async fn schedule(&mut self, proposal: ProposedLastResort, ctx: SchedCtx) -> Result<ClearanceOutcome, PlanError> {
        let (entry, depart) = proposal.into_accepted()?;
        if entry.tote.is_some() {
            let context = ctx.task_context::<ReturnClearedTote>(json!({}))?;
            let child = ReturnClearedTote::new(&context, entry, self.station, self.source, depart, self.clock.clone())?
                .with_return_relay(self.return_relay.take());
            context.finish()?;
            ctx.add_task(child).await;
            return Ok(ClearanceOutcome::Cleared);
        }
        let context = ctx.task_context::<ParkClaimedHtm>(json!({}))?;
        let child = ParkClaimedHtm::new(&context, entry, depart)?;
        context.finish()?;
        if ctx.add_task(child).await == ParkingOutcome::Parked {
            return Ok(ClearanceOutcome::Cleared);
        }
        // No bay took the HTM, and the failure restored its stay. A decision
        // made early, in scheduler time, cannot declare the bays full for the
        // whole stay: a new clearance competes again once the barrier reaches
        // the planned departure, and after that whenever a path commit may
        // have freed a bay, until the barrier passes the departure. A failed
        // search changes the fleet (its take and restore) but never the
        // oracle, so clearances short of a bay cannot wake each other forever.
        if self.clock.value() < depart {
            ctx.barrier().wait_until(depart).await?;
        } else {
            let oracle_version = self.navigation.oracle.version();
            let oracle_seen = oracle_version.map(Version::seen);
            select! {
                biased;
                passed = barrier_passes(ctx.barrier(), depart) => passed?,
                _ = async {
                    match oracle_version.zip(oracle_seen) {
                        Some((version, seen)) => version.past(seen).await,
                        None => pending().await,
                    }
                } => {}
            }
            if self.clock.value() > depart {
                if !self.available.is_available(self.entry.availability_id()) {
                    return Ok(ClearanceOutcome::Superseded);
                }
                return Err(PlanError::ClearanceCannotDepart {
                    station: self.station,
                    robot: self.entry.robot(),
                    departure: depart,
                    cause: ClearanceBlock::NoParkingBay,
                });
            }
        }
        Ok(ctx.add_task(self.again()).await)
    }
}

impl Drop for ClearPresentation {
    fn drop(&mut self) {
        if let Some(unused) = self.return_relay.take() {
            unused.cancel();
        }
    }
}

#[cfg(test)]
#[path = "clear_presentation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "clear_presentation_give_back_tests.rs"]
mod give_back_tests;
