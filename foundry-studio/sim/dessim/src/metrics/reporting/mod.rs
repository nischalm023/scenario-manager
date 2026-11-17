//! Within `metrics`, `reporting` is the COMPUTATION of canonical reports:
//! turning captured facts into typed, scoped, bucketed report tables.
//!
//! This module owns the ask-and-answer semantics every builder must share —
//! report requests, time buckets and membership, trailing windows, lossless
//! numerator/denominator ratios, support status — plus the report builders
//! themselves (one file per metric family, each owning its descriptors).
//! Facts stay in `observer`; meaning vocabulary in `metrics::descriptor`;
//! transport in `metrics::streams`. Nothing here knows any rendering surface.

mod bucket;
mod d30_state;
mod destination_striping;
mod distribution;
mod extraction;
mod occupancy;
mod operations;
mod request;
mod spans;
mod state;
mod trailing;

use serde::{Deserialize, Serialize};

use crate::SimTime;
use crate::model::RobotId;

pub use crate::metrics::descriptor::{AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit};
pub use bucket::{BucketInputError, FixedPeriod, HOUR_MS, fixed_buckets};
#[cfg(any(test, feature = "test-support"))]
pub use d30_state::d30_stretches;
pub use d30_state::{
    D30_STATES, D30_TRAVEL_KINDS, D30_WAIT_CATEGORIES, D30_WAIT_REASONS, D30DistanceRow, D30State, D30StateData,
    D30StateDurationRow, D30TravelKind, D30VehicleDistanceRow, D30VehicleStateDurationRow, D30VehicleWaitCategoryRow,
    D30VehicleWaitDurationRow, D30WaitCategory, D30WaitCategoryRow, D30WaitDurationRow, D30WaitReason,
    d30_state_descriptors, d30_state_report,
};
pub use destination_striping::{
    DestinationPresentationRow, DestinationStripingData, PlannedDestinationRow, StripePlanShareRow, StripePlanSnapshot,
    StripingUnattributedRow, destination_striping_descriptors, destination_striping_report,
};
pub use distribution::{
    AisleStationWorkRow, CompletedWorkData, DcStationWorkRow, UnattributedWorkRow, completed_work_descriptors,
    completed_work_report,
};
pub use extraction::{ExtractionData, ExtractionRow, extraction_descriptors, extraction_report, extraction_stream};
pub use occupancy::{AisleOccupancyData, AisleOccupancyRow, aisle_occupancy_descriptors, aisle_occupancy_report};
pub use operations::{
    DA50_OPERATION_KINDS, DA50_PAYLOAD_GROUPS, Da50OperationCountRow, Da50PayloadGroup, Da50ThroughputData,
    da50_operations_descriptors, da50_throughput_report,
};
pub use request::{ReportRange, ReportRequest};
pub use spans::{
    D30StateDefinition, D30WaitDefinition, Da50StateDefinition, Da50WaitDefinition, SpanDefinition, SpanError,
    SpanEvent, SpanReader, SpanWindow, SpanWindows,
};
#[cfg(any(test, feature = "test-support"))]
pub use state::da50_stretches;
pub use state::{
    DA50_LOAD_STATES, DA50_STATES, DA50_WAIT_REASONS, Da50DistanceRow, Da50State, Da50StateData, Da50StateDurationRow,
    Da50TravelLoad, Da50VehicleDistanceRow, Da50VehicleStateDurationRow, Da50VehicleWaitDurationRow,
    Da50WaitDurationRow, Da50WaitReason, da50_state_descriptors, da50_state_report,
};
pub use trailing::{
    AdditiveSample, RatioSample, TrailingInputError, TrailingRatioPoint, TrailingSlicePoint, TrailingSpec,
    TrailingSpecError, TrailingSumPoint, trailing_buckets, trailing_ratio, trailing_slice_sum, trailing_sum,
};

/// Time aggregation requested for a metric report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportPeriod {
    /// One aggregate spanning the selected scope.
    Overall,
    /// Adjacent half-open buckets on a fixed time grid.
    Fixed(FixedPeriod),
    /// Causal `(t - window, t]` values emitted at an independent resolution.
    Trailing(TrailingSpec),
}

/// One canonical, typed report ready for any presentation surface.
///
/// `descriptors` are supplied by the producer that built `data`; they are the
/// report's self-description, not a selection from any external list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsReport<T> {
    pub request: ReportRequest,
    pub descriptors: Vec<MetricDescriptor>,
    pub data: T,
}

