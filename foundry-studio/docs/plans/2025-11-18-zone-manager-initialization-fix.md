# Zone Manager Initialization Fix

## Problem

Zone-balanced initial placement is completely ineffective due to incorrect initialization order:

1. `WarehouseBuilder.buildGridPopularityManager()` - creates manager
2. `WarehouseBuilder.buildInventory()` - distributes inventory using `PutProcessor` (zone balancing NOT enabled)
3. `DesmojSimulationEngine.initializeRelaySystem()` - creates ZoneManager
4. `gridPopularityManager.enableZoneLoadBalancing(zoneManager)` - **too late!**

By the time zone balancing is enabled, all inventory is already placed without zone awareness.

## Solution

Move `ZoneManager` creation to `WarehouseBuilder` so it exists before inventory initialization.

## Design Decisions

- **ZoneManager lives in WarehouseComponents** - Zones are fundamentally part of the warehouse layout
- **VTM assignment stays in DesmojSimulationEngine** - Minimal change approach
- **Reuse existing config flag** - `relayZoneLoadBalancingEnabled` controls both runtime and init-time balancing

## Changes

### 1. SimulationMetrics.java

Add `zoneManager` field to `WarehouseComponents` record:

```java
@Builder
public record WarehouseComponents(
    WarehouseMap map,
    GridPopularityManager gridPopularityManager,
    ItemCatalog itemCatalog,
    WarehouseInventory inventory,
    List<Rack> racks,
    List<Tote> totes,
    List<Robot> robots,
    StationManager stationManager,
    ZoneManager zoneManager  // NEW - nullable for RTP
) {}
```

### 2. WarehouseBuilder.java

Add new field and method:

```java
private ZoneManager zoneManager;  // null for RTP

private void buildZoneManager() {
    if (config.systemType() == SystemType.RELAY) {
        zoneManager = new ZoneManager(map);
        zoneManager.generateZones();
        log.info("Generated {} zones", zoneManager.getAllZones().size());
    }
}
```

Update `build()` flow:

```java
public WarehouseComponents build() {
    buildMap();
    buildGridPopularityManager();
    buildZoneManager();  // NEW - after map, before inventory

    // Enable zone balancing BEFORE inventory init
    if (zoneManager != null && config.algorithmConfig().relayZoneLoadBalancingEnabled()) {
        gridPopularityManager.enableZoneLoadBalancing(zoneManager);
        log.info("Zone load balancing enabled for inventory initialization");
    }

    addRacks();
    if (config.systemType() == SystemType.RELAY) {
        addTotes();
    }

    buildItemCatalog();
    buildInventory();  // Now uses zone-aware GridPopularityManager
    buildStationManager();
    buildRobotFleet();

    return WarehouseComponents.builder()
        // ... existing fields
        .zoneManager(zoneManager)  // NEW
        .build();
}
```

### 3. DesmojSimulationEngine.java

Remove ZoneManager creation from `initializeRelaySystem()`:

```java
private void initializeRelaySystem() {
    log.info("Initializing relay system components...");

    // Get ZoneManager from warehouse components (already created by WarehouseBuilder)
    zoneManager = warehouseComponents.zoneManager();

    if (zoneManager == null) {
        throw new IllegalStateException("ZoneManager not found in warehouse components for relay system");
    }

    // REMOVED: zoneManager = new ZoneManager(...)
    // REMOVED: zoneManager.generateZones()

    // Wire ZoneManager to StoragePositionManager for soft zone support
    storagePositionManager.setZoneManager(zoneManager);
    log.info("Wired ZoneManager to StoragePositionManager for soft zone support");

    // VTM assignment stays here
    List<Integer> vtmIds = warehouseComponents.robots().stream()
        .filter(r -> r.getRobotType() == Robot.RobotType.VTM)
        .map(Robot::getId)
        .toList();

    zoneManager.balanceVtmAssignments(vtmIds);
    log.info("Assigned {} VTMs to zones: {}", vtmIds.size(), zoneManager.getStats());

    // ... rest of method unchanged (VTM positioning, etc.)
}
```

Remove zone balancing enablement from `run()`:

```java
// REMOVE these lines from run():
// if (config.algorithmConfig().relayZoneLoadBalancingEnabled() && zoneManager != null) {
//     warehouseComponents.gridPopularityManager().enableZoneLoadBalancing(zoneManager);
//     log.info("Zone load balancing enabled for inventory initialization");
// }
```

## Testing

1. Run relay simulation with `relayZoneLoadBalancingEnabled: true`
2. Verify logs show "Zone load balancing enabled for inventory initialization" before inventory distribution
3. Confirm zone-balanced placement is actually happening (check initial inventory distribution across zones)
