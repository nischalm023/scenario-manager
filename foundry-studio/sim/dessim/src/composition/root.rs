//! A JSON root selects a registered task and supplies instance parameters.
//! Deserialization checks the linked registration instead of a central enum.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::PlanError;
use crate::task::registered;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Entry")]
pub struct RootTask {
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(skip_serializing_if = "empty")]
    pub config: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    #[serde(rename = "type")]
    task_type: String,
    #[serde(default = "empty_config")]
    config: Value,
    #[serde(default)]
    after: Vec<String>,
}

pub(crate) fn empty_config() -> Value {
    Value::Object(Map::new())
}
fn empty(value: &Value) -> bool {
    value.as_object().is_some_and(Map::is_empty)
}

impl TryFrom<Entry> for RootTask {
    type Error = PlanError;
    fn try_from(entry: Entry) -> Result<Self, PlanError> {
        registered(&entry.task_type)?.validate_root(&entry.config)?;
        Ok(Self { task_type: entry.task_type, config: entry.config, after: entry.after })
    }
}

impl RootTask {
    pub fn name(&self) -> &str {
        &self.task_type
    }
}
