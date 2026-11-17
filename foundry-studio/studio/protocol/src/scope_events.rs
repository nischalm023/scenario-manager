//! Within `studio-protocol`, `scope_events` is what `GET /api/scope-events` sends a page that follows scopes
//! (spec 01 section 3.6): `scope.moved` when a named scope lands a changeset, so the page reads it again, and `ping`
//! every 15 s of wall clock, the conversation stream's `Ping`. Neither carries an `id:` line: the stream is live only,
//! and a page that reconnects reads the heads again rather than replaying.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::{ScopePath, Seq};

/// The events the stream sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "scope_events.ts")]
pub enum ScopeEventKind {
    #[serde(rename = "scope.moved")]
    ScopeMoved,
    #[serde(rename = "ping")]
    Ping,
}

impl ScopeEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScopeMoved => "scope.moved",
            Self::Ping => "ping",
        }
    }
}

/// `scope.moved`'s body: `scope` moved to changeset `seq`; `gap` when changesets between the last one this stream
/// told and `seq` went untold, which a page that only re-reads may ignore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scope_events.ts")]
pub struct ScopeMoved {
    pub scope: ScopePath,
    pub seq: Seq,
    pub gap: bool,
}
