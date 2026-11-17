# Zone Aisle-Based Model Refactoring Design

## Overview

**Problem:** Current implementation creates 2 zones per rack block (left and right aisles), resulting in 28 zones for 14 blocks. This creates duplicate overlapping zones in shared aisles.

**Target State:** One zone per physical aisle. A zone represents the aisle between two adjacent rack blocks (or at the edge). VTMs operate within an aisle and service both adjacent blocks.

**Expected Outcome:** For a 7x2 block layout (14 blocks): 8 aisle columns x 2 rows = 16 zones.

## Zone Naming Convention

Zone names derived from adjacent block IDs: `zone_{leftBlockId}_{rightBlockId}`

| Position | Zone Name | Description |
|----------|-----------|-------------|
| Left edge | `zone_null_block_0` | No left block |
| Between | `zone_block_0_block_1` | Between block_0 and block_1 |
| Right edge | `zone_block_6_null` | No right block |

For row 1 (blocks 7-13): `zone_null_block_7`, `zone_block_7_block_8`, ..., `zone_block_13_null`

## Zone Data Model

```java
public class Zone {
  private final String zoneId;
  private final String zoneType;              // "aisle"
  private final String leftBlockId;           // nullable for left edge
  private final String rightBlockId;          // nullable for right edge
  private final List<GridCoordinate> coverageArea;
  private final List<GridCoordinate> servicedRackCoordinates;
  private final Set<Integer> assignedVtmIds;
  private final List<RelayPosition> relayPositions;
  private int reserved;
}
```

**Serviced Racks:**
- From left block: rightmost rack column only (side = "right")
- From right block: leftmost rack column only (side = "left")

## Zone Generation Algorithm

```java
public void generateZones() {
  // Group blocks by row (overlapping row coordinates)
  Map<Integer, List<RackBlockGridDefinition>> blocksByRow =
    groupBlocksByRowRange(map.getRackBlocks());

  for (List<RackBlockGridDefinition> rowBlocks : blocksByRow.values()) {
    // Sort by column position
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
}

private Zone createZone(RackBlockGridDefinition leftBlock,
                        RackBlockGridDefinition rightBlock) {
  String leftId = leftBlock != null ? leftBlock.blockId() : "null";
  String rightId = rightBlock != null ? rightBlock.blockId() : "null";
  String zoneId = "zone_" + leftId + "_" + rightId;

  // Coverage area: aisle column between blocks
  // Serviced racks: left block's right column + right block's left column
}
```

## Relay Position Ownership

**Phase 1: Relay Generation**
```java
// RelayPositionManager creates relays with:
- id: "relay_{blockId}_{L|R}_r{row}"
- blockId: the rack block
- side: "left" or "right"
// No ownerZoneId during creation
```

**Phase 2: Zones Claim Relays**
```java
public void registerRelayPositions(RelayPositionManager relayPositionManager) {
  for (Zone zone : zones.values()) {
    for (RelayPosition relay : relayPositionManager.getPositionsById().values()) {
      if (zoneOwnsRelay(zone, relay)) {
        zone.addRelayPosition(relay);
        relay.setOwnerZoneId(zone.getZoneId());
      }
    }
  }
}

private boolean zoneOwnsRelay(Zone zone, RelayPosition relay) {
  return (relay.getBlockId().equals(zone.getLeftBlockId())
          && relay.getSide().equals("right"))
      || (relay.getBlockId().equals(zone.getRightBlockId())
          && relay.getSide().equals("left"));
}
```

## VTM Assignment Algorithm

**Principle:** Locality-first distribution - VTMs assigned to zones in same column.

```java
public void balanceVtmAssignments(List<Integer> vtmIds) {
  // 1. Group zones by column position
  Map<Integer, List<Zone>> zonesByColumn = groupZonesByColumn();
  List<Integer> columns = new ArrayList<>(zonesByColumn.keySet());
  Collections.sort(columns);

  // 2. Calculate distribution
  int numColumns = columns.size();
  int numVtms = vtmIds.size();
  int vtmsPerColumn = numVtms / numColumns;
  int remainingVtms = numVtms % numColumns;

  // 3. Assign VTMs to columns, then to zones within columns
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
}
```

**Distribution Rules:**
- Each VTM assigned to single zone
- Some zones may get more VTMs than others (uneven distribution)
- Locality preserved: VTMs stay in their column

## Files to Modify

| File | Changes |
|------|---------|
| `Zone.java` | Replace `rackBlockIds` with `leftBlockId`, `rightBlockId`; update constructor |
| `ZoneManager.java` | Rewrite `generateZones()`, `registerRelayPositions()`, `balanceVtmAssignments()` |
| `RelayPosition.java` | Make `ownerZoneId` mutable (add setter) |
| `WarehouseBuilder.java` | Add `zoneManager.registerRelayPositions(relayPositionManager)` call |

## Build Order in WarehouseBuilder

```java
// Phase 2: System-specific (RELAY only)
if (config.systemType() == SystemType.RELAY) {
  buildRelayPositionManager();  // Creates relays with blockId, side
  buildZoneManager();           // Creates zones with leftBlockId, rightBlockId
  zoneManager.registerRelayPositions(relayPositionManager);  // Zones claim relays
  validatePhase("system-specific", zoneManager, relayPositionManager);
}
```

## Test Updates

- Update tests creating zones with new constructor
- Verify zone count (28 → 16 for 7x2 layout)
- Verify relay ownership assignments
- Verify VTM assignment distribution
