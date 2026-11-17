# Zone Aisle-Based Model Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor zone generation from per-block (2 zones per block) to per-aisle model (1 zone per physical aisle).

**Architecture:** Zones represent physical aisles between rack blocks. Each zone stores leftBlockId/rightBlockId and services specific sides of adjacent blocks. Relay positions are claimed by zones based on (blockId, side) matching.

**Tech Stack:** Java 21, JUnit 5, Mockito, Maven

---

## Task 1: Update Zone Data Model

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java`

**Step 1: Write failing test for new Zone constructor**

Add to `ZoneTest.java`:

```java
@Test
void testZoneCreationWithBlockIds() {
    List<GridCoordinate> coverage = List.of(new GridCoordinate(0, 5));
    List<GridCoordinate> servicedRacks = List.of(new GridCoordinate(0, 4), new GridCoordinate(0, 6));

    Zone zone = new Zone("zone_block_0_block_1", "aisle", "block_0", "block_1", coverage, servicedRacks);

    assertEquals("zone_block_0_block_1", zone.getZoneId());
    assertEquals("aisle", zone.getZoneType());
    assertEquals("block_0", zone.getLeftBlockId());
    assertEquals("block_1", zone.getRightBlockId());
    assertEquals(1, zone.getCoverageArea().size());
    assertEquals(2, zone.getServicedRackCoordinates().size());
}

@Test
void testZoneCreationWithNullLeftBlock() {
    Zone zone = new Zone("zone_null_block_0", "aisle", null, "block_0", List.of(), List.of());

    assertNull(zone.getLeftBlockId());
    assertEquals("block_0", zone.getRightBlockId());
}

@Test
void testZoneCreationWithNullRightBlock() {
    Zone zone = new Zone("zone_block_6_null", "aisle", "block_6", null, List.of(), List.of());

    assertEquals("block_6", zone.getLeftBlockId());
    assertNull(zone.getRightBlockId());
}
```

**Step 2: Run test to verify it fails**

```bash
cd apps/backend && mvn test -Dtest=ZoneTest#testZoneCreationWithBlockIds -q
```

Expected: FAIL - constructor doesn't exist

**Step 3: Update Zone class with new fields and constructor**

Replace in `Zone.java`:

```java
package com.greyorange.dessimulator.domain.robot;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import java.util.*;
import lombok.Getter;

/**
 * Represents a zone within the warehouse that robots (typically VTMs) are assigned to.
 *
 * <p>Zones own their relay positions and track reservation state for capacity management.
 * Each zone represents a physical aisle between two rack blocks.
 */
@Getter
public class Zone {
  private final String zoneId;
  private final String zoneType;
  private final String leftBlockId;  // nullable for left edge
  private final String rightBlockId; // nullable for right edge
  private final List<GridCoordinate> coverageArea;
  private final List<GridCoordinate> servicedRackCoordinates;
  private final Set<Integer> assignedVtmIds;
  private final List<RelayPosition> relayPositions = new ArrayList<>();
  private int reserved = 0;

  /**
   * Create a zone representing an aisle between two blocks.
   *
   * @param zoneId Unique zone identifier
   * @param zoneType Type of zone (typically "aisle")
   * @param leftBlockId Block ID on the left (null for left edge)
   * @param rightBlockId Block ID on the right (null for right edge)
   * @param coverageArea Aisle positions covered by this zone
   * @param servicedRackCoordinates Rack positions serviced by this zone
   */
  public Zone(
      String zoneId,
      String zoneType,
      String leftBlockId,
      String rightBlockId,
      List<GridCoordinate> coverageArea,
      List<GridCoordinate> servicedRackCoordinates) {
    this.zoneId = zoneId;
    this.zoneType = zoneType;
    this.leftBlockId = leftBlockId;
    this.rightBlockId = rightBlockId;
    this.coverageArea = coverageArea != null ? new ArrayList<>(coverageArea) : new ArrayList<>();
    this.servicedRackCoordinates =
        servicedRackCoordinates != null
            ? new ArrayList<>(servicedRackCoordinates)
            : new ArrayList<>();
    this.assignedVtmIds = new HashSet<>();
  }

  /**
   * Legacy constructor for backward compatibility.
   *
   * @deprecated Use constructor with leftBlockId/rightBlockId instead
   */
  @Deprecated
  public Zone(
      String zoneId,
      String zoneType,
      List<String> rackBlockIds,
      List<GridCoordinate> coverageArea,
      List<GridCoordinate> servicedRackCoordinates) {
    this.zoneId = zoneId;
    this.zoneType = zoneType;
    // Convert legacy format: assume first block is left, second is right
    this.leftBlockId = rackBlockIds != null && !rackBlockIds.isEmpty() ? rackBlockIds.get(0) : null;
    this.rightBlockId = rackBlockIds != null && rackBlockIds.size() > 1 ? rackBlockIds.get(1) : null;
    this.coverageArea = coverageArea != null ? new ArrayList<>(coverageArea) : new ArrayList<>();
    this.servicedRackCoordinates =
        servicedRackCoordinates != null
            ? new ArrayList<>(servicedRackCoordinates)
            : new ArrayList<>();
    this.assignedVtmIds = new HashSet<>();
  }

