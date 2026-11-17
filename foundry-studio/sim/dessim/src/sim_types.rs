//! Within dessim, `sim_types` is the WHOLE-RUN surface: the types that describe
//! a simulation run from outside it, as opposed to anything happening inside
//! one. Two groups. How a run is told to stop and reports why it did
//! ([`RunFlags`], [`StopReason`], [`StopSignal`]), and what it hands back
//! ([`SimResult`] once finished, [`Counters`] while it is still running: the
//! figures its final slices state, which a caller polls).
//!
//! Nothing here models the warehouse. A field on [`SimResult`] earns its place
//! by being a fact about the run as a whole, which is why the resolved putaway
//! bias sits here: it is the effective value the run used, recoverable
//! afterwards from the output alone.
//!
//! TODO(overstuffed): [`picks_per_hour`] is a throughput formula, not a
//! run-level type. It lives here because [`SimResult`] was its first caller and
//! `solver.rs` needed the same divisor; it belongs with the throughput metrics.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::metrics::DetailedMetrics;
use crate::simtime::{SimOffset, SimTime};

/// Runtime flags controlling diagnostics and behavior for a simulation run.
#[derive(Clone, Debug, Default)]
pub struct RunFlags {
    pub no_watchdog: bool,
    pub allow_incomplete: bool,
}

/// Why the simulation stopped.
#[derive(Clone, Debug)]
pub enum StopReason {
    Complete,
    UserStop,
    Watchdog(String),
    WallTimeout,
    SimTimeLimit,
    Error(String),
}

impl Default for StopReason {
    fn default() -> Self {
        StopReason::Complete
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::Complete => write!(f, "all orders completed"),
            StopReason::UserStop => write!(f, "stopped by user"),
            StopReason::Watchdog(d) => write!(f, "watchdog: {d}"),
            StopReason::WallTimeout => write!(f, "wall-clock timeout"),
            StopReason::SimTimeLimit => write!(f, "sim duration limit reached"),
            StopReason::Error(d) => write!(f, "error: {d}"),
        }
    }
}

/// Shared cancellation signal. Any thread can request a stop via `stop()`.
/// First writer wins.
#[derive(Debug)]
pub struct StopSignal {
    flag: AtomicBool,
    reason: Mutex<Option<StopReason>>,
}

impl StopSignal {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { flag: AtomicBool::new(false), reason: Mutex::new(None) })
    }

    pub fn stop(&self, reason: StopReason) {
        let mut r = self.reason.lock().unwrap();
        if r.is_none() {
            *r = Some(reason);
        }
        self.flag.store(true, Ordering::Release);
    }

    pub fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    pub fn take_reason(&self) -> Option<StopReason> {
        self.reason.lock().unwrap().take()
    }

    /// Whether the run stopped on its simtime limit, leaving the reason for whoever takes it.
    pub fn stopped_on_sim_time_limit(&self) -> bool {
        matches!(*self.reason.lock().unwrap(), Some(StopReason::SimTimeLimit))
    }
}

/// What a caller watching a run reads while it goes. Clone is cheap (Arc
/// refs).
///
/// The shown figures are the run's picture: only the observer sets them, as
/// it makes each slice final, so they state what the slices made final so far
/// state and never a fact B has not passed.
#[derive(Clone)]
pub struct Counters {
    shown_picks: Arc<AtomicU64>,
    shown_orders_completed: Arc<AtomicU64>,
    /// In milliseconds, since an atomic holds no `SimTime`.
    shown_sim_ms: Arc<AtomicU64>,
}

