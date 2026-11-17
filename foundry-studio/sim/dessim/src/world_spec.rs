//! The spec file for a world-protocol run: one JSON that says which world
//! document to load and what workload and settings to run over it.
//!
//! Sibling of `sim_spec`, not part of it. A `SimSpec` carries its warehouse
//! INLINE -- the design's keys sit at the JSON root -- because a v3 design is
//! one self-contained object. A world-protocol scenario is a layered document
//! set with its own loader, so this spec REFERENCES it by path instead of
//! embedding it. That split is the ruled shape: workload and settings are
//! dessim-owned document kinds the world layer never depends on (SPEC §12.6
//! defers their semantics), and single-file packaging of "which world, which
//! workload" is legal without the world document knowing.
//!
//! ```json
//! {
//!   "world": "cencora_v3.json",
//!   "scenario": { "num_skus": 500, ... },
//!   "settings": { "num_htms": 10, ... }
//! }
//! ```
//!
//! EVERY path a spec states -- `world`, and the workload CSVs inside
//! `scenario` -- resolves relative to the spec file's own directory, the same
//! rule every include-like reference in this repo follows, so a spec can sit
//! next to the files it names and the set moves together. An absolute path
//! stays absolute.
//!
//! One rule for all of them, because the alternative bit: the CSV paths used
//! to resolve against the process CWD while `world` resolved against the spec,
//! so `dfw6_real_spec.json` loaded its warehouse from anywhere and found its
//! order data only when run from one directory. Nothing caught it -- no test
//! reads those CSVs, so a broken data path stays green and fails at run time.
//!
//! A fleet's robot figures belong to the world once its robot entities carry
//! aspects: `load` writes them over the spec's `settings`, and refuses a spec
//! that states one of them, the same as `sim_spec` refuses it from
//! `--settings`.

use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Error as JsonError;
use serde_json::Value;

use crate::composition::Composition;
use crate::config::SimConfig;
use crate::scenario::{CsvScenario, ScenarioConfig, SiteExtractScenario, StockPolicy, UnknownContainerPolicy};
use crate::worldmap::{self, WorldMap, WorldMapError};

/// The parsed spec: a reference to a world document plus the two blocks the
/// world layer deliberately does not carry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpec {
    /// Free text about the run this spec describes, as a world document
    /// carries beside its data: a scenario states its inputs where they are
    /// set. Read by nobody.
    #[serde(default, rename = "$comment")]
    pub comment: Option<String>,
    /// Path to the world-protocol scenario document, relative to the spec file.
    pub world: PathBuf,
    /// Named composition or an inline top-level task list. Overrides a saved
    /// settings composition; absent means the settings/default selection.
    #[serde(default)]
    pub composition: Option<Composition>,
    #[serde(default)]
    pub scenario: ScenarioConfig,
    #[serde(default)]
    pub settings: SimConfig,
}

/// A spec with its world loaded: everything a caller needs to reach
/// `solver::resolve_scenario_with_racking` and from there the planner and
/// solver, which are unchanged.
pub struct LoadedWorldSpec {
    pub world: WorldMap,
    pub scenario: ScenarioConfig,
    pub settings: SimConfig,
    /// The world document's resolved path — what `world` was loaded from.
    /// Callers that EMBED the document (a `PerfProfile`) read it back from
    /// here rather than threading raw JSON through every load.
    pub world_path: PathBuf,
    /// The spec file and the world document as the bytes they were read from
    /// disk as, and every workload file the scenario names, keyed by the
    /// reference as the scenario states it. A recording carries them so
    /// replaying it needs no file beside it.
    pub documents: SpecDocuments,
}

