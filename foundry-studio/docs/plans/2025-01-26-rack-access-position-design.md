# Rack Access Position Design

## Problem

VTMs currently navigate directly to rack positions (`rack.getCurrentCoordinate()`), but rack cells should be obstacles. This causes unrealistic pathfinding where VTMs path through rack positions instead of using aisles.

## Solution

Add VTM access positions to racks (aisle positions) and mark rack cells as non-traversable for VTMs.

## Scope

- **Relay system only** - RTP pathfinding unchanged
- Racks accessible from **one side only** (left or right based on position in block)
- Access positions calculated at **map generation time** (static property)

## Data Model Changes

### 1. New record in `WarehouseMap.java`

```java
public record RackPositionMapData(
    GridCoordinate position,           // Where rack physically sits
    GridCoordinate vtmAccessPosition,  // Aisle position for VTM access
    String rackBlockId,
    String side                        // "left" or "right"
) {}
```

### 2. New field in `Rack.java`

```java
@Setter
private GridCoordinate vtmAccessPosition;  // null for RTP system
```

### 3. New map in `WarehouseMap.java`

```java
private final Map<GridCoordinate, RackPositionMapData> rackPositionDataByPosition;
```

## Side Calculation Logic

Racks are assigned to left or right side based on their column position relative to the block midpoint:

```
blockMidCol = (topLeft.col + bottomRight.col) / 2

if (rack.col <= blockMidCol):
    side = "left"
    vtmAccessCol = topLeft.col - 1
else:
    side = "right"
    vtmAccessCol = bottomRight.col + 1
```

**Bounds validation required**: If `vtmAccessCol < 0` or `vtmAccessCol >= gridWidth`, the rack won't have an access position (edge case for racks at grid boundary).

## Visual Representation

```
  Left Aisle    Rack Block (cols 5-8)    Right Aisle
    (col 4)                                (col 9)
       |                                      |
       |    +----+----+----+----+             |
       +--->| R1 | R2 | R3 | R4 |<------------+
       |    | <L | <L | R> | R> |             |
       |    +----+----+----+----+             |
       +--->| R5 | R6 | R7 | R8 |<------------+
       |    | <L | <L | R> | R> |             |
       |    +----+----+----+----+             |

  R1,R2,R5,R6 -> vtmAccessPosition at col 4 (left aisle)
  R3,R4,R7,R8 -> vtmAccessPosition at col 9 (right aisle)
```

## Implementation Changes

### WarehouseMap.java

Add new method `generateRackPositionData()`:

```java
private Map<GridCoordinate, RackPositionMapData> generateRackPositionData() {
    if (systemType != SystemType.RELAY) {
        return Map.of();
    }

    Map<GridCoordinate, RackPositionMapData> dataByPosition = new HashMap<>();

    for (var block : rackBlocks) {
        int midCol = (block.topLeft().col() + block.bottomRight().col()) / 2;

        for (var rackPos : block.getRackPositions()) {
            String side;
            int accessCol;

            if (rackPos.col() <= midCol) {
                side = "left";
                accessCol = block.topLeft().col() - 1;
            } else {
                side = "right";
                accessCol = block.bottomRight().col() + 1;
            }

            // Bounds check
            if (accessCol >= 0 && accessCol < config.getGridWidth()) {
                var vtmAccess = new GridCoordinate(rackPos.row(), accessCol);
                dataByPosition.put(rackPos, new RackPositionMapData(
                    rackPos, vtmAccess, block.blockId(), side));
            }
        }
    }
    return dataByPosition;
}
```

### WarehouseBuilder.java

Wire access position during rack creation:

```java
// When creating rack for Relay system
var rackMapData = warehouseMap.getRackPositionData().get(rackPosition);
if (rackMapData != null) {
    rack.setVtmAccessPosition(rackMapData.vtmAccessPosition());
}
```

### PathFinder.java

Update `isTraversableByRobotType()` method:

```java
private boolean isTraversableByRobotType(
    GridCoordinate position, Robot.RobotType robotType, WarehouseMap map) {
  return switch (robotType) {
    case VTM -> !map.isHighway(position) && !map.isRackPosition(position);
    case HTM -> true;
    case RTP -> true;
  };
}
```

### DesmojVtmProcess.java

Update `travelToRack()` to use access position:

```java
private double travelToRack(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.MOVING_TO_RACK);
    GridCoordinate destination = ctx.sourceRack().getVtmAccessPosition();
    // Fallback for edge cases (null access position)
    if (destination == null) {
        destination = ctx.sourceRack().getCurrentCoordinate();
    }
    return navigateTo(destination, false, "to rack access");
}
```

Same pattern applies to return journey when VTM brings tote back to rack.

## Files Changed Summary

| File | Change |
|------|--------|
| `WarehouseMap.java` | Add `RackPositionMapData` record, `rackPositionDataByPosition` map, `generateRackPositionData()` method, `getRackPositionData()` getter |
| `Rack.java` | Add `@Setter private GridCoordinate vtmAccessPosition` field |
| `WarehouseBuilder.java` | Set `rack.setVtmAccessPosition()` from map data during rack creation (Relay only) |
| `PathFinder.java` | Add `&& !map.isRackPosition(position)` to VTM case |
| `DesmojVtmProcess.java` | Navigate to `rack.getVtmAccessPosition()` instead of `rack.getCurrentCoordinate()` |

## Notes

- VTM loading/unloading: Once at access position, VTM holds for loading time. No explicit "reach into rack" action modeled.
- Zone alignment: The left/right side calculation naturally aligns with zone assignment (which uses the same geometric logic).
- No dependency on zones: Access positions are calculated purely from geometry, zones are created later.
