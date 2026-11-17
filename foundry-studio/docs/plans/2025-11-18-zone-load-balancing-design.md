# Zone Load Balancing Design

## Overview

This document describes a zone load balancing system for the VTM/Relay warehouse simulation. The system tracks active tasks per zone and uses this information to balance work across VTMs, maximizing throughput by minimizing idle time.

## Problem Statement

### VTM vs RTP Fundamental Difference

In RTP (Rack-to-Person) systems, any robot can service any rack anywhere - it's a global resource pool. Load naturally distributes because the system picks the nearest idle robot.

In VTM/Relay systems, robots are **zone-bound**. This creates a capacity constraint:
- Each zone has limited VTM capacity (e.g., 300 totes/hour)
- Each PPS has throughput demand (e.g., 40 totes/hour)
- Without load balancing, work piles onto some zones while VTMs in other zones sit idle

### Goal

**Minimize VTM idle time** - Maximize throughput by utilizing the full warehouse capacity evenly.

### Approach

Track active task count per zone as shared state. Both order selection and back-to-store operations query this state to prefer zones with lower load, while using distance and other criteria as tiebreakers within similar-load zones.

---

## Components

### ZoneLoadTracker

New component that tracks active task count per zone.

**Responsibilities:**
- Track pick task count per zone
- Track totes at relay awaiting return
- Provide bucketed zone selection for consumers

**Interface:**
```java
public class ZoneLoadTracker {
    // Configuration
    void setBucketSize(int size);
    int getBucketSize();

    // Load updates
    synchronized void incrementLoad(String zoneId);
    synchronized void decrementLoad(String zoneId);

    // Queries
    int getLoad(String zoneId);
    Map<String, Integer> getAllLoads();

    // Bucketed selection
    List<String> getZonesInLowestBucket();
}
```

**Thread Safety:** All methods synchronized for future production use outside simulation.

**System Type:** Not instantiated for RTP systems (null tracker).

---

### BackToStoreSelector

New component that implements multi-criteria zone selection for returning totes.

**Responsibilities:**
- Select optimal zone for back-to-store using spec criteria
- Atomic select-and-reserve to prevent race conditions

**Interface:**
```java
public class BackToStoreSelector {
    // Atomic operation - selects zone and reserves in one call
    public synchronized String selectAndReserveZone(Tote tote, StationId originStationId);
}
```

**Dependencies:**
- `ZoneLoadTracker` - for load bucketing
- `StoragePositionManager` - for soft reservation
- `ZoneManager` - for zone lookup
- `GridPopularityManager` - for distance calculations

**Selection Criteria (from spec):**
1. Zone load bucketing (via ZoneLoadTracker)
2. Hard filtering (free relay positions, free storage by category)
3. Popularity-based location preference (A-B-C sequences)
4. Availability ratio per category
5. Distance from PPS (bucketed)

---

## Load Tracking Lifecycle

### What Counts as Load

Zone load represents **pending VTM work** - tasks that VTMs need to complete.

### Lifecycle Events

| Event | Action | Where |
|-------|--------|-------|
| Order selection confirms tasks | Increment source zone | Order strategy |
| VTM delivers tote to relay | Decrement source zone | VTM process |
| Tote at relay has no pending tasks | Increment target zone | Station/HTM process |
| VTM returns tote to rack | Decrement target zone | VTM process |

### Flow Diagram

```
ORDER SELECTION
    |
    v
[Confirm tasks] --> incrementLoad(sourceZoneId)
    |
    v
VTM picks tote from rack
    |
    v
VTM delivers to relay --> decrementLoad(sourceZoneId)
    |
    v
HTM takes to station
    |
    v
Picks completed, tote empty/done
    |
    v
[Tote ready for return] --> incrementLoad(targetZoneId)
    |
    v
HTM returns to relay
    |
    v
VTM returns to rack --> decrementLoad(targetZoneId)
```

### Integration Points

**Increment (outbound):** In order selection strategy, after task generation and validation
```java
// In AbstractOrderSelectionStrategy or concrete strategy
for (PickTask task : confirmedTasks) {
    String zoneId = getZoneForTask(task);
    zoneLoadTracker.incrementLoad(zoneId);
}
```

**Decrement (outbound complete):** In VTM process, after relay delivery
```java
// In DesmojVtmProcess
zoneLoadTracker.decrementLoad(tote.getZoneId());
```

