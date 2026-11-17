//! Within `dessim`, `harness` is HOW A SPEC BECOMES A RUN: it reads a world
//! spec off disk, folds in the handful of things a caller may say about the run
//! itself, executes the simulation while its `.worldview` is written, and
//! hands back the run's profile and summary.
//!
//! It exists because two programs run specs — desvis headless and the Crucible
//! server — and a recording written by one has to be the recording written by
//! the other. Everything that decides a number lives here; what stays outside
//! is presentation (desvis's progress bar and config printout) and what the
//! caller does with the profile afterwards.
//!
//! The two steps are separate because desvis prints the resolved configuration
//! between them: [`prepare`] reads the spec and settles the configuration, and
//! [`PreparedRun::execute`] runs it. A world document desvis opens with no spec
//! beside it is prepared by [`prepare_world`], under the same rules.

pub mod identity;
mod run;

use std::io::{Error as IoError, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Error as JsonError, Value};

use crate::config::SimConfig;
use crate::metrics::profile::{IdTables, PerfProfile, RunDocuments, ScenarioSource, SummaryStats, VtmFleet};
use crate::metrics::worldview::RecordingError;
use crate::runner::RunnerHooks;
use crate::scenario::ScenarioConfig;
use crate::scenario_gen::Racking;
use crate::sim_spec::{apply_cli_settings, apply_scenario_settings};
use crate::world_spec::{self, WorldSpecError};
use crate::worldmap::{self, WorldMap, WorldMapError};
use desmap::WarehouseMap;

pub use identity::RunIdentity;

/// Where a run's recording is written as the run goes, which run and try at
/// it the recording says it is (a server with a registry behind it states
/// both, desvis neither: empty and zero), and what in the same process reads
/// it as it is written.
pub struct RecordingTarget {
    pub path: PathBuf,
    pub run: String,
    pub attempt: u32,
    /// Handed every byte of the recording, in order, as the file takes it. A
    /// tap that refuses stops the run, as a full disk does.
    pub tap: Option<Box<dyn Write + Send>>,
}

/// What a caller may say about a run beyond the spec itself.
///
/// Every field here is what somebody typed, and `None` means they typed
/// nothing — so the spec's own `settings` block stands. That is what makes one
/// spec one run whoever submits it: desvis's command line and the server's
/// submission reach this type the same way, and neither can overwrite a
/// setting the spec states just by leaving a flag alone. A command line that
/// wrote its own defaults in here would silently record a different run than
/// the server records from the same file.
#[derive(Debug, Default, Clone)]
pub struct RunOverrides {
    /// Executor threads. `None` leaves the spec's `num_threads` alone.
    pub num_threads: Option<usize>,
    /// Simulated duration, replacing the spec's.
    pub sim_duration: Option<Duration>,
    /// Slice width in milliseconds, replacing the spec's.
    pub slice_width_ms: Option<u64>,
    /// Cap on the orders the scenario generates and the run accepts.
    pub max_orders: Option<usize>,
    /// Wall-clock limit on the whole run.
    pub wall_timeout: Option<Duration>,
    /// Movement fidelity, spelled as `FidelityMode` parses it.
    pub fidelity: Option<String>,
    /// `key=value` settings applied last, on top of everything above.
    pub settings: Vec<String>,
    /// Record every simulation event.
    pub events: bool,
    /// Run without the deadlock watchdog. `None` leaves the spec's own answer.
    pub no_watchdog: Option<bool>,
    /// Run with the tripwire assertions on. `None` leaves the spec's own.
    pub tripwires: Option<bool>,
    /// The run's name in the recording; the default states the spec and the
    /// code it ran on.
    pub run_name: Option<String>,
}

/// A spec read, its world loaded, and its configuration settled: everything the
/// run needs, and the few facts a caller wants to show before starting it.
pub struct PreparedRun {
    /// The configuration the runner will see. A caller may print it or change
    /// it before [`PreparedRun::execute`], which resolves the scenario against
    /// it. The order cap and `--settings` have already shaped the scenario by
    /// then, so those are changed through [`RunOverrides`], never here.
    pub config: SimConfig,
    /// The world document's own name, without directory or extension.
    pub map_name: String,
    /// How many DA50s the world document deploys — map-derived, so a caller
    /// reads it here rather than from the configuration.
    pub vtm_count: usize,
    /// The picks the scenario says it will ask for, which a caller compares
    /// against what the run achieved.
    pub expected_picks: u64,
    /// What this run will be called in its own recording: the name the caller
    /// gave, or the spec and the code it runs on. Settled here so a caller that
    /// writes the run down elsewhere writes the same string the recording
    /// carries.
    pub name: String,
    identity: RunIdentity,
    world_doc: Value,
    documents: RunDocuments,
    ids: IdTables,
    map: WarehouseMap,
    racking: Racking,
    scenario_source: ScenarioSource,
    label: String,
    events: bool,
}

