//! Within `reporting`, `destination_striping` answers WHERE each
//! destination's picking was PLANNED to happen versus where its pick
//! presentations ACTUALLY happened: the planned stripe-share matrix
//! (destination × pick station, from the run's `StripePlan` snapshot) and
//! the per-destination completed-presentation series — kept as separate
//! tables so no surface can mix plan and fact in one cell (the
//! planned-vs-completed rule of specs/dematic_kpi_reporting/plan.md). Owns
//! the destination-striping metric descriptors. Surface-agnostic.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use desgraph::StationId;
use serde::{Deserialize, Serialize};

use crate::events::StripePlanEvent;
use crate::observer::output::PickRecord;

use super::{
    AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, Ratio, ReportBuildError, ReportRequest, TimeBucket,
    event_in_bucket, report_buckets,
};

/// One planned cell: a destination's stripe share at one pick station.
/// Plan-derived (no time bucket) — the static v1 stripe map is constant for
/// the whole run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripePlanShareRow {
    /// Display form of the destination (the DC id for the v1
    /// `DistributionCenter` kind).
    pub destination: String,
    /// The destination's total demand units — what hand-validation checks
    /// shares against, and what distinguishes a 6.9%-of-demand destination
    /// from a 0.06% one when both render a single "100%" cell.
    pub destination_units: u64,
    pub station_id: StationId,
    /// This station's overlap with the destination's interval / the whole
    /// interval, in exact unit·N arithmetic — shares sum to 1 per
    /// destination. Allocation semantics are proportional to these shares
    /// (D13): any majority-station marker is derived by surfaces, not
    /// carried here.
    pub share_of_destination: Ratio,
}

/// Distinct completed pick presentations for one destination in one bucket
/// — the fact-side series the planned matrix reconciles against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationPresentationRow {
    pub bucket: TimeBucket,
    pub destination: String,
    pub presentations: u64,
}

/// Facts that could not be attributed to a planned destination, retained
/// explicitly rather than silently dropped (same honesty convention as
/// `completed_work`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripingUnattributedRow {
    pub bucket: TimeBucket,
    /// Distinct presentations with no destination at all (pre-formed-order
    /// picks carry no DC).
    pub destination_unattributed_presentations: u64,
    /// Pick lines with no presentation id; counted as raw lines because
    /// they cannot be deduplicated into presentations.
    pub unidentified_pick_lines: u64,
    /// Distinct presentations whose destination is absent from the stripe
    /// plan — a plan-vs-fact drift tripwire. Always 0 when the run carried
    /// no plan.
    pub unplanned_destination_presentations: u64,
}

/// One stripe-plan snapshot: the plan that took effect at `from_ms` and
/// governs until the next snapshot (or run end). The stripe state is a
/// step-function time series sampled at change events — exactly one t=0
/// snapshot under the static v1 map, more once dynamic restriping lands
/// (Daniel, 2026-07-30: series from day one, so restriping changes the
/// data, not the interface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripePlanSnapshot {
    /// Sim time this plan took effect.
    pub from_ms: u64,
    /// Pick stations in planned physical order — the column order for the
    /// planned matrix.
    pub stations: Vec<StationId>,
    /// Total demand units across the plan — the denominator for a
    /// destination's share of overall demand.
    pub total_units: u64,
    /// Every planned destination in plan (first-appearance) order with its
    /// units — the row axis. Separate from `rows` so a zero-volume
    /// destination (no share cells) still appears as an empty row instead
    /// of vanishing (Codex review, 2026-07-30).
    pub destinations: Vec<PlannedDestinationRow>,
    pub rows: Vec<StripePlanShareRow>,
}

/// One planned destination: the row-axis entry of the planned matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedDestinationRow {
    pub destination: String,
    pub units: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationStripingData {
    /// Stripe-plan snapshots ordered by `from_ms`. Empty = the run carried
    /// no stripe plan (pre-formed scenario, or a profile predating stripe
    /// capture).
    pub plan_snapshots: Vec<StripePlanSnapshot>,
    pub presentation_rows: Vec<DestinationPresentationRow>,
    pub unattributed_rows: Vec<StripingUnattributedRow>,
}

/// The metric descriptors this producer owns and emits with every report.
/// Callable without data, so surfaces can enumerate destination-striping
/// metrics before a run exists. The planned matrix is the run's
/// configuration made visible, not a measurement, so it carries no
/// descriptor.
pub fn destination_striping_descriptors() -> Vec<MetricDescriptor> {
    vec![MetricDescriptor::new(
        "destination_striping.presentations",
        "Destination pick presentations",
        "Distinct completed pick presentations attributed to their destination (forward DC in v1).",
        MetricUnit::Count,
        None,
        AggregationClass::DistinctCount,
    )]
}

