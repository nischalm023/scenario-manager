# Simulation Engine Refactoring - Phase 1 Design

**Date**: 2025-11-19
**Status**: Draft
**Scope**: DesmojWarehouseModel and DesmojSimulationEngine

## Summary

Refactor `DesmojWarehouseModel` and `DesmojSimulationEngine` to address critical code quality issues identified in code review: constructor complexity, thread safety, exception handling, and edge case handling.

## Scope

### In Scope (Phase 1)

- Reduce constructor from 16 to 8 parameters
- Use thread-safe collections for shared state
- Add proper exception handling with cleanup
- Fail fast on empty orders

### Out of Scope (Phase 2)

- Section markers for navigation
- Missing method documentation
- Switch expressions for SystemType
- TODO comment improvements

## Success Criteria

- Constructor has 8 parameters with clear semantic grouping
- All shared collections use concurrent implementations
- Simulation cleanup happens even on failure
- Empty orders scenario fails with clear error message
- All existing tests pass
- No functional behavior changes

## Architectural Changes

### 1. Constructor Simplification

**Remove redundant parameters** - These 5 are already in `warehouseComponents`:
- `storagePositionManager`
- `storageSpaceManager`
- `zoneManager`
- `relayPositionManager`
- `gridPopularityManager`

**Create `OrderProcessingRuntime` record** (package-private, nested in DesmojWarehouseModel):

```java
record OrderProcessingRuntime(
    OrderManager orderManager,
    BiConsumer<String, String> orderCompletionCallback,
    Map<String, List<String>> orderTaskMapping,
    ZoneLoadTracker zoneLoadTracker) {}
```

**Resulting constructor** (8 parameters):

```java
public DesmojWarehouseModel(
    Model owner,
    String modelName,
    boolean showInReport,
    boolean showInTrace,
    SimulationConfig config,
    WarehouseComponents warehouseComponents,
    MetricsManager metricsManager,
    OrderProcessingRuntime orderProcessingRuntime) {
```

### 2. Internal Accessors

`DesmojWarehouseModel` will extract managers from `warehouseComponents` internally:

```java
private StoragePositionManager storagePositionManager() {
    return warehouseComponents.storagePositionManager();
}

private StorageSpaceManager storageSpaceManager() {
    return warehouseComponents.storageSpaceManager();
}

private ZoneManager zoneManager() {
    return warehouseComponents.zoneManager();
}

private RelayPositionManager relayPositionManager() {
    return warehouseComponents.relayPositionManager();
}

private GridPopularityManager gridPopularityManager() {
    return warehouseComponents.gridPopularityManager();
}
```

## Thread Safety Changes

### DesmojSimulationEngine

Replace HashMap/HashSet with concurrent implementations:

```java
// Before
private final Map<String, List<String>> orderTaskMapping = new HashMap<>();
private final Set<String> completedTasks = new HashSet<>();

// After
private final Map<String, List<String>> orderTaskMapping = new ConcurrentHashMap<>();
private final Set<String> completedTasks = ConcurrentHashMap.newKeySet();
```

This matches the existing `activeStorageSpaces` pattern in `DesmojWarehouseModel`.

## Exception Handling Changes

### `run()` method - Add cleanup in finally block

```java
public void run() {
    try {
        if (config.systemType() == SystemType.RELAY) {
            initializeRelaySystem();
        }
        loadAndProcessOrders();
        setupExperiment();
        runSimulation();
        log.info("Simulation completed successfully");
    } catch (Exception e) {
        log.error("Simulation failed: {}", e.getMessage(), e);
        throw new RuntimeException("Desmo-J simulation failed", e);
    } finally {
        cleanupExperiment();
    }
}

private void cleanupExperiment() {
    if (experiment != null) {
        try {
            experiment.finish();
        } catch (Exception ignored) {
            // Already finished or failed - ignore
        }
    }
}
```

### `loadAndProcessOrders()` - Fail fast on empty orders

```java
if (batchOrders.isEmpty()) {
    throw new IllegalStateException(
        "No orders loaded from scenario: " + config.scenarioId());
}
```

## Files Changed

| File | Changes |
|------|---------|
| `DesmojWarehouseModel.java` | Add nested `OrderProcessingRuntime` record; reduce constructor to 8 params; add internal accessors for managers |
| `DesmojSimulationEngine.java` | Use `ConcurrentHashMap` for shared state; add `cleanupExperiment()` method; update `run()` with finally block; fail fast on empty orders; update model construction call |

## Testing Approach

**Automated:**
```bash
cd apps/backend && mvn test
```

All existing tests must pass.

**Manual:**
- Frontend integration testing by user

## Risks

- **Low risk**: Changes are mostly structural (parameter grouping, collection types)
- **Exception handling**: Ensure `runSimulation()` doesn't double-call `experiment.finish()`