/// What a spec named, for a caller that will record the run.
///
/// The workload files are NAMED here and not read: a run that will be
/// recorded needs their bytes, and `desvis info` printing a map's topology
/// does not — a site extract's CSVs run to hundreds of megabytes.
#[derive(Debug, Default)]
pub struct SpecDocuments {
    /// The spec file as read. Free: `load` already holds it.
    pub spec: Vec<u8>,
    /// Each workload file the scenario names: the reference exactly as the
    /// spec writes it, and the path that reference resolved to. The reference
    /// is what a recording keys the bytes by, because a resolved path means
    /// nothing on another machine.
    pub data: Vec<(String, PathBuf)>,
}

impl SpecDocuments {
    /// Read every workload file, keyed by its reference as written.
    pub fn read_data(&self) -> Result<Vec<(String, Vec<u8>)>, WorldSpecError> {
        self.data
            .iter()
            .map(|(reference, path)| {
                let bytes =
                    std::fs::read(path).map_err(|source| WorldSpecError::Read { path: path.clone(), source })?;
                Ok((reference.clone(), bytes))
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorldSpecError {
    #[error(
        "{path} declares {document} order carriers and the spec states {spec}. The document owns \
         the tote population; remove `order_totes` from the spec, or change the pool"
    )]
    CarrierCountConflict { path: PathBuf, document: u32, spec: u32 },

    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },

    #[error(
        "the scenario names {slots} workload files and the recording carries {embedded}. They are paired by \
         position, so a recording written by another build cannot be repeated from its own bytes"
    )]
    WorkloadCount { slots: usize, embedded: usize },
    #[error("{path} is not a world spec: {source}")]
    Parse { path: PathBuf, source: JsonError },
    #[error("loading the world document {path} failed: {source}")]
    World { path: PathBuf, source: WorldMapError },
    #[error(
        "{spec} states settings {key}, a robot figure, and the world document {world} gives that fleet's \
         robot figures from its robot entities; remove it from the spec, or change the robot entity in the world"
    )]
    RobotFigure { spec: PathBuf, key: String, world: PathBuf },
}

