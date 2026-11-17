# PUT Strategy Pattern Refactoring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor PutProcessor to use strategy pattern (enum, interface, abstract class, factory) matching the OrderSelectionStrategy architecture for type-safe, extensible PUT algorithm selection.

**Architecture:** Extract greedy and LP algorithms into separate strategy classes extending AbstractPutStrategy. Use PutStrategyFactory for adaptive strategy selection (auto-fallback to greedy for large problems). PutProcessor delegates to strategy.initializeInventory().

**Tech Stack:** Java 21, Lombok, OR-Tools (CP-SAT), JUnit 5, Spring Boot Test

---

## Prerequisites

**Required Knowledge:**
- Strategy pattern basics
- Java records, enums, sealed types (see `docs/JAVA_STYLE_GUIDE.md`)
- Existing code: `OrderSelectionAlgorithm.java`, `OrderSelectionStrategy.java`, `AbstractOrderSelectionStrategy.java` (reference architecture)

**Files to Review:**
- `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/PutProcessor.java` (current implementation)
- `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderSelectionAlgorithm.java` (pattern reference)

---

## Task 1: Create PutAlgorithm Enum

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/PutAlgorithm.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/PutAlgorithmTest.java`

**Step 1: Write the failing test**

Create test file:

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

class PutAlgorithmTest {

  @Test
  void shouldParseGreedyFromString() {
    PutAlgorithm algorithm = PutAlgorithm.fromValue("greedy");
    assertEquals(PutAlgorithm.GREEDY, algorithm);
    assertEquals("greedy", algorithm.getValue());
  }

  @Test
  void shouldParseMinWeightedCycleDistanceFromString() {
    PutAlgorithm algorithm = PutAlgorithm.fromValue("min_weighted_cycle_distance");
    assertEquals(PutAlgorithm.MIN_WEIGHTED_CYCLE_DISTANCE, algorithm);
    assertEquals("min_weighted_cycle_distance", algorithm.getValue());
  }

  @Test
  void shouldThrowExceptionForInvalidValue() {
    assertThrows(
        IllegalArgumentException.class,
        () -> PutAlgorithm.fromValue("invalid_algorithm"));
  }

  @Test
  void shouldSerializeToJsonValue() {
    assertEquals("greedy", PutAlgorithm.GREEDY.getValue());
    assertEquals(
        "min_weighted_cycle_distance", PutAlgorithm.MIN_WEIGHTED_CYCLE_DISTANCE.getValue());
  }
}
```

**Step 2: Run test to verify it fails**

```bash
cd apps/backend
mvn test -Dtest=PutAlgorithmTest
```

Expected: FAIL - "cannot find symbol: class PutAlgorithm"

**Step 3: Write minimal implementation**

Create enum file:

```java
package com.greyorange.dessimulator.domain.put;

import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonValue;
import lombok.Getter;
import lombok.RequiredArgsConstructor;

/**
 * PUT algorithms for optimizing inventory placement in storage spaces.
 *
 * <p>Algorithms trade off between speed and placement quality. Greedy is fast and suitable for
 * large problems. LP-based optimization finds better placements but has higher computational cost.
 */
@Getter
@RequiredArgsConstructor
public enum PutAlgorithm {
  /**
   * Greedy algorithm that places SKUs in closest storage spaces based on popularity.
   *
   * <p>Fast O(n*m) placement suitable for 5K+ SKUs and storage spaces. Respects ABC tier
   * constraints and applies popularity-based distance scoring.
   */
  GREEDY("greedy"),

  /**
   * Linear programming algorithm that minimizes weighted cycle distance.
   *
   * <p>Uses OR-Tools CP-SAT solver to find optimal placement maximizing inventory fill while
   * minimizing popular item distances. Falls back to greedy for problems exceeding 5M
   * combinations.
   */
  MIN_WEIGHTED_CYCLE_DISTANCE("min_weighted_cycle_distance");

  private final String value;

  @JsonValue
  public String getValue() {
    return value;
  }

  @JsonCreator
  public static PutAlgorithm fromValue(String value) {
    for (PutAlgorithm algorithm : values()) {
      if (algorithm.value.equals(value)) {
        return algorithm;
      }
    }
    throw new IllegalArgumentException("Unknown PutAlgorithm: " + value);
  }
}
```

**Step 4: Run test to verify it passes**

```bash
mvn test -Dtest=PutAlgorithmTest
```

Expected: PASS - All 4 tests green

**Step 5: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/PutAlgorithm.java
git add src/test/java/com/greyorange/dessimulator/domain/put/PutAlgorithmTest.java
git commit -m "feat: add PutAlgorithm enum for type-safe algorithm selection"
```

---

## Task 2: Create PutStrategy Interface

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/PutStrategy.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/PutStrategyTest.java`

**Step 1: Write the failing test**

Create test with mock implementation:

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

class PutStrategyTest {

  @Test
  void shouldImplementStrategyInterface() {
    PutStrategy strategy = new TestPutStrategy();

    assertEquals("test", strategy.getAlgorithmName());
    assertDoesNotThrow(strategy::initializeInventory);
  }

  // Minimal test implementation
  private static class TestPutStrategy implements PutStrategy {
    private boolean initialized = false;

    @Override
    public void initializeInventory() {
      initialized = true;
    }

    @Override
    public String getAlgorithmName() {
      return "test";
    }

    boolean isInitialized() {
      return initialized;
    }
  }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=PutStrategyTest
```

Expected: FAIL - "cannot find symbol: class PutStrategy"

**Step 3: Write minimal implementation**

Create interface:

```java
package com.greyorange.dessimulator.domain.put;

/**
 * Strategy interface for PUT algorithms that initialize warehouse inventory.
 *
 * <p>Implementations provide different heuristics for placing SKUs into storage spaces (racks or
 * totes) to optimize metrics like picks-per-face, weighted cycle distance, or placement speed.
 *
 * <p>The strategy is invoked once at simulation startup via {@link
 * #initializeInventory()}.
 */
public interface PutStrategy {

  /**
   * Initialize warehouse inventory by placing SKUs into storage spaces.
   *
   * <p>Implementations should:
   *
   * <ul>
   *   <li>Read SKUs and quantities from {@code ItemCatalog}
   *   <li>Apply placement algorithm (greedy, LP optimization, etc.)
   *   <li>Update {@code WarehouseInventory} with storage space assignments
   *   <li>Respect constraints (capacity, SKU diversity, ABC tiers)
   * </ul>
   *
   * <p>Called once during simulation setup.
   */
  void initializeInventory();

