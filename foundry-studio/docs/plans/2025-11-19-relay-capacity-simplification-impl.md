# Relay Position Capacity Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Simplify relay position model from queue with variable capacity to single tote slot (0 or 1).

**Architecture:** Replace `Queue<Tote>` with `Optional<Tote>` in RelayPosition, inline ZoneCapacity into Zone with direct relay position references, derive occupancy from relay state instead of manual counters.

**Tech Stack:** Java 21, Spring Boot 3.2, Desmo-J 2.5.1e, JUnit 5, Lombok

---

## Breaking Changes

**IMPORTANT:** This refactoring introduces breaking changes:

1. **RelayPosition constructors** - Remove `int capacity` parameter entirely (capacity is always 1)
2. **Zone constructor** - Remove `int relayCapacity` parameter (capacity derived from relay positions)
3. **Zone.getCapacity()** - DELETE this method entirely (use `zone.tryReserve()` etc. directly)
4. **RelayPositionManager constructor** - Remove `int capacityPerPosition` parameter

All callers must be updated. There is no backward compatibility layer.

---

## Task 1: Simplify RelayPosition to Optional<Tote>

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java`

**Step 1: Write the failing test for new Optional-based methods**

```java
// Add to RelayPositionTest.java
@Test
void testOptionalToteStorage() {
  RelayPosition relay =
      new RelayPosition(
          "relay_1", new GridCoordinate(5, 10), "block_0", "left", "zone_block_0_left");

  // Initially empty
  assertTrue(relay.isEmpty());
  assertFalse(relay.getTote().isPresent());

  // peek() returns empty Optional
  assertTrue(relay.peek().isEmpty());
}

