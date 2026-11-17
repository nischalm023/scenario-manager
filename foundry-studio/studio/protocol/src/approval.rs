//! Within `studio-protocol`, `approval` is a person's yes to an act (spec 01 sections 3.5, 3.6 and 3.10): what the
//! agent asks, the decision, the moved-since note, each change of the approved act's execution, the bodies a card
//! posts, and the approval as its routes answer it, which a page with no conversation attached reads once a second
//! (wall clock) after a press until its execution ends. An execution entry names whose yes it carries out and, for an
//! apply, the draft it applies, so a browser tab that did not press, or one reloaded, draws the same card as the tab
//! that pressed (spec 02 section 3.2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::WallClock;
use crate::grammar::{ScopePath, Seq, Ulid};
use crate::person::Person;
use crate::projection::{ApprovalState, CallId, Execution};

/// One row of a card's facts; consecutive rows of one label share it (spec 02 section 3.1, ApprovalCard).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct FactRow {
    pub label: String,
    pub value: String,
}

/// What the card says before any button: its lead, its rows, its notes, the approve button's words and a footnote
/// when the act's owner writes one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalFacts {
    pub lead: String,
    pub display: Vec<FactRow>,
    pub notes: Vec<String>,
    pub action_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub footnote: Option<String>,
}

/// `approval.requested`'s body: the asking call, the stored input that alone executes, and the card's facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalRequested {
    pub approval: Ulid,
    pub call: Option<CallId>,
    pub tool: String,
    pub class: String,
    pub summary: String,
    pub input: Value,
    pub facts: ApprovalFacts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "approval.ts")]
pub enum Decision {
    Approve,
    Deny,
}

/// `approval.decided`'s body, and its reason when the person denied with one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalDecided {
    pub approval: Ulid,
    pub decision: Decision,
    pub author: Person,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

/// `approval.expired`'s body: the turn was stopped, or its wait ran out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalExpired {
    pub approval: Ulid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "approval.ts")]
pub enum NoteKind {
    Moved,
}

/// `approval.noted`'s body, journaled once, when a scope the stored plan names first moves past its planned
/// changeset; spec 08 writes the note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalNoted {
    pub approval: Ulid,
    pub kind: NoteKind,
    pub note: String,
}

/// The changesets that landed matching the act which the check cannot prove were this approval's (spec 04's
/// `Unattributable`); the approver answers with `POST /api/approvals/{a}/attribution`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct Unattributable {
    // ordered: the card names the scopes in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
}

/// `approval.execution`'s body, journaled on each change of an approved act's execution, a button's included: the
/// act owner's `sentence` when it failed or is not yet known; `approver`, whose yes it carries out (the person who
/// pressed a button, else the one who decided); and `draft`, the changeset draft an apply executes (spec 01 section
/// 3.11), absent for any other act. A button's approval journals nothing before this entry (spec 01 section 3.10,
/// rule 6), so these two fields are the only record of who pressed and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalExecuted {
    pub approval: Ulid,
    pub execution: Execution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sentence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unattributable: Option<Unattributable>,
    pub approver: Person,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub draft: Option<Ulid>,
}

/// `POST /api/approvals/{a}/decision`'s body; the reason only with a denial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct PostDecision {
    pub decision: Decision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

/// `POST /api/approvals/{a}/attribution`'s body: whether the landed change was this approval's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct PostAttribution {
    pub was_this: bool,
}

/// How an approval was given: on the agent's card, or by a person's own press in a panel (rule 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "approval.ts")]
pub enum ApprovalVia {
    Card,
    Button,
}

/// An approval: its tool and class, how it was given, its state and its execution, with the sentence of a failed or
/// unknown execution and what the act answered once it ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalRecord {
    pub id: Ulid,
    pub conversation: Option<Ulid>,
    pub tool: String,
    pub class: String,
    pub via: ApprovalVia,
    pub state: ApprovalState,
    pub execution: Option<Execution>,
    pub sentence: Option<String>,
    pub outcome: Option<Value>,
    pub requested_at: WallClock,
    pub decided_by: Option<Person>,
    pub decided_at: Option<WallClock>,
}

/// The approval routes' answer: Apply's and Run's `202`, a decision's and an attribution's `200`, and
/// `GET /api/approvals/{a}` (spec 01 section 3.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "approval.ts")]
pub struct ApprovalAnswer {
    pub approval: ApprovalRecord,
}
