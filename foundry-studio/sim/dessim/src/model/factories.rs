//! Construct specific warehouse managers on first request. Factories wire
//! immutable topology and shared dependencies; setup tasks register robots,
//! populate inventory, reserve initial positions and announce physical state.

use std::any::type_name;
use std::sync::Arc;

use desgraph::{CardinalDirection, CellKind};

use super::bot::BotService;
use super::inventory::InventoryService;
use super::navigation::Navigation;
use super::parking::ParkingRegistry;
use super::relay::RelayService;
use super::reservation::ReservationTable;
use super::reservation::oracle::{GhostOracle, ReservationOracle, TableOracle};
use super::station::{Station, StationSet};
use super::station_service::StationService;
use super::{SimDuration, SimTime, StationArrivalDiagnostics, StationFetchBounds};
use crate::config::{FidelityMode, SimConfig};
use crate::events::{EventJournal, EventSink};
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::pathfinding::Pathfinder;
use crate::scenario_gen::ScenarioData;
use crate::signals::Mut;
use crate::state::{StateError, StateManager, States};

// TODO(module): with `world_inputs`, a bag of the whole run's inputs; each module should receive only what it reads.
/// Raw construction environment, published once before tasks run. Running
/// tasks retain only the specific managers and parameters they need.
pub struct WorldInputs {
    pub data: Arc<ScenarioData>,
    pub config: SimConfig,
    pub(crate) pathfinder: Arc<dyn Pathfinder>,
    /// The run's event stream. It is private, so the world reaches it only
    /// through the `EventJournal` built from it here, and every fact the world
    /// reports is checked against B.
    event_sink: Option<Arc<dyn EventSink>>,
    pub clock: Mut<SimTime>,
}

impl WorldInputs {
    pub(crate) fn new(
        data: Arc<ScenarioData>,
        config: SimConfig,
        pathfinder: Arc<dyn Pathfinder>,
        event_sink: Option<Arc<dyn EventSink>>,
        clock: Mut<SimTime>,
    ) -> Self {
        Self { data, config, pathfinder, event_sink, clock }
    }
}

pub fn world_inputs(states: &States) -> Result<Arc<WorldInputs>, StateError> {
    states.configuration()
}

impl StateManager for BotService {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        inputs
            .data
            .robot_models
            .validate_counts(inputs.data.htm_initial_positions.len(), inputs.data.vtm_zone_assignments.len())
            .map_err(|detail| StateError::Construction { kind: type_name::<Self>(), detail })?;
        Ok(Self::from_runtime(&inputs.data.robot_models, inputs.data.vtm_zone_assignments.len()))
    }
}

impl StateManager for EventJournal {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        Ok(Self::new(inputs.event_sink.clone()))
    }
}

impl StateManager for Navigation {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let oracle: Arc<dyn ReservationOracle> = match inputs.config.fidelity {
            FidelityMode::CollisionDes => {
                // Where a robot hands a tote over: a relay's access cell, a
                // station's presentation cell.
                let relays = inputs.data.map.relays.iter().map(|relay| relay.htm_access);
                let stations = inputs.data.station_data.iter().map(|station| station.presentation_cell);
                Arc::new(TableOracle::emitting_handoff_holds(
                    ReservationTable::default(),
                    relays.chain(stations).collect(),
                    states.get::<EventJournal>()?.as_ref().clone(),
                ))
            }
            _ => Arc::new(GhostOracle),
        };
        Ok(Self::new(
            Arc::new(inputs.data.map.clone()),
            inputs.pathfinder.clone(),
            states.get::<BotService>()?.as_ref().clone(),
            oracle,
        ))
    }
}

impl StateManager for InventoryService {
    fn create(states: &States) -> Result<Self, StateError> {
        let bots = states.get::<BotService>()?;
        let inventory = Self::with_event_sink(states.get::<EventJournal>()?.sink());
        inventory.set_bot_service(bots.as_ref().clone());
        Ok(inventory)
    }
}