  /** Add a relay position to this zone. */
  public void addRelayPosition(RelayPosition relay) {
    relayPositions.add(relay);
  }

  /** Get unmodifiable list of relay positions. */
  public List<RelayPosition> getRelayPositions() {
    return Collections.unmodifiableList(relayPositions);
  }

  /** Count occupied relay positions (have a tote). */
  public int countOccupied() {
    return (int) relayPositions.stream().filter(r -> r.getTote().isPresent()).count();
  }

  /** Get physical capacity (number of relay positions). */
  public int getPhysicalCapacity() {
    return relayPositions.size();
  }

  /** Get available capacity for new reservations. */
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

  /** Complete reservation when tote arrives at relay. */
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
    dict.put("left_block_id", leftBlockId);
    dict.put("right_block_id", rightBlockId);
    dict.put("assigned_vtm_ids", new ArrayList<>(assignedVtmIds));
    dict.put("vtm_count", assignedVtmIds.size());
    dict.put("coverage_area_size", coverageArea.size());
    dict.put("serviced_racks_count", servicedRackCoordinates.size());
    dict.put("relay_count", relayPositions.size());
    return dict;
  }
}
```

**Step 4: Run tests to verify they pass**

```bash
cd apps/backend && mvn test -Dtest=ZoneTest -q
```

Expected: All tests PASS

**Step 5: Update existing tests in ZoneTest.java to use new constructor**

Update the `setUp()` method and any tests that use the old constructor:

```java
@BeforeEach
void setUp() {
  zone =
      new Zone(
          "zone_1",
          "aisle",
          "block_0",  // leftBlockId
          null,       // rightBlockId (edge zone)
          List.of(new GridCoordinate(0, 0)),
          List.of(new GridCoordinate(1, 1)));
}
```

Update other tests that create zones manually:

```java
@Test
void testZoneCreation() {
  List<GridCoordinate> coverage =
      List.of(new GridCoordinate(0, 5), new GridCoordinate(1, 5), new GridCoordinate(2, 5));

  Zone zone = new Zone("zone_0", "aisle", "block_0", "block_1", coverage, List.of());

  assertEquals("zone_0", zone.getZoneId());
  assertEquals("aisle", zone.getZoneType());
  assertEquals(3, zone.getCoverageArea().size());
}

@Test
void testContainsPosition() {
  List<GridCoordinate> coverage = List.of(new GridCoordinate(5, 10));
  Zone zone = new Zone("zone_0", "aisle", null, "block_0", coverage, List.of());

  assertTrue(zone.containsPosition(new GridCoordinate(5, 10)));
  assertFalse(zone.containsPosition(new GridCoordinate(6, 10)));
}

@Test
void testVtmAssignment() {
  Zone zone = new Zone("zone_0", "aisle", "block_0", "block_1", List.of(), List.of());

  zone.addVtm(1);
  zone.addVtm(2);

  assertTrue(zone.getAssignedVtmIds().contains(1));
  assertTrue(zone.getAssignedVtmIds().contains(2));

  zone.removeVtm(1);
  assertFalse(zone.getAssignedVtmIds().contains(1));
}

@Test
void testRelayRegistrationFlow() {
  // Simulate the full registration flow
  Zone testZone =
      new Zone(
          "zone_1",
          "aisle",
          "block_0",
          null,
          List.of(new GridCoordinate(0, 0)),
          List.of(new GridCoordinate(1, 1)));

  // ... rest of test remains the same
}
```

**Step 6: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/Zone.java src/test/java/com/greyorange/dessimulator/domain/robot/ZoneTest.java
git commit -m "feat(zone): add leftBlockId/rightBlockId fields to Zone"
```

---

## Task 2: Update RelayPosition for Mutable ownerZoneId

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java`
- Create: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionOwnerZoneTest.java`

**Step 1: Write failing test for mutable ownerZoneId**

Create `RelayPositionOwnerZoneTest.java`:

