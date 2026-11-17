//! Within `reporting`, `bucket` is the TIME GRID a report is cut along: the
//! fixed-width period a caller asks for, and the split of a report scope into
//! adjacent buckets clipped to it. Also names the hour — the width every
//! default report and per-hour rate is stated in. Knows nothing about what is
//! being measured inside a bucket.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::TimeBucket;

/// One hour of simulated time, in milliseconds. The unit reporting buckets and
/// per-hour rates are expressed in, named once so call sites don't re-derive it.
pub const HOUR_MS: u64 = 3_600_000;

/// Adjacent fixed-width reporting buckets aligned to one simulation timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FixedPeriod {
    width_ms: NonZeroU64,
    alignment_ms: u64,
}

impl FixedPeriod {
    /// Hourly buckets aligned to simulation start — the grain every surface
    /// reports at by default. Infallible where [`FixedPeriod::new`] is not,
    /// because the width is a non-zero literal.
    pub const HOURLY: Self =
        Self { width_ms: NonZeroU64::new(HOUR_MS).expect("one hour is non-zero"), alignment_ms: 0 };

    pub fn new(width_ms: u64, alignment_ms: u64) -> Result<Self, BucketInputError> {
        let width_ms = NonZeroU64::new(width_ms).ok_or(BucketInputError::ZeroWidth)?;
        Ok(Self { width_ms, alignment_ms })
    }

    pub const fn width_ms(self) -> u64 {
        self.width_ms.get()
    }

    pub const fn alignment_ms(self) -> u64 {
        self.alignment_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BucketInputError {
    #[error("fixed bucket width must be greater than zero")]
    ZeroWidth,
    #[error("scope end {end_ms} precedes scope start {start_ms}")]
    InvalidScope { start_ms: u64, end_ms: u64 },
    #[error("bucket alignment {alignment_ms} follows scope start {scope_start_ms}")]
    AlignmentAfterScope { alignment_ms: u64, scope_start_ms: u64 },
    #[error("report time arithmetic overflowed")]
    TimeOverflow,
}

/// Split a half-open scope `[scope_start_ms, scope_end_ms)` into a fixed grid.
///
/// Boundary buckets are clipped to the scope and marked partial. A zero-length
/// scope returns no buckets. The alignment is normally simulation start (`0`).
pub fn fixed_buckets(
    scope_start_ms: u64,
    scope_end_ms: u64,
    period: FixedPeriod,
) -> Result<Vec<TimeBucket>, BucketInputError> {
    if scope_end_ms < scope_start_ms {
        return Err(BucketInputError::InvalidScope { start_ms: scope_start_ms, end_ms: scope_end_ms });
    }
    if scope_start_ms < period.alignment_ms() {
        return Err(BucketInputError::AlignmentAfterScope { alignment_ms: period.alignment_ms(), scope_start_ms });
    }
    if scope_start_ms == scope_end_ms {
        return Ok(Vec::new());
    }

    let width_ms = period.width_ms();
    let offset_ms = scope_start_ms - period.alignment_ms();
    let grid_steps = offset_ms / width_ms;
    let grid_span_ms = grid_steps.checked_mul(width_ms).ok_or(BucketInputError::TimeOverflow)?;
    let mut grid_start_ms = period.alignment_ms().checked_add(grid_span_ms).ok_or(BucketInputError::TimeOverflow)?;
    let mut buckets = Vec::new();

    while grid_start_ms < scope_end_ms {
        let grid_end_ms = grid_start_ms.checked_add(width_ms).ok_or(BucketInputError::TimeOverflow)?;
        let start_ms = grid_start_ms.max(scope_start_ms);
        let end_ms = grid_end_ms.min(scope_end_ms);
        buckets.push(TimeBucket {
            start_ms,
            end_ms,
            coverage_ms: end_ms - start_ms,
            partial: start_ms != grid_start_ms || end_ms != grid_end_ms,
        });
        grid_start_ms = grid_end_ms;
    }

    Ok(buckets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hour_boundaries_are_complete() {
        let buckets = fixed_buckets(0, 7_200_000, FixedPeriod::HOURLY).unwrap();

        assert_eq!(buckets, [TimeBucket::complete(0, 3_600_000), TimeBucket::complete(3_600_000, 7_200_000),]);
    }

    #[test]
    fn scope_edges_are_clipped_and_marked_partial() {
        let buckets = fixed_buckets(900_000, 4_500_000, FixedPeriod::HOURLY).unwrap();

        assert_eq!(
            buckets,
            [
                TimeBucket { start_ms: 900_000, end_ms: 3_600_000, coverage_ms: 2_700_000, partial: true },
                TimeBucket { start_ms: 3_600_000, end_ms: 4_500_000, coverage_ms: 900_000, partial: true },
            ]
        );
    }

    #[test]
    fn zero_length_scope_has_no_buckets() {
        assert!(fixed_buckets(100, 100, FixedPeriod::new(10, 0).unwrap()).unwrap().is_empty());
    }

    #[test]
    fn rejects_alignment_after_scope_start() {
        assert_eq!(
            fixed_buckets(100, 200, FixedPeriod::new(10, 101).unwrap()),
            Err(BucketInputError::AlignmentAfterScope { alignment_ms: 101, scope_start_ms: 100 })
        );
    }
}
