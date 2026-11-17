//! Within `parity`, `walk` is THE STRETCH WALK: each robot's time cut wherever its interval rows or its channel change,
//! every stretch named by the report category the rows give it and the one the channel gives it.
//!
//! A stretch takes one category from the rows, as the D30 and DA50 reports partition them (`d30_stretches`,
//! `da50_stretches`), and one from the channel, by the span engine's definitions. Where the two differ, the stretch is
//! named by the pair and by what the report reads it from, and only these pairs may differ:
//! - the waits the rows explain after the fact. A resident D30's stand before its pickup, a DA50's wait before its
//!   cycle, and a D30's stand at a station after the operator's work until its next move is committed are explained
//!   only once B has passed them, so the channel states them as no task.
//! - a stand before a move whose nav was read before it departed, which the rows file as no task: the channel states
//!   it as awaiting that departure from the instant the nav was read.
//! - the honesty residuals: time the rows leave unattributed, unfacted or in conflict, which the channel states; and
//!   station processing the report infers between two rows at one station, where the channel states the stand
//!   before the operator's work.

use std::collections::BTreeMap;
use std::path::Path;

use desunits::RobotId;
use rustc_hash::FxHashMap;
use worldview::container::frame::Tag;
use worldview::frames::playback::RobotClass;
use worldview::frames::robot_state::RobotState;
use worldview::frames::{Delta, Keyframe, instant};
use worldview::{Reader, Scenario};

use crate::metrics::profile::PerfProfile;
use crate::metrics::reporting::{
    D30State, D30StateDefinition, D30WaitCategory, D30WaitDefinition, Da50State, Da50StateDefinition,
    Da50WaitDefinition, Da50WaitReason, SpanDefinition, d30_stretches, da50_stretches,
};
use crate::observer::output::{HtmLegRecord, HtmWaitCause, VtmLegRecord};

/// A report's category of a stretch of one robot's time: the state and, in Waiting, the D30's ratified category or
/// the DA50's reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    D30(D30State, Option<D30WaitCategory>),
    Da50(Da50State, Option<Da50WaitReason>),
}

impl Category {
    /// A wait the rows explain once B has passed it, by the commit of the task after it, which the channel states as
    /// no task until that commit.
    fn explained_after_the_fact(self) -> bool {
        matches!(
            self,
            Self::D30(
                _,
                Some(D30WaitCategory::AwaitingRelayDepositResident | D30WaitCategory::AwaitingStationDeparture)
            ) | Self::Da50(
                _,
                Some(
                    Da50WaitReason::AwaitingDemand
                        | Da50WaitReason::AwaitingTote
                        | Da50WaitReason::AwaitingSlot
                        | Da50WaitReason::AwaitingD30Deposit
                )
            )
        )
    }

    /// Time the report leaves to an honesty residual: facts that leave it unattributed, never reach it, or
    /// contradict each other over it.
    fn residual(self) -> bool {
        matches!(
            self,
            Self::D30(D30State::Unknown, _)
                | Self::D30(
                    _,
                    Some(D30WaitCategory::Unknown | D30WaitCategory::NoFacts | D30WaitCategory::ConflictingFacts)
                )
                | Self::Da50(Da50State::Unknown, _)
                | Self::Da50(_, Some(Da50WaitReason::Unknown | Da50WaitReason::ConflictingFacts))
        )
    }

    fn no_task(self) -> bool {
        matches!(
            self,
            Self::D30(_, Some(D30WaitCategory::NoTaskParked | D30WaitCategory::NoTaskElsewhere))
                | Self::Da50(_, Some(Da50WaitReason::NoTask))
        )
    }

    /// A D30 awaiting the departure of a move committed before it departs: on a station's presentation cell after
    /// the operator's work, or anywhere else.
    fn awaiting_departure(self) -> bool {
        matches!(
            self,
            Self::D30(_, Some(D30WaitCategory::AwaitingStationDeparture | D30WaitCategory::AwaitingDepartureElsewhere))
        )
    }
}