impl Counters {
    pub fn new() -> Self {
        Self {
            shown_picks: Arc::new(AtomicU64::new(0)),
            shown_orders_completed: Arc::new(AtomicU64::new(0)),
            shown_sim_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The picks the slices made final so far state.
    pub fn shown_picks(&self) -> u64 {
        self.shown_picks.load(Ordering::Relaxed)
    }

    /// The orders the slices made final so far state completed.
    pub fn shown_orders_completed(&self) -> u64 {
        self.shown_orders_completed.load(Ordering::Relaxed)
    }

    /// The simtime the shown figures reach: the end of the newest slice made
    /// final, and once the stream has closed, the run's end.
    pub fn shown_sim_time(&self) -> SimTime {
        SimTime::ms(self.shown_sim_ms.load(Ordering::Relaxed))
    }

    /// Add a slice just made final to the shown figures: its picks, its
    /// completed orders, and the simtime it reaches.
    pub(crate) fn show_slice(&self, picks: u64, orders_completed: u64, reaches: SimTime) {
        self.shown_picks.fetch_add(picks, Ordering::Relaxed);
        self.shown_orders_completed.fetch_add(orders_completed, Ordering::Relaxed);
        self.show_reached(reaches);
    }

    /// The shown figures reach `at`.
    pub(crate) fn show_reached(&self, at: SimTime) {
        self.shown_sim_ms.store(at.as_ms(), Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// SimResult
// ---------------------------------------------------------------------------

/// Aggregate simulation results collected after the simulation completes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SimResult {
    /// Wall-clock time taken to execute the simulation.
    pub wall_clock_duration: Duration,
    /// Simulated duration.
    pub sim_duration: Duration,
    /// Number of orders where ALL tasks were picked.
    pub total_orders_completed: usize,
    /// Cumulative picks across all completed station visits.
    pub total_picks: usize,
    /// `total_picks / (sim_duration_hours - embargo_hours)`. Picking is
    /// suspended during the configured embargo lead-in, so this divides by
    /// only the active picking window.
    pub throughput_picks_per_hour: f64,
    /// Putaway cycles per hour (one count per cycle, qty-independent).
    /// Counts physical fill completions through the reported simulation end.
    pub throughput_putaways_per_hour: f64,
    /// Putaway items per hour — qty-weighted analog of
    /// `throughput_picks_per_hour`.
    /// Counts inducted units through the reported simulation end.
    pub throughput_putaway_items_per_hour: f64,
    /// Order picks that completed (excludes putaway).
    pub completed_order_picks: usize,
    /// Expected order picks from scenario config.
    pub expected_order_picks: usize,
    /// Total number of events processed by the simulation engine.
    pub event_count: u64,
    /// Detailed instrumentation metrics.
    pub metrics: DetailedMetrics,
    /// Total inventory units placed at simulation start.
    pub initial_inventory: u64,
    /// Resolved putaway direct-chain bias — the value the run actually used.
    /// When `putaway.direct_bias_secs` is auto (`None`) this is the map's
    /// Fleet Pressure Index; a fixed config keeps that value; zero when there
    /// are no putaway stations. Surfaced so the effective (and auto-derived)
    /// bias is recoverable from the run output.
    #[serde(default, rename = "putaway_direct_bias_ms")]
    pub putaway_direct_bias: SimOffset,
    /// Why the simulation stopped.
    #[serde(skip)]
    pub stop_reason: StopReason,
}

/// Picks/hr over the active picking window: `picks / (sim_duration - embargo)`.
///
/// Picking is suspended during the configured embargo lead-in (see
/// `PickConfig::embargo_secs`), so throughput must divide by only the
/// post-embargo window. Every result path funnels through this one function so
/// the numerator and denominator always cover the same window and no run can
/// report different throughput depending on which code path produced it
/// (greymatter-foundry#208).
///
/// `picks` MUST already exclude any pre-embargo completions (see
/// `ScalarCounters::pre_embargo_picks`): counting a pre-embargo pick here would
/// both inflate the numerator and enjoy the shrunken denominator.
pub fn picks_per_hour(picks: u64, sim_duration: Duration, embargo_secs: f64) -> f64 {
    let pickable_hours = (sim_duration.as_secs_f64() - embargo_secs) / 3600.0;
    if pickable_hours > 0.0 { picks as f64 / pickable_hours } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// The bias field is typed `SimOffset` but its wire key keeps the `_ms`
    /// name, and the encoding stays a bare integer. Both halves matter: the
    /// field is `#[serde(default)]`, so a key this side no longer writes would
    /// not fail on read, it would silently deserialize as zero and quietly
    /// change how every archived run reports its effective bias.
    #[test]
    fn putaway_bias_wire_key_and_shape_are_stable() {
        let r = SimResult { putaway_direct_bias: SimOffset::ms(-3_000), ..Default::default() };
        let json: Value = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json.get("putaway_direct_bias_ms"),
            Some(&serde_json::json!(-3_000)),
            "wire key must stay `putaway_direct_bias_ms` and stay a bare integer"
        );
        assert!(json.get("putaway_direct_bias").is_none(), "the Rust field name must not reach the wire");

        // An archive written before the retype still reads back correctly.
        let legacy = serde_json::json!({ "putaway_direct_bias_ms": -3_000 });
        let back: SimResult = serde_json::from_value(legacy).unwrap();
        assert_eq!(back.putaway_direct_bias, SimOffset::ms(-3_000));
    }

    #[test]
    fn divides_by_post_embargo_window_not_full_window() {
        // 100 picks, 2h sim, 1h embargo => 1h pickable => 100/hr (NOT the 50/hr
        // a full-window divisor would report). This is the alignment guarantee
        // that both result paths now share.
        let r = picks_per_hour(100, Duration::from_secs(7200), 3600.0);
        assert!((r - 100.0).abs() < 1e-9, "expected 100/hr, got {r}");
    }

    #[test]
    fn non_positive_window_yields_zero() {
        // Embargo == sim window, and embargo > sim window: no pickable time.
        assert_eq!(picks_per_hour(50, Duration::from_secs(3600), 3600.0), 0.0);
        assert_eq!(picks_per_hour(50, Duration::from_secs(1800), 3600.0), 0.0);
    }

    #[test]
    fn excluding_pre_embargo_picks_prevents_inflation() {
        // 100 completed, 10 of them pre-embargo. Feeding only the 90 post-embargo
        // picks yields 90/hr; feeding all 100 would wrongly report 100/hr.
        let picks_completed = 100u64;
        let pre_embargo = 10u64;
        let post = picks_completed.saturating_sub(pre_embargo);
        let honest = picks_per_hour(post, Duration::from_secs(7200), 3600.0);
        let inflated = picks_per_hour(picks_completed, Duration::from_secs(7200), 3600.0);
        assert_eq!(honest, 90.0);
        assert_eq!(inflated, 100.0);
        assert!(honest < inflated, "excluding pre-embargo picks must lower the rate");
    }
}
