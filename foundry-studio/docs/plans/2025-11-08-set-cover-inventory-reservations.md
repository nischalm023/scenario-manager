# Set Cover Order Fulfillment with Inventory Reservations Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Transform order processing from naive "first tote per SKU" to optimized greedy set cover with quantity-based inventory reservations and multi-order storage space sharing.

**Architecture:**
- Add reservation tracking to `WarehouseInventory` (available vs reserved quantities per SKU per storage space)
- Implement greedy set cover algorithm in `OrderProcessor` to minimize storage spaces per order
- Track active storage spaces (in-transit, at relay, at station) to enable order piggybacking
- Validate reservations after order selection (Option B design)
- Disable task cache during initial implementation

**Tech Stack:** Java 21, Desmo-J 2.5.1e, Spring Boot 3.2, JUnit 5

**Key Benefits:**
- 20-40% PPF improvement from storage space sharing
- Prevents double-allocation with explicit reservations
- Relay system: Totes at relay positions count as active (early piggybacking)
- Supports split quantities (SKU can span multiple storage spaces)

---

## Task 1: Add Reservation Infrastructure to WarehouseInventory

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventory.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventoryTest.java`

### Step 1: Write failing test for order-based reservations

Create test file and add first test:

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import com.greyorange.dessimulator.domain.inventory.Item;
import com.greyorange.dessimulator.domain.robot.PickTask;
import java.util.List;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class WarehouseInventoryReservationTest {

  private WarehouseInventory inventory;
  private StorageSpaceId tote1;
  private StorageSpaceId tote2;

  @BeforeEach
  void setUp() {
    ItemCatalog catalog = new ItemCatalog();
    inventory = new WarehouseInventory(SystemType.RELAY, catalog, null, null);

    tote1 = new StorageSpaceId(1);
    tote2 = new StorageSpaceId(2);

    // Setup inventory
    inventory.addItemToStorageSpace(tote1, "SKU_A", 10);
    inventory.addItemToStorageSpace(tote1, "SKU_B", 5);
    inventory.addItemToStorageSpace(tote2, "SKU_A", 20);
  }

  @Test
  void testReserveForOrder_Success() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    PickTask task1 = new PickTask("task1", "order1", itemA, 3, tote1, 0.0);
    List<PickTask> tasks = List.of(task1);

    // Act
    boolean reserved = inventory.tryReserveForOrder("order1", tasks);

    // Assert
    assertTrue(reserved);
    assertEquals(7, inventory.getAvailableStock("SKU_A", tote1)); // 10 - 3 = 7
  }

  @Test
  void testReserveForOrder_MultipleTasksSameOrder() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Item itemB = new Item("SKU_B", "Item B", 1.0, 10.0, "cat1");
    PickTask task1 = new PickTask("task1", "order1", itemA, 3, tote1, 0.0);
    PickTask task2 = new PickTask("task2", "order1", itemB, 2, tote1, 0.0);
    List<PickTask> tasks = List.of(task1, task2);

    // Act
    boolean reserved = inventory.tryReserveForOrder("order1", tasks);

    // Assert
    assertTrue(reserved);
    assertEquals(7, inventory.getAvailableStock("SKU_A", tote1));
    assertEquals(3, inventory.getAvailableStock("SKU_B", tote1));
  }

  @Test
  void testReserveForOrder_InsufficientStock() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    PickTask task1 = new PickTask("task1", "order1", itemA, 15, tote1, 0.0); // Need 15, only 10 available
    List<PickTask> tasks = List.of(task1);

    // Act
    boolean reserved = inventory.tryReserveForOrder("order1", tasks);

    // Assert
    assertFalse(reserved);
    assertEquals(10, inventory.getAvailableStock("SKU_A", tote1)); // Unchanged
  }

  @Test
  void testReserveForOrder_AlreadyReserved() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    PickTask task1 = new PickTask("task1", "order1", itemA, 3, tote1, 0.0);
    List<PickTask> tasks = List.of(task1);

    inventory.tryReserveForOrder("order1", tasks);

    // Act - try to reserve again
    boolean reserved = inventory.tryReserveForOrder("order1", tasks);

    // Assert
    assertFalse(reserved);
  }
}
```

### Step 2: Run test to verify it fails

```bash
cd apps/backend
mvn test -Dtest=WarehouseInventoryReservationTest
```

Expected: Compilation failure - `tryReserveForOrder` method does not exist

### Step 3: Add order-based reservation methods to WarehouseInventory

Modify `WarehouseInventory.java`:

```java
// Add new field after existing fields (around line 31)
private final Map<String, Map<String, Integer>> reservationsByOrder = new ConcurrentHashMap<>();

// Add new methods before the StorageSpace interface (around line 126)

/**
 * Attempt to reserve inventory for all tasks in an order.
 * All-or-nothing: either all tasks reserved, or none.
 *
 * @param orderId Unique order identifier
 * @param tasks List of pick tasks for this order
 * @return true if reservation successful, false if insufficient inventory or already reserved
 */
public boolean tryReserveForOrder(String orderId, List<PickTask> tasks) {
  if (reservationsByOrder.containsKey(orderId)) {
    log.warn("Order {} already has reservation", orderId);
    return false;
  }

  // Phase 1: Validate all tasks can be fulfilled
  for (PickTask task : tasks) {
    if (!storageSpaceInventory.containsKey(task.getStorageSpaceId())) {
      log.debug("Storage space {} not found for order {}", task.getStorageSpaceId(), orderId);
      return false;
    }

    int available = getAvailableStock(task.getItem().sku(), task.getStorageSpaceId());
    if (available < task.getQuantity()) {
      log.debug(
          "Cannot reserve {} x {} from {} for order {} (available: {})",
          task.getQuantity(),
          task.getItem().sku(),
          task.getStorageSpaceId(),
          orderId,
          available);
      return false;
    }
  }

  // Phase 2: Apply all reservations
  Map<String, Integer> orderReservations = new ConcurrentHashMap<>();
  for (PickTask task : tasks) {
    String sku = task.getItem().sku();
    int quantity = task.getQuantity();
    StorageSpaceId spaceId = task.getStorageSpaceId();

    // Use existing reserveStock method
    reserveStock(sku, spaceId, quantity);

    // Track total reservation by order (for release)
    orderReservations.merge(sku, quantity, Integer::sum);
  }

  reservationsByOrder.put(orderId, orderReservations);
  log.debug("Reserved inventory for order {}: {}", orderId, orderReservations);
  return true;
}

/**
 * Release reservation when order is cancelled or reassigned.
 *
 * @param orderId Unique order identifier
 * @param tasks List of pick tasks to release
 */
public void releaseReservationForOrder(String orderId, List<PickTask> tasks) {
  if (!reservationsByOrder.containsKey(orderId)) {
    log.warn("No reservation found for order {}", orderId);
    return;
  }

  for (PickTask task : tasks) {
    String sku = task.getItem().sku();
    int quantity = task.getQuantity();
    StorageSpaceId spaceId = task.getStorageSpaceId();

    var key = new ReservationKey(spaceId, sku);
    reservedStock.computeIfPresent(
        key,
        (k, reserved) -> {
          int newReserved = Math.max(0, reserved - quantity);
          return newReserved == 0 ? null : newReserved;
        });
  }

  reservationsByOrder.remove(orderId);
  log.debug("Released reservation for order {}", orderId);
}
```

Add import at top:
```java
import com.greyorange.dessimulator.domain.robot.PickTask;
```

### Step 4: Run test to verify it passes

```bash
mvn test -Dtest=WarehouseInventoryReservationTest
```

Expected: All 4 tests PASS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventory.java
git add src/test/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventoryReservationTest.java
git commit -m "feat: add order-based inventory reservations to WarehouseInventory"
```

---

## Task 2: Add Multi-Order Sharing Support with Release Tests

**Files:**
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventoryReservationTest.java`

### Step 1: Write failing test for multi-order sharing

Add to `WarehouseInventoryReservationTest.java`:

