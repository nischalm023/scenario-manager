# Zone-Balanced Initial Inventory Placement Design

**Goal**: Balance initial inventory placement across zones to prevent VTM bottlenecks, while maintaining reasonable distance optimization.

**Problem**: Current `GreedyPutStrategy` places popular SKUs in closest storage spaces, concentrating A-tier SKUs in zones nearest stations. Runtime zone load balancing cannot compensate for fundamentally imbalanced initial state.

---

## Core Concept

`GridPopularityManager.getPopularity()` returns a hybrid score when zone load balancing is enabled:

```
hybridScore = (1 - ZONE_BALANCE_WEIGHT) * distanceScore + ZONE_BALANCE_WEIGHT * zoneLoadScore
```

Where:
- `ZONE_BALANCE_WEIGHT = 0.7` (constant in file)
- `distanceScore`: Existing normalized distance (1 = closest)
- `zoneLoadScore`: Based on deviation from fair share

```java
idealPerZone = totalPopularityPoints / numZones
zoneLoadScore = 1 - min(1, zoneLoad / (idealPerZone * 1.5))
```

**Rationale**: HTMs are fast in practice, so VTM utilization is the primary bottleneck. Zone balance matters more than raw distance (hence 0.7 weight).

**Zone Load Metric**: Sum of `pastOrderedQuantities` (popularity points) for all SKUs placed in zone. Directly measures expected demand/workload.

---

## Behavior

- **RTP or balancing disabled**: Pure distance-based scoring (current behavior)
- **Relay with balancing enabled**: Hybrid scoring that spreads popular SKUs across zones

**Visualization Impact**: Frontend map will show more even coloring across zones (reflecting the PUT pattern) instead of pure distance gradient.

**Scope**: Only `GreedyPutStrategy` uses zone balancing. LP strategy (`MinWeightedCycleDistanceStrategy`) remains distance-optimized as a benchmark.

---

## Component Changes

### 1. GridPopularityManager

**New fields**:
```java
private static final double ZONE_BALANCE_WEIGHT = 0.7;

private ZoneManager zoneManager;  // nullable
private boolean zoneLoadBalancingEnabled = false;
private final Map<String, Long> zonePopularityPoints = new HashMap<>();
private long totalPopularityPoints = 0;
```

**New methods**:

```java
/**
 * Enable zone load balancing (call after ZoneManager is created).
 */
public void enableZoneLoadBalancing(ZoneManager zoneManager) {
  this.zoneManager = zoneManager;
  this.zoneLoadBalancingEnabled = true;
  // Initialize all zones to 0
  for (String zoneId : zoneManager.getZones().keySet()) {
    zonePopularityPoints.put(zoneId, 0L);
  }
}

/**
 * Record placement for zone load tracking.
 * Call after placing SKU in a storage space.
 */
public void recordPlacement(GridCoordinate pos, int popularityPoints) {
  if (!zoneLoadBalancingEnabled) return;
  String zoneId = zoneManager.getZoneForPosition(pos);
  if (zoneId != null) {
    zonePopularityPoints.merge(zoneId, (long) popularityPoints, Long::sum);
    totalPopularityPoints += popularityPoints;
  }
}

/**
 * Get zone load score (1 = least loaded, 0 = most loaded).
 */
private double getZoneLoadScore(String zoneId) {
  if (zonePopularityPoints.isEmpty()) return 1.0;

  int numZones = zonePopularityPoints.size();
  double idealPerZone = (double) totalPopularityPoints / numZones;
  if (idealPerZone == 0) return 1.0;

  long zoneLoad = zonePopularityPoints.getOrDefault(zoneId, 0L);
  return 1 - Math.min(1.0, zoneLoad / (idealPerZone * 1.5));
}
```

**Modified `getPopularity()`**:
```java
public double getPopularity(GridCoordinate pos) {
  double distanceScore = popularityScores.getOrDefault(pos, 0.0);

  if (!zoneLoadBalancingEnabled || zoneManager == null) {
    return distanceScore;
  }

  String zoneId = zoneManager.getZoneForPosition(pos);
  if (zoneId == null) {
    return distanceScore;
  }

  double zoneLoadScore = getZoneLoadScore(zoneId);
  return (1 - ZONE_BALANCE_WEIGHT) * distanceScore + ZONE_BALANCE_WEIGHT * zoneLoadScore;
}
```

---

### 2. GreedyPutStrategy

**Key change**: Re-sort storage spaces by `getPopularity()` before placing each SKU, and record placements.

**Modified `initializeInventory()`**:

```java
@Override
public void initializeInventory() {
  log.info("Using greedy algorithm for inventory initialization...");
  long startTime = System.currentTimeMillis();

  // ... existing metadata indexing ...

  Map<String, Integer> pastOrderedQuantities = itemCatalog.getPastOrderedQuantities();

  // Pre-compute storage space coordinates for sorting
  List<StorageSpaceWithCoord> storageSpaceList = new ArrayList<>();
  for (WarehouseInventory.StorageSpace ss : storageSpaces) {
    GridCoordinate coord = getCoordinate(ss);
    if (coord != null) {
      storageSpaceList.add(new StorageSpaceWithCoord(ss.getId(), coord));
    }
  }

  // Iterate through SKUs (sorted by popularity descending)
  for (String sku : sortedSkus) {
    int maxInv = maxInventoryQuantities.getOrDefault(sku, 0);
    if (maxInv == 0) continue;

    int skuPopularity = pastOrderedQuantities.getOrDefault(sku, 0);
    int targetInv = computeTargetInventory(maxInv);
    int remaining = targetInv;

    // Re-sort by current hybrid popularity score (zone loads have changed)
    storageSpaceList.sort((a, b) -> Double.compare(
        gridPopularityManager.getPopularity(b.coord),
        gridPopularityManager.getPopularity(a.coord)));

    for (StorageSpaceWithCoord ssCoord : storageSpaceList) {
      if (remaining <= 0) break;

      // ... existing capacity checks ...

      if (qtyToPlace > 0) {
        inventory.addItemToStorageSpace(ssCoord.id, sku, qtyToPlace);

        // Record placement for zone load tracking
        gridPopularityManager.recordPlacement(ssCoord.coord, skuPopularity);

        // ... existing capacity tracking updates ...
        remaining -= qtyToPlace;
      }
    }
  }

  // ... existing logging and export ...
}

// Helper record
private record StorageSpaceWithCoord(StorageSpaceId id, GridCoordinate coord) {}
```

**Note**: The `recordPlacement()` call passes `skuPopularity` (the total past ordered quantity for this SKU), not `qtyToPlace`. This ensures zone load reflects expected demand, not just units placed.

---

### 3. DesmojSimulationEngine

**Location**: After `initializeRelaySystem()`, before PUT strategy runs.

```java
GridPopularityManager gridPopularityManager = new GridPopularityManager(warehouseMap);
// ... create inventory, catalog ...

initializeRelaySystem();  // Creates ZoneManager

// Enable zone balancing if relay system with balancing enabled
if (config.systemType() == SystemType.RELAY
    && config.algorithmConfig().relayZoneLoadBalancingEnabled()
    && zoneManager != null) {
  gridPopularityManager.enableZoneLoadBalancing(zoneManager);
  log.info("Zone load balancing enabled for inventory initialization");
}

// ... create PUT strategy and run initializeInventory() ...
```

**Note**: The same `relayZoneLoadBalancingEnabled` config flag controls both initial inventory placement and runtime order selection.

---

### 4. Frontend UI Control

**Location**: `ConfigurationPanel.tsx` - Algorithm section, after Order selection algorithm

**Behavior**:
- Only visible when `systemType === 'relay'`
- Checkbox for `relayZoneLoadBalancingEnabled`
- Defaults to `true` (matching backend default)

**Component addition** (after order selection algorithm, around line 488):

```tsx
{config.systemType === 'relay' && (
  <div className="flex items-center space-x-2 mt-3">
    <Checkbox
      id="relayZoneLoadBalancing"
      checked={config.relayZoneLoadBalancingEnabled ?? true}
      onCheckedChange={(checked) =>
        onConfigChange('relayZoneLoadBalancingEnabled', checked)
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

**Type changes** (in `SimulationConfig` type):

```typescript
interface SimulationConfig {
  // ... existing fields ...
  relayZoneLoadBalancingEnabled?: boolean;  // defaults to true
}
```

---

## Unchanged Components

- `MinWeightedCycleDistanceStrategy` (LP stays distance-optimized as benchmark)
- `WarehouseMapService` (preview shows distance-only popularity)
- `WarehouseMapController`
- Runtime zone load balancing (already implemented)
- Frontend map components (automatically reflect new scores in simulation results)

---

## Testing

### Unit Tests

**GridPopularityManager**:
- Verify `getPopularity()` returns distance-based score when disabled
- Verify hybrid score when enabled
- Verify zone load scores converge toward balance as placements recorded
- Verify `recordPlacement()` accumulates popularity points per zone

**GreedyPutStrategy**:
- Mock `GridPopularityManager` to verify `recordPlacement()` called with SKU popularity
- Verify re-sorting happens per SKU

### Integration Test

- Run relay simulation with zone balancing enabled
- Assert zone popularity points are within X% of each other after initialization
- Compare VTM utilization variance with/without balancing

---

## Expected Outcome

After initialization, each zone should have roughly equal `totalPopularityPoints / numZones` popularity points, leading to:
- Balanced VTM workload during simulation
- Even coloring in frontend map visualization (in simulation results)
- Reduced variance in VTM utilization metrics
