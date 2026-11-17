//! Within `studio-protocol`, `panel` is spec 03 section 3.3, the panel commands by which the agent moves what a person
//! sees: the four tools' inputs as the model writes them, the `panel.command` entry the browser acts on, and the
//! result the model reads back. An input keeps the model's strings as written, because the API parses each with the
//! grammar and names the malformed one (`ref-malformed`, step 1 of section 3.3's order); an option the model leaves
//! out stays out when the input is written, never `null`, since the tools' schemas give no option a null type. The
//! entry holds what the API resolved. Every `sim_ms` is simtime in integer milliseconds.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use desunits::SimTime;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::{GrammarError, Location, ObjectKey, Ref, ScopePath, Seq, Ulid, string_form};
use crate::journal::JournalSeq;

/// The panels a command opens. The page context (spec 03 section 3.4) also names `home`, `templates` and `history`,
/// which no command opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "panel.ts")]
pub enum Panel {
    Map,
    Flow,
    Template,
    Scenario,
    Runs,
}

/// What a command does: `show` navigates the initiating browser tab; `highlight` replaces the agent's highlights;
/// `select` replaces the panel's one selection; `clear` removes the agent's highlights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "panel.ts")]
pub enum PanelAction {
    Show,
    Highlight,
    Select,
    Clear,
}

/// A layer of the Flow panel; both are drawn unless a command or the address turns one off (ruling D103).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "panel.ts")]
pub enum FlowLayer {
    Material,
    Agent,
}

/// What a command's panel shows: a location, or a Postgres object (`scenario:01KK…`, `run:01KN…`). In the text, the
/// first of `:`, `/` and `@` tells them apart, since a slug holds none of the three and an object kind no `/` or `@`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "panel.ts")]
pub enum PanelTarget {
    Location(Location),
    Object(ObjectKey),
}

impl FromStr for PanelTarget {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.find([':', '/', '@']).is_some_and(|at| text[at..].starts_with(':')) {
            text.parse().map(Self::Object)
        } else {
            text.parse().map(Self::Location)
        }
    }
}

impl Display for PanelTarget {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Location(location) => write!(out, "{location}"),
            Self::Object(object) => write!(out, "{object}"),
        }
    }
}

string_form!(PanelTarget);

/// A scope and the changeset a command resolved it to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "panel.ts")]
pub struct ScopeSeq {
    pub scope: ScopePath,
    pub seq: Seq,
}

/// One highlight group as the model writes it: 1 to 200 references, and a note of at most 160 characters saying why,
/// which the announcer reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "panel.ts")]
pub struct HighlightGroupInput {
    pub refs: Vec<String>,
    pub note: String,
}

/// `panel.show`'s input. `location` may be left out for a list (`template`, `scenario`, `runs`); `draft` overlays a
/// changeset draft on the Flow panel; `compare` holds 2 to 12 runs, `metrics` 1 to 8 metric names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "panel.ts", optional_fields)]
pub struct PanelShowInput {
    pub panel: Panel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Vec<HighlightGroupInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<FlowLayer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional)]
    pub sim_ms: Option<SimTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Vec<String>>,
}

/// `panel.highlight`'s input: up to eight groups, replacing the agent's highlights on that panel. Without `location`,
/// a command acts on the turn's last `show`, else on the page context the turn's message carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "panel.ts", optional_fields)]
pub struct PanelHighlightInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel: Option<Panel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub highlight: Vec<HighlightGroupInput>,
}

/// `panel.select`'s input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "panel.ts", optional_fields)]
pub struct PanelSelectInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel: Option<Panel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub select: String,
}

/// `panel.clear`'s input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "panel.ts", optional_fields)]
pub struct PanelClearInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel: Option<Panel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// A number of marks: the entities a highlight outlines on the panel in view, once the rule table of spec 03 section
/// 3.5 has expanded its references. A number in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "panel.ts")]
pub struct MarkCount(u32);

impl MarkCount {
    pub const fn new(marks: u32) -> Self {
        Self(marks)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One highlight group as journaled: its references resolved, the model's note, and how many marks the group drew,
/// in all and on the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "panel.ts")]
pub struct HighlightGroup {
    pub refs: Vec<Ref>,
    pub note: String,
    pub marks: MarkCount,
    pub on_floor: MarkCount,
}

/// The body of a `panel.command` entry: what the API resolved and journaled for one command. `route` is the canonical
/// address as the agent asked; `as_it_was` reopens the exact cut the command read and is null for a location at a
/// changeset or a tag, which never moves; `announce` is the sentence the announcer speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "panel.ts")]
pub struct PanelCommandEntry {
    pub action: PanelAction,
    pub panel: Panel,
    pub location: Option<PanelTarget>,
    pub resolved: Vec<ScopeSeq>,
    pub select: Option<Ref>,
    pub layers: Option<Vec<FlowLayer>>,
    pub draft: Option<Ulid>,
    #[ts(type = "number | null")]
    pub sim_ms: Option<SimTime>,
    pub compare: Option<Vec<Ulid>>,
    pub metrics: Option<Vec<String>>,
    pub highlight: Option<Vec<HighlightGroup>>,
    pub route: String,
    pub as_it_was: Option<String>,
    pub announce: String,
}

/// How many marks one highlight group drew, in all and on the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "panel.ts")]
pub struct GroupMarkCounts {
    pub marks: MarkCount,
    pub on_floor: MarkCount,
}

/// What a panel tool answers the model: the entry it journaled, its route, the cut it resolved and each group's marks.
/// It says the entry was journaled, never that a browser showed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "panel.ts")]
pub struct PanelCommandResult {
    pub journaled: JournalSeq,
    pub route: String,
    pub resolved: Vec<ScopeSeq>,
    pub groups: Vec<GroupMarkCounts>,
}
