//! Within `studio-protocol`, `page` is spec 03 section 3.4, the page context: what a person was looking at when they
//! sent a message, shown above the composer before it is sent and carried as the message's `context`, so the agent
//! reads the changesets the person saw rather than whatever the head is when the turn runs. A panel switch is never
//! sent on its own (consolidated question 9, lead decided); it rides with the next message.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::Key;
use crate::journal::JournalSeq;
use crate::panel::ScopeSeq;

/// The view a page context was taken on: the five panels a command opens, and Home, the Template list and History,
/// which no command opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "page.ts")]
pub enum ContextPanel {
    Home,
    Templates,
    Template,
    Map,
    Flow,
    History,
    Scenario,
    Runs,
}

/// The one selected thing on the panel: the key its elements carry, and its label as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "page.ts")]
pub struct ContextSelection {
    pub key: Key,
    pub label: String,
}

/// `context` on a message and on a panel act that posts a user entry: the canonical address, the panel, its heading
/// as shown ("Map: Small L-shape, 6 aisles, map at changeset 1"), the cut it rendered (empty for a view of Postgres
/// objects only), the selection, and the journal number of the agent's highlights on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "page.ts")]
pub struct PageContext {
    pub route: String,
    pub panel: ContextPanel,
    pub title: String,
    pub seen: Vec<ScopeSeq>,
    pub selection: Option<ContextSelection>,
    pub highlights: Option<JournalSeq>,
}
