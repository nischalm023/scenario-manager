# Relay Capacity Reservation System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement zone-based capacity reservation to prevent HTM deadlock when returning totes to full relays

**Architecture:** Two-sided flow control with atomic reservation at decision points - VTMs reserve before picking from rack, HTMs reserve at station exit before returning. Single `reserved` counter per zone tracks in-flight commitments.

**Tech Stack:** Java 21, Desmo-J 2.5.1e, JUnit 5, Spring Boot 3.2

**Design Document:** `docs/plans/2025-11-12-relay-capacity-reservation-design.md`

---

## Task 1: Create ZoneCapacity Class

**Goal:** Create capacity tracking class with atomic reservation operations

**Files:**
- Create: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneCapacity.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneCapacityTest.java`

### Step 1: Write the failing test

**File:** `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneCapacityTest.java`

```java
package com.greyorange.dessimulator.domain.robot;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class ZoneCapacityTest {
  private ZoneCapacity capacity;

  @BeforeEach
  void setUp() {
    capacity = new ZoneCapacity("zone_1", 5); // 5 total slots
  }

  @Test
  void testInitialAvailability() {
    assertEquals(5, capacity.available());
    assertEquals(0, capacity.getCurrentOccupancy());
    assertEquals(0, capacity.getReserved());
  }

  @Test
  void testReservationReducesAvailability() {
    assertTrue(capacity.tryReserve());
    assertEquals(4, capacity.available());
    assertEquals(1, capacity.getReserved());
  }

  @Test
  void testFulfillConvertsReservationToOccupancy() {
    capacity.tryReserve();
    capacity.fulfill();

    assertEquals(4, capacity.available());
    assertEquals(1, capacity.getCurrentOccupancy());
    assertEquals(0, capacity.getReserved());
  }

  @Test
  void testRemoveIncreasesAvailability() {
    capacity.tryReserve();
    capacity.fulfill();
    capacity.remove();

    assertEquals(5, capacity.available());
    assertEquals(0, capacity.getCurrentOccupancy());
  }

  @Test
  void testReservationFailsWhenFull() {
    // Reserve all 5 slots
    for (int i = 0; i < 5; i++) {
      assertTrue(capacity.tryReserve());
    }

    // 6th reservation should fail
    assertFalse(capacity.tryReserve());
    assertEquals(0, capacity.available());
  }

  @Test
  void testAtomicReservationUnderConcurrency() throws InterruptedException {
    ZoneCapacity sharedCapacity = new ZoneCapacity("test", 10);
    int threadCount = 20;
    int[] successCount = {0};

    Thread[] threads = new Thread[threadCount];
    for (int i = 0; i < threadCount; i++) {
      threads[i] = new Thread(() -> {
        if (sharedCapacity.tryReserve()) {
          synchronized (successCount) {
            successCount[0]++;
          }
        }
      });
      threads[i].start();
    }

    for (Thread thread : threads) {
      thread.join();
    }

    // Exactly 10 should succeed (capacity = 10)
    assertEquals(10, successCount[0]);
    assertEquals(0, sharedCapacity.available());
  }
}
```

### Step 2: Run test to verify it fails

```bash
~/.asdf/shims/mvn test -Dtest=ZoneCapacityTest
```

**Expected:** Compilation error - `ZoneCapacity` class not found

### Step 3: Write minimal implementation

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneCapacity.java`

