//! Within `reporting`, `occupancy` answers HOW FULL each aisle was: storage
//! slots by payload state and diving-board positions, as capacity-time
//! ratios per (bucket, zone), computed from the observer's aisle state
//! snapshots. Owns the occupancy metric descriptors. Surface-agnostic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::observer::output::AisleStateRecord;

use super::{
    AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, Ratio, ReportBuildError, ReportPeriod,
    ReportRequest, SupportStatus, TimeBucket, report_buckets,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AisleOccupancyRow {
    pub bucket: TimeBucket,
    pub zone_id: u32,
    pub status: SupportStatus,
    /// Occupied storage slot-time / available storage slot-time.
    pub storage_occupied: Option<Ratio>,
    /// Empty-tote storage slot-time / available storage slot-time.
    pub storage_empty: Option<Ratio>,
    /// Non-empty-tote storage slot-time / available storage slot-time.
    pub storage_non_empty: Option<Ratio>,
    /// Unknown-payload storage slot-time / available storage slot-time.
    pub storage_unknown: Option<Ratio>,
    /// Physically occupied diving-board position-time / available
    /// diving-board position-time. Reservation state is not used.
    pub diving_board_occupied: Option<Ratio>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AisleOccupancyData {
    pub rows: Vec<AisleOccupancyRow>,
}

/// The metric descriptors this producer owns and emits with every report.
/// Callable without data, so surfaces can enumerate utilization metrics
/// before a run exists.
pub fn aisle_occupancy_descriptors() -> Vec<MetricDescriptor> {
    vec![
        MetricDescriptor::new(
            "storage.slots.occupied",
            "Occupied storage slots",
            "Slot-time holding any tote over capacity slot-time, by aisle.",
            MetricUnit::Ratio,
            Some("Storage capacity slot-time in the aisle".into()),
            AggregationClass::Ratio,
        ),
        MetricDescriptor::new(
            "storage.slots.empty_tote",
            "Empty totes in storage",
            "Slot-time holding an empty container over capacity slot-time, by aisle.",
            MetricUnit::Ratio,
            Some("Storage capacity slot-time in the aisle".into()),
            AggregationClass::Ratio,
        ),
        MetricDescriptor::new(
            "storage.slots.non_empty_tote",
            "Non-empty totes in storage",
            "Slot-time holding a container with physical contents over capacity slot-time, by aisle.",
            MetricUnit::Ratio,
            Some("Storage capacity slot-time in the aisle".into()),
            AggregationClass::Ratio,
        ),
        MetricDescriptor::new(
            "storage.slots.unknown_payload",
            "Unknown-payload totes in storage",
            "Slot-time holding a tote of unknown payload over capacity slot-time, by aisle.",
            MetricUnit::Ratio,
            Some("Storage capacity slot-time in the aisle".into()),
            AggregationClass::Ratio,
        ),
        MetricDescriptor::new(
            "relay.positions.occupied",
            "Occupied diving-board positions",
            "Physical tote position-time over capacity position-time, by aisle.",
            MetricUnit::Ratio,
            Some("Diving-board capacity position-time in the aisle".into()),
            AggregationClass::Ratio,
        ),
    ]
}

/// Integrate exact per-aisle state transitions into capacity-time ratios.
///
/// The returned ratios retain numerator and denominator. Presentation layers
/// may show an average count by dividing slot-time by bucket coverage, or a
/// fill/utilization percentage by dividing numerator by capacity-time.
pub fn aisle_occupancy_report(
    records: &[AisleStateRecord],
    request: ReportRequest,
) -> Result<MetricsReport<AisleOccupancyData>, ReportBuildError> {
    let descriptors = aisle_occupancy_descriptors();
    let buckets = report_buckets(request)?;
    let by_zone = records_by_zone(records);
    let mut rows = Vec::with_capacity(buckets.len() * by_zone.len());
    for bucket in buckets {
        for (&zone_id, zone_records) in &by_zone {
            rows.push(integrate_zone(zone_id, zone_records, bucket)?);
        }
    }
    rows.sort_by_key(|row| (row.bucket.start_ms, row.zone_id));

    Ok(MetricsReport::new(request, descriptors, AisleOccupancyData { rows }))
}

fn records_by_zone(records: &[AisleStateRecord]) -> BTreeMap<u32, Vec<AisleStateRecord>> {
    let mut by_zone: BTreeMap<u32, Vec<_>> = BTreeMap::new();
    for &record in records {
        by_zone.entry(record.zone_id).or_default().push(record);
    }
    for zone_records in by_zone.values_mut() {
        zone_records.sort_by_key(|record| record.sim_time_ms);
    }
    by_zone
}

fn integrate_zone(
    zone_id: u32,
    records: &[AisleStateRecord],
    bucket: TimeBucket,
) -> Result<AisleOccupancyRow, ReportBuildError> {
    let Some(mut current) = records.iter().rev().find(|record| record.sim_time_ms <= bucket.start_ms).copied() else {
        return Ok(unknown_row(zone_id, bucket));
    };
    if !valid_state(current) {
        return Ok(unknown_row(zone_id, bucket));
    }

    let mut totals = IntegratedState::default();
    let mut cursor_ms = bucket.start_ms;
    for &next in
        records.iter().filter(|record| record.sim_time_ms > bucket.start_ms && record.sim_time_ms < bucket.end_ms)
    {
        totals.add(current, next.sim_time_ms - cursor_ms)?;
        if !valid_state(next) {
            return Ok(unknown_row(zone_id, bucket));
        }
        current = next;
        cursor_ms = next.sim_time_ms;
    }
    totals.add(current, bucket.end_ms - cursor_ms)?;

    Ok(AisleOccupancyRow {
        bucket,
        zone_id,
        status: SupportStatus::Measured,
        storage_occupied: Some(Ratio::new(totals.storage_occupied, totals.storage_capacity)),
        storage_empty: Some(Ratio::new(totals.storage_empty, totals.storage_capacity)),
        storage_non_empty: Some(Ratio::new(totals.storage_non_empty, totals.storage_capacity)),
        storage_unknown: Some(Ratio::new(totals.storage_unknown, totals.storage_capacity)),
        diving_board_occupied: Some(Ratio::new(totals.diving_board_occupied, totals.diving_board_capacity)),
    })
}

/// Whether a snapshot's parts sum to its whole, so the row can be `Measured`.
///
/// The composition sum is widened to `u64` rather than saturated: this is the
/// predicate that decides `Measured` vs `Unknown`, so a sum that clamped
/// instead of overflowing could make an inconsistent snapshot compare equal
/// and pass. Widening cannot overflow — three slot counts are far below
/// `u64::MAX` — so the comparison is exact for every input.
fn valid_state(record: AisleStateRecord) -> bool {
    let composition_sum =
        u64::from(record.storage_empty) + u64::from(record.storage_non_empty) + u64::from(record.storage_unknown);
    record.storage_occupied <= record.storage_capacity
        && u64::from(record.storage_occupied) == composition_sum
        && record.diving_board_occupied <= record.diving_board_capacity
}

fn unknown_row(zone_id: u32, bucket: TimeBucket) -> AisleOccupancyRow {
    AisleOccupancyRow {
        bucket,
        zone_id,
        status: SupportStatus::Unknown,
        storage_occupied: None,
        storage_empty: None,
        storage_non_empty: None,
        storage_unknown: None,
        diving_board_occupied: None,
    }
}

#[derive(Default)]
struct IntegratedState {
    storage_capacity: u64,
    storage_occupied: u64,
    storage_empty: u64,
    storage_non_empty: u64,
    storage_unknown: u64,
    diving_board_capacity: u64,
    diving_board_occupied: u64,
}

impl IntegratedState {
    fn add(&mut self, state: AisleStateRecord, duration_ms: u64) -> Result<(), ReportBuildError> {
        self.storage_capacity = add_product(self.storage_capacity, state.storage_capacity, duration_ms)?;
        self.storage_occupied = add_product(self.storage_occupied, state.storage_occupied, duration_ms)?;
        self.storage_empty = add_product(self.storage_empty, state.storage_empty, duration_ms)?;
        self.storage_non_empty = add_product(self.storage_non_empty, state.storage_non_empty, duration_ms)?;
        self.storage_unknown = add_product(self.storage_unknown, state.storage_unknown, duration_ms)?;
        self.diving_board_capacity = add_product(self.diving_board_capacity, state.diving_board_capacity, duration_ms)?;
        self.diving_board_occupied = add_product(self.diving_board_occupied, state.diving_board_occupied, duration_ms)?;
        Ok(())
    }
}

fn add_product(current: u64, count: u32, duration_ms: u64) -> Result<u64, ReportBuildError> {
    let contribution = u64::from(count).checked_mul(duration_ms).ok_or(ReportBuildError::ArithmeticOverflow)?;
    current.checked_add(contribution).ok_or(ReportBuildError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::reporting::{FixedPeriod, MetricScope};

    fn state(at_ms: u64, occupied: u32, empty: u32, non_empty: u32, relay_occupied: u32) -> AisleStateRecord {
        AisleStateRecord {
            sim_time_ms: at_ms,
            zone_id: 1,
            storage_capacity: 2,
            storage_occupied: occupied,
            storage_empty: empty,
            storage_non_empty: non_empty,
            storage_unknown: 0,
            diving_board_capacity: 1,
            diving_board_occupied: relay_occupied,
        }
    }

    #[test]
    fn overall_integrates_storage_composition_and_physical_relay_time() {
        let records = [state(0, 1, 1, 0, 0), state(10, 2, 1, 1, 1), state(20, 1, 0, 1, 0)];
        let report = aisle_occupancy_report(
            &records,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 30, 0),
        )
        .unwrap();
        let row = report.data.rows[0];

        assert_eq!(row.storage_occupied, Some(Ratio::new(40, 60)));
        assert_eq!(row.storage_empty, Some(Ratio::new(20, 60)));
        assert_eq!(row.storage_non_empty, Some(Ratio::new(20, 60)));
        assert_eq!(row.storage_unknown, Some(Ratio::new(0, 60)));
        assert_eq!(row.diving_board_occupied, Some(Ratio::new(10, 30)));
    }

    #[test]
    fn fixed_buckets_split_step_state_at_boundaries() {
        let records = [state(0, 1, 1, 0, 0), state(10, 2, 1, 1, 1), state(20, 1, 0, 1, 0)];
        let report = aisle_occupancy_report(
            &records,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::new(15, 0).unwrap()), 30, 0),
        )
        .unwrap();

        assert_eq!(report.data.rows[0].storage_occupied, Some(Ratio::new(20, 30)));
        assert_eq!(report.data.rows[1].storage_occupied, Some(Ratio::new(20, 30)));
    }

    #[test]
    fn post_embargo_uses_state_effective_at_scope_boundary() {
        let records = [state(0, 1, 1, 0, 0), state(10, 2, 1, 1, 1)];
        let report = aisle_occupancy_report(
            &records,
            ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 20, 10),
        )
        .unwrap();

        assert_eq!(report.data.rows[0].storage_occupied, Some(Ratio::new(20, 20)));
        assert_eq!(report.data.rows[0].diving_board_occupied, Some(Ratio::new(10, 10)));
    }

    #[test]
    fn missing_seed_snapshot_is_unknown_not_zero() {
        // First record arrives mid-bucket: the state before it is not in the
        // captured facts, so the whole (zone, bucket) row is unattributable.
        let late = [state(5, 2, 1, 1, 0)];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0);
        let report = aisle_occupancy_report(&late, request).unwrap();
        assert_eq!(report.data.rows[0].status, SupportStatus::Unknown);
        assert_eq!(report.data.rows[0].diving_board_occupied, None);
    }

    #[test]
    fn mid_bucket_invalid_snapshot_poisons_the_row() {
        // Valid seed, then a snapshot violating its own accounting identity:
        // integrating through corrupt capture would launder a contradiction,
        // so the row is Unknown even though the seed was fine.
        let records = [state(0, 1, 1, 0, 0), state(4, 3, 1, 2, 0)];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0);
        let report = aisle_occupancy_report(&records, request).unwrap();
        assert_eq!(report.data.rows[0].status, SupportStatus::Unknown);
        assert_eq!(report.data.rows[0].storage_occupied, None);
    }

    #[test]
    fn inconsistent_facts_are_unknown_not_zero() {
        // A snapshot violating its own accounting identity cannot be
        // attributed: the row is Unknown with no values — never a
        // fabricated zero. A run with no aisle records at all simply has
        // no zones, hence no rows: nothing measured, nothing invented.
        let invalid = [state(0, 3, 1, 2, 0)];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0);
        let report = aisle_occupancy_report(&invalid, request).unwrap();
        assert_eq!(report.data.rows[0].status, SupportStatus::Unknown);
        assert_eq!(report.data.rows[0].storage_occupied, None);

        let empty = aisle_occupancy_report(&[], request).unwrap();
        assert!(empty.data.rows.is_empty());
        assert_eq!(empty.descriptors.len(), 5);
    }
}