/// The report category a D30's state in the channel counts under, by the span engine's D30 definitions.
fn d30_category(state: RobotState) -> Category {
    let counted = D30StateDefinition::category(state).expect("the D30 state definition counts every state");
    Category::D30(counted, D30WaitDefinition::category(state))
}

/// The report category a DA50's state in the channel counts under, by the span engine's DA50 definitions.
fn da50_category(state: RobotState) -> Category {
    let counted = Da50StateDefinition::category(state).expect("the DA50 state definition counts every state");
    Category::Da50(counted, Da50WaitDefinition::category(state))
}

/// One robot's states as the channel lays them end to end: each span's start, end and state.
type Timeline = Vec<(u64, u64, RobotState)>;

/// A delta's spans of each robot it names, as lengths in ms and states, not yet laid against the slice's end.
type NamedSpans = FxHashMap<RobotId, Vec<(u64, RobotState)>>;

/// One frame of the channel as `lay` takes it: every robot's state a keyframe states, the spans a delta names, or
/// the run's stop.
enum Frame {
    Keyframe(Vec<(RobotId, RobotState)>),
    Delta(NamedSpans),
    End,
}

/// Each robot's class and its states as the channel lays them end to end from simtime 0, slice after slice, and the
/// width of a slice.
fn channel_of(path: &Path) -> (BTreeMap<RobotId, RobotClass>, BTreeMap<RobotId, Timeline>, u64) {
    let mut reader = Reader::open(path).expect("the recording opens");
    let scenario = Scenario::read_from(&mut reader).expect("the scenario block reads");
    let classes: BTreeMap<RobotId, RobotClass> =
        scenario.robots.iter().map(|robot| (robot.robot, robot.class)).collect();
    let records = reader.index().records().to_vec();
    let frames = records.iter().filter_map(|record| {
        let frame = match record.tag {
            Tag::Keyframe => {
                let sections = reader.sections_at(record).expect("a keyframe's sections read");
                let keyframe = Keyframe::decode(&sections, record.offset).expect("a keyframe decodes");
                Frame::Keyframe(keyframe.robot_state.expect("every keyframe states every robot's state").0)
            }
            Tag::Delta => {
                let sections = reader.sections_at(record).expect("a delta's sections read");
                let delta = Delta::decode(&sections, record.offset).expect("a delta decodes");
                let named = delta.robot_state.expect("every delta states the channel");
                let spans = named
                    .0
                    .into_iter()
                    .map(|entry| (entry.robot, entry.spans.iter().map(|span| (span.dt.as_ms(), span.state)).collect()));
                Frame::Delta(spans.collect())
            }
            Tag::End => Frame::End,
            _ => return None,
        };
        Some((instant(record.t).as_ms(), frame))
    });
    let timelines = lay(frames);
    (classes, timelines, scenario.slice_width.as_ms())
}

/// Lay `frames`, each at its instant in ms, end to end into each robot's timeline. A slice ends where the next frame
/// begins: a keyframe, the next slice, or the end block at the run's stop. A keyframe states the span beginning at
/// its instant, so the slice ending there is laid before the keyframe's states are held: a robot that slice leaves
/// unnamed stays in the state it held, not the one the keyframe states.
fn lay(frames: impl IntoIterator<Item = (u64, Frame)>) -> BTreeMap<RobotId, Timeline> {
    let mut held: BTreeMap<RobotId, RobotState> = BTreeMap::new();
    let mut timelines: BTreeMap<RobotId, Timeline> = BTreeMap::new();
    let mut pending: Option<(u64, NamedSpans)> = None;
    for (at, frame) in frames {
        if let Some((begins, mut spans)) = pending.take() {
            for (&robot, state) in held.iter_mut() {
                let timeline = timelines.entry(robot).or_default();
                let mut from = begins;
                for (dt, span_state) in spans.remove(&robot).unwrap_or_else(|| vec![(at - begins, *state)]) {
                    timeline.push((from, from + dt, span_state));
                    from += dt;
                    *state = span_state;
                }
                assert_eq!(
                    from, at,
                    "robot {robot:?}'s spans lie across the slice at {begins} ms, up to the next frame"
                );
            }
            assert!(spans.is_empty(), "a delta names only robots a keyframe stated");
        }
        match frame {
            Frame::Keyframe(stated) => held.extend(stated),
            Frame::Delta(named) => pending = Some((at, named)),
            Frame::End => {}
        }
    }
    timelines
}

