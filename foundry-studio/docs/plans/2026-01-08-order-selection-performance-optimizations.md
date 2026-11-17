# Order Selection Performance Optimizations

**Date:** 2026-01-08
**Status:** Implemented (may need re-application)
**JFR Profiling:** Identified hot spots in `MaximizePpfWithTieringStrategy.selectOrders()` and `AbstractOrderSelectionStrategy.prepareOrderCandidates()`

---

## Quick Re-Apply Checklist

After making changes, run:
```bash
cd apps/fleet-simulator && mvn spotless:apply
npx nx test fleet-simulator
```

---

## Optimization 1: Tiering Strategy Scoring Loop

### Problem
JFR profiling showed `HashMap$HashIterator.nextNode()` and `retainAll(Collection)` as hot spots. The inner scoring loop created 3 new HashSets per order and performed 5 set operations (`retainAll`, `removeAll`) for each candidate scored.

### Solution
Precompute `activeStorageSpaces` intersection once per candidate, then use iteration-based counting in the scoring loop.

### File to Modify
`apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfWithTieringStrategy.java`

### Step-by-Step Changes

#### Step 1.1: Add HashMap Import
**Location:** After line 11 (imports section)

Add this import:
```java
import java.util.HashMap;
```

Full imports should include:
```java
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
```

#### Step 1.2: Add ScoringCandidate Record
**Location:** Inside the class, after line 31 (after `public class MaximizePpfWithTieringStrategy extends AbstractOrderSelectionStrategy {`)

Add this record:
```java
  /**
   * Scoring candidate with precomputed active storage space overlap. This avoids creating HashSets
   * in the inner scoring loop.
   */
  private record ScoringCandidate(
      OrderCandidate base, Set<StorageSpaceId> activeOverlap, int activeOverlapCount) {}
```

#### Step 1.3: Replace the Entire selectOrders() Method
**Location:** Replace the entire `selectOrders()` method (from `@Override` to closing brace)

**REPLACE THIS (original):**
```java
  @Override
  public List<OrderSelection> selectOrders(
      List<Order> pendingOrders,
      Set<StorageSpaceId> activeStorageSpaces,
      List<PickTask> processingTasks,
      int maxOrders,
      double currentTime) {

    // PPF strategies don't use processingTasks (not needed for face calculation)

    // Step 1: Prepare candidate pool with precomputed fulfillment plans
    Map<String, OrderCandidate> candidates =
        prepareOrderCandidates(pendingOrders, currentTime, activeStorageSpaces);

    if (candidates.isEmpty()) {
      return List.of();
    }

    // Step 2: Group by zone load bucket (TreeMap ensures sorted by bucket)
    Map<Integer, List<OrderCandidate>> buckets = groupCandidatesByZoneLoadBucket(candidates);

    List<OrderSelection> selectedOrders = new ArrayList<>();
    Set<StorageSpaceId> newlyOpenedStorageSpaces = new HashSet<>();
    Set<String> processedOrderIds = new HashSet<>();

    // Step 3: Process buckets in order (lowest load first)
    for (List<OrderCandidate> bucketCandidates : buckets.values()) {
      if (selectedOrders.size() >= maxOrders) {
        break;
      }

      // Build list of order IDs in this bucket that haven't been processed
      List<String> validOrderIds = new ArrayList<>();
      for (OrderCandidate c : bucketCandidates) {
        if (!processedOrderIds.contains(c.order().getOrderId())) {
          validOrderIds.add(c.order().getOrderId());
        }
      }

      // Apply tiered scoring within this bucket
      while (selectedOrders.size() < maxOrders && !validOrderIds.isEmpty()) {
        String bestOrderId = null;
        double bestScore = Double.NEGATIVE_INFINITY;
        Set<StorageSpaceId> bestTrulyNewStorageSpaces = new HashSet<>();

        // Score all remaining orders in this bucket
        for (String orderId : validOrderIds) {
          OrderCandidate candidate = candidates.get(orderId);
          Set<StorageSpaceId> requiredStorageSpaces = candidate.requiredStorageSpaces();
          int totalTasksQuantity = candidate.totalTasksQuantity();

          // Calculate which storage spaces are truly new
          Set<StorageSpaceId> alreadyActiveStorageSpaces = new HashSet<>(requiredStorageSpaces);
          alreadyActiveStorageSpaces.retainAll(activeStorageSpaces);

          Set<StorageSpaceId> alreadyOpenedThisRound = new HashSet<>(requiredStorageSpaces);
          alreadyOpenedThisRound.retainAll(newlyOpenedStorageSpaces);

          Set<StorageSpaceId> trulyNewStorageSpaces = new HashSet<>(requiredStorageSpaces);
          trulyNewStorageSpaces.removeAll(activeStorageSpaces);
          trulyNewStorageSpaces.removeAll(newlyOpenedStorageSpaces);

          // Tiered scoring system
          int newStorageSpacePenalty = trulyNewStorageSpaces.size() * 1000;
          int activeReuseBonus = alreadyActiveStorageSpaces.size() * 100;
          int openedReuseBonus = alreadyOpenedThisRound.size() * 100;

          double finalScore =
              totalTasksQuantity + activeReuseBonus + openedReuseBonus - newStorageSpacePenalty;

          if (finalScore > bestScore) {
            bestScore = finalScore;
            bestOrderId = orderId;
            bestTrulyNewStorageSpaces = trulyNewStorageSpaces;
          }
        }

        if (bestOrderId != null) {
          OrderCandidate bestCandidate = candidates.get(bestOrderId);
          StationId stationId =
              selectOptimalStationForOrder(bestCandidate.order(), bestCandidate.tasks());

          selectedOrders.add(
              new OrderSelection(bestCandidate.order(), bestCandidate.tasks(), stationId));
          newlyOpenedStorageSpaces.addAll(bestTrulyNewStorageSpaces);
          validOrderIds.remove(bestOrderId);
          processedOrderIds.add(bestOrderId);
        } else {
          break;
        }
      }
    }

    // Log results
    if (!selectedOrders.isEmpty()) {
      log.debug(
          "Tiering strategy: {} orders selected, {} new storage spaces opened",
          selectedOrders.size(),
          newlyOpenedStorageSpaces.size());
    }

    return selectedOrders;
  }
```

