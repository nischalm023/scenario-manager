# Code Organization Exemplar

Gold standard: `DesmojVtmProcess.java` (914 lines, 9 functional sections)

## Annotated Section Layout

```java
@Slf4j
public class DesmojVtmProcess extends SimProcess {

  // ========================================
  // Constants                        (L55)
  // ========================================
  // IDLE_CHECK_INTERVAL, LOADING_TIME, UNLOADING_TIME, thresholds

  // ========================================
  // Fields                           (L64)
  // ========================================
  // Dependencies: Robot, ZoneManager, RelayPositionManager, etc.
  // All final. Nullable fields annotated with // nullable comment.

  // ========================================
  // Context Records                  (L78)
  // ========================================
  // VtmCycleContext: bundles tote + rack + relay + zone for a cycle
  // DropResult: return value from relay drop operation
  // These replace long parameter lists and make method signatures clean.

  // ========================================
  // Constructor                      (L95)
  // ========================================
  // All fields assigned. No logic beyond assignment.

  // ========================================
  // Lifecycle                        (L125)
  // ========================================
  // lifeCycle() override - the main loop.
  // Priority-based: congestion > zone backpressure > forward work > opportunistic return > idle
  // Uses inline comments for each priority level.

  // ========================================
  // Congestion Management            (L182)
  // ========================================
  // getReturnTaskIfPhysicalRelayCongestionPresent()
  // shouldIdleForZoneCongestion()
  // Both public orchestration AND private helpers together.

  // ========================================
  // Cycle Execution                  (L209)
  // ========================================
  // findAndPrepareForCycle(), executeCycle(), travelToRack(),
  // travelToRelay(), dropAtRelay()
  // Phase comments: "Phase 1: Move to relay", "Phase 5: Unload"
  // SAFETY NOTE for thread-safety assumptions

  // ========================================
  // Tote Finding                     (L462)
  // ========================================
  // findNextToteForRelay(), findReturnTaskInZone(),
  // findOpportunisticReturnTask(), createReturnTaskIfValid()

  // ========================================
  // Return Journey                   (L654)
  // ========================================
  // executeReturnJourney() - complete return cycle
  // SAFETY NOTE for race condition analysis

  // ========================================
  // Helper Methods                   (L748)
  // ========================================
  // now(), navigateTo(), handleIdleState(), selectRelayForTote()
  // Pure utilities with no domain knowledge
}
```

## Why Functional Grouping > Access Modifier Grouping

**Bad** (access modifier grouping):
```
Public API: findAndPrepareForCycle(), executeCycle(), findNextTote()
Private: travelToRack(), travelToRelay(), dropAtRelay(), selectRelay()
```

Reading `executeCycle()` requires jumping 300 lines to find `travelToRack()`.

**Good** (functional grouping - what VtmProcess does):
```
Cycle Execution: findAndPrepareForCycle(), executeCycle(),
                 travelToRack(), travelToRelay(), dropAtRelay()
```

Reading `executeCycle()` and its helpers is a contiguous block.

## Context Records Pattern

Bundle method parameters into named records when 3+ params travel together:

```java
// Before: 7 params threaded through 5 methods
private void executeCycle(Tote tote, Rack rack, int zIndex,
    RelayPosition relay, Zone zone, String zoneId, double startTime)

// After: 1 record, clean signatures
private record VtmCycleContext(
    Tote tote, Rack sourceRack, int sourceZIndex,
    RelayPosition relay, Zone targetZone,
    String targetZoneId, double cycleStartTime) {}

private void executeCycle(VtmCycleContext ctx) { ... }
```

## Comment Patterns in Practice

### Phase Comments (from executeCycle/dropAtRelay)

```java
// Phase 1: Move to relay
vtm.setState(RobotState.MOVING_TO_RACK);
double travelTime = navigateTo(relay.getPosition(), false, "to relay");

// Phase 5: Unload tote from relay
hold(new TimeSpan(UNLOADING_TIME, TimeUnit.SECONDS));
```

### SAFETY NOTEs (threading assumptions)

```java
// SAFETY NOTE: Thread-safe due to Desmo-J single-threaded execution model.
// No other VTM process can execute between tote selection and this assignment
// because we haven't called hold() yet.
```

### CRITICAL (blocking calls)

```java
// This BLOCKS if relay is full (Desmo-J Res automatic blocking)
storageSpaceManager.moveToteFromRobotToRelay(...);
```

## Process Section Comparison

| Section | VtmProcess (9) | HtmProcess (7) | RtpProcess (8) |
|---------|---------------|-----------------|-----------------|
| Constants | Y | Y | Y |
| Fields | Y | Y | Y |
| Context Records | Y | Y | Y |
| Constructor | Y | Y | Y |
| Lifecycle | Y | Y | Y |
| Congestion Mgmt | Y | - | - |
| Cycle Execution | Y | Y | Cycle Orchestration |
| Tote Finding | Y | - | - |
| Return Journey | Y | - | - |
| Transport Phases | - | - | Y |
| Routing/Selection | - | - | Y |
| Metrics | - | - | Y |
| Helper Methods | Y | Y | Y |

Simpler processes have fewer domain sections. The pattern scales naturally:
more domain complexity = more sections. Helpers always go last.
