//! Register the declared robot fleet and reserve its initial floor positions.
//! Construction resolves empty services; scheduling commits the physical setup.

use desgraph::{CellId, RobotType, WorldPos};

use crate::events::{EventJournal, InitialPositionsEvent, SimEvent};
use crate::model::ParkingRegistry;
use crate::model::bot::{BotKind, BotService};
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::navigation::Navigation;
use crate::model::navigation::heading::unassigned_htm_orientation;
use crate::pathfinding::{CommitPlan, ReservationRequest};
use crate::prelude::*;
use crate::task;
use crate::task::TaskContext;

#[task]
#[derive(Display)]
#[display("InitializeFleet")]
pub struct InitializeFleet {
    positions: Vec<(RobotId, CellId, RobotType, WorldPos)>,
    navigation: Arc<Navigation>,
    bots: BotService,
    available: AvailableHtmRegistry,
    parking: ParkingRegistry,
    events: EventJournal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_positions_without_heading_metadata_remain_readable() {
        let event = InitialPositionsEvent {
            positions: vec![(RobotId(1), CellId(2), RobotType::Htm, WorldPos { x_mm: 0.0, y_mm: 1000.0 })],
            htm_headings_rad: vec![(RobotId(1), 1.5)],
        };
        let mut encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(serde_json::from_value::<InitialPositionsEvent>(encoded.clone()).unwrap(), event);
        encoded.as_object_mut().unwrap().remove("htm_headings_rad");
        let legacy: InitialPositionsEvent = serde_json::from_value(encoded).unwrap();
        assert_eq!(legacy.positions, event.positions);
        assert!(legacy.htm_headings_rad.is_empty());
    }
}

#[async_trait(?Send)]
impl Task for InitializeFleet {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let inputs = world_inputs(ctx.states())?;
        let data = &inputs.data;
        let mut positions = Vec::new();
        for (index, position) in data.vtm_initial_positions.iter().enumerate() {
            positions.push((
                RobotId(index as u32),
                position.anchor(),
                RobotType::Vtm,
                position.world_pos(&data.map.graph).expect("validated VTM initial position"),
            ));
        }
        let vtm_count = data.vtm_zone_assignments.len();
        for (index, &cell) in data.htm_initial_positions.iter().enumerate() {
            let position = data.map.cell_world_pos(cell).ok_or_else(|| PlanError::TaskGraphInvariant {
                context: "InitializeFleet",
                detail: format!("robot starts at unknown cell {cell:?}"),
            })?;
            positions.push((RobotId((vtm_count + index) as u32), cell, RobotType::Htm, position));
        }
        Ok(Self {
            positions,
            navigation: ctx.state()?,
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: ctx.state::<ParkingRegistry>()?.as_ref().clone(),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
        })
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        let mut htm_headings_rad = Vec::new();
        for &(robot, cell, kind, _) in &self.positions {
            if kind != RobotType::Htm {
                continue;
            }
            let orientation = unassigned_htm_orientation(
                &self.navigation.map,
                cell,
                self.navigation.model(robot)?.estimate[0].as_ref(),
            )
            .map_err(|error| PlanError::TaskGraphInvariant { context: "InitializeFleet", detail: error.to_string() })?;
            self.bots.register(robot, BotKind::Htm, cell)?;
            let plan = CommitPlan::single(ReservationRequest::spanning(cell, SimTime::ZERO, None));
            let reservation = self
                .navigation
                .oracle
                .commit_path(robot, &plan, None)
                .map_err(|error| PlanError::TaskGraphInvariant {
                    context: "InitializeFleet",
                    detail: format!("cannot reserve {cell:?} for {robot:?}: {error:?}"),
                })?
                .into_iter()
                .next()
                .ok_or_else(|| PlanError::TaskGraphInvariant {
                    context: "InitializeFleet",
                    detail: format!("no initial reservation for {robot:?}"),
                })?;
            if self.parking.is_candidate(cell) {
                self.parking.reserve(cell, robot, SimTime::ZERO);
            }
            self.available.register(reservation, SimTime::ZERO, orientation, None)?;
            htm_headings_rad.push((robot, orientation.degrees().to_radians()));
        }
        self.events.emit(SimEvent::InitialPositions(InitialPositionsEvent {
            positions: self.positions.clone(),
            htm_headings_rad,
        }));
        Ok(())
    }
}
