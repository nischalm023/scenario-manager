# DesmojVtmProcess Refactoring Design

**Date**: 2025-11-19
**Status**: Implemented

## 1. Problem Statement

The `DesmojVtmProcess.java` class has several code quality issues similar to its sibling `DesmojHtmProcess`:

1. **Oversized `lifeCycle()` method** - 252 lines making it difficult to read, test, and maintain
2. **Excessive logging** - 28 verbose per-step logs clutter output and hurt readability
3. **Undocumented safety assumptions** - Tote assignment and removal patterns rely on Desmo-J single-threaded behavior without documentation
4. **Missing section markers** - 559-line class lacks navigation aids
5. **Code duplication** - Zone lookup logic repeated in multiple places

## 2. Design Goals

### In Scope

- Refactor `lifeCycle()` into smaller, focused methods
- Simplify logging to cycle summaries
- Document Desmo-J safety guarantees
- Add section markers for navigation
- Extract duplicate zone lookup logic

### Non-Goals

- Changing VTM behavior or logic
- Modifying return journey handling (already well-structured)
- Changing existing helper methods (`findToteWithTasksInZone`, etc.)

## 3. Solution Design

### 3.1 VtmCycleContext Record

Encapsulate per-cycle state to pass between extracted methods:

```java
private record VtmCycleContext(
    Tote tote,
    Rack sourceRack,
    RackId sourceRackId,
    RelayPosition relay,
    Zone targetZone,
    String targetZoneId,
    double cycleStartTime
) {}

private record DropResult(
    boolean wasBlocked,
    double blockTime
) {}
```

### 3.2 Method Extraction

**Main orchestration:**
```java
lifeCycle()                           // Main loop - return priority, find tote, execute
  ├─ findAndPrepareForCycle()         // Find tote, select relay, reserve capacity
  │                                   // Returns Optional<VtmCycleContext>
  └─ executeCycle(ctx)                // Orchestrates the 4 phases below
       ├─ travelToRack(ctx)           // Phase 1
       ├─ loadTote(ctx)               // Phase 2
       ├─ transportToRelay(ctx)       // Phase 3
       ├─ dropAtRelay(ctx)            // Phase 4 (returns DropResult)
       └─ emitCycleMetrics(ctx, ...)  // Metrics emission
```

**Existing methods (no changes):**
```java
findToteWithTasksInZone()             // Already well-structured
isHighRelayUtilization()              // Already well-structured
findReturnTask()                      // Already well-structured
executeReturnJourney()                // Already well-structured
getRacksInTargetZone()                // Already well-structured
extractRelayNumericId()               // Existing helper
now()                                 // Existing helper
```

**New utility helpers:**
```java
findZoneForRack(rack)                 // Extract duplicate zone lookup
holdForTravel(distance)               // Wrap hold() with navFactor
```

### 3.3 Estimated Line Counts

| Method | Lines | Purpose |
|--------|-------|---------|
| `lifeCycle()` | ~45 | Main loop with error handling |
| `findAndPrepareForCycle()` | ~50 | Find tote, select relay, reserve capacity |
| `executeCycle()` | ~15 | Orchestrator for phases |
| `travelToRack()` | ~15 | Phase 1: Move to source rack |
| `loadTote()` | ~15 | Phase 2: Load tote onto VTM |
| `transportToRelay()` | ~15 | Phase 3: Transport to relay position |
| `dropAtRelay()` | ~45 | Phase 4: Drop with blocking, zone updates |
| `emitCycleMetrics()` | ~20 | Emit VTM cycle completion event |
| `findZoneForRack()` | ~10 | New helper for zone lookup |
| `holdForTravel()` | ~5 | New helper for navFactor application |

**Total new/modified**: ~235 lines (vs current ~320 in lifeCycle alone)

### 3.4 Section Markers

```java
@Slf4j
public class DesmojVtmProcess extends SimProcess {

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
  // Cycle Execution
  // ========================================

  // ========================================
  // Return Journey
  // ========================================

  // ========================================
  // Tote Finding
  // ========================================

  // ========================================
  // Helper Methods
  // ========================================
}
```

### 3.5 Logging Strategy

**Rules:**
- INFO: Lifecycle start/end only
- DEBUG: Cycle summary (start + end), idle state, zone load changes
- WARN: Unexpected conditions (blocking, not found)

