# Pathfinding Phase 1: Orientation-Aware A* Navigation

**Date**: 2025-11-12
**Status**: ✅ Implemented (with critical enhancements)
**Scope**: RTP robots only (relay system VTM/HTM in future phases)

## Executive Summary

Replace current Manhattan distance "teleportation" with realistic pathfinding that considers:
- Robot orientation (N/S/E/W facing direction)
- Turn costs (linear with angle: 90° < 180°)
- Forward/backward movement (robot must stop to turn)
- Static obstacle avoidance (racks, walls)
- **Load state awareness** (empty vs carrying rack) - Critical requirement discovered during implementation

**Algorithm**: A* with expanded state space (position + orientation + load state)
**Impact**: More realistic travel times, enables future physics modeling

---

## Implementation Findings

### Critical Missing Requirements

During implementation, a critical domain requirement was discovered that was not captured in the original design:

**RTP Robot Load States**: RTP robots have two distinct physical states that affect pathfinding:

1. **Empty (lifted down)**: Robot can pass through rack positions to reach target racks for pickup
2. **Loaded (carrying rack, lifted up)**: Robot CANNOT pass through other rack positions due to physical interference

**Impact on Design**: The pathfinding API required an additional `isCarryingRack` boolean parameter to switch between these two traversability modes. This is not just an optimization - it's a correctness requirement. Without it, loaded robots would attempt impossible paths through other racks.

**Workflow Example**:
```
Robot at position A, rack at position B (with rack C in between):
- Empty: A → C → B (can pass through C to reach B) ✓
- Loaded: A → (detour around C) → B (must avoid C) ✓
- Return journey: B → (detour around C) → storage (still loaded) ✓
```

**Implementation**: See updated PathFinder signature below and Fix 4 in implementation plan.

### Robot Lifecycle Phase Separation

The original design did not specify the granularity of the robot lifecycle methods. During implementation, a critical bug was discovered:

**Problem**: The initial implementation had a single `transportRackToStation()` method that performed:
1. Move to rack position (empty robot)
2. Load rack
3. Transport to station (loaded robot)

This method was called FOR EACH station in multi-station orders, causing robots to attempt picking up racks from station positions on the second iteration.

**Solution**: Split into two methods:
- `pickUpRack()`: Called ONCE before station loop
- `transportToStation()`: Called for EACH station visit

**Impact**: This is a critical lifecycle design pattern for multi-destination trips. The original design did not specify this level of detail.

### Empty Path Semantics

The design did not specify how to handle the case where `start == goal`. During implementation, we clarified:
- Pathfinding returns empty waypoints (0 waypoints, 0 time)
- This is NOT an error - robot is already at destination
- Distinct from "no path found" (unreachable destination)

---

## Requirements (Updated)

### Movement Model
- Robot has orientation state (NORTH, SOUTH, EAST, WEST)
- Can move forward OR backward in current heading direction
- Must **stop completely** before turning
- Turn time is **linear with angle**: `turnTime = (degrees / 90) * baseTurnTime90`
- Can turn to any direction (not restricted to 90° increments from current)

### Pathfinding Behavior
- **Once per trip**: Calculate path at start of movement, follow completely (no replanning)
- **Static obstacles only**: Avoid racks, walls, out-of-bounds (ignore other robots)
- **Highways ignored**: Treat highway cells as regular open cells (Phase 1 baseline)
- **Goal matching**: Path ends when robot reaches goal position (any final orientation)
  - Note: Future enhancement will require specific final orientation

### Optimization Goal
- **Minimize total time** (movement time + turn time), not just distance
- A* should find time-optimal paths considering both travel and rotation costs

---

## Architecture

### Component Overview

```
┌─────────────────┐
│  PathFinder     │  Core A* algorithm
│  (stateless)    │  - Expanded state: (position, orientation)
└────────┬────────┘  - Returns PathResult with waypoints
         │
         │ Used by
         ▼
┌─────────────────┐
│  Robot          │  Domain entity
│  + orientation  │  - Tracks current position + orientation
│  + executeAction│  - Executes navigation actions
└────────┬────────┘
         │
         │ Controlled by
         ▼
┌──────────────────────┐
│ DesmojRtpRobotProcess│  Simulation process
│ (waypoint loop)      │  - Plans path once per trip
└──────────────────────┘  - Follows waypoint by waypoint with hold()
```

