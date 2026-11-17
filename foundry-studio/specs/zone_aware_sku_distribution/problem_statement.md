# Zone-Aware SKU Distribution for VTM Load Balancing

## Problem Statement

In the relay system, VTMs (Vertical Tote Movers) are zone-bound robots responsible for retrieving totes from racks within their assigned aisles. The current SKU placement algorithm (inherited from RTP system) does not distribute work evenly across zones, resulting in:

- **Front zone VTMs**: Overloaded (handling popular Category A/B SKUs)
- **Back zone VTMs**: Idle (handling only slow-moving Category C SKUs)
- **Inefficient utilization**: Some VTMs have no work while others are constantly busy

## Root Cause Analysis

### Current SKU Placement Behavior

The `PutProcessor.initializeInventoryGreedy()` algorithm (lines 212-277) places SKUs based on **average distance to stations**:

1. Storage spaces sorted by distance (closest first)
2. Popular SKUs (Category A) placed in closest racks → **front zones**
3. Medium SKUs (Category B) placed in mid-distance racks → **middle zones**
4. Slow SKUs (Category C) placed in farthest racks → **back zones**

**Result:** Zone workload is directly proportional to SKU popularity in that zone.

### Why This Matters

- **VTMs are zone-bound** - a VTM in a back zone cannot help front zones
- **Order selection doesn't consider zone balance** - `OrderProcessor.selectNextOrders()` uses set cover algorithm optimized for minimizing storage spaces, not zone distribution
- **Static placement** - Once SKUs are placed at initialization, they remain in those zones

## Proposed Solution

### Two-Pronged Approach

**1. Smart Initial Placement (Option A - Zone-Aware Greedy)**
- Duplicate popular SKUs across multiple zones at initialization
- Distribution based on SKU category and demand

**2. Dynamic Return Routing (Option B - Zone-Aware Return)**
- When totes return from picking stations, route them to underutilized zones
- Preserves SKU contents (no refilling), just reassigns storage location

---

## Design Decisions

### 1. Zone Distribution Strategy (Initial Placement)

**Hybrid Approach: Tier-Based + Demand-Driven**

```
For each SKU:
  Category A (popular, from ItemCatalog.getPopularItems()):
    N = ALL zones (100% coverage)

  Category B/C (non-popular):
    N = ceil(pastOrderedQuantity / avg_picks_per_zone_capacity)
    Clamped to: max(1, min(N, total_zones))
```

**Zone Selection Algorithm: Round-Robin**

Distribute SKUs sequentially across zones in circular pattern:

```
Example with 4 zones:
  SKU1 (A, N=4): zones [0, 1, 2, 3]  ← all zones
  SKU2 (A, N=4): zones [0, 1, 2, 3]  ← all zones
  SKU3 (B, N=2): zones [0, 1]        ← start at 0
  SKU4 (B, N=2): zones [2, 3]        ← continue from 2
  SKU5 (B, N=2): zones [0, 1]        ← wrap to 0
  SKU6 (C, N=1): zones [2]           ← continue from 2
```

**Rationale:**
- Guarantees uniform distribution across zones
- Simple, predictable, deterministic
- Directly addresses VTM workload balancing

**Alternative Considered: Bin-Packing**
- More sophisticated (optimize for capacity balance, SKU complementarity)
- Rejected for initial implementation due to complexity
- Can be added later if round-robin proves insufficient

---

### 2. Zone Utilization Metric

**Metric: Pending Task Count per Zone**

