//! Per-cell drop-aggregate for deferred duty-stat computation.
//!
//! `Inner::prune_finalized_below_watermark` folds finalized intervals
//! into the per-cell aggregate as they're dropped. `Inner::cell_duty_stats`
//! combines the aggregate with a walk of the still-live list to produce
//! `CellDutyStats` at report time. Live `Vec<CellReservation>` is the
//! source of truth for everything not yet pruned; the aggregate is the
//! source of truth for everything that's been dropped.
//!
//! Compared to an eager accumulator (state maintained inline on every
//! create/finalize), this defers all stat work until either prune or
//! report time. The hot path pays nothing. Mid-list inserts and
//! re-finalizes need no special handling — the live list always
//! reflects the current state, and the aggregate only sees finalized
//! intervals at the moment they're dropped (by which time their
//! windows are stable).

/// Drop-aggregate for one cell. Holds the cumulative contribution of
/// every finalized reservation that has been pruned from the cell's
/// live list, plus the boundary state needed to compute the gap and
/// interval that straddle the prune cutoff at report time.
#[derive(Default, Clone, Debug)]
pub struct CellDutyAggregate {
    pub reservations: usize,
    pub first_start_ms: Option<u64>,
    pub frontier_ms: u64,
    pub busy_ms: u64,
    pub gap_ms: u64,
    pub gap_count: usize,
    pub longest_gap_ms: u64,
    pub longest_hold_ms: u64,
    /// End of the last finalized interval ingested. Used to compute the
    /// gap from the last-dropped entry to the first live entry at
    /// report time.
    pub last_end_ms: Option<u64>,
    /// Start of the last finalized interval ingested. Used to compute
    /// the inter-start interval from the last-dropped entry to the
    /// first live entry at report time.
    pub last_start_ms: Option<u64>,
    /// Running sums for inter-start interval variance. Stddev derived
    /// at report time via `interval_stddev_ms`.
    pub interval_count: usize,
    pub interval_sum_ms: u64,
    pub interval_sum_sq_ms: u128,
}

impl CellDutyAggregate {
    /// Fold one finalized interval `[start_ms, end_ms]` into the
    /// aggregate. Source-list invariants (sorted-by-start, non-overlap)
    /// make this strictly sequential: each call's `start_ms` is `>=
    /// self.last_end_ms`. Used at both prune time (for dropped entries)
    /// and report time (for the walk of the still-live list, treating
    /// open-ended entries as synthetic finite at `horizon_ms`).
    pub fn ingest_finalized(&mut self, start_ms: u64, end_ms: u64) {
        self.reservations += 1;
        self.first_start_ms = Some(self.first_start_ms.map_or(start_ms, |s| s.min(start_ms)));
        let duration = end_ms.saturating_sub(start_ms);
        self.busy_ms += duration;
        self.longest_hold_ms = self.longest_hold_ms.max(duration);
        self.frontier_ms = self.frontier_ms.max(end_ms);

        if let Some(prev_end) = self.last_end_ms {
            if start_ms > prev_end {
                let gap = start_ms - prev_end;
                self.gap_ms += gap;
                self.gap_count += 1;
                self.longest_gap_ms = self.longest_gap_ms.max(gap);
            }
        }
        self.last_end_ms = Some(self.last_end_ms.map_or(end_ms, |e| e.max(end_ms)));

        if let Some(prev_start) = self.last_start_ms {
            let interval = start_ms.saturating_sub(prev_start);
            self.interval_count += 1;
            self.interval_sum_ms += interval;
            self.interval_sum_sq_ms += (interval as u128) * (interval as u128);
        }
        self.last_start_ms = Some(start_ms);
    }

    /// Sample stddev of inter-start intervals (ms). Returns 0.0 when
    /// fewer than 2 intervals have been observed — sample variance is
    /// undefined for n<2.
    ///
    /// Computed from running integer sums to keep the hot path free of
    /// f64 arithmetic. Numerical precision is sufficient for the
    /// diagnostic-report use case (intervals in the 100ms–10s range,
    /// counts in the thousands).
    pub fn interval_stddev_ms(&self) -> f64 {
        if self.interval_count < 2 {
            return 0.0;
        }
        let n = self.interval_count as f64;
        let mean = self.interval_sum_ms as f64 / n;
        let mean_sq = (self.interval_sum_sq_ms as f64) / n;
        let variance_population = (mean_sq - mean * mean).max(0.0);
        // Sample variance: bias-corrected by Bessel's n/(n-1) factor.
        let variance_sample = variance_population * n / (n - 1.0);
        variance_sample.sqrt()
    }
}
