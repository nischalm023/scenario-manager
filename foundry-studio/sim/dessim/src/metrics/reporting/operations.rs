//! Within `reporting`, `operations` counts WHAT the DA50 fleet physically
//! did: throughput count tables from immutable pickup/drop facts, by
//! operation kind, tote payload group, and zone. Owns the DA50 operation
//! metric descriptors. Surface-agnostic.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::events::{TotePayloadState, VtmOperationKind};
use crate::observer::output::VtmOperationRecord;

use super::{
    AdditiveSample, AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, ReportBuildError, ReportPeriod,
    ReportRequest, TimeBucket, TrailingInputError, TrailingSpec, event_in_bucket, report_buckets, trailing_sum,
};

/// Canonical report ordering of DA50 operation kinds. Consumers must use
/// this, never a private copy — the taxonomy has exactly one home.
pub const DA50_OPERATION_KINDS: [VtmOperationKind; 4] = [
    VtmOperationKind::PickupStorage,
    VtmOperationKind::DropRelay,
    VtmOperationKind::PickupRelay,
    VtmOperationKind::DropStorage,
];

/// Canonical report ordering of payload groups. Single home; see above.
pub const DA50_PAYLOAD_GROUPS: [Da50PayloadGroup; 4] =
    [Da50PayloadGroup::All, Da50PayloadGroup::Empty, Da50PayloadGroup::NonEmpty, Da50PayloadGroup::Unknown];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Da50PayloadGroup {
    All,
    Empty,
    NonEmpty,
    Unknown,
}

impl Da50PayloadGroup {
    fn includes(self, payload: TotePayloadState) -> bool {
        match self {
            Self::All => true,
            Self::Empty => payload == TotePayloadState::Empty,
            Self::NonEmpty => payload == TotePayloadState::NonEmpty,
            Self::Unknown => payload == TotePayloadState::Unknown,
        }
    }
}

/// One count cell. `zone_id = None` is the warehouse total; `Some` is one
/// aisle. Rows include zeros so charts and tables have a stable shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50OperationCountRow {
    pub bucket: TimeBucket,
    pub zone_id: Option<u32>,
    pub operation: VtmOperationKind,
    pub payload: Da50PayloadGroup,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50ThroughputData {
    pub rows: Vec<Da50OperationCountRow>,
}

/// The metric descriptors this producer owns and emits with every report.
/// Callable without data, so surfaces can enumerate DA50 operation metrics
/// before a run exists.
pub fn da50_operations_descriptors() -> Vec<MetricDescriptor> {
    vec![MetricDescriptor::new(
        "da50.operations",
        "DA50 operations",
        "Physical pickups and drops by endpoint and tote payload state.",
        MetricUnit::Count,
        None,
        AggregationClass::Additive,
    )]
}

/// Build the canonical DA50 operation report from immutable operation facts.
///
/// Event counts use scheduled transfer timestamps, so they follow the active
/// motion mode. Fixed buckets are half-open except that the selected range's
/// final timestamp is included; this keeps a run-ending operation in the
/// full-run total. Trailing windows retain their explicit `(start, end]`
/// semantics.
pub fn da50_throughput_report(
    records: &[VtmOperationRecord],
    request: ReportRequest,
) -> Result<MetricsReport<Da50ThroughputData>, ReportBuildError> {
    let descriptors = da50_operations_descriptors();

    let mut records = records.to_vec();
    records.sort_by_key(|record| record.sim_time_ms);
    let zones: Vec<Option<u32>> = std::iter::once(None)
        .chain(records.iter().map(|record| record.zone_id).collect::<BTreeSet<_>>().into_iter().map(Some))
        .collect();

    let rows = match request.period {
        ReportPeriod::Trailing(spec) => {
            trailing_rows(&records, &zones, request.range.start_ms, request.range.end_ms, spec)?
        }
        ReportPeriod::Overall | ReportPeriod::Fixed(_) => {
            count_rows_for_buckets(&records, &zones, &report_buckets(request)?, request)
        }
    };

    Ok(MetricsReport::new(request, descriptors, Da50ThroughputData { rows }))
}

