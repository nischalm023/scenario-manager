# Zone-Balanced Initial Inventory Placement Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement zone-aware hybrid scoring in `GridPopularityManager` to balance initial inventory placement across zones, preventing VTM bottlenecks.

**Architecture:** Extend `GridPopularityManager` with zone load tracking and hybrid scoring (70% zone balance + 30% distance). `GreedyPutStrategy` re-sorts per SKU and records placements. Frontend checkbox controls feature via `relayZoneLoadBalancingEnabled` config.

**Tech Stack:** Java 21, Lombok, React/TypeScript, shadcn/ui

---

## Task 1: Add Zone Load Tracking Fields to GridPopularityManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java:1-70`

**Step 1: Add imports**

Add after line 4 (after `import com.greyorange.dessimulator.domain.map.WarehouseMap;`):

```java
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
```

**Step 2: Add new fields**

Add after line 13 (after `private final Map<GridCoordinate, Double> averageDistances;`):

```java
  private static final double ZONE_BALANCE_WEIGHT = 0.7;

  private ZoneManager zoneManager;
  private boolean zoneLoadBalancingEnabled = false;
  private final Map<String, Long> zonePopularityPoints = new HashMap<>();
  private long totalPopularityPoints = 0;
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java
git commit -m "feat: add zone load tracking fields to GridPopularityManager"
```

---

## Task 2: Add enableZoneLoadBalancing Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java`

**Step 1: Add method after getAverageDistance (after line 69)**

```java
  /**
   * Enable zone load balancing for inventory placement.
   *
   * <p>Call after ZoneManager is created but before PUT strategy runs.
   *
   * @param zoneManager Zone manager for coordinate-to-zone lookup
   */
  public void enableZoneLoadBalancing(ZoneManager zoneManager) {
    this.zoneManager = zoneManager;
    this.zoneLoadBalancingEnabled = true;
    // Initialize all zones to 0
    for (String zoneId : zoneManager.getZones().keySet()) {
      zonePopularityPoints.put(zoneId, 0L);
    }
    log.info(
        "Zone load balancing enabled with {} zones, weight={}",
        zonePopularityPoints.size(),
        ZONE_BALANCE_WEIGHT);
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java
git commit -m "feat: add enableZoneLoadBalancing method"
```

---

## Task 3: Add recordPlacement Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java`

**Step 1: Add method after enableZoneLoadBalancing**

```java
  /**
   * Record placement for zone load tracking.
   *
   * <p>Call after placing SKU in a storage space during initialization.
   *
   * @param pos Storage space coordinate
   * @param popularityPoints SKU popularity (pastOrderedQuantities)
   */
  public void recordPlacement(GridCoordinate pos, int popularityPoints) {
    if (!zoneLoadBalancingEnabled || zoneManager == null) {
      return;
    }
    Zone zone = zoneManager.getZoneForRack(pos);
    if (zone != null) {
      zonePopularityPoints.merge(zone.getZoneId(), (long) popularityPoints, Long::sum);
      totalPopularityPoints += popularityPoints;
    }
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java
git commit -m "feat: add recordPlacement method for zone load tracking"
```

---

## Task 4: Add getZoneLoadScore Private Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java`

**Step 1: Add private method after recordPlacement**

```java
  /**
   * Get zone load score (1 = least loaded, 0 = most loaded).
   *
   * <p>Score based on deviation from fair share (ideal = total / numZones).
   *
   * @param zoneId Zone identifier
   * @return Score between 0 and 1
   */
  private double getZoneLoadScore(String zoneId) {
    if (zonePopularityPoints.isEmpty()) {
      return 1.0;
    }

    int numZones = zonePopularityPoints.size();
    double idealPerZone = (double) totalPopularityPoints / numZones;
    if (idealPerZone == 0) {
      return 1.0;
    }

    long zoneLoad = zonePopularityPoints.getOrDefault(zoneId, 0L);
    return 1 - Math.min(1.0, zoneLoad / (idealPerZone * 1.5));
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java
git commit -m "feat: add getZoneLoadScore private method"
```

---

## Task 5: Modify getPopularity to Return Hybrid Score

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java:63-65`

**Step 1: Replace getPopularity method**

Replace:
```java
  public double getPopularity(GridCoordinate gridCoordinate) {
    return popularityScores.getOrDefault(gridCoordinate, 0.0);
  }
