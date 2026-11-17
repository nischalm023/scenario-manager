//! Design lead times for rack retrieval, used to size the inventory pipeline.
//! These contention-free estimates reserve nothing. They include the VTM and
//! empty HTM in parallel, then the loaded journey and destination handling;
//! they are not promises about completion under traffic or resource queues.

use desgraph::{CellId, HtmTraversal, WorldPos};
use desmap::{BlockSide, ConveyorMachine};
use rustc_hash::FxHashSet;

use crate::config::SimConfig;
use crate::error::PlanError;
use crate::model::navigation::Navigation;
use crate::model::navigation::heading::unassigned_htm_orientation;
use crate::model::{AdmissionLaw, RackSlot, SimDuration};
use crate::pathfinding::estimate::{cell_point_mm, manhattan_mm};
use crate::pathfinding::{Orientation, PathCacheKey};
use crate::robots::runtime::RobotModelId;
use crate::scenario_gen::{RackUse, ScenarioData};

use super::invalid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchBound {
    pub vtm_retrieval: SimDuration,
    pub empty_htm_approach: SimDuration,
    pub relay_pickup: SimDuration,
    pub loaded_travel: SimDuration,
    /// Setting the tote down on the machine's belt; zero for a destination
    /// that is not a conveyor machine.
    pub deposit: SimDuration,
    pub inbound_conveyor: SimDuration,
}

impl FetchBound {
    pub fn total(&self) -> SimDuration {
        self.vtm_retrieval.max(self.empty_htm_approach)
            + self.relay_pickup
            + self.loaded_travel
            + self.deposit
            + self.inbound_conveyor
    }
}

struct Source {
    model: RobotModelId,
    loaded_turn: SimDuration,
    cell: CellId,
    orientation: Orientation,
    pool: RackUse,
    vtm: SimDuration,
    empty_approach: SimDuration,
}

pub struct FetchBounds {
    pub admission: AdmissionLaw,
    pub reference_cell: CellId,
    pub origin_count: usize,
    sources: Vec<Source>,
    pickup: SimDuration,
    conveyor_deposit: SimDuration,
}

impl FetchBounds {
    /// Reuse the same reference across machines. Empty routes go origin →
    /// ordinary floor reference → relay, avoiding every origin × relay pair.
    /// Those detours and two possible half-turns make a deliberately generous
    /// estimate; loaded routes use the actual directed static path to each target.
    pub fn new(data: &ScenarioData, navigation: &Navigation, config: &SimConfig) -> Result<Self, PlanError> {
        if !config.queue_factor.is_finite() || config.queue_factor < 0.0 {
            return Err(invalid("inventory queue_factor must be finite and nonnegative"));
        }
        if config.order.max_inflight_picks_override == Some(0) {
            return Err(invalid("inventory max_inflight_picks_override must be positive"));
        }
        let period = seconds(config.operator.time_per_pick_secs)?;
        if period == SimDuration::ZERO {
            return Err(invalid("inventory pipeline needs a positive transfer period"));
        }
        let models = navigation.models();
        if models.is_empty() {
            return Err(invalid("inventory fetch estimate has no eligible HTM models"));
        }
        let sources = source_cycles(data)?;
        if sources.is_empty() {
            return Err(invalid("inventory fetch estimate has no rack/relay sources"));
        }
        let mut origins: Vec<_> = data
            .relay_data
            .iter()
            .map(|relay| relay.htm_access)
            .chain(data.htm_initial_positions.iter().copied())
            // A conveyor handover retains its robot through the collection
            // queue; it is not an empty-fleet frontier toward arbitrary relays.
            .chain(
                data.station_data
                    .iter()
                    .filter(|station| station.conveyor.is_none())
                    .map(|station| station.presentation_cell),
            )
            .filter(|&cell| {
                navigation
                    .map
                    .graph
                    .neighbors(cell)
                    .iter()
                    .any(|(_, edge)| edge.traversability.htm_traversal != HtmTraversal::None)
            })
            .collect();
        origins.sort_unstable();
        origins.dedup();
        let reference = reference_cell(navigation, &origins)?;
        let mut prepared = Vec::with_capacity(sources.len() * models.len());
        for model in models {
            let kinematics = &model.kinematics;
            let mut to_reference = SimDuration::ZERO;
            for &origin in &origins {
                to_reference = to_reference.max(static_travel(
                    navigation,
                    model.id,
                    origin,
                    unassigned_htm_orientation(&navigation.map, origin, model.estimate[0].as_ref())
                        .map_err(|error| invalid(error.to_string()))?,
                    reference,
                    false,
                    kinematics.empty_turn,
                )?);
            }
            for &(cell, pool, vtm) in &sources {
                let from_reference = static_travel(
                    navigation,
                    model.id,
                    reference,
                    Orientation::East,
                    cell,
                    false,
                    kinematics.empty_turn,
                )?;
                prepared.push(Source {
                    model: model.id,
                    loaded_turn: kinematics.loaded_turn,
                    cell,
                    orientation: unassigned_htm_orientation(&navigation.map, cell, model.estimate[0].as_ref())
                        .map_err(|error| invalid(error.to_string()))?,
                    pool,
                    vtm,
                    empty_approach: to_reference + from_reference,
                });
            }
        }
        Ok(Self {
            admission: AdmissionLaw {
                multiplier: config.queue_factor,
                period,
                override_cap: config.order.max_inflight_picks_override,
            },
            reference_cell: reference,
            origin_count: origins.len(),
            sources: prepared,
            pickup: models.iter().map(|model| model.figures.pickup_time()).max().expect("validated eligible models"),
            conveyor_deposit: models
                .iter()
                .map(|model| model.figures.conveyor_deposit_time())
                .max()
                .expect("validated eligible models"),
        })
    }

