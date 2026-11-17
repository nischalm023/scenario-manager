//! Per-station chain-admission inflight tracking.
//!
//! `InflightSpan` is the RAII-ish wrapper that opens a span on a shared
//! `Arc<Mutex<TemporalCounter>>` at chain admission and closes it when the
//! HTM physically arrives at the presentation cell. It is **not** a regulator
//! by itself — it's the bookkeeping primitive that the demand-side regulators
//! (`PutawaySupplyDemand`, `OrderPickDemand`) read against to decide whether to
//! admit a fresh chain.
//!
//! # The "inflight" span
//!
//! Inflight is `[inflight_start, presentation_arrival)` — exclusive of the
//! operator dwell. `inflight_start` is the chain's earliest departure from its
//! source (HTM `available_at` for direct, relay `reservation.start` for
//! from-rack); `presentation_arrival` is the physical landing on the
//! presentation cell, set by `HtmDeliver` at commit using the path's terminal
//! `PathEnd` time. The span explicitly **excludes** dwell because
//! presentation-cell occupancy is tracked separately by `station_timeline`
//! (next-free-slot frontier); inflight tracks chains-in-motion, not slot-busy.
//!
//! # CONWIP count cap (per consumer)
//!
//! The cap on inflight count is a per-consumer count gate that the demand-side
//! regulators apply via `TemporalCounter::earliest_new_inflight_start`. Both
//! sides derive it per station through `AdmissionLaw`: the pick side divides
//! the station's `F_max` by the operator's dwell-plus-pallet-swap period and
//! scales by `queue_factor`; the putaway supply side divides the same `F_max`
//! by the induction pacing interval and scales by `putaway.htm_wip_factor`
//! (1.0 unless the run states one). The cap is a **safety ceiling, not a
//! target** (Spearman, Woodruff & Hopp, *CONWIP: a pull alternative to
//! kanban*, IJPR 28(5), 1990): CONWIP limits the maximum WIP in the system but
//! does not by itself drive WIP toward any operating point.
//!
//! # Pacing lives elsewhere
//!
//! Pure-CONWIP exhibits a "flooding-and-draining" failure mode: per Spearman
//! et al. 1990, CONWIP regulates the count of releases, not their spacing —
//! so under abundant candidates a system at the cap admits in tight bursts
//! up to `W_0`, drains over `~F_max`, then bursts again. Throughput is
//! correct (Little's Law); inter-arrival variance is bimodal. The standard
//! remedy is a separate pacing reference; in this system that role is filled
//! by `target_arrival = station_timeline.frontier − (queue_factor−1)·dwell`,
//! computed and applied in `PutawayInduction` (putaway) and `OrderPickDemand`
//! (pick). The target
//! is *not* part of `InflightSpan`'s contract — it operates on a different
//! timeline (presentation-cell consumer events, not chain releases) and may
//! differ in shape between demand modules.
//!
//! # The chicken-and-egg disclaimer
//!
//! In distributed online MRTA, predicting arrival time is genuinely
//! ill-posed: predicted arrival depends on path planning, which depends on
//! resource state, which depends on what other agents do (Korsah, Stentz &
//! Dias, *A comprehensive taxonomy for multi-robot task allocation*, IJRR
//! 32(12), 2013). This is **not** that regime. The greymatter-des solver is
//! centralized and offline; quotes are computed against an already-committed
//! reservation timeline that the pathfinder respects atomically. The
//! `target_arrival` and the inflight counter are constrained reads against a
//! single committed source-of-truth, not independent predictions.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::{SimDuration, SimTime};
use crate::pathfinding::{Path, WaypointAction};
use crate::signals::Version;
use crate::utils::temporal_counter::TemporalCounter;

// TODO(inflight-queue-consolidation): introduce `pub struct InflightQueue`
// here that bundles the inflight counter, `inflight_cap`, service timeline
// (currently `station_timeline` in each demand module — tracks
// `[arrival, arrival+service_time)` per admitted item to derive the next
// free slot and the pacing target), `service_time`, and `pacing_lead`.
// Rationale: today the dual-regulator decisioning (CONWIP cap check +
// pacing-target derivation) is duplicated in `PutawaySupplyDemand` and
// `OrderPickDemand`. Centralizing it eliminates drift between the two and
// gives one canonical place to audit "is this admit allowed?".
//
// Sketch:
//   pub struct InflightQueue {
//       inflight: Arc<Mutex<TemporalCounter>>,
//       inflight_cap: isize,
//       service_timeline: TemporalCounter,
//       service_time: SimDuration,
//       pacing_lead: SimDuration,
//   }
//
// Out of scope: `f_max` stays with the demand module — it's a search-window
// property of the supply side, not of the queue. See the discussion in
// `specs/cencora-perf/vtm-service-refactor.md` for the design call.

