//! Resolve the run's declared machines into independent conveyance managers.
//! Constructing them reserves nothing and moves no tote or robot.

use std::any::type_name;

use desmap::ConveyorMachine;

use crate::model::conveyor::ConveyorId;
use crate::model::factories::world_inputs;
use crate::state::{StateError, StateManager, States};

use super::{Conveyance, ConveyanceError};

// Current EasyLoop equipment holds eight inventory totes across the whole circuit.
const EASYLOOP_TOTE_CAPACITY: usize = 8;

pub struct Conveyances {
    machines: Vec<(ConveyorMachine, Conveyance)>,
}

impl Conveyances {
    pub fn machines(&self) -> &[(ConveyorMachine, Conveyance)] {
        &self.machines
    }

    fn from_layouts(mut layouts: Vec<ConveyorMachine>) -> Result<Self, ConveyanceError> {
        layouts.sort_by_key(|machine| machine.conveyor_group);
        if layouts.windows(2).any(|pair| pair[0].conveyor_group == pair[1].conveyor_group) {
            return Err(ConveyanceError::InvalidEquipment);
        }
        let machines = layouts
            .into_iter()
            .map(|layout| {
                let capacity = layout.inventory_holding_capacity();
                let conveyance = Conveyance::new(
                    ConveyorId(layout.conveyor_group),
                    EASYLOOP_TOTE_CAPACITY,
                    capacity,
                    layout.presentations.iter().map(|presentation| presentation.station_id),
                )?;
                Ok((layout, conveyance))
            })
            .collect::<Result<_, ConveyanceError>>()?;
        Ok(Self { machines })
    }
}

impl StateManager for Conveyances {
    fn create(states: &States) -> Result<Self, StateError> {
        let world = world_inputs(states)?;
        Self::from_layouts(world.data.map.conveyor_machines.clone())
            .map_err(|error| StateError::Construction { kind: type_name::<Self>(), detail: error.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::model::{RobotId, SimTime, ToteId};
    use crate::worldmap;

    fn layout() -> ConveyorMachine {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop.json");
        worldmap::load(&path).unwrap().map.conveyor_machines.remove(0)
    }

    #[test]
    fn machines_are_sorted_independent_and_empty_until_work_claims_capacity() {
        let mut later = layout();
        later.conveyor_group = 7;
        let mut earlier = later.clone();
        earlier.conveyor_group = 2;
        let managers = Conveyances::from_layouts(vec![later, earlier]).unwrap();
        assert_eq!(managers.machines().iter().map(|(layout, _)| layout.conveyor_group).collect::<Vec<_>>(), [2, 7]);
        for (layout, service) in managers.machines() {
            assert_eq!(service.id(), ConveyorId(layout.conveyor_group));
            assert_eq!(service.unfinished(), 0);
            assert!(service.visits().is_empty());
            assert!(service.transfers().is_empty());
            assert_eq!(layout.presentations.len(), 2);
        }
        let service = &managers.machines()[0].1;
        let claims: Vec<_> =
            (0..8).map(|id| service.try_admit(ToteId(id), RobotId(id), SimTime::ZERO).unwrap().unwrap()).collect();
        assert!(service.try_admit(ToteId(8), RobotId(8), SimTime::ZERO).unwrap().is_none());
        assert!(managers.machines()[1].1.try_admit(ToteId(8), RobotId(8), SimTime::ZERO).unwrap().is_some());
        drop(claims);
        assert_eq!(service.unfinished(), 0);
    }

    #[test]
    fn extra_waiting_cells_do_not_increase_conveyor_capacity() {
        let mut layout = layout();
        let extra = layout.inventory_queue.as_ref().unwrap().path.clone();
        layout.inventory_queue.as_mut().unwrap().path.extend(extra);
        let managers = Conveyances::from_layouts(vec![layout]).unwrap();
        let service = &managers.machines()[0].1;
        let _claims: Vec<_> =
            (0..8).map(|id| service.try_admit(ToteId(id), RobotId(id), SimTime::ZERO).unwrap().unwrap()).collect();
        assert!(service.try_admit(ToteId(8), RobotId(8), SimTime::ZERO).unwrap().is_none());
    }

    #[test]
    fn duplicate_machine_identity_or_missing_holding_capacity_is_refused() {
        let layout = layout();
        assert!(matches!(
            Conveyances::from_layouts(vec![layout.clone(), layout.clone()]),
            Err(ConveyanceError::InvalidEquipment)
        ));
        let mut broken = layout;
        broken.inventory_queue = None;
        assert!(matches!(Conveyances::from_layouts(vec![broken]), Err(ConveyanceError::InvalidEquipment)));
    }
}