    /// Rack pool → owned HTM arrival. Order-container retrieval can use this
    /// without paying the inventory conveyor's unload and inbound travel.
    pub fn for_destination(
        &self,
        navigation: &Navigation,
        pool: RackUse,
        target: CellId,
    ) -> Result<FetchBound, PlanError> {
        let mut estimate = FetchBound {
            vtm_retrieval: SimDuration::ZERO,
            empty_htm_approach: SimDuration::ZERO,
            relay_pickup: self.pickup,
            loaded_travel: SimDuration::ZERO,
            deposit: SimDuration::ZERO,
            inbound_conveyor: SimDuration::ZERO,
        };
        let mut count = 0;
        for source in self.sources.iter().filter(|source| source.pool == pool) {
            count += 1;
            estimate.vtm_retrieval = estimate.vtm_retrieval.max(source.vtm);
            estimate.empty_htm_approach = estimate.empty_htm_approach.max(source.empty_approach);
            estimate.loaded_travel = estimate.loaded_travel.max(static_travel(
                navigation,
                source.model,
                source.cell,
                source.orientation,
                target,
                true,
                source.loaded_turn,
            )?);
        }
        if count == 0 {
            return Err(invalid(format!("fetch estimate has no {pool:?} rack sources")));
        }
        Ok(estimate)
    }

    pub fn for_machine(&self, navigation: &Navigation, machine: &ConveyorMachine) -> Result<FetchBound, PlanError> {
        let dropoff = machine.dropoff_cell.ok_or_else(|| invalid("inventory fetch target has no dropoff cell"))?;
        let mut estimate = self.for_destination(navigation, RackUse::Inventory, dropoff)?;
        estimate.deposit = self.conveyor_deposit;
        // Match the current full-circuit/equal-leg timing used by FulfillOrder.
        estimate.inbound_conveyor = machine
            .presentations
            .iter()
            .filter_map(|presentation| navigation.map.stations.iter().find(|s| s.id == presentation.station_id))
            .map(|station| SimDuration::ms(station.conveyor_transit_ms.unwrap_or(0) / 2))
            .max()
            .unwrap_or(SimDuration::ZERO);
        Ok(estimate)
    }
}

