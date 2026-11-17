//! Simulation execution: fresh runs and repeat runs.
//!
//! What a run IS lives in `dessim::harness`, so that desvis and the Crucible
//! server record the same recording for the same spec. What lives here is the
//! command line's business: turning flags into the harness's overrides,
//! attaching the progress bar, printing the resolved configuration, naming
//! the recording the run writes, and writing the comparison and the workbook
//! out.

mod progress;

use std::path::{Path, PathBuf};
use std::time::Duration;

use dessim::harness::{RecordingTarget, RunOverrides, prepare, prepare_recorded};
use dessim::metrics::profile::{IdTables, ScenarioSource};
use dessim::metrics::worldview::read_worldview;
use dessim::runner::{RunnerHooks, RunnerLiveHandles};
use dessim::sim_types::Counters;
use dessim::{SimConfig, TraceConfig, world_spec, worldmap};
use workbook::{XlsxData, write_xlsx};

use crate::output::{check_completion, expected_picks_from_source, print_comparison, print_summary, recording_path};
use progress::ProgressReporter;

/// Execute a fresh simulation benchmark from a map file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_fresh(
    map_path: String,
    output_dir: &Path,
    run_name: Option<String>,
    num_threads: Option<usize>,
    _allow_incomplete: bool,
    simtime: Option<Duration>,
    stations: Option<usize>,
    orders: Option<usize>,
    events: bool,
    progress: bool,
    settings: Vec<String>,
    no_watchdog: bool,
    tripwires: bool,
    wall_timeout: Option<Duration>,
    fidelity: Option<String>,
    time_slice_ms: Option<u64>,
    xlsx_path: Option<&Path>,
    trace: bool,
    trace_filter: Vec<String>,
    trace_window: Option<String>,
    trace_format: String,
    trace_out: Option<PathBuf>,
    title_label: Option<String>,
) {
    if stations.is_some() {
        // v3's --stations sliced the design before compiling; a world document
        // states its stations as entities and slicing the compiled map would
        // orphan their queue cells. Rule a subset in the spec instead.
        eprintln!("--stations is not supported for world documents; ignoring");
    }

    // Every override is what somebody typed: a flag left alone is `None` and
    // the spec's own `settings` block stands, so this run and the same spec
    // run through the Crucible server record the same file.
    let overrides = RunOverrides {
        num_threads,
        sim_duration: simtime,
        slice_width_ms: time_slice_ms,
        max_orders: orders,
        wall_timeout,
        fidelity,
        settings,
        events,
        no_watchdog: no_watchdog.then_some(true),
        tripwires: tripwires.then_some(true),
        run_name,
    };
    let mut prepared = prepare(Path::new(&map_path), &overrides)
        .unwrap_or_else(|e| panic!("failed to load world spec {map_path}: {e}"));
    // Tracing is desvis's own: the flags spell a filter, a window and an output
    // sink no other caller of the harness offers.
    prepared.config.trace =
        TraceConfig::from_cli(trace, trace_filter, trace_window.as_deref(), &trace_format, trace_out)
            .unwrap_or_else(|e| panic!("{}", e));

    // Positive confirmation that map-file + CLI + --settings overrides
    // propagated into the actual SimConfig the solver will see. Without
    // this, silent CLI-arg→SimConfig mapping bugs have caused great
    // confusion in past stacked-run sessions.
    crate::sim_config_display::print_summary(
        &prepared.config,
        prepared.vtm_count,
        &prepared.map_name,
        title_label.as_deref(),
    );

    let expected_picks = prepared.expected_picks;
    // --progress: attach the run's progress figures and render a bar from a
    // polling thread.
    let (hooks, reporter) = match progress {
        true => {
            let counters = Counters::new();
            let reporter = ProgressReporter::start(counters.clone(), prepared.config.sim_duration);
            let live = RunnerLiveHandles { counters: Some(counters), stop_signal: None };
            (RunnerHooks { live: Some(live), subscribers: Vec::new(), recording: None }, Some(reporter))
        }
        false => (RunnerHooks::default(), None),
    };
    let profile_path = recording_path(prepared.identity(), output_dir);
    let target = RecordingTarget { path: profile_path.clone(), run: String::new(), attempt: 0, tap: None };
    let (new_profile, summary) =
        prepared.execute(hooks, Some(target)).unwrap_or_else(|e| panic!("failed to write the recording: {e}"));
    // The bar is cleared before the recording's own lines print.
    drop(reporter);

    // Print summary table.
    print_summary(&new_profile, &summary, &profile_path);

    // Check completion.
    check_completion(summary.total_picks, expected_picks, &new_profile.sim_result.stop_reason);

    // xlsx export.
    if let Some(xlsx) = xlsx_path {
        if let Err(e) = write_xlsx(xlsx, &XlsxData::from_profile(&new_profile)) {
            eprintln!("xlsx export failed: {e}");
        }
    }
}

