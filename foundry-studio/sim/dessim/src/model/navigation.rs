//! Shared floor routing: immutable geometry, one cache per deployed model,
//! checked robot bindings and the reservation oracle. BotService owns the
//! physical models; routing uses their search and execution drives.

pub mod heading;

use crate::pathfinding::{
    CellCandidate, CellSelection, NearestCellSelector, Orientation, PathCacheKey, PathSelectionError,
};
use std::sync::Arc;

use crate::model::relay::PickupUnits;

use super::bot::BotService;
use super::cell::registry::CellRegistry;
use super::reservation::oracle::ReservationOracle;
use crate::error::PlanError;
use crate::model::{RobotId, SimDuration, SimTime};
use crate::pathfinding::{Cached, Pathfinder};
use crate::robots::runtime::{FloorRobotModel, RobotModelId};
use desgraph::{CellId, HtmTraversal};
use desmap::WarehouseMap;
use rustc_hash::FxHashMap;

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;

pub struct Navigation {
    pub map: Arc<WarehouseMap>,
    pub pathfinder: Arc<dyn Pathfinder>,
    pub bots: BotService,
    caches: FxHashMap<RobotModelId, Cached>,
    pub oracle: Arc<dyn ReservationOracle>,
    pub cells: CellRegistry,
    /// Picked-up totes' inbound units, moved by every journey's windows.
    pub pickup_units: PickupUnits,
}

impl Navigation {
    pub fn new(
        map: Arc<WarehouseMap>,
        pathfinder: Arc<dyn Pathfinder>,
        bots: BotService,
        oracle: Arc<dyn ReservationOracle>,
    ) -> Self {
        let caches = bots
            .models()
            .into_iter()
            .map(|model| {
                (model.id, Cached::from_arc(pathfinder.clone(), model.estimate.clone(), model.execution.clone()))
            })
            .collect();
        let cells = CellRegistry::new(oracle.clone());
        Self { map, pathfinder, bots, caches, oracle, cells, pickup_units: PickupUnits::default() }
    }

    pub fn models(&self) -> &[Arc<FloorRobotModel>] {
        self.bots.models()
    }

    pub fn model(&self, robot: RobotId) -> Result<Arc<FloorRobotModel>, PlanError> {
        self.bots.model(robot)
    }

    pub fn cache(&self, robot: RobotId) -> Result<&Cached, PlanError> {
        self.cache_for_model(self.model(robot)?.id)
    }