  /**
   * Get algorithm name for logging and metrics.
   *
   * @return Human-readable algorithm name (e.g., "greedy", "min_weighted_cycle_distance")
   */
  String getAlgorithmName();
}
```

**Step 4: Run test to verify it passes**

```bash
mvn test -Dtest=PutStrategyTest
```

Expected: PASS

**Step 5: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/PutStrategy.java
git add src/test/java/com/greyorange/dessimulator/domain/put/PutStrategyTest.java
git commit -m "feat: add PutStrategy interface for inventory initialization"
```

---

## Task 3: Create AbstractPutStrategy Base Class

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/AbstractPutStrategy.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/AbstractPutStrategyTest.java`

**Step 1: Write the failing test**

Test shared helper methods:

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class AbstractPutStrategyTest {

  private TestAbstractPutStrategy strategy;
  private WarehouseInventory inventory;
  private ItemCatalog itemCatalog;
  private List<Rack> racks;

  @BeforeEach
  void setUp() {
    // Create test catalog with 3 SKUs (1 popular, 2 non-popular)
    Item popularItem = new Item("SKU_A", "Product A", true, 1.0);
    Item mediumItem = new Item("SKU_B", "Product B", false, 1.0);
    Item slowItem = new Item("SKU_C", "Product C", false, 1.0);

    Map<String, Item> items = Map.of(
        "SKU_A", popularItem,
        "SKU_B", mediumItem,
        "SKU_C", slowItem);

    Map<String, Integer> pastOrderedQuantities = Map.of(
        "SKU_A", 1000,  // Popular
        "SKU_B", 300,   // Medium (top 37.5% of non-popular)
        "SKU_C", 100);  // Slow

    Map<String, Integer> maxInventoryQuantities = Map.of(
        "SKU_A", 100,
        "SKU_B", 100,
        "SKU_C", 100);

    itemCatalog = new ItemCatalog(items, pastOrderedQuantities, maxInventoryQuantities);

    // Create 3 racks with 80 capacity
    racks = List.of(
        new Rack(new StorageSpaceId("R1"), 80, null),
        new Rack(new StorageSpaceId("R2"), 80, null),
        new Rack(new StorageSpaceId("R3"), 80, null));

    inventory = new WarehouseInventory();
    for (Rack rack : racks) {
      inventory.getStorageSpaceInventory().put(rack.getId(), new HashMap<>());
    }

    GridPopularityManager gridManager = new GridPopularityManager(10, 10, List.of());

    strategy = new TestAbstractPutStrategy(
        SystemType.RTP,
        itemCatalog,
        racks,
        gridManager,
        inventory);
  }

  @Test
  void shouldBuildAbcTierClassification() {
    Map<String, Integer> skuMaxPerRack = strategy.exposeBuildAbcTierClassification();

    // Category A (popular): 80% of 80 = 64
    assertEquals(64, skuMaxPerRack.get("SKU_A"));

    // Category B (top 37.5% non-popular): 50% of 80 = 40
    assertEquals(40, skuMaxPerRack.get("SKU_B"));

    // Category C (bottom 62.5% non-popular): 30% of 80 = 24
    assertEquals(24, skuMaxPerRack.get("SKU_C"));
  }

  @Test
  void shouldIndexStorageSpaceMetadata() {
    AbstractPutStrategy.MetadataIndex index = strategy.exposeIndexStorageSpaceMetadata();

    assertEquals(3, index.storageSpaceMaxCapacity().size());
    assertEquals(80, index.storageSpaceMaxCapacity().get(new StorageSpaceId("R1")));
    assertEquals(0, index.storageSpaceCurrentCapacity().get(new StorageSpaceId("R1")));
    assertEquals(0, index.storageSpaceSkuCount().get(new StorageSpaceId("R1")));
  }

  @Test
  void shouldComputeTargetInventory() {
    // With TARGET_INVENTORY_FILL_RATE = 1.0
    assertEquals(100, strategy.exposeComputeTargetInventory(100));
    assertEquals(50, strategy.exposeComputeTargetInventory(50));

    // Edge case: maxInv > 0 but target would be 0, ensure at least 1
    assertEquals(1, strategy.exposeComputeTargetInventory(1));
  }

  @Test
  void shouldGetAlgorithmName() {
    assertEquals("test_algorithm", strategy.getAlgorithmName());
  }

  // Test implementation that exposes protected methods
  private static class TestAbstractPutStrategy extends AbstractPutStrategy {

    TestAbstractPutStrategy(
        SystemType systemType,
        ItemCatalog itemCatalog,
        List<? extends WarehouseInventory.StorageSpace> storageSpaces,
        GridPopularityManager gridPopularityManager,
        WarehouseInventory inventory) {
      super(systemType, itemCatalog, storageSpaces, gridPopularityManager, inventory);
    }

    @Override
    public void initializeInventory() {
      // No-op for testing
    }

    @Override
    public String getAlgorithmName() {
      return "test_algorithm";
    }

    Map<String, Integer> exposeBuildAbcTierClassification() {
      return buildAbcTierClassification();
    }

    MetadataIndex exposeIndexStorageSpaceMetadata() {
      return indexStorageSpaceMetadata();
    }

    int exposeComputeTargetInventory(int maxInv) {
      return computeTargetInventory(maxInv);
    }
  }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=AbstractPutStrategyTest
```

Expected: FAIL - "cannot find symbol: class AbstractPutStrategy"

**Step 3: Write minimal implementation**

Create abstract base class (extract shared logic from PutProcessor.java lines 28-296):

```java
package com.greyorange.dessimulator.domain.put;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import java.io.FileWriter;
import java.io.IOException;
import java.util.*;
import java.util.stream.Collectors;
import lombok.RequiredArgsConstructor;
import lombok.extern.slf4j.Slf4j;

/**
 * Base class for PUT strategies with shared inventory placement logic.
 *
 * <p>Provides default implementations for common operations used across all PUT algorithms:
 *
 * <ul>
 *   <li>ABC tier classification based on popularity ({@link #buildAbcTierClassification})
 *   <li>Storage space metadata indexing ({@link #indexStorageSpaceMetadata})
 *   <li>Target inventory calculation ({@link #computeTargetInventory})
 *   <li>CSV export for debugging ({@link #exportInventoryAssignmentToCSV})
 * </ul>
 *
 * <p>Subclasses implement {@link PutStrategy#initializeInventory()} with custom placement
 * algorithms while reusing these common helper methods.
 */
@Slf4j
@RequiredArgsConstructor
public abstract class AbstractPutStrategy implements PutStrategy {

  // Constants for inventory placement
  protected static final double TARGET_INVENTORY_FILL_RATE = 1.0;
  protected static final int MAX_SKUS_PER_RACK = 80;
  protected static final int MAX_SKUS_PER_TOTE = 2;

  // ABC tier utilization limits (fraction of storage space capacity)
  protected static final double CATEGORY_A_RACK_MAX_UTILIZATION = 0.8;
  protected static final double CATEGORY_A_TOTE_MAX_UTILIZATION = 0.67;
  protected static final double CATEGORY_B_RACK_MAX_UTILIZATION = 0.5;
  protected static final double CATEGORY_B_TOTE_MAX_UTILIZATION = 0.5;
  protected static final double CATEGORY_C_RACK_MAX_UTILIZATION = 0.3;
  protected static final double CATEGORY_C_TOTE_MAX_UTILIZATION = 0.17;
  protected static final double CATEGORY_B_PERCENTILE = 0.375;

  // Dependencies
  protected final SystemType systemType;
  protected final ItemCatalog itemCatalog;
  protected final List<? extends WarehouseInventory.StorageSpace> storageSpaces;
  protected final GridPopularityManager gridPopularityManager;
  protected final WarehouseInventory inventory;

  /**
   * Build ABC tier classification mapping SKU to max quantity per storage space.
   *
   * <p>Classifies SKUs into three categories based on popularity:
   *
   * <ul>
   *   <li>Category A: Popular items (is_popular = true) - Higher density allowed
   *   <li>Category B: Top 37.5% of non-popular SKUs - Medium density
   *   <li>Category C: Bottom 62.5% of non-popular SKUs - Lower density
   * </ul>
   *
   * @return Map from SKU to max units allowed per storage space
   */
  protected Map<String, Integer> buildAbcTierClassification() {
    Map<String, Integer> pastOrderedQuantities = itemCatalog.getPastOrderedQuantities();

    // Sort SKUs by popularity (descending)
    List<String> sortedSkus =
        pastOrderedQuantities.entrySet().stream()
            .sorted((e1, e2) -> Integer.compare(e2.getValue(), e1.getValue()))
            .map(Map.Entry::getKey)
            .toList();

    Map<String, Integer> skuMaxPerStorageSpace = new HashMap<>();

    if (storageSpaces.isEmpty()) {
      log.warn("No storage spaces available for ABC classification");
      return skuMaxPerStorageSpace;
    }

    // Get capacity from first storage space (all have same capacity)
    int storageCapacity =
        storageSpaces.get(0) instanceof Rack
            ? ((Rack) storageSpaces.get(0)).getMaxCapacity()
            : ((Tote) storageSpaces.get(0)).getMaxCapacity();

    // Determine utilization factors based on system type
    boolean isRackSystem = storageSpaces.get(0) instanceof Rack;
    double categoryAUtilization =
        isRackSystem ? CATEGORY_A_RACK_MAX_UTILIZATION : CATEGORY_A_TOTE_MAX_UTILIZATION;
    double categoryBUtilization =
        isRackSystem ? CATEGORY_B_RACK_MAX_UTILIZATION : CATEGORY_B_TOTE_MAX_UTILIZATION;
    double categoryCUtilization =
        isRackSystem ? CATEGORY_C_RACK_MAX_UTILIZATION : CATEGORY_C_TOTE_MAX_UTILIZATION;

    // Category A: popular items from ItemCatalog
    Set<String> popularSkus =
        itemCatalog.getPopularItems().stream().map(Item::sku).collect(Collectors.toSet());

    List<String> nonPopular = new ArrayList<>();
    int categoryACount = 0;

    for (String sku : sortedSkus) {
      if (popularSkus.contains(sku)) {
        int maxPerStorageSpace = (int) (storageCapacity * categoryAUtilization);
        skuMaxPerStorageSpace.put(sku, maxPerStorageSpace);
        categoryACount++;
      } else {
        nonPopular.add(sku);
      }
    }

    // Categories B & C: split non-popular by pastOrderedQuantity
    int categoryBCount = (int) (nonPopular.size() * CATEGORY_B_PERCENTILE);
    for (int i = 0; i < nonPopular.size(); i++) {
      String sku = nonPopular.get(i);
      boolean isCategoryB = i < categoryBCount;
      double utilization = isCategoryB ? categoryBUtilization : categoryCUtilization;
      int maxPerStorageSpace = (int) (storageCapacity * utilization);
      skuMaxPerStorageSpace.put(sku, maxPerStorageSpace);
    }

    int categoryCCount = nonPopular.size() - categoryBCount;
    log.info(
        "ABC Classification: A={} SKUs, B={} SKUs, C={} SKUs",
        categoryACount,
        categoryBCount,
        categoryCCount);
    log.info(
        "Per-storage-space limits: Category A ≤ {} units, Category B ≤ {} units, Category C ≤ {} units",
        (int) (storageCapacity * categoryAUtilization),
        (int) (storageCapacity * categoryBUtilization),
        (int) (storageCapacity * categoryCUtilization));

    return skuMaxPerStorageSpace;
  }

  /**
   * Pre-index storage space metadata for O(1) lookups during placement.
   *
   * <p>Builds maps for max capacity, current capacity, and SKU count per storage space to avoid
   * O(m) searches during greedy placement.
   *
   * @return Indexed metadata for all storage spaces
   */
  protected MetadataIndex indexStorageSpaceMetadata() {
    Map<StorageSpaceId, Integer> storageSpaceMaxCapacity = new HashMap<>();
    Map<StorageSpaceId, Integer> storageSpaceCurrentCapacity = new HashMap<>();
    Map<StorageSpaceId, Integer> storageSpaceSkuCount = new HashMap<>();

    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      StorageSpaceId id = storageSpace.getId();
      int maxCapacity =
          storageSpace instanceof Rack
              ? ((Rack) storageSpace).getMaxCapacity()
              : ((Tote) storageSpace).getMaxCapacity();

      storageSpaceMaxCapacity.put(id, maxCapacity);
      storageSpaceCurrentCapacity.put(id, 0);
      storageSpaceSkuCount.put(id, 0);
      inventory.getStorageSpaceInventory().put(id, new HashMap<>());
    }

    log.info("Pre-indexed {} storage spaces", storageSpaces.size());
    return new MetadataIndex(
        storageSpaceMaxCapacity, storageSpaceCurrentCapacity, storageSpaceSkuCount);
  }

  /**
   * Compute target inventory quantity applying fill rate.
   *
   * <p>Applies {@link #TARGET_INVENTORY_FILL_RATE} to max inventory. Ensures at least 1 unit if
   * maxInv > 0.
   *
   * @param maxInv Maximum inventory quantity for SKU
   * @return Target quantity to place (always ≥ 1 if maxInv > 0)
   */
  protected int computeTargetInventory(int maxInv) {
    int targetInv = (int) (maxInv * TARGET_INVENTORY_FILL_RATE);
    if (targetInv == 0 && maxInv > 0) {
      targetInv = 1; // Ensure at least 1 unit
    }
    return targetInv;
  }

  /**
   * Export inventory assignment to CSV for debugging PUT algorithm.
   *
   * <p>Format: StorageSpaceId,TotalQuantity,SKUList
   *
   * <p>SKUList format: "SKU1:qty1,SKU2:qty2,..."
   */
  protected void exportInventoryAssignmentToCSV() {
    String filename = "/tmp/put_assignment_debug.csv";
    try (FileWriter writer = new FileWriter(filename)) {
      writer.write("StorageSpaceId,TotalQuantity,SKUList\n");

      List<StorageSpaceId> sortedSpaceIds =
          new ArrayList<>(inventory.getStorageSpaceInventory().keySet());
      sortedSpaceIds.sort(Comparator.comparing(StorageSpaceId::value));

      for (StorageSpaceId spaceId : sortedSpaceIds) {
        Map<String, Integer> skuInventory = inventory.getStorageSpaceInventory().get(spaceId);

        if (skuInventory == null || skuInventory.isEmpty()) {
          continue;
        }

        int totalQuantity = skuInventory.values().stream().mapToInt(Integer::intValue).sum();

        StringBuilder skuList = new StringBuilder();
        List<Map.Entry<String, Integer>> sortedSkus = new ArrayList<>(skuInventory.entrySet());
        sortedSkus.sort(Map.Entry.comparingByKey());

        for (int i = 0; i < sortedSkus.size(); i++) {
          Map.Entry<String, Integer> entry = sortedSkus.get(i);
          skuList.append(entry.getKey()).append(":").append(entry.getValue());
          if (i < sortedSkus.size() - 1) {
            skuList.append(",");
          }
        }

        writer.write(
            String.format("%s,%d,\"%s\"\n", spaceId.value(), totalQuantity, skuList.toString()));
      }

      log.info("Exported inventory assignment to CSV: {}", filename);
    } catch (IOException e) {
      log.error("Failed to export inventory assignment to CSV", e);
    }
  }

  /**
   * Pre-indexed metadata for storage spaces.
   *
   * <p>Provides O(1) lookups for capacity tracking during placement.
   *
   * @param storageSpaceMaxCapacity Maximum capacity per storage space
   * @param storageSpaceCurrentCapacity Current used capacity per storage space
   * @param storageSpaceSkuCount Number of distinct SKUs per storage space
   */
  protected record MetadataIndex(
      Map<StorageSpaceId, Integer> storageSpaceMaxCapacity,
      Map<StorageSpaceId, Integer> storageSpaceCurrentCapacity,
      Map<StorageSpaceId, Integer> storageSpaceSkuCount) {}
}
```

**Step 4: Run test to verify it passes**

```bash
mvn test -Dtest=AbstractPutStrategyTest
```

Expected: PASS - All 5 tests green

**Step 5: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/AbstractPutStrategy.java
git add src/test/java/com/greyorange/dessimulator/domain/put/AbstractPutStrategyTest.java
git commit -m "feat: add AbstractPutStrategy with shared placement logic"
```

---

## Task 4: Create strategies/ Directory and GreedyPutStrategy

**Files:**
- Create directory: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/`
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategy.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategyTest.java`

**Step 1: Create strategies directory**

```bash
cd apps/backend
mkdir -p src/main/java/com/greyorange/dessimulator/domain/put/strategies
mkdir -p src/test/java/com/greyorange/dessimulator/domain/put/strategies
```

**Step 2: Write the failing test**

Create test file:

```java
package com.greyorange.dessimulator.domain.put.strategies;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class GreedyPutStrategyTest {

  private GreedyPutStrategy strategy;
  private WarehouseInventory inventory;
  private ItemCatalog itemCatalog;
  private List<Rack> racks;

  @BeforeEach
  void setUp() {
    // Create simple catalog with 2 SKUs
    Item itemA = new Item("SKU_A", "Product A", true, 1.0);
    Item itemB = new Item("SKU_B", "Product B", false, 1.0);

    Map<String, Item> items = Map.of("SKU_A", itemA, "SKU_B", itemB);
    Map<String, Integer> pastOrderedQuantities = Map.of("SKU_A", 100, "SKU_B", 50);
    Map<String, Integer> maxInventoryQuantities = Map.of("SKU_A", 80, "SKU_B", 40);

    itemCatalog = new ItemCatalog(items, pastOrderedQuantities, maxInventoryQuantities);

    // Create 2 racks
    racks = List.of(
        new Rack(new StorageSpaceId("R1"), 80, new StorageCoordinate(0, 0)),
        new Rack(new StorageSpaceId("R2"), 80, new StorageCoordinate(1, 0)));

    inventory = new WarehouseInventory();

    GridPopularityManager gridManager = new GridPopularityManager(10, 10, List.of());

    strategy = new GreedyPutStrategy(
        SystemType.RTP,
        itemCatalog,
        racks,
        gridManager,
        inventory);
  }

  @Test
  void shouldInitializeInventoryWithGreedyPlacement() {
    strategy.initializeInventory();

    // Verify SKU_A (popular) was placed
    Map<StorageSpaceId, Map<String, Integer>> spaceInventory =
        inventory.getStorageSpaceInventory();

    long totalSkuA = spaceInventory.values().stream()
        .mapToLong(inv -> inv.getOrDefault("SKU_A", 0))
        .sum();

    // Should place 80 units (maxInventoryQuantities * TARGET_INVENTORY_FILL_RATE)
    assertEquals(80, totalSkuA);
  }

  @Test
  void shouldReturnGreedyAlgorithmName() {
    assertEquals("greedy", strategy.getAlgorithmName());
  }

  @Test
  void shouldRespectStorageSpaceCapacity() {
    strategy.initializeInventory();

    for (Rack rack : racks) {
      Map<String, Integer> rackInventory =
          inventory.getStorageSpaceInventory().get(rack.getId());

      int totalUnits = rackInventory.values().stream().mapToInt(Integer::intValue).sum();
      assertTrue(totalUnits <= rack.getMaxCapacity(),
          "Rack " + rack.getId() + " exceeded capacity: " + totalUnits);
    }
  }
}
```

**Step 3: Run test to verify it fails**

```bash
mvn test -Dtest=GreedyPutStrategyTest
```

Expected: FAIL - "cannot find symbol: class GreedyPutStrategy"

**Step 4: Write minimal implementation**

Extract greedy algorithm from PutProcessor.java lines 86-296:

```java
package com.greyorange.dessimulator.domain.put.strategies;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import com.greyorange.dessimulator.domain.put.AbstractPutStrategy;
import java.util.*;
import lombok.extern.slf4j.Slf4j;

/**
 * Greedy PUT strategy that places SKUs in closest storage spaces based on popularity.
 *
 * <p>OPTIMIZED for 5K+ SKUs and 5K+ storage spaces:
 *
 * <ul>
 *   <li>Pre-indexed storage space metadata (no O(m) searches)
 *   <li>In-place capacity tracking (no repeated streaming/summing)
 *   <li>ABC tier constraints for SKU density limits
 *   <li>Early termination when storage spaces are full
 * </ul>
 *
 * <p>Time complexity: O(n * m) where n = SKUs, m = storage spaces
 *
 * <p>Use when problem size exceeds LP feasibility or when speed is critical.
 */
@Slf4j
public class GreedyPutStrategy extends AbstractPutStrategy {