Track number of tasks waiting in each zone's queue:
- Simple, directly measures workload
- Easy to implement (count tasks assigned to zone's storage spaces)
- Real-time metric (updates as tasks assigned/completed)

**Alternatives Considered:**
- VTM idle time tracking (more direct but requires overhead)
- Relay position queue depth (doesn't isolate VTM utilization)

---

### 3. Return Journey Routing Logic

When `DesmojVtmProcess` completes picking and returns tote:

**Algorithm:**
```java
1. Select target zone:
   - Calculate pending task count per zone
   - Choose zone with LOWEST pending task count (underutilized)

2. Find rack in target zone:
   a. Try to find rack that already contains ANY of tote's SKUs (consolidation)
   b. If no matching rack or no capacity, find any low-utilization rack
   c. If target zone has no space, try second-least utilized zone
   d. Fallback: return to original zone

3. Route tote to selected rack
```

**Key Principle:** Preserve SKU contents (no refilling), just reassign storage location.

---

### 4. Edge Cases

**Multi-SKU Totes (MAX_SKUS_PER_TOTE = 2):**
- When finding matching rack, check if rack contains **ANY** of the tote's SKUs
- Partial match is acceptable (e.g., tote has [SKU_A, SKU_B], rack has only SKU_A)

**Tote SKUs Not in Target Zone:**
- If tote's SKUs were never initialized in target zone, place in any available rack
- This naturally expands SKU distribution over time (adaptive rebalancing)

**All Zones Equally Utilized:**
- If pending task counts are equal, fall back to round-robin or nearest zone

**Zone Has No Available Racks:**
- Try next-least utilized zone
- Ultimate fallback: return to original zone (preserve correctness)

---

## Implementation Scope

### Files to Modify

1. **`PutProcessor.java`** (apps/backend-java/.../domain/put/)
   - Modify `initializeInventoryGreedy()` to implement zone-aware distribution
   - Add zone assignment logic (round-robin)
   - Track SKU-to-zones mapping

2. **`DesmojVtmProcess.java`** (apps/backend-java/.../simulator/orchestrator/)
   - Modify return journey logic in `lifeCycle()`
   - Add zone utilization check
   - Implement rack selection in target zone

3. **`ZoneManager.java`** (apps/backend-java/.../domain/relay/)
   - Add method: `getPendingTaskCountPerZone()`
   - Add method: `selectUnderutilizedZone()`

4. **`WarehouseInventory.java`** (apps/backend-java/.../domain/inventory/)
   - Add method: `findRackWithAnySku(zone, skuSet)` - find racks in zone containing any SKU
   - Add method: `findLeastUtilizedRack(zone)` - find rack with most free capacity

### New Data Structures

```java
// In PutProcessor or new ZoneDistributionManager
Map<String, List<Integer>> skuToZones;  // SKU → assigned zone IDs
Map<Integer, Set<String>> zoneToSkus;   // Zone ID → SKUs in that zone

// In ZoneManager or DesmojWarehouseModel
Map<Integer, Integer> zonePendingTasks; // Zone ID → pending task count
```

### Metrics to Track

Add new metric events:
- `TOTE_REASSIGNED_TO_ZONE`: Track when tote is routed to different zone
- `ZONE_UTILIZATION_SNAPSHOT`: Periodic snapshot of pending tasks per zone

Enhance existing collectors:
- `VtmCollector`: Add per-zone idle time tracking
- `ToteCollector`: Add zone reassignment count

---

## Success Criteria

1. **VTM Idle Time Variance**: Reduce variance in idle time across VTMs by >50%
2. **Zone Utilization Balance**: All zones maintain 70-100% utilization during peak periods
3. **SKU Coverage**: Category A SKUs present in 100% of zones
4. **Picks Per Face**: Maintain or improve (consolidation via matching rack preference)

---

## Testing Strategy

### Unit Tests
- Zone assignment algorithm (verify round-robin distribution)
- Pending task count calculation
- Rack selection logic (matching vs fallback)

### Integration Tests
- Run relay simulation with zone-aware distribution enabled
- Compare VTM idle times across zones
- Verify tote reassignment counts

### Validation
- Run 4-hour simulation with existing algorithm (baseline)
- Run 4-hour simulation with zone-aware distribution (comparison)
- Metrics: VTM idle time per zone, picks per hour per zone, total throughput

---

## Open Questions

1. **Zone Count Calculation for B/C SKUs:**
   - What should `avg_picks_per_zone_capacity` be?
   - Option: `(totes_per_zone * picks_per_tote) / simulation_duration`
   - Need empirical data to calibrate

2. **Reassignment Frequency:**
   - Should we limit how often a tote can be reassigned to prevent thrashing?
   - E.g., "Max 1 reassignment per tote per N cycles"

3. **Multi-Zone Storage Space Tracking:**
   - If a SKU is duplicated across zones, how do we track which zone's inventory was consumed?
   - Does `WarehouseInventory.tryReserveForOrder()` need zone awareness?

4. **Performance Impact:**
   - Does checking pending tasks per zone on every return journey add significant overhead?
   - Should we cache zone utilization metrics and update periodically?

---

## Next Steps

1. Create detailed implementation plan with task breakdown
2. Implement zone-aware initial placement (PutProcessor changes)
3. Implement return journey routing (DesmojVtmProcess changes)
4. Add metrics and logging for debugging
5. Run validation simulations (baseline vs zone-aware)
6. Tune parameters based on results (zone count formula, reassignment policy)

---

## References

- Current greedy algorithm: `PutProcessor.java:86-290`
- VTM process: `DesmojVtmProcess.java`
- Zone management: `ZoneManager.java`
- Set cover order selection: `OrderProcessor.java`
