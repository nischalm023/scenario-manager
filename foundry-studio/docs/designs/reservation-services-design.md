# Reservation Services Architecture Design

**Version:** 2.0
**Date:** 2025-01-22
**Status:** Approved

---

## 1. Overview

This design document describes the reservation architecture for the GTP Robotics Queue Simulator. The system uses a **strategy pattern** for back-to-store operations with clear ownership of reservation state.

### 1.1 Design Principles

- **Strategy Pattern**: Separate implementations for RTP vs Relay systems
- **Single Responsibility**: Each component manages one type of reservation
- **Atomic Operations**: Select-and-reserve happens atomically to prevent race conditions
- **No Duplicate State**: Reservations tracked in exactly one place
- **Thread-Safe**: All services handle concurrent access safely

### 1.2 Component Overview

```
┌─────────────────────────────────────────────────────────────────┐
│ LAYER 1: Robot Processes (Callers)                             │
│ - VtmProcess, HtmProcess, RtpRobotProcess                       │
│ - Coordinate their workflows, call back-to-store strategies    │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ LAYER 2: Back-to-Store Strategies (System-Specific)            │
│ - AbstractBackToStoreSelector: Common base                     │
│ - RtpBackToStoreSelector: Rack position reservations           │
│ - RelayBackToStoreSelector: Tote slot + relay selection        │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│ LAYER 3: Core Domain (Managers & Entities)                     │
│ - RelayPositionManager: Relay registry + reservations          │
│ - StorageSpaceManager: Location tracking, moves                │
│ - Zone, ZoneManager: Spatial organization                      │
│ - Rack, Tote: Entities                                         │
└─────────────────────────────────────────────────────────────────┘
```

---

## 2. RelayPositionManager (Registry + Reservations)

### 2.1 Responsibility

**Single Responsibility:** Manage all relay positions in warehouse, including their availability and reservations.

**Why not a separate service?**
- Relay positions hold **ONE tote** (simple 1:1 relationship)
- Manager already owns the entities it's managing
- Avoids extra layer of indirection
- Natural cohesion: "manage relay positions and their availability"

### 2.2 State

```java
// Registry: All relay positions
private final Map<String, RelayPosition> positionsById = new ConcurrentHashMap<>();
private final Map<String, List<RelayPosition>> positionsByZone = new ConcurrentHashMap<>();

// Reservations: Which tote reserved which relay
private final Map<ToteId, ReservationRecord> reservationsByTote = new ConcurrentHashMap<>();
private final Map<String, Set<ToteId>> reservationsByRelay = new ConcurrentHashMap<>();

private record ReservationRecord(String relayId, Instant createdAt) {}
```

### 2.3 API

```java
// ========================================
// Registry Operations
// ========================================

void registerRelayPosition(RelayPosition position);
RelayPosition getById(String relayId);
List<RelayPosition> getPositionsByZone(String zoneId);
int getTotalCount();

// ========================================
// Availability Queries (Reservation-Aware)
// ========================================

List<RelayPosition> getAvailableInZone(String zoneId);
int getAvailableCapacity(String relayId);
boolean isAvailable(String relayId);
boolean isReserved(String relayId);

// ========================================
// Reservation Operations
// ========================================

boolean tryReserve(String relayId, ToteId toteId);
void releaseReservation(ToteId toteId);
Optional<String> getReservation(ToteId toteId);
int cleanupOrphanedReservations(long maxAgeSeconds);
```

### 2.4 Capacity Calculation

```java
public int getAvailableCapacity(String relayId) {
  RelayPosition relay = positionsById.get(relayId);
  if (relay == null) return 0;

  int occupied = relay.getTote().isPresent() ? 1 : 0; // RelayPosition holds ONE tote
  int reserved = isReserved(relayId) ? 1 : 0;

  return Math.max(0, 1 - occupied - reserved); // Capacity is always 1
}
```

### 2.5 Key Features

**1. Double-Booking Prevention:**
```java
if (reservationsByTote.containsKey(toteId)) {
    throw new IllegalStateException("Tote already has reservation");
}
```

