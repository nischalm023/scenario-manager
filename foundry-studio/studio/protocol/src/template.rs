//! Within `studio-protocol`, `template` is spec 05's site templates as the Template list and one template show them
//! (sections 3.2, 3.6 and 3.8): a template's card and knob declarations, its default evaluation's thumbnail with its
//! census and known problems. Lengths are whole millimetres and counts whole numbers (ruling D74).

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::{ScopePath, Seq, TagName};
use crate::problem::Severity;

/// A knob's unit: a count, whole millimetres, or degrees in quarter turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "template.ts")]
pub enum KnobUnit {
    Count,
    Mm,
    Deg,
}

/// The integer a knob is set to, in its own unit: a count, whole millimetres, or degrees in quarter turns (ruling
/// D74); the engine's knob value too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct KnobValue(pub i64);

impl Display for KnobValue {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct KnobOption {
    pub value: KnobValue,
    pub label: String,
}

/// What a knob nobody set stands at: one of its valid values, or an expression over the other knobs that each
/// evaluation works out (section 3.2; D120), such as an outline that follows the aisles and stations it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "template.ts")]
pub enum KnobDefault {
    Value(KnobValue),
    Expression(String),
}

/// One knob as its template declares it (`template.knob`, its checks left to the engine): a range (`min`, `max`,
/// `step`) or `options`, never both; `description` is the form's hint, `group` its fieldset and `order` its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct KnobDeclaration {
    pub key: String,
    pub label: String,
    pub unit: KnobUnit,
    pub min: Option<KnobValue>,
    pub max: Option<KnobValue>,
    pub step: Option<KnobValue>,
    pub options: Vec<KnobOption>,
    pub default: KnobDefault,
    pub description: Option<String>,
    pub group: Option<String>,
    pub order: Option<i64>,
}

/// A template's card (`template.description`): its title, its one line, and what it was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct TemplateDescription {
    pub title: String,
    pub text: String,
    pub built_from: Option<String>,
}

/// What an evaluation made, as discovered (section 3.6): the instances each count knob's placements made, the made
/// entities per component, and what the floor compile derived per component.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct Census {
    // ordered: every census map is user-visible JSON.
    pub by_knob: BTreeMap<String, u64>,
    pub declared: BTreeMap<String, u64>,
    pub derived: BTreeMap<String, u64>,
}

/// One problem of an evaluation (section 3.11): `text` opens with its severity in words, `knob` names the knob to
/// change and `ref` the template entity or source key at fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct EvaluationProblem {
    pub severity: Severity,
    pub code: String,
    pub knob: Option<String>,
    #[serde(rename = "ref")]
    pub ref_: Option<String>,
    pub text: String,
}

/// Something an evaluation left out without fault, or read only in part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct EvaluationNote {
    #[serde(rename = "ref")]
    pub ref_: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "template.ts")]
pub enum ThumbnailState {
    Evaluating,
    Ready,
    Failed,
}

/// A template's default evaluation as a card shows it: its floor document's key, which the page's viewer draws (ruling
/// D121: one renderer), the census and the known problems; `evaluating` until it is computed the first time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct Thumbnail {
    pub state: ThumbnailState,
    pub floor_key: Option<String>,
    pub census: Option<Census>,
    pub problems: Vec<EvaluationProblem>,
}

/// One card of the Template list: the template's root scope, read at each scope's changeset (`seqs`; `tag` null
/// until world-proto-server has tags), its knobs, its material edges' names and its thumbnail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct TemplateCard {
    #[serde(rename = "ref")]
    pub ref_: ScopePath,
    pub title: String,
    pub text: String,
    pub built_from: Option<String>,
    pub tag: Option<TagName>,
    // ordered: a template's scopes are listed in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
    pub knobs: Vec<KnobDeclaration>,
    pub flows: Vec<String>,
    pub thumbnail: Thumbnail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct TemplateList {
    pub items: Vec<TemplateCard>,
}

/// `GET /api/templates/{*location}`'s answer: one template's declarations and its default evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "template.ts")]
pub struct TemplateDetail {
    #[serde(rename = "ref")]
    pub ref_: ScopePath,
    pub tag: Option<TagName>,
    // ordered: a template's scopes are listed in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
    pub description: TemplateDescription,
    pub knobs: Vec<KnobDeclaration>,
    pub flows: Vec<String>,
    pub default: Thumbnail,
}
