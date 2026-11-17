use serde::{Deserialize, Serialize};

use super::{MetricScope, ReportPeriod};

/// Resolved half-open simulation range `[start_ms, end_ms)` for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReportRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl ReportRange {
    pub const fn duration_ms(self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    pub const fn is_empty(self) -> bool {
        self.start_ms == self.end_ms
    }
}

/// Fully resolved report parameters and the stable cache key for report data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReportRequest {
    pub scope: MetricScope,
    pub range: ReportRange,
    pub period: ReportPeriod,
}

impl ReportRequest {
    /// Resolve a full-run or post-embargo request against one simulation run.
    ///
    /// An embargo after the run clamps to an empty range at `run_end_ms`.
    pub const fn for_run(scope: MetricScope, period: ReportPeriod, run_end_ms: u64, pick_embargo_ms: u64) -> Self {
        let start_ms = match scope {
            MetricScope::FullRun => 0,
            MetricScope::PostEmbargo => {
                if pick_embargo_ms < run_end_ms {
                    pick_embargo_ms
                } else {
                    run_end_ms
                }
            }
        };
        Self { scope, range: ReportRange { start_ms, end_ms: run_end_ms }, period }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_run_starts_at_zero() {
        let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 10_000, 2_000);

        assert_eq!(request.range, ReportRange { start_ms: 0, end_ms: 10_000 });
    }

    #[test]
    fn post_embargo_range_excludes_lead_in() {
        let request = ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 10_000, 2_000);

        assert_eq!(request.range, ReportRange { start_ms: 2_000, end_ms: 10_000 });
    }

    #[test]
    fn embargo_after_run_is_an_empty_range() {
        let request = ReportRequest::for_run(MetricScope::PostEmbargo, ReportPeriod::Overall, 10_000, 12_000);

        assert!(request.range.is_empty());
        assert_eq!(request.range.duration_ms(), 0);
    }
}