**WITH THIS (optimized):**
```java
  @Override
  public List<OrderSelection> selectOrders(
      List<Order> pendingOrders,
      Set<StorageSpaceId> activeStorageSpaces,
      List<PickTask> processingTasks,
      int maxOrders,
      double currentTime) {

    // PPF strategies don't use processingTasks (not needed for face calculation)

    // Step 1: Prepare candidate pool with precomputed fulfillment plans
    Map<String, OrderCandidate> candidates =
        prepareOrderCandidates(pendingOrders, currentTime, activeStorageSpaces);

    if (candidates.isEmpty()) {
      return List.of();
    }

    // Step 1.5: Precompute active storage space overlap for each candidate (done once per call)
    // This avoids creating HashSets in the inner scoring loop
    Map<String, ScoringCandidate> scoringCandidates = new HashMap<>();
    for (OrderCandidate candidate : candidates.values()) {
      Set<StorageSpaceId> activeOverlap = new HashSet<>(candidate.requiredStorageSpaces());
      activeOverlap.retainAll(activeStorageSpaces);
      scoringCandidates.put(
          candidate.order().getOrderId(),
          new ScoringCandidate(candidate, activeOverlap, activeOverlap.size()));
    }

    // Step 2: Group by zone load bucket (TreeMap ensures sorted by bucket)
    Map<Integer, List<OrderCandidate>> buckets = groupCandidatesByZoneLoadBucket(candidates);

    List<OrderSelection> selectedOrders = new ArrayList<>();
    Set<StorageSpaceId> newlyOpenedStorageSpaces = new HashSet<>();
    Set<String> processedOrderIds = new HashSet<>();

    // Step 3: Process buckets in order (lowest load first)
    for (List<OrderCandidate> bucketCandidates : buckets.values()) {
      if (selectedOrders.size() >= maxOrders) {
        break;
      }

      // Build list of order IDs in this bucket that haven't been processed
      List<String> validOrderIds = new ArrayList<>();
      for (OrderCandidate c : bucketCandidates) {
        if (!processedOrderIds.contains(c.order().getOrderId())) {
          validOrderIds.add(c.order().getOrderId());
        }
      }

      // Apply tiered scoring within this bucket
      while (selectedOrders.size() < maxOrders && !validOrderIds.isEmpty()) {
        String bestOrderId = null;
        double bestScore = Double.NEGATIVE_INFINITY;

        // Score all remaining orders in this bucket using precomputed overlaps
        for (String orderId : validOrderIds) {
          ScoringCandidate sc = scoringCandidates.get(orderId);
          Set<StorageSpaceId> requiredStorageSpaces = sc.base().requiredStorageSpaces();
          int totalTasksQuantity = sc.base().totalTasksQuantity();

          // Precomputed - no allocation needed
          int activeReuseBonus = sc.activeOverlapCount() * 100;

          // Count overlaps via iteration (no HashSet allocation)
          int openedReuseCount = 0;
          int trulyNewCount = 0;
          for (StorageSpaceId id : requiredStorageSpaces) {
            boolean inActive = sc.activeOverlap().contains(id);
            boolean inNewlyOpened = newlyOpenedStorageSpaces.contains(id);

            if (inNewlyOpened) {
              openedReuseCount++;
            }
            if (!inActive && !inNewlyOpened) {
              trulyNewCount++;
            }
          }

          // Tiered scoring system
          int openedReuseBonus = openedReuseCount * 100;
          int newStorageSpacePenalty = trulyNewCount * 1000;

          double finalScore =
              totalTasksQuantity + activeReuseBonus + openedReuseBonus - newStorageSpacePenalty;

          if (finalScore > bestScore) {
            bestScore = finalScore;
            bestOrderId = orderId;
          }
        }

        if (bestOrderId != null) {
          ScoringCandidate bestSc = scoringCandidates.get(bestOrderId);

          // Compute trulyNew set only for the winner (lazy computation)
          Set<StorageSpaceId> bestTrulyNewStorageSpaces = new HashSet<>();
          for (StorageSpaceId id : bestSc.base().requiredStorageSpaces()) {
            if (!activeStorageSpaces.contains(id) && !newlyOpenedStorageSpaces.contains(id)) {
              bestTrulyNewStorageSpaces.add(id);
            }
          }

          StationId stationId =
              selectOptimalStationForOrder(bestSc.base().order(), bestSc.base().tasks());

          selectedOrders.add(
              new OrderSelection(bestSc.base().order(), bestSc.base().tasks(), stationId));
          newlyOpenedStorageSpaces.addAll(bestTrulyNewStorageSpaces);
          validOrderIds.remove(bestOrderId);
          processedOrderIds.add(bestOrderId);
        } else {
          break;
        }
      }
    }

    // Log results
    if (!selectedOrders.isEmpty()) {
      log.debug(
          "Tiering strategy: {} orders selected, {} new storage spaces opened",
          selectedOrders.size(),
          newlyOpenedStorageSpaces.size());
    }

    return selectedOrders;
  }
```