```java
package com.greyorange.dessimulator.domain.robot;

import lombok.Getter;

/**
 * Tracks relay capacity for a zone with atomic reservation support.
 *
 * <p>Prevents deadlock by ensuring total totes (occupancy + reserved) never exceeds physical
 * capacity. VTMs and HTMs must reserve capacity before committing to deliver/return.
 */
@Getter
public class ZoneCapacity {
  private final String zoneId;
  private final int physicalCapacity;
  private int currentOccupancy = 0;
  private int reserved = 0;

  public ZoneCapacity(String zoneId, int physicalCapacity) {
    this.zoneId = zoneId;
    this.physicalCapacity = physicalCapacity;
  }

  /**
   * Calculate available capacity for new reservations.
   *
   * @return number of slots available for reservation
   */
  public synchronized int available() {
    return physicalCapacity - currentOccupancy - reserved;
  }

  /**
   * Attempt to reserve a slot for delivery/return.
   *
   * <p>Thread-safe atomic operation - only one thread can succeed if one slot remains.
   *
   * @return true if reservation successful, false if no capacity
   */
  public synchronized boolean tryReserve() {
    if (available() > 0) {
      reserved++;
      return true;
    }
    return false;
  }

  /**
   * Convert reservation to actual occupancy when tote arrives at relay.
   *
   * <p>Must be called after successful tryReserve() when tote is delivered.
   */
  public synchronized void fulfill() {
    if (reserved <= 0) {
      throw new IllegalStateException(
          "Cannot fulfill - no active reservations in zone " + zoneId);
    }
    reserved--;
    currentOccupancy++;
  }

  /**
   * Remove tote from relay (HTM picked it up or VTM returned it to rack).
   *
   * <p>Increases available capacity for new reservations.
   */
  public synchronized void remove() {
    if (currentOccupancy <= 0) {
      throw new IllegalStateException("Cannot remove - zone " + zoneId + " is empty");
    }
    currentOccupancy--;
  }

  /**
   * Release reservation without fulfilling (e.g., operation cancelled).
   *
   * <p>Use carefully - normally reservations should be fulfilled.
   */
  public synchronized void releaseReservation() {
    if (reserved <= 0) {
      throw new IllegalStateException("Cannot release - no active reservations in zone " + zoneId);
    }
    reserved--;
  }
}
```

### Step 4: Run test to verify it passes

```bash
~/.asdf/shims/mvn test -Dtest=ZoneCapacityTest
```

**Expected:** All tests PASS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/robot/ZoneCapacity.java \
        src/test/java/com/greyorange/dessimulator/domain/robot/ZoneCapacityTest.java
git commit -m "feat: add ZoneCapacity with atomic reservation system

Implements thread-safe capacity tracking for relay zones:
- tryReserve(): Atomic check-and-reserve operation
- fulfill(): Convert reservation to occupancy
- remove(): Free up capacity when tote leaves relay

Prevents deadlock by ensuring reserved + occupied <= capacity

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: Integrate ZoneCapacity into ZoneManager

**Goal:** Add capacity tracking to each zone and provide reservation API

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java`

### Step 1: Add ZoneCapacity to Zone class

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java`

Find the Zone class fields and add ZoneCapacity:

```java
@Getter
public class Zone {
  private final String zoneId;
  private final List<GridCoordinate> coverageArea;
  private final ZoneCapacity capacity;  // ADD THIS

  public Zone(String zoneId, List<GridCoordinate> coverageArea, int relayCapacity) {
    this.zoneId = zoneId;
    this.coverageArea = coverageArea;
    this.capacity = new ZoneCapacity(zoneId, relayCapacity);  // ADD THIS
  }

  // ... rest of class
}
```

### Step 2: Update ZoneManager to calculate zone capacity

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`

Find the zone creation logic and update to pass relay capacity:

```java
public ZoneManager(
    WarehouseMap map, RelayPositionManager relayPositionManager, int capacityPerRelay) {
  // ... existing code ...

  // When creating zones, calculate total relay capacity for that zone
  for (RackBlockGridDefinition block : map.getRackBlocks()) {
    // ... zone coverage calculation ...

    // Count relays in this zone
    List<RelayPosition> zoneRelays = relayPositionManager.getPositionsForBlock(block.blockId());
    int totalZoneCapacity = zoneRelays.size() * capacityPerRelay;

    Zone zone = new Zone(zoneId, coverageArea, totalZoneCapacity);
    zones.put(zoneId, zone);
  }
}
```

### Step 3: Add capacity query method to ZoneManager

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`

Add method to get zone capacity:

```java
/**
 * Get ZoneCapacity for a specific zone.
 *
 * @param zoneId Zone identifier
 * @return ZoneCapacity tracker for this zone
 */
public ZoneCapacity getZoneCapacity(String zoneId) {
  Zone zone = zones.get(zoneId);
  if (zone == null) {
    throw new IllegalArgumentException("Unknown zone: " + zoneId);
  }
  return zone.getCapacity();
}

/**
 * Try to reserve capacity in a zone.
 *
 * @param zoneId Zone to reserve in
 * @return true if reservation successful
 */
public boolean tryReserveInZone(String zoneId) {
  return getZoneCapacity(zoneId).tryReserve();
}
```

### Step 4: Compile and verify no errors

```bash
~/.asdf/shims/mvn compile -DskipTests
```

**Expected:** BUILD SUCCESS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java \
        src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java
git commit -m "feat: integrate ZoneCapacity into Zone and ZoneManager

Each Zone now tracks relay capacity and provides reservation API.
ZoneManager calculates total capacity from relay count × capacity per relay.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: Add VTM Admission Control

**Goal:** VTM reserves relay capacity BEFORE picking tote from rack

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

### Step 1: Add capacity check in VTM lifecycle

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

Find the VTM main loop (around line 86) and add reservation logic:

```java
// Priority 2: Find tote with tasks in VTM's assigned zones
Optional<Tote> toteOpt = findToteWithTasksInZone();

if (toteOpt.isEmpty()) {
  // No tasks, robot idle
  vtm.setState(RobotState.IDLE);
  // ... existing idle logic ...
  continue;
}

Tote tote = toteOpt.get();

// NEW: Get target zone for this tote
String targetZoneId = getTargetZoneForTote(tote);
Zone targetZone = zoneManager.getZones().get(targetZoneId);

if (targetZone == null) {
  log.error("VTM {}: Could not determine target zone for tote {}", vtm.getId(), tote.getId());
  hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
  continue;
}

// NEW: Try to reserve capacity in target zone
if (!targetZone.getCapacity().tryReserve()) {
  // No capacity available - VTM idles instead of picking
  vtm.setState(RobotState.IDLE);

  log.debug(
      "VTM {} idle - target zone {} has no capacity (available: {})",
      vtm.getId(),
      targetZoneId,
      targetZone.getCapacity().available());

  if (metricsManager != null) {
    metricsManager.emit(
        DomainEvent.vtmBlockedOnCapacity(now(), vtm.getId(), targetZoneId));
  }

  hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
  continue;
}

// Reservation successful - proceed with existing pickup logic
log.debug(
    "VTM {} reserved capacity in zone {} (available: {}), picking tote {}",
    vtm.getId(),
    targetZoneId,
    targetZone.getCapacity().available(),
    tote.getId());

double cycleStart = now();
// ... rest of existing VTM cycle logic ...
```

### Step 2: Add helper method to determine target zone

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

Add method after the main lifecycle:

```java
/**
 * Determine target zone for tote delivery.
 *
 * <p>Uses existing relay selection logic to determine which zone the tote will be delivered to.
 *
 * @param tote Tote to deliver
 * @return Zone ID where tote will be delivered
 */
private String getTargetZoneForTote(Tote tote) {
  // Get tote's current rack
  if (!(tote.getCurrentHoldingEntity() instanceof RackId)) {
    throw new IllegalStateException("Tote not in rack: " + tote.getId());
  }

  RackId rackId = (RackId) tote.getCurrentHoldingEntity();
  Rack rack = storageSpaceManager.getRack(rackId);

  // Use existing relay selection logic
  Optional<RelayPosition> relayOpt =
      relayPositionManager.selectOptimalRelayForVtm(
          vtm, tote, rack, zoneManager, stationManager);

  if (relayOpt.isEmpty()) {
    throw new IllegalStateException("No relay position found for tote " + tote.getId());
  }

  RelayPosition relay = relayOpt.get();

  // Find which zone contains this relay
  for (Map.Entry<String, Zone> entry : zoneManager.getZones().entrySet()) {
    Zone zone = entry.getValue();
    if (zone.getCoverageArea().contains(relay.getPosition())) {
      return entry.getKey();
    }
  }

  throw new IllegalStateException("No zone found for relay " + relay.getId());
}
```