**2. Thread Safety:**
```java
public synchronized boolean tryReserve(String relayId, ToteId toteId) {
    // Atomic check-and-reserve
}
```

**3. Orphan Cleanup:**
```java
public synchronized int cleanupOrphanedReservations(long maxAgeSeconds) {
    // Remove reservations older than threshold
}
```

---

## 3. Zone (No Reservations!)

### 3.1 Responsibility

**Spatial definition and VTM assignment ONLY.** Does NOT track reservations.

### 3.2 What Was Removed

```java
// ❌ DELETED
private int reserved = 0;

// ❌ DELETED
public boolean tryReserve() { ... }
public void completeReservation() { ... }
```

### 3.3 What Remains

```java
@Getter
public class Zone {
  private final String zoneId;
  private final List<RelayPosition> relayPositions;
  private final Set<Integer> assignedVtmIds;

  // Physical capacity queries
  public int getPhysicalCapacity() {
    return relayPositions.size();
  }

  public int getOccupiedCount() {
    return (int) relayPositions.stream()
        .filter(r -> r.getTote().isPresent())
        .count();
  }

  // VTM management
  public void addVtm(int vtmId) { ... }
  public void removeVtm(int vtmId) { ... }

  // Spatial queries
  public boolean servicesRack(GridCoordinate rackCoord) { ... }
}
```

---

## 4. ZoneManager (Capacity Queries)

### 4.1 Responsibility

Aggregate-level queries for zones, **accounting for reservations from RelayPositionManager**.

### 4.2 API

```java
public class ZoneManager {
  private final Map<String, Zone> zones;
  private final RelayPositionManager relayManager; // Injected

  /**
   * Get available capacity for a zone (accounting for reservations).
   *
   * <p>Calculation: physical - occupied - reserved
   */
  public int getAvailableCapacity(String zoneId) {
    Zone zone = zones.get(zoneId);
    if (zone == null) return 0;

    int physical = zone.getPhysicalCapacity();
    int occupied = zone.getOccupiedCount();

    // Query relay manager for reserved count
    int reserved = (int) zone.getRelayPositions().stream()
        .filter(r -> relayManager.isReserved(r.getId()))
        .count();

    return physical - occupied - reserved;
  }

  public Zone getZone(String zoneId) { ... }
  public Map<String, Zone> getZones() { ... }
}
```

---

## 5. Back-to-Store Strategies (System-Specific)

### 5.1 Architecture (Strategy Pattern)

```
AbstractBackToStoreSelector (base)
  ├─ RtpBackToStoreSelector (RTP: rack position reservations)
  └─ RelayBackToStoreSelector (Relay: tote slot reservations + relay selection)
```

### 5.2 AbstractBackToStoreSelector

**Common dependencies and utilities:**

```java
public abstract class AbstractBackToStoreSelector {
  protected final WarehouseMap map;
  protected final WarehouseInventory inventory;
  protected final GridPopularityManager gridPopularityManager;
  protected final StorageSpaceManager storageSpaceManager;

  // Common utilities
  public void assignInitialPositions(List<Rack> racks) { ... }
  protected record PositionDistance(GridCoordinate position, double distance) {}
}
```

---

### 5.3 RtpBackToStoreSelector

**Responsibility:** Select and reserve grid positions for RTP rack returns.

#### State

```java
// Rack position reservations (GridCoordinate ↔ RackId)
private final Map<GridCoordinate, RackId> reservedPositions = new ConcurrentHashMap<>();
private final Map<RackId, GridCoordinate> rackReservations = new ConcurrentHashMap<>();
```

#### API

```java
/**
 * Atomically select and reserve optimal grid position for rack return.
 */
public synchronized GridCoordinate findAndReservePositionForRack(Rack rack);

/**
 * Release rack position reservation when rack arrives.
 */
public synchronized void releaseReservation(RackId rackId);

/**
 * Get reserved position for a rack.
 */
public Optional<GridCoordinate> getReservedPosition(RackId rackId);
```

#### Selection Logic

