//! Within `spans`, `parity` holds the robot state channel a run records, and the span engine's windows read from it, to
//! what the run's interval rows say: stretch by stretch, but where the channel redefines a wait and where today's
//! reports leave time to their honesty residuals (`walk`); and window by window against today's report builders, at
//! the slice width and at an hour, but for exactly those differences, a reader beginning at any keyframe giving the
//! same windows from there (`windows`).
//!
//! The fast gate checks the small floor; the slow gate adds a slice of DFW6. The delivered runs are simulated only
//! when asked, in release, and print the shares that move:
//! `cargo test --release -p dessim --lib spans::parity -- --ignored --nocapture`.

mod walk;
mod windows;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use desunits::SimTime;

use crate::harness::{RecordingTarget, RunOverrides, prepare};
use crate::metrics::reporting::HOUR_MS;
use crate::metrics::worldview::read_worldview;
use crate::runner::RunnerHooks;
use walk::Walk;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// Simulate one fixture for `simulated`, recording it, with `settings` on top of the spec's own. Each recording has a
/// file of its own, since the tests run side by side.
fn recorded(name: &str, simulated: Option<Duration>, settings: &[&str]) -> PathBuf {
    static RECORDINGS: AtomicU32 = AtomicU32::new(0);
    let nth = RECORDINGS.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("robot-state-parity-{}-{nth}-{name}.worldview", std::process::id()));
    let settings = settings.iter().map(|setting| setting.to_string()).collect();
    let overrides = RunOverrides { sim_duration: simulated, settings, ..RunOverrides::default() };
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    let prepared = prepare(&fixture(name), &overrides).unwrap_or_else(|e| panic!("preparing {name}: {e}"));
    prepared.execute(RunnerHooks::default(), Some(target)).unwrap_or_else(|e| panic!("recording {name}: {e}"));
    path
}

/// Record one run; walk it and check the differences, printed by windows `print_window_ms` wide; hold the engine's
/// windows to the builders' at the slice width and at an hour; and hold a reader seeded at each later keyframe to
/// the one from simtime 0: the walk, and how many keyframes a reader was seeded at.
fn check_run(name: &str, simulated: Option<Duration>, settings: &[&str], print_window_ms: u64) -> (Walk, usize) {
    let path = recorded(name, simulated, settings);
    let run = read_worldview(&path).expect("the recording reads back").profile;
    let walk = walk::walk(&path, &run);
    walk.print(name, print_window_ms);
    walk.check(name);
    let told = windows::told(&path, SimTime::ZERO);
    for width_ms in [walk.slice_ms, HOUR_MS] {
        windows::hold_to_the_builders(&told, &run.summary, &walk, name, width_ms);
    }
    let seeded = windows::hold_seeded_readers(&path, &told);
    let _ = std::fs::remove_file(&path);
    (walk, seeded)
}

/// The small floor in the fast gate, and a slice of DFW6, whose conveyors and stations exercise visits and relay
/// waits, in the slow gate. The writer states a keyframe every 30 slices, 300 s: the floor runs past several, each
/// seeding a reader, but the DFW6 slice ends before its first after the start, so it seeds none.
#[test]
fn the_channel_and_the_engines_windows_say_what_the_rows_say_but_for_the_redefined_waits_and_the_residuals() {
    let (walk, seeded) = check_run("minimal_easyloop_spec.json", None, &[], HOUR_MS);
    assert!(walk.leaves_a_residual(), "the floor leaves time to a residual, and the channel states it");
    assert!(seeded > 0, "the floor runs past a keyframe after its first");
    if cfg!(feature = "slow-gate") {
        check_run("walmart_dfw6_smoke_spec.json", Some(Duration::from_secs(120)), &[], HOUR_MS);
    }
}

/// The settings every Cencora DA50 baseline run is launched with.
const CENCORA_BASELINE: [&str; 3] =
    ["putaway.ignore_hour_gate=true", "putaway.random_slots=true", "putaway.forbid_rack_after_secs=7150"];

#[test]
#[ignore = "DFW6 30 min and the Cencora DA50 baseline 1 h simulated; run in release with --ignored --nocapture"]
fn the_channel_and_the_engines_windows_say_what_the_rows_say_on_dfw6_30_min_and_the_cencora_baseline_1_h() {
    let (_, seeded) = check_run("walmart_dfw6_throughput_spec.json", Some(Duration::from_secs(1_800)), &[], 600_000);
    assert!(seeded > 0, "DFW6 runs past a keyframe after its first");
    let simulated = Some(Duration::from_secs(3_600));
    let (_, seeded) = check_run("cencora_v3_da50_baseline_pick9_spec.json", simulated, &CENCORA_BASELINE, HOUR_MS);
    assert!(seeded > 0, "the Cencora baseline runs past a keyframe after its first");
}

#[test]
#[ignore = "the Cencora DA50 baseline 3 h simulated, for the shares that move; run in release with --ignored --nocapture"]
fn the_channel_and_the_engines_windows_say_what_the_rows_say_on_the_cencora_baseline_3_h() {
    let simulated = Some(Duration::from_secs(10_800));
    let (_, seeded) = check_run("cencora_v3_da50_baseline_pick9_spec.json", simulated, &CENCORA_BASELINE, HOUR_MS);
    assert!(seeded > 0, "the Cencora baseline runs past a keyframe after its first");
}