/// Parse a spec file and load the world it references.
pub fn load(path: &Path) -> Result<LoadedWorldSpec, WorldSpecError> {
    let raw =
        std::fs::read_to_string(path).map_err(|source| WorldSpecError::Read { path: path.to_path_buf(), source })?;
    let spec: WorldSpec =
        serde_json::from_str(&raw).map_err(|source| WorldSpecError::Parse { path: path.to_path_buf(), source })?;

    // Relative to the spec file, absolute stays absolute: `Path::join` on an
    // absolute right-hand side is the right-hand side.
    let world_path = match path.parent() {
        Some(dir) => dir.join(&spec.world),
        None => spec.world.clone(),
    };
    let world =
        worldmap::load(&world_path).map_err(|source| WorldSpecError::World { path: world_path.clone(), source })?;
    let document: Value =
        serde_json::from_str(&raw).map_err(|source| WorldSpecError::Parse { path: path.to_path_buf(), source })?;
    for fleet in ["htm", "vtm"] {
        let stated = document["settings"][fleet].as_object().into_iter().flat_map(|block| block.keys());
        if let Some(key) = stated.map(|key| format!("{fleet}.{key}")).find(|key| world.robot_catalog.states(key)) {
            return Err(WorldSpecError::RobotFigure { spec: path.to_path_buf(), key, world: world_path });
        }
    }

    if document["settings"].get("motion_model").is_some() && world.robot_catalog.states("motion_model") {
        return Err(WorldSpecError::RobotFigure {
            spec: path.to_path_buf(),
            key: "motion_model".into(),
            world: world_path,
        });
    }

    // The world document carries its own fleet; a settings block that says
    // nothing about HTMs (the config default is 0) means "deploy what the
    // document declares", mirroring how VTM count is always map-determined.
    // Without this, a spec-driven run deploys zero HTMs and deadlocks on
    // dispatch. An explicit non-zero num_htms still wins.
    let mut settings = spec.settings;
    if let Some(composition) = spec.composition {
        settings.composition = composition;
    }
    if settings.num_htms == 0 {
        settings.num_htms = world.map.fleet_config().htms;
    }

    // The tote population is the DOCUMENT's, the same way the fleet is. A
    // `carrier_pool` expands into one carrier per tote, and the census counts
    // that expansion; a spec that also states `order_totes` is stating a
    // second time what the document already says, so a disagreement is
    // refused rather than silently resolved.
    //
    // EVERY scenario kind, not just the generated one. A site extract states
    // demand and never a tote population, so it defaulted to zero order totes:
    // the DFW6 four-hour run had no order-designated tote anywhere, close-out
    // could never find one to load a closed order into, and every one of its
    // 22,160 orders finished with zero loads and zero extractions.
    let mut scenario = spec.scenario;
    // The references as the spec writes them, taken before they are resolved:
    // a recording keys its embedded files by these, and a resolved path is
    // this machine's business.
    let references: Vec<String> = workload_slots(&mut scenario).into_iter().map(|slot| slot.clone()).collect();
    rebase_scenario_paths(&mut scenario, path.parent());

    if !world.carriers.is_empty() {
        let stated = match &mut scenario {
            ScenarioConfig::Generated(generated) => Some(&mut generated.order_totes),
            ScenarioConfig::SiteExtract(extract) => Some(&mut extract.order_totes),
            ScenarioConfig::Inline(_) | ScenarioConfig::Csv(_) | ScenarioConfig::BulkSource(_) => None,
        };
        if let Some(stated) = stated {
            if *stated != 0 && *stated != world.carriers.order {
                return Err(WorldSpecError::CarrierCountConflict {
                    path: world_path,
                    document: world.carriers.order,
                    spec: *stated,
                });
            }
            *stated = world.carriers.order;
        }
    }

    let resolved: Vec<PathBuf> = workload_slots(&mut scenario).into_iter().map(|slot| PathBuf::from(&*slot)).collect();
    let documents = SpecDocuments { spec: raw.into_bytes(), data: references.into_iter().zip(resolved).collect() };
    Ok(LoadedWorldSpec { world, scenario, settings, world_path, documents })
}

/// Every place a scenario names a workload file, in one fixed order.
///
/// One list, because three things walk it — resolving the references against
/// the spec's directory, remembering what they said before that, and pointing
/// them at a recording's own copies — and two of them that disagreed about
/// the order would pair a file with the wrong reference.
fn workload_slots(scenario: &mut ScenarioConfig) -> Vec<&mut String> {
    match scenario {
        ScenarioConfig::SiteExtract(e) => {
            let mut slots = vec![&mut e.outbound_csv, &mut e.onhand_csv];
            slots.extend(e.stock_top_up_csv.as_mut());
            slots
        }
        ScenarioConfig::Csv(c) => vec![&mut c.items_csv, &mut c.orders_csv],
        ScenarioConfig::BulkSource(b) => vec![&mut b.items_csv, &mut b.bulk_csv],
        // These state their workload inline and name no file.
        ScenarioConfig::Generated(_) | ScenarioConfig::Inline(_) => Vec::new(),
    }
}

/// Rewrite every workload path in `scenario` to sit under `dir`, the spec
/// file's own directory.
///
/// `Path::join` on an absolute right-hand side is the right-hand side, so an
/// absolute path in a spec still points where it says. `None` is a spec path
/// with no directory component, which already means "beside the spec".
fn rebase_scenario_paths(scenario: &mut ScenarioConfig, dir: Option<&Path>) {
    let Some(dir) = dir else { return };
    for slot in workload_slots(scenario) {
        *slot = dir.join(&*slot).to_string_lossy().into_owned();
    }
}