**Increment (return needed):** When tote has no more pending tasks
```java
// After last pick from tote
zoneLoadTracker.incrementLoad(targetZoneId);
```

**Decrement (return complete):** In VTM process, after rack return
```java
// In DesmojVtmProcess
zoneLoadTracker.decrementLoad(tote.getTargetZoneId());
```

---

## Order Selection Integration

### Two-Pass Approach

Zone load is the **primary sorting criteria**. Orders are first bucketed by zone load, then scored within each bucket using existing algorithms (PPF/TPQ).

```java
// Step 1: Prepare candidates (existing - generates tasks once)
Map<String, OrderCandidate> candidates = prepareOrderCandidates(
    pendingOrders, currentTime, activeStorageSpaces);

// Step 2: Group by zone load bucket (new)
Map<Integer, List<OrderCandidate>> buckets = groupCandidatesByZoneLoadBucket(candidates);

// Step 3: Process buckets in order, score within each
for (int bucket : buckets.keySet()) {
    for (OrderCandidate candidate : buckets.get(bucket)) {
        double score = calculateOrderScore(candidate, ...);
        // existing best-score selection
    }
    if (selectedEnough) break;
}
```

### Max Zone Load (Bottleneck) for Multi-Zone Orders

An order may require totes from multiple zones. Use the **maximum zone load** to represent the order's impact on the most constrained zone.

```java
private int getMaxZoneLoad(List<PickTask> tasks) {
    int maxLoad = 0;
    for (PickTask task : tasks) {
        String zoneId = getZoneForTask(task);
        int load = zoneLoadTracker.getLoad(zoneId);
        maxLoad = Math.max(maxLoad, load);
    }
    return maxLoad;
}
```

**Rationale:** Adding work to an already-overloaded zone is the constraint. Even if 90% of an order's tasks go to a low-load zone, we still care about the 10% hitting the bottleneck.

### Zone Lookup from Task

Tasks have `StorageSpaceId` (ToteId). Look up zone via coordinates:

```java
private String getZoneForTask(PickTask task) {
    Tote tote = storageSpaceManager.getTote((ToteId) task.getStorageSpaceId());
    GridCoordinate coord = tote.getCurrentCoordinate();
    return zoneManager.getZoneForPosition(coord);
}
```

### Shared Logic in AbstractOrderSelectionStrategy

Add bucketing method to base class:

```java
public abstract class AbstractOrderSelectionStrategy {

    protected ZoneLoadTracker zoneLoadTracker; // nullable for RTP
    protected ZoneManager zoneManager;
    protected StorageSpaceManager storageSpaceManager;

    /**
     * Group order candidates by zone load bucket.
     *
     * <p>Uses max zone load (bottleneck) for orders spanning multiple zones.
     * Returns all candidates in bucket 0 for RTP systems (no zone tracking).
     *
     * @param candidates Pre-computed order candidates with tasks
     * @return Map of bucket index to candidates in that bucket
     */
    protected Map<Integer, List<OrderCandidate>> groupCandidatesByZoneLoadBucket(
            Map<String, OrderCandidate> candidates) {

        if (zoneLoadTracker == null) {
            // RTP: all candidates in same bucket
            return Map.of(0, new ArrayList<>(candidates.values()));
        }

        Map<Integer, List<OrderCandidate>> buckets = new TreeMap<>();
        int bucketSize = zoneLoadTracker.getBucketSize();

        for (OrderCandidate candidate : candidates.values()) {
            int maxLoad = getMaxZoneLoad(candidate.tasks());
            int bucket = maxLoad / bucketSize;

            buckets.computeIfAbsent(bucket, k -> new ArrayList<>()).add(candidate);
        }

        return buckets;
    }

    private int getMaxZoneLoad(List<PickTask> tasks) {
        int maxLoad = 0;
        for (PickTask task : tasks) {
            String zoneId = getZoneForTask(task);
            int load = zoneLoadTracker.getLoad(zoneId);
            maxLoad = Math.max(maxLoad, load);
        }
        return maxLoad;
    }
}
```

### RTP Compatibility

When `zoneLoadTracker` is null (RTP system), all orders are placed in bucket 0, and existing scoring takes over entirely.

---

## Back-to-Store Integration

### BackToStoreSelector Implementation

Implements multi-criteria zone selection from the Storage Position Manager Design spec, with atomic select-and-reserve to prevent race conditions.

