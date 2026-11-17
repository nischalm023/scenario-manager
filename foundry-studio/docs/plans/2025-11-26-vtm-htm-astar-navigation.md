# VTM/HTM A* Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Introduce A* pathfinding for VTM and HTM robots with robot-type-specific traversability constraints (VTMs stay in aisles/zones, HTMs use highways).

**Architecture:** Add `RobotType` parameter to PathFinder, implement robot-type-specific traversability checks in `generateSuccessors()`, and replace all `moveToPosition()` calls in VTM/HTM processes with waypoint-based pathfinding loops.

**Tech Stack:** Java 21, Desmo-J simulation framework, A* pathfinding

---

## Prerequisites

Before starting, ensure:
1. Java backend compiles: `cd apps/backend && mvn clean compile -DskipTests`
2. Tests pass: `cd apps/backend && mvn test`

---

## Task 1: Add Area Classification Methods to WarehouseMap

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMap.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/map/WarehouseMapTest.java`

### Step 1.1: Write failing test for `isHighway()`

```java
// In WarehouseMapTest.java - add new test class or method

@Test
void testIsHighway_returnsTrue_forHighwayPosition() {
    // Given: A warehouse map with highways
    var config = createTestWarehouseMapConfig();  // Use existing test helper
    var map = new WarehouseMap(SystemType.RTP, config);

    // Get a known highway position from the map
    var highwaySegments = map.getHighwaySegments();
    assertFalse(highwaySegments.isEmpty(), "Test requires at least one highway");
    GridCoordinate highwayPos = highwaySegments.get(0).getAllCoordinates().get(0);

    // When/Then
    assertTrue(map.isHighway(highwayPos));
}

@Test
void testIsHighway_returnsFalse_forNonHighwayPosition() {
    var config = createTestWarehouseMapConfig();
    var map = new WarehouseMap(SystemType.RTP, config);

    // Get a rack position (definitely not a highway)
    var rackPositions = map.getRackPositions();
    assertFalse(rackPositions.isEmpty(), "Test requires at least one rack");
    GridCoordinate rackPos = rackPositions.get(0);

    assertFalse(map.isHighway(rackPos));
}
```

### Step 1.2: Run test to verify it fails

```bash
cd apps/backend && mvn test -Dtest=WarehouseMapTest#testIsHighway* -q
```

Expected: FAIL with "cannot find symbol: method isHighway"

### Step 1.3: Implement `isHighway()` with cached position set

Add to `WarehouseMap.java` after line 20 (field declarations):

```java
private final Set<GridCoordinate> highwayPositionsSet; // Cached for O(1) lookup
```

Add initialization in constructor after line 34 (`this.rackPositionsSet = ...`):

```java
this.highwayPositionsSet = calculateHighwayPositions();
```

Add method after `isRackPosition()` (around line 104):

```java
/**
 * Check if a position is on a highway. Used by pathfinding to restrict VTM movement.
 *
 * @param position Grid coordinate to check
 * @return true if position is on a highway, false otherwise
 */
public boolean isHighway(GridCoordinate position) {
    return highwayPositionsSet.contains(position);
}

private Set<GridCoordinate> calculateHighwayPositions() {
    return highwaySegments.stream()
        .flatMap(segment -> segment.getAllCoordinates().stream())
        .collect(Collectors.toSet());
}
```

### Step 1.4: Run test to verify it passes

```bash
cd apps/backend && mvn test -Dtest=WarehouseMapTest#testIsHighway* -q
```

Expected: PASS

### Step 1.5: Write failing test for `isAisle()`

```java
@Test
void testIsAisle_returnsTrue_forAislePosition() {
    var config = createTestWarehouseMapConfig();
    var map = new WarehouseMap(SystemType.RELAY, config);

    // Get aisle positions from rack blocks
    var rackBlocks = map.getRackBlocks();
    assertFalse(rackBlocks.isEmpty(), "Test requires at least one rack block");
    var aislePositions = rackBlocks.get(0).aislePositions();
    assertFalse(aislePositions.isEmpty(), "Test requires at least one aisle position");
    GridCoordinate aislePos = aislePositions.get(0);

    assertTrue(map.isAisle(aislePos));
}

