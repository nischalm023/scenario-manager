//! Within `scenarios`, `check` is spec 07 section 3.2's load of one version against one world, CPU-bound and run on
//! the compute pool by its callers: dessim's reader over the exported world, the two site facts, the closed values
//! the runner image accepts, the workload generated in process, then the simulator's own `harness::prepare` and the
//! scenario's resolution and stock placement over a scratch folder laid out as a run's, so a workload a run would
//! refuse is refused before a job starts, in the reader's words. Readiness calls it, and I1.09's plan per variant.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use dessim::harness::{HarnessError, RunOverrides as HarnessOverrides, prepare};
use dessim::robots::SUCCESSOR;
use dessim::runner::resolve_scenario_with_racking;
use dessim::world_spec::load;
use dessim::worldmap::{WorldMap, load_from_bytes};
use scenario_spec::{
    Compiled, Generated, PICK_SEQUENCING, RELEASE, RUN_FILES, compile, effective, generate, pick_sequencing,
    require_catalog_motion, site_facts, word_of,
};
use studio_protocol::{EntityId, Key, ProblemItem, Release, RunOverrides, ScenarioContent, Severity, WorkloadSummary};

/// The closed values a runner image accepts, by setting.
pub type Accepts = BTreeMap<String, Vec<String>>;

/// What a check found: its problems, errors and warnings, and what the generator made, with the digest of its
/// stream, when it got that far.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckOutcome {
    pub problems: Vec<ProblemItem>,
    pub workload: Option<WorkloadSummary>,
    pub workload_sha256: Option<String>,
    pub compiled: Option<Compiled>,
}

/// Placement does not prove that all generated stock fits the racks.
const NOT_CHECKED: &str = "This check does not establish that all generated stock fits the racks.";

/// Load `content` with `overrides` against `world`, a site's flagged export's bytes, as a run of the image accepting
/// `accepts` would.
pub fn check(world: &[u8], content: &ScenarioContent, overrides: &RunOverrides, accepts: &Accepts) -> CheckOutcome {
    let mut problems = vec![item("not-checked", Severity::Warning, None, None, NOT_CHECKED.to_owned())];
    let map = match load_from_bytes(world) {
        Ok(map) => map,
        Err(fault) => {
            problems.push(item("floor-compile-failed", Severity::Error, None, None, fault.to_string()));
            return CheckOutcome { problems, workload: None, workload_sha256: None, compiled: None };
        }
    };
    if let Err(words) = require_catalog_motion(&map) {
        problems.push(item("catalog-motion-missing", Severity::Error, None, None, words));
        return CheckOutcome { problems, workload: None, workload_sha256: None, compiled: None };
    }
    for (id, name) in unnamed_overrides(&map) {
        let text = format!("The agent override {name} is named by no cohort, so it changes nothing a run reads.");
        let key = id.parse::<EntityId>().ok().map(Key::Entity);
        problems.push(item("override-unused", Severity::Warning, key, None, text));
    }
    for (node, sequencing) in pick_sequencing(&map) {
        if !accepted(accepts, PICK_SEQUENCING, &sequencing) {
            let text = format!(
                "The pick node {node} sequences its picks `{sequencing}`, which the runner image does not run; it runs \
                 {}.",
                accepts.get(PICK_SEQUENCING).map_or_else(String::new, |values| values.join(" and "))
            );
            let key = node.parse::<EntityId>().ok().map(Key::Entity);
            problems.push(item("unsupported-value", Severity::Error, key, Some("/flow.node/work/sequencing"), text));
        }
    }
    let release = content.workload.generator.release;
    if !accepted(accepts, RELEASE, &word_of(&release)) {
        let what = match release {
            Release::AllAtStart => "Every order at the start",
            Release::Even => "Arrivals spread evenly through each hour",
            Release::Random => "Random arrivals",
            Release::Waves => "Orders released in waves",
        };
        let text = format!("{what} need a runner image that reads arrival times, and the local one does not.");
        problems.push(item("unsupported-value", Severity::Error, None, Some("/workload/generator/release"), text));
    }
    let (duration, _) = effective(content, overrides);
    let workload = generate(&content.workload.generator, duration, site_facts(&map));
    let compilation = compile(&map, content, &workload);
    problems.extend(compilation.problems);
    if let Some(compiled) = &compilation.compiled
        && let Err((code, words)) = run_folder(world, content, overrides, &workload, compiled)
    {
        problems.push(item(code, Severity::Error, None, None, words));
    }
    CheckOutcome {
        problems,
        workload: Some(workload.summary()),
        workload_sha256: Some(workload.sha256()),
        compiled: compilation.compiled,
    }
}

