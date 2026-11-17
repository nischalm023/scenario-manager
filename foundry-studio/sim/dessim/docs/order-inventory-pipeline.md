# Order/Inventory/Dispatch Pipeline

## Pipeline Overview

```
ScenarioGen                OrderManager              Dispatcher              Robots
    │                          │                         │                     │
    ├─ orders, inventory ─────>│                         │                     │
    │                          │                         │                     │
    │               Phase 1: Assignment                  │                     │
    │                ┌─────────┴──────────┐              │                     │
    │                │ score_order_for_   │              │                     │
    │                │ station() [sum]    │              │                     │
    │                │ reserve_inventory()│              │                     │
    │                │ compute_schedule() │              │                     │
    │                │ [LFF ordering]     │              │                     │
    │                └─────────┬──────────┘              │                     │
    │                          │ PickTasks               │                     │
    │                          ├────────────────────────>│                     │
    │                          │                         │                     │
    │               Phase 2: Dispatch (at scheduled time)│                     │
    │                          │                    find idle HTM              │
    │                          │                         ├─ HtmPushAssignment─>│
    │                          │                         │                     │
    │                          │                         │        HTM travels  │
    │                          │                         │        to relay,    │
    │                          │                         │        loads tote,  │
    │                          │                         │        delivers to  │
    │                          │                         │        station      │
    │                          │                         │                     │
    │                          │<── complete_tasks_batch ─┤<── pick complete ──│
    │                          │                         │                     │
    │               Triggers Phase 1 if station needs    │                     │
    │               more orders                          │                     │
```

## Entity Capacity Model

| Entity | Capacity | Queue? | Notes |
|--------|----------|--------|-------|
| Station (OperatorManager) | `num_bins` concurrent (default 2) | Unbounded FIFO wait_queue | Multiple HTMs can be at station; excess FIFO-queued |
| HTM | 1 tote, 1 assignment | No | Idle or busy, single state machine |
| VTM | 1 tote, 1 assignment | No | Idle or busy, single state machine |
| Relay position | 1 tote, 1 reservation (mutex) | No | VTM picks another relay if taken |
| Rack cell | `vertical_tote_capacity` slots (default 8) | No VTM contention | Multiple z-indices per column |
| StationPipeline | `max_inflight_picks` orders | VecDeque of ScheduledPick | Controls dispatch rate |

## Two-Phase Dispatch

### Phase 1 — Assignment (OrderManager)

Triggered by: order completion (T1), new order pending (T3), periodic safety (T5).

For each station with `assigned_count < max_inflight_picks`:
1. Score all pending orders: `score = Σ min_manhattan(sku, station)`
2. Pick lowest-score order (station rotation for fairness)
3. Reserve inventory (all-or-nothing set-cover via `try_reserve_for_order()`)
4. Sort picks by descending F_est (LFF — critical-path first)
5. Compute per-pick: `target_arrival`, `dispatch_time = target_arrival - F_est`
6. Push ScheduledPick entries into station's pipeline
7. Update `next_free_time += N * P` (N picks, P = dwell time per pick)

Inventory is committed here. HTM is NOT committed yet.

### Phase 2 — Dispatch (scheduled events)

At each computed `dispatch_time`:
1. Find nearest idle HTM to the tote's storage/relay position
2. If HTM available: send HtmPushAssignment, mark pick as dispatched
3. If no HTM available: mark as overdue — T4 trigger retries when HTM frees

HTM is committed here. Inventory was already committed in Phase 1.

## queue_factor Semantics

Controls arrival timing at the station:
- `1.0` = next HTM arrives at the exact instant current departs (zero gap)
- `>1.0` = arrives early (buffer). E.g., `1.2` = 0.2*P early
- `<1.0` = arrives late (gap). E.g., `0.9` = 0.1*P late
- `2.0` = one HTM always waiting in queue

Formula:
```
target_arrival = previous_departure - (queue_factor - 1.0) * P
dispatch_time  = target_arrival - F_est
```

## Scoring: Sum vs Max

- **Sum** of min_manhattan: station ASSIGNMENT — which station gets the order
  (minimizes total HTM travel across all picks)
- **Max** of min_manhattan: F_est — critical path / order lead time
  (the longest-travel pick determines when the order can start)

Order-level scoring controls HTM delivery destination. The Dispatcher's HTM
matching only controls which HTM picks up the tote — it cannot change where
the tote goes. Scoring is therefore critical for minimizing delivery distance.

## Throughput Bounds (cencora_phase0)

| Bound | Value | Constraint |
|-------|-------|-----------|
| Station-bound max | 4,000 pres/hr | 10 stations × 400 pres/hr (P=9s) |
| HTMs needed for saturation | ~39 | Each HTM ~ 103 pres/hr (F~35s round trip) |
| Fleet size | 400 HTMs | ~10x over-provisioned |
| max_inflight_picks | ~9 | ceil(F_max/P)+2 = ceil(35/9)+2 |
| VTMs needed (full mode) | ~44 | Each VTM ~ 72 pres/hr (F~50s round trip) |

Pipeline depth (assignment depth × queue_factor interaction) is the actual
throughput bottleneck — not fleet size, not relay capacity.

## Nearest Map Invalidation

After `reserve_order_for_station()` depletes a (SKU, space) to available()==0:
- Remove that entry from the `nearest` BTreeMap
- Recompute min distance for affected (SKU, station) pairs
- Invalidation is monotonic — entries are only removed, never re-added
  (no reservation release in current model)