  public GreedyPutStrategy(
      SystemType systemType,
      ItemCatalog itemCatalog,
      List<? extends WarehouseInventory.StorageSpace> storageSpaces,
      GridPopularityManager gridPopularityManager,
      WarehouseInventory inventory) {
    super(systemType, itemCatalog, storageSpaces, gridPopularityManager, inventory);
  }

  @Override
  public void initializeInventory() {
    log.info("Using greedy algorithm for inventory initialization...");
    long startTime = System.currentTimeMillis();

    // Pre-index storage space metadata to avoid O(m) searches
    MetadataIndex metadata = indexStorageSpaceMetadata();
    Map<StorageSpaceId, Integer> storageSpaceMaxCapacity = metadata.storageSpaceMaxCapacity();
    Map<StorageSpaceId, Integer> storageSpaceCurrentCapacity = metadata.storageSpaceCurrentCapacity();
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

    // Pre-compute storage space distances (sorted by average distance)
    List<StorageSpaceDistance> sortedStorageSpaces = new ArrayList<>();
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      double dist = 0.0;
      if (storageSpace instanceof Rack rack && rack.getStorageCoordinate() != null) {
        dist = gridPopularityManager.getAverageDistance(rack.getStorageCoordinate());
      } else if (storageSpace instanceof Tote tote && tote.getCurrentCoordinate() != null) {
        dist = gridPopularityManager.getAverageDistance(tote.getCurrentCoordinate());
      }
      sortedStorageSpaces.add(new StorageSpaceDistance(dist, storageSpace.getId()));
    }
    sortedStorageSpaces.sort(Comparator.comparingDouble(ssd -> ssd.distance));
    log.info("Sorted {} storage spaces by distance", sortedStorageSpaces.size());

