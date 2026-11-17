//! The VTM family's laziness audit: how many live cycle candidacies named
//! the same VTM at each acceptance — the winner plus the rivals that had
//! independently selected it.
//!
//! A depth of 1 every time says cycles are being accepted the instant they
//! are proposed, with no choice to arbitrate; a depth that grows says
//! several chains are contending for one VTM and the acceptance order is
//! doing real work. Sibling of the retrieve family's
//! [`AcceptanceStats`](super::super::acceptance_stats::AcceptanceStats),
//! kept separate because the two families count different things and print
//! different lines.

use std::fmt::Write as _;

#[derive(Default)]
pub struct VtmAcceptanceStats {
    accepts: u64,
    depth_sum: u64,
    depth_max: u64,
}

impl VtmAcceptanceStats {
    pub fn record_accept(&mut self, depth: usize) {
        let depth = depth as u64;
        self.accepts += 1;
        self.depth_sum += depth;
        self.depth_max = self.depth_max.max(depth);
    }

    /// End-of-run line; `None` when no cycle was ever accepted.
    pub fn report(&self) -> Option<String> {
        if self.accepts == 0 {
            return None;
        }
        let mut out = String::new();
        let _ = write!(
            out,
            "vtm accepts={} choice_depth avg={:.2} max={}",
            self.accepts,
            self.depth_sum as f64 / self.accepts as f64,
            self.depth_max,
        );
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stats_report_nothing() {
        assert!(VtmAcceptanceStats::default().report().is_none());
    }

    #[test]
    fn depth_average_and_max_read_off_the_accepts() {
        let mut stats = VtmAcceptanceStats::default();
        stats.record_accept(1);
        stats.record_accept(4);
        let report = stats.report().expect("accepts were recorded");
        assert_eq!(report, "vtm accepts=2 choice_depth avg=2.50 max=4");
    }

    /// An uncontended fleet still reports — depth 1 every time is the
    /// finding, not the absence of one.
    #[test]
    fn uncontended_accepts_report_depth_one() {
        let mut stats = VtmAcceptanceStats::default();
        stats.record_accept(1);
        stats.record_accept(1);
        let report = stats.report().expect("accepts were recorded");
        assert_eq!(report, "vtm accepts=2 choice_depth avg=1.00 max=1");
    }
}