@Test
void testIsEmptyAndGetTote() {
  RelayPosition relay =
      new RelayPosition(
          "relay_1", new GridCoordinate(5, 10), "block_0", "left", "zone_block_0_left");

  assertTrue(relay.isEmpty());
  assertEquals(Optional.empty(), relay.getTote());
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest#testOptionalToteStorage -q`
Expected: FAIL - constructor doesn't exist without capacity parameter

**Step 3: Modify RelayPosition.java**

Replace queue-based storage with Optional:

```java
package com.greyorange.dessimulator.domain.robot;

import co.paralleluniverse.fibers.SuspendExecution;
import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.inventory.Tote;
import com.greyorange.dessimulator.simulator.DesmojWarehouseModel;
import desmoj.core.advancedModellingFeatures.Res;
import desmoj.core.simulator.SimProcess;
import java.util.Optional;
import lombok.Getter;
import lombok.extern.slf4j.Slf4j;

/**
 * Represents a physical relay position where VTMs and HTMs exchange totes.
 *
 * <p>Each relay position holds exactly 0 or 1 tote. Uses Desmo-J Res with capacity=1 as safeguard
 * for blocking if a process tries to add to an occupied relay.
 */
@Slf4j
@Getter
public class RelayPosition {
  private final String id;
  private final GridCoordinate position;
  private final String rackBlockId;
  private final String side;
  private final String ownerZoneId;

  private Res resource;
  private Optional<Tote> tote = Optional.empty();

  /**
   * Constructor with owner zone ID for direct zone ownership.
   *
   * <p>Creates RelayPosition with Desmo-J resource immediately.
   */
  public RelayPosition(
      DesmojWarehouseModel model,
      String id,
      GridCoordinate position,
      String rackBlockId,
      String side,
      String ownerZoneId) {
    this.id = id;
    this.position = position;
    this.rackBlockId = rackBlockId;
    this.side = side;
    this.ownerZoneId = ownerZoneId;
    this.resource = new Res(model, id + "_capacity", 1, false, false);
  }

  /**
   * Two-phase constructor for WarehouseBuilder.
   *
   * <p>Creates RelayPosition with metadata only. Call initializeRuntime() later.
   */
  public RelayPosition(
      String id,
      GridCoordinate position,
      String rackBlockId,
      String side,
      String ownerZoneId) {
    this.id = id;
    this.position = position;
    this.rackBlockId = rackBlockId;
    this.side = side;
    this.ownerZoneId = ownerZoneId;
    this.resource = null;
  }

  /**
   * Initialize Desmo-J runtime resources.
   *
   * @param model DesmojWarehouseModel for creating Res resource
   */
  public void initializeRuntime(DesmojWarehouseModel model) {
    if (this.resource != null) {
      throw new IllegalStateException("Runtime already initialized for relay " + id);
    }
    this.resource = new Res(model, id + "_capacity", 1, false, false);
  }

  public boolean isRuntimeInitialized() {
    return this.resource != null;
  }

  /**
   * Add tote to relay position. Blocks via Res if occupied.
   */
  public void addTote(SimProcess process, Tote tote) throws SuspendExecution {
    log.debug(
        "Relay {} attempting to add tote {} (occupied: {})",
        this.id,
        tote.getId(),
        this.tote.isPresent());

    // Request resource slot - blocks if relay is occupied
    resource.provide(1);

    this.tote = Optional.of(tote);
    tote.moveToRelay(this.id);

    log.debug("Relay {} added tote {}", this.id, tote.getId());
  }

  /**
   * Remove tote from relay position.
   *
   * @return The tote if present, empty otherwise
   */
  public Optional<Tote> removeTote() {
    if (tote.isEmpty()) {
      log.debug("Relay {} removeTote called but empty", this.id);
      return Optional.empty();
    }

    Tote removed = tote.get();
    tote = Optional.empty();
    resource.takeBack(1);

    log.debug("Relay {} removed tote {}", this.id, removed.getId());
    return Optional.of(removed);
  }

  /**
   * Peek at the tote without removing it.
   *
   * @return The tote if present, empty otherwise
   */
  public Optional<Tote> peek() {
    return tote;
  }

  /**
   * Check if relay is empty (can accept a tote).
   */
  public boolean isEmpty() {
    return tote.isEmpty();
  }

  /**
   * Check if relay has a tote with pending tasks.
   *
   * @return true if tote exists and has tasks
   */
  public boolean hasToteWithTasks() {
    return tote.map(t -> !t.getTasksByStation().isEmpty()).orElse(false);
  }
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest -q`
Expected: PASS

**Step 5: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java && git commit -m "refactor: simplify RelayPosition to Optional<Tote>"
```

---

## Task 2: Update Zone to Hold Relay Positions and Inline Capacity

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java`

**Step 1: Write failing test for new Zone capacity methods**

```java
package com.greyorange.dessimulator.domain.robot;

import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.inventory.Tote;
import java.util.List;
import java.util.Optional;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class ZoneTest {
  private Zone zone;

  @BeforeEach
  void setUp() {
    zone = new Zone(
        "zone_1",
        "aisle",
        List.of("block_0"),
        List.of(new GridCoordinate(0, 0)),
        List.of(new GridCoordinate(1, 1)));
  }

  @Test
  void testAddRelayPosition() {
    RelayPosition relay = new RelayPosition(
        "relay_1", new GridCoordinate(5, 10), "block_0", "left", "zone_1");

    zone.addRelayPosition(relay);

    assertEquals(1, zone.getPhysicalCapacity());
    assertEquals(1, zone.getRelayPositions().size());
  }

  @Test
  void testCountOccupied() {
    // Create relay with mocked tote
    RelayPosition relay1 = mock(RelayPosition.class);
    RelayPosition relay2 = mock(RelayPosition.class);

    when(relay1.getTote()).thenReturn(Optional.of(mock(Tote.class)));
    when(relay2.getTote()).thenReturn(Optional.empty());

    zone.addRelayPosition(relay1);
    zone.addRelayPosition(relay2);

    assertEquals(1, zone.countOccupied());
  }

  @Test
  void testAvailable() {
    RelayPosition relay1 = mock(RelayPosition.class);
    RelayPosition relay2 = mock(RelayPosition.class);

    when(relay1.getTote()).thenReturn(Optional.empty());
    when(relay2.getTote()).thenReturn(Optional.empty());

    zone.addRelayPosition(relay1);
    zone.addRelayPosition(relay2);

    // 2 relays, 0 occupied, 0 reserved = 2 available
    assertEquals(2, zone.available());
  }

  @Test
  void testTryReserve() {
    RelayPosition relay = mock(RelayPosition.class);
    when(relay.getTote()).thenReturn(Optional.empty());
    zone.addRelayPosition(relay);

    assertTrue(zone.tryReserve());
    assertEquals(0, zone.available());
    assertEquals(1, zone.getReserved());
  }

  @Test
  void testTryReserveFailsWhenFull() {
    RelayPosition relay = mock(RelayPosition.class);
    when(relay.getTote()).thenReturn(Optional.of(mock(Tote.class)));
    zone.addRelayPosition(relay);

    // 1 relay, 1 occupied = 0 available
    assertFalse(zone.tryReserve());
  }

  @Test
  void testCompleteReservation() {
    RelayPosition relay = mock(RelayPosition.class);
    when(relay.getTote()).thenReturn(Optional.empty());
    zone.addRelayPosition(relay);

    zone.tryReserve();
    assertEquals(1, zone.getReserved());

    zone.completeReservation();
    assertEquals(0, zone.getReserved());
  }
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=ZoneTest -q`
Expected: FAIL - Zone doesn't have these methods yet

**Step 3: Modify Zone.java**

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import java.util.*;
import lombok.Getter;

/**
 * Represents a zone within the warehouse that robots (typically VTMs) are assigned to.
 *
 * <p>Zones own their relay positions and track reservation state for capacity management.
 */
@Getter
public class Zone {
  private final String zoneId;
  private final String zoneType;
  private final List<String> rackBlockIds;
  private final List<GridCoordinate> coverageArea;
  private final List<GridCoordinate> servicedRackCoordinates;
  private final Set<Integer> assignedVtmIds;
  private final List<RelayPosition> relayPositions = new ArrayList<>();
  private int reserved = 0;

  public Zone(
      String zoneId,
      String zoneType,
      List<String> rackBlockIds,
      List<GridCoordinate> coverageArea,
      List<GridCoordinate> servicedRackCoordinates) {
    this.zoneId = zoneId;
    this.zoneType = zoneType;
    this.rackBlockIds = rackBlockIds != null ? new ArrayList<>(rackBlockIds) : new ArrayList<>();
    this.coverageArea = coverageArea != null ? new ArrayList<>(coverageArea) : new ArrayList<>();
    this.servicedRackCoordinates =
        servicedRackCoordinates != null
            ? new ArrayList<>(servicedRackCoordinates)
            : new ArrayList<>();
    this.assignedVtmIds = new HashSet<>();
  }

  /**
   * Add a relay position to this zone.
   */
  public void addRelayPosition(RelayPosition relay) {
    relayPositions.add(relay);
  }

  /**
   * Get unmodifiable list of relay positions.
   */
  public List<RelayPosition> getRelayPositions() {
    return Collections.unmodifiableList(relayPositions);
  }

  /**
   * Count occupied relay positions (have a tote).
   */
  public int countOccupied() {
    return (int) relayPositions.stream()
        .filter(r -> r.getTote().isPresent())
        .count();
  }

  /**
   * Get physical capacity (number of relay positions).
   */
  public int getPhysicalCapacity() {
    return relayPositions.size();
  }

  /**
   * Get available capacity for new reservations.
   */
  public int available() {
    return getPhysicalCapacity() - countOccupied() - reserved;
  }

  /**
   * Attempt to reserve a slot for delivery/return.
   *
   * @return true if reservation successful, false if no capacity
   */
  public boolean tryReserve() {
    if (available() > 0) {
      reserved++;
      return true;
    }
    return false;
  }

  /**
   * Complete reservation when tote arrives at relay.
   */
  public void completeReservation() {
    if (reserved <= 0) {
      throw new IllegalStateException("Cannot complete - no active reservations in zone " + zoneId);
    }
    reserved--;
  }

  /** Assign a VTM to this zone. */
  public void addVtm(int vtmId) {
    assignedVtmIds.add(vtmId);
  }

  /** Remove a VTM from this zone. */
  public void removeVtm(int vtmId) {
    assignedVtmIds.remove(vtmId);
  }

  /** Check if a position is within this zone's coverage area. */
  public boolean containsPosition(GridCoordinate position) {
    return coverageArea.contains(position);
  }

  /** Check if this zone services a particular rack coordinate. */
  public boolean servicesRack(GridCoordinate rackCoordinate) {
    return servicedRackCoordinates.contains(rackCoordinate);
  }

  /** Get number of VTMs assigned to this zone. */
  public int getVtmCount() {
    return assignedVtmIds.size();
  }

  /** Check if this zone has any assigned VTMs. */
  public boolean hasVtms() {
    return !assignedVtmIds.isEmpty();
  }

  /** Serialize to dictionary. */
  public Map<String, Object> toDict() {
    Map<String, Object> dict = new HashMap<>();
    dict.put("zone_id", zoneId);
    dict.put("zone_type", zoneType);
    dict.put("rack_block_ids", rackBlockIds);
    dict.put("assigned_vtm_ids", new ArrayList<>(assignedVtmIds));
    dict.put("vtm_count", assignedVtmIds.size());
    dict.put("coverage_area_size", coverageArea.size());
    dict.put("serviced_racks_count", servicedRackCoordinates.size());
    dict.put("relay_count", relayPositions.size());
    return dict;
  }
}
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=ZoneTest -q`
Expected: PASS

**Step 5: Write integration test for relay registration flow**

```java
// Add to ZoneTest.java
@Test
void testRelayRegistrationFlow() {
  // Simulate the full registration flow
  Zone zone = new Zone(
      "zone_1",
      "aisle",
      List.of("block_0"),
      List.of(new GridCoordinate(0, 0)),
      List.of(new GridCoordinate(1, 1)));

  // Create real relay positions (not mocks)
  RelayPosition relay1 = new RelayPosition(
      "relay_1", new GridCoordinate(5, 10), "block_0", "left", "zone_1");
  RelayPosition relay2 = new RelayPosition(
      "relay_2", new GridCoordinate(6, 10), "block_0", "left", "zone_1");

  // Register relays with zone
  zone.addRelayPosition(relay1);
  zone.addRelayPosition(relay2);

  // Verify capacity derived correctly
  assertEquals(2, zone.getPhysicalCapacity());
  assertEquals(0, zone.countOccupied());
  assertEquals(2, zone.available());

  // Test reservation flow
  assertTrue(zone.tryReserve());
  assertEquals(1, zone.available());

  assertTrue(zone.tryReserve());
  assertEquals(0, zone.available());

  // Third reservation should fail
  assertFalse(zone.tryReserve());

  // Complete one reservation
  zone.completeReservation();
  assertEquals(1, zone.available());
}
```

**Step 6: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=ZoneTest -q`
Expected: PASS

**Step 7: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java && git commit -m "refactor: inline capacity tracking into Zone with relay position ownership"
```

---

## Task 3: Delete ZoneCapacity Class

**Files:**
- Delete: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneCapacity.java`
- Delete: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneCapacityTest.java`

**Step 1: Delete the files**

```bash
cd apps/backend && rm src/main/java/com/greyorange/dessimulator/domain/robot/ZoneCapacity.java src/test/java/com/greyorange/dessimulator/domain/robot/ZoneCapacityTest.java
```

**Step 2: Verify compilation still works**

Run: `cd apps/backend && mvn compile -q`
Expected: FAIL - other files still reference ZoneCapacity

**Step 3: Commit deletion (will fix references in later tasks)**

```bash
cd apps/backend && git add -A && git commit -m "refactor: delete ZoneCapacity class (functionality inlined to Zone)"
```

---

## Task 4: Update RelayPositionManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java`

**Step 1: Remove capacityPerPosition and update relay creation**

Remove the `capacityPerPosition` field and update all relay creation to use simplified constructor. Update method references to use new RelayPosition API.

Key changes:
- Remove `capacityPerPosition` field and constructor parameter
- Update `generateRelayPositionsMetadataOnly()` to not pass capacity
- Update `selectOptimalRelayForVtm()`: replace `!relay.isFull()` with `relay.isEmpty()`
- Update `getPositionsWithTotes()`: replace `RelayPosition::hasTotes` with `relay -> relay.getTote().isPresent()`
- Update `findRelayWithTotes()`: replace `RelayPosition::firstToteHasTasks` with `RelayPosition::hasToteWithTasks`
- Update `selectRelayForHtmReturn()`: replace `!relay.isFull()` with `relay.isEmpty()`
- Remove `getPendingTotesCount` comparisons

**Step 2: Run compilation to verify**

Run: `cd apps/backend && mvn compile -q`

**Step 3: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java && git commit -m "refactor: update RelayPositionManager for simplified relay model"
```

---

## Task 5: Update ZoneManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`

**Step 1: Remove initializeCapacityTracking method**

Delete the `initializeCapacityTracking()` method entirely. Update zone creation to use new constructor without relayCapacity.

**Step 2: Add relay registration method**

```java
/**
 * Register relay positions with their owner zones.
 *
 * @param relayPositionManager Manager containing all relay positions
 */
public void registerRelayPositions(RelayPositionManager relayPositionManager) {
  for (RelayPosition relay : relayPositionManager.getPositionsById().values()) {
    String zoneId = relay.getOwnerZoneId();
    Zone zone = zones.get(zoneId);
    if (zone != null) {
      zone.addRelayPosition(relay);
    }
  }
}
```

**Step 3: Run compilation**

Run: `cd apps/backend && mvn compile -q`

**Step 4: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java && git commit -m "refactor: update ZoneManager to register relay positions with zones"
```

---

## Task 6: Update DesmojVtmProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Update all zone capacity calls**

Replace:
- `targetZone.getCapacity().tryReserve()` → `targetZone.tryReserve()`
- `targetZone.getCapacity().available()` → `targetZone.available()`
- `targetZone.getCapacity().fulfill()` → `targetZone.completeReservation()`
- `zone.getCapacity().remove()` → remove line (not needed)
- `ctx.relay().isFull()` → `ctx.relay().getTote().isPresent()`
- `relay.peekFirstTote()` → `relay.peek()`

Update `findCongestedZone()`:
```java
int total = zone.countOccupied() + zone.getReserved();
double utilization = (total * 100.0) / zone.getPhysicalCapacity();
```

**Step 2: Run compilation**

Run: `cd apps/backend && mvn compile -q`

**Step 3: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java && git commit -m "refactor: update DesmojVtmProcess for simplified zone capacity API"
```

---

## Task 7: Update DesmojHtmProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

**Step 1: Update capacity calls**

Replace:
- `selectedZone.getCapacity().fulfill()` → `selectedZone.completeReservation()`
- `relay.peekFirstTote()` → `relay.peek()`
- Remove `sourceZone.getCapacity().remove()` line (not needed)

**Step 2: Run compilation**

Run: `cd apps/backend && mvn compile -q`

**Step 3: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java && git commit -m "refactor: update DesmojHtmProcess for simplified zone capacity API"
```

---

## Task 8: Update BackToStoreSelector

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/BackToStoreSelector.java`

**Step 1: Update capacity calls**

Replace:
- `zone.getCapacity().available()` → `zone.available()`
- `zone.getCapacity().tryReserve()` → `zone.tryReserve()`
- `RelayPosition::hasAvailableCapacity` → `RelayPosition::isEmpty`

**Step 2: Run compilation**

Run: `cd apps/backend && mvn compile -q`

**Step 3: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/domain/put/BackToStoreSelector.java && git commit -m "refactor: update BackToStoreSelector for simplified capacity API"
```

---

## Task 9: Update DesmojSimulationEngine

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Remove initializeCapacityTracking call, add registerRelayPositions**

Remove:
```java
zoneManager().initializeCapacityTracking(
    relayPositionManager(), relayPositionManager().getCapacityPerPosition());
```

Add:
```java
zoneManager().registerRelayPositions(relayPositionManager());
```

**Step 2: Run compilation**

Run: `cd apps/backend && mvn compile -q`

**Step 3: Commit**

```bash
cd apps/backend && git add src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java && git commit -m "refactor: update DesmojSimulationEngine to register relay positions with zones"
```

---

## Task 10: Update Remaining Files

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/process/DesmojSnapshotCaptureProcess.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojMetricMonitoringProcess.java`

**Step 1: Update WarehouseBuilder.buildRelayPositionManager()**

```java
/**
 * Build relay position manager (metadata only).
 *
 * <p>Creates RelayPositionManager with position metadata but no Desmo-J resources.
 * Capacity is always 1 per position (simplified model).
 */
private void buildRelayPositionManager() {
  relayPositionManager = new RelayPositionManager(map);
  log.info(
      "Built RelayPositionManager with {} positions (metadata only)",
      relayPositionManager.getPositionsById().size());
}
```

**Step 2: Update WarehouseBuilder.buildZoneManager()**

Zone no longer needs relayCapacity parameter - capacity is derived from registered relay positions.

```java
/** Build zone manager for relay systems. */
private void buildZoneManager() {
  if (config.systemType() == SystemType.RELAY) {
    zoneManager = new ZoneManager(map);
    zoneManager.generateZones();
    log.info("Generated {} zones", zoneManager.getAllZones().size());
  }
}
```

Note: Zone constructor changes from:
```java
new Zone(zoneId, zoneType, rackBlockIds, coverageArea, servicedRackCoordinates, relayCapacity)
```
to:
```java
new Zone(zoneId, zoneType, rackBlockIds, coverageArea, servicedRackCoordinates)
```

**Step 3: Update DesmojSnapshotCaptureProcess**

Remove capacity field from relay snapshot. Find the relay snapshot creation and remove:
```java
relay.getCapacity()  // DELETE - field no longer exists
```

Update to only capture:
- `id`, `position`, `ownerZoneId`, `hasTote` (boolean), `toteId` (if present)

**Step 4: Update DesmojMetricMonitoringProcess**

Update capacity diagnostic logging:

```java
// Before:
ZoneCapacity capacity = zone.getCapacity();
int physicalCapacity = capacity.getPhysicalCapacity();
int currentOccupancy = capacity.getCurrentOccupancy();
int reserved = capacity.getReserved();

// After:
int physicalCapacity = zone.getPhysicalCapacity();
int currentOccupancy = zone.countOccupied();
int reserved = zone.getReserved();
```

**Step 5: Run compilation**

Run: `cd apps/backend && mvn compile -q`
Expected: PASS

**Step 6: Commit**

```bash
cd apps/backend && git add -A && git commit -m "refactor: update remaining files for simplified relay capacity model"
```

---

## Task 11: Update Tests

**Files:**
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java`

**Step 1: Update RelayPositionTest**

Update tests to use new API (no capacity parameter, Optional-based methods).

**Step 2: Update RelayPositionManagerTest**

Remove capacity-related tests, update for new relay selection logic.

**Step 3: Run all tests**

Run: `cd apps/backend && mvn test -q`
Expected: PASS

**Step 4: Commit**

```bash
cd apps/backend && git add -A && git commit -m "test: update tests for simplified relay capacity model"
```

---

## Task 12: Run Integration Tests and Format

**Step 1: Run full test suite**

Run: `cd apps/backend && mvn test -q`
Expected: All tests pass

**Step 2: Format code**

Run: `cd apps/backend && mvn spotless:apply`

**Step 3: Run tests again after formatting**

Run: `cd apps/backend && mvn test -q`
Expected: All tests pass

**Step 4: Final commit**

```bash
cd apps/backend && git add -A && git commit -m "style: format code with spotless"
```

---

## Verification

After completing all tasks:

1. Run full test suite: `cd apps/backend && mvn test`
2. Start server: `npx nx serve backend`
3. Run a relay system simulation and verify metrics
4. Check logs for any capacity-related errors

## Summary of Changes

| Category | Files Changed |
|----------|--------------|
| Domain | RelayPosition.java, Zone.java |
| Deleted | ZoneCapacity.java, ZoneCapacityTest.java |
| Managers | RelayPositionManager.java, ZoneManager.java |
| Processes | DesmojVtmProcess.java, DesmojHtmProcess.java |
| Services | BackToStoreSelector.java |
| Engine | DesmojSimulationEngine.java |
| Supporting | WarehouseBuilder.java, DesmojSnapshotCaptureProcess.java, DesmojMetricMonitoringProcess.java |
| Tests | RelayPositionTest.java, RelayPositionManagerTest.java, ZoneTest.java (new) |