/// Point a scenario's workload references at files written out of a
/// recording's own bytes, so a run repeats with the source files gone.
///
/// The embedded files are paired with the scenario's slots by position, which
/// is the order `workload_slots` fixes and the order the recording stored
/// them in. A count that does not match is refused rather than guessed at:
/// pairing the orders file with the items reference would run a different
/// workload and say nothing.
pub fn rehome_workload(
    scenario: &mut ScenarioConfig,
    data: &[(String, Vec<u8>)],
    into: &Path,
) -> Result<(), WorldSpecError> {
    let slots = workload_slots(scenario);
    if slots.len() != data.len() {
        return Err(WorldSpecError::WorkloadCount { slots: slots.len(), embedded: data.len() });
    }
    for (slot, (reference, bytes)) in slots.into_iter().zip(data) {
        let path = into.join(reference.trim_start_matches(['/', '.']));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| WorldSpecError::Read { path: parent.to_path_buf(), source })?;
        }
        std::fs::write(&path, bytes).map_err(|source| WorldSpecError::Read { path: path.clone(), source })?;
        *slot = path.to_string_lossy().into_owned();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_with_an_unknown_key_is_refused() {
        // The design keys of a v3 SimSpec sit at the JSON root, so the likeliest
        // authoring mistake is feeding one of those files here. `deny_unknown_fields`
        // turns that into a parse error naming the key instead of a world path
        // error naming nothing.
        let err = serde_json::from_str::<WorldSpec>(r#"{ "world": "w.json", "grid": { "width": 3 } }"#).unwrap_err();
        assert!(err.to_string().contains("grid"), "{err}");
    }

    /// A workload CSV named by a spec sits beside the spec, not beside
    /// whatever directory the binary happened to start in.
    ///
    /// This is the test the original bug did not have. `dfw6_real_spec.json`
    /// carried an absolute path under one developer's home directory for as
    /// long as it did because nothing read those CSVs -- the spec's only test
    /// parses it -- so the run worked on one machine and nowhere else.
    #[test]
    fn a_workload_path_resolves_beside_the_spec_not_the_working_directory() {
        let mut scenario = ScenarioConfig::SiteExtract(SiteExtractScenario {
            outbound_csv: "walmart-data/outbound.csv".into(),
            onhand_csv: "walmart-data/onhand.csv".into(),
            stock_top_up_csv: Some("top-up.csv".into()),
            unknown_container: UnknownContainerPolicy::AsCarton,
            inventory_distribution: Default::default(),
            stock: StockPolicy::CoverDemand,
            order_totes: 0,
            units_per_tote: 1,
        });
        rebase_scenario_paths(&mut scenario, Path::new("sim/dessim/tests/fixtures/s.json").parent());
        let ScenarioConfig::SiteExtract(e) = &scenario else { panic!("kind changed") };
        assert_eq!(e.outbound_csv, "sim/dessim/tests/fixtures/walmart-data/outbound.csv");
        assert_eq!(e.onhand_csv, "sim/dessim/tests/fixtures/walmart-data/onhand.csv");
        assert_eq!(e.stock_top_up_csv.as_deref(), Some("sim/dessim/tests/fixtures/top-up.csv"));
    }

    /// An absolute path in a spec still points where it says.
    #[test]
    fn an_absolute_workload_path_is_left_alone() {
        let mut scenario = ScenarioConfig::Csv(CsvScenario {
            items_csv: "/srv/extracts/items.csv".into(),
            orders_csv: "orders.csv".into(),
            units_per_tote: 1,
        });
        rebase_scenario_paths(&mut scenario, Some(Path::new("/tmp/specs")));
        let ScenarioConfig::Csv(c) = &scenario else { panic!("kind changed") };
        assert_eq!(c.items_csv, "/srv/extracts/items.csv");
        assert_eq!(c.orders_csv, "/tmp/specs/orders.csv");
    }

    #[test]
    fn scenario_and_settings_default_when_absent() {
        let spec: WorldSpec = serde_json::from_str(r#"{ "world": "w.json" }"#).unwrap();
        assert_eq!(spec.world, PathBuf::from("w.json"));
    }
}