### Step 3: Update fulfill/remove calls

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

After relay delivery (around line 168), add fulfill call:

```java
// Add tote to relay (existing code)
relay.addTote(this, tote);

// NEW: Fulfill the reservation
targetZone.getCapacity().fulfill();

log.debug("VTM {} delivered tote {} to relay {}", vtm.getId(), tote.getId(), relay.getId());
```

At end of VTM cycle when returning tote to rack (if applicable), add remove call:

```java
// When VTM returns empty tote to rack (in return journey)
// ... existing code ...

// NEW: Update zone capacity
String sourceZoneId = getTargetZoneForTote(tote);  // Zone where we picked it from relay
Zone sourceZone = zoneManager.getZones().get(sourceZoneId);
sourceZone.getCapacity().remove();
```

### Step 4: Compile and check for errors

```bash
~/.asdf/shims/mvn compile -DskipTests
```

**Expected:** BUILD SUCCESS

### Step 5: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "feat: add VTM admission control with capacity reservation

VTM now reserves relay capacity BEFORE picking tote from rack:
- Check target zone capacity
- Reserve if available
- Idle if no capacity (backpressure)
- Fulfill reservation when delivered

Prevents VTM from picking totes it cannot deliver.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: Add HTM Return Reservation

**Goal:** HTM reserves relay capacity at station exit BEFORE returning tote

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

### Step 1: Remove old relay selection logic

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

Find and REMOVE the lines added in previous attempt (around line 222-234):

```java
// DELETE THIS ENTIRE BLOCK:
// // Phase 6: Return tote to relay
// // CRITICAL: Select relay with available capacity to avoid deadlock
// // Prefer original relay if not full, otherwise find nearest with capacity
// RelayPosition returnRelay =
//     relayPositionManager.selectRelayForHtmReturn(htm.getPosition(), relay);
//
// if (!returnRelay.getId().equals(relay.getId())) {
//   log.debug(
//       "HTM {} switching return relay from {} to {} (original full)",
//       htm.getId(),
//       relay.getId(),
//       returnRelay.getId());
// }
```

### Step 2: Add station exit reservation logic

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

After station processing (around line 215), add reservation logic:

```java
// Release operator resource
operatorResource.takeBack(1);

log.debug(
    "HTM {} processed {} picks at station {}",
    htm.getId(),
    processingMetrics.totalPicks(),
    station.getStationId());

// Phase 6: Return tote to relay with capacity reservation
// Get candidate zones from "back to store" algorithm (for now, use original zone)
String originalZoneId = getZoneForRelay(relay);
List<String> candidateZoneIds = getCandidateZonesForReturn(tote, originalZoneId);

// Try to reserve capacity in candidate zones (best first)
String selectedZoneId = null;
for (String zoneId : candidateZoneIds) {
  Zone zone = zoneManager.getZones().get(zoneId);
  if (zone != null && zone.getCapacity().tryReserve()) {
    selectedZoneId = zoneId;
    log.debug(
        "HTM {} reserved capacity in zone {} for return (available: {})",
        htm.getId(),
        zoneId,
        zone.getCapacity().available());
    break;
  }
}

if (selectedZoneId == null) {
  // All zones full - HTM must wait
  log.warn(
      "HTM {} cannot return tote {} - all candidate zones full, waiting",
      htm.getId(),
      tote.getId());

  // Wait and retry (creates backpressure)
  hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
  continue;  // Retry the return
}

// Reservation successful - select relay in that zone
Zone selectedZone = zoneManager.getZones().get(selectedZoneId);
RelayPosition returnRelay = selectRelayInZone(selectedZoneId, htm.getPosition());

log.debug(
    "HTM {} returning tote {} to relay {} in zone {}",
    htm.getId(),
    tote.getId(),
    returnRelay.getId(),
    selectedZoneId);

// Travel to relay and deliver
htm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
tote.setState(Tote.ToteState.BEING_RETURNED_TO_RELAY);

double returnTravel = htm.moveToPosition(returnRelay.getPosition());
double unloadingTime = UNLOADING_TIME;
double returnTime = returnTravel + unloadingTime;

hold(new TimeSpan(returnTime, TimeUnit.SECONDS));

// Add tote to relay
htm.removeCarryingTote(tote);
returnRelay.addTote(this, tote);

// Fulfill the reservation
selectedZone.getCapacity().fulfill();

log.debug(
    "HTM {} returned tote {} to relay {} (return time: {}s)",
    htm.getId(),
    tote.getId(),
    returnRelay.getId(),
    returnTime);
```