impl<T> MetricsReport<T> {
    pub fn new(request: ReportRequest, descriptors: Vec<MetricDescriptor>, data: T) -> Self {
        Self { request, descriptors, data }
    }
}

/// The bucket a time cursor selects: the first bucket that contains `at_ms`
/// under this request's own membership rule, clamped to the ends — a cursor
/// at (or past) the range end selects the last bucket, one before the first
/// selects the first. Never silently empty for a non-empty schedule, so a
/// surface resolving a cursor cannot quietly fall back to something else.
pub fn bucket_at(buckets: &[TimeBucket], at_ms: u64, request: ReportRequest) -> Option<TimeBucket> {
    buckets
        .iter()
        .copied()
        .find(|bucket| event_in_bucket(at_ms, *bucket, request))
        .or_else(|| buckets.last().copied().filter(|last| at_ms >= last.end_ms))
        .or_else(|| buckets.first().copied())
}

/// Which portion of a simulation run contributes to a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricScope {
    /// Include the complete simulation run.
    FullRun,
    /// Exclude time and events before the configured pick embargo.
    PostEmbargo,
}

/// Whether a value is genuinely measured.
///
/// Support derives from what the model produces and captures: absence of
/// activity is a measured zero; a value the producer cannot attribute from
/// the captured facts is `Unknown`; behavior the simulator does not model is
/// `Unsupported`. The latter two carry no fabricated value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SupportStatus {
    /// The simulator emitted the facts needed to measure this value.
    Measured,
    /// The producer could not attribute this value from the captured facts
    /// — e.g. a missing or internally inconsistent state snapshot. Never a
    /// fabricated zero, and nothing to do with any profile version.
    Unknown,
    /// The simulator does not currently model this behavior.
    Unsupported,
}

/// Why a canonical report could not be built from its request and facts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportBuildError {
    #[error("invalid report source stream: {0}")]
    SourceStream(String),
    #[error(transparent)]
    Bucket(#[from] BucketInputError),
    #[error(transparent)]
    Trailing(#[from] TrailingInputError),
    #[error("report arithmetic overflowed")]
    ArithmeticOverflow,
    #[error(
        "DA50 states do not reconcile in [{bucket_start_ms}, {bucket_end_ms}): measured {measured_ms} ms, fleet-time {fleet_time_ms} ms"
    )]
    StateReconciliation { bucket_start_ms: u64, bucket_end_ms: u64, measured_ms: u64, fleet_time_ms: u64 },
    #[error(
        "DA50 wait reasons do not reconcile in [{bucket_start_ms}, {bucket_end_ms}): reasons {reason_ms} ms, waiting {waiting_ms} ms"
    )]
    WaitReconciliation { bucket_start_ms: u64, bucket_end_ms: u64, reason_ms: u64, waiting_ms: u64 },
    // The unprefixed reconciliation variants above predate the D30 family
    // and carry DA50 text; the D30 builder gets its own so a failed report
    // names the right fleet.
    #[error(
        "D30 states do not reconcile in [{bucket_start_ms}, {bucket_end_ms}): measured {measured_ms} ms, fleet-time {fleet_time_ms} ms"
    )]
    D30StateReconciliation { bucket_start_ms: u64, bucket_end_ms: u64, measured_ms: u64, fleet_time_ms: u64 },
    #[error(
        "D30 wait reasons do not reconcile in [{bucket_start_ms}, {bucket_end_ms}): reasons {reason_ms} ms, waiting {waiting_ms} ms"
    )]
    D30WaitReconciliation { bucket_start_ms: u64, bucket_end_ms: u64, reason_ms: u64, waiting_ms: u64 },
    /// Facts name more distinct D30s than the configured eligible fleet.
    /// Counts, not ids: robot ids are not range-bound to the fleet size
    /// (VTMs and HTMs number independently), so only the count can breach.
    #[error("journey facts name {observed} distinct D30s, exceeding the configured fleet of {configured}")]
    D30FleetExceeded { configured: u32, observed: u32 },
    #[error(
        "overlapping D30 travel facts for robot {robot_id}: legs starting at {first_start_ms} ms and {second_start_ms} ms overlap, so distance would double-count"
    )]
    D30TravelOverlap { robot_id: u32, first_start_ms: u64, second_start_ms: u64 },
    #[error(
        "D30 robot {robot_id} has an impossible post-dwell station interval [{start}, {end}): open-ended scheduling reservations are not simulation-time occupancy"
    )]
    D30PostDwellStationHold { robot_id: RobotId, start: SimTime, end: SimTime },
    /// Statically unreachable while `D30TravelKind` partitions Travel legs
    /// exhaustively — this guards a future taxonomy edit that breaks the
    /// partition, not any state today's producer can reach.
    #[error(
        "D30 distance kinds do not reconcile in [{bucket_start_ms}, {bucket_end_ms}): split {split_mm} mm, total {total_mm} mm"
    )]
    D30DistanceReconciliation { bucket_start_ms: u64, bucket_end_ms: u64, split_mm: u64, total_mm: u64 },
}

