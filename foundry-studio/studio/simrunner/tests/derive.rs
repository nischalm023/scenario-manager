//! The two files are the recording's (brief I1.09, lead decided): `fixtures/canned/metrics.json` and `series.json`
//! as committed equal what `derive` makes of the committed recording, three whole hours of simtime that stopped at its
//! limit. The fixture outlived the canned runner: the API's result tests are written against its numbers.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use dessim::events::{OrderCompleteEvent, SimEvent};
use dessim::metrics::profile::{
    IdTables, ProfileMetadata, ResolvedWorkload, RunDocuments, ScenarioSource, SummaryStats, VtmFleet,
};
use dessim::metrics::worldview::{RunRecording, RunScenario};
use dessim::model::{OrderId, SimTime};
use dessim::observer::SimObserver;
use dessim::observer::time_slice::MetricsConfig;
use dessim::sim_spec::SLICE_WIDTH_MS;
use dessim::sim_types::{SimResult, StopReason, StopSignal};
use dessim::world_spec;
use simrunner::recording::derive;
use studio_protocol::{MetricsFile, SeriesFile, SeriesKind};
use tempfile::tempdir;
use worldview::{End, Ms, Opaque, Payload, Reader, Tag, Writer};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned").join(name);
    fs::read(&path).unwrap_or_else(|fault| panic!("{} reads: {fault}", path.display()))
}

#[test]
fn the_two_files_are_derived_from_the_recording() {
    let (metrics, series, stop) = derive(&fixture("recording.worldview")).expect("the canned recording derives");
    let committed: MetricsFile = serde_json::from_slice(&fixture("metrics.json")).expect("metrics.json reads");
    assert_eq!(metrics, committed, "metrics.json is the recording's");
    let committed: SeriesFile = serde_json::from_slice(&fixture("series.json")).expect("series.json reads");
    assert_eq!(series, committed, "series.json is the recording's");
    assert_eq!(stop.reason, "SimTimeLimit", "the run stopped at its limit, not because its orders ran out");
    assert!(stop.at.as_ms() >= 10_800_000, "the recording holds three whole hours of simtime");
}

/// `metrics.json` as `derive` makes it of the committed recording, written over the committed file: run once, with
/// `--ignored`, after `derive` learns to say more, then commit what it wrote.
#[test]
#[ignore = "rewrites the committed fixture"]
fn rewrite_the_committed_metrics_file() {
    let (metrics, _, _) = derive(&fixture("recording.worldview")).expect("the canned recording derives");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned/metrics.json");
    fs::write(&path, serde_json::to_vec(&metrics).expect("JSON")).expect("written");
}

/// Every accumulating metric's series holds a value at each whole hour in rising instants, so an hour's value is a
/// difference of two recorded instants. Its last value is not always the end block's final one: dessim declares some
/// metrics additive, `inventory_units` among them, whose final value is no sum of their slices.
#[test]
fn every_running_series_holds_the_whole_hours_in_rising_instants() {
    let (metrics, series, _) = derive(&fixture("recording.worldview")).expect("the canned recording derives");
    assert!(metrics.metrics.len() > series.metrics.len(), "the summary's figures follow the declared metrics");
    assert!(metrics.metrics.iter().any(|metric| metric.metric == "run_total_picks"), "the Status rows are among them");
    for (declared, values) in metrics.metrics.iter().zip(&series.metrics) {
        assert_eq!(declared.metric, values.metric, "one series per declared metric, in descriptor order");
        if values.kind != SeriesKind::Running || values.points.is_empty() {
            continue;
        }
        for hour in 1..=3u64 {
            assert!(
                values.points.iter().any(|point| point.0.as_ms() == hour * 3_600_000),
                "{} has a running value at {hour} h",
                declared.metric
            );
        }
        let instants: Vec<u64> = values.points.iter().map(|point| point.0.as_ms()).collect();
        assert!(instants.windows(2).all(|pair| pair[0] < pair[1]), "{}'s instants rise", declared.metric);
    }
}

/// A recording whose end block's summary does not read is refused, naming it, rather than stored without the
/// summary's Status figures.
#[test]
fn an_end_block_whose_summary_does_not_read_refuses_the_recording() {
    let canned = fixture("recording.worldview");
    let mut reader = Reader::from_bytes(&canned).expect("the canned recording opens");
    let mut writer = Writer::new(Vec::new());
    let mut end = None;
    for record in reader.index().records().to_vec() {
        let frame = reader.frame_at(record.offset).expect("a frame reads");
        match record.tag {
            Tag::End => end = Some((record.t, End::from_bytes(&frame.contents(record.offset).unwrap(), 0).unwrap())),
            Tag::Index => {}
            _ => {
                writer.append(&frame).expect("copied");
            }
        }
    }
    let (t, mut end) = end.expect("an end block");
    // 0xc1 is the one byte MessagePack never uses.
    end.summary = Opaque(vec![0xc1]);
    let broken = writer.finish(t, &end.to_bytes().unwrap()).expect("closed");
    let refused = derive(&broken).expect_err("a summary that does not read");
    assert!(refused.to_string().contains("summary statistics"), "{refused}");
}

