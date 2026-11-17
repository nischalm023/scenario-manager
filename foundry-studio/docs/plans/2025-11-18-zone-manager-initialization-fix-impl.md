# Zone Manager Initialization Fix - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the critical bug where zone-balanced initial placement is ineffective because ZoneManager is created after inventory distribution.

**Architecture:** Move ZoneManager creation from DesmojSimulationEngine to WarehouseBuilder. This ensures ZoneManager exists before GridPopularityManager.enableZoneLoadBalancing() is called, which must happen before PutProcessor.initializeInventory() distributes inventory.

**Tech Stack:** Java 21, Lombok, Spring Boot

---

## Task 1: Add ZoneManager to WarehouseComponents Record

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java:122-131`

**Step 1: Add import for ZoneManager**

At line 10, add the import:

```java
import com.greyorange.dessimulator.domain.robot.ZoneManager;
```

**Step 2: Add zoneManager field to WarehouseComponents record**

Modify lines 122-131 to add the new field:

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
      ZoneManager zoneManager) {}
```

**Step 3: Compile to verify changes**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/metrics/SimulationMetrics.java
git commit -m "feat: add zoneManager to WarehouseComponents record"
```

---

## Task 2: Add ZoneManager Creation to WarehouseBuilder

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java`

**Step 1: Add ZoneManager import**

At line 12, add:

```java
import com.greyorange.dessimulator.domain.robot.ZoneManager;
```

**Step 2: Add zoneManager field**

After line 42 (after stationManager field), add:

```java
  private ZoneManager zoneManager;
```

**Step 3: Add buildZoneManager method**

After the `buildGridPopularityManager()` method (after line 106), add:

```java
  /** Build zone manager for relay systems. */
  private void buildZoneManager() {
    if (config.systemType() == SystemType.RELAY) {
      zoneManager = new ZoneManager(map);
      zoneManager.generateZones();
      log.info("Generated {} zones", zoneManager.getAllZones().size());
    }
  }
```

**Step 4: Update build() method to call buildZoneManager and enable zone balancing**

Replace lines 50-93 with:

```java
  /** Build all warehouse components and return them. */
  public WarehouseComponents build() {
    log.info("Building warehouse infrastructure...");

    // Build map and layout
    buildMap();
    buildGridPopularityManager();
    buildZoneManager();

    // Enable zone balancing BEFORE inventory init
    if (zoneManager != null && config.algorithmConfig().relayZoneLoadBalancingEnabled()) {
      gridPopularityManager.enableZoneLoadBalancing(zoneManager);
      log.info("Zone load balancing enabled for inventory initialization");
    }

    addRacks();

    if (config.systemType() == SystemType.RELAY) {
      addTotes();
    }

    // Build scenario data
    buildItemCatalog();
    buildInventory();

    // Build station manager
    buildStationManager();

    // Build robot fleet
    buildRobotFleet();

    log.info("Warehouse built: {} racks, {} robots", racks.size(), robots.size());

    // Validate required components
    if (map == null
        || gridPopularityManager == null
        || itemCatalog == null
        || warehouseInventory == null
        || stationManager == null) {
      throw new IllegalStateException(
          "Map, grid popularity manager, item catalog, warehouse inventory, or station manager is not built");
    }

    return WarehouseComponents.builder()
        .map(map)
        .gridPopularityManager(gridPopularityManager)
        .itemCatalog(itemCatalog)
        .inventory(warehouseInventory)
        .racks(List.copyOf(racks))
        .totes(List.copyOf(totes))
        .robots(List.copyOf(robots))
        .stationManager(stationManager)
        .zoneManager(zoneManager)
        .build();
  }
```

**Step 5: Compile to verify changes**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 6: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/WarehouseBuilder.java
git commit -m "feat: create ZoneManager in WarehouseBuilder before inventory init"
```

---

## Task 3: Update DesmojSimulationEngine to Use ZoneManager from WarehouseComponents

**Files:**
- Modify: `apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java`

**Step 1: Remove zone balancing enablement from run() method**

In the `run()` method (lines 166-192), remove lines 175-179:

```java
        // Enable zone load balancing for initial inventory placement
        if (config.algorithmConfig().relayZoneLoadBalancingEnabled() && zoneManager != null) {
          warehouseComponents.gridPopularityManager().enableZoneLoadBalancing(zoneManager);
          log.info("Zone load balancing enabled for inventory initialization");
        }
```

The `run()` method should now look like:

```java
  /** Run the simulation (main entry point). */
  public void run() {
    log.info("Starting Desmo-J simulation engine...");

    try {
      // Initialize relay system components if needed
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
    }
  }
