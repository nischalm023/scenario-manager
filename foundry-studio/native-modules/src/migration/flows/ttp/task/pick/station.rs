//! One pick station's live admission state: the two independent ceilings a
//! station enforces — how many pick chains may travel toward it, and how many
//! orders may hold a palletizer bin — plus the presentation frontier that
//! paces arrivals.

use std::sync::Mutex;

use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use desgraph::{CellId, StationId};

use crate::composition::policy::StationPolicy;
use crate::flows::ttp::inflight::{BinHold, InflightSpan};
use crate::prelude::*;
use crate::utils::temporal_counter::TemporalCounter;

pub struct StationState {
    pub id: StationId,
    pub presentation_cell: CellId,
    bins: usize,
    /// Chains that may be in flight toward this station at once, derived from
    /// the station's own `F_max` when it was built.
    pipeline_cap: isize,
    policy: StationPolicy,
    picks_pending: AtomicU32,
    picks_in_flight: Arc<Mutex<TemporalCounter>>,
    bin_occupancy: Arc<Mutex<TemporalCounter>>,
    presentation_frontier: Mutex<TemporalCounter>,
    order_bins: Mutex<FxHashMap<OrderId, BinHold>>,
    version: Version,
}

impl StationState {
    pub fn new(station: &Station, policy: StationPolicy, f_max: SimDuration, version: Version) -> Self {
        Self {
            id: station.id,
            presentation_cell: station.presentation_cell,
            bins: station.bins,
            pipeline_cap: policy.admission.cap(f_max) as isize,
            policy,
            picks_pending: AtomicU32::new(0),
            picks_in_flight: Arc::new(Mutex::new(TemporalCounter::new())),
            bin_occupancy: Arc::new(Mutex::new(TemporalCounter::new())),
            presentation_frontier: Mutex::new(TemporalCounter::new()),
            order_bins: Mutex::new(FxHashMap::default()),
            version,
        }
    }

    /// How many orders may hold a bin at once. This is a physical count of
    /// palletizer positions and nothing else: the chain pipeline is bounded
    /// separately by [`Self::admission_after`], so clamping bins to the WIP
    /// budget as well would let a travel-time quantity shrink the station's
    /// order coverage.
    fn effective_bins(&self) -> isize {
        self.bins.max(1) as isize
    }

    pub fn bin_floor(&self, after: SimTime) -> Option<SimTime> {
        self.bin_occupancy.lock().unwrap().earliest_new_inflight_start(self.effective_bins(), after)
    }

    pub fn open_bin(&self, order: OrderId, floor: SimTime) {
        let hold = BinHold::open(self.bin_occupancy.clone(), floor, self.policy.pallet_swap, self.version.clone());
        self.order_bins.lock().unwrap().insert(order, hold);
    }

    pub fn bin(&self, order: OrderId) -> Option<BinHold> {
        self.order_bins.lock().unwrap().get(&order).cloned()
    }

    pub fn take_bin(&self, order: OrderId) {
        self.order_bins.lock().unwrap().remove(&order);
    }

    /// Earliest instant a fresh pick chain fits under the pipeline cap, or
    /// `None` while the cap is saturated for the whole forward timeline.
    pub fn admission_after(&self, after: SimTime) -> Option<SimTime> {
        let budget = self.pipeline_cap - self.picks_pending.load(Ordering::Relaxed) as isize;
        if budget <= 0 {
            return None;
        }
        self.picks_in_flight.lock().unwrap().earliest_new_inflight_start(budget, after)
    }

    pub fn reserve_pick_slot(&self) {
        self.picks_pending.fetch_add(1, Ordering::Relaxed);
        self.version.bump();
    }

    pub fn open_pick_span(&self, start: SimTime) -> InflightSpan {
        self.picks_pending.fetch_sub(1, Ordering::Relaxed);
        let span = InflightSpan::open(self.picks_in_flight.clone(), start, self.version.clone());
        self.version.bump();
        span
    }

    pub fn arrival_aim(&self) -> Option<SimTime> {
        self.policy.aim_arrivals.then(|| self.presentation_frontier.lock().unwrap().frontier().0)
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn pick_dwell(&self) -> SimDuration {
        self.policy.pick_dwell
    }

    pub fn book_presentation(&self, arrival: SimTime) {
        if !self.policy.aim_arrivals {
            return;
        }
        let mut frontier = self.presentation_frontier.lock().unwrap();
        let start = frontier.frontier().0.max(arrival);
        frontier.add_span(start, start + self.policy.pick_dwell);
        drop(frontier);
        self.version.bump();
    }

    pub fn idle(&self) -> bool {
        self.picks_pending.load(Ordering::Relaxed) == 0
            && self.picks_in_flight.lock().unwrap().frontier().1 == 0
            && self.order_bins.lock().unwrap().is_empty()
    }
}