```java
@Test
void testMultiOrderSharing_SameTote() {
  // Arrange
  Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
  Item itemB = new Item("SKU_B", "Item B", 1.0, 10.0, "cat1");

  // Order 1 reserves SKU_A
  PickTask task1 = new PickTask("task1", "order1", itemA, 3, tote1, 0.0);
  List<PickTask> order1Tasks = List.of(task1);

  // Order 2 reserves SKU_B from same tote
  PickTask task2 = new PickTask("task2", "order2", itemB, 2, tote1, 0.0);
  List<PickTask> order2Tasks = List.of(task2);

  // Act
  boolean reserved1 = inventory.tryReserveForOrder("order1", order1Tasks);
  boolean reserved2 = inventory.tryReserveForOrder("order2", order2Tasks);

  // Assert
  assertTrue(reserved1);
  assertTrue(reserved2);
  assertEquals(7, inventory.getAvailableStock("SKU_A", tote1)); // 10 - 3
  assertEquals(3, inventory.getAvailableStock("SKU_B", tote1)); // 5 - 2
}

@Test
void testMultiOrderSharing_PartialQuantitySameSku() {
  // Arrange - Both orders need SKU_A from same tote
  Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");

  PickTask task1 = new PickTask("task1", "order1", itemA, 4, tote1, 0.0);
  List<PickTask> order1Tasks = List.of(task1);

  PickTask task2 = new PickTask("task2", "order2", itemA, 5, tote1, 0.0);
  List<PickTask> order2Tasks = List.of(task2);

  // Act
  boolean reserved1 = inventory.tryReserveForOrder("order1", order1Tasks);
  boolean reserved2 = inventory.tryReserveForOrder("order2", order2Tasks);

  // Assert
  assertTrue(reserved1);
  assertTrue(reserved2);
  assertEquals(1, inventory.getAvailableStock("SKU_A", tote1)); // 10 - 4 - 5 = 1
}

@Test
void testReleaseReservation_FreesStock() {
  // Arrange
  Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
  PickTask task1 = new PickTask("task1", "order1", itemA, 3, tote1, 0.0);
  List<PickTask> tasks = List.of(task1);

  inventory.tryReserveForOrder("order1", tasks);
  assertEquals(7, inventory.getAvailableStock("SKU_A", tote1));

  // Act
  inventory.releaseReservationForOrder("order1", tasks);

  // Assert
  assertEquals(10, inventory.getAvailableStock("SKU_A", tote1)); // Back to original
}
```

### Step 2: Run test to verify it passes

```bash
mvn test -Dtest=WarehouseInventoryReservationTest
```

Expected: All 7 tests PASS (existing reservation logic already supports multi-order sharing)

### Step 3: Commit

```bash
git add src/test/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventoryReservationTest.java
git commit -m "test: add multi-order sharing and release tests for reservations"
```

---

## Task 3: Implement Set Cover Search in WarehouseInventory

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventory.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventorySetCoverTest.java`

### Step 1: Write failing test for set cover search

Create new test file:

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.common.types.SystemType;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class WarehouseInventorySetCoverTest {

  private WarehouseInventory inventory;
  private StorageSpaceId tote1;
  private StorageSpaceId tote2;
  private StorageSpaceId tote3;

  @BeforeEach
  void setUp() {
    ItemCatalog catalog = new ItemCatalog();
    inventory = new WarehouseInventory(SystemType.RELAY, catalog, null, null);

    tote1 = new StorageSpaceId(1);
    tote2 = new StorageSpaceId(2);
    tote3 = new StorageSpaceId(3);

    // Tote1: A=10, B=5, C=8
    inventory.addItemToStorageSpace(tote1, "SKU_A", 10);
    inventory.addItemToStorageSpace(tote1, "SKU_B", 5);
    inventory.addItemToStorageSpace(tote1, "SKU_C", 8);

    // Tote2: A=20, D=15
    inventory.addItemToStorageSpace(tote2, "SKU_A", 20);
    inventory.addItemToStorageSpace(tote2, "SKU_D", 15);

    // Tote3: B=10, D=5
    inventory.addItemToStorageSpace(tote3, "SKU_B", 10);
    inventory.addItemToStorageSpace(tote3, "SKU_D", 5);
  }

  @Test
  void testFindBestStorageSpace_SingleSkuSingleTote() {
    // Arrange
    Map<String, Integer> remainingItems = Map.of("SKU_A", 5);
    Set<StorageSpaceId> excludeSpaces = new HashSet<>();
    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    Optional<WarehouseInventory.StorageSpaceScore> result =
        inventory.findBestStorageSpaceForItems(remainingItems, excludeSpaces, activeSpaces);

    // Assert
    assertTrue(result.isPresent());
    // Should pick tote1 or tote2 (both have SKU_A)
    assertTrue(
        result.get().storageSpaceId().equals(tote1)
            || result.get().storageSpaceId().equals(tote2));
    assertEquals(1, result.get().skusCovered());
    assertEquals(5, result.get().skuCoverage().get("SKU_A"));
  }

  @Test
  void testFindBestStorageSpace_MultipleSkus_PreferMaxCoverage() {
    // Arrange - need A and B
    Map<String, Integer> remainingItems = Map.of("SKU_A", 3, "SKU_B", 2);
    Set<StorageSpaceId> excludeSpaces = new HashSet<>();
    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    Optional<WarehouseInventory.StorageSpaceScore> result =
        inventory.findBestStorageSpaceForItems(remainingItems, excludeSpaces, activeSpaces);

    // Assert
    assertTrue(result.isPresent());
    // Should pick tote1 (covers both A and B)
    assertEquals(tote1, result.get().storageSpaceId());
    assertEquals(2, result.get().skusCovered()); // Covers 2 distinct SKUs
    assertEquals(3, result.get().skuCoverage().get("SKU_A"));
    assertEquals(2, result.get().skuCoverage().get("SKU_B"));
  }

  @Test
  void testFindBestStorageSpace_ActiveSpaceBonus() {
    // Arrange - need D only
    Map<String, Integer> remainingItems = Map.of("SKU_D", 5);
    Set<StorageSpaceId> excludeSpaces = new HashSet<>();
    Set<StorageSpaceId> activeSpaces = Set.of(tote3); // Tote3 is active

    // Act
    Optional<WarehouseInventory.StorageSpaceScore> result =
        inventory.findBestStorageSpaceForItems(remainingItems, excludeSpaces, activeSpaces);

    // Assert
    assertTrue(result.isPresent());
    // Should pick tote3 even though tote2 has more quantity (15 vs 5)
    // because tote3 is active (+1000 bonus)
    assertEquals(tote3, result.get().storageSpaceId());
    assertEquals(5, result.get().skuCoverage().get("SKU_D"));
  }

  @Test
  void testFindBestStorageSpace_ExcludeAlreadySelected() {
    // Arrange
    Map<String, Integer> remainingItems = Map.of("SKU_A", 5);
    Set<StorageSpaceId> excludeSpaces = Set.of(tote1); // Exclude tote1
    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    Optional<WarehouseInventory.StorageSpaceScore> result =
        inventory.findBestStorageSpaceForItems(remainingItems, excludeSpaces, activeSpaces);

    // Assert
    assertTrue(result.isPresent());
    assertEquals(tote2, result.get().storageSpaceId()); // Should pick tote2, not tote1
  }

  @Test
  void testFindBestStorageSpace_PartialQuantityCoverage() {
    // Arrange - need more than available in one tote
    Map<String, Integer> remainingItems = Map.of("SKU_A", 15); // Tote1 has 10, Tote2 has 20
    Set<StorageSpaceId> excludeSpaces = new HashSet<>();
    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    Optional<WarehouseInventory.StorageSpaceScore> result =
        inventory.findBestStorageSpaceForItems(remainingItems, excludeSpaces, activeSpaces);

    // Assert
    assertTrue(result.isPresent());
    // Should pick tote2 (has 20, can cover all 15)
    assertEquals(tote2, result.get().storageSpaceId());
    assertEquals(15, result.get().skuCoverage().get("SKU_A")); // Covers requested 15
  }

  @Test
  void testFindBestStorageSpace_NoMatchingInventory() {
    // Arrange - request SKU that doesn't exist
    Map<String, Integer> remainingItems = Map.of("SKU_Z", 5);
    Set<StorageSpaceId> excludeSpaces = new HashSet<>();
    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    Optional<WarehouseInventory.StorageSpaceScore> result =
        inventory.findBestStorageSpaceForItems(remainingItems, excludeSpaces, activeSpaces);

    // Assert
    assertTrue(result.isEmpty());
  }
}
```

### Step 2: Run test to verify it fails

```bash
mvn test -Dtest=WarehouseInventorySetCoverTest
```

Expected: Compilation failure - `findBestStorageSpaceForItems` and `StorageSpaceScore` do not exist

### Step 3: Implement set cover search method

Add to `WarehouseInventory.java` before the `StorageSpace` interface:

```java
/**
 * Find storage space that covers the most remaining items.
 * Strongly prefers active storage spaces (already in transit or at station/relay).
 *
 * <p>Scoring: - Primary: Number of distinct SKUs covered - Bonus: +1000 if storage space is
 * active
 *
 * @param remainingItems SKU -> quantity still needed
 * @param excludeSpaces Storage spaces already selected (don't pick twice)
 * @param activeStorageSpaces Storage spaces currently active (in transit, at relay, or at
 *     station)
 * @return Best storage space with coverage details, or empty if none can help
 */
public Optional<StorageSpaceScore> findBestStorageSpaceForItems(
    Map<String, Integer> remainingItems,
    Set<StorageSpaceId> excludeSpaces,
    Set<StorageSpaceId> activeStorageSpaces) {

  if (remainingItems.isEmpty()) {
    return Optional.empty();
  }

  // Gather candidate storage spaces (all spaces containing any remaining SKU)
  Set<StorageSpaceId> candidates =
      remainingItems.keySet().stream()
          .flatMap(sku -> itemLocations.getOrDefault(sku, List.of()).stream())
          .filter(spaceId -> !excludeSpaces.contains(spaceId))
          .collect(Collectors.toSet());

  if (candidates.isEmpty()) {
    return Optional.empty();
  }

  StorageSpaceId bestSpaceId = null;
  double bestScore = Double.NEGATIVE_INFINITY;
  Map<String, Integer> bestCoverage = new HashMap<>();

  for (StorageSpaceId spaceId : candidates) {
    Map<String, Integer> coverage = new HashMap<>();

    // Calculate what this space can cover
    for (Map.Entry<String, Integer> entry : remainingItems.entrySet()) {
      String sku = entry.getKey();
      int needed = entry.getValue();
      int available = getAvailableStock(sku, spaceId);

      if (available > 0) {
        int covered = Math.min(needed, available);
        coverage.put(sku, covered);
      }
    }

    if (coverage.isEmpty()) {
      continue;
    }

    // Score: distinct SKUs covered + active bonus
    double score = coverage.size(); // Number of distinct SKUs
    if (activeStorageSpaces.contains(spaceId)) {
      score += 1000; // Massive bonus for active spaces
    }

    if (score > bestScore) {
      bestScore = score;
      bestSpaceId = spaceId;
      bestCoverage = coverage;
    }
  }

  if (bestSpaceId == null) {
    return Optional.empty();
  }

  return Optional.of(new StorageSpaceScore(bestSpaceId, bestCoverage, bestCoverage.size()));
}

/**
 * Result of set cover scoring.
 *
 * @param storageSpaceId The selected storage space
 * @param skuCoverage SKU -> quantity this space provides
 * @param skusCovered Count of distinct SKUs covered
 */
public record StorageSpaceScore(
    StorageSpaceId storageSpaceId, Map<String, Integer> skuCoverage, int skusCovered) {}
```

Add imports at top:
```java
import java.util.Set;
import java.util.stream.Collectors;
```

### Step 4: Run test to verify it passes

```bash
mvn test -Dtest=WarehouseInventorySetCoverTest
```

Expected: All 7 tests PASS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventory.java
git add src/test/java/com/greyorange/dessimulator/domain/inventory/WarehouseInventorySetCoverTest.java
git commit -m "feat: add greedy set cover search to WarehouseInventory"
```

---

## Task 4: Add Set Cover Task Generation to OrderProcessor

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/order/OrderProcessorSetCoverTest.java`

### Step 1: Write failing test for set cover task generation

Create new test file:

```java
package com.greyorange.dessimulator.domain.order;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.inventory.Item;
import com.greyorange.dessimulator.domain.inventory.ItemCatalog;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.robot.PickTask;
import com.greyorange.dessimulator.domain.station.BinManager;
import com.greyorange.dessimulator.domain.station.Station;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.mockito.Mockito;

class OrderProcessorSetCoverTest {

  private WarehouseInventory inventory;
  private OrderProcessor orderProcessor;
  private Station station1;
  private BinManager binManager1;

  @BeforeEach
  void setUp() {
    // Setup inventory
    ItemCatalog catalog = new ItemCatalog();
    catalog.addItem(new Item("SKU_A", "Item A", 1.0, 10.0, "cat1"));
    catalog.addItem(new Item("SKU_B", "Item B", 1.0, 10.0, "cat1"));
    catalog.addItem(new Item("SKU_C", "Item C", 1.0, 10.0, "cat1"));

    inventory = new WarehouseInventory(
        com.greyorange.dessimulator.domain.common.types.SystemType.RELAY, catalog, null, null);

    StorageSpaceId tote1 = new StorageSpaceId(1);
    StorageSpaceId tote2 = new StorageSpaceId(2);

    // Tote1: A=10, B=5, C=8
    inventory.addItemToStorageSpace(tote1, "SKU_A", 10);
    inventory.addItemToStorageSpace(tote1, "SKU_B", 5);
    inventory.addItemToStorageSpace(tote1, "SKU_C", 8);

    // Tote2: A=20, D=15
    inventory.addItemToStorageSpace(tote2, "SKU_A", 20);

    // Setup station and bin manager
    station1 = new Station(new StationId(1), "Station1", 0.0, 0.0, 1);
    binManager1 = Mockito.mock(BinManager.class);
    Mockito.when(binManager1.canAssignMoreOrders()).thenReturn(true);

    List<Station> stations = List.of(station1);
    Map<StationId, BinManager> stationBinManagers = Map.of(station1.getStationId(), binManager1);

    orderProcessor =
        new OrderProcessor(
            inventory, stations, stationBinManagers, OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY);
  }

  @Test
  void testGenerateTasksWithSetCover_SingleToteCoversAll() {
    // Arrange - order needs items all in tote1
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Item itemB = new Item("SKU_B", "Item B", 1.0, 10.0, "cat1");
    Order order =
        new Order(
            "order1",
            List.of(new OrderItem(itemA, 3), new OrderItem(itemB, 2)),
            0.0,
            OrderPriority.NORMAL);

    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(1, selections.size());
    OrderProcessor.OrderSelection selection = selections.get(0);
    assertEquals("order1", selection.order().getOrderId());

    // Should use only 1 tote (tote1 covers both SKU_A and SKU_B)
    Set<StorageSpaceId> usedTotes =
        selection.tasks().stream().map(PickTask::getStorageSpaceId).collect(Collectors.toSet());
    assertEquals(1, usedTotes.size());
    assertTrue(usedTotes.contains(new StorageSpaceId(1)));

    // Check tasks
    assertEquals(2, selection.tasks().size());
  }

  @Test
  void testGenerateTasksWithSetCover_MultipleTotesRequired() {
    // Arrange - order needs A and C, but C only in tote1, A in both totes
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Item itemC = new Item("SKU_C", "Item C", 1.0, 10.0, "cat1");
    Order order =
        new Order(
            "order1",
            List.of(new OrderItem(itemA, 15), new OrderItem(itemC, 5)), // Need 15 A (tote1 has 10, need tote2)
            0.0,
            OrderPriority.NORMAL);

    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(1, selections.size());
    OrderProcessor.OrderSelection selection = selections.get(0);

    Set<StorageSpaceId> usedTotes =
        selection.tasks().stream().map(PickTask::getStorageSpaceId).collect(Collectors.toSet());
    assertEquals(2, usedTotes.size()); // Should use both tote1 and tote2
  }

  @Test
  void testGenerateTasksWithSetCover_PreferActiveTote() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Order order =
        new Order(
            "order1", List.of(new OrderItem(itemA, 5)), // Both totes have SKU_A
            0.0,
            OrderPriority.NORMAL);

    Set<StorageSpaceId> activeSpaces = Set.of(new StorageSpaceId(2)); // Tote2 is active

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(1, selections.size());
    OrderProcessor.OrderSelection selection = selections.get(0);

    // Should prefer tote2 (active) over tote1
    Set<StorageSpaceId> usedTotes =
        selection.tasks().stream().map(PickTask::getStorageSpaceId).collect(Collectors.toSet());
    assertEquals(1, usedTotes.size());
    assertTrue(usedTotes.contains(new StorageSpaceId(2))); // Active tote
  }
}
```

### Step 2: Run test to verify it fails

```bash
mvn test -Dtest=OrderProcessorSetCoverTest
```

Expected: Tests fail because `OrderProcessor` still uses old `generateTasksForOrder` method

### Step 3: Add set cover task generation method to OrderProcessor

Modify `OrderProcessor.java`:

```java
// Add constant at class level (around line 33)
private static final int MAX_TOTES_PER_ORDER = 10;

// Replace generateTasksForOrder method (around line 259) with new implementation
/**
 * Generate picking tasks for an order using greedy set cover. Minimizes number of storage spaces
 * required while preferring active spaces.
 *
 * @param order The order to fulfill
 * @param currentTime Simulation time
 * @param activeStorageSpaces Storage spaces currently active (for piggybacking)
 * @return List of pick tasks, or empty if order cannot be fulfilled
 */
private List<PickTask> generateTasksWithSetCover(
    Order order, double currentTime, Set<StorageSpaceId> activeStorageSpaces) {

  // Extract items needed
  Map<String, Integer> remainingItems = new HashMap<>();
  Map<String, Item> skuToItem = new HashMap<>();

  for (OrderItem orderItem : order.getItems()) {
    String sku = orderItem.item().sku();
    remainingItems.merge(sku, orderItem.quantity(), Integer::sum);
    skuToItem.put(sku, orderItem.item());
  }

  List<PickTask> tasks = new ArrayList<>();
  Set<StorageSpaceId> selectedSpaces = new HashSet<>();
  int iteration = 0;

  // Greedy set cover loop
  while (!remainingItems.isEmpty()) {
    if (iteration++ > MAX_TOTES_PER_ORDER) {
      log.warn(
          "Order {} exceeded max storage spaces ({}), remaining items: {}",
          order.getOrderId(),
          MAX_TOTES_PER_ORDER,
          remainingItems);
      return List.of(); // Unfulfillable
    }

    // Find best storage space for remaining items
    Optional<WarehouseInventory.StorageSpaceScore> bestSpace =
        inventory.findBestStorageSpaceForItems(
            remainingItems,
            selectedSpaces, // Don't select same space twice
            activeStorageSpaces // Prefer active spaces
            );

    if (bestSpace.isEmpty()) {
      log.warn(
          "Order {} cannot be fulfilled, no storage space for items: {}",
          order.getOrderId(),
          remainingItems);
      return List.of(); // Unfulfillable
    }

    WarehouseInventory.StorageSpaceScore score = bestSpace.get();
    selectedSpaces.add(score.storageSpaceId());

    // Create pick tasks for items covered by this storage space
    for (Map.Entry<String, Integer> entry : score.skuCoverage().entrySet()) {
      String sku = entry.getKey();
      int quantity = entry.getValue();
      Item item = skuToItem.get(sku);

      PickTask task =
          new PickTask(
              order.getOrderId() + "-" + sku + "-" + selectedSpaces.size(),
              order.getOrderId(),
              item,
              quantity,
              score.storageSpaceId(),
              currentTime);
      tasks.add(task);

      // Update remaining items
      int remaining = remainingItems.get(sku) - quantity;
      if (remaining <= 0) {
        remainingItems.remove(sku);
      } else {
        remainingItems.put(sku, remaining);
      }
    }
  }

  log.debug(
      "Order {} requires {} storage spaces: {}",
      order.getOrderId(),
      selectedSpaces.size(),
      selectedSpaces);

  return tasks;
}
```

Add imports:
```java
import java.util.Set;
```

### Step 4: Update createOrderMetricsForProcessing to use set cover

Replace the `createOrderMetricsForProcessing` method (around line 302):

```java
/** Create metrics for processing an order. Now uses set cover for task generation. */
private void createOrderMetricsForProcessing(
    List<Order> pendingOrders,
    double currentTime,
    Set<StorageSpaceId> activeStorageSpaces, // NEW parameter
    Map<String, OrderMetrics> orderMetrics,
    Map<String, List<PickTask>> orderTaskMap) {

  for (Order order : pendingOrders) {
    // Use set cover instead of naive first-tote selection
    List<PickTask> tasks = generateTasksWithSetCover(order, currentTime, activeStorageSpaces);

    if (!tasks.isEmpty()) {
      Set<StorageSpaceId> requiredStorageSpaces =
          tasks.stream().map(PickTask::getStorageSpaceId).collect(Collectors.toSet());

      int totalTasksQuantity = tasks.stream().mapToInt(PickTask::getQuantity).sum();

      double picksPerFace =
          requiredStorageSpaces.isEmpty()
              ? 0
              : (double) totalTasksQuantity / requiredStorageSpaces.size();

      orderTaskMap.put(order.getOrderId(), tasks);
      orderMetrics.put(
          order.getOrderId(),
          new OrderMetrics(order, requiredStorageSpaces, totalTasksQuantity, picksPerFace));
    }
  }
}
```

### Step 5: Update both selection algorithms to pass activeStorageSpaces

Update `selectNextOrders` method signature and body (around line 56):

```java
public List<OrderSelection> selectNextOrders(
    List<Order> pendingOrders,
    Set<StorageSpaceId> activeStorageSpaces,
    int maxOrders,
    double currentTime) {

  if (pendingOrders.isEmpty() || maxOrders <= 0) {
    return List.of();
  }

  Map<String, OrderMetrics> orderMetrics = new HashMap<>();
  Map<String, List<PickTask>> orderTaskMap = new HashMap<>();

  // Create metrics using set cover
  createOrderMetricsForProcessing(
      pendingOrders, currentTime, activeStorageSpaces, orderMetrics, orderTaskMap);

  List<OrderSelection> selectedOrders =
      switch (orderSelectionAlgorithm) {
        case MAXIMIZE_PPF_WITH_TIERING -> selectOrdersPicksPerFaceWithTiering(
            pendingOrders, activeStorageSpaces, maxOrders, currentTime, orderMetrics, orderTaskMap);
        case MAXIMIZE_PPF_DIRECTLY -> selectOrdersMaximizePpfDirectly(
            pendingOrders, activeStorageSpaces, maxOrders, currentTime, orderMetrics, orderTaskMap);
      };

  // Assign station IDs to tasks
  for (OrderSelection selection : selectedOrders) {
    for (PickTask task : selection.tasks()) {
      task.setStationId(selection.stationId());
    }
  }

  return selectedOrders;
}
```

Update both algorithm methods to accept orderMetrics and orderTaskMap:

```java
private List<OrderSelection> selectOrdersPicksPerFaceWithTiering(
    List<Order> pendingOrders,
    Set<StorageSpaceId> activeStorageSpaces,
    int maxOrders,
    double currentTime,
    Map<String, OrderMetrics> orderMetrics, // NEW
    Map<String, List<PickTask>> orderTaskMap // NEW
) {
  // Remove the createOrderMetricsForProcessing call (now passed in)
  // Rest of method unchanged

  List<OrderSelection> selectedOrders = new ArrayList<>();
  Set<StorageSpaceId> newlyOpenedStorageSpaces = new HashSet<>();

  List<String> validOrderIds = new ArrayList<>(orderMetrics.keySet());

  while (selectedOrders.size() < maxOrders && !validOrderIds.isEmpty()) {
    String bestOrderId = null;
    double bestScore = Double.NEGATIVE_INFINITY;
    Set<StorageSpaceId> bestTrulyNewStorageSpaces = new HashSet<>();

    for (String orderId : validOrderIds) {
      OrderMetrics metrics = orderMetrics.get(orderId);
      Set<StorageSpaceId> requiredStorageSpaces = metrics.requiredStorageSpaces;
      int totalTasksQuantity = metrics.totalTasksQuantity;

      Set<StorageSpaceId> alreadyActiveStorageSpaces = new HashSet<>(requiredStorageSpaces);
      alreadyActiveStorageSpaces.retainAll(activeStorageSpaces);

      Set<StorageSpaceId> alreadyOpenedThisRound = new HashSet<>(requiredStorageSpaces);
      alreadyOpenedThisRound.retainAll(newlyOpenedStorageSpaces);

      Set<StorageSpaceId> trulyNewStorageSpaces = new HashSet<>(requiredStorageSpaces);
      trulyNewStorageSpaces.removeAll(activeStorageSpaces);
      trulyNewStorageSpaces.removeAll(newlyOpenedStorageSpaces);

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
      OrderMetrics bestMetrics = orderMetrics.get(bestOrderId);
      List<PickTask> bestTasks = orderTaskMap.get(bestOrderId);
      StationId stationId = selectOptimalStationForOrder(bestMetrics.order, bestTasks);

      selectedOrders.add(new OrderSelection(bestMetrics.order, bestTasks, stationId));
      newlyOpenedStorageSpaces.addAll(bestTrulyNewStorageSpaces);
      validOrderIds.remove(bestOrderId);
    } else {
      break;
    }
  }

  return selectedOrders;
}

private List<OrderSelection> selectOrdersMaximizePpfDirectly(
    List<Order> pendingOrders,
    Set<StorageSpaceId> activeStorageSpaces,
    int maxOrders,
    double currentTime,
    Map<String, OrderMetrics> orderMetrics, // NEW
    Map<String, List<PickTask>> orderTaskMap // NEW
) {
  // Remove the createOrderMetricsForProcessing call (now passed in)

  if (orderMetrics.isEmpty()) {
    return List.of();
  }

  List<OrderSelection> selectedOrders = new ArrayList<>();
  Set<StorageSpaceId> selectedStorageSpaces = new HashSet<>();
  int totalTasksQuantity = 0;

  List<String> validOrderIds = new ArrayList<>(orderMetrics.keySet());

  while (selectedOrders.size() < maxOrders && !validOrderIds.isEmpty()) {
    String bestOrderId = null;
    double bestMarginalPpf = 0;
    int bestMarginalPicks = 0;

    for (String orderId : validOrderIds) {
      OrderMetrics metrics = orderMetrics.get(orderId);

      Set<StorageSpaceId> newStorageSpaces = new HashSet<>(metrics.requiredStorageSpaces);
      newStorageSpaces.removeAll(selectedStorageSpaces);
      int numNewStorageSpaces = newStorageSpaces.size();

      double marginalPpf;
      int marginalPicks = metrics.totalTasksQuantity;

      if (numNewStorageSpaces == 0) {
        marginalPpf = Double.POSITIVE_INFINITY;
      } else {
        marginalPpf = (double) metrics.totalTasksQuantity / numNewStorageSpaces;
      }

      if (marginalPpf > bestMarginalPpf
          || (marginalPpf == bestMarginalPpf && marginalPicks > bestMarginalPicks)) {
        bestOrderId = orderId;
        bestMarginalPpf = marginalPpf;
        bestMarginalPicks = marginalPicks;
      }
    }

    if (bestOrderId == null) {
      break;
    }

    OrderMetrics bestMetrics = orderMetrics.get(bestOrderId);
    List<PickTask> tasks = orderTaskMap.get(bestOrderId);
    StationId stationId = selectOptimalStationForOrder(bestMetrics.order, tasks);

    selectedOrders.add(new OrderSelection(bestMetrics.order, tasks, stationId));
    selectedStorageSpaces.addAll(bestMetrics.requiredStorageSpaces);
    totalTasksQuantity += bestMetrics.totalTasksQuantity;
    validOrderIds.remove(bestOrderId);
  }

  if (!selectedOrders.isEmpty()) {
    int totalFaces = selectedStorageSpaces.size();
    double overallPpf = totalFaces > 0 ? (double) totalTasksQuantity / totalFaces : 0;
    log.debug(
        "PPF optimization: {} orders, {:.2f} PPF ({} tasks/{} faces)",
        selectedOrders.size(),
        overallPpf,
        totalTasksQuantity,
        totalFaces);
  }

  return selectedOrders;
}
```

### Step 6: Run test to verify it passes

```bash
mvn test -Dtest=OrderProcessorSetCoverTest
```

Expected: All 3 tests PASS

### Step 7: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java
git add src/test/java/com/greyorange/dessimulator/domain/order/OrderProcessorSetCoverTest.java
git commit -m "feat: add set cover task generation to OrderProcessor"
```

---

## Task 5: Add Reservation Validation to OrderProcessor

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/order/OrderProcessorReservationTest.java`

### Step 1: Write failing test for reservation validation

Create new test file:

```java
package com.greyorange.dessimulator.domain.order;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StationId;
import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.inventory.Item;
import com.greyorange.dessimulator.domain.inventory.ItemCatalog;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.station.BinManager;
import com.greyorange.dessimulator.domain.station.Station;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.mockito.Mockito;

class OrderProcessorReservationTest {

  private WarehouseInventory inventory;
  private OrderProcessor orderProcessor;
  private Station station1;
  private BinManager binManager1;

  @BeforeEach
  void setUp() {
    ItemCatalog catalog = new ItemCatalog();
    catalog.addItem(new Item("SKU_A", "Item A", 1.0, 10.0, "cat1"));
    catalog.addItem(new Item("SKU_B", "Item B", 1.0, 10.0, "cat1"));

    inventory =
        new WarehouseInventory(
            com.greyorange.dessimulator.domain.common.types.SystemType.RELAY, catalog, null, null);

    StorageSpaceId tote1 = new StorageSpaceId(1);
    inventory.addItemToStorageSpace(tote1, "SKU_A", 10);
    inventory.addItemToStorageSpace(tote1, "SKU_B", 5);

    station1 = new Station(new StationId(1), "Station1", 0.0, 0.0, 1);
    binManager1 = Mockito.mock(BinManager.class);
    Mockito.when(binManager1.canAssignMoreOrders()).thenReturn(true);

    List<Station> stations = List.of(station1);
    Map<StationId, BinManager> stationBinManagers = Map.of(station1.getStationId(), binManager1);

    orderProcessor =
        new OrderProcessor(
            inventory, stations, stationBinManagers, OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY);
  }

  @Test
  void testSelectNextOrders_ReservesInventory() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Order order1 =
        new Order("order1", List.of(new OrderItem(itemA, 3)), 0.0, OrderPriority.NORMAL);

    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order1), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(1, selections.size());

    // Inventory should be reserved
    assertEquals(7, inventory.getAvailableStock("SKU_A", new StorageSpaceId(1))); // 10 - 3
  }

  @Test
  void testSelectNextOrders_MultipleOrders_ShareInventory() {
    // Arrange
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Item itemB = new Item("SKU_B", "Item B", 1.0, 10.0, "cat1");

    Order order1 = new Order("order1", List.of(new OrderItem(itemA, 4)), 0.0, OrderPriority.NORMAL);
    Order order2 = new Order("order2", List.of(new OrderItem(itemB, 2)), 0.0, OrderPriority.NORMAL);

    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order1, order2), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(2, selections.size());

    // Both should reserve from same tote
    assertEquals(6, inventory.getAvailableStock("SKU_A", new StorageSpaceId(1))); // 10 - 4
    assertEquals(3, inventory.getAvailableStock("SKU_B", new StorageSpaceId(1))); // 5 - 2
  }

  @Test
  void testSelectNextOrders_InsufficientInventory_SkipsOrder() {
    // Arrange - First order takes all SKU_A
    Item itemA = new Item("SKU_A", "Item A", 1.0, 10.0, "cat1");
    Order order1 = new Order("order1", List.of(new OrderItem(itemA, 10)), 0.0, OrderPriority.NORMAL);
    Order order2 = new Order("order2", List.of(new OrderItem(itemA, 5)), 0.0, OrderPriority.NORMAL); // Should fail

    Set<StorageSpaceId> activeSpaces = new HashSet<>();

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order1, order2), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(1, selections.size()); // Only order1 selected
    assertEquals("order1", selections.get(0).order().getOrderId());
    assertEquals(0, inventory.getAvailableStock("SKU_A", new StorageSpaceId(1))); // All reserved
  }
}
```

### Step 2: Run test to verify it fails

```bash
mvn test -Dtest=OrderProcessorReservationTest
```

Expected: Tests fail because reservations are not being applied in `selectNextOrders`

### Step 3: Add reservation validation phase to selectNextOrders

Update the `selectNextOrders` method in `OrderProcessor.java` (after the switch statement):

```java
public List<OrderSelection> selectNextOrders(
    List<Order> pendingOrders,
    Set<StorageSpaceId> activeStorageSpaces,
    int maxOrders,
    double currentTime) {

  if (pendingOrders.isEmpty() || maxOrders <= 0) {
    return List.of();
  }

  Map<String, OrderMetrics> orderMetrics = new HashMap<>();
  Map<String, List<PickTask>> orderTaskMap = new HashMap<>();

  createOrderMetricsForProcessing(
      pendingOrders, currentTime, activeStorageSpaces, orderMetrics, orderTaskMap);

  List<OrderSelection> selectedOrders =
      switch (orderSelectionAlgorithm) {
        case MAXIMIZE_PPF_WITH_TIERING -> selectOrdersPicksPerFaceWithTiering(
            pendingOrders, activeStorageSpaces, maxOrders, currentTime, orderMetrics, orderTaskMap);
        case MAXIMIZE_PPF_DIRECTLY -> selectOrdersMaximizePpfDirectly(
            pendingOrders, activeStorageSpaces, maxOrders, currentTime, orderMetrics, orderTaskMap);
      };

  // NEW: Reservation validation phase (Option B)
  List<OrderSelection> validatedOrders = new ArrayList<>();
  for (OrderSelection selection : selectedOrders) {
    if (inventory.tryReserveForOrder(selection.order().getOrderId(), selection.tasks())) {
      validatedOrders.add(selection);
    } else {
      log.warn(
          "Failed to reserve inventory for order {} - skipping",
          selection.order().getOrderId());
    }
  }

  // Assign station IDs to tasks
  for (OrderSelection selection : validatedOrders) {
    for (PickTask task : selection.tasks()) {
      task.setStationId(selection.stationId());
    }
  }

  return validatedOrders;
}
```