### State Space

**Search State**: `(GridCoordinate position, Orientation orientation)`
- **Size**: gridWidth × gridDepth × 4 = 22 × 26 × 4 = **2,288 states**
- **Uniqueness**: (5, 10, NORTH) ≠ (5, 10, EAST)

**Actions from each state**:
1. `MOVE_FORWARD` - advance 1 cell in facing direction
2. `MOVE_BACKWARD` - retreat 1 cell opposite facing direction
3. `TURN_90_CW` - rotate 90° clockwise
4. `TURN_90_CCW` - rotate 90° counter-clockwise
5. `TURN_180` - reverse facing direction

**Edge Costs**:
- Movement: `1.0 / robotSpeed` seconds per grid cell
- Turn 90°: `baseTurnTime90` seconds
- Turn 180°: `2 * baseTurnTime90` seconds

---

## A* Algorithm Details

### Heuristic Function (Admissible)

```java
private double heuristic(GridCoordinate current, GridCoordinate goal, double speed) {
    return current.manhattanDistance(goal) / speed;
}
```

**Why ignore turn costs?**
- Must never overestimate true cost (admissibility requirement)
- Minimum turns to goal unknown without solving problem
- Pure distance provides valid lower bound
- Still guarantees optimal path, just explores slightly more states

### Successor Generation

**Movement successors** (check obstacles + load state):
```java
// Forward
GridCoordinate forwardPos = current.position.add(current.orientation.delta());
boolean isAvailable = map.isPositionAvailable(forwardPos);
boolean isRackPos = map.isRackPosition(forwardPos);
boolean isGoal = forwardPos.equals(goal);

// Can move if: position available AND (not carrying OR not rack OR is goal)
boolean canMove = isAvailable && (!isCarryingRack || !isRackPos || isGoal);
if (canMove) {
    addSuccessor(forwardPos, current.orientation, MOVE_FORWARD, 1.0 / speed);
}

// Backward (same logic)
GridCoordinate backwardPos = current.position.subtract(current.orientation.delta());
isAvailable = map.isPositionAvailable(backwardPos);
isRackPos = map.isRackPosition(backwardPos);
isGoal = backwardPos.equals(goal);
canMove = isAvailable && (!isCarryingRack || !isRackPos || isGoal);
if (canMove) {
    addSuccessor(backwardPos, current.orientation, MOVE_BACKWARD, 1.0 / speed);
}
```

**Key Logic**:
- Empty robots (`isCarryingRack = false`): Can go anywhere that's available
- Loaded robots (`isCarryingRack = true`): Avoid racks EXCEPT the goal position
- Goal exception: `|| isGoal` allows loaded robots to reach their destination rack

**Turn successors** (always available, position unchanged):
```java
addSuccessor(current.position, current.orientation.rotateCW(), TURN_90_CW, baseTurnTime90);
addSuccessor(current.position, current.orientation.rotateCCW(), TURN_90_CCW, baseTurnTime90);
addSuccessor(current.position, current.orientation.reverse(), TURN_180, baseTurnTime90 * 2);
```

### Priority Queue

States ordered by **f-cost**:
- `fCost = gCost + heuristic`
- `gCost` = actual accumulated time from start (movement + turns)
- `heuristic` = Manhattan distance / speed (ignores turns)

---

## Data Structures

### Core Classes (Java)

**PathFinder** (new)
```java
package com.greyorange.dessimulator.domain.navigation;

public class PathFinder {
    public PathResult findPath(
        GridCoordinate start,
        Orientation startOrientation,
        GridCoordinate goal,
        WarehouseMap map,
        boolean isCarryingRack,  // NEW - Load state awareness
        double robotSpeed,
        double baseTurnTime90
    );
}
```

**Load State Parameter**:
- `isCarryingRack = false`: Robot is empty, can pass through rack positions
- `isCarryingRack = true`: Robot is loaded, must avoid rack positions (except goal)
- Exception: Goal position is always reachable even when loaded