    pub fn cache_for_model(&self, model: RobotModelId) -> Result<&Cached, PlanError> {
        self.caches.get(&model).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "Navigation",
            detail: format!("model {model:?} has no route cache"),
        })
    }

    /// Before assignment, all eligible models must be able to serve the route.
    /// The largest physical time is the conservative sizing/quote policy.
    pub fn unassigned_travel_time(&self, key: PathCacheKey) -> Result<SimDuration, PlanError> {
        let mut maximum = None;
        for model in self.models() {
            let cache = self.cache_for_model(model.id)?;
            let time = if key.from == key.to { SimDuration::ZERO } else { cache.travel_time(key)? };
            maximum = Some(maximum.unwrap_or(SimDuration::ZERO).max(time));
        }
        maximum.ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "Navigation",
            detail: "no eligible floor robot models".into(),
        })
    }

    /// Retain the spatial-selection heuristic for a single model. Mixed
    /// fleets compare conservative physical quotes because their execution
    /// selectors can differ as well as their search speeds.
    pub fn unassigned_selection_time(&self, key: PathCacheKey) -> Result<SimDuration, PlanError> {
        let models = self.models();
        if models.len() == 1 {
            let cache = self.cache_for_model(models[0].id)?;
            return if key.from == key.to { Ok(SimDuration::ZERO) } else { Ok(cache.find_path(key)?.travel_time) };
        }
        self.unassigned_travel_time(key)
    }

    /// Chooses the candidate an HTM should travel from to reach `to`: each
    /// candidate's cell is a path's source and `to` its destination, driven by
    /// the robot `robot` names for it, starting as `orientation` faces, laden
    /// if `carrying`. `score` turns a candidate and a travel time into the
    /// simtime it would complete at, or into `None` to rule it out; the lowest
    /// score wins. The travel time that decides is the route cache's: the
    /// run's drive over the cached path, with no other robot on the floor,
    /// and zero for a candidate already on `to`. Candidates no path reaches
    /// are passed over; when none is left the error is `NoCandidate`. A
    /// candidate whose robot has no model or no route cache is a
    /// `ModelBinding` error, and any other failure of the pathfinder a
    /// `Pathfinder` error.
    ///
    /// With one floor robot model, candidates are shortlisted. Each is scored
    /// first on a Manhattan estimate of its travel time, and dropped if
    /// `score` rules it out there. The rest are put in the order of that
    /// estimated score, equal ones keeping the order of `candidates`, and are
    /// taken ten at a time. The first ten to hold a usable candidate decide:
    /// the lowest score on the route cache's travel time wins, and of equal
    /// scores the earliest in that order. A cell with no position, `to` or a
    /// candidate's, is a `MissingCellPosition` error.
    ///
    /// With several models there is no shortlist, since a distant faster
    /// model may win: every candidate is scored on the route cache's travel
    /// time, and of equal scores the earliest in `candidates` wins.
    pub fn select_htm_to<C, I, R, O, S>(
        &self,
        candidates: I,
        to: CellId,
        robot: R,
        orientation: O,
        carrying: bool,
        score: S,
    ) -> Result<CellSelection<C>, PathSelectionError>
    where
        I: IntoIterator<Item = C>,
        C: CellCandidate + Clone + Send + Sync,
        R: Fn(&C) -> RobotId + Sync,
        O: Fn(&C) -> Orientation + Sync,
        S: Fn(&C, SimDuration) -> Option<SimTime> + Sync,
    {
        let candidates: Vec<_> = candidates.into_iter().collect();
        // Validate every identity, including zero-distance candidates.
        for candidate in &candidates {
            self.cache(robot(candidate)).map_err(|e| PathSelectionError::ModelBinding(e.to_string()))?;
        }
        if self.models().len() == 1 {
            let model = &self.models()[0];
            let cache = self.cache_for_model(model.id).map_err(|e| PathSelectionError::ModelBinding(e.to_string()))?;
            return NearestCellSelector::new(cache, &self.map).select_nearest_to(
                candidates,
                to,
                orientation,
                carrying,
                |_| true,
                score,
            );
        }
        NearestCellSelector::select_all(candidates, to, score, |candidate| {
            let cache = self.cache(robot(candidate)).map_err(|e| PathSelectionError::ModelBinding(e.to_string()))?;
            if candidate.cell() == to {
                return Ok(SimDuration::ZERO);
            }
            Ok(cache.travel_time(PathCacheKey {
                from: candidate.cell(),
                from_orientation: orientation(candidate),
                to,
                carrying_cargo: carrying,
            })?)
        })
    }

    /// A necessary first-step check, not full route feasibility. Finite holds
    /// and open holds beginning after departure remain for SIPP to time.
    pub fn htm_departure_is_blocked(
        &self,
        robot: RobotId,
        from: CellId,
        target: CellId,
        carrying: bool,
        depart: SimTime,
    ) -> bool {
        if from == target {
            return false;
        }
        let mut exits = self.htm_exit_cells(from, target, carrying).peekable();
        // Missing legal edges is a static map error, not a reason to wait.
        exits.peek().is_some()
            && exits.all(|next| {
                self.oracle
                    .open_ended_reservation(next)
                    .is_some_and(|hold| hold.robot() != robot && hold.window().start() <= depart)
            })
    }

    /// Reject a candidate only when every graph-admissible first neighbor has no opening
    /// within its route-search horizon. An opening is not proof that the move
    /// fits: motion duration, heading and downstream traffic remain SIPP's job.
    pub fn htm_departure_exceeds_horizon(
        &self,
        robot: RobotId,
        from: CellId,
        target: CellId,
        carrying: bool,
        depart: SimTime,
        max_wait: SimDuration,
    ) -> bool {
        self.htm_departure_opening(robot, from, target, carrying, depart)
            .is_none_or(|opening| opening >= depart + max_wait)
    }

    /// Earliest first-neighbor opening, including committed future clearance.
    /// This is a conservative departure anchor, not a full movement quote.
    pub fn htm_departure_opening(
        &self,
        robot: RobotId,
        from: CellId,
        target: CellId,
        carrying: bool,
        depart: SimTime,
    ) -> Option<SimTime> {
        let mut exits = self.htm_exit_cells(from, target, carrying).peekable();
        if from == target || exits.peek().is_none() {
            return Some(depart);
        }
        exits
            .filter_map(|next| {
                let own = self.oracle.open_ended_reservation(next).filter(|hold| hold.robot() == robot);
                self.oracle.earliest_available(next, depart, SimDuration::ms(1), own.as_ref())
            })
            .min()
    }

    fn htm_exit_cells(&self, from: CellId, target: CellId, carrying: bool) -> impl Iterator<Item = CellId> + '_ {
        self.map
            .graph
            .neighbors(from)
            .into_iter()
            .filter(move |(_, edge)| {
                let allowed = match edge.traversability.htm_traversal {
                    HtmTraversal::None => false,
                    HtmTraversal::UnladenOnly => !carrying,
                    HtmTraversal::Both => true,
                };
                allowed && edge.dest_only.is_none_or(|gate| gate.admits(target))
            })
            .map(|(next, _)| next)
    }
}
