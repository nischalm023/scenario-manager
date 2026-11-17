//! A run recorded as it goes and read back is the same run.
//!
//! The recording is the only way a run leaves the process, so "the same run"
//! has to mean every number a consumer reads — the slices, the metrics, the
//! journals, the counters, the summary. Two small fixtures are run headless
//! here through the harness, recorded as they run, read back, and compared
//! whole as JSON documents with the run model the harness returned.
//!
//! Two things the format states differently are held out of that comparison,
//! each with a check of its own: a heading, which travels as milliradians;
//! and which relay cells are held, which a recording carries through the
//! transitions where the observer re-snapshots its running set every time it
//! creates a slice. A world position is NOT one of them: the recording carries
//! whole millimetres by ruling, so the run's own f64 is rounded on the way in
//! and every position comes back on that rounding, exactly. A journal comes
//! back in the order its rows were stated, each in the frame written once B
//! had passed the instant that makes it final (a leg's close), so both sides
//! are compared in instant order, legs sharing a start in the order they
//! close.

use std::collections::{BTreeSet, HashMap};
use std::io::{Error as IoError, ErrorKind, Result as IoResult, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use desgraph::{RobotType, StationId, WorldPos};
use dessim::events::{OrderCompleteEvent, RelayStateChangeEvent, ResourceEventKind, SimEvent};
use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::metrics::profile::{
    IdTables, PerfProfile, ProfileMetadata, ResolvedWorkload, RunDocuments, ScalarClass, ScenarioSource, SummaryStats,
    VtmFleet, classed_scalars,
};
use dessim::metrics::worldview::{RecordingError, RunRecording, RunScenario, read_worldview};
use dessim::model::{OrderId, RobotId, SimDuration, SimTime, SkuId};
use dessim::observer::SimObserver;
use dessim::observer::activity::ActivityKind;
use dessim::observer::finality::recording::{FinalSlice, RecordingFailed, SliceRecorder, SliceRows};
use dessim::observer::output::{MetricsOutput, PutawayRecord};
use dessim::observer::robot_state::RobotSliceSpans;
use dessim::observer::time_slice::{BotLocation, MetricsConfig, TimeSliceData};
use dessim::runner::RunnerHooks;
use dessim::scenario::ScenarioConfig;
use dessim::sim_spec::SLICE_WIDTH_MS;
use dessim::sim_types::{SimResult, StopReason, StopSignal};
use dessim::world_spec;
use native_modules as _;
use serde_json::Value;
use smallvec::SmallVec;
use worldview::container::writer::AppendSink;
use worldview::frames::declare::{Declare, Descriptors};
use worldview::frames::robot_state::RobotState;
use worldview::frames::scalars::AggregationClass;
use worldview::frames::state::RunState;
use worldview::frames::{Delta, Keyframe, instant};
use worldview::{IndexRecord, Reader, Scenario, Tag};

/// Two fixtures: the small generated floor, and a slice of DFW6, whose
/// conveyors put totes in transit and whose stations fill queues — so the
/// journals, the tote births, the relay reservations and the transits all
/// have rows to carry. The DFW6 slice is minutes of unoptimised simulation,
/// so the fast gate round-trips the small floor alone and the slow gate
/// (`slow-gate` feature, `scripts/test-slow-gate.sh`) both.
fn fixtures() -> &'static [&'static str] {
    if cfg!(feature = "slow-gate") {
        &["minimal_easyloop_spec.json", "walmart_dfw6_smoke_spec.json"]
    } else {
        &["minimal_easyloop_spec.json"]
    }
}

/// How long the DFW6 slice runs for: long enough to fill every journal and
/// put totes on the conveyors, short enough to be a test. The keyframe
/// cadence is crossed three times by the first fixture, which runs 109
/// slices.
const DFW6_SIM_SECONDS: u64 = 120;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("worldview-roundtrip-{}-{name}", std::process::id()))
}

/// Run one fixture through the harness, recording it at `path` as it goes and
/// handing `tap`, when there is one, what the file takes, and return the run
/// model the harness assembled.
fn recorded(name: &str, path: &Path, tap: Option<Box<dyn Write + Send>>) -> PerfProfile {
    let sim_duration = name.starts_with("walmart_dfw6").then(|| Duration::from_secs(DFW6_SIM_SECONDS));
    recorded_for(name, path, tap, sim_duration)
}

/// `recorded`, limited to `sim_duration` of simtime, or to the fixture's own.
fn recorded_for(
    name: &str,
    path: &Path,
    tap: Option<Box<dyn Write + Send>>,
    sim_duration: Option<Duration>,
) -> PerfProfile {
    let overrides = RunOverrides { sim_duration, ..RunOverrides::default() };
    let target = RecordingTarget { path: path.to_path_buf(), run: String::new(), attempt: 0, tap };
    let prepared = prepare(&fixture(name), &overrides).unwrap_or_else(|e| panic!("preparing {name}: {e}"));
    let (profile, _) =
        prepared.execute(RunnerHooks::default(), Some(target)).unwrap_or_else(|e| panic!("recording {name}: {e}"));
    profile
}

/// Each journal in instant order, rows sharing an instant in the order they
/// were stated, and legs and VTM cycles sharing a start in the order they
/// end: the order the workbook prints, and one a recording and the run it
/// recorded agree on.
fn by_instant(summary: &mut SummaryStats) {
    summary.htm_leg_log.sort_by_key(|row| (row.start, row.start + row.duration));
    summary.vtm_leg_log.sort_by_key(|row| (row.start_ms, row.start_ms + row.duration_ms));
    summary.vtm_cycle_log.sort_by_key(|row| (row.sim_time_ms, row.sim_time_ms + row.total_cycle_ms));
    summary.vtm_operation_log.sort_by_key(|row| row.sim_time_ms);
    summary.pick_log.sort_by_key(|row| row.sim_time_ms);
    summary.putaway_log.sort_by_key(|row| row.sim_time_ms);
    summary.completed_order_log.sort_by_key(|row| row.sim_time_ms);
    summary.aisle_state_log.sort_by_key(|row| row.sim_time_ms);
    summary.aisle_flux_log.sort_by_key(|row| row.sim_time_ms);
}