/// Build the destination-striping report: the stripe-plan snapshot series
/// (verbatim from the run's `StripePlan` emissions, one snapshot per
/// (re)computation) plus the bucketed per-destination presentation series
/// from pick facts.
///
/// Presentations are distinct-counted by presentation id, exactly like
/// `completed_work` — SKU quantity and repeated lines within one
/// presentation never inflate the count. Per bucket the output reconciles:
/// identifiable presentations = Σ per-destination `presentations`
/// + `destination_unattributed_presentations`.
pub fn destination_striping_report(
    stripe_plans: &[StripePlanEvent],
    picks: &[PickRecord],
    request: ReportRequest,
) -> Result<MetricsReport<DestinationStripingData>, ReportBuildError> {
    let descriptors = destination_striping_descriptors();
    let buckets = report_buckets(request)?;

    // Downstream selection (`rposition` by from_ms, `.last()` = current
    // plan) depends on emission-time order; the observer appends in event
    // order, but make the invariant local rather than assumed.
    let mut plan_snapshots: Vec<StripePlanSnapshot> = stripe_plans
        .iter()
        .map(|plan| {
            let mut rows = Vec::new();
            for entry in &plan.entries {
                // The overlaps are numerators over the destination's whole
                // interval in the same unit·N space, so their sum IS the
                // denominator — shares sum to exactly 1 per destination.
                let denominator: u64 = entry.eligible.iter().map(|&(_, overlap)| overlap).sum();
                for &(station_id, overlap) in &entry.eligible {
                    rows.push(StripePlanShareRow {
                        // v1 `Destination` Display is the DC id, matching
                        // the `dc_id` strings on pick facts.
                        destination: entry.destination.to_string(),
                        destination_units: entry.units,
                        station_id,
                        share_of_destination: Ratio::new(overlap, denominator),
                    });
                }
            }
            StripePlanSnapshot {
                from_ms: plan.sim_time_ms,
                stations: plan.stations.clone(),
                total_units: plan.total_units,
                destinations: plan
                    .entries
                    .iter()
                    .map(|entry| PlannedDestinationRow {
                        destination: entry.destination.to_string(),
                        units: entry.units,
                    })
                    .collect(),
                rows,
            }
        })
        .collect();
    plan_snapshots.sort_by_key(|snapshot| snapshot.from_ms);
    // The row axis unions every snapshot (a destination planned at ANY
    // time deserves its series row)…
    let ever_planned: BTreeSet<String> =
        stripe_plans.iter().flat_map(|plan| plan.entries.iter().map(|e| e.destination.to_string())).collect();
    // …but the drift tripwire is time-aware: a pick is unplanned iff its
    // destination is absent from the snapshot GOVERNING its instant —
    // membership in a future snapshot does not retroactively plan an
    // earlier pick (Codex review, 2026-07-30).
    let snapshot_starts: Vec<u64> = plan_snapshots.iter().map(|s| s.from_ms).collect();
    let planned_sets: Vec<BTreeSet<&str>> =
        plan_snapshots.iter().map(|s| s.destinations.iter().map(|d| d.destination.as_str()).collect()).collect();
    let planned_at = |t_ms: u64| -> Option<&BTreeSet<&str>> {
        let index = snapshot_starts.iter().rposition(|&from| from <= t_ms).unwrap_or(0);
        planned_sets.get(index)
    };

    // The series axis: planned destinations in first-snapshot-appearance
    // order, then any observed-only destinations appended in sorted order.
    // Every (bucket × destination) cell is emitted — a bucket with no
    // presentations is a measured zero, not a missing row, so a chart never
    // interpolates across it (Opus review, 2026-07-30).
    let mut series_destinations: Vec<String> = Vec::new();
    for plan in stripe_plans {
        for entry in &plan.entries {
            let name = entry.destination.to_string();
            if !series_destinations.contains(&name) {
                series_destinations.push(name);
            }
        }
    }
    let observed_only: BTreeSet<&str> = picks
        .iter()
        .filter_map(|record| record.dc_id.as_deref())
        .filter(|dc_id| !ever_planned.contains(*dc_id))
        .collect();
    series_destinations.extend(observed_only.iter().map(|dc_id| dc_id.to_string()));

    let mut presentation_rows = Vec::new();
    let mut unattributed_rows = Vec::new();
    for bucket in buckets {
        let mut cells: BTreeMap<&str, HashSet<u64>> = BTreeMap::new();
        let mut unattributed: HashSet<u64> = HashSet::new();
        let mut unplanned: HashSet<u64> = HashSet::new();
        let mut unidentified_pick_lines = 0u64;
        for record in picks.iter().filter(|record| event_in_bucket(record.sim_time_ms, bucket, request)) {
            let Some(presentation_id) = record.presentation_id else {
                unidentified_pick_lines += 1;
                continue;
            };
            match record.dc_id.as_deref() {
                None => {
                    unattributed.insert(presentation_id);
                }
                Some(dc_id) => {
                    cells.entry(dc_id).or_default().insert(presentation_id);
                    if planned_at(record.sim_time_ms).is_some_and(|planned| !planned.contains(dc_id)) {
                        unplanned.insert(presentation_id);
                    }
                }
            }
        }
        for destination in &series_destinations {
            presentation_rows.push(DestinationPresentationRow {
                bucket,
                destination: destination.clone(),
                presentations: cells.get(destination.as_str()).map_or(0, |ids| ids.len() as u64),
            });
        }
        unattributed_rows.push(StripingUnattributedRow {
            bucket,
            destination_unattributed_presentations: unattributed.len() as u64,
            unidentified_pick_lines,
            unplanned_destination_presentations: unplanned.len() as u64,
        });
    }

    Ok(MetricsReport::new(
        request,
        descriptors,
        DestinationStripingData { plan_snapshots, presentation_rows, unattributed_rows },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::StripePlanEntry;
    use crate::metrics::reporting::{FixedPeriod, MetricScope, ReportPeriod};
    use crate::model::{Destination, SkuId};

    fn plan() -> StripePlanEvent {
        // Two stations; A owns s1 outright, B straddles: 3 of its 4 units'
        // worth of overlap at s2, 1 at s1 (unit·N numerators).
        StripePlanEvent {
            sim_time_ms: 0,
            stations: vec![StationId(1), StationId(2)],
            total_units: 8,
            entries: vec![
                StripePlanEntry {
                    destination: Destination::DistributionCenter("A".into()),
                    units: 4,
                    eligible: vec![(StationId(1), 8)],
                },
                StripePlanEntry {
                    destination: Destination::DistributionCenter("B".into()),
                    units: 4,
                    eligible: vec![(StationId(1), 2), (StationId(2), 6)],
                },
            ],
        }
    }

    fn pick(t_ms: u64, dc: Option<&str>, presentation: Option<u64>) -> PickRecord {
        PickRecord {
            sim_time_ms: t_ms,
            station_id: StationId(1),
            sku: SkuId(7),
            quantity: 1,
            dc_id: dc.map(str::to_string),
            order_id: None,
            tote_id: None,
            source_zone_id: None,
            source_relay_id: None,
            presentation_id: presentation,
        }
    }

    fn overall_request() -> ReportRequest {
        ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10_000, 0)
    }

    #[test]
    fn plan_shares_sum_to_one_per_destination() {
        let report = destination_striping_report(&[plan()], &[], overall_request()).unwrap();
        assert_eq!(report.data.plan_snapshots.len(), 1);
        let snapshot = &report.data.plan_snapshots[0];
        assert_eq!(snapshot.from_ms, 0);
        for dest in ["A", "B"] {
            let dest_rows: Vec<_> = snapshot.rows.iter().filter(|r| r.destination == dest).collect();
            let num: u64 = dest_rows.iter().map(|r| r.share_of_destination.numerator).sum();
            let denom = dest_rows[0].share_of_destination.denominator;
            assert_eq!(num, denom, "{dest} shares must sum to 1");
        }
        assert_eq!(snapshot.stations, vec![StationId(1), StationId(2)]);
        assert_eq!(snapshot.total_units, 8);
        let destinations: Vec<(&str, u64)> =
            snapshot.destinations.iter().map(|d| (d.destination.as_str(), d.units)).collect();
        assert_eq!(destinations, [("A", 4), ("B", 4)]);
    }

    #[test]
    fn zero_volume_destination_stays_visible_as_a_row() {
        // A zero-volume destination has no share cells but must still hold
        // its row in the planned matrix (Codex review).
        let mut plan = plan();
        plan.entries.insert(
            1,
            StripePlanEntry { destination: Destination::DistributionCenter("Z".into()), units: 0, eligible: vec![] },
        );
        let report = destination_striping_report(&[plan], &[], overall_request()).unwrap();
        let snapshot = &report.data.plan_snapshots[0];
        let names: Vec<&str> = snapshot.destinations.iter().map(|d| d.destination.as_str()).collect();
        assert_eq!(names, ["A", "Z", "B"]);
        assert!(!snapshot.rows.iter().any(|row| row.destination == "Z"), "Z has no share cells");
    }

    #[test]
    fn restripe_snapshots_form_an_ordered_series() {
        // The stripe state is a step function: a second emission (as
        // dynamic restriping will produce) appends a snapshot, and a
        // destination planned in ANY snapshot is not "unplanned".
        let mut later = plan();
        later.sim_time_ms = 5_000;
        later.entries.push(StripePlanEntry {
            destination: Destination::DistributionCenter("C".into()),
            units: 2,
            eligible: vec![(StationId(2), 4)],
        });
        let picks = vec![pick(6_000, Some("C"), Some(41))];
        let report = destination_striping_report(&[plan(), later], &picks, overall_request()).unwrap();
        assert_eq!(report.data.plan_snapshots.len(), 2);
        assert_eq!(report.data.plan_snapshots[0].from_ms, 0);
        assert_eq!(report.data.plan_snapshots[1].from_ms, 5_000);
        // C exists only in the later snapshot; picking it AFTER that
        // snapshot takes effect is not a plan-vs-fact violation.
        assert_eq!(report.data.unattributed_rows[0].unplanned_destination_presentations, 0);
        let c = report.data.presentation_rows.iter().find(|row| row.destination == "C").unwrap();
        assert_eq!(c.presentations, 1);
    }

    #[test]
    fn pick_before_its_destination_enters_the_plan_is_unplanned() {
        // "Planned" is time-aware: C joins the plan at t=5s, but the pick
        // happens at t=2s under the original plan — that IS drift, and a
        // whole-run union would hide it (Codex review).
        let mut later = plan();
        later.sim_time_ms = 5_000;
        later.entries.push(StripePlanEntry {
            destination: Destination::DistributionCenter("C".into()),
            units: 2,
            eligible: vec![(StationId(2), 4)],
        });
        let picks = vec![pick(2_000, Some("C"), Some(41))];
        let report = destination_striping_report(&[plan(), later], &picks, overall_request()).unwrap();
        assert_eq!(report.data.unattributed_rows[0].unplanned_destination_presentations, 1);
    }

    #[test]
    fn presentations_are_distinct_counted_not_line_counted() {
        // Three lines, two distinct presentations, one destination. The
        // series is dense: planned-but-idle B gets a measured-zero row.
        let picks =
            vec![pick(1_000, Some("A"), Some(41)), pick(1_100, Some("A"), Some(41)), pick(2_000, Some("A"), Some(42))];
        let report = destination_striping_report(&[plan()], &picks, overall_request()).unwrap();
        let rows = &report.data.presentation_rows;
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].destination.as_str(), rows[0].presentations), ("A", 2));
        assert_eq!((rows[1].destination.as_str(), rows[1].presentations), ("B", 0));
    }

    #[test]
    fn buckets_are_dense_with_measured_zeros() {
        // Hourly buckets over 2h; A picks only in hour 1. Hour 2 must carry
        // explicit zero rows for every destination — a chart that only got
        // hour-1 rows would interpolate across the silence.
        let request = ReportRequest::for_run(
            MetricScope::FullRun,
            ReportPeriod::Fixed(FixedPeriod::new(3_600_000, 0).unwrap()),
            7_200_000,
            0,
        );
        let picks = vec![pick(600_000, Some("A"), Some(41))];
        let report = destination_striping_report(&[plan()], &picks, request).unwrap();
        let rows = &report.data.presentation_rows;
        // 2 buckets × 2 planned destinations, in (bucket, plan-order) order.
        assert_eq!(rows.len(), 4);
        let counts: Vec<(&str, u64)> = rows.iter().map(|row| (row.destination.as_str(), row.presentations)).collect();
        assert_eq!(counts, [("A", 1), ("B", 0), ("A", 0), ("B", 0)]);
        assert!(rows[2].bucket.start_ms >= 3_600_000, "second bucket rows follow the first");
    }

    #[test]
    fn unattributed_unidentified_and_unplanned_are_counted_never_dropped() {
        let picks = vec![
            pick(1_000, None, Some(41)),          // no destination
            pick(1_500, Some("A"), None),         // no presentation id
            pick(2_000, Some("GHOST"), Some(43)), // destination absent from plan
        ];
        let report = destination_striping_report(&[plan()], &picks, overall_request()).unwrap();
        let u = &report.data.unattributed_rows[0];
        assert_eq!(u.destination_unattributed_presentations, 1);
        assert_eq!(u.unidentified_pick_lines, 1);
        assert_eq!(u.unplanned_destination_presentations, 1);
        // The unplanned presentation still appears in the series, appended
        // after the planned destinations — it is a real fact; the tripwire
        // only flags the plan mismatch.
        let ghost = report.data.presentation_rows.iter().find(|row| row.destination == "GHOST").unwrap();
        assert_eq!(ghost.presentations, 1);
    }

    #[test]
    fn no_plan_means_empty_snapshots_and_no_unplanned_tripwire() {
        let picks = vec![pick(1_000, Some("A"), Some(41))];
        let report = destination_striping_report(&[], &picks, overall_request()).unwrap();
        assert!(report.data.plan_snapshots.is_empty());
        assert_eq!(report.data.presentation_rows[0].presentations, 1);
        assert_eq!(report.data.unattributed_rows[0].unplanned_destination_presentations, 0);
    }
}