    int placedCount = 0;
    int totalUnits = 0;

    // System-type aware SKU limit
    int maxSkusPerStorageSpace =
        (systemType == SystemType.RELAY) ? MAX_SKUS_PER_TOTE : MAX_SKUS_PER_RACK;

    // Iterate through SKUs and use pre-sorted storage spaces
    for (String sku : sortedSkus) {
      int maxInv = maxInventoryQuantities.getOrDefault(sku, 0);
      if (maxInv == 0) {
        continue;
      }

      int targetInv = computeTargetInventory(maxInv);
      int remaining = targetInv;

      for (int i = 0; i < sortedStorageSpaces.size() && remaining > 0; i++) {
        StorageSpaceId storageSpaceId = sortedStorageSpaces.get(i).storageSpaceId;

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

  @Override
  public String getAlgorithmName() {
    return "greedy";
  }

  // Helper record
  private record StorageSpaceDistance(double distance, StorageSpaceId storageSpaceId) {}
}
```

**Step 5: Run test to verify it passes**

```bash
mvn test -Dtest=GreedyPutStrategyTest
```

Expected: PASS - All 3 tests green

**Step 6: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategy.java
git add src/test/java/com/greyorange/dessimulator/domain/put/strategies/GreedyPutStrategyTest.java
git commit -m "feat: add GreedyPutStrategy for fast inventory placement"
```

---

## Task 5: Create MinWeightedCycleDistancePutStrategy

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/strategies/MinWeightedCycleDistancePutStrategy.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/strategies/MinWeightedCycleDistancePutStrategyTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.put.strategies;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class MinWeightedCycleDistancePutStrategyTest {

  private MinWeightedCycleDistancePutStrategy strategy;
  private WarehouseInventory inventory;
  private ItemCatalog itemCatalog;
  private List<Rack> racks;

  @BeforeEach
  void setUp() {
    // Create simple catalog for LP testing
    Item itemA = new Item("SKU_A", "Product A", true, 1.0);
    Item itemB = new Item("SKU_B", "Product B", false, 1.0);

    Map<String, Item> items = Map.of("SKU_A", itemA, "SKU_B", itemB);
    Map<String, Integer> pastOrderedQuantities = Map.of("SKU_A", 100, "SKU_B", 50);
    Map<String, Integer> maxInventoryQuantities = Map.of("SKU_A", 50, "SKU_B", 30);

    itemCatalog = new ItemCatalog(items, pastOrderedQuantities, maxInventoryQuantities);

    // Create 3 racks
    racks = List.of(
        new Rack(new StorageSpaceId("R1"), 80, new StorageCoordinate(0, 0)),
        new Rack(new StorageSpaceId("R2"), 80, new StorageCoordinate(1, 0)),
        new Rack(new StorageSpaceId("R3"), 80, new StorageCoordinate(2, 0)));

    inventory = new WarehouseInventory();

    GridPopularityManager gridManager = new GridPopularityManager(10, 10, List.of());

    strategy = new MinWeightedCycleDistancePutStrategy(
        SystemType.RTP,
        itemCatalog,
        racks,
        gridManager,
        inventory);
  }

  @Test
  void shouldInitializeInventoryWithLpOptimization() {
    strategy.initializeInventory();

    // Verify inventory was placed
    Map<StorageSpaceId, Map<String, Integer>> spaceInventory =
        inventory.getStorageSpaceInventory();

    long totalUnits = spaceInventory.values().stream()
        .flatMap(inv -> inv.values().stream())
        .mapToLong(Integer::intValue)
        .sum();

    assertTrue(totalUnits > 0, "LP should place some inventory");
  }

  @Test
  void shouldReturnMinWeightedCycleDistanceAlgorithmName() {
    assertEquals("min_weighted_cycle_distance", strategy.getAlgorithmName());
  }

  @Test
  void shouldRespectStorageSpaceCapacity() {
    strategy.initializeInventory();

    for (Rack rack : racks) {
      Map<String, Integer> rackInventory =
          inventory.getStorageSpaceInventory().get(rack.getId());

      int totalUnits = rackInventory.values().stream().mapToInt(Integer::intValue).sum();
      assertTrue(totalUnits <= rack.getMaxCapacity(),
          "Rack " + rack.getId() + " exceeded capacity: " + totalUnits);
    }
  }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=MinWeightedCycleDistancePutStrategyTest
```

Expected: FAIL - "cannot find symbol: class MinWeightedCycleDistancePutStrategy"

**Step 3: Write minimal implementation**

Extract LP algorithm from PutProcessor.java lines 298-622:

```java
package com.greyorange.dessimulator.domain.put.strategies;

import com.google.ortools.Loader;
import com.google.ortools.sat.*;
import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import com.greyorange.dessimulator.domain.put.AbstractPutStrategy;
import java.util.*;
import lombok.extern.slf4j.Slf4j;

/**
 * LP-based PUT strategy that minimizes weighted cycle distance using OR-Tools CP-SAT solver.
 *
 * <p>Optimizes placement by maximizing inventory fill while minimizing popular item distances.
 * Uses linear programming with constraints:
 *
 * <ul>
 *   <li>Storage space capacity limits
 *   <li>Max SKUs per storage space (80 for racks, 2 for totes)
 *   <li>Max inventory per SKU (respecting target fill rate)
 * </ul>
 *
 * <p>Pre-filters valid (storage_space, SKU) combinations to reduce problem size. Suitable for
 * problems up to 5M combinations. Beyond that, use {@link GreedyPutStrategy}.
 *
 * <p>Solver parameters: 15-30s time limit, 5% optimality gap, 4 parallel workers
 */
@Slf4j
public class MinWeightedCycleDistancePutStrategy extends AbstractPutStrategy {

  // LP-specific constants
  private static final int MAX_STORAGE_SPACES_PER_SKU_RTP = 100;
  private static final int MAX_STORAGE_SPACES_PER_SKU_RELAY = 600;

  public MinWeightedCycleDistancePutStrategy(
      SystemType systemType,
      ItemCatalog itemCatalog,
      List<? extends WarehouseInventory.StorageSpace> storageSpaces,
      GridPopularityManager gridPopularityManager,
      WarehouseInventory inventory) {
    super(systemType, itemCatalog, storageSpaces, gridPopularityManager, inventory);
  }

  @Override
  public void initializeInventory() {
    log.info("Initializing inventory with LP-based min weighted cycle distance algorithm...");
    long startTime = System.currentTimeMillis();

    // Load OR-Tools native library
    Loader.loadNativeLibraries();

    // System-type aware constraints
    int maxSkusPerStorageSpace =
        (systemType == SystemType.RELAY) ? MAX_SKUS_PER_TOTE : MAX_SKUS_PER_RACK;
    int maxStorageSpacesPerSku =
        (systemType == SystemType.RELAY)
            ? MAX_STORAGE_SPACES_PER_SKU_RELAY
            : MAX_STORAGE_SPACES_PER_SKU_RTP;

    // Extract data from catalog
    List<String> allSkus = new ArrayList<>(itemCatalog.getItems().keySet());
    Map<String, Integer> pastOrderedQuantities = itemCatalog.getPastOrderedQuantities();
    Map<String, Integer> maxInventoryQuantities = itemCatalog.getMaxInventoryQuantities();

    // Calculate normalized frequencies
    int maxOrderedQty = pastOrderedQuantities.values().stream().max(Integer::compare).orElse(1);
    Map<String, Double> normalizedFreq = new HashMap<>();
    for (String sku : allSkus) {
      normalizedFreq.put(sku, (double) pastOrderedQuantities.getOrDefault(sku, 0) / maxOrderedQty);
    }

    // Get storage space distances
    Map<StorageSpaceId, Double> storageSpaceDistances = new HashMap<>();
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      if (storageSpace instanceof Rack rack && rack.getStorageCoordinate() != null) {
        storageSpaceDistances.put(
            storageSpace.getId(),
            gridPopularityManager.getAverageDistance(rack.getStorageCoordinate()));
      } else if (storageSpace instanceof Tote tote && tote.getCurrentCoordinate() != null) {
        storageSpaceDistances.put(
            storageSpace.getId(),
            gridPopularityManager.getAverageDistance(tote.getCurrentCoordinate()));
      }
    }

    log.info(
        "Optimizing placement for {} SKUs across {} storage spaces",
        allSkus.size(),
        storageSpaces.size());
    log.info(
        "Max SKUs per storage space: {}, Max storage spaces per SKU: {}",
        maxSkusPerStorageSpace,
        maxStorageSpacesPerSku);

    // Pre-filter valid combinations
    Set<StorageSpaceSkuPair> validCombinations =
        prefilterValidCombinations(
            allSkus, normalizedFreq, storageSpaceDistances, maxStorageSpacesPerSku);

    // Pre-compute valid SKUs per storage space and valid storage spaces per SKU
    Map<StorageSpaceId, List<String>> validSkusPerStorageSpace = new HashMap<>();
    Map<String, List<StorageSpaceId>> validStorageSpacesPerSku = new HashMap<>();

    for (WarehouseInventory.StorageSpace ss : storageSpaces) {
      validSkusPerStorageSpace.put(ss.getId(), new ArrayList<>());
    }
    for (String sku : allSkus) {
      validStorageSpacesPerSku.put(sku, new ArrayList<>());
    }

    for (StorageSpaceSkuPair pair : validCombinations) {
      validSkusPerStorageSpace.get(pair.storageSpaceId).add(pair.sku);
      validStorageSpacesPerSku.get(pair.sku).add(pair.storageSpaceId);
    }

    // Create the CP-SAT model
    CpModel model = new CpModel();

    // Decision variables (only for valid combinations)
    Map<StorageSpaceSkuPair, IntVar> q = new HashMap<>();

    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      int maxCapacity =
          storageSpace instanceof Rack
              ? ((Rack) storageSpace).getMaxCapacity()
              : ((Tote) storageSpace).getMaxCapacity();

      for (String sku : validSkusPerStorageSpace.get(storageSpace.getId())) {
        int maxInv = maxInventoryQuantities.getOrDefault(sku, maxCapacity);
        int targetInv = computeTargetInventory(maxInv);
        int maxQty = Math.min(maxCapacity, targetInv);
        StorageSpaceSkuPair pair = new StorageSpaceSkuPair(storageSpace.getId(), sku);
        q.put(pair, model.newIntVar(0, maxQty, "q_" + storageSpace.getId() + "_" + sku));
      }
    }

    log.info("Created {} integer variables", q.size());

    // Constraint 1: Storage space capacity
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      List<String> validSkus = validSkusPerStorageSpace.get(storageSpace.getId());
      if (!validSkus.isEmpty()) {
        int maxCapacity =
            storageSpace instanceof Rack
                ? ((Rack) storageSpace).getMaxCapacity()
                : ((Tote) storageSpace).getMaxCapacity();

        LinearExprBuilder expr = LinearExpr.newBuilder();
        for (String sku : validSkus) {
          expr.add(q.get(new StorageSpaceSkuPair(storageSpace.getId(), sku)));
        }
        model.addLessOrEqual(expr, maxCapacity);
      }
    }

    // Constraint 2: Max SKUs per storage space
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      List<String> validSkus = validSkusPerStorageSpace.get(storageSpace.getId());
      if (!validSkus.isEmpty()) {
        List<BoolVar> indicators = new ArrayList<>();
        for (String sku : validSkus) {
          BoolVar isPlaced = model.newBoolVar("placed_" + storageSpace.getId() + "_" + sku);
          StorageSpaceSkuPair pair = new StorageSpaceSkuPair(storageSpace.getId(), sku);
          model.addGreaterThan(q.get(pair), 0).onlyEnforceIf(isPlaced);
          model.addEquality(q.get(pair), 0).onlyEnforceIf(isPlaced.not());
          indicators.add(isPlaced);
        }
        LinearExprBuilder expr = LinearExpr.newBuilder();
        for (BoolVar indicator : indicators) {
          expr.add(indicator);
        }
        model.addLessOrEqual(expr, maxSkusPerStorageSpace);
      }
    }

