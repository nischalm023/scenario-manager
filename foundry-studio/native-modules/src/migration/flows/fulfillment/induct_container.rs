//! Acquire a newly supplied container at a floor cell. The arriving HTM holds
//! the source through creation and pickup, so concurrent requests cannot leave
//! overlapping, unclaimed containers at a single induction point. An optional
//! isolation mode supplies the box instantly at the selected empty HTM instead.

use desgraph::{CellId, StationId, ZoneId};
use std::future::pending;
use tokio::select;

use crate::error::planner::ErrContext;
use crate::events::{EventJournal, ToteCreatedEvent, ToteMovedEvent, TripEndpoint, TripPurpose};
use crate::flows::transport::{HtmPosition, Navigate};
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::model::ParkingRegistry;
use crate::model::ToteType;
use crate::model::bot::LegSpec;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::htm::registry::{
    AvailableHtm, AvailableHtmRegistry, PickupHeadroom, PickupSelection, ProposedHtmPickup,
};
use crate::model::navigation::Navigation;
use crate::observer::time_slice::ToteVisualState;
use crate::pathfinding::CellCandidate;
use crate::prelude::*;

use super::tote_handoff::validate_position;

#[derive(Display)]
#[display("InductContainer(at={source:?})")]
pub struct InductContainer {
    source: CellId,
    tote_type: ToteType,
    designation: ToteDesignation,
    after: SimTime,

    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    clock: Mut<SimTime>,
    station: Option<StationId>,
    headroom: Option<PickupHeadroom>,
    instant_supply: bool,
}

#[derive(Clone)]
struct Candidate(AvailableHtm);

impl CellCandidate for Candidate {
    fn cell(&self) -> CellId {
        self.0.cell()
    }
}

impl InductContainer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: CellId,
        tote_type: ToteType,
        designation: ToteDesignation,
        after: SimTime,

        navigation: Arc<Navigation>,
        bots: BotService,
        inventory: InventoryService,
        available: AvailableHtmRegistry,
        parking: Option<ParkingRegistry>,
        events: EventJournal,
        clock: Mut<SimTime>,
    ) -> Self {
        Self {
            source,
            tote_type,
            designation,
            after,

            navigation,
            bots,
            inventory,
            available,
            parking,
            events,
            clock,
            station: None,
            headroom: None,
            instant_supply: false,
        }
    }

    pub fn with_station(mut self, station: StationId) -> Self {
        self.station = Some(station);
        self
    }

    pub fn with_headroom(mut self, headroom: PickupHeadroom) -> Self {
        self.headroom = Some(headroom);
        self
    }

    /// Eliminate induction travel and dwell; source becomes the downstream
    /// destination used to choose a nearby empty HTM. No robot is teleported.
    pub fn with_instant_supply(mut self) -> Self {
        self.instant_supply = true;
        self.station = None;
        self
    }

    fn select(&self) -> Result<Option<PickupSelection>, PlanError> {
        let target = self.navigation.map.cell_world_pos(self.source).ok_or_else(|| {
            super::invalid(format!("container induction source {:?} has no world position", self.source))
        })?;
        let holder = self
            .navigation
            .oracle
            .reservations_for_cell(self.source)
            .into_iter()
            .find(|hold| hold.is_open_ended())
            .map(|hold| hold.robot());
        let mut candidates = self.available.find_nearest(
            target,
            |htm| {
                htm.tote.is_none()
                    && (self.instant_supply || holder.is_none_or(|robot| htm.robot() == robot))
                    && !self.navigation.htm_departure_is_blocked(
                        htm.robot(),
                        htm.cell(),
                        self.source,
                        self.instant_supply,
                        htm.available_at().max(self.clock.value()),
                    )
            },
            usize::MAX,
        );
        while !candidates.is_empty() {
            let selected = self
                .navigation
                .select_htm_to(
                    candidates.iter().cloned().map(Candidate),
                    self.source,
                    |candidate| candidate.0.robot(),
                    |candidate| candidate.0.orientation,
                    self.instant_supply,
                    |candidate, travel| Some((candidate.0.available_at() + travel).max(self.after)),
                )
                .err_context("InductContainer")?;
            let htm = selected.candidate.0;
            let depart_anchor = if self.instant_supply {
                self.after.max(htm.available_at())
            } else {
                self.after.saturating_sub(selected.travel_time).max(htm.available_at())
            };
            if self.navigation.htm_departure_is_blocked(
                htm.robot(),
                htm.cell(),
                self.source,
                self.instant_supply,
                depart_anchor.max(self.clock.value()),
            ) {
                candidates.retain(|candidate| candidate.robot() != htm.robot());
                continue;
            }
            return Ok(Some(PickupSelection {
                resident: self.instant_supply || htm.cell() == self.source,
                depart_anchor,
                travel: if self.instant_supply { SimDuration::ZERO } else { selected.travel_time },
                htm,
            }));
        }
        Ok(None)
    }
}

