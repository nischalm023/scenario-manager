//! Bots are the model's floor robots — HTMs and RTPs. VTMs are not bots
//! here; they belong to [`crate::model::relay::vtm::VtmService`].
//!
//! BotService is the system of record for bot identity, drives and plans, and
//! answers the spatiotemporal availability queries that dispatch runs against.
//! The temporal plan (BotPlan) is the single source of truth for position,
//! carrying state, and availability — no snapshot fields.

pub mod plan;
#[cfg(test)]
mod plan_tests;
pub mod types;

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use desgraph::CellId;

use crate::model::ToteId;
use crate::model::inventory::InventoryService;
use crate::model::parking::ParkingRegistry;
use crate::model::{RobotId, SimTime};
use crate::pathfinding::DriveModel;
use crate::robots::runtime::{FloorRobotModel, RuntimeRobots};

use plan::{BotExpectation, Leg, PlanError, TaskKind, WaypointKind};
use plan::{BotPlan, PlanTaskId};
use types::BotData;
pub use types::BotKind;

use crate::pathfinding::ReservationRequest;

/// Describes a single leg to commit to a bot's plan.
///
/// `requests` are the cell-occupancy truth (one per reservation table entry).
/// `action_ready_arrival` is when the bot is action-ready at the leg's
/// destination — used by availability checks. For Move-derived requests it
/// equals the producing Move's `end`; for direct holds (no Move), it equals
/// the leg's last request's `start` (the request is already action-ready
/// framed by construction).
pub struct LegSpec<'a> {
    pub leg: Leg,
    pub requests: &'a [ReservationRequest],
    pub action_ready_arrival: SimTime,
    pub wp_kind: WaypointKind,
    pub expect: BotExpectation,
    pub carrying_after: Option<ToteId>,
}

/// Mutable state shared via Arc<RwLock<State>>.
struct State {
    bots: BTreeMap<RobotId, BotData>,
}

impl State {
    fn get(&self, id: RobotId) -> Option<&BotData> {
        self.bots.get(&id)
    }

    fn get_mut(&mut self, id: RobotId) -> Option<&mut BotData> {
        self.bots.get_mut(&id)
    }

    fn iter_kind(&self, kind: BotKind) -> impl Iterator<Item = (&RobotId, &BotData)> {
        self.bots.iter().filter(move |(_, d)| d.kind == kind)
    }
}

struct Inner {
    state: RwLock<State>,
    /// Immutable physical deployment, keyed by simulation robot identity.
    bindings: BTreeMap<RobotId, Arc<FloorRobotModel>>,
    legacy_model: Option<Arc<FloorRobotModel>>,
    models: Vec<Arc<FloorRobotModel>>,
}

/// Manages all bot state: identity, plans, and spatiotemporal availability.
///
/// Candidate selection does NOT live here: `pathfinding::nearest` prescreens by
/// manhattan lower bound and the task layer A*s the finalists. This service
/// answers position, availability and plan commitment only.
///
/// Clone is a cheap Arc bump.
#[derive(Clone)]
pub struct BotService(Arc<Inner>);

impl BotService {
    /// Explicit synthetic single-model service. Only registered robots acquire a model.
    pub fn new(model: Arc<FloorRobotModel>) -> Self {
        Self(Arc::new(Inner {
            state: RwLock::new(State { bots: BTreeMap::new() }),
            bindings: BTreeMap::new(),
            models: vec![model.clone()],
            legacy_model: Some(model),
        }))
    }

    /// Physical deployment bindings; missing robot ids are never filled from settings.
    pub fn from_runtime(models: &RuntimeRobots, vtm_count: usize) -> Self {
        let bindings = models
            .htm
            .iter()
            .enumerate()
            .map(|(index, model)| (RobotId((vtm_count + index) as u32), model.clone()))
            .collect();
        Self(Arc::new(Inner {
            state: RwLock::new(State { bots: BTreeMap::new() }),
            bindings,
            models: models.unique_htm_models(),
            legacy_model: None,
        }))
    }

    pub fn models(&self) -> &[Arc<FloorRobotModel>] {
        &self.0.models
    }

    pub fn model(&self, id: RobotId) -> Result<Arc<FloorRobotModel>, PlanError> {
        let state = self.0.state.read().unwrap();
        state.get(id).map(|bot| bot.model.clone()).or_else(|| self.0.bindings.get(&id).cloned()).ok_or_else(|| {
            PlanError::TaskGraphInvariant {
                context: "BotService::model",
                detail: format!("bot {id:?} has no model binding"),
            }
        })
    }

    /// Register a new bot at its starting position.
    ///
    /// Creates an initial Park waypoint at T=0 so the bot has a temporal
    /// position from the start. The plan is the single source of truth.
    ///
    /// VTMs do not register here — see [`crate::model::relay::vtm::VtmService`].
    pub fn register(&self, id: RobotId, kind: BotKind, pos: CellId) -> Result<(), PlanError> {
        if kind != BotKind::Htm {
            return Err(PlanError::TaskGraphInvariant {
                context: "BotService::register",
                detail: "VTMs are owned by VtmService".into(),
            });
        }
        let model = self.0.bindings.get(&id).cloned().or_else(|| self.0.legacy_model.clone()).ok_or_else(|| {
            PlanError::TaskGraphInvariant {
                context: "BotService::register",
                detail: format!("bot {id:?} has no model binding"),
            }
        })?;
        let mut state = self.0.state.write().unwrap();
        if state.bots.contains_key(&id) {
            return Err(PlanError::TaskGraphInvariant {
                context: "BotService::register",
                detail: format!("bot {id:?} is already registered"),
            });
        }
        let plans = BotPlan::new_at(id, pos);
        state.bots.insert(id, BotData { kind, model, plans });
        Ok(())
    }

