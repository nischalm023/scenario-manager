//! Within `reporting`, `trailing` owns CAUSAL WINDOW semantics: the
//! `(t − window, t]` schedules and lossless trailing sums/ratios emitted on
//! an independent grid. The one home of the trailing schedule.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Ratio, TimeBucket};

/// Causal trailing-window configuration.
///
/// `window_ms` and `step_ms` are intentionally independent. Output points
/// align to `alignment_ms + n * step_ms` and begin only once a complete
/// window fits inside the requested scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrailingSpec {
    window_ms: NonZeroU64,
    step_ms: NonZeroU64,
    alignment_ms: u64,
}

impl TrailingSpec {
    pub fn new(window_ms: u64, step_ms: u64, alignment_ms: u64) -> Result<Self, TrailingSpecError> {
        let window_ms = NonZeroU64::new(window_ms).ok_or(TrailingSpecError::ZeroWindow)?;
        let step_ms = NonZeroU64::new(step_ms).ok_or(TrailingSpecError::ZeroStep)?;
        Ok(Self { window_ms, step_ms, alignment_ms })
    }

    pub const fn window_ms(self) -> u64 {
        self.window_ms.get()
    }

    pub const fn step_ms(self) -> u64 {
        self.step_ms.get()
    }

    pub const fn alignment_ms(self) -> u64 {
        self.alignment_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TrailingSpecError {
    #[error("trailing window must be greater than zero")]
    ZeroWindow,
    #[error("trailing output step must be greater than zero")]
    ZeroStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TrailingInputError {
    #[error("scope end {end_ms} precedes scope start {start_ms}")]
    InvalidScope { start_ms: u64, end_ms: u64 },
    #[error("sample timestamps are not sorted at index {index}")]
    UnsortedSamples { index: usize },
    #[error("report arithmetic overflowed")]
    ArithmeticOverflow,
}

/// One discrete additive contribution at an exact simulation timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdditiveSample {
    pub at_ms: u64,
    pub amount: u64,
}

/// Numerator and denominator contributions captured at one timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatioSample {
    pub at_ms: u64,
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailingSumPoint {
    pub bucket: TimeBucket,
    pub sum: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailingRatioPoint {
    pub bucket: TimeBucket,
    pub ratio: Ratio,
}

/// One result from a coarse, slice-aligned trailing sum.
///
/// This is used when a profile exposes only already-bucketed floating-point
/// values. `end_index` identifies the inclusive source slice at the window
/// endpoint so a renderer can recover the exact source timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailingSlicePoint {
    pub end_index: usize,
    pub sum: f64,
}

/// Causal trailing sum for coarse per-slice values.
///
/// The first result is emitted after `window_slices` complete source slices;
/// subsequent results are `step_slices` apart. This preserves the same
/// no-future-data and complete-leading-window rules as [`trailing_sum`] while
/// making the source precision explicit.
pub fn trailing_slice_sum(values: &[f64], window_slices: usize, step_slices: usize) -> Option<Vec<TrailingSlicePoint>> {
    if window_slices == 0 || step_slices == 0 || values.len() < window_slices {
        return None;
    }

    let mut prefix = Vec::with_capacity(values.len() + 1);
    prefix.push(0.0);
    for value in values {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + value);
    }

    Some(
        ((window_slices - 1)..values.len())
            .step_by(step_slices)
            .map(|end_index| {
                let end = end_index + 1;
                let start = end - window_slices;
                TrailingSlicePoint { end_index, sum: prefix[end] - prefix[start] }
            })
            .collect(),
    )
}

/// Sum exact event contributions in `(t - window, t]`.
///
/// No point is emitted until a complete window fits in
/// `[scope_start_ms, scope_end_ms]`.
pub fn trailing_sum(
    samples: &[AdditiveSample],
    scope_start_ms: u64,
    scope_end_ms: u64,
    spec: TrailingSpec,
) -> Result<Vec<TrailingSumPoint>, TrailingInputError> {
    validate_input(samples.iter().map(|sample| sample.at_ms), scope_start_ms, scope_end_ms)?;

    let endpoints = endpoints(scope_start_ms, scope_end_ms, spec)?;
    let mut points = Vec::with_capacity(endpoints.len());
    let mut lower = 0;
    let mut upper = 0;
    let mut sum = 0u64;

    for end_ms in endpoints {
        while upper < samples.len() && samples[upper].at_ms <= end_ms {
            sum = sum.checked_add(samples[upper].amount).ok_or(TrailingInputError::ArithmeticOverflow)?;
            upper += 1;
        }

        let start_ms = end_ms - spec.window_ms();
        while lower < upper && samples[lower].at_ms <= start_ms {
            sum = sum.checked_sub(samples[lower].amount).ok_or(TrailingInputError::ArithmeticOverflow)?;
            lower += 1;
        }

        points.push(TrailingSumPoint { bucket: TimeBucket::complete(start_ms, end_ms), sum });
    }

    Ok(points)
}

/// The trailing window schedule for a scope, as report buckets: each window
/// is `(end - window, end]` on the spec's emission grid. This is the one
/// home of the schedule — consumers must never reconstruct it by running an
/// aggregation over empty samples.
pub fn trailing_buckets(
    scope_start_ms: u64,
    scope_end_ms: u64,
    spec: TrailingSpec,
) -> Result<Vec<TimeBucket>, TrailingInputError> {
    validate_input(std::iter::empty(), scope_start_ms, scope_end_ms)?;
    Ok(endpoints(scope_start_ms, scope_end_ms, spec)?
        .into_iter()
        .map(|end_ms| TimeBucket::complete(end_ms - spec.window_ms(), end_ms))
        .collect())
}

/// Aggregate a trailing ratio by summing its raw components before division.
pub fn trailing_ratio(
    samples: &[RatioSample],
    scope_start_ms: u64,
    scope_end_ms: u64,
    spec: TrailingSpec,
) -> Result<Vec<TrailingRatioPoint>, TrailingInputError> {
    validate_input(samples.iter().map(|sample| sample.at_ms), scope_start_ms, scope_end_ms)?;

    let endpoints = endpoints(scope_start_ms, scope_end_ms, spec)?;
    let mut points = Vec::with_capacity(endpoints.len());
    let mut lower = 0;
    let mut upper = 0;
    let mut numerator = 0u64;
    let mut denominator = 0u64;

    for end_ms in endpoints {
        while upper < samples.len() && samples[upper].at_ms <= end_ms {
            numerator =
                numerator.checked_add(samples[upper].numerator).ok_or(TrailingInputError::ArithmeticOverflow)?;
            denominator =
                denominator.checked_add(samples[upper].denominator).ok_or(TrailingInputError::ArithmeticOverflow)?;
            upper += 1;
        }

        let start_ms = end_ms - spec.window_ms();
        while lower < upper && samples[lower].at_ms <= start_ms {
            numerator =
                numerator.checked_sub(samples[lower].numerator).ok_or(TrailingInputError::ArithmeticOverflow)?;
            denominator =
                denominator.checked_sub(samples[lower].denominator).ok_or(TrailingInputError::ArithmeticOverflow)?;
            lower += 1;
        }

        points.push(TrailingRatioPoint {
            bucket: TimeBucket::complete(start_ms, end_ms),
            ratio: Ratio::new(numerator, denominator),
        });
    }

    Ok(points)
}

fn validate_input(
    timestamps: impl Iterator<Item = u64>,
    scope_start_ms: u64,
    scope_end_ms: u64,
) -> Result<(), TrailingInputError> {
    if scope_end_ms < scope_start_ms {
        return Err(TrailingInputError::InvalidScope { start_ms: scope_start_ms, end_ms: scope_end_ms });
    }

    let mut previous = None;
    for (index, at_ms) in timestamps.enumerate() {
        if previous.is_some_and(|prior| prior > at_ms) {
            return Err(TrailingInputError::UnsortedSamples { index });
        }
        previous = Some(at_ms);
    }
    Ok(())
}

fn endpoints(scope_start_ms: u64, scope_end_ms: u64, spec: TrailingSpec) -> Result<Vec<u64>, TrailingInputError> {
    let first_complete = scope_start_ms.checked_add(spec.window_ms()).ok_or(TrailingInputError::ArithmeticOverflow)?;
    if first_complete > scope_end_ms {
        return Ok(Vec::new());
    }

    let mut end_ms = align_up(first_complete, spec.alignment_ms(), spec.step_ms())?;
    let mut values = Vec::new();
    while end_ms <= scope_end_ms {
        values.push(end_ms);
        match end_ms.checked_add(spec.step_ms()) {
            Some(next) => end_ms = next,
            None => break,
        }
    }
    Ok(values)
}

fn align_up(value: u64, alignment: u64, step: u64) -> Result<u64, TrailingInputError> {
    if value <= alignment {
        return Ok(alignment);
    }
    let delta = value - alignment;
    let steps = delta.div_ceil(step);
    let span = steps.checked_mul(step).ok_or(TrailingInputError::ArithmeticOverflow)?;
    alignment.checked_add(span).ok_or(TrailingInputError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(window_ms: u64, step_ms: u64) -> TrailingSpec {
        TrailingSpec::new(window_ms, step_ms, 0).unwrap()
    }

    #[test]
    fn suppresses_leading_partial_windows() {
        let samples = [AdditiveSample { at_ms: 5, amount: 3 }];

        let points = trailing_sum(&samples, 0, 9, spec(10, 2)).unwrap();

        assert!(points.is_empty());
    }

    #[test]
    fn window_and_resolution_are_independent() {
        let samples = [
            AdditiveSample { at_ms: 5, amount: 1 },
            AdditiveSample { at_ms: 10, amount: 2 },
            AdditiveSample { at_ms: 15, amount: 4 },
            AdditiveSample { at_ms: 20, amount: 8 },
        ];

        let points = trailing_sum(&samples, 0, 20, spec(10, 5)).unwrap();

        assert_eq!(points.iter().map(|point| point.bucket.end_ms).collect::<Vec<_>>(), [10, 15, 20]);
        assert_eq!(points.iter().map(|point| point.sum).collect::<Vec<_>>(), [3, 6, 12]);
    }

    #[test]
    fn trailing_window_excludes_left_and_includes_right_boundary() {
        let samples = [
            AdditiveSample { at_ms: 0, amount: 1 },
            AdditiveSample { at_ms: 10, amount: 2 },
            AdditiveSample { at_ms: 20, amount: 4 },
        ];

        let points = trailing_sum(&samples, 0, 20, spec(10, 10)).unwrap();

        assert_eq!(points[0].sum, 2, "sample at the left boundary is excluded");
        assert_eq!(points[1].sum, 4, "sample at the right boundary is included");
    }

    #[test]
    fn coarse_slice_sum_suppresses_leading_windows_and_keeps_step_independent() {
        let points = trailing_slice_sum(&[1.0, 2.0, 4.0, 8.0, 16.0], 3, 2).unwrap();
        assert_eq!(
            points,
            vec![TrailingSlicePoint { end_index: 2, sum: 7.0 }, TrailingSlicePoint { end_index: 4, sum: 28.0 },]
        );
        assert_eq!(trailing_slice_sum(&[1.0, 2.0], 3, 1), None);
    }

    #[test]
    fn ratio_sums_components_instead_of_averaging_percentages() {
        let samples = [
            RatioSample { at_ms: 5, numerator: 1, denominator: 2 },
            RatioSample { at_ms: 10, numerator: 9, denominator: 10 },
        ];

        let points = trailing_ratio(&samples, 0, 10, spec(10, 10)).unwrap();

        assert_eq!(points[0].ratio, Ratio::new(10, 12));
        assert!((points[0].ratio.percent().unwrap() - 83.333_333).abs() < 0.000_001);
    }

    #[test]
    fn rejects_unsorted_samples() {
        let samples = [AdditiveSample { at_ms: 10, amount: 1 }, AdditiveSample { at_ms: 5, amount: 1 }];

        assert_eq!(trailing_sum(&samples, 0, 10, spec(10, 10)), Err(TrailingInputError::UnsortedSamples { index: 1 }));
    }

    #[test]
    fn aligns_first_complete_window_to_requested_grid() {
        let points = trailing_sum(&[], 3, 30, TrailingSpec::new(10, 5, 0).unwrap()).unwrap();

        assert_eq!(points.iter().map(|point| point.bucket.end_ms).collect::<Vec<_>>(), [15, 20, 25, 30]);
        assert!(points.iter().all(|point| point.bucket.coverage_ms == 10 && !point.bucket.partial));
    }
}