/// A recording made since slices end at the run's stop: its last slice ends at the end block, inside the slice's width,
/// and every running series closes there. Built from the canned recording with its slice past the stop left out and
/// its end block stamped half way into the slice before.
#[test]
fn the_last_slice_ends_at_the_end_block_where_the_run_stopped() {
    let canned = fixture("recording.worldview");
    let mut reader = Reader::from_bytes(&canned).expect("the canned recording opens");
    let records = reader.index().records().to_vec();
    let last_delta = records.iter().filter(|record| record.tag == Tag::Delta).map(|record| record.t).max().unwrap();
    let mut writer = Writer::new(Vec::new());
    let mut end = None;
    for record in records {
        let frame = reader.frame_at(record.offset).expect("a frame reads");
        match record.tag {
            Tag::End => end = Some(End::from_bytes(&frame.contents(record.offset).unwrap(), 0).unwrap()),
            Tag::Index => {}
            _ if record.t >= last_delta => {}
            _ => {
                writer.append(&frame).expect("copied");
            }
        }
    }
    let stop = Ms(last_delta.0 - 5_000);
    let cut = writer.finish(stop, &end.expect("an end block").to_bytes().unwrap()).expect("closed");
    let (_, series, stopped) = derive(&cut).expect("the cut recording derives");
    assert_eq!((series.end_sim_ms.as_ms(), stopped.at.as_ms()), (stop.0, stop.0), "the series and the stop");
    let picks = series.metrics.iter().find(|values| values.metric == "picks_completed").expect("a pick count");
    assert_eq!(picks.points.last().map(|point| point.0.as_ms()), Some(stop.0), "its last value is at the stop");
}

/// A run of the canned world's scenario with no robots, recorded straight from `facts` by the observer in slices of
/// the recording's width, stopped on its simtime limit `limit_ms`, with the summary's order count taken from its journal
/// as the runner takes it.
fn recorded_from(facts: Vec<SimEvent>, limit_ms: u64) -> Vec<u8> {
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned/spec.json");
    let loaded = world_spec::load(&spec).expect("the canned spec loads");
    let frame = loaded.world.world.frames.primary().map(|frame| frame.to_string()).unwrap_or_default();
    let scenario = RunScenario {
        name: "the canned world, unpeopled".to_owned(),
        metadata: ProfileMetadata {
            branch: "test".to_owned(),
            commit: "0123456789abcdef".to_owned(),
            commit_short: "0123456".to_owned(),
            dirty: false,
            timestamp: "2026-11-07T00:00:00Z".to_owned(),
            hostname: "testhost".to_owned(),
            grid_width: 1,
            grid_depth: 1,
            slice_width_ms: loaded.settings.slice_width_ms,
        },
        documents: RunDocuments {
            spec: loaded.documents.spec.clone(),
            world: fs::read(&loaded.world_path).expect("the world document reads"),
            data: Vec::new(),
        },
        ids: IdTables { frame, ..IdTables::default() },
        sim_config: loaded.settings,
        scenario: ScenarioSource::from_config(loaded.scenario),
        workload: ResolvedWorkload::default(),
        vtm_fleet: VtmFleet::default(),
        run: String::new(),
        attempt: 0,
    };
    let folder = tempdir().expect("a folder for the recording");
    let path = folder.path().join("recording.worldview");
    let recording = Arc::new(Mutex::new(RunRecording::create(&path, None, scenario).expect("the recording opens")));
    let on_limit = StopSignal::new();
    on_limit.stop(StopReason::SimTimeLimit);
    let sim_time_limit = Some(SimTime::ms(limit_ms));
    let config = MetricsConfig { slice_width_ms: SLICE_WIDTH_MS, sim_time_limit, ..MetricsConfig::default() };
    let (told, stream) = mpsc::channel();
    for fact in facts {
        told.send(fact).expect("the observer's stream is open");
    }
    drop(told);
    let observer = SimObserver::new(config, stream).with_recorder(recording.clone()).with_failure_stop(on_limit);
    let output = observer.run().expect("the observer reads the stream to its close");
    let recording = Arc::into_inner(recording).expect("the observer let go of the recording");
    let recording = recording.into_inner().expect("no thread panicked holding it");
    let total_orders_completed = output.completions_through(limit_ms).completed_orders;
    let sim_time_secs = limit_ms as f64 / 1000.0;
    let mut summary = SummaryStats { sim_time_secs, total_orders_completed, ..SummaryStats::default() };
    let result = SimResult { stop_reason: StopReason::SimTimeLimit, ..SimResult::default() };
    recording.finish(&output, &mut summary, &result).expect("the recording finishes");
    fs::read(&path).expect("the recording reads")
}

/// A run stopped on its simtime limit at its first slice's end, with orders completed half way into the slice, at the
/// stop and half a slice past it, the last committed before the stop: the figures simrunner stores count two, the
/// end block's total that `orders_completed` states and the summary's `run_total_orders` alike.
#[test]
fn the_stored_figures_count_the_orders_completed_at_the_stop_and_none_past_it() {
    let width = SLICE_WIDTH_MS;
    let orders = [width / 2, width, width + width / 2].into_iter().map(|sim_time_ms| {
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(sim_time_ms as u32), sim_time_ms })
    });
    let (metrics, _, stop) = derive(&recorded_from(orders.collect(), width)).expect("the recording derives");
    assert_eq!(stop.at.as_ms(), width, "the run stopped at its limit");
    let stored =
        |name: &str| metrics.metrics.iter().find(|metric| metric.metric == name).and_then(|metric| metric.value);
    assert_eq!(stored("orders_completed"), Some(2.0), "the end block's total");
    assert_eq!(stored("run_total_orders"), Some(2.0), "the summary's");
}