#### Precompute Active Overlap (after prepareOrderCandidates)
```java
// Step 1.5: Precompute active storage space overlap for each candidate (done once per call)
Map<String, ScoringCandidate> scoringCandidates = new HashMap<>();
for (OrderCandidate candidate : candidates.values()) {
  Set<StorageSpaceId> activeOverlap = new HashSet<>(candidate.requiredStorageSpaces());
  activeOverlap.retainAll(activeStorageSpaces);
  scoringCandidates.put(
      candidate.order().getOrderId(),
      new ScoringCandidate(candidate, activeOverlap, activeOverlap.size()));
}
```

#### Replace Inner Loop (iteration-based counting)
```java
for (String orderId : validOrderIds) {
  ScoringCandidate sc = scoringCandidates.get(orderId);
  Set<StorageSpaceId> requiredStorageSpaces = sc.base().requiredStorageSpaces();
  int totalTasksQuantity = sc.base().totalTasksQuantity();

  // Precomputed - no allocation needed
  int activeReuseBonus = sc.activeOverlapCount() * 100;

  // Count overlaps via iteration (no HashSet allocation)
  int openedReuseCount = 0;
  int trulyNewCount = 0;
  for (StorageSpaceId id : requiredStorageSpaces) {
    boolean inActive = sc.activeOverlap().contains(id);
    boolean inNewlyOpened = newlyOpenedStorageSpaces.contains(id);

    if (inNewlyOpened) openedReuseCount++;
    if (!inActive && !inNewlyOpened) trulyNewCount++;
  }

  int openedReuseBonus = openedReuseCount * 100;
  int newStorageSpacePenalty = trulyNewCount * 1000;

  double finalScore =
      totalTasksQuantity + activeReuseBonus + openedReuseBonus - newStorageSpacePenalty;

  if (finalScore > bestScore) {
    bestScore = finalScore;
    bestOrderId = orderId;
  }
}
```

#### Lazy trulyNew Set Computation (only for winner)
```java
if (bestOrderId != null) {
  ScoringCandidate bestSc = scoringCandidates.get(bestOrderId);

  // Compute trulyNew set only for the winner (lazy computation)
  Set<StorageSpaceId> bestTrulyNewStorageSpaces = new HashSet<>();
  for (StorageSpaceId id : bestSc.base().requiredStorageSpaces()) {
    if (!activeStorageSpaces.contains(id) && !newlyOpenedStorageSpaces.contains(id)) {
      bestTrulyNewStorageSpaces.add(id);
    }
  }
  // ... rest of selection logic
}
```

