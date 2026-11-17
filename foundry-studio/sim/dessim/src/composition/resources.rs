//! The run's inputs that a composition publishes for its tasks (`RunInputs`,
//! `InventorySeed`), and the factories of two pieces of state: the orders the
//! order tasks share (`Orders`) and the conveyor loops. Consumers request
//! these independently; none relies on a particular composition or on root
//! registration order.

use std::sync::Arc;

use super::Definition;
use crate::config::SimConfig;
use crate::model::conveyor::{ConveyorId, ConveyorLoops};
use crate::model::factories::world_inputs;
use crate::model::order::OrderService;
use crate::model::{OrderSpec, SimDuration, SkuId, ToteId};
use crate::scenario_gen::{DemandPool, SkuInfo, TotePlacement};
use crate::state::{StateError, StateManager, States};

pub struct InventorySeed {
    pub placements: Vec<TotePlacement>,
    pub contents: Vec<(SkuId, ToteId, u32)>,
}

// TODO(module): with `inputs`, a bag of the whole run's inputs; each module should receive only the inputs it reads.
pub struct RunInputs {
    pub definition: Definition,
    pub config: SimConfig,
    pub orders: Vec<OrderSpec>,
    pub skus: Vec<SkuInfo>,
    pub demand_pool: Option<DemandPool>,
}

pub fn inputs(states: &States) -> Result<Arc<RunInputs>, StateError> {
    states.configuration::<RunInputs>()
}

pub struct Orders {
    pub service: OrderService,
}

impl StateManager for Orders {
    fn create(states: &States) -> Result<Self, StateError> {
        inputs(states)?;
        Ok(Self { service: OrderService::new() })
    }
}

impl StateManager for ConveyorLoops {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let data = &inputs.data;
        Ok(Self::new(
            data.station_data
                .iter()
                .filter_map(|station| Some((station.station_id, ConveyorId(station.conveyor?))))
                .collect(),
            data.station_data
                .iter()
                .filter_map(|station| Some((ConveyorId(station.conveyor?), station.conveyor_transit?)))
                .collect(),
            SimDuration::from_secs(inputs.config.operator.time_per_pick_secs),
        ))
    }
}