/// What a spec can fail to become before anything is simulated.
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    // The loaders' errors are boxed: they are large, and every step of
    // preparing a run returns this.
    #[error("{0}")]
    Spec(#[source] Box<WorldSpecError>),
    #[error("cannot read the world document {path}: {source}")]
    World { path: PathBuf, source: IoError },
    #[error("{path} is not a world document: {source}")]
    Parse { path: PathBuf, source: JsonError },
    #[error("the world document {path} does not load: {source}")]
    WorldLoad { path: PathBuf, source: Box<WorldMapError> },
    #[error("the scenario's workload files could not be read: {0}")]
    Workload(String),
    #[error("{0}")]
    Settings(String),
    #[error("{0}")]
    Provenance(String),
    #[error("the world document states no grid, so there is nothing to run on")]
    NoGrid,
    #[error("the world document deploys a fleet this build cannot run: {0}")]
    Fleet(String),
    #[error(
        "the world document declares no primary frame, so there is no frame for a run's positions to be \
         measured in"
    )]
    NoPrimaryFrame,
}

impl From<WorldSpecError> for HarnessError {
    fn from(error: WorldSpecError) -> Self {
        HarnessError::Spec(Box::new(error))
    }
}

/// Read a spec, load its world, and settle the configuration the run will use.
pub fn prepare(spec_path: &Path, overrides: &RunOverrides) -> Result<PreparedRun, HarnessError> {
    let loaded = world_spec::load(spec_path)?;
    // The document's bytes are read once, here, where the run model that will
    // carry them is assembled — the spec's own load names the files and opens
    // none of them, so `info`, `fpi` and `route` still cost one world load.
    let (world_bytes, world_doc) = read_world_document(&loaded.world_path)?;
    let data = loaded.documents.read_data().map_err(|error| HarnessError::Workload(error.to_string()))?;
    let documents = RunDocuments { spec: loaded.documents.spec, world: world_bytes, data };
    let inputs = LoadedInputs {
        world: loaded.world,
        world_path: loaded.world_path,
        world_doc,
        documents,
        scenario: loaded.scenario,
        settings: loaded.settings,
    };
    settle(spec_path, inputs, overrides)
}

/// Prepare a run from a world document alone, with no spec beside it: the
/// document is the whole input, so the run takes the default scenario and the
/// default configuration with the fleet the document deploys, which is what a
/// spec that states neither gets.
pub fn prepare_world(world_path: &Path, overrides: &RunOverrides) -> Result<PreparedRun, HarnessError> {
    let (world_bytes, world_doc) = read_world_document(world_path)?;
    let world = worldmap::load_parsed(world_path, &world_doc)
        .map_err(|source| HarnessError::WorldLoad { path: world_path.to_path_buf(), source: Box::new(source) })?;
    // The rule `world_spec::load` applies to a spec whose settings omit
    // `num_htms`: a run deploys the D30s the document declares.
    let settings = SimConfig { num_htms: world.map.fleet_config().htms, ..SimConfig::default() };
    let documents = RunDocuments { spec: Vec::new(), world: world_bytes, data: Vec::new() };
    let inputs = LoadedInputs {
        world,
        world_path: world_path.to_path_buf(),
        world_doc,
        documents,
        scenario: ScenarioConfig::default(),
        settings,
    };
    settle(world_path, inputs, overrides)
}

/// What a run is prepared from once its files are read: the world, the
/// documents the recording carries, and the scenario and configuration the
/// files state, before anything a caller says about the run.
struct LoadedInputs {
    world: WorldMap,
    world_path: PathBuf,
    world_doc: Value,
    documents: RunDocuments,
    scenario: ScenarioConfig,
    settings: SimConfig,
}