/// One stretch of a robot's time as the rows have it: the report's category, and what the report reads it from.
type Stretch = (u64, u64, Category, Basis);

/// What the report reads a stretch's category from: a row that covers it, or an inference for time no row covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Basis {
    Row,
    /// No row covers it, and the row before it is a station visit's wait for service: the operator's work, which the
    /// rows leave for the report to infer.
    VisitInference,
    Inference,
}

/// What the report reads a stretch where the two sides differ from, the category the rows give it, the channel's.
type Difference = (Basis, Category, Category);

/// Every stretch of the run's time where the rows and the channel name different categories, by the pair and by
/// whether a row states it, and every stretch's categories on both sides, by slice.
#[derive(Default)]
pub struct Walk {
    pub run_end_ms: u64,
    differences: BTreeMap<Difference, u64>,
    /// The first few stretches of each difference, robot and span, for a reader to look at.
    examples: BTreeMap<Difference, Vec<(RobotId, u64, u64)>>,
    /// `(slice start, old category, new category)` → ms, every stretch, the agreeing ones included.
    slices: BTreeMap<(u64, Category, Category), u64>,
    pub slice_ms: u64,
    fleets: BTreeMap<&'static str, u64>,
}

/// Give each stretch the report's basis: a row when one of `rows` (start, end, whether it is a visit's wait for
/// service) covers it. A stretch ends at every row's edge, so a row covers all of it or none.
fn with_basis(stretches: impl Iterator<Item = (u64, u64, Category)>, mut rows: Vec<(u64, u64, bool)>) -> Vec<Stretch> {
    rows.retain(|(start, end, _)| end > start);
    rows.sort_unstable();
    let mut covered: Vec<(u64, u64)> = Vec::new();
    for &(start, end, _) in &rows {
        match covered.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => covered.push((start, end)),
        }
    }
    let awaiting_service: Vec<u64> = rows.iter().filter(|row| row.2).map(|row| row.1).collect();
    stretches
        .map(|(start, end, category)| {
            let before = covered.partition_point(|(from, _)| *from <= start);
            let basis = if before > 0 && covered[before - 1].1 > start {
                Basis::Row
            } else if awaiting_service.contains(&start) {
                Basis::VisitInference
            } else {
                Basis::Inference
            };
            (start, end, category, basis)
        })
        .collect()
}

