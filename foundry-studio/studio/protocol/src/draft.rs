//! Within `studio-protocol`, `draft` is a draft changeset as the journal tells it (spec 01 sections 3.5 and 3.11;
//! spec 02 section 3.1, ChangesetCard): each update with the first operations of every touched scope in words, and
//! the entry that ends it, applied, discarded or stale.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::grammar::{ScopePath, Seq, SitePath, Ulid};
use crate::problem::ProblemItem;
use crate::projection::DraftState;

/// Who checked a draft: Agent Studio's own checks, or world-proto-server's preview once it exists (spec 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "draft.ts")]
pub enum CheckedBy {
    AgentStudio,
    WorldProtoServer,
}

/// One operation in words, as the scope's owner writes it (spec 06 words a flow's): what it does, the key of what it
/// touches, that thing's name, and for a change the field with its value before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "draft.ts")]
pub struct DraftOp {
    pub op: String,
    pub key: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub was: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub becomes: Option<String>,
}

/// One touched scope (ruling D72): how many operations it holds, and the first 12 of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "draft.ts")]
pub struct DraftPart {
    pub scope: ScopePath,
    pub ops_total: u32,
    pub ops: Vec<DraftOp>,
}

/// `draft.updated`'s body: the site root it was initiated on, each written scope's base, its parts, who checked it,
/// what the check found, and its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "draft.ts")]
pub struct DraftUpdated {
    pub draft: Ulid,
    pub initiated_on: SitePath,
    // ordered: the card names the scopes in path order.
    pub bases: BTreeMap<ScopePath, Seq>,
    pub ops_total: u32,
    pub parts: Vec<DraftPart>,
    pub checked_by: CheckedBy,
    pub problems: Vec<ProblemItem>,
    pub state: DraftState,
}

/// `draft.applied`'s body: each touched scope's new changeset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "draft.ts")]
pub struct DraftApplied {
    pub draft: Ulid,
    // ordered: "Applied as flow changeset 2 and map changeset 1" names them in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "draft.ts")]
pub struct DraftDiscarded {
    pub draft: Ulid,
}

/// Why a draft can no longer be applied (spec 01 section 3.11), each with spec 04 section 3.8's sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "draft.ts")]
pub enum StaleReason {
    HeadMoved,
    Refused,
    GeneratorsMoved,
}

/// `draft.stale`'s body, with the problem body (spec 01 section 3.14) that says it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "draft.ts")]
pub struct DraftStale {
    pub draft: Ulid,
    pub reason: StaleReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub problem: Option<Value>,
}