```java
package com.greyorange.dessimulator.domain.robot;

import static org.junit.jupiter.api.Assertions.*;

import com.greyorange.dessimulator.domain.common.coordinates.GridCoordinate;
import org.junit.jupiter.api.Test;

class RelayPositionOwnerZoneTest {

  @Test
  void testSetOwnerZoneId() {
    RelayPosition relay =
        new RelayPosition("relay_1", new GridCoordinate(5, 10), "block_0", "right", null);

    assertNull(relay.getOwnerZoneId());

    relay.setOwnerZoneId("zone_block_0_block_1");

    assertEquals("zone_block_0_block_1", relay.getOwnerZoneId());
  }

  @Test
  void testCreateRelayWithoutOwnerZone() {
    RelayPosition relay =
        new RelayPosition("relay_1", new GridCoordinate(5, 10), "block_0", "left", null);

    assertEquals("relay_1", relay.getId());
    assertEquals("block_0", relay.getRackBlockId());
    assertEquals("left", relay.getSide());
    assertNull(relay.getOwnerZoneId());
  }
}
```

**Step 2: Run test to verify it fails**

```bash
cd apps/backend && mvn test -Dtest=RelayPositionOwnerZoneTest -q
```

Expected: FAIL - no setOwnerZoneId method

**Step 3: Make ownerZoneId mutable in RelayPosition**

In `RelayPosition.java`, change the field from final to mutable and add setter:

```java
// Change this line (around line 26):
private final String ownerZoneId;

// To:
private String ownerZoneId;

// Add setter method after getters (around line 80):
/**
 * Set the owner zone ID.
 *
 * <p>Called by ZoneManager when claiming relay positions during registration.
 *
 * @param ownerZoneId The zone ID that owns this relay
 */
public void setOwnerZoneId(String ownerZoneId) {
  this.ownerZoneId = ownerZoneId;
}
```

**Step 4: Run tests to verify they pass**

```bash
cd apps/backend && mvn test -Dtest=RelayPositionOwnerZoneTest -q
```

Expected: All tests PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/RelayPosition.java src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionOwnerZoneTest.java
git commit -m "feat(relay): make ownerZoneId mutable with setter"
```

---

## Task 3: Update RelayPositionManager to Create Relays Without ownerZoneId

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java`

**Step 1: Write failing test for relays created without ownerZoneId**

Add to `RelayPositionManagerTest.java`:

```java
@Test
void testRelaysCreatedWithoutOwnerZoneId() {
    WarehouseMap map = mock(WarehouseMap.class);
    RackBlockGridDefinition block =
        new RackBlockGridDefinition(
            "block_0",
            new GridCoordinate(0, 1),
            new GridCoordinate(2, 2),
            3,
            List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block));

    RelayPositionManager manager = new RelayPositionManager(map);

    // All relays should have null ownerZoneId initially
    for (RelayPosition relay : manager.getPositionsById().values()) {
        assertNull(relay.getOwnerZoneId(), "Relay " + relay.getId() + " should have null ownerZoneId");
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cd apps/backend && mvn test -Dtest=RelayPositionManagerTest#testRelaysCreatedWithoutOwnerZoneId -q
```

Expected: FAIL - ownerZoneId is not null

**Step 3: Update RelayPositionManager to pass null for ownerZoneId**

In `RelayPositionManager.java`, update `generateRelayPositionsMetadataOnly()` method (around line 71-111):

```java
private void generateRelayPositionsMetadataOnly() {
  for (RackBlockGridDefinition block : warehouseMap.getRackBlocks()) {
    List<RelayPosition> blockRelays = new ArrayList<>();

    int leftCol = block.topLeft().col() - 1;
    int rightCol = block.bottomRight().col() + 1;

    for (int row = block.topLeft().row(); row <= block.bottomRight().row(); row++) {
      // Left relay - ownerZoneId will be set by ZoneManager
      if (leftCol >= 0) {
        GridCoordinate leftCoord = new GridCoordinate(row, leftCol);
        RelayPosition leftRelay =
            new RelayPosition(
                "relay_" + block.blockId() + "_L_r" + row,
                leftCoord,
                block.blockId(),
                "left",
                null);  // Changed from leftOwnerZoneId to null
        blockRelays.add(leftRelay);
        positionsById.put(leftRelay.getId(), leftRelay);
      }

      // Right relay - ownerZoneId will be set by ZoneManager
      GridCoordinate rightCoord = new GridCoordinate(row, rightCol);
      RelayPosition rightRelay =
          new RelayPosition(
              "relay_" + block.blockId() + "_R_r" + row,
              rightCoord,
              block.blockId(),
              "right",
              null);  // Changed from rightOwnerZoneId to null
      blockRelays.add(rightRelay);
      positionsById.put(rightRelay.getId(), rightRelay);
    }

    positionsByBlock.put(block.blockId(), blockRelays);
  }

  log.info("Generated {} relay positions (metadata only, no zone assignment)", positionsById.size());
}
```

**Step 4: Run tests to verify they pass**

```bash
cd apps/backend && mvn test -Dtest=RelayPositionManagerTest -q
```

