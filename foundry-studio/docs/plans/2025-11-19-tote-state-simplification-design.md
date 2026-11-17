# Tote State Simplification Design

## Overview

Simplify Tote state management by eliminating redundant fields and centralizing location tracking in StorageSpaceManager. State is derived from location, never stored.

## Problem

The Tote class has 10+ fields with significant redundancy:
- `state` (7 enum values) duplicates what can be derived from which container holds the tote
- `currentHoldingEntity` duplicates information already in Rack.totes, RelayPosition.tote, Robot.carryingTotes
- `currentCoordinate` duplicates the container's coordinate
- `targetRelayId`, `targetStationId` are journey intent that belongs with the transport operation

This redundancy causes state drift bugs:
- `tote.getState()` returns AT_RELAY but tote isn't in any RelayPosition
- `tote.getCurrentHoldingEntity()` points to wrong Rack/Relay

**Root cause:** Same information stored in multiple places with no single source of truth.

## Solution

### Ownership Model

- **Containers** (Rack, RelayPosition, Robot) remain source of truth for physical containment
- **StorageSpaceManager** maintains a location index and orchestrates all moves
- **Tote** becomes minimal: just `id`, `tasks`, and `claims`
- **State** is derived from location, never stored

---

## ToteLocation Model

New sealed type representing where a tote physically is:

```java
public sealed interface ToteLocation {

    /** Tote stored in a rack at a specific slot. */
    record InRack(RackId rackId, int zIndex) implements ToteLocation {}

    /** Tote at a relay position waiting for pickup. */
    record AtRelay(String relayId) implements ToteLocation {}

    /** Tote being carried by a robot. */
    record WithRobot(int robotId, RobotType robotType, TransportIntent intent)
        implements ToteLocation {}
}

public enum RobotType { VTM, HTM }

public sealed interface TransportIntent {
    record ToRelay(String relayId) implements TransportIntent {}
    record ToStation(StationId stationId) implements TransportIntent {}
    record ReturningToRelay(String relayId) implements TransportIntent {}
    record ReturningToRack() implements TransportIntent {}
}
```

### Design Decisions

1. **No coordinates stored** - derived from container (rack, relay, or robot position)
2. **zIndex included in InRack** - needed for slot-specific operations
3. **Intent bundled with WithRobot** - captures where tote is going, enables state derivation
4. **Sealed types** - exhaustive pattern matching, compiler checks all cases

### Coordinate Derivation

- `InRack` → `storageSpaceManager.getRack(rackId).getCurrentCoordinate()`
- `AtRelay` → `relayPositionManager.getPosition(relayId).getPosition()`
- `WithRobot` → `robot.getPosition()`

---

## StorageSpaceManager Changes

StorageSpaceManager becomes the orchestrator for all tote movements.

### New Fields

```java
public class StorageSpaceManager {
    // Existing
    private final Map<RackId, Rack> racksById;
    private final Map<ToteId, Tote> totesById;

    // New: location index
    private final Map<ToteId, ToteLocation> toteLocations = new ConcurrentHashMap<>();
}
```

### New Orchestration Methods

```java
// Move operations - single source of truth for all tote movements
public void moveToteToRack(ToteId toteId, RackId rackId, int zIndex) {
    Rack rack = getRack(rackId);
    Tote tote = getTote(toteId);
    rack.addToteInternal(tote, zIndex);
    toteLocations.put(toteId, new ToteLocation.InRack(rackId, zIndex));
}

public void moveToteToRelay(ToteId toteId, String relayId, RelayPosition relay) {
    Tote tote = getTote(toteId);
    relay.setToteInternal(tote);
    toteLocations.put(toteId, new ToteLocation.AtRelay(relayId));
}

public void moveToteToRobot(ToteId toteId, int robotId, RobotType type, TransportIntent intent) {
    toteLocations.put(toteId, new ToteLocation.WithRobot(robotId, type, intent));
}

public void moveToteFromRackToRobot(ToteId toteId, RackId rackId, int zIndex,
        int robotId, RobotType type, TransportIntent intent) {
    Rack rack = getRack(rackId);
    rack.removeToteInternal(zIndex);
    toteLocations.put(toteId, new ToteLocation.WithRobot(robotId, type, intent));
}

public void moveToteFromRobotToRelay(ToteId toteId, String relayId, RelayPosition relay) {
    Tote tote = getTote(toteId);
    relay.setToteInternal(tote);
    toteLocations.put(toteId, new ToteLocation.AtRelay(relayId));
}

public void moveToteFromRelayToRobot(ToteId toteId, String relayId,
        int robotId, RobotType type, TransportIntent intent) {
    RelayPosition relay = // get relay by id
    relay.clearToteInternal();
    toteLocations.put(toteId, new ToteLocation.WithRobot(robotId, type, intent));
}

public void moveToteFromRobotToRack(ToteId toteId, RackId rackId, int zIndex) {
    Rack rack = getRack(rackId);
    Tote tote = getTote(toteId);
    rack.addToteInternal(tote, zIndex);
    toteLocations.put(toteId, new ToteLocation.InRack(rackId, zIndex));
}
```