**Before (verbose - 28 logs):**
```java
log.debug("VTM {} starting cycle with tote {} at t={}", ...);
log.debug("VTM {} reserved capacity in zone {} (available: {}), picking tote {} via relay {}", ...);
log.debug("VTM {} moved to rack {} (travel time: {}s) at t={}s", ...);
log.debug("VTM {} loaded tote {} (loading time: {}s) at t={}s", ...);
log.debug("VTM {} transported tote {} to relay {} (travel time: {}s) at t={}s", ...);
log.debug("VTM {} dropped tote {} at relay {} (blocked: {}s) at t={}s", ...);
log.debug("VTM {} completed cycle with tote {} at t={} (duration: {}s)", ...);
```

**After (clean - 12 logs):**
```java
log.info("VTM {} starting lifecycle at t={}", vtm.getId(), presentTime());

log.debug("VTM {}: Starting cycle - tote={}, rack={}, relay={}, zone={}",
    vtm.getId(), tote.getId(), sourceRack.getId(), relay.getId(), targetZoneId);

// Phase methods have no internal logging

log.debug("VTM {}: Cycle complete - tote={}, travel={:.1f}s, blocked={:.1f}s, total={:.1f}s",
    vtm.getId(), tote.getId(), totalTravelTime, blockTime, cycleTime);

log.info("VTM {} ending lifecycle at t={}", vtm.getId(), presentTime());
```

### 3.6 Race Condition Documentation

**Location 1: Tote assignment (in findAndPrepareForCycle)**

```java
// SAFETY NOTE: Thread-safe due to Desmo-J single-threaded execution model.
// No other VTM process can execute between tote selection and this assignment
// because we haven't called hold() yet. This prevents duplicate tote pickups.
// If future changes add a hold() before this point, this must be refactored
// to use an atomic claim operation.
tote.setBeingTransportedToRelay(relay.getId());
tote.setAssignedVtmId(vtm.getId());
```

**Location 2: Return journey tote removal (in executeReturnJourney)**

```java
// SAFETY NOTE: No race condition between findReturnTask() and here because
// Desmo-J is single-threaded. The hold() in travel simulation doesn't allow
// other VTMs to claim this specific tote since the relay uses FIFO removal.
// If this warning ever triggers, it indicates a programming error.
Optional<Tote> removed = task.relayPosition().removeTote();

if (removed.isEmpty()) {
    log.error("VTM {}: Tote {} was removed from relay {} between find and execute - " +
        "this should not happen in single-threaded Desmo-J model",
        vtm.getId(), task.tote().getId(), task.relayPosition().getId());
    return;
}
```

**Location 3: Zone capacity reservation (in findAndPrepareForCycle)**

```java
// SAFETY NOTE: Zone capacity reservation is safe because:
// 1. Desmo-J single-threaded model - no other process executes between
//    tryReserve() and the actual tote delivery
// 2. If reservation succeeds, we MUST proceed with pickup (cannot abort)
// 3. fulfill() is called only after successful relay drop
// If future changes allow reservation cancellation, add corresponding
// release() calls in error paths.
if (!targetZone.getCapacity().tryReserve()) {
```

### 3.7 Code Duplication Fixes

**Extract zone lookup helper:**

```java
/**
 * Find the zone ID that services a given rack.
 *
 * @param rack The rack to find zone for
 * @return Zone ID if found, null otherwise
 */
private String findZoneForRack(Rack rack) {
    for (Map.Entry<String, Zone> entry : zoneManager.getZones().entrySet()) {
        if (entry.getValue().servicesRack(rack.getCurrentCoordinate())) {
            return entry.getKey();
        }
    }
    return null;
}
```

**Extract navFactor helper:**

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

**Use LOADING_TIME constant consistently:**

Current (line 467):
```java
double unloadingTime = 6.0;
```

Fixed:
```java
hold(new TimeSpan((travelToRack * navFactor) + LOADING_TIME, TimeUnit.SECONDS));
```

## 4. Detailed Method Specifications

### 4.1 findAndPrepareForCycle()