    /// The drive bot `id` moves with in the load state `carrying_cargo`: the one a
    /// task passes when it reserves that bot's path. An unregistered bot fails as
    /// it does in [`Self::commit_task`].
    pub fn drive(&self, id: RobotId, carrying_cargo: bool) -> Result<Arc<dyn DriveModel>, PlanError> {
        let state = self.0.state.read().unwrap();
        let data = state.get(id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "BotService::drive",
            detail: format!("bot {id:?} not registered"),
        })?;
        Ok(data.model.execution[carrying_cargo as usize].clone())
    }

    /// Commit a task with one or more legs to a bot's plan. Gives the task's
    /// id, which [`Self::extend_task`] takes for a journey's later windows.
    pub fn commit_task(
        &self,
        id: RobotId,
        kind: TaskKind,
        legs: &[LegSpec<'_>],
        inventory: &InventoryService,
        parking: Option<&ParkingRegistry>,
    ) -> Result<PlanTaskId, PlanError> {
        let mut state = self.0.state.write().unwrap();
        let data = state.get_mut(id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "BotService::commit_task",
            detail: format!("bot {id:?} not registered"),
        })?;
        let task = data.plans.register_task(kind);
        append_legs(&mut data.plans, task, legs, inventory, parking)?;
        Ok(task)
    }

    /// Append the [`LegSpec`]s to `task` on a bot's plan: a journey committed
    /// in windows is one task.
    pub fn extend_task(
        &self,
        id: RobotId,
        task: PlanTaskId,
        legs: &[LegSpec<'_>],
        inventory: &InventoryService,
        parking: Option<&ParkingRegistry>,
    ) -> Result<(), PlanError> {
        let mut state = self.0.state.write().unwrap();
        let data = state.get_mut(id).ok_or_else(|| PlanError::TaskGraphInvariant {
            context: "BotService::extend_task",
            detail: format!("bot {id:?} not registered"),
        })?;
        append_legs(&mut data.plans, task, legs, inventory, parking)
    }

    /// Bot's planned position at a given time (read-only plan query).
    pub fn position_at(&self, id: RobotId, at: SimTime) -> Option<CellId> {
        let state = self.0.state.read().unwrap();
        state.get(id).and_then(|d| d.plans.position_at(at)).map(|wp| wp.cell)
    }

    /// Bot kind, or `None` when `id` was never registered.
    pub fn bot_kind(&self, id: RobotId) -> Option<BotKind> {
        let state = self.0.state.read().unwrap();
        state.get(id).map(|d| d.kind)
    }

    /// All bot IDs of a given kind with their frontier availability.
    /// Returns (id, available_at_ms, frontier_cell).
    ///
    /// NOTE: This is frontier-only (single timestamp), not interval-aware.
    /// If a plan contains future-assigned work with internal idle gaps, those
    /// gaps are not exposed here. Scheduler logic must enforce the
    /// interaction-time gap invariant or use an interval-aware query when
    /// gap-filling is required.
    pub fn all_availability(&self, kind: BotKind) -> Vec<(RobotId, u64, CellId)> {
        let state = self.0.state.read().unwrap();
        state
            .iter_kind(kind)
            .filter_map(|(&id, data)| {
                let (t, cell) = data.plans.availability()?;
                Some((id, t.as_ms(), cell))
            })
            .collect()
    }

    /// Total number of registered bots.
    pub fn len(&self) -> usize {
        let state = self.0.state.read().unwrap();
        state.bots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn count_kind(&self, kind: BotKind) -> usize {
        let state = self.0.state.read().unwrap();
        state.iter_kind(kind).count()
    }

    pub fn for_each<F>(&self, kind: BotKind, mut f: F)
    where
        F: FnMut(RobotId, &BotPlan),
    {
        let state = self.0.state.read().unwrap();
        for (&id, data) in &state.bots {
            if data.kind == kind {
                f(id, &data.plans);
            }
        }
    }

    pub fn for_each_bot<F>(&self, mut f: F)
    where
        F: FnMut(RobotId, BotKind, &BotPlan),
    {
        let state = self.0.state.read().unwrap();
        for (&id, data) in &state.bots {
            f(id, data.kind, &data.plans);
        }
    }
}

fn append_legs(
    plan: &mut BotPlan,
    task: PlanTaskId,
    legs: &[LegSpec<'_>],
    inventory: &InventoryService,
    parking: Option<&ParkingRegistry>,
) -> Result<(), PlanError> {
    for leg in legs {
        plan.append_leg(
            task,
            leg.leg,
            leg.requests,
            leg.action_ready_arrival,
            leg.wp_kind,
            leg.expect.clone(),
            leg.carrying_after,
            inventory,
            parking,
        )?;
    }
    Ok(())
}
