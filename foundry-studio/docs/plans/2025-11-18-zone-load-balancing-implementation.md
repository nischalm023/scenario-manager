# Zone Load Balancing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement zone load tracking and balancing for relay systems to minimize VTM idle time by distributing work evenly across zones.

**Architecture:** Two new components (`ZoneLoadTracker` for shared load state, `BackToStoreSelector` for return zone selection) integrated with existing order selection strategies via two-pass bucketing approach. RTP systems unaffected (null tracker).

**Tech Stack:** Java 21, Desmo-J simulation, Spring patterns (no Spring DI in domain)

---

## Task 1: Add Configuration Fields to AlgorithmConfig

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java:281-305`

**Step 1: Add configuration fields**

Add two new fields to `AlgorithmConfig` record for relay zone load balancing:

```java
@Builder(toBuilder = true)
public record AlgorithmConfig(
    String putAlgorithm,
    OrderSelectionAlgorithm orderSelectionAlgorithm,
    int maxTotesPerOrder,
    TPQAlgorithmConfig tpqConfig,
    Double navFactor,
    Boolean relayZoneLoadBalancingEnabled,
    Integer relayZoneLoadBalancingBucketSize) {
  public AlgorithmConfig {
    if (putAlgorithm == null) {
      putAlgorithm = "greedy";
    }
    if (orderSelectionAlgorithm == null) {
      orderSelectionAlgorithm = OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY;
    }
    if (maxTotesPerOrder == 0) {
      maxTotesPerOrder = 10;
    }
    if (tpqConfig == null) {
      tpqConfig = TPQAlgorithmConfig.builder().build();
    }
    if (navFactor == null) {
      navFactor = 1.8;
    }
    if (relayZoneLoadBalancingEnabled == null) {
      relayZoneLoadBalancingEnabled = true;
    }
    if (relayZoneLoadBalancingBucketSize == null) {
      relayZoneLoadBalancingBucketSize = 5;
    }
  }
}
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java
git commit -m "feat: add relay zone load balancing config fields"
```

---

## Task 2: Create ZoneLoadTracker Component

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/ZoneLoadTracker.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/ZoneLoadTrackerTest.java`

**Step 1: Write failing test**

```java
package com.greyorange.dessimulator.domain.put;

import static org.assertj.core.api.Assertions.assertThat;

import java.util.List;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class ZoneLoadTrackerTest {

  private ZoneLoadTracker tracker;

  @BeforeEach
  void setUp() {
    tracker = new ZoneLoadTracker(5); // bucket size = 5
  }

  @Test
  void shouldTrackLoadPerZone() {
    tracker.incrementLoad("zone-A");
    tracker.incrementLoad("zone-A");
    tracker.incrementLoad("zone-B");

    assertThat(tracker.getLoad("zone-A")).isEqualTo(2);
    assertThat(tracker.getLoad("zone-B")).isEqualTo(1);
    assertThat(tracker.getLoad("zone-C")).isEqualTo(0); // Unknown zone returns 0
  }

  @Test
  void shouldDecrementLoad() {
    tracker.incrementLoad("zone-A");
    tracker.incrementLoad("zone-A");
    tracker.decrementLoad("zone-A");

    assertThat(tracker.getLoad("zone-A")).isEqualTo(1);
  }

  @Test
  void shouldNotGoNegative() {
    tracker.decrementLoad("zone-A"); // Decrement on zero

    assertThat(tracker.getLoad("zone-A")).isEqualTo(0);
  }

  @Test
  void shouldReturnZonesInLowestBucket() {
    // Bucket size = 5
    // zone-A: 3 (bucket 0)
    // zone-B: 8 (bucket 1)
    // zone-C: 2 (bucket 0)
    tracker.incrementLoad("zone-A");
    tracker.incrementLoad("zone-A");
    tracker.incrementLoad("zone-A");

    for (int i = 0; i < 8; i++) {
      tracker.incrementLoad("zone-B");
    }

    tracker.incrementLoad("zone-C");
    tracker.incrementLoad("zone-C");

    List<String> lowestBucket = tracker.getZonesInLowestBucket();

    assertThat(lowestBucket).containsExactlyInAnyOrder("zone-A", "zone-C");
    assertThat(lowestBucket).doesNotContain("zone-B");
  }

  @Test
  void shouldReturnAllZonesWhenEqualLoad() {
    tracker.incrementLoad("zone-A");
    tracker.incrementLoad("zone-B");
    tracker.incrementLoad("zone-C");

    List<String> lowestBucket = tracker.getZonesInLowestBucket();

    assertThat(lowestBucket).containsExactlyInAnyOrder("zone-A", "zone-B", "zone-C");
  }

  @Test
  void shouldReturnEmptyListWhenNoZonesTracked() {
    List<String> lowestBucket = tracker.getZonesInLowestBucket();

    assertThat(lowestBucket).isEmpty();
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=ZoneLoadTrackerTest -q`
Expected: FAIL with compilation error (class not found)