### Step 3: Add helper methods

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

Add these methods after the main lifecycle:

```java
/**
 * Get zone ID for a relay position.
 *
 * @param relay Relay position
 * @return Zone ID containing this relay
 */
private String getZoneForRelay(RelayPosition relay) {
  for (Map.Entry<String, Zone> entry : zoneManager.getZones().entrySet()) {
    Zone zone = entry.getValue();
    if (zone.getCoverageArea().contains(relay.getPosition())) {
      return entry.getKey();
    }
  }
  throw new IllegalStateException("No zone found for relay " + relay.getId());
}

/**
 * Get candidate zones for tote return, ranked by preference.
 *
 * <p>TODO: Integrate with "back to store" algorithm for optimal storage selection. For now,
 * returns original zone + nearby zones sorted by distance.
 *
 * @param tote Tote to return
 * @param originalZoneId Zone where tote was picked up
 * @return List of zone IDs ranked by preference
 */
private List<String> getCandidateZonesForReturn(Tote tote, String originalZoneId) {
  // For now: prefer original zone, then any other zone
  // TODO: Use "back to store" algorithm to rank by storage optimality
  List<String> candidates = new ArrayList<>();
  candidates.add(originalZoneId);

  // Add other zones as fallback
  zoneManager.getZones().keySet().stream()
      .filter(zoneId -> !zoneId.equals(originalZoneId))
      .forEach(candidates::add);

  return candidates;
}

/**
 * Select a relay position within a specific zone.
 *
 * @param zoneId Target zone
 * @param htmPosition HTM's current position
 * @return Nearest relay in the zone
 */
private RelayPosition selectRelayInZone(String zoneId, GridCoordinate htmPosition) {
  Zone zone = zoneManager.getZones().get(zoneId);
  if (zone == null) {
    throw new IllegalArgumentException("Unknown zone: " + zoneId);
  }

  // Find all relays in this zone
  return relayPositionManager.getPositionsById().values().stream()
      .filter(relay -> zone.getCoverageArea().contains(relay.getPosition()))
      .min(Comparator.comparingInt(rp -> rp.getPosition().manhattanDistance(htmPosition)))
      .orElseThrow(() -> new IllegalStateException("No relay found in zone " + zoneId));
}
```

### Step 4: Update HTM pickup to decrement zone capacity

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

After HTM picks tote from relay (around line 146):

```java
// HTM picks tote from relay (existing code)
Optional<Tote> toteOpt = relay.removeTote();
// ... validation ...
Tote tote = toteOpt.get();

// NEW: Decrement zone occupancy when tote leaves relay
String sourceZoneId = getZoneForRelay(relay);
Zone sourceZone = zoneManager.getZones().get(sourceZoneId);
sourceZone.getCapacity().remove();

log.debug(
    "HTM {} picked up tote {} from relay {} (zone {} capacity now: {})",
    htm.getId(),
    tote.getId(),
    relay.getId(),
    sourceZoneId,
    sourceZone.getCapacity().available());
```

### Step 5: Compile and check for errors

```bash
~/.asdf/shims/mvn compile -DskipTests
```

**Expected:** BUILD SUCCESS

### Step 6: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java
git commit -m "feat: add HTM return reservation at station exit

HTM now reserves relay capacity BEFORE returning tote:
- Reserve at station exit (after 'back to store' decision)
- Try candidate zones in preference order
- Wait if all zones full (backpressure)
- Fulfill reservation when delivered

