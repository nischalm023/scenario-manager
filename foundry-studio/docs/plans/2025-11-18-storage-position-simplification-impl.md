# Storage Position Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove redundant `storageCoordinate` from Rack and implement on-demand position computation with reservations in StoragePositionManager.

**Architecture:** Rack keeps only `currentCoordinate`. StoragePositionManager computes optimal storage positions on demand and tracks reservations. Totes use two-level reservations: soft (zone) and hard (rack slot). Zone filtering prevents relay deadlock.

**Tech Stack:** Java 21, Spring Boot, Desmo-J, JUnit 5, ConcurrentHashMap for thread safety

---

## Task 1: Add Rack Position Reservation to StoragePositionManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/StoragePositionManagerReservationTest.java`

**Step 1: Write the failing test for rack reservation**

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class StoragePositionManagerReservationTest {

  private StoragePositionManager manager;
  private WarehouseMap mockMap;
  private GridPopularityManager mockPopularityManager;
  private WarehouseInventory mockInventory;

  @BeforeEach
  void setUp() {
    mockMap = mock(WarehouseMap.class);
    mockPopularityManager = mock(GridPopularityManager.class);
    mockInventory = mock(WarehouseInventory.class);

    // Setup rack positions
    Set<GridCoordinate> rackPositions = new HashSet<>();
    rackPositions.add(new GridCoordinate(0, 0));
    rackPositions.add(new GridCoordinate(1, 0));
    rackPositions.add(new GridCoordinate(2, 0));
    when(mockMap.getRackPositions()).thenReturn(rackPositions);

    // All positions same distance for simplicity
    when(mockPopularityManager.getAverageDistance(any())).thenReturn(1.0);
    when(mockInventory.calculatePopularityScore(any())).thenReturn(0.5);

    manager = new StoragePositionManager(mockMap, mockPopularityManager, mockInventory);
  }

  @Test
  void reservePositionForRack_reservesOptimalPosition() {
    Rack rack = new Rack(new RackId(1), new GridCoordinate(0, 0), 12);

    GridCoordinate reserved = manager.reservePositionForRack(rack);

    assertNotNull(reserved);
    assertTrue(manager.getReservedPosition(rack.getId()).isPresent());
    assertEquals(reserved, manager.getReservedPosition(rack.getId()).get());
  }

  @Test
  void reservePositionForRack_excludesAlreadyReservedPositions() {
    Rack rack1 = new Rack(new RackId(1), new GridCoordinate(0, 0), 12);
    Rack rack2 = new Rack(new RackId(2), new GridCoordinate(1, 0), 12);

    GridCoordinate reserved1 = manager.reservePositionForRack(rack1);
    GridCoordinate reserved2 = manager.reservePositionForRack(rack2);

    assertNotEquals(reserved1, reserved2);
  }

  @Test
  void releaseReservation_makesPositionAvailable() {
    Rack rack = new Rack(new RackId(1), new GridCoordinate(0, 0), 12);

    manager.reservePositionForRack(rack);
    manager.releaseReservation(rack.getId());

    assertFalse(manager.getReservedPosition(rack.getId()).isPresent());
  }

  @Test
  void getReservedPosition_returnsEmptyForUnreservedRack() {
    RackId unknownRackId = new RackId(999);

    assertFalse(manager.getReservedPosition(unknownRackId).isPresent());
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=StoragePositionManagerReservationTest -q`

Expected: FAIL with compilation errors - methods don't exist

**Step 3: Write minimal implementation**

Add to `StoragePositionManager.java` after line 25 (after existing fields):

```java
  // Rack position reservations
  private final Map<GridCoordinate, RackId> reservedPositions = new ConcurrentHashMap<>();
  private final Map<RackId, GridCoordinate> rackReservations = new ConcurrentHashMap<>();
```

Add import at top:
```java
import java.util.concurrent.ConcurrentHashMap;
```

Add methods before the closing brace:

```java
  /**
   * Reserve optimal position for rack returning to storage.
   *
   * <p>Called at station completion (before robot assignment).
   *
   * @param rack The rack to reserve position for
   * @return The reserved grid coordinate
   */
  public GridCoordinate reservePositionForRack(Rack rack) {
    // Get occupied = physically occupied + reserved
    Set<GridCoordinate> unavailable = new HashSet<>();
    for (GridCoordinate pos : map.getRackPositions()) {
      // Check if position is physically occupied (has a rack at rest)
      // For now, just use reserved positions
    }
    unavailable.addAll(reservedPositions.keySet());

    GridCoordinate optimal = getOptimalPositionForRack(rack, unavailable);

    reservedPositions.put(optimal, rack.getId());
    rackReservations.put(rack.getId(), optimal);

    return optimal;
  }

  /**
   * Release reservation when rack arrives at storage position.
   *
   * @param rackId The rack ID to release reservation for
   */
  public void releaseReservation(RackId rackId) {
    GridCoordinate position = rackReservations.remove(rackId);
    if (position != null) {
      reservedPositions.remove(position);
    }
  }

  /**
   * Get reserved position for a rack (for robot navigation).
   *
   * @param rackId The rack ID to look up
   * @return Optional containing the reserved position, or empty if not reserved
   */
  public Optional<GridCoordinate> getReservedPosition(RackId rackId) {
    return Optional.ofNullable(rackReservations.get(rackId));
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=StoragePositionManagerReservationTest -q`

Expected: PASS (4 tests)

**Step 5: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java
git add src/test/java/com/greyorange/dessimulator/domain/put/StoragePositionManagerReservationTest.java
git commit -m "feat: add rack position reservation to StoragePositionManager"
```

---

## Task 2: Add Tote Zone Field and Soft Reservation Tracking

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/StoragePositionManagerToteSoftReservationTest.java`

**Step 1: Write the failing test for soft zone reservation**

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneCapacity;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class StoragePositionManagerToteSoftReservationTest {

  private StoragePositionManager manager;
  private ZoneManager mockZoneManager;

  @BeforeEach
  void setUp() {
    WarehouseMap mockMap = mock(WarehouseMap.class);
    GridPopularityManager mockPopularityManager = mock(GridPopularityManager.class);
    WarehouseInventory mockInventory = mock(WarehouseInventory.class);
    mockZoneManager = mock(ZoneManager.class);

    // Setup zones with capacity
    Map<String, Zone> zones = new HashMap<>();
    Zone zone1 = mock(Zone.class);
    Zone zone2 = mock(Zone.class);
    when(zone1.getZoneId()).thenReturn("zone1");
    when(zone2.getZoneId()).thenReturn("zone2");
    when(zone1.getCapacity()).thenReturn(new ZoneCapacity("zone1", 10));
    when(zone2.getCapacity()).thenReturn(new ZoneCapacity("zone2", 10));
    zones.put("zone1", zone1);
    zones.put("zone2", zone2);
    when(mockZoneManager.getZones()).thenReturn(zones);

    manager = new StoragePositionManager(mockMap, mockPopularityManager, mockInventory);
    manager.setZoneManager(mockZoneManager);
  }

  @Test
  void reserveSoftZone_assignsZoneToTote() {
    Tote tote = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));

    String zoneId = manager.reserveSoftZone(tote);

    assertNotNull(zoneId);
    assertEquals(zoneId, tote.getTargetZoneId().orElse(null));
  }

  @Test
  void reserveSoftZone_reducesEffectiveCapacity() {
    Tote tote1 = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));
    Tote tote2 = new Tote(new ToteId(2), 1, new GridCoordinate(0, 0));

    String zone1 = manager.reserveSoftZone(tote1);
    String zone2 = manager.reserveSoftZone(tote2);

    // Both totes should be tracked
    int softCount = manager.getSoftReservationCount(zone1);
    if (zone1.equals(zone2)) {
      assertEquals(2, softCount);
    } else {
      assertEquals(1, softCount);
      assertEquals(1, manager.getSoftReservationCount(zone2));
    }
  }

  @Test
  void releaseSoftReservation_freesCapacity() {
    Tote tote = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));

    String zoneId = manager.reserveSoftZone(tote);
    int countBefore = manager.getSoftReservationCount(zoneId);

    manager.releaseSoftReservation(tote.getId());

    assertEquals(countBefore - 1, manager.getSoftReservationCount(zoneId));
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=StoragePositionManagerToteSoftReservationTest -q`

Expected: FAIL with compilation errors

**Step 3: Add targetZoneId field to Tote.java**

Add after line 31 (after existing fields):

```java
  private String targetZoneId;
```

Add methods before closing brace:

```java
  /**
   * Set the target zone for this tote's return journey.
   *
   * @param zoneId The zone ID to target
   */
  public void setTargetZoneId(String zoneId) {
    this.targetZoneId = zoneId;
  }

  /**
   * Get the target zone for this tote's return journey.
   *
   * @return Optional containing zone ID, or empty if not set
   */
  public Optional<String> getTargetZoneId() {
    return Optional.ofNullable(targetZoneId);
  }
```

**Step 4: Add soft reservation fields and methods to StoragePositionManager.java**

Add fields after the rack reservation fields:

```java
  // Tote soft reservations (zone level)
  private final Map<String, Set<ToteId>> softZoneReservations = new ConcurrentHashMap<>();
  private final Map<ToteId, String> toteSoftReservations = new ConcurrentHashMap<>();

  private static final int MIN_FREE_RELAY_POSITIONS = 3;

  private ZoneManager zoneManager;
```

Add imports:
```java
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
```

Add setter for ZoneManager:
```java
  /**
   * Set the zone manager for tote zone reservations.
   *
   * @param zoneManager The zone manager
   */
  public void setZoneManager(ZoneManager zoneManager) {
    this.zoneManager = zoneManager;
  }
```

Add soft reservation methods:

```java
  /**
   * Get available relay capacity for a zone, accounting for soft reservations.
   *
   * @param zone The zone to check
   * @return Effective free relay count
   */
  private int getEffectiveFreeRelayCount(Zone zone) {
    int physicalFree = zone.getCapacity().available();
    int softReserved = softZoneReservations.getOrDefault(zone.getZoneId(), Set.of()).size();
    return physicalFree - softReserved;
  }

  /**
   * Soft-reserve zone when tote is assigned tasks (VTM will deliver to relay).
   *
   * @param tote The tote to reserve zone for
   * @return The assigned zone ID
   */
  public String reserveSoftZone(Tote tote) {
    if (zoneManager == null) {
      throw new IllegalStateException("ZoneManager not set");
    }

    // Filter zones by relay availability
    List<Zone> eligibleZones = zoneManager.getZones().values().stream()
        .filter(z -> getEffectiveFreeRelayCount(z) >= MIN_FREE_RELAY_POSITIONS)
        .toList();

    // Fallback if all zones below threshold
    if (eligibleZones.isEmpty()) {
      log.warn("All zones below relay threshold ({}), using best available", MIN_FREE_RELAY_POSITIONS);
      eligibleZones = selectBestAvailableZones();
    }

    // Select optimal zone (for now, pick first with most capacity)
    String targetZoneId = eligibleZones.stream()
        .max(Comparator.comparingInt(this::getEffectiveFreeRelayCount))
        .map(Zone::getZoneId)
        .orElse(zoneManager.getZones().keySet().iterator().next());

    // Track soft reservation
    softZoneReservations.computeIfAbsent(targetZoneId, k -> ConcurrentHashMap.newKeySet())
        .add(tote.getId());
    toteSoftReservations.put(tote.getId(), targetZoneId);

    // Set on tote
    tote.setTargetZoneId(targetZoneId);

    return targetZoneId;
  }

  /**
   * Select zones with best available capacity when all below threshold.
   *
   * @return List of zones sorted by available capacity
   */
  private List<Zone> selectBestAvailableZones() {
    return zoneManager.getZones().values().stream()
        .sorted(Comparator.comparingInt(this::getEffectiveFreeRelayCount).reversed())
        .toList();
  }

  /**
   * Release soft reservation when tote arrives at relay position.
   *
   * @param toteId The tote ID to release
   */
  public void releaseSoftReservation(ToteId toteId) {
    String zoneId = toteSoftReservations.remove(toteId);
    if (zoneId != null) {
      Set<ToteId> zoneSet = softZoneReservations.get(zoneId);
      if (zoneSet != null) {
        zoneSet.remove(toteId);
      }
    }
  }

  /**
   * Get count of soft reservations for a zone.
   *
   * @param zoneId The zone ID
   * @return Count of totes soft-reserved to this zone
   */
  public int getSoftReservationCount(String zoneId) {
    return softZoneReservations.getOrDefault(zoneId, Set.of()).size();
  }
```

**Step 5: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=StoragePositionManagerToteSoftReservationTest -q`

Expected: PASS (3 tests)

**Step 6: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java
git add src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java
git add src/test/java/com/greyorange/dessimulator/domain/put/StoragePositionManagerToteSoftReservationTest.java
git commit -m "feat: add tote zone field and soft reservation tracking"
```

---

## Task 3: Add Tote Hard Slot Reservation

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/put/StoragePositionManagerToteHardReservationTest.java`

**Step 1: Write the failing test for hard slot reservation**

```java
package com.greyorange.dessimulator.domain.put;

import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.RackId;
import com.greyorange.dessimulator.domain.common.types.ToteId;
import com.greyorange.dessimulator.domain.inventory.Rack;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.domain.inventory.WarehouseInventory;
import com.greyorange.dessimulator.domain.map.WarehouseMap;
import com.greyorange.dessimulator.domain.robot.Zone;
import com.greyorange.dessimulator.domain.robot.ZoneCapacity;
import com.greyorange.dessimulator.domain.robot.ZoneManager;
import java.util.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class StoragePositionManagerToteHardReservationTest {

  private StoragePositionManager manager;
  private ZoneManager mockZoneManager;
  private List<Rack> testRacks;

  @BeforeEach
  void setUp() {
    WarehouseMap mockMap = mock(WarehouseMap.class);
    GridPopularityManager mockPopularityManager = mock(GridPopularityManager.class);
    WarehouseInventory mockInventory = mock(WarehouseInventory.class);
    mockZoneManager = mock(ZoneManager.class);

    // Setup zone
    Map<String, Zone> zones = new HashMap<>();
    Zone zone1 = mock(Zone.class);
    when(zone1.getZoneId()).thenReturn("zone1");
    when(zone1.getCapacity()).thenReturn(new ZoneCapacity("zone1", 10));
    zones.put("zone1", zone1);
    when(mockZoneManager.getZones()).thenReturn(zones);

    // Setup racks in zone
    testRacks = new ArrayList<>();
    Rack rack1 = new Rack(new RackId(1), new GridCoordinate(0, 0), 12);
    Rack rack2 = new Rack(new RackId(2), new GridCoordinate(1, 0), 12);
    testRacks.add(rack1);
    testRacks.add(rack2);

    manager = new StoragePositionManager(mockMap, mockPopularityManager, mockInventory);
    manager.setZoneManager(mockZoneManager);
  }

  @Test
  void reserveHardSlot_reservesRackAndSlot() {
    Tote tote = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));
    tote.setTargetZoneId("zone1");

    StoragePositionManager.ToteSlotReservation reservation =
        manager.reserveHardSlot(tote, testRacks);

    assertNotNull(reservation);
    assertNotNull(reservation.rackId());
    assertTrue(reservation.zIndex() >= 0);
  }

  @Test
  void reserveHardSlot_excludesAlreadyReservedSlots() {
    Tote tote1 = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));
    Tote tote2 = new Tote(new ToteId(2), 1, new GridCoordinate(0, 0));
    tote1.setTargetZoneId("zone1");
    tote2.setTargetZoneId("zone1");

    StoragePositionManager.ToteSlotReservation res1 = manager.reserveHardSlot(tote1, testRacks);
    StoragePositionManager.ToteSlotReservation res2 = manager.reserveHardSlot(tote2, testRacks);

    // Either different racks or different slots
    assertFalse(res1.rackId().equals(res2.rackId()) && res1.zIndex() == res2.zIndex());
  }

  @Test
  void releaseHardReservation_freesSlot() {
    Tote tote = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));
    tote.setTargetZoneId("zone1");

    StoragePositionManager.ToteSlotReservation reservation =
        manager.reserveHardSlot(tote, testRacks);

    manager.releaseHardReservation(tote.getId());

    // Should be able to reserve same slot again
    Tote tote2 = new Tote(new ToteId(2), 1, new GridCoordinate(0, 0));
    tote2.setTargetZoneId("zone1");
    StoragePositionManager.ToteSlotReservation res2 = manager.reserveHardSlot(tote2, testRacks);

    assertEquals(reservation.rackId(), res2.rackId());
    assertEquals(reservation.zIndex(), res2.zIndex());
  }

  @Test
  void getToteReservation_returnsReservation() {
    Tote tote = new Tote(new ToteId(1), 1, new GridCoordinate(0, 0));
    tote.setTargetZoneId("zone1");

    manager.reserveHardSlot(tote, testRacks);

    assertTrue(manager.getToteReservation(tote.getId()).isPresent());
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=StoragePositionManagerToteHardReservationTest -q`

Expected: FAIL with compilation errors

**Step 3: Add hard reservation fields and methods to StoragePositionManager.java**

Add the record and fields after soft reservation fields:

```java
  /** Reservation for a tote slot in a specific rack. */
  public record ToteSlotReservation(RackId rackId, int zIndex) {}

  // Tote hard reservations (rack slot level)
  private final Map<RackId, Set<Integer>> reservedToteSlots = new ConcurrentHashMap<>();
  private final Map<ToteId, ToteSlotReservation> toteReservations = new ConcurrentHashMap<>();
```

Add methods:

```java
  /**
   * Hard-reserve specific rack slot when VTM starts return journey.
   *
   * @param tote The tote to reserve slot for
   * @param availableRacks List of racks to consider
   * @return The reservation with rack ID and z-index
   */
  public ToteSlotReservation reserveHardSlot(Tote tote, List<Rack> availableRacks) {
    // Find rack with free slot
    Rack targetRack = null;
    int targetZIndex = -1;

    for (Rack rack : availableRacks) {
      Set<Integer> reservedSlots = reservedToteSlots.getOrDefault(rack.getId(), Set.of());

      // Find first free slot in this rack
      for (int z = 0; z < rack.getMaxToteCapacity(); z++) {
        if (!reservedSlots.contains(z) && rack.getToteAtPosition(z) == null) {
          targetRack = rack;
          targetZIndex = z;
          break;
        }
      }
      if (targetRack != null) break;
    }

    if (targetRack == null) {
      throw new IllegalStateException("No free rack slots available for tote " + tote.getId());
    }

    // Reserve the slot
    reservedToteSlots.computeIfAbsent(targetRack.getId(), k -> ConcurrentHashMap.newKeySet())
        .add(targetZIndex);
    ToteSlotReservation reservation = new ToteSlotReservation(targetRack.getId(), targetZIndex);
    toteReservations.put(tote.getId(), reservation);

    return reservation;
  }

  /**
   * Release hard reservation when tote arrives at rack slot.
   *
   * @param toteId The tote ID to release
   */
  public void releaseHardReservation(ToteId toteId) {
    ToteSlotReservation reservation = toteReservations.remove(toteId);
    if (reservation != null) {
      Set<Integer> slots = reservedToteSlots.get(reservation.rackId());
      if (slots != null) {
        slots.remove(reservation.zIndex());
      }
    }
  }

  /**
   * Get the hard reservation for a tote.
   *
   * @param toteId The tote ID
   * @return Optional containing the reservation
   */
  public Optional<ToteSlotReservation> getToteReservation(ToteId toteId) {
    return Optional.ofNullable(toteReservations.get(toteId));
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=StoragePositionManagerToteHardReservationTest -q`

Expected: PASS (4 tests)

**Step 5: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java
git add src/test/java/com/greyorange/dessimulator/domain/put/StoragePositionManagerToteHardReservationTest.java
git commit -m "feat: add tote hard slot reservation to StoragePositionManager"
```

---

## Task 4: Remove storageCoordinate from Rack

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Rack.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/inventory/RackTest.java` (if exists, or create)

**Step 1: Write test for updated Rack behavior**

```java
package com.greyorange.dessimulator.domain.inventory;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.common.types.RackId;
import org.junit.jupiter.api.Test;

class RackStorageSimplificationTest {

  @Test
  void rack_hasOnlyCurrentCoordinate() {
    GridCoordinate initialPos = new GridCoordinate(5, 10);
    Rack rack = new Rack(new RackId(1), initialPos, 12);

    assertEquals(initialPos, rack.getCurrentCoordinate());
    assertEquals(RackState.IN_STORAGE, rack.getState());
  }

  @Test
  void moveToStorage_updatesCurrentCoordinateAndState() {
    Rack rack = new Rack(new RackId(1), new GridCoordinate(0, 0), 12);
    rack.setBeingTransported(1);
    rack.moveToStation(new GridCoordinate(10, 10));

    GridCoordinate newStorage = new GridCoordinate(3, 3);
    rack.moveToStorage(newStorage);

    assertEquals(newStorage, rack.getCurrentCoordinate());
    assertEquals(RackState.IN_STORAGE, rack.getState());
    assertNull(rack.getAssignedRobotId());
  }

  @Test
  void moveToStation_updatesCurrentCoordinate() {
    Rack rack = new Rack(new RackId(1), new GridCoordinate(0, 0), 12);
    rack.setBeingTransported(1);

    GridCoordinate stationPos = new GridCoordinate(15, 15);
    rack.moveToStation(stationPos);

    assertEquals(stationPos, rack.getCurrentCoordinate());
    assertEquals(RackState.AT_STATION, rack.getState());
  }
}
```

**Step 2: Run test to verify current behavior**

Run: `cd apps/backend && mvn test -Dtest=RackStorageSimplificationTest -q`

Expected: May pass or fail depending on current Rack implementation

**Step 3: Remove storageCoordinate from Rack.java**

1. Delete line 36: `@Setter private GridCoordinate storageCoordinate;`

2. Update constructor (around line 43) - remove storageCoordinate initialization:
```java
  public Rack(RackId id, GridCoordinate initialPosition, int maxToteCapacity) {
    this.id = id;
    this.maxCapacity = DEFAULT_CAPACITY;
    this.maxToteCapacity = maxToteCapacity;
    this.currentCoordinate = initialPosition;
    this.state = RackState.IN_STORAGE;
  }
```

3. Update `moveToStorage()` method (around line 96-104):
```java
  /**
   * Move rack to storage and update state.
   *
   * @param storagePosition The position to store at (from StoragePositionManager)
   */
  public void moveToStorage(GridCoordinate storagePosition) {
    if (this.state == RackState.IN_STORAGE) {
      throw new IllegalStateException("Rack is already in storage");
    }

    this.state = RackState.IN_STORAGE;
    this.currentCoordinate = storagePosition;
    this.assignedRobotId = null;
  }
```

4. Delete the `getStorageCoordinate()` method if it exists as an explicit getter (Lombok @Getter generates it from field)

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RackStorageSimplificationTest -q`

Expected: PASS (3 tests)

**Step 5: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/inventory/Rack.java
git add src/test/java/com/greyorange/dessimulator/domain/inventory/RackStorageSimplificationTest.java
git commit -m "refactor: remove storageCoordinate from Rack, keep only currentCoordinate"
```

---

## Task 5: Update StoragePositionManager to Remove storageCoordinate References

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java`

**Step 1: Update getOptimalPositionForRack fallback**

Find line 49 (in `getOptimalPositionForRack`):
```java
return rack.getStorageCoordinate();
```

Replace with:
```java
return rack.getCurrentCoordinate();
```

**Step 2: Update assignInitialPositions**

Find lines 89-90 (in `assignInitialPositions`):
```java
assignments.put(rack.getId(), position);
rack.setStorageCoordinate(position);
rack.setCurrentCoordinate(position);
```

Replace with:
```java
assignments.put(rack.getId(), position);
rack.setCurrentCoordinate(position);
```

**Step 3: Compile to verify**

Run: `cd apps/backend && mvn clean compile -DskipTests -q`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java
git commit -m "refactor: update StoragePositionManager to use currentCoordinate only"
```

---

## Task 6: Update DesmojRtpRobotProcess to Use Reservations

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java`

**Step 1: Find returnRackToStorage method (around line 423)**

Locate line 429:
```java
GridCoordinate storagePosition = rack.getStorageCoordinate();
```

**Step 2: Update to use StoragePositionManager reservation**

First, ensure StoragePositionManager is available in the process. Find the field declarations and add if not present:
```java
private final StoragePositionManager storagePositionManager;
```

Update constructor to receive StoragePositionManager.

**Step 3: Update returnRackToStorage to use reservation**

Replace the storage position retrieval (around line 429):

```java
// Get reserved position from StoragePositionManager
GridCoordinate storagePosition = storagePositionManager.getReservedPosition(rack.getId())
    .orElseThrow(() -> new IllegalStateException(
        "No reserved storage position for rack " + rack.getId()));
```

**Step 4: Add reservation at station completion**

Find where station work completes and rack is ready to return. Add before the return journey starts:

```java
// Reserve storage position when station work completes
storagePositionManager.reservePositionForRack(rack);
```

**Step 5: Release reservation when rack arrives**

After `rack.moveToStorage(storagePosition)` (around line 484), add:

```java
// Release the reservation
storagePositionManager.releaseReservation(rack.getId());
```

**Step 6: Compile and run existing tests**

Run: `cd apps/backend && mvn clean compile -DskipTests -q`

Then run RTP tests:
Run: `cd apps/backend && mvn test -Dtest=*Rtp* -q`

Expected: BUILD SUCCESS, tests pass (may need constructor updates in tests)

**Step 7: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java
git commit -m "feat: update RTP process to use StoragePositionManager reservations"
```

---

## Task 7: Update DesmojVtmProcess for Soft/Hard Reservations

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Add StoragePositionManager dependency**

Ensure the process has access to StoragePositionManager.

**Step 2: Add soft reservation at tote task assignment**

When tote gets assigned tasks (around line 114-122), add:

```java
// Soft-reserve zone when tote is assigned
storagePositionManager.reserveSoftZone(tote);
```

**Step 3: Release soft reservation at relay arrival**

After tote arrives at relay (around line 238-241), add:

```java
// Release soft reservation - physical occupancy takes over
storagePositionManager.releaseSoftReservation(tote.getId());
```

**Step 4: Add hard reservation at return journey start**

In `executeReturnJourney()` (around line 446), before navigating to rack:

```java
// Hard-reserve specific rack slot for return
List<Rack> racksInZone = getRacksInTargetZone(tote);
StoragePositionManager.ToteSlotReservation reservation =
    storagePositionManager.reserveHardSlot(tote, racksInZone);

// Use reservation for navigation
Rack targetRack = rackManager.getRack(reservation.rackId());
```

**Step 5: Release hard reservation at rack arrival**

After tote is added to rack (around line 472), add:

```java
// Release hard reservation
storagePositionManager.releaseHardReservation(tote.getId());
```

**Step 6: Compile and run VTM tests**

Run: `cd apps/backend && mvn clean compile -DskipTests -q`

Then run VTM tests:
Run: `cd apps/backend && mvn test -Dtest=*Vtm* -q`

Expected: BUILD SUCCESS

**Step 7: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: update VTM process to use soft/hard reservations"
```

---

## Task 8: Update All Remaining storageCoordinate References

**Files:**
- Multiple files that reference `rack.getStorageCoordinate()`

**Step 1: Search for remaining references**

Run: `cd apps/backend && grep -r "getStorageCoordinate\|storageCoordinate" --include="*.java" src/`

**Step 2: Update each reference**

Common replacements:

1. **WarehouseBuilder.java** (line ~368): Position for stats
   ```java
   // Old: position = storage_space.getStorageCoordinate()
   // New: position = storage_space.getCurrentCoordinate()
   ```

2. **Put strategy files** (GreedyPutStrategy, MinWeightedCycleDistancePutStrategy):
   ```java
   // Replace getStorageCoordinate() with getCurrentCoordinate()
   ```

3. **MaximizeTPQStrategy.java**: Similar replacement

**Step 3: Compile to verify all references updated**

Run: `cd apps/backend && mvn clean compile -DskipTests -q`

Expected: BUILD SUCCESS with no compilation errors

**Step 4: Run full test suite**

Run: `cd apps/backend && mvn test -q`

Expected: All tests pass

**Step 5: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add -A
git commit -m "refactor: update all remaining storageCoordinate references to currentCoordinate"
```

---

## Task 9: Update DesmojWarehouseModel Initialization

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/model/DesmojWarehouseModel.java`

**Step 1: Wire StoragePositionManager to processes**

Ensure StoragePositionManager is passed to robot processes during initialization.

**Step 2: Set ZoneManager on StoragePositionManager**

After ZoneManager is created, add:
```java
storagePositionManager.setZoneManager(zoneManager);
```

**Step 3: Compile and verify**

Run: `cd apps/backend && mvn clean compile -DskipTests -q`

**Step 4: Run integration test**

Run: `cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q`

Expected: PASS

**Step 5: Commit**

```bash
cd apps/backend
mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/model/DesmojWarehouseModel.java
git commit -m "feat: wire StoragePositionManager with ZoneManager in model initialization"
```

---

## Task 10: Final Verification and Cleanup

**Files:**
- All modified files

**Step 1: Run full test suite**

Run: `cd apps/backend && mvn clean test`

Expected: BUILD SUCCESS, all tests pass

**Step 2: Run integration test with both system types**

```bash
# Test RTP system
cd apps/backend
mvn test -Dtest=DesmojIntegrationTest -q

# Manually verify with relay system config if needed
```

**Step 3: Format all code**

Run: `cd apps/backend && mvn spotless:apply`

**Step 4: Final commit**

```bash
cd apps/backend
git add -A
git commit -m "test: verify storage position simplification with full test suite"
```

---

## Summary

This implementation plan covers:

1. **Tasks 1-3**: Add reservation infrastructure to StoragePositionManager
2. **Task 4**: Remove storageCoordinate from Rack
3. **Tasks 5-8**: Update all code that referenced storageCoordinate
4. **Task 9**: Wire up dependencies in model initialization
5. **Task 10**: Final verification

Each task follows TDD with bite-sized steps: write failing test, implement minimal code, verify, commit.

**Total estimated tasks: 10**
**Each task: 2-5 minutes per step, 5-6 steps per task**