@Test
void testIsAisle_returnsFalse_forHighwayPosition() {
    var config = createTestWarehouseMapConfig();
    var map = new WarehouseMap(SystemType.RELAY, config);

    var highwaySegments = map.getHighwaySegments();
    assertFalse(highwaySegments.isEmpty(), "Test requires at least one highway");
    GridCoordinate highwayPos = highwaySegments.get(0).getAllCoordinates().get(0);

    assertFalse(map.isAisle(highwayPos));
}
```

### Step 1.6: Run test to verify it fails

```bash
cd apps/backend && mvn test -Dtest=WarehouseMapTest#testIsAisle* -q
```

Expected: FAIL with "cannot find symbol: method isAisle"

### Step 1.7: Implement `isAisle()` with cached position set

Add field after `highwayPositionsSet`:

```java
private final Set<GridCoordinate> aislePositionsSet; // Cached for O(1) lookup
```

Add initialization in constructor:

```java
this.aislePositionsSet = calculateAislePositions();
```

Add method after `isHighway()`:

```java
/**
 * Check if a position is in an aisle (between rack blocks). Used by pathfinding to restrict HTM
 * movement.
 *
 * @param position Grid coordinate to check
 * @return true if position is in an aisle, false otherwise
 */
public boolean isAisle(GridCoordinate position) {
    return aislePositionsSet.contains(position);
}

private Set<GridCoordinate> calculateAislePositions() {
    return rackBlocks.stream()
        .flatMap(block -> block.aislePositions().stream())
        .collect(Collectors.toSet());
}
```

### Step 1.8: Run test to verify it passes

```bash
cd apps/backend && mvn test -Dtest=WarehouseMapTest#testIsAisle* -q
```

Expected: PASS

### Step 1.9: Write failing test for `isRelayPosition()`

```java
@Test
void testIsRelayPosition_returnsTrue_forRelayPosition() {
    var config = createTestWarehouseMapConfig();
    var map = new WarehouseMap(SystemType.RELAY, config);

    var relayPositions = map.getRelayPositions();
    assertFalse(relayPositions.isEmpty(), "Test requires relay positions for RELAY system");
    GridCoordinate relayPos = relayPositions.get(0);

    assertTrue(map.isRelayPosition(relayPos));
}

@Test
void testIsRelayPosition_returnsFalse_forRackPosition() {
    var config = createTestWarehouseMapConfig();
    var map = new WarehouseMap(SystemType.RELAY, config);

    var rackPositions = map.getRackPositions();
    GridCoordinate rackPos = rackPositions.get(0);

    assertFalse(map.isRelayPosition(rackPos));
}
```

### Step 1.10: Run test to verify it fails

```bash
cd apps/backend && mvn test -Dtest=WarehouseMapTest#testIsRelayPosition* -q
```

Expected: FAIL

### Step 1.11: Implement `isRelayPosition()`

Add field:

```java
private final Set<GridCoordinate> relayPositionsSet; // Cached for O(1) lookup
```

Add initialization in constructor (after relayPositions is set):

```java
this.relayPositionsSet = new HashSet<>(this.relayPositions);
```

Add method:

```java
/**
 * Check if a position is a relay position. Relay positions are traversable by both VTMs and HTMs.
 *
 * @param position Grid coordinate to check
 * @return true if position is a relay position, false otherwise
 */