### Query Operations

```java
public ToteLocation getToteLocation(ToteId toteId) {
    return toteLocations.get(toteId);
}

public Optional<GridCoordinate> getToteCoordinate(ToteId toteId) {
    ToteLocation loc = toteLocations.get(toteId);
    return switch (loc) {
        case InRack(RackId rackId, _) ->
            Optional.ofNullable(getRack(rackId)).map(Rack::getCurrentCoordinate);
        case AtRelay(String relayId) ->
            // Need RelayPositionManager reference
            Optional.empty(); // TODO: implement
        case WithRobot(int robotId, _, _) ->
            // Need Robot registry reference
            Optional.empty(); // TODO: implement
    };
}

public ToteState deriveToteState(ToteId toteId) {
    ToteLocation loc = toteLocations.get(toteId);
    if (loc == null) {
        return ToteState.IN_RACK; // Default for untracked
    }
    return switch (loc) {
        case InRack _ -> ToteState.IN_RACK;
        case AtRelay _ -> ToteState.AT_RELAY_POSITION;
        case WithRobot(_, RobotType.VTM, ToRelay _) -> ToteState.BEING_TRANSPORTED_TO_RELAY;
        case WithRobot(_, RobotType.HTM, ToStation _) -> ToteState.BEING_TRANSPORTED_TO_STATION;
        case WithRobot(_, RobotType.HTM, ReturningToRelay _) -> ToteState.BEING_RETURNED_TO_RELAY;
        case WithRobot(_, RobotType.VTM, ReturningToRack _) -> ToteState.BEING_RETURNED_TO_RACK;
        case WithRobot _ -> ToteState.BEING_TRANSPORTED_TO_RELAY; // Fallback
    };
}
```

### Container Internal Methods

Containers get package-private methods (no external notification):
- `Rack.addToteInternal(Tote tote, int zIndex)`
- `Rack.removeToteInternal(int zIndex)`
- `RelayPosition.setToteInternal(Tote tote)`
- `RelayPosition.clearToteInternal()`

---

## Simplified Tote

Tote becomes minimal - only identity, tasks, and claims:

```java
@Slf4j
@Getter
public final class Tote implements WarehouseInventory.StorageSpace {
    private final ToteId id;
    private final int maxCapacity = 6;
    private final List<PickTask> tasks = new ArrayList<>();

    // Claims - prevent duplicate selection by robots
    @Setter private Integer assignedVtmId;
    @Setter private Integer assignedHtmId;

    public Tote(ToteId id) {
        this.id = id;
    }

    // Task management methods (unchanged)
    public synchronized void addTask(PickTask task) { ... }
    public synchronized boolean hasTasks() { ... }
    public synchronized Optional<PickTask> getNextTask() { ... }
    public synchronized Optional<PickTask> peekNextTask() { ... }
    public synchronized Map<StationId, List<PickTask>> getTasksByStation() { ... }
    public synchronized boolean hasTasksForStation(StationId stationId) { ... }
    public synchronized Optional<PickTask> getNextTaskForStation(StationId stationId) { ... }
}
```

### Removed Fields (7 total)

- `state` → use `storageSpaceManager.deriveToteState(toteId)`
- `currentHoldingEntity` → use `storageSpaceManager.getToteLocation(toteId)`
- `currentCoordinate` → use `storageSpaceManager.getToteCoordinate(toteId)`
- `targetRelayId` → now in `TransportIntent.ToRelay`
- `targetStationId` → now in `TransportIntent.ToStation`
- `createdTime` → metrics system
- `completionTime` → metrics system

