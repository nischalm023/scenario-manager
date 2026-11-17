# VTM/HTM A* Navigation - Design Document

## Problem Statement

### Current State

The warehouse simulation uses two navigation approaches:

1. **RTP (Racks-to-Picker) robots**: Full A* pathfinding with orientation awareness, obstacle avoidance, and realistic turn time calculations.

2. **VTM/HTM (Relay system) robots**: Simplified Manhattan distance teleportation via `moveToPosition()` - instant position change with calculated travel time, but no actual path planning.

```java
// Current VTM/HTM navigation (simplified)
double travelTime = robot.moveToPosition(destination);  // Teleport + calculate time
holdForTravel(travelTime);  // Wait for calculated time
```

```java
// Current RTP navigation (full pathfinding)
PathResult path = robot.planPath(destination, pathFinder, map, isCarryingRack);
for (PathWaypoint waypoint : path.waypoints()) {
    robot.executeAction(waypoint.action());  // Move step by step
    holdForTravel(waypoint.timeDelta());
}
```

### Problems with Current Approach

1. **No obstacle avoidance**: VTM/HTM robots "teleport" through racks, stations, and other obstacles. This is unrealistic and doesn't model real warehouse constraints.

2. **No movement restrictions**: In real warehouses:
   - **VTMs** operate within narrow aisles between rack blocks - they cannot traverse highways
   - **HTMs** travel on highways to move totes between relay positions and stations - they should not cut through aisles

3. **No turn time accounting**: Manhattan distance ignores robot orientation. A robot facing NORTH that needs to go EAST must first turn 90° (typically 2 seconds). Current VTM/HTM navigation ignores this.

4. **Inconsistent simulation fidelity**: RTP robots have realistic pathfinding while VTM/HTM robots use a simplified model. This creates inconsistency when comparing system types.

### Business Impact

- **Inaccurate TPQ estimates**: Travel times are underestimated because turn times and detours around obstacles are not modeled
- **Unrealistic congestion modeling**: Robots passing through obstacles don't compete for space
- **Invalid system comparisons**: RTP vs Relay comparisons are skewed by different navigation fidelity

---

## Proposed Solution

### Overview

Extend the existing A* pathfinding system to support **robot-type-specific traversability constraints**, then replace all `moveToPosition()` calls in VTM/HTM processes with waypoint-based pathfinding.

### Design Principles

1. **Single PathFinder, Multiple Behaviors**: Instead of creating separate VtmPathFinder/HtmPathFinder classes, add a `RobotType` parameter to the existing `findPath()` method. This keeps pathfinding logic centralized and maintainable.

2. **Explicit Area Classification**: Add `isHighway()`, `isAisle()`, `isRelayPosition()` methods to `WarehouseMap` with O(1) cached lookups. This makes traversability rules explicit and testable.

3. **Relay Positions as Shared Zones**: Relay positions are special - they must be accessible by both VTMs (to drop off totes) and HTMs (to pick up totes). The traversability logic must account for this.

4. **Preserve Existing Patterns**: Use the same waypoint execution loop that RTP robots use. This ensures consistent behavior across all robot types.

---

## Architecture

### Traversability Matrix

| Area Type | VTM | HTM | RTP |
|-----------|-----|-----|-----|
| Rack positions | ✅ (to pick totes) | ❌ | ✅ |
| Aisles (between rack blocks) | ✅ | ❌ | ✅ |
| Relay positions | ✅ | ✅ | N/A |
| Highways | ❌ | ✅ | ✅ |
| Station access positions | ❌ | ✅ | ✅ |

### Warehouse Layout Visualization

```
┌─────────────────────────────────────────────────────────────┐
│  STATION   STATION   STATION                    [HIGHWAY]   │
├─────────────────────────────────────────────────────────────┤
│  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │  ← Highway (HTM only)
├─────────────────────────────────────────────────────────────┤
│  R│▓│R│▓│R│▓│R  │  │  R│▓│R│▓│R│▓│R  │  │  R│▓│R│▓│R│▓│R  │
│  A│░│A│░│A│░│A  │A │  A│░│A│░│A│░│A  │A │  A│░│A│░│A│░│A  │
│  C│░│C│░│C│░│C  │I │  C│░│C│░│C│░│C  │I │  C│░│C│░│C│░│C  │
│  K│░│K│░│K│░│K  │S │  K│░│K│░│K│░│K  │S │  K│░│K│░│K│░│K  │
│   │░│ │░│ │░│   │L │   │░│ │░│ │░│   │L │   │░│ │░│ │░│   │
│   │░│ │░│ │░│   │E │   │░│ │░│ │░│   │E │   │░│ │░│ │░│   │
│   │▓│ │▓│ │▓│   │  │   │▓│ │▓│ │▓│   │  │   │▓│ │▓│ │▓│   │
├─────────────────────────────────────────────────────────────┤
│  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │  ← Highway (HTM only)
└─────────────────────────────────────────────────────────────┘

Legend:
  R = Rack position (VTM can access)
  ▓ = Relay position (VTM + HTM can access)
  ░ = Highway (HTM only)
  │ = Aisle (VTM only)
```

