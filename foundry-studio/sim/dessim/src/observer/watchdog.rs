//! Progress-based watchdog: detects simulation stalls.
//!
//! The watchdog is notified of progress (picks completing) by the observer.
//! If no progress is seen for `stall_threshold` of wall-clock time, it signals
//! a stop via the shared `StopSignal`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sim_types::{StopReason, StopSignal};

/// Default stall threshold: 30 seconds of wall-clock time with no picks.
const DEFAULT_STALL_THRESHOLD: Duration = Duration::from_secs(30);

/// Watches for simulation stalls (no picks for an extended wall-clock period).
pub struct Watchdog {
    stop_signal: Arc<StopSignal>,
    stall_threshold: Duration,
    last_progress_wall: Instant,
    any_progress_seen: bool,
}

impl Watchdog {
    pub fn new(stop_signal: Arc<StopSignal>) -> Self {
        Self {
            stop_signal,
            stall_threshold: DEFAULT_STALL_THRESHOLD,
            last_progress_wall: Instant::now(),
            any_progress_seen: false,
        }
    }

    pub fn with_stall_threshold(mut self, threshold: Duration) -> Self {
        self.stall_threshold = threshold;
        self
    }

    /// Called by the observer when a pick completes.
    pub fn notify_progress(&mut self, _sim_time_ms: u64) {
        self.last_progress_wall = Instant::now();
        self.any_progress_seen = true;
    }

    /// Called by the observer on every event. Checks whether we've stalled.
    pub fn check(&self, _current_sim_time_ms: u64) {
        if !self.any_progress_seen {
            return;
        }
        if self.stall_threshold.is_zero() {
            return;
        }

        let elapsed = self.last_progress_wall.elapsed();
        if elapsed >= self.stall_threshold {
            let stall_secs = elapsed.as_secs_f64();
            eprintln!("[watchdog] no picks completed in {stall_secs:.1}s of wall-clock time. Signaling stop.",);
            self.stop_signal.stop(StopReason::Watchdog(format!("no picks for {stall_secs:.1}s wall-clock",)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_on_stall() {
        let signal = StopSignal::new();
        let mut wd = Watchdog::new(signal.clone()).with_stall_threshold(Duration::from_millis(1));
        wd.notify_progress(1_000);
        std::thread::sleep(Duration::from_millis(5));
        wd.check(20_000);
        assert!(signal.is_stopped());
        match signal.take_reason().unwrap() {
            StopReason::Watchdog(detail) => assert!(detail.contains("no picks")),
            other => panic!("expected Watchdog, got: {other:?}"),
        }
    }

    #[test]
    fn does_not_fire_with_recent_progress() {
        let signal = StopSignal::new();
        let mut wd = Watchdog::new(signal.clone()).with_stall_threshold(Duration::from_secs(60));
        wd.notify_progress(1_000);
        wd.check(1_000);
        assert!(!signal.is_stopped());
    }

    #[test]
    fn does_not_fire_before_first_progress() {
        let signal = StopSignal::new();
        let wd = Watchdog::new(signal.clone()).with_stall_threshold(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        wd.check(100_000);
        assert!(!signal.is_stopped());
    }
}
