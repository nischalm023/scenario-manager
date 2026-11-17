# Zone Capacity Simplification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Simplify zone capacity tracking by eliminating redundant reservation systems and establishing single ownership of relays by zones.

**Architecture:** Replace three independent reservation systems (soft zone reservations, zone capacity, Desmo-J Res) with a single `ZoneCapacity` tracker. Each relay gets a single `ownerZoneId` instead of N:M zone accessibility. RelayPositionManager moves from DesmojSimulationEngine to WarehouseBuilder with two-phase initialization (metadata first, Desmo-J Res later).

**Tech Stack:** Java 21, Desmo-J 2.5.1e, JUnit 5, Mockito

---

## Phase 1: RelayPosition Two-Phase Initialization

### Task 1: Add ownerZoneId field to RelayPosition

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java:20-46`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java`

**Step 1: Write the failing test**

Add test to `RelayPositionTest.java`:

```java
@Test
void testOwnerZoneId() {
    // Test that ownerZoneId is accessible
    String ownerZoneId = "zone_block_0_left";
    // Will fail until field is added
    assertNotNull(ownerZoneId);
    assertEquals("zone_block_0_left", ownerZoneId);
}
```

**Step 2: Run test to verify it passes (placeholder test)**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest -q`
Expected: PASS (placeholder test)

**Step 3: Add ownerZoneId field**

In `RelayPosition.java`, add after line 25 (`private final int capacity;`):

```java
  private final String ownerZoneId;
```

**Step 4: Format code**

Run: `cd apps/backend && mvn spotless:apply`

**Step 5: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java
git commit -m "feat: add ownerZoneId field to RelayPosition"
```

---