/// A world document's bytes, as the recording carries them, and the JSON they
/// hold.
fn read_world_document(path: &Path) -> Result<(Vec<u8>, Value), HarnessError> {
    let bytes = std::fs::read(path).map_err(|source| HarnessError::World { path: path.to_path_buf(), source })?;
    let doc =
        serde_json::from_slice(&bytes).map_err(|source| HarnessError::Parse { path: path.to_path_buf(), source })?;
    Ok((bytes, doc))
}

/// Fold what a caller says about the run into what its files state, and check
/// the world can run it. `named` is the file the caller named, whose stem the
/// run is called by.
fn settle(named: &Path, inputs: LoadedInputs, overrides: &RunOverrides) -> Result<PreparedRun, HarnessError> {
    let LoadedInputs { world, world_path, world_doc, documents, scenario: mut scenario_config, settings: mut config } =
        inputs;
    let ids = id_tables(&world)?;

    if let Some(orders) = overrides.max_orders {
        limit_orders(&mut scenario_config, &mut config, orders);
    }

    let map_name = named.file_stem().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();

    // The command line's overrides sit on top of what the spec settled, and
    // `--settings` sits on top of them, because it is the escape hatch for
    // anything the named overrides do not reach.
    if let Some(threads) = overrides.num_threads {
        config.num_threads = threads;
    }
    if let Some(no_watchdog) = overrides.no_watchdog {
        config.no_watchdog = no_watchdog;
    }
    if let Some(tripwires) = overrides.tripwires {
        config.tripwires = tripwires;
    }
    if let Some(timeout) = overrides.wall_timeout {
        config.wall_timeout_secs = Some(timeout.as_secs_f64());
    }
    if let Some(duration) = overrides.sim_duration {
        config.sim_duration = duration;
    }
    if let Some(slice_width_ms) = overrides.slice_width_ms {
        config.slice_width_ms = slice_width_ms;
    }
    if let Some(fidelity) = &overrides.fidelity {
        config.fidelity = fidelity.parse().map_err(HarnessError::Settings)?;
    }
    apply_cli_settings(&mut config, &overrides.settings, &world.robot_catalog, &world_path)
        .map_err(|e| HarnessError::Settings(e.to_string()))?;
    apply_scenario_settings(&mut scenario_config, &overrides.settings)
        .map_err(|e| HarnessError::Settings(e.to_string()))?;

    let expected_picks = scenario_config.total_order_picks().unwrap_or(0) as u64;
    let scenario_source = ScenarioSource::from_config(scenario_config);
    let vtm_count = world.map.vtm_count();

    // Read off the map here rather than mid-run: a document with no grid, or a
    // fleet this build cannot size, is a spec that cannot run, and a caller
    // that hears so while it still holds the socket can say why.
    if world.map.grid_bounds.is_none() {
        return Err(HarnessError::NoGrid);
    }
    VtmFleet::try_from(vtm_count).map_err(|failure| HarnessError::Fleet(failure.to_string()))?;

    let identity = RunIdentity::try_capture().map_err(HarnessError::Provenance)?;
    let name = overrides
        .run_name
        .clone()
        .unwrap_or_else(|| format!("{} @ {}/{}", map_name, identity.branch, identity.commit_short));

    Ok(PreparedRun {
        config,
        label: map_name.clone(),
        map_name,
        vtm_count,
        expected_picks,
        name,
        identity,
        world_doc,
        documents,
        ids,
        map: world.map,
        racking: world.racking,
        scenario_source,
        events: overrides.events,
    })
}

/// Prepare a run from a recording rather than from a spec: the world document,
/// the workload and the configuration all come out of the file, so the run is
/// the recorded one executed against today's code.
#[allow(clippy::too_many_arguments)]
pub fn prepare_recorded(
    world_doc: Value,
    documents: RunDocuments,
    ids: IdTables,
    map: WarehouseMap,
    racking: Racking,
    scenario_source: ScenarioSource,
    config: SimConfig,
    run_name: Option<String>,
    label: String,
) -> PreparedRun {
    let vtm_count = map.vtm_count();
    let identity = RunIdentity::try_capture().unwrap_or_else(|failure| panic!("{failure}"));
    let name = run_name.unwrap_or_else(|| format!("{} @ {}/{}", label, identity.branch, identity.commit_short));
    PreparedRun {
        config,
        map_name: label.clone(),
        vtm_count,
        expected_picks: 0,
        name,
        identity,
        world_doc,
        documents,
        ids,
        map,
        racking,
        scenario_source,
        label,
        events: false,
    }
}