### Component Changes

```
┌─────────────────────────────────────────────────────────────┐
│                      PathFinder                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  findPath(start, goal, map, robotType, ...)         │    │
│  │                                                      │    │
│  │  generateSuccessors()                               │    │
│  │    └─> isTraversableByRobotType(pos, robotType)     │    │
│  │          └─> switch(robotType) {                    │    │
│  │                VTM -> !isHighway || isRelay         │    │
│  │                HTM -> !isAisle || isRelay           │    │
│  │                RTP -> true                          │    │
│  │              }                                      │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     WarehouseMap                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  isHighway(position) -> highwayPositionsSet.contains │    │
│  │  isAisle(position)   -> aislePositionsSet.contains   │    │
│  │  isRelayPosition(pos)-> relayPositionsSet.contains   │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                              │
│  Cached Sets (O(1) lookup):                                 │
│  - highwayPositionsSet: from HighwayGridSegment             │
│  - aislePositionsSet: from RackBlockGridDefinition          │
│  - relayPositionsSet: from relayPositions list              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                        Robot                                 │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  planPath(destination, pathFinder, map, isCarrying) │    │
│  │    └─> pathFinder.findPath(..., this.robotType, ...)│    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

### Process Flow (After Implementation)

```
VTM Forward Journey:
  1. findToteWithTasks() -> select tote in zone
  2. navigateTo(rack)    -> A* path through aisles (avoiding highways)
  3. loadTote()          -> pick up tote
  4. navigateTo(relay)   -> A* path to relay position
  5. dropAtRelay()       -> hand off to HTM

HTM Forward Journey:
  1. findRelayWithTotes() -> select relay with totes
  2. navigateTo(relay)    -> A* path via highways
  3. loadTote()           -> pick up from relay
  4. navigateTo(station)  -> A* path to station (via highways)
  5. processAtStation()   -> operator picks items
  6. navigateTo(returnRelay) -> A* path back to relay
  7. dropAtRelay()        -> hand off to VTM for return
