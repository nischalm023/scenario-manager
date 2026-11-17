# WarehouseBuilder Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Improve WarehouseBuilder.java code quality with type-safe DTOs, builder safety, and documentation.

**Architecture:** Add nested record DTOs to replace `Map<String, Object>`, add reuse protection and phase-based validation, organize with section markers. Maintain backward compatibility by converting DTOs to Map format for existing callers.

**Tech Stack:** Java 21 records, Lombok @Slf4j

---

## Task 1: Add Section Markers and Expanded Javadoc

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java:24-52`

**Step 1: Replace class Javadoc with expanded version**

Find lines 24-29 and replace with:

```java
/**
 * Builds and initializes warehouse components for discrete event simulation.
 *
 * <p>Key responsibilities:
 * <ul>
 *   <li>Build warehouse map and layout from configuration</li>
 *   <li>Generate rack positions and populate with inventory</li>
 *   <li>Create robot fleet (RTP or Relay system based on config)</li>
 *   <li>Initialize station management and zone managers</li>
 * </ul>
 *
 * <p>This is a single-use builder that produces immutable WarehouseComponents.
 * Once {@link #build()} is called, the builder cannot be reused.
 *
 * <p>Build order (4 phases):
 * <ol>
 *   <li>Map - WarehouseMap and GridPopularityManager</li>
 *   <li>System-specific - ZoneManager and RelayPositionManager (RELAY only)</li>
 *   <li>Inventory - Racks, totes, item catalog, warehouse inventory</li>
 *   <li>Robots - StationManager and robot fleet</li>
 * </ol>
 */
