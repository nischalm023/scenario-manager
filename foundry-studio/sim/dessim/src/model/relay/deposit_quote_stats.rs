//! The forward family's deposit-quote audit: how much later than the relay
//! slot's start each committed forward cycle actually lands its tote.
//!
//! A chain is admitted against the retrieve reservation's `start` — the
//! instant the demand module chose and asked the VTM selector to meet. The
//! selector then returns the earliest arrival some VTM can actually
//! manage, which is never earlier and is often later. That difference is
//! the deposit quote error, and it is what an HTM discovers when it drives
//! to meet a deposit and finds no tote on the board.
//!
//! Distinct from the pre-cycle idle gap `VtmIdleReason` attributes: that
//! one measures the VTM waiting before it starts, this one measures the
//! tote landing after it was wanted.

use std::fmt::Write as _;

use crate::model::SimDuration;

#[derive(Default)]
pub struct DepositQuoteStats {
    slips_ms: Vec<u64>,
    sum_ms: u64,
    max_ms: u64,
}

impl DepositQuoteStats {
    pub fn record(&mut self, slip: SimDuration) {
        let ms = slip.as_ms();
        self.slips_ms.push(ms);
        self.sum_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    /// End-of-run line; `None` when no forward cycle committed.
    pub fn report(&self) -> Option<String> {
        let n = self.slips_ms.len() as u64;
        if n == 0 {
            return None;
        }
        // Sorted on demand rather than kept sorted: this runs once, at the
        // end, and the record path is on every forward commit.
        let mut sorted = self.slips_ms.clone();
        sorted.sort_unstable();
        let p90 = sorted[((sorted.len() * 9) / 10).min(sorted.len() - 1)];
        let mut out = String::new();
        let _ = write!(
            out,
            "deposit quote error: n={n}, mean={:.1}s p90={:.1}s max={:.1}s",
            self.sum_ms as f64 / n as f64 / 1000.0,
            p90 as f64 / 1000.0,
            self.max_ms as f64 / 1000.0,
        );
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stats_report_nothing() {
        assert!(DepositQuoteStats::default().report().is_none());
    }

    #[test]
    fn mean_p90_and_max_read_off_the_recorded_slips() {
        let mut stats = DepositQuoteStats::default();
        for secs in 1..=10 {
            stats.record(SimDuration::seconds(secs));
        }
        let report = stats.report().expect("slips were recorded");
        assert!(report.contains("n=10"), "{report}");
        assert!(report.contains("mean=5.5s"), "{report}");
        // Ten sorted samples: index 9 is the last, the 90th percentile.
        assert!(report.contains("p90=10.0s"), "{report}");
        assert!(report.contains("max=10.0s"), "{report}");
    }

    #[test]
    fn an_on_time_fleet_reports_zeroes_rather_than_nothing() {
        let mut stats = DepositQuoteStats::default();
        stats.record(SimDuration::ZERO);
        let report = stats.report().expect("a zero slip is still a sample");
        assert!(report.contains("n=1, mean=0.0s p90=0.0s max=0.0s"), "{report}");
    }
}
