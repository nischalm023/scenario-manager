//! Within `studio-protocol`, `plan` is spec 08 section 3.3's submission plan, which exists so that an approval
//! approves something concrete: every run a submission would create, its pinned inputs (each scope's changeset, the
//! scenario version, the variant, the seed and the runner image by digest) and the digests the check recorded, all
//! fixed before anyone approves. It holds the plan, the routes that prepare and read it (section 3.7's
//! `POST /api/runs/plan` and `GET /api/runs/plan/{p}`), and the press that executes it (`POST /api/runs`). Preparing
//! happens in wall clock; every `_sim_ms` is simtime.

use std::collections::BTreeMap;

use desunits::SimDuration;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::WallClock;
use crate::experiment::{ExperimentKind, Patch};
use crate::grammar::{ScopePath, Seq, Ulid};
use crate::problem::Problem;
use crate::run::RunPosition;
use crate::scenario::{ReportWindow, Revision, RunCount, Seed, VersionNumber};
use crate::workload::UnitCut;

/// The plan's format, 1 until release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlanFormat(pub u32);

/// How many worlds a plan's check has loaded: one per distinct variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct WorldCount(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "plan.ts")]
pub enum PlanState {
    Preparing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlanProgress {
    pub worlds_done: WorldCount,
    pub worlds: WorldCount,
}

/// Whether the plan names the registry's default image or one a developer chose (ruling D78).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "plan.ts")]
pub enum RunnerChosen {
    Default,
    Developer,
}

/// The runner image a plan names: its reference by digest, the tag it was built at, and who chose it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlanRunner {
    pub image: String,
    pub tag: String,
    pub chosen: RunnerChosen,
}

/// The scenario version a plan was prepared on, at the revision the experiment was made at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlanScenario {
    pub id: Ulid,
    pub version: VersionNumber,
    pub revision: Revision,
    pub name: String,
}

/// The digests the check recorded for one run (section 3.1, "expectation"): its compiled flow plan's digest (null
/// while no flow compiler writes a composition, which a finding compares as equal to null), its world's sha256 before
/// the variant, the compiler's version, and its generated workload's digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct Expectation {
    pub compile_sha256: Option<String>,
    pub world_sha256: String,
    pub compiler: String,
    pub workload_sha256: String,
}

/// One run a submission of the plan creates, its id minted with the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlannedRun {
    pub run: Ulid,
    pub position: RunPosition,
    pub seed: Seed,
    #[ts(type = "number")]
    pub duration_sim_ms: SimDuration,
    pub report_window: ReportWindow,
    pub variant: Vec<Patch>,
    pub expected: Expectation,
}

/// The queue as it stood while the plan was prepared: information for the card, never an input, so the plan's hash
/// leaves it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct QueueSnapshot {
    pub limit: RunCount,
    pub going: RunCount,
    pub waiting: RunCount,
    pub as_of: WallClock,
}

/// A prepared plan, as section 3.3 draws it: `reads` is every scope the check's export carried, at its changeset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct Plan {
    pub v: PlanFormat,
    pub id: Ulid,
    pub experiment: Ulid,
    pub kind: ExperimentKind,
    pub scenario: PlanScenario,
    pub cut: UnitCut,
    // ordered: a plan lists its reads in scope order, as its hash needs.
    pub reads: BTreeMap<ScopePath, Seq>,
    pub prepared_at: WallClock,
    pub runner: PlanRunner,
    pub runs: Vec<PlannedRun>,
    pub queue: QueueSnapshot,
}

/// `POST /api/runs/plan`'s body: the experiment, and the changesets a panel showed, which the plan must read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "plan.ts")]
pub struct PlanRequest {
    pub experiment: Ulid,
    // ordered: a cut lists its scopes in path order.
    #[serde(default)]
    #[ts(optional)]
    pub cut: Option<BTreeMap<ScopePath, Seq>>,
}

/// `202` to `POST /api/runs/plan`: the plan being prepared, read once a second (wall clock) until it is ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlanAccepted {
    pub plan: Ulid,
}

/// `GET /api/runs/plan/{p}`'s answer: the plan once ready with its hash, or the one problem that failed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct PlanRead {
    pub id: Ulid,
    pub state: PlanState,
    pub progress: PlanProgress,
    pub plan: Option<Plan>,
    pub plan_sha256: Option<String>,
    pub problem: Option<Problem>,
}

/// `POST /api/runs`'s body: the plan to execute and the hash its approver saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "plan.ts")]
pub struct SubmitRequest {
    pub plan: Ulid,
    pub plan_sha256: String,
}

/// What a submission's execution answered, kept as its approval's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "plan.ts")]
pub struct Submitted {
    pub submission: Ulid,
    pub runs: Vec<Ulid>,
}
