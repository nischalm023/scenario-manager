# DesmojRtpRobotProcess Refactoring Design

**Date**: 2025-11-19
**Status**: Implemented

## 1. Problem Statement

The `DesmojRtpRobotProcess.java` class has code quality issues similar to its sibling VTM/HTM processes, though it's in relatively better shape due to existing method extraction:

1. **Oversized `lifeCycle()` method** - 242 lines with substantial inline logic for rack finding/selection
2. **Excessive logging** - 28+ verbose per-step logs clutter output and hurt readability
3. **Undocumented safety assumptions** - Rack reservation and resource operations rely on Desmo-J single-threaded behavior without documentation
4. **Missing section markers** - 677-line class lacks navigation aids
5. **No context record pattern** - Parameters passed individually instead of as cohesive context

## 2. Design Goals

### In Scope

- Extract rack finding and selection logic into focused method
- Extract station visiting loop into separate method
- Simplify logging to cycle summaries
- Document Desmo-J safety guarantees
- Add section markers for navigation
- Add context record pattern for cleaner parameter passing

### Non-Goals

- Changing RTP robot behavior or logic
- Modifying existing well-structured transport methods (`pickUpRack`, `transportToStation`, `returnRackToStorage`)
- Changing station optimization algorithm

## 3. Solution Design

### 3.1 RtpCycleContext Record

Encapsulate per-cycle state to pass between methods:

```java
private record RtpCycleContext(
    Rack selectedRack,
    List<StationId> stationsToVisit,
    double cycleStartTime
) {}
```

### 3.2 Method Extraction

**Main orchestration:**
```java
lifeCycle()                              // Main loop - find rack, execute cycle
  ├─ findAndSelectRack()                 // Find available rack, optimize route
  │                                      // Returns Optional<RtpCycleContext>
  └─ executeCycle(ctx)                   // Orchestrates the phases below
       ├─ pickUpRack(rack)               // EXISTING - Phases 1-2
       ├─ visitStations(ctx)             // NEW - Iterate stations
       │    └─ transportToStation()      // EXISTING - Per-station transport
       │    └─ processAtStation()        // Uses DesmojStationHelper
       ├─ returnRackToStorage(rack)      // EXISTING - Final phase
       └─ emitRtpCycle(...)              // EXISTING - Metrics emission
```

**Existing methods (no changes needed):**
```java
pickUpRack()                             // Already well-structured (55 lines)
transportToStation()                     // Already well-structured (31 lines)
returnRackToStorage()                    // Already structured (111 lines, reduce logging)
selectOptimalRack()                      // Already clean (19 lines)
optimizeStationRoute()                   // Already clean (39 lines)
now()                                    // Existing helper (3 lines)
emitRtpCycle()                           // Already structured (65 lines)
```

**New utility helper:**
```java
holdForTravel(distance)                  // Wrap hold() with navFactor
```

### 3.3 Estimated Line Counts

| Method | Current Lines | After Refactoring | Purpose |
|--------|---------------|-------------------|---------|
| `lifeCycle()` | 242 | ~50 | Main loop with error handling |
| `findAndSelectRack()` | (inline) | ~50 | Find rack, optimize route, reserve |
| `executeCycle()` | (inline) | ~25 | Orchestrator for phases |
| `visitStations()` | (inline) | ~40 | Station visiting loop |
| `pickUpRack()` | 55 | ~35 | Reduced logging |
| `transportToStation()` | 31 | ~20 | Reduced logging |
| `returnRackToStorage()` | 111 | ~50 | Significant logging reduction |
| `selectOptimalRack()` | 19 | 19 | No change |
| `optimizeStationRoute()` | 39 | 39 | No change |
| `emitRtpCycle()` | 65 | 65 | No change |
| `now()` | 3 | 3 | No change |
| `holdForTravel()` | N/A | ~5 | New helper |

**Total**: ~677 lines -> ~500 lines (26% reduction)

### 3.4 Section Markers

```java
@Slf4j
public class DesmojRtpRobotProcess extends SimProcess {

  // ========================================
  // Constants
  // ========================================

  // ========================================
  // Fields
  // ========================================

  // ========================================
  // Context Records
  // ========================================

  // ========================================
  // Constructor
  // ========================================

  // ========================================
  // Lifecycle
  // ========================================

  // ========================================
  // Cycle Orchestration
  // ========================================

  // ========================================
  // Transport Phases
  // ========================================

  // ========================================
  // Routing and Selection
  // ========================================

  // ========================================
  // Metrics
  // ========================================

  // ========================================
  // Helper Methods
  // ========================================
}
```

### 3.5 Logging Strategy

**Rules:**
- INFO: Lifecycle start/end only
- DEBUG: Cycle summary (start + end), idle state
- WARN: Unexpected conditions (filtered racks, no paths, station not found)
- ERROR: Critical failures (pathfinding failure)