    // Constraint 3: Max inventory per SKU
    for (String sku : allSkus) {
      int maxInv = maxInventoryQuantities.getOrDefault(sku, 0);
      if (maxInv > 0) {
        int targetInv = computeTargetInventory(maxInv);

        List<StorageSpaceId> validStorageSpaces = validStorageSpacesPerSku.get(sku);
        if (!validStorageSpaces.isEmpty()) {
          LinearExprBuilder expr = LinearExpr.newBuilder();
          for (StorageSpaceId ssId : validStorageSpaces) {
            expr.add(q.get(new StorageSpaceSkuPair(ssId, sku)));
          }
          model.addLessOrEqual(expr, targetInv);
        }
      }
    }

    // Objective: MAXIMIZE placement while penalizing distant placement of popular items
    long scaleFactor = 1000;
    double maxDistance = storageSpaceDistances.values().stream().max(Double::compare).orElse(1.0);

    LinearExprBuilder objectiveExpr = LinearExpr.newBuilder();
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      for (String sku : validSkusPerStorageSpace.get(storageSpace.getId())) {
        long placementReward = -scaleFactor;

        double normalizedDistance = storageSpaceDistances.get(storageSpace.getId()) / maxDistance;
        long distancePenalty =
            (long) (normalizedFreq.get(sku) * normalizedDistance * scaleFactor * 0.5);

        long coef = placementReward + distancePenalty;

        StorageSpaceSkuPair pair = new StorageSpaceSkuPair(storageSpace.getId(), sku);
        objectiveExpr.addTerm(q.get(pair), coef);
      }
    }
    model.minimize(objectiveExpr);
    log.info("Objective: MAXIMIZE placement with distance penalty for popular items (50% weight)");

