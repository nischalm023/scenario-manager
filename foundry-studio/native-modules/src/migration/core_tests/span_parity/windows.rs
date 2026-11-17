//! Within `parity`, `windows` holds the span engine's D30 and DA50 definitions, read from a recording's channel window
//! by window, to today's report builders over the same run's interval rows, but for the differences the stretch walk
//! allows, and to themselves read from a later keyframe.

use std::collections::BTreeMap;
use std::path::Path;

use desunits::{RobotId, SimDuration, SimTime};
use worldview::container::frame::Tag;
use worldview::frames::playback::RobotClass;
use worldview::frames::{Delta, Keyframe, instant};
use worldview::{Reader, Scenario};

use crate::metrics::profile::SummaryStats;
use crate::metrics::reporting::{
    D30StateDefinition, D30WaitDefinition, Da50StateDefinition, Da50WaitDefinition, FixedPeriod, MetricScope,
    ReportPeriod, ReportRange, ReportRequest, SpanDefinition, SpanEvent, SpanReader, SpanWindow, SpanWindows,
    SupportStatus, TimeBucket, d30_state_report, da50_state_report,
};

use super::walk::{Category, Walk};

/// What one reader of a recording's channel told, slice by slice.
pub struct Told {
    /// The instant of the keyframe the reader was seeded at.
    seed: SimTime,
    /// Each slice's start with what the reader told as it took the slice, then the run's stop with what it told there.
    taken: Vec<(SimTime, Vec<SpanEvent>)>,
    roster: Vec<(RobotId, RobotClass)>,
    slice_ms: u64,
}

/// Read the recording at `path` with a reader seeded at its keyframe at `seed`.
pub fn told(path: &Path, seed: SimTime) -> Told {
    let mut reader = Reader::open(path).expect("the recording opens");
    let scenario = Scenario::read_from(&mut reader).expect("the scenario block reads");
    let roster: Vec<(RobotId, RobotClass)> = scenario.robots.iter().map(|robot| (robot.robot, robot.class)).collect();
    let mut spans: Option<SpanReader> = None;
    let mut taken = Vec::new();
    for record in reader.index().records().to_vec() {
        let at = instant(record.t);
        if matches!(record.tag, Tag::Keyframe) && at == seed && spans.is_none() {
            let sections = reader.sections_at(&record).expect("a keyframe's sections read");
            let keyframe = Keyframe::decode(&sections, record.offset).expect("a keyframe decodes");
            let states = keyframe.robot_state.expect("every keyframe states every robot's state");
            let seeded = SpanReader::seeded(roster.iter().copied(), scenario.slice_width, at, &states);
            spans = Some(seeded.expect("a reader seeds from any keyframe"));
            continue;
        }
        let Some(spans) = spans.as_mut() else { continue };
        match record.tag {
            Tag::Delta => {
                let sections = reader.sections_at(&record).expect("a delta's sections read");
                let delta = Delta::decode(&sections, record.offset).expect("a delta decodes");
                let told = spans.slice(at, delta.robot_state.expect("every delta states the channel"));
                taken.push((at, told.expect("the reader takes every slice")));
            }
            Tag::End => taken.push((at, spans.end(at).expect("the run's stop ends its last slice"))),
            _ => {}
        }
    }
    assert!(spans.is_some(), "the recording has a keyframe at {seed:?}");
    Told { seed, taken, roster, slice_ms: scenario.slice_width.as_ms() }
}

impl Told {
    /// One definition's windows `width_ms` wide, each taken in as the reader took its slice. A window goes out with the
    /// first slice beginning at or past its end, and no sooner; one ending past the run's stop, at the stop.
    fn windows<D: SpanDefinition>(&self, width_ms: u64) -> Vec<SpanWindow<D::Category>> {
        let mut windows = SpanWindows::<D>::new(SimDuration::ms(width_ms), self.seed).expect("a width");
        let mut out = Vec::new();
        let mut before: Option<u64> = None;
        for (index, (at, events)) in self.taken.iter().enumerate() {
            let at_run_end = index + 1 == self.taken.len();
            for window in windows.take(events).expect("the windows take what the reader told") {
                // Where the window ends on the grid, before the run's end clips the last one.
                let end = (window.bucket.start_ms / width_ms + 1) * width_ms;
                let in_turn = (end <= at.as_ms() || at_run_end) && before.is_none_or(|before| end > before);
                assert!(in_turn, "the window ending at {end} ms went out with what was told at {at:?}");
                out.push(window);
            }
            before = Some(at.as_ms());
        }
        out
    }