#[async_trait(?Send)]
impl Task for InductContainer {
    type Proposal = ProposedHtmPickup;
    type Schedule = HtmCarryingTote;

    async fn propose(&self, owner: ProposalId) -> Result<Self::Proposal, PlanError> {
        loop {
            let fleet_seen = self.available.version().seen();
            let oracle_version = self.navigation.oracle.version();
            let oracle_seen = oracle_version.map(Version::seen);
            let policy_seen = self.headroom.as_ref().map(|guard| guard.version().seen());
            let selection = self.select()?.and_then(|mut selection| {
                if let Some(headroom) = &self.headroom {
                    selection.depart_anchor = headroom
                        .earliest_allowed_at(&self.available, selection.depart_anchor.max(self.clock.value()))?;
                }
                Some(selection)
            });
            if let Some(selection) = selection {
                let mut proposal = ProposedHtmPickup::new(
                    owner,
                    self.available.clone(),
                    self.navigation.clone(),
                    if self.instant_supply { selection.htm.cell() } else { self.source },
                    selection,
                    fleet_seen,
                    self.clock.clone(),
                );
                if let Some(headroom) = &self.headroom {
                    proposal = proposal.with_headroom(headroom.clone());
                }
                return Ok(proposal);
            }
            select! {
                _ = self.available.version().past(fleet_seen) => {},
                _ = async {
                    match oracle_version.zip(oracle_seen) {
                        Some((version, seen)) => version.past(seen).await,
                        None => pending().await,
                    }
                } => {},
                _ = async {
                    match self.headroom.as_ref().zip(policy_seen) {
                        Some((guard, seen)) => guard.version().past(seen).await,
                        None => pending().await,
                    }
                } => {},
            }
        }
    }

    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let (htm, depart) = proposal.into_accepted()?;
        let presentation = self.events.next_presentation_id();
        let position = HtmPosition {
            cell_reservation: htm.cell_reservation,
            action_ready_at: depart,
            orientation: htm.orientation,
        };
        // Even same-cell acquisition must establish a productive plan frontier:
        // availability is advertised at the end of the previously committed leg.
        let target = if self.instant_supply { position.cell_reservation.cell() } else { self.source };
        let mut navigate = Navigate::new(
            position,
            target,
            None,
            self.navigation.clone(),
            self.bots.clone(),
            self.inventory.clone(),
            self.parking.clone(),
            self.events.clone(),
            self.clock.clone(),
        );
        if let Some(station) = self.station {
            navigate = navigate.with_trip(TripPurpose::ToStation, Some(presentation), TripEndpoint::Station(station));
        }
        let mut position = ctx.add_task(navigate).await;
        let zone = self
            .navigation
            .map
            .graph
            .cell(position.cell_reservation.cell())
            .ok_or_else(|| super::invalid("unknown container supply cell"))?
            .zone;
        position.action_ready_at = position.action_ready_at.max(self.after);
        let (position, tote) = ctx
            .add_task(CreateContainer {
                position: Some(position),
                tote_type: self.tote_type,
                designation: self.designation,
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                events: self.events.clone(),
            })
            .await;
        let dwell = if self.instant_supply {
            SimDuration::ZERO
        } else {
            self.bots.model(position.cell_reservation.robot())?.figures.conveyor_pickup_time()
        };
        Ok(ctx
            .add_task(PickupContainer {
                position: Some(position),
                tote,
                zone,
                presentation,
                origin: self.station.map(TripEndpoint::Station),
                dwell,
                inventory: self.inventory.clone(),
                bots: self.bots.clone(),
                events: self.events.clone(),
            })
            .await)
    }
}