#[test]
fn a_run_recorded_as_it_goes_and_read_back_is_the_same_run() {
    for name in fixtures() {
        let path = scratch(&format!("{name}.worldview"));
        let mut before = recorded(name, &path, None);
        let recording = read_worldview(&path).unwrap_or_else(|e| panic!("reading {name}: {e}"));
        assert!(!recording.truncated, "{name}: a closed file is not truncated");
        let mut after = recording.profile;

        assert_eq!(
            before.summary.log_streams.len(),
            after.summary.log_streams.len(),
            "{name}: the streams the summary carries"
        );
        for (was, is) in before.summary.log_streams.iter().zip(&after.summary.log_streams) {
            assert_eq!(was.descriptor, is.descriptor, "{name}: {}'s descriptor", was.descriptor.id);
            assert_eq!(was.entries, is.entries, "{name}: {}'s rows", was.descriptor.id);
        }

        reservations_are_carried_forward(name, &after);

        by_instant(&mut before.summary);
        by_instant(&mut after.summary);
        let mut was = serde_json::to_value(&before).expect("the run model is JSON");
        let mut is = serde_json::to_value(&after).expect("the run model is JSON");
        lift_reservations(&mut was);
        lift_reservations(&mut is);
        same(name, &was, &is);
        let _ = std::fs::remove_file(&path);
    }
}

/// A tap that keeps what it is handed where the test can read it.
#[derive(Clone, Default)]
struct Kept(Arc<Mutex<Vec<u8>>>);