/// Open `[inflight_start, arrival)` span on a shared `TemporalCounter`.
/// `+1` happens at construction; `-1` at `record_arrival`.
pub struct InflightSpan {
    counter: Arc<Mutex<TemporalCounter>>,
    inflight_start: SimTime,
    closed: bool,
    version: Version,
}

/// Physical-bin hold for one pick order.
///
/// A palletizer bin is occupied from when an order starts until its last item
/// is physically picked. The hold marks that span on the station's shared
/// occupancy counter so no more than `effective_bins` orders ever overlap. A
/// clone rides in each of the order's pick chains and the final pick closes
/// it — the completion tracker fires once, and the shared `closed` flag
/// asserts the single close.
///
/// `bin_free` — when the bin becomes available, fixed at admission from a
/// predecessor's actual close — gates the order's pick starts and is the JIT
/// arrival target for its quotes.
#[derive(Clone)]
pub struct BinHold {
    counter: Arc<Mutex<TemporalCounter>>,
    version: Version,
    bin_free: SimTime,
    /// Latest pick completion (ms) recorded across the order's chains. The
    /// completion tracker fires on the last pick *recorded*, which under
    /// out-of-arrival-order planning may not be the sim-time-latest pick;
    /// closing at this running max keeps the bin held through the true last
    /// pick regardless of planning order.
    last_pick_done_ms: Arc<AtomicU64>,
    /// The finished pallet must be cleared (removed/wrapped/staged) before the
    /// bin accepts a new order, so the span is released this long after the
    /// last pick (`operator.pallet_swap_secs`).
    pallet_swap: SimDuration,
    closed: Arc<AtomicBool>,
}

