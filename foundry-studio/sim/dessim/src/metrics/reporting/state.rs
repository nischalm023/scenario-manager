//! Within `reporting`, `state` partitions DA50 FLEET-TIME: journey-leg
//! facts become state/wait-reason/travel-distance tables where every
//! measured bucket partitions each DA50's time exactly once. "State" here
//! means DA50 operational state (Pickup, Drop, Alignment, ...), not
//! observer or program state. Owns the DA50 state metric descriptors.
//! Surface-agnostic.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::events::{VtmCycleDirection, VtmIdleReasonKind};
use crate::observer::output::{Activity, VtmJournalEndpoint, VtmLegRecord};

use super::{
    AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, Ratio, ReportBuildError, ReportPeriod,
    ReportRequest, SupportStatus, TimeBucket, report_buckets,
};

/// Canonical report ordering of DA50 states. Consumers must use this,
/// never a private copy — the taxonomy has exactly one home.
pub const DA50_STATES: [Da50State; 9] = [
    Da50State::Pickup,
    Da50State::Drop,
    Da50State::Alignment,
    Da50State::EmptyTravel,
    Da50State::LadenTravel,
    Da50State::Waiting,
    Da50State::TravelToPark,
    Da50State::TravelToUnblockOther,
    Da50State::Unknown,
];

/// Canonical report ordering of wait reasons. Single home; see above.
pub const DA50_WAIT_REASONS: [Da50WaitReason; 7] = [
    Da50WaitReason::AwaitingDemand,
    Da50WaitReason::AwaitingTote,
    Da50WaitReason::AwaitingSlot,
    Da50WaitReason::AwaitingD30Deposit,
    Da50WaitReason::NoTask,
    Da50WaitReason::Unknown,
    Da50WaitReason::ConflictingFacts,
];

/// Canonical report ordering of travel-load states. Single home; see above.
pub const DA50_LOAD_STATES: [Da50TravelLoad; 2] = [Da50TravelLoad::Empty, Da50TravelLoad::Laden];

/// Mutually exclusive DA50 states used by duration and percentage reports.
///
/// `TravelToPark` and `TravelToUnblockOther` intentionally remain in the
/// stable taxonomy even though the simulator does not model them yet. Their
/// rows carry `Unsupported`, not a fabricated zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Da50State {
    Pickup,
    Drop,
    Alignment,
    EmptyTravel,
    LadenTravel,
    Waiting,
    TravelToPark,
    TravelToUnblockOther,
    /// Conflicting or incomplete facts overlap this interval.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Da50WaitReason {
    AwaitingDemand,
    AwaitingTote,
    AwaitingSlot,
    AwaitingD30Deposit,
    /// No selected cycle occupied the interval. This includes leading,
    /// between-cycle, and trailing availability gaps.
    NoTask,
    /// A wait fact exists but its reason is absent or invalid.
    Unknown,
    /// Multiple overlapping wait facts disagree on their reason.
    ConflictingFacts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Da50TravelLoad {
    Empty,
    Laden,
}

impl Da50State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pickup => "Pickup",
            Self::Drop => "Drop",
            Self::Alignment => "Alignment",
            Self::EmptyTravel => "Empty travel",
            Self::LadenTravel => "Laden travel",
            Self::Waiting => "Waiting",
            Self::TravelToPark => "Travel to park",
            Self::TravelToUnblockOther => "Travel to unblock other",
            Self::Unknown => "Unknown / unattributed",
        }
    }
}

impl Da50WaitReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::AwaitingDemand => "Awaiting demand",
            Self::AwaitingTote => "Awaiting tote",
            Self::AwaitingSlot => "Awaiting slot",
            Self::AwaitingD30Deposit => "Awaiting D30 deposit",
            Self::NoTask => "No selected task",
            Self::Unknown => "Unknown wait reason",
            Self::ConflictingFacts => "Conflicting facts",
        }
    }
}

