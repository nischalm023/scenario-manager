//! Order-level dispatch queue: SKU-pool readiness and lifecycle for pick orders.
//!
//! # Intent
//!
//! Implements **two-stage allocation** for a concurrent putaway+pick warehouse.
//! Industry name: ATP soft-allocation + hard-allocation (Manhattan WMOS / SAP
//! EWM / Blue Yonder). Foundational ref: Hausman/Schwarz/Graves, *"Optimal
//! storage assignment in automatic warehousing systems"*, Mgmt Sci 1976.
//! Modern restatement: Boysen/de Koster/Weidinger, *"Warehousing in the
//! e-commerce era"*, EJOR 2019, §3.2 "Dynamic order release".
//!
//! - **Stage 1 (this module)**: an order moves Blocked → Ready when its full
//!   SKU demand can ever be satisfied (forward-looking, accounting for other
//!   orders' reservations). At order-start (`start_picking`), the planner
//!   atomically books all required SKU/qty against the inventory ledger via
//!   `InventoryService::reserve_for_order` — the **anti-snaking guarantee**.
//! - **Stage 2 (`relay_service::reserve_relay_for_retrieve`)**: per-line
//!   dispatch late-binds to a specific tote, choosing closest-by-travel-time
//!   from any zone at dispatch instant. Late binding is the value proposition
//!   over pre-binding: the closest tote at line N can differ from the closest
//!   at order-start because the warehouse state evolves between picks.
//!
//! The SKU-pool reservation prevents inventory snaking; late binding preserves
//! travel-distance optimality. Pre-binding to specific totes/zones at
//! order-start would freeze the choice and lose adaptivity.
//!
//! # Lifecycle
//!
//! ```text
//!   Blocked  ──putaway delivers──►  Ready  ──reserve_for_order──►  Picking  ──all lines──►  Complete
//!                needed SKU                  + start_picking                    dispatched
//! ```
//!
//! Phase membership is implicit in which collection holds the order:
//! - **Blocked**: in `blocked` (HashSet) and listed in some `blocked_by_sku[X]`
//! - **Ready**:   in `ready` (VecDeque, FIFO across stations)
//! - **Picking**: in `station_active[stn]` (Vec, FIFO continuation priority)
//! - **Complete**: present in `orders` only; removed from `station_active` on
//!   `complete_order`. Note: orders are never removed from `orders` itself.
//!
//! # `blocked_by_sku` is an inverted index
//!
//! `SKU → set of orders blocked on that SKU`. Exists so that when putaway
//! delivers SKU X, only orders depending on X are revisited (`recheck_
//! blocked_for_sku`), not the entire blocked set. Triggered by
//! `OrderPickDemand` polling `InventoryService::ledger_generation` and
//! calling recheck per dirty SKU.
//!
//! Set-valued (`BTreeSet`) for two reasons: (a) `(sku, order)` membership
//! has only two meaningful states, present or absent — bag semantics are
//! actively harmful, see Gotcha 1; (b) iteration is deterministic by
//! OrderId, useful for reproducible tests.
//!
//! # Gotchas
//!
//! 1. **Idempotent insert is load-bearing.** The else-branch of
//!    `recheck_blocked_for_sku` (still-blocked path) must re-add the order
//!    to other-blocking-SKU lists to handle "case 2": a SKU that was
//!    sufficient at `build()` can become blocking mid-flight as other
//!    orders' reservations consume projected inventory. Without idempotent
//!    insert, the common path (SKU already in list) creates duplicates;
//!    duplicates compound geometrically across rechecks (~2.6× per pass
//!    observed empirically on 10-SKU fixtures). With `BTreeSet`, case 2
//!    is handled and case 1 is a no-op. Do not switch this back to `Vec`.
//!
//! 2. **Stale entries on order completion.** When recheck on SKU X moves
//!    an order Blocked → Ready, its entries in `blocked_by_sku` for the
//!    *other* blocking SKUs are cleaned up at the if-branch. But if the
//!    order subsequently transitions through Picking → Complete and is
//!    later still listed in some `blocked_by_sku[Y]`, recheck on Y
//!    silently skips it (the `!self.blocked.contains(&order_id)` guard).
//!    With `BTreeSet`, this is a bounded leak — entries don't multiply,
//!    just linger until garbage-collected on the next recheck of that
//!    SKU. If putaway never delivers Y again, the entry stays forever.
//!    Acceptable for finite-order-set runs; revisit if used long-running.
//!
//! 3. **Recheck gating undercounts changes.** Recheck only fires when
//!    `inventory.ledger_generation()` advances, and the generation counter
//!    bumps only on putaway delivery (`record_inventory_added`), NOT on
//!    `reserve_for_order` or `record_pick_consumed`. But `has_sku_at_or_
//!    after` reads `inventory − reserved`. Consequence: an order whose
//!    `reserve_for_order` fails because another order reserved the
//!    inventory first won't trigger a recheck — it spins in `ready` and
//!    re-fails each demand cycle until next putaway delivery breaks the
//!    spin. Bounded retry waste in steady state; latent wedge if putaway
//!    quiesces with orders still un-served. See FIXME at
//!    `crate::model::inventory::InventoryService::ledger_generation`.
//!
//! 4. **`station_active.push` is non-idempotent.** `start_picking` assumes
//!    a single call per order. If multi-call ever becomes possible (e.g.,
//!    resume after partial abort), the same order would be listed twice
//!    in `station_active[stn]` and counted twice for continuation
//!    priority. Either guard with a presence check or convert to set-valued.
//!
//! 5. **Mixed readiness predicates.** `build()` uses `sku_available_at(sku, t)
//!    < qty` (qty-aware) to decide initial blocking; `recheck_blocked_for_sku`
//!    and `aggregated_demand` use `has_sku_at_or_after(sku, t)` (existence
//!    only). The discrepancy is bounded — once an order becomes Ready, the
//!    atomic `reserve_for_order` re-checks qty before claiming — but the
//!    asymmetry means an order can flip Blocked→Ready on existence and
//!    fail reservation on qty, falling back into the spin described in
//!    Gotcha 3.
//!
//! # Future work
//!
//! - For pharma DCs (cencora-real), the SKU reservation key needs to lift
//!   from `SkuId` to `(SkuId, Lot, ExpiryClass)` for FEFO compliance. The
//!   factorization stays; the lookup key gets richer.
//! - The inventory ledger's coarse generation gate should be replaced
//!   with per-SKU dirty tracking (TODO at
//!   `crate::model::inventory::InventoryService::ledger_generation`).
//!   Doing so independently fixes Gotchas 2 and 3.

