//! Within `reporting`, `distribution` answers WHERE completed work
//! happened: the attribution matrices — aisle x station tote counts,
//! DC x station tote and distinct-order counts, with explicit unattributed
//! remainders — built from presentation and order-completion facts. Owns
//! the completed-work metric descriptors. Surface-agnostic.
//!
//! TODO(module): the filename undersells the charter — `completed_work.rs`
//! says what this is; rename with the next slice that touches it.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use desgraph::StationId;
use serde::{Deserialize, Serialize};

use crate::model::OrderId;
use crate::observer::output::{CompletedOrderRecord, PickRecord};

use super::{
    AggregationClass, MetricDescriptor, MetricUnit, MetricsReport, Ratio, ReportBuildError, ReportPeriod,
    ReportRequest, TimeBucket, event_in_bucket, report_buckets,
};

/// Completed tote presentations from one source aisle to one pick station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AisleStationWorkRow {
    pub bucket: TimeBucket,
    pub zone_id: u32,
    pub station_id: StationId,
    pub tote_count: u64,
    /// This aisle's totes / all aisle-attributed totes at the station, so
    /// shares sum to 1 within the matrix. Aisle-unattributed presentations
    /// are outside the denominator (see [`UnattributedWorkRow`]).
    pub aisle_share_of_station: Ratio,
}

/// Completed tote and order work from one DC at one pick station.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DcStationWorkRow {
    pub bucket: TimeBucket,
    pub dc_id: String,
    pub station_id: StationId,
    pub tote_count: u64,
    /// This station's totes / the DC's attributed totes across stations, so
    /// shares sum to 1 within the matrix (see [`UnattributedWorkRow`]).
    pub tote_station_share_of_dc: Ratio,
    pub order_count: u64,
    /// This station's orders / the DC's attributed orders across stations,
    /// same denominator convention as the tote share.
    pub order_station_share_of_dc: Ratio,
}

/// Facts that could not be placed in a matrix are retained explicitly rather
/// than silently counted as zero or assigned to a fabricated aisle/DC.
///
/// A presentation can be attributable in one matrix and unattributed in the
/// other (e.g. a pre-formed-order pick has a source aisle but no DC); each
/// counter here is scoped to the matrix it reconciles against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnattributedWorkRow {
    pub bucket: TimeBucket,
    /// Distinct presentations with no source aisle. Excluded from the
    /// aisle × station matrix; may still appear in the DC matrix.
    pub aisle_unattributed_presentations: u64,
    /// Distinct presentations with no DC. Excluded from the DC × station
    /// matrix; may still appear in the aisle matrix.
    pub dc_unattributed_presentations: u64,
    /// Pick lines with no presentation ID at all. They cannot be
    /// deduplicated into presentations, so they are counted as raw lines
    /// (not presentations) and appear in no matrix.
    pub unidentified_pick_lines: u64,
    /// Distinct completed orders missing a station or DC; excluded from the
    /// DC × station matrix.
    pub unattributed_orders: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedWorkData {
    pub aisle_station_rows: Vec<AisleStationWorkRow>,
    pub dc_station_rows: Vec<DcStationWorkRow>,
    pub unattributed_rows: Vec<UnattributedWorkRow>,
}

/// The metric descriptors this producer owns and emits with every report.
/// Callable without data, so surfaces can enumerate completed-work metrics
/// before a run exists.
pub fn completed_work_descriptors() -> Vec<MetricDescriptor> {
    vec![
        MetricDescriptor::new(
            "completed_work.tote_presentations",
            "Completed tote presentations",
            "Completed tote presentations attributed by source aisle and pick station.",
            MetricUnit::Count,
            None,
            AggregationClass::DistinctCount,
        ),
        MetricDescriptor::new(
            "completed_work.orders",
            "Completed orders",
            "Distinct completed orders attributed by DC and pick station.",
            MetricUnit::Count,
            None,
            AggregationClass::DistinctCount,
        ),
    ]
}

