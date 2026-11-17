//! Within `studio-protocol`, `panel_event` is spec 01 section 3.6's panel event: a panel's own act told to the agent
//! as a `panel.event` entry, which starts no turn and which the next turn reads; one type is both the body a panel
//! posts and the entry's body, held to its kinds and its three fields wherever it is written. Each kind belongs to
//! the spec whose panel posts it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::journal::JournalSeq;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "panel_event.ts")]
pub enum PanelEventKind {
    #[serde(rename = "knob.changed")]
    KnobChanged,
    #[serde(rename = "flow.draft_changed")]
    FlowDraftChanged,
    #[serde(rename = "scenario.changed")]
    ScenarioChanged,
    #[serde(rename = "experiment.prepared")]
    ExperimentPrepared,
    #[serde(rename = "runs.submitted")]
    RunsSubmitted,
}

/// `panel.event`'s body, and `POST /api/conversations/{c}/panel-events`'s, which is journaled as it came: the kind
/// its panel's spec names, `text` the sentence the agent reads and the log shows, `body` the kind's own fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "panel_event.ts")]
pub struct PanelEvent {
    pub kind: PanelEventKind,
    pub text: String,
    pub body: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "panel_event.ts")]
pub struct PanelEventWritten {
    pub seq: JournalSeq,
}