### Expected Improvement
- **Before:** 3 HashSet allocations + 5 set operations per candidate per iteration
- **After:** 1 HashSet allocation per candidate (precomputed) + O(n) iteration for counting
- For 100 candidates: ~300 → ~100 HashSet allocations per call

---

## Optimization 2: OrderCandidate Fulfillment Plan Caching

### Problem
`prepareOrderCandidates()` called `generateTasksWithSetCover()` for **every pending order, every cycle**. With 500 pending orders, this ran 500× expensive set cover calculations per cycle, even though order items never change and inventory changes minimally.

### Solution
Cache `OrderCandidate` results with SKU-level inventory-aware invalidation. Only regenerate fulfillment plans when relevant inventory changes.

### Files to Modify
1. `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventory.java`
2. `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/order/strategies/AbstractOrderSelectionStrategy.java`

---

### Step-by-Step: WarehouseInventory.java

**File:** `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventory.java`

#### Step 2.1: Add Import for HashSet
**Location:** After line 8 (in imports section)

Ensure this import exists:
```java
import java.util.HashSet;
```

#### Step 2.2: Add Dirty SKU Tracking Field
**Location:** After line 29 (after `reservationsByOrder` field)

Add this field:
```java
  private final Set<String> dirtySkus = ConcurrentHashMap.newKeySet();
```

#### Step 2.3: Add getAndClearDirtySkus() Method
**Location:** After line 47 (after `getAvailableStock` method, around line 48)

Add this method:
```java
  /**
   * Get SKUs that have had inventory changes since last call, and clear the set.
   * Used by OrderCandidate caching to invalidate affected fulfillment plans.
   *
   * @return Set of SKUs with inventory changes (consumed or reserved)
   */
  public Set<String> getAndClearDirtySkus() {
    Set<String> result = new HashSet<>(dirtySkus);
    dirtySkus.clear();
    return result;
  }
```

#### Step 2.4: Modify reserveStock() Method
**Location:** Line 52 (inside `reserveStock` method)

**REPLACE THIS:**
```java
  public void reserveStock(String sku, StorageSpaceId storageSpaceId, int quantity) {
    var key = new ReservationKey(storageSpaceId, sku);
    reservedStock.merge(key, quantity, Integer::sum);
  }
```

**WITH THIS:**
```java
  public void reserveStock(String sku, StorageSpaceId storageSpaceId, int quantity) {
    var key = new ReservationKey(storageSpaceId, sku);
    reservedStock.merge(key, quantity, Integer::sum);
    dirtySkus.add(sku);
  }
```

#### Step 2.5: Modify consumeStock() Method
**Location:** Line 55-71 (the `consumeStock` method)

**REPLACE THIS:**
```java
  public void consumeStock(String sku, StorageSpaceId storageSpaceId, int quantity) {
    storageSpaceInventory.computeIfPresent(
        storageSpaceId,
        (id, inventory) -> {
          inventory.compute(
              sku, (s, current) -> Math.max(0, (current != null ? current : 0) - quantity));
          return inventory;
        });

    var key = new ReservationKey(storageSpaceId, sku);
    reservedStock.computeIfPresent(
        key,
        (k, reserved) -> {
          int newReserved = Math.max(0, reserved - quantity);
          return newReserved == 0 ? null : newReserved;
        });
  }
```

**WITH THIS:**
```java
  public void consumeStock(String sku, StorageSpaceId storageSpaceId, int quantity) {
    storageSpaceInventory.computeIfPresent(
        storageSpaceId,
        (id, inventory) -> {
          inventory.compute(
              sku, (s, current) -> Math.max(0, (current != null ? current : 0) - quantity));
          return inventory;
        });

    var key = new ReservationKey(storageSpaceId, sku);
    reservedStock.computeIfPresent(
        key,
        (k, reserved) -> {
          int newReserved = Math.max(0, reserved - quantity);
          return newReserved == 0 ? null : newReserved;
        });
    dirtySkus.add(sku);
  }
```

---

### Step-by-Step: AbstractOrderSelectionStrategy.java

**File:** `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/order/strategies/AbstractOrderSelectionStrategy.java`

#### Step 2.6: Add Iterator Import
**Location:** After line 23 (in imports section)

Add this import:
```java
import java.util.Iterator;
```

