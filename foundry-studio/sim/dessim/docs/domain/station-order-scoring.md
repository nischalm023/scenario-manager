# Station-Order Scoring: Nearest-to-Station per SKU

## Problem

When assigning an order to a station, estimate HTM travel cost to bias toward the station with the shortest total trip distance. Each SKU line = one tote fetch = one HTM round trip.

## Inventory Distribution (cencora_phase0)

The `generate_relay_inventory()` round-robin cursor places:
- **Popular SKUs** (100): 400 units each → 40 contiguous tote slots (~2 relay zones)
- **Unpopular SKUs** (400): 25 units each → 3 contiguous slots (~1 zone)
- **Total**: 720 slots, ~7.2 SKU-slot pairs per slot (cursor wraps 5.56×)
- **Typical order**: 2 lines (1 popular @ 4 picks, 1 unpopular @ 1 pick)

## Algorithm

```
score(order, station) = Σ over SKU lines:  min_distance(sku, station)

min_distance(sku, station) = min over spaces with stock for sku:
    |space.x - station.x| + |space.y - station.y|
```

Assign order to station with lowest score among those with pipeline capacity (available slots).

Precompute `nearest: BTreeMap<(SkuId, StationId), u32>` at init. Lazily invalidate when a space's available stock hits zero.

## Why Not Alternatives

| Approach | Problem |
|----------|---------|
| All-candidates sum | Over-weights popular SKUs (40 terms vs 3). Each line = 1 trip, so each should contribute 1 distance. |
| Weighted centroid | Center of a 40-slot band doesn't correspond to any actual tote. |
| Most-stock candidate | 10 units/slot cap → many ties → effectively random selection. |
| Zone-level aggregation | Loses intra-zone precision. Unnecessary indirection when per-space data is cheap. |

## Cost

| Phase | Work | Time |
|-------|------|------|
| Init precomputation | 500 SKUs × 10 stations × ~25 spaces = 125K distance ops | ~0.1ms |
| Per-order scoring | 2 lines × 10 stations = 20 lookups | ~ns |
| Per-cycle (200 orders) | 3,200 lookups | <0.01ms |

Scales linearly in SKU line count — no combinatorial blowup even for future multi-line orders.

## Sum vs Max Distinction

- **Sum of min_manhattan** is used for **station assignment**: "which station should process this order?" Minimizes total HTM travel across all picks. This is the primary scoring function in `score_order_for_station()`.
- **Max of min_manhattan** is used for **F_est** (critical path): "what is the longest fetch time for any pick in this order?" The max-F pick determines the order's lead time — when the first HTM must be dispatched, and thus the order's start time at the station.
- Both use the same precomputed `nearest` map but serve different purposes.

### Nearest Map Invalidation

After `reserve_order_for_station()` (formerly `try_reserve_for_order()`) reserves stock that depletes `available()` to 0 for a (SKU, space) pair, that entry must be removed from the nearest map and the minimum recomputed. Without invalidation, scoring uses stale data leading to suboptimal station assignments as stock depletes.

## Relationship to Set-Cover

The greedy set-cover selects totes by max SKU coverage, not proximity. The actual tote may not be the nearest, but the station ranking is preserved: the station with the closest inventory for each SKU will still score best regardless of which specific tote the set-cover picks.
