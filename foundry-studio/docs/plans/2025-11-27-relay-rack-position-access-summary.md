# Relay and Rack Position Access - Implementation Status

**Date**: November 27, 2025  
**Status**: Relay Position Complete ✅ | Rack Position Pending 🔄

---

## Overview

This document tracks the implementation of dual access point architecture for both relay positions and rack positions in the relay warehouse system. The goal is to properly model physical positions and their robot-specific access points, enabling realistic navigation for different robot types.

---

## Part 1: Relay Position Dual Access ✅ COMPLETED

### Problem Statement

Relay positions were incorrectly modeled with:
1. **Duplication** - Generated in both `WarehouseMap` and `RelayPositionManager`
2. **Wrong position** - Located at ±1 columns from rack edges instead of at the rack edge itself
3. **Single access point** - Both VTMs and HTMs navigated to the same coordinate

### Correct Model

- **Physical position**: Relay is located AT the rack edge (underneath the rack)
- **HTM access**: Same coordinate as the physical position (HTMs navigate on highways and can access rack edge)
- **VTM access**: ±1 column from the physical position
  - Left side relay: VTM accesses from `leftEdgeCol - 1`
  - Right side relay: VTM accesses from `rightEdgeCol + 1`

### Implementation Summary

#### Key Changes

**Coordinate Generation Logic:**
```java
// OLD (WRONG): Relay at access position
int leftCol = topLeft.col() - 1;    // Access position
int rightCol = bottomRight.col() + 1;  // Access position

// NEW (CORRECT): Relay at rack edge
int leftEdgeCol = topLeft.col();       // Physical position at rack edge
int rightEdgeCol = bottomRight.col();  // Physical position at rack edge

// Access positions calculated relative to physical position
GridCoordinate vtmAccessLeft = new GridCoordinate(row, leftEdgeCol - 1);
GridCoordinate vtmAccessRight = new GridCoordinate(row, rightEdgeCol + 1);
```

#### Files Modified (9 files)

1. **WarehouseMap.java**
   - Added `RelayPositionMapData` record with `position`, `htmAccessPosition`, `vtmAccessPosition`
   - Replaced `List<GridCoordinate> relayPositions` with `Map<String, RelayPositionMapData> relayPositionDataById`
   - Rewrote `generateRelayPositions()` as `generateRelayPositionData()` with correct coordinate logic
   - Added `getRelayPositionData()` accessor method
   - Updated `getRelayPositions()` and `isRelayPosition()` methods

2. **RelayPosition.java**
   - Added `htmAccessPosition` and `vtmAccessPosition` fields
   - Updated constructor to accept both access positions (3 coordinates total)
   - Getters auto-generated via `@Getter`

3. **RelayPositionManager.java**
   - Removed duplicate relay generation logic
   - Updated `buildRelayPositions()` to fetch from `WarehouseMap.getRelayPositionData()`
   - Updated distance calculations in `selectOptimalRelayForVtm()` to use `getVtmAccessPosition()`
   - Updated distance calculations in `selectOptimalRelayForHtm()` to use `getHtmAccessPosition()`

4. **DesmojVtmProcess.java**
   - Updated `transportToRelay()` to navigate to `relay.getVtmAccessPosition()`
   - Updated `executeReturnJourney()` to navigate to `relay.getVtmAccessPosition()`
   - Updated `findReturnTaskInZone()` distance calculation to use `getVtmAccessPosition()`

5. **DesmojHtmProcess.java**
   - Updated `travelToRelayAndLoad()` to navigate to `relay.getHtmAccessPosition()`
   - Updated `returnToRelay()` to navigate to `relay.getHtmAccessPosition()`

6. **RelayPositionTest.java**
   - Updated all constructor calls with HTM and VTM access positions

7. **RelayPositionOwnerZoneTest.java**
   - Updated constructor calls with HTM and VTM access positions

8. **Test files** - All integration tests pass

9. **DesmojSnapshotCaptureProcess.java** - No changes needed (uses physical position for visualization)

### Architecture Decisions

1. **Coordinate generation in WarehouseMap**: Physical layout property, belongs with map generation
2. **HashMap access pattern**: `RelayPositionManager` fetches complete data via `Map<String, RelayPositionMapData>`
3. **Similar to StationGridPosition**: Pattern match with `position` + `accessPosition` model
4. **Zone assignment unchanged**: Uses `rackBlockId` and `side`, not coordinates
5. **Snapshot uses physical position**: Visualization shows where relay actually is (at rack edge)

### Validation Results

- ✅ No compilation errors
- ✅ All linting warnings are pre-existing (not related to changes)
- ✅ Zone assignment logic unchanged
- ✅ Relay physical position now at rack edge column (not ±1)
- ✅ VTMs navigate to ±1 column from relay position
- ✅ HTMs navigate to the relay position itself
- ✅ A* pathfinding works correctly with new access positions