/// Expand a report request into its time buckets: one complete bucket for
/// `Overall` (none when the range is empty), the fixed grid for `Fixed`,
/// and the trailing window schedule for `Trailing`. This module owns
/// time/scoping semantics; builders must use this instead of expanding
/// periods themselves.
pub fn report_buckets(request: ReportRequest) -> Result<Vec<TimeBucket>, ReportBuildError> {
    if request.range.end_ms < request.range.start_ms {
        return Err(
            BucketInputError::InvalidScope { start_ms: request.range.start_ms, end_ms: request.range.end_ms }.into()
        );
    }
    match request.period {
        ReportPeriod::Overall => Ok((!request.range.is_empty())
            .then(|| TimeBucket::complete(request.range.start_ms, request.range.end_ms))
            .into_iter()
            .collect()),
        ReportPeriod::Fixed(period) => Ok(fixed_buckets(request.range.start_ms, request.range.end_ms, period)?),
        ReportPeriod::Trailing(spec) => Ok(trailing_buckets(request.range.start_ms, request.range.end_ms, spec)?),
    }
}

/// Whether a single-instant event at `at_ms` belongs to `bucket` under the
/// request's period semantics: trailing windows are `(start, end]`; fixed and
/// overall buckets are half-open `[start, end)` except that the requested
/// range's final timestamp is included, so a run-ending event stays in the
/// full-run total. This module owns membership; builders must not re-derive it.
pub fn event_in_bucket(at_ms: u64, bucket: TimeBucket, request: ReportRequest) -> bool {
    match request.period {
        ReportPeriod::Trailing(_) => at_ms > bucket.start_ms && at_ms <= bucket.end_ms,
        ReportPeriod::Overall | ReportPeriod::Fixed(_) => {
            at_ms >= bucket.start_ms
                && (at_ms < bucket.end_ms || (bucket.end_ms == request.range.end_ms && at_ms == request.range.end_ms))
        }
    }
}

/// A lossless ratio used for percentages and utilization.
///
/// Consumers must sum numerators and denominators before dividing. Keeping the
/// pair intact prevents accidental averages of already-computed percentages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratio {
    pub numerator: u64,
    pub denominator: u64,
}

impl Ratio {
    pub const fn new(numerator: u64, denominator: u64) -> Self {
        Self { numerator, denominator }
    }

    /// Fraction in `[0, +inf)`, or `None` when no denominator exists.
    pub fn fraction(self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }

    /// Percentage value, or `None` when no denominator exists.
    pub fn percent(self) -> Option<f64> {
        self.fraction().map(|fraction| fraction * 100.0)
    }
}

/// One emitted reporting interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimeBucket {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Source time represented by this bucket.
    pub coverage_ms: u64,
    /// True only when the source does not cover the complete requested period.
    pub partial: bool,
}

impl TimeBucket {
    pub const fn complete(start_ms: u64, end_ms: u64) -> Self {
        Self { start_ms, end_ms, coverage_ms: end_ms - start_ms, partial: false }
    }
}

#[cfg(test)]
mod tests {
    use super::Ratio;

    #[test]
    fn zero_denominator_is_not_reported_as_zero_percent() {
        assert_eq!(Ratio::new(0, 0).percent(), None);
    }

    #[test]
    fn ratio_retains_raw_values() {
        let ratio = Ratio::new(3, 4);
        assert_eq!(ratio.numerator, 3);
        assert_eq!(ratio.denominator, 4);
        assert_eq!(ratio.percent(), Some(75.0));
    }
}