fn count_rows_for_buckets(
    records: &[VtmOperationRecord],
    zones: &[Option<u32>],
    buckets: &[TimeBucket],
    request: ReportRequest,
) -> Vec<Da50OperationCountRow> {
    let mut rows =
        Vec::with_capacity(buckets.len() * zones.len() * DA50_OPERATION_KINDS.len() * DA50_PAYLOAD_GROUPS.len());
    for &bucket in buckets {
        for &zone_id in zones {
            for operation in DA50_OPERATION_KINDS {
                for payload in DA50_PAYLOAD_GROUPS {
                    let count = records
                        .iter()
                        .filter(|record| {
                            event_in_bucket(record.sim_time_ms, bucket, request)
                                && zone_id.is_none_or(|zone_id| zone_id == record.zone_id)
                                && record.kind == operation
                                && payload.includes(record.payload)
                        })
                        .count() as u64;
                    rows.push(Da50OperationCountRow { bucket, zone_id, operation, payload, count });
                }
            }
        }
    }
    rows
}

fn trailing_rows(
    records: &[VtmOperationRecord],
    zones: &[Option<u32>],
    scope_start_ms: u64,
    scope_end_ms: u64,
    spec: TrailingSpec,
) -> Result<Vec<Da50OperationCountRow>, TrailingInputError> {
    let mut rows = Vec::new();
    for &zone_id in zones {
        for operation in DA50_OPERATION_KINDS {
            for payload in DA50_PAYLOAD_GROUPS {
                let samples: Vec<_> = records
                    .iter()
                    .filter(|record| {
                        zone_id.is_none_or(|zone_id| zone_id == record.zone_id)
                            && record.kind == operation
                            && payload.includes(record.payload)
                    })
                    .map(|record| AdditiveSample { at_ms: record.sim_time_ms, amount: 1 })
                    .collect();
                for point in trailing_sum(&samples, scope_start_ms, scope_end_ms, spec)? {
                    rows.push(Da50OperationCountRow {
                        bucket: point.bucket,
                        zone_id,
                        operation,
                        payload,
                        count: point.sum,
                    });
                }
            }
        }
    }
    rows.sort_by_key(|row| {
        (row.bucket.end_ms, row.zone_id, operation_index(row.operation), payload_index(row.payload))
    });
    Ok(rows)
}

fn operation_index(operation: VtmOperationKind) -> u8 {
    match operation {
        VtmOperationKind::PickupStorage => 0,
        VtmOperationKind::DropRelay => 1,
        VtmOperationKind::PickupRelay => 2,
        VtmOperationKind::DropStorage => 3,
    }
}