use std::collections::VecDeque;

use desgraph::StationId;
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::model::SimTime;
use crate::model::destination::Destination;
use crate::model::ids::{OrderId, SkuId};
use crate::model::inventory::InventoryService;
use crate::model::order::{OrderService, OrderSpec};
use crate::model::station::StationSet;
use crate::scenario_gen::demand_pool::DistributionCenterId;

/// Per-line dispatch state within an order.
#[derive(Clone, Debug)]
pub struct LineState {
    pub sku: SkuId,
    pub qty: u32,
    /// Has this line's VtmForward been pushed?
    pub dispatched: bool,
}

/// Per-order dispatch state.
#[derive(Clone, Debug)]
pub struct OrderState {
    pub order_id: OrderId,
    pub station_id: Option<StationId>,
    pub lines: Vec<LineState>,
    /// Whether SKU reservation has been placed on InventoryService.
    pub reserved: bool,
    /// Simtime at which the ledger holds this order's inventory. Set by the
    /// bulk builder before enqueueing, or at admission for pre-formed orders.
    /// Every pick must start at or after this bound.
    pub reserved_at: Option<SimTime>,
    /// Forward distribution center. `Some` for bulk-mode orders; `None` for
    /// pre-formed scenarios. Propagated into the CycleComplete pick event.
    pub dc_id: Option<DistributionCenterId>,
    /// Pre-aggregated SKU demand across undispatched lines. Invariant:
    /// always equals `compute_demand(&lines)`. Maintained by `new` and
    /// `mark_dispatched`; the per-call HashMap rebuild used to be ~16% of
    /// total wall in the schedule loop.
    cached_demand: Vec<(SkuId, u32)>,
}

