//! Within `series`, `trailing` is ONE SERIES' SLICES FOLDED INTO CAUSAL WINDOWS: `(t - window, t]` at every multiple
//! of the step from the first instant one whole window fits, a window's value the sum of its slices for a quantity the
//! slices contributed and their mean for a rate or a reading. Window and step are whole slices (`TrailingSpec::fits`),
//! so each window holds exactly the slices it says. It is the server's stand-in for desvis's smoothing, which reads
//! slices later than its own and so cannot stream; a window here reads only the slices taken.

use std::collections::VecDeque;

use desunits::{SimDuration, SimTime};
use studio_protocol::{MetricPoint, TrailingSpec};

pub(crate) struct Trailing {
    window: SimDuration,
    step: SimDuration,
    sums: bool,
    /// The slices within the window so far, oldest first, each its end and its value.
    held: VecDeque<(SimTime, Option<f64>)>,
}

impl Trailing {
    /// Windows of `spec`, which the slices fit, summed when `sums`, averaged otherwise.
    pub(crate) fn new(spec: TrailingSpec, sums: bool) -> Self {
        Self { window: spec.window_sim_ms, step: spec.step_sim_ms, sums, held: VecDeque::new() }
    }

    /// One slice in; the window ending at the slice's end out, when that end is a multiple of the step and a whole
    /// window has gone by. A window every slice of which stated nothing has no value.
    pub(crate) fn push(&mut self, point: &MetricPoint) -> Option<MetricPoint> {
        let to = point.to_sim_ms;
        self.held.push_back((to, point.value));
        let from = to.as_ms().saturating_sub(self.window.as_ms());
        while self.held.front().is_some_and(|(end, _)| end.as_ms() <= from) {
            self.held.pop_front();
        }
        if !to.as_ms().is_multiple_of(self.step.as_ms()) || to.as_ms() < self.window.as_ms() {
            return None;
        }
        let stated: Vec<f64> = self.held.iter().filter_map(|(_, value)| *value).collect();
        let value = match (stated.is_empty(), self.sums) {
            (true, _) => None,
            (false, true) => Some(stated.iter().sum()),
            (false, false) => Some(stated.iter().sum::<f64>() / stated.len() as f64),
        };
        Some(MetricPoint {
            from_sim_ms: SimTime::ms(from),
            to_sim_ms: to,
            keys: Vec::new(),
            value,
            numerator: None,
            denominator: None,
            whole_run: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(index: u64, value: Option<f64>) -> MetricPoint {
        MetricPoint {
            from_sim_ms: SimTime::ms(index * 10_000),
            to_sim_ms: SimTime::ms((index + 1) * 10_000),
            keys: Vec::new(),
            value,
            numerator: None,
            denominator: None,
            whole_run: false,
        }
    }

    /// A 30 s window at a 20 s step over 10 s slices: nothing until one window fits, then at 40 s and 60 s the mean
    /// (or sum) of the three slices ending in `(t - 30 s, t]`, a slice stating nothing left out.
    #[test]
    fn windows_come_at_the_step_once_one_fits_and_read_only_the_slices_within() {
        let spec = TrailingSpec { window_sim_ms: SimDuration::ms(30_000), step_sim_ms: SimDuration::ms(20_000) };
        let mut mean = Trailing::new(spec, false);
        let mut sum = Trailing::new(spec, true);
        let values = [Some(1.0), Some(2.0), Some(3.0), None, Some(5.0), Some(6.0)];
        let means: Vec<Option<MetricPoint>> =
            values.iter().enumerate().map(|(i, v)| mean.push(&slice(i as u64, *v))).collect();
        let sums: Vec<Option<MetricPoint>> =
            values.iter().enumerate().map(|(i, v)| sum.push(&slice(i as u64, *v))).collect();
        assert!(
            means[0].is_none() && means[1].is_none() && means[2].is_none(),
            "30 s has gone by at 30 s, but 30 s is not a multiple of 20 s"
        );
        let at_40 = means[3].as_ref().expect("a window at 40 s");
        assert_eq!((at_40.from_sim_ms, at_40.to_sim_ms), (SimTime::ms(10_000), SimTime::ms(40_000)));
        assert_eq!(at_40.value, Some(2.5), "the mean of 2 and 3, the fourth slice stating nothing");
        assert_eq!(sums[3].as_ref().and_then(|point| point.value), Some(5.0));
        assert!(means[4].is_none());
        assert_eq!(means[5].as_ref().and_then(|point| point.value), Some(5.5), "the mean of 5 and 6 over (30 s, 60 s]");
        assert_eq!(sums[5].as_ref().and_then(|point| point.value), Some(11.0));
    }

    #[test]
    fn a_window_of_slices_stating_nothing_has_no_value() {
        let spec = TrailingSpec { window_sim_ms: SimDuration::ms(10_000), step_sim_ms: SimDuration::ms(10_000) };
        let mut trailing = Trailing::new(spec, false);
        let point = trailing.push(&slice(0, None)).expect("a window per slice");
        assert_eq!(point.value, None);
        assert_eq!(trailing.push(&slice(1, Some(4.0))).and_then(|point| point.value), Some(4.0));
    }
}