```java
/**
 * Find a tote with tasks and prepare for cycle execution.
 *
 * <p>This method handles:
 * <ul>
 *   <li>Finding available totes in VTM's assigned zones</li>
 *   <li>Validating source rack exists</li>
 *   <li>Selecting optimal relay position</li>
 *   <li>Reserving zone capacity</li>
 *   <li>Marking tote as being transported (prevents duplicate pickup)</li>
 * </ul>
 *
 * @return Optional containing cycle context if preparation successful
 */
private Optional<VtmCycleContext> findAndPrepareForCycle() throws SuspendExecution {
    // 1. Find tote with tasks in zone
    Optional<Tote> toteOpt = findToteWithTasksInZone();
    if (toteOpt.isEmpty()) {
        return Optional.empty();
    }

    Tote tote = toteOpt.get();

    // 2. Get source rack
    RackId rackId = (RackId) tote.getCurrentHoldingEntity();
    Rack sourceRack = storageSpaceManager.getRack(rackId);
    if (sourceRack == null) {
        log.warn("VTM {}: Source rack {} not found for tote {}",
            vtm.getId(), rackId, tote.getId());
        return Optional.empty();
    }

    // 3. Select optimal relay
    Optional<RelayPosition> relayOpt = relayPositionManager.selectOptimalRelayForVtm(
        vtm, tote, sourceRack, zoneManager, stationManager);
    if (relayOpt.isEmpty()) {
        log.warn("VTM {}: No relay position available for tote {}",
            vtm.getId(), tote.getId());
        return Optional.empty();
    }

    RelayPosition relay = relayOpt.get();
    String targetZoneId = relay.getOwnerZoneId();
    Zone targetZone = zoneManager.getZones().get(targetZoneId);

    if (targetZone == null) {
        log.error("VTM {}: Could not find zone {} for relay {}",
            vtm.getId(), targetZoneId, relay.getId());
        return Optional.empty();
    }

    // 4. Reserve zone capacity
    // SAFETY NOTE: [see section 3.6]
    if (!targetZone.getCapacity().tryReserve()) {
        vtm.setState(RobotState.IDLE);
        log.debug("VTM {} idle - target zone {} has no capacity (available: {})",
            vtm.getId(), targetZoneId, targetZone.getCapacity().available());

        if (metricsManager != null) {
            metricsManager.emit(DomainEvent.vtmBlockedOnCapacity(
                now(), vtm.getId(), targetZoneId));
        }
        return Optional.empty();
    }

    // 5. Mark tote as being transported
    // SAFETY NOTE: [see section 3.6]
    tote.setBeingTransportedToRelay(relay.getId());
    tote.setAssignedVtmId(vtm.getId());

    log.debug("VTM {}: Starting cycle - tote={}, rack={}, relay={}, zone={}",
        vtm.getId(), tote.getId(), sourceRack.getId(), relay.getId(), targetZoneId);

    return Optional.of(new VtmCycleContext(
        tote, sourceRack, rackId, relay, targetZone, targetZoneId, now()));
}
```

### 4.2 executeCycle()

```java
/**
 * Execute a complete VTM cycle: rack to relay.
 *
 * @param ctx Cycle context with tote, rack, and relay information
 */
private void executeCycle(VtmCycleContext ctx) throws SuspendExecution {
    double travelToRackTime = travelToRack(ctx);
    loadTote(ctx);
    double travelToRelayTime = transportToRelay(ctx);
    DropResult dropResult = dropAtRelay(ctx);
    emitCycleMetrics(ctx, dropResult, travelToRackTime, travelToRelayTime);
}
```

### 4.3 Phase Methods

```java
/**
 * Phase 1: Travel to source rack.
 *
 * @param ctx Cycle context
 * @return Travel time in seconds (before navFactor)
 */
private double travelToRack(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.MOVING_TO_RACK);
    double travelTime = vtm.moveToPosition(ctx.sourceRack().getCurrentCoordinate());
    holdForTravel(travelTime);
    return travelTime;
}

/**
 * Phase 2: Load tote from rack.
 *
 * @param ctx Cycle context
 */
private void loadTote(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.LOADING_RACK);
    hold(new TimeSpan(LOADING_TIME, TimeUnit.SECONDS));
    vtm.addCarryingTote(ctx.tote());
}

/**
 * Phase 3: Transport loaded tote to relay position.
 *
 * @param ctx Cycle context
 * @return Travel time in seconds (before navFactor)
 */
private double transportToRelay(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.TRANSPORTING_RACK_TO_STORAGE);
    double travelTime = vtm.moveToPosition(ctx.relay().getPosition());
    holdForTravel(travelTime);
    return travelTime;
}

/**
 * Phase 4: Drop tote at relay position.
 *
 * <p>May block if relay is full (Desmo-J Res automatic blocking).
 * Handles zone load decrement and capacity fulfillment.
 *
 * @param ctx Cycle context
 * @return Drop result with blocking time information
 */
private DropResult dropAtRelay(VtmCycleContext ctx) throws SuspendExecution {
    vtm.setState(RobotState.IDLE);

    double beforeBlock = now();
    boolean wasBlocked = ctx.relay().isFull();

    // This BLOCKS if relay is full (Desmo-J Res automatic blocking)
    ctx.relay().addTote(this, ctx.tote());

    // Decrement zone load - VTM outbound work complete
    if (zoneLoadTracker != null) {
        String sourceZoneId = findZoneForRack(ctx.sourceRack());
        if (sourceZoneId != null) {
            zoneLoadTracker.decrementLoad(sourceZoneId);
            log.debug("VTM {} decremented source zone {} load for tote {}",
                vtm.getId(), sourceZoneId, ctx.tote().getId());
        }
    }

    // Fulfill the reservation
    ctx.targetZone().getCapacity().fulfill();

    double afterBlock = now();
    double blockTime = afterBlock - beforeBlock;

    if (wasBlocked) {
        log.warn("VTM {} blocked at relay {} for {}s",
            vtm.getId(), ctx.relay().getId(), blockTime);

        if (metricsManager != null) {
            metricsManager.emit(DomainEvent.relayPositionBlocked(
                afterBlock, vtm.getId(),
                extractRelayNumericId(ctx.relay().getId()), blockTime));
        }
    }

    // Tote dropped successfully
    vtm.removeCarryingTote(ctx.tote());
    ctx.tote().setAssignedVtmId(null);

    // Mark tote as active (now at relay, available for piggybacking)
    ((DesmojWarehouseModel) getModel()).markStorageSpaceActive(ctx.tote().getId());

    return new DropResult(wasBlocked, blockTime);
}
```

