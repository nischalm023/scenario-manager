# HTM Dispatch Choreography

How HTMs are dispatched to stations so that every tote arrives just in time — despite wildly differing journey times from relays across the warehouse. The overarching goal is **choreography**: a continuous flow of tote presentations with zero station idle gaps, achieved by coordinating pipeline depth, order bins, and presentation slots.

**Reading order:** Start here for the conceptual model, then see `orchestration-model.md` for robot timing and relay coordination, and `picking-workflow.md` for inventory reservation and set cover.

---

## Overview

Each station has a continuous pipeline of HTM deliveries. The goal is perfect choreography: as one HTM finishes presenting, the next is already waiting — regardless of whether its tote traveled 15 seconds or 140 seconds from its relay. Three independent resource constraints serve the choreography:

| Resource | Capacity | When Claimed | Managed By |
|----------|----------|--------------|------------|
| **Pipeline picks** | `max_inflight_picks` per station | At dispatch time | OrderManager |
| **Order bins** | `num_bins` per station (typically 2) | At first HTM arrival | BinManager |
| **Presentation cell** | 1 per access cell | At presentation time | OperatorManager |

An HTM must pass through all three gates to present at a station. The three resources are decoupled: an HTM can be dispatched (consuming pipeline budget) for an order that has no bin yet, and can arrive at the station before the presentation cell is free.

---

## Pipeline Depth (max_inflight_picks)

The **pipeline depth** (`max_inflight_picks`) is the maximum number of in-flight picks (dispatched but not yet completed) per station. It is a safety ceiling, not a target. It is computed as:

```
effective_P = pick_time_secs + transition_time_secs   (default: 9s + 3s = 12s)
max_inflight_picks = ceil(F_max / effective_P) + 2
```

Where `F_max` is the worst-case one-way fetch time from any relay to this station (per-station, not global), `pick_time_secs` is the operator dwell time (default 9s), and `transition_time_secs` is the bot-to-bot handoff gap at the station (default 3s). The `+2` compensates for F_est underestimate (the HTM-to-relay leg, ~10s, is not modeled in v1).

For cencora: F_max ~ 90s, effective_P = 12s, so max_inflight_picks = ceil(90/12) + 2 = 10.

The pipeline depth acts as a **budget**. Each dispatched pick consumes one unit of budget. When the budget is exhausted, no more picks are dispatched for that station until existing picks complete and free budget. Budget is released per-pick (not per-order): each completed pick immediately frees one unit.

## Queue Factor (queue_factor)

The **queue factor** (`queue_factor`, f64, default 1.0) controls arrival timing at the station. It specifies the target number of arrived-but-not-yet-presented HTMs at the station, measured at the instant the currently-presenting HTM completes and departs.

- `queue_factor=1.0`: 1 HTM arrived at departure instant (zero wait, zero gap -- theoretical ideal)
- `queue_factor=2.0`: 2 HTMs queued at departure (buffer against variance)
- `queue_factor=0.5`: No HTM yet; arrives 0.5*effective_P later (deliberate idle gap)

The queue factor adjusts target arrival times:

```
target_arrival(i) = next_free_time + i * effective_P - (queue_factor - 1) * effective_P
dispatch_time(i)  = target_arrival(i) - F_est(relay_i, station)
```

Production systems typically run `queue_factor=1.1-1.3`. The value `1.0` is the theoretical ideal for zero-variance simulation.

---

## Order Bins

