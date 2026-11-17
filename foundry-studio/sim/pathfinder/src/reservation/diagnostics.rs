//! Per-cell duty / occupancy stats for the end-of-run report.
//!
//! `ReservationTable::cell_duty_stats` (in `table.rs`) is the
//! production producer: it combines the per-cell drop-aggregate (see
//! `accumulator.rs`) with a walk of the still-live list.
//! `cell_duty_stats_from_history` here is a `#[cfg(test)]` golden-test
//! helper that recomputes the same scalars from a `Vec<CellReservation>`
//! for cross-checking.

use desgraph::CellId;

#[cfg(test)]
use super::cell_reservation::CellReservation;
use super::oracle::ReservationOracle;

#[derive(Clone, Debug, Default)]
pub struct CellDutyStats {
    pub reservations: usize,
    pub open_ended: usize,
    pub first_start_ms: Option<u64>,
    pub frontier_ms: u64,
    pub busy_ms: u64,
    pub gap_ms: u64,
    pub gap_count: usize,
    pub longest_gap_ms: u64,
    pub longest_hold_ms: u64,
    /// Welford stddev of inter-start intervals (ms). 0.0 when fewer than
    /// two intervals have been observed.
    pub start_interval_stddev_ms: f64,
}

impl CellDutyStats {
    pub fn duty_pct(&self) -> f64 {
        let Some(first) = self.first_start_ms else { return 0.0 };
        let span = self.frontier_ms.saturating_sub(first);
        if span == 0 { 0.0 } else { (self.busy_ms as f64 / span as f64) * 100.0 }
    }
}

pub fn report_horizon_ms(oracle: &dyn ReservationOracle, cells: impl IntoIterator<Item = CellId>) -> u64 {
    let mut horizon = 0u64;
    for cell in cells {
        for reservation in oracle.reservations_for_cell(cell) {
            let window = reservation.window();
            horizon = horizon.max(window.start().as_ms());
            if let Some(end) = window.end() {
                horizon = horizon.max(end.as_ms());
            }
        }
    }
    horizon
}

#[cfg(test)]
pub fn cell_duty_stats_from_history(reservations: Vec<CellReservation>, horizon_ms: u64) -> CellDutyStats {
    let mut intervals: Vec<_> = reservations
        .iter()
        .map(|reservation| {
            let window = reservation.window();
            (window.start().as_ms(), window.end().map(|end| end.as_ms()).unwrap_or(horizon_ms))
        })
        .collect();
    intervals.sort_unstable();

    let mut first_start_ms = None;
    let mut frontier_ms = 0u64;
    let mut busy_ms = 0u64;
    let mut gap_ms = 0u64;
    let mut gap_count = 0usize;
    let mut longest_gap_ms = 0u64;
    let mut longest_hold_ms = 0u64;
    let mut current_start = None::<u64>;
    let mut current_end = 0u64;

    for (start, end) in intervals {
        first_start_ms = first_start_ms.or(Some(start));
        frontier_ms = frontier_ms.max(end);
        longest_hold_ms = longest_hold_ms.max(end.saturating_sub(start));

        if current_start.is_none() {
            current_start = Some(start);
            current_end = end;
            continue;
        }
        if start > current_end {
            busy_ms += current_end.saturating_sub(current_start.expect("set in prior iteration"));
            let gap = start - current_end;
            gap_ms += gap;
            gap_count += 1;
            longest_gap_ms = longest_gap_ms.max(gap);
            current_start = Some(start);
            current_end = end;
        } else {
            current_end = current_end.max(end);
        }
    }
    if let Some(start) = current_start {
        busy_ms += current_end.saturating_sub(start);
    }

    // Inter-start interval stddev — sorted starts, consecutive deltas.
    let mut starts: Vec<u64> = reservations.iter().map(|r| r.window().start().as_ms()).collect();
    starts.sort_unstable();
    let intervals_between: Vec<u64> = starts.windows(2).map(|w| w[1].saturating_sub(w[0])).collect();
    let start_interval_stddev_ms = stddev_u64(&intervals_between);

    CellDutyStats {
        reservations: reservations.len(),
        open_ended: reservations.iter().filter(|reservation| reservation.is_open_ended()).count(),
        first_start_ms,
        frontier_ms,
        busy_ms,
        gap_ms,
        gap_count,
        longest_gap_ms,
        longest_hold_ms,
        start_interval_stddev_ms,
    }
}

#[cfg(test)]
fn stddev_u64(values: &[u64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<u64>() as f64 / n;
    let variance = values.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0);
    variance.sqrt()
}

#[cfg(test)]
mod tests {
    use super::super::accumulator::CellDutyAggregate;
    use super::*;

    /// Seed a drop-aggregate by ingesting a sequence of finalized
    /// intervals in start-order — what `prune_finalized_below_watermark`
    /// would do when dropping these entries.
    fn seed(starts_ends: &[(u64, u64)]) -> CellDutyAggregate {
        let mut aggregate = CellDutyAggregate::default();
        for &(start_ms, end_ms) in starts_ends {
            aggregate.ingest_finalized(start_ms, end_ms);
        }
        aggregate
    }

    #[test]
    fn append_path_matches_history_scan_via_aggregate() {
        // No live entries — the aggregate IS the whole history.
        let starts_ends = [(10_000u64, 20_000u64), (25_000, 30_000), (60_000, 80_000)];
        let aggregate = seed(&starts_ends);

        // Compare against the history scan.
        use super::super::cell_reservation::{CellReservation, ReservationId};
        use crate::TimeRange;
        use crate::{RobotId, SimTime};
        use desgraph::CellId;
        let reservations: Vec<CellReservation> = starts_ends
            .iter()
            .enumerate()
            .map(|(idx, &(start_ms, end_ms))| {
                CellReservation::new(
                    ReservationId::new(idx as u64 + 1),
                    CellId(1),
                    RobotId(idx as u32 + 1),
                    TimeRange::new(SimTime::ms(start_ms), SimTime::ms(end_ms)),
                )
            })
            .collect();
        let history = cell_duty_stats_from_history(reservations, 100_000);

        assert_eq!(aggregate.reservations, history.reservations);
        assert_eq!(aggregate.first_start_ms, history.first_start_ms);
        assert_eq!(aggregate.frontier_ms, history.frontier_ms);
        assert_eq!(aggregate.busy_ms, history.busy_ms);
        assert_eq!(aggregate.gap_ms, history.gap_ms);
        assert_eq!(aggregate.gap_count, history.gap_count);
        assert_eq!(aggregate.longest_gap_ms, history.longest_gap_ms);
        assert_eq!(aggregate.longest_hold_ms, history.longest_hold_ms);
        assert!(
            (aggregate.interval_stddev_ms() - history.start_interval_stddev_ms).abs() < 1e-9,
            "stddev mismatch: aggregate={} history={}",
            aggregate.interval_stddev_ms(),
            history.start_interval_stddev_ms,
        );
    }

    #[test]
    fn ingest_finalized_handles_consecutive_intervals() {
        let aggregate = seed(&[(10_000, 20_000), (20_000, 30_000)]);

        assert_eq!(aggregate.first_start_ms, Some(10_000));
        assert_eq!(aggregate.gap_count, 0, "back-to-back intervals produce no gaps");
        assert_eq!(aggregate.gap_ms, 0);
        assert_eq!(aggregate.longest_gap_ms, 0);
        assert_eq!(aggregate.busy_ms, 20_000);
        assert_eq!(aggregate.frontier_ms, 30_000);
    }
}