### Removed Methods

- `setBeingTransportedToRelay()`, `moveToRelay()`, `setBeingTransportedToStation()`
- `moveToStation()`, `setBeingReturnedToRelay()`, `setBeingReturnedToRack()`
- `returnToRack()`, `reset()`, `toDict()`

---

## Process Changes (VTM/HTM)

### VTM Forward Journey

```java
// Claim stays on tote
tote.setAssignedVtmId(vtm.getId());

// Travel to rack...

// Pick up tote
storageSpaceManager.moveToteFromRackToRobot(
    toteId, rackId, zIndex, vtm.getId(),
    RobotType.VTM, new TransportIntent.ToRelay(relayId));
vtm.addCarryingTote(tote);

// Travel to relay...

// Drop at relay
storageSpaceManager.moveToteFromRobotToRelay(toteId, relayId, relay);
vtm.removeCarryingTote(tote);
tote.setAssignedVtmId(null);
```

### VTM Return Journey

```java
tote.setAssignedVtmId(vtm.getId());

// Travel to relay...

// Pick up from relay
storageSpaceManager.moveToteFromRelayToRobot(
    toteId, relayId, vtm.getId(),
    RobotType.VTM, new TransportIntent.ReturningToRack());
vtm.addCarryingTote(tote);

// Travel to rack...

// Return to rack
storageSpaceManager.moveToteFromRobotToRack(toteId, rackId, zIndex);
vtm.removeCarryingTote(tote);
tote.setAssignedVtmId(null);
```

### HTM Pickup and Delivery

```java
// Pick up from relay
storageSpaceManager.moveToteFromRelayToRobot(
    toteId, relayId, htm.getId(),
    RobotType.HTM, new TransportIntent.ToStation(stationId));
htm.addCarryingTote(tote);
tote.setAssignedHtmId(htm.getId());

// Travel to station and process...

// Return to relay
storageSpaceManager.moveToteFromRobotToRelay(toteId, returnRelayId, returnRelay);
htm.removeCarryingTote(tote);
tote.setAssignedHtmId(null);
```

---

## Testing

### Tests to Update

Remove tests for deleted functionality:
- Tests for `tote.setState()`, `tote.setCurrentHoldingEntity()`
- Tests for state transition methods

Update tests to use StorageSpaceManager:
```java
// BEFORE
assertThat(tote.getState()).isEqualTo(ToteState.AT_RELAY_POSITION);

// AFTER
assertThat(storageSpaceManager.deriveToteState(toteId))
    .isEqualTo(ToteState.AT_RELAY_POSITION);
assertThat(storageSpaceManager.getToteLocation(toteId))
    .isInstanceOf(ToteLocation.AtRelay.class);
```

### New Tests Needed

- `ToteLocationTest` - test sealed type pattern matching
- `StorageSpaceManager.moveTote*()` - test orchestration updates location correctly
- `StorageSpaceManager.deriveToteState()` - test all state derivations
- Integration tests verifying VTM/HTM cycles update locations correctly

### Tests to Delete

- Any tests that rely on `Tote.state` field directly
- Tests for removed state transition methods

---

## Migration Strategy

1. **Add ToteLocation and new StorageSpaceManager methods** (additive, no breaking changes)
2. **Update VTM/HTM processes** to use orchestration methods
3. **Add validation** - temporarily check derived state matches old state field
4. **Remove deprecated fields** from Tote once validation passes
5. **Update snapshots/serialization** to use derived state

### Risk Mitigation

- Keep `ToteState` enum for API compatibility (derived, not stored)
- Run parallel validation during migration to catch any drift
- Integration test with full simulation before removing old fields

---

## Benefits

1. **Eliminates state drift bugs** - can't have inconsistent state when there's only one source
2. **Simpler Tote class** - 7 fewer fields, no state transition methods
3. **Centralized orchestration** - all moves go through StorageSpaceManager
4. **Better encapsulation** - containers don't leak their internal state
5. **Exhaustive state derivation** - sealed types ensure all cases handled

---

## Future Work

Apply same pattern to Rack:
- Remove `state`, `currentCoordinate`, `assignedRobotId` from Rack
- Create `RackLocation` sealed type
- StorageSpaceManager orchestrates rack moves

This will be done after Tote simplification is stable.