/// The run's folder written to a scratch directory and read as the runner reads it: `harness::prepare`, then the
/// scenario's resolution and stock placement with the run's settings.
fn run_folder(
    world: &[u8],
    content: &ScenarioContent,
    overrides: &RunOverrides,
    workload: &Generated,
    compiled: &Compiled,
) -> Result<(), (&'static str, String)> {
    let unreadable = |words: String| ("workload-unreadable", words);
    let dir =
        tempfile::tempdir().map_err(|fault| ("check-failed", format!("no scratch folder for the check: {fault}")))?;
    let spec = serde_json::to_vec(&compiled.spec(content, overrides, RUN_FILES)).expect("a spec is JSON");
    for (file, bytes) in [
        ("spec.json", spec.as_slice()),
        (RUN_FILES.world, world),
        (RUN_FILES.items, &workload.items_csv()),
        (RUN_FILES.orders, &workload.orders_csv()),
    ] {
        write(dir.path(), file, bytes)
            .map_err(|fault| ("check-failed", format!("the check could not write {file}: {fault}")))?;
    }
    let spec_path = dir.path().join("spec.json");
    prepare(&spec_path, &HarnessOverrides::default()).map_err(|fault| match fault {
        HarnessError::Provenance(words) => ("run-identity-unknown", words),
        fault => unreadable(fault.to_string()),
    })?;
    let loaded = load(&spec_path).map_err(|fault| unreadable(fault.to_string()))?;
    resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &loaded.settings)
        .map(drop)
        .map_err(unreadable)
}

fn write(dir: &Path, file: &str, bytes: &[u8]) -> io::Result<()> {
    let path = dir.join(file);
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::write(path, bytes)
}

/// Each agent override (spec 07 section 3.7) no cohort's class names, by id with its name, in id order.
fn unnamed_overrides(map: &WorldMap) -> Vec<(String, String)> {
    let entities = map.world.entities.values();
    let named: Vec<&str> =
        entities.clone().filter_map(|entity| entity.components.get("cohort")?.get("robot_class")?.as_str()).collect();
    entities
        .filter(|entity| entity.has(SUCCESSOR))
        .map(|entity| (entity.id.to_string(), entity.name.clone().unwrap_or_else(|| entity.id.to_string())))
        .filter(|(id, _)| !named.iter().any(|class| class.split('@').next() == Some(id.as_str())))
        .collect()
}

fn accepted(accepts: &Accepts, setting: &str, value: &str) -> bool {
    accepts.get(setting).is_some_and(|values| values.iter().any(|accepted| accepted == value))
}

fn item(code: &str, severity: Severity, ref_: Option<Key>, field: Option<&str>, text: String) -> ProblemItem {
    ProblemItem {
        code: code.to_owned(),
        severity,
        ref_,
        scope: None,
        field: field.map(str::to_owned),
        knob: None,
        text,
    }
}

#[cfg(test)]
mod catalog_motion_tests {
    use super::check;
    use scenario_spec::{stated_defaults, this_build_accepts};
    use studio_protocol::{RunOverrides, Severity};

    #[test]
    fn planning_refuses_unstated_catalog_kind_before_generating_workload() {
        let world = include_bytes!("../../../../sim/dessim/tests/fixtures/minimal_putaway.json");
        let outcome = check(world, &stated_defaults(), &RunOverrides::default(), &this_build_accepts());
        assert!(outcome.workload.is_none() && outcome.workload_sha256.is_none());
        let refusal = outcome.problems.iter().find(|problem| problem.code == "catalog-motion-missing").unwrap();
        assert_eq!(refusal.severity, Severity::Error);
        assert!(refusal.text.contains("floor_drive.kind"), "{refusal:?}");
    }
}
