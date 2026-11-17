use std::cell::RefCell;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{find_registration, invalid, task_name};
use crate::error::PlanError;
use crate::scheduler::task::Task;
use crate::state::{StateManager, States};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskDefaults(pub FxHashMap<String, Value>);

impl TaskDefaults {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Child types are deliberately unregistered: their names and fields cannot
    /// all be checked here. Validate their resolved fields when constructed.
    pub fn validate(&self) -> Result<(), PlanError> {
        let mut names: Vec<_> = self.0.keys().collect();
        names.sort();
        for name in names {
            let config = &self.0[name];
            if !config.is_object() {
                return Err(invalid(format!("{name} defaults must be an object")));
            }
            if let Some(root) = find_registration(name)? {
                root.validate(config)?;
            }
        }
        Ok(())
    }

    pub fn resolve(&self, name: &str, overrides: &Value) -> Result<Value, PlanError> {
        let mut config = match self.0.get(name) {
            Some(Value::Object(fields)) => fields.clone(),
            None => Map::new(),
            Some(_) => return Err(invalid(format!("{name} defaults must be an object"))),
        };
        let fields = overrides.as_object().ok_or_else(|| invalid(format!("{name} config must be an object")))?;
        config.extend(fields.clone());
        Ok(Value::Object(config))
    }
}

pub struct TaskContext {
    name: &'static str,
    states: States,
    config: Value,
    consumed: RefCell<FxHashSet<String>>,
}

impl TaskContext {
    pub fn new<T: Task>(states: States, defaults: &TaskDefaults, overrides: Value) -> Result<Self, PlanError> {
        let name = task_name::<T>();
        let config = defaults.resolve(name, &overrides)?;
        T::validate_config(&config)?;
        Ok(Self { name, states, config, consumed: RefCell::default() })
    }

    pub fn parameter<T: DeserializeOwned>(&self, name: &str, default: T) -> Result<T, PlanError> {
        self.consumed.borrow_mut().insert(name.into());
        match self.config.get(name) {
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|e| invalid(format!("{}.{name}: {e}", self.name)))
            }
            None => Ok(default),
        }
    }

    /// Optional for tasks with a substantial configuration object. Simple tasks
    /// can read individual parameters and need no companion config type.
    pub fn config<T: DeserializeOwned>(&self) -> Result<T, PlanError> {
        let config = serde_json::from_value(self.config.clone()).map_err(|e| invalid(format!("{}: {e}", self.name)))?;
        self.consumed.borrow_mut().extend(self.config.as_object().unwrap().keys().cloned());
        Ok(config)
    }

    pub fn state<M: StateManager>(&self) -> Result<Arc<M>, PlanError> {
        Ok(self.states.get::<M>()?)
    }

    pub fn states(&self) -> &States {
        &self.states
    }

    pub fn finish(&self) -> Result<(), PlanError> {
        let consumed = self.consumed.borrow();
        let mut unknown: Vec<_> =
            self.config.as_object().unwrap().keys().filter(|name| !consumed.contains(*name)).collect();
        unknown.sort();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(invalid(format!("{} has unused configuration fields: {unknown:?}", self.name)))
        }
    }
}