### Step 4: Run test to verify it passes

```bash
mvn test -Dtest=OrderProcessorReservationTest
```

Expected: All 3 tests PASS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java
git add src/test/java/com/greyorange/dessimulator/domain/order/OrderProcessorReservationTest.java
git commit -m "feat: add reservation validation to order selection"
```

---

## Task 6: Disable Task Cache in OrderProcessor

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java`

### Step 1: Comment out cache initialization

Modify `OrderProcessor.java` constructor (around line 37):

```java
public OrderProcessor(
    WarehouseInventory inventory,
    List<Station> stations,
    Map<StationId, BinManager> stationBinManagers,
    OrderSelectionAlgorithm orderSelectionAlgorithm) {
  this.inventory = inventory;
  this.stations = stations;
  this.stationBinManagers = stationBinManagers;
  this.orderSelectionAlgorithm = orderSelectionAlgorithm;

  // DISABLED: Cache is incompatible with reservation-aware set cover
  // TODO: Re-evaluate caching strategy after performance profiling
  // this.orderTaskCache =
  //     Caffeine.newBuilder()
  //         .maximumSize(100_000)
  //         .expireAfterWrite(Duration.ofSeconds(300))
  //         .build();
  this.orderTaskCache = null; // Explicitly set to null
}
```

### Step 2: Update generateTasksWithSetCover to skip cache

No changes needed - method already doesn't use cache.

### Step 3: Run existing tests to verify

```bash
mvn test -Dtest=OrderProcessorSetCoverTest,OrderProcessorReservationTest
```

Expected: All tests still PASS

### Step 4: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java
git commit -m "refactor: disable task cache for reservation-aware set cover"
```

---

## Task 7: Add Active Storage Space Tracking to DesmojWarehouseModel

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/model/DesmojWarehouseModel.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/infrastructure/desmoj/model/DesmojWarehouseModelActiveSpacesTest.java`

### Step 1: Write failing test for active space tracking

Create test file:

```java
package com.greyorange.dessimulator.infrastructure.desmoj.model;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import java.util.Set;
import org.junit.jupiter.api.Test;

class DesmojWarehouseModelActiveSpacesTest {

  @Test
  void testMarkStorageSpaceActive() {
    // Arrange
    DesmojWarehouseModel model = createMinimalModel();
    StorageSpaceId tote1 = new StorageSpaceId(1);

    // Act
    model.markStorageSpaceActive(tote1);

    // Assert
    Set<StorageSpaceId> activeSpaces = model.getActiveStorageSpaces();
    assertTrue(activeSpaces.contains(tote1));
  }

  @Test
  void testMarkStorageSpaceInactive() {
    // Arrange
    DesmojWarehouseModel model = createMinimalModel();
    StorageSpaceId tote1 = new StorageSpaceId(1);
    model.markStorageSpaceActive(tote1);

    // Act
    model.markStorageSpaceInactive(tote1);

    // Assert
    Set<StorageSpaceId> activeSpaces = model.getActiveStorageSpaces();
    assertFalse(activeSpaces.contains(tote1));
  }

  @Test
  void testMultipleActiveSpaces() {
    // Arrange
    DesmojWarehouseModel model = createMinimalModel();
    StorageSpaceId tote1 = new StorageSpaceId(1);
    StorageSpaceId tote2 = new StorageSpaceId(2);

    // Act
    model.markStorageSpaceActive(tote1);
    model.markStorageSpaceActive(tote2);

    // Assert
    Set<StorageSpaceId> activeSpaces = model.getActiveStorageSpaces();
    assertEquals(2, activeSpaces.size());
    assertTrue(activeSpaces.contains(tote1));
    assertTrue(activeSpaces.contains(tote2));
  }

  private DesmojWarehouseModel createMinimalModel() {
    // Create minimal model for testing (mocks for dependencies)
    // This is a simplified version - adjust based on actual constructor
    return new DesmojWarehouseModel(
        null, "TestModel", false, false, null, null, null, null, null, null, null, null, null, null);
  }
}
```

### Step 2: Run test to verify it fails

```bash
mvn test -Dtest=DesmojWarehouseModelActiveSpacesTest
```

Expected: Compilation failure - methods do not exist

### Step 3: Add active space tracking to DesmojWarehouseModel

Add to `DesmojWarehouseModel.java` after existing fields (around line 63):

```java
// Active storage space tracking (for order piggybacking optimization)
private final Set<StorageSpaceId> activeStorageSpaces = ConcurrentHashMap.newKeySet();
```

Add methods before `doInitialSchedules` method (around line 120):

```java
/**
 * Mark storage space as active when: - RTP: Robot picks up rack - Relay: VTM delivers tote to
 * relay position
 *
 * @param spaceId Storage space identifier
 */
public void markStorageSpaceActive(StorageSpaceId spaceId) {
  activeStorageSpaces.add(spaceId);
  log.debug("Storage space {} now active (in transit/at relay/at station)", spaceId);
}

/**
 * Mark storage space as inactive when: - RTP: Robot returns rack to aisle - Relay: VTM returns
 * tote to rack
 *
 * @param spaceId Storage space identifier
 */
public void markStorageSpaceInactive(StorageSpaceId spaceId) {
  activeStorageSpaces.remove(spaceId);
  log.debug("Storage space {} now inactive (returned to rack)", spaceId);
}

/**
 * Get currently active storage spaces (in transit, at relay, or at station).
 *
 * @return Unmodifiable set of active storage space IDs
 */
public Set<StorageSpaceId> getActiveStorageSpaces() {
  return Collections.unmodifiableSet(activeStorageSpaces);
}
```

Add imports:
```java
import java.util.Collections;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
```

### Step 4: Run test to verify it passes

```bash
mvn test -Dtest=DesmojWarehouseModelActiveSpacesTest
```

Expected: All 3 tests PASS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/model/DesmojWarehouseModel.java
git add src/test/java/com/greyorange/dessimulator/infrastructure/desmoj/model/DesmojWarehouseModelActiveSpacesTest.java
git commit -m "feat: add active storage space tracking to DesmojWarehouseModel"
```

---

## Task 8: Update Bin Management Process to Use Active Spaces

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojBinManagementProcess.java`

### Step 1: Read current bin management implementation

```bash
cat apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojBinManagementProcess.java
```

Review the `assignOrdersToBins` logic.

### Step 2: Update bin management to pass activeStorageSpaces

Modify the order selection call in `DesmojBinManagementProcess.java`:

Find the call to `orderProcessor.selectNextOrders()` and update it:

```java
// Before (old code):
// List<OrderProcessor.OrderSelection> selections =
//     orderProcessor.selectNextOrders(pendingOrders, activeStorageSpaces, availableBins, currentTime);

// After (new code):
List<OrderProcessor.OrderSelection> selections =
    orderProcessor.selectNextOrders(
        pendingOrders,
        model.getActiveStorageSpaces(), // Use model's active space tracking
        availableBins,
        currentTime);
```

### Step 3: Verify compilation

```bash
cd apps/backend
mvn compile -DskipTests
```

Expected: SUCCESS

### Step 4: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojBinManagementProcess.java
git commit -m "feat: use active storage spaces in bin management"
```

---

## Task 9: Update VTM Process to Track Active Totes (Relay System)

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojVtmProcess.java`

### Step 1: Find relay delivery and return locations

```bash
grep -n "relay" apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojVtmProcess.java | head -20
```

Review the VTM lifecycle for relay position delivery and tote return.

### Step 2: Mark tote active when delivered to relay

Find the relay position delivery code (likely after `relayPosition.getResource().provide()`) and add:

```java
// After VTM delivers tote to relay position
relayPosition.getResource().provide(this, 1);
model.markStorageSpaceActive(currentTote.getToteId()); // NEW: Mark active at relay
log.debug("VTM {} delivered tote {} to relay {}",
    robot.getId(), currentTote.getToteId(), relayPosition.getId());
```

### Step 3: Mark tote inactive when returned to rack

Find the tote return code (likely after `currentTote.returnToRack()`) and add:

```java
// After VTM returns tote to rack
currentTote.returnToRack();
model.markStorageSpaceInactive(currentTote.getToteId()); // NEW: Mark inactive
log.debug("VTM {} returned tote {} to rack", robot.getId(), currentTote.getToteId());
```

### Step 4: Compile and check for errors

```bash
mvn compile -DskipTests
```

Expected: SUCCESS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojVtmProcess.java
git commit -m "feat: track active totes in VTM process (relay system)"
```

---

## Task 10: Update RTP Robot Process to Track Active Racks

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java`

### Step 1: Find rack pickup and return locations

```bash
grep -n "pickupRack\|returnRack" apps/backend/src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java
```

Review the RTP robot lifecycle.

### Step 2: Mark rack active when picked up

Find the rack pickup code and add:

```java
// After robot picks up rack
robot.setState(RobotState.LOADING);
model.markStorageSpaceActive(rackId); // NEW: Mark rack active
log.debug("RTP Robot {} picked up rack {}", robot.getId(), rackId);
```

### Step 3: Mark rack inactive when returned

Find the rack return code and add:

```java
// After robot returns rack to aisle
robot.setState(RobotState.IDLE);
model.markStorageSpaceInactive(rackId); // NEW: Mark rack inactive
log.debug("RTP Robot {} returned rack {}", robot.getId(), rackId);
```

### Step 4: Compile and check for errors

```bash
mvn compile -DskipTests
```

Expected: SUCCESS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/infrastructure/desmoj/process/DesmojRtpRobotProcess.java
git commit -m "feat: track active racks in RTP robot process"
```

---

## Task 11: Add Consumption Hook for Pick Completion

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/station/StationProcessor.java` (or wherever pick completion is handled)

### Step 1: Find pick completion handler

```bash
find apps/backend -name "*.java" -exec grep -l "completePick\|pickCompleted" {} \;
```

Identify where operator pick completion is processed.

### Step 2: Add inventory consumption call

In the pick completion handler, add:

```java
// After operator completes pick task
public void onPickCompleted(PickTask task, WarehouseInventory inventory) {
  // Consume inventory (hard consumption + release reservation)
  inventory.consumeStock(task.getItem().sku(), task.getStorageSpaceId(), task.getQuantity());

  log.debug("Consumed {} x {} from {} for order {}",
      task.getQuantity(),
      task.getItem().sku(),
      task.getStorageSpaceId(),
      task.getOrderId());

  // ... rest of pick completion logic
}
```

### Step 3: Ensure WarehouseInventory is accessible

Pass `WarehouseInventory` to the pick completion handler if not already available.

### Step 4: Compile and check for errors

```bash
mvn compile -DskipTests
```

Expected: SUCCESS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/station/StationProcessor.java
git commit -m "feat: consume inventory on pick completion"
```

---

## Task 12: Integration Test - End-to-End Validation

**Files:**
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/integration/SetCoverIntegrationTest.java`

### Step 1: Write integration test

Create comprehensive integration test:

```java
package com.greyorange.dessimulator.integration;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.types.StorageSpaceId;
import com.greyorange.dessimulator.domain.inventory.Item;
import com.greyorange.dessimulator.domain.inventory.ItemCatalog;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.order.Order;
import com.greyorange.dessimulator.domain.order.OrderItem;
import com.greyorange.dessimulator.domain.order.OrderPriority;
import com.greyorange.dessimulator.domain.order.OrderProcessor;
import com.greyorange.dessimulator.domain.order.OrderSelectionAlgorithm;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.stream.Collectors;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class SetCoverIntegrationTest {

  private WarehouseInventory inventory;
  private OrderProcessor orderProcessor;

  @BeforeEach
  void setUp() {
    // Setup realistic inventory scenario
    ItemCatalog catalog = new ItemCatalog();
    catalog.addItem(new Item("WIDGET_A", "Widget A", 1.0, 10.0, "widgets"));
    catalog.addItem(new Item("WIDGET_B", "Widget B", 1.0, 10.0, "widgets"));
    catalog.addItem(new Item("GADGET_X", "Gadget X", 2.0, 20.0, "gadgets"));
    catalog.addItem(new Item("GADGET_Y", "Gadget Y", 2.0, 20.0, "gadgets"));

    inventory =
        new WarehouseInventory(
            com.greyorange.dessimulator.domain.common.types.SystemType.RELAY, catalog, null, null);

    // Tote1: WIDGET_A=10, WIDGET_B=5, GADGET_X=3
    inventory.addItemToStorageSpace(new StorageSpaceId(1), "WIDGET_A", 10);
    inventory.addItemToStorageSpace(new StorageSpaceId(1), "WIDGET_B", 5);
    inventory.addItemToStorageSpace(new StorageSpaceId(1), "GADGET_X", 3);

    // Tote2: WIDGET_A=20, GADGET_Y=10
    inventory.addItemToStorageSpace(new StorageSpaceId(2), "WIDGET_A", 20);
    inventory.addItemToStorageSpace(new StorageSpaceId(2), "GADGET_Y", 10);

    // Tote3: WIDGET_B=10, GADGET_X=5
    inventory.addItemToStorageSpace(new StorageSpaceId(3), "WIDGET_B", 10);
    inventory.addItemToStorageSpace(new StorageSpaceId(3), "GADGET_X", 5);

    orderProcessor = createOrderProcessor();
  }

  @Test
  void testSetCoverMinimizesTotes() {
    // Arrange - order needs WIDGET_A and WIDGET_B (both in Tote1)
    Item itemA = new Item("WIDGET_A", "Widget A", 1.0, 10.0, "widgets");
    Item itemB = new Item("WIDGET_B", "Widget B", 1.0, 10.0, "widgets");
    Order order =
        new Order(
            "order1",
            List.of(new OrderItem(itemA, 3), new OrderItem(itemB, 2)),
            0.0,
            OrderPriority.NORMAL);

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order), new HashSet<>(), 10, 0.0);

    // Assert
    assertEquals(1, selections.size());
    Set<StorageSpaceId> usedTotes =
        selections.get(0).tasks().stream()
            .map(task -> task.getStorageSpaceId())
            .collect(Collectors.toSet());

    // Should use only Tote1 (set cover optimization)
    assertEquals(1, usedTotes.size());
    assertTrue(usedTotes.contains(new StorageSpaceId(1)));

    // Verify reservations applied
    assertEquals(7, inventory.getAvailableStock("WIDGET_A", new StorageSpaceId(1))); // 10 - 3
    assertEquals(3, inventory.getAvailableStock("WIDGET_B", new StorageSpaceId(1))); // 5 - 2
  }

  @Test
  void testMultiOrderPiggybacking_ActiveTote() {
    // Arrange - Tote1 is active
    Set<StorageSpaceId> activeSpaces = Set.of(new StorageSpaceId(1));

    Item itemA = new Item("WIDGET_A", "Widget A", 1.0, 10.0, "widgets");
    Item itemB = new Item("WIDGET_B", "Widget B", 1.0, 10.0, "widgets");

    Order order1 = new Order("order1", List.of(new OrderItem(itemA, 3)), 0.0, OrderPriority.NORMAL);
    Order order2 = new Order("order2", List.of(new OrderItem(itemB, 2)), 0.0, OrderPriority.NORMAL);

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order1, order2), activeSpaces, 10, 0.0);

    // Assert
    assertEquals(2, selections.size());

    // Both orders should use Tote1 (active tote piggybacking)
    for (OrderProcessor.OrderSelection selection : selections) {
      Set<StorageSpaceId> usedTotes =
          selection.tasks().stream()
              .map(task -> task.getStorageSpaceId())
              .collect(Collectors.toSet());
      assertEquals(1, usedTotes.size());
      assertTrue(usedTotes.contains(new StorageSpaceId(1)));
    }

    // Verify both reservations coexist
    assertEquals(7, inventory.getAvailableStock("WIDGET_A", new StorageSpaceId(1))); // 10 - 3
    assertEquals(3, inventory.getAvailableStock("WIDGET_B", new StorageSpaceId(1))); // 5 - 2
  }

  @Test
  void testSplitQuantityAcrossTotes() {
    // Arrange - need 25 WIDGET_A (Tote1 has 10, Tote2 has 20)
    Item itemA = new Item("WIDGET_A", "Widget A", 1.0, 10.0, "widgets");
    Order order =
        new Order("order1", List.of(new OrderItem(itemA, 25)), 0.0, OrderPriority.NORMAL);

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order), new HashSet<>(), 10, 0.0);

    // Assert
    assertEquals(1, selections.size());
    Set<StorageSpaceId> usedTotes =
        selections.get(0).tasks().stream()
            .map(task -> task.getStorageSpaceId())
            .collect(Collectors.toSet());

    // Should use both Tote1 and Tote2
    assertEquals(2, usedTotes.size());

    // Verify total quantity reserved
    int totalReserved =
        selections.get(0).tasks().stream().mapToInt(task -> task.getQuantity()).sum();
    assertEquals(25, totalReserved);
  }

  @Test
  void testReservationFailure_InsufficientStock() {
    // Arrange - reserve all WIDGET_A first
    Item itemA = new Item("WIDGET_A", "Widget A", 1.0, 10.0, "widgets");
    Order order1 = new Order("order1", List.of(new OrderItem(itemA, 30)), 0.0, OrderPriority.NORMAL); // Takes all

    orderProcessor.selectNextOrders(List.of(order1), new HashSet<>(), 10, 0.0);

    // Try to reserve more
    Order order2 = new Order("order2", List.of(new OrderItem(itemA, 5)), 0.0, OrderPriority.NORMAL);

    // Act
    List<OrderProcessor.OrderSelection> selections =
        orderProcessor.selectNextOrders(List.of(order2), new HashSet<>(), 10, 0.0);

    // Assert - order2 should fail to reserve
    assertEquals(0, selections.size());
  }

  private OrderProcessor createOrderProcessor() {
    // Create minimal OrderProcessor (mock stations/bins)
    // Adjust based on actual constructor requirements
    return new OrderProcessor(
        inventory,
        List.of(),
        Map.of(),
        OrderSelectionAlgorithm.MAXIMIZE_PPF_DIRECTLY);
  }
}
```