**Supporting Types** (new)
```java
// Unique identifier for visited tracking
record StateKey(GridCoordinate position, Orientation orientation) { }

// State in A* search
record PathState(
    GridCoordinate position,
    Orientation orientation,
    double gCost,
    double fCost,
    NavigationAction actionToReach
) implements Comparable<PathState> { }

// Actions robot can take
enum NavigationAction {
    MOVE_FORWARD, MOVE_BACKWARD,
    TURN_90_CW, TURN_90_CCW, TURN_180
}

// Direction robot is facing
enum Orientation {
    NORTH(0, -1), SOUTH(0, 1), EAST(1, 0), WEST(-1, 0);

    public GridCoordinate delta();
    public Orientation rotateCW();
    public Orientation rotateCCW();
    public Orientation reverse();
}

// Result returned by PathFinder
record PathResult(
    List<PathWaypoint> waypoints,
    double totalTime,
    double totalDistance,
    int totalTurns
) { }

// Individual step in path
record PathWaypoint(
    GridCoordinate position,
    Orientation orientation,
    NavigationAction action,
    double timeDelta,        // Time for this action
    double cumulativeTime    // Total time from start
) { }
```

### Modified Classes

**Robot.java**
```java
public class Robot {
    private GridCoordinate currentPosition;
    private Orientation currentOrientation;  // NEW
    private final double speed;
    private final double baseTurnTime90;     // NEW

    public PathResult planPath(
        GridCoordinate destination,
        PathFinder pathFinder,
        WarehouseMap map,
        boolean isCarryingRack  // NEW - Load state parameter
    );

    public void executeAction(NavigationAction action);  // NEW
}
```

**DesmojRtpRobotProcess.java** (process loop changes)
```java
// BEFORE (current - line ~275):
double travelTime = robot.moveToPosition(rack.getCurrentCoordinate());
hold(new TimeSpan(travelTime, TimeUnit.SECONDS));

// AFTER (split pickup from transport):

// 1. Pick up rack ONCE (before station loop)
PickUpMetrics pickUpMetrics = pickUpRack(selectedRack);

// 2. Visit each station
for (StationId stationId : stationsToVisit) {
    Station station = stationManager.getStation(stationId);

    // Transport to this station (robot already carrying rack)
    TransportMetrics transportMetrics =
        transportToStation(selectedRack, station.getPosition().getAccessPosition());

    // Process tasks at station...
}

// 3. Return rack to storage
ReturnTransportMetrics returnMetrics = returnRackToStorage(selectedRack);

// === Helper Methods ===

private PickUpMetrics pickUpRack(Rack rack) {
    // Move to rack (EMPTY - can go through racks)
    PathResult pathToRack = robot.planPath(
        rack.getCurrentCoordinate(),
        pathFinder,
        warehouseMap,
        false  // isCarryingRack = false (empty)
    );

    for (PathWaypoint waypoint : pathToRack.waypoints()) {
        robot.executeAction(waypoint.action());
        hold(new TimeSpan(waypoint.timeDelta(), TimeUnit.SECONDS));
    }

    // Load rack...
}

private TransportMetrics transportToStation(Rack rack, GridCoordinate stationPos) {
    // Move to station (LOADED - avoid racks)
    PathResult pathToStation = robot.planPath(
        stationPos,
        pathFinder,
        warehouseMap,
        true  // isCarryingRack = true (loaded)
    );

    for (PathWaypoint waypoint : pathToStation.waypoints()) {
        robot.executeAction(waypoint.action());
        hold(new TimeSpan(waypoint.timeDelta(), TimeUnit.SECONDS));
    }

    rack.moveToStation(stationPos);
}

private ReturnTransportMetrics returnRackToStorage(Rack rack) {
    // Return to storage (LOADED - avoid racks except goal)
    PathResult pathToStorage = robot.planPath(
        rack.getStorageCoordinate(),
        pathFinder,
        warehouseMap,
        true  // isCarryingRack = true (loaded)
    );

    for (PathWaypoint waypoint : pathToStorage.waypoints()) {
        robot.executeAction(waypoint.action());
        hold(new TimeSpan(waypoint.timeDelta(), TimeUnit.SECONDS));
    }

    rack.moveToStorage(rack.getStorageCoordinate());
}
```