An **order bin** is a physical slot at the station where the operator accumulates items for a single customer order. Each station has `num_bins` bins (read from the map's per-station `bins` field; cencora uses 2).

### Bin Lifecycle

1. **Unassigned**: The bin is empty and available.
2. **Assigned**: An order occupies this bin. Items from incoming totes are placed into it.
3. **Released**: All picks for the order are complete. The bin is freed for the next order.

An order holds its bin from first pick to last pick. Multi-tote orders (where items arrive on separate HTMs) hold the bin across all tote presentations — the bin accumulates items from multiple totes into one outbound order.

### Bin Assignment is Lazy

Bins are assigned **at the moment the first HTM for that order arrives at the station**, not at dispatch time. This maximizes pipeline flexibility: HTMs can be dispatched and in-transit for orders that don't yet have bins. The bin is claimed on arrival if one is free.

### Bin Count Limits Concurrent Orders

At most `num_bins` orders can be simultaneously in-progress (bin-assigned) at a station. This is a hard constraint separate from pipeline depth. With `num_bins = 2`, at most 2 orders have bins at any time, even though the pipeline may have picks for additional orders in transit.

---

## Dispatch Choreography Algorithm

Dispatch runs when orders complete (freeing pipeline budget), on a periodic safety net, or at bootstrap. The algorithm orchestrates HTM launches so that arrivals are spaced exactly P seconds apart at the station, despite varying relay distances. For each station:

### Phase 1: Fill to `num_bins` Distinct Orders

The first priority is ensuring `num_bins` distinct orders are in-flight for each station. This is breadth-first at the order level: pick the best-scoring pending order, dispatch all its picks, then pick the next order.

```
active_order_count = count of dispatched-but-incomplete orders for this station
budget = max_inflight_picks - in_flight_picks

while active_order_count < num_bins and budget > 0 and pending_orders exist:
    order = best_scoring_pending_order(station)   # proximity scoring
    reserve_inventory(order)
    dispatch_all_picks(order, station)
    active_order_count += 1
    budget -= order.pick_count
```

The budget check prevents runaway dispatch, but the primary constraint in Phase 1 is reaching `num_bins` distinct orders.

### Phase 2: Speculative Pre-Dispatch

Once `num_bins` orders are dispatched, the system looks ahead. If a bin is predicted to become free before a new HTM could arrive, it speculatively dispatches the next order's HTMs so they arrive just-in-time when the bin frees.

```
while budget > 0 and pending_orders exist:
    order = best_scoring_pending_order(station)
    f_est = estimated_fetch_time(order, station)
    soonest_bin_free = min(remaining_picks(o) * P for o in binned_orders)

    if soonest_bin_free <= f_est:
        # Bin will free before HTM arrives — dispatch speculatively
        reserve_inventory(order)
        dispatch_all_picks(order, station)
        budget -= order.pick_count
    else:
        break   # no bin predicted to free in time
```

The speculative condition prevents dispatching HTMs that would wait too long in staging.

### Dispatch-All-Picks-Per-Order

When an order is selected for dispatch, **all its picks are dispatched at once**. This is a deliberate simplification: it avoids tracking which picks within an order have been dispatched vs. pending. The trade-off is that all HTMs for one order launch simultaneously, creating a small cluster of arrivals. In practice, most orders have 1-3 picks (totes), so the cluster is small.

Within the batch, picks are sorted by **Longest-Fetch-First (LFF)** ordering: the pick from the most distant relay dispatches first. This ensures the longest-traveling HTM departs first and arrivals are spread more evenly.

---

## Two Queues at Each Station

When an HTM arrives at a station, it passes through two sequential queues:

### 1. Staging Queue (Bin Gate)

An HTM whose order does not yet have a bin enters the **staging queue**. It waits here until a bin becomes available. When an order completes and its bin is released:

1. The first staged HTM's order receives the freed bin (lazy assignment).
2. All staged HTMs for that order move from staging to the presentation FIFO.

### 2. Presentation FIFO (Access Cell Gate)

An HTM whose order has a bin enters the **presentation FIFO**. This is the existing OperatorManager queue. HTMs present one at a time per access cell (capacity 1). When the current HTM finishes presenting and departs, the next in line presents.

### Flow

```
HTM arrives at station
    │
    ▼
Order has bin? ──No──► Staging Queue ──(bin frees)──► Presentation FIFO
    │                                                       │
    Yes                                                     │
    │                                                       │
    ▼                                                       ▼
Presentation FIFO ──(access cell free)──► Present to Operator
```

---

## Presentation Slot

The **presentation slot** is the physical docking position (access cell) where an HTM presents its tote to the operator. Each access cell supports exactly one HTM at a time. This is a physical constraint tied to the station's geometry — one presentation slot per access cell, regardless of how many bins the station has.

The presentation slot serializes tote processing: even if multiple HTMs are ready (for different orders with different bins), only one presents at a time. The operator picks from one tote, the HTM departs, and the next HTM presents.

---

## Order Scoring

Orders are scored for each candidate station using **proximity scoring**: the sum of nearest manhattan distances from each order line's closest inventory location to the station. Lower score = better fit (items are physically closer to the station).

```
score(order, station) = sum over lines: min_manhattan(sku, station)
```

This ensures orders are routed to stations near their inventory, minimizing HTM travel distance.

---

## Arrival Choreography: Compensating for Distance

The core choreography insight: an HTM fetching from a distant relay (F=60s) and an HTM fetching from a nearby relay (F=15s) must both arrive at P-spaced intervals. The system achieves this by dispatching the distant HTM earlier. Within a dispatched batch of picks (all for one order), each pick has a computed target arrival time and dispatch time:

```
target_arrival(i) = next_free_time + i * P
dispatch_time(i) = target_arrival(i) - F_est(relay_i, station)
```

Where `next_free_time` is when the station will next be free (after processing all previously scheduled picks), and `F_est` is the estimated one-way fetch time for this pick's relay.

Picks with `dispatch_time <= now` dispatch immediately. Picks with `dispatch_time > now` are deferred and fire via scheduled events at the computed time.

LFF ordering ensures that the pick with the longest travel time dispatches first (its `dispatch_time` is earliest), giving it the most lead time. Shorter-fetch picks dispatch later but arrive on schedule because their shorter travel compensates for the later start.

---

## Steady-State Choreography

At steady state with 2 bins per station:

1. **Order A** is in-progress (has bin 0). Its HTMs present one by one.
2. **Order B** has been dispatched. Its HTMs are in transit. When order A's last pick completes, bin 0 frees. Order B's first HTM (already arrived, in staging or presentation FIFO) gets the bin and presents immediately — zero transition gap.
3. **Order C** may be speculatively dispatched if B is nearly done and the pipeline depth budget allows.

This creates a continuous overlapping pipeline:

```
Time ──────────────────────────────────────────────►

Order A:  [dispatch]═══════[present][present][present][done]
Order B:         [dispatch]════════════════[present][present][done]
Order C:                        [dispatch]═══════════════[present]...

Station:  ─────────────────[A1][A2][A3][B1][B2][C1]────────
                            no gaps between presentations
```

---

## Key Invariants

1. **Pipeline depth cap**: `in_flight_picks <= max_inflight_picks` per station (enforced at dispatch time).
2. **Bin constraint**: `in_progress_orders <= num_bins` per station (enforced at presentation time via lazy bin assignment).
3. **Presentation serialization**: At most 1 HTM presenting per access cell (enforced by OperatorManager).
4. **All-or-nothing inventory**: An order's inventory is fully reserved at dispatch or not at all.
5. **Bin holds across totes**: A multi-tote order holds its bin for all tote presentations until all picks complete.
6. **LFF dispatch ordering**: Within an order, longest-fetch picks dispatch first.
7. **Lazy bin assignment**: Bins are assigned at first HTM arrival, not at dispatch time.
