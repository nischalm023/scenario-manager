//! Within `spans`, `windows` is ONE DEFINITION'S TIME WINDOW BY WINDOW: what a reader tells, apportioned to adjacent
//! windows of one width on a grid from simtime 0. A window goes out once the reader has told through its end, which
//! it does once it has taken the slice past that end, and the last at the run's stop, clipped there. Nothing told
//! later changes a window gone out.

use std::collections::VecDeque;
use std::fmt::Debug;
use std::marker::PhantomData;

use crate::metrics::reporting::TimeBucket;
use crate::model::{SimDuration, SimTime};

use super::{SpanDefinition, SpanError, SpanEvent};

/// One definition's time over one window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanWindow<C> {
    pub bucket: TimeBucket,
    /// The time every robot the definition measures spent in the window, whatever its state: what a share of the
    /// fleet's time divides by.
    pub fleet_time: SimDuration,
    /// Every category the definition lists, in its order, with the time spent in it within the window.
    pub times: Vec<(C, SimDuration)>,
}

/// One definition's windows of one width, from the instant its reader was seeded at.
#[derive(Debug)]
pub struct SpanWindows<D: SpanDefinition> {
    width: SimDuration,
    from: SimTime,
    /// The windows not yet out, oldest first, the oldest being the `first`th window on the grid.
    open: VecDeque<SpanWindow<D::Category>>,
    first: u64,
    run_end: Option<SimTime>,
    definition: PhantomData<D>,
}

impl<D: SpanDefinition> SpanWindows<D> {
    /// Windows `width` wide on a grid from simtime 0, the first beginning at `from`, where the reader was seeded.
    pub fn new(width: SimDuration, from: SimTime) -> Result<Self, SpanError> {
        if width == SimDuration::ZERO {
            return Err(SpanError::ZeroWidth);
        }
        let first = from.as_ms() / width.as_ms();
        Ok(Self { width, from, open: VecDeque::new(), first, run_end: None, definition: PhantomData })
    }

    /// Take what a reader told, in the order it told it; out come the windows it made final, in order.
    pub fn take(&mut self, events: &[SpanEvent]) -> Result<Vec<SpanWindow<D::Category>>, SpanError> {
        let mut out = Vec::new();
        for event in events {
            if let Some(run_end) = self.run_end {
                return Err(SpanError::AfterRunEnd { run_end });
            }
            match *event {
                SpanEvent::Spent { class, state, from, to } if class == D::ROBOTS => {
                    let category = D::category(state);
                    let mut at = from;
                    while at < to {
                        let window = self.window(at)?;
                        let until = to.min(SimTime::ms(window.bucket.end_ms));
                        window.fleet_time += until - at;
                        if let Some(category) = category {
                            *time_in(window, category)? += until - at;
                        }
                        at = until;
                    }
                }
                SpanEvent::Spent { .. } => {}
                SpanEvent::Through(at) => {
                    while self.grid_start(self.first + 1) <= at {
                        out.push(self.next_out());
                    }
                }
                SpanEvent::RunEnd(at) => {
                    while self.grid_start(self.first).max(self.from) < at {
                        let mut window = self.next_out();
                        let bucket = &mut window.bucket;
                        if bucket.end_ms > at.as_ms() {
                            bucket.end_ms = at.as_ms();
                            bucket.coverage_ms = bucket.end_ms - bucket.start_ms;
                            bucket.partial = true;
                        }
                        out.push(window);
                    }
                    self.open.clear();
                    self.run_end = Some(at);
                }
            }
        }
        Ok(out)
    }

    /// The open window holding `at`, opening it and any before it that nothing was told in yet.
    fn window(&mut self, at: SimTime) -> Result<&mut SpanWindow<D::Category>, SpanError> {
        let index = at.as_ms() / self.width.as_ms();
        if index < self.first || at < self.from {
            return Err(SpanError::OutsideOpenWindows { at });
        }
        while self.first + self.open.len() as u64 <= index {
            let window = self.opened(self.first + self.open.len() as u64);
            self.open.push_back(window);
        }
        Ok(&mut self.open[(index - self.first) as usize])
    }

    /// The oldest open window, out of the open ones, whether or not anything was told in it.
    fn next_out(&mut self) -> SpanWindow<D::Category> {
        let window = self.open.pop_front().unwrap_or_else(|| self.opened(self.first));
        self.first += 1;
        window
    }

    fn grid_start(&self, index: u64) -> SimTime {
        SimTime::ZERO + self.width * index
    }

    /// The `index`th window on the grid, with nothing told in it: clipped to begin where the reader was seeded.
    fn opened(&self, index: u64) -> SpanWindow<D::Category> {
        let grid_start = self.grid_start(index);
        let (start, end) = (grid_start.max(self.from), grid_start + self.width);
        let (start_ms, end_ms) = (start.as_ms(), end.as_ms());
        SpanWindow {
            bucket: TimeBucket { start_ms, end_ms, coverage_ms: end_ms - start_ms, partial: start != grid_start },
            fleet_time: SimDuration::ZERO,
            times: D::CATEGORIES.iter().map(|category| (*category, SimDuration::ZERO)).collect(),
        }
    }
}