impl BinHold {
    /// Open the order's bin span. Increments the counter at `bin_free`.
    pub fn open(
        counter: Arc<Mutex<TemporalCounter>>,
        bin_free: SimTime,
        pallet_swap: SimDuration,
        version: Version,
    ) -> Self {
        counter.lock().unwrap().add_open(bin_free);
        Self {
            counter,
            version,
            bin_free,
            last_pick_done_ms: Arc::new(AtomicU64::new(0)),
            pallet_swap,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Earliest instant the order's bin is physically free.
    pub fn bin_free(&self) -> SimTime {
        self.bin_free
    }

    /// Record one pick's completion. Every pick of the order reports here so
    /// [`Self::close`] frees the bin at the order's true last pick.
    pub fn record_pick_done(&self, done: SimTime) {
        self.last_pick_done_ms.fetch_max(done.as_ms(), Ordering::SeqCst);
    }

    /// Close the span at the latest recorded pick completion (including
    /// `final_done`) plus the pallet-swap time. Called once, by the
    /// `Presentation` whose `record_pick` completed the order.
    pub fn close(&self, final_done: SimTime) {
        let already = self.closed.swap(true, Ordering::SeqCst);
        assert!(!already, "BinHold::close called twice (bin_free={})", self.bin_free);
        let prior = self.last_pick_done_ms.fetch_max(final_done.as_ms(), Ordering::SeqCst);
        let last = prior.max(final_done.as_ms());
        self.counter.lock().unwrap().close_open(SimTime::ms(last) + self.pallet_swap);
        self.version.bump();
    }
}

impl InflightSpan {
    /// Open a new span. Increments the counter at `inflight_start`.
    pub fn open(counter: Arc<Mutex<TemporalCounter>>, inflight_start: SimTime, version: Version) -> Self {
        counter.lock().unwrap().add_open(inflight_start);
        Self { counter, inflight_start, closed: false, version }
    }

    /// Close the span at the path's terminal `PathEnd` time. The path is the
    /// authoritative source for the arrival time — this signature prevents
    /// callers from re-deriving the time and risking deviation from what the
    /// pathfinder actually planned. Asserts that the path's terminal waypoint
    /// is a `PathEnd` (a `Path` invariant per `pathfinding::path`).
    pub fn record_arrival(&mut self, path: &Path) {
        assert!(!self.closed, "InflightSpan::record_arrival called twice (start={})", self.inflight_start);
        let last =
            path.waypoints.last().unwrap_or_else(|| panic!("InflightSpan::record_arrival: path has no waypoints"));
        let arrival = match last.action {
            WaypointAction::PathEnd { time } => time,
            ref other => {
                panic!("InflightSpan::record_arrival: path terminal waypoint is not PathEnd (got {:?})", other)
            }
        };
        self.counter.lock().unwrap().close_open(arrival);
        self.closed = true;
        self.version.bump();
    }

    /// Withdraw a span whose journey was never committed: the counter loses
    /// the `+1` at its start, as though it had never opened.
    pub fn cancel(&mut self) {
        assert!(!self.closed, "InflightSpan::cancel after the span closed (start={})", self.inflight_start);
        self.counter.lock().unwrap().cancel_open(self.inflight_start);
        self.closed = true;
        self.version.bump();
    }

    pub fn inflight_start(&self) -> SimTime {
        self.inflight_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SimTime;
    use crate::pathfinding::{Orientation, Waypoint, WaypointAction};
    use desgraph::CellId;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    fn path_ending_at(end: SimTime) -> Path {
        Path {
            waypoints: vec![
                Waypoint {
                    cell: CellId(0),
                    orientation: Orientation::East,
                    action: WaypointAction::PathStart { time: SimTime::ZERO },
                },
                Waypoint {
                    cell: CellId(0),
                    orientation: Orientation::East,
                    action: WaypointAction::PathEnd { time: end },
                },
            ],
            travel_time: end - SimTime::ZERO,
        }
    }

    #[test]
    fn open_then_record_arrival_writes_span() {
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let mut span = InflightSpan::open(counter.clone(), t(100), Version::new());
        assert_eq!(counter.lock().unwrap().count_at(t(100)), 1);
        assert_eq!(counter.lock().unwrap().count_at(t(500)), 1);

        span.record_arrival(&path_ending_at(t(500)));
        assert_eq!(counter.lock().unwrap().count_at(t(499)), 1);
        assert_eq!(counter.lock().unwrap().count_at(t(500)), 0);
    }

    #[test]
    #[should_panic(expected = "called twice")]
    fn record_arrival_twice_panics() {
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let mut span = InflightSpan::open(counter, t(100), Version::new());
        span.record_arrival(&path_ending_at(t(500)));
        span.record_arrival(&path_ending_at(t(600)));
    }

    /// A journey refused before it committed anything withdraws its span: the
    /// count is as though the span never opened, and the change is signalled.
    #[test]
    fn a_cancelled_span_leaves_no_count() {
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let version = Version::new();
        let mut span = InflightSpan::open(counter.clone(), t(100), version.clone());
        let seen = version.seen();
        span.cancel();
        assert_eq!(counter.lock().unwrap().count_at(t(100)), 0);
        assert_eq!(counter.lock().unwrap().frontier().1, 0);
        assert!(version.seen() > seen, "the cancel is signalled");
    }

    #[test]
    #[should_panic(expected = "cancel after the span closed")]
    fn a_span_that_recorded_its_arrival_cannot_be_cancelled() {
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let mut span = InflightSpan::open(counter, t(100), Version::new());
        span.record_arrival(&path_ending_at(t(500)));
        span.cancel();
    }

    #[test]
    fn bin_hold_open_close_spans_occupancy() {
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let hold = BinHold::open(counter.clone(), t(200), SimDuration::ZERO, Version::new());
        assert_eq!(hold.bin_free(), t(200));
        // Open-ended until the final pick closes it.
        assert_eq!(counter.lock().unwrap().count_at(t(99_999)), 1);
        // A clone (another chain's Presentation) closes at the actual done.
        let clone = hold.clone();
        clone.close(t(900));
        assert_eq!(counter.lock().unwrap().count_at(t(899)), 1);
        assert_eq!(counter.lock().unwrap().count_at(t(900)), 0);
    }

    #[test]
    fn bin_hold_release_includes_pallet_swap() {
        // The bin stays occupied for the pallet-clear time after the last
        // pick — a successor can't claim the position mid-swap.
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let hold = BinHold::open(counter.clone(), t(100), SimDuration::seconds(30), Version::new());
        hold.close(t(900));
        assert_eq!(counter.lock().unwrap().count_at(t(900)), 1, "bin still held during pallet swap");
        assert_eq!(counter.lock().unwrap().count_at(t(900) + SimDuration::seconds(30)), 0);
    }

    #[test]
    fn bin_hold_closes_at_latest_recorded_pick_not_the_closer() {
        // Chains can plan out of arrival order: the pick that completes the
        // tracker (and calls close) may not be the sim-time-latest pick. The
        // bin must stay held through the latest RECORDED pick.
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let hold = BinHold::open(counter.clone(), t(100), SimDuration::ZERO, Version::new());
        let late_chain = hold.clone();
        late_chain.record_pick_done(t(950)); // sim-time-latest pick, recorded first
        hold.record_pick_done(t(700));
        hold.close(t(700)); // closer's own pick is earlier
        assert_eq!(counter.lock().unwrap().count_at(t(949)), 1, "bin held through the true last pick");
        assert_eq!(counter.lock().unwrap().count_at(t(950)), 0, "bin freed at the latest recorded completion");
    }

    #[test]
    #[should_panic(expected = "close called twice")]
    fn bin_hold_double_close_panics_across_clones() {
        let counter = Arc::new(Mutex::new(TemporalCounter::new()));
        let hold = BinHold::open(counter, t(100), SimDuration::ZERO, Version::new());
        let clone = hold.clone();
        hold.close(t(500));
        clone.close(t(600)); // shared closed flag — must panic
    }
}