```

**Step 2: Update initializeRelaySystem() to get ZoneManager from WarehouseComponents**

Replace lines 213-259 with:

```java
  private void initializeRelaySystem() {
    log.info("Initializing relay system components...");

    // Get ZoneManager from warehouse components (already created by WarehouseBuilder)
    zoneManager = warehouseComponents.zoneManager();

    if (zoneManager == null) {
      throw new IllegalStateException(
          "ZoneManager not found in warehouse components for relay system");
    }

    log.info("Using ZoneManager with {} zones from WarehouseComponents",
        zoneManager.getAllZones().size());

    // Wire ZoneManager to StoragePositionManager for soft zone support
    storagePositionManager.setZoneManager(zoneManager);
    log.info("Wired ZoneManager to StoragePositionManager for soft zone support");

    // NOTE: RelayPositionManager will be created in setupExperiment() after model is connected
    // to experiment, as it needs Desmo-J resources which require an active experiment

    // Get VTM robot IDs
    List<Integer> vtmIds =
        warehouseComponents.robots().stream()
            .filter(r -> r.getRobotType() == Robot.RobotType.VTM)
            .map(Robot::getId)
            .toList();

    // Assign VTMs to zones using balanced assignment
    zoneManager.balanceVtmAssignments(vtmIds);

    log.info("Assigned {} VTMs to zones: {}", vtmIds.size(), zoneManager.getStats());

    // Position VTMs at their zone center positions
    for (Robot robot : warehouseComponents.robots()) {
      if (robot.getRobotType() == Robot.RobotType.VTM) {
        String primaryZoneId = zoneManager.getZoneForVtm(robot.getId());
        if (primaryZoneId != null) {
          Zone zone = zoneManager.getZones().get(primaryZoneId);
          if (zone != null && !zone.getCoverageArea().isEmpty()) {
            // Calculate zone center position
            GridCoordinate centerPos = calculateZoneCenterPosition(zone);
            robot.teleportTo(centerPos);
            log.debug(
                "Positioned VTM {} at zone {} center: {}", robot.getId(), primaryZoneId, centerPos);
          }
        }
      }
    }

    log.info("Relay system initialization complete (RelayPositionManager creation deferred)");
  }
```

**Step 3: Compile to verify changes**

Run: `cd apps/backend && mvn clean compile -DskipTests`

Expected: BUILD SUCCESS

**Step 4: Commit**

```bash
git add apps/backend/src/main/java/com/greyorange/dessimulator/simulator/DesmojSimulationEngine.java
git commit -m "refactor: get ZoneManager from WarehouseComponents instead of creating it"
```

---

## Task 4: Format Code and Run Tests

**Step 1: Format all modified files**

Run: `cd apps/backend && mvn spotless:apply`

Expected: Files formatted successfully

**Step 2: Run compilation with all tests**

Run: `cd apps/backend && mvn clean compile test`

Expected: BUILD SUCCESS, all tests pass

**Step 3: Commit formatting changes (if any)**

```bash
git add -A
git diff --cached --name-only
# If files changed:
git commit -m "chore: format code"
```

---

## Task 5: Manual Verification

**Step 1: Start the backend server**

```bash
rm -f /tmp/server.log
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
sleep 2
npx nx serve backend > /tmp/server.log 2>&1 &
sleep 15
curl -s http://localhost:8080/health
```

Expected: Health check returns OK

**Step 2: Run a relay simulation with zone load balancing enabled**

Create test config at `/tmp/test-zone-fix.json`:

```json
{
  "scenarioId": "default",
  "simulationDuration": 1,
  "systemType": "relay",
  "mapConfig": {
    "gridWidth": 20,
    "gridDepth": 10,
    "rackBlockWidth": 2,
    "rackBlockDepth": 5,
    "aisleWidth": 2,
    "numStations": 2,
    "verticalToteCapacityInRack": 3
  },
  "operatorConfig": {
    "numBins": 10,
    "operatorTimePerPick": 5.0,
    "targetPicksPerFace": 3.0
  },
  "robotConfig": {
    "numVtms": 4,
    "numHtms": 2,
    "vtmHorizontalSpeed": 2.0,
    "vtmVerticalSpeed": 1.0,
    "htmSpeed": 3.0
  },
  "algorithmConfig": {
    "putAlgorithm": "greedy",
    "orderSelectionAlgorithm": "fifo",
    "relayZoneLoadBalancingEnabled": true,
    "relayZoneLoadBalancingBucketSize": 60.0
  }
}
```

Run simulation:

```bash
curl -X POST http://localhost:8080/simulation/run \
  -H "Content-Type: application/json" \
  -d @/tmp/test-zone-fix.json > /tmp/zone-fix-result.json 2>&1
```

**Step 3: Check logs for correct initialization order**

```bash
grep -E "Zone load balancing enabled|Generated.*zones|Distributed inventory" /tmp/server.log
```

Expected output order:
1. `Generated X zones`
2. `Zone load balancing enabled for inventory initialization`
3. `Distributed inventory across Y totes`

This confirms zone balancing is enabled BEFORE inventory distribution.

**Step 4: Cleanup**

```bash
lsof -ti:8080 | xargs kill -9 2>/dev/null || true
rm -f /tmp/server.log /tmp/test-zone-fix.json /tmp/zone-fix-result.json
```

---

## Summary

| Task | Description | Estimated Time |
|------|-------------|----------------|
| 1 | Add ZoneManager to WarehouseComponents | 2 min |
| 2 | Add ZoneManager creation to WarehouseBuilder | 5 min |
| 3 | Update DesmojSimulationEngine | 5 min |
| 4 | Format and test | 3 min |
| 5 | Manual verification | 5 min |

**Total: ~20 minutes**