Full imports should include:
```java
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.TreeMap;
import java.util.stream.Collectors;
```

#### Step 2.7: Add Cache Fields
**Location:** After line 57 (after `protected ZoneManager zoneManager;`)

Add these fields:
```java
  // OrderCandidate cache for avoiding repeated set cover calculations
  private final Map<String, OrderCandidate> orderCandidateCache = new HashMap<>();
  private final Map<String, Set<String>> orderSkuDependencies = new HashMap<>();
```

#### Step 2.8: Replace prepareOrderCandidates() Method
**Location:** Lines 214-242 (the entire `prepareOrderCandidates` method)

**REPLACE THIS (original):**
```java
  /**
   * Build candidate pool with precomputed fulfillment plans.
   *
   * <p>For each pending order, generates tasks using set cover and calculates derived properties
   * (storage spaces needed, picks, PPF). Orders that cannot be fulfilled are excluded from the
   * result.
   *
   * @param pendingOrders Available orders to process
   * @param currentTime Simulation time
   * @param activeStorageSpaces Storage spaces currently active (for piggybacking)
   * @return Map of order ID to candidate data (excludes unfulfillable orders)
   */
  protected Map<String, OrderCandidate> prepareOrderCandidates(
      List<Order> pendingOrders, double currentTime, Set<StorageSpaceId> activeStorageSpaces) {

    Map<String, OrderCandidate> candidates = new HashMap<>();

    for (Order order : pendingOrders) {
      // Use set cover to generate fulfillment plan
      List<PickTask> tasks = generateTasksWithSetCover(order, currentTime, activeStorageSpaces);

      if (!tasks.isEmpty()) {
        Set<StorageSpaceId> requiredStorageSpaces =
            tasks.stream().map(PickTask::getStorageSpaceId).collect(Collectors.toSet());

        int totalTasksQuantity = tasks.stream().mapToInt(PickTask::getQuantity).sum();

        double picksPerFace =
            requiredStorageSpaces.isEmpty()
                ? 0
                : (double) totalTasksQuantity / requiredStorageSpaces.size();

        candidates.put(
            order.getOrderId(),
            new OrderCandidate(
                order, tasks, requiredStorageSpaces, totalTasksQuantity, picksPerFace));
      }
    }

    return candidates;
  }
```

**WITH THIS (cached):**
```java
  /**
   * Build candidate pool with precomputed fulfillment plans.
   *
   * <p>For each pending order, generates tasks using set cover and calculates derived properties
   * (storage spaces needed, picks, PPF). Orders that cannot be fulfilled are excluded from the
   * result.
   *
   * <p>Uses caching with SKU-level invalidation: fulfillment plans are cached and only regenerated
   * when inventory for relevant SKUs changes (via dirtySkus tracking in WarehouseInventory).
   *
   * @param pendingOrders Available orders to process
   * @param currentTime Simulation time
   * @param activeStorageSpaces Storage spaces currently active (for piggybacking)
   * @return Map of order ID to candidate data (excludes unfulfillable orders)
   */
  protected Map<String, OrderCandidate> prepareOrderCandidates(
      List<Order> pendingOrders, double currentTime, Set<StorageSpaceId> activeStorageSpaces) {

    // Step 1: Get dirty SKUs and invalidate affected cache entries
    Set<String> dirtySkus = inventory.getAndClearDirtySkus();
    if (!dirtySkus.isEmpty()) {
      Iterator<Map.Entry<String, Set<String>>> it = orderSkuDependencies.entrySet().iterator();
      while (it.hasNext()) {
        Map.Entry<String, Set<String>> entry = it.next();
        String orderId = entry.getKey();
        Set<String> orderSkus = entry.getValue();

        // Check if any of this order's SKUs are dirty
        for (String sku : orderSkus) {
          if (dirtySkus.contains(sku)) {
            orderCandidateCache.remove(orderId);
            it.remove();
            break;
          }
        }
      }
    }

    // Step 2: Build set of current pending order IDs for cleanup
    Set<String> pendingOrderIds =
        pendingOrders.stream().map(Order::getOrderId).collect(Collectors.toSet());

    // Step 3: Remove cache entries for orders no longer pending
    orderCandidateCache.keySet().retainAll(pendingOrderIds);
    orderSkuDependencies.keySet().retainAll(pendingOrderIds);

    // Step 4: Build candidates - use cache or generate
    Map<String, OrderCandidate> candidates = new HashMap<>();

    for (Order order : pendingOrders) {
      String orderId = order.getOrderId();

      // Check cache first
      OrderCandidate cached = orderCandidateCache.get(orderId);
      if (cached != null) {
        candidates.put(orderId, cached);
        continue;
      }

      // Generate new fulfillment plan
      List<PickTask> tasks = generateTasksWithSetCover(order, currentTime, activeStorageSpaces);

      if (!tasks.isEmpty()) {
        Set<StorageSpaceId> requiredStorageSpaces =
            tasks.stream().map(PickTask::getStorageSpaceId).collect(Collectors.toSet());

        int totalTasksQuantity = tasks.stream().mapToInt(PickTask::getQuantity).sum();

        double picksPerFace =
            requiredStorageSpaces.isEmpty()
                ? 0
                : (double) totalTasksQuantity / requiredStorageSpaces.size();

        OrderCandidate candidate =
            new OrderCandidate(
                order, tasks, requiredStorageSpaces, totalTasksQuantity, picksPerFace);

        // Cache the result
        orderCandidateCache.put(orderId, candidate);

        // Track SKU dependencies for invalidation
        Set<String> orderSkus =
            order.getItems().stream().map(oi -> oi.item().sku()).collect(Collectors.toSet());
        orderSkuDependencies.put(orderId, orderSkus);

        candidates.put(orderId, candidate);
      }
    }

    return candidates;
  }
```