impl PreparedRun {
    /// The code and machine the run will say it ran on.
    pub fn identity(&self) -> &RunIdentity {
        &self.identity
    }

    /// Run it, writing its recording to `recording` as it goes. `hooks` is how
    /// a caller watches: counters for a progress bar or a status reply, a stop
    /// signal for a cancel, nothing at all for a plain headless run. A
    /// recorder the hooks carry takes the slices when there is no file to
    /// record into; a run handed both is refused before it starts.
    ///
    /// Anything the simulator refuses — a composition that will not build, a
    /// scenario that will not resolve, a scheduler invariant — panics, as it
    /// does for every other caller of the runner: a run that cannot proceed is
    /// not a result to record. A caller that must survive it runs this on a
    /// thread of its own and reads the panic off the join. A recording that
    /// could not be written stops the run, and is this function's error.
    pub fn execute(
        self,
        hooks: RunnerHooks,
        recording: Option<RecordingTarget>,
    ) -> Result<(PerfProfile, SummaryStats), RecordingError> {
        run::run_simulation(
            self.world_doc,
            self.documents,
            self.ids,
            self.map,
            self.racking,
            &self.scenario_source,
            self.name,
            self.identity,
            &self.label,
            self.events,
            hooks,
            recording,
            self.config,
        )
    }
}

/// What the world document calls the ids the run mints. Nothing in the
/// simulator reads them; a recording carries them so a reader asking which
/// aisle cell 4102 is has an answer.
///
/// The primary frame is the one every recorded position is measured in, so it
/// is carried as the id the document states. Loading a world already refuses a
/// document that declares no primary frame — nothing in it could be turned
/// into millimetres — but a `WorldMap` can also be assembled by hand, and this
/// says which frame is missing rather than writing a run measured from nowhere.
fn id_tables(world: &WorldMap) -> Result<IdTables, HarnessError> {
    let frame = world.world.frames.primary().ok_or(HarnessError::NoPrimaryFrame)?;
    Ok(IdTables {
        cells: world.index.cell_entities.iter().map(|(&cell, entity)| (cell, entity.to_string())).collect(),
        stations: world.index.station_entities.iter().map(|(&station, entity)| (station, entity.to_string())).collect(),
        robots: Vec::new(),
        frame: frame.to_string(),
    })
}

/// Hold the run to the first `orders` orders, shrinking a generated scenario's
/// pick total in the same proportion so the two still describe one workload.
fn limit_orders(scenario: &mut ScenarioConfig, config: &mut SimConfig, orders: usize) {
    if let ScenarioConfig::Generated(generated) = scenario
        && orders < generated.num_orders
    {
        let ratio = orders as f64 / generated.num_orders as f64;
        let picks = (generated.total_order_picks as f64 * ratio).round() as usize;
        eprintln!("Limiting to {} of {} orders ({} picks)", orders, generated.num_orders, picks);
        generated.num_orders = orders;
        generated.total_order_picks = picks;
    }
    config.max_orders = Some(orders);
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;

    /// A world document alone prepares the run a spec stating no scenario and
    /// no settings prepares — the default scenario, the default configuration
    /// with the D30s the document deploys — under the caller's overrides, and
    /// the run's recording carries the document and no spec.
    #[test]
    fn a_world_document_alone_prepares_the_default_run_on_its_fleet() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_easyloop.json");
        let overrides = RunOverrides { sim_duration: Some(Duration::from_secs(60)), ..RunOverrides::default() };
        let prepared = prepare_world(&path, &overrides).expect("the fixture's world prepares");
        let fleet = worldmap::load(&path).expect("the fixture's world loads").map.fleet_config().htms;
        assert_ne!(fleet, 0, "the document deploys D30s");
        assert_eq!(prepared.config.num_htms, fleet);
        assert_eq!(prepared.config.sim_duration, Duration::from_secs(60), "the overrides apply");
        let ScenarioSource::Parametric(scenario) = &prepared.scenario_source else { panic!("a parametric scenario") };
        let json = |scenario: &ScenarioConfig| serde_json::to_value(scenario).expect("a scenario is JSON");
        assert_eq!(json(scenario), json(&ScenarioConfig::default()));
        assert!(prepared.documents.spec.is_empty(), "no spec stands beside the document");
        assert_eq!(prepared.documents.world, std::fs::read(&path).expect("the fixture reads"));
        assert_eq!(prepared.map_name, "minimal_easyloop");
    }
}