```java
public GridCoordinate getOptimalPositionForRackInRtp(
    Rack rack, Set<GridCoordinate> occupiedPositions) {

  // Rank available positions by distance to stations
  List<PositionDistance> available = new ArrayList<>();
  for (GridCoordinate pos : map.getRackPositions()) {
    if (!occupiedPositions.contains(pos)) {
      double distance = gridPopularityManager.getAverageDistance(pos);
      available.add(new PositionDistance(pos, distance));
    }
  }
  available.sort(Comparator.comparingDouble(PositionDistance::distance));

  // Assign based on popularity percentile (hot racks closer to stations)
  double popularityScore = inventory.calculatePopularityScore(rack);
  int index = (int) (available.size() * (1 - popularityScore));
  index = Math.min(index, available.size() - 1);

  return available.get(index).position();
}
```

---

### 5.4 RelayBackToStoreSelector

**Responsibility:**
- Select zone and relay for HTM tote returns
- Reserve tote slots in racks for VTM return journeys
- Delegate relay position reservations to RelayPositionManager

#### Dependencies

```java
// Additional dependencies beyond AbstractBackToStoreSelector
private final ZoneLoadTracker zoneLoadTracker;
private final ZoneManager zoneManager;
private final RelayPositionManager relayPositionManager; // For relay reservations
```

#### State

```java
// Tote slot reservations (rack slot level)
private final Map<RackId, Set<Integer>> reservedToteSlots = new ConcurrentHashMap<>();
private final Map<ToteId, ToteSlotReservation> toteReservations = new ConcurrentHashMap<>();

public record ToteSlotReservation(RackId rackId, int zIndex) {}
```

#### API

```java
// ========================================
// Relay + Zone Selection
// ========================================

/**
 * Select optimal relay for HTM tote return.
 *
 * <p>Delegates relay reservation to RelayPositionManager.
 */
public synchronized RelayPosition selectRelayForReturn(
    Tote tote, StationId originStationId, GridCoordinate htmPosition);

/**
 * Release relay reservation (delegates to RelayPositionManager).
 */
public synchronized void releaseRelayReservation(ToteId toteId);

// ========================================
// Tote Slot Reservations
// ========================================

/**
 * Reserve specific rack slot for VTM return journey.
 */
public synchronized ToteSlotReservation reserveHardSlot(
    Tote tote, List<Rack> availableRacks);

/**
 * Release rack slot reservation when tote arrives.
 */
public synchronized void releaseHardReservation(ToteId toteId);

/**
 * Get tote slot reservation for a tote.
 */
public Optional<ToteSlotReservation> getToteReservation(ToteId toteId);
```

#### Selection Logic

**Zone Selection:**
```java
public synchronized RelayPosition selectRelayForReturn(
    Tote tote, StationId originStationId, GridCoordinate htmPosition) {

  // Stage 1: Get zones in lowest load bucket
  List<String> lowLoadZones = zoneLoadTracker.getZonesInLowestBucket();

  // Stage 2: Filter by hard criteria (relay capacity, rack space)
  List<String> eligibleZones = filterEligibleZones(lowLoadZones);

  if (eligibleZones.isEmpty()) {
    // Fallback: try all zones
    eligibleZones = filterEligibleZones(new ArrayList<>(zoneManager.getZones().keySet()));
  }

  // Stage 3: Select zone by capacity
  String selectedZone = selectByAdditionalCriteria(eligibleZones, tote, originStationId);

  // Stage 4: Find best relay in zone (nearest to HTM)
  RelayPosition selectedRelay = findBestRelayInZone(selectedZone, htmPosition);

  // Stage 5: Reserve relay (DELEGATES to RelayPositionManager)
  if (!relayPositionManager.tryReserve(selectedRelay.getId(), tote.getId())) {
    throw new IllegalStateException("Failed to reserve relay " + selectedRelay.getId());
  }

  // Stage 6: Update zone load tracking
  zoneLoadTracker.incrementLoad(selectedRelay.getOwnerZoneId());

  return selectedRelay;
}
```

