//! Every row a view states while the run goes, the frames taken one by one as a runner sends them, is the row a view
//! opened once the run has ended states, rebuilt from the finished recording: no window goes out before it is final,
//! and none is revised. The series of time in a state are also held to the robot state channel itself, integrated
//! span by span. And a segment a runner sends again after a lost connection changes no row. The recording is the
//! small floor's, simulated here by today's writer, so it carries the channel.

use std::collections::BTreeMap;
use std::env::temp_dir;
use std::fs::{File, remove_file};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::metrics::reporting::{
    D30State, D30StateDefinition, D30WaitCategory, D30WaitDefinition, Da50State, Da50StateDefinition,
    Da50WaitDefinition, Da50WaitReason, SpanDefinition,
};
use dessim::runner::RunnerHooks;
use desunits::{RobotId, SimDuration};
use native_modules as _;
use rustc_hash::FxHashMap;
use studio_metrics::{Ask, Follower, Metric, StateSeries, TimeSeries, Unservable, catalogue, find};
use studio_protocol::{MetricPoint, TrailingSpec};
use worldview::container::reader::frame_at;
use worldview::frames::playback::RobotClass;
use worldview::frames::robot_state::RobotState;
use worldview::frames::scalars::ScalarsDelta;
use worldview::frames::{Delta, Keyframe, instant};
use worldview::{Declare, Descriptors, Reader, Scenario, Tag};

/// The small floor simulated for its spec's half hour, recorded by today's writer, into a file of its own, since the
/// tests run side by side.
fn recorded() -> PathBuf {
    recorded_until(None)
}

/// The small floor simulated for `simulated`, or its spec's half hour.
fn recorded_until(simulated: Option<Duration>) -> PathBuf {
    static RECORDINGS: AtomicU32 = AtomicU32::new(0);
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_easyloop_spec.json");
    let nth = RECORDINGS.fetch_add(1, Ordering::Relaxed);
    let path = temp_dir().join(format!("studio-metrics-live-and-rebuilt-{}-{nth}.worldview", process::id()));
    let target = RecordingTarget { path: path.clone(), run: String::new(), attempt: 0, tap: None };
    let overrides = RunOverrides { sim_duration: simulated, ..RunOverrides::default() };
    let prepared = prepare(&spec, &overrides).expect("the small floor prepares");
    prepared.execute(RunnerHooks::default(), Some(target)).expect("the small floor records");
    path
}

/// The recording's frames in file order, its index block left out, as a runner sends them.
fn frames(path: &Path) -> Vec<(Tag, Vec<u8>)> {
    let reader = Reader::open(path).expect("the recording opens");
    let mut file = File::open(path).expect("it reads");
    let records = reader.index().records().to_vec();
    records
        .into_iter()
        .filter(|record| record.tag != Tag::Index)
        .map(|record| (record.tag, frame_at(&mut file, record.offset).expect("a frame").to_bytes()))
        .collect()
}

/// Every metric in its own bucketing, and trailing windows of each family besides.
fn every_ask() -> Vec<Ask> {
    let windows = |metric, window_ms, step_ms| Ask {
        metric,
        trailing: Some(TrailingSpec {
            window_sim_ms: SimDuration::ms(window_ms),
            step_sim_ms: SimDuration::ms(step_ms),
        }),
    };
    let mut asks: Vec<Ask> = catalogue()
        .iter()
        .map(|described| Ask { metric: find(&described.metric).expect("named"), trailing: None })
        .collect();
    asks.extend([
        windows(Metric::State(StateSeries::D30State), 60_000, 30_000),
        windows(Metric::State(StateSeries::D30Wait), 10_000, 10_000),
        windows(Metric::State(StateSeries::Da50State), 120_000, 60_000),
        windows(Metric::Series(TimeSeries::PicksPerSlice), 60_000, 30_000),
        windows(find("da50_operations").expect("named"), 600_000, 300_000),
    ]);
    asks
}