    fn fleet(&self, class: RobotClass) -> u64 {
        self.roster.iter().filter(|(_, robot_class)| *robot_class == class).count() as u64
    }

    fn run_end(&self) -> SimTime {
        self.taken.last().expect("the recording ends").0
    }
}

/// The engine's four definitions over the channel, as `told` from simtime 0, against today's D30 and DA50 report
/// builders over the interval rows of the run's `summary`, in fixed windows `width_ms` wide: the same windows and
/// fleet time, and in every window each category's time is the builder's plus what the walk's differences move into
/// it there, none in a category the definition calls unsupported.
pub fn hold_to_the_builders(told: &Told, summary: &SummaryStats, walk: &Walk, name: &str, width_ms: u64) {
    let run_end_ms = told.run_end().as_ms();
    assert_eq!(run_end_ms, walk.run_end_ms, "{name}: the end block's instant is the run's stop");
    let request = ReportRequest {
        scope: MetricScope::FullRun,
        range: ReportRange { start_ms: 0, end_ms: run_end_ms },
        period: ReportPeriod::Fixed(FixedPeriod::new(width_ms, 0).expect("a width")),
    };
    let (d30s, da50s) = (told.fleet(RobotClass::Htm), told.fleet(RobotClass::Vtm));
    let d30 = d30_state_report(&summary.htm_leg_log, d30s as u32, request).expect("the D30 report builds").data;
    let da50 = da50_state_report(&summary.vtm_leg_log, da50s as u32, request).expect("the DA50 report builds").data;
    let label = |definition: &str| format!("{name}, windows of {} s, {definition}", width_ms / 1_000);

    let mut misses = compare::<D30StateDefinition>(
        &label("D30 states"),
        told.windows::<D30StateDefinition>(width_ms),
        d30s,
        d30.state_rows.iter().map(|row| (row.bucket, row.state, measured(row.status, row.duration_ms))),
        walk.moved(width_ms, |category| match category {
            Category::D30(state, _) => Some(state),
            Category::Da50(..) => None,
        }),
    );
    misses.extend(compare::<D30WaitDefinition>(
        &label("D30 waits"),
        told.windows::<D30WaitDefinition>(width_ms),
        d30s,
        d30.wait_category_rows.iter().map(|row| (row.bucket, row.category, measured(row.status, row.duration_ms))),
        walk.moved(width_ms, |category| match category {
            Category::D30(_, wait) => wait,
            Category::Da50(..) => None,
        }),
    ));
    misses.extend(compare::<Da50StateDefinition>(
        &label("DA50 states"),
        told.windows::<Da50StateDefinition>(width_ms),
        da50s,
        da50.state_rows.iter().map(|row| (row.bucket, row.state, measured(row.status, row.duration_ms))),
        walk.moved(width_ms, |category| match category {
            Category::Da50(state, _) => Some(state),
            Category::D30(..) => None,
        }),
    ));
    misses.extend(compare::<Da50WaitDefinition>(
        &label("DA50 waits"),
        told.windows::<Da50WaitDefinition>(width_ms),
        da50s,
        da50.wait_rows.iter().map(|row| (row.bucket, row.reason, measured(row.status, row.duration_ms))),
        walk.moved(width_ms, |category| match category {
            Category::Da50(_, reason) => reason,
            Category::D30(..) => None,
        }),
    ));
    let shown = &misses[..misses.len().min(30)];
    assert!(misses.is_empty(), "{} windowed times differ by more than the walk allows: {shown:#?}", misses.len());
}

/// A builder row's time: a state the report marks unsupported reports none, and no row of its builder ever counts
/// time under it.
fn measured(status: SupportStatus, duration_ms: Option<u64>) -> u64 {
    duration_ms.unwrap_or_else(|| {
        assert_eq!(status, SupportStatus::Unsupported, "only an unsupported state reports no time");
        0
    })
}