Fixes TOCTOU race condition in previous approach.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: Add Capacity Metrics

**Goal:** Track reservation failures and capacity utilization for observability

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/DomainEvent.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEvent.java`

### Step 1: Add new metric event types

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEvent.java`

Add to the enum:

```java
public enum MetricEvent {
  // ... existing events ...

  // Capacity events
  VTM_BLOCKED_ON_CAPACITY,  // VTM couldn't reserve capacity
  HTM_BLOCKED_ON_CAPACITY,  // HTM couldn't reserve capacity for return
  ZONE_CAPACITY_SNAPSHOT    // Periodic zone capacity status
}
```

### Step 2: Add event factory methods

**File:** `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/events/DomainEvent.java`

Add factory methods:

```java
public static MetricEventData vtmBlockedOnCapacity(
    double timestamp, int robotId, String zoneId) {
  Map<String, Object> data = new HashMap<>();
  data.put("robot_id", robotId);
  data.put("zone_id", zoneId);
  return new MetricEventData(
      MetricEvent.VTM_BLOCKED_ON_CAPACITY, timestamp, "VTM blocked on capacity", data);
}

public static MetricEventData htmBlockedOnCapacity(
    double timestamp, int robotId, String zoneId) {
  Map<String, Object> data = new HashMap<>();
  data.put("robot_id", robotId);
  data.put("zone_id", zoneId);
  return new MetricEventData(
      MetricEvent.HTM_BLOCKED_ON_CAPACITY, timestamp, "HTM blocked on capacity", data);
}

public static MetricEventData zoneCapacitySnapshot(
    double timestamp,
    String zoneId,
    int physicalCapacity,
    int occupied,
    int reserved,
    int available) {
  Map<String, Object> data = new HashMap<>();
  data.put("zone_id", zoneId);
  data.put("physical_capacity", physicalCapacity);
  data.put("occupied", occupied);
  data.put("reserved", reserved);
  data.put("available", available);
  data.put("utilization_pct", ((occupied + reserved) * 100.0) / physicalCapacity);
  return new MetricEventData(
      MetricEvent.ZONE_CAPACITY_SNAPSHOT, timestamp, "Zone capacity snapshot", data);
}
```

### Step 3: Compile and verify

```bash
~/.asdf/shims/mvn compile -DskipTests
```

**Expected:** BUILD SUCCESS

### Step 4: Commit

```bash
git add src/main/java/com/greyorange/dessimulator/simulator/metrics/events/DomainEvent.java \
        src/main/java/com/greyorange/dessimulator/simulator/metrics/MetricEvent.java
git commit -m "feat: add zone capacity metrics events

Adds events for capacity tracking:
- VTM_BLOCKED_ON_CAPACITY: VTM couldn't reserve
- HTM_BLOCKED_ON_CAPACITY: HTM couldn't reserve
- ZONE_CAPACITY_SNAPSHOT: Periodic capacity status

Enables observability of admission control system.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: Integration Test

**Goal:** Verify no deadlock with 1 VTM + 1 HTM configuration

**Files:**
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/simulator/RelayCapacityIntegrationTest.java`

### Step 1: Write integration test

**File:** `apps/backend/src/test/java/com/greyorange/dessimulator/simulator/RelayCapacityIntegrationTest.java`

```java
package com.greyorange.dessimulator.simulator;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.api.dto.SimulationConfig;
import com.greyorange.dessimulator.api.dto.SimulationResult;
import java.nio.file.Files;
import java.nio.file.Paths;
import org.junit.jupiter.api.Test;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

class RelayCapacityIntegrationTest {
  private static final Logger log = LoggerFactory.getLogger(RelayCapacityIntegrationTest.class);

  @Test
  void testNoDeadlockWith1Vtm1Htm() throws Exception {
    // Load test configuration
    String configJson =
        Files.readString(
            Paths.get("src/test/resources/relay-capacity-test-config.json"));

    SimulationConfig config = parseConfig(configJson);

    // Run simulation
    DesmojSimulationEngine engine = new DesmojSimulationEngine();
    SimulationResult result = engine.run(config);

    // Verify simulation completed
    assertTrue(result.isSuccess(), "Simulation should complete without deadlock");

    // Verify HTM completed multiple cycles
    int htmCycles = result.getMetrics().getRobotMetrics().stream()
        .filter(r -> r.getRobotId() == 1)  // HTM ID
        .findFirst()
        .map(r -> r.getCyclesCompleted())
        .orElse(0);

    assertTrue(htmCycles > 1, "HTM should complete multiple cycles (was: " + htmCycles + ")");

    log.info("Test passed: HTM completed {} cycles without deadlock", htmCycles);
  }

  private SimulationConfig parseConfig(String json) {
    // Use existing JSON parsing logic
    return new com.fasterxml.jackson.databind.ObjectMapper()
        .readValue(json, SimulationConfig.class);
  }
}
```