**Critical Pattern**: Separate `pickUpRack()` (once) from `transportToStation()` (per destination) to avoid attempting to pick up racks from station positions on multi-station orders.

---

## Integration

### Dependency Injection

**PathFinder as shared component**:
1. Created once in `DesmojWarehouseModel.init()`
2. Passed to RTP robot processes via constructor
3. Stateless - safe for concurrent use by multiple robots

### Robot Initialization

**Orientation must be set at spawn**:
- Default: Face nearest station? Face East?
- Configuration parameter: `initialRobotOrientation`
- All robots start with same orientation (Phase 1 simplification)

### Configuration Changes

Add to simulation config:
```json
{
  "baseTurnTime90Degrees": 2.0,  // seconds for 90° turn
  "initialRobotOrientation": "EAST"
}
```

---

## Testing Strategy

### Unit Tests

**PathFinderTest.java**:
- Empty grid pathfinding (0,0)→(5,5)
- Obstacle avoidance with rack barriers
- Orientation scenarios:
  - Goal directly behind (should move backward)
  - Goal requires 180° turn
  - Complex path with multiple turns
- Edge cases:
  - Start = goal (empty path)
  - Unreachable goal (return empty Optional)
  - Robot facing away from goal

**OrientationTest.java**:
- Rotation methods (rotateCW, rotateCCW, reverse)
- Delta calculation for each direction
- Round-trip rotations (4× CW = identity)

### Integration Tests

**DesmojRtpPathfindingTest.java**:
- Small warehouse (10×10) with obstacles
- Place rack behind barriers
- Verify:
  - Robot reaches rack successfully
  - Total time = sum of waypoint times
  - Orientation updates correctly
  - No movement through obstacles

**Performance Test**:
- Worst-case: Opposite corners with maze obstacles
- Measure pathfinding time (target: < 100ms for 22×26 grid)
- Memory usage validation

### Validation Test (Old vs New)

Run same scenario with:
1. Old system: Manhattan teleportation
2. New system: Orientation-aware pathfinding

Compare:
- New system should have **longer travel times** (turn costs + routing)
- Verify turn costs impact metrics correctly
- Ensure simulation completes successfully

---

## File Changes

### New Files

```
apps/backend/src/main/java/com/greyorange/dessimulator/domain/navigation/
├── PathFinder.java
├── PathResult.java
├── PathWaypoint.java
├── NavigationAction.java
├── Orientation.java
├── PathState.java (package-private)
└── StateKey.java (package-private)

apps/backend/src/test/java/com/greyorange/dessimulator/domain/navigation/
├── PathFinderTest.java
└── OrientationTest.java

apps/backend/src/test/java/com/greyorange/dessimulator/desmoj/
└── DesmojRtpPathfindingTest.java
```

### Modified Files

- `Robot.java` - add orientation, baseTurnTime90, executeAction(), planPath()
- `DesmojRtpRobotProcess.java` - replace single hold() with waypoint loop
- `DesmojWarehouseModel.java` - create PathFinder, inject into RTP processes
- Integration test config JSON - add turn time parameters

---

## Performance Considerations

### State Space Size
- 22 × 26 × 4 = 2,288 states (trivial for A*)
- Even 100×100 warehouse = 40,000 states (still fast)

### Pathfinding Frequency
- Once per robot trip (not every time step)
- ~4 robots × ~10 trips/hour = 40 path calculations per simulation hour
- Total for 4-hour sim: ~160 pathfinding calls
- Expected: < 50ms per call = ~8 seconds total overhead

### Memory Usage
- Priority queue: O(states) = O(width × height × 4)
- Visited set: Same
- Path storage: O(path length) per robot
- Negligible impact for grid sizes < 100×100

---

## Future Enhancements (Out of Scope for Phase 1)

### Phase 2: Physics
- Velocity/acceleration state in Robot
- Trapezoidal velocity profiles (accelerate → cruise → decelerate)
- Break movement into smaller time steps (0.1s updates)
- Energy consumption modeling

