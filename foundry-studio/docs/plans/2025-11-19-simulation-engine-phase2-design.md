# Simulation Engine Refactoring - Phase 2 Design

**Date**: 2025-11-19
**Status**: Draft
**Scope**: DesmojWarehouseModel and DesmojSimulationEngine

## Summary

Improve code navigation and maintainability in `DesmojWarehouseModel` and `DesmojSimulationEngine` by adding section markers, method documentation, and modernizing control flow with switch expressions.

## Scope

### In Scope

- Section markers for classes >200 lines
- Method documentation for public methods and overrides
- Switch expressions for SystemType handling

### Out of Scope

- TODO comment improvements (deferred)
- Functional changes

## Success Criteria

- Both files have section markers following style guide format
- All public methods have at least single-line Javadoc
- Override methods (`init()`, `doInitialSchedules()`) have Javadoc explaining what they do
- All `if-else` chains on `SystemType` converted to switch expressions
- Code compiles and all tests pass
- No functional behavior changes

## Section Markers

Per style guide §5, use section markers for classes >200 lines.

### DesmojWarehouseModel.java (~450 lines)

```java
  // ========================================
  // Nested Types
  // ========================================

  public record OrderProcessingRuntime(...) {}

  // ========================================
  // Fields
  // ========================================

  @Getter private final SimulationConfig config;
  // ...

  // ========================================
  // Internal Accessors
  // ========================================

  private StoragePositionManager storagePositionManager() { ... }

  // ========================================
  // Constructor
  // ========================================

  // ========================================
  // Public API
  // ========================================

  public void markStorageSpaceActive(...) { ... }

  // ========================================
  // Desmo-J Lifecycle
  // ========================================

  @Override public String description() { ... }
  @Override public void init() { ... }
  @Override public void doInitialSchedules() { ... }

  // ========================================
  // Process Scheduling
  // ========================================

  private void scheduleRtpRobots() { ... }
  private void scheduleRelayRobots() { ... }
```

### DesmojSimulationEngine.java (~540 lines)

```java
  // ========================================
  // Fields
  // ========================================

  // ========================================
  // Constructor
  // ========================================

  // ========================================
  // Public API
  // ========================================

  public void run() { ... }
  public List<WarehouseSnapshotV2> getSnapshots() { ... }

  // ========================================
  // Simulation Lifecycle
  // ========================================

  private void initializeRelaySystem() { ... }
  private void loadAndProcessOrders() { ... }
  private void setupExperiment() { ... }
  private void runSimulation() { ... }
  private void cleanupExperiment() { ... }

  // ========================================
  // Order Processing
  // ========================================

  private void assignTasksAndUpdateTracking(...) { ... }
  private void checkOrderCompletion(...) { ... }

  // ========================================
  // Utilities
  // ========================================

  public double now() { ... }
  public void optimizeInitialStorageSpacePositions() { ... }
```

## Method Documentation

Per style guide §2, public methods need documentation. Simple = single-line, complex = multi-line.

### DesmojWarehouseModel.java

```java
/** Return model description for Desmo-J reporting. */
@Override
public String description() {

/** Initialize operator resources and pathfinder. */
@Override
public void init() {

/** Schedule all simulation processes (robots, bin management, monitoring). */
@Override
public void doInitialSchedules() {
```

### DesmojSimulationEngine.java

```java
/** Run the simulation from start to completion. */
public void run() {

/**
 * Get captured snapshots from the simulation.
 *
 * @return List of warehouse snapshots, or empty list if disabled
 */
public List<WarehouseSnapshotV2> getSnapshots() {

/** Get current simulation time in seconds. */
public double now() {

/**
 * Optimize initial storage space positions using configured put algorithm.
 *
 * <p>Assigns racks to optimal grid positions based on popularity and distance.
 */
public void optimizeInitialStorageSpacePositions() {
```

## Switch Expressions

Per style guide §8.6, use switch expressions for cleaner, exhaustive pattern matching.

### DesmojWarehouseModel.java - doInitialSchedules

```java
// Before
if (config.systemType() == SystemType.RTP) {
    scheduleRtpRobots();
} else if (config.systemType() == SystemType.RELAY) {
    scheduleRelayRobots();
} else {
    log.warn("Unknown system type: {}", config.systemType());
}

// After
switch (config.systemType()) {
    case RTP -> scheduleRtpRobots();
    case RELAY -> scheduleRelayRobots();
}
```

### DesmojSimulationEngine.java - assignTasksAndUpdateTracking

```java
// Before
if (config.systemType() == SystemType.RTP) {
    rackIdValue = ((RackId) task.getStorageSpaceId()).value();
} else {
    var toteId = (ToteId) task.getStorageSpaceId();
    // ... get rack from tote
}

// After
int rackIdValue = switch (config.systemType()) {
    case RTP -> ((RackId) task.getStorageSpaceId()).value();
    case RELAY -> {
        var toteId = (ToteId) task.getStorageSpaceId();
        Tote tote = storageSpaceManager().getTote(toteId);
        if (tote != null && tote.getCurrentHoldingEntity() instanceof RackId rackId) {
            yield rackId.value();
        }
        log.warn("Tote {} not found or not in rack for task {}", toteId, task.getTaskId());
        yield 0;
    }
};
```

Similarly for task assignment to rack/tote block.

## Files Changed

| File | Changes |
|------|---------|
| `DesmojWarehouseModel.java` | Add 6 section markers; add Javadoc for 3 override methods; convert 1 if-else to switch |
| `DesmojSimulationEngine.java` | Add 6 section markers; add Javadoc for 4 public methods; convert 2-3 if-else chains to switch |

## Testing Approach

**Automated:**
```bash
cd apps/backend && mvn test
```

All existing tests must pass.

**Manual:**
- Frontend integration testing by user

## Risks

- **Very low risk**: Documentation and code style only, no functional changes
- **Switch exhaustiveness**: Compiler enforces all enum cases covered
