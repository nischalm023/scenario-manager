# WarehouseBuilder Improvements Design

## Overview

Improve code quality of WarehouseBuilder.java by addressing type safety, builder safety, and documentation issues identified in code review.

## Scope

**In Scope:**
- Replace `Map<String, Object>` return type with proper DTOs (nested records)
- Add reuse protection with `built` flag
- Add phase-based validation (4 phases)
- Add section markers with heavy dividers
- Expand class-level Javadoc

**Out of Scope:**
- Testability improvements (factory pattern) - deferred for later
- Extracting magic numbers to config - minor, can be done separately

**Files Changed:**
- `WarehouseBuilder.java` - all changes in this single file

**Risk:** Low - all changes are internal to the class. Only API change is `getInitialInventoryState()` return type from `Map<String, Object>` to `InventoryState` record.

## Type-Safe DTOs

### Nested Records

```java
public class WarehouseBuilder {

    // ========================================
    // Public DTOs
    // ========================================

    /** Complete inventory state for metrics tracking. */
    public record InventoryState(
        Map<String, Integer> skuData,
        Map<Integer, StorageMetrics> storageData,
        InventorySummary summary
    ) {}

    /** Metrics for a single storage space (rack or aggregated totes). */
    public record StorageMetrics(
        Position position,
        double popularityScore,
        int uniqueSkus,
        int totalQuantity,
        int popularQuantity,
        int regularQuantity,
        Map<String, Integer> inventory
    ) {}

    /** Grid position. */
    public record Position(int row, int col) {}

    /** Warehouse-level inventory summary. */
    public record InventorySummary(
        int totalSkus,
        int totalUnits,
        int popularSkus,
        int regularSkus,
        int totalPopularQuantity,
        int totalRegularQuantity,
        double popularPercentage,
        Map<String, SkuDetail> skuDetails
    ) {}

    /** Details for a single SKU. */
    public record SkuDetail(int quantity, boolean isPopular) {}
}
```

### Method Signature Change

```java
// Before
public Map<String, Object> getInitialInventoryState()

// After
public InventoryState getInitialInventoryState()
```

This eliminates all three `@SuppressWarnings("unchecked")` annotations.

## Builder Safety

### Reuse Protection

```java
private boolean built = false;

public WarehouseComponents build() {
    if (built) {
        throw new IllegalStateException("WarehouseBuilder can only be used once");
    }

    // ... build logic ...

    built = true;
    return components;
}
```

### Phase-Based Validation

```java
/** Validate that all components in a phase were built successfully. */
private void validatePhase(String phaseName, Object... components) {
    for (Object component : components) {
        if (component == null) {
            throw new IllegalStateException(
                "Failed to build " + phaseName + " phase: component is null");
        }
    }
}
```

### Build Method with Phase Validation

```java
public WarehouseComponents build() {
    if (built) {
        throw new IllegalStateException("WarehouseBuilder can only be used once");
    }

    log.info("Building warehouse infrastructure...");

    // Phase 1: Map
    buildMap();
    buildGridPopularityManager();
    validatePhase("map", map, gridPopularityManager);

    // Phase 2: System-specific
    if (config.systemType() == SystemType.RELAY) {
        buildRelayPositionManager();
        buildZoneManager();
        // Enable zone load balancing...
        validatePhase("system-specific", zoneManager, relayPositionManager);
    }

    // Phase 3: Inventory
    addRacks();
    if (config.systemType() == SystemType.RELAY) {
        addTotes();
    }
    buildItemCatalog();
    buildInventory();
    validatePhase("inventory", itemCatalog, warehouseInventory, storageManager);

    // Phase 4: Robots
    buildStationManager();
    buildRobotFleet();
    validatePhase("robots", stationManager);

    // ... build WarehouseComponents ...

    built = true;
    return components;
}
```

## Documentation and Section Organization

### Section Markers

```java
@Slf4j
public class WarehouseBuilder {

    // ========================================
    // Public DTOs
    // ========================================

    // ========================================
    // Fields
    // ========================================

    // ========================================
    // Constructor
    // ========================================

    // ========================================
    // Public API
    // ========================================

    // ========================================
    // Phase 1: Map Build Methods
    // ========================================

    // ========================================
    // Phase 2: System-Specific Build Methods
    // ========================================

    // ========================================
    // Phase 3: Inventory Build Methods
    // ========================================

    // ========================================
    // Phase 4: Robot Build Methods
    // ========================================

    // ========================================
    // Inventory State Helpers
    // ========================================

    // ========================================
    // Validation Helpers
    // ========================================
}
```

### Expanded Class-Level Javadoc

```java
/**
 * Builds and initializes warehouse components for discrete event simulation.
 *
 * <p>Key responsibilities:
 * <ul>
 *   <li>Build warehouse map and layout from configuration</li>
 *   <li>Generate rack positions and populate with inventory</li>
 *   <li>Create robot fleet (RTP or Relay system based on config)</li>
 *   <li>Initialize station management and zone managers</li>
 * </ul>
 *
 * <p>This is a single-use builder that produces immutable WarehouseComponents.
 * Once {@link #build()} is called, the builder cannot be reused.
 *
 * <p>Build order (4 phases):
 * <ol>
 *   <li>Map - WarehouseMap and GridPopularityManager</li>
 *   <li>System-specific - ZoneManager and RelayPositionManager (RELAY only)</li>
 *   <li>Inventory - Racks, totes, item catalog, warehouse inventory</li>
 *   <li>Robots - StationManager and robot fleet</li>
 * </ol>
 */
```

## Implementation Notes

### Callers to Update

Search for usages of `getInitialInventoryState()` - callers will need to use the new record accessors:

```java
// Before
Map<String, Object> state = builder.getInitialInventoryState();
Map<String, Integer> skuData = (Map<String, Integer>) state.get("initialInventorySkuData");

// After
WarehouseBuilder.InventoryState state = builder.getInitialInventoryState();
Map<String, Integer> skuData = state.skuData();
```

### Testing

- Verify all existing tests pass after refactoring
- Add test for builder reuse prevention
- Test phase validation with intentional failures
