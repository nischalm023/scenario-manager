//! A composition is a scenario's selection of top-level task specifications.
//! Named selections are JSON documents in dessim/compositions; this module
//! resolves them, validates their wiring, and hands their roots to the scheduler.
//! Task construction and state-manager factories remain independent of the names.

mod dependencies;
pub mod flowgraph;
mod model;
mod parameters;
pub mod policy;
pub mod resources;
mod root;

use std::sync::Arc;

use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::config::SimConfig;
use crate::error::PlanError;
use crate::events::EventSink;
use crate::model::ids::ToteId;
use crate::pathfinding::Pathfinder;
use crate::reporting::RunReports;
use crate::scenario_gen::ScenarioData;
use crate::scheduler::Scheduler;
use crate::simtime::{SimDuration, SimOffset};
use desgraph::StationType;

use crate::task::{TaskDefaults, parse_config, registered};
pub use model::validate_admission_config;
pub use parameters::{
    CloseOutTaskConfig, InductionSequence, InductionTaskConfig, InventoryTaskConfig, PickTaskConfig, SupplyTaskConfig,
};
use resources::{InventorySeed, RunInputs};
pub use root::RootTask;
use root::empty_config;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Composition {
    Named(String),
    Inline(Definition),
}

impl Default for Composition {
    fn default() -> Self {
        Self::Named("cencora".into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub tasks: Vec<RootTask>,
    #[serde(default, skip_serializing_if = "TaskDefaults::is_empty")]
    pub task_defaults: TaskDefaults,
}

fn invalid(detail: impl Into<String>) -> PlanError {
    PlanError::TaskGraphInvariant { context: "composition", detail: detail.into() }
}

impl Composition {
    pub fn resolve(&self) -> Result<Definition, PlanError> {
        let definition = match self {
            Self::Inline(definition) => definition.clone(),
            Self::Named(name) => {
                let json = match name.as_str() {
                    "cencora" => include_str!("../../compositions/cencora.json"),
                    "walmart" => include_str!("../../compositions/walmart.json"),
                    _ => {
                        return Err(invalid(format!(
                            "unknown composition {name:?}; expected cencora, walmart, or an inline task list"
                        )));
                    }
                };
                serde_json::from_str(json).map_err(|e| invalid(format!("composition {name}: {e}")))?
            }
        };
        definition.validate()?;
        Ok(definition)
    }
}

impl Definition {
    pub fn validate(&self) -> Result<(), PlanError> {
        self.task_defaults.validate()?;
        let mut names = FxHashSet::default();
        for task in &self.tasks {
            registered(task.name())?.validate_root(&self.config(task.name())?)?;
            if !names.insert(task.name()) {
                return Err(invalid(format!("{} is a run-wide root and appears twice", task.name())));
            }
        }
        if names.contains("LoopDeposit") != names.contains("LoopCollect") {
            return Err(invalid("LoopDeposit and LoopCollect must be composed together"));
        }
        self.ordered_roots()?;
        Ok(())
    }

    pub(crate) fn config(&self, name: &str) -> Result<Value, PlanError> {
        let overrides = self
            .tasks
            .iter()
            .find(|task| task.name() == name)
            .map(|task| task.config.clone())
            .unwrap_or_else(empty_config);
        self.task_defaults.resolve(name, &overrides)
    }

    pub fn parameters<T: DeserializeOwned>(&self, name: &str) -> Result<T, PlanError> {
        parse_config(&self.config(name)?)
    }

    pub fn induction_period(&self, settings: &SimConfig) -> Result<SimDuration, PlanError> {
        policy::induction_period(settings, &self.parameters("PutawayInduction")?)
    }

    fn supply_bias(&self, settings: &SimConfig) -> Result<Option<f64>, PlanError> {
        Ok(self
            .parameters::<SupplyTaskConfig>("PutawaySupplyDemand")?
            .direct_bias_secs
            .or(settings.putaway.direct_bias_secs))
    }

    fn validate_parameters(&self, settings: &SimConfig) -> Result<(), PlanError> {
        for task in &self.tasks {
            match task.name() {
                "PickDemand" => {
                    let config: PickTaskConfig = self.parameters(task.name())?;
                    policy::pick(settings, &config)?;
                    policy::pick_start(settings, &config)?;
                }
                "PutawayInduction" | "PutawaySupplyDemand" => {
                    self.induction_period(settings)?;
                    if self.supply_bias(settings)?.is_some_and(|secs| !secs.is_finite()) {
                        return Err(invalid("PutawaySupplyDemand.direct_bias_secs must be finite"));
                    }
                }
                "OrderCloseOut" => {
                    let config: CloseOutTaskConfig = self.parameters(task.name())?;
                    policy::duration(
                        "OrderCloseOut.buffer_dwell_secs",
                        config.buffer_dwell_secs.unwrap_or(settings.order.buffer_dwell_secs),
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Publish input data and register the composition's roots. Managers are
/// constructed only when tasks request them; reports never initiate construction.
pub fn build(
    data: &mut ScenarioData,
    config: &SimConfig,
    pathfinder: Arc<dyn Pathfinder>,
    event_sink: Option<Arc<dyn EventSink>>,
) -> Result<(Scheduler, RunReports, SimOffset), PlanError> {
    let definition = config.composition.resolve()?;
    if definition.tasks.iter().any(|t| t.name() == "PickDemand")
        && data.station_data.iter().any(|s| s.conveyor.is_some())
        && !definition.tasks.iter().any(|t| t.name() == "LoopDeposit")
    {
        return Err(invalid(
            "For EasyLoop fulfillment, select composition \"walmart\"; to retain PickDemand at conveyor-backed stations, explicitly compose both LoopDeposit and LoopCollect",
        ));
    }
    definition.validate_parameters(config)?;
    if let Some(n) = config.vtm.per_zone_override {
        model::set_vtms_per_zone(data, n);
    }
    let mut scheduler = Scheduler::new()
        .with_barrier_enabled(config.barrier.enabled)
        .with_task_defaults(definition.task_defaults.clone())?;
    model::configure_world(&scheduler.states(), data, config, pathfinder, event_sink, scheduler.barrier().clock())?;
    let bias = if definition.tasks.iter().any(|task| task.name() == "PutawaySupplyDemand") {
        policy::direct_bias(
            data.station_data.iter().any(|station| station.station_type == StationType::Putaway),
            &data.map,
            definition.supply_bias(config)?,
        )?
    } else {
        SimOffset::ZERO
    };
    scheduler.states().configure(InventorySeed {
        placements: data.tote_placements.clone(),
        contents: data.inventory.iter().map(|(&(sku, space), &qty)| (sku, ToteId(space.0), qty)).collect(),
    })?;
    scheduler.states().configure(RunInputs {
        definition: definition.clone(),
        config: config.clone(),
        orders: data.orders.clone(),
        skus: data.skus.clone(),
        demand_pool: data.demand_pool.take(),
    })?;
    eprintln!(
        "[composition] {:?}: {}",
        config.composition,
        definition.tasks.iter().map(RootTask::name).collect::<Vec<_>>().join(", ")
    );
    definition.spawn(&mut scheduler)?;
    let reports = RunReports::new(scheduler.states());
    Ok((scheduler, reports, bias))
}