/// Walk the recording at `path`, whose run model is `run`: every robot's two partitions of the run laid side by side,
/// slice by slice.
pub fn walk(path: &Path, run: &PerfProfile) -> Walk {
    let run_end_ms = run.sim_result.sim_duration.as_millis() as u64;
    let summary = &run.summary;
    let (classes, timelines, slice_ms) = channel_of(path);
    let mut htm: FxHashMap<RobotId, Vec<HtmLegRecord>> = FxHashMap::default();
    for row in &summary.htm_leg_log {
        htm.entry(row.robot_id).or_default().push(row.clone());
    }
    let mut vtm: FxHashMap<u32, Vec<VtmLegRecord>> = FxHashMap::default();
    for row in &summary.vtm_leg_log {
        vtm.entry(row.vtm_id).or_default().push(row.clone());
    }

    let mut walk = Walk { run_end_ms, slice_ms, ..Walk::default() };
    for (robot, class) in classes {
        let (old, to_category): (Vec<Stretch>, fn(RobotState) -> Category) = match class {
            RobotClass::Htm => {
                let rows = htm.remove(&robot).unwrap_or_default();
                let stretches = match rows.is_empty() {
                    true => vec![(0, run_end_ms, D30State::Waiting, Some(D30WaitCategory::NoFacts))],
                    false => d30_stretches(&rows, 0, run_end_ms),
                };
                let awaiting = |row: &HtmLegRecord| {
                    row.hold.is_some_and(|hold| hold.cause == HtmWaitCause::AwaitingStationService)
                };
                let spans = rows
                    .iter()
                    .map(|row| (row.start.as_ms(), (row.start + row.duration).as_ms(), awaiting(row)))
                    .collect();
                let stretches =
                    stretches.into_iter().map(|(start, end, state, wait)| (start, end, Category::D30(state, wait)));
                (with_basis(stretches, spans), d30_category)
            }
            RobotClass::Vtm => {
                let rows = vtm.remove(&robot.0).unwrap_or_default();
                let spans = rows.iter().map(|row| (row.start_ms, row.start_ms + row.duration_ms, false)).collect();
                let stretches = da50_stretches(&rows, 0, run_end_ms);
                let stretches = stretches
                    .into_iter()
                    .map(|(start, end, state, reason)| (start, end, Category::Da50(state, reason)));
                (with_basis(stretches, spans), da50_category)
            }
            RobotClass::Rtp => continue,
        };
        *walk.fleets.entry(if class == RobotClass::Htm { "D30" } else { "DA50" }).or_default() += 1;
        let new: Vec<(u64, u64, Category)> = timelines[&robot]
            .iter()
            .filter(|(start, ..)| *start < run_end_ms)
            .map(|&(start, end, state)| (start, end.min(run_end_ms), to_category(state)))
            .collect();
        walk.take(robot, &old, &new);
    }
    walk
}

impl Walk {
    /// Walk one robot's two partitions of the run together, cutting at every edge of either and at every slice's.
    fn take(&mut self, robot: RobotId, old: &[Stretch], new: &[(u64, u64, Category)]) {
        let (mut i, mut j, mut at) = (0, 0, 0);
        while i < old.len() && j < new.len() {
            let slice = at / self.slice_ms * self.slice_ms;
            let end = old[i].1.min(new[j].1).min(slice + self.slice_ms);
            if end > at {
                let (was, is) = (old[i].2, new[j].2);
                *self.slices.entry((slice, was, is)).or_default() += end - at;
                if was != is {
                    let key = (old[i].3, was, is);
                    *self.differences.entry(key).or_default() += end - at;
                    let examples = self.examples.entry(key).or_default();
                    if examples.len() < 3 {
                        examples.push((robot, at, end));
                    }
                }
                at = end;
            }
            i += usize::from(old[i].1 <= at);
            j += usize::from(new[j].1 <= at);
        }
        assert_eq!(at, self.run_end_ms, "both sides cover the run");
    }

    /// Where a row states the time, every difference is a wait the rows explain after the fact, which the channel
    /// states as no task; a stand the rows file as no task, which the channel states as awaiting a departure already
    /// committed; or time the report leaves to a residual. The operator's work inside a visit, which no row covers,
    /// the channel states as the rows bound it. Elsewhere no row covers the time and the report infers it: a residual
    /// the channel states, or, between two rows at one station, station processing, where the channel states the
    /// stand before the operator's work as the wait for it, or as no task until that work was committed.
    pub fn check(&self, name: &str) {
        let explained = |basis: Basis, old: Category, new: Category| match basis {
            Basis::Row => {
                old.residual()
                    || old.explained_after_the_fact() && new.no_task()
                    || old.no_task() && new.awaiting_departure()
            }
            Basis::VisitInference => false,
            Basis::Inference => {
                let processing = old == Category::D30(D30State::StationProcessing, None);
                let presenting = new == Category::D30(D30State::Waiting, Some(D30WaitCategory::Presenting));
                old.residual() || processing && (presenting || new.no_task())
            }
        };
        let unexplained: Vec<_> =
            self.differences.iter().filter(|((basis, old, new), _)| !explained(*basis, *old, *new)).collect();
        assert!(unexplained.is_empty(), "{name}: differences nothing above explains: {unexplained:#?}");
    }