**Filtering Logic:**
```java
private List<String> filterEligibleZones(List<String> zones) {
  List<String> eligible = new ArrayList<>();

  for (String zoneId : zones) {
    Zone zone = zoneManager.getZone(zoneId);

    // Check 1: Zone has available relay capacity (via ZoneManager)
    int availableCapacity = zoneManager.getAvailableCapacity(zoneId);
    if (availableCapacity <= 0) {
      continue;
    }

    // Check 2: Zone has available rack space
    if (!hasAvailableRackSpace(zone)) {
      continue;
    }

    eligible.add(zoneId);
  }

  return eligible;
}
```

**Tote Slot Reservation:**
```java
public synchronized ToteSlotReservation reserveHardSlot(
    Tote tote, List<Rack> availableRacks) {

  // Find rack with free unreserved slot
  for (Rack rack : availableRacks) {
    Set<Integer> reservedSlots = reservedToteSlots.getOrDefault(rack.getId(), Set.of());

    for (int z = 0; z < rack.getMaxToteCapacity(); z++) {
      if (!reservedSlots.contains(z) && rack.getToteAtPosition(z).isEmpty()) {
        // Reserve this slot
        reservedToteSlots
            .computeIfAbsent(rack.getId(), k -> ConcurrentHashMap.newKeySet())
            .add(z);

        ToteSlotReservation reservation = new ToteSlotReservation(rack.getId(), z);
        toteReservations.put(tote.getId(), reservation);

        return reservation;
      }
    }
  }

  throw new IllegalStateException("No free rack slots available for tote " + tote.getId());
}
```

---

## 6. StorageSpaceManager (Updated)

### 6.1 Updated Responsibility

**Location tracking and move orchestration ONLY.** No reservations.

### 6.2 What Was Removed

All reservation methods from original design (those moved to back-to-store selectors).

### 6.3 What Remains

```java
// ========================================
// Location Tracking
// ========================================

ToteLocation getToteLocation(ToteId toteId);
Rack getRack(RackId rackId);
Tote getTote(ToteId toteId);

// ========================================
// Move Orchestration
// ========================================

void moveToteFromRackToRobot(...);
void moveToteFromRobotToRelay(...);
Optional<Tote> moveToteFromRelayToRobot(...);
void moveToteFromRobotToRack(...);

// ========================================
// Query Operations
// ========================================

List<Rack> getAllRacks();
List<Tote> getAllTotes();
List<Rack> findRacksWithCapacity();
List<Tote> getTotesInRack(RackId rackId);
```

---

## 7. Workflows

### 7.1 VTM Forward Journey (Order Fulfillment)

```java
// In DesmojVtmProcess.lifeCycle()

// Stage 1: Select tote from rack
Tote tote = selectToteForForwardJourney(assignedZone);
Rack sourceRack = findRackContainingTote(tote);

// Stage 2: Select relay in zone
List<RelayPosition> available = relayPositionManager.getAvailableInZone(assignedZone);
RelayPosition targetRelay = available.stream()
    .min(Comparator.comparingInt(r -> r.getPosition().manhattanDistance(vtmPosition)))
    .orElseThrow();

// Stage 3: Reserve relay
if (!relayPositionManager.tryReserve(targetRelay.getId(), tote.getId())) {
    // Try next relay
    idle();
    continue;
}

// Stage 4: Execute journey
travelToRack(sourceRack);
storageSpaceManager.moveToteFromRackToRobot(tote.getId(), ...);
travelToRelay(targetRelay);
storageSpaceManager.moveToteFromRobotToRelay(tote.getId(), targetRelay.getId(), targetRelay, this);

// Stage 5: Release reservation
relayPositionManager.releaseReservation(tote.getId());

log.info("VTM {} completed forward journey: tote {} to relay {}",
    robotId, tote.getId(), targetRelay.getId());
```

### 7.2 HTM Return Journey (Back-to-Store)