fn payload_index(payload: Da50PayloadGroup) -> u8 {
    match payload {
        Da50PayloadGroup::All => 0,
        Da50PayloadGroup::Empty => 1,
        Da50PayloadGroup::NonEmpty => 2,
        Da50PayloadGroup::Unknown => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::reporting::{FixedPeriod, MetricScope, ReportPeriod, ReportRequest, TrailingSpec};

    fn record(at_ms: u64, zone_id: u32, kind: VtmOperationKind, payload: TotePayloadState) -> VtmOperationRecord {
        VtmOperationRecord {
            sim_time_ms: at_ms,
            vtm_id: 1,
            vtm_cycle_id: at_ms,
            zone_id,
            kind,
            rack_id: 2,
            rack_level: Some(0),
            relay_id: 3,
            tote_id: 4,
            payload,
            presentation_id: Some(5),
        }
    }

    fn count(
        report: &MetricsReport<Da50ThroughputData>,
        end_ms: u64,
        zone_id: Option<u32>,
        operation: VtmOperationKind,
        payload: Da50PayloadGroup,
    ) -> u64 {
        report
            .data
            .rows
            .iter()
            .find(|row| {
                row.bucket.end_ms == end_ms
                    && row.zone_id == zone_id
                    && row.operation == operation
                    && row.payload == payload
            })
            .unwrap()
            .count
    }

    #[test]
    fn overall_includes_run_end_and_splits_payload_and_aisle() {
        let records = [
            record(0, 1, VtmOperationKind::PickupStorage, TotePayloadState::Empty),
            record(30, 2, VtmOperationKind::PickupStorage, TotePayloadState::NonEmpty),
        ];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 30, 10);

        let report = da50_throughput_report(&records, request).unwrap();

        assert_eq!(count(&report, 30, None, VtmOperationKind::PickupStorage, Da50PayloadGroup::All,), 2);
        assert_eq!(count(&report, 30, Some(1), VtmOperationKind::PickupStorage, Da50PayloadGroup::Empty,), 1);
        assert_eq!(count(&report, 30, Some(2), VtmOperationKind::PickupStorage, Da50PayloadGroup::NonEmpty,), 1);
    }

    #[test]
    fn post_embargo_fixed_buckets_are_scoped_and_boundary_stable() {
        let records = [
            record(9, 1, VtmOperationKind::DropRelay, TotePayloadState::Empty),
            record(10, 1, VtmOperationKind::DropRelay, TotePayloadState::Empty),
            record(20, 1, VtmOperationKind::DropRelay, TotePayloadState::Empty),
            record(30, 1, VtmOperationKind::DropRelay, TotePayloadState::Empty),
        ];
        let request = ReportRequest::for_run(
            MetricScope::PostEmbargo,
            ReportPeriod::Fixed(FixedPeriod::new(10, 0).unwrap()),
            30,
            10,
        );

        let report = da50_throughput_report(&records, request).unwrap();

        assert_eq!(count(&report, 20, None, VtmOperationKind::DropRelay, Da50PayloadGroup::All,), 1);
        assert_eq!(count(&report, 30, None, VtmOperationKind::DropRelay, Da50PayloadGroup::All,), 2);
    }

    #[test]
    fn trailing_report_uses_open_left_closed_right_window() {
        let records = [
            record(0, 1, VtmOperationKind::PickupRelay, TotePayloadState::NonEmpty),
            record(10, 1, VtmOperationKind::PickupRelay, TotePayloadState::NonEmpty),
            record(20, 1, VtmOperationKind::PickupRelay, TotePayloadState::NonEmpty),
        ];
        let request = ReportRequest::for_run(
            MetricScope::FullRun,
            ReportPeriod::Trailing(TrailingSpec::new(10, 10, 0).unwrap()),
            20,
            0,
        );

        let report = da50_throughput_report(&records, request).unwrap();

        assert_eq!(count(&report, 10, None, VtmOperationKind::PickupRelay, Da50PayloadGroup::All,), 1);
        assert_eq!(count(&report, 20, None, VtmOperationKind::PickupRelay, Da50PayloadGroup::All,), 1);
    }

    #[test]
    fn taxonomy_arrays_are_exhaustive() {
        // A new enum variant must break this match, forcing the canonical
        // ordering arrays to be updated with it.
        for group in DA50_PAYLOAD_GROUPS {
            match group {
                Da50PayloadGroup::All
                | Da50PayloadGroup::Empty
                | Da50PayloadGroup::NonEmpty
                | Da50PayloadGroup::Unknown => {}
            }
        }
        assert_eq!(DA50_OPERATION_KINDS.len(), 4);
    }

    #[test]
    fn empty_operations_are_a_measured_zero_and_carry_descriptors() {
        // A run with no DA50 operations truthfully measured zero of them —
        // there is no profile-version ladder to declare data Unknown. The
        // producer still emits its own descriptors with the report.
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0);

        let report = da50_throughput_report(&[], request).unwrap();

        let total: u64 = report.data.rows.iter().map(|row| row.count).sum();
        assert_eq!(total, 0);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(report.descriptors[0].id, "da50.operations");
    }
}