    /// The time the differences move into each category of one definition in each window `width_ms` wide: the time
    /// whose stretches the channel names by `category` and the rows by another, less the time the rows name by it
    /// and the channel by another. `project` reads a stretch's category in the definition's terms, `None` for one it
    /// does not count. Every difference is one `check` allows, so a definition's time from the channel is the time
    /// today's builder gives plus this.
    pub fn moved<C: Copy + Ord>(
        &self,
        width_ms: u64,
        project: impl Fn(Category) -> Option<C>,
    ) -> BTreeMap<(u64, C), i64> {
        let mut moved = BTreeMap::new();
        for (&(slice, old, new), &ms) in &self.slices {
            let (from, to) = (project(old), project(new));
            if from == to {
                continue;
            }
            let window = slice / width_ms * width_ms;
            if let Some(to) = to {
                *moved.entry((window, to)).or_default() += ms as i64;
            }
            if let Some(from) = from {
                *moved.entry((window, from)).or_default() -= ms as i64;
            }
        }
        moved
    }

    /// The differences, and each category's time on both sides by window `window_ms` wide.
    pub fn print(&self, name: &str, window_ms: u64) {
        println!("\n## {name}: {} s simulated; fleets {:?}", self.run_end_ms / 1_000, self.fleets);
        println!("\n### Differences: the report's basis, old category from the rows -> new from the channel (s)");
        for (key, ms) in &self.differences {
            let (basis, old, new) = key;
            let examples = &self.examples[key];
            println!("{basis:?}: {old:?} -> {new:?}: {:.1}, as {examples:?}", *ms as f64 / 1_000.0);
        }
        let mut totals: BTreeMap<(u64, Category), (u64, u64)> = BTreeMap::new();
        for (&(slice, old, new), &ms) in &self.slices {
            let window = slice / window_ms * window_ms;
            totals.entry((window, old)).or_default().0 += ms;
            totals.entry((window, new)).or_default().1 += ms;
        }
        println!("\n### Time by window and category: old s, new s");
        for ((window, category), (old, new)) in totals {
            let (old, new) = (old as f64 / 1_000.0, new as f64 / 1_000.0);
            println!("from {} min {category:?}: {old:.1} {new:.1}", window / 60_000);
        }
    }

    /// Whether the rows leave any time to a residual the channel states.
    pub fn leaves_a_residual(&self) -> bool {
        self.differences.keys().any(|(_, old, _)| old.residual())
    }
}

/// A robot a slice leaves unnamed stays in no task through it, though the keyframe where the slice ends states the
/// empty travel beginning there.
#[test]
fn a_slice_ending_at_a_keyframe_is_laid_in_the_states_held_before_it() {
    let robot = RobotId(1);
    let idle = RobotState::NoTask { cell: None };
    let travel: NamedSpans = [(robot, vec![(10_000, RobotState::EmptyTravel)])].into_iter().collect();
    let frames = [
        (0, Frame::Keyframe(vec![(robot, idle)])),
        (0, Frame::Delta(NamedSpans::default())),
        (10_000, Frame::Keyframe(vec![(robot, RobotState::EmptyTravel)])),
        (10_000, Frame::Delta(travel)),
        (20_000, Frame::End),
    ];
    let laid = lay(frames);
    assert_eq!(laid[&robot], vec![(0, 10_000, idle), (10_000, 20_000, RobotState::EmptyTravel)]);
}