impl StateManager for ParkingRegistry {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        Ok(Self::with_policy(&inputs.data.map, inputs.config.htm.idle_selection, inputs.config.rng_seed))
    }
}

impl StateManager for AvailableHtmRegistry {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let inventory = states.get::<InventoryService>()?;
        let parking = states.get::<ParkingRegistry>()?;
        let idle_relay_cells = parking.candidate_cells().collect();
        Ok(Self::new(inventory.as_ref().clone(), &inputs.data.map, idle_relay_cells, Some(parking.as_ref().clone())))
    }
}

impl StateManager for RelayService {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let inventory = states.get::<InventoryService>()?;
        let navigation = states.get::<Navigation>()?;
        let available = states.get::<AvailableHtmRegistry>()?;
        let relay =
            Self::new(&inputs.data, inventory.as_ref().clone(), navigation.clone(), &available, &inputs.config.vtm)
                .map_err(|error| StateError::Construction { kind: type_name::<Self>(), detail: error.to_string() })?;
        if inputs.config.putaway.random_slots {
            relay.set_random_deposit_slots(inputs.config.rng_seed);
        }
        relay.set_admission_floor(inputs.clock.clone());
        relay.vtm_service.set_admission_floor(inputs.clock.clone());
        Ok(relay)
    }
}

impl StateManager for StationSet {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let stations = inputs
            .data
            .station_data
            .iter()
            .map(|station| {
                Station::new(
                    station.station_id,
                    station.station_type,
                    station.presentation_cell,
                    vec![],
                    CardinalDirection::East,
                    station.bins.unwrap_or(inputs.config.operator.num_bins),
                )
            })
            .collect();
        Ok(Self::new(stations))
    }
}

impl StateManager for StationService {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let navigation = states.get::<Navigation>()?;
        let presentations =
            inputs.data.station_data.iter().map(|station| (station.station_id, station.presentation_cell)).collect();
        Ok(Self::new(presentations, navigation.oracle.clone()))
    }
}

impl StateManager for StationFetchBounds {
    fn create(states: &States) -> Result<Self, StateError> {
        let inputs = world_inputs(states)?;
        let stations = states.get::<StationSet>()?;
        let mut sources: Vec<_> = inputs
            .data
            .relay_data
            .iter()
            .map(|relay| relay.htm_access)
            .chain(stations.pick_stations().iter().map(|station| station.presentation_cell))
            .chain(stations.putaway_stations().iter().map(|station| station.presentation_cell))
            .chain(inputs.data.rack_cells.values().copied())
            .chain(inputs.data.map.graph.cells().filter(|cell| cell.kind == CellKind::Charging).map(|cell| cell.id))
            .collect();
        sources.sort_unstable();
        sources.dedup();
        let models = inputs.data.robot_models.unique_htm_models();
        let traverse = models
            .iter()
            .map(|model| model.estimate[1].min_travel_time(inputs.data.map.cell_size_mm()))
            .max()
            .ok_or_else(|| StateError::Construction {
                kind: type_name::<Self>(),
                detail: "station fetch sizing needs an eligible HTM model".into(),
            })?;
        let pickup = models.iter().map(|model| model.figures.pickup_time()).max().expect("validated models");
        // This is a linear planning lookback, not a physical VTM cycle quote.
        // Keep its explicit allowance; acceleration is not repeated per grid cell.
        let fetch_allowance = SimDuration::ms(
            ((inputs.config.vtm.load_time_secs + inputs.config.vtm.unload_time_secs) * 1000.0) as u64,
        );
        Self::from_source_cells(
            stations.pick_stations().iter().chain(stations.putaway_stations().iter()),
            sources,
            &inputs.data.map,
            traverse,
            fetch_allowance + pickup,
        )
        .map_err(|error| StateError::Construction { kind: type_name::<Self>(), detail: error.to_string() })
    }
}

impl StateManager for StationArrivalDiagnostics {
    fn create(_: &States) -> Result<Self, StateError> {
        Ok(Self::new())
    }
}

#[cfg(test)]
#[path = "factories_tests.rs"]
mod tests;
