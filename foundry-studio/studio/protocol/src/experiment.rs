//! Within `studio-protocol`, `experiment` is spec 07 section 3.6's experiment: what one person's yes would launch, a
//! single run, repeats of it with the next simulation seeds, or a sweep of one field over a few values with repeats.
//! A request names the form; the API derives the variants, each with at most one patch, and the order the runs go in,
//! and nothing changes once it is created (spec 08 needs it unchangeable once a plan names it).

use desunits::SimDuration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::grammar::Ulid;
use crate::runs::RunSummary;
use crate::scenario::{ReportWindow, Revision, Seed, VersionNumber};

/// How many runs of each variant: 1 to 10, each with the next simulation seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct Repeats(pub u32);

/// A variant's number, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct VariantNumber(pub u32);

/// What a check run sets for all its runs instead of the version's run controls.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "experiment.ts", optional_fields)]
pub struct RunOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number")]
    pub duration_sim_ms: Option<SimDuration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_window: Option<ReportWindow>,
}

/// The one field a sweep varies, as a request names it: a target, a JSON pointer into it, and 2 to 12 values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "experiment.ts")]
pub struct VariesRequest {
    pub target: String,
    pub pointer: String,
    pub values: Vec<Value>,
}

/// `POST /api/experiments`'s body. It carries no variants: those are derived, never authored (D95).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "experiment.ts", optional_fields)]
pub struct ExperimentRequest {
    #[ts(optional = false)]
    pub scenario: Ulid,
    #[ts(optional = false)]
    pub version: VersionNumber,
    #[ts(optional = false)]
    pub revision: Revision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub varies: Option<VariesRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeats: Option<Repeats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_overrides: Option<RunOverrides>,
}

/// Where a patch lands: in the world the runner fetched, or in the workload spec and the run's spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "experiment.ts")]
pub enum AppliedTo {
    World,
    Spec,
}

/// One value set for one run: `target` is an entity's bare id for the world, `scenario:{ULID}` for the spec; `label`
/// names the field in words, from the field catalogue or spec 06's parameter list, never from the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct Patch {
    pub target: String,
    pub pointer: String,
    pub value: Value,
    pub label: String,
    pub applied_to: AppliedTo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct Variant {
    pub n: VariantNumber,
    pub label: String,
    pub patches: Vec<Patch>,
}

/// The varied field as a sweep keeps it for display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct Varied {
    pub target: String,
    pub pointer: String,
    pub label: String,
    pub short_label: String,
    pub unit: String,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct Sweep {
    pub varies: Varied,
}

/// One run's place in the order runs go in: every variant at the first seed, then every variant at the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct RunSlot {
    pub variant: VariantNumber,
    pub seed: Seed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "experiment.ts")]
pub enum ExperimentKind {
    Single,
    Repeats,
    Sweep,
}

/// An experiment as the API answers it; `run_order` and `kind` are derived, and `submission` is spec 08's, null until
/// a submission names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct Experiment {
    pub id: Ulid,
    pub scenario: Ulid,
    pub version: VersionNumber,
    pub revision: Revision,
    pub sweep: Option<Sweep>,
    pub variants: Vec<Variant>,
    pub seeds: Vec<Seed>,
    pub run_overrides: RunOverrides,
    pub run_order: Vec<RunSlot>,
    pub kind: ExperimentKind,
    pub submission: Option<Ulid>,
}

/// `GET /api/experiments/{e}`'s answer: the experiment, with its submission once one names it, and that submission's
/// runs in position order, spec 08's, none before it is submitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "experiment.ts")]
pub struct ExperimentRead {
    pub experiment: Experiment,
    pub runs: Vec<RunSummary>,
}