fn time_in<C: Copy + Eq + Debug>(window: &mut SpanWindow<C>, category: C) -> Result<&mut SimDuration, SpanError> {
    let listed = window.times.iter_mut().find(|(listed, _)| *listed == category);
    listed.map(|(_, time)| time).ok_or_else(|| SpanError::UnlistedCategory { category: format!("{category:?}") })
}

#[cfg(test)]
mod tests {
    use worldview::frames::playback::RobotClass;
    use worldview::frames::playback::RobotClass::{Htm, Vtm};
    use worldview::frames::robot_state::{CellRole, RobotState};

    use super::*;
    use crate::metrics::reporting::{D30State, D30StateDefinition, D30WaitCategory, D30WaitDefinition};

    const TEN: SimDuration = SimDuration::seconds(10);
    const PARKED: RobotState = RobotState::NoTask { cell: Some(CellRole::Relay) };

    fn at(seconds: u64) -> SimTime {
        SimTime::seconds(seconds)
    }

    fn spent(class: RobotClass, state: RobotState, from: u64, to: u64) -> SpanEvent {
        SpanEvent::Spent { class, state, from: at(from), to: at(to) }
    }

    fn time<C: Copy + Eq + Debug>(window: &SpanWindow<C>, category: C) -> SimDuration {
        window.times.iter().find(|(listed, _)| *listed == category).expect("every category listed").1
    }

    /// Laden travel from 5 s to 25 s falls 5 s, 10 s and 5 s in windows of 10 s. The first two go out once the
    /// reader has told through their ends, and the last at the run's end, clipped there.
    #[test]
    fn time_falls_in_the_windows_it_crosses_and_a_window_goes_out_once_told_through_its_end() {
        let mut windows = SpanWindows::<D30StateDefinition>::new(TEN, at(0)).expect("a width");
        let told = [spent(Htm, PARKED, 0, 5), spent(Htm, RobotState::LadenTravel, 5, 25), SpanEvent::Through(at(10))];
        let first = windows.take(&told).expect("taken");
        assert_eq!(first.len(), 1, "only the window ending at 10 s is told through");
        assert_eq!((first[0].bucket, first[0].fleet_time), (TimeBucket::complete(0, 10_000), SimDuration::seconds(10)));
        assert_eq!(time(&first[0], D30State::Waiting), SimDuration::seconds(5));
        assert_eq!(time(&first[0], D30State::LadenTravel), SimDuration::seconds(5));

        let second = windows.take(&[SpanEvent::Through(at(20))]).expect("taken");
        assert_eq!(time(&second[0], D30State::LadenTravel), SimDuration::seconds(10));
        assert!(windows.take(&[SpanEvent::Through(at(25))]).expect("taken").is_empty(), "20 s to 30 s is open");
        let last = windows.take(&[SpanEvent::RunEnd(at(25))]).expect("taken");
        let clipped = TimeBucket { start_ms: 20_000, end_ms: 25_000, coverage_ms: 5_000, partial: true };
        assert_eq!((last.len(), last[0].bucket, last[0].fleet_time), (1, clipped, SimDuration::seconds(5)));
        assert_eq!(time(&last[0], D30State::LadenTravel), SimDuration::seconds(5));
    }

    /// A DA50's time is no D30's, and a D30's travel is fleet time a wait definition counts under no category.
    #[test]
    fn a_definition_counts_only_its_robots_and_the_states_it_measures() {
        let mut windows = SpanWindows::<D30WaitDefinition>::new(TEN, at(0)).expect("a width");
        let told = [
            spent(Vtm, PARKED, 0, 10),
            spent(Htm, RobotState::LadenTravel, 0, 4),
            spent(Htm, PARKED, 4, 10),
            SpanEvent::Through(at(10)),
        ];
        let window = windows.take(&told).expect("taken").remove(0);
        assert_eq!(window.fleet_time, SimDuration::seconds(10), "the one D30's time, travel included");
        assert_eq!(time(&window, D30WaitCategory::NoTaskParked), SimDuration::seconds(6));
        let waiting = window.times.iter().map(|(_, time)| time.as_ms()).sum::<u64>();
        assert_eq!(waiting, 6_000, "waiting only");
    }

    /// Windows from a reader seeded at 15 s begin there. A window nothing was told in still goes out in its turn, and
    /// time told outside the windows still open is refused.
    #[test]
    fn a_window_nothing_was_told_in_goes_out_in_turn_and_time_outside_the_open_ones_is_refused() {
        let mut windows = SpanWindows::<D30StateDefinition>::new(TEN, at(15)).expect("a width");
        let out = windows.take(&[SpanEvent::Through(at(40))]).expect("taken");
        let buckets: Vec<TimeBucket> = out.iter().map(|window| window.bucket).collect();
        let seeded = TimeBucket { start_ms: 15_000, end_ms: 20_000, coverage_ms: 5_000, partial: true };
        assert_eq!(buckets, [seeded, TimeBucket::complete(20_000, 30_000), TimeBucket::complete(30_000, 40_000)]);
        assert!(out.iter().all(|window| window.fleet_time == SimDuration::ZERO));
        let late = windows.take(&[spent(Htm, PARKED, 35, 45)]).expect_err("35 s lies in a window gone out");
        assert!(matches!(late, SpanError::OutsideOpenWindows { at } if at == SimTime::seconds(35)), "{late}");
        assert!(SpanWindows::<D30StateDefinition>::new(SimDuration::ZERO, at(0)).is_err(), "windows of no width");
    }
}