impl Da50TravelLoad {
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::Laden => "Laden",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50StateDurationRow {
    pub bucket: TimeBucket,
    pub state: Da50State,
    pub status: SupportStatus,
    /// `None` distinguishes unknown/unsupported from a measured zero.
    pub duration_ms: Option<u64>,
    /// State duration divided by DA50 fleet-time in this bucket.
    pub fleet_share: Option<Ratio>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50WaitDurationRow {
    pub bucket: TimeBucket,
    pub reason: Da50WaitReason,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// Reason duration divided by all Waiting time in this bucket.
    pub waiting_share: Option<Ratio>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50DistanceRow {
    pub bucket: TimeBucket,
    pub load: Da50TravelLoad,
    pub status: SupportStatus,
    pub distance_mm: Option<u64>,
}

/// One observed DA50's share of one state in one bucket — the per-vehicle
/// drilldown of [`Da50StateDurationRow`]. Fleet rows equal the sum of these
/// plus the Waiting/NoTask backfill for configured-but-unobserved vehicles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50VehicleStateDurationRow {
    pub bucket: TimeBucket,
    pub vtm_id: u32,
    pub state: Da50State,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// State duration divided by this DA50's included time in the bucket.
    pub vehicle_share: Option<Ratio>,
}

/// One observed DA50's waiting time for one reason in one bucket — the
/// per-vehicle drilldown of [`Da50WaitDurationRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50VehicleWaitDurationRow {
    pub bucket: TimeBucket,
    pub vtm_id: u32,
    pub reason: Da50WaitReason,
    pub status: SupportStatus,
    pub duration_ms: Option<u64>,
    /// Reason duration divided by this DA50's Waiting time in the bucket.
    pub waiting_share: Option<Ratio>,
}

/// One observed DA50's travel distance under one load state in one bucket —
/// the per-vehicle drilldown of [`Da50DistanceRow`]; fleet rows equal the
/// sum of these (unobserved vehicles travel nowhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50VehicleDistanceRow {
    pub bucket: TimeBucket,
    pub vtm_id: u32,
    pub load: Da50TravelLoad,
    pub status: SupportStatus,
    pub distance_mm: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Da50StateData {
    pub fleet_size: u32,
    pub state_rows: Vec<Da50StateDurationRow>,
    pub wait_rows: Vec<Da50WaitDurationRow>,
    pub distance_rows: Vec<Da50DistanceRow>,
    pub vehicle_state_rows: Vec<Da50VehicleStateDurationRow>,
    pub vehicle_wait_rows: Vec<Da50VehicleWaitDurationRow>,
    pub vehicle_distance_rows: Vec<Da50VehicleDistanceRow>,
}

/// The metric descriptors this producer owns and emits with every report.
/// Callable without data, so surfaces can enumerate DA50 state metrics
/// before a run exists.
pub fn da50_state_descriptors() -> Vec<MetricDescriptor> {
    vec![
        MetricDescriptor::new(
            "da50.state.duration",
            "DA50 state time",
            "Time attributed to each mutually exclusive DA50 state.",
            MetricUnit::DurationMilliseconds,
            Some("DA50 fleet availability time".into()),
            AggregationClass::Interval,
        ),
        MetricDescriptor::new(
            "da50.wait.duration",
            "DA50 waiting time",
            "Selected-plan schedule gaps attributed post hoc by reason.",
            MetricUnit::DurationMilliseconds,
            Some("DA50 waiting time".into()),
            AggregationClass::Interval,
        ),
        MetricDescriptor::new(
            "da50.travel.distance.empty",
            "DA50 empty travel distance",
            "Rail distance traveled without a tote on board.",
            MetricUnit::DistanceMillimeters,
            None,
            AggregationClass::Additive,
        ),
        MetricDescriptor::new(
            "da50.travel.distance.laden",
            "DA50 laden travel distance",
            "Rail distance traveled with a tote on board.",
            MetricUnit::DistanceMillimeters,
            None,
            AggregationClass::Additive,
        ),
    ]
}

/// Build canonical DA50 state, selected-plan waiting, and travel-distance
/// tables from journey-leg facts.
///
/// Every measured bucket partitions each DA50's fleet-time exactly once.
/// Intervals with no selected leg are Waiting/NoTask. Conflicting overlapping
/// facts are Unknown instead of being double-counted. Waiting reasons describe
/// the chosen schedule post hoc; they do not prove congestion or optimality.
///
/// Per-vehicle drilldown rows carry the same partition per observed DA50:
/// each vehicle's states partition its own bucket time, and fleet rows are
/// the vehicle sums plus Waiting/NoTask backfill for configured-but-
/// unobserved vehicles.
pub fn da50_state_report(
    records: &[VtmLegRecord],
    configured_fleet_size: u32,
    request: ReportRequest,
) -> Result<MetricsReport<Da50StateData>, ReportBuildError> {
    let descriptors = da50_state_descriptors();
    let buckets = report_buckets(request)?;
    let observed_ids: BTreeSet<_> = records.iter().map(|record| record.vtm_id).collect();
    let fleet_size = configured_fleet_size.max(observed_ids.len() as u32);

    let mut state_rows = Vec::with_capacity(buckets.len() * DA50_STATES.len());
    let mut wait_rows = Vec::with_capacity(buckets.len() * DA50_WAIT_REASONS.len());
    let mut distance_rows = Vec::with_capacity(buckets.len() * DA50_LOAD_STATES.len());
    let mut vehicle_state_rows = Vec::new();
    let mut vehicle_wait_rows = Vec::new();
    let mut vehicle_distance_rows = Vec::new();
    let records_by_robot = records_by_robot(records, &observed_ids);

    for bucket in buckets {
        let mut state_durations = zero_states();
        let mut wait_durations = zero_wait_reasons();
        for (&vtm_id, robot_records) in &records_by_robot {
            let mut vehicle_states = zero_states();
            let mut vehicle_waits = zero_wait_reasons();
            accumulate_robot_time(robot_records, bucket, &mut vehicle_states, &mut vehicle_waits)?;

            // Each observed DA50 partitions its own bucket time exactly once;
            // the fleet reconciliation below then holds by construction.
            let vehicle_total = checked_sum(vehicle_states.values().copied())?;
            if vehicle_total != bucket.coverage_ms {
                return Err(ReportBuildError::StateReconciliation {
                    bucket_start_ms: bucket.start_ms,
                    bucket_end_ms: bucket.end_ms,
                    measured_ms: vehicle_total,
                    fleet_time_ms: bucket.coverage_ms,
                });
            }
            let vehicle_waiting_ms = vehicle_states[&Da50State::Waiting];
            let vehicle_reason_ms = checked_sum(vehicle_waits.values().copied())?;
            if vehicle_reason_ms != vehicle_waiting_ms {
                return Err(ReportBuildError::WaitReconciliation {
                    bucket_start_ms: bucket.start_ms,
                    bucket_end_ms: bucket.end_ms,
                    reason_ms: vehicle_reason_ms,
                    waiting_ms: vehicle_waiting_ms,
                });
            }

            for state in DA50_STATES {
                let measured_ms = vehicle_states[&state];
                add_duration(&mut state_durations, state, measured_ms)?;
                let status = state_support(state);
                let duration_ms = (status == SupportStatus::Measured).then_some(measured_ms);
                vehicle_state_rows.push(Da50VehicleStateDurationRow {
                    bucket,
                    vtm_id,
                    state,
                    status,
                    duration_ms,
                    vehicle_share: duration_ms.map(|duration_ms| Ratio::new(duration_ms, bucket.coverage_ms)),
                });
            }
            for reason in DA50_WAIT_REASONS {
                let duration_ms = vehicle_waits[&reason];
                add_duration(&mut wait_durations, reason, duration_ms)?;
                vehicle_wait_rows.push(Da50VehicleWaitDurationRow {
                    bucket,
                    vtm_id,
                    reason,
                    status: SupportStatus::Measured,
                    duration_ms: Some(duration_ms),
                    waiting_share: Some(Ratio::new(duration_ms, vehicle_waiting_ms)),
                });
            }
            for load in DA50_LOAD_STATES {
                vehicle_distance_rows.push(Da50VehicleDistanceRow {
                    bucket,
                    vtm_id,
                    load,
                    status: SupportStatus::Measured,
                    distance_mm: Some(distance_for_bucket(robot_records.iter().copied(), bucket, load)?),
                });
            }
        }

        let unobserved_robots = fleet_size.saturating_sub(records_by_robot.len() as u32);
        let unobserved_ms =
            bucket.coverage_ms.checked_mul(unobserved_robots as u64).ok_or(ReportBuildError::ArithmeticOverflow)?;
        add_duration(&mut state_durations, Da50State::Waiting, unobserved_ms)?;
        add_duration(&mut wait_durations, Da50WaitReason::NoTask, unobserved_ms)?;

        let fleet_time_ms =
            bucket.coverage_ms.checked_mul(fleet_size as u64).ok_or(ReportBuildError::ArithmeticOverflow)?;
        let measured_total = checked_sum(state_durations.values().copied())?;
        if measured_total != fleet_time_ms {
            return Err(ReportBuildError::StateReconciliation {
                bucket_start_ms: bucket.start_ms,
                bucket_end_ms: bucket.end_ms,
                measured_ms: measured_total,
                fleet_time_ms,
            });
        }

        let waiting_ms = state_durations[&Da50State::Waiting];
        let reason_ms = checked_sum(wait_durations.values().copied())?;
        if reason_ms != waiting_ms {
            return Err(ReportBuildError::WaitReconciliation {
                bucket_start_ms: bucket.start_ms,
                bucket_end_ms: bucket.end_ms,
                reason_ms,
                waiting_ms,
            });
        }
        for state in DA50_STATES {
            let status = state_support(state);
            let duration_ms = (status == SupportStatus::Measured).then_some(state_durations[&state]);
            state_rows.push(Da50StateDurationRow {
                bucket,
                state,
                status,
                duration_ms,
                fleet_share: duration_ms.map(|duration_ms| Ratio::new(duration_ms, fleet_time_ms)),
            });
        }
        for reason in DA50_WAIT_REASONS {
            let duration_ms = wait_durations[&reason];
            wait_rows.push(Da50WaitDurationRow {
                bucket,
                reason,
                status: SupportStatus::Measured,
                duration_ms: Some(duration_ms),
                waiting_share: Some(Ratio::new(duration_ms, waiting_ms)),
            });
        }
        for load in DA50_LOAD_STATES {
            distance_rows.push(Da50DistanceRow {
                bucket,
                load,
                status: SupportStatus::Measured,
                distance_mm: Some(distance_for_bucket(records, bucket, load)?),
            });
        }
    }

    Ok(MetricsReport::new(
        request,
        descriptors,
        Da50StateData {
            fleet_size,
            state_rows,
            wait_rows,
            distance_rows,
            vehicle_state_rows,
            vehicle_wait_rows,
            vehicle_distance_rows,
        },
    ))
}

fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64, ReportBuildError> {
    values.try_fold(0u64, |sum, value| sum.checked_add(value).ok_or(ReportBuildError::ArithmeticOverflow))
}

fn records_by_robot<'a>(
    records: &'a [VtmLegRecord],
    observed_ids: &BTreeSet<u32>,
) -> BTreeMap<u32, Vec<&'a VtmLegRecord>> {
    let mut by_robot: BTreeMap<_, Vec<_>> = observed_ids.iter().copied().map(|id| (id, Vec::new())).collect();
    for record in records {
        by_robot.entry(record.vtm_id).or_default().push(record);
    }
    for robot_records in by_robot.values_mut() {
        robot_records.sort_by_key(|record| (record.start_ms, record.duration_ms));
    }
    by_robot
}

