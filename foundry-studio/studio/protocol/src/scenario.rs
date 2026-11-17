//! Within `studio-protocol`, `scenario` is spec 07's scenario as the API keeps and answers it (sections 3.1, 3.2, 3.5
//! and 3.8): a version's content, the workload spec a seeded generator makes orders and stock from and the run
//! controls, each quantity in its own type and each simtime one named `_sim_ms`; who last set each field; the scenario
//! routes' bodies and answers; and the field catalogue's entries, by which every surface names a field. The crate
//! `scenario-spec` holds the logic over these types.

use std::collections::BTreeMap;

use desunits::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::clock::{Clock, WallClock};
use crate::grammar::{SitePath, Ulid};
use crate::person::Person;
use crate::projection::TurnNumber;

/// The generator's quantities (spec 07 section 3.2), each a JSON number: orders an hour, the arrival rate; lines an
/// order on average; SKUs; the share of the site's inventory tote slots stocked at simtime 0, in percent; units one
/// stored tote holds when the stock is placed; FillInventory's order trays.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct OrdersPerHour(pub f64);

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct LinesPerOrder(pub f64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct SkuCount(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct FillPercent(pub f64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct UnitsPerTote(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct OrderTrays(pub u32);

/// A seed, the generator's or the simulation's: an integer 0 to 2^53 − 1, exact in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct Seed(pub u64);

/// When the generated orders arrive in simtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scenario.ts")]
pub enum Release {
    AllAtStart,
    Even,
    Random,
    Waves,
}

/// dessim's fidelity: reserved cells (`CollisionDes`) or ghost moves (`Ghost`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scenario.ts")]
pub enum FloorTraffic {
    Reserved,
    Ghost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct GeneratorSpec {
    pub orders_per_hour: OrdersPerHour,
    pub lines_per_order: LinesPerOrder,
    pub sku_count: SkuCount,
    pub inventory_fill_pct: FillPercent,
    pub units_per_stored_tote: UnitsPerTote,
    pub release: Release,
    /// Waves only: the simtime between two waves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional)]
    pub wave_every_sim_ms: Option<SimDuration>,
    pub seed: Seed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct Workload {
    pub generator: GeneratorSpec,
    pub order_trays: OrderTrays,
}

/// What a run's summary covers, in simtime: from `from_sim_ms` to `to_sim_ms`, null meaning the end; `before_name`
/// is what the excluded start is called ("warm-up").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct ReportWindow {
    #[ts(type = "number")]
    pub from_sim_ms: SimTime,
    #[ts(type = "number | null")]
    pub to_sim_ms: Option<SimTime>,
    pub before_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct RunControls {
    #[ts(type = "number")]
    pub duration_sim_ms: SimDuration,
    pub seed: Seed,
    pub floor_traffic: FloorTraffic,
    pub report_window: ReportWindow,
}

/// A version's content: every field stated, so no default of dessim's decides a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioContent {
    pub workload: Workload,
    pub run_controls: RunControls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct VersionNumber(pub u32);

/// A version's revision, which grows by one with each accepted edit while the version is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct Revision(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scenario.ts")]
pub enum VersionState {
    Open,
    Sealed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scenario.ts")]
pub enum Setter {
    Agent,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct SetInTurn {
    pub id: Ulid,
    pub n: TurnNumber,
}

/// The last change of one field: by the agent or a person, in which turn, when (wall clock) and at which revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct SetBy {
    pub by: Setter,
    pub author: Person,
    pub turn: Option<SetInTurn>,
    pub at: WallClock,
    pub revision: Revision,
}

/// A scenario's row; `deleted_by` and `deleted_at` are set together by a delete, after which nothing edits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioRow {
    pub id: Ulid,
    pub name: String,
    pub description: String,
    pub site_scope: SitePath,
    pub latest_version: VersionNumber,
    pub created_by: Person,
    pub created_at: WallClock,
    pub changed_at: WallClock,
    pub deleted_by: Option<Person>,
    pub deleted_at: Option<WallClock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct RunCount(pub u32);

/// `runs` counts spec 08's runs of the scenario, every one and those running; both 0 until spec 08's tables exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct RunCounts {
    pub total: RunCount,
    pub running: RunCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioSummary {
    pub id: Ulid,
    pub name: String,
    pub latest_version: VersionNumber,
    pub state: VersionState,
    pub runs: RunCounts,
    pub changed_at: WallClock,
    pub deleted_at: Option<WallClock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioList {
    pub items: Vec<ScenarioSummary>,
}

/// `POST /api/scenarios`'s body: a field left out takes section 3.2's stated default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts", optional_fields)]
pub struct CreateScenario {
    #[ts(optional = false)]
    pub name: String,
    pub description: Option<String>,
    #[ts(optional = false)]
    pub site_scope: SitePath,
    pub workload: Option<Workload>,
    pub run_controls: Option<RunControls>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct CreatedScenario {
    pub scenario: Ulid,
    pub version: VersionNumber,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct VersionSummary {
    pub version: VersionNumber,
    pub state: VersionState,
    pub sealed_at: Option<WallClock>,
    pub experiments: Vec<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct VersionRead {
    pub n: VersionNumber,
    pub state: VersionState,
    pub revision: Revision,
    pub content: ScenarioContent,
    // ordered: a version's fields are listed in pointer order.
    pub set_by: BTreeMap<String, SetBy>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioRead {
    pub scenario: ScenarioRow,
    pub versions: Vec<VersionSummary>,
    pub version: VersionRead,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct FieldSet {
    pub pointer: String,
    pub value: Value,
}

/// `PATCH /api/scenarios/{s}`'s body: the version and revision the edit was based on, and the fields it sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioEdit {
    pub version: VersionNumber,
    pub revision: Revision,
    pub set: Vec<FieldSet>,
}

/// An accepted edit: the version and revision it made, `opened` when it opened a new version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioEdited {
    pub version: VersionNumber,
    pub revision: Revision,
    pub opened: bool,
}

/// One field an edit could not set, with its value and its last change since the edit's base: `scenario-changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct FieldConflict {
    pub pointer: String,
    pub value: Value,
    pub set_by: Option<SetBy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "scenario.ts", optional_fields)]
pub struct DuplicateScenario {
    pub name: Option<String>,
    pub version: Option<VersionNumber>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct FieldChange {
    pub pointer: String,
    pub label: String,
    pub from: Value,
    pub to: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct ScenarioDiff {
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct FieldOption {
    pub value: String,
    pub words: String,
    pub accepted: bool,
}

/// One entry of the field catalogue: a scenario field's pointer, its label and short label, its unit in words, the
/// stored value of one shown unit (3,600,000 for a duration shown in hours of simtime), its clock, whether a sweep
/// may vary it, and a closed field's options, each `accepted` when this build runs it (spec 07 section 5.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct FieldEntry {
    pub pointer: String,
    pub label: String,
    pub short_label: String,
    pub unit: String,
    pub per_unit: f64,
    pub clock: Option<Clock>,
    pub sweepable: bool,
    pub options: Vec<FieldOption>,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "scenario.ts")]
pub struct FieldCatalogue {
    pub items: Vec<FieldEntry>,
}