impl OrderState {
    pub fn new(order_id: OrderId, station_id: Option<StationId>, lines: Vec<LineState>) -> Self {
        let cached_demand = Self::compute_demand(&lines);
        Self { order_id, station_id, lines, reserved: false, reserved_at: None, dc_id: None, cached_demand }
    }

    fn compute_demand(lines: &[LineState]) -> Vec<(SkuId, u32)> {
        let mut demand: FxHashMap<SkuId, u32> = FxHashMap::default();
        for line in lines {
            if !line.dispatched {
                *demand.entry(line.sku).or_insert(0) += line.qty;
            }
        }
        demand.into_iter().collect()
    }

    /// Aggregate SKU demand across undispatched lines (for reservation check).
    /// Returns the cached aggregation — `O(1)`.
    pub fn aggregated_demand(&self) -> &[(SkuId, u32)] {
        &self.cached_demand
    }

    /// Flip a line to dispatched and refresh the demand cache.
    fn mark_dispatched(&mut self, line_idx: usize) {
        if line_idx < self.lines.len() && !self.lines[line_idx].dispatched {
            self.lines[line_idx].dispatched = true;
            self.cached_demand = Self::compute_demand(&self.lines);
        }
    }

    /// Next undispatched line index, if any.
    pub fn next_undispatched(&self) -> Option<usize> {
        self.lines.iter().position(|l| !l.dispatched)
    }

    /// Are all lines dispatched?
    pub fn fully_dispatched(&self) -> bool {
        self.lines.iter().all(|l| l.dispatched)
    }
}

/// Order lifecycle phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderPhase {
    /// Waiting for SKU availability.
    Blocked,
    /// All SKUs available, waiting for scheduling turn.
    Ready,
    /// First pick dispatched, SKU reservation placed. Remaining picks
    /// dispatch as totes become available.
    Picking,
    /// All picks consumed.
    Complete,
}

/// Order-level dispatch queue. See module docs for the two-stage allocation
/// design, lifecycle, and gotchas.
pub struct LineDispatch {
    pub sku: SkuId,
    pub qty: u32,
    pub dc: Option<DistributionCenterId>,
    pub first: bool,
    pub last: bool,
}

pub struct OrderReadyQueue {
    /// All order dispatch states, keyed by OrderId. Entries are never
    /// removed; lifecycle phase is encoded by membership in the other
    /// collections.
    pub orders: FxHashMap<OrderId, OrderState>,

    /// Orders ready for dispatch (FIFO across stations).
    pub ready: VecDeque<OrderId>,

    /// Orders blocked on SKU availability. Canonical "is this order
    /// blocked?" set; the inverted index `blocked_by_sku` is derived.
    pub blocked: FxHashSet<OrderId>,

    /// SKU → set of orders blocked on that SKU. Inverted index for
    /// incremental rechecks when putaway delivers. Set-valued because
    /// `(sku, order)` membership is set-valued, not bag-valued — see
    /// module docs Gotcha 1; non-idempotent push compounds geometrically.
    /// `recheck_blocked_for_sku` sorts the drained set once, preserving
    /// the OrderId-deterministic iteration the consumer relies on.
    pub blocked_by_sku: FxHashMap<SkuId, FxHashSet<OrderId>>,

    /// Per-station: in-flight orders (Picking phase). FIFO continuation
    /// priority — their next undispatched line goes before any new order
    /// at the same station. Note: `start_picking` push is non-idempotent
    /// (Gotcha 4); single-call-per-order is required.
    pub station_active: FxHashMap<StationId, Vec<OrderId>>,
}