```java
@Slf4j
@RequiredArgsConstructor
public class BackToStoreSelector {

    private final ZoneLoadTracker zoneLoadTracker;
    private final StoragePositionManager storagePositionManager;
    private final ZoneManager zoneManager;
    private final GridPopularityManager gridPopularityManager;

    /**
     * Select optimal zone and reserve atomically.
     *
     * <p>Prevents race condition between zone selection and reservation.
     * Uses zone load bucketing as primary criteria, then applies spec criteria
     * within the same bucket.
     *
     * @param tote The tote to return to storage
     * @param originStationId The station the tote is returning from
     * @return The selected zone ID
     */
    public synchronized String selectAndReserveZone(Tote tote, StationId originStationId) {
        // Step 1: Get zones in lowest load bucket
        List<String> lowLoadZones = zoneLoadTracker.getZonesInLowestBucket();

        // Step 2: Apply hard filtering (free relay, free storage by category)
        List<String> eligibleZones = filterEligibleZones(lowLoadZones, tote);

        // Step 3: If no eligible zones in lowest bucket, try next buckets
        if (eligibleZones.isEmpty()) {
            eligibleZones = filterEligibleZones(zoneManager.getAllZoneIds(), tote);
        }

        // Step 4: Apply remaining criteria from spec
        String selectedZone = selectBySpecCriteria(eligibleZones, tote, originStationId);

        // Step 5: Reserve atomically
        storagePositionManager.reserveSoftZoneForReturn(tote, selectedZone);
        zoneLoadTracker.incrementLoad(selectedZone);

        return selectedZone;
    }

    /**
     * Filter zones by hard criteria.
     *
     * @param zones Zones to filter
     * @param tote The tote being returned
     * @return Zones with free relay positions and storage for tote's category
     */
    private List<String> filterEligibleZones(List<String> zones, Tote tote) {
        List<String> eligible = new ArrayList<>();

        for (String zoneId : zones) {
            Zone zone = zoneManager.getZone(zoneId);

            // Check free relay positions
            boolean hasFreeRelay = zone.getCapacity().available() > 0;

            // Check free storage for tote's category
            boolean hasFreeStorage = hasStorageForCategory(zone, tote.getCategory());

            if (hasFreeRelay && hasFreeStorage) {
                eligible.add(zoneId);
            }
        }

        return eligible;
    }

    /**
     * Select zone using spec criteria within eligible zones.
     *
     * <p>Criteria (in order):
     * 1. Popularity-based location preference (A-B-C)
     * 2. Availability ratio for tote's category
     * 3. Distance from origin PPS (bucketed)
     *
     * @param eligibleZones Zones that passed hard filtering
     * @param tote The tote being returned
     * @param originStationId Station for distance calculation
     * @return Selected zone ID
     */
    private String selectBySpecCriteria(
            List<String> eligibleZones,
            Tote tote,
            StationId originStationId) {

        if (eligibleZones.isEmpty()) {
            // Fallback: return any zone (edge case)
            log.warn("No eligible zones for tote {}, using fallback", tote.getId());
            return zoneManager.getAllZoneIds().get(0);
        }

        if (eligibleZones.size() == 1) {
            return eligibleZones.get(0);
        }

        // Score zones by spec criteria
        return eligibleZones.stream()
            .min(Comparator.comparingDouble(zoneId ->
                calculateZoneScore(zoneId, tote, originStationId)))
            .orElse(eligibleZones.get(0));
    }

    /**
     * Calculate score for zone selection (lower is better).
     */
    private double calculateZoneScore(String zoneId, Tote tote, StationId originStationId) {
        Zone zone = zoneManager.getZone(zoneId);

        // Availability ratio (higher is better, so negate)
        double availabilityScore = -getAvailabilityRatio(zone, tote.getCategory());

        // Distance from PPS (lower is better)
        double distanceScore = getDistanceFromStation(zone, originStationId);

        // Combine with weights
        return (availabilityScore * 100) + distanceScore;
    }
}
```

### Call Flow

```
HTM Process (tote ready for return)
    |
    v
BackToStoreSelector.selectAndReserveZone(tote, stationId)
    |
    +-- Get zones in lowest load bucket
    +-- Filter by relay + storage capacity
    +-- Score by availability + distance
    +-- Reserve soft zone (StoragePositionManager)
    +-- Increment load (ZoneLoadTracker)
    |
    v
Returns zoneId --> HTM delivers to relay in that zone
```

### Replacing Current Logic

Current `StoragePositionManager.reserveSoftZone()` selects by relay capacity only. The new flow:

**Before:**
```java
// In HTM process
String zoneId = storagePositionManager.reserveSoftZone(tote);
```

**After:**
```java
// In HTM process
String zoneId = backToStoreSelector.selectAndReserveZone(tote, currentStationId);
```

---

## Configuration

### Configuration Parameters

Add to existing `AlgorithmConfig`:

```java
public record AlgorithmConfig(
    // ... existing fields

    /** Enable relay zone load balancing (default: true, ignored for RTP) */
    boolean relayZoneLoadBalancingEnabled,

    /** Bucket size for relay zone load comparison (default: 5 tasks) */
    int relayZoneLoadBalancingBucketSize
) {}
```

### Usage in Simulation Setup

```java
// In DesmojSimulationEngine or model initialization

ZoneLoadTracker zoneLoadTracker = null;

if (systemType == SystemType.RELAY && config.algorithmConfig().relayZoneLoadBalancingEnabled()) {
    zoneLoadTracker = new ZoneLoadTracker();
    zoneLoadTracker.setBucketSize(config.algorithmConfig().relayZoneLoadBalancingBucketSize());
}

// Pass to order strategies
OrderSelectionStrategy strategy = createOrderSelectionStrategy(
    ...,
    zoneLoadTracker,  // nullable for RTP
    zoneManager,
    storageSpaceManager
);

// Create back-to-store selector (relay only)
BackToStoreSelector backToStoreSelector = null;
if (zoneLoadTracker != null) {
    backToStoreSelector = new BackToStoreSelector(
        zoneLoadTracker,
        storagePositionManager,
        zoneManager,
        gridPopularityManager
    );
}
```

### Configuration Examples

**Default (relay with load balancing):**
```json
{
  "systemType": "relay",
  "algorithmConfig": {
    "relayZoneLoadBalancingEnabled": true,
    "relayZoneLoadBalancingBucketSize": 5
  }
}
```

**Disabled (for comparison testing):**
```json
{
  "systemType": "relay",
  "algorithmConfig": {
    "relayZoneLoadBalancingEnabled": false
  }
}
```

**Tuned bucket size:**
```json
{
  "systemType": "relay",
  "algorithmConfig": {
    "relayZoneLoadBalancingEnabled": true,
    "relayZoneLoadBalancingBucketSize": 3
  }
}
```

---

## Edge Cases

### All Zones Equally Loaded
- **Scenario:** Zone-A: 10, Zone-B: 10, Zone-C: 10
- **Behavior:** All in same bucket - existing scoring (PPF/TPQ, distance) takes over
- **Handling:** No special case needed

### All Zones Empty (Simulation Start)
- **Scenario:** Zone-A: 0, Zone-B: 0, Zone-C: 0
- **Behavior:** All in lowest bucket - existing scoring determines selection
- **Handling:** No special case needed

### All Zones Heavily Loaded
- **Scenario:** Zone-A: 50, Zone-B: 48, Zone-C: 52
- **Behavior:** All within bucket tolerance - existing scoring takes over
- **Handling:** System is saturated but balanced, continues normally

### Single Zone System
- **Scenario:** Only Zone-A exists
- **Behavior:** Always in lowest bucket - existing scoring takes over
- **Handling:** Zone load irrelevant with one zone

### Order Can Only Come From Overloaded Zone
- **Scenario:** Order needs SKU only available in Zone-A (high load)
- **Behavior:** Order is in higher bucket, but gets selected when lower buckets exhausted
- **Handling:** Two-pass approach naturally handles this - process buckets in order

### No Zones Have Free Relay + Storage
- **Scenario:** All zones at capacity
- **Behavior:** `BackToStoreSelector` returns best available (least bad option)
- **Handling:** Fallback to any zone with warning log

### Warmup Period Imbalance
- **Scenario:** Early orders cluster in nearby zones
- **Behavior:** Bucket tolerance (default 5) absorbs small imbalances
- **Handling:** System self-corrects as more orders flow

---

## Summary

This design introduces zone load balancing for relay systems through:

1. **ZoneLoadTracker** - Shared state for zone task counts
2. **BackToStoreSelector** - Multi-criteria zone selection with atomic reservation
3. **Order selection integration** - Two-pass bucketing approach
4. **Configuration** - Toggle and bucket size in AlgorithmConfig

The design maintains RTP compatibility (null tracker), avoids race conditions (synchronized/atomic operations), and handles edge cases gracefully (fallbacks, bucket tolerance).