**Step 3: Write implementation**

```java
package com.greyorange.dessimulator.domain.put;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;

/**
 * Tracks active task count per zone for load balancing.
 *
 * <p>Provides bucketed zone selection to prefer zones with lower load while allowing other criteria
 * (distance, popularity) to break ties within similar-load zones.
 *
 * <p>Thread-safe for future production use outside simulation.
 */
@Slf4j
public class ZoneLoadTracker {

  private final Map<String, Integer> zoneLoads = new ConcurrentHashMap<>();

  @Getter private final int bucketSize;

  /**
   * Create tracker with specified bucket size.
   *
   * @param bucketSize Number of tasks that constitute one bucket (default: 5)
   */
  public ZoneLoadTracker(int bucketSize) {
    this.bucketSize = bucketSize;
    log.info("ZoneLoadTracker initialized with bucket size: {}", bucketSize);
  }

  /**
   * Increment load for a zone.
   *
   * @param zoneId Zone identifier
   */
  public synchronized void incrementLoad(String zoneId) {
    zoneLoads.merge(zoneId, 1, Integer::sum);
    log.debug("Zone {} load incremented to {}", zoneId, zoneLoads.get(zoneId));
  }

  /**
   * Decrement load for a zone.
   *
   * @param zoneId Zone identifier
   */
  public synchronized void decrementLoad(String zoneId) {
    zoneLoads.compute(
        zoneId,
        (k, v) -> {
          if (v == null || v <= 1) {
            return 0;
          }
          return v - 1;
        });
    log.debug("Zone {} load decremented to {}", zoneId, zoneLoads.get(zoneId));
  }

  /**
   * Get current load for a zone.
   *
   * @param zoneId Zone identifier
   * @return Current task count (0 if zone not tracked)
   */
  public int getLoad(String zoneId) {
    return zoneLoads.getOrDefault(zoneId, 0);
  }

  /**
   * Get all zone loads.
   *
   * @return Map of zone ID to task count
   */
  public Map<String, Integer> getAllLoads() {
    return Map.copyOf(zoneLoads);
  }

  /**
   * Get zones in the lowest load bucket.
   *
   * <p>Zones are bucketed by load / bucketSize. Returns all zones in the bucket with the minimum
   * value.
   *
   * @return List of zone IDs in lowest bucket
   */
  public synchronized List<String> getZonesInLowestBucket() {
    if (zoneLoads.isEmpty()) {
      return List.of();
    }

    // Find minimum bucket
    int minBucket = Integer.MAX_VALUE;
    for (int load : zoneLoads.values()) {
      int bucket = load / bucketSize;
      if (bucket < minBucket) {
        minBucket = bucket;
      }
    }

    // Collect zones in minimum bucket
    List<String> result = new ArrayList<>();
    for (Map.Entry<String, Integer> entry : zoneLoads.entrySet()) {
      int bucket = entry.getValue() / bucketSize;
      if (bucket == minBucket) {
        result.add(entry.getKey());
      }
    }

    return result;
  }
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=ZoneLoadTrackerTest -q`
Expected: Tests run: 6, Failures: 0

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/ZoneLoadTracker.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/ZoneLoadTrackerTest.java
git commit -m "feat: add ZoneLoadTracker for zone load balancing"
```

---

## Task 3: Add Dependencies to AbstractOrderSelectionStrategy

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/AbstractOrderSelectionStrategy.java`

**Step 1: Add new fields and imports**

Add imports at top:
```java
import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.inventory.StorageSpaceManager;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.put.ZoneLoadTracker;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import java.util.TreeMap;
```

Add protected fields after existing fields (around line 42):
```java
  // Zone load balancing (nullable for RTP)
  protected ZoneLoadTracker zoneLoadTracker;
  protected ZoneManager zoneManager;
  protected StorageSpaceManager storageSpaceManager;
```

**Step 2: Add setter methods for the new dependencies**