```java
// In DesmojHtmProcess.lifeCycle()

// Stage 1: Complete picking at station
while (tote.hasTasks()) {
    PickTask task = tote.getNextTaskForStation(currentStation);
    processTask(task);
}

// Stage 2: Select and reserve relay (via RelayBackToStoreSelector)
RelayPosition targetRelay = relayBackToStoreSelector.selectRelayForReturn(
    tote, currentStation, getCurrentPosition());

// Stage 3: Reserve rack slot for eventual VTM return
List<Rack> availableRacks = storageSpaceManager.findRacksWithCapacity();
ToteSlotReservation slotReservation = relayBackToStoreSelector.reserveHardSlot(tote, availableRacks);

// Stage 4: Travel to relay and deliver
travelToRelay(targetRelay);
storageSpaceManager.moveToteFromRobotToRelay(tote.getId(), targetRelay.getId(), targetRelay, this);

// Stage 5: Release relay reservation (HTM part complete)
relayBackToStoreSelector.releaseRelayReservation(tote.getId());

log.info("HTM {} delivered tote {} to relay {} (target rack: {}, slot: {})",
    robotId, tote.getId(), targetRelay.getId(),
    slotReservation.rackId(), slotReservation.zIndex());
```

### 7.3 VTM Return Journey (Relay to Rack)

```java
// In DesmojVtmProcess.lifeCycle() - return journey branch

// Stage 1: Find empty tote at relay in assigned zone
List<RelayPosition> relaysInZone = relayPositionManager.getPositionsByZone(assignedZone);
Optional<Tote> toteToReturn = relaysInZone.stream()
    .flatMap(relay -> relay.getTote().stream())
    .filter(t -> !t.hasTasks())  // Empty totes only
    .filter(t -> t.getAssignedVtmId() == null)  // Not claimed
    .min(Comparator.comparingInt(t -> findClosestRelay(t).getPosition().manhattanDistance(vtmPosition)))
    .map(t -> { t.setAssignedVtmId(robotId); return t; });  // Claim

if (toteToReturn.isEmpty()) {
    idle();
    continue;
}

Tote tote = toteToReturn.get();

// Stage 2: Look up pre-reserved rack slot
ToteSlotReservation slotReservation = relayBackToStoreSelector.getToteReservation(tote.getId())
    .orElseThrow(() -> new IllegalStateException("No slot reservation for tote " + tote.getId()));

Rack targetRack = storageSpaceManager.getRack(slotReservation.rackId());

// Stage 3: Execute return journey
RelayPosition sourceRelay = findRelayContainingTote(tote);
travelToRelay(sourceRelay);
storageSpaceManager.moveToteFromRelayToRobot(tote.getId(), sourceRelay.getId(), sourceRelay, robotId, ...);
travelToRack(targetRack);
storageSpaceManager.moveToteFromRobotToRack(tote.getId(), targetRack.getId(), slotReservation.zIndex());

// Stage 4: Release reservations and claim
relayBackToStoreSelector.releaseHardReservation(tote.getId());
tote.setAssignedVtmId(null);

log.info("VTM {} returned tote {} to rack {} slot {}",
    robotId, tote.getId(), targetRack.getId(), slotReservation.zIndex());
```

### 7.4 RTP Robot Return Journey

```java
// In DesmojRtpRobotProcess.lifeCycle()

// Stage 1: Complete picking at station
while (rack.hasTasks()) {
    PickTask task = rack.getNextTaskForStation(currentStation);
    processTask(task);
}

// Stage 2: Reserve rack position (via RtpBackToStoreSelector)
GridCoordinate targetPosition = rtpBackToStoreSelector.findAndReservePositionForRack(rack);

// Stage 3: Travel to storage position
travelTo(targetPosition);
rack.moveToStorage(targetPosition);

// Stage 4: Release reservation
rtpBackToStoreSelector.releaseReservation(rack.getId());

log.info("RTP Robot {} returned rack {} to position {}",
    robotId, rack.getId(), targetPosition);
```

---

## 8. Thread Safety & Concurrency

### 8.1 Synchronization Strategy

**RelayPositionManager:**
- All public methods `synchronized`
- Prevents concurrent reservation conflicts
- Internal state uses `ConcurrentHashMap` for safe iteration