**Before (verbose - 28+ logs):**
```java
log.debug("Robot {} checking for work at t={}", ...);
log.debug("All racks state check...");  // 7 debug logs for rack states
log.debug("Robot {} picking up rack {}", ...);
log.debug("Robot {} moved to rack position (travel time: {}s)", ...);
log.debug("Robot {} picked up rack (loading time: {}s)", ...);
log.debug("Robot {} transporting rack {} to station {}", ...);
log.debug("Robot {} arrived at station {} (travel time: {}s)", ...);
log.debug("Robot {} acquiring operator at station {}", ...);
log.debug("Robot {} acquired operator at station {}", ...);
log.debug("Robot {} processed {} tasks at station {}", ...);
log.debug("Robot {} releasing operator at station {}", ...);
log.debug("Robot {} released operator at station {}", ...);
log.debug("Robot {} returning rack {} to storage position", ...);
// ... many more
```

**After (clean - ~10 logs):**
```java
log.info("Robot {} starting lifecycle at t={}", robot.getId(), presentTime());

log.debug("Robot {}: Starting cycle - rack={}, stations={}",
    robot.getId(), selectedRack.getId(), stationsToVisit.size());

// Phase methods have minimal internal logging

log.debug("Robot {}: Cycle complete - rack={}, picks={}, duration={:.1f}s",
    robot.getId(), selectedRack.getId(), totalPicks, cycleTime);

log.info("Robot {} ending lifecycle at t={}", robot.getId(), presentTime());
```

### 3.6 Race Condition Documentation

**Location 1: Rack claim (in findAndSelectRack)**

```java
// SAFETY NOTE: Thread-safe due to Desmo-J single-threaded execution model.
// Between selectOptimalRack() and here, no hold() was called, so no other
// robot process can execute. This prevents duplicate rack pickups.
// If future changes add a hold() before this point, this must be refactored
// to use an atomic claim operation.
selectedRack.setBeingTransported(robot.getId());
robot.setCarryingRack(selectedRack);
```

**Location 2: Operator resource (in visitStations)**

```java
// SAFETY NOTE: Desmo-J Res.provide() is a blocking call.
// If operator is busy, this robot process suspends until released.
// This is the correct Desmo-J pattern for resource contention.
operatorResource.provide(1);
```

**Location 3: Storage position reservation (in executeCycle)**

```java
// SAFETY NOTE: Position reservation is safe because Desmo-J is single-threaded.
// Between completing station work and here, no other robot can claim this position.
// The reservation will be released in returnRackToStorage() after the rack arrives.
storagePositionManager.reservePositionForRack(selectedRack);
```

## 4. Detailed Method Specifications

### 4.1 findAndSelectRack()

```java
/**
 * Find an available rack with tasks and prepare for cycle execution.
 *
 * <p>This method handles:
 * <ul>
 *   <li>Finding racks with tasks in IN_STORAGE state
 *   <li>Selecting optimal rack based on distance/tasks
 *   <li>Getting station assignments and optimizing route
 *   <li>Reserving rack (prevents duplicate pickup)
 * </ul>
 *
 * @return Optional containing cycle context if preparation successful
 */
private Optional<RtpCycleContext> findAndSelectRack() {
    // 1. Get all racks and filter available ones
    List<Rack> allRacks = storageSpaceManager.getAllRacks();
    List<Rack> availableRacks = allRacks.stream()
        .filter(rack -> rack.hasTasks() && rack.getState() == Rack.RackState.IN_STORAGE)
        .collect(Collectors.toList());

    if (availableRacks.isEmpty()) {
        return Optional.empty();
    }

    // 2. Select optimal rack
    Rack selectedRack = selectOptimalRack(robot, availableRacks);
    if (selectedRack == null) {
        return Optional.empty();
    }

    // 3. Get and validate station tasks
    Map<StationId, List<PickTask>> stationTasks = selectedRack.getTasksByStation();
    if (stationTasks.isEmpty()) {
        log.warn("Robot {}: Rack {} has tasks but no station assignments",
            robot.getId(), selectedRack.getId());
        return Optional.empty();
    }

    // 4. Optimize station route
    List<StationId> stationsToVisit = optimizeStationRoute(
        robot.getPosition(), new ArrayList<>(stationTasks.keySet()));

    // 5. Reserve rack
    // SAFETY NOTE: [see section 3.6]
    selectedRack.setBeingTransported(robot.getId());
    robot.setCarryingRack(selectedRack);

    log.debug("Robot {}: Starting cycle - rack={}, stations={}",
        robot.getId(), selectedRack.getId(), stationsToVisit.size());

    return Optional.of(new RtpCycleContext(selectedRack, stationsToVisit, now()));
}
```

### 4.2 executeCycle()