/// Each window of one definition where the engine's time in a category is not the builder's plus what the walk
/// moves into it, or whose bounds or fleet time are not the builder's; and, printed, how many windows were held to
/// the builder's and how much time the walk moved between categories.
fn compare<D: SpanDefinition>(
    label: &str,
    windows: Vec<SpanWindow<D::Category>>,
    fleet: u64,
    rows: impl Iterator<Item = (TimeBucket, D::Category, u64)>,
    moved: BTreeMap<(u64, D::Category), i64>,
) -> Vec<String>
where
    D::Category: Ord,
{
    let mut built: BTreeMap<(u64, D::Category), u64> = BTreeMap::new();
    let mut buckets: Vec<TimeBucket> = Vec::new();
    for (bucket, category, ms) in rows {
        if buckets.last() != Some(&bucket) {
            buckets.push(bucket);
        }
        built.insert((bucket.start_ms, category), ms);
    }
    let bounds: Vec<TimeBucket> = windows.iter().map(|window| window.bucket).collect();
    if bounds != buckets {
        return vec![format!("{label}: the engine's {} windows are not the builder's {}", bounds.len(), buckets.len())];
    }
    let mut misses = Vec::new();
    for window in &windows {
        let start = window.bucket.start_ms;
        if window.fleet_time.as_ms() != fleet * window.bucket.coverage_ms {
            misses.push(format!("{label}, from {start} ms: fleet time {}", window.fleet_time));
        }
        for (category, time) in &window.times {
            let key = (start, *category);
            let (built, moved) = (built.get(&key).copied().unwrap_or(0), moved.get(&key).copied().unwrap_or(0));
            let engine = time.as_ms();
            if engine as i64 != built as i64 + moved {
                misses
                    .push(format!("{label}, from {start} ms, {category:?}: {engine} ms, built {built}, moved {moved}"));
            }
            if D::UNSUPPORTED.contains(category) && engine != 0 {
                misses.push(format!(
                    "{label}, from {start} ms: {engine} ms in {category:?}, a category the definition calls unsupported"
                ));
            }
        }
    }
    let into: i64 = moved.values().filter(|ms| **ms > 0).sum();
    let out_of: i64 = moved.values().filter(|ms| **ms < 0).sum();
    let (into, out_of) = (into as f64 / 1_000.0, -out_of as f64 / 1_000.0);
    println!("{label}: {} windows; the walk moves {into:.1} s into a category and {out_of:.1} s out", windows.len());
    misses
}

/// A reader seeded at each keyframe after the first gives, for every window from there, the time the reader
/// `from_zero` gives: how many keyframes after the first the run has, each one checked.
pub fn hold_seeded_readers(path: &Path, from_zero: &Told) -> usize {
    let reader = Reader::open(path).expect("the recording opens");
    let keyframes: Vec<SimTime> = reader
        .index()
        .records()
        .iter()
        .filter(|record| matches!(record.tag, Tag::Keyframe))
        .map(|record| instant(record.t))
        .filter(|at| *at > SimTime::ZERO)
        .collect();
    for &seed in &keyframes {
        let seeded = told(path, seed);
        agrees_from::<D30StateDefinition>(from_zero, &seeded);
        agrees_from::<D30WaitDefinition>(from_zero, &seeded);
        agrees_from::<Da50StateDefinition>(from_zero, &seeded);
        agrees_from::<Da50WaitDefinition>(from_zero, &seeded);
    }
    keyframes.len()
}

fn agrees_from<D: SpanDefinition>(from_zero: &Told, seeded: &Told) {
    let seed = seeded.seed.as_ms();
    let width = seeded.slice_ms;
    let whole: Vec<_> =
        from_zero.windows::<D>(width).into_iter().filter(|window| window.bucket.start_ms >= seed).collect();
    let part = seeded.windows::<D>(width);
    assert_eq!(whole.len(), part.len(), "from {seed} ms, both readers' windows reach the run's stop");
    for (whole, part) in whole.iter().zip(&part) {
        assert_eq!(whole, part, "seeded at {seed} ms, from {} ms", part.bucket.start_ms);
    }
}