Expected: All tests PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/RelayPositionManager.java src/test/java/com/greyorange/dessimulator/domain/robot/RelayPositionManagerTest.java
git commit -m "feat(relay-manager): create relays without ownerZoneId"
```

---

## Task 4: Implement New Zone Generation Algorithm

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java`

**Step 1: Write failing tests for new zone generation**

Add to `ZoneManagerTest.java`:

```java
@Test
void testGenerateZonesForSingleRowOfBlocks() {
    WarehouseMap map = mock(WarehouseMap.class);

    // Create 3 blocks in a row (columns 1-2, 4-5, 7-8)
    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(2, 2), 3, List.of());
    RackBlockGridDefinition block1 = new RackBlockGridDefinition(
        "block_1", new GridCoordinate(0, 4), new GridCoordinate(2, 5), 3, List.of());
    RackBlockGridDefinition block2 = new RackBlockGridDefinition(
        "block_2", new GridCoordinate(0, 7), new GridCoordinate(2, 8), 3, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0, block1, block2));

    ZoneManager manager = new ZoneManager(map);
    manager.generateZones();

    // Should create 4 zones: null-0, 0-1, 1-2, 2-null
    assertEquals(4, manager.getAllZones().size());

    // Verify zone names
    assertNotNull(manager.getZones().get("zone_null_block_0"));
    assertNotNull(manager.getZones().get("zone_block_0_block_1"));
    assertNotNull(manager.getZones().get("zone_block_1_block_2"));
    assertNotNull(manager.getZones().get("zone_block_2_null"));
}

@Test
void testGenerateZonesForTwoRowsOfBlocks() {
    WarehouseMap map = mock(WarehouseMap.class);

    // Create 2x2 layout (4 blocks total)
    // Row 0 (bottom): blocks 0, 1
    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(2, 2), 3, List.of());
    RackBlockGridDefinition block1 = new RackBlockGridDefinition(
        "block_1", new GridCoordinate(0, 4), new GridCoordinate(2, 5), 3, List.of());
    // Row 1 (top): blocks 2, 3
    RackBlockGridDefinition block2 = new RackBlockGridDefinition(
        "block_2", new GridCoordinate(5, 1), new GridCoordinate(7, 2), 3, List.of());
    RackBlockGridDefinition block3 = new RackBlockGridDefinition(
        "block_3", new GridCoordinate(5, 4), new GridCoordinate(7, 5), 3, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0, block1, block2, block3));

    ZoneManager manager = new ZoneManager(map);
    manager.generateZones();

    // Should create 6 zones: 3 per row
    // Row 0: null-0, 0-1, 1-null
    // Row 1: null-2, 2-3, 3-null
    assertEquals(6, manager.getAllZones().size());
}

@Test
void testZoneHasCorrectBlockIds() {
    WarehouseMap map = mock(WarehouseMap.class);

    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(2, 2), 3, List.of());
    RackBlockGridDefinition block1 = new RackBlockGridDefinition(
        "block_1", new GridCoordinate(0, 4), new GridCoordinate(2, 5), 3, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0, block1));

    ZoneManager manager = new ZoneManager(map);
    manager.generateZones();

    Zone middleZone = manager.getZones().get("zone_block_0_block_1");
    assertNotNull(middleZone);
    assertEquals("block_0", middleZone.getLeftBlockId());
    assertEquals("block_1", middleZone.getRightBlockId());

    Zone leftEdge = manager.getZones().get("zone_null_block_0");
    assertNotNull(leftEdge);
    assertNull(leftEdge.getLeftBlockId());
    assertEquals("block_0", leftEdge.getRightBlockId());
}
```

**Step 2: Run tests to verify they fail**

```bash
cd apps/backend && mvn test -Dtest=ZoneManagerTest#testGenerateZonesForSingleRowOfBlocks -q
```

Expected: FAIL - wrong number of zones or wrong names

**Step 3: Implement new generateZones algorithm**

Replace the `generateZones()` and related methods in `ZoneManager.java`:

```java
/**
 * Generate zones based on aisles between rack blocks.
 *
 * <p>Creates one zone per physical aisle. Each zone services the right side of the left block
 * and the left side of the right block.
 */
public void generateZones() {
  // Group blocks by row (blocks with overlapping row ranges)
  Map<Integer, List<RackBlockGridDefinition>> blocksByRow = groupBlocksByRow(map.getRackBlocks());

  for (List<RackBlockGridDefinition> rowBlocks : blocksByRow.values()) {
    if (rowBlocks.isEmpty()) continue;

    // Sort blocks by column position (left to right)
    rowBlocks.sort(Comparator.comparingInt(b -> b.topLeft().col()));

    // Left edge zone
    createZone(null, rowBlocks.get(0));

    // Between zones
    for (int i = 0; i < rowBlocks.size() - 1; i++) {
      createZone(rowBlocks.get(i), rowBlocks.get(i + 1));
    }

    // Right edge zone
    createZone(rowBlocks.get(rowBlocks.size() - 1), null);
  }

  log.info("Generated {} zones for {} rack blocks", zones.size(), map.getRackBlocks().size());
}

/**
 * Group blocks by their row range.
 *
 * <p>Blocks with overlapping row coordinates are in the same row.
 */
private Map<Integer, List<RackBlockGridDefinition>> groupBlocksByRow(
    List<RackBlockGridDefinition> blocks) {
  Map<Integer, List<RackBlockGridDefinition>> result = new HashMap<>();

  for (RackBlockGridDefinition block : blocks) {
    // Use topLeft row as the row key (blocks in same row have same topLeft.row)
    int rowKey = block.topLeft().row();
    result.computeIfAbsent(rowKey, k -> new ArrayList<>()).add(block);
  }

  return result;
}

/**
 * Create a zone for an aisle between two blocks.
 *
 * @param leftBlock Block on the left (null for left edge)
 * @param rightBlock Block on the right (null for right edge)
 */
private void createZone(RackBlockGridDefinition leftBlock, RackBlockGridDefinition rightBlock) {
  String leftId = leftBlock != null ? leftBlock.blockId() : "null";
  String rightId = rightBlock != null ? rightBlock.blockId() : "null";
  String zoneId = "zone_" + leftId + "_" + rightId;

  // Calculate coverage area (aisle column)
  List<GridCoordinate> coverageArea = new ArrayList<>();
  int aisleCol;
  int startRow, endRow;

  if (leftBlock != null && rightBlock != null) {
    // Between two blocks - aisle is to the right of left block
    aisleCol = leftBlock.bottomRight().col() + 1;
    startRow = Math.min(leftBlock.topLeft().row(), rightBlock.topLeft().row());
    endRow = Math.max(leftBlock.bottomRight().row(), rightBlock.bottomRight().row());
  } else if (leftBlock != null) {
    // Right edge - aisle is to the right of the block
    aisleCol = leftBlock.bottomRight().col() + 1;
    startRow = leftBlock.topLeft().row();
    endRow = leftBlock.bottomRight().row();
  } else {
    // Left edge - aisle is to the left of the block
    aisleCol = rightBlock.topLeft().col() - 1;
    startRow = rightBlock.topLeft().row();
    endRow = rightBlock.bottomRight().row();
  }

  for (int row = startRow; row <= endRow; row++) {
    coverageArea.add(new GridCoordinate(row, aisleCol));
  }

  // Calculate serviced rack coordinates
  List<GridCoordinate> servicedRacks = new ArrayList<>();

  // From left block: rightmost column (side = "right")
  if (leftBlock != null) {
    int rightCol = leftBlock.bottomRight().col();
    for (int row = leftBlock.topLeft().row(); row <= leftBlock.bottomRight().row(); row++) {
      servicedRacks.add(new GridCoordinate(row, rightCol));
    }
  }

  // From right block: leftmost column (side = "left")
  if (rightBlock != null) {
    int leftCol = rightBlock.topLeft().col();
    for (int row = rightBlock.topLeft().row(); row <= rightBlock.bottomRight().row(); row++) {
      servicedRacks.add(new GridCoordinate(row, leftCol));
    }
  }

  Zone zone = new Zone(
      zoneId,
      "aisle",
      leftBlock != null ? leftBlock.blockId() : null,
      rightBlock != null ? rightBlock.blockId() : null,
      coverageArea,
      servicedRacks);

  zones.put(zoneId, zone);
  log.debug("Created zone {}", zoneId);
}
```

**IMPORTANT: Remove these old methods from ZoneManager.java:**
- `generateZonesForBlock(RackBlockGridDefinition block)` (around line 63-77)
- `createAisleZone(String zoneId, RackBlockGridDefinition block, String side)` (around line 79-98)

**Step 4: Update getZonesForRackBlock method**

The existing `getZonesForRackBlock` method uses `getRackBlockIds()` which no longer exists. Update it in `ZoneManager.java`:

```java
/** Get zones for a specific rack block. */
public List<Zone> getZonesForRackBlock(String blockId) {
  return zones.values().stream()
      .filter(zone -> blockId.equals(zone.getLeftBlockId())
                   || blockId.equals(zone.getRightBlockId()))
      .collect(Collectors.toList());
}
```

**Step 5: Write test for getZonesForRackBlock**

Add to `ZoneManagerTest.java`:

```java
@Test
void testGetZonesForRackBlock() {
    WarehouseMap map = mock(WarehouseMap.class);

    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(2, 2), 3, List.of());
    RackBlockGridDefinition block1 = new RackBlockGridDefinition(
        "block_1", new GridCoordinate(0, 4), new GridCoordinate(2, 5), 3, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0, block1));

    ZoneManager manager = new ZoneManager(map);
    manager.generateZones();

    // block_0 should be in 2 zones: zone_null_block_0 (right), zone_block_0_block_1 (left)
    List<Zone> block0Zones = manager.getZonesForRackBlock("block_0");
    assertEquals(2, block0Zones.size());

    // block_1 should be in 2 zones: zone_block_0_block_1 (right), zone_block_1_null (left)
    List<Zone> block1Zones = manager.getZonesForRackBlock("block_1");
    assertEquals(2, block1Zones.size());
}
```

**Step 6: Update existing testBalanceVtmAssignments to use new constructor**

Update the existing test in `ZoneManagerTest.java`:

```java
@Test
void testBalanceVtmAssignments() {
  WarehouseMap map = mock(WarehouseMap.class);
  when(map.getRackBlocks()).thenReturn(List.of());

  ZoneManager manager = new ZoneManager(map);
  manager.generateZones();

  // Manually add zones with new constructor
  manager.getZones().put("zone_0", new Zone("zone_0", "aisle", null, "block_0", List.of(), List.of()));
  manager.getZones().put("zone_1", new Zone("zone_1", "aisle", "block_0", null, List.of(), List.of()));

  manager.balanceVtmAssignments(List.of(0, 1));

  assertEquals(1, manager.getZonesForVtm(0).size());
  assertEquals(1, manager.getZonesForVtm(1).size());
}
```

**Step 7: Run tests to verify they pass**

```bash
cd apps/backend && mvn test -Dtest=ZoneManagerTest -q
```

Expected: All tests PASS

**Step 8: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java
git commit -m "feat(zone-manager): implement aisle-based zone generation"
```

---

## Task 5: Implement Zone Relay Registration

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java`

**Step 1: Write failing test for relay registration**

Add to `ZoneManagerTest.java`:

```java
@Test
void testRegisterRelayPositionsClaimsRelays() {
    WarehouseMap map = mock(WarehouseMap.class);

    // Create 2 blocks
    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(1, 2), 2, List.of());
    RackBlockGridDefinition block1 = new RackBlockGridDefinition(
        "block_1", new GridCoordinate(0, 4), new GridCoordinate(1, 5), 2, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0, block1));

    // Create managers
    ZoneManager zoneManager = new ZoneManager(map);
    zoneManager.generateZones();

    RelayPositionManager relayManager = new RelayPositionManager(map);

    // Register relays with zones
    zoneManager.registerRelayPositions(relayManager);

    // Verify relays have ownerZoneId set
    RelayPosition relay0R = relayManager.getPositionsById().get("relay_block_0_R_r0");
    assertNotNull(relay0R);
    assertEquals("zone_block_0_block_1", relay0R.getOwnerZoneId());

    RelayPosition relay1L = relayManager.getPositionsById().get("relay_block_1_L_r0");
    assertNotNull(relay1L);
    assertEquals("zone_block_0_block_1", relay1L.getOwnerZoneId());

    // Verify zone has relays
    Zone middleZone = zoneManager.getZones().get("zone_block_0_block_1");
    assertEquals(4, middleZone.getRelayPositions().size()); // 2 rows x 2 sides
}

@Test
void testEdgeZonesGetCorrectRelays() {
    WarehouseMap map = mock(WarehouseMap.class);

    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(1, 2), 2, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0));

    ZoneManager zoneManager = new ZoneManager(map);
    zoneManager.generateZones();

    RelayPositionManager relayManager = new RelayPositionManager(map);
    zoneManager.registerRelayPositions(relayManager);

    // Left edge zone gets left-side relays
    Zone leftEdge = zoneManager.getZones().get("zone_null_block_0");
    assertEquals(2, leftEdge.getRelayPositions().size()); // 2 rows, left side only

    // Right edge zone gets right-side relays
    Zone rightEdge = zoneManager.getZones().get("zone_block_0_null");
    assertEquals(2, rightEdge.getRelayPositions().size()); // 2 rows, right side only
}
```

**Step 2: Run tests to verify they fail**

```bash
cd apps/backend && mvn test -Dtest=ZoneManagerTest#testRegisterRelayPositionsClaimsRelays -q
```

Expected: FAIL - relays not claimed correctly

**Step 3: Update registerRelayPositions method**

Replace `registerRelayPositions()` in `ZoneManager.java`:

```java
/**
 * Register relay positions with their owner zones.
 *
 * <p>Zones claim relay positions based on (blockId, side) matching:
 * - Zone with leftBlockId=X claims relays where blockId=X and side="right"
 * - Zone with rightBlockId=X claims relays where blockId=X and side="left"
 *
 * @param relayPositionManager Manager containing all relay positions
 */
public void registerRelayPositions(RelayPositionManager relayPositionManager) {
  for (Zone zone : zones.values()) {
    for (RelayPosition relay : relayPositionManager.getPositionsById().values()) {
      if (zoneOwnsRelay(zone, relay)) {
        zone.addRelayPosition(relay);
        relay.setOwnerZoneId(zone.getZoneId());
      }
    }
  }
  log.info("Registered relay positions with {} zones", zones.size());
}

/**
 * Check if a zone owns a relay position.
 *
 * @param zone The zone
 * @param relay The relay position
 * @return true if the zone should own this relay
 */
private boolean zoneOwnsRelay(Zone zone, RelayPosition relay) {
  String relayBlockId = relay.getRackBlockId();
  String relaySide = relay.getSide();

  // Zone owns relay if:
  // - relay.blockId == zone.leftBlockId AND relay.side == "right"
  // - relay.blockId == zone.rightBlockId AND relay.side == "left"
  boolean leftMatch = relayBlockId.equals(zone.getLeftBlockId()) && "right".equals(relaySide);
  boolean rightMatch = relayBlockId.equals(zone.getRightBlockId()) && "left".equals(relaySide);

  return leftMatch || rightMatch;
}
```

**Step 4: Run tests to verify they pass**

```bash
cd apps/backend && mvn test -Dtest=ZoneManagerTest -q
```

Expected: All tests PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java
git commit -m "feat(zone-manager): zones claim relays during registration"
```

---

## Task 6: Implement Locality-First VTM Assignment

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java`
- Modify: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java`

**Step 1: Write failing tests for locality-first assignment**

Add to `ZoneManagerTest.java`:

```java
@Test
void testBalanceVtmAssignmentsLocalityFirst() {
    WarehouseMap map = mock(WarehouseMap.class);

    // Create 2x2 layout (4 blocks, 6 zones)
    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(2, 2), 3, List.of());
    RackBlockGridDefinition block1 = new RackBlockGridDefinition(
        "block_1", new GridCoordinate(0, 4), new GridCoordinate(2, 5), 3, List.of());
    RackBlockGridDefinition block2 = new RackBlockGridDefinition(
        "block_2", new GridCoordinate(5, 1), new GridCoordinate(7, 2), 3, List.of());
    RackBlockGridDefinition block3 = new RackBlockGridDefinition(
        "block_3", new GridCoordinate(5, 4), new GridCoordinate(7, 5), 3, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0, block1, block2, block3));

    ZoneManager manager = new ZoneManager(map);
    manager.generateZones();

    // 6 VTMs for 6 zones - each VTM gets 1 zone
    manager.balanceVtmAssignments(List.of(0, 1, 2, 3, 4, 5));

    // Each VTM should have exactly 1 zone
    for (int vtmId = 0; vtmId < 6; vtmId++) {
        assertEquals(1, manager.getZonesForVtm(vtmId).size(),
            "VTM " + vtmId + " should have 1 zone");
    }
}

@Test
void testVtmAssignmentWithMoreVtmsThanZones() {
    WarehouseMap map = mock(WarehouseMap.class);

    // Create single block (2 zones: left edge, right edge)
    RackBlockGridDefinition block0 = new RackBlockGridDefinition(
        "block_0", new GridCoordinate(0, 1), new GridCoordinate(2, 2), 3, List.of());

    when(map.getRackBlocks()).thenReturn(List.of(block0));

    ZoneManager manager = new ZoneManager(map);
    manager.generateZones();

    // 4 VTMs for 2 zones
    manager.balanceVtmAssignments(List.of(0, 1, 2, 3));

    // Each zone should have 2 VTMs
    Zone leftZone = manager.getZones().get("zone_null_block_0");
    Zone rightZone = manager.getZones().get("zone_block_0_null");

    assertEquals(2, leftZone.getVtmCount());
    assertEquals(2, rightZone.getVtmCount());
}
```

**Step 2: Run tests to verify they fail**

```bash
cd apps/backend && mvn test -Dtest=ZoneManagerTest#testBalanceVtmAssignmentsLocalityFirst -q
```

Expected: FAIL - current implementation doesn't use locality

**Step 3: Implement locality-first balanceVtmAssignments**

Replace `balanceVtmAssignments()` in `ZoneManager.java`:

```java
/**
 * Balance VTM assignments across zones using locality-first distribution.
 *
 * <p>Distributes VTMs to zones preferring same column (aisle) for minimal travel. Each VTM is
 * assigned to exactly one zone.
 *
 * @param vtmIds List of VTM IDs to assign
 */
