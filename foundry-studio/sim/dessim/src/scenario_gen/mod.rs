//! Scenario generation: WarehouseDesign + ScenarioConfig + SimConfig -> actor initial state.
//!
//! Converts the compact warehouse design, scenario parameters, and simulation config
//! into all the data needed to populate Dispatcher, OrderManager, and robot actors
//! before the simulation clock starts.

pub mod demand_pool;
mod htm_placement;
mod inventory;
mod orders;
pub mod pipeline;
pub(crate) mod resolve;
pub mod topology;
mod types;
mod validation;
mod vtm_placement;
/// Reader for Walmart's delivered site extracts. The one module that knows
/// their column vocabulary; everything outside it speaks the engine's.
pub(crate) mod walmart;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_integration;

pub use demand_pool::{DemandPool, DistributionCenterId, load_demand_pool};
pub use pipeline::{
    build_scenario_data, generate_scenario_from_map, generate_scenario_from_map_with_racking,
    generate_scenario_with_bulk, generate_scenario_with_loaded_data,
};
pub use topology::{RackColumn, RackUse, Racking, racking_from_map};
pub use types::*;