    // Solve the model
    CpSolver solver = new CpSolver();
    solver.getParameters().setMaxTimeInSeconds(15.0);
    solver.getParameters().setRelativeGapLimit(0.05);
    solver.getParameters().setNumSearchWorkers(4);
    solver.getParameters().setLogSearchProgress(false);

    log.info("Solving LP optimization problem (5% gap, 15s limit, 4 workers)...");
    CpSolverStatus status = solver.solve(model);

    long solveTime = System.currentTimeMillis() - startTime;

    // Process results
    processSolverResults(
        status,
        solver,
        q,
        validSkusPerStorageSpace,
        scaleFactor,
        solveTime,
        maxSkusPerStorageSpace,
        maxStorageSpacesPerSku);
  }

  @Override
  public String getAlgorithmName() {
    return "min_weighted_cycle_distance";
  }

  /**
   * Pre-filter valid (storage_space, SKU) combinations to reduce problem size.
   *
   * <p>For each SKU, selects top K closest storage spaces based on popularity-weighted distance
   * scoring.
   */
  private Set<StorageSpaceSkuPair> prefilterValidCombinations(
      List<String> allSkus,
      Map<String, Double> normalizedFreq,
      Map<StorageSpaceId, Double> storageSpaceDistances,
      int maxStorageSpacesPerSku) {

    log.info("Pre-filtering valid storage space-SKU combinations...");
    Set<StorageSpaceSkuPair> validCombinations = new HashSet<>();

    for (String sku : allSkus) {
      double freq = normalizedFreq.get(sku);

      List<StorageSpaceScore> storageSpaceScores = new ArrayList<>();
      for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
        double distance = storageSpaceDistances.getOrDefault(storageSpace.getId(), 0.0);
        double score = freq * distance; // Lower score is better
        storageSpaceScores.add(new StorageSpaceScore(storageSpace.getId(), score));
      }

      storageSpaceScores.sort(Comparator.comparingDouble(sss -> sss.score));
      int topK = Math.min(maxStorageSpacesPerSku, storageSpaceScores.size());

      for (int i = 0; i < topK; i++) {
        validCombinations.add(
            new StorageSpaceSkuPair(storageSpaceScores.get(i).storageSpaceId, sku));
      }
    }

    long originalSize = (long) allSkus.size() * storageSpaces.size();
    long filteredSize = validCombinations.size();
    double reductionPct = originalSize > 0 ? (1.0 - (double) filteredSize / originalSize) * 100 : 0;

    log.info("Reduced combinations: {} → {} ({:.1f}% reduction)", originalSize, filteredSize, reductionPct);
    return validCombinations;
  }

  /** Process solver results and apply inventory placement. */
  private void processSolverResults(
      CpSolverStatus status,
      CpSolver solver,
      Map<StorageSpaceSkuPair, IntVar> q,
      Map<StorageSpaceId, List<String>> validSkusPerStorageSpace,
      long scaleFactor,
      long solveTime,
      int maxSkusPerStorageSpace,
      int maxStorageSpacesPerSku) {

    // Initialize empty storage spaces
    for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
      inventory.getStorageSpaceInventory().put(storageSpace.getId(), new HashMap<>());
    }

    if (status == CpSolverStatus.OPTIMAL || status == CpSolverStatus.FEASIBLE) {
      String statusStr = status == CpSolverStatus.OPTIMAL ? "OPTIMAL" : "FEASIBLE";
      double objectiveValue = solver.objectiveValue() / scaleFactor;
      log.info("Solution found: {}", statusStr);
      log.info("Solve time: {}ms", solveTime);
      log.info("Objective value (scaled): {}", objectiveValue);

      int totalItemsPlaced = 0;
      int totalSkuPlacements = 0;

      for (WarehouseInventory.StorageSpace storageSpace : storageSpaces) {
        for (String sku : validSkusPerStorageSpace.get(storageSpace.getId())) {
          StorageSpaceSkuPair pair = new StorageSpaceSkuPair(storageSpace.getId(), sku);
          long qty = solver.value(q.get(pair));
          if (qty > 0) {
            inventory.addItemToStorageSpace(storageSpace.getId(), sku, (int) qty);
            totalItemsPlaced += qty;
            totalSkuPlacements++;
          }
        }
      }

      log.info("Placed {} items across {} storage spaces", totalItemsPlaced, storageSpaces.size());
      log.info("Total SKU-storage space placements: {}", totalSkuPlacements);

      exportInventoryAssignmentToCSV();

    } else if (status == CpSolverStatus.INFEASIBLE) {
      log.error("LP model is INFEASIBLE - constraints cannot be satisfied");
      log.error("Max SKUs per storage space: {}, Max storage spaces per SKU: {}",
          maxSkusPerStorageSpace, maxStorageSpacesPerSku);
    } else {
      log.error("LP solver returned unexpected status: {}", status);
    }
  }

  // Helper records
  private record StorageSpaceScore(StorageSpaceId storageSpaceId, double score) {}

  private record StorageSpaceSkuPair(StorageSpaceId storageSpaceId, String sku) {}
}
```

**Step 4: Run test to verify it passes**

```bash
mvn test -Dtest=MinWeightedCycleDistancePutStrategyTest
```

Expected: PASS - All 3 tests green

**Step 5: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/strategies/MinWeightedCycleDistancePutStrategy.java
git add src/test/java/com/greyorange/dessimulator/domain/put/strategies/MinWeightedCycleDistancePutStrategyTest.java
git commit -m "feat: add MinWeightedCycleDistancePutStrategy for LP optimization"
```

