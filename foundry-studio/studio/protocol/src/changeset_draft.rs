//! Within `studio-protocol`, `changeset_draft` is spec 01 section 3.11's draft changeset as its routes carry it, in
//! the first cut a person's own: the operations an edit adds (patches only until declare and delete arrive with the
//! agent's `flow.draft`), what a draft answers after each edit with its problems on the complete would-be site, and
//! the draft as read; the press that applies it answers its approval (`approval`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::grammar::{EntityId, ScopePath, Seq, Ulid};
use crate::problem::ProblemItem;
use crate::projection::DraftState;

/// One operation of a draft: one field of one flow entity or map sector set to `value`, the pointer opening with
/// the component's name (`/flow.node/intake/0/admission_factor`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "lowercase")]
#[ts(export_to = "changeset_draft.ts")]
pub enum ChangesetOp {
    Patch { target: EntityId, pointer: String, value: Value },
}

/// `POST /api/drafts`: the site as the panel read it, the changeset each of its scopes was read at (the read's own
/// `seqs`, which the draft is built on, ruling D43), the summary and the first operations. The draft's events go to
/// the site's live conversation, which the API finds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "changeset_draft.ts")]
pub struct NewDraft {
    pub site: String,
    pub seqs: BTreeMap<ScopePath, Seq>, // ordered: user-visible JSON
    pub summary: String,
    pub ops: Vec<ChangesetOp>,
}

/// `PATCH /api/drafts/{d}`: more operations at the revision the panel last read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "changeset_draft.ts")]
pub struct AmendDraft {
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<String>,
    pub ops: Vec<ChangesetOp>,
}

/// How many operations a draft holds for one scope it writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "changeset_draft.ts")]
pub struct ChangesetPart {
    pub scope: ScopePath,
    pub ops: u32,
}

/// What `POST /api/drafts` and `PATCH /api/drafts/{d}` answer: the draft at its new revision, its state, its
/// operations counted, and every problem of the complete would-be site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "changeset_draft.ts")]
pub struct DraftChecked {
    pub draft: Ulid,
    pub revision: u32,
    pub state: DraftState,
    pub ops_total: u32,
    pub parts: Vec<ChangesetPart>,
    pub problems: Vec<ProblemItem>,
    /// Each handle a declaration named and the id minted for it; empty while drafts hold patches only.
    pub minted: BTreeMap<String, EntityId>, // ordered: user-visible JSON
}

/// A draft as `GET /api/drafts/{d}` reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "changeset_draft.ts")]
pub struct DraftRead {
    pub draft: Ulid,
    pub site: ScopePath,
    pub revision: u32,
    pub state: DraftState,
    /// For a stale draft, why: `head-moved` or `refused`, with its sentence.
    pub stale_reason: Option<Value>,
    pub summary: String,
    /// Every scope of the site at the changeset the draft was built on.
    pub cut: BTreeMap<ScopePath, Seq>, // ordered: user-visible JSON
    pub ops: Vec<ChangesetOp>,
    pub parts: Vec<ChangesetPart>,
    pub problems: Vec<ProblemItem>,
    /// The press that applies it: while `applying`, the approval claiming it; then the one that applied it or found
    /// its scopes moved. A press a refusal ended is released, and the draft is open again.
    pub approval: Option<Ulid>,
    pub applied_seqs: Option<BTreeMap<ScopePath, Seq>>,
}

/// `GET /api/drafts?site=&mine=1`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "changeset_draft.ts")]
pub struct DraftList {
    pub items: Vec<DraftRead>,
}