**Back-to-Store Selectors:**
- All public methods `synchronized`
- Ensures atomic select-and-reserve operations
- Each selector manages its own state independently

**StorageSpaceManager:**
- Move operations `synchronized` on entity level
- Location tracking uses `ConcurrentHashMap`
- No global synchronization needed

### 8.2 Desmo-J Single-Threaded Model

**Note:** Desmo-J simulation is single-threaded (fiber-based), so `synchronized` is primarily for:
- Unit testing (multi-threaded scenarios)
- Code clarity (explicit critical sections)
- Future-proofing for distributed simulation

---

## 9. Error Handling

### 9.1 Reservation Failures

**Relay Unavailable:**
```java
if (!relayPositionManager.tryReserve(relayId, toteId)) {
    log.debug("Relay {} unavailable, trying next", relayId);
    // Select alternative relay
}
```

**No Eligible Zones:**
```java
if (eligibleZones.isEmpty()) {
    throw new IllegalStateException("No eligible zones - system deadlock");
    // In production: trigger backpressure, idle robots
}
```

### 9.2 Double-Booking Prevention

**Application-Level:**
```java
if (reservationsByTote.containsKey(toteId)) {
    throw new IllegalStateException("Tote already has reservation");
}
```

**Database-Level (future):**
```sql
CREATE UNIQUE INDEX idx_tote_reservation ON relay_reservations(tote_id);
```

### 9.3 Orphaned Reservations

**Cleanup:**
```java
// Periodic cleanup (every 5 minutes simulation time)
@Scheduled(fixedRate = 300000)
public void cleanupOrphanedReservations() {
    int cleaned = relayPositionManager.cleanupOrphanedReservations(3600);  // 1 hour
    if (cleaned > 0) {
        log.warn("Cleaned up {} orphaned relay reservations", cleaned);
    }
}
```

---

## 10. Database Schema (Future)

### 10.1 Tables

```sql
-- Relay position reservations
CREATE TABLE relay_reservations (
    id BIGINT PRIMARY KEY AUTO_INCREMENT,
    relay_id VARCHAR(50) NOT NULL,
    tote_id VARCHAR(50) NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    INDEX idx_relay_id (relay_id),
    INDEX idx_tote_id (tote_id)
);

-- Rack position reservations (RTP)
CREATE TABLE rack_position_reservations (
    rack_id VARCHAR(50) PRIMARY KEY,
    grid_x INT NOT NULL,
    grid_y INT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    UNIQUE INDEX idx_position (grid_x, grid_y)
);

-- Tote slot reservations (Relay)
CREATE TABLE tote_slot_reservations (
    tote_id VARCHAR(50) PRIMARY KEY,
    rack_id VARCHAR(50) NOT NULL,
    z_index INT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    INDEX idx_rack_slot (rack_id, z_index)
);
```

---

## 11. Testing Strategy

### 11.1 Unit Tests

```java
@Test
void testRelayPositionManager_preventDoubleBooking() {
    RelayPositionManager manager = new RelayPositionManager();
    ToteId tote1 = new ToteId("T1");

    // Reserve relay R1
    assertTrue(manager.tryReserve("R1", tote1));

    // Attempt to reserve R2 for same tote (should fail)
    assertThrows(IllegalStateException.class, () -> {
        manager.tryReserve("R2", tote1);
    });
}

@Test
void testRtpBackToStoreSelector_atomicReservation() {
    RtpBackToStoreSelector selector = createSelector();
    Rack rack = createTestRack();

    // Reserve position
    GridCoordinate position = selector.findAndReservePositionForRack(rack);

    // Verify reservation
    assertTrue(selector.getReservedPosition(rack.getId()).isPresent());
    assertEquals(position, selector.getReservedPosition(rack.getId()).get());

    // Release
    selector.releaseReservation(rack.getId());
    assertFalse(selector.getReservedPosition(rack.getId()).isPresent());
}

@Test
void testRelayBackToStoreSelector_zoneSelection() {
    RelayBackToStoreSelector selector = createSelector();

    // Setup: Zone A (load=5), Zone B (load=3)
    setupZoneLoads(Map.of("A", 5, "B", 3));

    // Select relay
    RelayPosition relay = selector.selectRelayForReturn(tote, station, htmPos);

    // Should prefer Zone B (lower load)
    assertEquals("B", relay.getOwnerZoneId());
}
```