```

---

## Design Decisions

### Decision 1: Parameter-Based vs Subclass-Based PathFinder

**Options Considered:**

| Option | Pros | Cons |
|--------|------|------|
| A. Add `RobotType` parameter | Single class, minimal changes, clear call site | Slightly more complex switch logic |
| B. VtmPathFinder/HtmPathFinder subclasses | Type-safe, OCP compliant | More classes, inheritance overhead |
| C. Separate obstacle maps per robot type | Pre-computed, O(1) always | 3x memory, duplication |

**Decision**: Option A (Parameter-Based)

**Rationale**:
- Pathfinding logic is already complex - adding 2 subclasses increases maintenance burden
- Robot type is known at call time - no need for compile-time type safety
- Performance is already O(1) with cached position sets in WarehouseMap
- Single source of truth for pathfinding algorithm

### Decision 2: Where to Enforce Traversability

**Options Considered:**

| Option | Description |
|--------|-------------|
| A. In PathFinder.generateSuccessors() | Filter invalid moves during search |
| B. In WarehouseMap.isPositionAvailable() | Make traversability part of availability |
| C. In Robot.planPath() | Pre-filter before calling PathFinder |

**Decision**: Option A (In PathFinder)

**Rationale**:
- `isPositionAvailable()` is for static obstacles (stations, walls) - not robot-type constraints
- Robot shouldn't contain pathfinding logic - it delegates to PathFinder
- PathFinder already filters positions in generateSuccessors() - adding robotType check is natural

### Decision 3: Relay Position Handling

**Problem**: Relay positions are physically located adjacent to aisles. If we mark all aisle-adjacent positions as "aisle", HTMs can't reach relays.

**Solution**: Relay positions are **explicitly whitelisted** in traversability:

```java
case VTM -> !map.isHighway(position) || map.isRelayPosition(position);
case HTM -> !map.isAisle(position) || map.isRelayPosition(position);
```

This means:
- VTM can go to relay even if relay is on highway (unlikely but handled)
- HTM can go to relay even if relay is adjacent to aisle

### Decision 4: isCarryingCargo Semantics

**Current**: `isCarryingRack` boolean in PathFinder (for RTP collision avoidance)

**Updated**: Rename to `isCarryingCargo` - applies to both:
- RTP carrying rack (avoid other rack positions)
- VTM/HTM carrying tote (same obstacle avoidance)

**Rationale**: The collision avoidance logic is the same regardless of cargo type.

---

## Performance Considerations

### Pathfinding Cost

| Metric | Value |
|--------|-------|
| State space | O(width × height × 4 orientations) |
| Worst case | O(b^d) where b=5 actions, d=path depth |
| Safety limit | MAX_ITERATIONS = 20,000 |
| Typical warehouse | 50×50 grid = 10,000 states max |

### Caching Strategy

| Data | Storage | Lookup |
|------|---------|--------|
| Highway positions | HashSet<GridCoordinate> | O(1) |
| Aisle positions | HashSet<GridCoordinate> | O(1) |
| Relay positions | HashSet<GridCoordinate> | O(1) |
| Rack positions | HashSet<GridCoordinate> | O(1) (already exists) |

### Expected Impact

- **Initial path calculation**: ~1-5ms per path (acceptable for simulation)
- **Memory**: ~3 additional HashSets, negligible for typical warehouse sizes
- **Simulation speed**: Slight slowdown due to waypoint-by-waypoint execution (vs teleport), but more realistic

---

## Risk Analysis

### Risk 1: Pathfinding Failures

**Scenario**: VTM can't find path from aisle to relay if relay is blocked by obstacles.

**Mitigation**:
- Relay positions are explicitly traversable by VTMs
- `navigateTo()` throws RuntimeException on path failure - visible in logs
- Test coverage for edge cases

### Risk 2: Performance Degradation

**Scenario**: A* more expensive than Manhattan distance, slowing simulation.

**Mitigation**:
- MAX_ITERATIONS limit prevents runaway searches
- Cached position sets ensure O(1) area checks
- Monitor simulation time in integration tests

### Risk 3: Inconsistent navFactor Application

**Scenario**: navFactor applied differently in different places.

**Mitigation**:
- navFactor applied once in `holdForTravel()` - single point of application
- PathResult returns raw times, navFactor applied at execution
- Unit tests verify navFactor behavior

### Risk 4: Metric Accuracy

**Scenario**: Travel time metrics change due to different path calculation.

**Mitigation**:
- `navigateTo()` returns raw travel time (sum of waypoint timeDelta)
- Metrics apply navFactor consistently
- Before/after comparison in integration tests

---

## Testing Strategy

### Unit Tests

1. **WarehouseMap area classification**
   - `isHighway()` returns true/false correctly
   - `isAisle()` returns true/false correctly
   - `isRelayPosition()` returns true/false correctly

2. **PathFinder traversability**
   - VTM cannot traverse highways
   - HTM cannot traverse aisles
   - Both can traverse relay positions
   - RTP unrestricted

3. **Robot.planPath() integration**
   - Passes robotType correctly
   - Returns valid path for each robot type

### Integration Tests

1. **VTM forward journey**: Rack → Relay (aisle path only)
2. **VTM return journey**: Relay → Rack (aisle path only)
3. **HTM forward journey**: Relay → Station (highway path only)
4. **HTM return journey**: Station → Relay (highway path only)
5. **Full relay cycle**: VTM + HTM coordination

### Performance Tests

1. **Path calculation time**: < 10ms for typical warehouse
2. **Simulation throughput**: No more than 20% slowdown vs teleport

---

## Rollout Plan

### Phase 1: Foundation (Tasks 1-3)
- Add area classification to WarehouseMap
- Add RobotType to PathFinder
- Update Robot.planPath()
- **Checkpoint**: Existing tests pass, RTP behavior unchanged

### Phase 2: VTM Migration (Tasks 4-5)
- Replace VTM moveToPosition() calls
- Test VTM pathfinding in isolation
- **Checkpoint**: VTM uses A* navigation, relay system functional

### Phase 3: HTM Migration (Tasks 6-7)
- Replace HTM moveToPosition() calls
- Set initial orientation for robots
- **Checkpoint**: Full relay system uses A* navigation

### Phase 4: Verification (Task 8)
- Full integration testing
- Performance benchmarking
- **Checkpoint**: Ready for production use

---

## Future Enhancements

1. **Dynamic obstacle avoidance**: Consider other robots' positions in pathfinding
2. **Path caching**: Cache common paths (relay→station) for repeated use
3. **Multi-goal pathfinding**: Optimize station visit order using TSP
4. **Congestion-aware routing**: Prefer less congested paths

---

## Appendix: File Reference

| File | Purpose |
|------|---------|
| `domain/map/WarehouseMap.java` | Area classification methods |
| `domain/navigation/PathFinder.java` | A* algorithm with robotType |
| `domain/navigation/PathResult.java` | Path output with waypoints |
| `domain/navigation/PathWaypoint.java` | Single path step |
| `domain/robot/Robot.java` | planPath() with robotType |
| `simulator/orchestrator/DesmojVtmProcess.java` | VTM navigation |
| `simulator/orchestrator/DesmojHtmProcess.java` | HTM navigation |
| `simulator/orchestrator/DesmojRtpRobotProcess.java` | RTP navigation (reference) |