### Step 2: Create test configuration

**File:** `apps/backend/src/test/resources/relay-capacity-test-config.json`

```json
{
  "systemType": "relay",
  "duration": 1,
  "scenario": "scenario_a",
  "binsCount": 8,
  "htmSpeed": 1.5,
  "mapConfig": {
    "gridWidth": 22,
    "gridDepth": 26,
    "cellSizeMeters": 1,
    "useAdvancedLayout": true,
    "highwayConfig": {
      "highwaysPerEdge": 3
    },
    "rackBlockConfig": {
      "rowsPerBlock": 10,
      "rackColumns": 2,
      "crossAisleWidthGridUnits": 1
    },
    "stationLayout": {
      "layoutType": "single_bottom"
    },
    "verticalToteCapacityInRack": 4
  },
  "numHtms": 1,
  "numVtms": 1,
  "operatorTime": 12,
  "orderSelectionAlgorithm": "maximize_ppf_directly",
  "picksPerFace": 3,
  "putAlgorithm": "greedy",
  "rtpRobotCount": 4,
  "rtpRobotSpeed": 1.5,
  "vtmSpeed": 1.5
}
```

### Step 3: Run test

```bash
~/.asdf/shims/mvn test -Dtest=RelayCapacityIntegrationTest
```

**Expected:** Test PASSES - HTM completes multiple cycles without deadlock

### Step 4: Commit

```bash
git add src/test/java/com/greyorange/dessimulator/simulator/RelayCapacityIntegrationTest.java \
        src/test/resources/relay-capacity-test-config.json
git commit -m "test: add integration test for relay capacity reservation

Verifies that 1 VTM + 1 HTM configuration completes without deadlock.
HTM should complete multiple cycles, proving reservation system works.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Verification

After completing all tasks, verify the system works:

### Manual Test

```bash
# Start server
~/.asdf/shims/mvn spring-boot:run

# In another terminal, run simulation with 1 VTM + 1 HTM
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @src/test/resources/relay-capacity-test-config.json

# Check logs for HTM activity
grep "HTM 1.*completed cycle" logs/application.log | wc -l
# Should show multiple cycles (not just 1)
```

### Expected Behavior

**Before fix:**
- HTM completes 1 cycle
- HTM blocks trying to return tote
- Simulation runs to end with HTM stuck

**After fix:**
- HTM completes many cycles
- VTM may idle when capacity low (backpressure)
- System self-regulates under load
- No deadlocks

### Metrics to Check

Monitor these in simulation results:
- `vtmBlockedOnCapacity`: Count of VTM admission control blocks
- `htmBlockedOnCapacity`: Count of HTM return blocks (should be rare)
- `zoneCapacityUtilization`: Should stay below 100%
- `htmCyclesCompleted`: Should be >> 1

---

## Notes

**DRY**: ZoneCapacity class is reused by all zones, single source of truth for capacity logic

**YAGNI**: No premature optimization - simple synchronized methods sufficient for now

**TDD**: Tests written before implementation, verified to fail then pass

**Frequent commits**: Each task is one logical unit, committed immediately after verification

**Skills used:**
- @superpowers:test-driven-development: Failing test → minimal impl → verify pass
- @superpowers:verification-before-completion: Run tests to confirm before claiming success
