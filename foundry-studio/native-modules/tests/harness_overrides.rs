//! One spec, one configuration, whoever asks for the run.
//!
//! desvis's command line and the Crucible server's submission both reach
//! `dessim::harness::prepare`, and a recording written by one has to be the
//! recording written by the other. That holds only while an override is what
//! somebody typed: a caller that passes its own defaults as `Some(...)` writes
//! over whatever the spec's `settings` block states, and the two callers then
//! record different runs from the same file without a word.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dessim::harness::{RunOverrides, prepare};

/// A spec stating the two fields a command line is most likely to overwrite,
/// beside the fixture world it names.
fn spec_stating_threads_and_slices(dir: &Path) -> PathBuf {
    let world = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_putaway.json");
    let spec = serde_json::json!({
        "world": world,
        "scenario": { "num_skus": 4, "total_inventory_units": 40, "num_orders": 1, "total_order_picks": 1 },
        "settings": {
            "num_htms": 2,
            "num_threads": 3,
            "slice_width_ms": 60_000,
            "no_watchdog": true,
            "tripwires": true,
            "sim_duration": { "secs": 120, "nanos": 0 }
        }
    });
    let path = dir.join("stated_spec.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&spec).expect("a spec encodes")).expect("the spec is written");
    path
}

#[test]
fn a_spec_keeps_its_settings_when_no_flag_was_passed() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = spec_stating_threads_and_slices(dir.path());

    // What the server hands over, and what desvis hands over when nobody typed
    // `--threads`, `--time-slice`, `--no-watchdog` or `--tripwires`.
    let prepared = prepare(&path, &RunOverrides::default()).expect("the spec prepares");
    assert_eq!(prepared.config.num_threads, 3, "the spec's thread count stands");
    assert_eq!(prepared.config.slice_width_ms, 60_000, "the spec's slice width stands");
    assert!(prepared.config.no_watchdog, "the spec's watchdog answer stands");
    assert!(prepared.config.tripwires, "the spec's tripwire answer stands");
    assert_eq!(prepared.config.sim_duration, Duration::from_secs(120));
}

#[test]
fn a_flag_that_was_passed_wins() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = spec_stating_threads_and_slices(dir.path());

    let typed = RunOverrides {
        num_threads: Some(1),
        slice_width_ms: Some(10_000),
        sim_duration: Some(Duration::from_secs(600)),
        no_watchdog: Some(false),
        tripwires: Some(false),
        ..RunOverrides::default()
    };
    let prepared = prepare(&path, &typed).expect("the spec prepares");
    assert_eq!(prepared.config.num_threads, 1);
    assert_eq!(prepared.config.slice_width_ms, 10_000);
    assert!(!prepared.config.no_watchdog);
    assert!(!prepared.config.tripwires);
    assert_eq!(prepared.config.sim_duration, Duration::from_secs(600));
}

/// A robot figure the spec's world gives cannot be typed over, whichever
/// caller reaches `prepare`, and the refusal names the key and the world. A
/// setting that is not a robot figure still lands.
#[test]
fn a_typed_robot_figure_the_world_gives_is_refused() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = spec_stating_threads_and_slices(dir.path());

    let typed = RunOverrides { settings: vec!["htm.loaded_speed=9".into()], ..RunOverrides::default() };
    let refused = prepare(&path, &typed).err().expect("a robot figure typed over the world's").to_string();
    assert!(refused.contains("htm.loaded_speed") && refused.contains("minimal_putaway.json"), "{refused}");

    let typed = RunOverrides { settings: vec!["htm.loaded_constant_speed=5".into()], ..RunOverrides::default() };
    let prepared = prepare(&path, &typed).expect("a constant cruise is a setting, not a robot figure");
    assert_eq!(prepared.config.htm.loaded_constant_speed, 5.0);
}
