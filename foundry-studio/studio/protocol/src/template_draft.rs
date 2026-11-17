//! Within `studio-protocol`, `template_draft` is a template draft as its routes and its stream items carry it (spec 05
//! sections 3.8, 3.10 and 3.12): starting one, its knobs with who set each and when, a knob change and its answer,
//! the live `template.evaluation`, the `knob.changed` panel event, and a write's end.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::grammar::{ScopePath, Seq, SitePath, TagName, Ulid};
use crate::person::Person;
use crate::problem::Problem;
use crate::template::{EvaluationProblem, KnobDeclaration, KnobValue};

/// An evaluation's number within its draft: 1 for the first, and each next one given under the draft's row lock, so
/// none is shared or skipped (section 3.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct EvaluationNumber(pub u32);

impl Display for EvaluationNumber {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "template_draft.ts")]
pub enum TemplateDraftState {
    Open,
    Writing,
    Created,
    Updated,
    Abandoned,
    Expired,
}

/// Whether a draft makes a new site or writes the template again over a made one (update, after 23 October).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "template_draft.ts")]
pub enum DraftMode {
    Create,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "template_draft.ts")]
pub enum EvaluationState {
    InProgress,
    Ready,
    Failed,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "template_draft.ts")]
pub enum SetByKind {
    Template,
    Agent,
    User,
}

/// Who set a knob's value and in which revision: the template's default, the agent in a turn, or a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct KnobSetBy {
    pub by: SetByKind,
    pub turn: Option<u32>,
    pub author: Option<Person>,
    pub revision: u32,
}

/// An evaluation by its number within its draft, and its state when the answer was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct EvaluationRef {
    pub n: EvaluationNumber,
    pub state: EvaluationState,
}

/// A template as a draft read it: its root scope, its tag (null until tags), and each of its scopes' changesets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct TemplateAt {
    #[serde(rename = "ref")]
    pub ref_: ScopePath,
    pub tag: Option<TagName>,
    // ordered: a template's scopes are listed in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
}

/// `POST /api/template-drafts`'s body: a template location (`template/site/l-shape@main`), or a made site for an
/// update draft; the New site conversation the draft runs in, when one is attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "template_draft.ts", optional_fields)]
pub struct StartDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

/// `201` to a started draft: its first evaluation runs at the template's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct DraftStarted {
    pub draft: Ulid,
    pub revision: u32,
    pub evaluation: EvaluationRef,
}

/// What a write made: the site, each of its scopes' changesets, and its made-from record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct SiteMade {
    pub site: SitePath,
    // ordered: a site's scopes are listed in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
    pub made_from: Ulid,
}

/// A draft's write from the press on: the approval it runs under, the evaluation it writes, and once it ends either
/// what it made or the problem that refused it, the draft `open` again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct DraftWrite {
    pub approval: Ulid,
    pub evaluation: EvaluationNumber,
    pub result: Option<SiteMade>,
    pub problem: Option<Problem>,
}

/// `GET /api/template-drafts/{d}`'s answer: the draft's state and revision, the template as it read it with its
/// title, the values a person or the agent set (every other knob follows its default, D120), who set each knob, the
/// knob declarations, its newest evaluation and its write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct TemplateDraft {
    pub draft: Ulid,
    pub state: TemplateDraftState,
    pub mode: DraftMode,
    pub revision: u32,
    pub template: TemplateAt,
    pub title: String,
    pub conversation: Option<Ulid>,
    // ordered: knobs are echoed in key order.
    pub knobs: BTreeMap<String, KnobValue>,
    pub set_by: BTreeMap<String, KnobSetBy>,
    pub declarations: Vec<KnobDeclaration>,
    pub latest: Option<EvaluationRef>,
    pub write: Option<DraftWrite>,
}

/// `PATCH /api/template-drafts/{d}`'s body: the values to set, read at `revision`; `null` returns a knob to its
/// default, which an expression default follows again (D120).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "template_draft.ts")]
pub struct SetKnobs {
    pub revision: u32,
    // ordered: knobs are echoed in key order.
    pub knobs: BTreeMap<String, Option<KnobValue>>,
}

/// The answer to a knob change and to evaluating again: the draft's revision and the evaluation it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct KnobsSet {
    pub draft: Ulid,
    pub revision: u32,
    pub evaluation: EvaluationRef,
}

/// `template.evaluation`'s body, live on the draft's conversation and never journaled: the panel takes the knob
/// values of `revision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct EvaluationEvent {
    pub draft: Ulid,
    pub n: EvaluationNumber,
    pub revision: u32,
    pub state: EvaluationState,
    pub problems: Vec<EvaluationProblem>,
}

/// One knob's change, `null` standing for its default: set from its default, or returned to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct KnobChange {
    pub knob: String,
    pub from: Option<KnobValue>,
    pub to: Option<KnobValue>,
}

/// The `body` of a `panel.event` of kind `knob.changed`: every change a person made since the last such entry, and
/// the evaluation that finished with them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template_draft.ts")]
pub struct KnobsChanged {
    pub draft: Ulid,
    pub changes: Vec<KnobChange>,
    pub evaluation: EvaluationNumber,
    pub problems: Vec<EvaluationProblem>,
}