---

## Task 6: Create PutStrategyFactory

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/PutStrategyFactory.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/PutStrategyFactoryTest.java`

**Step 1: Write the failing test**

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.*;
import com.greyorange.dessimulator.domain.put.strategies.*;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class PutStrategyFactoryTest {

  private ItemCatalog itemCatalog;
  private List<Rack> racks;
  private GridPopularityManager gridManager;
  private WarehouseInventory inventory;

  @BeforeEach
  void setUp() {
    Item item = new Item("SKU_A", "Product A", true, 1.0);
    Map<String, Item> items = Map.of("SKU_A", item);
    Map<String, Integer> pastOrderedQuantities = Map.of("SKU_A", 100);
    Map<String, Integer> maxInventoryQuantities = Map.of("SKU_A", 50);

    itemCatalog = new ItemCatalog(items, pastOrderedQuantities, maxInventoryQuantities);

    racks = List.of(new Rack(new StorageSpaceId("R1"), 80, null));

    gridManager = new GridPopularityManager(10, 10, List.of());
    inventory = new WarehouseInventory();
  }

  @Test
  void shouldCreateGreedyStrategy() {
    PutStrategy strategy = PutStrategyFactory.createStrategy(
        PutAlgorithm.GREEDY,
        SystemType.RTP,
        itemCatalog,
        racks,
        gridManager,
        inventory);

    assertInstanceOf(GreedyPutStrategy.class, strategy);
    assertEquals("greedy", strategy.getAlgorithmName());
  }

  @Test
  void shouldCreateMinWeightedCycleDistanceStrategy() {
    PutStrategy strategy = PutStrategyFactory.createStrategy(
        PutAlgorithm.MIN_WEIGHTED_CYCLE_DISTANCE,
        SystemType.RTP,
        itemCatalog,
        racks,
        gridManager,
        inventory);

    assertInstanceOf(MinWeightedCycleDistancePutStrategy.class, strategy);
    assertEquals("min_weighted_cycle_distance", strategy.getAlgorithmName());
  }

  @Test
  void shouldFallbackToGreedyForLargeProblems() {
    // Create large problem: 5000 SKUs × 1001 racks = 5,005,000 combinations (> 5M threshold)
    Map<String, Item> largeItems = new HashMap<>();
    Map<String, Integer> largePastOrdered = new HashMap<>();
    Map<String, Integer> largeMaxInventory = new HashMap<>();

    for (int i = 0; i < 5000; i++) {
      String sku = "SKU_" + i;
      largeItems.put(sku, new Item(sku, "Product " + i, false, 1.0));
      largePastOrdered.put(sku, 10);
      largeMaxInventory.put(sku, 10);
    }

    ItemCatalog largeCatalog = new ItemCatalog(largeItems, largePastOrdered, largeMaxInventory);

    List<Rack> largeRacks = new ArrayList<>();
    for (int i = 0; i < 1001; i++) {
      largeRacks.add(new Rack(new StorageSpaceId("R" + i), 80, null));
    }

    PutStrategy strategy = PutStrategyFactory.createStrategy(
        PutAlgorithm.MIN_WEIGHTED_CYCLE_DISTANCE,  // Request LP
        SystemType.RTP,
        largeCatalog,
        largeRacks,
        gridManager,
        inventory);

    // Should fallback to greedy due to problem size
    assertInstanceOf(GreedyPutStrategy.class, strategy);
  }
}
```

**Step 2: Run test to verify it fails**

```bash
mvn test -Dtest=PutStrategyFactoryTest
```

Expected: FAIL - "cannot find symbol: class PutStrategyFactory"

**Step 3: Write minimal implementation**

```java
package com.greyorange.dessimulator.domain.put;

import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.GridPopularityManager;
import com.greyorange.dessimulator.domain.inventory.ItemCatalog;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.put.strategies.GreedyPutStrategy;
import com.greyorange.dessimulator.domain.put.strategies.MinWeightedCycleDistancePutStrategy;
import java.util.List;
import lombok.extern.slf4j.Slf4j;

/**
 * Factory for creating PUT strategy instances with adaptive algorithm selection.
 *
 * <p>Handles problem size-based fallback: if LP is requested but problem exceeds 5M combinations,
 * automatically returns greedy strategy instead.
 *
 * <p>Problem size calculation: SKU count × storage space count
 */
@Slf4j
public class PutStrategyFactory {

  private static final long PROBLEM_SIZE_GREEDY_THRESHOLD = 5_000_000L;

  /**
   * Create PUT strategy instance based on algorithm and problem size.
   *
   * <p>Adaptive selection: LP requested for problems > 5M combinations → returns greedy instead
   *
   * @param algorithm Desired PUT algorithm
   * @param systemType System type (RTP or RELAY)
   * @param itemCatalog SKU catalog with quantities
   * @param storageSpaces Available storage spaces (racks or totes)
   * @param gridPopularityManager Grid-based popularity manager
   * @param inventory Warehouse inventory to populate
   * @return Strategy instance (may fallback to greedy for large problems)
   */
  public static PutStrategy createStrategy(
      PutAlgorithm algorithm,
      SystemType systemType,
      ItemCatalog itemCatalog,
      List<? extends WarehouseInventory.StorageSpace> storageSpaces,
      GridPopularityManager gridPopularityManager,
      WarehouseInventory inventory) {

    // Calculate problem size for adaptive selection
    long problemSize = (long) itemCatalog.getItems().size() * storageSpaces.size();
    log.info(
        "Problem size: {} SKUs × {} storage spaces = {} combinations",
        itemCatalog.getItems().size(),
        storageSpaces.size(),
        problemSize);

    // Adaptive fallback: LP too large → use greedy
    if (algorithm == PutAlgorithm.MIN_WEIGHTED_CYCLE_DISTANCE
        && problemSize > PROBLEM_SIZE_GREEDY_THRESHOLD) {
      log.warn(
          "Problem too large for LP ({} combinations > {} threshold). Falling back to greedy.",
          problemSize,
          PROBLEM_SIZE_GREEDY_THRESHOLD);
      algorithm = PutAlgorithm.GREEDY;
    }

    // Create strategy instance
    return switch (algorithm) {
      case GREEDY ->
          new GreedyPutStrategy(
              systemType, itemCatalog, storageSpaces, gridPopularityManager, inventory);
      case MIN_WEIGHTED_CYCLE_DISTANCE ->
          new MinWeightedCycleDistancePutStrategy(
              systemType, itemCatalog, storageSpaces, gridPopularityManager, inventory);
    };
  }
}
```

**Step 4: Run test to verify it passes**

```bash
mvn test -Dtest=PutStrategyFactoryTest
```

Expected: PASS - All 3 tests green

**Step 5: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/PutStrategyFactory.java
git add src/test/java/com/greyorange/dessimulator/domain/put/PutStrategyFactoryTest.java
git commit -m "feat: add PutStrategyFactory with adaptive selection"
```

---

## Task 7: Refactor PutProcessor to Use Strategy Pattern

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/PutProcessor.java`

