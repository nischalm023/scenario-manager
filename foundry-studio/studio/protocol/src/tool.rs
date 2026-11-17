//! Within `studio-protocol`, `tool` is a tool call's entries (spec 01 section 3.5): its start, its progress notes and
//! its end, each naming the call by the id the API minted for it, as spec 02's ToolRow draws them in place.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::projection::{CallId, CallState};

/// `tool.started`'s body: the tool, its approval class, and the call's summary in the tool's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "tool.ts")]
pub struct ToolStarted {
    pub call: CallId,
    pub tool: String,
    pub class: String,
    pub summary: String,
}

/// `tool.progress`'s body: the latest note of a call still running, and how far it is when the tool can say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "tool.ts")]
pub struct ToolProgress {
    pub call: CallId,
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fraction: Option<f64>,
}

/// `tool.finished`'s body: how the call ended, its summary (which carries a failure's error), the references it
/// resolved, and the problem body (spec 01 section 3.14) of a call that did not end `ok`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "tool.ts")]
pub struct ToolFinished {
    pub call: CallId,
    pub outcome: CallState,
    pub summary: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub problem: Option<Value>,
}