fn zero_states() -> BTreeMap<Da50State, u64> {
    DA50_STATES.into_iter().map(|state| (state, 0)).collect()
}

fn zero_wait_reasons() -> BTreeMap<Da50WaitReason, u64> {
    DA50_WAIT_REASONS.into_iter().map(|reason| (reason, 0)).collect()
}

fn accumulate_robot_time(
    records: &[&VtmLegRecord],
    bucket: TimeBucket,
    states: &mut BTreeMap<Da50State, u64>,
    waits: &mut BTreeMap<Da50WaitReason, u64>,
) -> Result<(), ReportBuildError> {
    each_classified_stretch(records, bucket, |start_ms, end_ms, state, wait_reason| {
        let duration_ms = end_ms - start_ms;
        add_duration(states, state, duration_ms)?;
        if state == Da50State::Waiting {
            add_duration(waits, wait_reason.unwrap_or(Da50WaitReason::Unknown), duration_ms)?;
        }
        Ok(())
    })
}

/// Each stretch of one DA50's time over `[start_ms, end_ms)` as this report
/// partitions it from `records`, the robot's whole journal: its state and, in
/// Waiting, its reason. A stretch ends wherever a fact begins or ends, so two
/// that touch may carry one state. The robot state channel's parity test
/// reads it; it goes with the interval rows.
#[cfg(any(test, feature = "test-support"))]
pub fn da50_stretches(
    records: &[VtmLegRecord],
    start_ms: u64,
    end_ms: u64,
) -> Vec<(u64, u64, Da50State, Option<Da50WaitReason>)> {
    let mut sorted: Vec<&VtmLegRecord> = records.iter().collect();
    sorted.sort_by_key(|record| (record.start_ms, record.duration_ms));
    let mut stretches = Vec::new();
    let walked =
        each_classified_stretch(&sorted, TimeBucket::complete(start_ms, end_ms), |start, end, state, reason| {
            let reason = (state == Da50State::Waiting).then(|| reason.unwrap_or(Da50WaitReason::Unknown));
            stretches.push((start, end, state, reason));
            Ok(())
        });
    walked.expect("collecting a stretch fails nothing");
    stretches
}

