//! The retrieve family's laziness audit: how deep the choice pool is at
//! each acceptance — the winner plus its live rivals on the accepted
//! relay, and the family-wide tentative count at the same instant —
//! bucketed by the accepted bind's hour. Printed with the end-of-run
//! reports, halted runs included, so acceptance-trigger changes are
//! comparable across runs.

use std::fmt::Write as _;

use crate::model::SimTime;

// ordered: report lines print in hour order
use std::collections::BTreeMap;

const DEPTH_BUCKETS: usize = 9;

#[derive(Default)]
pub struct AcceptanceStats {
    accepts: u64,
    depth_sum: u64,
    depth_max: u64,
    // Choice depth 1..=8 in place, 9+ in the last bucket.
    depth_histogram: [u64; DEPTH_BUCKETS],
    pool_sum: u64,
    pool_max: u64,
    per_hour: BTreeMap<u64, (u64, u64)>,
}

impl AcceptanceStats {
    pub fn record_accept(&mut self, depth: usize, pool: usize, start: SimTime) {
        let depth = depth as u64;
        self.accepts += 1;
        self.depth_sum += depth;
        self.depth_max = self.depth_max.max(depth);
        let bucket = (depth.max(1) as usize - 1).min(DEPTH_BUCKETS - 1);
        self.depth_histogram[bucket] += 1;
        self.pool_sum += pool as u64;
        self.pool_max = self.pool_max.max(pool as u64);
        let entry = self.per_hour.entry(start.as_ms() / 3_600_000).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += depth;
    }

    pub fn report(&self) -> Option<String> {
        if self.accepts == 0 {
            return None;
        }
        let avg = |sum: u64, n: u64| if n == 0 { 0.0 } else { sum as f64 / n as f64 };
        let mut out = String::new();
        let _ = writeln!(out, "=== Retrieve acceptance stats (laziness audit) ===");
        let _ = writeln!(
            out,
            "accepts={} choice_depth avg={:.2} max={}",
            self.accepts,
            avg(self.depth_sum, self.accepts),
            self.depth_max,
        );
        let _ = write!(out, "depth histogram:");
        for (i, n) in self.depth_histogram.iter().enumerate() {
            if *n > 0 {
                let label = if i == DEPTH_BUCKETS - 1 { format!("{}+", DEPTH_BUCKETS) } else { format!("{}", i + 1) };
                let _ = write!(out, " {label}:{n}");
            }
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "global_tentatives_at_accept avg={:.1} max={}",
            avg(self.pool_sum, self.accepts),
            self.pool_max,
        );
        let _ = write!(out, "per-hour (accepted start):");
        for (hour, (n, depth_sum)) in &self.per_hour {
            let _ = write!(out, " h{hour} n={n} avg={:.2} |", avg(*depth_sum, *n));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stats_report_nothing() {
        assert!(AcceptanceStats::default().report().is_none());
    }

    #[test]
    fn depth_and_pool_averages() {
        let mut stats = AcceptanceStats::default();
        stats.record_accept(1, 10, SimTime::ZERO);
        stats.record_accept(3, 20, SimTime::ZERO);
        let report = stats.report().expect("accepts were recorded");
        assert!(report.contains("accepts=2 choice_depth avg=2.00 max=3"), "{report}");
        assert!(report.contains("global_tentatives_at_accept avg=15.0 max=20"), "{report}");
        assert!(report.contains("depth histogram: 1:1 3:1"), "{report}");
    }

    #[test]
    fn deep_choices_land_in_the_overflow_bucket() {
        let mut stats = AcceptanceStats::default();
        stats.record_accept(9, 9, SimTime::ZERO);
        stats.record_accept(40, 40, SimTime::ZERO);
        let report = stats.report().expect("accepts were recorded");
        assert!(report.contains("depth histogram: 9+:2"), "{report}");
    }

    #[test]
    fn accepts_bucket_by_start_hour() {
        let mut stats = AcceptanceStats::default();
        stats.record_accept(2, 2, SimTime::ms(30 * 60 * 1000));
        stats.record_accept(4, 4, SimTime::ms(2 * 3_600_000 + 1));
        let report = stats.report().expect("accepts were recorded");
        assert!(report.contains("h0 n=1 avg=2.00"), "{report}");
        assert!(report.contains("h2 n=1 avg=4.00"), "{report}");
    }
}