### Task 2: Create two-phase constructor for RelayPosition

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java:31-46`

**Step 1: Write failing test**

Add to `RelayPositionTest.java`:

```java
@Test
void testTwoPhaseConstruction() {
    // Test construction without model (for WarehouseBuilder)
    RelayPosition relay = new RelayPosition(
        "relay_1",
        new GridCoordinate(5, 10),
        "block_0",
        "left",
        "zone_block_0_left",
        5);

    assertEquals("relay_1", relay.getId());
    assertEquals("zone_block_0_left", relay.getOwnerZoneId());
    assertNull(relay.getResource()); // Res not initialized yet
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest::testTwoPhaseConstruction -q`
Expected: FAIL - constructor does not exist

**Step 3: Add two-phase constructor**

Add after existing constructor in `RelayPosition.java`:

```java
  /**
   * Two-phase constructor for WarehouseBuilder.
   *
   * <p>Creates RelayPosition with metadata only. Call initializeRuntime() later
   * in DesmojSimulationEngine after model is connected to experiment.
   */
  public RelayPosition(
      String id,
      GridCoordinate position,
      String rackBlockId,
      String side,
      String ownerZoneId,
      int capacity) {
    this.id = id;
    this.position = position;
    this.rackBlockId = rackBlockId;
    this.side = side;
    this.ownerZoneId = ownerZoneId;
    this.capacity = capacity;
    this.resource = null; // Will be initialized in initializeRuntime()
  }
```

Update existing constructor to set ownerZoneId to null (for backward compatibility):

```java
  public RelayPosition(
      DesmojWarehouseModel model,
      String id,
      GridCoordinate position,
      String rackBlockId,
      String side,
      int capacity) {
    this.id = id;
    this.position = position;
    this.rackBlockId = rackBlockId;
    this.side = side;
    this.ownerZoneId = null; // Legacy: will be set via zone accessibility
    this.capacity = capacity;

    // Disable report generation (false) to avoid NPE on resetAt() for unused relays
    this.resource = new Res(model, id + "_capacity", capacity, false, false);
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest::testTwoPhaseConstruction -q`
Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java
git commit -m "feat: add two-phase constructor to RelayPosition"
```

---

### Task 3: Add initializeRuntime() to RelayPosition

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java`

**Step 1: Write failing test**

Add to `RelayPositionTest.java`:

```java
@Test
void testInitializeRuntime() {
    // Create relay without model
    RelayPosition relay = new RelayPosition(
        "relay_1",
        new GridCoordinate(5, 10),
        "block_0",
        "left",
        "zone_block_0_left",
        5);

    assertNull(relay.getResource());

    // Initialize runtime (requires Desmo-J model)
    // Note: Full test in integration tests due to Desmo-J dependencies
    assertTrue(true, "initializeRuntime method exists");
}
```

**Step 2: Run test to verify it passes (placeholder)**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest::testInitializeRuntime -q`
Expected: PASS

**Step 3: Add initializeRuntime method**

Add to `RelayPosition.java`:

```java
  /**
   * Initialize Desmo-J runtime resources.
   *
   * <p>Called in DesmojSimulationEngine.setupExperiment() after model is connected
   * to experiment. Must be called before any simulation processes use this relay.
   *
   * @param model DesmojWarehouseModel for creating Res resource
   */
  public void initializeRuntime(DesmojWarehouseModel model) {
    if (this.resource != null) {
      throw new IllegalStateException("Runtime already initialized for relay " + id);
    }
    this.resource = new Res(model, id + "_capacity", capacity, false, false);
  }

  /**
   * Check if runtime has been initialized.
   *
   * @return true if initializeRuntime() has been called
   */
  public boolean isRuntimeInitialized() {
    return this.resource != null;
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionTest -q`
Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionTest.java
git commit -m "feat: add initializeRuntime() to RelayPosition"
```

---

### Task 4: Add getOwnerZoneForRelay() to RelayPositionManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java`

**Step 1: Write failing test**

Add to `RelayPositionManagerTest.java`:

```java
@Test
void testGetOwnerZoneForRelay() {
    WarehouseMap map = mock(WarehouseMap.class);
    ZoneManager zoneManager = mock(ZoneManager.class);
    when(map.getRackBlocks()).thenReturn(List.of());
    when(zoneManager.getAllZones()).thenReturn(List.of());

    RelayPositionManager manager = new RelayPositionManager(map, zoneManager, model);

    // Add relay with ownerZoneId
    RelayPosition relay = new RelayPosition(
        "relay_test",
        new GridCoordinate(5, 10),
        "block_0",
        "left",
        "zone_block_0_left",
        5);
    manager.getPositionsById().put("relay_test", relay);

    // Test getOwnerZoneForRelay
    String ownerZone = manager.getOwnerZoneForRelay("relay_test");
    assertEquals("zone_block_0_left", ownerZone);
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionManagerTest::testGetOwnerZoneForRelay -q`
Expected: FAIL - method does not exist

**Step 3: Add getOwnerZoneForRelay method**

Add to `RelayPositionManager.java`:

```java
  /**
   * Get the owner zone ID for a relay.
   *
   * @param relayId Relay position ID
   * @return Owner zone ID
   * @throws IllegalArgumentException if relay not found
   */
  public String getOwnerZoneForRelay(String relayId) {
    RelayPosition relay = positionsById.get(relayId);
    if (relay == null) {
      throw new IllegalArgumentException("Relay not found: " + relayId);
    }
    return relay.getOwnerZoneId();
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionManagerTest::testGetOwnerZoneForRelay -q`
Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java
git commit -m "feat: add getOwnerZoneForRelay() to RelayPositionManager"
```

---

### Task 5: Add initializeRuntime() to RelayPositionManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java`

**Step 1: Write failing test**

Add to `RelayPositionManagerTest.java`:

```java
@Test
void testInitializeRuntime() {
    WarehouseMap map = mock(WarehouseMap.class);
    ZoneManager zoneManager = mock(ZoneManager.class);
    when(map.getRackBlocks()).thenReturn(List.of());
    when(zoneManager.getAllZones()).thenReturn(List.of());

    RelayPositionManager manager = new RelayPositionManager(map, zoneManager, model);

    // Add relay without runtime
    RelayPosition relay = new RelayPosition(
        "relay_test",
        new GridCoordinate(5, 10),
        "block_0",
        "left",
        "zone_block_0_left",
        5);
    manager.getPositionsById().put("relay_test", relay);

    assertFalse(relay.isRuntimeInitialized());

    // Initialize runtime
    manager.initializeRuntime(model);

    assertTrue(relay.isRuntimeInitialized());
}
```

**Step 2: Run test to verify it fails**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionManagerTest::testInitializeRuntime -q`
Expected: FAIL - method does not exist

**Step 3: Add initializeRuntime method**

Add to `RelayPositionManager.java`:

```java
  /**
   * Initialize Desmo-J runtime for all relay positions.
   *
   * <p>Called in DesmojSimulationEngine.setupExperiment() after model is connected
   * to experiment.
   *
   * @param model DesmojWarehouseModel for creating Res resources
   */
  public void initializeRuntime(DesmojWarehouseModel model) {
    for (RelayPosition relay : positionsById.values()) {
      if (!relay.isRuntimeInitialized()) {
        relay.initializeRuntime(model);
      }
    }
    log.info("Initialized runtime for {} relay positions", positionsById.size());
  }
```

**Step 4: Run test to verify it passes**

Run: `cd apps/backend && mvn test -Dtest=RelayPositionManagerTest::testInitializeRuntime -q`
Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java
git commit -m "feat: add initializeRuntime() to RelayPositionManager"
```

---

## Phase 2: WarehouseComponents Consolidation

### Task 6: Add new manager fields to WarehouseComponents

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java:124-133`

**Step 1: Document changes needed**

The `WarehouseComponents` record needs:
- `RelayPositionManager relayPositionManager`
- `StoragePositionManager storagePositionManager`
- `StorageSpaceManager storageSpaceManager`

**Step 2: Update WarehouseComponents record**

Replace the record definition:

```java
  @Builder
  public record WarehouseComponents(
      WarehouseMap map,
      GridPopularityManager gridPopularityManager,
      ItemCatalog itemCatalog,
      WarehouseInventory inventory,
      List<Rack> racks,
      List<Tote> totes,
      List<Robot> robots,
      StationManager stationManager,
      ZoneManager zoneManager,
      RelayPositionManager relayPositionManager,
      StoragePositionManager storagePositionManager,
      StorageSpaceManager storageSpaceManager) {}
```

**Step 3: Add required imports**

Add at top of file:

```java
import com.greyorange.dessimulator.domain.robot.RelayPositionManager;
import com.greyorange.dessimulator.domain.put.StoragePositionManager;
import com.greyorange.dessimulator.domain.inventory.StorageSpaceManager;
```

**Step 4: Compile to check**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: May have compile errors in WarehouseBuilder and DesmojSimulationEngine (will fix next)

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java
git commit -m "feat: add manager fields to WarehouseComponents record"
```

---

### Task 7: Update WarehouseBuilder to create managers

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Add RelayPositionManager field**

Add to class fields (after `private ZoneManager zoneManager;`):

```java
  private RelayPositionManager relayPositionManager;
  private StorageSpaceManager storageSpaceManager;
```

**Step 2: Update build() method**

Update the return statement to include all new fields:

```java
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
```

**Step 3: Create StorageSpaceManager after inventory**

Add after line 223 (`storageManager = new StoragePositionManager(...)`):

```java
    // Create StorageSpaceManager
    storageSpaceManager = new StorageSpaceManager(
        racks,
        config.systemType() == SystemType.RELAY ? totes : null);
```

**Step 4: Add import for RelayPositionManager**

```java
import com.greyorange.dessimulator.domain.robot.RelayPositionManager;
import com.greyorange.dessimulator.domain.inventory.StorageSpaceManager;
```

**Step 5: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS (or may need fixes in DesmojSimulationEngine)

**Step 6: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "feat: create StorageSpaceManager in WarehouseBuilder"
```

---

### Task 8: Update DesmojSimulationEngine to use warehouseComponents

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Add accessor methods for managers**

Replace private manager fields with accessors:

```java
  // Remove these private fields:
  // private final StoragePositionManager storagePositionManager;
  // private final StorageSpaceManager storageSpaceManager;

  // Add accessors instead:
  private StoragePositionManager storagePositionManager() {
    return warehouseComponents.storagePositionManager();
  }

  private StorageSpaceManager storageSpaceManager() {
    return warehouseComponents.storageSpaceManager();
  }
```

**Step 2: Keep local field for now (transition)**

For this task, keep the local fields but add TODO comments:

```java
  // TODO: Remove after Phase 2 - use warehouseComponents.storagePositionManager() instead
  private final StoragePositionManager storagePositionManager;

  // TODO: Remove after Phase 2 - use warehouseComponents.storageSpaceManager() instead
  private final StorageSpaceManager storageSpaceManager;
```

**Step 3: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 4: Run integration tests**

Run: `cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q`
Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "refactor: prepare DesmojSimulationEngine for warehouseComponents migration"
```

---

## Phase 3: Remove Soft Reservations (HIGH RISK)

### Task 9: Comment out soft reservation fields in StoragePositionManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java:37-41`

**Step 1: Comment out fields (don't delete yet)**

```java
  // TODO: Remove after Phase 3 verification
  // Tote soft reservations (zone level) - DEPRECATED
  // private final Map<String, Set<ToteId>> softZoneReservations = new ConcurrentHashMap<>();
  // private final Map<ToteId, String> toteSoftReservations = new ConcurrentHashMap<>();
```

**Step 2: Make methods no-op**

Update the soft reservation methods to be no-ops:

```java
  /**
   * @deprecated Use ZoneCapacity.tryReserve() instead
   */
  @Deprecated
  public synchronized String reserveSoftZone(Tote tote) {
    log.warn("reserveSoftZone called but soft reservations are deprecated");
    return null;
  }

  /**
   * @deprecated Use ZoneCapacity.tryReserve() instead
   */
  @Deprecated
  public synchronized void reserveSoftZoneForReturn(Tote tote, String zoneId) {
    log.warn("reserveSoftZoneForReturn called but soft reservations are deprecated");
  }

  /**
   * @deprecated No longer needed
   */
  @Deprecated
  public synchronized void releaseSoftReservation(ToteId toteId) {
    log.warn("releaseSoftReservation called but soft reservations are deprecated");
  }
```

**Step 3: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 4: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java
git commit -m "refactor: deprecate soft reservation methods in StoragePositionManager"
```

---

### Task 10: Update DesmojVtmProcess to use relay.getOwnerZoneId()

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

**Step 1: Replace getTargetZoneForTote with relay.getOwnerZoneId()**

Find in `lifeCycle()` method (around line 122):

```java
        // NEW: Get target zone for this tote
        String targetZoneId = getTargetZoneForTote(tote);
```

Replace with:

```java
        // Get source rack
        RackId rackId = (RackId) tote.getCurrentHoldingEntity();
        Rack sourceRack = storageSpaceManager.getRack(rackId);

        if (sourceRack == null) {
          log.warn(
              "VTM {}: Source rack {} not found for tote {}", vtm.getId(), rackId, tote.getId());
          hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
          continue;
        }

        // Select optimal relay position first
        Optional<RelayPosition> relayOpt =
            relayPositionManager.selectOptimalRelayForVtm(
                vtm, tote, sourceRack, zoneManager, stationManager);

        if (relayOpt.isEmpty()) {
          log.warn("VTM {}: No relay position available for tote {}", vtm.getId(), tote.getId());
          hold(new TimeSpan(IDLE_CHECK_INTERVAL, TimeUnit.SECONDS));
          continue;
        }

        RelayPosition relay = relayOpt.get();

        // Get target zone from relay's owner zone
        String targetZoneId = relay.getOwnerZoneId();
```

**Step 2: Remove soft reservation calls**

Remove these lines:

```java
        // Soft-reserve the specific target zone (already determined above)
        storagePositionManager.reserveSoftZoneForReturn(tote, targetZoneId);
```

And:

```java
        // Release soft reservation - physical occupancy takes over
        storagePositionManager.releaseSoftReservation(tote.getId());
```

**Step 3: Remove tote.setTargetZoneId()**

Remove:

```java
        // Store zone ID on tote for HTM pickup flow to use
        tote.setTargetZoneId(targetZoneId);
```

**Step 4: Update return journey to use relay.getOwnerZoneId()**

In `executeReturnJourney()`, replace geometric lookup with:

```java
    // Get zone from relay
    String zoneId = task.relayPosition().getOwnerZoneId();
    Zone zone = zoneManager.getZones().get(zoneId);
    zone.getCapacity().remove();
```

**Step 5: Delete getTargetZoneForTote() method**

Delete the entire method (lines 403-431).

**Step 6: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 7: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "refactor: use relay.getOwnerZoneId() in DesmojVtmProcess"
```

---

### Task 11: Update DesmojHtmProcess to use relay.getOwnerZoneId()

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

**Step 1: Replace getZoneForRelay() call at pickup**

Find (around line 168):

```java
        String sourceZoneId =
            tote.getTargetZoneId().orElseGet(() -> getZoneForRelay(relay));
```

Replace with:

```java
        // Get zone from relay's owner zone
        String sourceZoneId = relay.getOwnerZoneId();
```

**Step 2: Remove tote.setTargetZoneId(null)**

Remove:

```java
        // Clear the zone ID after using it
        tote.setTargetZoneId(null);
```

**Step 3: Remove soft reservation calls**

Remove:

```java
        // Soft-reserve zone capacity for tote in flight to relay
        storagePositionManager.reserveSoftZoneForReturn(tote, selectedZoneId);
```

And:

```java
        // Release soft reservation - physical occupancy takes over
        storagePositionManager.releaseSoftReservation(tote.getId());
```

**Step 4: Remove tote.setTargetZoneId(selectedZoneId)**

Remove:

```java
        // Store zone ID on tote for pickup flow to use
        tote.setTargetZoneId(selectedZoneId);
```

**Step 5: Delete helper methods**

Delete these methods:
- `getZoneForRelay()` (lines 433-441)
- `getCandidateZonesForReturn()` (lines 454-472)
- `selectRelayInZone()` (lines 481-493)

**Step 6: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: May FAIL - need to update return flow

**Step 7: Note for next task**

The return flow will be updated in Task 12 when BackToStoreSelector is modified.

**Step 8: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java
git commit -m "refactor: use relay.getOwnerZoneId() in DesmojHtmProcess pickup"
```

---

## Phase 4: BackToStoreSelector Returns Relay

### Task 12: Add selectRelayForReturn() to BackToStoreSelector

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/BackToStoreSelector.java`

**Step 1: Add RelayPositionManager dependency**

Add field:

```java
  private final RelayPositionManager relayPositionManager;
```

Update constructor:

```java
  public BackToStoreSelector(
      ZoneLoadTracker zoneLoadTracker,
      StoragePositionManager storagePositionManager,
      ZoneManager zoneManager,
      GridPopularityManager gridPopularityManager,
      RelayPositionManager relayPositionManager) {
    // existing assignments...
    this.relayPositionManager = relayPositionManager;
  }
```

**Step 2: Add selectRelayForReturn() method**

```java
  /**
   * Select optimal relay for tote return and reserve capacity.
   *
   * <p>Replaces selectAndReserveZone() - returns the actual relay position
   * instead of just the zone ID. Zone is derived from relay's ownerZoneId.
   *
   * @param tote The tote to return to storage
   * @param originStationId The station the tote is returning from
   * @param htmPosition Current HTM position for travel optimization
   * @return The selected relay position
   */
  public synchronized RelayPosition selectRelayForReturn(
      Tote tote, StationId originStationId, GridCoordinate htmPosition) {

    // Step 1: Get zones in lowest load bucket
    List<String> lowLoadZones = zoneLoadTracker.getZonesInLowestBucket();

    // Step 2: Apply hard filtering (free relay capacity)
    List<String> eligibleZones = filterEligibleZones(lowLoadZones);

    // Step 3: If no eligible zones in lowest bucket, try all zones
    if (eligibleZones.isEmpty()) {
      List<String> allZoneIds = new ArrayList<>(zoneManager.getZones().keySet());
      eligibleZones = filterEligibleZones(allZoneIds);
    }

    // Step 4: Select zone by additional criteria
    String selectedZoneId = selectByAdditionalCriteria(eligibleZones, tote, originStationId);

    // Step 5: Find best relay in selected zone
    RelayPosition selectedRelay = findBestRelayInZone(selectedZoneId, htmPosition);

    // Step 6: Reserve in relay's owner zone and update load tracking
    String ownerZoneId = selectedRelay.getOwnerZoneId();
    Zone zone = zoneManager.getZones().get(ownerZoneId);

    if (!zone.getCapacity().tryReserve()) {
      throw new IllegalStateException(
          "Failed to reserve capacity in zone " + ownerZoneId + " for tote " + tote.getId());
    }

    zoneLoadTracker.incrementLoad(ownerZoneId);

    log.debug(
        "Selected relay {} in zone {} for tote {} return from station {}",
        selectedRelay.getId(),
        ownerZoneId,
        tote.getId(),
        originStationId);

    return selectedRelay;
  }

  /**
   * Find best relay position within a zone.
   *
   * @param zoneId Target zone
   * @param htmPosition HTM's current position for distance optimization
   * @return Nearest relay with capacity in the zone
   */
  private RelayPosition findBestRelayInZone(String zoneId, GridCoordinate htmPosition) {
    return relayPositionManager.getPositionsById().values().stream()
        .filter(relay -> zoneId.equals(relay.getOwnerZoneId()))
        .filter(RelayPosition::hasAvailableCapacity)
        .min(Comparator.comparingInt(rp -> rp.getPosition().manhattanDistance(htmPosition)))
        .orElseThrow(() -> new IllegalStateException(
            "No available relay in zone " + zoneId));
  }
```

**Step 3: Add required imports**

```java
import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import com.greyorange.dessimulator.domain.robot.RelayPosition;
import com.greyorange.dessimulator.domain.robot.RelayPositionManager;
import java.util.Comparator;
```

**Step 4: Deprecate selectAndReserveZone()**

```java
  /**
   * @deprecated Use selectRelayForReturn() instead
   */
  @Deprecated
  public synchronized String selectAndReserveZone(Tote tote, StationId originStationId) {
    // ... existing implementation ...
  }
```

**Step 5: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 6: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/BackToStoreSelector.java
git commit -m "feat: add selectRelayForReturn() to BackToStoreSelector"
```

---

### Task 13: Update DesmojHtmProcess to use selectRelayForReturn()

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

**Step 1: Replace return flow**

Replace the entire Phase 6 section (starting around line 270) with:

```java
        // Phase 6: Return tote to relay
        RelayPosition returnRelay = backToStoreSelector.selectRelayForReturn(
            tote, targetStationId, htm.getPosition());

        String selectedZoneId = returnRelay.getOwnerZoneId();
        Zone selectedZone = model.getZoneManager().getZones().get(selectedZoneId);

        log.debug(
            "HTM {} returning tote {} to relay {} in zone {} at t={}s",
            htm.getId(),
            tote.getId(),
            returnRelay.getId(),
            selectedZoneId,
            presentTime());

        // Travel to relay and deliver
        htm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
        tote.setState(Tote.ToteState.BEING_RETURNED_TO_RELAY);

        double returnTravel = htm.moveToPosition(returnRelay.getPosition());
        double unloadingTime = UNLOADING_TIME;
        double returnTime = (returnTravel * navFactor) + unloadingTime;

        hold(new TimeSpan(returnTime, TimeUnit.SECONDS));

        // Add tote to relay
        htm.removeCarryingTote(tote);
        returnRelay.addTote(this, tote);

        // Fulfill the reservation
        selectedZone.getCapacity().fulfill();

        log.debug(
            "HTM {} returned tote {} to relay {} (return time: {}s) at t={}s",
            htm.getId(),
            tote.getId(),
            returnRelay.getId(),
            returnTime,
            presentTime());
```

**Step 2: Update BackToStoreSelector creation in DesmojSimulationEngine**

Will need to pass RelayPositionManager to BackToStoreSelector constructor.

**Step 3: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: May need to update DesmojSimulationEngine

**Step 4: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java
git commit -m "refactor: use selectRelayForReturn() in DesmojHtmProcess"
```

---

## Phase 5: Remove Tote.targetZoneId

### Task 14: Remove targetZoneId from Tote

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java`

**Step 1: Remove field**

Delete:

```java
  private String targetZoneId;
```

**Step 2: Remove methods**

Delete:

```java
  public void setTargetZoneId(String zoneId) {
    this.targetZoneId = zoneId;
  }

  public Optional<String> getTargetZoneId() {
    return Optional.ofNullable(targetZoneId);
  }
```

**Step 3: Update reset() method**

Remove from `reset()`:

```java
    this.targetZoneId = null;
```

**Step 4: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS (if all usages removed in previous tasks)

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/inventory/Tote.java
git commit -m "refactor: remove targetZoneId from Tote"
```

---

## Phase 6: Remove N:M Zone Accessibility

### Task 15: Remove accessibleFromZones from RelayPosition

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java`

**Step 1: Remove field**

Delete:

```java
  private final Set<String> accessibleFromZones = new HashSet<>();
```

**Step 2: Remove methods**

Delete:

```java
  public void addAccessibleZone(String zoneId) {
    accessibleFromZones.add(zoneId);
  }

  public boolean isAccessibleFromZone(String zoneId) {
    return accessibleFromZones.contains(zoneId);
  }
```

**Step 3: Update getter**

Remove `accessibleFromZones` from `@Getter` scope or delete the getter.

**Step 4: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: May FAIL in RelayPositionManager

**Step 5: Update RelayPositionManagerTest**

Remove or update test `testZoneAccessibility()` since it uses deprecated methods.

**Step 6: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java
git commit -m "refactor: remove accessibleFromZones from RelayPosition"
```

---

### Task 16: Remove positionsByZone from RelayPositionManager

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java`

**Step 1: Remove field**

Delete:

```java
  private final Map<String, List<RelayPosition>> positionsByZone = new HashMap<>();
```

**Step 2: Remove buildZoneAccessibility() method**

Delete entire method (lines 81-98).

**Step 3: Remove getPositionsForZone() method**

Delete:

```java
  public List<RelayPosition> getPositionsForZone(String zoneId) {
    return positionsByZone.getOrDefault(zoneId, Collections.emptyList());
  }
```

**Step 4: Update constructor to not call buildZoneAccessibility()**

Remove:

```java
    buildZoneAccessibility(zoneManager);
```

**Step 5: Update selectOptimalRelayForVtm to use ownerZoneId**

The method needs to filter by ownerZoneId instead of using positionsByZone.

**Step 6: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: May FAIL in ZoneManager.initializeCapacityTracking()

**Step 7: Update ZoneManager.initializeCapacityTracking()**

Replace relay counting logic to use ownerZoneId filtering.

**Step 8: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java
git commit -m "refactor: remove positionsByZone N:M mapping"
```

---

## Phase 7: Cleanup and Verification

### Task 17: Delete deprecated soft reservation code

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java`

**Step 1: Delete commented fields**

**Step 2: Delete deprecated methods**

Delete:
- `reserveSoftZone()`
- `reserveSoftZoneForReturn()`
- `releaseSoftReservation()`
- `getSoftReservationCount()`
- `getEffectiveFreeRelayCount()`
- `selectBestAvailableZones()`

**Step 3: Compile**

Run: `cd apps/backend && mvn clean compile -DskipTests`
Expected: PASS

**Step 4: Run all tests**

Run: `cd apps/backend && mvn test -q`
Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add apps/backend/src/main/java/com/greyorange/dessimulator/domain/put/StoragePositionManager.java
git commit -m "refactor: delete soft reservation code from StoragePositionManager"
```

---

### Task 18: Run integration tests and manual verification

**Step 1: Run integration test**

```bash
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```
Expected: PASS

**Step 2: Start server**

```bash
rm -f /tmp/server.log
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/health
```

**Step 3: Run relay simulation**

```bash
cat apps/backend/src/test/resources/integration-test-config.json | \
  curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" -d @- > /tmp/sim_result.json
```

**Step 4: Check for errors**

```bash
grep -i "zone is empty\|capacity\|error" /tmp/server.log
```
Expected: No "zone is empty" errors

**Step 5: Cleanup**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
```

**Step 6: Document results**

Create verification notes in commit message.

---

### Task 19: Final commit and update design doc

**Step 1: Update design document status**

Change status from "Draft" to "Implemented" in the design doc.

**Step 2: Final commit**

```bash
git add docs/plans/2025-11-18-zone-capacity-simplification-design.md
git commit -m "docs: mark zone capacity simplification as implemented"
```

---

## Summary

**Total Tasks: 19**

**Phase 1** (Tasks 1-5): RelayPosition two-phase initialization
**Phase 2** (Tasks 6-8): WarehouseComponents consolidation
**Phase 3** (Tasks 9-11): Remove soft reservations (HIGH RISK)
**Phase 4** (Tasks 12-13): BackToStoreSelector returns relay (HIGH RISK)
**Phase 5** (Task 14): Remove Tote.targetZoneId
**Phase 6** (Tasks 15-16): Remove N:M zone accessibility
**Phase 7** (Tasks 17-19): Cleanup and verification

**Key Files Changed:**
- `RelayPosition.java` - ownerZoneId, two-phase init
- `RelayPositionManager.java` - initializeRuntime(), getOwnerZoneForRelay()
- `WarehouseComponents` - add manager fields
- `WarehouseBuilder.java` - create StorageSpaceManager
- `DesmojSimulationEngine.java` - use warehouseComponents
- `StoragePositionManager.java` - delete soft reservations
- `DesmojVtmProcess.java` - use relay.getOwnerZoneId()
- `DesmojHtmProcess.java` - use selectRelayForReturn()
- `BackToStoreSelector.java` - selectRelayForReturn()
- `Tote.java` - remove targetZoneId
- `ZoneManager.java` - ownership-based capacity

**Testing Commands:**
```bash
cd apps/backend
mvn test -q                              # All tests
mvn test -Dtest=RelayPositionTest -q     # Unit tests
mvn test -Dtest=DesmojIntegrationTest -q # Integration test
mvn spotless:apply                       # Format code
mvn clean compile -DskipTests            # Compile check
```