public boolean isRelayPosition(GridCoordinate position) {
    return relayPositionsSet.contains(position);
}
```

### Step 1.12: Run test to verify it passes

```bash
cd apps/backend && mvn test -Dtest=WarehouseMapTest#testIsRelayPosition* -q
```

Expected: PASS

### Step 1.13: Commit Task 1

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMap.java
git add src/test/java/com/greyorange/dessimulator/domain/map/WarehouseMapTest.java
git commit -m "$(cat <<'EOF'
feat(navigation): add area classification methods to WarehouseMap

Add isHighway(), isAisle(), and isRelayPosition() methods with cached
position sets for O(1) lookup. These will be used by PathFinder to
implement robot-type-specific traversability constraints.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add RobotType Parameter to PathFinder

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/navigation/PathFinder.java`
- Test: `apps/backend/src/test/java/com/greyorange/dessimulator/domain/navigation/PathFinderTest.java`

### Step 2.1: Write failing test for VTM traversability constraint

```java
@Test
void testFindPath_vtmCannotTraverseHighways() {
    // Given: VTM needs to go from aisle to relay, but direct path crosses highway
    var map = createMapWithHighwayBetweenAisleAndRelay();
    var pathFinder = new PathFinder();

    GridCoordinate start = new GridCoordinate(5, 2);  // In aisle
    GridCoordinate goal = new GridCoordinate(5, 10);  // At relay (across highway)
    Orientation startOrientation = Orientation.EAST;

    // When: VTM plans path
    PathResult result = pathFinder.findPath(
        start,
        startOrientation,
        goal,
        map,
        RobotType.VTM,  // NEW PARAMETER
        false,  // isCarryingRack
        1.0,    // robotSpeed
        2.0     // baseTurnTime90
    );

    // Then: Path should exist but avoid highways
    assertFalse(result.isEmpty(), "VTM should find a path");
    for (PathWaypoint waypoint : result.waypoints()) {
        assertFalse(map.isHighway(waypoint.position()),
            "VTM path should not traverse highways: " + waypoint.position());
    }
}
```

### Step 2.2: Run test to verify it fails

```bash
cd apps/backend && mvn test -Dtest=PathFinderTest#testFindPath_vtmCannotTraverseHighways -q
```