fn source_cycles(data: &ScenarioData) -> Result<Vec<(CellId, RackUse, SimDuration)>, PlanError> {
    let mut sources = Vec::new();
    for zone in data.relay_zones.zones() {
        // One rack scan per zone, independent of relay count and tote units.
        let mut heights = [[None::<f64>; 3]; 2];
        for &rack in zone.rack_world_pos.keys() {
            let side = zone.rack_side.get(&rack).ok_or_else(|| invalid(format!("rack {rack:?} has no relay side")))?;
            let pool = data.rack_use.get(&rack).copied().unwrap_or_default();
            let height = zone.slot_height_m(RackSlot { rack_id: rack, level: zone.real_shelf_count.saturating_sub(1) });
            let maximum = &mut heights[side_index(*side)][pool_index(pool)];
            *maximum = Some(maximum.unwrap_or(0.0).max(height));
        }
        for relay in &zone.relays {
            for pool in [RackUse::Inventory, RackUse::OrderDepot, RackUse::OrderBuffer] {
                let Some(height) = heights[side_index(relay.side)][pool_index(pool)] else { continue };
                // A VTM may start at either end of its aisle. Both horizontal
                // legs use that full span; each leg's horizontal/lift motion
                // still overlaps exactly as in the shared forward-cycle model.
                let maximum = zone
                    .vtm_models
                    .iter()
                    .map(|model| {
                        let cycle =
                            model.forward_cycle(zone.zone_width_m, zone.zone_width_m, height, zone.relay_height_m);
                        seconds(cycle.total_secs)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .max();
                // A rack zone without an eligible VTM cannot supply retrieval work.
                if let Some(maximum) = maximum {
                    sources.push((relay.cell_id, pool, maximum));
                }
            }
        }
    }
    sources.sort_unstable_by_key(|&(cell, pool, _)| (cell, pool_index(pool)));
    Ok(sources)
}

fn side_index(side: BlockSide) -> usize {
    match side {
        BlockSide::Left => 0,
        BlockSide::Right => 1,
    }
}

fn pool_index(pool: RackUse) -> usize {
    match pool {
        RackUse::Inventory => 0,
        RackUse::OrderDepot => 1,
        RackUse::OrderBuffer => 2,
    }
}

fn static_travel(
    navigation: &Navigation,
    model: RobotModelId,
    from: CellId,
    orientation: Orientation,
    to: CellId,
    carrying: bool,
    quarter_turn: SimDuration,
) -> Result<SimDuration, PlanError> {
    let cache = navigation.cache_for_model(model)?;
    if from == to {
        return Ok(SimDuration::ZERO);
    }
    let travel =
        cache.travel_time(PathCacheKey { from, from_orientation: orientation, to, carrying_cargo: carrying })?;
    // Unconstrained available headings can differ from the reference East.
    // Relay headings are known, so no fictional turn is charged there.
    Ok(if navigation.map.graph.cell(from).is_some_and(|cell| cell.htm_rotation_forbidden) {
        travel
    } else {
        travel + quarter_turn + quarter_turn
    })
}

fn reference_cell(navigation: &Navigation, origins: &[CellId]) -> Result<CellId, PlanError> {
    let mut center = WorldPos { x_mm: 0.0, y_mm: 0.0 };
    for &origin in origins {
        let position =
            cell_point_mm(&navigation.map, origin).ok_or_else(|| invalid(format!("no position for {origin:?}")))?;
        center.x_mm += position.x_mm;
        center.y_mm += position.y_mm;
    }
    if origins.is_empty() {
        return Err(invalid("fetch estimate has no possible HTM frontiers"));
    }
    center.x_mm /= origins.len() as f64;
    center.y_mm /= origins.len() as f64;
    let machine_cells: FxHashSet<_> =
        navigation.map.conveyor_machines.iter().flat_map(|machine| machine.cells.iter().copied()).collect();
    navigation
        .map
        .graph
        .cells()
        .filter(|cell| !cell.htm_rotation_forbidden && !machine_cells.contains(&cell.id))
        .filter(|cell| {
            navigation
                .map
                .graph
                .neighbors(cell.id)
                .iter()
                .filter(|(_, edge)| {
                    edge.traversability.htm_traversal != HtmTraversal::None
                        && edge.dest_only.is_none()
                        && edge.vtm_zone_gate.is_none()
                })
                .count()
                >= 2
        })
        .filter_map(|cell| Some((manhattan_mm(cell_point_mm(&navigation.map, cell.id)?, center), cell.id)))
        .min_by(|(a, a_id), (b, b_id)| a.total_cmp(b).then_with(|| a_id.cmp(b_id)))
        .map(|(_, cell)| cell)
        .ok_or_else(|| invalid("fetch estimate needs an ordinary floor reference outside conveyor machines"))
}

fn seconds(value: f64) -> Result<SimDuration, PlanError> {
    if !value.is_finite() || value < 0.0 || value * 1000.0 >= u64::MAX as f64 {
        return Err(invalid(format!("invalid fetch duration {value}s")));
    }
    Ok(SimDuration::ms((value * 1000.0).ceil() as u64))
}

#[cfg(test)]
#[path = "fetch_bound_tests.rs"]
mod tests;