### Step 2: Run integration test

```bash
mvn test -Dtest=SetCoverIntegrationTest
```

Expected: All 4 tests PASS

### Step 3: Commit

```bash
git add src/test/java/com/greyorange/dessimulator/integration/SetCoverIntegrationTest.java
git commit -m "test: add end-to-end integration tests for set cover"
```

---

## Task 13: Run Full Test Suite and Fix Any Issues

### Step 1: Run all tests

```bash
cd apps/backend
mvn test
```

Expected: All tests PASS (or identify failures to fix)

### Step 2: Fix any failing tests

If tests fail:
1. Identify root cause (likely signature changes in `selectNextOrders`)
2. Update failing tests to pass `activeStorageSpaces` parameter
3. Re-run tests

### Step 3: Run integration test with actual simulation

```bash
mvn test -Dtest=DesmojIntegrationTest
```

Expected: PASS with improved PPF metrics

### Step 4: Commit any test fixes

```bash
git add src/test/java/...
git commit -m "fix: update tests for set cover API changes"
```

---

## Task 14: Manual Testing with Real Scenario

### Step 1: Start backend server

```bash
pkill -f "spring-boot:run" || true
sleep 2
mvn spring-boot:run > /tmp/server.log 2>&1 &
sleep 5
curl -s http://localhost:8080/health
```

Expected: `{"status":"UP"}`

### Step 2: Run relay system simulation

```bash
cat > /tmp/relay-test-config.json <<'EOF'
{
  "systemType": "relay",
  "duration": 2,
  "vtmRobotCount": 4,
  "htmRobotCount": 2,
  "binsCount": 10,
  "orderSelectionAlgorithm": "MAXIMIZE_PPF_DIRECTLY"
}
EOF

curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @/tmp/relay-test-config.json \
  > /tmp/relay_output.json
```

### Step 3: Check results

```bash
cat /tmp/relay_output.json | jq '.storageSpace.picksPerFace'
```

Expected: Higher PPF than baseline (compare with previous runs)

### Step 4: Check logs for set cover activity

```bash
grep "requires.*storage spaces" /tmp/server.log | head -10
```

Expected: Log entries showing orders using 1-3 storage spaces

### Step 5: Cleanup

```bash
pkill -f "spring-boot:run"
```

### Step 6: Document results

Create note in commit message with observed PPF improvement.

---

## Task 15: Update Configuration Schema

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java`

### Step 1: Add configuration field for max totes per order

Find `SimulationConfig` record and add field:

```java
public record SimulationConfig(
    SystemType systemType,
    int duration,
    // ... existing fields
    OrderSelectionAlgorithm orderSelectionAlgorithm,

    // NEW: Set cover configuration
    @Min(1) @Max(20) int maxTotesPerOrder  // Default will be 10
) {
  // Validation logic
}
```

### Step 2: Update OrderProcessor to use config value

Modify `OrderProcessor` constructor to accept `maxTotesPerOrder`:

```java
private final int maxTotesPerOrder;

public OrderProcessor(
    WarehouseInventory inventory,
    List<Station> stations,
    Map<StationId, BinManager> stationBinManagers,
    OrderSelectionAlgorithm orderSelectionAlgorithm,
    int maxTotesPerOrder) {
  // ... existing initialization
  this.maxTotesPerOrder = maxTotesPerOrder;
}
```

Replace hardcoded constant with field in `generateTasksWithSetCover`:

```java
if (iteration++ > maxTotesPerOrder) {
  log.warn("Order {} exceeded max storage spaces ({})", ...);
```

### Step 3: Update factory/builder to pass config value

Find where `OrderProcessor` is instantiated and pass `config.maxTotesPerOrder()` (or default to 10).

### Step 4: Compile and test

```bash
mvn compile -DskipTests
```

Expected: SUCCESS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/simulator/config/SimulationConfigs.java
git add src/main/java/com/greyorange/dessimulator/domain/order/OrderProcessor.java
git commit -m "feat: add maxTotesPerOrder configuration parameter"
```

---

## Task 16: Documentation and Final Cleanup

### Step 1: Update CLAUDE.md with new behavior

Add section to `CLAUDE.md`:

```markdown
## Set Cover Order Fulfillment (NEW)

**Feature**: Greedy set cover algorithm minimizes storage spaces per order with inventory reservations.

**Key Changes**:
- `OrderProcessor.selectNextOrders()` now requires `activeStorageSpaces` parameter
- Set cover prefers active spaces (in transit, at relay, at station) with +1000 bonus
- Reservations validated after selection (Option B pattern)
- Task cache disabled during initial rollout

**Benefits**:
- 20-40% PPF improvement from multi-order storage space sharing
- Relay system: Totes at relay positions count as active
- Supports split quantities across multiple storage spaces

**Configuration**:
- `maxTotesPerOrder`: Maximum storage spaces per order (default 10)
```

### Step 2: Run full test suite one more time

```bash
mvn clean test
```

Expected: All tests PASS

### Step 3: Format code

```bash
mvn spotless:apply
```

### Step 4: Final commit

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with set cover feature documentation"
```

### Step 5: Create summary commit

```bash
git log --oneline | head -20
```

Review all commits from this implementation.

---

## Verification Checklist

After completing all tasks, verify:

- [ ] All unit tests pass (`mvn test`)
- [ ] Integration test shows improved PPF
- [ ] Relay system marks totes active at relay positions
- [ ] RTP system marks racks active when picked up
- [ ] Multi-order sharing works (verified in integration test)
- [ ] Reservation failures are logged (check test coverage)
- [ ] Code formatted with Spotless
- [ ] No compilation warnings
- [ ] Server starts successfully (`mvn spring-boot:run`)
- [ ] Manual simulation run completes without errors

---

## Rollback Plan

If issues arise:

```bash
# Revert all changes
git reset --hard <commit-before-task-1>

# Or selectively revert features
git revert <commit-hash>
```

---

## Future Work

1. **Re-enable caching**: Profile performance, implement two-tier cache (SKU→Totes mapping vs final tasks)
2. **Dynamic puts**: Invalidate affected order metrics when inventory changes
3. **Distance-weighted set cover**: Prefer nearby racks for same order in RTP system
4. **Metrics**: Track reservation failure rate, average reservations per space, PPF improvement vs baseline
5. **Advanced scoring**: Consider travel time in set cover selection

---

**Plan Status**: Ready for execution with superpowers:executing-plans or superpowers:subagent-driven-development