```java
/**
 * Execute a complete RTP cycle: pickup, visit stations, return.
 *
 * @param ctx Cycle context with rack and station information
 */
private void executeCycle(RtpCycleContext ctx) throws SuspendExecution {
    Rack rack = ctx.selectedRack();

    // Phase 1-2: Pick up rack
    TransportMetrics pickupMetrics = pickUpRack(rack);

    // Phase 3-5: Visit all stations
    List<TransportMetrics> allTransportMetrics = new ArrayList<>();
    List<ProcessingMetrics> allProcessingMetrics = new ArrayList<>();
    allTransportMetrics.add(pickupMetrics);

    visitStations(ctx, allTransportMetrics, allProcessingMetrics);

    // Phase 6: Reserve return position and return rack
    // SAFETY NOTE: [see section 3.6]
    storagePositionManager.reservePositionForRack(rack);
    TransportMetrics returnMetrics = returnRackToStorage(rack);

    // Emit cycle metrics
    double cycleEndTime = now();
    emitRtpCycle(
        robot.getId(),
        rack.getId().value(),
        ctx.cycleStartTime(),
        cycleEndTime,
        ctx.stationsToVisit(),
        allTransportMetrics,
        allProcessingMetrics,
        returnMetrics);

    int totalPicks = allProcessingMetrics.stream()
        .mapToInt(ProcessingMetrics::totalPicks)
        .sum();

    log.debug("Robot {}: Cycle complete - rack={}, picks={}, duration={}s",
        robot.getId(), rack.getId(), totalPicks,
        String.format("%.1f", cycleEndTime - ctx.cycleStartTime()));
}
```

### 4.3 visitStations()

```java
/**
 * Visit all stations in optimized order, processing tasks at each.
 *
 * @param ctx Cycle context
 * @param allTransportMetrics List to accumulate transport metrics
 * @param allProcessingMetrics List to accumulate processing metrics
 */
private void visitStations(
    RtpCycleContext ctx,
    List<TransportMetrics> allTransportMetrics,
    List<ProcessingMetrics> allProcessingMetrics) throws SuspendExecution {

    Rack rack = ctx.selectedRack();

    for (StationId stationId : ctx.stationsToVisit()) {
        Station targetStation = stationManager.getStation(stationId);
        if (targetStation == null) {
            log.error("Robot {}: Station {} not found, skipping", robot.getId(), stationId);
            continue;
        }

        // Transport to station
        TransportMetrics transportMetrics = transportToStation(
            rack, targetStation.getPosition().getAccessPosition());
        allTransportMetrics.add(transportMetrics);

        // Acquire operator resource
        // SAFETY NOTE: [see section 3.6]
        Res operatorResource = operatorResources.get(stationId);
        operatorResource.provide(1);

        // Process tasks at station
        ProcessingMetrics processingMetrics = DesmojStationHelper.processStorageSpaceTasks(
            this, (DesmojWarehouseModel) getModel(), robot, rack,
            targetStation, orderCompletionCallback);
        allProcessingMetrics.add(processingMetrics);

        // Release operator resource
        operatorResource.takeBack(1);
    }
}
```

### 4.4 holdForTravel()

```java
/**
 * Hold for travel time with navFactor applied.
 *
 * @param travelTime Raw travel time in seconds
 */
private void holdForTravel(double travelTime) throws SuspendExecution {
    hold(new TimeSpan(travelTime * navFactor, TimeUnit.SECONDS));
}
```

## 5. Testing Strategy

1. **Compile and run existing tests** - Ensure no behavioral changes
2. **Run DesmojIntegrationTest** - Verify RTP system still works
3. **Manual simulation test** - Run full RTP simulation and verify metrics
4. **Log review** - Confirm logging is cleaner (28+ logs vs ~10)
5. **Diff review** - Ensure only structural changes, no behavioral changes
6. **Compare metrics** - Run before/after and compare simulation outputs

## 6. Expected Outcome

- `lifeCycle()` reduced from ~242 lines to ~50 lines
- Clear "table of contents" showing RTP cycle flow
- Each extracted method focused on single responsibility
- Easy navigation via section markers
- Safety assumptions documented for future maintainers
- ~26% reduction in total file size (677 -> ~500 lines)
- Improved readability without changing behavior
- Consistent with VTM/HTM refactoring patterns

## 7. Files Changed

| File | Changes |
|------|---------|
| `DesmojRtpRobotProcess.java` | Refactor lifecycle, add section markers, simplify logging, add safety docs, extract methods |

## 8. Implementation Priority

### High Priority (Must Fix)
1. Add section markers for navigation
2. Create `RtpCycleContext` record
3. Extract `findAndSelectRack()` method
4. Extract `visitStations()` method
5. Add `executeCycle()` orchestrator
6. Add race condition safety documentation (3 locations)

### Medium Priority (Should Fix)
7. Consolidate logging to start + summary pattern
8. Add `holdForTravel()` helper
9. Reduce verbose logging in `pickUpRack()` and `returnRackToStorage()`

### Low Priority (Nice to Have)
10. Consider further decomposition of `returnRackToStorage()` into smaller phases
