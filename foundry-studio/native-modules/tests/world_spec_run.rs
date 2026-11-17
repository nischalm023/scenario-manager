//! The S1 admission seam, end to end: a spec FILE names a world document and a
//! workload, and the simulator runs it.
//!
//! `worldproto_map.rs` proves the chain from a world document a caller already
//! chose; this proves the chain from a file a user hands the simulator --
//! parse, resolve the world reference relative to the spec, load, generate,
//! solve. It is the world-protocol counterpart of handing a `SimSpec` JSON to
//! any of the existing entry points.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use dessim::events::{EventSink, SimEvent};
use dessim::pathfinding::holding_cells;
use dessim::runner::{RunnerHooks, resolve_scenario_with_racking};
use dessim::scenario::ScenarioConfig;
use dessim::{FHSippPathfinder, FidelityMode, SimDuration, world_spec};
use native_modules as _;

fn example_document() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/seed/worlds/minimal-putaway2/minimal_putaway2.json")
}

fn write_spec(dir: &Path, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("spec dir");
    let path = dir.join("spec.json");
    std::fs::write(&path, body).expect("spec write");
    path
}

#[test]
fn the_world_reference_resolves_relative_to_the_spec_file() {
    // The spec sits in a scratch directory and names the committed example by
    // a RELATIVE path, which is the shape a checked-in spec would use.
    let dir = std::env::temp_dir().join("dessim-world-spec-relative");
    let document = example_document().canonicalize().expect("the example exists");
    let relative = pathdiff(&dir, &document);
    let spec_path = write_spec(&dir, &format!(r#"{{ "world": {:?} }}"#, relative.to_string_lossy()));

    let loaded = world_spec::load(&spec_path).expect("spec loads");
    assert!(loaded.world.map.graph.cell_count() > 0);
    // Defaults filled in for the blocks the file left out.
    assert!(matches!(loaded.scenario, ScenarioConfig::Generated(_)));
}

/// Enough of a relative-path derivation for one test: walk up from `from` and
/// then down to `to`.
fn pathdiff(from: &Path, to: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for _ in from.components() {
        out.push("..");
    }
    out.join(to.strip_prefix("/").unwrap_or(to))
}

#[test]
fn a_missing_world_document_names_the_resolved_path() {
    let dir = std::env::temp_dir().join("dessim-world-spec-missing");
    let spec_path = write_spec(&dir, r#"{ "world": "nowhere/absent.json" }"#);
    let Err(err) = world_spec::load(&spec_path) else {
        panic!("a spec naming an absent document must not load");
    };
    let text = err.to_string();
    assert!(
        text.contains("nowhere") && text.contains("absent.json"),
        "the error must name the path it resolved, got: {text}"
    );
}

#[test]
fn a_spec_file_runs_a_simulation() {
    let dir = std::env::temp_dir().join("dessim-world-spec-run");
    let document = example_document().canonicalize().expect("the example exists");
    let spec_path = write_spec(
        &dir,
        &format!(
            r#"{{
                "world": {:?},
                "scenario": {{
                    "num_skus": 5,
                    "total_inventory_units": 2000,
                    "num_orders": 100,
                    "total_order_picks": 200,
                    "units_per_tote": 1,
                    "popular_skus_pct": 0.0,
                    "inventory_popular_share": 0.0,
                    "order_popular_share": 0.0
                }},
                "settings": {{ "queue_factor": 3.0, "putaway": {{ "enabled": true }} }}
            }}"#,
            document.to_string_lossy()
        ),
    );

    let loaded = world_spec::load(&spec_path).expect("spec loads");
    let mut config = loaded.settings;
    // Fleet size comes from the document's own cohorts, exactly as the runner
    // example does; the spec's settings cover everything the world cannot say.
    config.num_htms = loaded.world.map.fleet_config().htms;
    config.fidelity = FidelityMode::CollisionDes;
    config.sim_duration = Duration::from_secs(30);

    let mut data = resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation over the spec's world");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding_cells(&data.map),
    ));
    let (capture_tx, capture_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || capture_rx.iter().collect::<Vec<_>>());
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, _metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![capture_tx], recording: None },
    )
    .expect("the run completes");
    let events = capture.join().expect("event capture");

    assert_eq!(result.sim_duration, Duration::from_secs(30));
    assert!(!events.is_empty(), "a 30-second run over a live map must emit events");
}