impl OrderReadyQueue {
    /// Build from OrderService and InventoryService.
    ///
    /// Classifies each pick-station order as Ready or Blocked based on
    /// current SKU availability. Putaway-station orders are not tracked
    /// here (putaway has its own manifest-driven dispatch).
    pub fn build(orders: &OrderService, inventory: &InventoryService, stations: &StationSet) -> Self {
        let mut queue = Self {
            orders: FxHashMap::default(),
            ready: VecDeque::new(),
            blocked: FxHashSet::default(),
            blocked_by_sku: FxHashMap::default(),
            station_active: FxHashMap::default(),
        };

        // Initialize station_active for all pick stations.
        for station in stations.pick_stations() {
            queue.station_active.insert(station.id, Vec::new());
        }

        // Build per-order state straight off the registry. Pick-station
        // ownership is bound only when a station starts the order.
        orders.with(|orders| {
            for order in orders.values() {
                let order_id = order.id;
                let line_states: Vec<LineState> =
                    order.pick_units().map(|(_, sku)| LineState { sku, qty: 1, dispatched: false }).collect();
                // Nothing to pick is not work: queued, it would be ready forever and
                // hold completion open.
                if line_states.is_empty() {
                    continue;
                }

                let order_state = OrderState::new(order_id, None, line_states);

                // Check readiness: will all SKUs ever become available?
                // Forward-looking: putaway may add inventory at T>0.
                let is_ready = order_state
                    .aggregated_demand()
                    .iter()
                    .all(|&(sku, qty)| inventory.has_sku_qty_at_or_after(sku, qty, SimTime::ZERO));

                if is_ready {
                    queue.ready.push_back(order_id);
                } else {
                    queue.blocked.insert(order_id);
                    // Index by blocking SKUs for re-check.
                    for &(sku, qty) in order_state.aggregated_demand() {
                        if inventory.sku_available_at(sku, SimTime::ZERO) < qty as isize {
                            queue.blocked_by_sku.entry(sku).or_default().insert(order_id);
                        }
                    }
                }
                queue.orders.insert(order_id, order_state);
            }
        });

        queue
    }

    /// Re-check blocked orders that need `sku`. Called when
    /// sku_inventory increments (putaway completion).
    ///
    /// Orders that are now ready move from blocked to ready.
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    pub fn recheck_blocked_for_sku(&mut self, sku: SkuId, inventory: &InventoryService, at: SimTime) {
        // Drain the set into a Vec and sort by OrderId so iteration is
        // deterministic — `ready` is a FIFO queue and downstream tests
        // rely on the order in which we push.
        let mut affected: Vec<OrderId> = match self.blocked_by_sku.remove(&sku) {
            Some(ids) => ids.into_iter().collect(),
            None => return,
        };
        affected.sort_unstable();

        let mut still_blocked_for_sku = Vec::new();

        for order_id in affected {
            if !self.blocked.contains(&order_id) {
                continue; // already moved to ready by a prior re-check
            }

            let demand = match self.orders.get(&order_id) {
                Some(o) => o.aggregated_demand(),
                None => continue,
            };

            let is_ready = demand.iter().all(|&(sku, qty)| inventory.has_sku_qty_at_or_after(sku, qty, at));

            if is_ready {
                // Drop the `demand` borrow before mutating `self.blocked`/etc.
                // by copying the few SKUs we still need into a stack buffer.
                let other_skus: SmallVec<[SkuId; 8]> =
                    demand.iter().filter_map(|&(s, _)| (s != sku).then_some(s)).collect();
                self.blocked.remove(&order_id);
                self.ready.push_back(order_id);
                // Clean up other blocked_by_sku entries for this order.
                for other_sku in other_skus {
                    if let Some(set) = self.blocked_by_sku.get_mut(&other_sku) {
                        set.remove(&order_id);
                    }
                }
            } else {
                // Still blocked — re-index for this SKU plus any newly-blocking
                // SKUs (case 2: a SKU sufficient at build can become blocking
                // mid-flight as other orders' reservations consume projected
                // inventory). Set-valued insert makes case 1 (already listed)
                // a no-op. See module docs Gotcha 1.
                let newly_blocking: SmallVec<[SkuId; 8]> = demand
                    .iter()
                    .filter_map(|&(s, q)| (s != sku && !inventory.has_sku_qty_at_or_after(s, q, at)).then_some(s))
                    .collect();
                still_blocked_for_sku.push(order_id);
                for other_sku in newly_blocking {
                    self.blocked_by_sku.entry(other_sku).or_default().insert(order_id);
                }
            }
        }

        if !still_blocked_for_sku.is_empty() {
            self.blocked_by_sku.entry(sku).or_default().extend(still_blocked_for_sku);
        }
    }