### 4.4 emitCycleMetrics()

```java
/**
 * Emit VTM cycle completion metrics.
 *
 * @param ctx Cycle context
 * @param dropResult Result from drop phase
 * @param travelToRackTime Travel time to rack (before navFactor)
 * @param travelToRelayTime Travel time to relay (before navFactor)
 */
private void emitCycleMetrics(
    VtmCycleContext ctx,
    DropResult dropResult,
    double travelToRackTime,
    double travelToRelayTime) {

    double cycleEnd = now();
    double totalTravelTime = (travelToRackTime + travelToRelayTime) * navFactor;
    double cycleTime = cycleEnd - ctx.cycleStartTime();

    if (metricsManager != null) {
        metricsManager.emit(DomainEvent.vtmCycleCompleted(
            cycleEnd,
            vtm.getId(),
            String.valueOf(ctx.tote().getId().value()),
            ctx.cycleStartTime(),
            cycleTime,
            ctx.sourceRack().getId().value(),
            extractRelayNumericId(ctx.relay().getId()),
            totalTravelTime));
    }

    log.debug("VTM {}: Cycle complete - tote={}, travel={:.1f}s, blocked={:.1f}s, total={:.1f}s",
        vtm.getId(), ctx.tote().getId(), totalTravelTime, dropResult.blockTime(), cycleTime);

    vtm.setState(RobotState.IDLE);
}
```

## 5. Testing Strategy

1. **Compile and run existing tests** - Ensure no behavioral changes
2. **Run DesmojIntegrationTest** - Verify relay system still works with VTM
3. **Manual simulation test** - Run full simulation and verify VTM metrics
4. **Log review** - Confirm logging is cleaner (12 logs vs 28)
5. **Diff review** - Ensure only structural changes, no behavioral changes

## 6. Expected Outcome

- `lifeCycle()` reduced from ~252 lines to ~45 lines
- Clear "table of contents" showing VTM cycle flow
- Each extracted method focused on single responsibility
- Easy navigation via section markers
- Safety assumptions documented for future maintainers
- ~25% reduction in total file size (559 → ~415 lines)
- Improved readability without changing behavior

## 7. Files Changed

| File | Changes |
|------|---------|
| `DesmojVtmProcess.java` | Refactor lifecycle, add section markers, simplify logging, add safety docs, extract helpers |

## 8. Implementation Priority

### High Priority (Must Fix)
1. Add section markers for navigation
2. Create `VtmCycleContext` and `DropResult` records
3. Extract `findAndPrepareForCycle()` method
4. Extract phase methods (`travelToRack`, `loadTote`, `transportToRelay`, `dropAtRelay`)
5. Extract `emitCycleMetrics()` method
6. Add race condition safety documentation

### Medium Priority (Should Fix)
7. Consolidate logging to start + summary pattern
8. Extract `findZoneForRack()` helper
9. Extract `holdForTravel()` helper
10. Use `LOADING_TIME` constant consistently

### Low Priority (Nice to Have)
11. Consider moving zone lookup methods to `ZoneManager` (future refactoring)
