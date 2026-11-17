//! Pick-embargo accounting (greymatter-foundry#208).
//!
//! The observer counts every completed pick into `picks_completed`, but
//! throughput divides by only the post-embargo window. A pick that lands
//! before the embargo lifts would therefore both inflate the numerator and
//! enjoy the shrunken denominator. These tests pin the two guards that stop
//! that: `pre_embargo_picks` counts the violation, and (in debug) a
//! `debug_assert` makes it loud.

use std::sync::mpsc;

use desgraph::{RobotType, StationId, StationType};
use rustc_hash::FxHashMap;

use crate::events::*;
use crate::model::RobotId;
use crate::model::ids::SkuId;
use crate::observer::output::MetricsOutput;
use crate::observer::time_slice::MetricsConfig;

use super::SimObserver;

const EMBARGO_MS: u64 = 60_000;
const STATION: StationId = StationId(0);

fn config() -> MetricsConfig {
    let mut station_types = FxHashMap::default();
    station_types.insert(STATION, StationType::Pick);
    MetricsConfig { slice_width_ms: 10_000, station_types, pick_embargo_ms: EMBARGO_MS, ..MetricsConfig::default() }
}

fn pick_at(sim_time_ms: u64, qty: u32) -> SimEvent {
    SimEvent::CycleComplete(CycleCompleteEvent {
        robot_id: RobotId(0),
        robot_type: RobotType::Htm,
        kind: CycleKind::Pick { sku: SkuId(0), qty },
        station_id: STATION,
        sim_time_ms,
        dwell_ms: 0,
        arrival_source: None,
        dc_id: None,
        presentation_id: None,
        tote_presentation: None,
    })
}

fn run(events: Vec<SimEvent>) -> MetricsOutput {
    let mut config = config();
    super::test_models::bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run().expect("the facts break no rule of the event stream")
}

#[test]
fn post_embargo_picks_do_not_register_as_violations() {
    let out = run(vec![pick_at(EMBARGO_MS, 3), pick_at(EMBARGO_MS + 5_000, 2)]);
    assert_eq!(out.cumulative_scalars.picks_completed, 5);
    assert_eq!(out.cumulative_scalars.pre_embargo_picks, 0);
}

// In debug builds the `debug_assert` fires, so feeding a pre-embargo pick
// panics the observer — this is the "loud in dev" guard. `run` drives the
// observer on the calling thread, so the panic surfaces here.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "pick dwell started before embargo")]
fn pre_embargo_pick_trips_debug_assert() {
    run(vec![pick_at(EMBARGO_MS - 1, 1)]);
}

// In release builds the assert is compiled out; the pick is still counted
// (qty-weighted) so it can be excluded from the throughput numerator instead
// of silently inflating it.
#[cfg(not(debug_assertions))]
#[test]
fn pre_embargo_pick_counted_but_excluded_from_throughput() {
    use crate::sim_types::picks_per_hour;
    use std::time::Duration;

    // 4 picks total, 1 of them (qty=1) pre-embargo.
    let out = run(vec![pick_at(EMBARGO_MS - 1, 1), pick_at(EMBARGO_MS + 1_000, 3)]);
    assert_eq!(out.cumulative_scalars.picks_completed, 4);
    assert_eq!(out.cumulative_scalars.pre_embargo_picks, 1);

    // Throughput excludes the pre-embargo pick: 3 picks over the post-embargo
    // window, not 4.
    let post = out.cumulative_scalars.picks_completed - out.cumulative_scalars.pre_embargo_picks;
    let honest = picks_per_hour(post, Duration::from_secs(3660), EMBARGO_MS as f64 / 1000.0);
    let inflated =
        picks_per_hour(out.cumulative_scalars.picks_completed, Duration::from_secs(3660), EMBARGO_MS as f64 / 1000.0);
    assert!(honest < inflated);
}