/// What each ask got, the frames taken one at a time with every view open from the scenario block on: its rows, or
/// why it was refused or failed.
fn live(frames: &[(Tag, Vec<u8>)], asks: &[Ask]) -> FxHashMap<Ask, Result<Vec<MetricPoint>, Unservable>> {
    let mut follower = Follower::from_scenario(&frames[0].1).expect("the scenario block reads");
    let mut got: FxHashMap<Ask, Result<Vec<MetricPoint>, Unservable>> =
        asks.iter().map(|ask| (*ask, follower.open(*ask).map(|opened| opened.rows))).collect();
    for (_, bytes) in &frames[1..] {
        for made in follower.frame(bytes).expect("every frame reads") {
            let held = got.get_mut(&made.ask).expect("an open view");
            match (held.as_mut(), made.rows) {
                (Ok(rows), Ok(more)) => rows.extend(more),
                (Ok(_), Err(fault)) => *held = Err(fault),
                (Err(_), _) => panic!("{:?} stated something after it failed", made.ask),
            }
        }
    }
    got
}

#[test]
fn every_row_stated_live_is_its_rebuild_from_the_finished_recording() {
    let path = recorded();
    let frames = frames(&path);
    let asks = every_ask();
    let sent = live(&frames, &asks);

    let mut rebuilt = Follower::from_scenario(&frames[0].1).expect("the scenario block reads");
    for (_, bytes) in &frames[1..] {
        rebuilt.frame(bytes).expect("every frame reads");
    }
    for ask in &asks {
        assert_eq!(sent[ask], rebuilt.open(*ask).map(|opened| opened.rows), "{ask:?}");
    }

    let channel = Channel::of(&path);
    let mut held = 0;
    for ask in asks.iter().filter(|ask| matches!(ask.metric, Metric::State(_))) {
        let Metric::State(series) = ask.metric else { continue };
        let rows = sent[ask].as_ref().expect("the recording carries the channel");
        assert!(!rows.is_empty(), "{ask:?} states its windows");
        for row in rows {
            assert_eq!((row.numerator, row.denominator), channel.share(series, row), "{ask:?}, {row:?}");
            held += 1;
        }
    }
    let class = |class| channel.classes.values().filter(|robot| **robot == class).count();
    let rows = |served: &Result<Vec<MetricPoint>, Unservable>| served.as_ref().map_or(0, Vec::len);
    let stated = asks.iter().filter(|ask| sent[*ask].as_ref().is_ok_and(|rows| !rows.is_empty())).count();
    let refused: Vec<&Ask> = asks.iter().filter(|ask| sent[*ask].is_err()).collect();
    println!(
        "{} D30s and {} DA50s over {} ms: {stated} of {} asks state rows, {} rows in all; {held} rows held to the channel; refused: {refused:?}",
        class(RobotClass::Htm),
        class(RobotClass::Vtm),
        channel.run_end,
        asks.len(),
        asks.iter().map(|ask| rows(&sent[ask])).sum::<usize>(),
    );
    assert!(held > 100, "{held} rows held to the channel");
    let _ = remove_file(&path);
}