Expected: FAIL (signature mismatch - RobotType parameter doesn't exist)

### Step 2.3: Update PathFinder.findPath() signature

Modify method signature in `PathFinder.java` (line 33):

```java
/**
 * Find the optimal path from start to goal.
 *
 * @param start Starting position
 * @param startOrientation Initial facing direction
 * @param goal Goal position (any final orientation)
 * @param map Warehouse map for obstacle checking
 * @param robotType Type of robot (affects traversability constraints)
 * @param isCarryingRack Whether the robot is carrying a rack (affects obstacle avoidance)
 * @param robotSpeed Robot speed in meters/second
 * @param baseTurnTime90 Time for 90-degree turn in seconds
 * @return PathResult with waypoints, or empty if unreachable
 */
public PathResult findPath(
    GridCoordinate start,
    Orientation startOrientation,
    GridCoordinate goal,
    WarehouseMap map,
    Robot.RobotType robotType,
    boolean isCarryingRack,
    double robotSpeed,
    double baseTurnTime90) {
```

Add import at top:

```java
import com.greyorange.dessimulator.domain.robot.Robot;
```

### Step 2.4: Update generateSuccessors() call

Update the call on line 93-94:

```java
List<PathState> successors =
    generateSuccessors(current, goal, map, robotType, isCarryingRack, robotSpeed, baseTurnTime90);
```

### Step 2.5: Update generateSuccessors() signature and add traversability check

Update method signature (around line 121):

```java
private List<PathState> generateSuccessors(
    PathState current,
    GridCoordinate goal,
    WarehouseMap map,
    Robot.RobotType robotType,
    boolean isCarryingRack,
    double robotSpeed,
    double baseTurnTime90) {
```

Add traversability helper method after `calculateActionTime()` (around line 289):

```java
/**
 * Check if a position is traversable by a specific robot type.
 *
 * <p>Traversability rules:
 * <ul>
 *   <li>VTM: Can use aisles, relay positions, rack positions. Cannot use highways.
 *   <li>HTM: Can use highways, relay positions, station access. Cannot use aisles.
 *   <li>RTP: Unrestricted (can use any traversable position).
 * </ul>
 *
 * @param position Position to check
 * @param robotType Type of robot
 * @param map Warehouse map for area classification
 * @return true if position is traversable by this robot type
 */
private boolean isTraversableByRobotType(
    GridCoordinate position, Robot.RobotType robotType, WarehouseMap map) {
  return switch (robotType) {
    case VTM -> !map.isHighway(position) || map.isRelayPosition(position);
    case HTM -> !map.isAisle(position) || map.isRelayPosition(position);
    case RTP -> true; // RTP robots are unrestricted
  };
}
```

Update the canMove check for MOVE_FORWARD (around line 142):

```java
boolean isAvailable = map.isPositionAvailable(forwardPos);
boolean isRackPos = map.isRackPosition(forwardPos);
boolean isGoal = forwardPos.equals(goal);
boolean isTraversable = isTraversableByRobotType(forwardPos, robotType, map);
boolean canMove = isAvailable && (!isCarryingRack || !isRackPos || isGoal) && isTraversable;
```

Update the canMove check for MOVE_BACKWARD (around line 172):

```java
boolean isAvailable = map.isPositionAvailable(backwardPos);
boolean isRackPos = map.isRackPosition(backwardPos);
boolean isGoal = backwardPos.equals(goal);
boolean isTraversable = isTraversableByRobotType(backwardPos, robotType, map);
boolean canMove = isAvailable && (!isCarryingRack || !isRackPos || isGoal) && isTraversable;
```

### Step 2.6: Run test to verify VTM constraint works

```bash
cd apps/backend && mvn test -Dtest=PathFinderTest#testFindPath_vtmCannotTraverseHighways -q
```

Expected: PASS

### Step 2.7: Write test for HTM traversability constraint

```java
@Test
void testFindPath_htmCannotTraverseAisles() {
    // Given: HTM needs to go from highway to station
    var map = createMapWithAisleBetweenHighwayAndStation();
    var pathFinder = new PathFinder();

    GridCoordinate start = new GridCoordinate(1, 5);  // On highway
    GridCoordinate goal = new GridCoordinate(0, 5);   // At station access
    Orientation startOrientation = Orientation.NORTH;

    // When: HTM plans path
    PathResult result = pathFinder.findPath(
        start,
        startOrientation,
        goal,
        map,
        RobotType.HTM,
        false,
        1.0,
        2.0
    );

    // Then: Path should avoid aisles
    assertFalse(result.isEmpty(), "HTM should find a path");
    for (PathWaypoint waypoint : result.waypoints()) {
        // Allow relay positions even if they're technically in aisle column
        if (!map.isRelayPosition(waypoint.position())) {
            assertFalse(map.isAisle(waypoint.position()),
                "HTM path should not traverse aisles: " + waypoint.position());
        }
    }
}
```

### Step 2.8: Run test to verify HTM constraint works

```bash
cd apps/backend && mvn test -Dtest=PathFinderTest#testFindPath_htmCannotTraverseAisles -q
```

Expected: PASS

### Step 2.9: Write test for RTP unrestricted movement

```java
@Test
void testFindPath_rtpCanTraverseAnywhere() {
    var map = createTestWarehouseMap();
    var pathFinder = new PathFinder();

    GridCoordinate start = new GridCoordinate(5, 2);  // In aisle
    GridCoordinate goal = new GridCoordinate(1, 10);  // On highway
    Orientation startOrientation = Orientation.EAST;

    // When: RTP plans path
    PathResult result = pathFinder.findPath(
        start,
        startOrientation,
        goal,
        map,
        RobotType.RTP,
        false,
        1.0,
        2.0
    );

    // Then: Path should exist (RTP can go anywhere)
    assertFalse(result.isEmpty(), "RTP should find a path");
}
```

### Step 2.10: Run all PathFinder tests

```bash
cd apps/backend && mvn test -Dtest=PathFinderTest -q
```

Expected: PASS

### Step 2.11: Commit Task 2

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/navigation/PathFinder.java
git add src/test/java/com/greyorange/dessimulator/domain/navigation/PathFinderTest.java
git commit -m "$(cat <<'EOF'
feat(navigation): add RobotType parameter to PathFinder

PathFinder.findPath() now accepts RobotType to enforce traversability:
- VTM: Cannot traverse highways (must stay in aisles/zones)
- HTM: Cannot traverse aisles (must use highways)
- RTP: Unrestricted movement

Relay positions are traversable by all robot types.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Update Robot.planPath() to Pass RobotType

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/domain/robot/Robot.java:193-203`
- Test: Existing tests should still pass

### Step 3.1: Update Robot.planPath() method

Modify `planPath()` in `Robot.java` (lines 193-203):

```java
/**
 * Plan a path from current position/orientation to destination.
 *
 * @param destination Goal position
 * @param pathFinder PathFinder instance (injected dependency)
 * @param map Warehouse map for obstacle checking
 * @param isCarryingCargo Whether robot is carrying cargo (rack for RTP, tote for VTM/HTM)
 * @return PathResult with waypoints, or empty if unreachable
 */
public PathResult planPath(
    GridCoordinate destination, PathFinder pathFinder, WarehouseMap map, boolean isCarryingCargo) {
  return pathFinder.findPath(
      this.currentPosition,
      this.currentOrientation,
      destination,
      map,
      this.robotType,  // Pass robot type for traversability constraints
      isCarryingCargo,
      this.speed,
      this.baseTurnTime90);
}
```

### Step 3.2: Run compilation check

```bash
cd apps/backend && mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

### Step 3.3: Run existing Robot tests

```bash
cd apps/backend && mvn test -Dtest=RobotTest -q
```

Expected: PASS (or create tests if none exist)

### Step 3.4: Commit Task 3

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/domain/robot/Robot.java
git commit -m "$(cat <<'EOF'
feat(navigation): pass robotType in Robot.planPath()

Robot.planPath() now passes this.robotType to PathFinder.findPath()
to enable robot-type-specific traversability constraints.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Update DesmojRtpRobotProcess for New Signature

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java`

The RTP process already uses `robot.planPath()` which now automatically passes the robot type. No changes needed to the process itself, but we need to verify it still works.

### Step 4.1: Run compilation check

```bash
cd apps/backend && mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

### Step 4.2: Run integration test

```bash
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```

Expected: PASS

### Step 4.3: Commit Task 4 (verification only)

```bash
git commit --allow-empty -m "$(cat <<'EOF'
verify: DesmojRtpRobotProcess compatible with new PathFinder signature

No code changes needed - Robot.planPath() handles robotType automatically.
Integration tests pass.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Replace moveToPosition() with A* Pathfinding in DesmojVtmProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`

There are 4 locations to update:
1. Line 303: `travelToRack()` - VTM moving to source rack
2. Line 332: `transportToRelay()` - VTM transporting tote to relay
3. Line 572: `executeReturnJourney()` - VTM moving to relay for pickup
4. Line 616: `executeReturnJourney()` - VTM moving to rack for unload

### Step 5.1: Add required imports

Add at top of file:

```java
import com.greyorange.dessimulator.domain.navigation.PathFinder;
import com.greyorange.dessimulator.domain.navigation.PathResult;
import com.greyorange.dessimulator.domain.navigation.PathWaypoint;
```

### Step 5.2: Create helper method for waypoint execution

Add after `holdForTravel()` method (around line 656):

```java
/**
 * Execute a path by iterating through waypoints.
 *
 * @param path PathResult from pathfinding
 * @param description Description for logging (e.g., "to rack", "to relay")
 * @return Total travel time in seconds (before navFactor)
 */
private double executePath(PathResult path, String description) throws SuspendExecution {
  if (path.isEmpty()) {
    log.error("VTM {}: No path found {}", vtm.getId(), description);
    throw new RuntimeException("Pathfinding failed - no path " + description);
  }

  double totalTime = 0.0;
  for (PathWaypoint waypoint : path.waypoints()) {
    vtm.executeAction(waypoint.action());
    holdForTravel(waypoint.timeDelta());
    totalTime += waypoint.timeDelta();
  }
  return totalTime;
}

/**
 * Plan and execute path from current position to destination.
 *
 * @param destination Target position
 * @param isCarryingTote Whether VTM is carrying a tote
 * @param description Description for logging
 * @return Travel time in seconds (before navFactor)
 */
private double navigateTo(GridCoordinate destination, boolean isCarryingTote, String description)
    throws SuspendExecution {
  DesmojWarehouseModel model = (DesmojWarehouseModel) getModel();
  PathFinder pathFinder = model.getPathFinder();

  PathResult path = vtm.planPath(
      destination,
      pathFinder,
      model.getWarehouseComponents().map(),
      isCarryingTote);

  return executePath(path, description);
}
```

### Step 5.3: Update travelToRack() method (line 301-306)

Replace:

```java
private double travelToRack(VtmCycleContext ctx) throws SuspendExecution {
  vtm.setState(RobotState.MOVING_TO_RACK);
  double travelTime = vtm.moveToPosition(ctx.sourceRack().getCurrentCoordinate());
  holdForTravel(travelTime);
  return travelTime;
}
```

With:

```java
private double travelToRack(VtmCycleContext ctx) throws SuspendExecution {
  vtm.setState(RobotState.MOVING_TO_RACK);
  // VTM is empty (not carrying tote), navigating to rack
  return navigateTo(ctx.sourceRack().getCurrentCoordinate(), false, "to rack");
}
```

### Step 5.4: Update transportToRelay() method (line 330-335)

Replace:

```java
private double transportToRelay(VtmCycleContext ctx) throws SuspendExecution {
  vtm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
  double travelTime = vtm.moveToPosition(ctx.relay().getPosition());
  holdForTravel(travelTime);
  return travelTime;
}
```

With:

```java
private double transportToRelay(VtmCycleContext ctx) throws SuspendExecution {
  vtm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
  // VTM is carrying tote, transporting to relay
  return navigateTo(ctx.relay().getPosition(), true, "to relay");
}
```

### Step 5.5: Update executeReturnJourney() - travel to relay (line 572-573)

Replace:

```java
double travelToRelay = vtm.moveToPosition(task.relayPosition().getPosition());
holdForTravel(travelToRelay);
```

With:

```java
// VTM is empty, moving to relay to pick up tote
double travelToRelay = navigateTo(task.relayPosition().getPosition(), false, "to relay for pickup");
```

### Step 5.6: Update executeReturnJourney() - travel to rack (line 616-617)

Replace:

```java
double travelToRack = vtm.moveToPosition(targetRack.getCurrentCoordinate());
holdForTravel(travelToRack);
```

With:

```java
// VTM is carrying tote, returning to rack
double travelToRack = navigateTo(targetRack.getCurrentCoordinate(), true, "to rack for unload");
```

### Step 5.7: Run compilation check

```bash
cd apps/backend && mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

### Step 5.8: Run integration test with relay system

```bash
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```

Expected: PASS

### Step 5.9: Commit Task 5

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java
git commit -m "$(cat <<'EOF'
feat(navigation): replace moveToPosition with A* pathfinding in VTM

VTM process now uses waypoint-based A* navigation instead of direct
Manhattan distance movement. This enables:
- Obstacle avoidance (racks, stations)
- Highway restriction (VTMs stay in aisles/zones)
- Realistic turn time accounting

Updated methods:
- travelToRack(): Navigate to source rack (empty)
- transportToRelay(): Navigate to relay (carrying tote)
- executeReturnJourney(): Navigate to relay and back to rack

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Replace moveToPosition() with A* Pathfinding in DesmojHtmProcess

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`

There are 3 locations to update:
1. Line 197: `travelToRelayAndLoad()` - HTM moving to relay
2. Line 230: `travelToStationAndProcess()` - HTM transporting tote to station
3. Line 271: `returnToRelay()` - HTM returning tote to relay

### Step 6.1: Add required imports

Add at top of file:

```java
import com.greyorange.dessimulator.domain.navigation.PathFinder;
import com.greyorange.dessimulator.domain.navigation.PathResult;
import com.greyorange.dessimulator.domain.navigation.PathWaypoint;
```

### Step 6.2: Create helper method for path execution

Add after `holdForTravel()` method (around line 413):

```java
/**
 * Execute a path by iterating through waypoints.
 *
 * @param path PathResult from pathfinding
 * @param description Description for logging
 * @return Total travel time in seconds (before navFactor)
 */
private double executePath(PathResult path, String description) throws SuspendExecution {
  if (path.isEmpty()) {
    log.error("HTM {}: No path found {}", htm.getId(), description);
    throw new RuntimeException("Pathfinding failed - no path " + description);
  }

  double totalTime = 0.0;
  for (PathWaypoint waypoint : path.waypoints()) {
    htm.executeAction(waypoint.action());
    holdForTravel(waypoint.timeDelta());
    totalTime += waypoint.timeDelta();
  }
  return totalTime;
}

/**
 * Plan and execute path from current position to destination.
 *
 * @param destination Target position
 * @param isCarryingTote Whether HTM is carrying a tote
 * @param description Description for logging
 * @return Travel time in seconds (before navFactor)
 */
private double navigateTo(GridCoordinate destination, boolean isCarryingTote, String description)
    throws SuspendExecution {
  PathFinder pathFinder = model.getPathFinder();

  PathResult path = htm.planPath(
      destination,
      pathFinder,
      model.getWarehouseComponents().map(),
      isCarryingTote);

  return executePath(path, description);
}
```

### Step 6.3: Update travelToRelayAndLoad() method (line 194-215)

Replace lines 196-198:

```java
htm.setState(RobotState.MOVING_TO_RACK);
double travelTime = htm.moveToPosition(ctx.sourceRelay().getPosition());
holdForTravel(travelTime);
```

With:

```java
htm.setState(RobotState.MOVING_TO_RACK);
// HTM is empty, moving to relay to pick up tote
double travelTime = navigateTo(ctx.sourceRelay().getPosition(), false, "to relay");
```

Note: Remove the duplicate `holdForTravel` since `navigateTo` handles it.

### Step 6.4: Update travelToStationAndProcess() method (line 229-231)

Replace lines 229-231:

```java
htm.setState(RobotState.TRANSPORTING_RACK_TO_STATION);
double travelTime = htm.moveToPosition(station.getPosition().position());
holdForTravel(travelTime);
```

With:

```java
htm.setState(RobotState.TRANSPORTING_RACK_TO_STATION);
// HTM is carrying tote, transporting to station
double travelTime = navigateTo(station.getPosition().position(), true, "to station");
```

### Step 6.5: Update returnToRelay() method (line 270-272)

Replace lines 270-272:

```java
htm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
double travelTime = htm.moveToPosition(returnRelay.getPosition());
holdForTravel(travelTime);
```

With:

```java
htm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
// HTM is carrying tote, returning to relay
double travelTime = navigateTo(returnRelay.getPosition(), true, "to return relay");
```

### Step 6.6: Run compilation check

```bash
cd apps/backend && mvn clean compile -DskipTests
```

Expected: BUILD SUCCESS

### Step 6.7: Run integration test with relay system

```bash
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```

Expected: PASS

### Step 6.8: Commit Task 6

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java
git commit -m "$(cat <<'EOF'
feat(navigation): replace moveToPosition with A* pathfinding in HTM

HTM process now uses waypoint-based A* navigation instead of direct
Manhattan distance movement. This enables:
- Obstacle avoidance (racks, stations)
- Aisle restriction (HTMs stay on highways)
- Realistic turn time accounting

Updated methods:
- travelToRelayAndLoad(): Navigate to relay (empty)
- travelToStationAndProcess(): Navigate to station (carrying tote)
- returnToRelay(): Navigate to return relay (carrying tote)

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Set Initial Orientation for VTM/HTM Robots

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java`

VTM and HTM robots need a meaningful initial orientation for A* pathfinding.

### Step 7.1: Locate robot initialization

Search for where VTM and HTM robots are created in `DesmojWarehouseModel.java`.

### Step 7.2: Ensure robots have initial orientation

If robots are created without explicit orientation, update to include one:

```java
// Example for VTM creation
Robot vtm = new Robot(
    vtmId,
    config.vtmSpeed(),
    Robot.RobotType.VTM,
    Orientation.EAST,  // Initial orientation
    Robot.BASE_TURN_TIME_90,
    zoneAssignments
);

// Example for HTM creation
Robot htm = new Robot(
    htmId,
    config.htmSpeed(),
    Robot.RobotType.HTM,
    Orientation.EAST,  // Initial orientation
    Robot.BASE_TURN_TIME_90
);
```

### Step 7.3: Run compilation and tests

```bash
cd apps/backend && mvn clean compile -DskipTests
cd apps/backend && mvn test -Dtest=DesmojIntegrationTest -q
```

Expected: PASS

### Step 7.4: Commit Task 7

```bash
cd apps/backend && mvn spotless:apply
git add src/main/java/com/greyorange/dessimulator/simulator/DesmojWarehouseModel.java
git commit -m "$(cat <<'EOF'
feat(navigation): set initial orientation for VTM/HTM robots

VTM and HTM robots now have explicit initial orientation (EAST) for
A* pathfinding. This ensures the first path calculation has a valid
starting orientation.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Integration Testing and Verification

### Step 8.1: Run full test suite

```bash
cd apps/backend && mvn clean test
```

Expected: All tests PASS

### Step 8.2: Run manual simulation test

```bash
# Start server
cd apps/backend
rm -f /tmp/server.log
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15

# Test relay system simulation
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d '{
    "systemType": "relay",
    "duration": 1,
    "vtmCount": 4,
    "htmCount": 4,
    "stationsCount": 2
  }' | jq '.metrics.taskMetrics'

# Cleanup
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
```

Expected: Simulation completes with task metrics showing picks processed

### Step 8.3: Verify path logs (optional debug)

Check logs for pathfinding activity:

```bash
grep -i "path\|waypoint" /tmp/server.log | head -20
```

### Step 8.4: Final commit

```bash
git commit --allow-empty -m "$(cat <<'EOF'
verify: VTM/HTM A* navigation integration complete

All tests pass. VTM and HTM robots now use A* pathfinding with:
- Robot-type-specific traversability constraints
- Waypoint-based movement execution
- Realistic turn time accounting

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Summary of Changes

| File | Changes |
|------|---------|
| `WarehouseMap.java` | Added `isHighway()`, `isAisle()`, `isRelayPosition()` with cached sets |
| `PathFinder.java` | Added `RobotType` parameter and `isTraversableByRobotType()` check |
| `Robot.java` | Updated `planPath()` to pass `this.robotType` |
| `DesmojVtmProcess.java` | Replaced 4 `moveToPosition()` calls with `navigateTo()` |
| `DesmojHtmProcess.java` | Replaced 3 `moveToPosition()` calls with `navigateTo()` |
| `DesmojWarehouseModel.java` | Ensured VTM/HTM robots have initial orientation |

## Risk Mitigation

1. **Pathfinding failures**: Each `navigateTo()` throws RuntimeException if no path found - ensures failures are visible
2. **Performance**: A* with MAX_ITERATIONS=20000 limit prevents runaway searches
3. **Backward compatibility**: RTP process unchanged, uses same `planPath()` method
4. **Metrics**: Travel times still tracked correctly (navFactor applied in `holdForTravel`)
