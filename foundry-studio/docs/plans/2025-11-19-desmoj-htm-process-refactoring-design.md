# DesmojHtmProcess Refactoring Design

**Date**: 2025-11-19
**Status**: Planned

## 1. Problem Statement

The `DesmojHtmProcess.java` class has several code quality issues:

1. **Oversized `lifeCycle()` method** - 260+ lines making it difficult to read, test, and maintain
2. **Excessive logging** - Verbose per-step logs clutter output and hurt readability
3. **Undocumented safety assumptions** - Peek/remove pattern relies on Desmo-J single-threaded behavior without documentation
4. **Missing section markers** - 358-line class lacks navigation aids

## 2. Design Goals

### In Scope

- Refactor `lifeCycle()` into smaller, focused methods
- Simplify logging to cycle summaries
- Document Desmo-J safety guarantees
- Add section markers for navigation

### Non-Goals

- Changing HTM behavior or logic
- Implementing multi-station support
- Removing unused `zoneLoadTracker` field

## 3. Solution Design

### 3.1 CycleContext Record

Encapsulate per-cycle state to pass between extracted methods:

```java
private record CycleContext(
    RelayPosition sourceRelay,
    Tote tote,
    StationId targetStationId,
    List<PickTask> tasks,
    double cycleStartTime
) {}
```

### 3.2 Method Extraction (Hybrid Approach)

**Main orchestration:**
```java
lifeCycle()                    // Main loop - find relay, prepare, execute
  └─ executeCycle(ctx)         // Orchestrates the 5 phases below
       ├─ travelToRelayAndLoad(ctx)
       ├─ travelToStation(ctx)
       ├─ processAtStation(ctx)
       ├─ returnToRelay(ctx)
       └─ emitCycleMetrics(ctx, processingResult)
```

**Preparation helpers:**
```java
findRelayWithTotes()           // Returns Optional<RelayPosition>
prepareForCycle(relay)         // Peek, validate, remove, build context
                               // Returns Optional<CycleContext>
```

**Utility:**
```java
now()                          // Existing helper
holdForTravel(distance)        // New: wraps hold() with navFactor
```

### 3.3 Estimated Line Counts

| Method | Lines | Purpose |
|--------|-------|---------|
| `lifeCycle()` | ~40 | Main loop with error handling |
| `prepareForCycle()` | ~35 | Validation and context creation |
| `executeCycle()` | ~10 | Orchestrator |
| `travelToRelayAndLoad()` | ~25 | Phases 1-2 |
| `travelToStation()` | ~20 | Phase 3 |
| `processAtStation()` | ~25 | Phases 4-5 |
| `returnToRelay()` | ~30 | Phase 6 |
| `emitCycleMetrics()` | ~25 | Metrics emission |

### 3.4 Section Markers

```java
@Slf4j
public class DesmojHtmProcess extends SimProcess {

  // ========================================
  // Constants
  // ========================================

  // ========================================
  // Fields
  // ========================================

  // ========================================
  // Context Record
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
  // Helper Methods
  // ========================================
}
```

### 3.5 Logging Strategy

**Rules:**
- INFO: Lifecycle start/end only
- DEBUG: Cycle summary (start + end)
- WARN: Unexpected conditions

**Before (verbose):**
```java
log.debug("HTM {}: Found relay {} with totes at distance {}", ...);
log.debug("HTM {}: Peeking at tote in relay {}", ...);
log.debug("HTM {}: Tote {} has {} tasks", ...);
log.debug("HTM {}: Removed tote {} from relay {}", ...);
// ... 15 more similar logs ...
```

**After (clean):**
```java
log.info("HTM {} starting lifecycle at t={}", htm.getId(), presentTime());

log.debug("HTM {}: Starting cycle - relay={}, tote={}, station={}",
    htm.getId(), relay.getId(), tote.getId(), targetStationId);

// Phase methods have minimal internal logging

log.debug("HTM {}: Cycle complete - {} picks in {:.1f}s",
    htm.getId(), totalPicks, cycleTime);

log.info("HTM {} ending lifecycle at t={}", htm.getId(), presentTime());
```

### 3.6 Race Condition Documentation

Add clear comment in `prepareForCycle()`:

```java
private Optional<CycleContext> prepareForCycle(RelayPosition relay) throws SuspendExecution {
    Optional<Tote> peekOpt = relay.peekFirstTote();
    if (peekOpt.isEmpty()) {
        return Optional.empty();
    }

    Tote tote = peekOpt.get();

    if (!tote.hasTasks()) {
        log.warn("HTM {}: Tote {} in relay {} has no tasks, skipping",
            htm.getId(), tote.getId(), relay.getId());
        return Optional.empty();
    }

    // SAFETY NOTE: No race condition here because Desmo-J is single-threaded.
    // Between peek and remove, there's no hold() call, so no other process
    // can execute. If future changes add a hold() between these operations,
    // this must be refactored to use an atomic removeFirstToteIf() operation.
    Optional<Tote> toteOpt = relay.removeTote();
    if (toteOpt.isEmpty()) {
        log.error("HTM {}: Relay {} peek succeeded but remove failed - " +
            "this should never happen in single-threaded Desmo-J",
            htm.getId(), relay.getId());
        return Optional.empty();
    }

    // ... build and return CycleContext ...
}
```

## 4. Testing Strategy

1. **Compile and run existing tests** - Ensure no behavioral changes
2. **Run DesmojIntegrationTest** - Verify relay system still works
3. **Manual log review** - Confirm logging is cleaner

## 5. Expected Outcome

- `lifeCycle()` reduced from ~260 lines to ~40 lines
- Clear "table of contents" showing HTM cycle flow
- Each extracted method focused on single responsibility
- Easy navigation via section markers
- Improved readability without changing behavior

## 6. Files Changed

| File | Changes |
|------|---------|
| `DesmojHtmProcess.java` | Refactor lifecycle, add section markers, simplify logging |