/// The small floor stopped at its simtime limit 5.5 s into a slice, with work in it. That slice's rows end at the end
/// block's stamp, not the slice's nominal end, and a series of the slice's width reads its 5.5 s, as computed here
/// straight off the recording's scalars: the D30s traveling are their duty's milliseconds over 5,500, and the average
/// sim speed the stop over every slice's wall clock.
#[test]
fn the_last_slice_ends_at_the_runs_stop_and_its_rates_are_over_its_own_width() {
    let path = recorded_until(Some(Duration::from_millis(455_500)));
    let mut reader = Reader::open(&path).expect("the recording opens");
    let mut descriptors = Descriptors::from_scenario(&Scenario::read_from(&mut reader).expect("the scenario reads"));
    let records = reader.index().records().to_vec();
    for record in records.iter().filter(|record| record.tag == Tag::Declare) {
        descriptors.push(&reader.payload_at::<Declare>(record).expect("a declaration reads"));
    }
    let deltas: Vec<_> = records.iter().filter(|record| record.tag == Tag::Delta).collect();
    let mut scalars = |record| {
        let delta = Delta::decode(&reader.sections_at(record).expect("sections"), record.offset).expect("decodes");
        delta.scalars.expect("a slice states its scalars")
    };
    let wall: f64 = deltas.iter().map(|record| scalars(record)).map(|stated| scalar(&descriptors, &stated, WALL)).sum();
    let last = deltas.last().expect("a slice");
    let in_last = scalars(last);
    let stop = records.iter().find(|record| record.tag == Tag::End).expect("an end block").t.0;
    let width_ms = (stop - last.t.0) as f64;
    assert_eq!(width_ms, 5_500.0, "the run stops 5.5 s into its last slice");
    let duty = |key| scalar(&descriptors, &in_last, key) / width_ms;
    let expected =
        [(TimeSeries::HtmTraveling, duty("htm_duty_traveling_ms")), (TimeSeries::SimSpeedAverage, stop as f64 / wall)];
    assert!(expected[0].1 > 0.0, "D30s travel in the last slice");

    let frames = frames(&path);
    let mut follower = Follower::from_scenario(&frames[0].1).expect("the scenario block reads");
    for (_, bytes) in &frames[1..] {
        follower.frame(bytes).expect("every frame reads");
    }
    assert_eq!(follower.reached().as_ms(), stop, "the follower has read to the end block");
    for (series, value) in expected {
        let rows = follower.open(Ask { metric: Metric::Series(series), trailing: None }).expect("served").rows;
        let row = rows.last().expect("a row per slice");
        // The average sim speed's bucket spans the run from its start.
        let from = if series == TimeSeries::SimSpeedAverage { 0 } else { last.t.0 };
        assert_eq!((row.from_sim_ms.as_ms(), row.to_sim_ms.as_ms()), (from, stop), "{series:?}'s last row");
        let stated = row.value.expect("a value");
        assert!((stated - value).abs() <= 1e-9 * value.abs().max(1.0), "{series:?}: {stated}, not {value}");
    }
    let _ = remove_file(&path);
}

/// The scalar a slice states the wall clock it took to simulate in.
const WALL: &str = "wall_clock_duration_ms";

/// The scalar `key` a slice's `stated` scalars hold.
fn scalar(descriptors: &Descriptors, stated: &ScalarsDelta, key: &str) -> f64 {
    stated.values[descriptors.metric_slot(key).expect(key)].expect(key)
}

/// After a lost connection a runner sends the open segment again from its keyframe, the declarations and slices in it
/// among it: taken again, they change no row any view states.
#[test]
fn a_segment_sent_again_after_a_lost_connection_changes_no_row() {
    let path = recorded();
    let frames = frames(&path);
    let keyframes: Vec<usize> =
        frames.iter().enumerate().filter(|(_, (tag, _))| *tag == Tag::Keyframe).map(|(at, _)| at).collect();
    let declared_mid_run = frames.iter().skip(keyframes[0]).filter(|(tag, _)| *tag == Tag::Declare).count();
    assert!(declared_mid_run > 0, "the run declares metrics as it goes, so declarations come again too");
    let mut again = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        again.push(frame.clone());
        // Each segment, once whole, comes again from its keyframe.
        if let Some(next) = keyframes.iter().position(|keyframe| *keyframe == index + 1)
            && next > 0
        {
            again.extend(frames[keyframes[next - 1]..=index].iter().cloned());
        }
    }
    let asks = every_ask();
    let (once, twice) = (live(&frames, &asks), live(&again, &asks));
    for ask in &asks {
        assert_eq!(once[ask], twice[ask], "{ask:?}");
    }
    let _ = remove_file(&path);
}

