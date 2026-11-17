//! Within `studio-protocol`, `dev` is the test writer's door (brief I1.04 section 5, lead decided): with
//! `STUDIO_ENV=local`, `POST /api/dev/conversations/{c}/entries` hands a developer's entry or live item to the same
//! writer and hub the agent loop uses, through which the browser tests replay the journal fixtures; a cluster never
//! mounts it. A fixture line is a `JournalEntry` or a `DevLive`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::grammar::Ulid;
use crate::journal::{JournalEntry, JournalKind, JournalSeq};
use crate::stream::LiveItem;

/// An entry as its writer chooses it; the number, the wall-clock `at` and the conversation are the journal's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "dev.ts")]
pub struct DevEntry {
    pub kind: JournalKind,
    pub body: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub caused_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dedup_key: Option<String>,
}

/// A live-only item for the hub, which no journal keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "dev.ts")]
pub struct DevLive {
    pub live: LiveItem,
}

/// The test writer's body: a live item, or an entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "dev.ts")]
pub enum DevWrite {
    Live(DevLive),
    Entry(DevEntry),
}

/// The test writer's answer for an entry: `201` with the entry written, or `200` naming the entry whose dedup key it
/// repeated, and nothing written. A live item answers `202` with no body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "dev.ts")]
pub enum DevWritten {
    Wrote { entry: Box<JournalEntry> },
    Held { held: JournalSeq },
}