impl Write for Kept {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

/// What a run's tap is handed as the run is recorded is, byte for byte, the
/// file the recording leaves once it is closed: a reader in the process reads
/// the recording itself.
#[test]
fn a_runs_tap_is_handed_its_recording_byte_for_byte() {
    let path = scratch("tapped.worldview");
    let tap = Kept::default();
    recorded(fixtures()[0], &path, Some(Box::new(tap.clone())));
    let file = std::fs::read(&path).expect("the closed recording reads");
    let handed = tap.0.lock().unwrap();
    assert!(!file.is_empty(), "the run wrote a recording");
    assert!(*handed == file, "the tap holds {} bytes and the file {}, or they differ", handed.len(), file.len());
    let _ = std::fs::remove_file(&path);
}

/// A run stops inside a slice, on its own or at its simtime limit: the end
/// frame is stamped at the stop, no delta begins at or after it, the last
/// slice states no step, change, transit, move or state span past it, and a
/// replay that takes each slice up to the next frame takes every one, its
/// robots' spans laid end to end up to there, and stands at the stop. The run model
/// read back is the one the run assembled, which ends at the stop too.
#[test]
fn a_run_that_stops_inside_a_slice_states_nothing_past_its_stop() {
    let name = fixtures()[0];
    // The small floor completes 6.285 s into a slice; at its limit 5.5 s into another, with work committed past it.
    for (simulated, stop_ms) in [(None, 1_086_285), (Some(Duration::from_millis(455_500)), 455_500)] {
        let path = scratch(&format!("{name}.{stop_ms}.worldview"));
        let mut before = recorded_for(name, &path, None, simulated);
        assert_eq!((before.summary.sim_time_secs * 1000.0).round() as u64, stop_ms, "the run's stop");

        let mut reader = Reader::open(&path).unwrap_or_else(|e| panic!("opening {name}: {e}"));
        let records: Vec<IndexRecord> = reader.index().records().to_vec();
        let ended = records.iter().find(|record| record.tag == Tag::End).expect("a closed file states an end frame");
        assert_eq!(ended.t.0, stop_ms, "the end frame is stamped at the stop");
        let deltas: Vec<&IndexRecord> = records.iter().filter(|record| record.tag == Tag::Delta).collect();
        assert!(deltas.iter().all(|delta| delta.t.0 < stop_ms), "no slice begins at or after the stop");
        assert!(deltas.last().is_some_and(|last| stop_ms - last.t.0 < SLICE_WIDTH_MS), "the last slice holds it");

        let scenario = Scenario::read_from(&mut reader).expect("the scenario block reads");
        let mut descriptors = Descriptors::from_scenario(&scenario);
        let mut state: Option<RunState> = None;
        for (index, record) in records.iter().enumerate() {
            let at = instant(record.t);
            match record.tag {
                Tag::Declare => {
                    descriptors.push(&reader.payload_at::<Declare>(record).expect("a declaration reads"));
                    state.iter_mut().for_each(|state| state.scalars.describe(&descriptors));
                }
                Tag::Keyframe if state.is_none() => {
                    let keyframe = Keyframe::decode(&reader.sections_at(record).expect("sections"), record.offset);
                    let keyframe = keyframe.expect("the first keyframe decodes");
                    state = Some(RunState::from_keyframe(at, &keyframe, &descriptors).expect("a state"));
                }
                Tag::Delta => {
                    let delta = Delta::decode(&reader.sections_at(record).expect("sections"), record.offset);
                    let next = records[index + 1..].iter().find(|next| matches!(next.tag, Tag::Delta | Tag::End));
                    let ends = instant(next.expect("a frame after every slice").t);
                    let state = state.as_mut().expect("a keyframe opens the run");
                    let delta = delta.expect("a delta decodes");
                    if ends.as_ms() == stop_ms {
                        states_nothing_past(stop_ms, at.as_ms(), &delta);
                    }
                    let taken = state.apply(at, ends, &delta);
                    taken.unwrap_or_else(|e| panic!("{name}, stopped at {stop_ms} ms: the slice at {at}: {e}"));
                }
                _ => {}
            }
        }
        assert_eq!(state.expect("replayed").reached, SimTime::ms(stop_ms), "the replay stands at the stop");

        let mut after = read_worldview(&path).unwrap_or_else(|e| panic!("reading {name}: {e}")).profile;
        by_instant(&mut before.summary);
        by_instant(&mut after.summary);
        let (mut was, mut is) = (serde_json::to_value(&before).unwrap(), serde_json::to_value(&after).unwrap());
        lift_reservations(&mut was);
        lift_reservations(&mut is);
        same(&format!("{name}, stopped at {stop_ms} ms"), &was, &is);
        let _ = std::fs::remove_file(&path);
    }
}

/// The slice the stop cuts, beginning at `start`: each robot's steps and state spans, each tote, relay and station
/// change, each tote transit and each move it states ends by the stop.
fn states_nothing_past(stop: u64, start: u64, delta: &Delta) {
    let past = |what: &str, at: u64| assert!(at <= stop, "{what} at {at} ms, past the stop at {stop} ms");
    let playback = delta.playback.as_ref().expect("the slice states its playback");
    for mover in &playback.movers {
        past("a robot's last step", start + mover.steps.iter().map(|step| step.dt.as_ms()).sum::<u64>());
    }
    let changes = playback.tote_creations.iter().map(|birth| birth.offset);
    let changes = changes.chain(playback.tote_transitions.iter().map(|change| change.offset));
    let changes = changes.chain(playback.tote_contents.iter().map(|change| change.offset));
    let changes = changes.chain(playback.relay_transitions.iter().map(|change| change.offset));
    for offset in changes.chain(playback.station_changes.iter().map(|change| change.offset)) {
        past("a tote, relay or station change", start + offset.as_ms());
    }
    for transit in &playback.tote_transits {
        past("a tote transit's end", transit.end.as_ms());
    }
    for robot in &delta.motion.as_ref().expect("the slice states its motion").0 {
        robot.spans.iter().for_each(|span| past("a move's end", span.end.as_ms()));
    }
    for robot in &delta.robot_state.as_ref().expect("the slice states its robots' states").0 {
        past("a state span's end", start + robot.spans.iter().map(|span| span.dt.as_ms()).sum::<u64>());
    }
}

/// Two run models are the same run, or the first ten places they differ.
fn same(name: &str, was: &Value, is: &Value) {
    let mut differences = Vec::new();
    compare(was, is, String::new(), &mut differences);
    assert!(differences.is_empty(), "{name}: {} differences, first ten: {:#?}", differences.len(), {
        differences.truncate(10);
        differences
    });
}

/// Every position in a recording is measured in the world document's primary
/// frame, and the scenario block is the one place that names it. A run that
/// names none, and a recording whose name disagrees with the document beside
/// it, both leave a reader with millimetres it cannot place.
/// The smallest run a recording can open for: the first fixture's documents
/// and settings, measured in `frame`, and nothing simulated.
fn bare_scenario(frame: String) -> RunScenario {
    let loaded = world_spec::load(&fixture(fixtures()[0])).expect("the fixture loads");
    RunScenario {
        name: "a bare run".to_owned(),
        metadata: ProfileMetadata {
            branch: "test".to_owned(),
            commit: "0123456789abcdef".to_owned(),
            commit_short: "0123456".to_owned(),
            dirty: false,
            timestamp: "2026-09-17T00:00:00Z".to_owned(),
            hostname: "testhost".to_owned(),
            grid_width: 1,
            grid_depth: 1,
            slice_width_ms: loaded.settings.slice_width_ms,
        },
        documents: RunDocuments {
            spec: loaded.documents.spec.clone(),
            world: std::fs::read(&loaded.world_path).expect("the world document reads"),
            data: Vec::new(),
        },
        ids: IdTables { frame, ..IdTables::default() },
        sim_config: loaded.settings,
        scenario: ScenarioSource::from_config(loaded.scenario),
        workload: ResolvedWorkload::default(),
        vtm_fleet: VtmFleet::default(),
        run: String::new(),
        attempt: 0,
    }
}

/// The primary frame the first fixture's world document declares.
fn declared_frame() -> String {
    let loaded = world_spec::load(&fixture(fixtures()[0])).expect("the fixture loads");
    loaded.world.world.frames.primary().map(|frame| frame.to_string()).unwrap_or_default()
}

/// Every position in a recording is measured in the world document's primary
/// frame, and the scenario block is the one place that names it. A run that
/// names none is not recorded, and a recording whose name disagrees with the
/// document beside it leaves a reader with millimetres it cannot place.
#[test]
fn a_recording_states_the_frame_its_positions_are_measured_in() {
    let declared = declared_frame();
    assert!(!declared.is_empty(), "the fixture's own document declares a primary frame");

    let path = scratch("frame-claim.worldview");
    let Err(refused) = RunRecording::create(&path, None, bare_scenario(String::new())) else {
        panic!("a run naming no frame is not recorded")
    };
    assert!(matches!(refused, RecordingError::NoPrimaryFrame), "got {refused}");

    let recording = RunRecording::create(&path, None, bare_scenario(format!("{declared}-somewhere-else")))
        .expect("the writer takes any frame the run names");
    let (output, mut summary, result) = (MetricsOutput::default(), SummaryStats::default(), SimResult::default());
    recording.finish(&output, &mut summary, &result).expect("a run that made no slice still closes");
    let Err(refused) = read_worldview(&path) else { panic!("the name and the document disagree") };
    assert!(refused.contains(&declared), "the refusal names the document's frame: {refused}");
    assert!(refused.contains("somewhere-else"), "and the one the recording claims: {refused}");
    let _ = std::fs::remove_file(&path);
}

/// A run's one slice, from 0 s to its stop at 10 s, with `rows` due with it.
fn only_slice<'a>(slice: &'a TimeSliceData, rows: SliceRows, output: &'a MetricsOutput) -> FinalSlice<'a> {
    FinalSlice {
        slice,
        next: None,
        ends: SimTime::ms(SLICE_WIDTH_MS),
        last: true,
        rows,
        robot_spans: Vec::new(),
        roster: &output.robot_types,
    }
}

/// The summary of a run that stopped at 10 s, where its one slice ends.
fn stopped_at_ten_seconds() -> SummaryStats {
    SummaryStats { sim_time_secs: 10.0, ..SummaryStats::default() }
}

/// A row reported once every slice had gone out, B having passed them all
/// before the stream closed, became known at the run's end: the end block
/// states it, and it reads back with the rows the slices stated.
#[test]
fn a_row_left_when_every_slice_went_out_is_stated_at_the_end() {
    let path = scratch("rows-left.worldview");
    let mut recording =
        RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
    let putaway = |sim_time_ms| PutawayRecord {
        sim_time_ms,
        station_id: StationId(1),
        sku: SkuId(2),
        quantity: 1,
        presentation_id: None,
    };
    let output = MetricsOutput::default();
    let slice = TimeSliceData { start_ms: 0, ..TimeSliceData::default() };
    let rows = SliceRows { putaways: vec![putaway(5_000)], ..SliceRows::default() };
    recording.record(only_slice(&slice, rows, &output)).expect("the slice is written");
    recording.keep_rows_left(SliceRows { putaways: vec![putaway(9_000), putaway(8_000)], ..SliceRows::default() });
    recording.finish(&output, &mut stopped_at_ten_seconds(), &SimResult::default()).expect("the recording closes");

    let read = read_worldview(&path).expect("the recording reads back");
    let stated: Vec<u64> = read.profile.summary.putaway_log.iter().map(|row| row.sim_time_ms).collect();
    assert_eq!(stated, [5_000, 9_000, 8_000], "the slice's row, then the two the end block states");
    let _ = std::fs::remove_file(&path);
}

/// A run stopped on its simtime limit 4 s into a slice, where work committed before the stop files a relay change 6 s
/// into it, past the stop, finishes its recording: the slice the stop cuts states the change at the stop and not the
/// one past it, and the end block follows it.
#[test]
fn a_run_stopped_inside_a_slice_with_a_change_committed_past_the_stop_finishes_its_recording() {
    let path = scratch("committed-past-the-stop.worldview");
    let recording = RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
    let recording = Arc::new(Mutex::new(recording));
    let on_limit = StopSignal::new();
    on_limit.stop(StopReason::SimTimeLimit);
    let limit = Some(SimTime::ms(4_000));
    let config = MetricsConfig { slice_width_ms: SLICE_WIDTH_MS, sim_time_limit: limit, ..MetricsConfig::default() };
    let (facts, stream) = mpsc::channel();
    for (at_ms, reserved) in [(4_000, true), (6_000, false)] {
        let change = RelayStateChangeEvent { cell_id: 7, reserved, time: SimTime::ms(at_ms) };
        facts.send(SimEvent::RelayStateChange(change)).expect("the observer's stream is open");
    }
    drop(facts);
    let observer = SimObserver::new(config, stream).with_recorder(recording.clone()).with_failure_stop(on_limit);
    let output = observer.run().expect("the observer reads the stream to its close");
    let recording = Arc::into_inner(recording).expect("the observer let go of the recording");
    let recording = recording.into_inner().expect("no thread panicked holding it");
    let result = SimResult { stop_reason: StopReason::SimTimeLimit, ..SimResult::default() };
    let mut summary = SummaryStats { sim_time_secs: 4.0, ..SummaryStats::default() };
    recording.finish(&output, &mut summary, &result).expect("the recording finishes");

    let read = read_worldview(&path).expect("the recording reads back");
    let _ = std::fs::remove_file(&path);
    assert!(!read.truncated, "it closes with its end block");
    let slices = &read.profile.time_slices;
    assert_eq!(slices.len(), 1, "one slice, cut at the stop");
    assert_eq!(slices[0].relay_state.transitions, [(7, 4_000, true)], "the change at the stop, and none past it");
}

/// A run stopped on its simtime limit at its first slice's end, 10 s, with orders completed at 5 s, at the stop and
/// at 12 s, the last committed before the stop, records two: its one slice states the order at the stop, and the
/// relay change there at the slice's end, and its end block, its run model and the summary's journal agree.
#[test]
fn a_run_stopped_on_its_limit_at_a_slices_end_records_the_facts_at_the_stop_and_none_past_it() {
    let path = scratch("facts-at-the-stop.worldview");
    let recording = RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
    let recording = Arc::new(Mutex::new(recording));
    let on_limit = StopSignal::new();
    on_limit.stop(StopReason::SimTimeLimit);
    let stop_ms = SLICE_WIDTH_MS;
    let limit = Some(SimTime::ms(stop_ms));
    let config = MetricsConfig { slice_width_ms: SLICE_WIDTH_MS, sim_time_limit: limit, ..MetricsConfig::default() };
    let (facts, stream) = mpsc::channel();
    for at_ms in [5_000, stop_ms, 12_000] {
        let order = OrderCompleteEvent { order_id: OrderId(at_ms as u32), sim_time_ms: at_ms };
        facts.send(SimEvent::OrderComplete(order)).expect("the observer's stream is open");
    }
    let change = RelayStateChangeEvent { cell_id: 7, reserved: true, time: SimTime::ms(stop_ms) };
    facts.send(SimEvent::RelayStateChange(change)).expect("the observer's stream is open");
    drop(facts);
    let observer = SimObserver::new(config, stream).with_recorder(recording.clone()).with_failure_stop(on_limit);
    let output = observer.run().expect("the observer reads the stream to its close");
    let recording = Arc::into_inner(recording).expect("the observer let go of the recording");
    let recording = recording.into_inner().expect("no thread panicked holding it");
    let result = SimResult { stop_reason: StopReason::SimTimeLimit, ..SimResult::default() };
    let total_orders_completed = output.completions_through(stop_ms).completed_orders;
    let mut summary = SummaryStats { sim_time_secs: 10.0, total_orders_completed, ..SummaryStats::default() };
    recording.finish(&output, &mut summary, &result).expect("the recording finishes");

    let read = read_worldview(&path).expect("the recording reads back");
    let _ = std::fs::remove_file(&path);
    let slices = &read.profile.time_slices;
    assert_eq!(slices.len(), 1, "one slice, ending at the stop");
    assert_eq!(slices[0].metrics.get("orders_completed"), Some(&2.0), "the slice states the order at the stop");
    assert_eq!(slices[0].relay_state.transitions, [(7, stop_ms, true)], "and the change there, at its end");
    assert_eq!(read.profile.cumulative_scalars.get("orders_completed"), Some(&2.0), "the end block's total");
    assert_eq!(output.cumulative_scalars.orders_completed, 2, "the run model's");
    assert_eq!(read.profile.summary.total_orders_completed, 2, "and the summary's, from the journal");
}

/// The robot state channel a recording states names the robots its playback places, each with a state, and the
/// recording ends where its last slice does. A slice naming a robot no keyframe places, a keyframe placing a robot
/// the channel gave no state, and a run stopped elsewhere than its last slice ends are each refused, and closing the
/// recording returns the refusal.
#[test]
fn a_channel_that_misses_the_robots_placed_or_an_end_off_the_last_slice_is_refused() {
    let output = MetricsOutput::default();
    let refused = |slice: &TimeSliceData, robot_spans: Vec<RobotSliceSpans>| {
        let path = scratch(&format!("channel-refused-{}.worldview", robot_spans.len()));
        let mut recording =
            RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
        let told = recording.record(FinalSlice { robot_spans, ..only_slice(slice, SliceRows::default(), &output) });
        let told = told.expect_err("the writer refuses the slice");
        let closed = recording.finish(&output, &mut stopped_at_ten_seconds(), &SimResult::default());
        let _ = std::fs::remove_file(&path);
        (told.0, closed.expect_err("and closing the recording returns the refusal"))
    };
    let no_task = RobotState::NoTask { cell: None };
    let whole_slice = vec![(SimDuration::ms(SLICE_WIDTH_MS), no_task)];
    let named = vec![RobotSliceSpans { robot: RobotId(3), continues: false, spans: whole_slice }];
    let (told, closed) = refused(&TimeSliceData { start_ms: 0, ..TimeSliceData::default() }, named);
    assert!(told.contains("placed by no keyframe"), "{told}");
    assert!(matches!(closed, RecordingError::RobotState { robot: RobotId(3), .. }), "{closed}");

    let placed = BotLocation {
        motion: None,
        start_cell: 0,
        start_pos: WorldPos::default(),
        start_z_mm: 0.0,
        start_toward: None,
        start_heading_rad: None,
        robot_type: RobotType::Htm,
        steps: SmallVec::new(),
    };
    let slice = TimeSliceData { start_ms: 0, bot_locations: [(4, placed)].into_iter().collect(), ..Default::default() };
    let (told, closed) = refused(&slice, Vec::new());
    assert!(told.contains("placed with no state"), "{told}");
    assert!(matches!(closed, RecordingError::RobotState { robot: RobotId(4), .. }), "{closed}");

    let path = scratch("end-off-the-last-slice.worldview");
    let mut recording =
        RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
    let slice = TimeSliceData { start_ms: 0, ..TimeSliceData::default() };
    recording.record(only_slice(&slice, SliceRows::default(), &output)).expect("the slice is written");
    let mut stopped_early = SummaryStats { sim_time_secs: 7.0, ..SummaryStats::default() };
    let closed = recording.finish(&output, &mut stopped_early, &SimResult::default()).expect_err("the stop is at 7 s");
    let off = |stop, last_ends| (stop, last_ends) == (SimTime::ms(7_000), SimTime::ms(SLICE_WIDTH_MS));
    assert!(
        matches!(closed, RecordingError::EndOffTheLastSlice { stop, last_ends } if off(stop, last_ends)),
        "{closed}"
    );
    let _ = std::fs::remove_file(&path);
}

/// A sink that takes its first `frames` writes and refuses every one after,
/// as a disk that fills up does. The writer hands it one frame per write.
struct FillsUp {
    frames: usize,
}

impl Write for FillsUp {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.frames =
            self.frames.checked_sub(1).ok_or_else(|| IoError::new(ErrorKind::StorageFull, "the disk is full"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl AppendSink for FillsUp {}

/// A frame the sink refuses ends the recording: the observer is told which
/// frame failed, and closing the recording returns that failure instead of
/// writing an end.
#[test]
fn a_frame_the_sink_refuses_ends_the_recording_with_that_failure() {
    let sink = FillsUp { frames: 2 };
    let mut recording =
        RunRecording::new(Box::new(sink), bare_scenario(declared_frame())).expect("the recording opens");
    let output = MetricsOutput::default();
    let slice = TimeSliceData { start_ms: 0, ..TimeSliceData::default() };
    let told = recording
        .record(only_slice(&slice, SliceRows::default(), &output))
        .expect_err("the scenario block and the keyframe land, and the slice's delta does not");
    assert!(told.0.starts_with("writing a delta at"), "the observer is told which frame failed: {told}");
    let closed = recording.finish(&output, &mut SummaryStats::default(), &SimResult::default());
    assert!(
        matches!(&closed, Err(RecordingError::Write { frame: "a delta", .. })),
        "closing returns the failure: {closed:?}"
    );
}

/// A tap that refuses a frame part way through the run is the run's error:
/// `execute` returns that failure, the first frame of the second slice, and
/// the file holds what went before it. That the failure stops the scheduler
/// is the observer's to prove (`finality`'s tests): here the scheduler may
/// finish first.
#[test]
fn a_tap_that_fails_mid_run_is_the_runs_error() {
    let path = scratch("tap-fails.worldview");
    // The scenario block, slice 0's keyframe and slice 0's delta.
    let tap: Box<dyn Write + Send> = Box::new(FillsUp { frames: 3 });
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: Some(tap) };
    let prepared = prepare(&fixture(fixtures()[0]), &RunOverrides::default()).expect("the fixture prepares");
    let failed = prepared.execute(RunnerHooks::default(), Some(target));
    assert!(
        matches!(&failed, Err(RecordingError::Write { at, .. }) if *at == SimTime::ms(SLICE_WIDTH_MS)),
        "the frame the tap refused while the run went: {:?}",
        failed.err()
    );
    let read = read_worldview(&path).expect("what the file took before the failure reads");
    assert!(read.truncated, "the recording ends at the failure, with no end block");
    let _ = std::fs::remove_file(&path);
}

/// A recorder that counts the slices it is handed.
#[derive(Default)]
struct Counted(usize);

impl SliceRecorder for Counted {
    fn record(&mut self, _: FinalSlice<'_>) -> Result<(), RecordingFailed> {
        self.0 += 1;
        Ok(())
    }

    fn keep_rows_left(&mut self, _: SliceRows) {}
}

/// A recorder the caller hands the run in its hooks, with no file to record
/// into, takes every slice the run makes final.
#[test]
fn a_recorder_the_caller_hands_the_run_takes_every_slice() {
    let counted = Arc::new(Mutex::new(Counted::default()));
    let hooks = RunnerHooks { recording: Some(counted.clone()), ..RunnerHooks::default() };
    let prepared = prepare(&fixture(fixtures()[0]), &RunOverrides::default()).expect("the fixture prepares");
    let (profile, _) = prepared.execute(hooks, None).expect("the run goes");
    assert_eq!(counted.lock().unwrap().0, profile.time_slices.len());
}

/// A run handed a recorder of its own and a file to record into is refused
/// before it starts: a run is recorded once.
#[test]
fn a_run_handed_a_recorder_and_a_file_is_refused() {
    let path = scratch("recorder-and-file.worldview");
    let hooks = RunnerHooks { recording: Some(Arc::new(Mutex::new(Counted::default()))), ..RunnerHooks::default() };
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    let prepared = prepare(&fixture(fixtures()[0]), &RunOverrides::default()).expect("the fixture prepares");
    let refused = prepared.execute(hooks, Some(target));
    assert!(matches!(refused, Err(RecordingError::RecorderAndFile)), "refused: {:?}", refused.err());
    assert!(!path.exists(), "no file is made for a run that never starts");
}

/// A metric only the run's totals state, which no slice stated, is declared
/// before the end block and reads back with its value.
#[test]
fn a_metric_only_the_totals_state_is_declared_before_the_end() {
    let path = scratch("totals-only.worldview");
    let mut recording =
        RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
    let mut output = MetricsOutput::default();
    let slice = TimeSliceData { start_ms: 0, ..TimeSliceData::default() };
    recording.record(only_slice(&slice, SliceRows::default(), &output)).expect("the slice is written");
    output.cumulative_scalars.vtm_forward_trips = 2;
    output.cumulative_scalars.vtm_forward_cycle_ms = 3_000;
    recording.finish(&output, &mut stopped_at_ten_seconds(), &SimResult::default()).expect("the recording closes");

    let read = read_worldview(&path).expect("the recording reads back");
    assert!(!read.profile.time_slices[0].metrics.contains_key("vtm_forward_cycle_avg_ms"), "no slice stated it");
    assert_eq!(read.profile.cumulative_scalars.get("vtm_forward_cycle_avg_ms"), Some(&1_500.0));
    let _ = std::fs::remove_file(&path);
}

/// Every metric `path` declares, in its scenario block and in the
/// declarations after it, with the class it is declared with.
fn declared_classes(path: &Path) -> Vec<(String, AggregationClass)> {
    let mut reader = Reader::open(path).expect("the recording opens");
    let mut declared = Descriptors::from_scenario(&Scenario::read_from(&mut reader).expect("its scenario block"));
    let declarations: Vec<IndexRecord> =
        reader.index().records().iter().filter(|record| record.tag == Tag::Declare).copied().collect();
    for record in &declarations {
        declared.push(&reader.payload_at::<Declare>(record).expect("a declaration"));
    }
    declared.metrics.into_iter().map(|metric| (metric.name, metric.semantics.aggregation)).collect()
}

/// Every metric a recording declares carries the class its key was written
/// with, wherever it is declared: in the scenario block, before a later slice
/// or before the end block. So no reading is carried as a sum for want of a
/// class, whatever its key is named.
#[test]
fn every_metric_is_declared_with_the_class_its_key_was_written_with() {
    let path = scratch("classes.worldview");
    let mut recording =
        RunRecording::create(&path, None, bare_scenario(declared_frame())).expect("the recording opens");
    let mut output = MetricsOutput::default();
    let mut first = TimeSliceData { start_ms: 0, ..TimeSliceData::default() };
    first.scalars.vtm_activity_counts.insert(ActivityKind::Gap, 3);
    first.scalars.rack.per_zone_occupancy.insert(2, 5);
    let mut later = TimeSliceData { start_ms: SLICE_WIDTH_MS, ..TimeSliceData::default() };
    later.scalars.htm_activity_counts.insert(ActivityKind::Travel, 1);
    later.scalars.picks_per_station.insert(StationId(4), 6);
    later.scalars.vtm_forward_trips = 1;
    for (slice, last) in [(&first, false), (&later, true)] {
        let ends = SimTime::ms(slice.start_ms + SLICE_WIDTH_MS);
        let rows = SliceRows::default();
        let roster = &output.robot_types;
        recording
            .record(FinalSlice { slice, next: None, ends, last, rows, robot_spans: Vec::new(), roster })
            .expect("the slice is written");
    }
    output.cumulative_scalars.htm_return_trips = 2;
    let mut stopped_at_twenty_seconds = SummaryStats { sim_time_secs: 20.0, ..SummaryStats::default() };
    recording.finish(&output, &mut stopped_at_twenty_seconds, &SimResult::default()).expect("the recording closes");

    let declared = declared_classes(&path);
    let _ = std::fs::remove_file(&path);
    let mut written = HashMap::new();
    for counters in [&first.scalars, &later.scalars, &output.cumulative_scalars] {
        written.extend(classed_scalars(counters).into_iter().map(|scalar| (scalar.key, scalar.class)));
    }
    assert_eq!(declared.len(), written.len(), "each key written is declared once");
    for (key, aggregation) in declared {
        let class = written.get(&key).unwrap_or_else(|| panic!("{key} is declared and was never written"));
        let expected = match class {
            ScalarClass::Sum => AggregationClass::Additive,
            ScalarClass::Reading => AggregationClass::SnapshotGauge,
        };
        assert_eq!(aggregation, expected, "{key}");
    }
}

/// The averages a run's totals state, each with the two figures its slices add up that it divides.
const AVERAGES: [(&str, &str, &str); 6] = [
    ("vtm_forward_cycle_avg_ms", "vtm_forward_cycle_ms", "vtm_forward_trips"),
    ("vtm_return_cycle_avg_ms", "vtm_return_cycle_ms", "vtm_return_trips"),
    ("htm_cycle_avg_ms", "htm_cycle_ms", "htm_pickup_trips"),
    ("htm_operator_wait_avg_ms", "htm_operator_wait_ms", "htm_operator_wait_count"),
    ("htm_round_trip_avg_distance_mm", "htm_round_trip_distance_mm", "htm_return_trips"),
    ("htm_round_trip_avg_ms", "htm_round_trip_ms", "htm_return_trips"),
];

/// A run stopped on its simtime limit at a slice's end, 450 s, with work committed before the stop completing after
/// it, totals what its slices state, in memory and in its recording alike: a figure the run adds up is the sum of its
/// slices, a level the last slice's reading, the rack's peak the highest a slice reached, and an average the summed
/// figures it divides, divided.
#[test]
fn a_run_stopped_on_its_limit_at_a_slices_end_totals_what_its_slices_state() {
    let name = fixtures()[0];
    let path = scratch(&format!("totals-{name}.worldview"));
    let stop = SimTime::ms(450_000);
    let overrides = RunOverrides { sim_duration: Some(Duration::from_millis(stop.as_ms())), ..RunOverrides::default() };
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    let prepared = prepare(&fixture(name), &overrides).unwrap_or_else(|e| panic!("preparing {name}: {e}"));
    let (told, heard) = mpsc::channel();
    let hooks = RunnerHooks { subscribers: vec![told], ..RunnerHooks::default() };
    let (in_memory, _) = prepared.execute(hooks, Some(target)).unwrap_or_else(|e| panic!("recording {name}: {e}"));
    let completed_past_the_stop = heard
        .try_iter()
        .filter(|fact| match fact {
            SimEvent::CycleComplete(_) | SimEvent::OrderComplete(_) => true,
            // Of the resource events, only a VTM's completed cycle is work completing, stamped at its completion.
            SimEvent::Resource(event) => matches!(
                event.kind,
                ResourceEventKind::VtmForwardCompleted { .. } | ResourceEventKind::VtmReturnCompleted { .. }
            ),
            _ => false,
        })
        .filter(|fact| fact.stamp().is_some_and(|stamp| stamp > stop))
        .count();
    assert!(completed_past_the_stop > 0, "work committed before the stop completes after it");
    let classes = declared_classes(&path);
    let read = read_worldview(&path).expect("the recording reads back").profile;
    let _ = std::fs::remove_file(&path);
    let slices = &read.time_slices;
    assert_eq!(slices.last().map(|slice| slice.start_ms), Some(440_000), "the last slice ends at the stop");
    let stated =
        |key: &str| -> Vec<f64> { slices.iter().filter_map(|slice| slice.metrics.get(key).copied()).collect() };
    let summed = |key: &str| stated(key).into_iter().reduce(|total, value| total + value);
    let mut expected = HashMap::new();
    for (key, class) in classes {
        let total = match (key.as_str(), class) {
            ("rack_occupancy_peak", _) => stated(&key).into_iter().reduce(f64::max),
            ("pipeline_depth_avg", _) => {
                let count = summed("pipeline_snapshot_count").unwrap_or(0.0);
                Some(if count > 0.0 { summed("pipeline_depth_sum").unwrap_or(0.0) / count } else { 0.0 })
            }
            (key, _) if AVERAGES.iter().any(|(average, ..)| *average == key) => {
                let (_, parts, count) = AVERAGES.iter().find(|(average, ..)| *average == key).copied().unwrap();
                summed(count).filter(|count| *count > 0.0).map(|count| summed(parts).unwrap_or(0.0) / count)
            }
            (_, AggregationClass::Additive) => summed(&key),
            _ => slices.last().and_then(|slice| slice.metrics.get(&key).copied()),
        };
        if let Some(total) = total {
            expected.insert(key, total);
        }
    }
    assert_eq!(read.cumulative_scalars, expected, "the recording's totals");
    assert_eq!(in_memory.cumulative_scalars, expected, "the run model's totals");
}

#[test]
fn the_slices_counters_add_up_to_the_runs_own() {
    for name in fixtures() {
        let path = scratch(&format!("counters-{name}.worldview"));
        let profile = recorded(name, &path, None);
        let _ = std::fs::remove_file(&path);
        let mut summed: HashMap<String, HashMap<u32, u64>> = HashMap::new();
        for slice in &profile.time_slices {
            for (layer, counts) in &slice.grid {
                let into = summed.entry(layer.clone()).or_default();
                for (&cell, &count) in counts {
                    *into.entry(cell).or_insert(0) += count;
                }
            }
        }
        assert_eq!(summed, profile.cumulative_grid, "{name}: the slices' counters sum to the run's");
    }
}

#[test]
fn a_file_cut_short_reads_as_far_as_it_goes() {
    for name in fixtures() {
        let whole = scratch(&format!("truncated-whole-{name}.worldview"));
        let profile = recorded(name, &whole, None);

        // Where the tenth frame ends. A file cut there has no index block, no
        // end frame and no trailer, which is what a run that died leaves.
        let bytes = std::fs::read(&whole).expect("the file");
        let reader = Reader::from_bytes(&bytes).expect("the whole file opens");
        let records: Vec<IndexRecord> = reader.index().records().to_vec();
        let frames = 10.min(records.len().saturating_sub(2));
        assert!(frames > 2, "{name}: the fixture writes {} frames, and this test needs more", records.len());
        let cut = records[frames - 1].end_offset() as usize;
        let torn = scratch(&format!("truncated-{name}.worldview"));
        std::fs::write(&torn, &bytes[..cut]).expect("the torn file");

        let recording = read_worldview(&torn).expect("a torn file still opens");
        assert!(recording.truncated, "{name}: a file with no end block reads as truncated");
        let deltas = records[..frames].iter().filter(|record| record.tag == Tag::Delta).count();
        assert_eq!(recording.profile.time_slices.len(), deltas, "{name}: every slice ahead of the break is there");

        // The prefix is compared the way a whole file is: every field of
        // every slice, through the same comparator — a truncated read that
        // dropped a journal row or a cell counter would otherwise pass on its
        // instants alone.
        let mut was = serde_json::to_value(&profile).expect("the run model is JSON");
        let mut is = serde_json::to_value(&recording.profile).expect("the run model is JSON");
        for document in [&mut was, &mut is] {
            lift_reservations(document);
            let Some(model) = document.as_object_mut() else { continue };
            // A crashed run has no end block, so its totals, summary and
            // result are absent on one side and whole on the other: what the
            // slices before the break say is what this compares.
            for absent in ["summary", "sim_result", "cumulative_scalars", "cumulative_grid"] {
                model.remove(absent);
            }
            if let Some(slices) = model.get_mut("time_slices").and_then(Value::as_array_mut) {
                slices.truncate(deltas);
            }
        }
        same(name, &was, &is);
        let _ = std::fs::remove_file(&whole);
        let _ = std::fs::remove_file(&torn);
    }
}

/// Take the reserved sets out of the comparison.
///
/// A recording states which relay cells are held as one set carried through
/// the transitions, where the observer re-snapshots its running set each time
/// it creates a slice — which, for a slice created out of order, is a set
/// from a later instant. What the reader states instead is pinned by
/// [`reservations_are_carried_forward`]; the transitions themselves are
/// compared as strictly as everything else.
fn lift_reservations(profile: &mut Value) {
    let Some(slices) = profile.get_mut("time_slices").and_then(Value::as_array_mut) else { return };
    for slice in slices {
        let Some(slice) = slice.as_object_mut() else { continue };
        let empty = match slice.get_mut("relay_state").and_then(Value::as_object_mut) {
            Some(relay) => {
                relay.remove("start_reserved");
                relay.get("transitions").and_then(Value::as_array).is_none_or(|rows| rows.is_empty())
            }
            None => false,
        };
        if empty {
            slice.remove("relay_state");
        }
    }
}

/// Every slice's reserved relay cells are the ones carried through the
/// transitions before it, which is the one reading a recording can state.
fn reservations_are_carried_forward(name: &str, profile: &PerfProfile) {
    let mut held: BTreeSet<u32> = Default::default();
    for (index, slice) in profile.time_slices.iter().enumerate() {
        if index == 0 {
            held = slice.relay_state.start_reserved.iter().copied().collect();
        }
        let stated: BTreeSet<u32> = slice.relay_state.start_reserved.iter().copied().collect();
        assert_eq!(stated, held, "{name}: slice {index}'s reserved cells are the ones carried into it");
        for &(cell, _, reserved) in &slice.relay_state.transitions {
            match reserved {
                true => held.insert(cell),
                false => held.remove(&cell),
            };
        }
    }
}

/// Walk two run models together and collect every place they differ.
fn compare(was: &Value, is: &Value, path: String, found: &mut Vec<String>) {
    match (was, is) {
        (Value::Object(left), Value::Object(right)) => {
            let mut keys: Vec<&String> = left.keys().chain(right.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                let (l, r) = (left.get(key), right.get(key));
                match (l, r) {
                    (Some(l), Some(r)) => compare(l, r, format!("{path}.{key}"), found),
                    _ => found.push(format!("{path}.{key}: {l:?} vs {r:?}")),
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                found.push(format!("{path}: {} entries vs {}", left.len(), right.len()));
                return;
            }
            for (index, (l, r)) in left.iter().zip(right).enumerate() {
                compare(l, r, format!("{path}[{index}]"), found);
            }
        }
        (Value::Number(left), Value::Number(right)) if left.as_f64().is_some() && right.as_f64().is_some() => {
            let (l, r) = (left.as_f64().unwrap(), right.as_f64().unwrap());
            // A heading travels as milliradians, so it comes back rounded to
            // the nearest one.
            let allowance = if path.ends_with("start_heading_rad") { 1e-3 } else { 0.0 };
            // A position travels as whole millimetres, so what has to come
            // back is the run's own number rounded — not the number itself,
            // and not something merely close to it.
            let was = if rounded(&path) { l.round() } else { l };
            if (was - r).abs() > allowance {
                found.push(format!("{path}: {was} vs {r}"));
            }
        }
        _ if was != is => found.push(format!("{path}: {was} vs {is}")),
        _ => {}
    }
}

/// Which numbers the recording states in whole millimetres: every coordinate
/// of every robot position, which live under the per-slice bot locations and
/// nowhere else.
fn rounded(path: &str) -> bool {
    path.contains("bot_locations") && (path.ends_with(".x_mm") || path.ends_with(".y_mm") || path.ends_with("z_mm"))
}

/// A recording carries the workload files the scenario names, so a run
/// repeats on a machine that never held them.
///
/// The spec's CSVs are the one thing a recording could not previously stand
/// in for: `repeat` re-read them from the paths the spec gave, which are this
/// machine's and nobody else's. This checks the two halves of the fix — that
/// the bytes are keyed by the reference the spec WRITES, not by the path it
/// resolves to, and that laying them back out gives a scenario whose loader
/// finds every file with the originals gone.
#[test]
fn a_recordings_workload_files_stand_in_for_the_ones_the_spec_named() {
    let spec = fixture("cencora_v3_spec.json");
    let loaded = world_spec::load(&spec).expect("the spec loads");
    let embedded = loaded.documents.read_data().expect("the workload files read");
    assert_eq!(embedded.len(), 2, "this spec names an items file and a bulk file");
    for (reference, _) in &embedded {
        assert!(reference.starts_with("../"), "keyed by the reference as written, not the resolved path");
    }

    let laid_out = scratch("repeat-workload");
    let _ = std::fs::remove_dir_all(&laid_out);
    let mut scenario = loaded.scenario;
    world_spec::rehome_workload(&mut scenario, &embedded, &laid_out).expect("the files lay out");

    let ScenarioConfig::BulkSource(bulk) = &scenario else { panic!("this spec states a bulk workload") };
    for named in [&bulk.items_csv, &bulk.bulk_csv] {
        let path = Path::new(named);
        assert!(path.starts_with(&laid_out), "the scenario now names the recording's own copy: {named}");
        assert!(path.exists(), "and the copy is there");
    }
    assert_eq!(
        std::fs::read(&bulk.items_csv).expect("the copy reads"),
        embedded[0].1,
        "byte for byte what the recording carries"
    );
    let _ = std::fs::remove_dir_all(&laid_out);
}