### Expected Improvement
- **Before:** 500 orders × `generateTasksWithSetCover()` per cycle
- **After:** Only regenerate for orders with changed SKU inventory
- Typical case: 10-50 picks per cycle → ~10-50 cache invalidations vs 500 full regenerations

---

## Verification

### Unit Test File for Optimization 1
**File:** `apps/fleet-simulator/src/test/java/com/greyorange/dessimulator/domain/order/strategies/MaximizePpfWithTieringStrategyScoringTest.java`

This test file verifies that the optimized iteration-based scoring produces identical results to the original HashSet-based implementation.

**Create this file if it doesn't exist:**
```java
package com.greyorange.dessimulator.domain.order.strategies;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.greymatter.common.types.StorageSpaceId;
import com.greyorange.greymatter.common.types.ToteId;
import java.util.HashSet;
import java.util.Set;
import org.junit.jupiter.api.Test;

/**
 * Tests for the tiered scoring calculation in MaximizePpfWithTieringStrategy.
 *
 * <p>These tests verify that the optimized iteration-based scoring produces identical results to
 * the original HashSet-based implementation.
 *
 * <p>Scoring formula: finalScore = totalTasksQuantity + activeReuseBonus + openedReuseBonus -
 * newStorageSpacePenalty
 *
 * <p>Where: - activeReuseBonus = |required ∩ active| * 100 - openedReuseBonus = |required ∩
 * newlyOpened| * 100 - newStorageSpacePenalty = |required - active - newlyOpened| * 1000
 */
class MaximizePpfWithTieringStrategyScoringTest {

  /**
   * Reference implementation using HashSet operations (original algorithm). Used to verify the
   * optimized implementation produces identical results.
   */
  private ScoringResult originalScoring(
      Set<StorageSpaceId> required,
      Set<StorageSpaceId> active,
      Set<StorageSpaceId> newlyOpened,
      int totalTasksQuantity) {

    Set<StorageSpaceId> alreadyActiveStorageSpaces = new HashSet<>(required);
    alreadyActiveStorageSpaces.retainAll(active);

    Set<StorageSpaceId> alreadyOpenedThisRound = new HashSet<>(required);
    alreadyOpenedThisRound.retainAll(newlyOpened);

    Set<StorageSpaceId> trulyNewStorageSpaces = new HashSet<>(required);
    trulyNewStorageSpaces.removeAll(active);
    trulyNewStorageSpaces.removeAll(newlyOpened);

    int activeReuseBonus = alreadyActiveStorageSpaces.size() * 100;
    int openedReuseBonus = alreadyOpenedThisRound.size() * 100;
    int newStorageSpacePenalty = trulyNewStorageSpaces.size() * 1000;

    double finalScore =
        totalTasksQuantity + activeReuseBonus + openedReuseBonus - newStorageSpacePenalty;

    return new ScoringResult(
        activeReuseBonus, openedReuseBonus, newStorageSpacePenalty, finalScore);
  }

  /**
   * Optimized implementation using iteration-based counting. This is the algorithm used in the
   * optimized MaximizePpfWithTieringStrategy.
   */
  private ScoringResult optimizedScoring(
      Set<StorageSpaceId> required,
      Set<StorageSpaceId> activeOverlap, // precomputed: required ∩ active
      Set<StorageSpaceId> newlyOpened,
      int totalTasksQuantity) {

    int activeReuseBonus = activeOverlap.size() * 100;

    int openedReuseCount = 0;
    int trulyNewCount = 0;
    for (StorageSpaceId id : required) {
      boolean inActive = activeOverlap.contains(id);
      boolean inNewlyOpened = newlyOpened.contains(id);

      if (inNewlyOpened) {
        openedReuseCount++;
      }
      if (!inActive && !inNewlyOpened) {
        trulyNewCount++;
      }
    }

    int openedReuseBonus = openedReuseCount * 100;
    int newStorageSpacePenalty = trulyNewCount * 1000;

    double finalScore =
        totalTasksQuantity + activeReuseBonus + openedReuseBonus - newStorageSpacePenalty;

    return new ScoringResult(
        activeReuseBonus, openedReuseBonus, newStorageSpacePenalty, finalScore);
  }

  /** Helper to compute activeOverlap (required ∩ active) for optimized scoring. */
  private Set<StorageSpaceId> computeActiveOverlap(
      Set<StorageSpaceId> required, Set<StorageSpaceId> active) {
    Set<StorageSpaceId> overlap = new HashSet<>(required);
    overlap.retainAll(active);
    return overlap;
  }

  private record ScoringResult(
      int activeReuseBonus, int openedReuseBonus, int newStorageSpacePenalty, double finalScore) {}

  // Test helper to create ToteIds
  private StorageSpaceId tote(int id) {
    return new ToteId(id);
  }

  @Test
  void emptyRequiredSet_bothAlgorithmsMatch() {
    Set<StorageSpaceId> required = Set.of();
    Set<StorageSpaceId> active = Set.of(tote(1), tote(2));
    Set<StorageSpaceId> newlyOpened = Set.of(tote(3));
    int totalTasksQuantity = 5;

    Set<StorageSpaceId> activeOverlap = computeActiveOverlap(required, active);

    ScoringResult original = originalScoring(required, active, newlyOpened, totalTasksQuantity);
    ScoringResult optimized =
        optimizedScoring(required, activeOverlap, newlyOpened, totalTasksQuantity);

    assertEquals(original, optimized);
    assertEquals(5.0, original.finalScore()); // Just totalTasksQuantity, no bonuses/penalties
  }

  @Test
  void fullOverlapWithActive_bothAlgorithmsMatch() {
    // All required storage spaces are already active
    Set<StorageSpaceId> required = Set.of(tote(1), tote(2));
    Set<StorageSpaceId> active = Set.of(tote(1), tote(2), tote(3));
    Set<StorageSpaceId> newlyOpened = Set.of();
    int totalTasksQuantity = 10;

    Set<StorageSpaceId> activeOverlap = computeActiveOverlap(required, active);

    ScoringResult original = originalScoring(required, active, newlyOpened, totalTasksQuantity);
    ScoringResult optimized =
        optimizedScoring(required, activeOverlap, newlyOpened, totalTasksQuantity);

    assertEquals(original, optimized);
    // 10 + (2*100) + 0 - 0 = 210
    assertEquals(210.0, original.finalScore());
  }

  @Test
  void noOverlap_allNew_bothAlgorithmsMatch() {
    // No overlap - all required storage spaces are truly new
    Set<StorageSpaceId> required = Set.of(tote(1), tote(2), tote(3));
    Set<StorageSpaceId> active = Set.of(tote(10), tote(11));
    Set<StorageSpaceId> newlyOpened = Set.of(tote(20));
    int totalTasksQuantity = 15;

    Set<StorageSpaceId> activeOverlap = computeActiveOverlap(required, active);

    ScoringResult original = originalScoring(required, active, newlyOpened, totalTasksQuantity);
    ScoringResult optimized =
        optimizedScoring(required, activeOverlap, newlyOpened, totalTasksQuantity);

    assertEquals(original, optimized);
    // 15 + 0 + 0 - (3*1000) = -2985
    assertEquals(-2985.0, original.finalScore());
  }

  @Test
  void partialOverlap_bothAlgorithmsMatch() {
    // Some overlap with active, some with newlyOpened, some truly new
    Set<StorageSpaceId> required = Set.of(tote(1), tote(2), tote(3), tote(4), tote(5));
    Set<StorageSpaceId> active = Set.of(tote(1), tote(2), tote(10)); // 2 overlap
    Set<StorageSpaceId> newlyOpened = Set.of(tote(3), tote(20)); // 1 overlap
    int totalTasksQuantity = 20;

    Set<StorageSpaceId> activeOverlap = computeActiveOverlap(required, active);

    ScoringResult original = originalScoring(required, active, newlyOpened, totalTasksQuantity);
    ScoringResult optimized =
        optimizedScoring(required, activeOverlap, newlyOpened, totalTasksQuantity);

    assertEquals(original, optimized);
    // 20 + (2*100) + (1*100) - (2*1000) = 20 + 200 + 100 - 2000 = -1680
    assertEquals(-1680.0, original.finalScore());
  }

  @Test
  void overlapBetweenActiveAndNewlyOpened_bothAlgorithmsMatch() {
    // Edge case: same storage space is in both active AND newlyOpened
    Set<StorageSpaceId> required = Set.of(tote(1), tote(2), tote(3));
    Set<StorageSpaceId> active = Set.of(tote(1), tote(2)); // 2 overlap with required
    Set<StorageSpaceId> newlyOpened = Set.of(tote(2), tote(3)); // tote(2) also in active!
    int totalTasksQuantity = 10;

    Set<StorageSpaceId> activeOverlap = computeActiveOverlap(required, active);

    ScoringResult original = originalScoring(required, active, newlyOpened, totalTasksQuantity);
    ScoringResult optimized =
        optimizedScoring(required, activeOverlap, newlyOpened, totalTasksQuantity);

    assertEquals(original, optimized);
    // activeReuse: tote(1), tote(2) = 2 -> 200
    // openedReuse: tote(2), tote(3) = 2 -> 200
    // trulyNew: required - active - newlyOpened = {} = 0 -> 0
    // Note: tote(2) counts for BOTH bonuses, tote(3) only for openedReuse
    // 10 + 200 + 200 - 0 = 410
    assertEquals(410.0, original.finalScore());
  }

  @Test
  void largeSet_bothAlgorithmsMatch() {
    // Test with larger sets to ensure no edge cases
    Set<StorageSpaceId> required = new HashSet<>();
    Set<StorageSpaceId> active = new HashSet<>();
    Set<StorageSpaceId> newlyOpened = new HashSet<>();

    // Required: 1-100
    for (int i = 1; i <= 100; i++) {
      required.add(tote(i));
    }
    // Active: 1-30 (30 overlap)
    for (int i = 1; i <= 30; i++) {
      active.add(tote(i));
    }
    // NewlyOpened: 25-50 (25-30 overlap with active, 31-50 just newlyOpened = 26 total)
    for (int i = 25; i <= 50; i++) {
      newlyOpened.add(tote(i));
    }
    int totalTasksQuantity = 500;

    Set<StorageSpaceId> activeOverlap = computeActiveOverlap(required, active);

    ScoringResult original = originalScoring(required, active, newlyOpened, totalTasksQuantity);
    ScoringResult optimized =
        optimizedScoring(required, activeOverlap, newlyOpened, totalTasksQuantity);

    assertEquals(original, optimized);
    // activeReuse: 30 -> 3000
    // openedReuse: 26 (25-50 in required) -> 2600
    // trulyNew: 51-100 = 50 -> 50000
    // 500 + 3000 + 2600 - 50000 = -43900
    assertEquals(-43900.0, original.finalScore());
  }
}
```