### Related Documentation

- Design Document: `docs/plans/2025-11-26-vtm-htm-astar-navigation-design.md`
- Implementation Summary: `docs/plans/2025-11-26-vtm-htm-astar-implementation-summary.md`

---

## Part 2: Rack Position Access 🔄 PENDING

### Problem Statement

**Current State:**
- `Rack` class has a mutable `currentCoordinate` field
- In **RTP system**: Racks are mobile (carried by robots), position updates dynamically
- In **RELAY system**: Racks should be stationary structures, but currently use the same model
- VTMs navigate directly to `rack.getCurrentCoordinate()` which might be inside a rack block

**Desired State for RELAY:**
- Racks are **fixed racking structures** (like warehouse shelving)
- Racks hold totes at specific Z-indexes (vertical slots)
- Each rack needs **access position(s)** where VTMs can pick/place totes
- Rack physical position ≠ VTM access position (similar to StationGridPosition pattern)
- Rack position should be immutable for RELAY system

### Current Issues

1. **State Mutation Misuse**: `rack.moveToStation()` and `rack.moveToStorage()` methods exist but should **never be called** in RELAY system
2. **No Access Position**: VTMs navigate directly to `rack.getCurrentCoordinate()`, which is the rack's physical position (inside rack block)
3. **Shared Model**: Same `Rack` class for both RTP (mobile) and RELAY (stationary) systems leads to confusion

### Design Options Analysis

#### Option 1: System-Specific Subclasses

Create separate rack implementations per system:

```java
// Base interface
public interface StorageRack {
    RackId getId();
    int addTote(Tote tote, Integer zIndex);
    Tote removeTote(int zIndex);
    Map<Integer, Tote> getTotes();
}

// RTP: Mobile racks
public class MobileRack implements StorageRack {
    private GridCoordinate currentCoordinate; // MUTABLE
    private RackState state;
    public void moveToStation(GridCoordinate position) { ... }
}

// RELAY: Stationary racks
public class StationaryRack implements StorageRack {
    private final GridCoordinate position; // IMMUTABLE
    private final GridCoordinate accessPosition; // VTM access point
    private final String rackBlockId;
}
```

