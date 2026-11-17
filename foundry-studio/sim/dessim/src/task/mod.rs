//! Registration and construction of scenario-selected tasks. Descriptors are
//! static; root factories are synchronous, with construction deferred only for
//! declared prerequisites. Children use ordinary constructors and can read the
//! same run defaults without registration. Tasks retain individual managers.

mod context;
mod deferred;
#[cfg(test)]
mod stand_ins;

use std::any::type_name;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::PlanError;
use crate::scheduler::Scheduler;
use crate::scheduler::task::{Task, TaskDependency};

pub use context::{TaskContext, TaskDefaults};
pub(crate) use deferred::Deferred;
pub use linkme as __linkme;
pub use linkme::distributed_slice as registration_slice;

pub(crate) fn task_name<T: Task>() -> &'static str {
    type_name::<T>().split('<').next().unwrap().rsplit("::").next().unwrap()
}

type SpawnRoot = fn(&mut Scheduler, Value, Vec<TaskDependency>) -> Result<TaskDependency, PlanError>;

pub struct Registration {
    pub name: &'static str,
    validate: fn(&Value) -> Result<(), PlanError>,
    spawn: SpawnRoot,
    finite: fn() -> bool,
}

impl Registration {
    pub const fn of<T: Task + 'static>(name: &'static str) -> Self {
        Self { name, validate: T::validate_config, spawn: spawn::<T>, finite: T::holds_completion }
    }

    pub fn validate(&self, config: &Value) -> Result<(), PlanError> {
        (self.validate)(config)
    }

    pub fn validate_root(&self, config: &Value) -> Result<(), PlanError> {
        self.validate(config)
    }

    pub fn holds_completion(&self) -> bool {
        (self.finite)()
    }

    pub fn spawn(&self, scheduler: &mut Scheduler, config: Value) -> Result<TaskDependency, PlanError> {
        self.spawn_after(scheduler, config, Vec::new())
    }

    pub fn spawn_after(
        &self,
        scheduler: &mut Scheduler,
        config: Value,
        after: Vec<TaskDependency>,
    ) -> Result<TaskDependency, PlanError> {
        (self.spawn)(scheduler, config, after)
    }
}

fn spawn<T: Task + 'static>(
    scheduler: &mut Scheduler,
    config: Value,
    after: Vec<TaskDependency>,
) -> Result<TaskDependency, PlanError> {
    Ok(scheduler.add_registered_after::<T>(config, after)?.dependency())
}

#[registration_slice]
pub static REGISTRATIONS: [Registration];

pub fn registered(name: &str) -> Result<&'static Registration, PlanError> {
    find_registration(name)?.ok_or_else(|| invalid(format!("unknown task {name:?}")))
}

pub(crate) fn find_registration(name: &str) -> Result<Option<&'static Registration>, PlanError> {
    let mut matches = REGISTRATIONS.iter().filter(|entry| entry.name == name);
    let entry = matches.next();
    if matches.next().is_some() {
        return Err(invalid(format!("task {name:?} is registered more than once")));
    }
    Ok(entry)
}

pub fn invalid(detail: impl Into<String>) -> PlanError {
    PlanError::TaskGraphInvariant { context: "task configuration", detail: detail.into() }
}

pub fn parse_config<T: DeserializeOwned>(config: &Value) -> Result<T, PlanError> {
    serde_json::from_value(config.clone()).map_err(|error| invalid(error.to_string()))
}