/// Hand `take` one robot's time over `bucket`, in order, cut wherever one of
/// its facts begins or ends, each stretch classified once; the first error
/// `take` returns ends the walk.
fn each_classified_stretch(
    records: &[&VtmLegRecord],
    bucket: TimeBucket,
    mut take: impl FnMut(u64, u64, Da50State, Option<Da50WaitReason>) -> Result<(), ReportBuildError>,
) -> Result<(), ReportBuildError> {
    let relevant: Vec<_> = records
        .iter()
        .copied()
        .filter(|record| {
            record.duration_ms > 0
                && record.start_ms < bucket.end_ms
                && record.start_ms.saturating_add(record.duration_ms) > bucket.start_ms
        })
        .collect();
    let mut boundaries = vec![bucket.start_ms, bucket.end_ms];
    for record in &relevant {
        boundaries.push(record.start_ms.max(bucket.start_ms));
        boundaries.push(record.start_ms.saturating_add(record.duration_ms).min(bucket.end_ms));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    for pair in boundaries.windows(2) {
        let start_ms = pair[0];
        let end_ms = pair[1];
        if start_ms == end_ms {
            continue;
        }
        let active: Vec<_> = relevant
            .iter()
            .copied()
            .filter(|record| record.start_ms < end_ms && record.start_ms.saturating_add(record.duration_ms) > start_ms)
            .collect();
        let (state, wait_reason) = classify_active(&active);
        take(start_ms, end_ms, state, wait_reason)?;
    }
    Ok(())
}

fn classify_active(records: &[&VtmLegRecord]) -> (Da50State, Option<Da50WaitReason>) {
    if records.is_empty() {
        return (Da50State::Waiting, Some(Da50WaitReason::NoTask));
    }
    let classifications: Vec<_> = records.iter().map(|record| classify(record)).collect();
    let first_state = classifications[0].0;
    if classifications.iter().any(|classification| classification.0 != first_state) {
        return (Da50State::Unknown, None);
    }
    if first_state != Da50State::Waiting {
        return (first_state, None);
    }
    let first_reason = classifications[0].1.unwrap_or(Da50WaitReason::Unknown);
    if classifications.iter().any(|classification| classification.1.unwrap_or(Da50WaitReason::Unknown) != first_reason)
    {
        return (Da50State::Waiting, Some(Da50WaitReason::ConflictingFacts));
    }
    (Da50State::Waiting, Some(first_reason))
}

fn classify(record: &VtmLegRecord) -> (Da50State, Option<Da50WaitReason>) {
    match record.activity {
        Activity::Travel => (if record.carrying_tote { Da50State::LadenTravel } else { Da50State::EmptyTravel }, None),
        Activity::Alignment => (Da50State::Alignment, None),
        Activity::Wait => (Da50State::Waiting, Some(wait_reason(record.wait_reason))),
        Activity::Mechanical => (mechanical_state(record), None),
    }
}

fn mechanical_state(record: &VtmLegRecord) -> Da50State {
    if record.notes.contains("get") {
        return Da50State::Pickup;
    }
    if record.notes.contains("put") {
        return Da50State::Drop;
    }
    match (record.direction, record.from) {
        (VtmCycleDirection::Forward, VtmJournalEndpoint::Rack(_))
        | (VtmCycleDirection::Return, VtmJournalEndpoint::Relay(_)) => Da50State::Pickup,
        (VtmCycleDirection::Forward, VtmJournalEndpoint::Relay(_))
        | (VtmCycleDirection::Return, VtmJournalEndpoint::Rack(_)) => Da50State::Drop,
        _ => Da50State::Unknown,
    }
}

fn wait_reason(reason: Option<VtmIdleReasonKind>) -> Da50WaitReason {
    match reason {
        Some(VtmIdleReasonKind::AwaitingDemand) => Da50WaitReason::AwaitingDemand,
        Some(VtmIdleReasonKind::AwaitingTote) => Da50WaitReason::AwaitingTote,
        Some(VtmIdleReasonKind::AwaitingSlot) => Da50WaitReason::AwaitingSlot,
        Some(VtmIdleReasonKind::AwaitingHtmDeposit) => Da50WaitReason::AwaitingD30Deposit,
        Some(VtmIdleReasonKind::Continuous) | None => Da50WaitReason::Unknown,
    }
}

fn state_support(state: Da50State) -> SupportStatus {
    match state {
        Da50State::TravelToPark | Da50State::TravelToUnblockOther => SupportStatus::Unsupported,
        _ => SupportStatus::Measured,
    }
}

fn add_duration<K: Ord + Copy>(
    values: &mut BTreeMap<K, u64>,
    key: K,
    duration_ms: u64,
) -> Result<(), ReportBuildError> {
    let value = values.entry(key).or_default();
    *value = value.checked_add(duration_ms).ok_or(ReportBuildError::ArithmeticOverflow)?;
    Ok(())
}

fn distance_for_bucket<'a>(
    records: impl IntoIterator<Item = &'a VtmLegRecord>,
    bucket: TimeBucket,
    load: Da50TravelLoad,
) -> Result<u64, ReportBuildError> {
    records
        .into_iter()
        .filter(|record| {
            record.activity == Activity::Travel
                && record.duration_ms > 0
                && record.carrying_tote == (load == Da50TravelLoad::Laden)
        })
        .try_fold(0u64, |sum, record| {
            let leg_end_ms =
                record.start_ms.checked_add(record.duration_ms).ok_or(ReportBuildError::ArithmeticOverflow)?;
            let overlap_start_ms = record.start_ms.max(bucket.start_ms);
            let overlap_end_ms = leg_end_ms.min(bucket.end_ms);
            if overlap_start_ms >= overlap_end_ms {
                return Ok(sum);
            }
            let before_start =
                proportional_distance(record.distance_mm, overlap_start_ms - record.start_ms, record.duration_ms)?;
            let before_end =
                proportional_distance(record.distance_mm, overlap_end_ms - record.start_ms, record.duration_ms)?;
            sum.checked_add(before_end - before_start).ok_or(ReportBuildError::ArithmeticOverflow)
        })
}

