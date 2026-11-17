//! Within `studio-protocol`, `made_from` is Create site's request and answer, and the made-from record a site keeps
//! of each act that made or updated it from a template (spec 05 sections 3.7 and 3.8); every `at` is wall clock.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::approval::ApprovalRecord;
use crate::clock::WallClock;
use crate::grammar::{ScopePath, Seq, SitePath, TagName, Ulid};
use crate::person::Person;
use crate::template::{KnobUnit, KnobValue};
use crate::template_draft::{DraftMode, EvaluationNumber};

/// `POST /api/template-drafts/{d}/site`'s body: the new site's customer slug, its slug and its name, and the
/// evaluation to write, which must be the draft's latest ready one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "made_from.ts", optional_fields)]
pub struct CreateSite {
    pub customer: String,
    pub slug: String,
    pub name: String,
    pub evaluation: EvaluationNumber,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

/// `202` to a write: the person's approval it runs under, as Apply's and Run's `202` answer theirs (spec 01 section
/// 3.10, rule 6: the buttons answer alike), whose `approval.execution` reports its end; and the draft it writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "made_from.ts")]
pub struct WriteAccepted {
    pub approval: ApprovalRecord,
    pub draft: Ulid,
}

/// A knob value as a made-from record shows it, with the label and unit the template declared at the changesets it
/// was read at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "made_from.ts")]
pub struct MadeFromKnob {
    pub key: String,
    pub label: String,
    pub unit: KnobUnit,
    pub value: KnobValue,
}

/// One act that made or updated a site from a template (section 3.7's `MadeFrom`), never overwritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "made_from.ts")]
pub struct MadeFrom {
    pub id: Ulid,
    pub site: SitePath,
    pub act: DraftMode,
    pub template: ScopePath,
    pub tag: Option<TagName>,
    // ordered: scopes are listed in path order.
    pub template_seqs: BTreeMap<ScopePath, Seq>,
    pub knobs: Vec<MadeFromKnob>,
    pub site_seqs: BTreeMap<ScopePath, Seq>,
    pub at: WallClock,
    pub author: Person,
    pub draft: Ulid,
    pub evaluation: EvaluationNumber,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "made_from.ts")]
pub struct MadeFromList {
    pub items: Vec<MadeFrom>,
}