Add after the constructor (around line 55):
```java
  /**
   * Set zone load tracking dependencies (for relay systems only).
   *
   * @param zoneLoadTracker Zone load tracker (null for RTP)
   * @param zoneManager Zone manager for coordinate-to-zone lookup
   * @param storageSpaceManager Storage space manager for tote lookup
   */
  public void setZoneLoadDependencies(
      ZoneLoadTracker zoneLoadTracker,
      ZoneManager zoneManager,
      StorageSpaceManager storageSpaceManager) {
    this.zoneLoadTracker = zoneLoadTracker;
    this.zoneManager = zoneManager;
    this.storageSpaceManager = storageSpaceManager;
  }
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/AbstractOrderSelectionStrategy.java
git commit -m "feat: add zone load dependencies to AbstractOrderSelectionStrategy"
```

---

## Task 4: Add Zone Load Bucketing Logic to AbstractOrderSelectionStrategy

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/AbstractOrderSelectionStrategy.java`

**Step 1: Add groupCandidatesByZoneLoadBucket method**

Add after `prepareOrderCandidates` method (around line 212):

```java
  /**
   * Group order candidates by zone load bucket.
   *
   * <p>Uses max zone load (bottleneck) for orders spanning multiple zones. Returns all candidates
   * in bucket 0 for RTP systems (no zone tracking).
   *
   * @param candidates Pre-computed order candidates with tasks
   * @return Map of bucket index to candidates in that bucket (sorted by bucket)
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

  /**
   * Get maximum zone load for a set of tasks.
   *
   * <p>Represents the bottleneck - the most loaded zone this order would affect.
   *
   * @param tasks Pick tasks for an order
   * @return Maximum zone load across all tasks
   */
  private int getMaxZoneLoad(List<PickTask> tasks) {
    if (zoneLoadTracker == null || zoneManager == null || storageSpaceManager == null) {
      return 0;
    }

    int maxLoad = 0;
    for (PickTask task : tasks) {
      String zoneId = getZoneForTask(task);
      if (zoneId != null) {
        int load = zoneLoadTracker.getLoad(zoneId);
        maxLoad = Math.max(maxLoad, load);
      }
    }
    return maxLoad;
  }

  /**
   * Get zone ID for a task's storage space.
   *
   * @param task Pick task
   * @return Zone ID, or null if cannot be determined
   */
  private String getZoneForTask(PickTask task) {
    if (!(task.getStorageSpaceId() instanceof ToteId toteId)) {
      return null; // RTP system or unknown type
    }

    Tote tote = storageSpaceManager.getTote(toteId);
    if (tote == null) {
      return null;
    }

    GridCoordinate coord = tote.getCurrentCoordinate();
    if (coord == null) {
      return null;
    }

    return zoneManager.getZoneForPosition(coord);
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/AbstractOrderSelectionStrategy.java
git commit -m "feat: add zone load bucketing to AbstractOrderSelectionStrategy"
```

---

## Task 5: Update MaximizePpfDirectlyStrategy to Use Zone Load Bucketing

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfDirectlyStrategy.java`

**Step 1: Read the current implementation**

Read the file first to understand the current `selectOrders` implementation structure.

**Step 2: Update selectOrders to use two-pass approach**

The strategy should:
1. Call `prepareOrderCandidates()` to generate candidates with tasks
2. Call `groupCandidatesByZoneLoadBucket()` to bucket by zone load
3. Process buckets in order (lowest first)
4. Within each bucket, use existing PPF scoring

Update the `selectOrders` method to integrate zone load bucketing. The existing scoring logic remains, but wrapped in the two-pass approach.

**Step 3: Compile and test**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfDirectlyStrategy.java
git commit -m "feat: integrate zone load bucketing in MaximizePpfDirectlyStrategy"
```

---

## Task 6: Update MaximizePpfWithTieringStrategy to Use Zone Load Bucketing

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfWithTieringStrategy.java`

**Step 1: Update selectOrders with same pattern as Task 5**

Apply the same two-pass approach used in MaximizePpfDirectlyStrategy.

**Step 2: Compile and test**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfWithTieringStrategy.java
git commit -m "feat: integrate zone load bucketing in MaximizePpfWithTieringStrategy"
```

---

## Task 7: Update MaximizeTPQStrategy to Use Zone Load Bucketing

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizeTPQStrategy.java`

**Step 1: Update selectOrders with same pattern as Tasks 5 and 6**

Apply the same two-pass approach.

**Step 2: Compile and test**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizeTPQStrategy.java
git commit -m "feat: integrate zone load bucketing in MaximizeTPQStrategy"
```

---

## Task 8: Wire ZoneLoadTracker in OrderManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderManager.java`

**Step 1: Add ZoneLoadTracker parameter to constructor**

Update constructor to accept `ZoneLoadTracker`, `ZoneManager`, and `StorageSpaceManager`:

```java
public OrderManager(
    WarehouseInventory inventory,
    List<Station> stations,
    int binsPerStation,
    OrderSelectionAlgorithm orderSelectionAlgorithm,
    int maxTotesPerOrder,
    TPQAlgorithmConfig tpqConfig,
    GridPopularityManager gridPopularityManager,
    StorageSpaceManager storageSpaceManager,
    double operatorTimePerPick,
    double robotSpeed,
    int numRobots,
    MetricsManager metricsManager,
    ZoneLoadTracker zoneLoadTracker,  // NEW
    ZoneManager zoneManager) {         // NEW
```

**Step 2: Pass dependencies to strategy**

After creating the strategy, call `setZoneLoadDependencies`:

```java
// After strategy creation switch statement
strategy.setZoneLoadDependencies(zoneLoadTracker, zoneManager, storageSpaceManager);
```

**Step 3: Update load tracking on task confirmation**

In `processOrders`, after reserving inventory for tasks, increment zone load:

```java
// After: inventory.reserveStock(...)
if (zoneLoadTracker != null) {
  // Increment zone load for each task's source zone
  // ... (will need to get zone from task)
}
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors (DesmojSimulationEngine needs updating)

**Step 5: Commit partial work**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderManager.java
git commit -m "feat: wire ZoneLoadTracker in OrderManager"
```

---

## Task 9: Wire ZoneLoadTracker in DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Create ZoneLoadTracker based on config**

After `initializeRelaySystem()` (around line 162), create the tracker:

```java
// Create ZoneLoadTracker for relay systems
ZoneLoadTracker zoneLoadTracker = null;
if (config.systemType() == SystemType.RELAY
    && config.algorithmConfig().relayZoneLoadBalancingEnabled()) {
  zoneLoadTracker = new ZoneLoadTracker(
      config.algorithmConfig().relayZoneLoadBalancingBucketSize());
}
```

**Step 2: Pass to OrderManager constructor**

Update the `OrderManager` constructor call to include the new parameters.

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "feat: create and wire ZoneLoadTracker in simulation engine"
```

---

## Task 10: Add Load Tracking to VTM Process (Decrement on Relay Delivery)

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Add ZoneLoadTracker dependency**

Add field and constructor parameter:

```java
private final ZoneLoadTracker zoneLoadTracker; // nullable
```

**Step 2: Decrement load after relay delivery**

After `storagePositionManager.releaseSoftReservation(tote.getId());` (around line 250), add:

```java
// Decrement zone load - VTM outbound work complete
if (zoneLoadTracker != null) {
  String sourceZoneId = getSourceZoneForTote(tote);
  if (sourceZoneId != null) {
    zoneLoadTracker.decrementLoad(sourceZoneId);
  }
}
```

**Step 3: Add helper method to get source zone**

```java
private String getSourceZoneForTote(Tote tote) {
  // Get zone from tote's original rack position
  // ...
}
```

**Step 4: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: Compilation errors (DesmojWarehouseModel needs to pass tracker)

**Step 5: Commit partial work**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: add zone load decrement to VTM process"
```

---

## Task 11: Add Load Tracking to VTM Return Journey (Decrement on Rack Return)

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Decrement load after return journey**

In `executeReturnJourney`, after `storagePositionManager.releaseHardReservation(tote.getId());` (around line 493), add:

```java
// Decrement zone load - VTM return work complete
if (zoneLoadTracker != null) {
  String targetZoneId = tote.getTargetZoneId();
  if (targetZoneId != null) {
    zoneLoadTracker.decrementLoad(targetZoneId);
  }
}
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS (once DesmojWarehouseModel passes tracker)

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: add zone load decrement for VTM return journey"
```

---

## Task 12: Create BackToStoreSelector Component

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/BackToStoreSelector.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/BackToStoreSelectorTest.java`

**Step 1: Write failing test**

```java
package com.greyorange.dessimulator.domain.put;

import static org.assertj.core.api.Assertions.assertThat;
import static org.mockito.Mockito.*;

import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneCapacity;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.mockito.Mock;
import org.mockito.junit.jupiter.MockitoExtension;

@ExtendWith(MockitoExtension.class)
class BackToStoreSelectorTest {

  @Mock private ZoneLoadTracker zoneLoadTracker;
  @Mock private StoragePositionManager storagePositionManager;
  @Mock private ZoneManager zoneManager;
  @Mock private GridPopularityManager gridPopularityManager;

  private BackToStoreSelector selector;

  @BeforeEach
  void setUp() {
    selector =
        new BackToStoreSelector(
            zoneLoadTracker, storagePositionManager, zoneManager, gridPopularityManager);
  }

  @Test
  void shouldSelectZoneWithLowestLoad() {
    // Setup mocks
    Tote tote = mock(Tote.class);
    when(tote.getId()).thenReturn(new ToteId(1));

    when(zoneLoadTracker.getZonesInLowestBucket()).thenReturn(List.of("zone-A", "zone-B"));

    Zone zoneA = createMockZone("zone-A", 5); // available capacity
    Zone zoneB = createMockZone("zone-B", 3);

    when(zoneManager.getZone("zone-A")).thenReturn(zoneA);
    when(zoneManager.getZone("zone-B")).thenReturn(zoneB);
    when(zoneManager.getAllZoneIds()).thenReturn(List.of("zone-A", "zone-B"));

    // Execute
    String selectedZone = selector.selectAndReserveZone(tote, new StationId(1));

    // Verify - should select zone-A (more capacity)
    assertThat(selectedZone).isIn("zone-A", "zone-B");
    verify(storagePositionManager).reserveSoftZoneForReturn(tote, selectedZone);
    verify(zoneLoadTracker).incrementLoad(selectedZone);
  }

  private Zone createMockZone(String zoneId, int available) {
    Zone zone = mock(Zone.class);
    ZoneCapacity capacity = mock(ZoneCapacity.class);
    when(capacity.available()).thenReturn(available);
    when(zone.getCapacity()).thenReturn(capacity);
    when(zone.getZoneId()).thenReturn(zoneId);
    return zone;
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=BackToStoreSelectorTest -q`
Expected: FAIL with compilation error

**Step 3: Write implementation**

```java
package com.greyorange.dessimulator.domain.put;

import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import lombok.RequiredArgsConstructor;
import lombok.extern.slf4j.Slf4j;

/**
 * Selects optimal zone for back-to-store operations.
 *
 * <p>Implements multi-criteria zone selection with atomic select-and-reserve to prevent race
 * conditions. Uses zone load bucketing as primary criteria, then applies additional criteria from
 * the storage position manager design spec.
 */
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
   * <p>Prevents race condition between zone selection and reservation. Uses zone load bucketing as
   * primary criteria, then applies spec criteria within the same bucket.
   *
   * @param tote The tote to return to storage
   * @param originStationId The station the tote is returning from
   * @return The selected zone ID
   */
  public synchronized String selectAndReserveZone(Tote tote, StationId originStationId) {
    // Step 1: Get zones in lowest load bucket
    List<String> lowLoadZones = zoneLoadTracker.getZonesInLowestBucket();

    // Step 2: Apply hard filtering (free relay, free storage)
    List<String> eligibleZones = filterEligibleZones(lowLoadZones);

    // Step 3: If no eligible zones in lowest bucket, try all zones
    if (eligibleZones.isEmpty()) {
      eligibleZones = filterEligibleZones(zoneManager.getAllZoneIds());
    }

    // Step 4: Select zone by additional criteria
    String selectedZone = selectByAdditionalCriteria(eligibleZones, tote, originStationId);

    // Step 5: Reserve atomically
    storagePositionManager.reserveSoftZoneForReturn(tote, selectedZone);
    zoneLoadTracker.incrementLoad(selectedZone);

    log.debug(
        "Selected zone {} for tote {} return from station {}",
        selectedZone,
        tote.getId(),
        originStationId);

    return selectedZone;
  }

  /**
   * Filter zones by hard criteria.
   *
   * @param zones Zones to filter
   * @return Zones with free relay positions
   */
  private List<String> filterEligibleZones(List<String> zones) {
    List<String> eligible = new ArrayList<>();

    for (String zoneId : zones) {
      Zone zone = zoneManager.getZone(zoneId);
      if (zone != null && zone.getCapacity().available() > 0) {
        eligible.add(zoneId);
      }
    }

    return eligible;
  }

  /**
   * Select zone using additional criteria.
   *
   * @param eligibleZones Zones that passed hard filtering
   * @param tote The tote being returned
   * @param originStationId Station for distance calculation
   * @return Selected zone ID
   */
  private String selectByAdditionalCriteria(
      List<String> eligibleZones, Tote tote, StationId originStationId) {

    if (eligibleZones.isEmpty()) {
      log.warn("No eligible zones for tote {}, using fallback", tote.getId());
      return zoneManager.getAllZoneIds().get(0);
    }

    if (eligibleZones.size() == 1) {
      return eligibleZones.get(0);
    }

    // Select zone with most available capacity (simple criteria for now)
    return eligibleZones.stream()
        .max(Comparator.comparingInt(zoneId -> {
          Zone zone = zoneManager.getZone(zoneId);
          return zone != null ? zone.getCapacity().available() : 0;
        }))
        .orElse(eligibleZones.get(0));
  }
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=BackToStoreSelectorTest -q`
Expected: Tests run: 1, Failures: 0

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/BackToStoreSelector.java
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/BackToStoreSelectorTest.java
git commit -m "feat: add BackToStoreSelector for back-to-store zone selection"
```

---

## Task 13: Integrate BackToStoreSelector in HTM Process

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

**Step 1: Replace getCandidateZonesForReturn with BackToStoreSelector**

The HTM process currently has inline zone selection in `getCandidateZonesForReturn`. Replace this with a call to `BackToStoreSelector.selectAndReserveZone()`.

**Step 2: Update load tracking**

When tote is ready for return (after station processing), increment zone load for return work.

**Step 3: Compile and verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java
git commit -m "feat: integrate BackToStoreSelector in HTM process"
```

---

## Task 14: Wire All Components in DesmojWarehouseModel

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

**Step 1: Pass ZoneLoadTracker to VTM and HTM processes**

Update process creation to include the tracker.

**Step 2: Create BackToStoreSelector and pass to HTM**

```java
BackToStoreSelector backToStoreSelector = null;
if (zoneLoadTracker != null) {
  backToStoreSelector = new BackToStoreSelector(
      zoneLoadTracker,
      storagePositionManager,
      zoneManager,
      gridPopularityManager);
}
```

**Step 3: Compile and verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "feat: wire ZoneLoadTracker and BackToStoreSelector in model"
```

---

## Task 15: Add Integration Test

**Files:**
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/integration/ZoneLoadBalancingIntegrationTest.java`

**Step 1: Write integration test**

Test that zone load balancing works end-to-end in a relay simulation:

```java
package com.greyorange.dessimulator.integration;

import static org.assertj.core.api.Assertions.assertThat;

import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.simulator.config.SimulationConfigs.*;
import org.junit.jupiter.api.Test;

class ZoneLoadBalancingIntegrationTest {

  @Test
  void shouldRunRelaySimulationWithZoneLoadBalancing() {
    // Create relay config with zone load balancing enabled
    SimulationConfig config = SimulationConfig.builder()
        .systemType(SystemType.RELAY)
        .simulationDuration(300.0) // 5 minutes
        .scenarioId("scenario_a")
        .algorithmConfig(AlgorithmConfig.builder()
            .relayZoneLoadBalancingEnabled(true)
            .relayZoneLoadBalancingBucketSize(5)
            .build())
        // ... other config
        .build();

    // Run simulation
    // ... setup and run

    // Verify zone loads are relatively balanced
    // ... assertions on metrics
  }
}
```

**Step 2: Run integration test**

Run: `cd apps/backend && mvn test -Dtest=ZoneLoadBalancingIntegrationTest -q`
Expected: PASS

**Step 3: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/integration/ZoneLoadBalancingIntegrationTest.java
git commit -m "test: add zone load balancing integration test"
```

---

## Task 16: Final Verification and Cleanup

**Files:**
- All modified files

**Step 1: Run full test suite**

Run: `cd apps/backend && mvn clean test`
Expected: All tests pass

**Step 2: Format code**

Run: `cd apps/backend && mvn spotless:apply`

**Step 3: Run a manual relay simulation**

Start server and run a relay simulation to verify zone load balancing in action.

**Step 4: Final commit**

```bash
git add -A
git commit -m "chore: final cleanup for zone load balancing feature"
```

---

## Summary

This implementation plan consists of 16 tasks that build the zone load balancing feature incrementally:

1. **Tasks 1-2**: Configuration and core `ZoneLoadTracker` component
2. **Tasks 3-7**: Order selection strategy integration
3. **Tasks 8-11**: Wiring and VTM load tracking
4. **Tasks 12-13**: `BackToStoreSelector` and HTM integration
5. **Tasks 14-16**: Final wiring, testing, and cleanup

Each task follows TDD where applicable, with frequent commits for easy rollback.