fn proportional_distance(distance_mm: u64, elapsed_ms: u64, duration_ms: u64) -> Result<u64, ReportBuildError> {
    let value = (distance_mm as u128).checked_mul(elapsed_ms as u128).ok_or(ReportBuildError::ArithmeticOverflow)?
        / duration_ms as u128;
    u64::try_from(value).map_err(|_| ReportBuildError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use desgraph::RelayId;

    use super::*;
    use crate::metrics::reporting::{FixedPeriod, MetricScope};
    use crate::model::RackId;

    fn leg(
        start_ms: u64,
        duration_ms: u64,
        activity: Activity,
        from: VtmJournalEndpoint,
        direction: VtmCycleDirection,
        carrying_tote: bool,
        distance_mm: u64,
        wait_reason: Option<VtmIdleReasonKind>,
        notes: &str,
    ) -> VtmLegRecord {
        VtmLegRecord {
            start_ms,
            duration_ms,
            activity,
            from,
            to: from,
            from_zone_id: None,
            to_zone_id: None,
            distance_mm,
            vtm_id: 1,
            vtm_cycle_id: Some(1),
            tote_id: carrying_tote.then_some(2),
            presentation_id: Some(3),
            direction,
            carrying_tote,
            wait_reason,
            notes: notes.to_string(),
        }
    }

    fn state_duration(report: &MetricsReport<Da50StateData>, state: Da50State) -> u64 {
        report.data.state_rows.iter().find(|row| row.state == state).and_then(|row| row.duration_ms).unwrap()
    }

    fn wait_duration(report: &MetricsReport<Da50StateData>, reason: Da50WaitReason) -> u64 {
        report.data.wait_rows.iter().find(|row| row.reason == reason).and_then(|row| row.duration_ms).unwrap()
    }

    #[test]
    fn states_wait_reasons_and_distances_reconcile_to_fleet_time() {
        let records = vec![
            leg(10, 10, Activity::Travel, VtmJournalEndpoint::Aisle, VtmCycleDirection::Forward, false, 100, None, ""),
            leg(
                20,
                5,
                Activity::Alignment,
                VtmJournalEndpoint::Rack(RackId(1)),
                VtmCycleDirection::Forward,
                false,
                0,
                None,
                "",
            ),
            leg(
                25,
                2,
                Activity::Mechanical,
                VtmJournalEndpoint::Rack(RackId(1)),
                VtmCycleDirection::Forward,
                true,
                0,
                None,
                "fork-get @ rack",
            ),
            leg(
                27,
                20,
                Activity::Travel,
                VtmJournalEndpoint::Rack(RackId(1)),
                VtmCycleDirection::Forward,
                true,
                200,
                None,
                "",
            ),
            leg(
                47,
                5,
                Activity::Alignment,
                VtmJournalEndpoint::Relay(RelayId(2)),
                VtmCycleDirection::Forward,
                true,
                0,
                None,
                "",
            ),
            leg(
                52,
                2,
                Activity::Mechanical,
                VtmJournalEndpoint::Relay(RelayId(2)),
                VtmCycleDirection::Forward,
                true,
                0,
                None,
                "buffer-put @ relay",
            ),
            leg(
                60,
                10,
                Activity::Wait,
                VtmJournalEndpoint::Relay(RelayId(2)),
                VtmCycleDirection::Forward,
                false,
                0,
                Some(VtmIdleReasonKind::AwaitingSlot),
                "",
            ),
        ];
        let report =
            da50_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 100, 0))
                .unwrap();

        assert_eq!(state_duration(&report, Da50State::EmptyTravel), 10);
        assert_eq!(state_duration(&report, Da50State::LadenTravel), 20);
        assert_eq!(state_duration(&report, Da50State::Alignment), 10);
        assert_eq!(state_duration(&report, Da50State::Pickup), 2);
        assert_eq!(state_duration(&report, Da50State::Drop), 2);
        assert_eq!(state_duration(&report, Da50State::Waiting), 56);
        assert_eq!(wait_duration(&report, Da50WaitReason::AwaitingSlot), 10);
        assert_eq!(wait_duration(&report, Da50WaitReason::NoTask), 46);
        assert_eq!(report.data.state_rows.iter().filter_map(|row| row.duration_ms).sum::<u64>(), 100);
        assert_eq!(report.data.distance_rows[0].distance_mm, Some(100));
        assert_eq!(report.data.distance_rows[1].distance_mm, Some(200));
    }

    #[test]
    fn fixed_buckets_split_duration_and_distance_without_rounding_drift() {
        let records =
            [leg(5, 10, Activity::Travel, VtmJournalEndpoint::Aisle, VtmCycleDirection::Forward, false, 101, None, "")];
        let report = da50_state_report(
            &records,
            1,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Fixed(FixedPeriod::new(10, 0).unwrap()), 20, 0),
        )
        .unwrap();

        let empty_distances: Vec<_> = report
            .data
            .distance_rows
            .iter()
            .filter(|row| row.load == Da50TravelLoad::Empty)
            .map(|row| row.distance_mm.unwrap())
            .collect();
        assert_eq!(empty_distances, [50, 51]);
        assert_eq!(empty_distances.iter().sum::<u64>(), 101);
        let empty_travel: Vec<_> = report
            .data
            .state_rows
            .iter()
            .filter(|row| row.state == Da50State::EmptyTravel)
            .map(|row| row.duration_ms.unwrap())
            .collect();
        assert_eq!(empty_travel, [5, 5]);
    }

    #[test]
    fn conflicting_overlap_is_unknown_instead_of_double_counted() {
        let records = [
            leg(0, 10, Activity::Travel, VtmJournalEndpoint::Aisle, VtmCycleDirection::Forward, false, 10, None, ""),
            leg(
                5,
                10,
                Activity::Alignment,
                VtmJournalEndpoint::Rack(RackId(1)),
                VtmCycleDirection::Forward,
                false,
                0,
                None,
                "",
            ),
        ];
        let report =
            da50_state_report(&records, 1, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 15, 0))
                .unwrap();

        assert_eq!(state_duration(&report, Da50State::EmptyTravel), 5);
        assert_eq!(state_duration(&report, Da50State::Alignment), 5);
        assert_eq!(state_duration(&report, Da50State::Unknown), 5);
    }

    #[test]
    fn taxonomy_arrays_are_exhaustive() {
        // A new enum variant must break these matches, forcing the canonical
        // ordering arrays to be updated with it.
        for state in DA50_STATES {
            match state {
                Da50State::Pickup
                | Da50State::Drop
                | Da50State::Alignment
                | Da50State::EmptyTravel
                | Da50State::LadenTravel
                | Da50State::Waiting
                | Da50State::TravelToPark
                | Da50State::TravelToUnblockOther
                | Da50State::Unknown => {}
            }
        }
        for reason in DA50_WAIT_REASONS {
            match reason {
                Da50WaitReason::AwaitingDemand
                | Da50WaitReason::AwaitingTote
                | Da50WaitReason::AwaitingSlot
                | Da50WaitReason::AwaitingD30Deposit
                | Da50WaitReason::NoTask
                | Da50WaitReason::Unknown
                | Da50WaitReason::ConflictingFacts => {}
            }
        }
        for load in DA50_LOAD_STATES {
            match load {
                Da50TravelLoad::Empty | Da50TravelLoad::Laden => {}
            }
        }
    }

    #[test]
    fn unmodeled_states_are_unsupported_never_fabricated_zeros() {
        // No records: modeled states report measured (truthful) zeros, while
        // states the simulator does not model stay Unsupported with no value
        // — support derives from what the model produces, not from any
        // profile version number.
        let report =
            da50_state_report(&[], 2, ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10, 0))
                .unwrap();

        assert!(report.data.state_rows.iter().any(|row| {
            row.state == Da50State::Pickup && row.status == SupportStatus::Measured && row.duration_ms == Some(0)
        }));
        assert!(report.data.state_rows.iter().any(|row| {
            row.state == Da50State::TravelToPark
                && row.status == SupportStatus::Unsupported
                && row.duration_ms.is_none()
        }));
        assert!(report.data.vehicle_state_rows.is_empty());
    }

    #[test]
    fn per_vehicle_rows_reconcile_to_fleet_rows() {
        let first =
            leg(0, 10, Activity::Travel, VtmJournalEndpoint::Aisle, VtmCycleDirection::Forward, false, 100, None, "");
        let mut second = first.clone();
        second.vtm_id = 2;
        second.carrying_tote = true;
        second.tote_id = Some(7);
        second.distance_mm = 250;
        // Fleet of 3 with 2 observed: fleet rows carry one unobserved
        // vehicle's Waiting/NoTask backfill on top of the vehicle sums.
        let report = da50_state_report(
            &[first, second],
            3,
            ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 20, 0),
        )
        .unwrap();

        let unobserved_backfill_ms = 20;
        assert_eq!(
            report.data.vehicle_state_rows.len(),
            2 * DA50_STATES.len(),
            "one row per observed vehicle per state"
        );
        for state in DA50_STATES {
            let fleet = report.data.state_rows.iter().find(|row| row.state == state).unwrap();
            let vehicle_sum: u64 = report
                .data
                .vehicle_state_rows
                .iter()
                .filter(|row| row.state == state)
                .filter_map(|row| row.duration_ms)
                .sum();
            let backfill = if state == Da50State::Waiting { unobserved_backfill_ms } else { 0 };
            match fleet.status {
                SupportStatus::Measured => {
                    assert_eq!(fleet.duration_ms, Some(vehicle_sum + backfill))
                }
                _ => assert_eq!(vehicle_sum, 0, "unmeasured states carry no vehicle time"),
            }
        }
        for reason in DA50_WAIT_REASONS {
            let fleet = wait_duration(&report, reason);
            let vehicle_sum: u64 = report
                .data
                .vehicle_wait_rows
                .iter()
                .filter(|row| row.reason == reason)
                .filter_map(|row| row.duration_ms)
                .sum();
            let backfill = if reason == Da50WaitReason::NoTask { unobserved_backfill_ms } else { 0 };
            assert_eq!(fleet, vehicle_sum + backfill);
        }
        for load in DA50_LOAD_STATES {
            let fleet =
                report.data.distance_rows.iter().find(|row| row.load == load).and_then(|row| row.distance_mm).unwrap();
            let vehicle_sum: u64 = report
                .data
                .vehicle_distance_rows
                .iter()
                .filter(|row| row.load == load)
                .filter_map(|row| row.distance_mm)
                .sum();
            assert_eq!(fleet, vehicle_sum, "unobserved vehicles travel nowhere");
        }
    }
}