/// Build completed-work matrices from presentation and order-completion facts.
///
/// SKU quantity never affects `tote_count`: one distinct presentation ID is
/// one tote presentation. Re-presenting the same tote under a new presentation
/// ID counts again. Orders are distinct-counted by order ID.
///
/// Each matrix qualifies facts only on the dimensions it actually uses, so a
/// missing DC never discards an aisle attribution (or vice versa). Per bucket
/// the output reconciles exactly:
///
/// - identifiable presentations = Σ aisle-matrix `tote_count`
///   + `aisle_unattributed_presentations`
/// - identifiable presentations = Σ DC-matrix `tote_count`
///   + `dc_unattributed_presentations`
/// - distinct completed orders = Σ DC-matrix `order_count`
///   + `unattributed_orders`
pub fn completed_work_report(
    picks: &[PickRecord],
    completed_orders: &[CompletedOrderRecord],
    request: ReportRequest,
) -> Result<MetricsReport<CompletedWorkData>, ReportBuildError> {
    let descriptors = completed_work_descriptors();
    let buckets = report_buckets(request)?;
    let zones: Vec<_> =
        picks.iter().filter_map(|record| record.source_zone_id).collect::<BTreeSet<_>>().into_iter().collect();
    let stations: Vec<_> = picks
        .iter()
        .map(|record| record.station_id)
        .chain(completed_orders.iter().filter_map(|record| record.station_id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let dcs: Vec<_> = picks
        .iter()
        .filter_map(|record| record.dc_id.clone())
        .chain(completed_orders.iter().filter_map(|record| record.dc_id.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut aisle_station_rows = Vec::new();
    let mut dc_station_rows = Vec::new();
    let mut unattributed_rows = Vec::new();
    for bucket in buckets {
        let pick_in_bucket = |record: &PickRecord| event_in_bucket(record.sim_time_ms, bucket, request);
        let order_in_bucket = |record: &CompletedOrderRecord| event_in_bucket(record.sim_time_ms, bucket, request);

        // One pass over the pick facts, placing every identifiable
        // presentation in each matrix independently.
        let mut aisle_cells: BTreeMap<(u32, StationId), HashSet<u64>> = BTreeMap::new();
        let mut dc_tote_cells: BTreeMap<(&str, StationId), HashSet<u64>> = BTreeMap::new();
        let mut aisle_unattributed = HashSet::new();
        let mut dc_unattributed = HashSet::new();
        let mut unidentified_pick_lines = 0u64;
        for record in picks.iter().filter(|record| pick_in_bucket(record)) {
            let Some(presentation_id) = record.presentation_id else {
                unidentified_pick_lines += 1;
                continue;
            };
            match record.source_zone_id {
                Some(zone_id) => {
                    aisle_cells.entry((zone_id, record.station_id)).or_default().insert(presentation_id);
                }
                None => {
                    aisle_unattributed.insert(presentation_id);
                }
            }
            match record.dc_id.as_deref() {
                Some(dc_id) => {
                    dc_tote_cells.entry((dc_id, record.station_id)).or_default().insert(presentation_id);
                }
                None => {
                    dc_unattributed.insert(presentation_id);
                }
            }
        }

        let mut dc_order_cells: BTreeMap<(&str, StationId), HashSet<OrderId>> = BTreeMap::new();
        let mut unattributed_order_ids = HashSet::new();
        for record in completed_orders.iter().filter(|record| order_in_bucket(record)) {
            match (record.dc_id.as_deref(), record.station_id) {
                (Some(dc_id), Some(station_id)) => {
                    dc_order_cells.entry((dc_id, station_id)).or_default().insert(record.order_id);
                }
                _ => {
                    unattributed_order_ids.insert(record.order_id);
                }
            }
        }

        // Cells partition their matrix (one presentation has one station and
        // at most one zone/DC), so per-station and per-DC totals are sums of
        // disjoint cell counts.
        let mut station_totals: BTreeMap<StationId, u64> = BTreeMap::new();
        for ((_, station_id), presentations) in &aisle_cells {
            *station_totals.entry(*station_id).or_default() += presentations.len() as u64;
        }
        let mut dc_tote_totals: BTreeMap<&str, u64> = BTreeMap::new();
        for ((dc_id, _), presentations) in &dc_tote_cells {
            *dc_tote_totals.entry(dc_id).or_default() += presentations.len() as u64;
        }
        let mut dc_order_totals: BTreeMap<&str, u64> = BTreeMap::new();
        for ((dc_id, _), order_ids) in &dc_order_cells {
            *dc_order_totals.entry(dc_id).or_default() += order_ids.len() as u64;
        }

        for &zone_id in &zones {
            for &station_id in &stations {
                let tote_count =
                    aisle_cells.get(&(zone_id, station_id)).map_or(0, |presentations| presentations.len() as u64);
                let station_total = station_totals.get(&station_id).copied().unwrap_or(0);
                aisle_station_rows.push(AisleStationWorkRow {
                    bucket,
                    zone_id,
                    station_id,
                    tote_count,
                    aisle_share_of_station: Ratio::new(tote_count, station_total),
                });
            }
        }

        for dc_id in &dcs {
            let dc_tote_total = dc_tote_totals.get(dc_id.as_str()).copied().unwrap_or(0);
            let dc_order_total = dc_order_totals.get(dc_id.as_str()).copied().unwrap_or(0);
            for &station_id in &stations {
                let tote_count = dc_tote_cells
                    .get(&(dc_id.as_str(), station_id))
                    .map_or(0, |presentations| presentations.len() as u64);
                let order_count =
                    dc_order_cells.get(&(dc_id.as_str(), station_id)).map_or(0, |order_ids| order_ids.len() as u64);
                dc_station_rows.push(DcStationWorkRow {
                    bucket,
                    dc_id: dc_id.clone(),
                    station_id,
                    tote_count,
                    tote_station_share_of_dc: Ratio::new(tote_count, dc_tote_total),
                    order_count,
                    order_station_share_of_dc: Ratio::new(order_count, dc_order_total),
                });
            }
        }

        unattributed_rows.push(UnattributedWorkRow {
            bucket,
            aisle_unattributed_presentations: aisle_unattributed.len() as u64,
            dc_unattributed_presentations: dc_unattributed.len() as u64,
            unidentified_pick_lines,
            unattributed_orders: unattributed_order_ids.len() as u64,
        });
    }

    aisle_station_rows.sort_by_key(|row| (row.bucket.end_ms, row.zone_id, row.station_id));
    dc_station_rows.sort_by(|left, right| {
        (left.bucket.end_ms, &left.dc_id, left.station_id).cmp(&(right.bucket.end_ms, &right.dc_id, right.station_id))
    });

    Ok(MetricsReport::new(
        request,
        descriptors,
        CompletedWorkData { aisle_station_rows, dc_station_rows, unattributed_rows },
    ))
}

#[cfg(test)]
mod tests {
    use desgraph::{RelayId, StationId};

    use super::*;
    use crate::metrics::reporting::{MetricScope, TrailingSpec};
    use crate::model::OrderId;
    use crate::model::ids::SkuId;

    fn pick(
        at_ms: u64,
        presentation_id: u64,
        tote_id: u32,
        zone_id: u32,
        station_id: u32,
        dc_id: &str,
        quantity: u32,
    ) -> PickRecord {
        PickRecord {
            sim_time_ms: at_ms,
            station_id: StationId(station_id),
            sku: SkuId(1),
            quantity,
            dc_id: Some(dc_id.into()),
            order_id: Some(OrderId(presentation_id as u32)),
            tote_id: Some(tote_id),
            source_zone_id: Some(zone_id),
            source_relay_id: Some(RelayId(zone_id)),
            presentation_id: Some(presentation_id),
        }
    }

    fn order(at_ms: u64, order_id: u32, station_id: u32, dc_id: &str) -> CompletedOrderRecord {
        CompletedOrderRecord {
            sim_time_ms: at_ms,
            order_id: OrderId(order_id),
            station_id: Some(StationId(station_id)),
            dc_id: Some(dc_id.into()),
        }
    }

    #[test]
    fn counts_presentations_not_sku_quantity_and_retains_ratio_denominators() {
        let picks = [
            pick(10, 1, 10, 1, 7, "A", 50),
            pick(20, 2, 10, 1, 7, "A", 1),
            pick(30, 3, 11, 2, 7, "A", 20),
            pick(40, 4, 12, 1, 8, "A", 1),
        ];
        let orders = [order(35, 1, 7, "A"), order(45, 4, 8, "A")];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 50, 0);

        let report = completed_work_report(&picks, &orders, request).unwrap();
        let aisle = report
            .data
            .aisle_station_rows
            .iter()
            .find(|row| row.zone_id == 1 && row.station_id == StationId(7))
            .unwrap();
        assert_eq!(aisle.tote_count, 2);
        assert_eq!(aisle.aisle_share_of_station, Ratio::new(2, 3));
        let dc_station =
            report.data.dc_station_rows.iter().find(|row| row.dc_id == "A" && row.station_id == StationId(7)).unwrap();
        assert_eq!(dc_station.tote_count, 3);
        assert_eq!(dc_station.tote_station_share_of_dc, Ratio::new(3, 4));
        assert_eq!(dc_station.order_count, 1);
        assert_eq!(dc_station.order_station_share_of_dc, Ratio::new(1, 2));
    }

    #[test]
    fn matrices_reconcile_with_partially_attributed_facts() {
        let picks = [
            // Fully attributed.
            pick(10, 1, 10, 1, 7, "A", 5),
            // Pre-formed-order pick: full source context, no DC.
            PickRecord { dc_id: None, ..pick(20, 2, 11, 2, 7, "A", 1) },
            // DC known, source aisle unknown.
            PickRecord { source_zone_id: None, source_relay_id: None, ..pick(30, 3, 12, 1, 8, "B", 1) },
            // No presentation ID: a raw line, not a countable presentation.
            PickRecord { presentation_id: None, ..pick(40, 9, 13, 1, 8, "B", 1) },
        ];
        let orders = [
            order(35, 1, 7, "A"),
            // Station known, DC missing.
            CompletedOrderRecord { dc_id: None, ..order(45, 2, 7, "A") },
            // DC known, station missing.
            CompletedOrderRecord { station_id: None, ..order(50, 3, 8, "B") },
        ];
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 60, 0);

        let report = completed_work_report(&picks, &orders, request).unwrap();
        let unattributed = report.data.unattributed_rows[0];
        let aisle_total: u64 = report.data.aisle_station_rows.iter().map(|row| row.tote_count).sum();
        let dc_tote_total: u64 = report.data.dc_station_rows.iter().map(|row| row.tote_count).sum();
        let dc_order_total: u64 = report.data.dc_station_rows.iter().map(|row| row.order_count).sum();

        // Three identifiable presentations (1, 2, 3); each matrix accounts
        // for all of them, attributed or not.
        assert_eq!(aisle_total + unattributed.aisle_unattributed_presentations, 3);
        assert_eq!(dc_tote_total + unattributed.dc_unattributed_presentations, 3);
        assert_eq!(unattributed.unidentified_pick_lines, 1);
        // Three distinct orders; only order 1 carries full attribution.
        assert_eq!(dc_order_total + unattributed.unattributed_orders, 3);
        assert_eq!(dc_order_total, 1);

        // The DC-less pick keeps its aisle attribution...
        assert!(
            report
                .data
                .aisle_station_rows
                .iter()
                .any(|row| { row.zone_id == 2 && row.station_id == StationId(7) && row.tote_count == 1 })
        );
        // ...and is explicitly visible as DC-unattributed, not silently lost.
        assert_eq!(unattributed.dc_unattributed_presentations, 1);
        assert_eq!(unattributed.aisle_unattributed_presentations, 1);
    }

    #[test]
    fn trailing_windows_are_causal_and_empty_orders_measure_zero() {
        let picks = [pick(10, 1, 10, 1, 7, "A", 1), pick(20, 2, 11, 1, 7, "A", 1)];
        let request = ReportRequest::for_run(
            MetricScope::FullRun,
            ReportPeriod::Trailing(TrailingSpec::new(10, 10, 0).unwrap()),
            20,
            0,
        );

        let report = completed_work_report(&picks, &[], request).unwrap();
        let counts: Vec<_> =
            report.data.aisle_station_rows.iter().map(|row| (row.bucket.end_ms, row.tote_count)).collect();
        assert_eq!(counts, [(10, 1), (20, 1)]);

        // No completed orders: the DC matrix keeps its stable shape with
        // measured zero order counts, and the descriptors still travel.
        assert_eq!(report.descriptors.len(), 2);
        let dc_rows = &report.data.dc_station_rows;
        assert_eq!(dc_rows.len(), 2, "one row per (bucket, dc, station) cell");
        assert!(dc_rows.iter().all(|row| row.order_count == 0));
        assert!(dc_rows.iter().all(|row| row.tote_count > 0));
    }
}