### Run Tests
```bash
cd apps/fleet-simulator && mvn spotless:apply
npx nx test fleet-simulator
```

### Performance Verification
Run JFR profiling again:
```bash
java -XX:StartFlightRecording=duration=60s,settings=profile,filename=app.jfr -jar fleet-simulator.jar
```

Expected: `generateTasksWithSetCover` and `retainAll` samples should drop significantly.

---

## Architecture

```
WarehouseInventory                     AbstractOrderSelectionStrategy
┌─────────────────────┐               ┌─────────────────────────────────┐
│ dirtySkus: Set<Sku> │──────────────▶│ orderCandidateCache: Map<ID,OC> │
│                     │  getAndClear  │ orderSkuDeps: Map<ID, Set<Sku>> │
│ markDirty(sku)      │               │                                 │
└─────────────────────┘               └─────────────────────────────────┘
         ▲                                           │
         │ consumeStock()                            │ invalidate orders
         │ reserveStock()                            │ with dirty SKUs
```

## Cache Invalidation Rules
1. **Order selected** → Removed from pending → Cache entry cleaned up
2. **SKU inventory changes** → `dirtySkus.add(sku)` → Affected orders invalidated on next `prepareOrderCandidates()` call
3. **Order no longer pending** → `retainAll(pendingOrderIds)` cleans up stale entries