### 11.2 Integration Tests

```java
@Test
void testFullRelaySystemCycle() {
    // VTM forward journey
    RelayPosition relay = relayManager.getAvailableInZone(vtmZone).get(0);
    assertTrue(relayManager.tryReserve(relay.getId(), tote.getId()));

    // Simulate delivery
    simulateVtmDelivery(tote, relay);
    relayManager.releaseReservation(tote.getId());

    // HTM return journey
    RelayPosition returnRelay = relaySelector.selectRelayForReturn(tote, station, htmPos);
    ToteSlotReservation slotRes = relaySelector.reserveHardSlot(tote, availableRacks);

    // Simulate HTM delivery
    simulateHtmDelivery(tote, returnRelay);
    relaySelector.releaseRelayReservation(tote.getId());

    // VTM return journey
    simulateVtmReturn(tote, slotRes);
    relaySelector.releaseHardReservation(tote.getId());

    // Verify no lingering reservations
    assertEquals(0, relayManager.getTotalReservationCount());
}
```

---

## 12. Migration Plan

### Phase 1: Update RelayPositionManager
**Duration:** 1-2 days

**Tasks:**
- Add reservation state and methods to RelayPositionManager
- Add unit tests
- No changes to callers yet

---

### Phase 2: Update ZoneManager
**Duration:** 1 day

**Tasks:**
- Add `getAvailableCapacity(zoneId)` that queries RelayPositionManager
- Remove `reserved` field from Zone
- Remove `tryReserve()` and `completeReservation()` from Zone
- Update tests

---

### Phase 3: Update RelayBackToStoreSelector
**Duration:** 1-2 days

**Tasks:**
- Change `selectRelayForReturn()` to call `relayPositionManager.tryReserve()` instead of `zone.tryReserve()`
- Add `releaseRelayReservation(ToteId)` method
- Update `filterEligibleZones()` to use `zoneManager.getAvailableCapacity()`
- Update tests

---

### Phase 4: Update Robot Processes
**Duration:** 2-3 days

**Tasks:**
- Update VTM forward journey to use `relayPositionManager.tryReserve/release()`
- Update HTM return journey to use `relayBackToStoreSelector.selectRelayForReturn()`
- Update VTM return journey to use `relayBackToStoreSelector.getToteReservation()`
- Integration tests

---

### Phase 5: Validation
**Duration:** 2-3 days

**Tasks:**
- Run full simulation suite
- Compare metrics with baseline
- Performance benchmarks
- Load testing

---

**Total Estimated Duration:** 7-11 days

---

## 13. Benefits Summary

### 13.1 Design Quality

✅ **Strategy Pattern**: Clean separation of RTP vs Relay logic
✅ **Single Responsibility**: Each component owns one type of reservation
✅ **No Duplicate State**: Reservations tracked in exactly one place
✅ **Natural Cohesion**: RelayPositionManager manages its entities' availability

### 13.2 Correctness

✅ **No Race Conditions**: Atomic select-and-reserve operations
✅ **No Double-Booking**: Validation at application + database level
✅ **Orphan Cleanup**: Automatic detection and cleanup
✅ **Single Source of Truth**: No Zone.reserved vs service inconsistency

### 13.3 Maintainability

✅ **Clear Ownership**: Easy to find where reservations are managed
✅ **Easy to Test**: Service boundaries enable isolated unit tests
✅ **Easy to Extend**: Add new system types via strategy pattern
✅ **Easy to Debug**: Centralized logging per reservation type

---

## 14. References

- **Relay System Design:** `specs/relay_system/design.md`
- **Relay System Implementation:** `specs/relay_system/IMPLEMENTATION_NOTES.md`
- **Java Style Guide:** `docs/JAVA_STYLE_GUIDE.md`
- **Project Instructions:** `CLAUDE.md`

---

**End of Design Document**