/// Each robot's class and its states laid end to end from simtime 0, read straight off the channel, and the run's end.
struct Channel {
    classes: BTreeMap<RobotId, RobotClass>,
    spans: BTreeMap<RobotId, Vec<(u64, u64, RobotState)>>,
    run_end: u64,
}

impl Channel {
    fn of(path: &Path) -> Self {
        let mut reader = Reader::open(path).expect("the recording opens");
        let scenario = Scenario::read_from(&mut reader).expect("the scenario block reads");
        let classes = scenario.robots.iter().map(|robot| (robot.robot, robot.class)).collect();
        let width = scenario.slice_width.as_ms();
        let mut states: BTreeMap<RobotId, RobotState> = BTreeMap::new();
        let mut spans: BTreeMap<RobotId, Vec<(u64, u64, RobotState)>> = BTreeMap::new();
        let mut run_end = 0;
        for record in reader.index().records().to_vec() {
            let at = instant(record.t).as_ms();
            match record.tag {
                Tag::Keyframe if states.is_empty() => {
                    let sections = reader.sections_at(&record).expect("a keyframe's sections read");
                    let keyframe = Keyframe::decode(&sections, record.offset).expect("a keyframe decodes");
                    states.extend(keyframe.robot_state.expect("a keyframe states every robot").0);
                }
                Tag::Delta => {
                    let sections = reader.sections_at(&record).expect("a delta's sections read");
                    let delta = Delta::decode(&sections, record.offset).expect("a delta decodes");
                    let mut named: BTreeMap<RobotId, Vec<(u64, RobotState)>> = BTreeMap::new();
                    for entry in delta.robot_state.expect("a delta states the channel").0 {
                        named.insert(
                            entry.robot,
                            entry.spans.iter().map(|span| (span.dt.as_ms(), span.state)).collect(),
                        );
                    }
                    for (robot, state) in &mut states {
                        let mut from = at;
                        for (dt, stated) in named.remove(robot).unwrap_or_else(|| vec![(width, *state)]) {
                            spans.entry(*robot).or_default().push((from, from + dt, stated));
                            (from, *state) = (from + dt, stated);
                        }
                    }
                }
                Tag::End => run_end = at,
                _ => {}
            }
        }
        Self { classes, spans, run_end }
    }

    /// A row's numerator and denominator as the channel states them: the time the series' robots spent in the row's
    /// category within its span, up to the run's end, over their whole time or their waiting time; none for an
    /// unsupported category.
    fn share(&self, series: StateSeries, row: &MetricPoint) -> (Option<f64>, Option<f64>) {
        let key = row.keys[0].as_str();
        match series {
            StateSeries::D30State => self.time::<D30StateDefinition>(row, key, D30State::label, false),
            StateSeries::D30Wait => self.time::<D30WaitDefinition>(row, key, D30WaitCategory::label, true),
            StateSeries::Da50State => self.time::<Da50StateDefinition>(row, key, Da50State::label, false),
            StateSeries::Da50Wait => self.time::<Da50WaitDefinition>(row, key, Da50WaitReason::label, true),
        }
    }

    fn time<D: SpanDefinition>(
        &self,
        row: &MetricPoint,
        key: &str,
        label: fn(D::Category) -> &'static str,
        of_waiting: bool,
    ) -> (Option<f64>, Option<f64>) {
        if D::UNSUPPORTED.iter().any(|category| label(*category) == key) {
            return (None, None);
        }
        let (from, to) = (row.from_sim_ms.as_ms(), row.to_sim_ms.as_ms().min(self.run_end));
        let (mut part, mut whole) = (0, 0);
        for (robot, spans) in &self.spans {
            if self.classes[robot] != D::ROBOTS {
                continue;
            }
            for (start, end, state) in spans {
                let overlap = end.min(&to).saturating_sub(*start.max(&from));
                let category = D::category(*state);
                if category.is_some_and(|category| label(category) == key) {
                    part += overlap;
                }
                if !of_waiting || category.is_some() {
                    whole += overlap;
                }
            }
        }
        (Some(part as f64), Some(whole as f64))
    }
}