public void balanceVtmAssignments(List<Integer> vtmIds) {
  if (zones.isEmpty()) {
    log.warn("No zones available for VTM assignment");
    return;
  }

  // Clear existing assignments
  vtmToZones.clear();
  zones.values().forEach(zone -> zone.getAssignedVtmIds().clear());

  // Group zones by column position for locality
  Map<Integer, List<Zone>> zonesByColumn = groupZonesByColumn();
  List<Integer> columns = new ArrayList<>(zonesByColumn.keySet());
  Collections.sort(columns);

  if (columns.isEmpty()) {
    log.warn("No zone columns found");
    return;
  }

  int numColumns = columns.size();
  int numVtms = vtmIds.size();
  int vtmsPerColumn = numVtms / numColumns;
  int remainingVtms = numVtms % numColumns;

  log.info(
      "Balancing {} VTMs across {} columns (~{} VTMs per column)",
      numVtms, numColumns, vtmsPerColumn);

  // Assign VTMs to columns, then distribute within column
  int vtmIndex = 0;
  for (int col : columns) {
    List<Zone> columnZones = zonesByColumn.get(col);
    int vtmsForThisColumn = vtmsPerColumn + (remainingVtms-- > 0 ? 1 : 0);

    // Distribute VTMs to zones within this column
    int zonesInColumn = columnZones.size();
    for (int i = 0; i < vtmsForThisColumn && vtmIndex < numVtms; i++) {
      int vtmId = vtmIds.get(vtmIndex++);
      Zone zone = columnZones.get(i % zonesInColumn);
      assignVtmToZone(vtmId, zone.getZoneId());
    }
  }

  log.info("VTM assignment complete: {}", getStats());
}

/**
 * Group zones by their aisle column position.
 *
 * @return Map of column position to zones in that column
 */
private Map<Integer, List<Zone>> groupZonesByColumn() {
  Map<Integer, List<Zone>> result = new HashMap<>();

  for (Zone zone : zones.values()) {
    if (zone.getCoverageArea().isEmpty()) continue;

    // Use first coverage position's column as the column key
    int col = zone.getCoverageArea().get(0).col();
    result.computeIfAbsent(col, k -> new ArrayList<>()).add(zone);
  }

  return result;
}
```

**Step 4: Run tests to verify they pass**

```bash
cd apps/backend && mvn test -Dtest=ZoneManagerTest -q
```

Expected: All tests PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/ZoneManager.java src/test/java/com/greyorange/dessimulator/domain/robot/ZoneManagerTest.java
git commit -m "feat(zone-manager): implement locality-first VTM assignment"
```

---

## Task 7: Update WarehouseBuilder and Remove Duplicate Registration

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Write failing integration test**

The existing integration test should fail because zones don't have relay positions registered.

```bash
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```

Check if it fails due to zones having zero capacity.

**Step 2: Update WarehouseBuilder to call registerRelayPositions**

In `WarehouseBuilder.java`, update the Phase 2 section (around line 137-141):

```java
// Phase 2: System-specific (RELAY only)
if (config.systemType() == SystemType.RELAY) {
  buildRelayPositionManager();
  buildZoneManager();
  zoneManager.registerRelayPositions(relayPositionManager);  // ADD THIS LINE
  validatePhase("system-specific", zoneManager, relayPositionManager);
}
```

**Step 3: Remove duplicate registerRelayPositions call from DesmojSimulationEngine**

In `DesmojSimulationEngine.java`, find and **REMOVE** the following line (around line 427):

```java
// REMOVE THIS LINE - registration now happens in WarehouseBuilder
zoneManager().registerRelayPositions(relayPositionManager());
```

This call is now handled during warehouse building, not during simulation setup.

**Step 4: Run integration test to verify it passes**

```bash
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```

Expected: PASS

**Step 5: Format and commit**

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "feat(warehouse-builder): register relay positions with zones"
```

---

## Task 8: Final Integration Testing

**Files:**
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/simulator/DesmojIntegrationTest.java`

**Step 1: Run full test suite**

```bash
cd apps/backend && mvn test -q
```

Expected: All tests PASS

**Step 2: Verify zone count with test config**

Create a quick verification in integration test or check via logs that zone count is correct for your test scenario.

For a 7x2 block layout: should have 16 zones (8 per row).

**Step 3: Run spotless and final commit**

```bash
cd apps/backend && mvn spotless:apply
git add -A
git commit -m "test: verify zone aisle refactoring integration"
```

---

## Summary

After completing all tasks:
- Zone count reduced from 28 to 16 (for 7x2 layout)
- Each zone represents a physical aisle
- Zones track leftBlockId/rightBlockId
- Relays claimed by zones during registration
- VTM assignment uses locality-first distribution
- WarehouseBuilder connects all components correctly