**Step 1: Write the failing test**

Modify existing PutProcessor tests to verify strategy delegation:

```bash
# No new test file - existing tests should pass with refactored implementation
# We'll verify backward compatibility in Task 8
```

**Step 2: Refactor PutProcessor**

Replace lines 28-680 in PutProcessor.java with strategy delegation:

```java
package com.greyorange.dessimulator.domain.put;

import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.GridPopularityManager;
import com.greyorange.dessimulator.domain.inventory.ItemCatalog;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import java.util.List;
import lombok.extern.slf4j.Slf4j;

/**
 * Processor for putting items into storage spaces (racks/totes).
 *
 * <p>Delegates to strategy implementations for different placement algorithms. Supports:
 *
 * <ul>
 *   <li>Greedy: Fast placement based on popularity and distance
 *   <li>Min Weighted Cycle Distance: LP optimization for optimal placement
 * </ul>
 *
 * <p>Uses {@link PutStrategyFactory} for adaptive algorithm selection (auto-fallback to greedy
 * for large problems).
 */
@Slf4j
public class PutProcessor {

  private final PutStrategy strategy;

  /**
   * Create PUT processor with specified algorithm.
   *
   * @param systemType System type (RTP or RELAY)
   * @param putAlgorithm Algorithm name ("greedy" or "min_weighted_cycle_distance")
   * @param itemCatalog SKU catalog with quantities
   * @param storageSpaces Available storage spaces
   * @param gridPopularityManager Grid-based popularity manager
   * @param inventory Warehouse inventory to populate
   */
  public PutProcessor(
      SystemType systemType,
      String putAlgorithm,
      ItemCatalog itemCatalog,
      List<? extends WarehouseInventory.StorageSpace> storageSpaces,
      GridPopularityManager gridPopularityManager,
      WarehouseInventory inventory) {

    // Convert string to enum (backward compatible with existing configs)
    PutAlgorithm algorithm;
    try {
      algorithm = PutAlgorithm.fromValue(putAlgorithm);
    } catch (IllegalArgumentException e) {
      log.warn("Unknown put algorithm '{}', using greedy", putAlgorithm);
      algorithm = PutAlgorithm.GREEDY;
    }

    // Create strategy via factory (handles adaptive selection)
    this.strategy =
        PutStrategyFactory.createStrategy(
            algorithm, systemType, itemCatalog, storageSpaces, gridPopularityManager, inventory);

    log.info("PutProcessor initialized with strategy: {}", strategy.getAlgorithmName());
  }

  /**
   * Initialize inventory based on configured strategy.
   *
   * <p>Delegates to strategy implementation (greedy or LP optimization).
   */
  public void initializeInventory() {
    strategy.initializeInventory();
  }
}
```

**Step 3: Run test to verify it passes**

```bash
mvn clean compile -DskipTests
```

Expected: Compilation success

**Step 4: Commit**

```bash
git add src/main/java/com/greyorange/dessimulator/domain/put/PutProcessor.java
git commit -m "refactor: PutProcessor delegates to strategy pattern"
```

---

## Task 8: Update Integration Tests

**Files:**
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/DesmojIntegrationTest.java`

**Step 1: Run existing integration tests**

```bash
mvn test -Dtest=DesmojIntegrationTest
```

Expected: PASS (backward compatibility maintained)

**Step 2: Add test for new enum-based configuration**

Add test method to `DesmojIntegrationTest.java`:

```java
@Test
void shouldSupportPutAlgorithmEnumConfiguration() throws IOException {
  // Load test config
  ObjectMapper mapper = new ObjectMapper();
  SimulationConfig config = mapper.readValue(
      new File("src/test/resources/integration-test-config.json"),
      SimulationConfig.class);

  // Verify putAlgorithm is parsed correctly
  assertNotNull(config.getPutAlgorithm());
  assertTrue(List.of("greedy", "min_weighted_cycle_distance").contains(config.getPutAlgorithm()));

  // Verify enum conversion works
  PutAlgorithm algorithm = PutAlgorithm.fromValue(config.getPutAlgorithm());
  assertNotNull(algorithm);
}
```

**Step 3: Run test to verify it passes**

```bash
mvn test -Dtest=DesmojIntegrationTest#shouldSupportPutAlgorithmEnumConfiguration
```

Expected: PASS

**Step 4: Run full test suite**

```bash
mvn test
```

Expected: All tests PASS

**Step 5: Commit**

```bash
git add src/test/java/com/greyorange/dessimulator/DesmojIntegrationTest.java
git commit -m "test: add integration test for PUT strategy pattern"
```

---

## Task 9: Verify Backward Compatibility

**Files:**
- Test: `apps/backend/src/test/resources/integration-test-config.json`

**Step 1: Verify greedy algorithm works**

```bash
cd apps/backend

# Start server with greedy algorithm
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 10

# Run simulation with greedy config
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "rtp",
    "duration": 1,
    "rtpRobotCount": 5,
    "binsCount": 10,
    "putAlgorithm": "greedy"
  }' | jq '.success'

# Cleanup
lsof -ti:8080 | xargs kill -9
```

Expected: Returns `true`

**Step 2: Verify min_weighted_cycle_distance algorithm works**

```bash
# Start server
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 10

# Run simulation with LP config
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "rtp",
    "duration": 1,
    "rtpRobotCount": 5,
    "binsCount": 10,
    "putAlgorithm": "min_weighted_cycle_distance"
  }' | jq '.success'

# Cleanup
lsof -ti:8080 | xargs kill -9
```

Expected: Returns `true`

**Step 3: Verify invalid algorithm falls back to greedy**

```bash
# Start server
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 10

# Run simulation with invalid algorithm
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "rtp",
    "duration": 1,
    "rtpRobotCount": 5,
    "binsCount": 10,
    "putAlgorithm": "invalid_algorithm"
  }' | jq '.success'

# Check logs for fallback warning
grep "Unknown put algorithm 'invalid_algorithm', using greedy" /tmp/server.log

# Cleanup
lsof -ti:8080 | xargs kill -9
```

Expected: Returns `true`, log shows fallback warning

**Step 4: Final commit**

```bash
git add .
git commit -m "test: verify backward compatibility for PUT strategies"
```

---

## Summary

**What We Built:**
- Type-safe `PutAlgorithm` enum (like `OrderSelectionAlgorithm`)
- `PutStrategy` interface with `initializeInventory()` contract
- `AbstractPutStrategy` base class extracting shared logic (ABC classification, metadata indexing, CSV export)
- `GreedyPutStrategy` and `MinWeightedCycleDistancePutStrategy` concrete implementations
- `PutStrategyFactory` with adaptive selection (auto-fallback for large problems)
- Refactored `PutProcessor` to delegate to strategies

**Benefits:**
- ✅ Type-safe algorithm selection (compile-time errors for invalid algorithms)
- ✅ Open/closed principle (add new strategies without modifying PutProcessor)
- ✅ Shared logic extracted to base class (DRY)
- ✅ Factory handles adaptive selection (LP → greedy fallback)
- ✅ Backward compatible (existing string configs still work)
- ✅ Consistent with order selection pattern

**Files Modified:** 9 created, 1 refactored
**Tests Added:** 7 test classes
**Lines Removed from PutProcessor:** ~650 (moved to strategies)

---

## Execution Options

Plan saved to `docs/plans/2025-11-17-put-strategy-pattern-refactoring.md`.

**Two execution approaches:**

**1. Subagent-Driven (this session)**
- I dispatch fresh subagent per task
- Review between tasks
- Fast iteration with quality gates
- **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development

**2. Parallel Session (separate)**
- Open new session with executing-plans
- Batch execution with checkpoints
- **REQUIRED SUB-SKILL:** superpowers:executing-plans in new session

**Which approach would you like?**