### Phase 3: Collision Avoidance
- Dynamic obstacle avoidance (other robots)
- Time-based path reservation system
- Replanning when paths become blocked
- Highway capacity limits with Desmo-J resources

### Phase 4: Optimizations
- Highway preference (weighted A*)
- Path caching for common routes
- Bidirectional A* for long paths
- Multi-agent path planning (coordinated routing)

### Other
- Require specific final orientation at goal
- Different initial orientations per robot
- Dynamic turn time based on load (loaded robots turn slower)
- Support for relay system (VTM/HTM processes)

---

## Success Criteria

✅ **Correctness**: Robots reach destinations without passing through obstacles
✅ **Optimality**: Paths minimize total time (movement + turns)
✅ **Realism**: Travel times reflect turn costs (longer than Manhattan teleportation)
✅ **Performance**: Pathfinding completes in < 100ms per call
✅ **Integration**: Works seamlessly with existing RTP robot processes
✅ **Testing**: Unit tests pass, integration tests validate end-to-end behavior

---

## Risk Assessment

### Low Risk
- ✅ State space size manageable (2,288 states)
- ✅ A* well-understood algorithm
- ✅ Clean integration points identified

### Medium Risk
- ⚠️ Initial orientation configuration (need sensible default)
- ⚠️ Backward movement edge cases (back into obstacles?)
- ⚠️ Path reconstruction correctness (careful with action sequence)

### Mitigation
- Comprehensive unit tests for edge cases
- Integration test with visual validation (if possible)
- Start with simple warehouse layouts, gradually add complexity

---

## Implementation Results

### Status: ✅ Complete

All requirements implemented successfully with critical enhancements discovered during implementation.

### Key Metrics

**Implementation Commits**: 16 commits total
- Initial implementation: 10 commits
- Load-state-aware pathfinding: 3 commits
- Diagnostic logging: 2 commits
- Multi-station order bug fix: 1 commit

**Test Results**:
- Unit tests: All passing (PathFinderTest, OrientationTest)
- Integration tests: Simulation completes successfully
- Verification: 641 successful rack returns, 0 pathfinding errors

**Files Changed**:
- New: 7 navigation domain classes + tests
- Modified: 3 core classes (Robot, DesmojRtpRobotProcess, DesmojWarehouseModel)
- Modified: 2 supporting classes (WarehouseMap, RobotMetrics)

### Discovered Requirements

Three critical requirements discovered during implementation that were not in the original design:

1. **Load-State-Aware Pathfinding**: `isCarryingRack` parameter required (Fix 4)
2. **Lifecycle Phase Separation**: Split pickup from transport for multi-station orders (Fix 5)
3. **Empty Path Handling**: Distinguish "already there" from "unreachable" (Fix 6)

See detailed analysis in [Implementation Plan](./2025-11-12-pathfinding-phase1-implementation.md#post-implementation-summary).

### Lessons Learned

1. **Domain complexity emerges during implementation**: Generic A* is insufficient for warehouse robotics. Physical robot states (load) must be modeled in the pathfinding API.

2. **Lifecycle design matters**: Methods that mix phases (pickup + transport) become bugs when used in loops. Clear separation of concerns is critical.

3. **Testing reveals edge cases**: Unit tests passed, but integration testing revealed semantic issues around load states and multi-station orders.

4. **Document as-built, not as-designed**: This design document has been updated to reflect the actual implementation, including all critical enhancements.

---

## References

### Code References
- Implemented PathFinder: `PathFinder.java` (domain/navigation)
- Modified Robot: `Robot.java` with orientation and planPath()
- Modified RTP process: `DesmojRtpRobotProcess.java` with pickUpRack/transportToStation split
- Grid coordinates: `GridCoordinate.java` with vector math (add/subtract)
- Warehouse map: `WarehouseMap.java` with isRackPosition() method

### Documentation References
- Implementation plan: [pathfinding-phase1-implementation.md](./2025-11-12-pathfinding-phase1-implementation.md)
- Post-implementation summary: See Fix 4, Fix 5, Fix 6 in implementation plan
- Commit history: See feature-navigation branch (commits a4740a2 through bc8bd92)