    /// Mark an order as entering Picking state. Called when the order's
    /// first VtmForward is about to dispatch.
    pub fn start_picking(&mut self, order_id: OrderId, station_id: StationId, reserved_at: SimTime) {
        if let Some(order) = self.orders.get_mut(&order_id) {
            order.reserved = true;
            order.reserved_at = Some(reserved_at);
            order.station_id = Some(station_id);
            self.station_active.entry(station_id).or_default().push(order_id);
        }
    }

    /// Dispatch one line: mark it, and when it was the order's last
    /// undispatched line, retire the order from the active set. `first`
    /// reports that no line of the order had been dispatched before.
    pub fn dispatch_line(&mut self, order_id: OrderId, line_idx: usize) -> Option<LineDispatch> {
        let (sku, qty, dc, first, last) = {
            let order = self.orders.get_mut(&order_id)?;
            let first = order.lines.iter().all(|line| !line.dispatched);
            let state = order.lines.get(line_idx)?;
            if state.dispatched {
                return None;
            }
            let (sku, qty) = (state.sku, state.qty);
            order.mark_dispatched(line_idx);
            (sku, qty, order.dc_id.clone(), first, order.fully_dispatched())
        };
        if last {
            self.complete_order(order_id);
        }
        Some(LineDispatch { sku, qty, dc, first, last })
    }

    /// Mark an order as complete. Removes from station_active.
    pub fn complete_order(&mut self, order_id: OrderId) {
        if let Some(order) = self.orders.get(&order_id) {
            if let Some(station_id) = order.station_id
                && let Some(active) = self.station_active.get_mut(&station_id)
            {
                active.retain(|&id| id != order_id);
            }
        }
    }

    /// Insert a freshly-built, pre-reserved order from the bulk-mode
    /// `OrderBuilder`. The builder has already called `reserve_for_order`
    /// on `inventory` at `reservation_time`, so dispatch must skip
    /// re-reservation and launch chains starting at `reservation_time`.
    ///
    /// Unlike [`Self::build`] there is no Ready/Blocked classification:
    /// the order's inventory is *guaranteed* simultaneously available at
    /// `reservation_time` (that's what `find_earliest_pallet` proved and
    /// what the reservation locked in), so the order goes straight into
    /// `ready`.
    /// Admit an order the demand pool built mid-run. It is a real `OrderSpec`,
    /// so unlike the pre-formed path it arrives with its traits intact.
    pub fn enqueue_reserved(&mut self, order: &OrderSpec, reservation_time: SimTime) {
        if self.orders.contains_key(&order.id) {
            panic!("OrderReadyQueue::enqueue_reserved: duplicate order_id {:?}", order.id);
        }

        let line_states: Vec<LineState> =
            order.pick_units().map(|(_, sku)| LineState { sku, qty: 1, dispatched: false }).collect();
        let mut order_state = OrderState::new(order.id, None, line_states);
        order_state.reserved = true;
        order_state.reserved_at = Some(reservation_time);
        order_state.dc_id = order.destination.as_ref().map(|Destination::DistributionCenter(dc)| dc.clone());

        self.ready.push_back(order.id);
        self.orders.insert(order.id, order_state);
    }
}