/// Re-run a previously recorded profile against current code.
pub(crate) fn run_repeat(
    repeat_file: &Path,
    output_dir: &Path,
    run_name: Option<String>,
    num_threads: usize,
    _allow_incomplete: bool,
) {
    println!("Repeating recording: {} ...", repeat_file.display());
    let recording = read_worldview(repeat_file)
        .unwrap_or_else(|e| panic!("failed to read recording {}: {}", repeat_file.display(), e));
    // A run that died left no end block, so its configuration is whatever the
    // scenario block stated and its summary and result are defaults. Repeating
    // that would compare this run against zeros and call it a regression.
    if recording.truncated {
        panic!(
            "{} has no end block — the run crashed or is still going, and its last intact slice is {} ms. \
             There is nothing to repeat against",
            repeat_file.display(),
            recording.profile.time_slices.last().map(|slice| slice.start_ms).unwrap_or(0),
        );
    }
    let old_profile = recording.profile;

    let old_summary = old_profile.summary.clone();

    let label = format!("{} (repeat)", old_profile.name);

    // Compute expected picks from scenario data.
    let expected_picks = expected_picks_from_source(&old_profile.scenario);

    // Re-use the embedded world document and workload files verbatim from the
    // recording: the world loads from the bytes it carries, and every CSV the
    // scenario names is written back out of the recording and pointed at, so
    // a run repeats on a machine that never held the files it was built from.
    let world = worldmap::load_from_bytes(&old_profile.documents.world)
        .unwrap_or_else(|e| panic!("failed to load the embedded world document: {}", e));
    let workload = std::env::temp_dir().join(format!("desvis-repeat-workload-{}", std::process::id()));
    let mut scenario_source = old_profile.scenario;
    if let ScenarioSource::Parametric(config) = &mut scenario_source {
        world_spec::rehome_workload(config, &old_profile.documents.data, &workload)
            .unwrap_or_else(|e| panic!("failed to lay out the recording's workload files: {e}"));
    }

    let config = SimConfig { num_threads, ..old_profile.sim_config };
    let prepared = prepare_recorded(
        old_profile.world,
        old_profile.documents,
        IdTables { robots: Vec::new(), ..old_profile.ids },
        world.map,
        world.racking,
        scenario_source,
        config,
        run_name,
        label,
    );
    let profile_path = recording_path(prepared.identity(), output_dir);
    let target = RecordingTarget { path: profile_path.clone(), run: String::new(), attempt: 0, tap: None };
    let (new_profile, new_summary) = prepared
        .execute(RunnerHooks::default(), Some(target))
        .unwrap_or_else(|e| panic!("failed to write the recording: {e}"));

    // Print summary + comparison.
    print_summary(&new_profile, &new_summary, &profile_path);
    print_comparison(&old_summary, &new_summary, repeat_file, &profile_path);

    // Check completion.
    check_completion(new_summary.total_picks, expected_picks, &new_profile.sim_result.stop_reason);
}