#[derive(Display)]
#[display("CreateContainer")]
struct CreateContainer {
    position: Option<HtmPosition>,
    tote_type: ToteType,
    designation: ToteDesignation,
    inventory: InventoryService,
    bots: BotService,
    events: EventJournal,
}

#[async_trait(?Send)]
impl Task for CreateContainer {
    type Proposal = EmptyProposal;
    type Schedule = (HtmPosition, ToteId);

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let mut position = self.position.take().ok_or_else(|| super::invalid("container was already created"))?;
        let at = position.action_ready_at.max(ctx.barrier().clock().value());
        validate_position(&position, &self.bots, None, at, at).map_err(|error| {
            super::invalid(format!(
                "CreateContainer {:?} ready={} at={}: {error}",
                position.cell_reservation.robot(),
                position.action_ready_at,
                at
            ))
        })?;
        let cell = position.cell_reservation.cell();
        let tote =
            self.inventory.create_tote(self.tote_type, self.designation, ToteLocation::AtCell { cell_id: cell }, at)?;
        self.events.emit(ToteCreatedEvent {
            tote_id: tote,
            tote_type: self.tote_type,
            designation: self.designation,
            initial_state: ToteVisualState::AtCell { cell_id: cell.0 },
            time: at,
        });
        position.action_ready_at = at;
        Ok((position, tote))
    }
}

#[derive(Display)]
#[display("PickupContainer({tote:?})")]
struct PickupContainer {
    position: Option<HtmPosition>,
    tote: ToteId,
    zone: ZoneId,
    presentation: PresentationId,
    origin: Option<TripEndpoint>,
    dwell: SimDuration,
    inventory: InventoryService,
    bots: BotService,
    events: EventJournal,
}

#[async_trait(?Send)]
impl Task for PickupContainer {
    type Proposal = EmptyProposal;
    type Schedule = HtmCarryingTote;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let position = self.position.take().ok_or_else(|| super::invalid("container was already picked up"))?;
        let start = position.action_ready_at.max(ctx.barrier().clock().value());
        let end = start + self.dwell;
        let robot = position.cell_reservation.robot();
        let cell = position.cell_reservation.cell();
        validate_position(&position, &self.bots, None, start, end).map_err(|error| {
            super::invalid(format!(
                "PickupContainer {:?} ready={} start={} end={}: {error}",
                robot, position.action_ready_at, start, end
            ))
        })?;
        let tote = self.inventory.tote(self.tote)?;
        if !tote.location_is_through(start, end, ToteLocation::AtCell { cell_id: cell }) {
            return Err(super::invalid("container pickup has no container at its held source cell"));
        }
        self.bots.commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell },
                requests: &[],
                action_ready_arrival: end,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::empty(),
                carrying_after: Some(self.tote),
            }],
            &self.inventory,
            None,
        )?;
        tote.move_to(ToteLocation::WithRobot { robot_id: robot }, end)?;
        self.events.emit(ToteMovedEvent {
            tote_id: self.tote,
            new_state: ToteVisualState::OnBot { bot_id: robot.0 },
            time: end,
        });
        Ok(HtmCarryingTote {
            tote_id: self.tote,
            zone_id: self.zone,
            cell_reservation: position.cell_reservation,
            action_ready_at: end,
            orientation: position.orientation,
            station_arrival_plan: None,
            presentation_id: self.presentation,
            source_relay_id: None,
            deliver_from: self.origin,
        })
    }
}

#[cfg(test)]
#[path = "induct_container_tests.rs"]
mod tests;