```

With:
```java
  /**
   * Get popularity score for a grid position.
   *
   * <p>When zone load balancing is enabled, returns hybrid score combining distance and zone load.
   * Otherwise returns pure distance-based score.
   *
   * @param gridCoordinate Position to score
   * @return Popularity score (0-1, higher is better)
   */
  public double getPopularity(GridCoordinate gridCoordinate) {
    double distanceScore = popularityScores.getOrDefault(gridCoordinate, 0.0);

    if (!zoneLoadBalancingEnabled || zoneManager == null) {
      return distanceScore;
    }

    Zone zone = zoneManager.getZoneForRack(gridCoordinate);
    if (zone == null) {
      return distanceScore;
    }

    double zoneLoadScore = getZoneLoadScore(zone.getZoneId());
    return (1 - ZONE_BALANCE_WEIGHT) * distanceScore + ZONE_BALANCE_WEIGHT * zoneLoadScore;
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/GridPopularityManager.java
git commit -m "feat: modify getPopularity to return hybrid score when zone balancing enabled"
```

---

## Task 6: Create GridPopularityManagerTest

**Files:**
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/GridPopularityManagerTest.java`

**Step 1: Write test file**

```java
package com.greyorange.dessimulator.domain.put;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.within;
import static org.mockito.Mockito.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.mockito.Mock;
import org.mockito.junit.jupiter.MockitoExtension;

@ExtendWith(MockitoExtension.class)
class GridPopularityManagerTest {

  @Mock private WarehouseMap warehouseMap;
  @Mock private ZoneManager zoneManager;

  private GridPopularityManager manager;

  @BeforeEach
  void setUp() {
    // Setup basic warehouse map with rack and station positions
    GridCoordinate rack1 = new GridCoordinate(5, 5);
    GridCoordinate rack2 = new GridCoordinate(5, 15);
    GridCoordinate rack3 = new GridCoordinate(5, 25);

    when(warehouseMap.getRackPositions()).thenReturn(List.of(rack1, rack2, rack3));
    when(warehouseMap.getStationPositions()).thenReturn(List.of(
        new com.greyorange.dessimulator.domain.common.coordinates.StationGridPosition(
            new GridCoordinate(0, 15), new GridCoordinate(1, 15),
            com.greyorange.dessimulator.domain.common.coordinates.StationEdge.BOTTOM)));

    manager = new GridPopularityManager(warehouseMap);
  }

  @Test
  void shouldReturnDistanceBasedScoreWhenZoneBalancingDisabled() {
    GridCoordinate pos = new GridCoordinate(5, 15); // closest to station
    double score = manager.getPopularity(pos);

    // Should be pure distance-based (1.0 for closest)
    assertThat(score).isEqualTo(1.0);
  }

  @Test
  void shouldReturnHybridScoreWhenZoneBalancingEnabled() {
    // Setup zone manager
    Zone zoneA = mock(Zone.class);
    Zone zoneB = mock(Zone.class);
    when(zoneManager.getZones()).thenReturn(Map.of("zone-A", zoneA, "zone-B", zoneB));
    when(zoneManager.getZoneForPosition(any())).thenReturn("zone-A");

    manager.enableZoneLoadBalancing(zoneManager);

    GridCoordinate pos = new GridCoordinate(5, 15);
    double score = manager.getPopularity(pos);

    // With no placements, zone load score = 1.0
    // hybrid = 0.3 * distanceScore + 0.7 * 1.0
    // For closest position, distanceScore = 1.0, so hybrid = 0.3 + 0.7 = 1.0
    assertThat(score).isCloseTo(1.0, within(0.01));
  }

  @Test
  void shouldAccumulatePopularityPointsPerZone() {
    Zone zoneA = mock(Zone.class);
    Zone zoneB = mock(Zone.class);
    when(zoneManager.getZones()).thenReturn(Map.of("zone-A", zoneA, "zone-B", zoneB));

    manager.enableZoneLoadBalancing(zoneManager);

    // Record placements in zone-A
    GridCoordinate pos1 = new GridCoordinate(5, 5);
    when(zoneManager.getZoneForPosition(pos1)).thenReturn("zone-A");
    manager.recordPlacement(pos1, 100);
    manager.recordPlacement(pos1, 200);

    // Record placement in zone-B
    GridCoordinate pos2 = new GridCoordinate(5, 25);
    when(zoneManager.getZoneForPosition(pos2)).thenReturn("zone-B");
    manager.recordPlacement(pos2, 50);

    // Zone-A has 300 points, zone-B has 50 points
    // Total = 350, ideal = 175
    // Zone-A score = 1 - min(1, 300 / (175 * 1.5)) = 1 - min(1, 1.14) = 0
    // Zone-B score = 1 - min(1, 50 / (175 * 1.5)) = 1 - min(1, 0.19) = 0.81

    when(zoneManager.getZoneForPosition(pos1)).thenReturn("zone-A");
    when(zoneManager.getZoneForPosition(pos2)).thenReturn("zone-B");

    double scoreA = manager.getPopularity(pos1);
    double scoreB = manager.getPopularity(pos2);

    // Zone-B (less loaded) should have higher score than zone-A
    assertThat(scoreB).isGreaterThan(scoreA);
  }

  @Test
  void shouldNotRecordPlacementWhenDisabled() {
    GridCoordinate pos = new GridCoordinate(5, 5);

    // Record placement without enabling zone balancing
    manager.recordPlacement(pos, 100);

    // Score should still be pure distance-based
    double score = manager.getPopularity(pos);
    assertThat(score).isLessThan(1.0); // This is farther from station
  }
}
```

**Step 2: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=GridPopularityManagerTest -q`
Expected: Tests run: 4, Failures: 0

**Step 3: Commit**

```bash
git add apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/GridPopularityManagerTest.java
git commit -m "test: add GridPopularityManager zone load balancing tests"
```

---

## Task 7: Update GreedyPutStrategy - Add Helper Record

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategy.java`

**Step 1: Add import for GridCoordinate**

Add after line 5 (after `import com.greyorange.dessimulator.domain.inventory.*;`):

```java
import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
```

**Step 2: Add new helper record after StorageSpaceDistance (after line 158)**

```java
  // Helper record for sorting with coordinates
  private record StorageSpaceWithCoord(StorageSpaceId id, GridCoordinate coord) {}
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategy.java
git commit -m "feat: add StorageSpaceWithCoord helper record to GreedyPutStrategy"
```

---

## Task 8: Update GreedyPutStrategy - Modify initializeInventory

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategy.java:39-150`

**Step 1: Replace initializeInventory method**

Replace the entire `initializeInventory` method with:

```java
  @Override
  public void initializeInventory() {
    log.info("Using greedy algorithm for inventory initialization...");
    long startTime = System.currentTimeMillis();

    // Pre-index storage space metadata to avoid O(m) searches
    MetadataIndex metadata = indexStorageSpaceMetadata();
    Map<StorageSpaceId, Integer> storageSpaceMaxCapacity = metadata.storageSpaceMaxCapacity();
    Map<StorageSpaceId, Integer> storageSpaceCurrentCapacity =
        metadata.storageSpaceCurrentCapacity();
    Map<StorageSpaceId, Integer> storageSpaceSkuCount = metadata.storageSpaceSkuCount();

    // Build ABC tier classification
    Map<String, Integer> skuMaxPerStorageSpace = buildAbcTierClassification();

    // Get SKUs sorted by popularity (descending)
    Map<String, Integer> pastOrderedQuantities = itemCatalog.getPastOrderedQuantities();
    Map<String, Integer> maxInventoryQuantities = itemCatalog.getMaxInventoryQuantities();

    List<String> sortedSkus =
        pastOrderedQuantities.entrySet().stream()
            .sorted((e1, e2) -> Integer.compare(e2.getValue(), e1.getValue()))
            .map(Map.Entry::getKey)
            .toList();

    // Build storage space list with coordinates for dynamic sorting
    List<StorageSpaceWithCoord> storageSpaceList = new ArrayList<>();
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      GridCoordinate coord = null;
      if (storageSpace instanceof Rack rack && rack.getCurrentCoordinate() != null) {
        coord = rack.getCurrentCoordinate();
      } else if (storageSpace instanceof Tote tote && tote.getCurrentCoordinate() != null) {
        coord = tote.getCurrentCoordinate();
      }
      if (coord != null) {
        storageSpaceList.add(new StorageSpaceWithCoord(storageSpace.getId(), coord));
      }
    }
    log.info("Built storage space list with {} entries", storageSpaceList.size());

    int placedCount = 0;
    int totalUnits = 0;

    // System-type aware SKU limit
    int maxSkusPerStorageSpace =
        (systemType == SystemType.RELAY) ? MAX_SKUS_PER_TOTE : MAX_SKUS_PER_RACK;

    // Iterate through SKUs and dynamically sort storage spaces
    for (String sku : sortedSkus) {
      int maxInv = maxInventoryQuantities.getOrDefault(sku, 0);
      if (maxInv == 0) {
        continue;
      }

      int skuPopularity = pastOrderedQuantities.getOrDefault(sku, 0);
      int targetInv = computeTargetInventory(maxInv);
      int remaining = targetInv;

      // Re-sort by current hybrid popularity score (zone loads have changed)
      storageSpaceList.sort(
          (a, b) ->
              Double.compare(
                  gridPopularityManager.getPopularity(b.coord),
                  gridPopularityManager.getPopularity(a.coord)));

      for (StorageSpaceWithCoord ssCoord : storageSpaceList) {
        if (remaining <= 0) {
          break;
        }

        StorageSpaceId storageSpaceId = ssCoord.id;

        // Use pre-indexed capacity (no O(m) search)
        int currentCapacity = storageSpaceCurrentCapacity.get(storageSpaceId);
        int currentSkus = storageSpaceSkuCount.get(storageSpaceId);
        int maxCapacity = storageSpaceMaxCapacity.get(storageSpaceId);

        // Early skip if storage space is full or has too many SKUs
        if (currentCapacity >= maxCapacity || currentSkus >= maxSkusPerStorageSpace) {
          continue;
        }

        // Apply ABC tier per-storage-space limit for this SKU
        int spaceAvailable = maxCapacity - currentCapacity;
        int maxAllowedForThisSku = skuMaxPerStorageSpace.get(sku);
        Map<String, Integer> spaceInventory =
            inventory.getStorageSpaceInventory().get(storageSpaceId);
        int currentSkuQtyInSpace = spaceInventory.getOrDefault(sku, 0);
        int skuSpaceAvailable = maxAllowedForThisSku - currentSkuQtyInSpace;

        // Place as much as possible within ABC tier limit
        int qtyToPlace = Math.min(remaining, Math.min(spaceAvailable, skuSpaceAvailable));

        if (qtyToPlace > 0) {
          // Update inventory
          inventory.addItemToStorageSpace(storageSpaceId, sku, qtyToPlace);

          // Record placement for zone load tracking
          gridPopularityManager.recordPlacement(ssCoord.coord, skuPopularity);

          // Update capacity tracking in-place
          storageSpaceCurrentCapacity.put(storageSpaceId, currentCapacity + qtyToPlace);

          // Update SKU count if this is a new SKU
          if (currentSkuQtyInSpace == 0) {
            storageSpaceSkuCount.put(storageSpaceId, currentSkus + 1);
          }

          remaining -= qtyToPlace;
          totalUnits += qtyToPlace;
        }
      }

      if (remaining < maxInv) {
        placedCount++;
      }
    }

    long elapsed = System.currentTimeMillis() - startTime;
    log.info("Greedy algorithm completed in {}ms", elapsed);
    log.info(
        "Placed {} SKUs, {} total units across {} storage spaces",
        placedCount,
        totalUnits,
        storageSpaces.size());

    exportInventoryAssignmentToCSV();
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategy.java
git commit -m "feat: update GreedyPutStrategy to use dynamic zone-balanced sorting"
```

---

## Task 9: Wire Zone Load Balancing in DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java:171-175`

**Step 1: Add zone balancing enablement after initializeRelaySystem call**

Find (around line 171-175):
```java
      // Initialize relay system components if needed
      if (config.systemType() == SystemType.RELAY) {
        initializeRelaySystem();
      }
```

Replace with:
```java
      // Initialize relay system components if needed
      if (config.systemType() == SystemType.RELAY) {
        initializeRelaySystem();

        // Enable zone load balancing for initial inventory placement
        if (config.algorithmConfig().relayZoneLoadBalancingEnabled() && zoneManager != null) {
          warehouseComponents.gridPopularityManager().enableZoneLoadBalancing(zoneManager);
          log.info("Zone load balancing enabled for inventory initialization");
        }
      }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "feat: wire zone load balancing to GridPopularityManager in simulation engine"
```

---

## Task 10: Add relayZoneLoadBalancingEnabled to Frontend SimulationConfig Type

**Files:**
- Modify: `apps/frontend/src/types/simulation.ts:91-115`

**Step 1: Add field to SimulationConfig interface**

Find the `SimulationConfig` interface (around line 91) and add after `tpqTarget?: number` (line 114):

```typescript
  relayZoneLoadBalancingEnabled?: boolean
```

So the interface becomes:
```typescript
export interface SimulationConfig {
  systemType: 'rtp' | 'relay'
  rtpRobotCount: number
  rtpRobotSpeed: number
  numVtms: number
  numHtms: number
  vtmSpeed: number
  htmSpeed: number
  simulationDuration: number
  operatorTime: number
  aisleSpacing: number
  stationCount: number
  binsCount: number
  picksPerFace: number
  orderSelectionAlgorithm: string
  putAlgorithm: string
  navFactor: number
  scenario: string
  xPathwayFrequency: number
  yPathwayFrequency: number
  mapConfig: MapConfig
  enableSnapshots: boolean
  snapshotIntervalSeconds: number
  tpqTarget?: number
  relayZoneLoadBalancingEnabled?: boolean
}
```

**Step 2: Commit**

```bash
git add apps/frontend/src/types/simulation.ts
git commit -m "feat: add relayZoneLoadBalancingEnabled to frontend SimulationConfig type"
```

---

## Task 11: Add Zone Load Balancing Checkbox to ConfigurationPanel

**Files:**
- Modify: `apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx:486-502`

**Step 1: Add checkbox after Navigation Factor section**

Find the Navigation Factor section (around line 490-501) and add after it (before the closing `</CollapsibleSection>`):

```tsx
            {config.systemType === 'relay' && (
              <div className="flex items-center space-x-2 mt-3">
                <Checkbox
                  id="relayZoneLoadBalancing"
                  checked={config.relayZoneLoadBalancingEnabled ?? true}
                  onCheckedChange={(checked) =>
                    onConfigChange('relayZoneLoadBalancingEnabled', checked === true)
                  }
                />
                <Label
                  htmlFor="relayZoneLoadBalancing"
                  className="text-xs text-gray-600 font-medium cursor-pointer"
                >
                  Enable Zone Load Balancing
                </Label>
              </div>
            )}
```

**Step 2: Run frontend typecheck**

Run: `cd apps/frontend && npm run check`
Expected: No errors

**Step 3: Commit**

```bash
git add apps/frontend/src/pages/simulator/components/ConfigurationPanel.tsx
git commit -m "feat: add zone load balancing checkbox to ConfigurationPanel"
```

---

## Task 12: Run Full Backend Test Suite

**Files:**
- All backend test files

**Step 1: Run all tests**

Run: `cd apps/backend && mvn clean test`
Expected: All tests pass

**Step 2: Fix any failures**

If tests fail, investigate and fix. Common issues:
- Missing mocks for zoneManager in existing tests
- Null pointer exceptions from zone tracking

**Step 3: Commit any fixes**

```bash
git add -A
git commit -m "fix: resolve test failures from zone load balancing integration"
```

---

## Task 13: Integration Test - Run Relay Simulation

**Files:**
- None (manual verification)

**Step 1: Start backend server**

```bash
rm -f /tmp/server.log
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
cd apps/backend && npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/health
```

**Step 2: Run relay simulation with zone balancing**

Create test config and run:
```bash
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d '{
    "simulationDuration": 3600,
    "scenarioId": "scenario_a",
    "systemType": "relay",
    "algorithmConfig": {
      "relayZoneLoadBalancingEnabled": true,
      "putAlgorithm": "greedy"
    }
  }' > /tmp/sim_result.json
```

**Step 3: Verify logs show zone balancing**

```bash
grep "Zone load balancing enabled" /tmp/server.log
```

Expected: Log message showing zone balancing was enabled

**Step 4: Cleanup**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
```

---

## Task 14: Format and Final Cleanup

**Files:**
- All modified files

**Step 1: Format Java code**

Run: `cd apps/backend && mvn spotless:apply`

**Step 2: Format frontend code**

Run: `cd apps/frontend && npm run check:fix`

**Step 3: Final commit**

```bash
git add -A
git commit -m "chore: format code for zone-balanced initial placement feature"
```

---

## Summary

This implementation plan consists of 14 tasks:

1. **Tasks 1-5**: `GridPopularityManager` zone load tracking and hybrid scoring
2. **Task 6**: Unit tests for `GridPopularityManager`
3. **Tasks 7-8**: `GreedyPutStrategy` dynamic sorting with zone awareness
4. **Task 9**: Wiring in `DesmojSimulationEngine`
5. **Tasks 10-11**: Frontend type and UI checkbox
6. **Tasks 12-14**: Testing and cleanup

Each task follows TDD where applicable, with frequent commits for easy rollback.