```

**Step 2: Add section marker before fields**

After line 31 (`public class WarehouseBuilder {`), add:

```java

  // ========================================
  // Fields
  // ========================================
```

**Step 3: Compile to verify syntax**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "docs: expand WarehouseBuilder class Javadoc and add Fields section marker"
```

---

## Task 2: Add DTO Records

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Add Public DTOs section after class declaration**

After line 31 (`public class WarehouseBuilder {`), before the Fields section marker, add:

```java

  // ========================================
  // Public DTOs
  // ========================================

  /** Complete inventory state for metrics tracking. */
  public record InventoryState(
      Map<String, Integer> skuData,
      Map<Integer, StorageMetrics> storageData,
      InventorySummary summary) {}

  /** Metrics for a single storage space (rack or aggregated totes). */
  public record StorageMetrics(
      Position position,
      double popularityScore,
      int uniqueSkus,
      int totalQuantity,
      int popularQuantity,
      int regularQuantity,
      Map<String, Integer> inventory) {}

  /** Grid position for JSON serialization. */
  public record Position(int row, int col) {}

  /** Warehouse-level inventory summary. */
  public record InventorySummary(
      int totalSkus,
      int totalUnits,
      int popularSkus,
      int regularSkus,
      int totalPopularQuantity,
      int totalRegularQuantity,
      double popularPercentage,
      Map<String, SkuDetail> skuDetails) {}

  /** Details for a single SKU. */
  public record SkuDetail(int quantity, boolean isPopular) {}

```

**Step 2: Compile to verify records are valid**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "feat: add type-safe DTO records to WarehouseBuilder"
```

---

## Task 3: Add Builder Safety (Reuse Protection and Validation Helper)

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Add built flag to fields**

After `private StorageSpaceManager storageSpaceManager;` (around line 47), add:

```java
  private boolean built = false;
```

**Step 2: Add Validation Helpers section at end of class**

Before the closing brace of the class, add:

```java

  // ========================================
  // Validation Helpers
  // ========================================

  /** Validate that all components in a phase were built successfully. */
  private void validatePhase(String phaseName, Object... components) {
    for (Object component : components) {
      if (component == null) {
        throw new IllegalStateException(
            "Failed to build " + phaseName + " phase: component is null");
      }
    }
  }
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "feat: add built flag and validatePhase helper to WarehouseBuilder"
```

---

## Task 4: Refactor build() Method with Phase Validation

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java:55-119`

**Step 1: Replace build() method**

Replace the entire `build()` method (lines 55-119) with:

```java
  /** Build all warehouse components and return them. */
  public WarehouseComponents build() {
    if (built) {
      throw new IllegalStateException("WarehouseBuilder can only be used once");
    }

    log.info("Building warehouse infrastructure...");

    // Phase 1: Map
    buildMap();
    buildGridPopularityManager();
    validatePhase("map", map, gridPopularityManager);

    // Phase 2: System-specific (RELAY only)
    if (config.systemType() == SystemType.RELAY) {
      buildRelayPositionManager();
      buildZoneManager();
      validatePhase("system-specific", zoneManager, relayPositionManager);
    }

    // Enable zone load balancing for initial inventory placement
    if (config.systemType() == SystemType.RELAY
        && config.algorithmConfig().relayZoneLoadBalancingEnabled()
        && zoneManager != null) {
      gridPopularityManager.enableZoneLoadBalancing(zoneManager);
      log.info("Zone load balancing enabled for inventory initialization");
    }

    // Phase 3: Inventory
    addRacks();
    if (config.systemType() == SystemType.RELAY) {
      addTotes();
    }
    buildItemCatalog();
    buildInventory();
    validatePhase("inventory", itemCatalog, warehouseInventory, storageManager, storageSpaceManager);

    // Phase 4: Robots
    buildStationManager();
    buildRobotFleet();
    validatePhase("robots", stationManager);

    log.info("Warehouse built: {} racks, {} robots", racks.size(), robots.size());

    built = true;

    return WarehouseComponents.builder()
        .map(map)
        .gridPopularityManager(gridPopularityManager)
        .itemCatalog(itemCatalog)
        .inventory(warehouseInventory)
        .racks(List.copyOf(racks))
        .totes(List.copyOf(totes))
        .robots(List.copyOf(robots))
        .stationManager(stationManager)
        .zoneManager(zoneManager)
        .relayPositionManager(relayPositionManager)
        .storagePositionManager(storageManager)
        .storageSpaceManager(storageSpaceManager)
        .build();
  }
```

**Step 2: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "refactor: add phase-based validation and reuse protection to build()"
```

---

## Task 5: Add Remaining Section Markers

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Add Constructor section marker**

Before the constructor (around line 49), add:

```java

  // ========================================
  // Constructor
  // ========================================
```

**Step 2: Add Public API section marker**

Before the `build()` method, add:

```java

  // ========================================
  // Public API
  // ========================================
```

**Step 3: Add Phase 1 section marker**

Before `buildMap()` method, add:

```java

  // ========================================
  // Phase 1: Map Build Methods
  // ========================================
```

**Step 4: Add Phase 2 section marker**

Before `buildZoneManager()` method, add:

```java

  // ========================================
  // Phase 2: System-Specific Build Methods
  // ========================================
```

**Step 5: Add Phase 3 section marker**

Before `addRacks()` method, add:

```java

  // ========================================
  // Phase 3: Inventory Build Methods
  // ========================================
```

**Step 6: Add Phase 4 section marker**

Before `buildRobotFleet()` method, add:

```java

  // ========================================
  // Phase 4: Robot Build Methods
  // ========================================
```

**Step 7: Add Inventory State Helpers section marker**

Before `getInitialInventoryState()` method, add:

```java

  // ========================================
  // Inventory State Helpers
  // ========================================
```

**Step 8: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 9: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "docs: add section markers to WarehouseBuilder for navigation"
```

---

## Task 6: Refactor getInitialInventoryState() to Use DTOs Internally

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Refactor calculateStorageSpaceMetrics to return StorageMetrics**

Replace the `calculateStorageSpaceMetrics` method with:

```java
  /** Calculate detailed metrics for a single storage space. */
  private StorageMetrics calculateStorageSpaceMetrics(
      Object storageSpace, Map<String, Integer> inventory, Set<String> popularSkus) {

    int totalQuantity = inventory.values().stream().mapToInt(Integer::intValue).sum();
    int popularQuantity =
        inventory.entrySet().stream()
            .filter(e -> popularSkus.contains(e.getKey()))
            .mapToInt(Map.Entry::getValue)
            .sum();

    // Get position based on storage space type
    GridCoordinate position;
    double popularityScore = 0.0;

    if (storageSpace instanceof Tote tote) {
      position = tote.getCurrentCoordinate();
    } else if (storageSpace instanceof Rack rack) {
      position = rack.getCurrentCoordinate();
      popularityScore = warehouseInventory.calculatePopularityScore(rack);
    } else {
      position = new GridCoordinate(0, 0);
    }

    return new StorageMetrics(
        new Position(position.row(), position.col()),
        popularityScore,
        inventory.size(),
        totalQuantity,
        popularQuantity,
        totalQuantity - popularQuantity,
        new HashMap<>(inventory));
  }
```

**Step 2: Refactor calculateInventorySummary to return InventorySummary**

Replace the `calculateInventorySummary` method with:

```java
  /** Calculate warehouse-level summary statistics. */
  private InventorySummary calculateInventorySummary(
      Map<String, Integer> skuData, Set<String> popularSkus) {

    int totalUnits = skuData.values().stream().mapToInt(Integer::intValue).sum();
    int totalPopularQuantity =
        skuData.entrySet().stream()
            .filter(e -> popularSkus.contains(e.getKey()))
            .mapToInt(Map.Entry::getValue)
            .sum();
    int totalRegularQuantity = totalUnits - totalPopularQuantity;

    double popularPercentage =
        totalUnits > 0 ? (double) totalPopularQuantity / totalUnits * 100 : 0.0;

    // Count popular SKUs that actually have inventory placed
    long popularSkusWithInventory = skuData.keySet().stream().filter(popularSkus::contains).count();

    // Build skuDetails map for frontend visualization
    Map<String, SkuDetail> skuDetails = new HashMap<>();
    for (Map.Entry<String, Integer> entry : skuData.entrySet()) {
      String sku = entry.getKey();
      skuDetails.put(sku, new SkuDetail(entry.getValue(), popularSkus.contains(sku)));
    }

    return new InventorySummary(
        skuData.size(),
        totalUnits,
        (int) popularSkusWithInventory,
        skuData.size() - (int) popularSkusWithInventory,
        totalPopularQuantity,
        totalRegularQuantity,
        popularPercentage,
        skuDetails);
  }
```

**Step 3: Compile to verify (will have errors - expected)**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: FAIL - methods now return different types

**Step 4: Commit partial progress**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "refactor: update helper methods to return type-safe DTOs"
```

---

## Task 7: Refactor getInitialInventoryState() Main Method

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Replace getInitialInventoryState() method**

Replace the entire `getInitialInventoryState()` method with:

```java
  /**
   * Get the initial inventory state for metrics tracking.
   *
   * <p>Returns data structure matching Python backend's format for frontend compatibility:
   * <ul>
   *   <li>initialInventorySkuData: SKU aggregated quantities</li>
   *   <li>initialInventoryRackData: Rack/tote level inventory details</li>
   *   <li>initialInventorySummary: Summary stats with skuDetails for visualization</li>
   * </ul>
   *
   * @return Map containing SKU-level data, storage space data, and summary statistics
   * @throws IllegalStateException if item catalog or warehouse inventory is not built
   */
  public Map<String, Object> getInitialInventoryState() {
    // Validate prerequisites
    if (itemCatalog == null || warehouseInventory == null) {
      throw new IllegalStateException("Item catalog or warehouse inventory is not built");
    }

    // Get popular SKUs set for lookups
    Set<String> popularSkuSet =
        itemCatalog.getPopularItems().stream().map(Item::sku).collect(Collectors.toSet());

    // Get storage spaces based on system type
    List<?> storageSpaces = config.systemType() == SystemType.RELAY ? totes : racks;
    log.info(
        "Building initial inventory state for {} storage spaces (system type: {})",
        storageSpaces.size(),
        config.systemType());

    // Collect inventory data from storage spaces
    Map<String, Integer> skuData = new HashMap<>();
    Map<Integer, StorageMetrics> storageData = new HashMap<>();

    for (Object storageSpace : storageSpaces) {
      Map<String, Integer> inventory;
      int storageId;

      if (storageSpace instanceof Rack rack) {
        storageId = rack.getId().value();
        inventory = warehouseInventory.getInventory(rack.getId());
      } else if (storageSpace instanceof Tote tote) {
        storageId = tote.getId().value();
        inventory = warehouseInventory.getInventory(tote.getId());
      } else {
        continue;
      }

      // Calculate storage space metrics using type-safe DTO
      StorageMetrics metrics = calculateStorageSpaceMetrics(storageSpace, inventory, popularSkuSet);
      storageData.put(storageId, metrics);

      // Aggregate SKU totals
      for (Map.Entry<String, Integer> entry : inventory.entrySet()) {
        skuData.merge(entry.getKey(), entry.getValue(), Integer::sum);
      }
    }

    // For Relay systems, aggregate tote-level data to rack-level for visualization
    if (config.systemType() == SystemType.RELAY) {
      storageData = aggregateTotesToRacks(storageData, storageSpaces.size());
    }

    // Build summary with skuDetails for frontend
    InventorySummary summary = calculateInventorySummary(skuData, popularSkuSet);

    // Convert DTOs to Map format for backward compatibility with callers
    Map<String, Object> initialState = new HashMap<>();
    initialState.put("initialInventorySkuData", skuData);
    initialState.put("initialInventoryRackData", convertStorageDataToMap(storageData));
    initialState.put("initialInventorySummary", convertSummaryToMap(summary));

    logInventorySummary(skuData, storageData, summary);

    return initialState;
  }

  /** Convert StorageMetrics map to legacy Map format for backward compatibility. */
  private Map<Integer, Map<String, Object>> convertStorageDataToMap(
      Map<Integer, StorageMetrics> storageData) {
    Map<Integer, Map<String, Object>> result = new HashMap<>();
    for (Map.Entry<Integer, StorageMetrics> entry : storageData.entrySet()) {
      StorageMetrics m = entry.getValue();
      Map<String, Object> metrics = new HashMap<>();
      metrics.put("position", Map.of("row", m.position().row(), "col", m.position().col()));
      metrics.put("popularityScore", m.popularityScore());
      metrics.put("uniqueSkus", m.uniqueSkus());
      metrics.put("totalQuantity", m.totalQuantity());
      metrics.put("popularQuantity", m.popularQuantity());
      metrics.put("regularQuantity", m.regularQuantity());
      metrics.put("inventory", m.inventory());
      result.put(entry.getKey(), metrics);
    }
    return result;
  }

  /** Convert InventorySummary to legacy Map format for backward compatibility. */
  private Map<String, Object> convertSummaryToMap(InventorySummary summary) {
    Map<String, Object> result = new HashMap<>();
    result.put("totalSkus", summary.totalSkus());
    result.put("totalUnits", summary.totalUnits());
    result.put("popularSkus", summary.popularSkus());
    result.put("regularSkus", summary.regularSkus());
    result.put("totalPopularQuantity", summary.totalPopularQuantity());
    result.put("totalRegularQuantity", summary.totalRegularQuantity());
    result.put("popularPercentage", summary.popularPercentage());

    // Convert SkuDetail records to maps
    Map<String, Map<String, Object>> skuDetails = new HashMap<>();
    for (Map.Entry<String, SkuDetail> entry : summary.skuDetails().entrySet()) {
      SkuDetail detail = entry.getValue();
      skuDetails.put(entry.getKey(), Map.of(
          "quantity", detail.quantity(),
          "isPopular", detail.isPopular()));
    }
    result.put("skuDetails", skuDetails);

    return result;
  }
```

**Step 2: Compile to verify (may still have errors)**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: May have errors from aggregateTotesToRacks or logInventorySummary

---

## Task 8: Update Remaining Helper Methods

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Refactor aggregateTotesToRacks to use StorageMetrics**

Replace the `aggregateTotesToRacks` method with:

```java
  /** Aggregate tote-level data to rack-level for visualization. */
  private Map<Integer, StorageMetrics> aggregateTotesToRacks(
      Map<Integer, StorageMetrics> toteData, int toteCount) {
    Integer vMax = config.mapConfig().getVerticalToteCapacityInRack();
    if (vMax == null) {
      log.warn("Vertical tote capacity not set, skipping aggregation");
      return toteData;
    }

    // Intermediate aggregation structure
    Map<Integer, RackAggregation> rackAggregations = new HashMap<>();

    for (Map.Entry<Integer, StorageMetrics> entry : toteData.entrySet()) {
      int toteId = entry.getKey();
      StorageMetrics toteMetrics = entry.getValue();
      int rackId = toteId / vMax;

      rackAggregations
          .computeIfAbsent(rackId, k -> new RackAggregation(toteMetrics.position()))
          .accumulate(toteMetrics);
    }

    // Convert aggregations to StorageMetrics
    Map<Integer, StorageMetrics> rackData = new HashMap<>();
    for (Map.Entry<Integer, RackAggregation> entry : rackAggregations.entrySet()) {
      rackData.put(entry.getKey(), entry.getValue().toStorageMetrics());
    }

    log.info("Aggregated {} totes into {} racks for visualization", toteCount, rackData.size());
    return rackData;
  }

  /** Helper class for aggregating tote metrics into rack metrics. */
  private static class RackAggregation {
    private final Position position;
    private final Set<String> uniqueSkus = new HashSet<>();
    private final Map<String, Integer> inventory = new HashMap<>();
    private int totalQuantity = 0;
    private int popularQuantity = 0;
    private int regularQuantity = 0;

    RackAggregation(Position position) {
      this.position = position;
    }

    void accumulate(StorageMetrics tote) {
      totalQuantity += tote.totalQuantity();
      popularQuantity += tote.popularQuantity();
      regularQuantity += tote.regularQuantity();
      uniqueSkus.addAll(tote.inventory().keySet());
      for (Map.Entry<String, Integer> entry : tote.inventory().entrySet()) {
        inventory.merge(entry.getKey(), entry.getValue(), Integer::sum);
      }
    }

    StorageMetrics toStorageMetrics() {
      double popularityScore = totalQuantity > 0 ? (double) popularQuantity / totalQuantity : 0.0;
      return new StorageMetrics(
          position,
          popularityScore,
          uniqueSkus.size(),
          totalQuantity,
          popularQuantity,
          regularQuantity,
          inventory);
    }
  }
```

**Step 2: Refactor logInventorySummary to use DTOs**

Replace the `logInventorySummary` method with:

```java
  /** Log inventory summary statistics. */
  private void logInventorySummary(
      Map<String, Integer> skuData,
      Map<Integer, StorageMetrics> storageData,
      InventorySummary summary) {
    String storageType = config.systemType() == SystemType.RELAY ? "totes" : "racks";

    log.info(
        "Initial inventory: {} SKUs, {} total units across {} {}",
        skuData.size(),
        summary.totalUnits(),
        storageData.size(),
        storageType);
    log.info(
        "Popular items: {} units ({} %)",
        summary.totalPopularQuantity(),
        String.format("%.1f", summary.popularPercentage()));
    log.info(
        "Regular items: {} units ({} %)",
        summary.totalRegularQuantity(),
        String.format("%.1f", 100 - summary.popularPercentage()));
  }
```

**Step 3: Remove old helper methods that are no longer needed**

Delete these methods as they're replaced by the type-safe versions:
- `createEmptyRackAggregation` (replaced by RackAggregation constructor)
- `accumulateToteIntoRack` (replaced by RackAggregation.accumulate)
- `finalizeRackAggregations` (replaced by RackAggregation.toStorageMetrics)

**Step 4: Compile to verify all changes**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "refactor: complete type-safe DTO refactoring for getInitialInventoryState"
```

---

## Task 9: Format and Run Tests

**Files:**
- All backend files

**Step 1: Format code**

Run: `cd apps/backend && mvn spotless:apply`
Expected: No errors

**Step 2: Run full test suite**

Run: `cd apps/backend && mvn test`
Expected: All tests pass

**Step 3: Final commit if formatting changed anything**

```bash
git add -A
git commit -m "style: format WarehouseBuilder after refactoring" --allow-empty
```

---

## Task 10: Manual Verification

**Step 1: Start backend server**

```bash
rm -f /tmp/server.log
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/health
```

Expected: `{"status":"UP"}`

**Step 2: Run a simulation to verify inventory state works**

```bash
cat apps/backend/src/test/resources/integration-test-config.json | \
  curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" -d @- > /tmp/sim_result.json 2>&1
```

Expected: Simulation completes without errors

**Step 3: Check logs for inventory state output**

```bash
grep "Initial inventory:" /tmp/server.log
```

Expected: Log line showing SKU count and total units

**Step 4: Cleanup**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
```

---

## Summary

**Total Tasks:** 10
**Estimated Commits:** 7-8

**Key Changes:**
1. Expanded class Javadoc with responsibilities and build order
2. Added 5 type-safe DTO records as nested classes
3. Added reuse protection with `built` flag
4. Added phase-based validation with clear error messages
5. Organized 600+ lines with 9 section markers
6. Eliminated all `@SuppressWarnings("unchecked")` annotations
7. Maintained backward compatibility with Map return type for callers

---

## Implementation Notes

### Backward Compatibility

The return type remains `Map<String, Object>` to maintain backward compatibility with callers:

- `Simulator.java:52` - passes to `metricsManager.setInitialInventoryState()`
- `MetricsCollectors.java:694` - stores and accesses keys like `"initialInventorySummary"`

The conversion methods (`convertStorageDataToMap`, `convertSummaryToMap`) produce the exact same key names as before:
- `"initialInventorySkuData"` - Map<String, Integer>
- `"initialInventoryRackData"` - Map<Integer, Map<String, Object>>
- `"initialInventorySummary"` - Map with keys: `totalUnits`, `totalPopularQuantity`, `popularSkus`, etc.

**No caller changes required.**

### ZoneManager Note

ZoneManager is only built for RELAY systems. The Phase 2 validation is correctly wrapped in a RELAY check to avoid null validation errors for RTP systems.

### Testing

- Run tests for both RTP and RELAY system types
- Verify InventoryCollector correctly reads the inventory state data
- Check that frontend receives proper inventory visualization data