**Pros:**
- ✅ Clear separation of concerns
- ✅ Type safety (can't call move methods on stationary racks)
- ✅ Immutable position for RELAY racks

**Cons:**
- ❌ Requires refactoring all rack usage sites
- ❌ More complex polymorphism handling
- ❌ Harder to migrate existing simulations

#### Option 2: Enhanced Rack with SystemType Flag

Keep single `Rack` class but add system-aware behavior:

```java
public class Rack {
    private final SystemType systemType;
    private final GridCoordinate initialPosition; // IMMUTABLE
    private final GridCoordinate accessPosition; // For RELAY
    private GridCoordinate currentCoordinate; // Mutable for RTP, fixed for RELAY
    
    public GridCoordinate getAccessPosition() {
        return systemType == SystemType.RELAY 
            ? accessPosition 
            : currentCoordinate;
    }
    
    public void moveToStation(GridCoordinate position) {
        if (systemType == SystemType.RELAY) {
            throw new UnsupportedOperationException("Cannot move stationary racks");
        }
        // RTP logic...
    }
}
```

**Pros:**
- ✅ Less invasive refactoring
- ✅ Single class to maintain
- ✅ Can guard methods with system type checks

**Cons:**
- ❌ Mixed responsibilities
- ❌ Runtime checks instead of compile-time safety
- ❌ Some fields null/unused depending on system

#### Option 3: Composition with RackPositionMapData (RECOMMENDED)

Similar to `StationGridPosition` and `RelayPositionMapData` pattern:

```java
// In WarehouseMap.java - rack position metadata
public record RackPositionMapData(
    String id,
    String rackBlockId,
    GridCoordinate position,        // Physical rack location
    GridCoordinate accessPosition   // Where VTMs access it (for RELAY)
) {}

public class Rack {
    private final RackId id;
    private final SystemType systemType;
    private final RackPositionMapData positionMetadata; // Immutable metadata
    
    // RTP dynamic state
    private GridCoordinate currentCoordinate; // Only changes in RTP
    private RackState state;
    
    // API that works for both systems
    public GridCoordinate getPosition() {
        return systemType == SystemType.RELAY 
            ? positionMetadata.position() 
            : currentCoordinate;
    }
    
    public GridCoordinate getAccessPosition() {
        return systemType == SystemType.RELAY
            ? positionMetadata.accessPosition()
            : currentCoordinate; // RTP: robot brings rack to station
    }
    
    // Guarded methods
    public void moveToStation(GridCoordinate pos) {
        if (systemType == SystemType.RELAY) {
            throw new UnsupportedOperationException("Cannot move stationary racks");
        }
        this.currentCoordinate = pos;
        this.state = RackState.AT_STATION;
    }
}
```

**Pros:**
- ✅ Follows existing pattern (StationGridPosition, RelayPositionMapData)
- ✅ Clear separation of position metadata from runtime state
- ✅ WarehouseMap generates position data (centralized logic)
- ✅ Type-safe position information
- ✅ Minimal changes to existing code

**Cons:**
- ❌ Adds another record type
- ❌ Still some conditional logic based on SystemType

### Recommended Implementation Plan

#### Phase 1: Add RackPositionMapData to WarehouseMap

```java
// 1. Add record to WarehouseMap.java
public record RackPositionMapData(
    String id,
    String rackBlockId,
    GridCoordinate position,        // Physical rack location
    GridCoordinate accessPosition   // VTM access point (aisle position)
) {}

// 2. Generate rack position data
private Map<String, RackPositionMapData> generateRackPositionData() {
    Map<String, RackPositionMapData> dataById = new HashMap<>();
    
    for (RackBlockGridDefinition block : rackBlocks) {
        // For each rack position in the block
        for (GridCoordinate rackPos : block.getRackPositions()) {
            // Calculate access position based on nearest aisle
            GridCoordinate accessPos = findNearestAislePosition(rackPos, block);
            
            String id = "rack_" + /* generate ID */;
            RackPositionMapData data = new RackPositionMapData(
                id, block.blockId(), rackPos, accessPos
            );
            dataById.put(id, data);
        }
    }
    
    return dataById;
}

private GridCoordinate findNearestAislePosition(GridCoordinate rackPos, RackBlockGridDefinition block) {
    // Find adjacent aisle position (similar to relay position logic)
    // Check left (col-1) and right (col+1) for aisles
    // Return the accessible aisle position
}
```

#### Phase 2: Update Rack Class

```java
public class Rack {
    private final RackId id;
    private final SystemType systemType;
    private final RackPositionMapData positionMetadata; // NEW - for RELAY
    private final int maxToteCapacity;
    
    // Existing fields for RTP
    private GridCoordinate currentCoordinate;
    private RackState state;
    private Integer assignedRobotId;
    private final Map<Integer, Tote> totes = new ConcurrentHashMap<>();
    
    // Constructor for RELAY system
    public Rack(RackId id, RackPositionMapData positionMetadata, int maxToteCapacity) {
        this.id = id;
        this.systemType = SystemType.RELAY;
        this.positionMetadata = positionMetadata;
        this.currentCoordinate = positionMetadata.position(); // Immutable for RELAY
        this.state = RackState.IN_STORAGE; // Always in storage
        this.maxToteCapacity = maxToteCapacity;
        this.assignedRobotId = null;
    }
    
    // Constructor for RTP system (existing)
    public Rack(RackId id, GridCoordinate initialPosition, int maxToteCapacity) {
        this.id = id;
        this.systemType = SystemType.RTP;
        this.positionMetadata = null;
        this.currentCoordinate = initialPosition;
        this.state = RackState.IN_STORAGE;
        this.maxToteCapacity = maxToteCapacity;
        this.assignedRobotId = null;
    }
    
    // Unified API
    public GridCoordinate getPosition() {
        return systemType == SystemType.RELAY 
            ? positionMetadata.position() 
            : currentCoordinate;
    }
    
    public GridCoordinate getAccessPosition() {
        if (systemType == SystemType.RELAY) {
            return positionMetadata.accessPosition();
        }
        // RTP: robot brings rack to station, so access = current position
        return currentCoordinate;
    }
    
    // Guarded methods
    public void moveToStation(GridCoordinate stationPosition) {
        if (systemType == SystemType.RELAY) {
            throw new UnsupportedOperationException(
                "Stationary racks cannot move in RELAY system");
        }
        // Existing RTP logic...
        this.state = RackState.AT_STATION;
        this.currentCoordinate = stationPosition;
    }
    
    public void moveToStorage(GridCoordinate storagePosition) {
        if (systemType == SystemType.RELAY) {
            throw new UnsupportedOperationException(
                "Stationary racks cannot move in RELAY system");
        }
        // Existing RTP logic...
        this.state = RackState.IN_STORAGE;
        this.currentCoordinate = storagePosition;
        this.assignedRobotId = null;
    }
}
```

#### Phase 3: Update VTM Process

```java
// In DesmojVtmProcess.java

// OLD: Navigate to rack's physical position
private double travelToRack(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.MOVING_TO_RACK);
    return navigateTo(ctx.sourceRack().getCurrentCoordinate(), false, "to rack");
}

// NEW: Navigate to rack's access position
private double travelToRack(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.MOVING_TO_RACK);
    // VTM navigates to access position (aisle) to pick up tote
    return navigateTo(ctx.sourceRack().getAccessPosition(), false, "to rack");
}

// Also update return journey
private void executeReturnJourney(ReturnTask task) throws SuspendExecution {
    // ...
    // OLD: Navigate to rack position
    // double travelToRack = navigateTo(targetRack.getCurrentCoordinate(), true, "to rack");
    
    // NEW: Navigate to rack access position
    double travelToRack = navigateTo(targetRack.getAccessPosition(), true, "to rack");
}
```

#### Phase 4: Update WarehouseBuilder

```java
// In WarehouseBuilder.java - addRacks() method

private void addRacks() {
    if (config.systemType() == SystemType.RELAY) {
        // NEW: Use RackPositionMapData from WarehouseMap
        Map<String, WarehouseMap.RackPositionMapData> rackData = 
            map.getRackPositionData();
        
        for (WarehouseMap.RackPositionMapData data : rackData.values()) {
            int rackIdInt = /* extract from data.id() */;
            RackId rackId = new RackId(rackIdInt);
            int maxToteCapacity = config.mapConfig().getVerticalToteCapacityInRack();
            
            Rack rack = new Rack(rackId, data, maxToteCapacity);
            racks.add(rack);
        }
    } else {
        // Existing RTP logic
        List<GridCoordinate> rackGridPositions = map.getRackPositions();
        for (int i = 0; i < rackGridPositions.size(); i++) {
            GridCoordinate gridPos = rackGridPositions.get(i);
            Rack rack = new Rack(new RackId(i + 1), gridPos, maxToteCapacity);
            racks.add(rack);
        }
    }
}
```

### Implementation Steps (Detailed)

1. **Add `RackPositionMapData` record to WarehouseMap**
2. **Implement `generateRackPositionData()` method** with aisle access logic
3. **Add `systemType` field to Rack class**
4. **Add `positionMetadata` field to Rack class**
5. **Create new Rack constructor** accepting `RackPositionMapData`
6. **Add `getAccessPosition()` method** to Rack
7. **Guard `moveToStation()` and `moveToStorage()`** with SystemType checks
8. **Update `WarehouseBuilder.addRacks()`** to use position data for RELAY
9. **Update `DesmojVtmProcess`** to use `getAccessPosition()`
10. **Update tests** - create racks with appropriate constructors
11. **Verify RTP system** - ensure no regressions
12. **Verify RELAY system** - VTMs navigate to accessible aisles

### Files to Modify (Estimated)

1. `WarehouseMap.java` - Add RackPositionMapData, generate method
2. `Rack.java` - Add fields, constructor, guards, getAccessPosition()
3. `WarehouseBuilder.java` - Update rack creation for RELAY
4. `DesmojVtmProcess.java` - Use getAccessPosition()
5. `RackTest.java` (if exists) - Update tests
6. `WarehouseBuilderTest.java` (if exists) - Update tests

### Expected Benefits

- ✅ Stationary racks properly modeled in RELAY system
- ✅ VTMs navigate to accessible aisle positions
- ✅ Clear separation between physical position and access position
- ✅ Consistent pattern with RelayPosition and StationGridPosition
- ✅ Type-safe position information
- ✅ No RTP regression (uses existing logic)

### Risks and Mitigation

**Risk 1**: Breaking existing RTP simulations
- **Mitigation**: Keep RTP constructor and logic unchanged; add RELAY-specific constructor

**Risk 2**: Complex aisle detection logic
- **Mitigation**: Reuse aisle detection from rack block generation; similar to relay position logic

**Risk 3**: Performance impact from position lookups
- **Mitigation**: Cache RackPositionMapData in HashMap (O(1) lookup)

---

## Next Steps

1. ✅ Complete relay position implementation (DONE)
2. 🔄 Review and approve rack position design approach
3. 🔄 Implement `RackPositionMapData` generation in WarehouseMap
4. 🔄 Update Rack class with dual constructor and access position API
5. 🔄 Update VTM processes to use rack access positions
6. 🔄 Test and validate RELAY system with proper rack access
7. 🔄 Verify no RTP regressions

---

## References

- Related: `docs/plans/2025-11-26-vtm-htm-astar-implementation-summary.md`
- Pattern: StationGridPosition with position + accessibilityDirection
- Pattern: RelayPositionMapData with position + htmAccess + vtmAccess

---

**Authors:** Kumar D., Claude (Anthropic)  
**Last Updated:** November 27, 2025

